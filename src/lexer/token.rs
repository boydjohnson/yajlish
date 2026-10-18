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

//! The RFC 8259 token grammar.
//!
//! The lexer runs over bytes, not `str`, because a read boundary can split a
//! multi-byte code point. Non-ASCII bytes can only legally appear inside
//! strings; the parser checks each string token is valid UTF-8.

use logos::Logos;

/// A JSON token. Tokens carry no data: the caller slices the input by span.
#[derive(Logos, Debug, Clone, Copy, PartialEq, Eq)]
#[logos(utf8 = false)]
// RFC 8259 whitespace only, not Unicode whitespace.
#[logos(skip br"[ \t\r\n]+")]
pub(crate) enum Token {
    #[token(b"{")]
    LeftBrace,
    #[token(b"}")]
    RightBrace,
    #[token(b"[")]
    LeftBracket,
    #[token(b"]")]
    RightBracket,
    #[token(b":")]
    Colon,
    #[token(b",")]
    Comma,
    #[token(b"true")]
    True,
    #[token(b"false")]
    False,
    #[token(b"null")]
    Null,

    /// An integer: optional minus, no leading zeros.
    #[regex(br"-?(0|[1-9][0-9]*)", priority = 10)]
    Int,
    /// A number with a fraction and/or an exponent.
    #[regex(br"-?(0|[1-9][0-9]*)(\.[0-9]+)?([eE][+-]?[0-9]+)?")]
    Number,

    /// A string, quotes included: no raw U+0000..U+001F, only the eight
    /// single-character escapes plus `\uXXXX`. Any byte `>= 0x80` is admitted
    /// here and checked as UTF-8 by the caller.
    #[regex(br#""([^"\\\x00-\x1F]|\\(["\\/bfnrt]|u[0-9a-fA-F]{4}))*""#)]
    String,
}

#[cfg(test)]
mod tests {
    use super::Token;
    use logos::Logos;

    fn lex(input: &[u8]) -> Vec<Result<(Token, &[u8]), ()>> {
        let mut lexer = Token::lexer(input);
        let mut out = vec![];
        while let Some(tok) = lexer.next() {
            out.push(tok.map(|t| (t, lexer.slice())));
        }
        out
    }

    /// The input lexes as exactly one token of `kind`, covering all of it.
    fn assert_single(input: &[u8], kind: Token) {
        assert_eq!(lex(input), vec![Ok((kind, input))], "{:?}", input);
    }

    /// The input does not lex as a single valid token.
    fn assert_rejected(input: &[u8]) {
        let toks = lex(input);
        assert!(
            toks.len() != 1 || toks[0].is_err(),
            "{:?} lexed as {:?}",
            String::from_utf8_lossy(input),
            toks
        );
    }

    #[test]
    fn test_punctuation_and_keywords() {
        assert_single(b"{", Token::LeftBrace);
        assert_single(b"}", Token::RightBrace);
        assert_single(b"[", Token::LeftBracket);
        assert_single(b"]", Token::RightBracket);
        assert_single(b":", Token::Colon);
        assert_single(b",", Token::Comma);
        assert_single(b"true", Token::True);
        assert_single(b"false", Token::False);
        assert_single(b"null", Token::Null);
    }

    #[test]
    fn test_numbers() {
        for int in [&b"0"[..], b"-0", b"27", b"-27", b"9223372036854775808"] {
            assert_single(int, Token::Int);
        }
        for num in [
            &b"0.5"[..],
            b"-0.5",
            b"27.0",
            b"1e5",
            b"1E5",
            b"1e+5",
            b"1e-5",
            b"-1.25e-10",
            b"0e0",
        ] {
            assert_single(num, Token::Number);
        }
    }

    #[test]
    fn test_strings() {
        for s in [
            &br#""""#[..],
            br#""foo""#,
            br#""\"\\\/\b\f\n\r\t""#,
            br#""\u00e9\uD83D\uDE00""#,
            br#""\ud800""#,
            "\"caf\u{e9} \u{1F600}\"".as_bytes(),
            b"\"\x7f\"",
        ] {
            assert_single(s, Token::String);
        }
    }

    #[test]
    fn test_whitespace_is_skipped() {
        assert_eq!(
            lex(b" \t\r\n[ 1 ,\n2 ]\n"),
            vec![
                Ok((Token::LeftBracket, &b"["[..])),
                Ok((Token::Int, &b"1"[..])),
                Ok((Token::Comma, &b","[..])),
                Ok((Token::Int, &b"2"[..])),
                Ok((Token::RightBracket, &b"]"[..])),
            ]
        );
    }

    #[test]
    fn test_rfc_8259_rejections() {
        for bad in [
            &b"01"[..],
            b"-01",
            b"-",
            b"1.",
            b".5",
            b"+1",
            b"1e",
            b"1e+",
            br#""\x""#,
            br#""\u12""#,
            br#""\u12G4""#,
            b"\"a\tb\"",
            b"\"a\nb\"",
            b"\"a\x00b\"",
            b"\"abc",
            b"True",
            b"nul",
            b"NaN",
            b"Infinity",
            b"'a'",
            b"\xc2\xa0",
            b"\x0c",
        ] {
            assert_rejected(bad);
        }
    }
}
