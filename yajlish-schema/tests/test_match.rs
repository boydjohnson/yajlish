//! `match = unique` and `match = first` with overlapping schemas.

use yajlish::Parser;
use yajlish_schema::{SchemaError, Status};

mod unique {
    yajlish_schema::handler! {
        pub struct OverlapHandler;
        build = false;
        schemas {
            Loose => "tests/schemas/loose.schema.json",
            Strict => "tests/schemas/strict.schema.json",
        }
    }
}

mod first {
    yajlish_schema::handler! {
        pub struct OverlapHandler;
        build = false;
        match = first;
        schemas {
            Loose => "tests/schemas/loose.schema.json",
            Strict => "tests/schemas/strict.schema.json",
        }
    }
}

macro_rules! run {
    ($m:ident, $input:expr) => {{
        let mut out: Vec<Result<$m::OverlapSchema, SchemaError>> = vec![];
        let mut handler = $m::OverlapHandler::new(|rec| {
            out.push(rec);
            Status::Continue
        });
        let mut parser = Parser::new(&mut handler);
        parser.parse(&mut &$input[..]).unwrap();
        parser.finish_parse().unwrap();
        handler.finish().unwrap();
        out.pop().unwrap()
    }};
}

#[test]
fn test_unique() {
    let err = run!(unique, br#"{"n": 1}"#).unwrap_err();
    assert_eq!(err.keyword, "match");
    assert!(err.detail.contains("Loose and Strict"), "{}", err);
    assert_eq!(
        run!(unique, br#"{"m": 1}"#),
        Ok(unique::OverlapSchema::Loose)
    );
    assert_eq!(
        run!(unique, br#"{"n": 1.5}"#),
        Ok(unique::OverlapSchema::Strict)
    );
    let err = run!(unique, br#"[]"#).unwrap_err();
    assert_eq!(err.keyword, "match");
    assert_eq!(err.causes.len(), 2);
}

#[test]
fn test_first() {
    assert_eq!(run!(first, br#"{"n": 1}"#), Ok(first::OverlapSchema::Loose));
    assert_eq!(
        run!(first, br#"{"n": 1.5}"#),
        Ok(first::OverlapSchema::Strict)
    );
}

#[cfg(feature = "pattern")]
mod pattern {
    use super::*;

    yajlish_schema::handler! {
        pub struct SlugHandler;
        schemas {
            Slug => "tests/schemas/slug.schema.json",
        }
    }

    fn slug(input: &[u8]) -> Result<SlugRecord, SchemaError> {
        let mut out = vec![];
        let mut handler = SlugHandler::new(|rec| {
            out.push(rec);
            Status::Continue
        });
        let mut parser = Parser::new(&mut handler);
        parser.parse(&mut &input[..]).unwrap();
        out.pop().unwrap()
    }

    #[test]
    fn test_pattern() {
        assert_eq!(slug(br#""abc-12""#), Ok(SlugRecord::Slug("abc-12".into())));
        assert_eq!(slug(br#""ABC-12""#).unwrap_err().keyword, "pattern");
        // Matched against the decoded string.
        assert!(slug(b"\"ab\\u0063-1\"").is_ok());
    }
}
