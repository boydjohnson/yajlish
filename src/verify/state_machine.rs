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

//! Proofs about the parser's state machine, `Parser::parse_source`.
//!
//! Each property is a `check_*` function over a `Case`: a token sequence
//! and how it ends. Kani builds the sequence from nondeterministic token
//! kinds (`kani_case`); Bolero and the exhaustive test lex real bytes
//! (`bytes_case`), so they also run logos and the real `number_value`.

use super::{
    parser::{Event, Recorder},
    reference,
    tokens::{lex_all, SeqToken, TokenSeq},
};
use crate::{lexer::Token, parser::ParseError, Enclosing, Parser};

/// A token sequence for the parser and the reference to consume.
pub(crate) struct Case<'a> {
    tokens: &'a [SeqToken<'a>],
    /// `Ok(())` if the sequence ends at `Eof`, else the lexer's error.
    end: &'a Result<(), ParseError>,
    /// The bytes the tokens were lexed from, if any.
    input: Option<&'a [u8]>,
}

impl<'a> Case<'a> {
    fn seq(&self) -> TokenSeq<'a> {
        TokenSeq::new(self.tokens, self.end)
    }

    fn recorder<const N: usize>(&self, strict: bool) -> Recorder<'a, N> {
        match self.input {
            Some(input) => Recorder::new(input, strict),
            None => Recorder::for_tokens(self.tokens.len() as u64, strict),
        }
    }

    /// `parse_source` then `finish_parse`'s checks. The sequence is the
    /// whole stream, so `finish_parse` would have nothing left to flush.
    fn parse<const N: usize>(&self, recorder: Recorder<'a, N>) -> (Recorder<'a, N>, bool) {
        let mut recorder = recorder;
        let mut parser = Parser::new(&mut recorder);
        let ok = parser
            .parse_source(&mut self.seq())
            .and_then(|()| parser.finish_checks())
            .is_ok();
        (recorder, ok)
    }

    /// Whether the reference accepts, and the events it emits.
    fn reference<const N: usize>(&self) -> (bool, [(Event, u64); N], usize) {
        let mut stack = [Enclosing::LeftBrace; N];
        let mut events = [(Event::Null, 0); N];
        let mut len = 0;
        let accepted = reference::accepts(&mut self.seq(), &mut stack, |event, offset| {
            events[len] = (event, offset);
            len += 1;
        });
        (accepted, events, len)
    }
}

/// `parse` then `finish_parse` never panics. Includes the
/// `dec_braces`/`dec_brackets` subtractions, which must never underflow.
fn check_never_panics<const N: usize>(case: &Case<'_>) {
    let _ = case.parse::<N>(case.recorder(false));
}

/// The `Recorder`'s strict assertions hold on every callback.
fn check_context_invariants<const N: usize>(case: &Case<'_>) {
    let _ = case.parse::<N>(case.recorder(true));
}

/// After a callback returns `Abort`, no callback follows (the `Recorder`
/// asserts that) and `parse_source` returns `Ok`. Holds because the parser
/// checks each token before calling the handler (K3b), so a handler never
/// sees a token that is then rejected.
fn check_no_callback_after_abort<const N: usize>(case: &Case<'_>, abort_at: usize) {
    let mut recorder = case.recorder::<N>(false).abort_at(abort_at);
    let mut parser = Parser::new(&mut recorder);
    let result = parser.parse_source(&mut case.seq());
    if recorder.events().len() > abort_at {
        assert!(result.is_ok(), "Abort must end the parse with Ok");
    }
}

/// Accepted by the parser ⇒ accepted by the reference.
fn check_accepts_only_valid_json<const N: usize>(case: &Case<'_>) {
    let (_, parsed) = case.parse::<N>(case.recorder(false));
    if parsed {
        assert!(case.reference::<N>().0, "parser accepted invalid JSON");
    }
}

/// Accepted by the reference ⇒ accepted by the parser.
fn check_accepts_all_valid_json<const N: usize>(case: &Case<'_>) {
    if case.reference::<N>().0 {
        let (_, parsed) = case.parse::<N>(case.recorder(false));
        assert!(parsed, "parser rejected valid JSON");
    }
}

/// On input the reference accepts, the parser delivers the same events.
fn check_events_match_reference<const N: usize>(case: &Case<'_>) {
    let (accepted, events, len) = case.reference::<N>();
    if accepted {
        let (recorder, _) = case.parse::<N>(case.recorder(false));
        assert_eq!(recorder.events(), &events[..len]);
    }
}

/// Stand-ins for `Context`'s stack under Kani. `Context` keeps it in a
/// `VecDeque`, whose ring-buffer indices become symbolic as paths merge,
/// and CBMC's array encoding of that does not finish even for two tokens.
/// These keep the same stack in a fixed array instead. A harness run has
/// exactly one `Context`, so one static stack is enough; everything that
/// reads the stack (the parser, `Recorder`) goes through these methods.
#[cfg(kani)]
#[allow(dead_code)] // Only named by `kani::stub`.
mod context_stack {
    use crate::{Context, Enclosing};

    static mut STACK: [Enclosing; super::TOKEN_BOUND] = [Enclosing::LeftBrace; super::TOKEN_BOUND];
    static mut DEPTH: usize = 0;

    pub(super) fn add_enclosing(_ctx: &mut Context, enclosing: Enclosing) {
        // SAFETY: Kani runs one harness, single-threaded.
        unsafe {
            STACK[DEPTH] = enclosing;
            DEPTH += 1;
        }
    }

    pub(super) fn last_enclosing(_ctx: &Context) -> Option<Enclosing> {
        // SAFETY: as above.
        unsafe { DEPTH.checked_sub(1).map(|i| STACK[i]) }
    }

    pub(super) fn remove_last_enclosing(_ctx: &mut Context) -> Option<Enclosing> {
        // SAFETY: as above.
        unsafe {
            let last = DEPTH.checked_sub(1).map(|i| STACK[i]);
            DEPTH = DEPTH.saturating_sub(1);
            last
        }
    }
}

/// Kani: number of tokens. See "Bounds" in `plans/04-kani-bolero.md`.
#[cfg(kani)]
const TOKEN_BOUND: usize = 3;

/// Calls `f` on any sequence of up to `TOKEN_BOUND` tokens, ending at
/// `Eof` or at a lexer error. The tokens have their `canonical` bytes, all
/// ASCII, which is what makes the `from_utf8` stubs sound here.
#[cfg(kani)]
fn kani_case(f: impl FnOnce(&Case<'_>)) {
    use super::tokens::{canonical_tokens, kind_from_byte};
    let mut kinds = [Token::Null; TOKEN_BOUND];
    for kind in kinds.iter_mut() {
        *kind = kind_from_byte(kani::any_where(|&b: &u8| b < 12));
    }
    let tokens = canonical_tokens(&kinds);
    let len: usize = kani::any_where(|&len| len <= TOKEN_BOUND);
    let end = if kani::any() {
        Ok(())
    } else {
        Err(ParseError::malformed(len as u64, "lexer error"))
    };
    f(&Case {
        tokens: &tokens[..len],
        end: &end,
        input: None,
    });
}

/// Bolero and the exhaustive tests: the tokens of `input`, from the real
/// lexer, ending where it stopped.
fn bytes_case(input: &[u8], f: impl FnOnce(&Case<'_>)) {
    let (tokens, end) = lex_all(input);
    f(&Case {
        tokens: &tokens,
        end: &end,
        input: Some(input),
    });
}

/// Bolero: input bytes.
#[cfg(not(kani))]
const BYTES_BOUND: usize = 8;
/// Bolero fuzz versions: longer inputs, cut to this.
const FUZZ_BOUND: usize = 256;

/// The Kani proof and the Bolero test of one check: under Kani, any token
/// sequence up to `TOKEN_BOUND`; otherwise up to `BYTES_BOUND` random bytes
/// through the real lexer, and a fuzz version on longer input.
macro_rules! harness {
    ($name:ident, $fuzz:ident, $check:ident $(, $attr:meta)*) => {
        #[test]
        #[cfg_attr(
            kani,
            kani::proof,
            kani::unwind(5),
            kani::stub(crate::parser::number_value, super::parser::stub_number_value),
            kani::stub(std::fmt::format, super::stub_format),
    kani::stub(std::str::from_utf8, super::stub_from_utf8_ascii),
    kani::stub(super::as_utf8, super::stub_as_utf8_ascii),
    kani::stub(crate::Context::add_enclosing, context_stack::add_enclosing),
    kani::stub(crate::Context::last_enclosing, context_stack::last_enclosing),
    kani::stub(crate::Context::remove_last_enclosing, context_stack::remove_last_enclosing)
            $(, $attr)*
        )]
        fn $name() {
            #[cfg(kani)]
            kani_case($check::<TOKEN_BOUND>);
            #[cfg(not(kani))]
            bolero::check!()
                .with_type::<([u8; BYTES_BOUND], u8)>()
                .cloned()
                .for_each(|(buf, len)| {
                    let input = &buf[..usize::from(len) % (BYTES_BOUND + 1)];
                    bytes_case(input, $check::<BYTES_BOUND>);
                });
        }

        #[test]
        fn $fuzz() {
            // Raw bytes, so that the seed corpus (JSON files) is the input.
            bolero::check!().for_each(|input: &[u8]| {
                let input = &input[..input.len().min(FUZZ_BOUND)];
                bytes_case(input, $check::<FUZZ_BOUND>);
            });
        }
    };
}

harness!(
    parser_never_panics,
    parser_never_panics_fuzz,
    check_never_panics
);
harness!(
    accepts_all_valid_json,
    accepts_all_valid_json_fuzz,
    check_accepts_all_valid_json
);
harness!(
    events_match_reference,
    events_match_reference_fuzz,
    check_events_match_reference
);

// These two failed until K3b: the parser accepted invalid token sequences
// (`tests::rejects_*`) and, on `[}`, called `handle_end_map` and carried on
// (`tests::invariants_after_mismatched_brace`). Both were `should_panic`.
harness!(
    context_invariants_hold,
    context_invariants_hold_fuzz,
    check_context_invariants
);
harness!(
    accepts_only_valid_json,
    accepts_only_valid_json_fuzz,
    check_accepts_only_valid_json
);

#[cfg(kani)]
#[kani::proof]
#[kani::unwind(5)]
#[kani::stub(crate::parser::number_value, super::parser::stub_number_value)]
#[kani::stub(std::fmt::format, super::stub_format)]
#[kani::stub(std::str::from_utf8, super::stub_from_utf8_ascii)]
#[kani::stub(super::as_utf8, super::stub_as_utf8_ascii)]
#[kani::stub(crate::Context::add_enclosing, context_stack::add_enclosing)]
#[kani::stub(crate::Context::last_enclosing, context_stack::last_enclosing)]
#[kani::stub(
    crate::Context::remove_last_enclosing,
    context_stack::remove_last_enclosing
)]
fn no_callback_after_abort() {
    let abort_at: usize = kani::any_where(|&i| i < TOKEN_BOUND);
    kani_case(|case| check_no_callback_after_abort::<TOKEN_BOUND>(case, abort_at));
}

#[test]
fn no_callback_after_abort_fuzz() {
    bolero::check!()
        .with_type::<(Vec<u8>, u8)>()
        .for_each(|(input, abort_at)| {
            let input = &input[..input.len().min(FUZZ_BOUND)];
            bytes_case(input, |case| {
                check_no_callback_after_abort::<FUZZ_BOUND>(case, usize::from(*abort_at));
            });
        });
}

/// Appends a JSON value chosen by `choices` to `out`, with whitespace
/// between tokens. Nesting stops at depth 4 and output at about 512 bytes.
fn gen_value(choices: &mut impl Iterator<Item = u8>, depth: u32, out: &mut Vec<u8>) {
    let c = choices.next().unwrap_or(0);
    let ws: &[u8] = match c >> 6 {
        0 => b"",
        1 => b" ",
        2 => b"\n\t",
        _ => b"\r\n  ",
    };
    out.extend_from_slice(ws);
    let container = depth < 4 && out.len() < 512;
    match c % 8 {
        6 if container => {
            out.push(b'[');
            for i in 0..choices.next().unwrap_or(0) % 4 {
                if i > 0 {
                    out.push(b',');
                }
                gen_value(choices, depth + 1, out);
            }
            out.extend_from_slice(ws);
            out.push(b']');
        }
        7 if container => {
            out.push(b'{');
            for i in 0..choices.next().unwrap_or(0) % 4 {
                if i > 0 {
                    out.push(b',');
                }
                out.extend_from_slice(ws);
                out.extend_from_slice(if i % 2 == 0 { b"\"k\"" } else { b"\"\\u00e9\"" });
                out.extend_from_slice(ws);
                out.push(b':');
                gen_value(choices, depth + 1, out);
            }
            out.extend_from_slice(ws);
            out.push(b'}');
        }
        0 => out.extend_from_slice(b"null"),
        1 => out.extend_from_slice(b"true"),
        2 => out.extend_from_slice(b"false"),
        3 => out.extend_from_slice(b"-12"),
        4 => out.extend_from_slice(b"1.5e3"),
        _ => out.extend_from_slice(b"\"a\\n\xc3\xa9\""),
    }
    out.extend_from_slice(ws);
}

/// The plan's generator form of `accepts_all_valid_json` and
/// `events_match_reference`: random valid documents with random
/// whitespace. Also checks that the reference accepts them.
#[test]
fn generated_json_is_accepted() {
    bolero::check!().with_type::<Vec<u8>>().for_each(|choices| {
        let mut input = vec![];
        gen_value(&mut choices.iter().copied(), 0, &mut input);
        bytes_case(&input, |case| {
            assert!(
                case.reference::<1024>().0,
                "{}",
                String::from_utf8_lossy(&input)
            );
            check_accepts_all_valid_json::<1024>(case);
            check_events_match_reference::<1024>(case);
            check_context_invariants::<1024>(case);
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The parser rejects `input`. Each test below is one class of
    /// counterexample to `accepts_only_valid_json` that K3 found, one per
    /// cause, with the JSONTestSuite files it explains (they were on
    /// `N_ALLOWLIST` until K3b fixed them).
    fn rejects(input: &[u8]) {
        bytes_case(input, check_accepts_only_valid_json::<16>);
    }

    /// `n_structure_no_data`, `n_single_space`.
    #[test]
    fn rejects_empty_document() {
        rejects(b"");
        rejects(b" ");
    }

    /// A `}` that closes nothing flags `LexicalError`, and the status is
    /// then overwritten. `n_structure_object_followed_by_closing_object`.
    #[test]
    fn rejects_unmatched_closing_brace() {
        rejects(b"}");
        rejects(b"{}}");
        rejects(b"[}]");
    }

    /// `:` anywhere but after a key is ignored. `n_object_double_colon`,
    /// `n_object_missing_key`.
    #[test]
    fn rejects_misplaced_colon() {
        rejects(b":");
        rejects(br#"{"a"::1}"#);
        rejects(br#"{:"b"}"#);
    }

    /// `[` and `{` do not check the status, so a value can follow a value.
    /// `n_structure_double_array`, `n_array_inner_array_no_comma`.
    #[test]
    fn rejects_value_after_value() {
        rejects(b"1[]");
        rejects(b"[][]");
        rejects(b"[3[4]]");
    }

    /// `n_array_extra_comma`, `n_array_number_and_comma`,
    /// `n_object_trailing_comma`.
    #[test]
    fn rejects_trailing_comma() {
        rejects(b"[1,]");
        rejects(br#"{"a":1,}"#);
    }

    /// A key with no `:` or value. `n_object_with_single_string`.
    #[test]
    fn rejects_key_without_value() {
        rejects(br#"{"a"}"#);
    }

    /// `context_invariants_hold`'s counterexample.
    #[test]
    fn invariants_after_mismatched_brace() {
        bytes_case(b"[}]", check_context_invariants::<8>);
    }

    /// A Kani-style case: `kinds` with their canonical bytes, ending at
    /// `Eof` (`eof`) or at a lexer error.
    fn token_case(kinds: &[Token], eof: bool, f: impl FnOnce(&Case<'_>)) {
        use super::super::tokens::canonical;
        let tokens: Vec<SeqToken<'_>> = kinds
            .iter()
            .enumerate()
            .map(|(i, &kind)| (kind, canonical(kind), i as u64))
            .collect();
        let end = if eof {
            Ok(())
        } else {
            Err(ParseError::malformed(kinds.len() as u64, "lexer error"))
        };
        f(&Case {
            tokens: &tokens,
            end: &end,
            input: None,
        });
    }

    /// Kani's counterexample to `accepts_only_valid_json`, from
    /// `--concrete-playback`: no tokens, then `Eof`.
    #[test]
    fn kani_playback_accepts_only_valid_json() {
        token_case(&[], true, check_accepts_only_valid_json::<4>);
    }

    /// Kani's counterexample to `context_invariants_hold`, from
    /// `--concrete-playback`: `[ "s" } {`, then `Eof`.
    #[test]
    fn kani_playback_context_invariants_hold() {
        token_case(
            &[
                Token::LeftBracket,
                Token::String,
                Token::RightBrace,
                Token::LeftBrace,
            ],
            true,
            check_context_invariants::<4>,
        );
    }

    /// The space the Kani proofs search, enumerated and extended: every
    /// sequence of up to `ENUM_TOKENS` canonical tokens (Kani stops at
    /// `TOKEN_BOUND`), ending at `Eof` or an error, with the real
    /// `number_value` and UTF-8 checks instead of Kani's stubs.
    #[test]
    fn exhaustive_token_sequences() {
        const ENUM_TOKENS: u32 = 6;
        use super::super::tokens::kind_from_byte;
        for len in 0..=ENUM_TOKENS {
            for code in 0..12usize.pow(len) {
                let kinds: Vec<Token> = (0..len)
                    .map(|i| kind_from_byte((code / 12usize.pow(i) % 12) as u8))
                    .collect();
                for eof in [true, false] {
                    token_case(&kinds, eof, |case| {
                        check_never_panics::<6>(case);
                        check_context_invariants::<6>(case);
                        check_accepts_only_valid_json::<6>(case);
                        check_accepts_all_valid_json::<6>(case);
                        check_events_match_reference::<6>(case);
                        for abort_at in 0..kinds.len() {
                            check_no_callback_after_abort::<6>(case, abort_at);
                        }
                    });
                }
            }
        }
    }

    /// Every check, over every byte string up to
    /// `ENUM_BYTES` and every string over `ALPHABET` up to `ENUM_ALPHABET`.
    #[test]
    fn exhaustive_short_inputs() {
        let check = |input: &[u8]| {
            bytes_case(input, |case| {
                check_never_panics::<8>(case);
                check_context_invariants::<8>(case);
                check_accepts_only_valid_json::<8>(case);
                check_accepts_all_valid_json::<8>(case);
                check_events_match_reference::<8>(case);
                for abort_at in 0..input.len() {
                    check_no_callback_after_abort::<8>(case, abort_at);
                }
            });
        };
        crate::verify::for_each_shorter_input(crate::verify::ENUM_BYTES + 1, check);
        crate::verify::for_each_alphabet_input(crate::verify::ENUM_ALPHABET, check);
    }
}
