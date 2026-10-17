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

/// A parse event with owned strings, for events buffered before the
/// discriminator is seen.
#[derive(Debug, Clone)]
enum OwnedEv {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    StartMap,
    Key(String),
    EndMap,
    StartArray,
    EndArray,
}

impl OwnedEv {
    fn new(ev: &Ev<'_>) -> Self {
        match *ev {
            Ev::Null => OwnedEv::Null,
            Ev::Bool(b) => OwnedEv::Bool(b),
            Ev::Int(i) => OwnedEv::Int(i),
            Ev::Float(x) => OwnedEv::Float(x),
            Ev::Str(s) => OwnedEv::Str(s.to_owned()),
            Ev::StartMap => OwnedEv::StartMap,
            Ev::Key(k) => OwnedEv::Key(k.to_owned()),
            Ev::EndMap => OwnedEv::EndMap,
            Ev::StartArray => OwnedEv::StartArray,
            Ev::EndArray => OwnedEv::EndArray,
        }
    }

    fn ev(&self) -> Ev<'_> {
        match self {
            OwnedEv::Null => Ev::Null,
            OwnedEv::Bool(b) => Ev::Bool(*b),
            OwnedEv::Int(i) => Ev::Int(*i),
            OwnedEv::Float(x) => Ev::Float(*x),
            OwnedEv::Str(s) => Ev::Str(s),
            OwnedEv::StartMap => Ev::StartMap,
            OwnedEv::Key(k) => Ev::Key(k),
            OwnedEv::EndMap => Ev::EndMap,
            OwnedEv::StartArray => Ev::StartArray,
            OwnedEv::EndArray => Ev::EndArray,
        }
    }
}

/// Events buffered, with their offsets, until the discriminator's value
/// says which candidates to run.
type Buffer = Vec<(u64, OwnedEv)>;

/// Past this many buffered events without seeing the discriminator, run
/// every candidate.
const MAX_BUFFERED: usize = 4096;

struct Pending {
    buf: Buffer,
    /// The last key of each open object in the record (`None` for arrays).
    keys: Vec<Option<String>>,
}

struct Record {
    /// Waiting for the discriminator; no candidate runs yet.
    pending: Option<Pending>,
    /// Live candidates: schema index and matcher.
    cands: Vec<(usize, Matcher)>,
    passed: Vec<usize>,
    failed: Vec<(usize, Box<Failure>)>,
    /// Candidates the discriminator ruled out, with the events up to it,
    /// to compute their errors if no candidate matches after all.
    rejected: Option<(Buffer, Vec<usize>)>,
    builder: Option<ValueBuilder>,
    depth: u32,
}

impl Record {
    fn new(cx: &Cx<'_>) -> Self {
        let cfg = cx.cfg;
        let mut r = Record {
            pending: None,
            cands: vec![],
            passed: vec![],
            failed: vec![],
            rejected: None,
            builder: if cfg.build {
                Some(ValueBuilder::default())
            } else {
                None
            },
            depth: 0,
        };
        if cfg.discriminator.is_some() {
            r.pending = Some(Pending {
                buf: vec![],
                keys: vec![],
            });
        } else {
            r.start(cx, (0..cfg.roots.len()).collect());
        }
        r
    }

    fn start(&mut self, cx: &Cx<'_>, which: Vec<usize>) {
        self.cands = which
            .into_iter()
            .map(|i| (i, Matcher::new(cx, cx.cfg.roots[i].1, 0)))
            .collect();
    }

    /// Feed one event; the outcome once the record's value has ended.
    fn feed(&mut self, cx: &Cx<'_>, ev: &Ev<'_>) -> Option<Outcome> {
        match ev {
            Ev::StartMap | Ev::StartArray => self.depth += 1,
            Ev::EndMap | Ev::EndArray => self.depth -= 1,
            _ => {}
        }
        let ended = self.depth == 0 && !matches!(ev, Ev::Key(_));

        if let Some(p) = &mut self.pending {
            p.buf.push((cx.offset, OwnedEv::new(ev)));
            let disc = cx
                .cfg
                .discriminator
                .as_ref()
                .expect("pending needs a discriminator");
            let mut chosen = None;
            if let Ev::Str(value) = ev {
                let at_path = p.keys.len() == disc.path.len()
                    && p.keys
                        .iter()
                        .zip(disc.path)
                        .all(|(k, d)| k.as_deref() == Some(*d));
                if at_path {
                    let hits: Vec<usize> = disc
                        .values
                        .iter()
                        .filter(|(v, _)| v == value)
                        .map(|&(_, i)| i)
                        .collect();
                    // No hit: every candidate fails; run them all for the
                    // errors.
                    chosen = Some(if hits.is_empty() {
                        (0..cx.cfg.roots.len()).collect()
                    } else {
                        hits
                    });
                }
            }
            match ev {
                Ev::StartMap | Ev::StartArray => p.keys.push(None),
                Ev::Key(k) => {
                    if let Some(top) = p.keys.last_mut() {
                        *top = Some((*k).to_owned());
                    }
                }
                Ev::EndMap | Ev::EndArray => {
                    p.keys.pop();
                }
                _ => {}
            }
            if chosen.is_none() && (ended || p.buf.len() > MAX_BUFFERED) {
                chosen = Some((0..cx.cfg.roots.len()).collect());
            }
            let chosen = chosen?;
            let buf = self.pending.take().map(|p| p.buf).unwrap_or_default();
            let rejected: Vec<usize> = (0..cx.cfg.roots.len())
                .filter(|i| !chosen.contains(i))
                .collect();
            self.start(cx, chosen);
            for (offset, owned) in &buf {
                self.feed_live(
                    &Cx {
                        cfg: cx.cfg,
                        offset: *offset,
                    },
                    &owned.ev(),
                );
            }
            if !rejected.is_empty() {
                self.rejected = Some((buf, rejected));
            }
        } else {
            self.feed_live(cx, ev);
        }

        if ended {
            Some(self.decide(cx))
        } else {
            None
        }
    }

    fn feed_live(&mut self, cx: &Cx<'_>, ev: &Ev<'_>) {
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
    }

    /// The errors of the candidates the discriminator ruled out: replay the
    /// events up to the discriminator, where each of them fails.
    fn rejected_failures(&mut self, cfg: &Config) {
        let Some((buf, rejected)) = self.rejected.take() else {
            return;
        };
        for idx in rejected {
            let mut m = Matcher::new(&Cx { cfg, offset: 0 }, cfg.roots[idx].1, 0);
            for (offset, owned) in &buf {
                let cx = Cx {
                    cfg,
                    offset: *offset,
                };
                match m.feed(&cx, &owned.ev()) {
                    Step::Continue => {}
                    Step::Pass => break,
                    Step::Fail(f) => {
                        self.failed.push((idx, f));
                        break;
                    }
                }
            }
        }
    }

    fn decide(&mut self, cx: &Cx<'_>) -> Outcome {
        let cfg = cx.cfg;
        self.passed.sort_unstable();
        let value = self.builder.as_mut().and_then(ValueBuilder::take);
        match (&self.passed[..], cfg.match_first) {
            ([winner, ..], true) | ([winner], false) => Ok((*winner, value)),
            ([], _) => {
                self.rejected_failures(cfg);
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
