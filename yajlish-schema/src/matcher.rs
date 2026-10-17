//! Streaming validation of one JSON value against one schema node.
//!
//! A [`Matcher`] is created right before the first event of a value and is
//! fed every event of that value, in order. It answers `Continue` until the
//! value ends, then `Pass`; or `Fail` as soon as the value is known to be
//! invalid, which may be long before it ends. A failed matcher is not fed
//! again, which is what lets the runtime drop a candidate schema the moment
//! a discriminating property rules it out.
//!
//! Every keyword of a schema object applies to the same value, so a matcher
//! for a node holds its local state (type checks, the object or array being
//! walked) plus one sub-matcher per applicator (`$ref`, `allOf`, `anyOf`,
//! `oneOf`, `not`), all fed the same events. Memory is proportional to the
//! nesting depth times the number of applicators alive at each level.

use crate::{
    __private::Config,
    error::{Failure, Seg},
    ir::{types, Literal, Node, NodeId, NodeKind, Num, Schema},
    num,
    value::{Value, ValueBuilder},
};

/// A parse event with strings and keys decoded.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Ev<'a> {
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

impl Ev<'_> {
    fn type_name(&self) -> &'static str {
        match self {
            Ev::Null => "null",
            Ev::Bool(_) => "boolean",
            Ev::Int(_) => "integer",
            Ev::Float(_) => "number",
            Ev::Str(_) => "string",
            Ev::StartMap | Ev::Key(_) | Ev::EndMap => "object",
            Ev::StartArray | Ev::EndArray => "array",
        }
    }
}

pub(crate) enum Step {
    Continue,
    Pass,
    Fail(Box<Failure>),
}

/// Shared, read-only state for all matchers of one handler.
pub(crate) struct Cx<'c> {
    pub(crate) cfg: &'c Config,
    pub(crate) offset: u64,
}

impl Cx<'_> {
    fn node(&self, id: NodeId) -> &'static Node {
        &self.cfg.nodes[id]
    }
}

/// Validates one value against one node.
pub(crate) struct Matcher {
    node: NodeId,
    depth: u32,
    local: Local,
    /// `$ref` and `allOf`: all must pass.
    all: Vec<Matcher>,
    any: Option<Group>,
    one: Option<Group>,
    not: Option<Group>,
    /// Captured value for `enum` / `const` on arrays and objects.
    capture: Option<ValueBuilder>,
}

/// The alternatives of `anyOf`, `oneOf` or `not`.
struct Group {
    live: Vec<Matcher>,
    passed: usize,
    failures: Vec<Failure>,
}

impl Group {
    fn new(cx: &Cx<'_>, ids: &[NodeId], depth: u32) -> Self {
        Group {
            live: ids
                .iter()
                .map(|&id| Matcher::new(cx, id, depth + 1))
                .collect(),
            passed: 0,
            failures: vec![],
        }
    }

    fn feed(&mut self, cx: &Cx<'_>, ev: &Ev<'_>) {
        let mut i = 0;
        while i < self.live.len() {
            match self.live[i].feed(cx, ev) {
                Step::Continue => i += 1,
                Step::Pass => {
                    self.passed += 1;
                    self.live.swap_remove(i);
                }
                Step::Fail(f) => {
                    self.failures.push(*f);
                    self.live.swap_remove(i);
                }
            }
        }
    }
}

enum Local {
    Start,
    Object(ObjState),
    Array(ArrState),
}

struct ObjState {
    seen_required: Vec<bool>,
    key: String,
    child: Option<Child>,
}

struct ArrState {
    index: u64,
    child: Option<Child>,
}

/// The matcher for a member or element, or a depth counter when no keyword
/// constrains it.
enum Child {
    Matcher(Box<Matcher>),
    Skip(u32),
}

impl Child {
    fn new(cx: &Cx<'_>, node: Option<NodeId>, depth: u32) -> Self {
        match node {
            Some(id) if !matches!(cx.node(id).kind, NodeKind::True) => {
                Child::Matcher(Box::new(Matcher::new(cx, id, depth)))
            }
            _ => Child::Skip(0),
        }
    }

    fn feed(&mut self, cx: &Cx<'_>, ev: &Ev<'_>) -> Step {
        match self {
            Child::Matcher(m) => m.feed(cx, ev),
            Child::Skip(depth) => {
                match ev {
                    Ev::StartMap | Ev::StartArray => *depth += 1,
                    Ev::EndMap | Ev::EndArray => *depth -= 1,
                    _ => {}
                }
                if *depth == 0 && !matches!(ev, Ev::Key(_)) {
                    Step::Pass
                } else {
                    Step::Continue
                }
            }
        }
    }
}

impl Matcher {
    pub(crate) fn new(cx: &Cx<'_>, node: NodeId, depth: u32) -> Self {
        let mut m = Matcher {
            node,
            depth,
            local: Local::Start,
            all: vec![],
            any: None,
            one: None,
            not: None,
            capture: None,
        };
        // Past the depth limit the matcher fails on its first event, so
        // adversarial nesting against a recursive schema cannot exhaust the
        // stack.
        if depth > cx.cfg.max_depth {
            return m;
        }
        if let NodeKind::Schema(s) = &cx.node(node).kind {
            if let Some(r) = s.ref_ {
                m.all.push(Matcher::new(cx, r, depth + 1));
            }
            for &id in s.all_of {
                m.all.push(Matcher::new(cx, id, depth + 1));
            }
            if !s.any_of.is_empty() {
                m.any = Some(Group::new(cx, s.any_of, depth));
            }
            if !s.one_of.is_empty() {
                m.one = Some(Group::new(cx, s.one_of, depth));
            }
            if let Some(n) = s.not {
                m.not = Some(Group::new(cx, &[n], depth));
            }
        }
        m
    }

    fn path(&self, cx: &Cx<'_>) -> &'static str {
        cx.node(self.node).path
    }

    fn fail(&self, cx: &Cx<'_>, keyword: &'static str, detail: impl Into<String>) -> Step {
        Step::Fail(Failure::new(self.path(cx), keyword, cx.offset, detail))
    }

    pub(crate) fn feed(&mut self, cx: &Cx<'_>, ev: &Ev<'_>) -> Step {
        if self.depth > cx.cfg.max_depth {
            return self.fail(
                cx,
                "",
                format!("nesting deeper than {} levels", cx.cfg.max_depth),
            );
        }

        let mut i = 0;
        while i < self.all.len() {
            match self.all[i].feed(cx, ev) {
                Step::Continue => i += 1,
                Step::Pass => {
                    self.all.swap_remove(i);
                }
                Step::Fail(f) => return Step::Fail(f),
            }
        }
        if let Some(g) = &mut self.any {
            g.feed(cx, ev);
            if g.live.is_empty() && g.passed == 0 {
                let causes = std::mem::take(&mut g.failures);
                return self.fail_group(cx, "anyOf", "no alternative matched", causes);
            }
        }
        if let Some(g) = &mut self.one {
            g.feed(cx, ev);
            if g.live.is_empty() && g.passed == 0 {
                let causes = std::mem::take(&mut g.failures);
                return self.fail_group(cx, "oneOf", "no alternative matched", causes);
            }
        }
        if let Some(g) = &mut self.not {
            g.feed(cx, ev);
        }
        if let Some(c) = &mut self.capture {
            c.event(ev);
        }

        match self.feed_local(cx, ev) {
            Step::Continue => Step::Continue,
            Step::Fail(f) => Step::Fail(f),
            // The value has ended, so every applicator has finished too.
            Step::Pass => self.finish(cx),
        }
    }

    fn fail_group(
        &self,
        cx: &Cx<'_>,
        keyword: &'static str,
        detail: &str,
        causes: Vec<Failure>,
    ) -> Step {
        let mut f = Failure::new(self.path(cx), keyword, cx.offset, detail);
        f.causes = causes.into_iter().map(|c| c.into_error()).collect();
        Step::Fail(f)
    }

    fn finish(&mut self, cx: &Cx<'_>) -> Step {
        if let Some(g) = &self.one {
            if g.passed != 1 {
                return self.fail(
                    cx,
                    "oneOf",
                    format!("{} alternatives matched, expected exactly one", g.passed),
                );
            }
        }
        if let Some(g) = &self.not {
            if g.passed > 0 {
                return self.fail(cx, "not", "the value matches the schema in `not`");
            }
        }
        if let (Some(mut c), NodeKind::Schema(s)) = (self.capture.take(), &cx.node(self.node).kind)
        {
            if let Some(v) = c.take() {
                return check_literals(self, cx, s, |lit| literal_eq_value(lit, &v));
            }
        }
        Step::Pass
    }

    fn feed_local(&mut self, cx: &Cx<'_>, ev: &Ev<'_>) -> Step {
        let node = self.node;
        let s = match &cx.node(self.node).kind {
            NodeKind::True => return self.skip_value(ev),
            NodeKind::False => {
                return fail_node(cx, node, "", "the schema is `false`, nothing is valid");
            }
            NodeKind::Schema(s) => s,
        };
        match &mut self.local {
            Local::Start => self.start(cx, s, ev),
            Local::Object(st) => {
                if let Some(child) = &mut st.child {
                    return match child.feed(cx, ev) {
                        Step::Continue => Step::Continue,
                        Step::Pass => {
                            st.child = None;
                            Step::Continue
                        }
                        Step::Fail(f) => Step::Fail(f.at(Seg::Key(std::mem::take(&mut st.key)))),
                    };
                }
                match ev {
                    Ev::Key(k) => {
                        if let Some(i) = s.required.iter().position(|r| r == k) {
                            st.seen_required[i] = true;
                        }
                        let child_node = match s.properties.iter().find(|(name, _)| name == k) {
                            Some(&(_, id)) => Some(id),
                            None => match s.additional_properties {
                                Some(id) if matches!(cx.node(id).kind, NodeKind::False) => {
                                    let f = Failure::new(
                                        cx.node(node).path,
                                        "additionalProperties",
                                        cx.offset,
                                        format!("property {:?} is not allowed", k),
                                    );
                                    return Step::Fail(f.at(Seg::Key((*k).to_owned())));
                                }
                                other => other,
                            },
                        };
                        st.key.clear();
                        st.key.push_str(k);
                        st.child = Some(Child::new(cx, child_node, self.depth + 1));
                        Step::Continue
                    }
                    Ev::EndMap => {
                        let missing: Vec<&str> = s
                            .required
                            .iter()
                            .zip(&st.seen_required)
                            .filter(|(_, seen)| !**seen)
                            .map(|(name, _)| *name)
                            .collect();
                        if missing.is_empty() {
                            Step::Pass
                        } else {
                            fail_node(
                                cx,
                                node,
                                "required",
                                format!("missing properties {:?}", missing),
                            )
                        }
                    }
                    _ => fail_node(cx, node, "", "unexpected event inside an object"),
                }
            }
            Local::Array(st) => {
                if let Some(child) = &mut st.child {
                    return match child.feed(cx, ev) {
                        Step::Continue => Step::Continue,
                        Step::Pass => {
                            st.child = None;
                            st.index += 1;
                            Step::Continue
                        }
                        Step::Fail(f) => Step::Fail(f.at(Seg::Index(st.index))),
                    };
                }
                if let Ev::EndArray = ev {
                    if let Some(min) = s.min_items {
                        if st.index < min {
                            return fail_node(
                                cx,
                                node,
                                "minItems",
                                format!("{} items, expected at least {}", st.index, min),
                            );
                        }
                    }
                    return Step::Pass;
                }
                let index = st.index;
                if let Some(max) = s.max_items {
                    if index >= max {
                        return fail_node(cx, node, "maxItems", format!("more than {} items", max));
                    }
                }
                let child_node = match usize::try_from(index)
                    .ok()
                    .and_then(|i| s.prefix_items.get(i))
                {
                    Some(&id) => Some(id),
                    None => match s.items {
                        Some(id) if matches!(cx.node(id).kind, NodeKind::False) => {
                            return fail_node(
                                cx,
                                node,
                                "items",
                                format!("only {} items are allowed", s.prefix_items.len()),
                            );
                        }
                        other => other,
                    },
                };
                let mut child = Child::new(cx, child_node, self.depth + 1);
                match child.feed(cx, ev) {
                    Step::Continue => {
                        if let Local::Array(st) = &mut self.local {
                            st.child = Some(child);
                        }
                        Step::Continue
                    }
                    Step::Pass => {
                        if let Local::Array(st) = &mut self.local {
                            st.index += 1;
                        }
                        Step::Continue
                    }
                    Step::Fail(f) => Step::Fail(f.at(Seg::Index(index))),
                }
            }
        }
    }

    /// For a `true` node: pass once the value ends.
    fn skip_value(&mut self, ev: &Ev<'_>) -> Step {
        // Reuse the array state's index as a depth counter.
        let depth = match &mut self.local {
            Local::Start => {
                self.local = Local::Array(ArrState {
                    index: 0,
                    child: None,
                });
                match &mut self.local {
                    Local::Array(st) => &mut st.index,
                    _ => unreachable!(),
                }
            }
            Local::Array(st) => &mut st.index,
            Local::Object(_) => unreachable!(),
        };
        match ev {
            Ev::StartMap | Ev::StartArray => *depth += 1,
            Ev::EndMap | Ev::EndArray => *depth -= 1,
            _ => {}
        }
        if *depth == 0 && !matches!(ev, Ev::Key(_)) {
            Step::Pass
        } else {
            Step::Continue
        }
    }

    /// The first event of the value.
    fn start(&mut self, cx: &Cx<'_>, s: &'static Schema, ev: &Ev<'_>) -> Step {
        if !type_allowed(s.types, ev) {
            return self.fail(
                cx,
                "type",
                format!("expected {}, found {}", type_names(s.types), ev.type_name()),
            );
        }
        match ev {
            Ev::StartMap => {
                self.local = Local::Object(ObjState {
                    seen_required: vec![false; s.required.len()],
                    key: String::new(),
                    child: None,
                });
                if s.enum_.is_some() || s.const_.is_some() {
                    let mut c = ValueBuilder::default();
                    c.event(ev);
                    self.capture = Some(c);
                }
                Step::Continue
            }
            Ev::StartArray => {
                self.local = Local::Array(ArrState {
                    index: 0,
                    child: None,
                });
                if s.enum_.is_some() || s.const_.is_some() {
                    let mut c = ValueBuilder::default();
                    c.event(ev);
                    self.capture = Some(c);
                }
                Step::Continue
            }
            Ev::Str(text) => {
                if s.min_length.is_some() || s.max_length.is_some() {
                    let len = text.chars().count() as u64;
                    if let Some(min) = s.min_length {
                        if len < min {
                            return self.fail(
                                cx,
                                "minLength",
                                format!("length {}, expected at least {}", len, min),
                            );
                        }
                    }
                    if let Some(max) = s.max_length {
                        if len > max {
                            return self.fail(
                                cx,
                                "maxLength",
                                format!("length {}, expected at most {}", len, max),
                            );
                        }
                    }
                }
                if let Some(p) = s.pattern {
                    if !cx.cfg.pattern_matches(p, text) {
                        return self.fail(
                            cx,
                            "pattern",
                            format!("does not match /{}/", cx.cfg.patterns[p]),
                        );
                    }
                }
                self.scalar_done(cx, s, ev)
            }
            Ev::Int(_) | Ev::Float(_) => {
                let n = match ev {
                    Ev::Int(i) => Num::Int(*i),
                    Ev::Float(x) => Num::Float(*x),
                    _ => unreachable!(),
                };
                if let Some(step) = self.check_number(cx, s, n) {
                    return step;
                }
                self.scalar_done(cx, s, ev)
            }
            Ev::Null | Ev::Bool(_) => self.scalar_done(cx, s, ev),
            Ev::Key(_) | Ev::EndMap | Ev::EndArray => {
                self.fail(cx, "", "unexpected event at the start of a value")
            }
        }
    }

    /// A scalar value ends on its only event; `feed` then finishes the
    /// applicators, which saw it first.
    fn scalar_done(&mut self, cx: &Cx<'_>, s: &'static Schema, ev: &Ev<'_>) -> Step {
        check_literals(self, cx, s, |lit| literal_eq_event(lit, ev))
    }

    fn check_number(&self, cx: &Cx<'_>, s: &Schema, n: Num) -> Option<Step> {
        use std::cmp::Ordering::{Greater, Less};
        let cmp = |bound: Num| num::cmp(n, bound);
        if let Some(b) = s.minimum {
            if cmp(b) == Some(Less) || cmp(b).is_none() {
                return Some(self.fail(
                    cx,
                    "minimum",
                    format!("{} < {}", num::show(n), num::show(b)),
                ));
            }
        }
        if let Some(b) = s.maximum {
            if cmp(b) == Some(Greater) || cmp(b).is_none() {
                return Some(self.fail(
                    cx,
                    "maximum",
                    format!("{} > {}", num::show(n), num::show(b)),
                ));
            }
        }
        if let Some(b) = s.exclusive_minimum {
            if cmp(b) != Some(Greater) {
                return Some(self.fail(
                    cx,
                    "exclusiveMinimum",
                    format!("{} <= {}", num::show(n), num::show(b)),
                ));
            }
        }
        if let Some(b) = s.exclusive_maximum {
            if cmp(b) != Some(Less) {
                return Some(self.fail(
                    cx,
                    "exclusiveMaximum",
                    format!("{} >= {}", num::show(n), num::show(b)),
                ));
            }
        }
        if let Some(m) = s.multiple_of {
            if !num::is_multiple_of(n, m) {
                return Some(self.fail(
                    cx,
                    "multipleOf",
                    format!("{} is not a multiple of {}", num::show(n), num::show(m)),
                ));
            }
        }
        None
    }
}

fn fail_node(cx: &Cx<'_>, node: NodeId, keyword: &'static str, detail: impl Into<String>) -> Step {
    Step::Fail(Failure::new(cx.node(node).path, keyword, cx.offset, detail))
}

fn check_literals(m: &Matcher, cx: &Cx<'_>, s: &Schema, eq: impl Fn(&Literal) -> bool) -> Step {
    if let Some(c) = s.const_ {
        if !eq(c) {
            return m.fail(cx, "const", "value differs from the `const`");
        }
    }
    if let Some(options) = s.enum_ {
        if !options.iter().any(eq) {
            return m.fail(cx, "enum", "value is not one of the `enum` values");
        }
    }
    Step::Pass
}

fn type_allowed(allowed: u8, ev: &Ev<'_>) -> bool {
    let bit = match ev {
        Ev::Null => types::NULL,
        Ev::Bool(_) => types::BOOLEAN,
        Ev::Int(_) => types::INTEGER | types::NUMBER,
        Ev::Float(x) if x.is_finite() && x.fract() == 0.0 => types::INTEGER | types::NUMBER,
        Ev::Float(_) => types::NUMBER,
        Ev::Str(_) => types::STRING,
        Ev::StartArray => types::ARRAY,
        Ev::StartMap => types::OBJECT,
        Ev::Key(_) | Ev::EndMap | Ev::EndArray => return false,
    };
    allowed & bit != 0
}

fn type_names(allowed: u8) -> String {
    let names = [
        (types::NULL, "null"),
        (types::BOOLEAN, "boolean"),
        (types::INTEGER, "integer"),
        (types::NUMBER, "number"),
        (types::STRING, "string"),
        (types::ARRAY, "array"),
        (types::OBJECT, "object"),
    ];
    let list: Vec<&str> = names
        .iter()
        .filter(|(bit, _)| allowed & bit != 0)
        .map(|(_, n)| *n)
        .collect();
    list.join(" or ")
}

fn literal_eq_event(lit: &Literal, ev: &Ev<'_>) -> bool {
    match (lit, ev) {
        (Literal::Null, Ev::Null) => true,
        (Literal::Bool(a), Ev::Bool(b)) => a == b,
        (Literal::Num(a), Ev::Int(i)) => {
            num::cmp(*a, Num::Int(*i)) == Some(std::cmp::Ordering::Equal)
        }
        (Literal::Num(a), Ev::Float(x)) => {
            num::cmp(*a, Num::Float(*x)) == Some(std::cmp::Ordering::Equal)
        }
        (Literal::Str(a), Ev::Str(b)) => a == b,
        _ => false,
    }
}

/// JSON equality: numbers by value, objects regardless of member order.
pub(crate) fn literal_eq_value(lit: &Literal, v: &Value) -> bool {
    match (lit, v) {
        (Literal::Null, Value::Null) => true,
        (Literal::Bool(a), Value::Bool(b)) => a == b,
        (Literal::Num(a), Value::Int(i)) => {
            num::cmp(*a, Num::Int(*i)) == Some(std::cmp::Ordering::Equal)
        }
        (Literal::Num(a), Value::Float(x)) => {
            num::cmp(*a, Num::Float(*x)) == Some(std::cmp::Ordering::Equal)
        }
        (Literal::Str(a), Value::String(b)) => a == b,
        (Literal::Array(a), Value::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(x, y)| literal_eq_value(x, y))
        }
        (Literal::Object(a), Value::Object(b)) => {
            // Duplicate keys in the instance: the last one counts.
            let mut keys: Vec<&str> = b.iter().map(|(k, _)| k.as_str()).collect();
            keys.sort_unstable();
            keys.dedup();
            keys.len() == a.len()
                && a.iter().all(|(k, x)| {
                    b.iter()
                        .rev()
                        .find(|(bk, _)| bk == k)
                        .is_some_and(|(_, y)| literal_eq_value(x, y))
                })
        }
        _ => false,
    }
}

/// Whether `v` is valid against `node`, by replaying it as events.
pub(crate) fn validate_value(cfg: &Config, node: NodeId, v: &Value) -> Result<(), Box<Failure>> {
    let cx = Cx { cfg, offset: 0 };
    let mut m = Matcher::new(&cx, node, 0);
    let mut result = None;
    v.replay(&mut |ev| {
        if result.is_none() {
            match m.feed(&cx, &ev) {
                Step::Continue => {}
                Step::Pass => result = Some(Ok(())),
                Step::Fail(f) => result = Some(Err(f)),
            }
        }
    });
    result.unwrap_or(Ok(()))
}
