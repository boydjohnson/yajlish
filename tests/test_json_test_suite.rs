//! Runs the vendored JSONTestSuite (`tests/json_test_suite/test_parsing`).
//!
//! - `y_*` must parse.
//! - `n_*` must fail, except those in `N_ALLOWLIST`, which is empty. A
//!   listed file that starts failing makes this test fail too, so the entry
//!   gets removed.
//! - `i_*` are implementation-defined. `I_ACCEPTED` records which ones we
//!   accept, so a change in behaviour shows up here.

use std::path::Path;
use yajlish::{Context, Handler, Parser, Status};

struct Sink;

impl Handler for Sink {
    fn handle_null(&mut self, _: &Context) -> Status {
        Status::Continue
    }
    fn handle_double(&mut self, _: &Context, _: f64) -> Status {
        Status::Continue
    }
    fn handle_int(&mut self, _: &Context, _: i64) -> Status {
        Status::Continue
    }
    fn handle_bool(&mut self, _: &Context, _: bool) -> Status {
        Status::Continue
    }
    fn handle_string(&mut self, _: &Context, _: &str) -> Status {
        Status::Continue
    }
    fn handle_start_map(&mut self, _: &Context) -> Status {
        Status::Continue
    }
    fn handle_end_map(&mut self, _: &Context) -> Status {
        Status::Continue
    }
    fn handle_map_key(&mut self, _: &Context, _: &str) -> Status {
        Status::Continue
    }
    fn handle_start_array(&mut self, _: &Context) -> Status {
        Status::Continue
    }
    fn handle_end_array(&mut self, _: &Context) -> Status {
        Status::Continue
    }
}

fn accepts(input: &[u8]) -> bool {
    let mut handler = Sink;
    let mut parser = Parser::new(&mut handler);
    let mut reader = input;
    parser.parse(&mut reader).is_ok() && parser.finish_parse().is_ok()
}

/// `n_*` files we accept. Empty since the parser hardening (Plan 4, K3b),
/// and it should stay that way; the mechanism is kept so that a deliberate
/// exception would be visible here.
const N_ALLOWLIST: &[&str] = &[];

/// `i_*` files we accept. Out-of-range numbers become `f64` (possibly
/// infinite or zero); lone surrogate escapes are valid grammar and are left
/// to `decode_str`; invalid UTF-8 and BOMs are rejected.
const I_ACCEPTED: &[&str] = &[
    "i_number_double_huge_neg_exp.json",
    "i_number_huge_exp.json",
    "i_number_neg_int_huge_exp.json",
    "i_number_pos_double_huge_exp.json",
    "i_number_real_neg_overflow.json",
    "i_number_real_pos_overflow.json",
    "i_number_real_underflow.json",
    "i_number_too_big_neg_int.json",
    "i_number_too_big_pos_int.json",
    "i_number_very_big_negative_int.json",
    "i_object_key_lone_2nd_surrogate.json",
    "i_string_1st_surrogate_but_2nd_missing.json",
    "i_string_1st_valid_surrogate_2nd_invalid.json",
    "i_string_incomplete_surrogate_and_escape_valid.json",
    "i_string_incomplete_surrogate_pair.json",
    "i_string_incomplete_surrogates_escape_valid.json",
    "i_string_invalid_lonely_surrogate.json",
    "i_string_invalid_surrogate.json",
    "i_string_inverted_surrogates_U+1D11E.json",
    "i_string_lone_second_surrogate.json",
    "i_structure_500_nested_arrays.json",
];

#[test]
fn test_json_test_suite() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/json_test_suite/test_parsing");
    let mut entries: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    entries.sort();

    let mut failures = vec![];
    let mut n_accepted = vec![];
    let mut i_accepted = vec![];
    for path in entries {
        let name = path.file_name().unwrap().to_str().unwrap().to_owned();
        let input = std::fs::read(&path).unwrap();
        let ok = accepts(&input);
        match &name[..2] {
            "y_" if !ok => failures.push(format!("rejected {}", name)),
            "n_" if ok => n_accepted.push(name),
            "i_" if ok => i_accepted.push(name),
            _ => {}
        }
    }

    for name in &n_accepted {
        if !N_ALLOWLIST.contains(&name.as_str()) {
            failures.push(format!("accepted {}", name));
        }
    }
    for name in N_ALLOWLIST {
        if !n_accepted.iter().any(|n| n == name) {
            failures.push(format!(
                "{} is rejected now, remove it from N_ALLOWLIST",
                name
            ));
        }
    }
    if i_accepted != I_ACCEPTED {
        failures.push(format!("i_* accepted changed: {:#?}", i_accepted));
    }

    assert!(failures.is_empty(), "{:#?}", failures);
}
