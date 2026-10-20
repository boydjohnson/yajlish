# Changelog

## yajlish 0.5.0, yajlish-ndjson 0.1.0, yajlish-schema 0.1.0 (2026-10-20)

### yajlish

- **Breaking:** the lexer is now our own, built on logos, replacing `json-tools`.
  It is strict RFC 8259: leading zeros, `1.`, `.5`, `-`, invalid escapes, raw
  control characters in strings and non-JSON whitespace are errors.
- **Breaking:** `ParseError` variants carry the byte offset of the problem
  (`MalformedJson { offset, msg }`, ...) and are `#[non_exhaustive]`;
  `offset()` and `message()` read them. `ParseError` is now exported.
- **Breaking:** the `ndjson` feature is gone. The converter moved to the
  `yajlish-ndjson` crate: replace `yajlish::ndjson_handler::` with `yajlish_ndjson::`.
- **Breaking:** the parser enforces the RFC 8259 grammar, not just the
  tokens. These are now `MalformedJson` errors: an empty or whitespace-only
  document, a `}` or `]` that closes nothing or the wrong container,
  trailing commas (`[1,]`, `{"a":1,}`), a `:` anywhere but after a key, a
  value directly after a value (`[1 2]`, `[][]`, two top-level documents),
  and a key with no value (`{"a"}`). JSONTestSuite's `n_*` files are all
  rejected now.
- Each token is checked before the handler sees it, so handlers are only
  called for a prefix of a valid document; a mismatched `}` or `]` no
  longer reaches `handle_end_map`/`handle_end_array`. A handler that returns
  `Status::Abort` always gets `Ok(())` from `parse`.
- **Breaking:** the input may be split across `parse` calls anywhere, even
  inside a token: `parse(b"[1")` then `parse(b"2]")` delivers one
  `handle_int(12)`. The end of a `parse` call's reader means "no more bytes
  for now"; only `finish_parse` marks the end of the input. So a token that
  could still grow at the end of a call is delivered later, and a document
  that is a bare top-level number (`12`) only delivers its event from
  `finish_parse`. Call `finish_parse` once all input is read; it checks that
  exactly one complete value was read, and content after it in a later
  `parse` call is an error. `with_max_token_len` bounds a token buffered
  across calls too. Before, the end of the first reader ended the document.
- Reader errors are returned as `ParseError::ReadError` instead of looking like EOF.
- No input can make the parser panic. Tokens are capped at 64 MiB by default
  (`Parser::with_max_token_len`).
- Tokens split across reads lex the same as unsplit input.
- New `decode_str` decodes the raw string text handlers receive.
- New `Handler::handle_number(ctx, raw, val)` gives a number's JSON text
  exactly as it appeared in the input, along with its value as a new public
  `Number` enum (`Int(i64)` or `Double(f64)`, `#[non_exhaustive]`). The
  default calls `handle_int` or `handle_double` with the same values as
  before, so existing handlers need no change.
- New `Context::offset`, the byte offset of the current event.
- About 3x faster on the bundled fixtures.

### yajlish-ndjson 0.1.0

- The converter from yajlish 0.4's `ndjson` feature, now with:
  - `Selector::Identifier` holds the decoded key (`"features"`, not
    `"\"features\""`), so `SelectorParser` output works directly and escaped
    keys match.
  - `NdJsonHandler::finish()` reports write errors (no more panics) and
    `SelectorNotFound`.
  - Numbers are written byte for byte as the input spelled them, through
    the new `Handler::handle_number`: `1.0`, `1e2` and `-0` keep their
    spelling, `12345678901234567890` and long decimals keep their digits,
    `1e-400` is no longer written as `0`, and `1e400` is written as is
    instead of failing. `NonFiniteNumber` is now only returned when
    `handle_double` is called directly with an infinite or NaN value.
  - The rest of the document is validated after the selected array; `stop_early()`
    restores the old behaviour.
  - An empty selected array writes nothing instead of a blank line.
  - The selector parser is hand-written; lalrpop and the build script are gone.
  - **Breaking:** selectors are a path followed exactly from the root.
    Before, a step only had to match at the right count of open braces or
    brackets, so arrays between object steps could be skipped
    (`d.gauss[1].feynman` matched `{"gauss": [_, [{"feynman": ...}]]}`; now
    that is `d.gauss[1][0].feynman`), and a match was not tied to the value
    the previous step matched: `d.a` on `{"a":{"x":[1]}}` wrote `{"x":1]}}`,
    and `d.a.b` on `{"a":{},"c":{"b":[1]}}` wrote `1` from `c.b`. Both are
    now `SelectorNotFound`, as is any path whose value is not an array.

### yajlish-schema 0.1.0

- `yajlish_schema::handler!` generates a `yajlish::Handler` from JSON Schema
  2020-12 files that validates each record against every schema as it streams,
  builds the typed value of the schema it matched, and reports errors with
  instance path, schema path, keyword and byte offset.
