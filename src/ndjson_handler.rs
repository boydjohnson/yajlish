//! This Handler implementation converts JSON
//! with an array into new-line delimited JSON.
//!
//! Requires feature `ndjson`.
//!

use crate::{common::ParserStatus, Context, Handler, Status};
use std::io::Write;

/// Parser.
pub struct NdJsonHandler<OUT> {
    out: OUT,

    context: NdjsonContext,
}

#[derive(Debug)]
enum NdjsonContext {
    Selecting(Select),
    Writing(WritingCtx),
}

#[derive(Debug)]
struct WritingCtx {
    num_open_braces: usize,
    num_open_brackets: usize,
    inside_array: bool,
    first: bool,
}

impl WritingCtx {
    fn new(braces: usize, brackets: usize) -> Self {
        WritingCtx {
            num_open_braces: braces,
            num_open_brackets: brackets,
            inside_array: false,
            first: true,
        }
    }

    fn first_context() -> Self {
        WritingCtx {
            num_open_braces: 0,
            num_open_brackets: 1,
            inside_array: false,
            first: true,
        }
    }

    fn is_at_correct_location(&self, ctx: &Context) -> bool {
        ctx.num_open_brackets() == self.num_open_brackets
            && ctx.num_open_braces() == self.num_open_braces
    }

    fn is_inside_array(&self) -> bool {
        self.inside_array
    }

    fn set_inside_array(&mut self) {
        self.inside_array = true;
    }

    fn is_first_value(&self) -> bool {
        self.is_inside_array() && self.first
    }

    fn first(&mut self) {
        self.first = false;
    }
}

#[derive(Debug)]
struct Select {
    stack: Vec<Box<dyn IsLocation + Send + Sync>>,
    selectors: Vec<Selector>,
    i: usize,
}

impl Select {
    fn is_identifier_selector_selection(&self, ctx: &Context, val: &str) -> bool {
        match self.selectors.last() {
            Some(Selector::Identifier(ident)) => {
                self.stack
                    .last()
                    .map_or(false, |s| (**s).is_correct_location(ctx))
                    && ident == val
            }
            _ => false,
        }
    }

    fn is_array_index_selector_selection(&self, ctx: &Context) -> bool {
        match self.selectors.last() {
            Some(Selector::Index(index)) => {
                self.stack
                    .last()
                    .map_or(false, |s| (**s).is_correct_location(ctx))
                    && self.i == *index
            }
            _ => false,
        }
    }

    fn remove_last(&mut self) -> bool {
        self.selectors.pop();
        self.stack.pop();
        self.i = 0;
        self.stack.is_empty() && self.selectors.is_empty()
    }

    fn new_array_location(&mut self, ctx: &Context) {
        if self
            .stack
            .last()
            .map_or(false, |s| (**s).is_correct_location(ctx))
            && ctx.parser_status() == ParserStatus::ArrayNeedVal
        {
            self.i += 1;
        }
    }
}

trait IsLocation: std::fmt::Debug {
    fn is_correct_location(&self, ctx: &Context) -> bool;
}

#[derive(Debug)]
struct ObjSelector {
    num_open_braces: usize,
}

impl IsLocation for ObjSelector {
    fn is_correct_location(&self, ctx: &Context) -> bool {
        self.num_open_braces + 1 == ctx.num_open_braces()
    }
}

#[derive(Debug)]
struct ArraySelector {
    num_open_brackets: usize,
}

impl IsLocation for ArraySelector {
    fn is_correct_location(&self, ctx: &Context) -> bool {
        self.num_open_brackets + 1 == ctx.num_open_brackets()
    }
}

impl<OUT> NdJsonHandler<OUT>
where
    OUT: Write,
{
    /// Constructor.
    pub fn new(out: OUT, selectors: Vec<Selector>) -> Self {
        let is_locations: Vec<Box<dyn IsLocation + Send + Sync>> = (0..selectors.len())
            .fold(
                vec![],
                |mut sel: Vec<Box<dyn IsLocation + Send + Sync + 'static>>, i| match selectors[i] {
                    Selector::Identifier(_) => {
                        let num_open_braces = (0..i).fold(0, |mut acc, j| {
                            if let Selector::Identifier(_) = selectors[j] {
                                acc += 1;
                            }
                            acc
                        });
                        sel.push(Box::new(ObjSelector { num_open_braces }));
                        sel
                    }
                    Selector::Index(_) => {
                        let num_open_brackets = (0..i).fold(0, |mut acc, j| {
                            if let Selector::Index(_) = selectors[j] {
                                acc += 1;
                            }
                            acc
                        });
                        sel.push(Box::new(ArraySelector { num_open_brackets }));
                        sel
                    }
                },
            )
            .into_iter()
            .rev()
            .collect();

        let context = if is_locations.is_empty() {
            NdjsonContext::Writing(WritingCtx::first_context())
        } else {
            NdjsonContext::Selecting(Select {
                stack: is_locations,
                selectors: selectors.into_iter().rev().collect(),
                i: 0,
            })
        };

        NdJsonHandler {
            out,
            context: context,
        }
    }

    fn increment_selectors(&mut self, ctx: &Context) {
        let update = match &mut self.context {
            NdjsonContext::Selecting(ref mut selecting) => selecting.remove_last(),
            NdjsonContext::Writing(_) => false,
        };

        if update {
            self.context = NdjsonContext::Writing(WritingCtx::new(
                ctx.num_open_braces(),
                ctx.num_open_brackets() + 1,
            ));
        }
    }

    fn map_key(&mut self, ctx: &Context, val: &str) {
        match &mut self.context {
            NdjsonContext::Selecting(ref mut select) => {
                if select.is_identifier_selector_selection(ctx, val) {
                    self.increment_selectors(ctx);
                }
            }
            NdjsonContext::Writing(writing) => {
                if !writing.is_at_correct_location(ctx)
                    && ctx.parser_status() == ParserStatus::MapNeedKey
                {
                    self.out.write_all(b",").expect("Unable to write");
                } else if writing.is_inside_array() && writing.is_at_correct_location(ctx) {
                    if writing.is_first_value() {
                        writing.first()
                    } else {
                        self.out.write_all(b"\n").expect("Unable to write");
                    }
                }
                write!(self.out, "\"{}\": ", val).expect("Unable to write to stdout")
            }
        }
    }

    fn handle_value<T: std::fmt::Display>(&mut self, ctx: &Context, val: T) -> Status {
        match &mut self.context {
            NdjsonContext::Selecting(ref mut select) => {
                select.new_array_location(ctx);
            }
            NdjsonContext::Writing(writing) => {
                if !writing.is_at_correct_location(ctx)
                    && ctx.parser_status() == ParserStatus::ArrayNeedVal
                {
                    self.out.write_all(b",").expect("Unable to write");
                } else if writing.is_inside_array() && writing.is_at_correct_location(ctx) {
                    if writing.is_first_value() {
                        writing.first()
                    } else {
                        self.out.write_all(b"\n").expect("Unable to write");
                    }
                }

                write!(self.out, "{}", val).expect("Unable to write to stdout");
            }
        }
        Status::Continue
    }

    fn map_start(&mut self, ctx: &Context) {
        match &mut self.context {
            NdjsonContext::Selecting(ref mut select) => {
                select.new_array_location(ctx);
                if select.is_array_index_selector_selection(ctx) {
                    self.increment_selectors(ctx);
                }
            }
            NdjsonContext::Writing(writing) => {
                if !writing.is_at_correct_location(ctx)
                    && ctx.parser_status() == ParserStatus::ArrayNeedVal
                {
                    self.out.write_all(b",").expect("Unable to write");
                } else if writing.is_at_correct_location(ctx) {
                    if writing.is_first_value() {
                        writing.first()
                    } else {
                        self.out.write_all(b"\n").expect("Unable to write");
                    }
                }
                self.out.write_all(b"{ ").expect("Unable to write");
            }
        }
    }

    fn map_end(&mut self, ctx: &Context) {
        if let NdjsonContext::Writing(writing) = &self.context {
            self.out.write_all(b" }").expect("Unable to write");
            if writing.is_at_correct_location(ctx) {
                self.out
                    .write_all(b"\n")
                    .expect("Unable to write to stdout");
            }
        }
    }

    fn array_start(&mut self, ctx: &Context) {
        if let NdjsonContext::Selecting(ref mut select) = &mut self.context {
            select.new_array_location(ctx);
            if select.is_array_index_selector_selection(ctx) {
                self.increment_selectors(ctx);
            }
        }

        if let NdjsonContext::Writing(writing) = &mut self.context {
            if writing.is_inside_array() {
                if !writing.is_at_correct_location(ctx)
                    && ctx.parser_status() == ParserStatus::ArrayNeedVal
                {
                    self.out.write_all(b",").expect("Unable to write");
                } else if writing.is_at_correct_location(ctx) {
                    if writing.is_first_value() {
                        writing.first()
                    } else {
                        self.out.write_all(b"\n").expect("Unable to write");
                    }
                }
                self.out.write_all(b"[").expect("Unable to write");
            } else {
                writing.set_inside_array();
            }
        }
    }

    fn array_end(&mut self, ctx: &Context) -> Status {
        if let NdjsonContext::Writing(writing) = &self.context {
            if writing.is_inside_array() && writing.is_at_correct_location(ctx) {
                self.out
                    .write_all(b"\n")
                    .expect("Unable to write to stdout");
            }

            if writing.is_at_correct_location(ctx) {
                return Status::Abort;
            }

            self.out.write_all(b"]").expect("Unable to write");
            Status::Continue
        } else {
            Status::Continue
        }
    }
}

impl<OUT: Write> Handler for NdJsonHandler<OUT> {
    fn handle_null(&mut self, ctx: &Context) -> Status {
        self.handle_value(ctx, "null")
    }

    fn handle_double(&mut self, ctx: &Context, val: f64) -> Status {
        self.handle_value(ctx, val)
    }

    fn handle_int(&mut self, ctx: &Context, val: i64) -> Status {
        self.handle_value(ctx, val)
    }

    fn handle_bool(&mut self, ctx: &Context, val: bool) -> Status {
        self.handle_value(ctx, val)
    }

    fn handle_string(&mut self, ctx: &Context, val: &str) -> Status {
        self.handle_value(ctx, &format!("\"{}\"", val))
    }

    fn handle_start_map(&mut self, ctx: &Context) -> Status {
        self.map_start(ctx);
        Status::Continue
    }

    fn handle_end_map(&mut self, ctx: &Context) -> Status {
        self.map_end(ctx);
        Status::Continue
    }

    fn handle_map_key(&mut self, ctx: &Context, key: &str) -> Status {
        self.map_key(ctx, key);
        Status::Continue
    }

    fn handle_start_array(&mut self, ctx: &Context) -> Status {
        self.array_start(ctx);

        Status::Continue
    }

    fn handle_end_array(&mut self, ctx: &Context) -> Status {
        self.array_end(ctx)
    }
}

/// Refers to either a key or an index in an array.
#[derive(Debug, PartialEq, Clone, Hash, PartialOrd, Ord, Eq)]
pub enum Selector {
    /// A key in a JSON object.
    Identifier(String),
    /// A JSON Array value by index.
    Index(usize),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Parser;
    use pretty_assertions::assert_eq;
    use std::io::BufReader;

    fn assert_ndjson(input: &[u8], selectors: Vec<Selector>, output: &[u8]) {
        let mut input = BufReader::new(input);

        let mut out = vec![];

        let mut handler = NdJsonHandler::new(&mut out, selectors);

        let mut parser = Parser::new(&mut handler);

        parser.parse::<BufReader<&[u8]>>(&mut input).unwrap();

        assert_eq!(out, output);
    }

    #[test]
    fn test_array_inside_of_array() {
        assert_ndjson(
            "[{ \"foo\": [1,2,3] },{ \"foo\": [5,4,5] }]".as_bytes(),
            vec![],
            "{ \"foo\": [1,2,3] }\n{ \"foo\": [5,4,5] }\n".as_bytes(),
        );
    }

    #[test]
    fn test_array_of_data() {
        assert_ndjson(
            "[{ \"foo\": true },{ \"foo\": false }]".as_bytes(),
            vec![],
            "{ \"foo\": true }\n{ \"foo\": false }\n".as_bytes(),
        );
    }

    #[test]
    fn test_spurious_key_before_correct_key() {
        assert_ndjson(
            "{ \"foo\": [1,2,3], \"bar\": { \"data\": 10}, \"data\": [10.4,4.4, 5.42] }".as_bytes(),
            vec![Selector::Identifier("data".to_string())],
            "10.4\n4.4\n5.42\n".as_bytes(),
        );
    }

    #[test]
    fn test_key_as_part_of_object_in_array() {
        assert_ndjson(
            "{ \"foo\": [{ \"bar\": { \"baz\": [null, true, false], \"data\": [6, 6.5, null]}}]}"
                .as_bytes(),
            vec![
                Selector::Identifier("foo".to_owned()),
                Selector::Index(0),
                Selector::Identifier("bar".to_owned()),
                Selector::Identifier("data".to_owned()),
            ],
            "6\n6.5\nnull\n".as_bytes(),
        );
    }

    #[test]
    fn test_selector_index() {
        assert_ndjson(
            "{ \"foo\": [[1,2,3], [8.68,null,2.667]]}".as_bytes(),
            vec![Selector::Identifier("foo".to_owned()), Selector::Index(1)],
            "8.68\nnull\n2.667\n".as_bytes(),
        );
    }

    #[test]
    fn test_objects_in_array() {
        assert_ndjson(
            "{ \"foo\": [{ \"bar\": 10}, {\"bar\": 11 }]}".as_bytes(),
            vec![Selector::Identifier("foo".to_owned())],
            "{ \"bar\": 10 }\n{ \"bar\": 11 }\n".as_bytes(),
        )
    }

    #[test]
    fn test_basic_success() {
        assert_ndjson(
            "{ \"foo\": [1, 2, 3] }".as_bytes(),
            vec![Selector::Identifier("foo".to_owned())],
            "1\n2\n3\n".as_bytes(),
        );
    }

    #[test]
    fn test_array_values_of_objects() {
        assert_ndjson(
            "{ \"foo\": [{ \"bar\": [false, null, 10.5, 50]}, { \"bar\": [true,\n 10.4578, null, 60] }]}".as_bytes(),
            vec![Selector::Identifier("foo".to_owned())],
            "{ \"bar\": [false,null,10.5,50] }\n{ \"bar\": [true,10.4578,null,60] }\n".as_bytes()
        );
    }

    #[test]
    fn test_double_index_selector() {
        assert_ndjson(
            "{ \"foo\": [[null,\"foo\",\"bar\"], [{ \"bar\": { \"bar\": [{ \"data\": [1,false,null,5.6]}]}}] }".as_bytes(),
            vec![Selector::Identifier("foo".to_owned()), Selector::Index(1), Selector::Identifier("bar".to_owned()), Selector::Identifier("bar".to_owned()), Selector::Index(0), Selector::Identifier("data".to_owned())],
            "1\nfalse\nnull\n5.6\n".as_bytes()
        );
    }

    #[test]
    fn test_complex() {
        assert_ndjson(
            "{ \"gauss\": [{ \"foo\": null}, [{ \"feynman\": [{ \"foo\": [1, false, \"bar\"]}]}]]}"
                .as_bytes(),
            vec![
                Selector::Identifier("gauss".to_owned()),
                Selector::Index(1),
                Selector::Identifier("feynman".to_owned()),
                Selector::Identifier("foo".to_owned()),
            ],
            "1\nfalse\n\"bar\"\n".as_bytes(),
        );
    }

    #[test]
    fn test_strings_in_array_in_array() {
        assert_ndjson(
            "{ \"gauss\": [false, [\"cauchey\", \"feynman\", \"riemann\"], 1, 2, true]}".as_bytes(),
            vec![Selector::Identifier("gauss".to_owned())],
            "false\n[\"cauchey\",\"feynman\",\"riemann\"]\n1\n2\ntrue\n".as_bytes(),
        );
    }
}
