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

//! Token sequences for the parser harnesses.
//!
//! Kani cannot get through `StreamLexer`, so the state-machine proofs feed
//! `Parser::parse_source` a `TokenSeq` of nondeterministic token kinds
//! directly. The Bolero and exhaustive versions lex real bytes into the
//! same type with `lex_all`, so every engine runs the same checks.

use super::Chunks;
use crate::{
    lexer::{Lexed, StreamLexer, Token, DEFAULT_MAX_TOKEN_LEN},
    parser::{ParseError, TokenSource},
};

/// Every token kind, so that a nondeterministic index picks any of them.
pub(crate) const KINDS: [Token; 11] = [
    Token::LeftBrace,
    Token::RightBrace,
    Token::LeftBracket,
    Token::RightBracket,
    Token::Colon,
    Token::Comma,
    Token::True,
    Token::False,
    Token::Null,
    Token::Int,
    Token::Number,
];

/// The kind for a fuzz byte or a nondeterministic index: any of `KINDS`,
/// or `String`.
pub(crate) fn kind_from_byte(b: u8) -> Token {
    KINDS
        .get(usize::from(b) % 12)
        .copied()
        .unwrap_or(Token::String)
}

/// The bytes a token of `kind` has in the Kani harnesses. The parser only
/// looks at the bytes of strings (as UTF-8) and numbers (through
/// `number_value`, which those harnesses stub), so one spelling per kind
/// loses nothing.
pub(crate) fn canonical(kind: Token) -> &'static [u8] {
    match kind {
        Token::LeftBrace => b"{",
        Token::RightBrace => b"}",
        Token::LeftBracket => b"[",
        Token::RightBracket => b"]",
        Token::Colon => b":",
        Token::Comma => b",",
        Token::True => b"true",
        Token::False => b"false",
        Token::Null => b"null",
        Token::Int => b"1",
        Token::Number => b"1.5",
        Token::String => b"\"s\"",
    }
}

/// A token of a `TokenSeq`: kind, bytes, absolute offset.
pub(crate) type SeqToken<'a> = (Token, &'a [u8], u64);

/// A `TokenSource` over a fixed list of tokens, then `end`: `Ok(())` for
/// `Eof`, or the error the lexer reported after the last token.
pub(crate) struct TokenSeq<'a> {
    tokens: &'a [SeqToken<'a>],
    end: &'a Result<(), ParseError>,
    pos: usize,
}

impl<'a> TokenSeq<'a> {
    pub(crate) fn new(tokens: &'a [SeqToken<'a>], end: &'a Result<(), ParseError>) -> Self {
        TokenSeq {
            tokens,
            end,
            pos: 0,
        }
    }
}

impl TokenSource for TokenSeq<'_> {
    fn next_token(&mut self) -> Result<Lexed<'_>, ParseError> {
        match self.tokens.get(self.pos) {
            Some(&(kind, bytes, offset)) => {
                self.pos += 1;
                Ok(Lexed::Token {
                    kind,
                    bytes,
                    offset,
                })
            }
            None => self.end.clone().map(|()| Lexed::Eof),
        }
    }
}

/// Token `i` of a Kani sequence is at offset `i`.
#[cfg(kani)]
pub(crate) fn canonical_tokens<const N: usize>(kinds: &[Token; N]) -> [SeqToken<'static>; N] {
    let mut tokens = [(Token::Null, &b""[..], 0); N];
    for (i, &kind) in kinds.iter().enumerate() {
        tokens[i] = (kind, canonical(kind), i as u64);
    }
    tokens
}

/// What `StreamLexer` makes of `input` in one chunk: its tokens (bytes
/// borrowed from `input`), and `Ok(())` if it ended at `Eof` or the error
/// it stopped at.
pub(crate) fn lex_all(input: &[u8]) -> (Vec<SeqToken<'_>>, Result<(), ParseError>) {
    let mut reader = Chunks::new(input, &[]);
    let mut lexer = StreamLexer::new(&mut reader, DEFAULT_MAX_TOKEN_LEN, 0);
    let mut tokens = vec![];
    loop {
        match lexer.next() {
            Ok(Lexed::Token {
                kind,
                bytes,
                offset,
            }) => {
                let at = offset as usize;
                tokens.push((kind, &input[at..at + bytes.len()], offset));
            }
            Ok(Lexed::Eof) => return (tokens, Ok(())),
            Err(e) => return (tokens, Err(e)),
        }
    }
}
