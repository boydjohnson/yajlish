/*
* Copyright 2026 Boyd Johnson
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

//! A streaming lexer over a `BufRead`.
//!
//! logos lexes a slice, but the input arrives in chunks of whatever size the
//! reader hands out. `StreamLexer` owns a buffer, lexes the unconsumed part
//! of it, and reads more whenever the token at the front could still change
//! with more input: a cut-off string (`"ab`), keyword (`tru`) or number
//! (`12` that may be `123`, `1.` that may be `1.5`).
//!
//! The rule is exact rather than heuristic. Take the bytes from the start of
//! the first token (or error) to the end of the buffer. If they are a prefix
//! of some longer valid token, more input may change what logos matches, so
//! refill and lex again. If they are not, every token that could start there
//! ends inside the buffer, and appending bytes cannot change the result.

mod token;

pub(crate) use token::Token;

use crate::parser::ParseError;
use logos::Logos;
use std::io::{BufRead, ErrorKind};

/// Default cap on the length of a single token, 64 MiB.
pub(crate) const DEFAULT_MAX_TOKEN_LEN: usize = 64 * 1024 * 1024;

/// One step of the lexer.
#[derive(Debug, PartialEq)]
pub(crate) enum Lexed<'b> {
    /// A token with its bytes and the absolute stream offset of its first byte.
    Token {
        kind: Token,
        bytes: &'b [u8],
        offset: u64,
    },
    /// The reader is exhausted and only whitespace remained.
    Eof,
}

/// Lexes JSON tokens from a `BufRead`, handling tokens split across reads.
pub(crate) struct StreamLexer<'r, R: ?Sized> {
    reader: &'r mut R,
    /// Unconsumed input is `buf[pos..]`.
    buf: Vec<u8>,
    pos: usize,
    /// Absolute stream offset of `buf[0]`.
    base: u64,
    eof: bool,
    max_token_len: usize,
}

impl<'r, R: BufRead + ?Sized> StreamLexer<'r, R> {
    pub(crate) fn new(reader: &'r mut R, max_token_len: usize, base: u64) -> Self {
        StreamLexer {
            reader,
            buf: Vec::new(),
            pos: 0,
            base,
            eof: false,
            max_token_len,
        }
    }

    /// Absolute offset of the first byte not yet returned as a token.
    pub(crate) fn offset(&self) -> u64 {
        self.base + self.pos as u64
    }

    /// The next token, or `Eof`.
    ///
    /// # Errors
    /// `MalformedJson` for bytes that are not a JSON token or for a token
    /// longer than `max_token_len`, `ReadError` if the reader fails.
    pub(crate) fn next(&mut self) -> Result<Lexed<'_>, ParseError> {
        loop {
            let input = &self.buf[self.pos..];
            let mut lexer = Token::lexer(input);
            let first = lexer.next();
            let span = lexer.span();

            let start = match first {
                // Only whitespace (or nothing) is buffered.
                None => {
                    if self.eof {
                        self.pos = self.buf.len();
                        return Ok(Lexed::Eof);
                    }
                    self.pos = self.buf.len();
                    self.refill()?;
                    continue;
                }
                Some(_) => span.start,
            };

            let rest = &input[start..];
            if !self.eof && may_extend(rest) {
                if rest.len() > self.max_token_len {
                    return Err(ParseError::MalformedJson(format!(
                        "token longer than {} bytes at offset {}",
                        self.max_token_len,
                        self.offset() + start as u64
                    )));
                }
                // Drop the whitespace before the token too, so that the
                // buffer holds at most the token in flight plus one read.
                self.pos += start;
                self.refill()?;
                continue;
            }

            let offset = self.offset() + start as u64;
            return match first {
                Some(Ok(_)) if span.len() > self.max_token_len => {
                    Err(ParseError::MalformedJson(format!(
                        "token longer than {} bytes at offset {}",
                        self.max_token_len, offset
                    )))
                }
                Some(Ok(kind)) => {
                    let begin = self.pos + span.start;
                    let end = self.pos + span.end;
                    self.pos = end;
                    Ok(Lexed::Token {
                        kind,
                        bytes: &self.buf[begin..end],
                        offset,
                    })
                }
                _ => Err(ParseError::MalformedJson(format!(
                    "invalid token {:?} at offset {}",
                    String::from_utf8_lossy(&rest[..rest.len().min(span.end - start).max(1)]),
                    offset
                ))),
            };
        }
    }

    /// Compact the buffer and append the reader's next chunk. Sets `eof`
    /// when the reader returns no bytes.
    fn refill(&mut self) -> Result<(), ParseError> {
        if self.pos > 0 {
            self.buf.drain(..self.pos);
            self.base += self.pos as u64;
            self.pos = 0;
        }
        loop {
            match self.reader.fill_buf() {
                Ok(chunk) => {
                    if chunk.is_empty() {
                        self.eof = true;
                    } else {
                        self.buf.extend_from_slice(chunk);
                    }
                    let n = chunk.len();
                    self.reader.consume(n);
                    return Ok(());
                }
                Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                Err(e) => {
                    return Err(ParseError::ReadError(format!(
                        "{} at offset {}",
                        e,
                        self.base + self.buf.len() as u64
                    )))
                }
            }
        }
    }
}

/// Whether some non-empty byte string can be appended to `b` so that the
/// result is a prefix of a valid token. `b` starts at a token boundary.
fn may_extend(b: &[u8]) -> bool {
    match b.first() {
        None => true,
        Some(b'"') => string_may_extend(&b[1..]),
        Some(b't') => b"true".starts_with(b) && b.len() < 4,
        Some(b'f') => b"false".starts_with(b) && b.len() < 5,
        Some(b'n') => b"null".starts_with(b) && b.len() < 4,
        Some(b'-' | b'0'..=b'9') => is_number_prefix(b),
        Some(_) => false,
    }
}

/// `b` is the content after an opening quote. True when it is a valid,
/// unterminated string body (so a closing quote can still follow).
fn string_may_extend(b: &[u8]) -> bool {
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'"' => return false,
            0x00..=0x1F => return false,
            b'\\' => {
                let Some(&esc) = b.get(i + 1) else {
                    return true;
                };
                match esc {
                    b'"' | b'\\' | b'/' | b'b' | b'f' | b'n' | b'r' | b't' => i += 2,
                    b'u' => {
                        let hex = &b[(i + 2).min(b.len())..(i + 6).min(b.len())];
                        if !hex.iter().all(u8::is_ascii_hexdigit) {
                            return false;
                        }
                        i += 6;
                    }
                    _ => return false,
                }
            }
            _ => i += 1,
        }
    }
    true
}

/// Whether `b` is a prefix of a string matching
/// `-?(0|[1-9][0-9]*)(\.[0-9]+)?([eE][+-]?[0-9]+)?`. Every such prefix can be
/// extended by at least one more byte, so for numbers this is exactly "may
/// extend".
fn is_number_prefix(b: &[u8]) -> bool {
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
    for &c in b {
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
            _ => return false,
        };
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::BufRead;

    /// A `BufRead` that hands out at most `chunk` bytes per `fill_buf`.
    struct Chunked<'a> {
        data: &'a [u8],
        chunk: usize,
    }

    impl std::io::Read for Chunked<'_> {
        fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
            let n = self.fill_buf()?.len().min(out.len());
            out[..n].copy_from_slice(&self.data[..n]);
            self.consume(n);
            Ok(n)
        }
    }

    impl BufRead for Chunked<'_> {
        fn fill_buf(&mut self) -> std::io::Result<&[u8]> {
            Ok(&self.data[..self.chunk.min(self.data.len())])
        }
        fn consume(&mut self, n: usize) {
            self.data = &self.data[n..];
        }
    }

    fn tokens(data: &[u8], chunk: usize) -> Result<Vec<(Token, Vec<u8>, u64)>, ParseError> {
        let mut reader = Chunked { data, chunk };
        let mut lexer = StreamLexer::new(&mut reader, DEFAULT_MAX_TOKEN_LEN, 0);
        let mut out = vec![];
        loop {
            match lexer.next()? {
                Lexed::Token {
                    kind,
                    bytes,
                    offset,
                } => out.push((kind, bytes.to_vec(), offset)),
                Lexed::Eof => return Ok(out),
            }
        }
    }

    const CORPUS: &[&[u8]] = &[
        b"[1, 23, -456, 7.5, -0.25e10, 1E+2, 0, -0]",
        b"{\"a\\\"b\": \"x\\\\y\\u00e9\", \"k\": [true, false, null]}",
        b"  \n\t 12345678901234567890  ",
        b"\"\xc3\xa9\xf0\x9f\x98\x80\"",
        b"[\"\",\"\",{}]",
        b"truefalse",
        b"1.5e-3,2",
    ];

    #[test]
    fn test_every_chunk_size_matches_unchunked() {
        for input in CORPUS {
            let whole = tokens(input, usize::MAX).unwrap();
            for chunk in 1..=input.len() {
                assert_eq!(
                    tokens(input, chunk).unwrap(),
                    whole,
                    "{:?} chunk {}",
                    String::from_utf8_lossy(input),
                    chunk
                );
            }
        }
    }

    #[test]
    fn test_errors_survive_chunking() {
        for bad in [
            &b"[1.]"[..],
            b"[tru]",
            b"\"abc",
            b"-",
            b"[1e]",
            b"@",
            b"\"a\x01\"",
        ] {
            for chunk in 1..=bad.len() {
                assert!(tokens(bad, chunk).is_err(), "{:?} chunk {}", bad, chunk);
            }
        }
    }

    #[test]
    fn test_offsets() {
        let toks = tokens(b" [ 10 ,\"a\"]", 2).unwrap();
        let offsets: Vec<u64> = toks.iter().map(|t| t.2).collect();
        assert_eq!(offsets, vec![1, 3, 6, 7, 10]);
    }

    #[test]
    fn test_max_token_len() {
        let mut data: &[u8] = b"\"aaaaaaaaaaaaaaaaaaaa\"";
        let mut lexer = StreamLexer::new(&mut data, 8, 0);
        assert!(matches!(lexer.next(), Err(ParseError::MalformedJson(_))));
        let mut data: &[u8] = b"\"aaaaaa\"";
        let mut lexer = StreamLexer::new(&mut data, 8, 0);
        assert!(lexer.next().is_ok());

        let mut reader = Chunked {
            data: b"\"aaaaaaaaaaaaaaaaaaaa\"",
            chunk: 1,
        };
        let mut lexer = StreamLexer::new(&mut reader, 8, 0);
        assert!(matches!(lexer.next(), Err(ParseError::MalformedJson(_))));
    }

    #[test]
    fn test_may_extend() {
        for yes in [
            &b""[..],
            b"\"",
            b"\"ab",
            b"\"a\\",
            b"\"\\u12",
            b"t",
            b"fals",
            b"-",
            b"0",
            b"1.",
            b"1e",
            b"1e-",
            b"12",
        ] {
            assert!(may_extend(yes), "{:?}", String::from_utf8_lossy(yes));
        }
        for no in [
            &b"\"ab\""[..],
            b"true",
            b"null",
            b"01",
            b"1.e",
            b"\"\\x",
            b"\"\\u12g",
            b"x",
            b"1,",
            b"truex",
        ] {
            assert!(!may_extend(no), "{:?}", String::from_utf8_lossy(no));
        }
    }
}
