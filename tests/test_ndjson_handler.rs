#![cfg(feature = "ndjson")]

mod common;

use common::test_ndjson_fixture;

const PATH: &str = "ndjson_fixtures";

#[test]
fn test_custom_geojson() {
    test_ndjson_fixture(
        PATH,
        "custom.geo.json",
        vec![yajlish::ndjson_handler::Selector::Identifier(
            "\"features\"".to_owned(),
        )],
    );
}

#[test]
fn test_custom_json() {
    test_ndjson_fixture(
        PATH,
        "prize.json",
        vec![yajlish::ndjson_handler::Selector::Identifier(
            "\"prizes\"".to_owned(),
        )],
    );
}
