# Plan 4: Bounded proofs with Kani, dual-engine harnesses with Bolero

Status: draft · Target: `yajlish` 0.5.x, `yajlish-ndjson` 0.1.x, `yajlish-schema` 0.1.x · Depends on: [Plan 1](01-logos-lexer.md) (lexer, `decode_str`), [Plan 2](02-schema-handler-macro.md), [Plan 3](03-ndjson-crate.md) · Unblocks: the parser-hardening list at the end of Plan 1

## Why

The workspace already has good dynamic coverage: unit tests, split-point tests (`tests/test_split_points.rs`), JSONTestSuite with an allowlist (`tests/test_json_test_suite.rs`), and proptest differentials against `serde_json` and `jsonschema`. What it lacks is any *exhaustive* statement. Every claim of the form "for all inputs" is currently checked on samples.

Kani gives exhaustive answers for small, bounded inputs, which is exactly the shape of the code most likely to hide off-by-one and boundary bugs: the lexer's "may this token still grow" rules, `\uXXXX` and surrogate handling, the parser's status transitions, and the schema crate's int/float comparisons. Bolero lets one harness body run three ways: as a plain `#[test]` with random inputs, under libFuzzer/AFL via `cargo bolero`, and under Kani. So each property is written once and gets both bounded proof and unbounded fuzzing.

Kani is **not** used for end-to-end RFC 8259 conformance over arbitrary documents. That stays with JSONTestSuite and the differential tests, which Bolero will now also fuzz.

## Goals

1. Kani proofs for every pure subroutine listed under "Harnesses" below, each finishing in under ~2 minutes in CI.
2. A Kani proof that the parser never panics and keeps its `Context` invariants on every byte string up to a fixed length (target 8, see "Bounds").
3. A Kani proof that streaming is chunk-independent: splitting the input at any point gives the same tokens and events as not splitting it.
4. A Kani soundness harness ("accepted only if valid") against a small reference recognizer. It **fails today** on the cases in `N_ALLOWLIST`; it becomes the regression oracle for the parser-hardening fix.
5. Bolero fuzz targets for the same properties plus the differential properties that are too big for Kani, seeded from the existing fixtures and JSONTestSuite.
6. CI: Bolero harnesses run as ordinary tests on every PR; Kani runs on every PR; fuzzing runs time-boxed on a schedule.

## Non-goals

- Proving full RFC 8259 compliance, or anything about inputs longer than the Kani bound.
- Verifying the proc macro's code generation under Kani. The macro's helper functions get Bolero fuzzing only.
- Verifying `logos`, `core::str::parse::<f64>` or `regex`. Where our code calls them we either let Kani check them as-is (logos-generated code, which is small) or stub them (float parsing, see "Stubs").
- Rewriting the existing proptest tests. They stay; new properties go in Bolero. **Default picked, flag if wrong.**

## Tooling

### Dependencies

```toml
# each crate that has harnesses
[dev-dependencies]
bolero = "0.13.7"

# yajlish only: under `cargo kani`, build logos's state-machine codegen (see "Stubs").
[target.'cfg(kani)'.dependencies]
logos = { version = "0.16", features = ["state_machine_codegen"] }

[lints.rust]
# CI builds with RUSTFLAGS=-Dwarnings, and rustc's check-cfg warns on unknown cfgs.
unexpected_cfgs = { level = "warn", check-cfg = ["cfg(kani)", "cfg(fuzzing)"] }
```

The `[lints.rust]` entry is required, not optional: without it, the first `#[cfg(kani)]` breaks the `-Dwarnings` CI job (`.github/workflows/ci.yaml`).

Kani itself is installed with `cargo install --locked kani-verifier && cargo kani setup`; it is not a Cargo dependency. The `kani` crate is injected only under `cargo kani`, so all direct uses of `kani::any()` sit behind `#[cfg(kani)]`.

### Where harnesses live

Most targets are `pub(crate)` (`is_number_prefix`, `hex4`, `Context`'s mutators, `num::cmp`), so harnesses live inside each crate:

```
src/verify/mod.rs           #[cfg(any(test, kani))] mod verify;  from lib.rs
src/verify/lexer.rs         lexer + may_extend proofs
src/verify/decode.rs        decode_str + hex4 proofs
src/verify/parser.rs        Recorder handler, chunked-parse harnesses
src/verify/state_machine.rs state-machine proofs over token sequences (K3)
src/verify/tokens.rs        TokenSeq: token sequences for parse_source (K3)
src/verify/ref_lexer.rs     reference lexer that stands in for logos under Kani (K1)
src/verify/reference.rs     the reference token-sequence recognizer (K3)
yajlish-ndjson/src/verify.rs
yajlish-schema/src/verify.rs
yajlish-schema-macros/tests/fuzz_names.rs   (public helpers only, Bolero only)
fuzz corpora: <crate>/src/verify/__fuzz__/<harness>/corpus/   (cargo-bolero's default)
```

### One body, three engines

Kani needs fixed-size inputs, and Bolero's Kani driver builds values with `kani::any()`. So shared harnesses take a fixed array plus a length, which also works unchanged as a fuzz input:

```rust
#[test]
#[cfg_attr(kani, kani::proof, kani::unwind(10))]
fn parser_never_panics() {
    bolero::check!()
        .with_type::<([u8; 8], u8)>()
        .cloned()
        .for_each(|(buf, len)| {
            let input = &buf[..usize::from(len) % 9];
            let mut handler = Recorder::default();
            let mut parser = Parser::new(&mut handler);
            let mut reader = input;
            let _ = parser.parse(&mut reader).and_then(|()| parser.finish_parse());
        });
}
```

Two corrections to the sketch in the request this plan answers: `Parser::parse` takes `&mut impl BufRead`, not a slice, and `Handler` has ten required methods with no `Default` dummy, so `src/verify/parser.rs` provides a `Recorder` that logs events into a fixed-capacity array (no `Vec` growth under Kani) and asserts the per-event invariants below.

Harnesses that only make sense under Kani (they use `kani::assume` or `kani::cover!`) are plain `#[cfg(kani)] #[kani::proof]` functions. Fuzz-only harnesses use `Vec<u8>` and are plain `#[test]` + `bolero::check!()`.

Commands:

```
cargo test --workspace                                  # every Bolero harness, random inputs
cargo kani --tests -p yajlish@0.5.0 -Z stubbing         # every Kani proof in a crate
cargo kani --tests -p yajlish@0.5.0 -Z stubbing --harness verify::lexer::complete_number_is_prefix --exact
cargo bolero test -p yajlish verify::parser::parser_never_panics --engine libfuzzer -T 5min
cargo kani --tests -Z concrete-playback --concrete-playback=print --harness <h>   # counterexample -> unit test
```

Measured corrections (K0): `-p yajlish` is ambiguous because the `yajlish_04` bench dev-dependency is also named `yajlish`, so it is `-p yajlish@0.5.0`; `#[kani::stub]` needs `-Z stubbing`; and Kani drops `--features dep/feature`, which is why the logos feature goes through a `cfg(kani)` target dependency.

Concrete playback is how a failing proof turns into a regular `#[test]`: every Kani counterexample found during this plan gets committed as a unit test next to the code it breaks.

## Bounds and stubs

### Bounds

Measured on 2026-10-17 with Kani 0.68.0 / CBMC 6.11.0 on an Apple-silicon Mac (`cargo kani --tests -p yajlish@0.5.0 -Z stubbing`). K1's 12 harnesses ran one at a time, about 9 minutes in all; K3's six ran three at a time, so their times are pessimistic. All 18 together take about 20 minutes in CI. The bounds are much smaller than the starting points this table used to list; the reasons are below the table.

| Harness | Kani input | Unwind | Time | Rest of the bound covered by |
|---|---|---|---|---|
| `number_prefix_is_prefix_closed` | ≤ 8 bytes | 10 | 6 s | |
| `number_prefix_always_extends` | ≤ 8 bytes | 10 | 19 s | |
| `string_may_extend_never_panics` | ≤ 8 bytes | 10 | 27 s | |
| `keyword_may_extend` | ≤ 6 bytes | 8 | 17 s | |
| `complete_number_is_prefix` | ≤ 4 bytes | 6 | 38 s | stubbed lexer, see "Stubs" |
| `string_may_extend_is_sound` | body ≤ 2, suffix 1–2 | 7 | 25 s | stubbed lexer |
| `ref_lexer_matches_logos` | 1 byte, real logos | 3 | 5 s | exhaustive test: all ≤ 2 bytes, all ≤ 4 over `ALPHABET`; Bolero |
| `hex4_matches_from_str_radix` | 4 bytes | 6 | 4 s | |
| `surrogate_pair_is_always_scalar` | all high × low pairs (arithmetic) | none | 0.1 s | `every_surrogate_pair_decodes` runs `decode_str` on every pair |
| `decode_never_panics` | exactly 4 bytes | 6 | 121 s | `exhaustive_short_decode_inputs`: every byte string ≤ 3 |
| `borrowed_iff_no_escape` | exactly 4 bytes | 6 | 123 s | same |
| `lexer_and_decoder_agree` | exactly 4 bytes, quoted | 6 | 152 s | same, real logos |
| `parser_never_panics` | ≤ 4 tokens, `Eof` or error | 6 | 63 s | `exhaustive_token_sequences` (the same space, without the stubs); bytes: `exhaustive_short_inputs`, Bolero |
| `no_callback_after_abort` | ≤ 4 tokens, abort at any event | 6 | 60 s | same |
| `accepts_all_valid_json` | ≤ 4 tokens | 6 | 112 s | same, plus `generated_json_is_accepted` (Bolero, random documents) |
| `events_match_reference` | ≤ 4 tokens | 6 | 120 s | same |
| `context_invariants_hold` | ≤ 4 tokens, `should_panic` | 6 | 115 s | fails today, see the parser table |
| `accepts_only_valid_json` | ≤ 4 tokens, `should_panic` | 6 | 125 s | fails today, see the parser table |
| `chunking_is_invisible`, `offsets_are_monotone`, `max_token_len_is_enforced`, `chunked_parse_is_invisible` | not under Kani | | > 10 min at 2 bytes | `exhaustive_short_stream_inputs` / `exhaustive_short_parse_inputs`: every byte string ≤ 2 bytes and every string ≤ 4 bytes over `ALPHABET`, every split, real logos; Bolero with chunk-size lists |

What the measurements showed:

- **Symbolic lengths are the main cost.** With `len` nondeterministic, every loop over the input (logos's states, UTF-8 validation, `decode_str`, `hex4`) is unrolled to the unwind bound on every path. Harnesses therefore run Kani on a concrete length (`for_each_prefix` for the cheap ones, exactly `N` bytes via `for_each_input` for `decode_str`) and enumerate the shorter inputs in ordinary `exhaustive_*` tests. Tight unwind bounds matter too: `complete_number_is_prefix` went from 142 s to 38 s going from unwind 10 to 6. Unwinding assertions stay on, so a bound that is too small fails rather than passing silently.
- **`decode_str` is expensive per byte.** A single 4-byte input takes about 2 minutes, almost all in CBMC's simplifier on the `String`/`Cow` heap objects. The plan's 10 bytes is out of reach; 4 bytes reaches a two-character escape and a two-byte UTF-8 character but not `\uXXXX`, which `every_surrogate_pair_decodes` covers by enumeration.
- **`StreamLexer` and `Parser` are out of reach entirely.** Even two symbolic bytes, with the lexer stubbed and `format` stubbed, did not finish in 10 minutes (three bytes: not in 20). K2's "green at bound 6" is not achievable with Kani, so K2's two properties are proved by enumeration instead, which also runs the real logos. This matters for K3 (see "Open questions").
- **The state machine fits at 4 tokens, behind `parse_source`** (K3). Even at 2 tokens it first timed out, three ways: UTF-8 validation of token bytes chosen through a symbolic pointer (symex 139 s), `format!` in error paths, and `Context`'s `VecDeque`, whose ring-buffer indices go symbolic once paths merge so that CBMC's array encoding never finishes (`--arrays-uf-always`/`-never` did not help). With the stubs below, 2 tokens takes 31 s and 4 tokens 60–125 s; 5 tokens timed out at 10 minutes.
- **std's `from_utf8` is slow under Kani** (it reads aligned words found with `align_offset`), so harnesses use `verify::as_utf8`, a byte-at-a-time check that `utf8_check_matches_std` tests against std on every input up to 3 bytes and every 4-byte input built from the table's range edges.

### Stubs

- **logos is stubbed after all.** The plan assumed logos-generated code is small per token. It is not, for CBMC: logos 0.16's default codegen is a set of mutually tail-calling state functions, which CBMC unrolls as recursion on every path, and `complete_number_is_prefix` did not finish on 2 bytes. The `state_machine_codegen` feature (one loop over a `match`; enabled only under `cargo kani`, see "Dependencies") makes 1 byte take 2 s, but 2 bytes still needs over 4 minutes, because paths that consumed different numbers of bytes merge and the offset becomes symbolic. So Kani harnesses that lex use `#[kani::stub(<Token as Logos>::lex, ref_lexer::ref_lex)]`: `src/verify/ref_lexer.rs` is a direct longest-match scanner for the grammar in `src/lexer/token.rs` (about 100 lines), written against logos's `internal::LexerInternal` API. Those proofs are therefore about the reference lexer, and the link to logos is checked separately on everything `StreamLexer` reads from it (first token kind and span, first error's start, whitespace-only): by Kani on one byte, by an exhaustive test over every input up to 2 bytes and every input up to 4 bytes over `verify::ALPHABET` (one byte per class the grammar distinguishes), and by Bolero. The stub needs an early-bound lifetime (`where 's: 's`) to match `Logos<'s>::lex`'s generic parameter count.
- **Float parsing.** As planned: the number branch of `parse_tokens` is now `parser::number_value(kind, text, offset)` (it needs `offset` for its error), and `verify::parser::stub_number_value` replaces it in the K3 harnesses. The stub is deterministic (in the kind and the text's length) rather than nondeterministic, so that the parser and the reference get the same value for the same token.
- **Error messages.** `verify::stub_format` replaces `std::fmt::format` in the K3 harnesses. It returns `String::from("e")`, not `String::new()`: Kani 0.68 models a `String::new()` returned from a stub with capacity 1 and a dangling pointer, and its drop then fails Kani's `__rust_dealloc` checks. It did not rescue the stream harnesses.
- **`Context`'s stack** (K3). `state_machine::context_stack` replaces `Context::add_enclosing`, `last_enclosing` and `remove_last_enclosing` with a fixed array in statics, since the `VecDeque` is what CBMC cannot encode (see "Bounds"). A harness run has one `Context` and everything reads the stack through these methods, so the stand-in is the same stack. What it does not check is `VecDeque` itself.
- **UTF-8 of token bytes** (K3). The Kani token sequences use one ASCII spelling per kind (`tokens::canonical`), so `std::str::from_utf8` and `verify::as_utf8` are stubbed to unchecked conversions there. `exhaustive_token_sequences` runs the same space without the stubs, and the byte-level tests run real input.

## Harnesses

Engine: **K** = Kani proof, **B** = Bolero (test + fuzz), **K+B** = shared body, **exhaustive** = an ordinary test that enumerates every short input (see "Bounds" for which rows ended up there and why). "Today" is what I expect from reading the code, not from a run.

### Core crate: lexer (`src/lexer/`)

| Name | Engine | Property | Today |
|---|---|---|---|
| `number_prefix_is_prefix_closed` | K | `is_number_prefix(b) ⇒ is_number_prefix(&b[..k])` for every `k` (`src/lexer/mod.rs:233`). | pass |
| `number_prefix_always_extends` | K | The doc comment's claim: if `is_number_prefix(b)` then some single byte `c` makes `b + c` a number prefix. Witness search over `b"0.e+-"`, so no ∃ is needed. | pass |
| `complete_number_is_prefix` | K | If `Token::lexer(b)` yields exactly one `Int`/`Number` spanning all of `b`, then `is_number_prefix(b)`. Ties the hand-written DFA to the logos regex. | pass |
| `string_may_extend_is_sound` | K | For any body ≤ 4 bytes and suffix ≤ 4 bytes: if `"` + body + suffix lexes as one `String` token, then `string_may_extend(body)` (`src/lexer/mod.rs:201`). A false here means the stream lexer reports an error at a chunk boundary for valid input. | pass |
| `string_may_extend_never_panics` | K | Covers the `\u` slicing at `src/lexer/mod.rs:214` for every truncation. | pass |
| `keyword_may_extend` | K | `may_extend` on `t`/`f`/`n` prefixes agrees with "is a proper prefix of the keyword". | pass |
| `chunking_is_invisible` | B + exhaustive | For any input ≤ 6 bytes and split `k`, `StreamLexer` over a two-chunk reader yields the same `(Token, bytes, offset)` sequence and the same error offset as over one chunk. Fuzz version uses arbitrary chunk-size lists and long inputs. This is the exhaustive form of `test_every_chunk_size_matches_unchunked`. | pass |
| `max_token_len_is_enforced` | B + exhaustive | With nondeterministic `max_token_len ≤ 8`, no `Lexed::Token` longer than the cap is ever returned, chunked or not. | pass |
| `offsets_are_monotone` | B + exhaustive | Token offsets strictly increase and each points at the token's first byte in the input. | pass |

### Core crate: `decode_str` (`src/decode.rs`)

| Name | Engine | Property | Today |
|---|---|---|---|
| `hex4_matches_from_str_radix` | K | For all 4-byte inputs, `hex4` equals `u16::from_str_radix` when all bytes are hex digits and `None` otherwise (`src/decode.rs:167`). | pass |
| `decode_never_panics` | K+B | Any `&str` ≤ 4 bytes (see "Bounds") (built from bytes with `kani::assume(from_utf8(..).is_ok())`). Covers the `&body[run..i]` slices. | pass |
| `surrogate_pair_is_always_scalar` | K | For every high/low pair, the `ok_or(LoneSurrogate)` at `src/decode.rs:141` is unreachable (`kani::cover!` on that branch must be unsatisfiable), so that error path is dead code. | pass |
| `lexer_and_decoder_agree` | K+B | If bytes lex as one `String` token and are valid UTF-8, `decode_str` returns `Ok` or `LoneSurrogate`, never `NotQuoted`, `UnescapedCharacter` or `InvalidEscape`. | pass |
| `decode_matches_serde_json` | B | Fuzz-only: `decode_str(raw) == serde_json::from_str::<String>(raw)` modulo lone surrogates. Extends the existing proptest to coverage-guided input. | pass |
| `borrowed_iff_no_escape` | K | `Cow::Borrowed` exactly when the body has no backslash. | pass |

### Core crate: parser state machine (`src/parser.rs`, `src/common.rs`)

**K3 as built.** Kani cannot run `Parser::parse` (see "Bounds"), so K3 refactored it (decided 2026-10-17, open question 2): the state machine is now `Parser::parse_source<S: TokenSource>`, and `parse` is `parse_source` over a `StreamLexer`, with no behaviour change (every existing test, gold file and `N_ALLOWLIST` entry is unchanged). The Kani harnesses feed `parse_source` a `TokenSeq` of up to 4 nondeterministic tokens, each with a canonical spelling, ending at `Eof` or at a lexer error. The Bolero versions lex random bytes with the real lexer into the same `TokenSeq` (`tokens::lex_all`), which replays exactly what `parse` sees, since the parser stops at the first lexer error either way. Each property is one `check_*` function in `src/verify/state_machine.rs` that all three engines call.

All use `Recorder`, which asserts on every callback:

- `ctx.num_open_braces() + ctx.num_open_brackets()` equals the depth `Recorder` tracks from start/end events, and `last_enclosing()` agrees with the innermost open event;
- `handle_map_key` only arrives inside a map, directly after `StartMap` or a value;
- `ctx.offset()` is within the input and strictly greater than the previous event's.

| Name | Engine | Property | Today |
|---|---|---|---|
| `parser_never_panics` | K+B | Any token sequence ≤ 4 (Kani) or input ≤ 8 bytes (Bolero), through `parse_source` then `finish_parse`. Includes the `dec_braces`/`dec_brackets` subtractions (`src/common.rs:151`, `:159`), which must never underflow. | **pass** |
| `context_invariants_hold` | K+B | The `Recorder` assertions above. | **fails** (K3 measured): on `[}`, `handle_end_map` is called, the context does not pop, and parsing continues, so the next callback sees the array still open after the handler saw it closed. Kani: `should_panic`, playback `[ "s" } {`. Bolero version ignored until K3b. |
| `no_callback_after_abort` | K | With a `Recorder` that returns `Abort` at a nondeterministic event index, no callback follows it and `parse` returns. | **pass**. `parse` does not always return `Ok` after an `Abort`: on `]` the parser calls `handle_end_array` before checking for a matching `[`, so the abort is followed by `MalformedJson`. Not checked; a K3b question. |
| `accepts_only_valid_json` | K+B | If `parse` and `finish_parse` both return `Ok`, the reference recognizer accepts the same token sequence. | **fails**, by design. Kani: `should_panic`, playback: the empty sequence. Six classes, one ignored test each in `state_machine::tests`, with the `N_ALLOWLIST` entries each explains: empty document (`""`, `" "`); a `}` that closes nothing or an array (`}`, `{}}`, `[}]`: `LexicalError` overwritten); `:` outside `MapSep` ignored (`:`, `{"a"::1}`, `{:"b"}`); `[`/`{` after a value (`1[]`, `[][]`, `[3[4]]`); trailing comma (`[1,]`, `{"a":1,}`); key without value (`{"a"}`). Found by enumerating every canonical sequence of ≤ 4 tokens; Kani's playback finds one at a time. |
| `accepts_all_valid_json` | K+B | The converse: every token sequence the reference accepts is accepted by the parser. Bolero also runs `generated_json_is_accepted` on random documents with random whitespace. | **pass** |
| `events_match_reference` | K+B | For input the reference accepts, the parser's events equal the reference's. | **pass** |
| `chunked_parse_is_invisible` | B + exhaustive | Same as `chunking_is_invisible`, at the event level through `Parser`. Exhaustive form of `tests/test_split_points.rs`. | pass |
| `split_across_parse_calls` | K+B | For any input ≤ 6 bytes and split `k`, `parse(&input[..k])`, `parse(&input[k..])`, `finish_parse()` gives the same events and result as one `parse` of the whole input followed by `finish_parse()`. | **fails**: `[1` then `2]` gives two numbers (see "Tokens span `parse` calls"). |

The reference recognizer (`src/verify/reference.rs`) is an explicit-stack pushdown automaton over `Token`s, written directly from the RFC 8259 grammar, about 60 lines, no recursion (so Kani needs no extra unwinding for depth). Before it is trusted as an oracle it is run over JSONTestSuite: it must accept every `y_*` and reject every `n_*` whose tokens lex, with **no allowlist**. *K3: it does (`reference_passes_json_test_suite`).*

`accepts_only_valid_json` is the plan's main payoff. It lands in K3 marked `#[cfg_attr(kani, kani::should_panic)]`, with each counterexample committed as a failing-then-ignored unit test. Fixing the parser so it passes is the parser-hardening task from Plan 1 and is a behaviour change, so it is a separate PR (K3b), which also empties `N_ALLOWLIST`. **Default picked, flag if you'd rather fold the fix into this plan.**

#### Tokens span `parse` calls (decided 2026-10-17)

A token cut off at the end of one `parse` call continues in the next: `parse(b"[1")` then `parse(b"2]")` must deliver one `handle_int(12)`. Today each `parse` builds a new `StreamLexer` (`src/parser.rs:62`), which treats the end of its reader as the end of the stream, so it emits `1` and then `2`.

The fix (K3c) keeps the lexer's buffer in `Parser` between calls instead of in a per-call `StreamLexer`. The end of one reader then means "no more bytes for now", and a token that may still extend stays buffered. Only `finish_parse` marks the true end of the stream and flushes it. One consequence callers will see: a document that is a bare top-level number (`12`) only produces its event from `finish_parse`, because until then a later call could append digits. Strings, keywords and containers are unaffected since they end in a byte that can't extend. That changes the documented contract of `parse` and `finish_parse`, so the changelog and the `Parser` docs say so.

### `yajlish-ndjson`

| Name | Engine | Property | Today |
|---|---|---|---|
| `selector_parse_never_panics` | K+B | Any `&str` ≤ 6 bytes through `SelectorParser::parse` and both token parsers (`yajlish-ndjson/src/selector_parser.rs:114` slices at `pos`). | pass |
| `selector_round_trip` | B | Printing a `Vec<Selector>` (identifiers without `.`/`[`, any index) and parsing it gives the same vector. Pins down whitespace handling inside keys, which `test_whitespace_and_unicode` shows is preserved. | pass, I expect |
| `ndjson_never_panics` | K+B | Parser + `NdJsonHandler` with selector `[0]` or `.a` on any input ≤ 8 bytes; `finish()` returns. | pass, I expect |
| `ndjson_lines_match_elements` | B | Fuzz version of `yajlish-ndjson/tests/test_prop_test.rs`: each output line parses (with `serde_json`) to the matching element of the selected array, and `SelectorNotFound` exactly when `serde_json`'s pointer lookup finds no array. | pass |

### `yajlish-schema` and the `handler!` macro

| Name | Engine | Property | Today |
|---|---|---|---|
| `cmp_int_float_is_exact` | K | For all `i64` and `f64`, `num::cmp` (`yajlish-schema/src/num.rs:21`) equals an exact reference that compares `i` with `f`'s decomposed mantissa and exponent in `i128`. `None` exactly when `f` is NaN. | pass |
| `cmp_is_antisymmetric` | K | `cmp(a, b) == cmp(b, a).map(Ordering::reverse)` for all `Num` pairs. | pass |
| `int_multiple_of_no_overflow` | K | `is_multiple_of(Int, Int)` (`num.rs:40`) never panics, including `i64::MIN` by `-1`, and equals `x.checked_rem(y) == Some(0)` where that is defined. | pass |
| `float_multiple_of_never_panics` | K | The float branch for all `f64` pairs. Only panic-freedom and "infinite quotient is not a multiple"; the 1e-9 tolerance is a spec choice, not something to prove. | pass |
| `i64_from_float_is_exact` | K | For all `f64`, `FromValue for i64` (`yajlish-schema/src/value.rs:203`) returns `Ok(i)` only when `i as f64 == x`, and errors exactly when `x` is fractional, non-finite or outside `[-2^63, 2^63)`. | pass |
| `runtime_never_panics` | K | For one small fixed schema generated by `handler!` in the test (an object with one required integer and one optional string), feed `Runtime::event` any sequence of ≤ 6 events built from `kani::any()` (including unbalanced ones), then `finish()`. | pass, I expect |
| `candidates_only_shrink` | K+B | `live_candidates()` never increases within a record. | pass, I expect |
| `unique_means_exactly_one` | B | With `match = unique`, an `Ok(Record)` is produced only when `jsonschema` accepts the element against exactly one schema. Fuzz version of `test_differential.rs` over the same schema list. | pass |
| `names_are_valid_idents` | B | In `yajlish-schema-macros`: for any `&str`, `ident(&snake(s))` and, whenever `pascal(s)` is non-empty, `ident(&pascal(s))` don't panic (`proc_macro2::Ident::new` panics on invalid identifiers; `lib.rs:639`, `types.rs:589`, `:607`). Also checks that every caller handles `pascal` returning an empty string. | unknown; `pascal` can return `""` by its doc comment, so the property checks that no caller passes that to `ident`. |

## Fuzz corpora and CI

- Seed every Bolero target that takes bytes from `tests/valid_json_fixtures/`, `tests/json_test_suite/test_parsing/` and `yajlish-ndjson`'s fixtures, copied into `__fuzz__/<target>/corpus/` by `scripts/seed-corpora.sh`.
- New CI jobs in `.github/workflows/ci.yaml`:
  - `kani` (every PR): `model-checking/kani-github-action@v1`, running `cargo kani --tests -p yajlish@0.5.0 -Z stubbing` (more `-p` as K4/K5 add crates; about 20 minutes after K3). Harnesses marked `should_panic` keep it green until K3b.
  - `fuzz` (nightly schedule, not PRs): nightly toolchain, `cargo install cargo-bolero`, 5 minutes per target with libFuzzer, uploading any crash as an artifact.
- The existing `cargo test --all` job runs every Bolero harness with random inputs at no extra setup cost.

## Milestones

| # | Deliverable | Done when |
|---|---|---|
| K0 | Tooling: `bolero` dev-dep, `[lints.rust]` check-cfg, `verify` modules, `Recorder`, `number_value` extraction, Kani CI job | `cargo kani` runs one trivial proof in CI; `cargo test` unchanged; solver times recorded here |
| K1 | Lexer and `decode_str` proofs | All lexer/decoder rows green under Kani and as tests |
| K2 | Chunk independence (lexer and parser levels) | `chunking_is_invisible`, `chunked_parse_is_invisible` green at bound 6. **Done differently:** Kani cannot run `StreamLexer`; both are exhaustive up to 2 bytes (all bytes) and 4 bytes (`ALPHABET`), plus Bolero |
| K3 | Parser state machine, reference recognizer, `accepts_only_valid_json` as `should_panic` | Reference passes JSONTestSuite with no allowlist; counterexamples committed as unit tests. **Done** at 4 tokens, through the `parse_source` refactor; `context_invariants_hold` also lands as `should_panic` |
| K3b | Parser hardening (separate PR, behaviour change) | `accepts_only_valid_json` passes without `should_panic`; `N_ALLOWLIST` is empty |
| K3c | Tokens span `parse` calls: lexer buffer moves into `Parser`, `finish_parse` flushes (separate PR, behaviour change) | `split_across_parse_calls` passes; `Parser` docs and changelog updated |
| K4 | ndjson and selector harnesses | Rows green |
| K5 | Schema `num`/`value` proofs, runtime harness, macro name fuzzing | Rows green |
| K6 | Corpora seeded, nightly fuzz job | One clean nightly run per target |

K0–K2 can be one PR. K3, K3b, K3c, K4 and K5 are separate PRs.

## Open questions

1. **Kani bound for the parser.** 8 bytes reaches `{"a":1}` and `[[[]]]` but not a surrogate pair inside a document. If K0's timings allow 12, that is worth the extra CI minutes. *Answered by K0–K2's measurements: no symbolic parser input fits at all, see 2.*
2. ~~**K3 cannot run `Parser::parse` under Kani.**~~ *Decided 2026-10-17: refactor. Done in K3 as `TokenSource` and `Parser::parse_source`, see the parser section.*
3. **What should K3b do about `Abort` on a malformed token?** Today a handler can `Abort` on `]` or `}` that the parser then rejects, so the caller sees the abort and an error. Validating before the callback (which K3b's hardening does anyway for `context_invariants_hold`) would make `parse` return `Ok` after every `Abort`.
