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
    lexer::{parse, JsonPrimitive},
    Context, Handler, Status,
};
use std::io::BufRead;

/// Main Parser struct.
pub struct Parser<'a> {
    handler: &'a mut dyn Handler,
    context: Context,

    buffer: Vec<u8>,
    buffer_offset: usize,
}

impl<'a> Parser<'a> {
    /// Construct a new Parser from a Handler.
    pub fn new(handler: &'a mut dyn Handler) -> Self {
        Parser {
            handler,
            context: Context::default(),
            buffer: vec![],
            buffer_offset: 0,
        }
    }

    /// Parse until Handler method returns Abort or EOF.
    ///
    /// # Errors
    ///    - Will return an error Result if the JSON is malformed, or if the underlying
    ///   Reader returns an error.
    pub fn parse<B: BufRead>(&mut self, read: &mut B) -> Result<(), ParseError> {
        let context = &mut self.context;

        while match context.parser_status() {
            ParserStatus::ParseComplete | ParserStatus::LexicalError => false,
            _ => true,
        } {
            let buffer = match read.fill_buf() {
                Ok(buffer) => buffer,
                Err(err) => return Err(ParseError::ReadError(err.to_string())),
            };

            let buffer_length = buffer.len();

            self.buffer.extend_from_slice(buffer);

            read.consume(buffer_length);

            if buffer_length == 0
                && (self.buffer.is_empty() || self.buffer_offset >= self.buffer.len())
            {
                context.update_status(ParserStatus::ParseComplete);
                return Ok(());
            }

            let buffer_length = self.buffer[self.buffer_offset..].len();

            let (status, consume_length) = match parse(&self.buffer[self.buffer_offset..]) {
                Ok((rest, JsonPrimitive::WS)) => (None, Some(buffer_length - rest.len())),
                Ok((rest, JsonPrimitive::RightBracket)) => {
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
                    (Some(status), Some(buffer_length - rest.len()))
                }
                Ok((rest, JsonPrimitive::RightBrace)) => {
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

                    (Some(status), Some(buffer_length - rest.len()))
                }
                Ok((rest, JsonPrimitive::LeftBracket)) => {
                    let status = self.handler.handle_start_array(context);
                    context.add_enclosing(Enclosing::LeftBracket);
                    context.inc_brackets();
                    context.update_status(ParserStatus::ArrayStart);
                    (Some(status), Some(buffer_length - rest.len()))
                }
                Ok((rest, JsonPrimitive::LeftBrace)) => {
                    let status = self.handler.handle_start_map(context);
                    context.add_enclosing(Enclosing::LeftBrace);
                    context.inc_braces();
                    context.update_status(ParserStatus::MapStart);

                    (Some(status), Some(buffer_length - rest.len()))
                }
                Ok((rest, JsonPrimitive::Null)) => {
                    let status = self.handler.handle_null(context);

                    update_context_status_value(context);

                    (Some(status), Some(buffer_length - rest.len()))
                }
                Ok((rest, JsonPrimitive::Integer(num))) => {
                    let status = self.handler.handle_int(context, num);

                    update_context_status_value(context);

                    (Some(status), Some(buffer_length - rest.len()))
                }
                Ok((rest, JsonPrimitive::Double(num))) => {
                    let status = self.handler.handle_double(context, num);

                    update_context_status_value(context);

                    (Some(status), Some(buffer_length - rest.len()))
                }
                Ok((rest, JsonPrimitive::JSONString(s))) => {
                    if context.parser_status() == ParserStatus::ArrayNeedVal
                        || context.parser_status() == ParserStatus::ArrayStart
                    {
                        let status = self.handler.handle_string(context, &s);
                        context.update_status(ParserStatus::ArrayGotVal);
                        (Some(status), Some(buffer_length - rest.len()))
                    } else if context.parser_status() == ParserStatus::MapNeedVal {
                        let status = self.handler.handle_string(context, &s);
                        context.update_status(ParserStatus::MapGotVal);
                        (Some(status), Some(buffer_length - rest.len()))
                    } else if context.parser_status() == ParserStatus::Start {
                        let status = self.handler.handle_string(context, &s);
                        context.update_status(ParserStatus::GotValue);
                        (Some(status), Some(buffer_length - rest.len()))
                    } else if context.parser_status() == ParserStatus::MapNeedKey
                        || context.parser_status() == ParserStatus::MapStart
                    {
                        let status = self.handler.handle_map_key(context, &s);
                        context.update_status(ParserStatus::MapSep);
                        (Some(status), Some(buffer_length - rest.len()))
                    } else {
                        context.update_status(ParserStatus::LexicalError);
                        (None, None)
                    }
                }
                Ok((rest, JsonPrimitive::Boolean(boolean))) => {
                    let status = self.handler.handle_bool(context, boolean);

                    update_context_status_value(context);

                    let length = buffer_length - rest.len();
                    (Some(status), Some(length))
                }
                Ok((rest, JsonPrimitive::Comma)) => {
                    if context.parser_status() == ParserStatus::MapGotVal {
                        context.update_status(ParserStatus::MapNeedKey);
                    } else if context.parser_status() == ParserStatus::ArrayGotVal {
                        context.update_status(ParserStatus::ArrayNeedVal);
                    } else {
                        context.update_status(ParserStatus::LexicalError);
                    }

                    (None, Some(buffer_length - rest.len()))
                }
                Ok((rest, JsonPrimitive::Colon)) => {
                    if context.parser_status() == ParserStatus::MapSep {
                        context.update_status(ParserStatus::MapNeedVal);
                    }

                    (None, Some(buffer_length - rest.len()))
                }
                Err(_) => (None, None),
            };

            if let Some(consume_length) = consume_length {
                self.buffer_offset += consume_length;
            }

            if self.buffer_offset > 1_000_000_000 {
                self.buffer.drain(0..self.buffer_offset);
                self.buffer_offset = 0;
            }

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
                writeln!(f, "Error converting bytes to UTF-8 encoded string: {}", msg)
            }
            ParseError::MalformedJson(ref msg) => writeln!(f, "Error: Malformed Json: {}", msg),
            ParseError::ReadError(ref msg) => writeln!(f, "Error: Read Error: {}", msg),
        }
    }
}

impl std::fmt::Debug for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self, f)
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
