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

use std::collections::VecDeque;

/// The Status that each Handler method returns.
#[derive(Debug, PartialEq, Eq)]
pub enum Status {
    /// Continue calling methods on the Handler.
    Continue,
    /// Stop calling methods.
    Abort,
}

/// Brackets and Braces to keep track of.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Enclosing {
    /// Left Brace '{'
    LeftBrace,
    /// Left Bracket '['
    LeftBracket,
}

/// The state that the parser is in.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum ParserStatus {
    /// Received the first token
    Start,
    /// Received the last token
    ParseComplete,
    /// Error
    ParseError,
    /// The json is malformed
    LexicalError,
    /// Received {
    MapStart,
    /// Received
    MapSep,
    /// Received
    MapNeedVal,
    /// Received Map value
    MapGotVal,
    /// Received , in a Map
    MapNeedKey,
    /// Received [
    ArrayStart,
    /// Received Array value
    ArrayGotVal,
    /// Received , in a Array
    ArrayNeedVal,
    /// Received a value that is the whole document like "true"
    GotValue,
}

/// The context passed to each Handler function. Context gives
/// basic information about where it is at in the json document.
#[derive(Debug)]
pub struct Context {
    stack: VecDeque<Enclosing>,

    status: ParserStatus,
    num_open_braces: usize,
    num_open_brackets: usize,
}

impl Default for Context {
    fn default() -> Self {
        Context {
            stack: VecDeque::default(),
            status: ParserStatus::Start,

            num_open_braces: 0,
            num_open_brackets: 0,
        }
    }
}

impl Context {
    /// The number of left brackets ([) encountered without
    /// corresponding right brackets, at this point in the parse.
    #[must_use]
    pub fn num_open_brackets(&self) -> usize {
        self.num_open_brackets
    }

    /// The number of left braces ({) encountered without
    /// corresponding right braces, at this point in the parse.
    #[must_use]
    pub fn num_open_braces(&self) -> usize {
        self.num_open_braces
    }

    /// The `ParserStatus`.
    #[must_use]
    pub fn parser_status(&self) -> ParserStatus {
        self.status
    }

    /// Update the parser status.
    pub(crate) fn update_status(&mut self, status: ParserStatus) {
        self.status = status;
    }

    /// Add an enclosing bracket, brace to the stack.
    pub(crate) fn add_enclosing(&mut self, enclosing: Enclosing) {
        self.stack.push_back(enclosing);
    }

    /// Read the last Enclosing.
    #[must_use]
    pub fn last_enclosing(&self) -> Option<Enclosing> {
        self.stack.back().copied()
    }

    /// Remove an enclosing bracket, brace from the stack.
    pub(crate) fn remove_last_enclosing(&mut self) -> Option<Enclosing> {
        self.stack.pop_back()
    }

    pub(crate) fn inc_braces(&mut self) {
        self.num_open_braces += 1;
    }

    pub(crate) fn dec_braces(&mut self) {
        self.num_open_braces -= 1;
    }

    pub(crate) fn inc_brackets(&mut self) {
        self.num_open_brackets += 1;
    }

    pub(crate) fn dec_brackets(&mut self) {
        self.num_open_brackets -= 1;
    }
}

/// Implement this trait to handle parse events.
pub trait Handler {
    /// Latest parsed value was a null.
    fn handle_null(&mut self, ctx: &Context) -> Status;

    /// Latest parsed value was a double.
    fn handle_double(&mut self, ctx: &Context, val: f64) -> Status;

    /// Latest parsed value was an int.
    fn handle_int(&mut self, ctx: &Context, val: i64) -> Status;

    /// Latest parsed value was a bool.
    fn handle_bool(&mut self, ctx: &Context, val: bool) -> Status;

    /// Latest parsed value was a string.
    fn handle_string(&mut self, ctx: &Context, val: &str) -> Status;

    /// Latest parsed value was a left curly brace ({).
    fn handle_start_map(&mut self, ctx: &Context) -> Status;

    /// Latest parsed value was a right curly brace (}).
    fn handle_end_map(&mut self, ctx: &Context) -> Status;

    /// Latest parsed value was a key to a JSON object.
    fn handle_map_key(&mut self, ctx: &Context, key: &str) -> Status;

    /// Latest parsed value was a left bracket ([).
    fn handle_start_array(&mut self, ctx: &Context) -> Status;

    /// Latest parsed value was a right bracket ([).
    fn handle_end_array(&mut self, ctx: &Context) -> Status;
}
