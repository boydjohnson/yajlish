mod common;

use common::{read_input_to_bytes, test_ndjson_fixture, test_ndjson_fixture_split};
use yajlish::Parser;
use yajlish_ndjson::{NdJsonError, NdJsonHandler, Selector, SelectorParser};

const PATH: &str = "ndjson_fixtures";

#[test]
fn test_custom_geojson() {
    test_ndjson_fixture(
        PATH,
        "custom.geo.json",
        vec![Selector::Identifier("features".to_owned())],
    );
}

#[test]
fn test_custom_json() {
    test_ndjson_fixture(
        PATH,
        "prize.json",
        vec![Selector::Identifier("prizes".to_owned())],
    );
}

#[test]
fn test_fixtures_split_invariant() {
    test_ndjson_fixture_split(
        PATH,
        "custom.geo.json",
        vec![Selector::Identifier("features".to_owned())],
    );
    test_ndjson_fixture_split(
        PATH,
        "prize.json",
        vec![Selector::Identifier("prizes".to_owned())],
    );
}

fn convert(
    input: &[u8],
    selector: &str,
) -> (
    Result<(), yajlish::ParseError>,
    Result<Vec<u8>, NdJsonError>,
) {
    let selectors = SelectorParser::new().parse(selector).unwrap();
    let mut handler = NdJsonHandler::new(vec![], selectors);
    let mut parser = Parser::new(&mut handler);
    let mut reader = input;
    let parsed = parser
        .parse(&mut reader)
        .and_then(|()| parser.finish_parse());
    (parsed, handler.finish())
}

#[test]
fn test_selector_parser_output_matches_fixtures() {
    for (file, selector) in [
        ("custom.geo.json", "d.features"),
        ("prize.json", "d.prizes"),
    ] {
        let input = read_input_to_bytes(PATH, file);
        let gold = std::fs::read(format!("tests/{}/{}.gold", PATH, file)).unwrap();
        let (parsed, out) = convert(input.as_bytes(), selector);
        assert_eq!(parsed, Ok(()));
        assert_eq!(out.unwrap(), gold);
    }
}

#[test]
fn test_escaped_key_matches() {
    let (parsed, out) = convert(
        b"{\"f\\u0065atures\": [1, 2], \"features\": [3]}",
        "d.features",
    );
    assert_eq!(parsed, Ok(()));
    assert_eq!(out.unwrap(), b"1\n2\n");
}

#[test]
fn test_missing_selector_is_an_error() {
    let (parsed, out) = convert(b"{\"other\": [1, 2]}", "d.features");
    assert_eq!(parsed, Ok(()));
    assert!(matches!(out, Err(NdJsonError::SelectorNotFound)));

    let (_, out) = convert(b"{\"features\": 7}", "d.features");
    assert!(matches!(out, Err(NdJsonError::SelectorNotFound)));
}

#[test]
fn test_malformed_tail_is_rejected() {
    let input = b"{\"features\": [1, 2], \"after\": tru}";
    let (parsed, out) = convert(input, "d.features");
    assert!(parsed.is_err());
    assert_eq!(out.unwrap(), b"1\n2\n");

    // Opting in to the early stop skips the tail.
    let mut handler =
        NdJsonHandler::new(vec![], SelectorParser::new().parse("d.features").unwrap()).stop_early();
    let mut parser = Parser::new(&mut handler);
    assert_eq!(parser.parse(&mut &input[..]), Ok(()));
    assert_eq!(handler.finish().unwrap(), b"1\n2\n");
}

#[test]
fn test_non_finite_double_is_rejected() {
    let (_, out) = convert(b"{\"features\": [1, 1e400]}", "d.features");
    assert!(matches!(out, Err(NdJsonError::NonFiniteNumber)));

    // Outside the selected array it does not matter.
    let (parsed, out) = convert(b"{\"x\": 1e400, \"features\": [1]}", "d.features");
    assert_eq!(parsed, Ok(()));
    assert_eq!(out.unwrap(), b"1\n");
}

/// A writer that accepts `budget` bytes, then fails.
struct FailAfter {
    budget: usize,
}

impl std::io::Write for FailAfter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if self.budget == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "closed",
            ));
        }
        let n = buf.len().min(self.budget);
        self.budget -= n;
        Ok(n)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[test]
fn test_write_errors_are_returned_not_panicked() {
    for (file, selector) in [
        ("custom.geo.json", "d.features"),
        ("prize.json", "d.prizes"),
    ] {
        let input = read_input_to_bytes(PATH, file);
        let gold_len = std::fs::read(format!("tests/{}/{}.gold", PATH, file))
            .unwrap()
            .len();
        for budget in 0..gold_len {
            let selectors = SelectorParser::new().parse(selector).unwrap();
            let mut handler = NdJsonHandler::new(FailAfter { budget }, selectors);
            let mut parser = Parser::new(&mut handler);
            assert_eq!(parser.parse(&mut input.as_bytes()), Ok(()));
            assert!(
                matches!(handler.finish(), Err(NdJsonError::Io(_))),
                "{} budget {}",
                file,
                budget
            );
        }
    }
}
