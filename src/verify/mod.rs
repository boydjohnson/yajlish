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

//! Bounded proofs and property tests, see `plans/04-kani-bolero.md`.
//!
//! Harnesses written as `#[test]` + `bolero::check!()` run three ways: as
//! ordinary tests with random inputs (`cargo test`), under a fuzzer
//! (`cargo bolero test`), and as Kani proofs (`cargo kani --tests`). Kani
//! needs fixed-size inputs, so shared harnesses take a fixed array plus a
//! length. Harnesses that only make sense under Kani are `#[cfg(kani)]`.

// `cargo kani --tests` also builds this crate without `cfg(test)` (as a
// dependency of the integration tests), where the `#[test]` harnesses and
// so their helpers and the imports they use are gone.
#![cfg_attr(not(test), allow(dead_code, unused_imports))]

mod decode;
mod lexer;
mod parser;
mod ref_lexer;
mod reference;
mod state_machine;
mod tokens;

use crate::parser::ParseError;
use std::io::{BufRead, Read};

/// Calls `f` on every prefix of `buf`, shortest first.
///
/// Under Kani a concrete length beats one nondeterministic length by
/// orders of magnitude: with a symbolic length, every loop over the input
/// (logos's states, UTF-8 validation, `decode_str`) is unrolled to the
/// unwind bound on every path.
#[cfg(kani)]
pub(crate) fn for_each_prefix<const N: usize>(buf: &[u8; N], mut f: impl FnMut(&[u8])) {
    for len in 0..=N {
        f(&buf[..len]);
    }
}

/// The input of a shared harness. Under Kani, all of `buf`: exactly `N`
/// symbolic bytes. Shorter inputs are enumerated by the `exhaustive_*`
/// unit tests, which is far cheaper than asking Kani for them too, so
/// together the two cover every input up to `N` bytes. Otherwise the one
/// prefix of length `len % (N + 1)` that Bolero picked.
pub(crate) fn for_each_input<const N: usize>(buf: &[u8; N], len: u8, mut f: impl FnMut(&[u8])) {
    if cfg!(kani) {
        f(&buf[..]);
    } else {
        f(&buf[..usize::from(len) % (N + 1)]);
    }
}

/// The input and split point of a Bolero harness: the prefix of length
/// `len % (N + 1)`, split at `split % (input.len() + 1)`.
pub(crate) fn for_each_split<const N: usize>(
    buf: &[u8; N],
    len: u8,
    split: u8,
    mut f: impl FnMut(&[u8], usize),
) {
    let input = &buf[..usize::from(len) % (N + 1)];
    f(input, usize::from(split) % (input.len() + 1));
}

/// Calls `f` on every byte string shorter than `n` bytes, whatever the
/// bytes: the inputs that `for_each_input` leaves out under Kani, and the
/// exhaustive form of the stream harnesses.
#[cfg(test)]
pub(crate) fn for_each_shorter_input(n: usize, mut f: impl FnMut(&[u8])) {
    fn go(input: &mut Vec<u8>, left: usize, f: &mut impl FnMut(&[u8])) {
        f(input);
        if left == 0 {
            return;
        }
        for c in 0..=255u8 {
            input.push(c);
            go(input, left - 1, f);
            input.pop();
        }
    }
    if n == 0 {
        return;
    }
    let mut count = 0usize;
    go(&mut Vec::with_capacity(n), n - 1, &mut |b: &[u8]| {
        count += 1;
        f(b);
    });
    assert_eq!(count, (0..n as u32).map(|k| 256usize.pow(k)).sum::<usize>());
}

/// How far the `exhaustive_*` tests of the stream lexer and parser go:
/// all byte strings up to `ENUM_BYTES` long, and strings over `ALPHABET`
/// up to `ENUM_ALPHABET` long.
#[cfg(test)]
pub(crate) const ENUM_BYTES: usize = 2;
#[cfg(test)]
pub(crate) const ENUM_ALPHABET: usize = 4;

/// One byte from each class of bytes the JSON grammar tells apart, plus a
/// few that only matter in context (`e`, `u`, hex digits, a UTF-8 lead
/// byte). Strings over it reach the lexer's behaviours at lengths where
/// all 256 byte values are too many to enumerate.
#[cfg(test)]
pub(crate) const ALPHABET: &[u8] = b" \n\"\\/{}[]:,-+.0159eEtrufalsnbxA\x01\x1f\x7f\xc3\xff";

/// Calls `f` on every string over `ALPHABET` of at most `max_len` bytes.
#[cfg(test)]
pub(crate) fn for_each_alphabet_input(max_len: usize, mut f: impl FnMut(&[u8])) {
    fn go(input: &mut Vec<u8>, left: usize, f: &mut impl FnMut(&[u8])) {
        f(input);
        if left == 0 {
            return;
        }
        for &c in ALPHABET {
            input.push(c);
            go(input, left - 1, f);
            input.pop();
        }
    }
    go(&mut Vec::with_capacity(max_len), max_len, &mut f);
}

/// `b` as a `str` if it is UTF-8: `std::str::from_utf8(b).ok()`, but under
/// Kani a byte-at-a-time check. std's validator reads aligned words found
/// with `align_offset`, which Kani handles very slowly. The two agree, see
/// `utf8_check_matches_std`.
pub(crate) fn as_utf8(b: &[u8]) -> Option<&str> {
    if !cfg!(kani) {
        return std::str::from_utf8(b).ok();
    }
    if !is_utf8(b) {
        return None;
    }
    // SAFETY: `is_utf8` accepts exactly what `from_utf8` accepts.
    Some(unsafe { std::str::from_utf8_unchecked(b) })
}

/// The UTF-8 well-formedness table (Unicode 15, table 3-7).
fn is_utf8(b: &[u8]) -> bool {
    let mut i = 0;
    while i < b.len() {
        let (n, second) = match b[i] {
            0x00..=0x7F => (1, 0x80..=0xBF),
            0xC2..=0xDF => (2, 0x80..=0xBF),
            0xE0 => (3, 0xA0..=0xBF),
            0xE1..=0xEC | 0xEE..=0xEF => (3, 0x80..=0xBF),
            0xED => (3, 0x80..=0x9F),
            0xF0 => (4, 0x90..=0xBF),
            0xF1..=0xF3 => (4, 0x80..=0xBF),
            0xF4 => (4, 0x80..=0x8F),
            _ => return false,
        };
        if b.len() - i < n {
            return false;
        }
        if n > 1 && !second.contains(&b[i + 1]) {
            return false;
        }
        if !b[i + 1..i + n]
            .iter()
            .skip(1)
            .all(|c| (0x80..=0xBF).contains(c))
        {
            return false;
        }
        i += n;
    }
    true
}

/// A `BufRead` over `data` that hands out `sizes[0]` bytes, then
/// `sizes[1]`, and so on, then whatever is left in one chunk. A size of 0
/// counts as 1, since an empty chunk means end of input.
pub(crate) struct Chunks<'a> {
    data: &'a [u8],
    sizes: &'a [usize],
}

impl<'a> Chunks<'a> {
    pub(crate) fn new(data: &'a [u8], sizes: &'a [usize]) -> Self {
        Chunks { data, sizes }
    }
}

impl Read for Chunks<'_> {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        let n = self.fill_buf()?.len().min(out.len());
        out[..n].copy_from_slice(&self.data[..n]);
        self.consume(n);
        Ok(n)
    }
}

impl BufRead for Chunks<'_> {
    fn fill_buf(&mut self) -> std::io::Result<&[u8]> {
        let n = match self.sizes.first() {
            Some(&size) => size.max(1),
            None => self.data.len(),
        };
        Ok(&self.data[..n.min(self.data.len())])
    }

    fn consume(&mut self, n: usize) {
        self.data = &self.data[n..];
        if n > 0 && !self.sizes.is_empty() {
            self.sizes = &self.sizes[1..];
        }
    }
}

/// Two errors are the same when they have the same variant and offset. The
/// message text is not compared.
pub(crate) fn same_error(a: &ParseError, b: &ParseError) -> bool {
    std::mem::discriminant(a) == std::mem::discriminant(b) && a.offset() == b.offset()
}

/// Same `Ok`, or errors that are `same_error`.
pub(crate) fn same_result(a: &Result<(), ParseError>, b: &Result<(), ParseError>) -> bool {
    match (a, b) {
        (Ok(()), Ok(())) => true,
        (Err(a), Err(b)) => same_error(a, b),
        _ => false,
    }
}

/// Stands in for `std::fmt::format` under Kani: building error messages
/// dominates solver time, and harnesses compare errors by variant and
/// offset only (`same_error`), never by message.
#[cfg(kani)]
#[allow(dead_code)] // Only named by `kani::stub`.
pub(crate) fn stub_format(_args: std::fmt::Arguments<'_>) -> String {
    // Not `String::new()`: Kani 0.68 models the `String::new()` returned
    // from a stub with capacity 1 and a dangling pointer, so dropping it
    // fails Kani's `__rust_dealloc` checks.
    String::from("e")
}

/// Stands in for `std::str::from_utf8` in harnesses whose token bytes are
/// all ASCII (`tokens::canonical`), where checking them only costs Kani
/// time. Wrong for any other input, so only those harnesses use it.
#[cfg(kani)]
#[allow(dead_code)] // Only named by `kani::stub`.
pub(crate) fn stub_from_utf8_ascii(v: &[u8]) -> Result<&str, std::str::Utf8Error> {
    // SAFETY: only used where every input is ASCII, see above.
    Ok(unsafe { std::str::from_utf8_unchecked(v) })
}

/// `stub_from_utf8_ascii` for `as_utf8`.
#[cfg(kani)]
#[allow(dead_code)] // Only named by `kani::stub`.
pub(crate) fn stub_as_utf8_ascii(v: &[u8]) -> Option<&str> {
    // SAFETY: as for `stub_from_utf8_ascii`.
    Some(unsafe { std::str::from_utf8_unchecked(v) })
}

#[cfg(test)]
mod tests {
    use super::is_utf8;

    /// Every input up to three bytes, and every four-byte input whose bytes
    /// are at the edges of the table's ranges.
    #[test]
    fn utf8_check_matches_std() {
        let check = |b: &[u8]| {
            assert_eq!(is_utf8(b), std::str::from_utf8(b).is_ok(), "{:x?}", b);
        };
        check(&[]);
        for a in 0..=255u8 {
            check(&[a]);
            for b in 0..=255u8 {
                check(&[a, b]);
                for c in 0..=255u8 {
                    check(&[a, b, c]);
                }
            }
        }
        let edges = [
            0x00, 0x7F, 0x80, 0x8F, 0x90, 0x9F, 0xA0, 0xBF, 0xC0, 0xC1, 0xC2, 0xDF, 0xE0, 0xE1,
            0xEC, 0xED, 0xEE, 0xEF, 0xF0, 0xF1, 0xF3, 0xF4, 0xF5, 0xFF,
        ];
        for &a in &edges {
            for &b in &edges {
                for &c in &edges {
                    for &d in &edges {
                        check(&[a, b, c, d]);
                    }
                }
            }
        }
    }

    #[test]
    fn utf8_check_matches_std_fuzz() {
        bolero::check!().with_type::<Vec<u8>>().for_each(|b| {
            assert_eq!(is_utf8(b), std::str::from_utf8(b).is_ok());
        });
    }
}
