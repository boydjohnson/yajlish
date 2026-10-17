//! Planning the Rust types for a lowered schema set.
//!
//! Every type is derived from the same IR node the validator runs, so the
//! builder and the validator agree node for node. Where a schema does not
//! pin a value down to one Rust type, the type is `yajlish_schema::Value`.

use crate::lower::{Ir, Kind, Lit, Schema, Type};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, PartialEq)]
pub enum Ty {
    Value,
    Unit,
    Bool,
    I64,
    F64,
    String,
    Option(Box<Ty>),
    Vec(Box<Ty>),
    Map(Box<Ty>),
    Box(Box<Ty>),
    Named(usize),
}

#[derive(Debug)]
pub struct Field {
    /// The property name as it appears in JSON.
    pub json: String,
    pub ident: String,
    pub ty: Ty,
    pub required: bool,
}

#[derive(Debug)]
pub enum DefKind {
    Struct {
        fields: Vec<Field>,
        /// Type of the members not listed in `properties`, if any are
        /// allowed.
        additional: Option<Ty>,
    },
    Tuple(Vec<Ty>),
    StrEnum(Vec<(String, String)>),
    /// `oneOf` / `anyOf`: variant ident, node, payload.
    Union(Vec<(String, usize, Ty)>),
}

#[derive(Debug)]
pub struct Def {
    pub name: String,
    pub doc: Option<String>,
    pub kind: DefKind,
}

#[derive(Debug)]
pub struct Plan {
    pub defs: Vec<Def>,
    /// The type of each document root, in document order.
    pub roots: Vec<Ty>,
}

/// A naming collision: two schema locations want the same type name.
#[derive(Debug)]
pub struct NameError {
    pub name: String,
    pub first: String,
    pub second: String,
}

struct Planner<'a> {
    ir: &'a Ir,
    defs: Vec<Option<Def>>,
    def_nodes: Vec<usize>,
    names: HashMap<String, usize>,
    memo: HashMap<usize, Ty>,
    in_progress: HashSet<usize>,
    error: Option<NameError>,
}

/// Plan types for the given documents. `root_names[i]` names document `i`'s
/// root; `None` for documents (such as an envelope) that get no type.
pub fn plan(ir: &Ir, root_names: &[Option<String>]) -> Result<Plan, NameError> {
    let mut p = Planner {
        ir,
        defs: vec![],
        def_nodes: vec![],
        names: HashMap::new(),
        memo: HashMap::new(),
        in_progress: HashSet::new(),
        error: None,
    };
    let mut roots = vec![];
    for (doc, name) in root_names.iter().enumerate() {
        if let Some(name) = name {
            roots.push(p.ty(ir.doc_roots[doc], name));
        }
    }
    if let Some(e) = p.error {
        return Err(e);
    }
    let mut defs: Vec<Def> = p.defs.into_iter().map(|d| d.expect("def filled")).collect();
    box_recursive_fields(&mut defs);
    Ok(Plan { defs, roots })
}

impl<'a> Planner<'a> {
    fn schema(&self, node: usize) -> Option<&'a Schema> {
        let ir: &'a Ir = self.ir;
        match &ir.nodes[node].kind {
            Kind::Schema(s) => Some(s),
            _ => None,
        }
    }

    /// The name a node's type gets: `$defs` key, then `title`, then `hint`.
    fn name_for(&self, node: usize, hint: &str) -> String {
        let n = &self.ir.nodes[node];
        n.def_name
            .as_deref()
            .or(n.title.as_deref())
            .map(pascal)
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| hint.to_owned())
    }

    fn new_def(&mut self, node: usize, name: String) -> usize {
        let id = self.defs.len();
        if let Some(&other) = self.names.get(&name) {
            if self.error.is_none() {
                self.error = Some(NameError {
                    name: name.clone(),
                    first: self.ir.nodes[self.def_nodes[other]].path.clone(),
                    second: self.ir.nodes[node].path.clone(),
                });
            }
        }
        self.names.insert(name, id);
        self.defs.push(None);
        self.def_nodes.push(node);
        id
    }

    fn ty(&mut self, node: usize, hint: &str) -> Ty {
        if let Some(t) = self.memo.get(&node) {
            return t.clone();
        }
        if !self.in_progress.insert(node) {
            // A cycle that does not pass through a named type.
            return Ty::Value;
        }
        let t = self.compute(node, hint);
        self.in_progress.remove(&node);
        self.memo.insert(node, t.clone());
        t
    }

    fn doc(&self, node: usize) -> Option<String> {
        self.ir.nodes[node].description.clone()
    }

    #[allow(clippy::too_many_lines)]
    fn compute(&mut self, node: usize, hint: &str) -> Ty {
        let Some(s) = self.schema(node) else {
            return Ty::Value;
        };
        let has_local = s.types.is_some()
            || s.enum_.is_some()
            || s.const_.is_some()
            || s.has_object_keywords()
            || !s.prefix_items.is_empty()
            || s.items.is_some();

        // A plain `$ref`: the target's type.
        if let Some(target) = s.ref_ {
            if !has_local && s.all_of.is_empty() && s.any_of.is_empty() && s.one_of.is_empty() {
                let name = self.name_for(target, hint);
                return self.ty(target, &name);
            }
        }

        let alternatives: Vec<usize> = if !s.one_of.is_empty() {
            s.one_of.clone()
        } else {
            s.any_of.clone()
        };
        if !alternatives.is_empty() && s.types.is_none() {
            let name = self.name_for(node, hint);
            let id = self.new_def(node, name.clone());
            self.memo.insert(node, Ty::Named(id));
            let mut variants = vec![];
            let mut used = HashSet::new();
            for (i, &alt) in alternatives.iter().enumerate() {
                let base = {
                    let n = &self.ir.nodes[alt];
                    let via_ref = self
                        .schema(alt)
                        .and_then(|s| s.ref_)
                        .map(|r| &self.ir.nodes[r]);
                    n.title
                        .as_deref()
                        .or(n.def_name.as_deref())
                        .or(via_ref.and_then(|r| r.def_name.as_deref().or(r.title.as_deref())))
                        .map(pascal)
                        .filter(|s| !s.is_empty())
                        .unwrap_or_else(|| format!("Variant{}", i))
                };
                let ident = dedup(&mut used, base);
                let payload = self.ty(alt, &format!("{}{}", name, ident));
                variants.push((ident, alt, payload));
            }
            self.defs[id] = Some(Def {
                name,
                doc: self.doc(node),
                kind: DefKind::Union(variants),
            });
            return Ty::Named(id);
        }

        if !s.all_of.is_empty() {
            let mut fragments = vec![];
            if s.has_object_keywords() || s.types.is_some() {
                fragments.push(node);
            }
            for &b in &s.all_of {
                fragments.push(self.follow_refs(b));
            }
            if fragments.iter().all(|&f| self.is_object_fragment(f)) {
                return self.merged_struct(node, hint, &fragments);
            }
            let first = s.all_of[0];
            return self.ty(first, hint);
        }

        if let Some(options) = &s.enum_ {
            let nullable = options.contains(&Lit::Null);
            let strings: Vec<&str> = options
                .iter()
                .filter_map(|l| match l {
                    Lit::Str(v) => Some(v.as_str()),
                    _ => None,
                })
                .collect();
            if !strings.is_empty() && strings.len() + usize::from(nullable) == options.len() {
                let name = self.name_for(node, hint);
                let id = self.new_def(node, name.clone());
                let mut used = HashSet::new();
                let variants = strings
                    .iter()
                    .map(|v| {
                        let base = pascal(v);
                        let base = if base.is_empty() {
                            "Empty".to_owned()
                        } else {
                            base
                        };
                        (dedup(&mut used, base), (*v).to_owned())
                    })
                    .collect();
                self.defs[id] = Some(Def {
                    name,
                    doc: self.doc(node),
                    kind: DefKind::StrEnum(variants),
                });
                return if nullable {
                    Ty::Option(Box::new(Ty::Named(id)))
                } else {
                    Ty::Named(id)
                };
            }
            return Ty::Value;
        }

        if let Some(c) = &s.const_ {
            return match c {
                Lit::Null => Ty::Unit,
                Lit::Bool(_) => Ty::Bool,
                Lit::Num(crate::lower::Num::Int(_)) => Ty::I64,
                Lit::Num(crate::lower::Num::Float(_)) => Ty::F64,
                Lit::Str(_) => Ty::String,
                _ => Ty::Value,
            };
        }

        let Some(types) = &s.types else {
            return Ty::Value;
        };
        let nullable = types.contains(&Type::Null);
        let rest: Vec<Type> = types.iter().copied().filter(|t| *t != Type::Null).collect();
        let base = match rest.as_slice() {
            [] => Ty::Unit,
            [Type::Integer, Type::Number] | [Type::Number, Type::Integer] => Ty::F64,
            [t] => match t {
                Type::Boolean => Ty::Bool,
                Type::Integer => Ty::I64,
                Type::Number => Ty::F64,
                Type::String => Ty::String,
                Type::Null => Ty::Unit,
                Type::Array => self.array_ty(node, hint),
                Type::Object => self.object_ty(node, hint),
            },
            _ => Ty::Value,
        };
        if nullable && !rest.is_empty() {
            Ty::Option(Box::new(base))
        } else {
            base
        }
    }

    fn follow_refs(&self, mut node: usize) -> usize {
        let mut seen = HashSet::new();
        while let Some(s) = self.schema(node) {
            match s.ref_ {
                Some(r)
                    if s.types.is_none()
                        && !s.has_object_keywords()
                        && s.all_of.is_empty()
                        && s.any_of.is_empty()
                        && s.one_of.is_empty()
                        && seen.insert(node) =>
                {
                    node = r;
                }
                _ => break,
            }
        }
        node
    }

    /// A schema that only describes an object: `type: object` (or no
    /// `type`) with object keywords, and nothing else that affects typing.
    fn is_object_fragment(&self, node: usize) -> bool {
        match self.schema(node) {
            Some(s) => {
                let object_type = match &s.types {
                    None => s.has_object_keywords(),
                    Some(t) => t == &[Type::Object],
                };
                object_type
                    && s.ref_.is_none()
                    && s.enum_.is_none()
                    && s.const_.is_none()
                    && s.any_of.is_empty()
                    && s.one_of.is_empty()
                    && s.all_of
                        .iter()
                        .all(|&b| self.is_object_fragment(self.follow_refs(b)))
            }
            None => false,
        }
    }

    fn merged_struct(&mut self, node: usize, hint: &str, fragments: &[usize]) -> Ty {
        let name = self.name_for(node, hint);
        let id = self.new_def(node, name.clone());
        self.memo.insert(node, Ty::Named(id));
        // Flatten nested allOf fragments.
        let mut all = vec![];
        let mut stack: Vec<usize> = fragments.iter().rev().copied().collect();
        while let Some(f) = stack.pop() {
            if all.contains(&f) {
                continue;
            }
            all.push(f);
            if let Some(s) = self.schema(f) {
                for &b in s.all_of.iter().rev() {
                    stack.push(self.follow_refs(b));
                }
            }
        }
        let mut props: Vec<(String, usize)> = vec![];
        let mut required: HashSet<String> = HashSet::new();
        let mut closed = false;
        for &f in &all {
            if let Some(s) = self.schema(f) {
                for (k, n) in &s.properties {
                    if !props.iter().any(|(pk, _)| pk == k) {
                        props.push((k.clone(), *n));
                    }
                }
                required.extend(s.required.iter().cloned());
                if s.additional_properties
                    .is_some_and(|a| matches!(self.ir.nodes[a].kind, Kind::False))
                {
                    closed = true;
                }
            }
        }
        let fields = self.fields(&name, &props, &required);
        self.defs[id] = Some(Def {
            name,
            doc: self.doc(node),
            kind: DefKind::Struct {
                fields,
                additional: if closed { None } else { Some(Ty::Value) },
            },
        });
        Ty::Named(id)
    }

    fn fields(
        &mut self,
        parent: &str,
        props: &[(String, usize)],
        required: &HashSet<String>,
    ) -> Vec<Field> {
        let mut used = HashSet::new();
        used.insert("additional_properties".to_owned());
        props
            .iter()
            .map(|(json, n)| {
                let ty = self.ty(*n, &format!("{}{}", parent, pascal(json)));
                Field {
                    json: json.clone(),
                    ident: dedup(&mut used, snake(json)),
                    ty,
                    required: required.contains(json),
                }
            })
            .collect()
    }

    fn object_ty(&mut self, node: usize, hint: &str) -> Ty {
        let s = self.schema(node).expect("schema node");
        let props = s.properties.clone();
        let required: HashSet<String> = s.required.iter().cloned().collect();
        let additional = s.additional_properties;
        if props.is_empty() {
            return match additional {
                Some(a) => {
                    let t = self.ty(a, &format!("{}Value", hint));
                    Ty::Map(Box::new(t))
                }
                None => Ty::Map(Box::new(Ty::Value)),
            };
        }
        let name = self.name_for(node, hint);
        let id = self.new_def(node, name.clone());
        self.memo.insert(node, Ty::Named(id));
        let fields = self.fields(&name, &props, &required);
        let additional = match additional {
            None => Some(Ty::Value),
            Some(a) if matches!(self.ir.nodes[a].kind, Kind::False) => None,
            Some(a) => Some(self.ty(a, &format!("{}Additional", name))),
        };
        self.defs[id] = Some(Def {
            name,
            doc: self.doc(node),
            kind: DefKind::Struct { fields, additional },
        });
        Ty::Named(id)
    }

    fn array_ty(&mut self, node: usize, hint: &str) -> Ty {
        let s = self.schema(node).expect("schema node");
        let prefix = s.prefix_items.clone();
        let items = s.items;
        let min = s.min_items.unwrap_or(0);
        if !prefix.is_empty() {
            let closed = items.is_some_and(|i| matches!(self.ir.nodes[i].kind, Kind::False));
            if closed && min >= prefix.len() as u64 {
                let name = self.name_for(node, hint);
                let id = self.new_def(node, name.clone());
                self.memo.insert(node, Ty::Named(id));
                let elems = prefix
                    .iter()
                    .enumerate()
                    .map(|(i, &p)| self.ty(p, &format!("{}{}", name, i)))
                    .collect();
                self.defs[id] = Some(Def {
                    name,
                    doc: self.doc(node),
                    kind: DefKind::Tuple(elems),
                });
                return Ty::Named(id);
            }
            return Ty::Vec(Box::new(Ty::Value));
        }
        match items {
            Some(i) => {
                let t = self.ty(i, &format!("{}Item", hint));
                Ty::Vec(Box::new(t))
            }
            None => Ty::Vec(Box::new(Ty::Value)),
        }
    }
}

/// Box every field or payload whose named type can reach back to the type
/// holding it without going through a `Vec` or map, so recursive types
/// have a finite size.
fn box_recursive_fields(defs: &mut [Def]) {
    fn direct(t: &Ty, out: &mut Vec<usize>) {
        match t {
            Ty::Named(i) => out.push(*i),
            Ty::Option(inner) => direct(inner, out),
            _ => {}
        }
    }
    let edges: Vec<Vec<usize>> = defs
        .iter()
        .map(|d| {
            let mut out = vec![];
            match &d.kind {
                DefKind::Struct { fields, .. } => {
                    fields.iter().for_each(|f| direct(&f.ty, &mut out))
                }
                DefKind::Tuple(elems) => elems.iter().for_each(|t| direct(t, &mut out)),
                DefKind::Union(vs) => vs.iter().for_each(|(_, _, t)| direct(t, &mut out)),
                DefKind::StrEnum(_) => {}
            }
            out
        })
        .collect();
    let reaches = |from: usize, to: usize| -> bool {
        let mut seen = vec![false; edges.len()];
        let mut stack = vec![from];
        while let Some(n) = stack.pop() {
            if n == to {
                return true;
            }
            if !std::mem::replace(&mut seen[n], true) {
                stack.extend(&edges[n]);
            }
        }
        false
    };
    let wrap = |t: &mut Ty, owner: usize| {
        let mut ids = vec![];
        direct(t, &mut ids);
        if ids.iter().any(|&i| reaches(i, owner)) {
            box_named(t);
        }
    };
    for (owner, def) in defs.iter_mut().enumerate() {
        match &mut def.kind {
            DefKind::Struct { fields, .. } => {
                fields.iter_mut().for_each(|f| wrap(&mut f.ty, owner))
            }
            DefKind::Tuple(elems) => elems.iter_mut().for_each(|t| wrap(t, owner)),
            DefKind::Union(vs) => vs.iter_mut().for_each(|(_, _, t)| wrap(t, owner)),
            DefKind::StrEnum(_) => {}
        }
    }
}

fn box_named(t: &mut Ty) {
    match t {
        Ty::Named(_) => *t = Ty::Box(Box::new(t.clone())),
        Ty::Option(inner) => box_named(inner),
        _ => {}
    }
}

fn words(s: &str) -> Vec<String> {
    let mut words = vec![];
    let mut cur = String::new();
    let mut prev_lower = false;
    for c in s.chars() {
        if c.is_alphanumeric() {
            if c.is_uppercase() && prev_lower && !cur.is_empty() {
                words.push(std::mem::take(&mut cur));
            }
            prev_lower = c.is_lowercase() || c.is_ascii_digit();
            cur.push(c);
        } else {
            if !cur.is_empty() {
                words.push(std::mem::take(&mut cur));
            }
            prev_lower = false;
        }
    }
    if !cur.is_empty() {
        words.push(cur);
    }
    words
        .into_iter()
        .map(|w| {
            w.chars()
                .filter(char::is_ascii_alphanumeric)
                .collect::<String>()
        })
        .filter(|w| !w.is_empty())
        .collect()
}

/// `PascalCase`, ASCII only, never starting with a digit (empty if `s` has
/// no ASCII letters or digits).
pub fn pascal(s: &str) -> String {
    let mut out: String = words(s)
        .iter()
        .map(|w| {
            let mut cs = w.chars();
            match cs.next() {
                Some(f) => f.to_ascii_uppercase().to_string() + &cs.as_str().to_ascii_lowercase(),
                None => String::new(),
            }
        })
        .collect();
    if out.starts_with(|c: char| c.is_ascii_digit()) {
        out.insert(0, 'V');
    }
    out
}

/// `snake_case`, ASCII only, a valid identifier (raw if a keyword).
pub fn snake(s: &str) -> String {
    let mut out = words(s)
        .iter()
        .map(|w| w.to_ascii_lowercase())
        .collect::<Vec<_>>()
        .join("_");
    if out.is_empty() {
        out = "field".to_owned();
    }
    if out.starts_with(|c: char| c.is_ascii_digit()) {
        out.insert(0, '_');
    }
    out
}

fn dedup(used: &mut HashSet<String>, base: String) -> String {
    let mut name = base.clone();
    let mut n = 2;
    while !used.insert(name.clone()) {
        name = format!(
            "{}{}{}",
            base,
            if base.ends_with(|c: char| c.is_ascii_digit()) {
                "_"
            } else {
                ""
            },
            n
        );
        n += 1;
    }
    name
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_case_conversion() {
        assert_eq!(pascal("geometry"), "Geometry");
        assert_eq!(pascal("bbox_values"), "BboxValues");
        assert_eq!(pascal("camelCase"), "CamelCase");
        assert_eq!(pascal("Point"), "Point");
        assert_eq!(pascal("2d"), "V2d");
        assert_eq!(pascal("caf\u{e9}"), "Caf");
        assert_eq!(snake("camelCase"), "camel_case");
        assert_eq!(snake("type"), "type");
        assert_eq!(snake("@id"), "id");
        assert_eq!(snake("0"), "_0");
        assert_eq!(snake(""), "field");
    }
}
