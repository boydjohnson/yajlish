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

//! Proofs about `decode_str` and `hex4`.

use super::{as_utf8, for_each_input};
use crate::{
    decode::{decode_str, DecodeError},
    lexer::Token,
};
use logos::Logos;

#[cfg(kani)]
use crate::decode::hex4;

/// Input bound for `decode_str`, in bytes, quotes included. See "Bounds"
/// in `plans/04-kani-bolero.md` for the measured Kani times.
const DECODE_BOUND: usize = 4;

#[cfg(kani)]
#[kani::proof]
#[kani::unwind(6)]
fn hex4_matches_from_str_radix() {
    let b: [u8; 4] = kani::any();
    let got = hex4(&b, 0);
    if b.iter().all(u8::is_ascii_hexdigit) {
        let text = std::str::from_utf8(&b).unwrap();
        assert_eq!(got, u16::from_str_radix(text, 16).ok());
        assert!(got.is_some());
    } else {
        assert_eq!(got, None);
    }
}

/// Any `&str` up to `DECODE_BOUND` bytes. Covers the `&body[run..i]`
/// slices. Skipping non-UTF-8 input is the plan's
/// `kani::assume(from_utf8(..).is_ok())`.
#[test]
#[cfg_attr(kani, kani::proof, kani::unwind(6))]
fn decode_never_panics() {
    bolero::check!()
        .with_type::<([u8; DECODE_BOUND], u8)>()
        .cloned()
        .for_each(|(buf, len)| {
            for_each_input(&buf, len, |b| {
                if let Some(raw) = as_utf8(b) {
                    let _ = decode_str(raw);
                }
            });
        });
}

/// The `ok_or(LoneSurrogate)` after joining a surrogate pair is dead code:
/// every high/low pair is a scalar value. Running `decode_str` on the pair
/// is out of Kani's reach (14 bytes), so `every_surrogate_pair_decodes`
/// enumerates all pairs instead.
#[cfg(kani)]
#[kani::proof]
fn surrogate_pair_is_always_scalar() {
    let high: u16 = kani::any_where(|u| (0xD800..=0xDBFF).contains(u));
    let low: u16 = kani::any_where(|u| (0xDC00..=0xDFFF).contains(u));
    let scalar = 0x10000 + ((u32::from(high) - 0xD800) << 10) + (u32::from(low) - 0xDC00);
    let c = char::from_u32(scalar);
    kani::cover!(c.is_none(), "a surrogate pair that is not a scalar value");
    assert!(c.is_some());
}

/// `decode_str` on every `"\uXXXX"` and on every high surrogate followed
/// by every low one agrees with `char::decode_utf16`.
#[test]
fn every_surrogate_pair_decodes() {
    let check = |units: &[u16]| {
        let mut raw = String::from("\"");
        for unit in units {
            raw.push_str(&format!("\\u{:04x}", unit));
        }
        raw.push('"');
        let want: Result<String, _> = char::decode_utf16(units.iter().copied()).collect();
        match (decode_str(&raw), want) {
            (Ok(got), Ok(want)) => assert_eq!(got, want),
            (Err(DecodeError::LoneSurrogate { .. }), Err(_)) => {}
            (got, want) => panic!("{}: {:?} vs {:?}", raw, got, want),
        }
    };
    for unit in 0..=u16::MAX {
        check(&[unit]);
    }
    for high in 0xD800..=0xDBFF {
        for low in 0xDC00..=0xDFFF {
            check(&[high, low]);
        }
        check(&[high, 0x0041]);
        check(&[high, 0xD800]);
    }
}

/// If bytes lex as one `String` token and are UTF-8, `decode_str` only
/// fails on lone surrogates, which the JSON grammar allows.
fn check_lexer_and_decoder_agree(raw: &[u8]) {
    let mut lexer = Token::lexer(raw);
    if lexer.next() != Some(Ok(Token::String)) || lexer.span() != (0..raw.len()) {
        return;
    }
    let Some(raw) = as_utf8(raw) else {
        return;
    };
    match decode_str(raw) {
        Ok(_) | Err(DecodeError::LoneSurrogate { .. }) => {}
        Err(e) => panic!("{:?} lexes as a string but decodes to {:?}", raw, e),
    }
}

#[test]
#[cfg_attr(
    kani,
    kani::proof,
    kani::unwind(6),
    kani::stub(<crate::lexer::Token as logos::Logos>::lex, super::ref_lexer::ref_lex)
)]
fn lexer_and_decoder_agree() {
    bolero::check!()
        .with_type::<([u8; DECODE_BOUND], u8)>()
        .cloned()
        .for_each(|(buf, len)| {
            // Under Kani the input is all of `buf`, and a string token
            // spanning it starts and ends with a quote. Saves exploring
            // every other first and last byte.
            #[cfg(kani)]
            kani::assume(buf[0] == b'"' && buf[DECODE_BOUND - 1] == b'"');
            for_each_input(&buf, len, check_lexer_and_decoder_agree);
        });
}

/// Pieces that fuzz inputs are built from, so random input is mostly
/// near-valid string bodies rather than noise.
const FRAGMENTS: &[&str] = &[
    "a",
    "\\",
    "\"",
    "u",
    "0",
    "1",
    "8",
    "9",
    "a",
    "b",
    "c",
    "d",
    "D",
    "e",
    "f",
    "F",
    "n",
    "t",
    "/",
    "x",
    " ",
    "\u{1}",
    "\u{1f}",
    "\u{7f}",
    "\u{e9}",
    "\u{1F600}",
    "\\u",
    "\\ud83d",
    "\\ude00",
    "\\\\",
    "\\\"",
];

fn fragments_to_raw(picks: &[u8]) -> String {
    let mut raw = String::from("\"");
    for &p in picks {
        raw.push_str(FRAGMENTS[usize::from(p) % FRAGMENTS.len()]);
    }
    raw.push('"');
    raw
}

#[test]
fn lexer_and_decoder_agree_fuzz() {
    bolero::check!().with_type::<Vec<u8>>().for_each(|picks| {
        check_lexer_and_decoder_agree(fragments_to_raw(picks).as_bytes());
    });
}

/// Fuzz-only: `decode_str` agrees with `serde_json`, which also rejects
/// lone surrogates.
#[test]
fn decode_matches_serde_json() {
    bolero::check!().with_type::<Vec<u8>>().for_each(|picks| {
        let raw = fragments_to_raw(picks);
        let ours = decode_str(&raw);
        let theirs = serde_json::from_str::<String>(&raw);
        match (&ours, &theirs) {
            (Ok(a), Ok(b)) => {
                assert_eq!(a, b, "{:?}", raw);
            }
            (Err(_), Err(_)) => {}
            _ => panic!("{:?}: ours {:?}, serde_json {:?}", raw, ours, theirs),
        }
    });
}

/// `Cow::Borrowed` exactly when the body has no backslash.
#[cfg(kani)]
#[kani::proof]
#[kani::unwind(6)]
fn borrowed_iff_no_escape() {
    let buf: [u8; DECODE_BOUND] = kani::any();
    for_each_input(&buf, 0, check_borrowed_iff_no_escape);
}

/// `Cow::Borrowed` exactly when the body has no backslash.
fn check_borrowed_iff_no_escape(b: &[u8]) {
    let Some(raw) = as_utf8(b) else {
        return;
    };
    if let Ok(value) = decode_str(raw) {
        let body = &b[1..b.len() - 1];
        let borrowed = matches!(value, std::borrow::Cow::Borrowed(_));
        assert_eq!(borrowed, !body.contains(&b'\\'));
    }
}

/// The inputs Kani leaves to enumeration: every byte string shorter than
/// `DECODE_BOUND`.
#[test]
fn exhaustive_short_decode_inputs() {
    super::for_each_shorter_input(DECODE_BOUND, |b| {
        if let Some(raw) = as_utf8(b) {
            let _ = decode_str(raw);
        }
        check_borrowed_iff_no_escape(b);
        check_lexer_and_decoder_agree(b);
    });
}
