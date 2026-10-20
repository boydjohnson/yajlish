//! Differential tests of the output lines against the input elements.
//!
//! `test_prop_lines_match_elements`: each output line is the element's
//! text with insignificant whitespace removed, byte for byte, so numbers
//! are written as the input spelled them. As a second check, it parses
//! (with `serde_json`) to the same value as the element.

use proptest::prelude::*;
use serde_json::Value;
use yajlish::Parser;
use yajlish_ndjson::{NdJsonHandler, SelectorParser};

/// RFC 8259 number text: optional `-`, an integer part without leading
/// zeros, an optional fraction and an optional exponent of either case and
/// any sign. Mantissas go past `i64` and `f64` precision, and exponents
/// past `f64`'s range both ways.
fn arb_number() -> impl Strategy<Value = String> {
    (
        any::<bool>(),
        "0|[1-9][0-9]{0,40}",
        proptest::option::of("[0-9]{1,40}"),
        proptest::option::of(("[eE]", "[+-]?", "[0-9]{1,5}")),
    )
        .prop_map(|(neg, int, frac, exp)| {
            let mut text = String::new();
            if neg {
                text.push('-');
            }
            text.push_str(&int);
            if let Some(frac) = frac {
                text.push('.');
                text.push_str(&frac);
            }
            if let Some((e, sign, digits)) = exp {
                text.push_str(&e);
                text.push_str(&sign);
                text.push_str(&digits);
            }
            text
        })
}

/// Whitespace JSON allows between tokens.
fn arb_ws() -> impl Strategy<Value = &'static str> {
    prop::sample::select(vec!["", "", " ", "\n  ", "\t", "\r\n"])
}

/// The text of a JSON value, with whitespace between its tokens.
fn arb_json_text() -> impl Strategy<Value = String> {
    let leaf = prop_oneof![
        Just("null".to_owned()),
        any::<bool>().prop_map(|b| b.to_string()),
        arb_number(),
        any::<String>().prop_map(|s| serde_json::to_string(&s).unwrap()),
    ];
    leaf.prop_recursive(4, 32, 6, |inner| {
        prop_oneof![
            (prop::collection::vec(inner.clone(), 0..6), arb_ws()).prop_map(|(items, ws)| {
                format!("[{ws}{}{ws}]", items.join(&format!("{ws},{ws}")))
            }),
            (
                prop::collection::vec(("[a-z\\\\\"]{0,4}", inner), 0..6),
                arb_ws()
            )
                .prop_map(|(members, ws)| {
                    let members: Vec<String> = members
                        .iter()
                        .map(|(k, v)| format!("{}{ws}:{ws}{v}", serde_json::to_string(k).unwrap()))
                        .collect();
                    format!("{{{ws}{}{ws}}}", members.join(&format!("{ws},{ws}")))
                }),
        ]
    })
}

/// `text` without the whitespace outside strings.
fn minify(text: &str) -> String {
    let mut out = String::new();
    let (mut in_string, mut escaped) = (false, false);
    for c in text.chars() {
        if in_string {
            match (escaped, c) {
                (true, _) => escaped = false,
                (false, '\\') => escaped = true,
                (false, '"') => in_string = false,
                _ => {}
            }
        } else if c == '"' {
            in_string = true;
        } else if matches!(c, ' ' | '\t' | '\n' | '\r') {
            continue;
        }
        out.push(c);
    }
    out
}

proptest! {
    #[test]
    fn test_prop_lines_match_elements(
        elements in prop::collection::vec(arb_json_text(), 0..8),
        before in arb_json_text(),
        after in arb_json_text(),
        ws in arb_ws(),
    ) {
        let text = format!(
            "{{{ws}\"before\":{ws}{before},{ws}\"a\":{ws}{{\"b\":{ws}[{ws}{}{ws}]}},{ws}\"after\":{ws}{after}{ws}}}",
            elements.join(&format!("{ws},{ws}")),
        );

        let selectors = SelectorParser::new().parse("d.a.b").unwrap();
        let mut handler = NdJsonHandler::new(vec![], selectors);
        let mut parser = Parser::new(&mut handler);
        parser.parse(&mut text.as_bytes()).unwrap();
        parser.finish_parse().unwrap();
        let out = String::from_utf8(handler.finish().unwrap()).unwrap();

        let lines: Vec<&str> = out.lines().collect();
        let expected: Vec<String> = elements.iter().map(|e| minify(e)).collect();
        prop_assert_eq!(&lines, &expected);
        for (line, element) in lines.iter().zip(&elements) {
            // serde_json refuses numbers beyond f64's range, like 1e400.
            if let Ok(value) = serde_json::from_str::<Value>(element) {
                prop_assert_eq!(serde_json::from_str::<Value>(line).unwrap(), value);
            }
        }
    }
}
