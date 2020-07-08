static DIFFICULT_JSON_WITH_COMMENTS: &str = "ac_difficult_json_c_test_case_with_comments.json";
static SIMPLE_JSON_WITH_COMMENTS: &str = "ac_simple_with_comments.json";
static FALSE_THEN_GARBAGE: &str = "ag_false_then_garbage.json";
static NULL_THEN_GARBAGE: &str = "ag_null_then_garbage.json";
static TRUE_THEN_GARBAGE: &str = "ag_true_then_garbage.json";
static EOF: &str = "am_eof.json";
static AM_INTEGERS: &str = "am_integers.json";
static AM_MULTIPLE: &str = "am_multiple.json";
static STUFF: &str = "am_stuff.json";
static ARRAY_OPEN: &str = "ap_array_open.json";
static EOF_STR: &str = "ap_eof_str.json";
static MAP_OPEN: &str = "ap_map_open.json";
static PARTIAL_OK: &str = "ap_partial_ok.json";
static ARRAY: &str = "array.json";
static ARRAY_CLOSE: &str = "array_close.json";
static BIGNUMS: &str = "bignums.json";
static BOGUS_CHAR: &str = "bogus_char.json";
static UNICODE_CODEPOINTS: &str = "codepoints_from_unicode_org.json";
static DEEP_ARRAYS: &str = "deep_arrays.json";
static DIFFICULT_TEST_CASE: &str = "difficult_json_c_test_case.json";
static DOUBLES: &str = "doubles.json";
static DOUBLES_IN_ARRAY: &str = "doubles_in_array.json";
static EMPTY_ARRAY: &str = "empty_array.json";
static EMPTY_STRING: &str = "empty_string.json";
static ESCAPED_BULGARIAN: &str = "escaped_bulgarian.json";
static ESCAPED_FOO_BAR: &str = "escaped_foobar.json";
static FALSE: &str = "false.json";
static FG_FALSE_THEN_GARBAGE: &str = "fg_false_then_garbage.json";
static FG_ISSUE_7: &str = "fg_issue_7.json";
static FG_NULL_THEN_GARBAGE: &str = "fg_null_then_garbage.json";
static FG_TRUE_THEN_GARBAGE: &str = "fg_true_then_garbage.json";
static FOUR_BYTE_UTF8: &str = "four_byte_utf8.json";
static HIGH_OVERFLOW: &str = "high_overflow.json";
static INTEGERS: &str = "integers.json";
static INVALID_UTF8: &str = "invalid_utf8.json";
static ISOLATED_SURROGATE_MARKER: &str = "isolated_surrogate_marker.json";
static LEADING_ZERO_IN_NUMBER: &str = "leading_zero_in_number.json";
static LONELY_MINUS_SIGN: &str = "lonely_minus_sign.json";
static LONELY_NUMBER: &str = "lonely_number.json";
static LOW_OVERFLOW: &str = "low_overflow.json";
static MAP_CLOSE: &str = "map_close.json";
static MISSING_INTEGER_AFTER_DECIMAL_POINT: &str = "missing_integer_after_decimal_point.json";
static MISSING_INTEGER_AFTER_EXPONENT: &str = "missing_integer_after_exponent.json";
static MULTIPLE: &str = "multiple.json";
static NON_UTF8_CHAR_IN_STRING: &str = "non_utf8_char_in_string.json";
static PARTIAL_BAD: &str = "np_partial_bad.json";
static NULL: &str = "null.json";
static NULLS_AND_BOOLS: &str = "nulls_and_bools.json";
static SIMPLE: &str = "simple.json";
static SIMPLE_WITH_COMMENTS: &str = "simple_with_comments.json";
static STRING_INVALID_ESCAPE: &str = "string_invalid_escape.json";
static STRING_INVALID_HEX: &str = "string_invalid_hex_char.json";
static STRING_WITH_ESCAPES: &str = "string_with_escapes.json";
static STRING_WITH_INVALID_NEWLINE: &str = "string_with_invalid_newline.json";
static THREE_BYTE_UTF8: &str = "three_byte_utf8.json";
static TRUE: &str = "true.json";
static UNESCAPED_BULGARIAN: &str = "unescaped_bulgarian.json";
static ZEROBYTE: &str = "zerobyte.json";

mod common;

use common::assert_output_equals;

fn read_input_to_bytes(s: &str) -> String {
    std::fs::read_to_string(format!("tests/fixtures/{}", s)).unwrap()
}

fn read_output_to_bytes(s: &str) -> String {
    std::fs::read_to_string(format!("tests/fixtures/{}.gold", s)).unwrap()
}

fn test_fixture(s: &str) {
    let input = read_input_to_bytes(s);
    let output = read_output_to_bytes(s);

    assert_output_equals(input.as_bytes(), output.as_bytes());
}

#[test]
#[ignore]
fn test_difficult_json_with_comments() {
    test_fixture(DIFFICULT_JSON_WITH_COMMENTS);
}

#[test]
#[ignore]
fn test_simple_json_with_comments() {
    test_fixture(SIMPLE_JSON_WITH_COMMENTS);
}

#[test]
#[ignore]
fn test_false_then_garbage() {
    test_fixture(FALSE_THEN_GARBAGE);
}

#[test]
#[ignore]
fn test_null_then_garbage() {
    test_fixture(NULL_THEN_GARBAGE);
}

#[test]
#[ignore]
fn test_true_then_garbage() {
    test_fixture(TRUE_THEN_GARBAGE);
}

#[test]
#[ignore]
fn test_eof() {
    test_fixture(EOF);
}

#[test]
#[ignore]
fn test_am_integers() {
    test_fixture(AM_INTEGERS);
}

#[test]
#[ignore]
fn test_am_multiple() {
    test_fixture(AM_MULTIPLE);
}

#[test]
#[ignore]
fn test_stuff() {
    test_fixture(STUFF);
}

#[test]
#[ignore]
fn test_array_open() {
    test_fixture(ARRAY_OPEN);
}

#[test]
#[ignore]
fn test_eof_str() {
    test_fixture(EOF_STR);
}

#[test]
#[ignore]
fn test_map_open() {
    test_fixture(MAP_OPEN);
}

#[test]
#[ignore]
fn test_partial_ok() {
    test_fixture(PARTIAL_OK);
}

#[test]
#[ignore]
fn test_array() {
    test_fixture(ARRAY);
}

#[test]
#[ignore]
fn test_array_close() {
    test_fixture(ARRAY_CLOSE);
}

#[test]
#[ignore]
fn test_bignums() {
    test_fixture(BIGNUMS);
}

#[test]
#[ignore]
fn test_bogus_char() {
    test_fixture(BOGUS_CHAR);
}

#[test]
#[ignore]
fn test_codepoints_from_unicode() {
    test_fixture(UNICODE_CODEPOINTS);
}

#[test]
fn test_deep_arrays() {
    test_fixture(DEEP_ARRAYS);
}

#[test]
#[ignore]
fn test_difficult_test_case() {
    test_fixture(DIFFICULT_TEST_CASE);
}

#[test]
#[ignore]
fn test_doubles() {
    test_fixture(DOUBLES);
}

#[test]
#[ignore]
fn test_doubles_in_array() {
    test_fixture(DOUBLES_IN_ARRAY);
}

#[test]
fn test_empty_array() {
    test_fixture(EMPTY_ARRAY);
}

#[test]
fn test_empty_string() {
    test_fixture(EMPTY_STRING);
}

#[test]
#[ignore]
fn test_escaped_bulgarian() {
    test_fixture(ESCAPED_BULGARIAN);
}

#[test]
#[ignore]
fn test_escaped_foobar() {
    test_fixture(ESCAPED_FOO_BAR);
}

#[test]
fn test_false() {
    test_fixture(FALSE);
}

#[test]
#[ignore]
fn test_fg_false_then_garbage() {
    test_fixture(FG_FALSE_THEN_GARBAGE);
}

#[test]
#[ignore]
fn test_issue_7() {
    test_fixture(FG_ISSUE_7);
}

#[test]
#[ignore]
fn test_fg_null_then_garbage() {
    test_fixture(FG_NULL_THEN_GARBAGE);
}

#[test]
#[ignore]
fn test_fg_true_then_garbage() {
    test_fixture(FG_TRUE_THEN_GARBAGE);
}

#[test]
#[ignore]
fn test_four_byte_utf8() {
    test_fixture(FOUR_BYTE_UTF8);
}

#[test]
#[ignore]
fn test_high_overflow() {
    test_fixture(HIGH_OVERFLOW);
}

#[test]
fn test_integers() {
    test_fixture(INTEGERS);
}

#[test]
#[ignore]
fn test_invalid_utf8() {
    test_fixture(INVALID_UTF8);
}

#[test]
#[ignore]
fn test_isolated_surrogate_marker() {
    test_fixture(ISOLATED_SURROGATE_MARKER);
}

#[test]
#[ignore]
fn test_leading_zeros_in_number() {
    test_fixture(LEADING_ZERO_IN_NUMBER);
}

#[test]
#[ignore]
fn test_lonely_minus_sign() {
    test_fixture(LONELY_MINUS_SIGN);
}

#[test]
#[ignore]
fn test_lonely_number() {
    test_fixture(LONELY_NUMBER);
}

#[test]
#[ignore]
fn test_low_overflow() {
    test_fixture(LOW_OVERFLOW);
}

#[test]
#[ignore]
fn test_map_close() {
    test_fixture(MAP_CLOSE);
}

#[test]
#[ignore]
fn test_missing_integer_after_decimal_point() {
    test_fixture(MISSING_INTEGER_AFTER_DECIMAL_POINT);
}

#[test]
#[ignore]
fn test_missing_integer_after_exponent() {
    test_fixture(MISSING_INTEGER_AFTER_EXPONENT);
}

#[test]
#[ignore]
fn test_multiple() {
    test_fixture(MULTIPLE);
}

#[test]
#[ignore]
fn test_non_utf8_char_in_string() {
    test_fixture(NON_UTF8_CHAR_IN_STRING);
}

#[test]
#[ignore]
fn test_partial_bad() {
    test_fixture(PARTIAL_BAD);
}

#[test]
fn test_null() {
    test_fixture(NULL);
}

#[test]
#[ignore]
fn test_nulls_and_bools() {
    test_fixture(NULLS_AND_BOOLS);
}

#[test]
#[ignore]
fn test_simple() {
    test_fixture(SIMPLE);
}

#[test]
#[ignore]
fn test_simple_with_comments() {
    test_fixture(SIMPLE_WITH_COMMENTS);
}

#[test]
#[ignore]
fn test_string_invalid_escape() {
    test_fixture(STRING_INVALID_ESCAPE);
}

#[test]
#[ignore]
fn test_invalid_hex_char() {
    test_fixture(STRING_INVALID_HEX);
}

#[test]
#[ignore]
fn test_string_with_escapes() {
    test_fixture(STRING_WITH_ESCAPES);
}

#[test]
#[ignore]
fn test_string_with_invalid_newline() {
    test_fixture(STRING_WITH_INVALID_NEWLINE);
}

#[test]
#[ignore]
fn test_three_byte_utf8() {
    test_fixture(THREE_BYTE_UTF8);
}

#[test]
fn test_true() {
    test_fixture(TRUE);
}

#[test]
#[ignore]
fn test_unescaped_bulgarian() {
    test_fixture(UNESCAPED_BULGARIAN);
}

#[test]
#[ignore]
fn test_zerobyte() {
    test_fixture(ZEROBYTE);
}
