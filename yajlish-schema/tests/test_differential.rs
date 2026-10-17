//! Differential test against the `jsonschema` crate: for the schemas in
//! `tests/schemas/` (those without references to other files), random
//! instances built from the schemas' own vocabulary must get the same
//! verdict from both validators.

mod support;

use proptest::prelude::*;
use serde_json::{json, Map, Number, Value};

const SCHEMAS: &[&str] = &[
    "tree.schema.json",
    "point_feature.schema.json",
    "polygon_feature.schema.json",
    "feature_collection.schema.json",
    "loose.schema.json",
    "strict.schema.json",
];

const KEYS: &[&str] = &[
    "label",
    "weight",
    "color",
    "children",
    "parent",
    "shape",
    "radius",
    "w",
    "h",
    "pair",
    "tags",
    "base",
    "name",
    "id",
    "type",
    "geometry",
    "coordinates",
    "properties",
    "features",
    "n",
    "extra",
];

const STRINGS: &[&str] = &[
    "",
    "x",
    "red",
    "light-blue",
    "blue",
    "Feature",
    "FeatureCollection",
    "Point",
    "Polygon",
];

fn arb_value() -> impl Strategy<Value = Value> {
    let leaf = prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::Bool),
        (-3i64..10).prop_map(|i| json!(i)),
        prop_oneof![Just(-0.5), Just(0.0), Just(1.0), Just(2.5), Just(1e300)]
            .prop_map(|f: f64| Value::Number(Number::from_f64(f).unwrap())),
        prop::sample::select(STRINGS).prop_map(|s| json!(s)),
    ];
    leaf.prop_recursive(5, 48, 6, |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 0..6).prop_map(Value::Array),
            prop::collection::vec((prop::sample::select(KEYS), inner), 0..6).prop_map(|kv| {
                Value::Object(
                    kv.into_iter()
                        .map(|(k, v)| (k.to_owned(), v))
                        .collect::<Map<_, _>>(),
                )
            }),
        ]
    })
}

fn load(name: &str) -> Value {
    let path = format!("{}/tests/schemas/{}", env!("CARGO_MANIFEST_DIR"), name);
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

proptest! {
    #[test]
    fn test_same_verdict_as_jsonschema(
        schema in prop::sample::select(SCHEMAS),
        instance in arb_value(),
        chunk in prop_oneof![Just(1usize), Just(7), Just(usize::MAX)],
    ) {
        let schema = load(schema);
        let theirs = jsonschema::validator_for(&schema).unwrap().is_valid(&instance);
        let cfg = support::config(&[schema], false).unwrap();
        let text = serde_json::to_string(&instance).unwrap();
        let ours = support::validate_text(cfg, text.as_bytes(), chunk);
        prop_assert_eq!(ours.is_ok(), theirs, "{} -> {:?}", text, ours);
    }
}

/// The random instances are mostly invalid; make sure valid ones are
/// covered too.
#[test]
fn test_known_valid_instances() {
    let cases = [
        (
            "tree.schema.json",
            json!({"label": "a", "children": [{"label": "b", "shape": {"w": 1, "h": 2}}]}),
        ),
        (
            "point_feature.schema.json",
            json!({"type": "Feature", "properties": null, "geometry": {"type": "Point", "coordinates": [1, 2]}}),
        ),
        ("loose.schema.json", json!({"n": 3})),
    ];
    for (name, instance) in cases {
        let schema = load(name);
        assert!(jsonschema::validator_for(&schema)
            .unwrap()
            .is_valid(&instance));
        let cfg = support::config(&[schema], false).unwrap();
        let text = serde_json::to_string(&instance).unwrap();
        assert!(support::validate_text(cfg, text.as_bytes(), 3).is_ok());
    }
}

fn arb_feature() -> impl Strategy<Value = Value> {
    let geometry = (
        prop::sample::select(&["Point", "Polygon", "Line"][..]),
        arb_value(),
        any::<bool>(),
    )
        .prop_map(|(t, coordinates, early)| {
            let mut m = Map::new();
            if early {
                m.insert("type".into(), json!(t));
            }
            m.insert("coordinates".into(), coordinates);
            if !early {
                m.insert("type".into(), json!(t));
            }
            Value::Object(m)
        });
    (geometry, arb_value(), any::<bool>()).prop_map(|(geometry, properties, with_type)| {
        let mut m = Map::new();
        if with_type {
            m.insert("type".into(), json!("Feature"));
        }
        m.insert("properties".into(), properties);
        m.insert("geometry".into(), geometry);
        Value::Object(m)
    })
}

proptest! {
    /// Waiting for the discriminator and running only the schemas it names
    /// gives exactly the outcome of running every schema.
    #[test]
    fn test_discriminator_does_not_change_outcomes(
        instance in prop_oneof![arb_feature(), arb_value()],
        chunk in prop_oneof![Just(1usize), Just(usize::MAX)],
    ) {
        let schemas = [load("point_feature.schema.json"), load("polygon_feature.schema.json")];
        let with = support::config_with(&schemas, false, true).unwrap();
        let without = support::config_with(&schemas, false, false).unwrap();
        prop_assert!(with.discriminator.is_some());
        let text = serde_json::to_string(&instance).unwrap();
        prop_assert_eq!(
            support::validate_text(with, text.as_bytes(), chunk),
            support::validate_text(without, text.as_bytes(), chunk)
        );
    }
}
