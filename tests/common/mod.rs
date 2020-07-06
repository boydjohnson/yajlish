use yajlish::Parser;

pub mod mock_handler;

use mock_handler::MockHandler;
use pretty_assertions::assert_eq;

pub fn assert_output_equals(mut input: &[u8], output: &[u8]) {
    let mut out = vec![];
    let mut handler = MockHandler::new(&mut out);

    let mut parser = Parser::new(&mut handler);

    parser.parse::<&[u8]>(&mut input).unwrap();

    assert_eq!(parser.finish_parse(), Ok(()));
    assert_eq!(out, output);
}

#[allow(unused)]
fn read_input_to_bytes(p: &str, s: &str) -> String {
    std::fs::read_to_string(format!("tests/{}/{}", p, s)).unwrap()
}

#[allow(unused)]
fn read_output_to_bytes(p: &str, s: &str) -> String {
    std::fs::read_to_string(format!("tests/{}/{}.gold", p, s)).unwrap()
}

#[allow(unused)]
pub fn test_fixture(p: &str, s: &str) {
    let input = read_input_to_bytes(p, s);
    let output = read_output_to_bytes(p, s);

    assert_output_equals(input.as_bytes(), output.as_bytes());
}

#[cfg(feature = "ndjson")]
#[allow(unused)]
pub fn test_ndjson_fixture(p: &str, s: &str, selectors: Vec<yajlish::ndjson_handler::Selector>) {
    let input = read_input_to_bytes(p, s);
    let output = read_output_to_bytes(p, s);

    let mut buf = vec![];

    let mut handler = yajlish::ndjson_handler::NdJsonHandler::new(&mut buf, selectors);
    let mut parser = Parser::new(&mut handler);

    assert_eq!(parser.parse(&mut input.as_bytes()), Ok(()));

    assert_eq!(output.as_bytes(), buf.as_slice());
}
