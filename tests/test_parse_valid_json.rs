mod common;

use common::test_fixture;

static PATH: &str = "valid_json_fixtures";

#[test]
fn test_array_of_arrays() {
    test_fixture(PATH, "array_of_arrays.json");
}

#[test]
fn test_nested_maps() {
    test_fixture(PATH, "nested_maps.json");
}

#[test]
fn test_nested_arrays() {
    test_fixture(PATH, "nested_arrays.json");
}

#[test]
fn test_quote_at_end() {
    test_fixture(PATH, "quote_at_end_of_value.json");
}
