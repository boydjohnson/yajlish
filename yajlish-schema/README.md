# yajlish-schema

Validate a JSON stream against JSON Schemas (draft 2020-12), and build typed
Rust values from it, with a [`yajlish`](https://crates.io/crates/yajlish)
`Handler` generated at compile time.

Validation happens as events stream in, so a document can be checked
without loading it, and each element of a large array can be handled as a
separate record.

```toml
[dependencies]
yajlish = "0.5"
yajlish-schema = "0.1"
```

## Example

Given `tests/schemas/order.schema.json`:

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
handler.finish()?;

assert_eq!(orders.len(), 2);
assert_eq!(orders[0].customer, "ada");
assert_eq!(orders[0].note.as_deref(), Some("gift \"wrap\""));
assert_eq!(errors.len(), 1);
assert_eq!(errors[0].keyword, "minimum");
assert_eq!(errors[0].instance_path, "/amount");
# Ok::<(), Box<dyn std::error::Error>>(())
```

## What the macro generates

For `pub struct OrderHandler;` with `schemas { Order => ... }`:

- `Order`, a Rust type for the schema, plus types for the objects, enums
  and tuples inside it. Optional properties are `Option`s.
- `OrderRecord`, an enum with one variant per schema (the handler's name
  without `Handler`, plus `Record`).
- `OrderHandler<F>`, a `yajlish::Handler` that validates each record
  against every schema at once, builds the value of the schema it
  matches, and calls `F: FnMut(Result<OrderRecord, SchemaError>) -> Status`.
  Return `Status::Abort` from the callback to stop the parse.

A `SchemaError` names the failed `keyword`, the `instance_path` (a JSON
Pointer into the record), the `schema_path`, and the byte `offset` in the
stream. An invalid record does not stop the parse. After parsing, call
`finish()` on the handler: errors that are not about one record (the
envelope around `at`, an `at` path that matched nothing, a document that
ended early) are reported there.

## Options

- `at = ".key[0].other"`: the records are the elements of the array at that
  path. Without it the whole document is the one record, and the callback
  runs once, at the end of the stream.
- `envelope = "file.json"`: with `at`, validate the whole document against
  this schema too, without building it. Its errors go to `finish()`.
- `match = unique | first`: with several schemas, whether exactly one must
  match each record (the default), or the first that matches, in
  declaration order, wins.
- `build = false`: validate only. No types are generated, nothing is
  allocated per value, and the callback receives `OrderSchema`, a
  fieldless enum naming the schema that matched.
- `max_depth = 256`: nesting limit for recursive schemas.

## Memory

Validation streams: with `build = false`, memory is proportional to nesting
depth, whatever the document size. Building holds one record in memory, as
`serde_json` would, so for one large document use `build = false` or
`at = ...`. With several schemas, each record is built once, and a
candidate schema is dropped as soon as the record violates it. Give
multi-schema records a discriminating `const` property (such as
`"type": {"const": "Point"}`) and all but one candidate are gone as soon as
that property is read.

## Supported JSON Schema

Draft 2020-12 only: `type`, `enum`, `const`, `properties`, `required`,
`additionalProperties`, `prefixItems`, `items`, `minItems`, `maxItems`,
`minLength`, `maxLength`, `pattern`, `minimum`, `maximum`,
`exclusiveMinimum`, `exclusiveMaximum`, `multipleOf`, `allOf`, `anyOf`,
`oneOf`, `not`, `$ref` (to `#...` pointers in the same file, or to another
file of the same `schemas {}` block or `envelope`), `$defs`, and the
annotations `title`, `description`, `default`, `examples`, `$comment`,
`format`, `deprecated`, `readOnly`, `writeOnly`. `$id` is accepted at a
document's root.

Any other keyword is a compile error, never silently ignored.

## Features

- `pattern`: support the `pattern` keyword, using the
  [`regex`](https://crates.io/crates/regex) crate's syntax. Without it,
  `pattern` is a compile error.
- `serde`: derive `Serialize` and `Deserialize` on the generated types.

## License

Apache 2.0.
