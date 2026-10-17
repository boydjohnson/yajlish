# Plan 2: A macro that generates a `Handler` impl from a set of JSON Schemas

Status: draft, decisions recorded 2026-10-17 · Depends on: [Plan 1](01-logos-lexer.md) M5 (string decoding) · New crate: `yajlish-schema`

## What it does

Given N JSON Schema files, the macro generates at compile time:

1. a Rust type per schema (structs, enums, `Vec`s, primitives),
2. an enum `Record` with one variant per schema, and
3. a struct implementing `yajlish::Handler` that, as events stream in, **validates** each record against all N schemas at once, **builds** the typed value for the schema it matches, and hands `Result<Record, SchemaError>` to a user callback, which returns `Status`.

A "record" is the whole document by default, or each element of an array located by a selector (the same idea as `NdJsonHandler`, which emits each element of the selected array as a line). That is what makes "a number of schemas" useful: a stream like a GeoJSON `features` array or an event log, where each element is one of several known shapes, is validated and typed element by element without ever materializing the whole document.

**Default picked, flag if wrong:** validate *and* build typed values. A validate-only mode (no types, no allocation, just `Ok(SchemaId)` / `Err`) falls out of the same IR for free and is included as `build = false`.

## Usage sketch

```rust
yajlish_schema::handler! {
    /// Handler for GeoJSON features.
    pub struct FeatureHandler;
    at = ".features";               // optional; omit = whole document is the record
    match = unique;                 // `unique` (exactly one schema must match) or `first`
    schemas {
        Point   => "schemas/point.schema.json",
        Polygon => "schemas/polygon.schema.json",
    }
}

let mut handler = FeatureHandler::new(|rec: Result<FeatureRecord, SchemaError>| {
    match rec {
        Ok(FeatureRecord::Point(p)) => { /* p: Point */ }
        Ok(FeatureRecord::Polygon(p)) => { /* ... */ }
        Err(e) => eprintln!("{} at {}", e.keyword, e.instance_path),
    }
    Status::Continue
});
let mut parser = yajlish::Parser::new(&mut handler);
parser.parse(&mut reader)?;
parser.finish_parse()?;
handler.finish()?;   // surfaces an error that caused Abort, see "Errors"
```

Paths resolve relative to `CARGO_MANIFEST_DIR`. The macro emits `const _: &[u8] = include_bytes!(...)` per schema so editing a schema triggers a rebuild.

Function-like macro rather than a derive: a derive attaches to one type, and the input here is a set of files plus options.

## Crate layout

Proc macros must live in their own crate, so the repo becomes a Cargo workspace:

```
Cargo.toml                  # [workspace] members = [".", "yajlish-schema", "yajlish-schema-macros"]
src/                        # yajlish (unchanged API)
yajlish-schema-macros/      # proc-macro = true; deps: syn, quote, proc-macro2, serde_json
yajlish-schema/             # runtime support + re-exports the macro; deps: yajlish
```

- `serde_json` is used only inside the proc macro to read schema files at compile time. Neither `yajlish` nor `yajlish-schema` gains a serde dependency at runtime.
- `yajlish-schema` holds the small runtime the generated code calls: `SchemaError`, the frame stack, `decode_str` re-export, number checks, and (behind a `pattern` feature) `regex`.
- CI's `cargo test --all` already covers workspace members.

## Pipeline inside the macro

```
schema files ──parse──▶ serde_json::Value ──lower──▶ IR graph ──check──▶ codegen (types + validator + builder)
```

### 1. Lowering to IR

```rust
struct Ir { nodes: Vec<Node>, roots: Vec<(Ident, NodeId)> }

enum Node {
    Any, Never,
    Null, Bool, Int { bounds: NumBounds }, Number { bounds: NumBounds },
    String { min_len: Option<u64>, max_len: Option<u64>, pattern: Option<String> },
    Enum(Vec<Literal>), Const(Literal),
    Object { props: Vec<(String, NodeId)>, required: Vec<String>, additional: NodeId },
    Array { prefix: Vec<NodeId>, items: NodeId, min: Option<u64>, max: Option<u64> },
    OneOf(Vec<NodeId>), AnyOf(Vec<NodeId>), AllOf(Vec<NodeId>),
    TypeUnion(Vec<NodeId>),   // "type": ["string", "null"]
    Ref(NodeId),              // resolved; may form cycles (recursive schemas)
}
```

`$ref` is resolved during lowering to a `NodeId`, so recursive schemas are just cycles in the graph. Only local refs (`#/$defs/...`, `#/definitions/...`) and refs to other files in the same `schemas {}` block are supported; remote URLs are a compile error.

### 2. Supported keywords (initial)

Draft 2020-12 only (decided 2026-10-17). A `$schema` naming another dialect is a `compile_error!`.

| Area | Supported | Notes |
|---|---|---|
| Types | `type` (single or array) | `integer` vs `number` maps to `handle_int` vs `handle_double`; a `number` node also accepts ints. `1.0` arrives as `handle_double`; per spec it *is* an integer, so integer nodes accept doubles with zero fraction. |
| Objects | `properties`, `required`, `additionalProperties` (bool or schema) | |
| Arrays | `items`, `prefixItems`, `minItems`, `maxItems` | |
| Values | `enum`, `const` | Literals compared after decoding (Plan 1 M5). |
| Numbers | `minimum`, `maximum`, `exclusiveMinimum`, `exclusiveMaximum`, `multipleOf` | `multipleOf` on floats uses an exact-ish check documented in the runtime. |
| Strings | `minLength`, `maxLength` (Unicode scalar count, per spec), `pattern` | `pattern` behind the `pattern` feature; regex compiled once per handler. |
| Composition | `oneOf`, `anyOf`, `allOf`, `$ref`, `$defs` | See "Composition" below. |
| Annotations | `title`, `description`, `default`, `examples`, `$comment`, `format` | `description` becomes rustdoc on generated types. `format` is annotation-only (the 2020-12 default). |

**Anything else is a `compile_error!`, never silently ignored.** For a high-integrity library, a schema that says `"uniqueItems": true` must not quietly validate without it. The error names the file, the JSON Pointer of the keyword, and the keyword. Later candidates: `uniqueItems`, `propertyNames`, `patternProperties`, `dependentRequired`, `minProperties`/`maxProperties`, `if`/`then`/`else`, `contains`.

### 3. The runtime state machine

The generated handler keeps, per live candidate, a stack of frames mirroring the parser's nesting:

```rust
struct Frame {
    node: NodeId,                 // a const index into generated tables
    state: FrameState,            // per node kind, see below
    builder: Option<Partial>,     // in-progress typed value (build mode)
}
enum FrameState {
    Object { seen_required: u64 /* bitset, spills to Vec for >64 */, pending_key: Option<PropIdx> },
    Array  { index: u64 },
    Scalar,
}
```

Each `Handler` method does the same three things: look at the top frame's node, check the event is allowed (`handle_int` on an `Object` node → type error), then push/pop/update. `handle_end_map` checks `required` against the bitset; `handle_end_array` checks `minItems`. `handle_map_key` decodes the key once (Plan 1 M5) and matches it against the node's property list with a generated `match` on the decoded string, which compiles to an efficient comparison.

Why decoded keys are a hard dependency: today keys arrive raw with quotes and escapes, so `"ab"` and `"ab"` are the same key but different strings. Matching raw text would accept documents the schema forbids. Hence Plan 1 M5 first.

**Locating records** (`at = ...`): parsed by the macro at compile time with the same grammar as `src/selector_parser.lalrpop` (`.key` and `[n]`), generated as a const path matcher checked against `Context` depth and the current key/index. Outside a record, events are ignored cheaply. This keeps `yajlish-schema` independent of the `ndjson` feature and of lalrpop at runtime.

### 4. Multiple schemas and composition

Multiple top-level schemas, `oneOf` and `anyOf` all reduce to the same mechanism: **a candidate set**. Each candidate has its own frame stack; each event is fed to every live candidate; a candidate that hits an error is dropped (its error kept for reporting). When the record (or the `oneOf` value) ends:

- `match = unique` / `oneOf`: exactly one candidate must survive, otherwise `SchemaError::NoMatch` or `AmbiguousMatch`.
- `match = first` / `anyOf`: the first surviving candidate in declaration order wins.
- `allOf`: all sub-schemas run as parallel candidates that must all survive; only the first contributes to the built type unless the sub-schemas are plain object fragments, which the macro merges into one struct at compile time.

Cost is O(live candidates) per event. Two compile-time optimizations. Discriminator detection is required for v1 (see Decisions: with one document per stream, candidates otherwise live until the end of the document); type pruning is optional:
- **Discriminator detection**: if every alternative has a `const`/single-`enum` property under the same key (e.g. `"type": "Point"`), generate a dispatch on that key and drop the non-matching candidates as soon as it is seen.
- **Type pruning**: candidates whose root type can't accept the first event are dropped immediately.

Building typed values per candidate is the expensive part when several candidates survive for a long time. Build mode therefore buffers builders per candidate only while more than one is alive; with a discriminator present that window is typically a single key.

### 5. Generated types

- object → `struct` with `#[derive(Debug, Clone, PartialEq)]`; optional props → `Option<T>`; `additionalProperties: <schema>` → an extra `BTreeMap<String, T>` field.
- array → `Vec<T>`; `prefixItems` → tuple struct.
- `oneOf`/`anyOf` → `enum`; `type: [X, "null"]` → `Option<X>`.
- `integer` → `i64`, `number` → `f64`, string → `String`, `enum` of strings → Rust `enum`.
- Names come from the schema key in `schemas {}`, then `title`, then the property path (`PointGeometry`); collisions are a compile error suggesting a `title`.
- Optional `serde` feature adds `Serialize/Deserialize` derives for users who want them.

`typify` (0.8) was considered for type generation. It produces good types but has no notion of incremental building, and the builder must follow the same IR node-for-node, so we generate both from our own IR. Worth revisiting if its type naming is clearly better.

## Errors

```rust
pub struct SchemaError {
    pub instance_path: String,   // JSON Pointer into the record, e.g. "/geometry/coordinates/0"
    pub schema_path: String,     // e.g. "point.schema.json#/properties/geometry/type"
    pub keyword: &'static str,   // "type", "required", "maximum", ...
    pub offset: Option<u64>,     // byte offset, once Plan 1 M4 lands
    pub detail: String,
}
```

The instance path is maintained from the frame stack (keys and indices), so it costs nothing until an error is built.

`Handler` methods can only return `Status`, and `Parser::parse` returns `Ok(())` after `Abort`. So the generated handler stores a fatal error (e.g. a document-level error with no `at` selector) and exposes it via `handler.finish() -> Result<(), SchemaError>`. Per-record errors go to the callback, which decides whether to `Continue` or `Abort`. If the parser later grows an error-carrying abort (see the parser-hardening list in Plan 1), `finish()` can go away.

## Testing

1. **Official JSON-Schema-Test-Suite** (draft2020-12 directory), filtered to supported keywords. A build script or `macro_rules!` test generator turns each case into a test that streams the instance through `Parser` and compares valid/invalid with the suite's expectation.
2. **Differential testing against the `jsonschema` crate** (0.58, dev-dependency only): for each schema in `tests/schemas/`, proptest-generate instances (from the schema and random mutations of them) and assert our verdict equals `jsonschema`'s.
3. **Round-trip of built values**: with the `serde` feature on in tests, serialize the built value and compare to `serde_json`'s parse of the input.
4. **Compile-fail tests** with `trybuild`: unsupported keyword, remote `$ref`, name collision, bad selector, missing file. Each asserts the error message points at the right file and keyword.
5. **Streaming/chunking**: reuse Plan 1's split-point harness so every schema test also runs with 1-byte reads.
6. **Fixtures**: the existing `tests/ndjson_fixtures/custom.geo.json` with GeoJSON schemas for `Point`/`Polygon`/`MultiPolygon` as an end-to-end example, which doubles as the README example.

## Milestones

| # | Deliverable | Done when |
|---|---|---|
| S0 | Workspace split, empty `yajlish-schema` and `-macros` crates, CI green | `cargo test --all` passes |
| S1 | Lowering + IR for scalars, objects, arrays, `required`, `$ref`/`$defs`; unsupported keyword → `compile_error!` | trybuild tests pass |
| S2 | Validate-only codegen (`build = false`) for one schema, whole document | Test-suite subset passes; differential test vs `jsonschema` passes |
| S3 | Multiple schemas + `oneOf`/`anyOf`/`allOf` via candidate sets; `match = unique/first`; discriminator detection | Ambiguity and no-match cases covered; discriminated candidates pruned at the key |
| S4 | `at = ...` record selection | GeoJSON fixture validates per feature |
| S5 | Type generation + builders | Round-trip tests pass |
| S6 | Numeric/string constraint keywords, `pattern` feature | Bench shows discriminator path near single-schema cost |

S0–S1 can start before Plan 1 lands; S2 needs Plan 1 M5 for correct key matching.

## Decisions (Boyd, 2026-10-17)

1. **Validate and build.** Build mode is the default; `build = false` stays as a cheap validate-only mode.
2. **Draft 2020-12 only.** Draft-07 compatibility (`definitions`, array-form `items`) is dropped from v1; a `$schema` naming any other dialect is a `compile_error!`.
3. **One document per stream** is the main case. Consequences for the design:
   - The whole document is the record by default, and the callback fires once, at the end of the stream. `at = ...` stays, but as an option for large documents rather than the headline feature.
   - Validation still streams: memory is O(nesting depth), whatever the document size.
   - Building the typed value means holding the entire document in memory, as `serde_json` would. For large documents that undoes the point of streaming, so the docs should steer users to `build = false` or to `at = ...` there.
   - With several schemas, the question becomes "which kind of document is this?". Without a discriminator, every candidate may stay alive until the last byte, and in build mode each one holds its own partial value (up to N× the document in memory). That makes discriminator detection (section 4) important, not optional: it moves from S6 to S3, and the docs recommend a `const` type field for multi-schema use.
   - `at = ...` needs one refinement under this model: the envelope around the selected array (e.g. a FeatureCollection's `type`) is validated against the schema too, while elements are emitted one at a time. Record errors go to the callback; envelope errors go to `finish()`.
   - Matches the parser: `Parser` already rejects a second top-level value, so NDJSON-style input stays out of scope.
