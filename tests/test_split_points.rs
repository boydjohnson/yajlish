//! Every input must produce the same events and the same result however
//! the reader splits it.

mod common;

use common::{chunked::Chunked, mock_handler::MockHandler};
use std::io::BufRead;
use yajlish::{ParseError, Parser};

fn run<B: BufRead>(mut reader: B) -> (Vec<u8>, Result<(), ParseError>) {
    let mut out = vec![];
    let mut handler = MockHandler::new(&mut out);
    let mut parser = Parser::new(&mut handler);
    let result = parser
        .parse(&mut reader)
        .and_then(|()| parser.finish_parse());
    (out, result)
}

fn assert_split_invariant(input: &[u8]) {
    let whole = run(input);
    for chunk in [1, 2, 3, 7, 4096] {
        assert_eq!(
            run(Chunked::new(input, chunk)),
            whole,
            "{:?} chunk {}",
            String::from_utf8_lossy(input),
            chunk
        );
    }
    for split in 1..input.len() {
        assert_eq!(
            run(Chunked::split_at(input, split)),
            whole,
            "{:?} split at {}",
            String::from_utf8_lossy(input),
            split
        );
    }
}

#[test]
fn test_fixtures_split_invariant() {
    for dir in [
        "tests/valid_json_fixtures",
        "yajlish-ndjson/tests/ndjson_fixtures",
    ] {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(dir);
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_some_and(|e| e == "json") {
                let input = std::fs::read(&path).unwrap();
                assert!(run(input.as_slice()).1.is_ok(), "{:?}", path);
                assert_split_invariant(&input);
            }
        }
    }
}

#[test]
fn test_tricky_inputs_split_invariant() {
    let corpus: &[&[u8]] = &[
        b"[1, 23, -456, 7.5, -0.25e10, 1E+2, 0, -0, 9223372036854775808]",
        b"{\"a\\\"b\": \"x\\\\y\\u00e9\\ud83d\\ude00\", \"k\": [true, false, null]}",
        b"  \n\t 12345678901234567890  ",
        b"\"\xc3\xa9\xf0\x9f\x98\x80\"",
        b"[\"\",\"\",{},[],[[]],{\"\":{}}]",
        b"1.5e-3",
        b"-0.0",
        // Errors must be the same error at every split.
        b"[1.]",
        b"[tru]",
        b"\"abc",
        b"[-]",
        b"[1e]",
        b"[01]",
        b"\"\xff\"",
        b"[\"a\x01\"]",
        b"[1, @]",
    ];
    for input in corpus {
        assert_split_invariant(input);
    }
}
