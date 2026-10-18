//! Fuzzing of the name helpers (Plan 4, K5, `names_are_valid_idents`). The
//! plan put this in `tests/`, but a proc-macro crate can only export
//! macros, so it is a unit test module.

use crate::{
    ident,
    types::{pascal, snake},
};

/// `ident(name)` does not panic, and its text parses back as one
/// identifier: `syn` rejects keywords that are not raw, so a keyword
/// missing from `KEYWORDS` fails here instead of in the generated code.
fn check_ident(name: &str) {
    let id = ident(name).to_string();
    assert!(
        syn::parse_str::<syn::Ident>(&id).is_ok(),
        "{:?} -> {:?} is not an identifier",
        name,
        id
    );
}

/// `snake` always gives a name `ident` accepts. `pascal` may give `""`
/// (its callers replace that with `"Empty"`, or append it to a non-empty
/// parent name), and otherwise a name `ident` accepts.
fn check_names(s: &str) {
    check_ident(&snake(s));
    let p = pascal(s);
    if !p.is_empty() {
        check_ident(&p);
    }
}

#[test]
fn names_are_valid_idents() {
    bolero::check!()
        .with_type::<String>()
        .for_each(|s| check_names(s));
}

/// Every keyword, and a few strings that only differ from one in case or
/// punctuation.
#[test]
fn keywords_are_valid_idents() {
    for kw in crate::KEYWORDS.iter().chain(&[
        "self", "Self", "super", "crate", "_", "union", "static", "r#type",
    ]) {
        check_names(kw);
        check_ident(&snake(kw));
    }
}

/// Every string of up to 3 characters over letters, digits, separators
/// and non-ASCII characters that are alphanumeric to `char` but differ in
/// what an identifier allows.
#[test]
fn exhaustive_short_names() {
    const ALPHABET: [char; 14] = [
        'a',
        'Z',
        '0',
        '9',
        '_',
        '-',
        ' ',
        '@',
        '\u{e9}',
        '\u{b2}',
        '\u{2160}',
        '\u{663}',
        '\u{aa}',
        '\u{1d7d8}',
    ];
    fn go(s: &mut String, left: usize) {
        check_names(s);
        if left == 0 {
            return;
        }
        for c in ALPHABET {
            s.push(c);
            go(s, left - 1);
            s.pop();
        }
    }
    go(&mut String::new(), 3);
}
