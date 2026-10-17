# Changelog

## yajlish 0.5.0, yajlish-ndjson 0.1.0, yajlish-schema 0.1.0 (unreleased)

### yajlish

- **Breaking:** the lexer is now our own, built on logos, replacing `json-tools`.
  It is strict RFC 8259: leading zeros, `1.`, `.5`, `-`, invalid escapes, raw
  control characters in strings and non-JSON whitespace are errors.
- **Breaking:** `ParseError` variants carry the byte offset of the problem
  (`MalformedJson { offset, msg }`, ...) and are `#[non_exhaustive]`;
  `offset()` and `message()` read them. `ParseError` is now exported.
- **Breaking:** the `ndjson` feature is gone. The converter moved to the
  `yajlish-ndjson` crate: replace `yajlish::ndjson_handler::` with `yajlish_ndjson::`.
- Reader errors are returned as `ParseError::ReadError` instead of looking like EOF.
- No input can make the parser panic. Tokens are capped at 64 MiB by default
  (`Parser::with_max_token_len`).
- Tokens split across reads lex the same as unsplit input.
- New `decode_str` decodes the raw string text handlers receive.
- New `Context::offset`, the byte offset of the current event.
- About 3x faster on the bundled fixtures.

### yajlish-ndjson 0.1.0

- The converter from yajlish 0.4's `ndjson` feature, now with:
  - `Selector::Identifier` holds the decoded key (`"features"`, not
    `"\"features\""`), so `SelectorParser` output works directly and escaped
    keys match.
  - `NdJsonHandler::finish()` reports write errors (no more panics),
    `SelectorNotFound`, and numbers that cannot be written as JSON (`1e400`).
  - The rest of the document is validated after the selected array; `stop_early()`
    restores the old behaviour.
  - An empty selected array writes nothing instead of a blank line.
  - The selector parser is hand-written; lalrpop and the build script are gone.

### yajlish-schema 0.1.0

- `yajlish_schema::handler!` generates a `yajlish::Handler` from JSON Schema
  2020-12 files that validates each record against every schema as it streams,
  builds the typed value of the schema it matched, and reports errors with
  instance path, schema path, keyword and byte offset.
