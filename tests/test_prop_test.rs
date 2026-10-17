//! Differential test against `serde_json`: serialize a random value, stream
//! it through `Parser`, rebuild the value from the events, compare.

mod common;

use common::chunked::Chunked;
use proptest::prelude::*;
use serde_json::{Map, Number, Value};
use yajlish::{decode_str, Context, Handler, Parser, Status};

/// Rebuilds a `serde_json::Value` from parse events.
#[derive(Default)]
struct Builder {
    stack: Vec<(Value, Option<String>)>,
    done: Option<Value>,
}

impl Builder {
    fn push_value(&mut self, v: Value) -> Status {
        match self.stack.last_mut() {
            Some((Value::Array(a), _)) => a.push(v),
            Some((Value::Object(m), key)) => {
                m.insert(key.take().expect("value without key"), v);
            }
            Some(_) => unreachable!(),
            None => self.done = Some(v),
        }
        Status::Continue
    }

    fn end(&mut self) -> Status {
        let (v, _) = self.stack.pop().expect("unbalanced end");
        self.push_value(v)
    }
}

impl Handler for Builder {
    fn handle_null(&mut self, _: &Context) -> Status {
        self.push_value(Value::Null)
    }
    fn handle_double(&mut self, _: &Context, val: f64) -> Status {
        self.push_value(Number::from_f64(val).map_or(Value::Null, Value::Number))
    }
    fn handle_int(&mut self, _: &Context, val: i64) -> Status {
        self.push_value(Value::Number(val.into()))
    }
    fn handle_bool(&mut self, _: &Context, val: bool) -> Status {
        self.push_value(Value::Bool(val))
    }
    fn handle_string(&mut self, _: &Context, val: &str) -> Status {
        self.push_value(Value::String(decode_str(val).unwrap().into_owned()))
    }
    fn handle_start_map(&mut self, _: &Context) -> Status {
        self.stack.push((Value::Object(Map::new()), None));
        Status::Continue
    }
    fn handle_end_map(&mut self, _: &Context) -> Status {
        self.end()
    }
    fn handle_map_key(&mut self, _: &Context, key: &str) -> Status {
        self.stack.last_mut().unwrap().1 = Some(decode_str(key).unwrap().into_owned());
        Status::Continue
    }
    fn handle_start_array(&mut self, _: &Context) -> Status {
        self.stack.push((Value::Array(vec![]), None));
        Status::Continue
    }
    fn handle_end_array(&mut self, _: &Context) -> Status {
        self.end()
    }
}

fn arb_json() -> impl Strategy<Value = Value> {
    let leaf = prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::Bool),
        any::<i64>().prop_map(|i| Value::Number(i.into())),
        // Finite doubles with a fraction, so they stay doubles on the way back.
        any::<f64>()
            .prop_filter("finite, non-integral", |f| f.is_finite()
                && f.fract() != 0.0)
            .prop_map(|f| Value::Number(Number::from_f64(f).unwrap())),
        any::<String>().prop_map(Value::String),
        "[\\x00-\\x7f]{0,8}".prop_map(Value::String),
    ];
    leaf.prop_recursive(6, 64, 8, |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 0..8).prop_map(Value::Array),
            prop::collection::btree_map(any::<String>(), inner, 0..8)
                .prop_map(|m| Value::Object(m.into_iter().collect())),
        ]
    })
}

fn parse(input: &[u8], chunk: usize) -> Value {
    let mut builder = Builder::default();
    let mut parser = Parser::new(&mut builder);
    parser.parse(&mut Chunked::new(input, chunk)).unwrap();
    parser.finish_parse().unwrap();
    builder.done.expect("no value")
}

proptest! {
    #[test]
    fn test_prop_matches_serde_json(json in arb_json(), chunk in 1usize..16) {
        for text in [
            serde_json::to_string(&json).unwrap(),
            serde_json::to_string_pretty(&json).unwrap(),
        ] {
            prop_assert_eq!(&parse(text.as_bytes(), usize::MAX), &json);
            prop_assert_eq!(&parse(text.as_bytes(), chunk), &json);
        }
    }

    /// serde_json writes non-ASCII as raw UTF-8; also check `\u` escapes,
    /// surrogate pairs included, decode to the same strings.
    #[test]
    fn test_prop_escaped_strings(s in any::<String>()) {
        let mut text = String::from("\"");
        for unit in s.encode_utf16() {
            text.push_str(&format!("\\u{:04x}", unit));
        }
        text.push('"');
        prop_assert_eq!(parse(text.as_bytes(), 3), Value::String(s));
    }
}
