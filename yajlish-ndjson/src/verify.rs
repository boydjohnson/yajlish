//! Bounded proofs and property tests (Plan 4, K4), see
//! `plans/04-kani-bolero.md` in the workspace root.
//!
//! None of these are Kani proofs. `NdJsonHandler` only runs behind
//! `yajlish::Parser::parse`, whose streaming lexer Kani cannot get through,
//! and Kani did not finish the selector parser on 3 symbolic bytes. So each
//! property is a Bolero test plus exhaustive enumeration of short inputs.

// `cargo kani --tests` also builds this crate without `cfg(test)`, where
// the `#[test]` harnesses and so their helpers are gone.
#![cfg_attr(not(test), allow(dead_code, unused_imports))]

use crate::{
    selector_parser::{ArrayTokenParser, IdentifierTokenParser, SelectorParser},
    NdJsonError, NdJsonHandler, Selector,
};
use yajlish::Parser;

/// All three selector parsers on `input`. None may panic.
fn check_selector_parsers(input: &str) {
    let _ = SelectorParser::new().parse(input);
    let _ = IdentifierTokenParser::new().parse(input);
    let _ = ArrayTokenParser::new().parse(input);
}

/// The selector parsers on random strings. Not a Kani proof: Kani did not
/// finish even on 3 symbolic ASCII bytes (char decoding, `String` and
/// `Vec` allocation, integer parsing), so `tests::exhaustive_selector_*`
/// are the exhaustive form.
#[test]
fn selector_parse_never_panics_fuzz() {
    bolero::check!().with_type::<String>().for_each(|input| {
        check_selector_parsers(input);
    });
}

/// The selector string for `selectors`, as a user would write it.
fn print_selectors(selectors: &[Selector]) -> String {
    let mut out = String::from("d");
    for selector in selectors {
        match selector {
            Selector::Identifier(key) => {
                out.push('.');
                out.push_str(key);
            }
            Selector::Index(index) => out.push_str(&format!("[{}]", index)),
        }
    }
    out
}

/// Printing selectors and parsing them back gives the same selectors, for
/// keys that are non-empty and contain no `.` or `[`. Whitespace inside
/// keys, at either end, is kept.
#[test]
fn selector_round_trip() {
    bolero::check!()
        .with_type::<Vec<(Option<String>, usize)>>()
        .for_each(|pieces| {
            let selectors: Vec<Selector> = pieces
                .iter()
                .map(|(key, index)| match key {
                    Some(key) => {
                        let key: String = key.chars().filter(|c| !matches!(c, '.' | '[')).collect();
                        Selector::Identifier(if key.is_empty() { "k".into() } else { key })
                    }
                    None => Selector::Index(*index),
                })
                .collect();
            let printed = print_selectors(&selectors);
            assert_eq!(
                SelectorParser::new().parse(&printed),
                Ok(selectors),
                "{:?}",
                printed
            );
        });
}

/// `NdJsonHandler` with the selector `d[0]` or `d.a`, through `parse` and
/// `finish_parse`, then `finish`. Nothing may panic.
fn check_ndjson_never_panics(input: &[u8]) {
    for selector in ["d[0]", "d.a"] {
        let selectors = SelectorParser::new().parse(selector).unwrap();
        let mut handler = NdJsonHandler::new(vec![], selectors);
        let mut parser = Parser::new(&mut handler);
        let _ = parser
            .parse(&mut &input[..])
            .and_then(|()| parser.finish_parse());
        let _ = handler.finish();
    }
}

#[test]
fn ndjson_never_panics() {
    bolero::check!()
        .with_type::<([u8; 8], u8)>()
        .cloned()
        .for_each(|(buf, len)| {
            check_ndjson_never_panics(&buf[..usize::from(len) % 9]);
        });
}

#[test]
fn ndjson_never_panics_fuzz() {
    // Raw bytes, so that the seed corpus (JSON files) is the input.
    bolero::check!().for_each(|input: &[u8]| {
        check_ndjson_never_panics(input);
    });
}

/// A JSON value chosen by `choices`, nesting at most `depth` more levels.
/// Doubles are non-integral so that they print back as doubles.
#[cfg(test)]
fn gen_value(choices: &mut impl Iterator<Item = u8>, depth: u32) -> serde_json::Value {
    use serde_json::Value;
    let c = choices.next().unwrap_or(0);
    match c % 9 {
        0 => Value::Null,
        1 => Value::Bool(c & 0x10 != 0),
        2 => Value::from(i64::from(c) - 128),
        3 => Value::from(i64::MIN + i64::from(c)),
        4 => Value::from(f64::from(c) + 0.5),
        5 => Value::from(["", "a\"b", "\u{e9}\\n", "\u{1F600}", "\u{1}"][usize::from(c) % 5]),
        6 if depth > 0 => {
            let n = choices.next().unwrap_or(0) % 4;
            Value::Array((0..n).map(|_| gen_value(choices, depth - 1)).collect())
        }
        7 if depth > 0 => {
            let n = choices.next().unwrap_or(0) % 4;
            Value::Object(
                (0..n)
                    .map(|i| {
                        (
                            ["b", "x", "\u{e9}", "a\"b"][usize::from(i)].to_string(),
                            gen_value(choices, depth - 1),
                        )
                    })
                    .collect(),
            )
        }
        _ => Value::from("s"),
    }
}

/// The fuzz version of `tests/test_prop_test.rs`: with the selector
/// `d.a.b` (or `d[1].b`), each output line parses to the matching element
/// of the array at `/a/b` (or `/1/b`), and `finish` is `SelectorNotFound`
/// exactly when there is no array there.
#[cfg(test)]
fn check_lines_match_elements(choices: &[u8], pretty: bool) {
    use serde_json::{json, Value};
    let mut choices = choices.iter().copied();
    let a = match choices.next().unwrap_or(0) % 4 {
        // The usual case: an array at /a/b.
        0 | 1 => {
            let n = choices.next().unwrap_or(0) % 6;
            json!({ "b": (0..n).map(|_| gen_value(&mut choices, 3)).collect::<Vec<_>>() })
        }
        // Something else at /a/b, or no /a/b at all.
        2 => json!({ "b": gen_value(&mut choices, 2) }),
        _ => gen_value(&mut choices, 2),
    };
    // The same `a` at `/a` of an object or at `/1` of an array.
    let (doc, selector, pointer) = if choices.next().unwrap_or(0) % 2 == 0 {
        let doc = json!({ "before": gen_value(&mut choices, 2), "a": a, "after": gen_value(&mut choices, 2) });
        (doc, "d.a.b", "/a/b")
    } else {
        let mut items = vec![gen_value(&mut choices, 2)];
        // Sometimes no element 1 at all.
        if choices.next().unwrap_or(0) % 4 != 0 {
            items.push(a);
            items.push(gen_value(&mut choices, 2));
        }
        (Value::Array(items), "d[1].b", "/1/b")
    };
    let text = if pretty {
        serde_json::to_string_pretty(&doc).unwrap()
    } else {
        serde_json::to_string(&doc).unwrap()
    };

    let selectors = SelectorParser::new().parse(selector).unwrap();
    let mut handler = NdJsonHandler::new(vec![], selectors);
    let mut parser = Parser::new(&mut handler);
    parser.parse(&mut text.as_bytes()).unwrap();
    parser.finish_parse().unwrap();
    match (doc.pointer(pointer), handler.finish()) {
        (Some(Value::Array(elements)), Ok(out)) => {
            let out = String::from_utf8(out).unwrap();
            let lines: Vec<Value> = out
                .lines()
                .map(|line| serde_json::from_str(line).unwrap())
                .collect();
            assert_eq!(&lines, elements, "{}", text);
        }
        (Some(Value::Array(_)), Err(e)) => panic!("{}: {:?}", text, e),
        (_, Err(NdJsonError::SelectorNotFound)) => {}
        (_, other) => panic!(
            "{}: expected SelectorNotFound, got {:?}",
            text,
            other.map(String::from_utf8)
        ),
    }
}

#[test]
fn ndjson_lines_match_elements() {
    bolero::check!()
        .with_type::<(Vec<u8>, bool)>()
        .for_each(|(choices, pretty)| check_lines_match_elements(choices, *pretty));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `selector` on `doc` selects no array, so `finish` must be
    /// `SelectorNotFound` and nothing may be written.
    fn selects_nothing(selector: &str, doc: &str) {
        let selectors = SelectorParser::new().parse(selector).unwrap();
        let mut handler = NdJsonHandler::new(vec![], selectors);
        let mut parser = Parser::new(&mut handler);
        parser.parse(&mut doc.as_bytes()).unwrap();
        parser.finish_parse().unwrap();
        let out = handler.finish().map(String::from_utf8);
        assert!(
            matches!(out, Err(NdJsonError::SelectorNotFound)),
            "{} on {}: {:?}",
            selector,
            doc,
            out
        );
    }

    /// Regression tests for a bug `ndjson_lines_match_elements` found: the
    /// selector was not anchored to what it matched. Once a step matched,
    /// the handler looked for the next step (or for an array, after the
    /// last step) anywhere later in the document at that depth, even after
    /// the matched value had ended. Here the selected value is not an
    /// array, and the next `[` started writing at the wrong depth: it wrote
    /// `{"x":1]}}`.
    #[test]
    fn non_array_selection_inside() {
        selects_nothing("d.a", r#"{"a":{"x":[1]}}"#);
    }

    /// The array after the selected value: the fuzz counterexample,
    /// `{"a":{"b":null},"after":null,"before":[]}`, wrote
    /// `null},"after":null,"before":]}`.
    #[test]
    fn non_array_selection_after() {
        selects_nothing("d.a.b", r#"{"a":{"b":null},"after":null,"before":[]}"#);
    }

    /// One step earlier: `a` is `null`, not an object, and the handler took
    /// `b` from the next object, `c`, and wrote `1`.
    #[test]
    fn non_array_selection_sibling() {
        selects_nothing("d.a.b", r#"{"a":null,"c":{"b":[1]}}"#);
    }

    /// With the right kinds: `a` is an object without `b`, and the handler
    /// took `b` from the next object, `c`.
    #[test]
    fn non_array_selection_missing_key() {
        selects_nothing("d.a.b", r#"{"a":{},"c":{"b":[1]}}"#);
    }

    /// Every ASCII string up to 3 bytes.
    #[test]
    fn exhaustive_selector_ascii() {
        fn go(input: &mut String, left: usize) {
            check_selector_parsers(input);
            if left == 0 {
                return;
            }
            for c in 0..=127u8 {
                input.push(char::from(c));
                go(input, left - 1);
                input.pop();
            }
        }
        go(&mut String::new(), 3);
    }

    /// Every string of up to 5 characters over an alphabet with one
    /// character from each class the selector grammar tells apart, plus
    /// non-ASCII letters and Unicode whitespace.
    #[test]
    fn exhaustive_selector_strings() {
        const ALPHABET: [char; 12] = [
            'd', '.', '[', ']', '0', '9', ' ', 'a', '\u{e9}', '\u{a0}', '\u{3000}', '\n',
        ];
        fn go(input: &mut String, left: usize) {
            check_selector_parsers(input);
            if left == 0 {
                return;
            }
            for c in ALPHABET {
                input.push(c);
                go(input, left - 1);
                input.pop();
            }
        }
        go(&mut String::new(), 5);
    }

    /// Every byte string up to 2 bytes, and every string up to 4 bytes over
    /// an alphabet with one byte from each class the JSON grammar tells
    /// apart plus `a`, the selected key.
    #[test]
    fn exhaustive_ndjson_inputs() {
        const ALPHABET: &[u8] = b" \"\\{}[]:,-019.etrufalsna\xc3\xa9\xff";
        for a in 0..=255u8 {
            check_ndjson_never_panics(&[a]);
            for b in 0..=255u8 {
                check_ndjson_never_panics(&[a, b]);
            }
        }
        fn go(input: &mut Vec<u8>, left: usize) {
            check_ndjson_never_panics(input);
            if left == 0 {
                return;
            }
            for &c in ALPHABET {
                input.push(c);
                go(input, left - 1);
                input.pop();
            }
        }
        go(&mut vec![], 4);
    }
}
