mod common;

use common::assert_output_equals;

#[test]
fn test_bool() {
    assert_output_equals(b"true", b"bool: true\n");
}

#[test]
fn test_empty_map() {
    assert_output_equals(b"{ }", b"map open '{'\nmap close '}'\n");
}

#[test]
fn test_simple_map() {
    assert_output_equals(
        b"{ \"foo\": 27 }",
        "map open '{'\nkey: \"foo\"\ninteger: 27\nmap close '}'\n".as_bytes(),
    );
}

#[test]
fn test_empty_array() {
    assert_output_equals(b"[ ]", b"array open '['\narray close ']'\n");
}

#[test]
fn test_simple_array() {
    assert_output_equals(
        b"[1, \"foo\", 7.5]",
        "array open '['\ninteger: 1\nstring: '\"foo\"'\ndouble: 7.5\narray close ']'\n".as_bytes(),
    );
}

#[test]
fn test_complex_map() {
    assert_output_equals("{ \"foo\": [{ \"bar\": [1, 2, null]}, true] }".as_bytes(),
        "map open '{'\nkey: \"foo\"\narray open '['\nmap open '{'\nkey: \"bar\"\narray open '['\ninteger: 1\ninteger: 2\nnull\narray close ']'\nmap close '}'\nbool: true\narray close ']'\nmap close '}'\n".as_bytes());
}

#[test]
fn test_complex_array() {
    assert_output_equals("[1, \"foo\", [null, 4.5, 8], false, { \"data\": null }]".as_bytes(), "array open '['\ninteger: 1\nstring: '\"foo\"'\narray open '['\nnull\ndouble: 4.5\ninteger: 8\narray close ']'\nbool: false\nmap open '{'\nkey: \"data\"\nnull\nmap close '}'\narray close ']'\n".as_bytes());
}

#[test]
fn test_bool_with_newline() {
    assert_output_equals(b"true\n ", b"bool: true\n");
}

#[test]
fn test_array_with_string() {
    assert_output_equals(
        b"[\"foo\"]",
        "array open '['\nstring: '\"foo\"'\narray close ']'\n".as_bytes(),
    );
}

use yajlish::{Context, Enclosing, Handler, Parser, ParserStatus, Status};

#[derive(Debug, PartialEq)]
pub enum Token {
    Key,
    LeftBrace,
    LeftBracket,
    RightBrace,
    RightBracket,
    Bool,
    Double,
    Int,
    String,
    Null,
}

struct ContextTestHandler(Vec<(Token, ParserStatus, Option<Enclosing>, usize, usize)>);

impl ContextTestHandler {
    fn handle(&mut self, ctx: &Context, tok: Token) -> Status {
        if let Some((token_type, status, enclosing, num_braces, num_brackets)) = self.0.pop() {
            assert_eq!(ctx.parser_status(), status);

            assert_eq!(ctx.last_enclosing(), enclosing);

            assert_eq!(ctx.num_open_braces(), num_braces);

            assert_eq!(ctx.num_open_brackets(), num_brackets);

            assert_eq!(tok, token_type);
        }
        Status::Continue
    }
}

impl Handler for ContextTestHandler {
    fn handle_bool(&mut self, ctx: &Context, _: bool) -> Status {
        self.handle(ctx, Token::Bool)
    }

    fn handle_double(&mut self, ctx: &Context, _: f64) -> Status {
        self.handle(ctx, Token::Double)
    }

    fn handle_end_array(&mut self, ctx: &Context) -> Status {
        self.handle(ctx, Token::RightBracket)
    }

    fn handle_map_key(&mut self, ctx: &Context, _: &str) -> Status {
        self.handle(ctx, Token::Key)
    }

    fn handle_end_map(&mut self, ctx: &Context) -> Status {
        self.handle(ctx, Token::RightBrace)
    }

    fn handle_int(&mut self, ctx: &Context, _: i64) -> Status {
        self.handle(ctx, Token::Int)
    }

    fn handle_null(&mut self, ctx: &Context) -> Status {
        self.handle(ctx, Token::Null)
    }

    fn handle_start_array(&mut self, ctx: &Context) -> Status {
        self.handle(ctx, Token::LeftBracket)
    }

    fn handle_start_map(&mut self, ctx: &Context) -> Status {
        self.handle(ctx, Token::LeftBrace)
    }

    fn handle_string(&mut self, ctx: &Context, _: &str) -> Status {
        self.handle(ctx, Token::String)
    }
}

#[test]
fn test_parser_context_array() {
    let mut input = "[false,5,5.5,\"foo\"]".as_bytes();

    let context_asserts = vec![
        (
            Token::RightBracket,
            ParserStatus::ArrayGotVal,
            Some(Enclosing::LeftBracket),
            0,
            1,
        ),
        (
            Token::String,
            ParserStatus::ArrayNeedVal,
            Some(Enclosing::LeftBracket),
            0,
            1,
        ),
        (
            Token::Double,
            ParserStatus::ArrayNeedVal,
            Some(Enclosing::LeftBracket),
            0,
            1,
        ),
        (
            Token::Int,
            ParserStatus::ArrayNeedVal,
            Some(Enclosing::LeftBracket),
            0,
            1,
        ),
        (
            Token::Bool,
            ParserStatus::ArrayStart,
            Some(Enclosing::LeftBracket),
            0,
            1,
        ),
        (Token::LeftBracket, ParserStatus::Start, None, 0, 0),
    ];

    let mut handler = ContextTestHandler(context_asserts);

    let mut parser = Parser::new(&mut handler);

    parser.parse(&mut input).unwrap();
}

#[test]
fn test_parser_context_map() {
    let mut input = "{\"foo\":{\"bar\":[1]},\"age\":25}".as_bytes();

    let context_asserts = vec![
        (
            Token::RightBrace,
            ParserStatus::MapGotVal,
            Some(Enclosing::LeftBrace),
            1,
            0,
        ),
        (
            Token::Int,
            ParserStatus::MapNeedVal,
            Some(Enclosing::LeftBrace),
            1,
            0,
        ),
        (
            Token::Key,
            ParserStatus::MapNeedKey,
            Some(Enclosing::LeftBrace),
            1,
            0,
        ),
        (
            Token::RightBrace,
            ParserStatus::MapGotVal,
            Some(Enclosing::LeftBrace),
            2,
            0,
        ),
        (
            Token::RightBracket,
            ParserStatus::ArrayGotVal,
            Some(Enclosing::LeftBracket),
            2,
            1,
        ),
        (
            Token::Int,
            ParserStatus::ArrayStart,
            Some(Enclosing::LeftBracket),
            2,
            1,
        ),
        (
            Token::LeftBracket,
            ParserStatus::MapNeedVal,
            Some(Enclosing::LeftBrace),
            2,
            0,
        ),
        (
            Token::Key,
            ParserStatus::MapStart,
            Some(Enclosing::LeftBrace),
            2,
            0,
        ),
        (
            Token::LeftBrace,
            ParserStatus::MapNeedVal,
            Some(Enclosing::LeftBrace),
            1,
            0,
        ),
        (
            Token::Key,
            ParserStatus::MapStart,
            Some(Enclosing::LeftBrace),
            1,
            0,
        ),
        (Token::LeftBrace, ParserStatus::Start, None, 0, 0),
    ];
    let mut handler = ContextTestHandler(context_asserts);

    let mut parser = Parser::new(&mut handler);

    parser.parse(&mut input).unwrap();
}
