//! The `handler!` macro of `yajlish-schema`. See that crate's docs.

mod lower;
mod types;

use lower::{Doc, Ir, Kind, Lit, LowerError, Num, Type};
use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote};
use std::collections::HashSet;
use syn::{
    braced,
    parse::{Parse, ParseStream},
    Attribute, Ident, LitBool, LitInt, LitStr, Token, Visibility,
};
use types::{Def, DefKind, Plan, Ty};

/// Generate a `yajlish::Handler` that validates a JSON stream against JSON
/// Schemas and builds typed values. See the `yajlish-schema` crate docs.
#[proc_macro]
pub fn handler(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let input = syn::parse_macro_input!(input as Input);
    match expand(&input) {
        Ok(ts) => ts.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

struct Input {
    attrs: Vec<Attribute>,
    vis: Visibility,
    name: Ident,
    at: Option<LitStr>,
    envelope: Option<LitStr>,
    match_first: bool,
    build: bool,
    max_depth: u32,
    schemas: Vec<(Ident, LitStr)>,
}

impl Parse for Input {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let attrs = input.call(Attribute::parse_outer)?;
        let vis: Visibility = input.parse()?;
        input.parse::<Token![struct]>()?;
        let name: Ident = input.parse()?;
        input.parse::<Token![;]>()?;

        let mut out = Input {
            attrs,
            vis,
            name,
            at: None,
            envelope: None,
            match_first: false,
            build: true,
            max_depth: 256,
            schemas: vec![],
        };
        let mut seen = HashSet::new();
        let mut have_schemas = false;
        while !input.is_empty() {
            if input.peek(Token![match]) {
                let kw: Token![match] = input.parse()?;
                if !seen.insert("match".to_owned()) {
                    return Err(syn::Error::new(kw.span, "`match` given twice"));
                }
                input.parse::<Token![=]>()?;
                let mode: Ident = input.parse()?;
                out.match_first = match mode.to_string().as_str() {
                    "unique" => false,
                    "first" => true,
                    _ => return Err(syn::Error::new(mode.span(), "expected `unique` or `first`")),
                };
                input.parse::<Token![;]>()?;
                continue;
            }
            let key: Ident = input.parse()?;
            let k = key.to_string();
            if k == "schemas" {
                if have_schemas {
                    return Err(syn::Error::new(key.span(), "`schemas` given twice"));
                }
                have_schemas = true;
                let content;
                braced!(content in input);
                while !content.is_empty() {
                    let ident: Ident = content.parse()?;
                    content.parse::<Token![=>]>()?;
                    let path: LitStr = content.parse()?;
                    if out.schemas.iter().any(|(i, _)| *i == ident) {
                        return Err(syn::Error::new(ident.span(), "duplicate schema name"));
                    }
                    out.schemas.push((ident, path));
                    if !content.is_empty() {
                        content.parse::<Token![,]>()?;
                    }
                }
                if out.schemas.is_empty() {
                    return Err(syn::Error::new(
                        key.span(),
                        "`schemas` needs at least one schema",
                    ));
                }
                continue;
            }
            if !seen.insert(k.clone()) {
                return Err(syn::Error::new(key.span(), format!("`{}` given twice", k)));
            }
            input.parse::<Token![=]>()?;
            match k.as_str() {
                "at" => out.at = Some(input.parse()?),
                "envelope" => out.envelope = Some(input.parse()?),
                "build" => out.build = input.parse::<LitBool>()?.value,
                "max_depth" => out.max_depth = input.parse::<LitInt>()?.base10_parse()?,
                _ => {
                    return Err(syn::Error::new(
                        key.span(),
                        "expected `at`, `envelope`, `match`, `build`, `max_depth` or `schemas`",
                    ))
                }
            }
            input.parse::<Token![;]>()?;
        }
        if !have_schemas {
            return Err(input.error("missing `schemas { Name => \"path\", ... }`"));
        }
        if let (Some(e), None) = (&out.envelope, &out.at) {
            return Err(syn::Error::new(e.span(), "`envelope` needs `at`"));
        }
        Ok(out)
    }
}

/// Parse `.key[0].other` into path segments.
fn parse_at(lit: &LitStr) -> syn::Result<Vec<(Option<String>, Option<u64>)>> {
    let s = lit.value();
    let err = |msg: &str| {
        syn::Error::new(
            lit.span(),
            format!(
                "{}: expected `.key` and `[index]` steps, like \".features\" or \".a[0].b\"",
                msg
            ),
        )
    };
    let mut out = vec![];
    let mut rest = s.as_str();
    while !rest.is_empty() {
        if let Some(r) = rest.strip_prefix('.') {
            let len = r.find(['.', '[']).unwrap_or(r.len());
            if len == 0 {
                return Err(err("empty key"));
            }
            out.push((Some(r[..len].to_owned()), None));
            rest = &r[len..];
        } else if let Some(r) = rest.strip_prefix('[') {
            let len = r.find(']').ok_or_else(|| err("missing `]`"))?;
            let n: u64 = r[..len].parse().map_err(|_| err("bad index"))?;
            out.push((None, Some(n)));
            rest = &r[len + 1..];
        } else {
            return Err(err("unexpected character"));
        }
    }
    if out.is_empty() {
        return Err(err("empty selector"));
    }
    Ok(out)
}

#[cfg(feature = "pattern")]
fn check_pattern(p: &str) -> Result<(), String> {
    regex::Regex::new(p)
        .map(|_| ())
        .map_err(|e| format!("invalid regex: {}", e))
}

#[allow(clippy::too_many_lines)]
fn expand(input: &Input) -> syn::Result<TokenStream> {
    let manifest = std::env::var("CARGO_MANIFEST_DIR")
        .map_err(|_| syn::Error::new(Span::call_site(), "CARGO_MANIFEST_DIR is not set"))?;

    // Documents: the schemas in order, then the envelope.
    let mut files: Vec<&LitStr> = input.schemas.iter().map(|(_, p)| p).collect();
    if let Some(e) = &input.envelope {
        files.push(e);
    }
    let mut docs = vec![];
    let mut abs_paths = vec![];
    for lit in &files {
        let rel = lit.value();
        let abs = std::path::Path::new(&manifest).join(&rel);
        let text = std::fs::read(&abs).map_err(|e| {
            syn::Error::new(lit.span(), format!("cannot read {}: {}", abs.display(), e))
        })?;
        let value: serde_json::Value = serde_json::from_slice(&text).map_err(|e| {
            syn::Error::new(lit.span(), format!("{} is not valid JSON: {}", rel, e))
        })?;
        docs.push(Doc { name: rel, value });
        abs_paths.push(abs.to_string_lossy().into_owned());
    }

    #[cfg(feature = "pattern")]
    let check: lower::PatternCheck<'_> = Some(&check_pattern);
    #[cfg(not(feature = "pattern"))]
    let check: lower::PatternCheck<'_> = None;

    let ir = lower::lower(&docs, check).map_err(|e: LowerError| {
        let span = files
            .iter()
            .find(|l| l.value() == e.file)
            .map_or_else(Span::call_site, |l| l.span());
        syn::Error::new(span, e.to_string())
    })?;

    let name = &input.name;
    let vis = &input.vis;
    let attrs = &input.attrs;
    let base = name.to_string();
    let base = base
        .strip_suffix("Handler")
        .filter(|b| !b.is_empty())
        .unwrap_or(&base);
    let record_ident = format_ident!("{}Record", base);
    let schema_ident = format_ident!("{}Schema", base);
    let module = format_ident!("__yajlish_schema_{}", types::snake(&name.to_string()));

    let n_nodes = ir.nodes.len();
    let nodes = ir.nodes.iter().map(|n| node_tokens(&ir, n));
    let roots = input.schemas.iter().enumerate().map(|(i, (ident, _))| {
        let s = ident.to_string();
        let r = ir.doc_roots[i];
        quote!((#s, #r))
    });
    let envelope = match &input.envelope {
        Some(_) => {
            let r = ir.doc_roots[input.schemas.len()];
            quote!(::core::option::Option::Some(#r))
        }
        None => quote!(::core::option::Option::None),
    };
    let at = match &input.at {
        Some(lit) => {
            let segs = parse_at(lit)?.into_iter().map(|seg| match seg {
                (Some(k), _) => quote!(::yajlish_schema::ir::PathSeg::Key(#k)),
                (None, Some(i)) => quote!(::yajlish_schema::ir::PathSeg::Index(#i)),
                (None, None) => unreachable!(),
            });
            quote!(::core::option::Option::Some(&[#(#segs),*]))
        }
        None => quote!(::core::option::Option::None),
    };
    let roots_only: Vec<usize> = (0..input.schemas.len()).map(|i| ir.doc_roots[i]).collect();
    let discriminator = match lower::discriminator(&ir, &roots_only) {
        Some((path, values)) => {
            let values = values.iter().map(|(v, i)| quote!((#v, #i)));
            quote!(::core::option::Option::Some(::yajlish_schema::__private::Discriminator {
                path: &[#(#path),*],
                values: &[#(#values),*],
            }))
        }
        None => quote!(::core::option::Option::None),
    };
    let patterns = ir.patterns.iter();
    let match_first = input.match_first;
    let build = input.build;
    let max_depth = input.max_depth;
    let includes = abs_paths.iter().map(|p| {
        quote!(
            const _: &[u8] = include_bytes!(#p);
        )
    });

    let tables = quote! {
        #[doc(hidden)]
        #[allow(non_snake_case, clippy::all, clippy::pedantic)]
        mod #module {
            #(#includes)*
            pub(super) static NODES: [::yajlish_schema::ir::Node; #n_nodes] = [#(#nodes),*];
            pub(super) static CONFIG: ::yajlish_schema::__private::Config = ::yajlish_schema::__private::Config {
                nodes: &NODES,
                roots: &[#(#roots),*],
                match_first: #match_first,
                at: #at,
                envelope: #envelope,
                build: #build,
                max_depth: #max_depth,
                patterns: &[#(#patterns),*],
                regexes: ::yajlish_schema::__private::Regexes::new(),
                discriminator: #discriminator,
            };
        }
    };

    let schema_docs: Vec<TokenStream> = input
        .schemas
        .iter()
        .enumerate()
        .map(|(i, _)| doc_attr(ir.nodes[ir.doc_roots[i]].description.as_deref()))
        .collect();
    let schema_idents: Vec<&Ident> = input.schemas.iter().map(|(i, _)| i).collect();
    let indices: Vec<usize> = (0..input.schemas.len()).collect();

    let (callback_ty, types_ts, build_fn) = if build {
        let mut root_names: Vec<Option<String>> = input
            .schemas
            .iter()
            .map(|(i, _)| Some(i.to_string()))
            .collect();
        if input.envelope.is_some() {
            root_names.push(None);
        }
        let plan = types::plan(&ir, &root_names).map_err(|e| {
            syn::Error::new(
                Span::call_site(),
                format!(
                    "two schemas would both generate a type named `{}`: {} and {}; give one of them a distinct `title`",
                    e.name, e.first, e.second
                ),
            )
        })?;
        let defs = plan.defs.iter().map(|d| def_tokens(&plan, d, &module, vis));
        let aliases = input
            .schemas
            .iter()
            .zip(&plan.roots)
            .filter_map(|((ident, _), ty)| {
                let named_same = matches!(ty, Ty::Named(i) if *ident == plan.defs[*i].name);
                if named_same {
                    None
                } else {
                    let t = ty_tokens(&plan, ty);
                    Some(quote!(#vis type #ident = #t;))
                }
            });
        let types_ts = quote! {
            /// One record, typed by the schema it matched.
            #[derive(::core::fmt::Debug, ::core::clone::Clone, ::core::cmp::PartialEq)]
            #vis enum #record_ident {
                #(#schema_docs #schema_idents(#schema_idents),)*
            }
            #(#defs)*
            #(#aliases)*
        };
        let build_fn = quote! {
            fn __build(
                index: usize,
                value: ::core::option::Option<::yajlish_schema::Value>,
            ) -> ::core::result::Result<#record_ident, ::yajlish_schema::SchemaError> {
                let value = match value {
                    ::core::option::Option::Some(v) => v,
                    ::core::option::Option::None => ::yajlish_schema::Value::Null,
                };
                match index {
                    #(#indices => <#schema_idents as ::yajlish_schema::FromValue>::from_value(value)
                        .map(#record_ident::#schema_idents)
                        .map_err(|e| ::yajlish_schema::__private::build_error(&#module::CONFIG, #indices, e)),)*
                    _ => ::core::result::Result::Err(::yajlish_schema::__private::build_error(
                        &#module::CONFIG,
                        0,
                        ::yajlish_schema::BuildError::new("unknown schema index"),
                    )),
                }
            }
        };
        (quote!(#record_ident), types_ts, build_fn)
    } else {
        let types_ts = quote! {
            /// Which schema a record matched.
            #[derive(::core::fmt::Debug, ::core::clone::Clone, ::core::marker::Copy, ::core::cmp::PartialEq, ::core::cmp::Eq)]
            #vis enum #schema_ident {
                #(#schema_docs #schema_idents,)*
            }
        };
        let build_fn = quote! {
            fn __build(
                index: usize,
                _value: ::core::option::Option<::yajlish_schema::Value>,
            ) -> ::core::result::Result<#schema_ident, ::yajlish_schema::SchemaError> {
                const ALL: &[#schema_ident] = &[#(#schema_ident::#schema_idents),*];
                ALL.get(index).copied().ok_or_else(|| ::yajlish_schema::__private::build_error(
                    &#module::CONFIG,
                    0,
                    ::yajlish_schema::BuildError::new("unknown schema index"),
                ))
            }
        };
        (quote!(#schema_ident), types_ts, build_fn)
    };

    let methods = [
        (quote!(handle_null), quote!(), quote!(Null)),
        (quote!(handle_start_map), quote!(), quote!(StartMap)),
        (quote!(handle_end_map), quote!(), quote!(EndMap)),
        (quote!(handle_start_array), quote!(), quote!(StartArray)),
        (quote!(handle_end_array), quote!(), quote!(EndArray)),
    ]
    .into_iter()
    .map(|(m, _, raw)| {
        quote! {
            fn #m(&mut self, ctx: &::yajlish_schema::yajlish::Context) -> ::yajlish_schema::Status {
                let out = self.rt.event(ctx.offset(), ::yajlish_schema::__private::Raw::#raw);
                self.__dispatch(out)
            }
        }
    });
    let value_methods = [
        (quote!(handle_double), quote!(f64), quote!(Float)),
        (quote!(handle_int), quote!(i64), quote!(Int)),
        (quote!(handle_bool), quote!(bool), quote!(Bool)),
        (quote!(handle_string), quote!(&str), quote!(Str)),
        (quote!(handle_map_key), quote!(&str), quote!(Key)),
    ]
    .into_iter()
    .map(|(m, t, raw)| {
        quote! {
            fn #m(&mut self, ctx: &::yajlish_schema::yajlish::Context, val: #t) -> ::yajlish_schema::Status {
                let out = self.rt.event(ctx.offset(), ::yajlish_schema::__private::Raw::#raw(val));
                self.__dispatch(out)
            }
        }
    });

    let doc_default = if attrs.iter().any(|a| a.path().is_ident("doc")) {
        quote!()
    } else {
        quote!(#[doc = "A `yajlish::Handler` generated by `yajlish_schema::handler!`."])
    };

    Ok(quote! {
        #tables
        #types_ts

        #(#attrs)*
        #doc_default
        #vis struct #name<F> {
            rt: ::yajlish_schema::__private::Runtime,
            callback: F,
        }

        #[allow(clippy::result_large_err)]
        impl<F> #name<F>
        where
            F: ::core::ops::FnMut(::core::result::Result<#callback_ty, ::yajlish_schema::SchemaError>) -> ::yajlish_schema::Status,
        {
            /// A handler that calls `callback` once per record.
            #vis fn new(callback: F) -> Self {
                #name {
                    rt: ::yajlish_schema::__private::Runtime::new(&#module::CONFIG),
                    callback,
                }
            }

            /// Errors that are not about one record: the envelope did not
            /// validate, the `at` array never appeared, a string could not be
            /// decoded, or the document ended early.
            ///
            /// # Errors
            /// The first such error.
            #vis fn finish(self) -> ::core::result::Result<(), ::yajlish_schema::SchemaError> {
                self.rt.finish()
            }

            /// How many schemas can still match the record being read.
            #[doc(hidden)]
            #vis fn live_candidates(&self) -> usize {
                self.rt.live_candidates()
            }

            #build_fn

            fn __dispatch(&mut self, out: ::core::option::Option<::yajlish_schema::__private::Outcome>) -> ::yajlish_schema::Status {
                if let ::core::option::Option::Some(outcome) = out {
                    let record = outcome.and_then(|(index, value)| Self::__build(index, value));
                    if (self.callback)(record) == ::yajlish_schema::Status::Abort {
                        self.rt.note_abort();
                        return ::yajlish_schema::Status::Abort;
                    }
                }
                if self.rt.is_fatal() {
                    ::yajlish_schema::Status::Abort
                } else {
                    ::yajlish_schema::Status::Continue
                }
            }
        }

        #[allow(clippy::result_large_err)]
        impl<F> ::yajlish_schema::yajlish::Handler for #name<F>
        where
            F: ::core::ops::FnMut(::core::result::Result<#callback_ty, ::yajlish_schema::SchemaError>) -> ::yajlish_schema::Status,
        {
            #(#methods)*
            #(#value_methods)*
        }
    })
}

fn doc_attr(doc: Option<&str>) -> TokenStream {
    match doc {
        Some(d) => {
            let lines = d.lines().map(|l| format!(" {}", l));
            quote!(#(#[doc = #lines])*)
        }
        None => quote!(),
    }
}

fn num_tokens(n: Num) -> TokenStream {
    match n {
        Num::Int(i) => quote!(::yajlish_schema::ir::Num::Int(#i)),
        Num::Float(f) => {
            let bits = f.to_bits();
            quote!(::yajlish_schema::ir::Num::Float(f64::from_bits(#bits)))
        }
    }
}

fn lit_tokens(l: &Lit) -> TokenStream {
    match l {
        Lit::Null => quote!(::yajlish_schema::ir::Literal::Null),
        Lit::Bool(b) => quote!(::yajlish_schema::ir::Literal::Bool(#b)),
        Lit::Num(n) => {
            let n = num_tokens(*n);
            quote!(::yajlish_schema::ir::Literal::Num(#n))
        }
        Lit::Str(s) => quote!(::yajlish_schema::ir::Literal::Str(#s)),
        Lit::Array(a) => {
            let items = a.iter().map(lit_tokens);
            quote!(::yajlish_schema::ir::Literal::Array(&[#(#items),*]))
        }
        Lit::Object(m) => {
            let members = m.iter().map(|(k, v)| {
                let v = lit_tokens(v);
                quote!((#k, #v))
            });
            quote!(::yajlish_schema::ir::Literal::Object(&[#(#members),*]))
        }
    }
}

fn opt<T>(v: Option<T>, f: impl FnOnce(T) -> TokenStream) -> TokenStream {
    match v {
        Some(v) => {
            let t = f(v);
            quote!(::core::option::Option::Some(#t))
        }
        None => quote!(::core::option::Option::None),
    }
}

fn node_tokens(_ir: &Ir, n: &lower::Node) -> TokenStream {
    let path = &n.path;
    let kind = match &n.kind {
        Kind::True => quote!(::yajlish_schema::ir::NodeKind::True),
        Kind::False => quote!(::yajlish_schema::ir::NodeKind::False),
        Kind::Schema(s) => {
            let types = match &s.types {
                None => quote!(::yajlish_schema::ir::types::ANY),
                Some(ts) => {
                    let bits = ts.iter().map(|t| match t {
                        Type::Null => quote!(::yajlish_schema::ir::types::NULL),
                        Type::Boolean => quote!(::yajlish_schema::ir::types::BOOLEAN),
                        Type::Integer => quote!(::yajlish_schema::ir::types::INTEGER),
                        Type::Number => quote!(::yajlish_schema::ir::types::NUMBER),
                        Type::String => quote!(::yajlish_schema::ir::types::STRING),
                        Type::Array => quote!(::yajlish_schema::ir::types::ARRAY),
                        Type::Object => quote!(::yajlish_schema::ir::types::OBJECT),
                    });
                    quote!(0 #(| #bits)*)
                }
            };
            let enum_ = opt(s.enum_.as_ref(), |e| {
                let items = e.iter().map(lit_tokens);
                quote!(&[#(#items),*])
            });
            let const_ = opt(s.const_.as_ref(), |c| {
                let c = lit_tokens(c);
                quote!(&#c)
            });
            let usize_opt = |v: Option<usize>| opt(v, |v| quote!(#v));
            let u64_opt = |v: Option<u64>| opt(v, |v| quote!(#v));
            let num_opt = |v: Option<Num>| opt(v, num_tokens);
            let ref_ = usize_opt(s.ref_);
            let not = usize_opt(s.not);
            let additional = usize_opt(s.additional_properties);
            let items = usize_opt(s.items);
            let pattern = usize_opt(s.pattern);
            let all_of = &s.all_of;
            let any_of = &s.any_of;
            let one_of = &s.one_of;
            let prefix = &s.prefix_items;
            let props = s.properties.iter().map(|(k, id)| quote!((#k, #id)));
            let required = &s.required;
            let (min_items, max_items) = (u64_opt(s.min_items), u64_opt(s.max_items));
            let (min_length, max_length) = (u64_opt(s.min_length), u64_opt(s.max_length));
            let (minimum, maximum) = (num_opt(s.minimum), num_opt(s.maximum));
            let (exmin, exmax) = (num_opt(s.exclusive_minimum), num_opt(s.exclusive_maximum));
            let multiple = num_opt(s.multiple_of);
            quote!(::yajlish_schema::ir::NodeKind::Schema(::yajlish_schema::ir::Schema {
                types: #types,
                enum_: #enum_,
                const_: #const_,
                ref_: #ref_,
                all_of: &[#(#all_of),*],
                any_of: &[#(#any_of),*],
                one_of: &[#(#one_of),*],
                not: #not,
                properties: &[#(#props),*],
                required: &[#(#required),*],
                additional_properties: #additional,
                prefix_items: &[#(#prefix),*],
                items: #items,
                min_items: #min_items,
                max_items: #max_items,
                min_length: #min_length,
                max_length: #max_length,
                pattern: #pattern,
                minimum: #minimum,
                maximum: #maximum,
                exclusive_minimum: #exmin,
                exclusive_maximum: #exmax,
                multiple_of: #multiple,
            }))
        }
    };
    quote!(::yajlish_schema::ir::Node { path: #path, kind: #kind })
}

const KEYWORDS: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "dyn", "else", "enum", "extern", "false",
    "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref",
    "return", "static", "struct", "trait", "true", "type", "unsafe", "use", "where", "while",
    "abstract", "become", "box", "do", "final", "gen", "macro", "override", "priv", "try",
    "typeof", "unsized", "virtual", "yield",
];

/// An identifier for `name`, raw if it is a keyword.
fn ident(name: &str) -> Ident {
    match name {
        "self" | "Self" | "super" | "crate" | "_" => format_ident!("{}_", name),
        n if KEYWORDS.contains(&n) => Ident::new_raw(n, Span::call_site()),
        n => Ident::new(n, Span::call_site()),
    }
}

fn ty_tokens(plan: &Plan, t: &Ty) -> TokenStream {
    match t {
        Ty::Value => quote!(::yajlish_schema::Value),
        Ty::Unit => quote!(()),
        Ty::Bool => quote!(bool),
        Ty::I64 => quote!(i64),
        Ty::F64 => quote!(f64),
        Ty::String => quote!(::std::string::String),
        Ty::Option(inner) => {
            let i = ty_tokens(plan, inner);
            quote!(::core::option::Option<#i>)
        }
        Ty::Vec(inner) => {
            let i = ty_tokens(plan, inner);
            quote!(::std::vec::Vec<#i>)
        }
        Ty::Map(inner) => {
            let i = ty_tokens(plan, inner);
            quote!(::std::collections::BTreeMap<::std::string::String, #i>)
        }
        Ty::Box(inner) => {
            let i = ty_tokens(plan, inner);
            quote!(::std::boxed::Box<#i>)
        }
        Ty::Named(i) => {
            let id = ident(&plan.defs[*i].name);
            quote!(#id)
        }
    }
}

fn serde_derive() -> TokenStream {
    if cfg!(feature = "serde") {
        quote! {
            #[derive(::yajlish_schema::serde::Serialize, ::yajlish_schema::serde::Deserialize)]
            #[serde(crate = "::yajlish_schema::serde")]
        }
    } else {
        quote!()
    }
}

fn serde_attr(ts: TokenStream) -> TokenStream {
    if cfg!(feature = "serde") {
        quote!(#[serde(#ts)])
    } else {
        quote!()
    }
}

#[allow(clippy::too_many_lines)]
fn def_tokens(plan: &Plan, d: &Def, module: &Ident, vis: &Visibility) -> TokenStream {
    let name = ident(&d.name);
    let doc = doc_attr(d.doc.as_deref());
    let derive_serde = serde_derive();
    let derives =
        quote!(#[derive(::core::fmt::Debug, ::core::clone::Clone, ::core::cmp::PartialEq)]);
    match &d.kind {
        DefKind::Struct { fields, additional } => {
            let decls = fields.iter().map(|f| {
                let id = ident(&f.ident);
                let ty = ty_tokens(plan, &f.ty);
                let json = &f.json;
                let field_doc = format!(" The `{}` property.", json);
                if f.required {
                    let rename = serde_attr(quote!(rename = #json));
                    quote!(#[doc = #field_doc] #rename pub #id: #ty)
                } else if matches!(f.ty, Ty::Option(_)) {
                    // Keep "absent" (None) apart from "null" (Some(None)).
                    let attr = serde_attr(quote!(
                        rename = #json,
                        default,
                        skip_serializing_if = "::core::option::Option::is_none",
                        deserialize_with = "::yajlish_schema::__private::double_option"
                    ));
                    quote!(#[doc = #field_doc] #attr pub #id: ::core::option::Option<#ty>)
                } else {
                    let attr = serde_attr(quote!(rename = #json, default, skip_serializing_if = "::core::option::Option::is_none"));
                    quote!(#[doc = #field_doc] #attr pub #id: ::core::option::Option<#ty>)
                }
            });
            let extra = additional.as_ref().map(|t| {
                let ty = ty_tokens(plan, t);
                let attr = serde_attr(quote!(flatten));
                quote!(
                    /// Members not listed in `properties`.
                    #attr pub additional_properties: ::std::collections::BTreeMap<::std::string::String, #ty>,
                )
            });
            let locals: Vec<Ident> = (0..fields.len()).map(|i| format_ident!("f{}", i)).collect();
            let arms = fields.iter().zip(&locals).map(|(f, l)| {
                let json = &f.json;
                quote!(#json => { #l = ::core::option::Option::Some(
                    ::yajlish_schema::FromValue::from_value(v).map_err(|e| e.in_key(#json))?); })
            });
            let other_arm = if additional.is_some() {
                quote!(_ => {
                    let item = ::yajlish_schema::FromValue::from_value(v).map_err(|e| e.in_key(&k))?;
                    additional.insert(k, item);
                })
            } else {
                quote!(_ => {})
            };
            let inits = fields.iter().zip(&locals).map(|(f, l)| {
                let id = ident(&f.ident);
                let json = &f.json;
                if f.required {
                    let msg = format!("missing required property {:?}", json);
                    quote!(#id: #l.ok_or_else(|| ::yajlish_schema::BuildError::new(#msg))?)
                } else {
                    quote!(#id: #l)
                }
            });
            let extra_init = additional
                .as_ref()
                .map(|_| quote!(additional_properties: additional,));
            let extra_decl = additional
                .as_ref()
                .map(|_| quote!(let mut additional = ::std::collections::BTreeMap::new();));
            quote! {
                #doc
                #derives
                #derive_serde
                #vis struct #name {
                    #(#decls,)*
                    #extra
                }

                impl ::yajlish_schema::FromValue for #name {
                    #[allow(unused_mut)]
                    fn from_value(value: ::yajlish_schema::Value) -> ::core::result::Result<Self, ::yajlish_schema::BuildError> {
                        #(let mut #locals = ::core::option::Option::None;)*
                        #extra_decl
                        for (k, v) in ::yajlish_schema::__private::into_members(value)? {
                            match k.as_str() {
                                #(#arms)*
                                #other_arm
                            }
                        }
                        ::core::result::Result::Ok(#name {
                            #(#inits,)*
                            #extra_init
                        })
                    }
                }
            }
        }
        DefKind::Tuple(elems) => {
            let tys: Vec<TokenStream> = elems.iter().map(|t| ty_tokens(plan, t)).collect();
            let n = elems.len();
            let idx = 0..n;
            quote! {
                #doc
                #derives
                #derive_serde
                #vis struct #name(#(pub #tys),*);

                impl ::yajlish_schema::FromValue for #name {
                    fn from_value(value: ::yajlish_schema::Value) -> ::core::result::Result<Self, ::yajlish_schema::BuildError> {
                        let mut it = ::yajlish_schema::__private::into_elements(value, #n)?;
                        ::core::result::Result::Ok(#name(#(
                            ::yajlish_schema::FromValue::from_value(
                                it.next().ok_or_else(|| ::yajlish_schema::BuildError::new("missing element"))?
                            ).map_err(|e| e.in_index(#idx))?
                        ),*))
                    }
                }
            }
        }
        DefKind::StrEnum(variants) => {
            let vs: Vec<Ident> = variants.iter().map(|(v, _)| ident(v)).collect();
            let values: Vec<&String> = variants.iter().map(|(_, s)| s).collect();
            let renames = values.iter().map(|v| serde_attr(quote!(rename = #v)));
            let docs = values.iter().map(|v| format!(" `{:?}`", v));
            quote! {
                #doc
                #[derive(::core::fmt::Debug, ::core::clone::Clone, ::core::marker::Copy, ::core::cmp::PartialEq, ::core::cmp::Eq, ::core::hash::Hash)]
                #derive_serde
                #vis enum #name {
                    #(#[doc = #docs] #renames #vs,)*
                }

                impl #name {
                    /// The string this variant stands for.
                    #vis fn as_str(&self) -> &'static str {
                        match self {
                            #(#name::#vs => #values,)*
                        }
                    }
                }

                impl ::yajlish_schema::FromValue for #name {
                    fn from_value(value: ::yajlish_schema::Value) -> ::core::result::Result<Self, ::yajlish_schema::BuildError> {
                        let s = ::yajlish_schema::__private::into_string(value)?;
                        match s.as_str() {
                            #(#values => ::core::result::Result::Ok(#name::#vs),)*
                            other => ::core::result::Result::Err(::yajlish_schema::BuildError::new(
                                ::std::format!("unexpected value {:?}", other),
                            )),
                        }
                    }
                }
            }
        }
        DefKind::Union(variants) => {
            let vs: Vec<Ident> = variants.iter().map(|(v, _, _)| ident(v)).collect();
            let nodes: Vec<usize> = variants.iter().map(|(_, n, _)| *n).collect();
            let tys = variants.iter().map(|(_, _, t)| ty_tokens(plan, t));
            let untagged = serde_attr(quote!(untagged));
            quote! {
                #doc
                #derives
                #derive_serde
                #untagged
                #vis enum #name {
                    #(#vs(#tys),)*
                }

                impl ::yajlish_schema::FromValue for #name {
                    fn from_value(value: ::yajlish_schema::Value) -> ::core::result::Result<Self, ::yajlish_schema::BuildError> {
                        #(
                            if ::yajlish_schema::__private::is_valid(&#module::CONFIG, #nodes, &value) {
                                return ::yajlish_schema::FromValue::from_value(value).map(#name::#vs);
                            }
                        )*
                        ::core::result::Result::Err(::yajlish_schema::BuildError::new("no alternative matched"))
                    }
                }
            }
        }
    }
}

/// What the macro reports for schemas it cannot accept. (These run
/// `expand` directly: trybuild compiles its test crates elsewhere, where
/// schema paths relative to `CARGO_MANIFEST_DIR` would not resolve.)
#[cfg(test)]
mod tests {
    use super::*;

    fn error(input: &str) -> String {
        let input: Input = syn::parse_str(input).expect("macro input parses");
        match expand(&input) {
            Ok(_) => panic!("expected an error for {}", input.name),
            Err(e) => e.to_string(),
        }
    }

    fn one(file: &str) -> String {
        error(&format!(
            "struct H; schemas {{ A => \"tests/schemas/{}\" }}",
            file
        ))
    }

    fn assert_contains(msg: &str, parts: &[&str]) {
        for p in parts {
            assert!(msg.contains(p), "{:?} does not mention {:?}", msg, p);
        }
    }

    #[test]
    fn test_unsupported_keyword_names_file_pointer_and_keyword() {
        assert_contains(
            &one("unsupported.json"),
            &[
                "tests/schemas/unsupported.json#/items/properties/tags/uniqueItems",
                "`uniqueItems`",
                "unsupported keyword",
            ],
        );
    }

    #[test]
    fn test_remote_ref() {
        assert_contains(
            &one("remote_ref.json"),
            &["remote_ref.json#/properties/a/$ref", "remote `$ref`"],
        );
    }

    #[test]
    fn test_other_dialect() {
        assert_contains(&one("draft7.json"), &["draft7.json#/$schema", "2020-12"]);
    }

    #[test]
    fn test_ref_cycle() {
        assert_contains(&one("ref_cycle.json"), &["without consuming any input"]);
    }

    #[test]
    fn test_name_collision_suggests_a_title() {
        assert_contains(
            &one("collision.json"),
            &["`Thing`", "/properties/a", "/properties/b", "`title`"],
        );
    }

    #[test]
    fn test_dangling_ref() {
        assert_contains(
            &one("dangling_ref.json"),
            &["#/$defs/missing", "points at nothing"],
        );
    }

    #[test]
    fn test_invalid_keyword_value() {
        assert_contains(
            &one("invalid_value.json"),
            &["/minimum", "expected a number"],
        );
    }

    #[test]
    fn test_missing_file_and_bad_json() {
        assert_contains(&one("nope.json"), &["cannot read", "nope.json"]);
        assert_contains(&one("not_json.json"), &["not valid JSON"]);
    }

    #[test]
    fn test_pattern_needs_the_feature() {
        if cfg!(feature = "pattern") {
            let input: Input =
                syn::parse_str("struct H; schemas { A => \"tests/schemas/pattern.json\" }")
                    .unwrap();
            assert!(expand(&input).is_ok());
        } else {
            assert_contains(&one("pattern.json"), &["`pattern` feature"]);
        }
    }

    #[test]
    fn test_bad_options() {
        let s = "schemas { A => \"tests/schemas/ok.json\" }";
        assert_contains(
            &error(&format!("struct H; at = \"features\"; {}", s)),
            &["`.key`"],
        );
        assert_contains(
            &error(&format!("struct H; at = \".a[x]\"; {}", s)),
            &["bad index"],
        );
        assert!(
            syn::parse_str::<Input>(&format!("struct H; envelope = \"x.json\"; {}", s))
                .is_err_and(|e| e.to_string().contains("`envelope` needs `at`"))
        );
        assert!(
            syn::parse_str::<Input>(&format!("struct H; match = most; {}", s))
                .is_err_and(|e| e.to_string().contains("`unique` or `first`"))
        );
        assert!(syn::parse_str::<Input>("struct H; schemas {}")
            .is_err_and(|e| e.to_string().contains("at least one")));
        assert!(syn::parse_str::<Input>("struct H;")
            .is_err_and(|e| e.to_string().contains("missing `schemas")));
        assert!(syn::parse_str::<Input>(&format!("struct H; colour = 1; {}", s)).is_err());
    }
}
