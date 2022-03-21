use nom::{
    branch::alt,
    bytes::streaming::{tag, take_while1},
    character::streaming::multispace0,
    combinator::{map, not, recognize},
    multi::many0,
    sequence::delimited,
    IResult, Parser,
};

pub fn parse(s: &[u8]) -> IResult<&[u8], TokenType> {
    alt((
        string,
        left_brace,
        left_bracket,
        null,
        right_brace,
        right_bracket,
        comma,
        colon,
        bool_true,
        bool_false,
    ))(s)
}

fn string_inner(s: &[u8]) -> IResult<&[u8], &[u8]> {
    recognize(quotation_mark
        .and(recognize(many0(
            recognize(not(take_while1(is_control_character)
                .or(quotation_mark)
                .or(reverse_solidus)))
            .or(quotation_mark
                .or(solidus)
                .or(reverse_solidus)
                .or(backspace)
                .or(formfeed)
                .or(newline)
                .or(carriage_return)
                .or(horizontal_tab)
                .or(u_with_hexadecimal_digits)),
        ))).and(quotation_mark))(s)
}

fn is_control_character(c: u8) -> bool {
    let c: char = c.into();

    c.is_control()
}

fn quotation_mark(s: &[u8]) -> IResult<&[u8], &[u8]> {
    nom::error::dbg_dmp(tag("\u{0022}"), "quote")(s)
}

fn solidus(s: &[u8]) -> IResult<&[u8], &[u8]> {
    nom::error::dbg_dmp(tag("/"), "solidus")(s)
}

fn reverse_solidus(s: &[u8]) -> IResult<&[u8], &[u8]> {
    nom::error::dbg_dmp(tag("\\"), "reverse_solidus")(s)
}

fn backspace(s: &[u8]) -> IResult<&[u8], &[u8]> {
    tag(r"\b")(s)
}

fn formfeed(s: &[u8]) -> IResult<&[u8], &[u8]> {
    tag(r"\f")(s)
}

fn newline(s: &[u8]) -> IResult<&[u8], &[u8]> {
    tag("\n")(s)
}

fn carriage_return(s: &[u8]) -> IResult<&[u8], &[u8]> {
    tag("\r")(s)
}

fn horizontal_tab(s: &[u8]) -> IResult<&[u8], &[u8]> {
    tag("\t")(s)
}

fn u_with_hexadecimal_digits(s: &[u8]) -> IResult<&[u8], &[u8]> {
    tag("\\u")(s)
}

fn ws(s: &[u8]) -> IResult<&[u8], &[u8]> {
    multispace0(s)
}

fn left_brace(s: &[u8]) -> IResult<&[u8], TokenType> {
    delimited(ws, map(tag("{"), |_| TokenType::LeftBrace), ws)(s)
}

fn left_bracket(s: &[u8]) -> IResult<&[u8], TokenType> {
    delimited(ws, map(tag("["), |_| TokenType::LeftBracket), ws)(s)
}

fn right_brace(s: &[u8]) -> IResult<&[u8], TokenType> {
    delimited(ws, map(tag("}"), |_| TokenType::RightBrace), ws)(s)
}

fn right_bracket(s: &[u8]) -> IResult<&[u8], TokenType> {
    delimited(ws, map(tag("]"), |_| TokenType::RightBracket), ws)(s)
}

fn string(s: &[u8]) -> IResult<&[u8], TokenType> {
    delimited(ws, map(string_inner, TokenType::String), ws)(s)
}

fn comma(s: &[u8]) -> IResult<&[u8], TokenType> {
    delimited(ws, map(tag(","), |_| TokenType::Comma), ws)(s)
}

fn colon(s: &[u8]) -> IResult<&[u8], TokenType> {
    delimited(ws, map(tag(":"), |_| TokenType::Colon), ws)(s)
}

fn bool_true(s: &[u8]) -> IResult<&[u8], TokenType> {
    delimited(ws, map(tag("true"), |_| TokenType::BoolTrue), ws)(s)
}

fn bool_false(s: &[u8]) -> IResult<&[u8], TokenType> {
    delimited(ws, map(tag("false"), |_| TokenType::BoolFalse), ws)(s)
}

fn null(s: &[u8]) -> IResult<&[u8], TokenType> {
    delimited(ws, map(tag("null"), |_| TokenType::Null), ws)(s)
}

#[derive(Debug)]
pub enum TokenType<'a> {
    LeftBrace,
    RightBrace,
    LeftBracket,
    RightBracket,
    Comma,
    Colon,
    String(&'a [u8]),
    BoolTrue,
    BoolFalse,
    Number(&'a [u8]),
    Null,
}

#[cfg(test)]
mod tests {
    use json_tools::{Lexer, Token};
    use pretty_assertions::assert_eq;

    use super::{string, TokenType};

    #[test]
    fn test_parse_string() {

        let test_string = r#""""#;



        let mut test_lexer = Lexer::new(
            test_string.as_bytes().iter().copied(),
            json_tools::BufferType::Bytes(20),
        );

        let token = test_lexer.next().unwrap();

        let (rest, parsed_token) = string(test_string.as_bytes()).unwrap();

        assert_eq!(parsed_token, token);
    }

    impl<'a> PartialEq<Token> for TokenType<'a> {
        fn eq(&self, other: &Token) -> bool {
            match (self, &other.kind) {
                (TokenType::LeftBrace, json_tools::TokenType::CurlyOpen) => true,
                (TokenType::RightBrace, json_tools::TokenType::CurlyClose) => true,
                (TokenType::LeftBracket, json_tools::TokenType::BracketOpen) => true,
                (TokenType::RightBracket, json_tools::TokenType::BracketClose) => true,
                (TokenType::Comma, json_tools::TokenType::Comma) => true,
                (TokenType::Colon, json_tools::TokenType::CurlyOpen) => true,
                (TokenType::Colon, json_tools::TokenType::CurlyClose) => true,
                (TokenType::Colon, json_tools::TokenType::BracketOpen) => true,
                (TokenType::Colon, json_tools::TokenType::BracketClose) => true,
                (TokenType::Colon, json_tools::TokenType::Colon) => true,
                (TokenType::String(inner), json_tools::TokenType::String) => {
                    json_tools::Buffer::MultiByte(inner.to_vec()) == other.buf
                }
                (TokenType::BoolTrue, json_tools::TokenType::BooleanTrue) => true,
                (TokenType::BoolFalse, json_tools::TokenType::BooleanFalse) => true,
                (TokenType::Number(inner), json_tools::TokenType::Number) => {
                    json_tools::Buffer::MultiByte(inner.to_vec()) == other.buf
                }
                (TokenType::Null, json_tools::TokenType::Null) => true,
                (TokenType::Null, json_tools::TokenType::Invalid) => true,
                (_, _) => false,
            }
        }
    }
}
