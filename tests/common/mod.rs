use yajlish::Parser;

pub mod mock_handler;

use mock_handler::MockHandler;

pub fn assert_output_equals(mut input: &[u8], output: &[u8]) {
    let mut out = vec![];
    let mut handler = MockHandler::new(&mut out);

    let mut parser = Parser::new(&mut handler);

    parser.parse::<&[u8]>(&mut input).unwrap();

    assert_eq!(parser.finish_parse(), Ok(()));
    assert_eq!(out, output);
}
