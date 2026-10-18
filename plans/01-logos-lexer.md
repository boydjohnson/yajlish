# Plan 1: Replace `json-tools` with our own lexer built on `logos`

Status: draft · Target crate version: 0.5.0 · Depends on: nothing · Unblocks: [Plan 2](02-schema-handler-macro.md)

## Why

`Parser::parse` (`src/parser.rs`) drives `json_tools::Lexer` through a byte-at-a-time iterator (`Wrapper`). Reading the code, that setup has several properties that don't fit a high-integrity library:

| Problem | Where | Effect |
|---|---|---|
| I/O errors are swallowed | `Wrapper::next`, `src/parser.rs:362` (`fill_buf().ok()`) | A failing reader looks like EOF. `ParseError::ReadError` is never produced by `parse`. |
| Panics on lexer output we don't expect | `src/parser.rs:142`, `:147`, `:163` | A malformed number or a `Buffer::Span` aborts the process instead of returning `MalformedJson`. |
| Lexer strictness is someone else's | `json-tools` | We don't control whether raw control characters in strings, leading zeros, `-`, `1.`, `.5`, invalid escapes etc. are rejected. |
| One byte per iterator call, one `Vec<u8>` allocation per string/number token | `Wrapper`, `Buffer::MultiByte` | Slow, and every string event allocates. |
| No error position | `ParseError::MalformedJson(String)` | Users can't tell where a multi-GB stream went bad. |

Writing the lexer ourselves with `logos` gives us a strict RFC 8259 token grammar we own, zero-copy `&str` events borrowed from our own buffer, and errors with byte offsets.

## Goals

1. Strict RFC 8259 tokens: anything `json-tools` accepted that RFC 8259 forbids becomes a lexical error.
2. No panics reachable from input bytes or reader behaviour.
3. Bounded memory: the lexer holds at most one read buffer plus the longest in-flight token, and the longest token is capped (configurable).
4. Reader errors surface as `ParseError::ReadError`.
5. **Observable `Handler` behaviour unchanged for valid JSON.** Strings and keys are still delivered raw, with surrounding quotes and escapes intact (e.g. `"\"foo\""`), because the gold files, `NdJsonHandler` (which re-emits raw keys/strings) and the README example all depend on it. Every existing test passes without edits to `.gold` files.
6. Remove the `json-tools` dependency.

## Non-goals

- Changing the `Handler` trait signature. (Decoded strings are an additive follow-up, see M5.)
- Fixing parser state-machine strictness. That is a separate task; the issues found while reading are listed at the end so they don't get lost.

## Design

### Dependency

```toml
[dependencies]
logos = "0.16"   # latest stable is 0.16.1 (Jan 2026)
```

`json-tools` is removed. MSRV moves to whatever logos 0.16 requires; the CI matrix already runs stable and nightly only.

### Token enum (`src/lexer/token.rs`)

```rust
use logos::Logos;

#[derive(Logos, Debug, Clone, Copy, PartialEq, Eq)]
#[logos(skip r"[ \t\r\n]+")]            // RFC 8259 whitespace only, not Unicode whitespace
pub(crate) enum Token {
    #[token("{")] LeftBrace,
    #[token("}")] RightBrace,
    #[token("[")] LeftBracket,
    #[token("]")] RightBracket,
    #[token(":")] Colon,
    #[token(",")] Comma,
    #[token("true")]  True,
    #[token("false")] False,
    #[token("null")]  Null,

    // int: no leading zeros, optional minus
    #[regex(r"-?(0|[1-9][0-9]*)")]
    Int,
    // number with a fraction and/or exponent
    #[regex(r"-?(0|[1-9][0-9]*)(\.[0-9]+)?([eE][+-]?[0-9]+)?")]
    Number,

    // String: no raw U+0000..U+001F, only the 8 escapes plus \uXXXX
    #[regex(r#""([^"\\\x00-\x1F]|\\(["\\/bfnrt]|u[0-9a-fA-F]{4}))*""#)]
    String,
}
```

Notes:
- `Int` and `Number` overlap on plain integers; logos picks the longest match and, on a tie, the higher priority. Give `Int` the explicit higher priority so `27` is `Int` and `27.0`, `1e5` are `Number`. The parser keeps today's behaviour: `Int` is parsed as `i64`, and on overflow falls back to `f64` → `handle_double`. `Number` always goes to `f64::from_str`, which cannot fail on text matched by the regex, so the panic at `src/parser.rs:142` disappears.
- Tokens carry no data. The parser takes the span and slices the buffer itself, which keeps the token `Copy` and makes every `handle_string`/`handle_map_key` call zero-copy.
- **Lex bytes, not `str`.** We read arbitrary chunks from a `BufRead`; a chunk boundary can split a multi-byte UTF-8 code point, so a chunk is not always valid `str`. Lex `&[u8]` (logos implements `Source` for `[u8]`) and validate UTF-8 per string token with `std::str::from_utf8` on the span; failure is `ParseError::Utf8Error` with the offset. Non-ASCII bytes can only legally appear inside strings, and the string regex's `[^"\\\x00-\x1F]` class must be written so it admits any byte `>= 0x80` (exact attribute for byte-mode in 0.16 to be confirmed in the M1 spike).
- Lone surrogates (`"\ud800"`) are valid JSON grammar but not valid Unicode. The lexer accepts them (grammar-level); the decoder in M5 rejects them by default.

### Streaming wrapper (`src/lexer/mod.rs`)

logos lexes a slice; we have a `BufRead`. The wrapper owns a growable buffer and handles tokens that straddle a read boundary.

```rust
pub(crate) struct StreamLexer<R> {
    reader: R,
    buf: Vec<u8>,
    pos: usize,        // first unconsumed byte in buf
    filled: usize,     // end of valid data in buf
    base: u64,         // absolute stream offset of buf[0]
    eof: bool,
    max_token_len: usize,
}

pub(crate) enum Lexed<'b> {
    Token { kind: Token, bytes: &'b [u8], offset: u64 },
    Eof,
}

impl<R: BufRead> StreamLexer<R> {
    pub(crate) fn next(&mut self) -> Result<Lexed<'_>, ParseError>;
}
```

The refill rule is the core of this plan, and it is small:

1. Lex `buf[pos..filled]` with a fresh `logos::Lexer` and take one token.
2. If it is **an error, and we are not at EOF**: the bytes at the error could be the prefix of a valid token cut off by the read boundary (`"abc`, `tru`, `-`, `1.`, `1e`). Compact (`buf.copy_within(pos..filled, 0)`), read more, retry from the same start. Only once EOF is reached, or `filled - pos > max_token_len`, is the error real.
3. If it is **an `Int` or `Number` whose span ends exactly at `filled`, and we are not at EOF**: the number may continue in the next chunk (`12` | `3`). Refill and retry exactly as above. Strings and the literal keywords are self-delimiting, so they never need this.
4. Otherwise return the token, advance `pos` to `span.end`.

Correctness argument: refilling only ever appends bytes after the region being lexed, a real error stays an error when more bytes are appended (no valid token has an invalid prefix that later becomes valid, except the cut-off cases step 2 retries), and step 3 covers the one class of token that is a valid prefix of a longer valid token. The test harness below checks this exhaustively against every split point.

Memory: compaction keeps the buffer at `max(read chunk, max_token_len)`. `max_token_len` defaults to something generous (e.g. 64 MiB) and is settable via a new `Parser::with_max_token_len`. Exceeding it is a `MalformedJson` error with the offset, not an OOM.

Reader errors: `fill_buf()` errors map through the existing `From<std::io::Error> for ParseError` into `ReadError`; `ErrorKind::Interrupted` is retried.

### Parser changes (`src/parser.rs`)

- Replace `json_tools::{Lexer, Token, TokenType, Buffer}` with `StreamLexer` and our `Token`. The `match` arms map 1:1; the state-machine logic is untouched in this plan.
- `String` arm: `from_utf8(bytes)` → `&str`, passed straight to `handle_string` / `handle_map_key` (still raw, with quotes, per Goal 5).
- `Int` / `Number` arms: `from_utf8` is infallible here (ASCII by regex) but use the checked form anyway; no `panic!`.
- Delete `Wrapper`.
- `ParseError` gains position information. Proposed: `MalformedJson { offset: u64, msg: String }` style variants, or a separate `offset()` accessor. Either is a breaking change to a public enum, hence the 0.5.0 bump; consider `#[non_exhaustive]` at the same time so future additions aren't breaking.

### Optional: resumable parsing

Today each `parse()` call builds a new lexer, so a document split across two `parse()` calls breaks at the boundary. With the lexer state living in `Parser`, yajl-style "feed bytes as they arrive" (`parse_chunk(&[u8])` + `finish_parse()`) becomes possible. Not required for this plan; noted because the buffer design should not preclude it.

## Testing

1. **Token unit tests** (`src/lexer/token.rs`): every token, and every RFC 8259 rejection: `01`, `-`, `1.`, `.5`, `+1`, `1e`, `"\x"`, `"\u12"`, raw tab/newline inside a string, `True`, `nul`.
2. **Split-point harness** (the important one): a test `BufRead` that returns at most N bytes per `fill_buf`. For every existing fixture and a corpus of tricky inputs, run with N = 1, 2, 3, 7, 4096 and with every possible single split point, and assert the event log equals the unsplit run. This is what proves steps 2 and 3 of the refill rule.
3. **Existing tests unchanged**: `tests/test_parser.rs`, `tests/test_parse_valid_json.rs`, and with `--features ndjson`, `tests/test_ndjson_handler.rs` plus the unit tests in `src/ndjson_handler.rs`. No `.gold` edits allowed in this plan.
4. **JSONTestSuite** (github.com/nst/JSONTestSuite): vendor `test_parsing/` under `tests/json_test_suite/`. All `y_*` must parse; all `n_*` must error (some `n_*` are parser-level, e.g. trailing commas, and will only pass after the parser-hardening task; track those in an explicit allowlist that shrinks to empty); `i_*` record our chosen behaviour.
5. **Differential property test** against `serde_json` (dev-dependency): generate `serde_json::Value`s, serialize them (compact and pretty), assert our event stream reconstructs the same value. `proptest-regressions/test_prop_test.txt` already exists but the test file doesn't; this revives it as `tests/test_prop_test.rs`.
6. **Reader errors**: a `BufRead` that fails after K bytes must yield `ParseError::ReadError`, never `Ok`.
7. **Fuzzing** (optional, follow-up): `cargo fuzz` target that feeds arbitrary bytes with arbitrary chunking and asserts no panic and agreement with `serde_json` on accept/reject.
8. **Bench**: add a `benches/` criterion bench (CI already runs `cargo check --benches`) comparing against the 0.4 parser on the existing fixtures.

## Milestones

| # | Deliverable | Done when |
|---|---|---|
| M1 | `src/lexer/token.rs` with the `Token` enum, byte-mode confirmed against logos 0.16 | Token unit tests pass; parser untouched |
| M2 | `StreamLexer` with refill/compaction and `max_token_len` | Split-point harness passes on all fixtures |
| M3 | Parser switched to `StreamLexer`; `json-tools` and `Wrapper` removed; panics gone | All existing tests pass with no gold edits; clippy/fmt/docs clean |
| M4 | Positions in `ParseError`; `ReadError` surfaced; JSONTestSuite vendored with allowlist | Suite runs in CI |
| M5 | Additive string decoding: `pub fn decode_str(raw: &str) -> Result<Cow<'_, str>, DecodeError>` (strips quotes, resolves escapes, joins surrogate pairs, rejects lone surrogates) | Unit tests incl. all escape forms. Plan 2 needs this for key matching. |

M1–M3 can be one PR; M4 and M5 are separate PRs.

## Found while reading, for a separate parser-hardening task

These come from reading `src/parser.rs`, not from running it, so each should get a failing test first.

*Done in Plan 4, K3b (2026-10-18): all but the last are fixed, with failing-then-passing tests in `src/verify/state_machine.rs`, and the JSONTestSuite allowlist is empty. The last is kept on purpose: an `Abort` still returns `Ok(())`, and now it always does (see Plan 4, open question 3).*

- `]` and `}` don't check the current state, so `[1,]` and `{"a":1,}` look accepted (trailing comma).
- A `:` outside `MapSep` is silently ignored (`src/parser.rs:226`), so `{"a"::1}` looks accepted.
- `handle_end_array` / `handle_end_map` are called before the closing bracket is checked against the enclosing stack (`src/parser.rs:61`, `:82`), so the handler sees an end event for `{]` before the error.
- A mismatched `}` sets `LexicalError` at `src/parser.rs:88`, but the `if/else` right after it unconditionally overwrites the status (`MapGotVal` / `ArrayGotVal` / `GotValue`), so the error is lost and `[}` may be accepted.
- `Status::Abort` returns `Ok(())`, indistinguishable from a successful parse to the caller.
