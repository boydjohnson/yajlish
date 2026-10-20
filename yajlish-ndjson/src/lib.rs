//! This Handler implementation converts JSON
//! with an array into new-line delimited JSON.
//!

use std::io::Write;
use yajlish::{decode_str, Context, Handler, Number, ParserStatus, Status};

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
    /// `handle_double` was called directly with an infinite or NaN value,
    /// which cannot be written as JSON. Parsing never returns this: numbers
    /// from the parser arrive through `handle_number` and are copied as
    /// their input text, so `1e400` is written as `1e400`.
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

/// Finds the selected array: the value reached from the document root by
/// following every selector. Tracks the containers open around the current
/// position and, for each, whether it is itself on the selector path, so a
/// match is always anchored to the container the previous step matched.
#[derive(Debug)]
struct Select {
    selectors: Vec<Selector>,
    /// Open containers, outermost first. `frames[p]` is the value at
    /// `selectors[..p]` when `on_path` is set, and its children are matched
    /// against `selectors[p]`.
    frames: Vec<Frame>,
}

#[derive(Debug)]
struct Frame {
    on_path: bool,
    is_map: bool,
    /// Maps: whether the current key is this frame's selector.
    key_matches: bool,
    /// Arrays: the index of the next element.
    next_index: usize,
}

/// What a value that is starting is, relative to the selector path.
#[derive(Debug, PartialEq)]
enum Place {
    /// The selected value itself.
    Target,
    /// The value at a proper prefix of the path: the next selector applies
    /// to its children.
    OnPath(usize),
    Off,
}

impl Select {
    /// A value starts at the current position: where is it?
    fn value_start(&mut self) -> Place {
        let k = self.frames.len();
        let on_path = match self.frames.last_mut() {
            // The document root is at the empty path.
            None => true,
            Some(frame) if frame.is_map => frame.on_path && frame.key_matches,
            Some(frame) => {
                let index = frame.next_index;
                frame.next_index += 1;
                frame.on_path && self.selectors.get(k - 1) == Some(&Selector::Index(index))
            }
        };
        if !on_path {
            Place::Off
        } else if k == self.selectors.len() {
            Place::Target
        } else {
            Place::OnPath(k)
        }
    }

    /// A container starts at `place`.
    fn push(&mut self, place: &Place, is_map: bool) {
        // On the path only if it is the kind of container the next
        // selector looks into.
        let on_path = match place {
            Place::OnPath(k) => matches!(
                (&self.selectors[*k], is_map),
                (Selector::Identifier(_), true) | (Selector::Index(_), false)
            ),
            Place::Target | Place::Off => false,
        };
        self.frames.push(Frame {
            on_path,
            is_map,
            key_matches: false,
            next_index: 0,
        });
    }

    fn key(&mut self, raw: &str) {
        let p = self.frames.len().wrapping_sub(1);
        let selector = self.selectors.get(p);
        if let Some(frame) = self.frames.last_mut() {
            frame.key_matches = frame.on_path
                && matches!(selector, Some(Selector::Identifier(ident))
                    if decode_str(raw).is_ok_and(|key| key == ident.as_str()));
        }
    }

    fn container_end(&mut self) {
        self.frames.pop();
    }
}

impl<OUT> NdJsonHandler<OUT>
where
    OUT: Write,
{
    /// A handler that writes the elements of the array that `selectors`
    /// select. They are a path from the document root, followed exactly:
    /// `Identifier("a")` is the value of key `a` in an object, `Index(1)`
    /// the second element of an array, and an empty path selects the root.
    /// The value at the end of the path must be an array; if it is
    /// anything else, or the path is not in the document, `finish` returns
    /// `SelectorNotFound`. With repeated keys, the first array found at
    /// the path is selected.
    pub fn new(out: OUT, selectors: Vec<Selector>) -> Self {
        let context = NdjsonContext::Selecting(Select {
            selectors,
            frames: vec![],
        });

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
    /// `Io` if a write failed, `SelectorNotFound` if no array was selected,
    /// `NonFiniteNumber` if `handle_double` was called directly with a
    /// value that cannot be written as JSON.
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

    fn map_key(&mut self, ctx: &Context, val: &str) {
        match &mut self.context {
            NdjsonContext::Selecting(ref mut select) => select.key(val),
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
                // A scalar is never the selected array, even at the path.
                select.value_start();
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
                // An object is never the selected array, even at the path.
                let place = select.value_start();
                select.push(&place, true);
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
        if let NdjsonContext::Selecting(select) = &mut self.context {
            select.container_end();
        }
        if let NdjsonContext::Writing(writing) = &self.context {
            self.sink.put(b"}");
            if writing.is_at_correct_location(ctx) {
                self.sink.put(b"\n");
            }
        }
    }

    fn array_start(&mut self, ctx: &Context) {
        if let NdjsonContext::Selecting(ref mut select) = &mut self.context {
            let place = select.value_start();
            if place == Place::Target {
                // The selected array: write its elements, one per line.
                // The parser counts this bracket after the callback.
                let mut writing =
                    WritingCtx::new(ctx.num_open_braces(), ctx.num_open_brackets() + 1);
                writing.set_inside_array();
                self.context = NdjsonContext::Writing(writing);
                self.found = true;
                return;
            }
            select.push(&place, false);
            return;
        }

        if let NdjsonContext::Writing(writing) = &mut self.context {
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
        }
    }

    fn array_end(&mut self, ctx: &Context) -> Status {
        if let NdjsonContext::Selecting(select) = &mut self.context {
            select.container_end();
        }
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

    /// Writes `raw`, the number exactly as the input spelled it. The parser
    /// only delivers RFC 8259 numbers, so it is always valid JSON.
    fn handle_number(&mut self, ctx: &Context, raw: &str, _val: Number) -> Status {
        self.handle_value(ctx, raw)
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
#[cfg(any(test, kani))]
mod verify;

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
        // `foo[1]` is an array holding the object, so the path needs `[0]`
        // too: selectors follow the document exactly, since 0.1.
        assert_ndjson(
            "{ \"foo\": [[null,\"foo\",\"bar\"], [{ \"bar\": { \"bar\": [{ \"data\": [1,false,null,5.6]}]}}]] }".as_bytes(),
            vec![Selector::Identifier("foo".to_owned()), Selector::Index(1), Selector::Index(0), Selector::Identifier("bar".to_owned()), Selector::Identifier("bar".to_owned()), Selector::Index(0), Selector::Identifier("data".to_owned())],
            "1\nfalse\nnull\n5.6\n".as_bytes()
        );
    }

    #[test]
    fn test_complex() {
        assert_ndjson(
            "{ \"gauss\": [{ \"foo\": null}, [{ \"feynman\": [{ \"foo\": [1, false, \"bar\"]}]}]]}"
                .as_bytes(),
            // `gauss[1]` and `feynman` are arrays holding the objects, so
            // the path names their `[0]`s too: selectors follow the
            // document exactly, since 0.1.
            vec![
                Selector::Identifier("gauss".to_owned()),
                Selector::Index(1),
                Selector::Index(0),
                Selector::Identifier("feynman".to_owned()),
                Selector::Index(0),
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
