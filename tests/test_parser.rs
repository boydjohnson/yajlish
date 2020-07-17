mod common;

use common::assert_output_equals;

#[test]
fn test_bool() {
    assert_output_equals("true".as_bytes(), "bool: true\n".as_bytes());
}

#[test]
fn test_empty_map() {
    assert_output_equals("{ }".as_bytes(), "map open '{'\nmap close '}'\n".as_bytes());
}

#[test]
fn test_simple_map() {
    assert_output_equals(
        "{ \"foo\": 27 }".as_bytes(),
        "map open '{'\nkey: foo\ninteger: 27\nmap close '}'\n".as_bytes(),
    );
}

#[test]
fn test_empty_array() {
    assert_output_equals(
        "[ ]".as_bytes(),
        "array open '['\narray close ']'\n".as_bytes(),
    );
}

#[test]
fn test_simple_array() {
    assert_output_equals(
        "[1, \"foo\", 7.5]".as_bytes(),
        "array open '['\ninteger: 1\nstring: 'foo'\ndouble: 7.5\narray close ']'\n".as_bytes(),
    );
}

#[test]
fn test_complex_map() {
    assert_output_equals("{ \"foo\": [{ \"bar\": [1, 2, null]}, true] }".as_bytes(),
        "map open '{'\nkey: foo\narray open '['\nmap open '{'\nkey: bar\narray open '['\ninteger: 1\ninteger: 2\nnull\narray close ']'\nmap close '}'\nbool: true\narray close ']'\nmap close '}'\n".as_bytes());
}

#[test]
fn test_complex_array() {
    assert_output_equals("[1, \"foo\", [null, 4.5, 8], false, { \"data\": null }]".as_bytes(), "array open '['\ninteger: 1\nstring: 'foo'\narray open '['\nnull\ndouble: 4.5\ninteger: 8\narray close ']'\nbool: false\nmap open '{'\nkey: data\nnull\nmap close '}'\narray close ']'\n".as_bytes());
}

#[test]
fn test_bool_with_newline() {
    assert_output_equals("true\n ".as_bytes(), "bool: true\n".as_bytes());
}

#[test]
fn test_array_with_string() {
    assert_output_equals(
        "[\"foo\"]".as_bytes(),
        "array open '['\nstring: 'foo'\narray close ']'\n".as_bytes(),
    );
}
