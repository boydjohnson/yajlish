use crate::{Context, Handler, Parser, Status};
use proptest::prelude::*;
use serde::Serialize;
use std::collections::HashMap;

proptest! {

    #[test]
    fn test_json_parse(json in arb_json()) {

        let mut out = vec![];
        let mut mock_handler = MockHandler::new(&mut out);
        let mut parser = Parser::new(&mut mock_handler);

        let json_string = serde_json::to_string(&json).unwrap();

        let mut buf = json_string.as_bytes();

        assert_eq!(parser.parse(&mut buf), Ok(()));
        assert_eq!(parser.finish_parse(), Ok(()));
    }
}

#[allow(unused)]
#[derive(Clone, Debug, Serialize)]
pub enum Json {
    #[serde(rename = "null")]
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Array(Vec<Json>),
    Map(HashMap<String, Json>),
}

#[allow(unused)]
pub fn arb_json() -> impl Strategy<Value = Json> {
    let leaf = prop_oneof![
        Just(Json::Null),
        any::<bool>().prop_map(Json::Bool),
        any::<f64>().prop_map(Json::Number),
        r#"[^"\\]*"#.prop_map(Json::String),
    ];
    leaf.prop_recursive(
        8,   // 8 levels deep
        256, // Shoot for maximum size of 256 nodes
        10,  // We put up to 10 items per collection
        |inner| {
            prop_oneof![
                // Take the inner strategy and make the two recursive cases.
                prop::collection::vec(inner.clone(), 0..10).prop_map(Json::Array),
                prop::collection::hash_map(r#"[^"\\]*"#, inner, 0..10).prop_map(Json::Map),
            ]
        },
    )
}

pub struct MockHandler<W> {
    write: W,
}

impl<W: std::io::Write> MockHandler<W> {
    pub fn new(out: W) -> Self {
        MockHandler { write: out }
    }
}

impl<W> Handler for MockHandler<W>
where
    W: std::io::Write,
{
    fn handle_bool(&mut self, _ctx: &Context, val: bool) -> Status {
        writeln!(self.write, "bool: {}", val).expect("Unable to write to stdout");
        Status::Continue
    }

    fn handle_double(&mut self, _ctx: &Context, val: f64) -> Status {
        writeln!(self.write, "double: {}", val).expect("Unable to write to stdout");
        Status::Continue
    }

    fn handle_end_array(&mut self, _ctx: &Context) -> Status {
        writeln!(self.write, "array close ']'").expect("Unable to write to stdout");
        Status::Continue
    }

    fn handle_end_map(&mut self, _ctx: &Context) -> Status {
        writeln!(self.write, "map close '}}'").expect("Unable to write to stdout");
        Status::Continue
    }

    fn handle_int(&mut self, _ctx: &Context, val: i64) -> Status {
        writeln!(self.write, "integer: {}", val).expect("Unable to write to stdout");
        Status::Continue
    }

    fn handle_map_key(&mut self, _ctx: &Context, key: &str) -> Status {
        writeln!(self.write, "key: {}", key).expect("Unable to write to stdout");
        Status::Continue
    }

    fn handle_null(&mut self, _ctx: &Context) -> Status {
        writeln!(self.write, "null").expect("Unable to write to stdout");
        Status::Continue
    }

    fn handle_start_array(&mut self, _ctx: &Context) -> Status {
        writeln!(self.write, "array open '['").expect("Unable to write to stdout");
        Status::Continue
    }

    fn handle_start_map(&mut self, _ctx: &Context) -> Status {
        writeln!(self.write, "map open '{{'").expect("Unable to write to stdout");
        Status::Continue
    }

    fn handle_string(&mut self, _ctx: &Context, val: &str) -> Status {
        writeln!(self.write, "string: '{}'", val).expect("Unable to write to stdout");
        Status::Continue
    }
}
