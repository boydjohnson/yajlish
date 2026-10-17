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

//! Proofs about the lexer's "may this token still grow" rules and about
//! `StreamLexer` over chunked input.

#[cfg(kani)]
use super::for_each_prefix;
use super::{for_each_split, same_error, Chunks};
use crate::lexer::{Lexed, StreamLexer, DEFAULT_MAX_TOKEN_LEN};

#[cfg(kani)]
use crate::lexer::{is_number_prefix, may_extend, string_may_extend, Token};
#[cfg(kani)]
use logos::Logos;

/// Input bounds, in bytes. See "Bounds" in `plans/04-kani-bolero.md` for
/// the measured Kani times.
#[cfg(kani)]
const NUMBER_TOKEN_BOUND: usize = 4;
/// For the body and for the suffix.
#[cfg(kani)]
const STRING_PART_BOUND: usize = 2;
const STREAM_BOUND: usize = 3;

/// Up to `N` bytes: `buf[..len]` for a nondeterministic `len <= N`.
#[cfg(kani)]
fn any_prefix<const N: usize>(buf: &[u8; N]) -> &[u8] {
    let len: usize = kani::any_where(|&len| len <= N);
    &buf[..len]
}

#[cfg(kani)]
#[kani::proof]
#[kani::unwind(10)]
fn number_prefix_is_prefix_closed() {
    let buf: [u8; 8] = kani::any();
    let b = any_prefix(&buf);
    let k: usize = kani::any_where(|&k| k <= b.len());
    if is_number_prefix(b) {
        assert!(is_number_prefix(&b[..k]));
    }
}

/// `is_number_prefix`'s doc comment: every number prefix can be extended
/// by one more byte. One of `0.e+-` always works, so no ∃ is needed.
#[cfg(kani)]
#[kani::proof]
#[kani::unwind(10)]
fn number_prefix_always_extends() {
    let buf: [u8; 8] = kani::any();
    let b = any_prefix(&buf);
    if is_number_prefix(b) {
        let mut ext = [0u8; 9];
        ext[..b.len()].copy_from_slice(b);
        let extends = b"0.e+-".iter().any(|&c| {
            ext[b.len()] = c;
            is_number_prefix(&ext[..=b.len()])
        });
        assert!(extends);
    }
}

/// Ties the hand-written DFA to the logos regex: a complete number token
/// is a number prefix.
#[cfg(kani)]
#[kani::proof]
#[kani::unwind(6)]
#[kani::stub(<Token as Logos>::lex, super::ref_lexer::ref_lex)]
fn complete_number_is_prefix() {
    let buf: [u8; NUMBER_TOKEN_BOUND] = kani::any();
    // A token spanning all of `b` starts at `b[0]`, and only these bytes
    // start a number (in `ref_first`, which the stub runs). Saves Kani
    // from exploring the string scanner.
    kani::assume(matches!(buf[0], b'-' | b'0'..=b'9'));
    for_each_prefix(&buf, |b| {
        let mut lexer = Token::lexer(b);
        if let Some(Ok(Token::Int | Token::Number)) = lexer.next() {
            if lexer.span() == (0..b.len()) {
                assert!(is_number_prefix(b));
            }
        }
    });
}

/// If `"` + body + a non-empty suffix lexes as one string token, the
/// stream lexer must not give up on `"` + body at a chunk boundary.
#[cfg(kani)]
#[kani::proof]
#[kani::unwind(7)]
#[kani::stub(<Token as Logos>::lex, super::ref_lexer::ref_lex)]
fn string_may_extend_is_sound() {
    let buf: [u8; 1 + 2 * STRING_PART_BOUND] = kani::any();
    let body_len: usize = kani::any_where(|&n| n <= STRING_PART_BOUND);
    let len: usize = kani::any_where(|&n| n > body_len && n <= body_len + STRING_PART_BOUND);
    kani::assume(buf[0] == b'"');
    let input = &buf[..1 + len];
    let mut lexer = Token::lexer(input);
    if let Some(Ok(Token::String)) = lexer.next() {
        if lexer.span() == (0..input.len()) {
            assert!(string_may_extend(&input[1..1 + body_len]));
        }
    }
}

/// Covers the `\u` slicing in `string_may_extend` for every truncation.
#[cfg(kani)]
#[kani::proof]
#[kani::unwind(10)]
fn string_may_extend_never_panics() {
    let buf: [u8; 8] = kani::any();
    let _ = string_may_extend(any_prefix(&buf));
}

/// On keyword prefixes, `may_extend` is "a proper prefix of the keyword".
#[cfg(kani)]
#[kani::proof]
#[kani::unwind(8)]
fn keyword_may_extend() {
    let buf: [u8; 6] = kani::any();
    let b = any_prefix(&buf);
    let keyword: &[u8] = match b.first() {
        Some(b't') => b"true",
        Some(b'f') => b"false",
        Some(b'n') => b"null",
        _ => return,
    };
    let proper_prefix = b.len() < keyword.len() && b.iter().zip(keyword).all(|(x, y)| x == y);
    assert_eq!(may_extend(b), proper_prefix);
}

/// No token longer than `max_token_len` is returned, chunked or not.
#[cfg(kani)]
#[kani::proof]
#[kani::unwind(5)]
#[kani::stub(std::fmt::format, super::stub_format)]
#[kani::stub(<crate::lexer::Token as logos::Logos>::lex, super::ref_lexer::ref_lex)]
fn max_token_len_is_enforced() {
    let buf: [u8; STREAM_BOUND] = kani::any();
    let max: usize = kani::any_where(|&m| m <= 8);
    for_each_split(&buf, 0, 0, |input, split| {
        let split = [split];
        let mut reader = Chunks::new(input, &split);
        let mut lexer = StreamLexer::new(&mut reader, max, 0);
        for _ in 0..=input.len() {
            match lexer.next() {
                Ok(Lexed::Token { bytes, .. }) => {
                    assert!(bytes.len() <= max);
                }
                Ok(Lexed::Eof) | Err(_) => return,
            }
        }
        unreachable!("more tokens than bytes");
    });
}

/// Token offsets strictly increase, and each is where the token's bytes
/// are in the input.
fn check_offsets(input: &[u8], sizes: &[usize]) {
    let mut reader = Chunks::new(input, sizes);
    let mut lexer = StreamLexer::new(&mut reader, DEFAULT_MAX_TOKEN_LEN, 0);
    let mut next_free = 0;
    for _ in 0..=input.len() {
        match lexer.next() {
            Ok(Lexed::Token { bytes, offset, .. }) => {
                let at = offset as usize;
                assert!(at >= next_free);
                assert!(!bytes.is_empty());
                assert_eq!(input.get(at..at + bytes.len()), Some(bytes));
                next_free = at + bytes.len();
            }
            Ok(Lexed::Eof) | Err(_) => return,
        }
    }
    unreachable!("more tokens than bytes");
}

#[test]
#[cfg_attr(
    kani,
    kani::proof,
    kani::unwind(5),
    kani::stub(std::fmt::format, super::stub_format),
    kani::stub(<crate::lexer::Token as logos::Logos>::lex, super::ref_lexer::ref_lex)
)]
fn offsets_are_monotone() {
    bolero::check!()
        .with_type::<([u8; STREAM_BOUND], u8, u8)>()
        .cloned()
        .for_each(|(buf, len, split)| {
            for_each_split(&buf, len, split, |input, split| {
                check_offsets(input, &[split]);
            });
        });
}

#[test]
fn offsets_are_monotone_fuzz() {
    bolero::check!()
        .with_type::<(Vec<u8>, Vec<u8>)>()
        .for_each(|(input, sizes)| {
            let sizes: Vec<usize> = sizes.iter().map(|&s| usize::from(s)).collect();
            check_offsets(input, &sizes);
        });
}

/// Lex `input` read in chunks of `sizes` and in one chunk, in lockstep:
/// the same tokens with the same offsets, and the same error at the same
/// offset.
fn check_chunking_is_invisible(input: &[u8], sizes: &[usize]) {
    let mut chunked = Chunks::new(input, sizes);
    let mut chunked = StreamLexer::new(&mut chunked, DEFAULT_MAX_TOKEN_LEN, 0);
    let mut whole = Chunks::new(input, &[]);
    let mut whole = StreamLexer::new(&mut whole, DEFAULT_MAX_TOKEN_LEN, 0);
    for _ in 0..=input.len() {
        match (chunked.next(), whole.next()) {
            (Ok(Lexed::Eof), Ok(Lexed::Eof)) => return,
            (Ok(a), Ok(b)) => {
                assert_eq!(a, b);
            }
            (Err(a), Err(b)) => {
                assert!(same_error(&a, &b), "{:?} vs {:?}", a, b);
                return;
            }
            (a, b) => panic!("{:?} vs {:?}", a, b),
        }
    }
    unreachable!("more tokens than bytes");
}

/// The exhaustive form of `test_every_chunk_size_matches_unchunked`: any
/// input up to 6 bytes, split anywhere.
#[test]
#[cfg_attr(
    kani,
    kani::proof,
    kani::unwind(5),
    kani::stub(std::fmt::format, super::stub_format),
    kani::stub(<crate::lexer::Token as logos::Logos>::lex, super::ref_lexer::ref_lex)
)]
fn chunking_is_invisible() {
    bolero::check!()
        .with_type::<([u8; STREAM_BOUND], u8, u8)>()
        .cloned()
        .for_each(|(buf, len, split)| {
            for_each_split(&buf, len, split, |input, split| {
                check_chunking_is_invisible(input, &[split]);
            });
        });
}

#[test]
fn chunking_is_invisible_fuzz() {
    bolero::check!()
        .with_type::<(Vec<u8>, Vec<u8>)>()
        .for_each(|(input, sizes)| {
            let sizes: Vec<usize> = sizes.iter().map(|&s| usize::from(s)).collect();
            check_chunking_is_invisible(input, &sizes);
        });
}
