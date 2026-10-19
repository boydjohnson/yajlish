# Plan 5: Raw number text in the `Handler` API, used by `yajlish-ndjson`

Status: draft · Target: `yajlish` 0.5.0 + `yajlish-ndjson` 0.1.0 (both unreleased) · Depends on: [Plan 3](03-ndjson-crate.md), [Plan 4](04-kani-bolero.md) · Closes: Plan 3 "Found while reading" item 5

## Why

The parser turns every number token into `i64` or `f64` (`number_value`, `src/parser.rs:400-413`) and the `Handler` trait only has `handle_int(i64)` and `handle_double(f64)` (`src/common.rs:170-173`). `NdJsonHandler` prints those values back with `Display` (`yajlish-ndjson/src/lib.rs:325`), so a number's text is rebuilt from a binary value instead of being copied.

Measured on main `2b83a82`, with each input as an element of the selected array:

| Input | ndjson output | Kind of change |
|---|---|---|
| `1.0`, `100.50`, `1e2`, `1.5e-7`, `-0` | `1`, `100.5`, `100`, `0.00000015`, `0` | spelling only |
| `12345678901234567890` | `12345678901234567000` | value changed |
| `3.141592653589793238462643383279` | `3.141592653589793` | value changed |
| `1e-400` | `0` | value changed, no error |
| `1e400` | `Err(NonFiniteNumber)` | valid input refused |

Strings and keys already reach handlers as raw text (`src/common.rs:176-182`, `:191-196`), and ndjson copies them through. Numbers are the one scalar ndjson cannot copy.

## Goals

1. A handler can get the exact bytes of every number token, as the input spelled them.
2. `yajlish-ndjson` writes every number byte-for-byte as it appeared in the input. All five rows above come out unchanged, `1e400` included.
3. No change for existing handlers. Every current `Handler` impl (tests, benches, the `handler!` macro's generated code, `yajlish-schema` test support) compiles and behaves the same with no edits.
4. Number errors stay where they are: a token the parser rejects today is rejected at the same offset with the same `ParseError`.
5. The Plan 4 proofs and fuzz targets keep passing, and the raw text gets its own checks.

## Non-goals

- Making the parser lazy about converting numbers. See Open question 1.
- Arbitrary-precision numbers in `yajlish-schema`. It could use the raw text later for exact `multipleOf` and integers beyond `i64`. That is a separate decision.
- Byte-exact ndjson output for whole elements. Whitespace between tokens is still dropped (`[1, 2]` becomes `[1,2]`). Only scalars are copied byte-for-byte.

## Design

### One new defaulted method on `Handler`

```rust
/// Latest parsed value was a number.
///
/// `raw` is the number's JSON text exactly as it appeared in the input,
/// for example `"-1.50e+3"`. `val` is the same number converted the way
/// `handle_int`/`handle_double` receive it: `Int` if the text is an
/// integer that fits in `i64`, otherwise `Double`, which may be rounded,
/// infinite or zero.
///
/// The default calls `handle_int` or `handle_double`, so a handler that
/// doesn't need the text can ignore this method.
fn handle_number(&mut self, ctx: &Context, raw: &str, val: Number) -> Status {
    match val {
        Number::Int(i) => self.handle_int(ctx, i),
        Number::Double(d) => self.handle_double(ctx, d),
    }
}
```

`Number` is today's `pub(crate) enum Number { Int(i64), Double(f64) }` (`src/parser.rs:392-395`). It becomes `pub` and `#[non_exhaustive]`, and moves to `common.rs` next to `Handler`. `lib.rs:81` re-exports it.

The parser change at `src/parser.rs:213-221`: keep the UTF-8 check and `number_value(kind, text, offset)?` exactly as they are, then call `self.handler.handle_number(context, text, num)` instead of matching on `num` there.

### Why this shape

| Option | Verdict |
|---|---|
| **Defaulted `handle_number(raw, val)`, conversion stays in the parser** (chosen) | Additive, so goal 3 holds. Errors don't move, so goal 4 holds. The Kani stub of `number_value` keeps working unchanged. Cost: the parser still converts every number, even for ndjson, which only wants the text. |
| Defaulted `handle_raw_number(raw)` only, with the default converting | Lets raw-only handlers skip the float parse. But the default method can't return a `ParseError`, so the parse error moves out of the parser or gets swallowed. And a `Status` alone can't express it. |
| Replace `handle_int`/`handle_double` with one required `handle_number` | Every implementor and the `handler!` codegen (`yajlish-schema-macros/src/lib.rs:410-411`) change, all for a cleaner trait. Not worth it. |
| A parser option such as `Parser::with_raw_numbers()` | A second code path in the state machine that Plan 4 would have to cover twice. |

Adding a provided method to a trait is a minor change under Rust semver. It only breaks an implementor that already has an inherent method named `handle_number` and calls it with method syntax. Since 0.5.0 is unreleased, this ships in the same release as the other breaking changes anyway.

### `yajlish-ndjson`

- Override `handle_number` to call `self.handle_value(ctx, raw)` (`lib.rs:306`). `handle_value` already takes any `Display`, so `&str` works unchanged.
- The `NonFiniteNumber` check in `handle_double` (`lib.rs:432-439`) is no longer reached from the parser, because the parser now calls `handle_number`. Keep `handle_int`/`handle_double` as they are, since the trait requires them and a caller could drive the handler directly. Keep the `NonFiniteNumber` variant to avoid a pointless public-enum break, and document it as returned only when `handle_double` is called directly with a non-finite value.
- The parser's number tokens are already strict RFC 8259 (Plan 1), so the raw text is always valid JSON. No validation is needed on output.

### What existing users see

Nothing, unless they override `handle_number`. `MockHandler`, `Recorder`, the benches, the schema runtime and the generated `handler!` impls all keep getting `handle_int`/`handle_double` with the same values through the default.

## Verification

### Core crate

| Check | Where | Engine |
|---|---|---|
| `raw` is the input's bytes at `ctx.offset()`, for every number event | `Recorder` in `src/verify/parser.rs` overrides `handle_number`, does the same `input.get(at..at+len)` assertion as `record_text` (`:163-181`), then records `Event::Int`/`Event::Double` from `val` as now | Kani (with the existing `number_value` stub), Bolero, proptest |
| The default `handle_number` gives the same event stream as before | Existing `Recorder`/reference comparisons pass unchanged, because the recorded events are still `Int`/`Double` | all three |
| `val` agrees with `raw` | New `#[test]` + Bolero target: for every number token the lexer produces, `number_value(kind, raw, _)` equals the `val` passed to `handle_number` | Bolero, proptest (float parsing is too big for Kani, as in Plan 4) |
| Chunk independence covers the raw text | `chunked_parse_is_invisible` compares number lengths too, so a number split across `parse` calls (Plan 4 K3c) still arrives whole | Kani bound 6, Bolero |

`Event::Int`/`Event::Double` gain no text field. The offset assertion above already pins the text to the input, which is stronger than storing a copy.

### `yajlish-ndjson`

1. **Regression table**: a unit test per row of the table in "Why", plus `-0.0`, `0E0`, `-9223372036854775808`, `9223372036854775808` and `1.5E+07`. Expected output is the input number followed by `\n`. Write it first. Today it fails on every row except the integers that fit in `i64`.
2. **Differential test becomes byte-exact for numbers**: `tests/test_prop_test.rs:13-18` and `src/verify.rs:113` currently keep doubles non-integral "so they print back as doubles". Drop that filter and generate number text directly with a strategy over the RFC 8259 grammar (sign, int part, optional fraction, optional exponent with any case and sign, long mantissas, huge and tiny exponents). Check that each output line equals the input element with insignificant whitespace removed. The `serde_json` value comparison stays as a second check.
3. **Fixtures**: both `.gold` files should stay unchanged. Every decimal in `custom.geo.json` and `prize.json` already appears verbatim in its `.gold` (checked on `2b83a82`), and neither file has exponents or trailing zeros. A fixture diff therefore means a bug. Add a third fixture with the regression-table numbers spread through a nested document.
4. **Kani/Bolero ndjson harnesses** (Plan 4 K4): unchanged, plus one Bolero target asserting that every number token's bytes appear verbatim in the output when the number is inside the selected array.

## Milestones

| # | Deliverable | Done when |
|---|---|---|
| R1 | Core: `Number` public, `handle_number` with default, parser calls it, `Recorder` offset check, `val`/`raw` agreement test, docs on `Handler` and in `lib.rs` example, CHANGELOG entry | `cargo test --all`, Kani job and fuzz targets green; no other `Handler` impl edited |
| R2 | ndjson: override `handle_number`, regression table, byte-exact differential, number fixture, `NonFiniteNumber` docs, CHANGELOG entry | All rows of the regression table pass; existing `.gold` files unchanged |

R1 and R2 can be one PR or two. Two is easier to review, because R1 should show no behaviour change at all.

## Open questions

1. **Lazy conversion.** ndjson pays for an `f64` parse per number it never uses. If the benches (`benches/parse.rs`) show it matters, a later change could pass a `RawNumber<'_>` with `as_str()` and a fallible `value()`. That needs an answer to where a conversion error goes (see the rejected option above). Until there is a number, keep it eager.
2. **Should `yajlish-schema` use `raw`?** Exact `multipleOf`, and `minimum`/`maximum` on integers beyond `i64`, become possible. Out of scope here. Worth a line in the schema crate's known limitations.
