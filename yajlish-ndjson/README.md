# yajlish-ndjson

Convert an array inside a JSON document into newline-delimited JSON
([ndjson](https://github.com/ndjson/ndjson-spec)), streaming.

`NdJsonHandler` is a [`yajlish`](https://crates.io/crates/yajlish) `Handler`
that writes each element of one array in the document as a line of output.
It holds one token at a time, so the input can be far larger than memory.

```toml
[dependencies]
yajlish = "0.5"
yajlish-ndjson = "0.1"
```

## Example

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
// Any `BufRead`: a `BufReader<File>`, stdin. Input can also be fed in
// pieces, one `parse` call per piece.
parser.parse(&mut &input[..])?;
parser.finish_parse()?;
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

## Selectors

A selector is a path from the document root to the array: `d` for the root,
then any mix of `.key` and `[index]`.

| Selector | Selects |
|---|---|
| `d` | the document itself, which must be an array |
| `d.orders` | the value of key `orders` in the root object |
| `d.data[0].rows` | key `rows` of the first element of `data` |

`SelectorParser::parse` turns the string into a `Vec<Selector>`, or you can
build the `Selector::Identifier` and `Selector::Index` values yourself. Keys
match by their decoded value, so `.foo` also matches a key written with
escapes. The path is followed exactly from the root: a key with the same
name deeper in the document does not match.

## Output

- Strings, keys and numbers are copied byte for byte from the input:
  `12.50` stays `12.50`, `1e400` stays `1e400`, and escapes are kept as
  written.
- Whitespace between tokens is dropped, so each element is one line.
- Every line ends with `\n`, including the last.

## Errors

Call `NdJsonHandler::finish` after parsing. It returns the output, or:

- `NdJsonError::Io` if a write failed,
- `NdJsonError::SelectorNotFound` if the path is not in the document or
  the value there is not an array.

Malformed JSON is reported by the parser, with the byte offset of the
problem. By default the handler lets the parser read the whole document, so
malformed JSON after the array is still an error; call `stop_early()` on the
handler to stop as soon as the array closes instead.

## Upgrading from yajlish 0.4

This converter used to be yajlish's `ndjson` feature. Replace
`yajlish::ndjson_handler::` with `yajlish_ndjson::`. `Selector::Identifier`
now holds the decoded key (`"features"`, not `"\"features\""`), and numbers
are no longer rewritten through `f64`. See the
[changelog](https://github.com/boydjohnson/yajlish/blob/main/CHANGELOG.md).

## License

Apache 2.0.
