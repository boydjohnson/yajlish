//! This Handler implementation converts JSON
//! with an array into new-line delimited JSON.
//!

use std::io::Write;
use yajlish::{decode_str, Context, Handler, ParserStatus, Status};

/// Writes each element of the selected array as one line of `OUT`.
///
/// After parsing, call [`NdJsonHandler::finish`]: write errors, a selector
/// that matched nothing, and values that cannot be written as JSON make the
/// handler abort the parse, and only `finish` reports them.
///
/// By default the handler keeps receiving (and ignoring) events after the
/// selected array closes, so that the parser checks the rest of the
/// document; call `Parser::finish_parse` too. [`NdJsonHandler::stop_early`]
/// opts out of that.
pub struct NdJsonHandler<OUT> {
    sink: Sink<OUT>,

    context: NdjsonContext,
    found: bool,
    stop_early: bool,
}

/// Why the conversion failed.
#[derive(Debug)]
#[non_exhaustive]
pub enum NdJsonError {
    /// Writing to the output failed.
    Io(std::io::Error),
    /// The selected key or index never appeared, or the value it selected
    /// is not an array.
    SelectorNotFound,
    /// A number too large for `f64`, which cannot be written as JSON.
    NonFiniteNumber,
}

impl std::fmt::Display for NdJsonError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NdJsonError::Io(e) => write!(f, "write error: {}", e),
            NdJsonError::SelectorNotFound => write!(f, "the selector matched no array"),
            NdJsonError::NonFiniteNumber => write!(f, "number out of range for f64"),
        }
    }
}

impl std::error::Error for NdJsonError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            NdJsonError::Io(e) => Some(e),
            _ => None,
        }
    }
}

/// The output plus the first error, after which nothing more is written.
struct Sink<OUT> {
    out: OUT,
    error: Option<NdJsonError>,
}

impl<OUT: Write> Sink<OUT> {
    fn put(&mut self, bytes: &[u8]) {
        if self.error.is_none() {
            if let Err(e) = self.out.write_all(bytes) {
                self.error = Some(NdJsonError::Io(e));
            }
        }
    }

    fn put_fmt(&mut self, args: std::fmt::Arguments<'_>) {
        if self.error.is_none() {
            if let Err(e) = self.out.write_fmt(args) {
                self.error = Some(NdJsonError::Io(e));
            }
        }
    }

    fn fail(&mut self, error: NdJsonError) {
        if self.error.is_none() {
            self.error = Some(error);
        }
    }
}

#[derive(Debug)]
enum NdjsonContext {
    Selecting(Select),
    Writing(WritingCtx),
    /// The selected array has closed.
    Done,
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
                    .is_some_and(|s| (**s).is_correct_location(ctx))
                    && decode_str(val).is_ok_and(|key| key == ident.as_str())
            }
            _ => false,
        }
    }

    fn is_array_index_selector_selection(&self, ctx: &Context) -> bool {
        match self.selectors.last() {
            Some(Selector::Index(index)) => {
                self.stack
                    .last()
                    .is_some_and(|s| (**s).is_correct_location(ctx))
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
            .is_some_and(|s| (**s).is_correct_location(ctx))
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
            sink: Sink { out, error: None },
            context,
            found: false,
            stop_early: false,
        }
    }

    /// Abort the parse as soon as the selected array closes, instead of
    /// letting the parser check the rest of the document. Malformed JSON
    /// after the array then goes unnoticed.
    #[must_use]
    pub fn stop_early(mut self) -> Self {
        self.stop_early = true;
        self
    }

    /// The output, or the error that stopped the conversion.
    ///
    /// # Errors
    /// `Io` if a write failed, `NonFiniteNumber` for a number that cannot be
    /// written as JSON, `SelectorNotFound` if no array was selected.
    pub fn finish(self) -> Result<OUT, NdJsonError> {
        if let Some(e) = self.sink.error {
            return Err(e);
        }
        if !self.found {
            return Err(NdJsonError::SelectorNotFound);
        }
        Ok(self.sink.out)
    }

    fn status(&self) -> Status {
        if self.sink.error.is_some() {
            Status::Abort
        } else {
            Status::Continue
        }
    }

    fn increment_selectors(&mut self, ctx: &Context) {
        let update = match &mut self.context {
            NdjsonContext::Selecting(ref mut selecting) => selecting.remove_last(),
            NdjsonContext::Writing(_) | NdjsonContext::Done => false,
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
                    self.sink.put(b",");
                } else if writing.is_inside_array() && writing.is_at_correct_location(ctx) {
                    if writing.is_first_value() {
                        writing.first()
                    } else {
                        self.sink.put(b"\n");
                    }
                }
                self.sink.put_fmt(format_args!("{}:", val));
            }
            NdjsonContext::Done => {}
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
                    self.sink.put(b",");
                } else if writing.is_inside_array() && writing.is_at_correct_location(ctx) {
                    if writing.is_first_value() {
                        writing.first()
                    } else {
                        self.sink.put(b"\n");
                    }
                }

                self.sink.put_fmt(format_args!("{}", val));
            }
            NdjsonContext::Done => {}
        }
        self.status()
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
                    self.sink.put(b",");
                } else if writing.is_at_correct_location(ctx) {
                    if writing.is_first_value() {
                        writing.first()
                    } else {
                        self.sink.put(b"\n");
                    }
                }
                self.sink.put(b"{");
            }
            NdjsonContext::Done => {}
        }
    }

    fn map_end(&mut self, ctx: &Context) {
        if let NdjsonContext::Writing(writing) = &self.context {
            self.sink.put(b"}");
            if writing.is_at_correct_location(ctx) {
                self.sink.put(b"\n");
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
                    self.sink.put(b",");
                } else if writing.is_at_correct_location(ctx) {
                    if writing.is_first_value() {
                        writing.first()
                    } else {
                        self.sink.put(b"\n");
                    }
                }
                self.sink.put(b"[");
            } else {
                writing.set_inside_array();
                self.found = true;
            }
        }
    }

    fn array_end(&mut self, ctx: &Context) -> Status {
        if let NdjsonContext::Writing(writing) = &self.context {
            // End the last line, if there was one: an empty array writes
            // nothing.
            if writing.is_inside_array() && writing.is_at_correct_location(ctx) && !writing.first {
                self.sink.put(b"\n");
            }

            if writing.is_at_correct_location(ctx) {
                self.context = NdjsonContext::Done;
                if self.stop_early {
                    return Status::Abort;
                }
                return self.status();
            }

            self.sink.put(b"]");
        }
        self.status()
    }
}

impl<OUT: Write> Handler for NdJsonHandler<OUT> {
    fn handle_null(&mut self, ctx: &Context) -> Status {
        self.handle_value(ctx, "null")
    }

    fn handle_double(&mut self, ctx: &Context, val: f64) -> Status {
        if !val.is_finite() {
            if let NdjsonContext::Writing(_) = self.context {
                self.sink.fail(NdJsonError::NonFiniteNumber);
                return Status::Abort;
            }
        }
        self.handle_value(ctx, val)
    }

    fn handle_int(&mut self, ctx: &Context, val: i64) -> Status {
        self.handle_value(ctx, val)
    }

    fn handle_bool(&mut self, ctx: &Context, val: bool) -> Status {
        self.handle_value(ctx, val)
    }

    fn handle_string(&mut self, ctx: &Context, val: &str) -> Status {
        self.handle_value(ctx, val)
    }

    fn handle_start_map(&mut self, ctx: &Context) -> Status {
        self.map_start(ctx);
        self.status()
    }

    fn handle_end_map(&mut self, ctx: &Context) -> Status {
        self.map_end(ctx);
        self.status()
    }

    fn handle_map_key(&mut self, ctx: &Context, key: &str) -> Status {
        self.map_key(ctx, key);
        self.status()
    }

    fn handle_start_array(&mut self, ctx: &Context) -> Status {
        self.array_start(ctx);
        self.status()
    }

    fn handle_end_array(&mut self, ctx: &Context) -> Status {
        self.array_end(ctx)
    }
}

/// Refers to either a key or an index in an array.
#[derive(Debug, PartialEq, Clone, Hash, PartialOrd, Ord, Eq)]
pub enum Selector {
    /// A key in a JSON object, as its decoded value: `Identifier("foo")`
    /// matches the key written `"foo"` and also one written with escapes,
    /// such as `"\u0066oo"`.
    Identifier(String),
    /// A JSON Array value by index.
    Index(usize),
}

pub mod selector_parser;

pub use self::selector_parser::{SelectorParseError, SelectorParser};

#[cfg(test)]
mod tests {
    use super::{
        selector_parser::{ArrayTokenParser, IdentifierTokenParser, SelectorParser},
        *,
    };
    use pretty_assertions::assert_eq;
    use std::io::BufReader;
    use yajlish::Parser;

    fn assert_ndjson(input: &[u8], selectors: Vec<Selector>, output: &[u8]) {
        let mut input = BufReader::new(input);

        let mut out = vec![];

        let mut handler = NdJsonHandler::new(&mut out, selectors);

        let mut parser = Parser::new(&mut handler);

        parser.parse::<BufReader<&[u8]>>(&mut input).unwrap();
        parser.finish_parse().unwrap();
        handler.finish().unwrap();

        assert_eq!(out, output);
    }

    #[test]
    fn test_array_inside_of_array() {
        assert_ndjson(
            "[{ \"foo\": [1,2,3] },{ \"foo\": [5,4,5] }]".as_bytes(),
            vec![],
            "{\"foo\":[1,2,3]}\n{\"foo\":[5,4,5]}\n".as_bytes(),
        );
    }

    #[test]
    fn test_empty_array_writes_nothing() {
        assert_ndjson(b"[]", vec![], b"");
        assert_ndjson(
            b"{ \"foo\": [] }",
            vec![Selector::Identifier("foo".to_owned())],
            b"",
        );
    }

    #[test]
    fn test_array_of_data() {
        assert_ndjson(
            "[{ \"foo\": true },{ \"foo\": false }]".as_bytes(),
            vec![],
            "{\"foo\":true}\n{\"foo\":false}\n".as_bytes(),
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
            "{\"bar\":10}\n{\"bar\":11}\n".as_bytes(),
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
            "{\"bar\":[false,null,10.5,50]}\n{\"bar\":[true,10.4578,null,60]}\n".as_bytes()
        );
    }

    #[test]
    fn test_double_index_selector() {
        assert_ndjson(
            "{ \"foo\": [[null,\"foo\",\"bar\"], [{ \"bar\": { \"bar\": [{ \"data\": [1,false,null,5.6]}]}}]] }".as_bytes(),
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

    #[test]
    fn test_selector_parser() {
        assert_eq!(
            IdentifierTokenParser::new().parse(".TOTAL_SALES"),
            Ok(Selector::Identifier("TOTAL_SALES".into()))
        );

        assert_eq!(
            IdentifierTokenParser::new().parse(".TOTAL_SALES"),
            Ok(Selector::Identifier("TOTAL_SALES".into()))
        );

        assert_eq!(ArrayTokenParser::new().parse("[4]"), Ok(Selector::Index(4)));

        assert_eq!(
            ArrayTokenParser::new().parse("[45]"),
            Ok(Selector::Index(45))
        );

        assert_eq!(SelectorParser::new().parse("d"), Ok(vec![]));

        assert_eq!(
            SelectorParser::new().parse("d[2]"),
            Ok(vec![Selector::Index(2)])
        );

        assert_eq!(
            SelectorParser::new().parse("d[2].manager"),
            Ok(vec![
                Selector::Index(2),
                Selector::Identifier("manager".into())
            ])
        );

        assert_eq!(
            SelectorParser::new().parse("d.TOTAL_SALES[4].manager"),
            Ok(vec![
                Selector::Identifier("TOTAL_SALES".into()),
                Selector::Index(4),
                Selector::Identifier("manager".into())
            ])
        );

        assert_eq!(
            SelectorParser::new().parse("d[4].TOTAL_SALES[2555].manager[4].salary.currency"),
            Ok(vec![
                Selector::Index(4),
                Selector::Identifier("TOTAL_SALES".into()),
                Selector::Index(2555),
                Selector::Identifier("manager".into()),
                Selector::Index(4),
                Selector::Identifier("salary".into()),
                Selector::Identifier("currency".into()),
            ])
        );
    }
}
