//! End to end: GeoJSON features validated and typed one at a time.

use yajlish::Parser;
use yajlish_schema::{SchemaError, Status};

yajlish_schema::handler! {
    /// Handler for GeoJSON features.
    pub struct FeatureHandler;
    at = ".features";
    envelope = "tests/schemas/feature_collection.schema.json";
    match = unique;
    schemas {
        Point => "tests/schemas/point_feature.schema.json",
        Polygon => "tests/schemas/polygon_feature.schema.json",
        MultiPolygon => "tests/schemas/multipolygon_feature.schema.json",
    }
}

fn run(
    input: &[u8],
) -> (
    Vec<Result<FeatureRecord, SchemaError>>,
    Result<(), SchemaError>,
) {
    let mut records = vec![];
    let mut handler = FeatureHandler::new(|rec| {
        records.push(rec);
        Status::Continue
    });
    let mut parser = Parser::new(&mut handler);
    parser.parse(&mut &input[..]).unwrap();
    // An envelope error aborts the parse early, so the parser may not
    // have reached the end; the handler's `finish` says why.
    let _ = parser.finish_parse();
    let finished = handler.finish();
    (records, finished)
}

#[test]
fn test_fixture_validates_per_feature() {
    let input = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../yajlish-ndjson/tests/ndjson_fixtures/custom.geo.json"
    ))
    .unwrap();
    let (records, finished) = run(&input);
    assert_eq!(finished, Ok(()));
    assert_eq!(records.len(), 1);
    match &records[0] {
        Ok(FeatureRecord::MultiPolygon(f)) => {
            assert_eq!(f.r#type, "Feature");
            assert_eq!(f.geometry.r#type, "MultiPolygon");
            assert_eq!(f.geometry.coordinates[0][0][0], vec![178.3736, -17.33992]);
            let props = f.properties.as_ref().unwrap();
            assert_eq!(
                props.get("admin"),
                Some(&yajlish_schema::Value::String("Fiji".into()))
            );
        }
        other => panic!("{:?}", other),
    }
}

#[test]
fn test_each_feature_is_typed_by_its_geometry() {
    let input = br#"{"type": "FeatureCollection", "features": [
        {"type": "Feature", "properties": null, "geometry": {"type": "Point", "coordinates": [1, 2.5]}},
        {"type": "Feature", "id": 7, "properties": {}, "geometry": {"type": "Polygon",
            "coordinates": [[[0, 0], [1, 0], [1, 1], [0, 0]]]}},
        {"type": "Feature", "properties": null, "geometry": {"type": "Point", "coordinates": [1]}},
        {"type": "Feature", "properties": null, "geometry": {"type": "Line", "coordinates": []}}
    ]}"#;
    let (records, finished) = run(input);
    assert_eq!(finished, Ok(()));
    assert_eq!(records.len(), 4);
    match &records[0] {
        Ok(FeatureRecord::Point(p)) => {
            assert_eq!(p.geometry.coordinates, vec![1.0, 2.5]);
            assert_eq!(p.properties, None);
        }
        other => panic!("{:?}", other),
    }
    assert!(matches!(&records[1], Ok(FeatureRecord::Polygon(p)) if p.id.is_some()));

    // Point geometry with one coordinate: only the Point schema got as far
    // as the coordinates, so its error is the specific one.
    let err = records[2].as_ref().unwrap_err();
    assert_eq!(err.keyword, "match");
    let point = &err.causes[0];
    assert_eq!(point.keyword, "minItems");
    assert_eq!(point.instance_path, "/geometry/coordinates");

    let err = records[3].as_ref().unwrap_err();
    assert_eq!(err.keyword, "match");
    assert!(err.causes.iter().all(|c| c.keyword == "const"));
}

#[test]
fn test_envelope_errors_go_to_finish() {
    let (records, finished) = run(br#"{"type": "Feature", "features": []}"#);
    assert!(records.is_empty());
    let err = finished.unwrap_err();
    assert_eq!(err.keyword, "const");
    assert_eq!(err.instance_path, "/type");
}

#[test]
fn test_missing_array_is_an_error() {
    let (_, finished) = run(br#"{"type": "FeatureCollection", "other": []}"#);
    assert_eq!(finished.unwrap_err().keyword, "required");
}

#[test]
fn test_candidates_are_pruned_at_the_discriminator() {
    let mut handler = FeatureHandler::new(|_| Status::Continue);
    let mut parser = Parser::new(&mut handler);
    parser
        .parse(&mut &br#"{"type": "FeatureCollection", "features": [{"type": "Feature", "geometry": {"type": "Polygon""#[..])
        .unwrap();
    drop(parser);
    assert_eq!(handler.live_candidates(), 1);
}
