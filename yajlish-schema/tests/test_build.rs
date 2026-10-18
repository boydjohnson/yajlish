//! Whole-document records: typed values, recursion, unions, and the
//! validate-only mode.

use std::collections::BTreeMap;
use yajlish::Parser;
use yajlish_schema::{SchemaError, Status, Value};

yajlish_schema::handler! {
    pub struct TreeHandler;
    schemas {
        Tree => "tests/schemas/tree.schema.json",
    }
}

mod validate_only {
    yajlish_schema::handler! {
        pub struct TreeHandler;
        build = false;
        schemas {
            Tree => "tests/schemas/tree.schema.json",
        }
    }
}

fn parse<H: yajlish::Handler>(handler: &mut H, input: &[u8]) {
    let mut parser = Parser::new(handler);
    parser.parse(&mut &input[..]).unwrap();
    parser.finish_parse().unwrap();
}

fn build(input: &[u8]) -> Result<Tree, SchemaError> {
    let mut out = vec![];
    let mut handler = TreeHandler::new(|rec| {
        out.push(rec);
        Status::Continue
    });
    parse(&mut handler, input);
    handler.finish().unwrap();
    assert_eq!(out.len(), 1);
    out.pop().unwrap().map(|TreeRecord::Tree(t)| t)
}

fn validate(input: &[u8]) -> Result<validate_only::TreeSchema, SchemaError> {
    let mut out = vec![];
    let mut handler = validate_only::TreeHandler::new(|rec| {
        out.push(rec);
        Status::Continue
    });
    parse(&mut handler, input);
    handler.finish().unwrap();
    out.pop().unwrap()
}

const FULL: &[u8] = br#"{
    "label": "root",
    "weight": 1.5,
    "color": "light-blue",
    "children": [
        {"label": "a", "weight": null, "shape": {"radius": 2}},
        {"label": "b", "shape": {"w": 1, "h": 2.0}, "children": [{"label": "c", "shape": "blob"}]}
    ],
    "parent": {"label": "up"},
    "pair": ["x", 4],
    "tags": {"one": 1, "two": 2},
    "base": {"name": "n", "id": 3, "extra": true}
}"#;

#[test]
fn test_build_full_tree() {
    let t = build(FULL).unwrap();
    assert_eq!(t.label, "root");
    assert_eq!(t.weight, Some(Some(1.5)));
    assert_eq!(t.color, Some(TreeColor::LightBlue));
    assert_eq!(t.color.unwrap().as_str(), "light-blue");
    assert_eq!(t.parent.as_ref().unwrap().label, "up");
    let children = t.children.as_ref().unwrap();
    assert_eq!(children[0].weight, Some(None));
    assert_eq!(
        children[0].shape,
        Some(TreeShape::Circle(Circle { radius: 2.0 }))
    );
    assert_eq!(
        children[1].shape,
        Some(TreeShape::Rect(Rect { w: 1, h: 2 }))
    );
    let grandchild = &children[1].children.as_ref().unwrap()[0];
    assert_eq!(grandchild.shape, Some(TreeShape::Variant2("blob".into())));
    assert_eq!(t.pair, Some(TreePair("x".into(), 4)));
    let tags: BTreeMap<String, i64> = [("one".to_owned(), 1), ("two".to_owned(), 2)].into();
    assert_eq!(t.tags, Some(tags));
    let base = t.base.unwrap();
    assert_eq!((base.name.as_str(), base.id), ("n", 3));
    assert_eq!(
        base.additional_properties.get("extra"),
        Some(&Value::Bool(true))
    );

    assert_eq!(validate(FULL), Ok(validate_only::TreeSchema::Tree));
}

#[test]
fn test_invalid_trees() {
    let cases: &[(&[u8], &str, &str)] = &[
        (br#"{}"#, "required", ""),
        (br#"{"label": ""}"#, "minLength", "/label"),
        (br#"{"label": "x", "weight": -1}"#, "minimum", "/weight"),
        (br#"{"label": "x", "color": "blue"}"#, "enum", "/color"),
        (
            br#"{"label": "x", "nope": 1}"#,
            "additionalProperties",
            "/nope",
        ),
        (
            br#"{"label": "x", "children": [{"label": "y"}, {}]}"#,
            "required",
            "/children/1",
        ),
        (
            br#"{"label": "x", "children": [{}, {}, {}, {}, {}]}"#,
            "required",
            "/children/0",
        ),
        (
            br#"{"label": "x", "parent": {"label": "y", "parent": {"label": 3}}}"#,
            "type",
            "/parent/parent/label",
        ),
        (
            br#"{"label": "x", "shape": {"radius": 0}}"#,
            "oneOf",
            "/shape",
        ),
        (
            br#"{"label": "x", "pair": ["a", 3]}"#,
            "multipleOf",
            "/pair/1",
        ),
        (br#"{"label": "x", "pair": ["a", 2, 3]}"#, "items", "/pair"),
        (br#"{"label": "x", "tags": {"a": "b"}}"#, "type", "/tags/a"),
        (
            br#"{"label": "x", "base": {"name": "n"}}"#,
            "required",
            "/base",
        ),
        (br#"[1]"#, "type", ""),
    ];
    for (input, keyword, path) in cases {
        let err = build(input).unwrap_err();
        assert_eq!(
            (err.keyword, err.instance_path.as_str()),
            (*keyword, *path),
            "{}: {}",
            String::from_utf8_lossy(input),
            err
        );
        assert_eq!(validate(input).unwrap_err(), err);
    }
}

#[test]
fn test_error_offsets_point_at_the_value() {
    let input = br#"{"label": "x", "weight": -1}"#;
    let err = build(input).unwrap_err();
    assert_eq!(err.offset, Some(25));
    assert_eq!(
        err.schema_path,
        "tree.schema.json#/properties/weight/minimum"
    );
}

#[test]
fn test_ints_out_of_range_are_build_errors() {
    let err = build(br#"{"label": "x", "pair": ["a", 1e300]}"#).unwrap_err();
    assert_eq!(err.keyword, "build");
    assert_eq!(err.instance_path, "/pair/1");
    // Valid per the schema, though.
    assert!(validate(br#"{"label": "x", "pair": ["a", 1e300]}"#).is_ok());
}

#[test]
fn test_deep_recursion_is_bounded() {
    let depth = 2000;
    let mut input = String::new();
    for _ in 0..depth {
        input.push_str(r#"{"label": "x", "parent": "#);
    }
    input.push_str(r#"{"label": "x"}"#);
    for _ in 0..depth {
        input.push('}');
    }
    let err = validate(input.as_bytes()).unwrap_err();
    assert_eq!(err.keyword, "");
    assert!(err.detail.contains("nesting"), "{}", err);
}

#[test]
fn test_incomplete_document() {
    let mut handler = TreeHandler::new(|_| Status::Continue);
    let mut parser = Parser::new(&mut handler);
    parser.parse(&mut &br#"{"label": "x""#[..]).unwrap();
    drop(parser);
    assert!(handler.finish().is_err());
}
