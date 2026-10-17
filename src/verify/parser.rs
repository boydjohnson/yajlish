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

//! Proofs about the parser, and `Recorder`, the handler they use.

use super::{for_each_split, same_result, Chunks};
use crate::{Context, Enclosing, Handler, ParseError, Parser, Status};

/// One handler callback. Strings and keys are recorded by position, which
/// `Recorder` checks against the input, so equal events mean equal text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Event {
    Null,
    Bool(bool),
    Int(i64),
    /// The bits of the `f64`, so that two NaNs compare equal.
    Double(u64),
    String {
        len: usize,
    },
    Key {
        len: usize,
    },
    StartMap,
    EndMap,
    StartArray,
    EndArray,
}

/// A handler that logs up to `N` events, each with `ctx.offset()`, in a
/// fixed-size array (no `Vec` growth under Kani).
///
/// With `strict`, every callback also asserts the `Context` invariants:
/// - `num_open_braces() + num_open_brackets()` is the depth implied by the
///   start and end events so far, and `last_enclosing()` is the innermost
///   open event;
/// - a key only arrives inside a map, directly after `StartMap` or a value;
/// - `ctx.offset()` is inside the input and greater than the previous
///   event's.
pub(crate) struct Recorder<'i, const N: usize> {
    input: &'i [u8],
    strict: bool,
    events: [(Event, u64); N],
    len: usize,
    /// The open containers, innermost last.
    open: [Enclosing; N],
    depth: usize,
    /// Whether a key may come next.
    key_ok: bool,
}

impl<'i, const N: usize> Recorder<'i, N> {
    pub(crate) fn new(input: &'i [u8], strict: bool) -> Self {
        Recorder {
            input,
            strict,
            events: [(Event::Null, 0); N],
            len: 0,
            open: [Enclosing::LeftBrace; N],
            depth: 0,
            key_ok: false,
        }
    }

    pub(crate) fn events(&self) -> &[(Event, u64)] {
        &self.events[..self.len]
    }

    fn innermost(&self) -> Option<Enclosing> {
        self.depth.checked_sub(1).map(|i| self.open[i])
    }

    fn record(&mut self, ctx: &Context, event: Event) -> Status {
        let offset = ctx.offset();
        assert!(
            (offset as usize) < self.input.len(),
            "offset past the input"
        );
        if self.strict {
            assert_eq!(
                ctx.num_open_braces() + ctx.num_open_brackets(),
                self.depth,
                "context depth"
            );
            assert_eq!(ctx.last_enclosing(), self.innermost(), "last_enclosing");
            if let Some(&(_, prev)) = self.events().last() {
                assert!(offset > prev, "offsets must increase");
            }
            if let Event::Key { .. } = event {
                assert!(self.key_ok, "key out of place");
            }
        }
        assert!(self.len < N, "Recorder is full");
        self.events[self.len] = (event, offset);
        self.len += 1;

        match event {
            Event::StartMap | Event::StartArray => {
                self.open[self.depth] = if event == Event::StartMap {
                    Enclosing::LeftBrace
                } else {
                    Enclosing::LeftBracket
                };
                self.depth += 1;
                self.key_ok = event == Event::StartMap;
            }
            Event::Key { .. } => self.key_ok = false,
            _ => {
                if matches!(event, Event::EndMap | Event::EndArray) {
                    self.depth = self.depth.saturating_sub(1);
                }
                self.key_ok = self.innermost() == Some(Enclosing::LeftBrace);
            }
        }
        Status::Continue
    }

    /// Record a string or key, checking that it is the input's bytes at
    /// `ctx.offset()`.
    fn record_text(&mut self, ctx: &Context, val: &str, key: bool) -> Status {
        let at = ctx.offset() as usize;
        assert_eq!(
            self.input.get(at..at + val.len()),
            Some(val.as_bytes()),
            "text is not the input at its offset"
        );
        let len = val.len();
        self.record(
            ctx,
            if key {
                Event::Key { len }
            } else {
                Event::String { len }
            },
        )
    }
}

impl<const N: usize> Handler for Recorder<'_, N> {
    fn handle_null(&mut self, ctx: &Context) -> Status {
        self.record(ctx, Event::Null)
    }
    fn handle_double(&mut self, ctx: &Context, val: f64) -> Status {
        self.record(ctx, Event::Double(val.to_bits()))
    }
    fn handle_int(&mut self, ctx: &Context, val: i64) -> Status {
        self.record(ctx, Event::Int(val))
    }
    fn handle_bool(&mut self, ctx: &Context, val: bool) -> Status {
        self.record(ctx, Event::Bool(val))
    }
    fn handle_string(&mut self, ctx: &Context, val: &str) -> Status {
        self.record_text(ctx, val, false)
    }
    fn handle_start_map(&mut self, ctx: &Context) -> Status {
        self.record(ctx, Event::StartMap)
    }
    fn handle_end_map(&mut self, ctx: &Context) -> Status {
        self.record(ctx, Event::EndMap)
    }
    fn handle_map_key(&mut self, ctx: &Context, key: &str) -> Status {
        self.record_text(ctx, key, true)
    }
    fn handle_start_array(&mut self, ctx: &Context) -> Status {
        self.record(ctx, Event::StartArray)
    }
    fn handle_end_array(&mut self, ctx: &Context) -> Status {
        self.record(ctx, Event::EndArray)
    }
}

/// Stands in for `parser::number_value` under Kani, where float parsing is
/// too large to check. Deterministic in `text`, so harnesses that compare
/// two runs see the same value for the same token. For the K3 parser
/// harnesses.
#[cfg(kani)]
#[allow(dead_code)] // Only named by `kani::stub`.
fn stub_number_value(
    kind: crate::lexer::Token,
    text: &str,
    _offset: u64,
) -> Result<crate::parser::Number, ParseError> {
    use crate::parser::Number;
    let mut hash = 0u64;
    for &b in text.as_bytes() {
        hash = hash.wrapping_mul(31).wrapping_add(u64::from(b));
    }
    Ok(if kind == crate::lexer::Token::Int {
        Number::Int(hash as i64)
    } else {
        Number::Double(f64::from_bits(hash))
    })
}

/// `parse` over `input` read in chunks of `sizes`, then `finish_parse`.
fn parse_chunked<'i, const N: usize>(
    input: &'i [u8],
    sizes: &[usize],
    strict: bool,
) -> (Recorder<'i, N>, Result<(), ParseError>) {
    let mut recorder = Recorder::new(input, strict);
    let mut parser = Parser::new(&mut recorder);
    let mut reader = Chunks::new(input, sizes);
    let result = parser
        .parse(&mut reader)
        .and_then(|()| parser.finish_parse());
    (recorder, result)
}

fn check_chunked_parse_is_invisible<const N: usize>(input: &[u8], sizes: &[usize]) {
    let (chunked, chunked_result) = parse_chunked::<N>(input, sizes, false);
    let (whole, whole_result) = parse_chunked::<N>(input, &[], false);
    assert_eq!(chunked.events(), whole.events());
    assert!(
        same_result(&chunked_result, &whole_result),
        "{:?} vs {:?}",
        chunked_result,
        whole_result
    );
}

/// Input size of the Bolero harness's small form.
const CHUNKED_PARSE_BOUND: usize = 6;

/// `chunking_is_invisible` at the event level, through one `parse` call.
/// The exhaustive form of `tests/test_split_points.rs`.
// Not a Kani proof: CBMC does not finish `Parser` even on two
// symbolic bytes (see "Bounds" in `plans/04-kani-bolero.md`).
// `tests::exhaustive_short_parse_inputs` is the exhaustive form instead.
#[test]
fn chunked_parse_is_invisible() {
    bolero::check!()
        .with_type::<([u8; CHUNKED_PARSE_BOUND], u8, u8)>()
        .cloned()
        .for_each(|(buf, len, split)| {
            for_each_split(&buf, len, split, |input, split| {
                check_chunked_parse_is_invisible::<CHUNKED_PARSE_BOUND>(input, &[split]);
            });
        });
}

#[test]
fn chunked_parse_is_invisible_fuzz() {
    bolero::check!()
        .with_type::<(Vec<u8>, Vec<u8>)>()
        .for_each(|(input, sizes)| {
            let input = &input[..input.len().min(256)];
            let sizes: Vec<usize> = sizes.iter().map(|&s| usize::from(s)).collect();
            check_chunked_parse_is_invisible::<256>(input, &sizes);
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `chunked_parse_is_invisible` over the inputs of
    /// `exhaustive_short_stream_inputs`, with the real `number_value` and
    /// logos.
    #[test]
    fn exhaustive_short_parse_inputs() {
        let check = |input: &[u8]| {
            for split in 0..=input.len() {
                check_chunked_parse_is_invisible::<8>(input, &[split]);
            }
        };
        crate::verify::for_each_shorter_input(crate::verify::ENUM_BYTES + 1, check);
        crate::verify::for_each_alphabet_input(crate::verify::ENUM_ALPHABET, check);
    }

    /// The strict checks hold on valid documents, so a failure under Kani
    /// is the parser's, not the `Recorder`'s.
    #[test]
    fn strict_recorder_accepts_valid_json() {
        for input in [
            &br#"{"a": [1, true, {"b": null}], "c": "d", "e": {}}"#[..],
            b"[[], {}, [[\"x\"]], -0.5e3]",
            b" \"top\" ",
            b"12",
        ] {
            let (recorder, result) = parse_chunked::<32>(input, &[], true);
            assert_eq!(result, Ok(()), "{:?}", String::from_utf8_lossy(input));
            assert!(!recorder.events().is_empty());
        }
    }

    #[test]
    fn recorder_logs_events_and_offsets() {
        let input = br#"{"a":[1.5,"b"]}"#;
        let (recorder, result) = parse_chunked::<16>(input, &[], true);
        assert_eq!(result, Ok(()));
        assert_eq!(
            recorder.events(),
            &[
                (Event::StartMap, 0),
                (Event::Key { len: 3 }, 1),
                (Event::StartArray, 5),
                (Event::Double(1.5f64.to_bits()), 6),
                (Event::String { len: 3 }, 10),
                (Event::EndArray, 13),
                (Event::EndMap, 14),
            ]
        );
    }
}
