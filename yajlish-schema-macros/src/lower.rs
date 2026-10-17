//! Lowering JSON Schema documents to an IR graph.
//!
//! This module depends only on `serde_json`, so `yajlish-schema`'s tests
//! include it directly to run the official test suite through the same
//! lowering the macro uses.

use serde_json::Value;
use std::collections::HashMap;

/// The 2020-12 meta-schema URI, the only dialect accepted.
pub const DIALECT: &str = "https://json-schema.org/draft/2020-12/schema";

/// One schema file.
pub struct Doc {
    /// The path as written in `schemas {}`, e.g. `schemas/point.schema.json`.
    pub name: String,
    pub value: Value,
}

/// The lowered schema set. Node ids index `nodes`.
#[derive(Debug)]
pub struct Ir {
    pub nodes: Vec<Node>,
    /// Root node of each document, in the order given.
    pub doc_roots: Vec<usize>,
    pub patterns: Vec<String>,
}

#[derive(Debug)]
pub struct Node {
    /// `point.schema.json#/properties/x`
    pub path: String,
    /// The document and JSON Pointer the node came from.
    pub doc: usize,
    pub pointer: String,
    pub kind: Kind,
    pub title: Option<String>,
    pub description: Option<String>,
    /// The key under `$defs` when the node is a definition.
    pub def_name: Option<String>,
}

#[derive(Debug)]
#[allow(clippy::large_enum_variant)]
pub enum Kind {
    True,
    False,
    Schema(Schema),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Type {
    Null,
    Boolean,
    Integer,
    Number,
    String,
    Array,
    Object,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Num {
    Int(i64),
    Float(f64),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Lit {
    Null,
    Bool(bool),
    Num(Num),
    Str(String),
    Array(Vec<Lit>),
    Object(Vec<(String, Lit)>),
}

#[derive(Debug, Default)]
pub struct Schema {
    /// `None` when there is no `type` keyword.
    pub types: Option<Vec<Type>>,
    pub enum_: Option<Vec<Lit>>,
    pub const_: Option<Lit>,
    pub ref_: Option<usize>,
    pub all_of: Vec<usize>,
    pub any_of: Vec<usize>,
    pub one_of: Vec<usize>,
    pub not: Option<usize>,
    pub properties: Vec<(String, usize)>,
    pub required: Vec<String>,
    pub additional_properties: Option<usize>,
    pub prefix_items: Vec<usize>,
    pub items: Option<usize>,
    pub min_items: Option<u64>,
    pub max_items: Option<u64>,
    pub min_length: Option<u64>,
    pub max_length: Option<u64>,
    pub pattern: Option<usize>,
    pub minimum: Option<Num>,
    pub maximum: Option<Num>,
    pub exclusive_minimum: Option<Num>,
    pub exclusive_maximum: Option<Num>,
    pub multiple_of: Option<Num>,
}

impl Schema {
    /// Whether any keyword that only applies to objects is present.
    pub fn has_object_keywords(&self) -> bool {
        !self.properties.is_empty()
            || !self.required.is_empty()
            || self.additional_properties.is_some()
    }
}

/// A schema the macro cannot accept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LowerError {
    /// The document, as written in `schemas {}`.
    pub file: String,
    /// JSON Pointer of the offending keyword or schema.
    pub pointer: String,
    /// The keyword, when the problem is one keyword.
    pub keyword: Option<String>,
    pub msg: String,
    /// Whether the problem is an unsupported (as opposed to invalid) schema.
    pub unsupported: bool,
}

impl std::fmt::Display for LowerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}#{}", self.file, self.pointer)?;
        if let Some(k) = &self.keyword {
            write!(f, " (`{}`)", k)?;
        }
        write!(f, ": {}", self.msg)
    }
}

/// Checks a `pattern`; `None` means the `pattern` feature is off.
pub type PatternCheck<'a> = Option<&'a dyn Fn(&str) -> Result<(), String>>;

/// Keywords that are annotations only.
const ANNOTATIONS: &[&str] = &[
    "title",
    "description",
    "default",
    "examples",
    "$comment",
    "format",
    "deprecated",
    "readOnly",
    "writeOnly",
];

struct Lowerer<'a> {
    docs: &'a [Doc],
    nodes: Vec<Node>,
    memo: HashMap<(usize, String), usize>,
    queue: Vec<usize>,
    patterns: Vec<String>,
    check_pattern: PatternCheck<'a>,
}

/// Lower every document. Each document's root and all its `$defs` are
/// lowered, so unsupported keywords anywhere are reported.
pub fn lower(docs: &[Doc], check_pattern: PatternCheck<'_>) -> Result<Ir, LowerError> {
    let mut l = Lowerer {
        docs,
        nodes: vec![],
        memo: HashMap::new(),
        queue: vec![],
        patterns: vec![],
        check_pattern,
    };
    let doc_roots: Vec<usize> = (0..docs.len())
        .map(|d| l.node_at(d, String::new()))
        .collect();
    while let Some(id) = l.queue.pop() {
        l.lower_node(id)?;
    }
    l.check_cycles()?;
    Ok(Ir {
        nodes: l.nodes,
        doc_roots,
        patterns: l.patterns,
    })
}

fn escape_pointer(key: &str) -> String {
    key.replace('~', "~0").replace('/', "~1")
}

fn basename(name: &str) -> &str {
    name.rsplit('/').next().unwrap_or(name)
}

impl<'a> Lowerer<'a> {
    fn err(
        &self,
        doc: usize,
        pointer: &str,
        keyword: Option<&str>,
        msg: impl Into<String>,
    ) -> LowerError {
        LowerError {
            file: self.docs[doc].name.clone(),
            pointer: pointer.to_owned(),
            keyword: keyword.map(str::to_owned),
            msg: msg.into(),
            unsupported: false,
        }
    }

    fn unsupported(
        &self,
        doc: usize,
        pointer: &str,
        keyword: &str,
        msg: impl Into<String>,
    ) -> LowerError {
        LowerError {
            unsupported: true,
            ..self.err(doc, pointer, Some(keyword), msg)
        }
    }

    /// The node for `doc#pointer`, allocated and queued on first use.
    fn node_at(&mut self, doc: usize, pointer: String) -> usize {
        if let Some(&id) = self.memo.get(&(doc, pointer.clone())) {
            return id;
        }
        let id = self.nodes.len();
        self.nodes.push(Node {
            path: format!("{}#{}", basename(&self.docs[doc].name), pointer),
            doc,
            pointer: pointer.clone(),
            kind: Kind::True,
            title: None,
            description: None,
            def_name: None,
        });
        self.memo.insert((doc, pointer), id);
        self.queue.push(id);
        id
    }

    fn resolve_pointer(&self, doc: usize, pointer: &str) -> Option<&'a Value> {
        let mut v = &self.docs[doc].value;
        if pointer.is_empty() {
            return Some(v);
        }
        for raw in pointer.strip_prefix('/')?.split('/') {
            let token = raw.replace("~1", "/").replace("~0", "~");
            v = match v {
                Value::Object(m) => m.get(&token)?,
                Value::Array(a) => a.get(token.parse::<usize>().ok()?)?,
                _ => return None,
            };
        }
        Some(v)
    }

    fn lower_node(&mut self, id: usize) -> Result<(), LowerError> {
        let doc = self.nodes[id].doc;
        let ptr = self.nodes[id].pointer.clone();
        let value = self
            .resolve_pointer(doc, &ptr)
            .ok_or_else(|| self.err(doc, &ptr, None, "no schema at this location"))?;
        let obj = match value {
            Value::Bool(true) => return Ok(()),
            Value::Bool(false) => {
                self.nodes[id].kind = Kind::False;
                return Ok(());
            }
            Value::Object(obj) => obj,
            _ => return Err(self.err(doc, &ptr, None, "a schema must be an object or a boolean")),
        };
        let mut s = Schema::default();
        for (key, v) in obj {
            let kp = format!("{}/{}", ptr, escape_pointer(key));
            self.keyword(id, &mut s, key, v, &kp)?;
        }
        self.nodes[id].kind = Kind::Schema(s);
        Ok(())
    }

    fn sub(&mut self, doc: usize, pointer: &str) -> usize {
        self.node_at(doc, pointer.to_owned())
    }

    fn sub_list(
        &mut self,
        doc: usize,
        kp: &str,
        key: &str,
        v: &Value,
    ) -> Result<Vec<usize>, LowerError> {
        match v {
            Value::Array(items) if !items.is_empty() => Ok((0..items.len())
                .map(|i| self.sub(doc, &format!("{}/{}", kp, i)))
                .collect()),
            _ => Err(self.err(doc, kp, Some(key), "expected a non-empty array of schemas")),
        }
    }

    fn count(&self, doc: usize, kp: &str, key: &str, v: &Value) -> Result<u64, LowerError> {
        match v {
            Value::Number(n) => n
                .as_u64()
                .or_else(|| {
                    n.as_f64()
                        .filter(|f| *f >= 0.0 && f.fract() == 0.0 && *f < 1.8e19)
                        .map(|f| f as u64)
                })
                .ok_or_else(|| self.err(doc, kp, Some(key), "expected a non-negative integer")),
            _ => Err(self.err(doc, kp, Some(key), "expected a non-negative integer")),
        }
    }

    fn number(&self, doc: usize, kp: &str, key: &str, v: &Value) -> Result<Num, LowerError> {
        match v {
            Value::Number(n) => Ok(num(n)),
            _ => Err(self.err(doc, kp, Some(key), "expected a number")),
        }
    }

    #[allow(clippy::too_many_lines)]
    fn keyword(
        &mut self,
        id: usize,
        s: &mut Schema,
        key: &str,
        v: &Value,
        kp: &str,
    ) -> Result<(), LowerError> {
        let doc = self.nodes[id].doc;
        let at_root = self.nodes[id].pointer.is_empty();
        match key {
            "$schema" => {
                if !at_root {
                    return Err(self.unsupported(
                        doc,
                        kp,
                        key,
                        "`$schema` is only supported at the root of a file",
                    ));
                }
                match v.as_str().map(|d| d.trim_end_matches('#')) {
                    Some(DIALECT) => {}
                    Some(other) => {
                        return Err(self.unsupported(
                            doc,
                            kp,
                            key,
                            format!(
                                "only JSON Schema draft 2020-12 ({}) is supported, not {}",
                                DIALECT, other
                            ),
                        ))
                    }
                    None => return Err(self.err(doc, kp, Some(key), "expected a string")),
                }
            }
            "$id" => {
                if !at_root {
                    return Err(self.unsupported(
                        doc,
                        kp,
                        key,
                        "`$id` is only supported at the root of a file",
                    ));
                }
                if !v.is_string() {
                    return Err(self.err(doc, kp, Some(key), "expected a string"));
                }
            }
            "$defs" => match v {
                Value::Object(defs) => {
                    for name in defs.keys() {
                        let child = self.sub(doc, &format!("{}/{}", kp, escape_pointer(name)));
                        self.nodes[child]
                            .def_name
                            .get_or_insert_with(|| name.clone());
                    }
                }
                _ => return Err(self.err(doc, kp, Some(key), "expected an object of schemas")),
            },
            "$ref" => {
                let r = v
                    .as_str()
                    .ok_or_else(|| self.err(doc, kp, Some(key), "expected a string"))?;
                s.ref_ = Some(self.resolve_ref(doc, kp, r)?);
            }
            "type" => {
                let names: Vec<&Value> = match v {
                    Value::Array(a) if !a.is_empty() => a.iter().collect(),
                    Value::String(_) => vec![v],
                    _ => {
                        return Err(self.err(
                            doc,
                            kp,
                            Some(key),
                            "expected a type name or an array of them",
                        ))
                    }
                };
                let mut types = vec![];
                for n in names {
                    let t = match n.as_str() {
                        Some("null") => Type::Null,
                        Some("boolean") => Type::Boolean,
                        Some("integer") => Type::Integer,
                        Some("number") => Type::Number,
                        Some("string") => Type::String,
                        Some("array") => Type::Array,
                        Some("object") => Type::Object,
                        _ => {
                            return Err(self.err(doc, kp, Some(key), format!("unknown type {}", n)))
                        }
                    };
                    if types.contains(&t) {
                        return Err(self.err(doc, kp, Some(key), "duplicate type"));
                    }
                    types.push(t);
                }
                s.types = Some(types);
            }
            "enum" => match v {
                Value::Array(a) => s.enum_ = Some(a.iter().map(lit).collect()),
                _ => return Err(self.err(doc, kp, Some(key), "expected an array")),
            },
            "const" => s.const_ = Some(lit(v)),
            "allOf" => s.all_of = self.sub_list(doc, kp, key, v)?,
            "anyOf" => s.any_of = self.sub_list(doc, kp, key, v)?,
            "oneOf" => s.one_of = self.sub_list(doc, kp, key, v)?,
            "not" => s.not = Some(self.sub(doc, kp)),
            "properties" => match v {
                Value::Object(props) => {
                    for name in props.keys() {
                        let child = self.sub(doc, &format!("{}/{}", kp, escape_pointer(name)));
                        s.properties.push((name.clone(), child));
                    }
                }
                _ => return Err(self.err(doc, kp, Some(key), "expected an object of schemas")),
            },
            "required" => match v {
                Value::Array(a) => {
                    for r in a {
                        let r = r.as_str().ok_or_else(|| {
                            self.err(doc, kp, Some(key), "expected an array of strings")
                        })?;
                        if s.required.iter().any(|x| x == r) {
                            return Err(self.err(doc, kp, Some(key), "duplicate property name"));
                        }
                        s.required.push(r.to_owned());
                    }
                }
                _ => return Err(self.err(doc, kp, Some(key), "expected an array of strings")),
            },
            "additionalProperties" => s.additional_properties = Some(self.sub(doc, kp)),
            "prefixItems" => s.prefix_items = self.sub_list(doc, kp, key, v)?,
            "items" => {
                if v.is_array() {
                    return Err(self.unsupported(
                        doc,
                        kp,
                        key,
                        "the array form of `items` is draft-07; use `prefixItems` in 2020-12",
                    ));
                }
                s.items = Some(self.sub(doc, kp));
            }
            "minItems" => s.min_items = Some(self.count(doc, kp, key, v)?),
            "maxItems" => s.max_items = Some(self.count(doc, kp, key, v)?),
            "minLength" => s.min_length = Some(self.count(doc, kp, key, v)?),
            "maxLength" => s.max_length = Some(self.count(doc, kp, key, v)?),
            "pattern" => {
                let p = v
                    .as_str()
                    .ok_or_else(|| self.err(doc, kp, Some(key), "expected a string"))?;
                match self.check_pattern {
                    None => {
                        return Err(self.unsupported(
                            doc,
                            kp,
                            key,
                            "`pattern` needs the `pattern` feature of yajlish-schema",
                        ))
                    }
                    Some(check) => check(p).map_err(|e| self.err(doc, kp, Some(key), e))?,
                }
                s.pattern = Some(self.patterns.len());
                self.patterns.push(p.to_owned());
            }
            "minimum" => s.minimum = Some(self.number(doc, kp, key, v)?),
            "maximum" => s.maximum = Some(self.number(doc, kp, key, v)?),
            "exclusiveMinimum" => s.exclusive_minimum = Some(self.number(doc, kp, key, v)?),
            "exclusiveMaximum" => s.exclusive_maximum = Some(self.number(doc, kp, key, v)?),
            "multipleOf" => {
                let n = self.number(doc, kp, key, v)?;
                let positive = match n {
                    Num::Int(i) => i > 0,
                    Num::Float(f) => f > 0.0,
                };
                if !positive {
                    return Err(self.err(doc, kp, Some(key), "expected a number greater than 0"));
                }
                s.multiple_of = Some(n);
            }
            "title" => {
                self.nodes[id].title = Some(
                    v.as_str()
                        .ok_or_else(|| self.err(doc, kp, Some(key), "expected a string"))?
                        .to_owned(),
                );
            }
            "description" => {
                self.nodes[id].description = Some(
                    v.as_str()
                        .ok_or_else(|| self.err(doc, kp, Some(key), "expected a string"))?
                        .to_owned(),
                );
            }
            k if ANNOTATIONS.contains(&k) => {}
            _ => {
                return Err(self.unsupported(
                    doc,
                    kp,
                    key,
                    format!("unsupported keyword `{}`", key),
                ));
            }
        }
        Ok(())
    }

    fn resolve_ref(&mut self, doc: usize, kp: &str, r: &str) -> Result<usize, LowerError> {
        let (file, frag) = match r.find('#') {
            Some(i) => (&r[..i], &r[i + 1..]),
            None => (r, ""),
        };
        if file.contains("://") || file.starts_with("urn:") {
            return Err(self.unsupported(
                doc,
                kp,
                "$ref",
                format!("remote `$ref` {:?} is not supported", r),
            ));
        }
        let target_doc = if file.is_empty() {
            doc
        } else {
            self.find_doc(doc, file).ok_or_else(|| {
                self.unsupported(
                    doc,
                    kp,
                    "$ref",
                    format!("`$ref` {:?} names a file that is not in the schema set", r),
                )
            })?
        };
        let pointer = percent_decode(frag);
        if !pointer.is_empty() && !pointer.starts_with('/') {
            return Err(self.unsupported(
                doc,
                kp,
                "$ref",
                format!("anchor `$ref` {:?} is not supported", r),
            ));
        }
        if self.resolve_pointer(target_doc, &pointer).is_none() {
            return Err(self.err(
                doc,
                kp,
                Some("$ref"),
                format!("`$ref` {:?} points at nothing", r),
            ));
        }
        Ok(self.node_at(target_doc, pointer))
    }

    /// A document by the name a `$ref` uses: relative to the referring
    /// file's directory, as written in `schemas {}`, or by base name.
    fn find_doc(&self, from: usize, file: &str) -> Option<usize> {
        let from_name = &self.docs[from].name;
        let dir = match from_name.rfind('/') {
            Some(i) => &from_name[..=i],
            None => "",
        };
        let joined = normalize(&format!("{}{}", dir, file));
        let by = |f: &dyn Fn(&Doc) -> bool| self.docs.iter().position(f);
        by(&|d| normalize(&d.name) == joined)
            .or_else(|| by(&|d| d.name == file))
            .or_else(|| by(&|d| basename(&d.name) == file))
    }

    /// Reject `$ref`/applicator cycles that do not pass through a property
    /// or item: they would make validation loop without consuming input.
    fn check_cycles(&self) -> Result<(), LowerError> {
        // 0 = unvisited, 1 = on stack, 2 = done
        let mut state = vec![0u8; self.nodes.len()];
        for start in 0..self.nodes.len() {
            if state[start] != 0 {
                continue;
            }
            let mut stack = vec![(start, 0usize)];
            state[start] = 1;
            while let Some(&mut (id, ref mut next)) = stack.last_mut() {
                let edges = self.applicators(id);
                if *next < edges.len() {
                    let to = edges[*next];
                    *next += 1;
                    match state[to] {
                        0 => {
                            state[to] = 1;
                            stack.push((to, 0));
                        }
                        1 => {
                            let n = &self.nodes[to];
                            return Err(self.err(
                                n.doc,
                                &n.pointer,
                                Some("$ref"),
                                "this schema refers to itself without consuming any input",
                            ));
                        }
                        _ => {}
                    }
                } else {
                    state[id] = 2;
                    stack.pop();
                }
            }
        }
        Ok(())
    }

    fn applicators(&self, id: usize) -> Vec<usize> {
        match &self.nodes[id].kind {
            Kind::Schema(s) => s
                .ref_
                .iter()
                .chain(&s.all_of)
                .chain(&s.any_of)
                .chain(&s.one_of)
                .chain(&s.not)
                .copied()
                .collect(),
            _ => vec![],
        }
    }
}

fn normalize(path: &str) -> String {
    let mut parts: Vec<&str> = vec![];
    for p in path.split('/') {
        match p {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            p => parts.push(p),
        }
    }
    parts.join("/")
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && b[i + 1].is_ascii_hexdigit()
            && b[i + 2].is_ascii_hexdigit()
        {
            let hex = |c: u8| (c as char).to_digit(16).unwrap_or(0) as u8;
            out.push(hex(b[i + 1]) * 16 + hex(b[i + 2]));
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn num(n: &serde_json::Number) -> Num {
    match n.as_i64() {
        Some(i) => Num::Int(i),
        None => Num::Float(n.as_f64().unwrap_or(f64::NAN)),
    }
}

/// A JSON value as a literal.
pub fn lit(v: &Value) -> Lit {
    match v {
        Value::Null => Lit::Null,
        Value::Bool(b) => Lit::Bool(*b),
        Value::Number(n) => Lit::Num(num(n)),
        Value::String(s) => Lit::Str(s.clone()),
        Value::Array(a) => Lit::Array(a.iter().map(lit).collect()),
        Value::Object(m) => Lit::Object(m.iter().map(|(k, v)| (k.clone(), lit(v))).collect()),
    }
}
