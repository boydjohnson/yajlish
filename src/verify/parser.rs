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

/// One handler callback. Strings and keys are recorded by length; when it
/// has the input, `Recorder` checks their text against it at the event's
/// offset, so equal events at equal offsets mean equal text.
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
///
/// With `abort_at(i)`, the `i`th callback returns `Abort`, and any callback
/// after it fails.
pub(crate) struct Recorder<'i, const N: usize> {
    /// The input, when the tokens came from bytes. Otherwise offsets are
    /// only checked against `end`.
    input: Option<&'i [u8]>,
    /// Offsets are below this.
    end: u64,
    strict: bool,
    abort_at: Option<usize>,
    aborted: bool,
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
            input: Some(input),
            end: input.len() as u64,
            strict,
            abort_at: None,
            aborted: false,
            events: [(Event::Null, 0); N],
            len: 0,
            open: [Enclosing::LeftBrace; N],
            depth: 0,
            key_ok: false,
        }
    }

    /// For tokens that did not come from bytes: offsets must be below
    /// `end`, and text is not checked.
    pub(crate) fn for_tokens(end: u64, strict: bool) -> Self {
        Recorder {
            input: None,
            end,
            ..Recorder::new(&[], strict)
        }
    }

    pub(crate) fn abort_at(mut self, index: usize) -> Self {
        self.abort_at = Some(index);
        self
    }

    pub(crate) fn events(&self) -> &[(Event, u64)] {
        &self.events[..self.len]
    }

    fn innermost(&self) -> Option<Enclosing> {
        self.depth.checked_sub(1).map(|i| self.open[i])
    }

    fn record(&mut self, ctx: &Context, event: Event) -> Status {
        assert!(!self.aborted, "callback after Abort");
        let offset = ctx.offset();
        assert!(offset < self.end, "offset past the input");
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
        if self.abort_at == Some(self.len - 1) {
            self.aborted = true;
            return Status::Abort;
        }
        Status::Continue
    }

    /// Record a string or key, checking that it is the input's bytes at
    /// `ctx.offset()`.
    fn record_text(&mut self, ctx: &Context, val: &str, key: bool) -> Status {
        if let Some(input) = self.input {
            let at = ctx.offset() as usize;
            assert_eq!(
                input.get(at..at + val.len()),
                Some(val.as_bytes()),
                "text is not the input at its offset"
            );
        }
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
/// too large to check. The state-machine harnesses only need a value that
/// the parser and the reference both get for the same token, so it depends
/// on the kind and the length of the text: no loop for Kani to unroll.
#[cfg(kani)]
#[allow(dead_code)] // Only named by `kani::stub`.
pub(crate) fn stub_number_value(
    kind: crate::lexer::Token,
    text: &str,
    _offset: u64,
) -> Result<crate::parser::Number, ParseError> {
    use crate::parser::Number;
    let len = text.len() as u64;
    Ok(if kind == crate::lexer::Token::Int {
        Number::Int(len as i64)
    } else {
        Number::Double(f64::from_bits(len))
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

/// One `parse` call per piece of `input`, cut at the offsets in `cuts`
/// (increasing), then `finish_parse`. Stops at the first error.
fn parse_in_calls<'i, const N: usize>(
    input: &'i [u8],
    cuts: &[usize],
) -> (Recorder<'i, N>, Result<(), ParseError>) {
    let mut recorder = Recorder::new(input, true);
    let mut parser = Parser::new(&mut recorder);
    let mut start = 0;
    let mut result = Ok(());
    for &cut in cuts.iter().chain(std::iter::once(&input.len())) {
        let end = cut.clamp(start, input.len());
        result = parser.parse(&mut &input[start..end]);
        if result.is_err() {
            break;
        }
        start = end;
    }
    let result = result.and_then(|()| parser.finish_parse());
    (recorder, result)
}

/// A document fed to `parse` in pieces gives the same events and result as
/// in one piece, wherever the cuts are, including inside a token (K3c).
fn check_split_across_parse_calls<const N: usize>(input: &[u8], cuts: &[usize]) {
    let (split, split_result) = parse_in_calls::<N>(input, cuts);
    let (whole, whole_result) = parse_in_calls::<N>(input, &[]);
    assert_eq!(split.events(), whole.events(), "cuts {:?}", cuts);
    assert!(
        same_result(&split_result, &whole_result),
        "cuts {:?}: {:?} vs {:?}",
        cuts,
        split_result,
        whole_result
    );
}

/// Not a Kani proof, for the same reason as `chunked_parse_is_invisible`;
/// `tests::exhaustive_split_across_parse_calls` is the exhaustive form.
#[test]
fn split_across_parse_calls() {
    bolero::check!()
        .with_type::<([u8; CHUNKED_PARSE_BOUND], u8, u8)>()
        .cloned()
        .for_each(|(buf, len, split)| {
            for_each_split(&buf, len, split, |input, split| {
                check_split_across_parse_calls::<CHUNKED_PARSE_BOUND>(input, &[split]);
            });
        });
}

#[test]
fn split_across_parse_calls_fuzz() {
    bolero::check!()
        .with_type::<(Vec<u8>, Vec<u8>)>()
        .for_each(|(input, sizes)| {
            let input = &input[..input.len().min(256)];
            let mut cuts = vec![];
            let mut at = 0;
            for &size in sizes {
                at += usize::from(size);
                cuts.push(at);
            }
            check_split_across_parse_calls::<256>(input, &cuts);
        });
}

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

    /// `split_across_parse_calls` over the same inputs, cut at every subset
    /// of the byte boundaries: one `parse` call per piece.
    #[test]
    fn exhaustive_split_across_parse_calls() {
        let check = |input: &[u8]| {
            let inner = input.len().saturating_sub(1);
            for mask in 0..1u32 << inner {
                let cuts: Vec<usize> = (0..inner)
                    .filter(|i| mask >> i & 1 == 1)
                    .map(|i| i + 1)
                    .collect();
                check_split_across_parse_calls::<8>(input, &cuts);
            }
        };
        crate::verify::for_each_shorter_input(crate::verify::ENUM_BYTES + 1, check);
        crate::verify::for_each_alphabet_input(crate::verify::ENUM_ALPHABET, check);
    }

    /// The plan's example: `[1` then `2]` is one number, 12.
    #[test]
    fn split_number_is_one_token() {
        let (recorder, result) = parse_in_calls::<8>(b"[12]", &[2]);
        assert_eq!(result, Ok(()));
        assert_eq!(
            recorder.events(),
            &[
                (Event::StartArray, 0),
                (Event::Int(12), 1),
                (Event::EndArray, 3)
            ]
        );
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
