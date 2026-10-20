# Yajlish - Rust event-based JSON parser

Write tools to parse JSON when you don't know its exact structure **and** can't load it all into memory.

![ci](https://github.com/boydjohnson/yajlish/actions/workflows/ci.yaml/badge.svg)

This repository holds three crates:

| Crate | What it does |
|---|---|
| [`yajlish`](https://docs.rs/yajlish) | The streaming parser. You implement `Handler`; the parser calls it once per JSON event. |
| [`yajlish-ndjson`](yajlish-ndjson) | A ready-made `Handler` that turns an array inside a JSON document into newline-delimited JSON. |
| [`yajlish-schema`](yajlish-schema) | A macro that generates a `Handler` from JSON Schemas (draft 2020-12): it validates the stream and builds typed Rust values. |

What you get from `yajlish`:

- based loosely on [yajl](https://github.com/yajl/yajl)
- strict RFC 8259 lexing, built on [logos](https://crates.io/crates/logos), and the grammar is checked before your handler sees each token, so a handler only ever sees a prefix of a valid document
- no panics on any input, and bounded memory: one token at a time, capped at 64 MiB by default (`Parser::with_max_token_len`)
- input can arrive in pieces: split it across `parse` calls anywhere, even inside a token
- byte offsets in every error, and `Context::offset()` for the current event
- strings, keys and numbers are also delivered as their raw JSON text, so they can be copied verbatim

### Libraries you probably need instead of this library

- [serde_json](https://crates.io/crates/serde_json)
- [rust-json](https://crates.io/crates/json)

If the document fits in memory and you know its shape, use one of those.

## yajlish

The examples below all use this document: a list of orders, which in real life
is a file too large to load.

```json
{
  "generated": "2026-10-20",
  "orders": [
    {"id": 1, "customer": "ada", "amount": 12.50, "note": "gift \"wrap\""},
    {"id": 2, "customer": "grace", "amount": 3},
    {"id": 3, "customer": "ada", "amount": 7.25}
  ]
}
```

### A complete example: totals per customer

A `Handler` gets one call per event: `handle_start_map`, `handle_map_key`,
`handle_string`, `handle_number`, `handle_end_array`, and so on. Each call
gets a `Context` that says where the parser is (how many `{` and `[` are
open, the byte offset) and returns `Status::Continue` or `Status::Abort`.

This handler adds up `amount` per `customer` over the elements of the
top-level `orders` array, without ever holding more than one order.

```rust
use std::collections::BTreeMap;
use yajlish::{decode_str, Context, Handler, Number, Parser, Status};

#[derive(Default)]
struct Totals {
    totals: BTreeMap<String, f64>,
    /// Inside the top-level "orders" array.
    in_orders: bool,
    /// The last key read at the top level, or inside the current order.
    key: String,
    customer: Option<String>,
    amount: Option<f64>,
}

impl Totals {
    /// The event is a member of an order: one `{` and one `[` are open.
    fn in_order(&self, ctx: &Context) -> bool {
        self.in_orders && ctx.num_open_braces() == 2 && ctx.num_open_brackets() == 1
    }
}

impl Handler for Totals {
    fn handle_map_key(&mut self, ctx: &Context, key: &str) -> Status {
        if ctx.num_open_brackets() == 0 || self.in_order(ctx) {
            // Keys arrive as raw JSON text, quotes and escapes included.
            // `decode_str` gives the value; a key that is not valid JSON
            // text cannot reach a handler, so this cannot fail.
            self.key = decode_str(key).unwrap().into_owned();
        }
        Status::Continue
    }

    fn handle_start_array(&mut self, ctx: &Context) -> Status {
        // The context is the one *before* this `[`.
        if ctx.num_open_braces() == 1 && ctx.num_open_brackets() == 0 && self.key == "orders" {
            self.in_orders = true;
        }
        Status::Continue
    }

    fn handle_end_array(&mut self, ctx: &Context) -> Status {
        if self.in_orders && ctx.num_open_braces() == 1 && ctx.num_open_brackets() == 1 {
            self.in_orders = false;
        }
        Status::Continue
    }

    fn handle_string(&mut self, ctx: &Context, val: &str) -> Status {
        if self.in_order(ctx) && self.key == "customer" {
            self.customer = Some(decode_str(val).unwrap().into_owned());
        }
        Status::Continue
    }

    // `handle_number` gets every number with its raw text, as the input
    // spelled it, and its value. Without it, the default calls
    // `handle_int` or `handle_double`.
    fn handle_number(&mut self, ctx: &Context, _raw: &str, val: Number) -> Status {
        if self.in_order(ctx) && self.key == "amount" {
            self.amount = Some(match val {
                Number::Int(i) => i as f64,
                Number::Double(d) => d,
                _ => return Status::Abort,
            });
        }
        Status::Continue
    }

    fn handle_end_map(&mut self, ctx: &Context) -> Status {
        // This `}` closes an order.
        if self.in_order(ctx) {
            if let (Some(customer), Some(amount)) = (self.customer.take(), self.amount.take()) {
                *self.totals.entry(customer).or_insert(0.0) += amount;
            }
        }
        Status::Continue
    }

    // Not called, since `handle_number` is overridden.
    fn handle_int(&mut self, _ctx: &Context, _val: i64) -> Status {
        Status::Continue
    }
    fn handle_double(&mut self, _ctx: &Context, _val: f64) -> Status {
        Status::Continue
    }

    fn handle_null(&mut self, _ctx: &Context) -> Status {
        Status::Continue
    }
    fn handle_bool(&mut self, _ctx: &Context, _val: bool) -> Status {
        Status::Continue
    }
    fn handle_start_map(&mut self, _ctx: &Context) -> Status {
        Status::Continue
    }
}

let input = br#"{
  "generated": "2026-10-20",
  "orders": [
    {"id": 1, "customer": "ada", "amount": 12.50, "note": "gift \"wrap\""},
    {"id": 2, "customer": "grace", "amount": 3},
    {"id": 3, "customer": "ada", "amount": 7.25}
  ]
}"#;

let mut totals = Totals::default();
let mut parser = Parser::new(&mut totals);
// `parse` takes any `BufRead`: a `BufReader<File>`, stdin, a socket.
// The input may arrive in pieces, split anywhere, even inside a token:
// each call continues where the last one stopped.
let (first, rest) = input.split_at(100);
parser.parse(&mut &first[..])?;
parser.parse(&mut &rest[..])?;
// The end of a reader only means "no more bytes for now".
// `finish_parse` marks the end of the input and checks that it was
// exactly one complete JSON document.
parser.finish_parse()?;

assert_eq!(totals.totals["ada"], 19.75);
assert_eq!(totals.totals["grace"], 3.0);

// Errors carry the byte offset where the problem was found.
let mut totals = Totals::default();
let mut parser = Parser::new(&mut totals);
let err = parser.parse(&mut &br#"{"orders": [1, 2,]}"#[..]).unwrap_err();
// "Error: Malformed Json at byte 17: unexpected RightBracket in state ArrayNeedVal"
assert_eq!(err.offset(), 17);
# Ok::<(), yajlish::ParseError>(())
```

To stop early, return `Status::Abort` from any handler method: `parse`
returns `Ok(())` and calls nothing more.

## yajlish-ndjson

`NdJsonHandler` writes each element of one array in the document as a line
of newline-delimited JSON. The array is picked by a selector: `d` for the
document root, then `.key` and `[index]` steps, like `d.orders` or
`d.data[0].rows`. Strings and numbers are copied byte for byte, so
`12.50` stays `12.50` and `1e400` stays `1e400`.

```rust
use yajlish::Parser;
use yajlish_ndjson::{NdJsonHandler, SelectorParser};

let input = br#"{
  "generated": "2026-10-20",
  "orders": [
    {"id": 1, "customer": "ada", "amount": 12.50, "note": "gift \"wrap\""},
    {"id": 2, "customer": "grace", "amount": 3},
    {"id": 3, "customer": "ada", "amount": 7.25}
  ]
}"#;

let selectors = SelectorParser::new().parse("d.orders")?;
// Any `std::io::Write`: a `BufWriter<File>`, stdout.
let mut handler = NdJsonHandler::new(Vec::new(), selectors);
let mut parser = Parser::new(&mut handler);
parser.parse(&mut &input[..])?;
parser.finish_parse()?;
// Reports write errors, and a selector that matched no array.
let out = handler.finish()?;

assert_eq!(
    String::from_utf8(out)?,
    r#"{"id":1,"customer":"ada","amount":12.50,"note":"gift \"wrap\""}
{"id":2,"customer":"grace","amount":3}
{"id":3,"customer":"ada","amount":7.25}
"#,
);
# Ok::<(), Box<dyn std::error::Error>>(())
```

Whitespace between tokens is dropped. By default the handler lets the
parser read to the end, so malformed JSON after the array is still an
error; `NdJsonHandler::stop_early()` stops as soon as the array closes.

The converter used to be yajlish's `ndjson` feature. Since 0.5.0, replace
`yajlish::ndjson_handler::` with `yajlish_ndjson::`.

## yajlish-schema

`yajlish_schema::handler!` reads JSON Schema files at compile time and
generates a Rust type per schema plus a `Handler` that validates the stream
against them and builds those types. Here is `order.schema.json`:

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "type": "object",
  "required": ["id", "customer", "amount"],
  "properties": {
    "id": { "type": "integer" },
    "customer": { "type": "string", "minLength": 1 },
    "amount": { "type": "number", "minimum": 0 },
    "note": { "type": "string" }
  },
  "additionalProperties": false
}
```

With `at = ".orders"`, each element of the `orders` array is a record, and
the callback runs once per record with the typed value or the reason it
does not match. Records are validated as their events stream in, and an
invalid record does not stop the parse.

```rust
use yajlish::Parser;
use yajlish_schema::Status;

yajlish_schema::handler! {
    /// Validates and builds each order.
    pub struct OrderHandler;
    at = ".orders";
    schemas {
        // Paths are relative to the crate's Cargo.toml.
        Order => "tests/schemas/order.schema.json",
    }
}

let input = br#"{
  "generated": "2026-10-20",
  "orders": [
    {"id": 1, "customer": "ada", "amount": 12.50, "note": "gift \"wrap\""},
    {"id": 2, "customer": "grace", "amount": -3},
    {"id": 3, "customer": "ada", "amount": 7.25}
  ]
}"#;

let mut orders = vec![];
let mut errors = vec![];
let mut handler = OrderHandler::new(|record| {
    match record {
        Ok(OrderRecord::Order(order)) => orders.push(order),
        Err(e) => errors.push(e),
    }
    Status::Continue
});
let mut parser = Parser::new(&mut handler);
parser.parse(&mut &input[..])?;
parser.finish_parse()?;
// Reports errors that are not about one record, such as `at`
// matching nothing.
handler.finish()?;

assert_eq!(orders.len(), 2);
assert_eq!(orders[0].customer, "ada");
assert_eq!(orders[0].note.as_deref(), Some("gift \"wrap\""));
assert_eq!(errors.len(), 1);
assert_eq!(errors[0].keyword, "minimum");
assert_eq!(errors[0].instance_path, "/amount");
# Ok::<(), Box<dyn std::error::Error>>(())
```

Other options: several schemas in one handler (`match = unique` or
`first`), `envelope = "file.json"` to validate the document around the
records, and `build = false` to validate only, in memory proportional to
nesting depth. See the [`yajlish-schema` docs](yajlish-schema/src/lib.rs)
for the options and the supported keywords. Unsupported keywords are a
compile error, never silently ignored.

## Verification

All three crates are fuzzed with [Bolero](https://github.com/camshaft/bolero)
and property-tested against `serde_json`. The lexer, the parser state
machine, `decode_str` and `yajlish-schema`'s number checks have
[Kani](https://github.com/model-checking/kani) proofs. `yajlish-schema`
also runs the official JSON Schema Test Suite and is tested against the
`jsonschema` crate.

# License

This library is licensed under the Apache 2.0 License.
