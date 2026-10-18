# Plan 3: Move the ndjson converter into its own crate, `yajlish-ndjson`

Status: draft · Target: `yajlish` 0.5.0 + `yajlish-ndjson` 0.1.0 · Depends on: nothing · Interacts with: [Plan 1](01-logos-lexer.md), [Plan 2](02-schema-handler-macro.md)

## Why

`NdJsonHandler`, `Selector` and the lalrpop selector grammar live in the core crate behind the `ndjson` feature (`src/lib.rs:75`, `src/ndjson_handler.rs`, `src/selector_parser.lalrpop`, `build.rs`). That costs the core crate more than it gives:

| Problem | Where | Effect |
|---|---|---|
| The core crate carries a build script and two optional deps for one consumer | `build.rs`, `Cargo.toml` `[features] ndjson` | `lalrpop` + `lalrpop-util` in the dependency tree of a "high-integrity" parser that doesn't use them. |
| ndjson tests don't run in CI | `.github/workflows/ci.yaml` runs `cargo test --all` with no `--features`/`--all-features` | `tests/test_ndjson_handler.rs` and the unit tests in `src/ndjson_handler.rs` are `cfg`'d out on every CI run (read from the workflow, not observed in a run). Plan 1 M3 relies on these tests passing. |
| Converter and parser are versioned together | one crate | A fix to the converter forces a parser release and vice versa. |
| It already only uses the public API | `src/ndjson_handler.rs:7` imports `ParserStatus`, `Context`, `Handler`, `Status`, all re-exported from `lib.rs` | Nothing ties it to the parser's internals, so the move is mechanical. |

The last row is what makes this cheap: the handler is an ordinary downstream `Handler` impl, and making it a separate crate turns that into something the compiler enforces.

## Goals

1. New workspace member `yajlish-ndjson/` exposing `NdJsonHandler`, `Selector` and `SelectorParser` at its crate root.
2. `yajlish` loses the `ndjson` feature, `build.rs`, `lalrpop` and `lalrpop-util`.
3. **Output byte-for-byte unchanged**: both `.gold` files in `tests/ndjson_fixtures/` and every unit test in `src/ndjson_handler.rs` pass after the move without edits.
4. ndjson tests run in CI by default.
5. Git history preserved (`git mv`), so `git blame` still works on the moved files.

## Non-goals (for the move itself)

- Behaviour changes to the converter. The bugs found while reading are listed below and fixed in follow-up milestones, each with a failing test first.
- A command-line binary. Easy to add later as `yajlish-ndjson/src/bin/`, not part of this plan.

## Design

### Workspace layout

Same shape Plan 2 S0 proposes, with one more member. Whichever plan lands first creates the `[workspace]` table; the other only adds members.

```
Cargo.toml                  # [workspace] members = [".", "yajlish-ndjson"]  (+ Plan 2's two crates later)
src/                        # yajlish: parser only
tests/                      # parser tests only
yajlish-ndjson/
  Cargo.toml
  build.rs                  # moved from the root
  src/lib.rs                # was src/ndjson_handler.rs
  src/selector_parser.lalrpop
  tests/test_ndjson_handler.rs
  tests/common/mod.rs       # just test_ndjson_fixture + file readers
  tests/ndjson_fixtures/    # moved from tests/ndjson_fixtures/
```

The root package stays at the root (`"."` as a member) so `yajlish`'s published layout, README and docs.rs path don't change.

Shared metadata goes in `[workspace.package]` (`authors`, `license`, `repository`, `edition = "2018"`) and is inherited with `field.workspace = true`, so Plan 2's crates get it for free. Dev-deps used by several crates (`pretty_assertions`) go in `[workspace.dependencies]`.

### `yajlish-ndjson/Cargo.toml`

```toml
[package]
name = "yajlish-ndjson"
version = "0.1.0"
description = "Convert a JSON array inside a JSON document into newline-delimited JSON, streaming"
keywords = ["json", "ndjson", "streaming"]
categories = ["parsing"]
build = "build.rs"
authors.workspace = true
license.workspace = true
repository.workspace = true
edition.workspace = true

[dependencies]
yajlish = { path = "..", version = "0.5" }
lalrpop-util = "0.19"

[build-dependencies]
lalrpop = "0.19"

[dev-dependencies]
pretty_assertions.workspace = true
```

`lalrpop` is no longer optional; it is only in the tree of people who asked for the converter. The `version` next to `path` is what lets `cargo publish` work.

### Code changes during the move

Kept to what the compiler demands:

- `src/ndjson_handler.rs` → `yajlish-ndjson/src/lib.rs`. `use crate::{common::ParserStatus, Context, Handler, Status}` becomes `use yajlish::{Context, Handler, ParserStatus, Status}`. The module doc comment becomes the crate doc.
- In the unit tests, `use crate::Parser` becomes `use yajlish::Parser`.
- `lalrpop_mod!(pub selector_parser)` stays; the grammar's `use crate::ndjson_handler;` becomes `use crate as ndjson_handler;` (or the paths are rewritten to `crate::Selector`), so the grammar file needs a one-line change.
- In `yajlish`: delete `#[cfg(feature = "ndjson")] pub mod ndjson_handler;`, `build.rs`, the `[build-dependencies]` and `[features]` sections, the lalrpop deps, and `test_ndjson_fixture` from `tests/common/mod.rs`.
- Public path changes from `yajlish::ndjson_handler::NdJsonHandler` to `yajlish_ndjson::NdJsonHandler`.

### No compatibility shim

A `yajlish` feature that re-exports `yajlish-ndjson` isn't possible: `yajlish-ndjson` depends on `yajlish`, and Cargo rejects the cycle even for optional dependencies. So the `ndjson` feature is simply removed. Plan 1 already makes 0.5.0 a breaking release (`ParseError` changes), so the removal ships in that same release with a README and changelog line: "the ndjson converter moved to the `yajlish-ndjson` crate; replace `yajlish::ndjson_handler::` with `yajlish_ndjson::`". **Default picked, flag if wrong.**

### CI

The workflow needs three small edits to cover all members:

- `cargo clippy --workspace --tests --examples`
- `cargo doc --workspace --no-deps`
- `cargo check --workspace --benches`

`cargo test --all` and `cargo fmt --all` already cover the workspace. Since the converter is no longer behind a feature, its tests run on every CI job from here on (Goal 4).

### Publishing order

`yajlish` 0.5.0 first, then `yajlish-ndjson` 0.1.0, because crates.io resolves the `version = "0.5"` requirement at publish time.

## How this interacts with Plans 1 and 2

**Plan 1 (logos lexer).**
- Plan 1 M3's gate is "all existing tests pass, including `--features ndjson`, with no gold edits". After this plan that gate is just `cargo test --all`, and CI enforces it. **Recommended order: land this plan's N1 before Plan 1 M3**, so the lexer swap is checked against the converter automatically instead of by remembering a feature flag.
- Plan 1 Goal 5 (strings and keys delivered raw, quotes and escapes intact) exists partly because `NdJsonHandler` re-emits them verbatim. After the split this is a contract between two crates. It should be written down in the `Handler::handle_string` / `handle_map_key` docs, and the converter's fixture tests become the regression test for it.
- `yajlish-ndjson` never names `ParseError`, so Plan 1 M4's breaking change to it only touches the converter's tests.
- Plan 1 M5 (`decode_str`) is what fixes the selector-quoting bug below (N3).
- Plan 1 M2's split-point harness should also run the two ndjson fixtures. It lives in `yajlish`'s tests; the cleanest way is to expose the chunking `BufRead` from a small `#[doc(hidden)]` test-support module, or copy it (it is ~20 lines). Default: copy it.

**Plan 2 (schema macro).**
- Plan 2 S0 creates the same workspace. If N1 lands first, S0 shrinks to adding two members; if S0 lands first, N1 adds one. Either way `[workspace.package]` from this plan is reused.
- Plan 2's `at = ...` selector uses "the same grammar as `src/selector_parser.lalrpop`", parsed inside the proc macro, and Plan 2 deliberately keeps `yajlish-schema` free of lalrpop. After this split the grammar lives in `yajlish-ndjson`. The grammar is tiny (`d`, then any mix of `.key` and `[n]`), so the simplest arrangement is a hand-written parser of a few dozen lines in each place, or one shared copy if a third user appears. N4 replaces lalrpop in `yajlish-ndjson` with exactly that parser, which also drops the last build script from the workspace.
- Plan 2 notes that `yajlish-schema` stays independent of the `ndjson` feature. That becomes structural: they are sibling crates and neither depends on the other.

## Found while reading, for the follow-up milestones

From reading `src/ndjson_handler.rs`, not from running it; each gets a failing test first.

1. **Write errors panic.** Every write is `.expect("Unable to write")`. A closed pipe (`yajlish-ndjson ... | head`) or a full disk aborts the process. Fix: store the first `io::Error`, return `Status::Abort`, and expose `fn finish(self) -> Result<OUT, io::Error>` (the same pattern Plan 2 uses for `SchemaError`).
2. **`SelectorParser` output never matches.** `SelectorParser::new().parse("d.features")` yields `Identifier("features")` (unit test `test_selector_parser`), but `handle_map_key` compares against the raw key `"\"features\""`. The fixture tests only pass because they build `Identifier("\"features\"")` by hand. Fix: compare against the decoded key using Plan 1 M5's `decode_str`, which also makes escaped keys (`"features"`) match correctly.
3. **Normal completion looks like an abort.** When the selected array closes, `array_end` returns `Status::Abort` (`src/ndjson_handler.rs`, `array_end`), and `Parser::parse` returns `Ok(())`, so the rest of the document is never checked. Malformed JSON after the selected array is silently accepted. Decide whether the converter should keep validating to the end (safer, the high-integrity default) or stop early as an explicit opt-in.
4. **Missing selector is silent.** If the selected key or index never appears, nothing is written and the result is `Ok`. `finish()` from item 1 can report `SelectorNotFound`.
5. **Floats are re-printed, not passed through.** `handle_double` writes `f64` via `Display`, so `1.0` becomes `1`, `1e400` becomes `inf` (invalid JSON) and long mantissas can change. Passing the raw number text through needs a raw-number event from the parser, which is a Plan 1 follow-up; until then, at minimum reject non-finite values instead of writing `inf`.

## Testing

1. **The move**: every existing ndjson unit test and both fixtures pass unchanged, now under plain `cargo test --all`.
2. **Write errors** (N2): a `Write` that fails after K bytes, for every K over a fixture, yields `Err` from `finish()` and never panics.
3. **Selector strings** (N3): `SelectorParser` output used directly against the fixtures, plus a key written with escapes.
4. **Differential**: proptest-generate `serde_json::Value` documents containing an array at a known path, and check that each output line parses with `serde_json` to the corresponding array element.
5. **Chunking**: the ndjson fixtures run through Plan 1's split-point harness once it exists.

## Milestones

| # | Deliverable | Done when |
|---|---|---|
| N1 | Workspace + `yajlish-ndjson` crate via `git mv`; `ndjson` feature, `build.rs` and lalrpop removed from `yajlish`; CI flags updated | `cargo test --all` runs and passes the ndjson tests with no gold edits; clippy/fmt/docs clean for the workspace |
| N2 | Write errors returned, not panicked; `finish()`; missing-selector error | Failing-writer test passes |
| N3 | Selector matching on decoded keys (needs Plan 1 M5) | `SelectorParser` output works end to end |
| N4 | Hand-written selector parser replaces lalrpop | `test_selector_parser` passes unchanged; no build scripts left in the workspace |
| N5 | Decision on early stop vs. validate-to-end; non-finite doubles rejected | Malformed-tail and `1e400` tests pass |

N1 is one small PR and has no dependencies, so it can go first. N2 and N4 are independent of the other plans. N3 waits on Plan 1 M5.
