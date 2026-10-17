//! The state machine behind every generated handler: locates records,
//! feeds them to one candidate matcher per schema, builds their values and
//! decides which schema matched.

use crate::{
    __private::Config,
    error::{Failure, SchemaError},
    ir::{NodeId, PathSeg},
    matcher::{Cx, Ev, Matcher, Step},
    value::{BuildError, Value, ValueBuilder},
};
use yajlish::decode_str;

/// A parse event as the `Handler` receives it: strings and keys raw.
#[doc(hidden)]
#[derive(Debug, Clone, Copy)]
pub enum Raw<'a> {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(&'a str),
    StartMap,
    Key(&'a str),
    EndMap,
    StartArray,
    EndArray,
}

/// A finished record: the index of the matching schema in `schemas {}` and,
/// in build mode, its value.
#[doc(hidden)]
pub type Outcome = Result<(usize, Option<Value>), SchemaError>;

struct Frame {
    is_map: bool,
    key: String,
    index: u64,
}

struct Record {
    /// Live candidates: schema index and matcher.
    cands: Vec<(usize, Matcher)>,
    passed: Vec<usize>,
    failed: Vec<(usize, Box<Failure>)>,
    builder: Option<ValueBuilder>,
    depth: u32,
}

impl Record {
    fn new(cx: &Cx<'_>) -> Self {
        Record {
            cands: cx
                .cfg
                .roots
                .iter()
                .enumerate()
                .map(|(i, &(_, node))| (i, Matcher::new(cx, node, 0)))
                .collect(),
            passed: vec![],
            failed: vec![],
            builder: if cx.cfg.build {
                Some(ValueBuilder::default())
            } else {
                None
            },
            depth: 0,
        }
    }

    /// Feed one event; the outcome once the record's value has ended.
    fn feed(&mut self, cx: &Cx<'_>, ev: &Ev<'_>) -> Option<Outcome> {
        let mut i = 0;
        while i < self.cands.len() {
            match self.cands[i].1.feed(cx, ev) {
                Step::Continue => i += 1,
                Step::Pass => {
                    let (idx, _) = self.cands.swap_remove(i);
                    self.passed.push(idx);
                }
                Step::Fail(f) => {
                    let (idx, _) = self.cands.swap_remove(i);
                    self.failed.push((idx, f));
                }
            }
        }
        if self.cands.is_empty() && self.passed.is_empty() {
            // Nothing can match any more: stop building.
            self.builder = None;
        }
        if let Some(b) = &mut self.builder {
            b.event(ev);
        }

        match ev {
            Ev::StartMap | Ev::StartArray => self.depth += 1,
            Ev::EndMap | Ev::EndArray => self.depth -= 1,
            _ => {}
        }
        if self.depth > 0 || matches!(ev, Ev::Key(_)) {
            return None;
        }
        Some(self.decide(cx))
    }

    fn decide(&mut self, cx: &Cx<'_>) -> Outcome {
        let cfg = cx.cfg;
        self.passed.sort_unstable();
        let value = self.builder.as_mut().and_then(ValueBuilder::take);
        match (&self.passed[..], cfg.match_first) {
            ([winner, ..], true) | ([winner], false) => Ok((*winner, value)),
            ([], _) => {
                self.failed.sort_by_key(|(idx, _)| *idx);
                let mut failed = std::mem::take(&mut self.failed);
                if failed.len() == 1 {
                    return Err(failed.remove(0).1.into_error());
                }
                let names: Vec<&str> = failed.iter().map(|(i, _)| cfg.roots[*i].0).collect();
                Err(SchemaError {
                    instance_path: String::new(),
                    schema_path: String::new(),
                    keyword: "match",
                    offset: Some(cx.offset),
                    detail: format!("matched none of {}", names.join(", ")),
                    causes: failed.into_iter().map(|(_, f)| f.into_error()).collect(),
                })
            }
            (many, false) => {
                let names: Vec<&str> = many.iter().map(|i| cfg.roots[*i].0).collect();
                Err(SchemaError {
                    instance_path: String::new(),
                    schema_path: String::new(),
                    keyword: "match",
                    offset: Some(cx.offset),
                    detail: format!(
                        "matched {}; `match = unique` needs exactly one",
                        names.join(" and ")
                    ),
                    causes: Box::default(),
                })
            }
        }
    }
}

/// Drives validation for a generated handler. Not for direct use.
#[doc(hidden)]
pub struct Runtime {
    cfg: &'static Config,
    stack: Vec<Frame>,
    /// Stack depth of the selected array while inside it.
    selected: Option<usize>,
    found: bool,
    record: Option<Record>,
    root_started: bool,
    root_done: bool,
    envelope: Option<Matcher>,
    envelope_done: bool,
    fatal: Option<SchemaError>,
    aborted: bool,
}

impl Runtime {
    #[must_use]
    pub fn new(cfg: &'static Config) -> Self {
        let envelope = cfg
            .envelope
            .map(|node| Matcher::new(&Cx { cfg, offset: 0 }, node, 0));
        Runtime {
            cfg,
            stack: vec![],
            selected: None,
            found: false,
            record: None,
            root_started: false,
            root_done: false,
            envelope_done: envelope.is_none(),
            envelope,
            fatal: None,
            aborted: false,
        }
    }

    /// Whether an envelope error has stopped validation.
    #[must_use]
    pub fn is_fatal(&self) -> bool {
        self.fatal.is_some()
    }

    /// The callback asked to stop.
    pub fn note_abort(&mut self) {
        self.aborted = true;
    }

    /// The number of schemas still able to match the current record, for
    /// tests of candidate pruning.
    #[must_use]
    pub fn live_candidates(&self) -> usize {
        self.record
            .as_ref()
            .map_or(0, |r| r.cands.len() + r.passed.len())
    }

    /// Process one event; the outcome of a record that ended with it.
    pub fn event(&mut self, offset: u64, raw: Raw<'_>) -> Option<Outcome> {
        if self.fatal.is_some() {
            return None;
        }
        let decoded;
        let ev = match raw {
            Raw::Null => Ev::Null,
            Raw::Bool(b) => Ev::Bool(b),
            Raw::Int(i) => Ev::Int(i),
            Raw::Float(x) => Ev::Float(x),
            Raw::StartMap => Ev::StartMap,
            Raw::EndMap => Ev::EndMap,
            Raw::StartArray => Ev::StartArray,
            Raw::EndArray => Ev::EndArray,
            Raw::Str(s) | Raw::Key(s) => {
                decoded = match decode_str(s) {
                    Ok(d) => d,
                    Err(e) => {
                        self.fatal = Some(SchemaError {
                            instance_path: String::new(),
                            schema_path: String::new(),
                            keyword: "string",
                            offset: Some(offset),
                            detail: format!("string cannot be decoded: {}", e),
                            causes: Box::default(),
                        });
                        return None;
                    }
                };
                if let Raw::Key(_) = raw {
                    Ev::Key(&decoded)
                } else {
                    Ev::Str(&decoded)
                }
            }
        };
        let cx = Cx {
            cfg: self.cfg,
            offset,
        };

        let starts_value = !matches!(ev, Ev::Key(_) | Ev::EndMap | Ev::EndArray);
        if self.record.is_none() && starts_value {
            let starts = match self.cfg.at {
                None => self.stack.is_empty() && !self.root_started,
                Some(_) => self.selected == Some(self.stack.len()),
            };
            if starts {
                self.root_started = true;
                self.record = Some(Record::new(&cx));
            }
        }

        if let Some(m) = &mut self.envelope {
            match m.feed(&cx, &ev) {
                Step::Continue => {}
                Step::Pass => {
                    self.envelope = None;
                    self.envelope_done = true;
                }
                Step::Fail(f) => {
                    self.envelope = None;
                    self.fatal = Some(f.into_error());
                }
            }
        }

        let mut out = None;
        if let Some(r) = &mut self.record {
            out = r.feed(&cx, &ev);
            if out.is_some() {
                self.record = None;
                if self.cfg.at.is_none() {
                    self.root_done = true;
                }
            }
        }

        match ev {
            Ev::StartMap => self.stack.push(Frame {
                is_map: true,
                key: String::new(),
                index: 0,
            }),
            Ev::Key(k) => {
                if let Some(top) = self.stack.last_mut() {
                    top.key.clear();
                    top.key.push_str(k);
                }
            }
            Ev::StartArray => {
                if self.selected.is_none() && self.at_selected_location() {
                    self.found = true;
                    self.selected = Some(self.stack.len() + 1);
                }
                self.stack.push(Frame {
                    is_map: false,
                    key: String::new(),
                    index: 0,
                });
            }
            Ev::EndMap | Ev::EndArray => {
                if self.selected == Some(self.stack.len()) {
                    self.selected = None;
                }
                self.stack.pop();
                self.next_index();
            }
            _ => self.next_index(),
        }
        out
    }

    fn next_index(&mut self) {
        if let Some(top) = self.stack.last_mut() {
            if !top.is_map {
                top.index += 1;
            }
        }
    }

    /// Whether the value about to start is at the `at` path.
    fn at_selected_location(&self) -> bool {
        let Some(at) = self.cfg.at else {
            return false;
        };
        at.len() == self.stack.len()
            && at.iter().zip(&self.stack).all(|(seg, frame)| match seg {
                PathSeg::Key(k) => frame.is_map && frame.key == *k,
                PathSeg::Index(i) => !frame.is_map && frame.index == *i,
            })
    }

    /// Errors that are not about a single record.
    ///
    /// # Errors
    /// The envelope did not validate, a string could not be decoded, the
    /// `at` array never appeared, or the document ended early.
    pub fn finish(&self) -> Result<(), SchemaError> {
        if let Some(e) = &self.fatal {
            return Err(e.clone());
        }
        if self.aborted {
            return Ok(());
        }
        let incomplete = |detail: String| SchemaError {
            instance_path: String::new(),
            schema_path: String::new(),
            keyword: "",
            offset: None,
            detail,
            causes: Box::default(),
        };
        if let Some(at) = self.cfg.at {
            if !self.found {
                let mut keyword = incomplete(format!("no array at {}", show_path(at)));
                keyword.keyword = "at";
                return Err(keyword);
            }
        } else if !self.root_done {
            return Err(incomplete(
                "the document ended before a complete value".into(),
            ));
        }
        if !self.envelope_done {
            return Err(incomplete(
                "the document ended before a complete value".into(),
            ));
        }
        Ok(())
    }
}

fn show_path(at: &[PathSeg]) -> String {
    let mut s = String::new();
    for seg in at {
        match seg {
            PathSeg::Key(k) => {
                s.push('.');
                s.push_str(k);
            }
            PathSeg::Index(i) => s.push_str(&format!("[{}]", i)),
        }
    }
    s
}

/// A `BuildError` for schema `name`, as a `SchemaError`.
#[doc(hidden)]
#[must_use]
pub fn build_error(cfg: &Config, schema: usize, e: BuildError) -> SchemaError {
    SchemaError {
        instance_path: e.instance_path,
        schema_path: cfg.nodes[cfg.roots[schema].1].path.to_owned(),
        keyword: "build",
        offset: None,
        detail: e.detail,
        causes: Box::default(),
    }
}

/// Whether `v` matches `node`, for generated `oneOf`/`anyOf` conversions.
#[doc(hidden)]
#[must_use]
pub fn is_valid(cfg: &Config, node: NodeId, v: &Value) -> bool {
    crate::matcher::validate_value(cfg, node, v).is_ok()
}
