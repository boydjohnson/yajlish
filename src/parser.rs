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
        let result = self.parse_tokens(&mut lexer);
        self.offset = lexer.offset();
        result
    }

    fn parse_tokens<B: BufRead + ?Sized>(
        &mut self,
        lexer: &mut StreamLexer<'_, B>,
    ) -> Result<(), ParseError> {
        let context = &mut self.context;

        while !matches!(
            context.parser_status(),
            ParserStatus::ParseComplete | ParserStatus::LexicalError
        ) {
            let (kind, bytes, offset) = match lexer.next()? {
                Lexed::Token {
                    kind,
                    bytes,
                    offset,
                } => (kind, bytes, offset),
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
                        return Err(ParseError::MalformedJson(format!("Parsed right bracket without a corresponding left bracket: braces: {}, brackets: {}", context.num_open_braces(), context.num_open_brackets())));
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
                    let text = std::str::from_utf8(bytes).map_err(|e| {
                        ParseError::Utf8Error(format!("{} at offset {}", e, offset))
                    })?;
                    let int = if kind == Token::Int {
                        text.parse::<i64>().ok()
                    } else {
                        None
                    };
                    let status = match int {
                        Some(num) => self.handler.handle_int(context, num),
                        // An `Int` that overflows i64 falls back to f64, as before.
                        None => match text.parse::<f64>() {
                            Ok(num) => self.handler.handle_double(context, num),
                            Err(e) => {
                                return Err(ParseError::MalformedJson(format!(
                                    "invalid number {:?} at offset {}: {}",
                                    text, offset, e
                                )))
                            }
                        },
                    };

                    update_context_status_value(context);

                    Some(status)
                }
                Token::String => {
                    let string = std::str::from_utf8(bytes).map_err(|e| {
                        ParseError::Utf8Error(format!(
                            "{} at offset {}",
                            e,
                            offset + e.valid_up_to() as u64
                        ))
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
            return Err(ParseError::MalformedJson(format!(
                "Parse failed due to malformed json: open braces: {}, open brackets: {}",
                self.context.num_open_braces(),
                self.context.num_open_brackets()
            )));
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
            return Err(ParseError::MalformedJson(
                "Did not reach a ParseComplete status".to_owned(),
            ));
        }
        if self.context.num_open_braces() != 0 {
            return Err(ParseError::MalformedJson(format!(
                "Number of open braces: {}, number of open brackets: {}",
                self.context.num_open_braces(),
                self.context.num_open_brackets()
            )));
        }
        if self.context.num_open_brackets() != 0 {
            return Err(ParseError::MalformedJson(format!(
                "Number of open braces: {}, number of open brackets: {}",
                self.context.num_open_braces(),
                self.context.num_open_brackets()
            )));
        }

        Ok(())
    }
}

/// `ParseError`
#[derive(PartialEq, Eq)]
pub enum ParseError {
    /// Bytes can't be decoded as UTF-8 to a string.
    Utf8Error(String),
    /// The json is malformed in some way, missing closing brace, bracket, or
    /// a value can't be parsed due to being malformed.
    MalformedJson(String),
    /// Error of the underlying Reader.
    ReadError(String),
}

impl std::error::Error for ParseError {
    fn description(&self) -> &str {
        match self {
            ParseError::Utf8Error(ref msg)
            | ParseError::MalformedJson(ref msg)
            | ParseError::ReadError(ref msg) => msg,
        }
    }

    fn cause(&self) -> Option<&dyn std::error::Error> {
        None
    }
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParseError::Utf8Error(ref msg) => {
                write!(f, "Error converting bytes to UTF-8 encoded string: {}", msg)
            }
            ParseError::MalformedJson(ref msg) => write!(f, "Error: Malformed Json: {}", msg),
            ParseError::ReadError(ref msg) => write!(f, "Error: Read Error: {}", msg),
        }
    }
}

impl std::fmt::Debug for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self, f)
    }
}

impl From<std::io::Error> for ParseError {
    fn from(other: std::io::Error) -> Self {
        ParseError::ReadError(other.to_string())
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
