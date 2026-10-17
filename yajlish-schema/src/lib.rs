//! Validate a JSON stream against JSON Schemas, and build typed values from
//! it, with a `yajlish::Handler` generated at compile time.
//!
//! ```ignore
//! yajlish_schema::handler! {
//!     /// Handler for GeoJSON features.
//!     pub struct FeatureHandler;
//!     at = ".features";        // optional: records are the elements of this array
//!     match = unique;          // `unique` (default) or `first`
//!     schemas {
//!         Point   => "schemas/point.schema.json",
//!         Polygon => "schemas/polygon.schema.json",
//!     }
//! }
//! ```
//!
//! The macro reads the schema files (paths relative to `CARGO_MANIFEST_DIR`)
//! and generates:
//!
//! - a Rust type per schema, named by its key in `schemas {}`, plus types
//!   for the objects, enums and tuples inside it;
//! - `FeatureRecord`, an enum with one variant per schema (the handler's
//!   name without `Handler`, plus `Record`);
//! - `FeatureHandler<F>`, a `yajlish::Handler` that validates each record
//!   against every schema at once as events stream in, builds the value of
//!   the schema it matches, and calls `F: FnMut(Result<FeatureRecord,
//!   SchemaError>) -> Status`.
//!
//! After parsing, call `finish()` on the handler: errors that are not about
//! one record (the envelope around `at`, a selector that matched nothing, a
//! document that ended early) are reported there.
//!
//! # Options
//!
//! - `at = ".key[0].other"`: the records are the elements of the array at
//!   that path. Without it the whole document is the one record, and the
//!   callback runs once, at the end of the stream.
//! - `envelope = "file.json"`: with `at`, validate the whole document
//!   against this schema too (without building it). Its errors go to
//!   `finish()`.
//! - `match = unique | first`: whether exactly one schema must match each
//!   record (the default), or the first that matches, in declaration
//!   order, wins.
//! - `build = false`: validate only. No types are generated, nothing is
//!   allocated per value, and the callback receives `FeatureSchema`, a
//!   fieldless enum naming the schema that matched. Validation memory is
//!   proportional to nesting depth, whatever the document size.
//! - `max_depth = 256`: nesting limit for recursive schemas.
//!
//! # Memory
//!
//! Validation streams. Building holds the whole record in memory, as
//! `serde_json` would, so for one large document prefer `build = false` or
//! `at = ...`. With several schemas, each record is built once, not once per
//! candidate, and a candidate is dropped the moment the record violates it:
//! give multi-schema records a discriminating `const` property (such as
//! `"type": {"const": "Point"}`) and all but one candidate are gone as soon
//! as that property is read.
//!
//! # Supported JSON Schema
//!
//! Draft 2020-12 only. `type`, `enum`, `const`, `properties`, `required`,
//! `additionalProperties`, `prefixItems`, `items`, `minItems`, `maxItems`,
//! `minLength`, `maxLength`, `pattern` (with the `pattern` feature, using
//! the `regex` crate's syntax), `minimum`, `maximum`, `exclusiveMinimum`,
//! `exclusiveMaximum`, `multipleOf`, `allOf`, `anyOf`, `oneOf`, `not`,
//! `$ref` (to `#...` pointers in the same file, or to another file of the
//! same `schemas {}` block or `envelope`), `$defs`, and the annotations
//! `title`, `description`, `default`, `examples`, `$comment`, `format`,
//! `deprecated`, `readOnly`, `writeOnly`. `$id` is accepted at a document's
//! root. Any other keyword is a compile error, never silently ignored.

pub mod ir;

mod error;
mod matcher;
mod num;
mod runtime;
mod value;

pub use error::SchemaError;
pub use value::{BuildError, FromValue, Value};
pub use yajlish::{self, Status};
pub use yajlish_schema_macros::handler;

#[cfg(feature = "serde")]
#[doc(hidden)]
pub use serde;

/// Used by generated code. Not part of the public API.
#[doc(hidden)]
pub mod __private {
    use crate::ir::{Node, NodeId, PathSeg};

    /// For optional, nullable fields: a present `null` is `Some(None)`.
    #[cfg(feature = "serde")]
    pub fn double_option<'de, T, D>(d: D) -> Result<Option<Option<T>>, D::Error>
    where
        T: serde::Deserialize<'de>,
        D: serde::Deserializer<'de>,
    {
        serde::Deserialize::deserialize(d).map(Some)
    }
    pub use crate::{
        runtime::{build_error, is_valid, Outcome, Raw, Runtime},
        value::{into_elements, into_members, into_string},
    };

    /// Everything the runtime needs to know about one generated handler.
    pub struct Config {
        pub nodes: &'static [Node],
        /// Name and root node of each schema, in declaration order.
        pub roots: &'static [(&'static str, NodeId)],
        pub match_first: bool,
        pub at: Option<&'static [PathSeg]>,
        pub envelope: Option<NodeId>,
        pub build: bool,
        pub max_depth: u32,
        pub patterns: &'static [&'static str],
        pub regexes: Regexes,
    }

    impl Config {
        pub(crate) fn pattern_matches(&self, index: usize, text: &str) -> bool {
            self.regexes.matches(self.patterns, index, text)
        }
    }

    /// The compiled `pattern`s of a `Config`, compiled on first use.
    pub struct Regexes {
        #[cfg(feature = "pattern")]
        cell: std::sync::OnceLock<Vec<Option<regex::Regex>>>,
    }

    impl Regexes {
        #[must_use]
        #[allow(clippy::new_without_default)]
        pub const fn new() -> Self {
            Regexes {
                #[cfg(feature = "pattern")]
                cell: std::sync::OnceLock::new(),
            }
        }

        #[cfg(feature = "pattern")]
        fn matches(&self, patterns: &[&str], index: usize, text: &str) -> bool {
            // The macro checks the syntax at compile time; a pattern that
            // still fails to compile matches nothing.
            let compiled = self
                .cell
                .get_or_init(|| patterns.iter().map(|p| regex::Regex::new(p).ok()).collect());
            compiled
                .get(index)
                .and_then(Option::as_ref)
                .is_some_and(|r| r.is_match(text))
        }

        #[cfg(not(feature = "pattern"))]
        #[allow(clippy::unused_self)]
        fn matches(&self, _: &[&str], _: usize, _: &str) -> bool {
            // The macro rejects `pattern` without the feature.
            false
        }
    }
}
