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

//! A hand-written stand-in for the logos-generated `Token::lex`.
//!
//! CBMC cannot get through logos's generated automaton beyond about two
//! symbolic bytes: paths with different offsets merge, the offset becomes
//! symbolic, and every later state branches on it. So Kani harnesses that
//! lex stub `<Token as Logos>::lex` with `ref_lex`, a direct longest-match
//! scanner for the grammar in `src/lexer/token.rs`.
//!
//! That makes those proofs conditional on `ref_lex` agreeing with logos on
//! what `StreamLexer` reads: the first token's kind and span, the start of
//! the first error, and "only whitespace". The agreement is checked here
//! exhaustively over short inputs, by Kani on one byte, and by Bolero.

use crate::lexer::Token;
use logos::Logos;
use std::ops::Range;

/// The first token of `b` after whitespace: its kind (or `Err` for bytes
/// that start no token) and its span. `None` if `b` is only whitespace.
/// For an error, only the span's start is meaningful.
pub(crate) fn ref_first(b: &[u8]) -> Option<(Result<Token, ()>, Range<usize>)> {
    let start = b
        .iter()
        .position(|c| !matches!(c, b' ' | b'\t' | b'\r' | b'\n'))?;
    let rest = &b[start..];
    let (kind, len) = match rest[0] {
        b'{' => (Ok(Token::LeftBrace), 1),
        b'}' => (Ok(Token::RightBrace), 1),
        b'[' => (Ok(Token::LeftBracket), 1),
        b']' => (Ok(Token::RightBracket), 1),
        b':' => (Ok(Token::Colon), 1),
        b',' => (Ok(Token::Comma), 1),
        b't' => keyword(rest, b"true", Token::True),
        b'f' => keyword(rest, b"false", Token::False),
        b'n' => keyword(rest, b"null", Token::Null),
        b'"' => string(rest),
        b'-' | b'0'..=b'9' => number(rest),
        _ => (Err(()), 1),
    };
    Some((kind, start..start + len))
}

fn keyword(b: &[u8], word: &[u8], kind: Token) -> (Result<Token, ()>, usize) {
    if b.len() >= word.len() && &b[..word.len()] == word {
        (Ok(kind), word.len())
    } else {
        (Err(()), 1)
    }
}

/// `"([^"\\\x00-\x1F]|\\(["\\/bfnrt]|u[0-9a-fA-F]{4}))*"`, where `b[0]` is
/// the opening quote.
fn string(b: &[u8]) -> (Result<Token, ()>, usize) {
    let mut i = 1;
    while i < b.len() {
        match b[i] {
            b'"' => return (Ok(Token::String), i + 1),
            0x00..=0x1F => break,
            b'\\' => match b.get(i + 1) {
                Some(b'"' | b'\\' | b'/' | b'b' | b'f' | b'n' | b'r' | b't') => i += 2,
                Some(b'u') => {
                    let hex = b.get(i + 2..i + 6);
                    if !hex.is_some_and(|h| h.iter().all(u8::is_ascii_hexdigit)) {
                        break;
                    }
                    i += 6;
                }
                _ => break,
            },
            _ => i += 1,
        }
    }
    (Err(()), 1)
}

/// Longest prefix of `b` matching `Int` (`-?(0|[1-9][0-9]*)`, which wins
/// ties) or `Number` (`Int` with an optional fraction and exponent).
fn number(b: &[u8]) -> (Result<Token, ()>, usize) {
    #[derive(Clone, Copy)]
    enum S {
        Start,
        Minus,
        Zero,
        Int,
        Dot,
        Frac,
        E,
        ESign,
        Exp,
    }
    let mut s = S::Start;
    let mut best: (Result<Token, ()>, usize) = (Err(()), 1);
    for (i, &c) in b.iter().enumerate() {
        s = match (s, c) {
            (S::Start, b'-') => S::Minus,
            (S::Start | S::Minus, b'0') => S::Zero,
            (S::Start | S::Minus, b'1'..=b'9') => S::Int,
            (S::Int, b'0'..=b'9') => S::Int,
            (S::Zero | S::Int, b'.') => S::Dot,
            (S::Dot | S::Frac, b'0'..=b'9') => S::Frac,
            (S::Zero | S::Int | S::Frac, b'e' | b'E') => S::E,
            (S::E, b'+' | b'-') => S::ESign,
            (S::E | S::ESign | S::Exp, b'0'..=b'9') => S::Exp,
            _ => break,
        };
        match s {
            S::Zero | S::Int => best = (Ok(Token::Int), i + 1),
            S::Frac | S::Exp => best = (Ok(Token::Number), i + 1),
            _ => {}
        }
    }
    best
}

/// Replaces `<Token as Logos>::lex` under Kani. `Lexer::next` has already
/// moved the token start to the end of the previous token.
#[cfg(kani)]
#[allow(dead_code)] // Only named by `kani::stub`.
                    // The `where` clause makes `'s` early-bound, so the stub has the one
                    // generic parameter that `kani::stub` expects of `Logos<'s>::lex`.
pub(crate) fn ref_lex<'s>(lex: &mut logos::Lexer<'s, Token>) -> Option<Result<Token, ()>>
where
    's: 's,
{
    use logos::internal::LexerInternal;
    let from = lex.offset();
    let (kind, span) = match ref_first(&lex.source()[from..]) {
        Some(first) => first,
        None => {
            lex.end(lex.source().len());
            lex.trivia();
            return None;
        }
    };
    lex.end(from + span.start);
    lex.trivia();
    lex.end(from + span.end);
    Some(kind)
}

/// What `StreamLexer` reads from logos for `b`, from logos itself.
pub(crate) fn logos_first(b: &[u8]) -> Option<(Result<Token, ()>, Range<usize>)> {
    let mut lexer = Token::lexer(b);
    let first = lexer.next()?;
    Some((first, lexer.span()))
}

/// `ref_first` agrees with logos on everything `StreamLexer` uses.
pub(crate) fn check_ref_matches_logos(b: &[u8]) {
    match (ref_first(b), logos_first(b)) {
        (None, None) => {}
        (Some((Ok(a), ra)), Some((Ok(l), rl))) => {
            assert_eq!((a, ra), (l, rl), "{:?}", String::from_utf8_lossy(b));
        }
        (Some((Err(()), ra)), Some((Err(()), rl))) => {
            assert_eq!(ra.start, rl.start, "{:?}", String::from_utf8_lossy(b));
        }
        (r, l) => panic!(
            "{:?}: reference {:?}, logos {:?}",
            String::from_utf8_lossy(b),
            r,
            l
        ),
    }
}

/// Kani can only run logos itself on one byte.
#[cfg(kani)]
#[kani::proof]
#[kani::unwind(3)]
fn ref_lexer_matches_logos() {
    let b: [u8; 1] = kani::any();
    check_ref_matches_logos(&b);
}

#[test]
fn ref_lexer_matches_logos_fuzz() {
    // Raw bytes, so that the seed corpus (JSON files) is the input.
    bolero::check!().for_each(|b: &[u8]| check_ref_matches_logos(b));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every input up to two bytes, and every input up to four bytes over
    /// `ALPHABET`.
    #[test]
    fn ref_lexer_matches_logos_exhaustively() {
        crate::verify::for_each_shorter_input(3, check_ref_matches_logos);
        crate::verify::for_each_alphabet_input(4, check_ref_matches_logos);
    }

    #[test]
    fn ref_lexer_matches_logos_on_long_tokens() {
        for b in [
            &br#""\u00e9\uD83D\uDE00 caf""#[..],
            br#""\u12G4""#,
            b"-1.25e-10,",
            b"12345678901234567890",
            b"  \n\t false",
            b"nul",
            b"1.e5",
            b"0.5e+",
        ] {
            check_ref_matches_logos(b);
        }
    }
}
