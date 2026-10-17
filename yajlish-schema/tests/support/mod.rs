//! Run the macro's lowering at test time, so that schemas only known at
//! runtime (the official test suite, generated schemas) go through the same
//! lowering and the same runtime as generated handlers.

#![allow(dead_code, clippy::result_large_err)]

#[path = "../../../yajlish-schema-macros/src/lower.rs"]
#[allow(clippy::all, clippy::pedantic)]
pub mod lower;

use lower::{Doc, Ir, Kind, Lit, LowerError, Num, Type};
use yajlish::{Context, Handler, Parser, Status};
use yajlish_schema::{
    __private::{Config, Outcome, Raw, Regexes, Runtime},
    ir, SchemaError,
};

fn leak<T>(v: Vec<T>) -> &'static [T] {
    Box::leak(v.into_boxed_slice())
}

fn leak_str(s: &str) -> &'static str {
    Box::leak(s.to_owned().into_boxed_str())
}

fn num(n: Num) -> ir::Num {
    match n {
        Num::Int(i) => ir::Num::Int(i),
        Num::Float(f) => ir::Num::Float(f),
    }
}

fn lit(l: &Lit) -> ir::Literal {
    match l {
        Lit::Null => ir::Literal::Null,
        Lit::Bool(b) => ir::Literal::Bool(*b),
        Lit::Num(n) => ir::Literal::Num(num(*n)),
        Lit::Str(s) => ir::Literal::Str(leak_str(s)),
        Lit::Array(a) => ir::Literal::Array(leak(a.iter().map(lit).collect())),
        Lit::Object(m) => {
            ir::Literal::Object(leak(m.iter().map(|(k, v)| (leak_str(k), lit(v))).collect()))
        }
    }
}

fn types(t: &Option<Vec<Type>>) -> u8 {
    match t {
        None => ir::types::ANY,
        Some(ts) => ts.iter().fold(0, |acc, t| {
            acc | match t {
                Type::Null => ir::types::NULL,
                Type::Boolean => ir::types::BOOLEAN,
                Type::Integer => ir::types::INTEGER,
                Type::Number => ir::types::NUMBER,
                Type::String => ir::types::STRING,
                Type::Array => ir::types::ARRAY,
                Type::Object => ir::types::OBJECT,
            }
        }),
    }
}

/// The runtime tables for a lowered IR.
pub fn tables(ir: &Ir) -> &'static [ir::Node] {
    leak(
        ir.nodes
            .iter()
            .map(|n| ir::Node {
                path: leak_str(&n.path),
                kind: match &n.kind {
                    Kind::True => ir::NodeKind::True,
                    Kind::False => ir::NodeKind::False,
                    Kind::Schema(s) => ir::NodeKind::Schema(ir::Schema {
                        types: types(&s.types),
                        enum_: s.enum_.as_ref().map(|e| leak(e.iter().map(lit).collect())),
                        const_: s.const_.as_ref().map(|c| &*Box::leak(Box::new(lit(c)))),
                        ref_: s.ref_,
                        all_of: leak(s.all_of.clone()),
                        any_of: leak(s.any_of.clone()),
                        one_of: leak(s.one_of.clone()),
                        not: s.not,
                        properties: leak(
                            s.properties
                                .iter()
                                .map(|(k, id)| (leak_str(k), *id))
                                .collect(),
                        ),
                        required: leak(s.required.iter().map(|r| leak_str(r)).collect()),
                        additional_properties: s.additional_properties,
                        prefix_items: leak(s.prefix_items.clone()),
                        items: s.items,
                        min_items: s.min_items,
                        max_items: s.max_items,
                        min_length: s.min_length,
                        max_length: s.max_length,
                        pattern: s.pattern,
                        minimum: s.minimum.map(num),
                        maximum: s.maximum.map(num),
                        exclusive_minimum: s.exclusive_minimum.map(num),
                        exclusive_maximum: s.exclusive_maximum.map(num),
                        multiple_of: s.multiple_of.map(num),
                    }),
                },
            })
            .collect(),
    )
}

fn check_pattern(p: &str) -> Result<(), String> {
    regex::Regex::new(p).map(|_| ()).map_err(|e| e.to_string())
}

/// Lower the schemas and build a validate-only `Config` with one candidate
/// per schema.
pub fn config(
    schemas: &[serde_json::Value],
    match_first: bool,
) -> Result<&'static Config, LowerError> {
    let docs: Vec<Doc> = schemas
        .iter()
        .enumerate()
        .map(|(i, v)| Doc {
            name: format!("schema{}.json", i),
            value: v.clone(),
        })
        .collect();
    let check: lower::PatternCheck<'_> = if cfg!(feature = "pattern") {
        Some(&check_pattern)
    } else {
        None
    };
    let ir = lower::lower(&docs, check)?;
    let roots: Vec<(&'static str, usize)> = ir
        .doc_roots
        .iter()
        .enumerate()
        .map(|(i, &r)| (leak_str(&format!("Schema{}", i)), r))
        .collect();
    Ok(Box::leak(Box::new(Config {
        nodes: tables(&ir),
        roots: leak(roots),
        match_first,
        at: None,
        envelope: None,
        build: true,
        max_depth: 256,
        patterns: leak(ir.patterns.iter().map(|p| leak_str(p)).collect()),
        regexes: Regexes::new(),
    })))
}

/// A `Handler` that forwards to a `Runtime` and keeps the outcomes.
pub struct Driver {
    pub rt: Runtime,
    pub outcomes: Vec<Outcome>,
}

impl Driver {
    pub fn new(cfg: &'static Config) -> Self {
        Driver {
            rt: Runtime::new(cfg),
            outcomes: vec![],
        }
    }

    fn ev(&mut self, ctx: &Context, raw: Raw<'_>) -> Status {
        if let Some(o) = self.rt.event(ctx.offset(), raw) {
            self.outcomes.push(o);
        }
        if self.rt.is_fatal() {
            Status::Abort
        } else {
            Status::Continue
        }
    }
}

impl Handler for Driver {
    fn handle_null(&mut self, ctx: &Context) -> Status {
        self.ev(ctx, Raw::Null)
    }
    fn handle_double(&mut self, ctx: &Context, v: f64) -> Status {
        self.ev(ctx, Raw::Float(v))
    }
    fn handle_int(&mut self, ctx: &Context, v: i64) -> Status {
        self.ev(ctx, Raw::Int(v))
    }
    fn handle_bool(&mut self, ctx: &Context, v: bool) -> Status {
        self.ev(ctx, Raw::Bool(v))
    }
    fn handle_string(&mut self, ctx: &Context, v: &str) -> Status {
        self.ev(ctx, Raw::Str(v))
    }
    fn handle_start_map(&mut self, ctx: &Context) -> Status {
        self.ev(ctx, Raw::StartMap)
    }
    fn handle_end_map(&mut self, ctx: &Context) -> Status {
        self.ev(ctx, Raw::EndMap)
    }
    fn handle_map_key(&mut self, ctx: &Context, v: &str) -> Status {
        self.ev(ctx, Raw::Key(v))
    }
    fn handle_start_array(&mut self, ctx: &Context) -> Status {
        self.ev(ctx, Raw::StartArray)
    }
    fn handle_end_array(&mut self, ctx: &Context) -> Status {
        self.ev(ctx, Raw::EndArray)
    }
}

/// Validate a JSON text; `Ok(schema index)` or the error.
pub fn validate_text(
    cfg: &'static Config,
    text: &[u8],
    chunk: usize,
) -> Result<usize, SchemaError> {
    let mut driver = Driver::new(cfg);
    let mut parser = Parser::new(&mut driver);
    let mut reader = Chunked { data: text, chunk };
    parser
        .parse(&mut reader)
        .expect("instance must be valid JSON");
    parser.finish_parse().expect("instance must be valid JSON");
    driver.rt.finish()?;
    assert_eq!(driver.outcomes.len(), 1);
    driver.outcomes.pop().unwrap().map(|(i, _)| i)
}

/// A `BufRead` that hands out at most `chunk` bytes per `fill_buf`.
pub struct Chunked<'a> {
    pub data: &'a [u8],
    pub chunk: usize,
}

impl std::io::Read for Chunked<'_> {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        use std::io::BufRead;
        let n = self.fill_buf()?.len().min(out.len());
        out[..n].copy_from_slice(&self.data[..n]);
        self.consume(n);
        Ok(n)
    }
}

impl std::io::BufRead for Chunked<'_> {
    fn fill_buf(&mut self) -> std::io::Result<&[u8]> {
        Ok(&self.data[..self.chunk.min(self.data.len())])
    }
    fn consume(&mut self, n: usize) {
        self.data = &self.data[n..];
    }
}
