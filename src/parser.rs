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

//! Parser struct should be used to parse bytes in to json.

use crate::{
    common::{Enclosing, ParserStatus},
    lexer::{Lexed, StreamLexer, Token, DEFAULT_MAX_TOKEN_LEN},
    Context, Handler, Status,
};
use std::io::BufRead;

/// Main Parser struct.
pub struct Parser<'a, H> {
    handler: &'a mut H,
    context: Context,
    max_token_len: usize,
    /// Bytes consumed by earlier `parse` calls, so offsets stay absolute.
    offset: u64,
}

impl<'a, H: Handler> Parser<'a, H> {
    /// Construct a new Parser from a Handler.
    pub fn new(handler: &'a mut H) -> Self {
        Parser {
            handler,
            context: Context::default(),
            max_token_len: DEFAULT_MAX_TOKEN_LEN,
            offset: 0,
        }
    }

    /// Cap the length in bytes of a single token (a string, key or number).
    /// A longer token is a `MalformedJson` error instead of unbounded
    /// buffering. The default is 64 MiB.
    #[must_use]
    pub fn with_max_token_len(mut self, max_token_len: usize) -> Self {
        self.max_token_len = max_token_len;
        self
    }

    /// Parse until Handler method returns Abort or EOF.
    ///
    /// # Errors
    ///    - Will return an error Result if the JSON is malformed, or if the underlying
    ///      Reader returns an error.
    pub fn parse<B: BufRead + ?Sized>(&mut self, read: &mut B) -> Result<(), ParseError> {
        let mut lexer = StreamLexer::new(read, self.max_token_len, self.offset);
        let result = self.parse_source(&mut lexer);
        self.offset = lexer.offset();
        result
    }

    /// The parser state machine over any source of tokens. `parse` feeds it
    /// a `StreamLexer`; the Kani harnesses feed it token sequences directly,
    /// since Kani cannot get through the lexer.
    pub(crate) fn parse_source<S: TokenSource + ?Sized>(
        &mut self,
        lexer: &mut S,
    ) -> Result<(), ParseError> {
        let context = &mut self.context;
        let mut last_offset = lexer.offset();

        while !matches!(
            context.parser_status(),
            ParserStatus::ParseComplete | ParserStatus::LexicalError
        ) {
            let (kind, bytes, offset) = match lexer.next_token()? {
                Lexed::Token {
                    kind,
                    bytes,
                    offset,
                } => {
                    last_offset = offset;
                    context.set_offset(offset);
                    (kind, bytes, offset)
                }
                Lexed::Eof => {
                    context.update_status(ParserStatus::ParseComplete);
                    break;
                }
            };

            let status = match kind {
                Token::RightBracket => {
                    let status = self.handler.handle_end_array(context);
                    if context.last_enclosing() == Some(Enclosing::LeftBracket) {
                        context.remove_last_enclosing();
                        context.dec_brackets();
                    } else {
                        return Err(ParseError::malformed(offset, format!("Parsed right bracket without a corresponding left bracket: braces: {}, brackets: {}", context.num_open_braces(), context.num_open_brackets())));
                    }

                    if context.last_enclosing() == Some(Enclosing::LeftBrace) {
                        context.update_status(ParserStatus::MapGotVal);
                    } else if context.last_enclosing() == Some(Enclosing::LeftBracket) {
                        context.update_status(ParserStatus::ArrayGotVal);
                    } else {
                        context.update_status(ParserStatus::GotValue);
                    }
                    Some(status)
                }
                Token::RightBrace => {
                    let status = self.handler.handle_end_map(context);

                    if context.last_enclosing() == Some(Enclosing::LeftBrace) {
                        context.remove_last_enclosing();
                        context.dec_braces();
                    } else {
                        context.update_status(ParserStatus::LexicalError);
                    }

                    if context.last_enclosing() == Some(Enclosing::LeftBrace) {
                        context.update_status(ParserStatus::MapGotVal);
                    } else if context.last_enclosing() == Some(Enclosing::LeftBracket) {
                        context.update_status(ParserStatus::ArrayGotVal);
                    } else {
                        context.update_status(ParserStatus::GotValue);
                    }

                    Some(status)
                }
                Token::LeftBracket => {
                    let status = self.handler.handle_start_array(context);
                    context.add_enclosing(Enclosing::LeftBracket);
                    context.inc_brackets();
                    context.update_status(ParserStatus::ArrayStart);
                    Some(status)
                }
                Token::LeftBrace => {
                    let status = self.handler.handle_start_map(context);
                    context.add_enclosing(Enclosing::LeftBrace);
                    context.inc_braces();
                    context.update_status(ParserStatus::MapStart);

                    Some(status)
                }
                Token::Null => {
                    let status = self.handler.handle_null(context);

                    update_context_status_value(context);

                    Some(status)
                }
                Token::Int | Token::Number => {
                    let text = std::str::from_utf8(bytes).map_err(|e| ParseError::Utf8Error {
                        offset: offset + e.valid_up_to() as u64,
                        msg: e.to_string(),
                    })?;
                    let status = match number_value(kind, text, offset)? {
                        Number::Int(num) => self.handler.handle_int(context, num),
                        Number::Double(num) => self.handler.handle_double(context, num),
                    };

                    update_context_status_value(context);

                    Some(status)
                }
                Token::String => {
                    let string = std::str::from_utf8(bytes).map_err(|e| ParseError::Utf8Error {
                        offset: offset + e.valid_up_to() as u64,
                        msg: e.to_string(),
                    })?;

                    if context.parser_status() == ParserStatus::ArrayNeedVal
                        || context.parser_status() == ParserStatus::ArrayStart
                    {
                        let status = self.handler.handle_string(context, string);
                        context.update_status(ParserStatus::ArrayGotVal);
                        Some(status)
                    } else if context.parser_status() == ParserStatus::MapNeedVal {
                        let status = self.handler.handle_string(context, string);
                        context.update_status(ParserStatus::MapGotVal);
                        Some(status)
                    } else if context.parser_status() == ParserStatus::Start {
                        let status = self.handler.handle_string(context, string);
                        context.update_status(ParserStatus::GotValue);
                        Some(status)
                    } else if context.parser_status() == ParserStatus::MapNeedKey
                        || context.parser_status() == ParserStatus::MapStart
                    {
                        let status = self.handler.handle_map_key(context, string);
                        context.update_status(ParserStatus::MapSep);
                        Some(status)
                    } else {
                        context.update_status(ParserStatus::LexicalError);
                        None
                    }
                }
                Token::True => {
                    let status = self.handler.handle_bool(context, true);

                    update_context_status_value(context);

                    Some(status)
                }
                Token::False => {
                    let status = self.handler.handle_bool(context, false);

                    update_context_status_value(context);

                    Some(status)
                }
                Token::Comma => {
                    if context.parser_status() == ParserStatus::MapGotVal {
                        context.update_status(ParserStatus::MapNeedKey);
                    } else if context.parser_status() == ParserStatus::ArrayGotVal {
                        context.update_status(ParserStatus::ArrayNeedVal);
                    } else {
                        context.update_status(ParserStatus::LexicalError);
                    }

                    None
                }
                Token::Colon => {
                    if context.parser_status() == ParserStatus::MapSep {
                        context.update_status(ParserStatus::MapNeedVal);
                    }

                    None
                }
            };

            if status == Some(Status::Abort) {
                return Ok(());
            }
        }
        if self.context.parser_status() == ParserStatus::LexicalError {
            return Err(ParseError::malformed(
                last_offset,
                format!(
                    "Parse failed due to malformed json: open braces: {}, open brackets: {}",
                    self.context.num_open_braces(),
                    self.context.num_open_brackets()
                ),
            ));
        }

        Ok(())
    }

    /// Parse has already returned from an EOF. This method checks that
    /// there were the right number of closing braces and brackets.
    ///
    /// # Errors
    ///    - Returns an error Result if the JSON was malformed.
    pub fn finish_parse(self) -> Result<(), ParseError> {
        if self.context.parser_status() != ParserStatus::ParseComplete {
            return Err(ParseError::malformed(
                self.offset,
                "Did not reach a ParseComplete status",
            ));
        }
        if self.context.num_open_braces() != 0 || self.context.num_open_brackets() != 0 {
            return Err(ParseError::malformed(
                self.offset,
                format!(
                    "Number of open braces: {}, number of open brackets: {}",
                    self.context.num_open_braces(),
                    self.context.num_open_brackets()
                ),
            ));
        }

        Ok(())
    }
}

/// Where `Parser::parse_source` gets its tokens.
pub(crate) trait TokenSource {
    /// The next token, or `Eof`.
    fn next_token(&mut self) -> Result<Lexed<'_>, ParseError>;

    /// Absolute offset of the first byte not yet returned as a token.
    fn offset(&self) -> u64;
}

impl<R: BufRead + ?Sized> TokenSource for StreamLexer<'_, R> {
    fn next_token(&mut self) -> Result<Lexed<'_>, ParseError> {
        self.next()
    }

    fn offset(&self) -> u64 {
        StreamLexer::offset(self)
    }
}

/// Why a parse failed, with the absolute byte offset into the stream where
/// the problem was found.
///
/// Offsets count every byte read by every `parse` call on the same
/// `Parser`. For errors found by `finish_parse`, the offset is the end of
/// the input.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ParseError {
    /// A string's bytes are not valid UTF-8.
    Utf8Error {
        /// Offset of the first invalid byte.
        offset: u64,
        /// Description of the problem.
        msg: String,
    },
    /// The json is malformed in some way: a byte sequence that is not a JSON
    /// token, a token in the wrong place, or missing closing braces or
    /// brackets.
    MalformedJson {
        /// Offset of the offending token.
        offset: u64,
        /// Description of the problem.
        msg: String,
    },
    /// The underlying reader returned an error.
    ReadError {
        /// Offset of the first byte that could not be read.
        offset: u64,
        /// The reader's error message.
        msg: String,
    },
}

impl ParseError {
    pub(crate) fn malformed(offset: u64, msg: impl Into<String>) -> Self {
        ParseError::MalformedJson {
            offset,
            msg: msg.into(),
        }
    }

    /// The byte offset in the stream where the error was found.
    #[must_use]
    pub fn offset(&self) -> u64 {
        match self {
            ParseError::Utf8Error { offset, .. }
            | ParseError::MalformedJson { offset, .. }
            | ParseError::ReadError { offset, .. } => *offset,
        }
    }

    /// The description of the error, without the offset.
    #[must_use]
    pub fn message(&self) -> &str {
        match self {
            ParseError::Utf8Error { msg, .. }
            | ParseError::MalformedJson { msg, .. }
            | ParseError::ReadError { msg, .. } => msg,
        }
    }
}

impl std::error::Error for ParseError {}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParseError::Utf8Error { offset, msg } => write!(
                f,
                "Error converting bytes to UTF-8 encoded string at byte {}: {}",
                offset, msg
            ),
            ParseError::MalformedJson { offset, msg } => {
                write!(f, "Error: Malformed Json at byte {}: {}", offset, msg)
            }
            ParseError::ReadError { offset, msg } => {
                write!(f, "Error: Read Error at byte {}: {}", offset, msg)
            }
        }
    }
}

/// The value of a number token.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Number {
    Int(i64),
    Double(f64),
}

/// The value of an `Int` or `Number` token whose text is `text`, found at
/// `offset`. Kept out of `parse_tokens` so that Kani harnesses can stub it:
/// float parsing is far too large to model check.
pub(crate) fn number_value(kind: Token, text: &str, offset: u64) -> Result<Number, ParseError> {
    if kind == Token::Int {
        if let Ok(num) = text.parse::<i64>() {
            return Ok(Number::Int(num));
        }
    }
    // An `Int` that overflows i64 falls back to f64, as before.
    match text.parse::<f64>() {
        Ok(num) => Ok(Number::Double(num)),
        Err(e) => Err(ParseError::malformed(
            offset,
            format!("invalid number {:?}: {}", text, e),
        )),
    }
}

fn update_context_status_value(context: &mut Context) {
    if context.parser_status() == ParserStatus::ArrayNeedVal
        || context.parser_status() == ParserStatus::ArrayStart
    {
        context.update_status(ParserStatus::ArrayGotVal);
    } else if context.parser_status() == ParserStatus::MapNeedVal {
        context.update_status(ParserStatus::MapGotVal);
    } else if context.parser_status() == ParserStatus::Start {
        context.update_status(ParserStatus::GotValue);
    } else {
        context.update_status(ParserStatus::LexicalError);
    }
}
