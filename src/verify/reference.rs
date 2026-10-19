/*
* Copyright 2020 Boyd Johnson
*
* Licensed under the Apache License, Version 2.0 (the "License");
* you may not use this file except in compliance with the License.
* You may obtain a copy of the License at
*
*     http://www.apache.org/licenses/LICENSE-2.0
*
* Unless required by applicable law or agreed to in writing, software
* distributed under the License is distributed on an "AS IS" BASIS,
* WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
* See the License for the specific language governing permissions and
* limitations under the License.
* ------------------------------------------------------------------------------
*/

//! The reference recognizer: RFC 8259's grammar over tokens, as an
//! explicit-stack pushdown automaton, written without looking at
//! `parse_source`. It is the oracle for the parser harnesses, so it is
//! checked against JSONTestSuite with no allowlist.
//!
//! ```text
//! JSON-text = value
//! value     = object / array / string / number / true / false / null
//! object    = "{" [ string ":" value *( "," string ":" value ) ] "}"
//! array     = "[" [ value *( "," value ) ] "]"
//! ```

use super::{as_utf8, parser::Event};
use crate::{
    lexer::{Lexed, Token},
    parser::{number_value, TokenSource},
    Enclosing, Number,
};

/// What the automaton accepts next.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Expect {
    /// A value: at the start, after `:`, or after `,` in an array.
    Value,
    /// A value or `]`, right after `[`.
    ValueOrEndArray,
    /// A key or `}`, right after `{`.
    KeyOrEndMap,
    /// A key, after `,` in an object.
    Key,
    /// `:` after a key.
    Colon,
    /// `,` or the end of the innermost container, after a value in it.
    CommaOrEnd,
    /// Nothing more: the top-level value is complete.
    Done,
}

/// Runs the grammar over `tokens`, calling `emit` with the event a
/// correct parser delivers for each token and its offset. True if the
/// tokens are exactly one JSON value and the source ends at `Eof`.
///
/// `stack` holds the open containers and must be at least as long as the
/// nesting depth; the number of tokens always suffices.
pub(crate) fn accepts(
    tokens: &mut impl TokenSource,
    stack: &mut [Enclosing],
    mut emit: impl FnMut(Event, u64),
) -> bool {
    let mut depth = 0;
    let mut expect = Expect::Value;
    loop {
        let (kind, bytes, offset) = match tokens.next_token() {
            Ok(Lexed::Token {
                kind,
                bytes,
                offset,
            }) => (kind, bytes, offset),
            Ok(Lexed::Eof) => return expect == Expect::Done,
            Err(_) => return false,
        };
        let mut close = None;
        expect = match (expect, kind) {
            (Expect::ValueOrEndArray, Token::RightBracket) => {
                close = Some(Event::EndArray);
                Expect::Done
            }
            (Expect::KeyOrEndMap, Token::RightBrace) => {
                close = Some(Event::EndMap);
                Expect::Done
            }
            (Expect::CommaOrEnd, Token::RightBracket)
                if stack[depth - 1] == Enclosing::LeftBracket =>
            {
                close = Some(Event::EndArray);
                Expect::Done
            }
            (Expect::CommaOrEnd, Token::RightBrace) if stack[depth - 1] == Enclosing::LeftBrace => {
                close = Some(Event::EndMap);
                Expect::Done
            }
            (Expect::CommaOrEnd, Token::Comma) => {
                if stack[depth - 1] == Enclosing::LeftBracket {
                    Expect::Value
                } else {
                    Expect::Key
                }
            }
            (Expect::KeyOrEndMap | Expect::Key, Token::String) => {
                if as_utf8(bytes).is_none() {
                    return false;
                }
                emit(Event::Key { len: bytes.len() }, offset);
                Expect::Colon
            }
            (Expect::Colon, Token::Colon) => Expect::Value,
            (Expect::Value | Expect::ValueOrEndArray, Token::LeftBrace) => {
                stack[depth] = Enclosing::LeftBrace;
                depth += 1;
                emit(Event::StartMap, offset);
                Expect::KeyOrEndMap
            }
            (Expect::Value | Expect::ValueOrEndArray, Token::LeftBracket) => {
                stack[depth] = Enclosing::LeftBracket;
                depth += 1;
                emit(Event::StartArray, offset);
                Expect::ValueOrEndArray
            }
            (Expect::Value | Expect::ValueOrEndArray, _) => {
                let event = match kind {
                    Token::Null => Event::Null,
                    Token::True => Event::Bool(true),
                    Token::False => Event::Bool(false),
                    Token::String => match as_utf8(bytes) {
                        Some(_) => Event::String { len: bytes.len() },
                        None => return false,
                    },
                    Token::Int | Token::Number => {
                        let Some(text) = as_utf8(bytes) else {
                            return false;
                        };
                        match number_value(kind, text, offset) {
                            Ok(Number::Int(i)) => Event::Int(i),
                            Ok(Number::Double(d)) => Event::Double(d.to_bits()),
                            Err(_) => return false,
                        }
                    }
                    _ => return false,
                };
                emit(event, offset);
                Expect::Done
            }
            _ => return false,
        };
        if let Some(event) = close {
            depth -= 1;
            emit(event, offset);
        }
        // After a complete value: done at the top level, else a `,` or the
        // end of the enclosing container.
        if expect == Expect::Done && depth > 0 {
            expect = Expect::CommaOrEnd;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::verify::tokens::{lex_all, TokenSeq};
    use std::path::Path;

    fn reference_accepts(input: &[u8]) -> Option<bool> {
        let (tokens, end) = lex_all(input);
        if end.is_err() {
            return None;
        }
        let mut seq = TokenSeq::new(&tokens, &end);
        let mut stack = vec![Enclosing::LeftBrace; tokens.len()];
        Some(accepts(&mut seq, &mut stack, |_, _| {}))
    }

    /// The reference accepts every `y_*` file and rejects every `n_*` file
    /// whose tokens lex, with no allowlist.
    #[test]
    fn reference_passes_json_test_suite() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/json_test_suite/test_parsing");
        let mut checked = 0;
        let mut failures = vec![];
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            let input = std::fs::read(&path).unwrap();
            let verdict = reference_accepts(&input);
            if name.starts_with("y_") {
                checked += 1;
                if verdict != Some(true) {
                    failures.push(format!("{}: {:?}", name, verdict));
                }
            } else if name.starts_with("n_") {
                checked += 1;
                if verdict == Some(true) {
                    failures.push(format!("accepted {}", name));
                }
            }
        }
        assert!(checked > 250, "only {} files checked", checked);
        assert!(failures.is_empty(), "{:#?}", failures);
    }

    #[test]
    fn reference_events() {
        let input = br#"{"a":[1,true,{}],"b":null}"#;
        let (tokens, end) = lex_all(input);
        let mut seq = TokenSeq::new(&tokens, &end);
        let mut stack = vec![Enclosing::LeftBrace; tokens.len()];
        let mut events = vec![];
        assert!(accepts(&mut seq, &mut stack, |e, at| events.push((e, at))));
        assert_eq!(
            events,
            vec![
                (Event::StartMap, 0),
                (Event::Key { len: 3 }, 1),
                (Event::StartArray, 5),
                (Event::Int(1), 6),
                (Event::Bool(true), 8),
                (Event::StartMap, 13),
                (Event::EndMap, 14),
                (Event::EndArray, 15),
                (Event::Key { len: 3 }, 17),
                (Event::Null, 21),
                (Event::EndMap, 25),
            ]
        );
    }
}
