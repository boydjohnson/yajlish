pub mod chunked;

use chunked::Chunked;
use pretty_assertions::assert_eq;
use yajlish::Parser;
use yajlish_ndjson::{NdJsonHandler, Selector};

fn read_input_to_bytes(p: &str, s: &str) -> String {
    std::fs::read_to_string(format!("tests/{}/{}", p, s)).unwrap()
}

fn read_output_to_bytes(p: &str, s: &str) -> String {
    std::fs::read_to_string(format!("tests/{}/{}.gold", p, s)).unwrap()
}

pub fn test_ndjson_fixture(p: &str, s: &str, selectors: Vec<Selector>) {
    let input = read_input_to_bytes(p, s);
    let output = read_output_to_bytes(p, s);

    let mut buf = vec![];

    let mut handler = NdJsonHandler::new(&mut buf, selectors);
    let mut parser = Parser::new(&mut handler);

    assert_eq!(parser.parse(&mut input.as_bytes()), Ok(()));

    assert_eq!(output.as_bytes(), buf.as_slice());
}

/// Run the converter with every chunk size in a small set and with a read
/// boundary at every byte; each run must write exactly the gold output.
pub fn test_ndjson_fixture_split(p: &str, s: &str, selectors: Vec<Selector>) {
    let input = read_input_to_bytes(p, s);
    let output = read_output_to_bytes(p, s);
    let input = input.as_bytes();

    let run = |mut reader: Chunked<'_>| {
        let mut buf = vec![];
        let mut handler = NdJsonHandler::new(&mut buf, selectors.clone());
        let mut parser = Parser::new(&mut handler);
        assert_eq!(parser.parse(&mut reader), Ok(()));
        buf
    };

    for chunk in [1, 2, 3, 7, 4096] {
        assert_eq!(
            output.as_bytes(),
            run(Chunked::new(input, chunk)).as_slice()
        );
    }
    for split in 1..input.len() {
        assert_eq!(
            output.as_bytes(),
            run(Chunked::split_at(input, split)).as_slice()
        );
    }
}
