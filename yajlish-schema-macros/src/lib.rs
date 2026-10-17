//! The `handler!` macro of `yajlish-schema`. See that crate's docs.

use proc_macro::TokenStream;

/// Generate a `yajlish::Handler` from JSON Schemas. See `yajlish-schema`.
#[proc_macro]
pub fn handler(_input: TokenStream) -> TokenStream {
    TokenStream::new()
}
