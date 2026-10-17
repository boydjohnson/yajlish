//! With the `serde` feature, a built value serializes back to the JSON it
//! was built from.
#![cfg(feature = "serde")]

use yajlish::Parser;
use yajlish_schema::Status;

yajlish_schema::handler! {
    struct TreeHandler;
    schemas {
        Tree => "tests/schemas/tree.schema.json",
    }
}

yajlish_schema::handler! {
    struct FeatureHandler;
    at = ".features";
    schemas {
        Point => "tests/schemas/point_feature.schema.json",
        Polygon => "tests/schemas/polygon_feature.schema.json",
        MultiPolygon => "tests/schemas/multipolygon_feature.schema.json",
    }
}

fn round_trip_tree(input: &str) {
    let mut out = vec![];
    let mut handler = TreeHandler::new(|rec| {
        out.push(rec.unwrap());
        Status::Continue
    });
    let mut parser = Parser::new(&mut handler);
    parser.parse(&mut input.as_bytes()).unwrap();
    parser.finish_parse().unwrap();
    let TreeRecord::Tree(tree) = out.pop().unwrap();
    let expected: serde_json::Value = serde_json::from_str(input).unwrap();
    assert_eq!(serde_json::to_value(&tree).unwrap(), expected);
    // And back.
    let again: Tree = serde_json::from_value(expected).unwrap();
    assert_eq!(again, tree);
}

#[test]
fn test_tree_round_trip() {
    round_trip_tree(
        r#"{"label": "root", "weight": 1.5, "color": "light-blue",
            "children": [{"label": "a", "weight": null, "shape": {"radius": 2.5}},
                         {"label": "b", "shape": {"w": 1, "h": 2}, "children": [{"label": "c", "shape": "blob"}]}],
            "parent": {"label": "up"}, "pair": ["x", 4], "tags": {"one": 1},
            "base": {"name": "n", "id": 3, "extra": [true, null, {"k": "v"}]}}"#,
    );
    round_trip_tree(r#"{"label": "only"}"#);
}

#[test]
fn test_features_round_trip() {
    let input = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../yajlish-ndjson/tests/ndjson_fixtures/custom.geo.json"
    ))
    .unwrap();
    let mut out = vec![];
    let mut handler = FeatureHandler::new(|rec| {
        out.push(rec.unwrap());
        Status::Continue
    });
    let mut parser = Parser::new(&mut handler);
    parser.parse(&mut input.as_bytes()).unwrap();
    parser.finish_parse().unwrap();
    handler.finish().unwrap();

    let doc: serde_json::Value = serde_json::from_str(&input).unwrap();
    let features = doc["features"].as_array().unwrap();
    assert_eq!(out.len(), features.len());
    for (rec, expected) in out.iter().zip(features) {
        let got = match rec {
            FeatureRecord::Point(f) => serde_json::to_value(f),
            FeatureRecord::Polygon(f) => serde_json::to_value(f),
            FeatureRecord::MultiPolygon(f) => serde_json::to_value(f),
        }
        .unwrap();
        assert_eq!(&got, expected);
    }
}
