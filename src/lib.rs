//
// Copyright 2020 Boyd Johnson
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
// ------------------------------------------------------------------------------
//

//! yajlish is a low-level, event-based json parser based (loosely) on [yajl](https://github.com/yajl/yajl).
//! Use
//! ```
//!     use yajlish::{Context, Handler, Status};
//!
//!     pub struct FooCountHandler {
//!         count: usize,
//!     }
//!
//!     impl Handler for FooCountHandler {
//!         
//!         fn handle_map_key(&mut self, _ctx: &Context, key: &str) -> Status {
//!             if key == "\"foo\"" {
//!                 self.count += 1;
//!             }
//!             Status::Continue
//!         }
//!     
//!         fn handle_null(&mut self, _ctx: &Context) -> Status {
//!             Status::Continue
//!         }
//!
//!         fn handle_bool(&mut self, _ctx: &Context, boolean: bool) -> Status {
//!             Status::Continue
//!         }
//!
//!         fn handle_double(&mut self, _ctx: &Context, val: f64) -> Status {
//!             Status::Continue
//!         }
//!         
//!         fn handle_int(&mut self, _ctx: &Context, val: i64) -> Status {
//!             Status::Continue
//!         }
//!
//!         fn handle_string(&mut self, _ctx: &Context, val: &str) -> Status {
//!             Status::Continue
//!         }
//!
//!         fn handle_start_map(&mut self, _ctx: &Context) -> Status {
//!             Status::Continue
//!         }
//!         
//!         fn handle_start_array(&mut self, _ctx: &Context) -> Status {
//!             Status::Continue
//!         }
//!
//!         fn handle_end_map(&mut self, _ctx: &Context) -> Status {
//!             Status::Continue
//!         }
//!
//!         fn handle_end_array(&mut self, _ctx: &Context) -> Status {
//!             Status::Continue
//!         }
//!     }
//! ```

mod common;
mod lexer;
#[cfg(feature = "ndjson")]
pub mod ndjson_handler;
mod parser;

pub use common::{Context, Enclosing, Handler, ParserStatus, Status};
pub use parser::Parser;
