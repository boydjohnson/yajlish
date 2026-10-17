//! Differential test against `serde_json`: each output line must parse to
//! the corresponding element of the selected array.

use proptest::prelude::*;
use serde_json::{json, Number, Value};
use yajlish::Parser;
use yajlish_ndjson::{NdJsonHandler, SelectorParser};

fn arb_json() -> impl Strategy<Value = Value> {
    let leaf = prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::Bool),
        any::<i64>().prop_map(|i| Value::Number(i.into())),
        // Doubles are re-printed, so keep them non-integral to stay doubles.
        any::<f64>()
            .prop_filter("finite, non-integral", |f| f.is_finite()
                && f.fract() != 0.0)
            .prop_map(|f| Value::Number(Number::from_f64(f).unwrap())),
        any::<String>().prop_map(Value::String),
    ];
    leaf.prop_recursive(4, 32, 6, |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 0..6).prop_map(Value::Array),
            prop::collection::btree_map("[a-z\\\\\"]{0,4}", inner, 0..6)
                .prop_map(|m| Value::Object(m.into_iter().collect())),
        ]
    })
}

proptest! {
    #[test]
    fn test_prop_lines_match_elements(
        elements in prop::collection::vec(arb_json(), 0..8),
        before in arb_json(),
        after in arb_json(),
        pretty in any::<bool>(),
    ) {
        let doc = json!({ "before": before, "a": { "b": elements.clone() }, "after": after });
        let text = if pretty {
            serde_json::to_string_pretty(&doc).unwrap()
        } else {
            serde_json::to_string(&doc).unwrap()
        };

        let selectors = SelectorParser::new().parse("d.a.b").unwrap();
        let mut handler = NdJsonHandler::new(vec![], selectors);
        let mut parser = Parser::new(&mut handler);
        parser.parse(&mut text.as_bytes()).unwrap();
        parser.finish_parse().unwrap();
        let out = String::from_utf8(handler.finish().unwrap()).unwrap();

        let lines: Vec<Value> = out
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        prop_assert_eq!(lines, elements);
    }
}
