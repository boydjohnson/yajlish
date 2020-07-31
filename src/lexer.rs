/*
* Copyright 2020 Boyd Johnson
*
* Licensed under the Apache License, Version 2.0 (the "License");
* you may not use this file except in compliance with the License.
* You may obtain a copy of the License at
*
*     http://www.apache.org/licenses/LICENSE-2.0
*
* Unless required by applicable law or agreed to in writing, software
* distributed under the License is distributed on an "AS IS" BASIS,
* WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
* See the License for the specific language governing permissions and
* limitations under the License.
* ------------------------------------------------------------------------------
*/

use nom::{
    alt,
    bytes::{
        complete,
        streaming::{tag, take_while, take_while1},
    },
    character::is_digit,
    combinator::{map, map_res, opt},
    named,
    number::complete::double,
    tag, IResult,
};

/// Primitive tokens in json.
#[derive(Debug, PartialEq)]
pub enum JsonPrimitive {
    /// Start Map
    LeftBrace,
    /// End Map
    RightBrace,
    /// Start Array
    LeftBracket,
    /// End Array
    RightBracket,
    /// A Comma
    Comma,
    /// A Colon
    Colon,
    /// Encountered a non-integer number
    Double(f64),
    /// Encountered an integer
    Integer(i64),
    /// Encountered a string, not necessarily a value.
    JSONString(String),
    /// Encountered a null
    Null,
    /// Encountered a boolean
    Boolean(bool),
    /// Encountered whitespace
    WS,
}

pub fn parse_start(data: &[u8]) -> IResult<&[u8], JsonPrimitive> {
    let v = parse_whitespace(data);
    if v.is_ok() {
        return v;
    }
    if let Err(nom::Err::Incomplete(_)) = v {
        return v;
    }

    let v = parse_left_brace(data);
    if v.is_ok() {
        return v;
    }
    if let Err(nom::Err::Incomplete(_)) = v {
        return v;
    }

    let v = parse_left_bracket(data);
    if v.is_ok() {
        return v;
    }
    if let Err(nom::Err::Incomplete(_)) = v {
        return v;
    }
    let v = parse_right_brace(data);
    if v.is_ok() {
        return v;
    }
    if let Err(nom::Err::Incomplete(_)) = v {
        return v;
    }
    let v = parse_right_bracket(data);
    if v.is_ok() {
        return v;
    }
    if let Err(nom::Err::Incomplete(_)) = v {
        return v;
    }
    let v = parse_comma(data);
    if v.is_ok() {
        return v;
    }
    if let Err(nom::Err::Incomplete(_)) = v {
        return v;
    }

    let v = parse_colon(data);
    if v.is_ok() {
        return v;
    }
    if let Err(nom::Err::Incomplete(_)) = v {
        return v;
    }

    let v = parse_string(data);
    if v.is_ok() {
        return v;
    }

    let v = parse_boolean(data);
    if v.is_ok() {
        return v;
    }
    if let Err(nom::Err::Incomplete(_)) = v {
        return v;
    }

    let v = parse_null(data);
    if v.is_ok() {
        return v;
    }
    if let Err(nom::Err::Incomplete(_)) = v {
        return v;
    }

    let v = parse_integer(data);
    if v.is_ok() {
        return v;
    }

    parse_double(data)
}

named!(pub parse<&[u8], JsonPrimitive>,
alt!(
    parse_whitespace
        | parse_left_brace
        | parse_left_bracket
        | parse_right_brace
        | parse_right_bracket
        | parse_comma
        | parse_colon
        | parse_string
        | parse_boolean
        | parse_null
        | parse_integer
        | parse_double
));

fn parse_whitespace(data: &[u8]) -> IResult<&[u8], JsonPrimitive> {
    whitespace(data).map(|(rest, _)| (rest, JsonPrimitive::WS))
}

fn whitespace(data: &[u8]) -> IResult<&[u8], &[u8]> {
    complete::is_a(" \n\r\t")(data)
}

named!(parse_colon_raw, tag!(":"));

named!(parse_comma_raw, tag!(","));

named!(parse_quotation, tag!("\""));

named!(parse_true, tag!("true"));

named!(parse_false, tag!("false"));

named!(parse_null_raw, tag!("null"));

named!(parse_boolean_raw, alt!(parse_true | parse_false));

named!(parse_left_brace_raw, tag!("{"));

named!(parse_right_brace_raw, tag!("}"));

named!(parse_left_bracket_raw, tag!("["));

named!(parse_right_bracket_raw, tag!("]"));

fn parse_null(data: &[u8]) -> IResult<&[u8], JsonPrimitive> {
    parse_null_raw(data).map(|(rest, _)| (rest, JsonPrimitive::Null))
}

fn parse_colon(data: &[u8]) -> IResult<&[u8], JsonPrimitive> {
    parse_colon_raw(data).map(|(rest, _)| (rest, JsonPrimitive::Colon))
}

fn parse_comma(data: &[u8]) -> IResult<&[u8], JsonPrimitive> {
    parse_comma_raw(data).map(|(rest, _)| (rest, JsonPrimitive::Comma))
}

fn parse_left_brace(data: &[u8]) -> IResult<&[u8], JsonPrimitive> {
    parse_left_brace_raw(data).map(|(rest, _)| (rest, JsonPrimitive::LeftBrace))
}

fn parse_right_brace(data: &[u8]) -> IResult<&[u8], JsonPrimitive> {
    parse_right_brace_raw(data).map(|(rest, _)| (rest, JsonPrimitive::RightBrace))
}

fn parse_left_bracket(data: &[u8]) -> IResult<&[u8], JsonPrimitive> {
    parse_left_bracket_raw(data).map(|(rest, _)| (rest, JsonPrimitive::LeftBracket))
}

fn parse_right_bracket(data: &[u8]) -> IResult<&[u8], JsonPrimitive> {
    parse_right_bracket_raw(data).map(|(rest, _)| (rest, JsonPrimitive::RightBracket))
}

fn parse_boolean(data: &[u8]) -> IResult<&[u8], JsonPrimitive> {
    parse_boolean_raw(data).map(|(rest, val)| {
        if val == b"true" {
            (rest, JsonPrimitive::Boolean(true))
        } else {
            (rest, JsonPrimitive::Boolean(false))
        }
    })
}

fn parse_double(data: &[u8]) -> IResult<&[u8], JsonPrimitive> {
    let (rest, first) = take_while1(is_digit_or_num_like)(data)?;
    parse_double_raw(first).map(|(_, v)| (rest, v))
}

fn parse_double_raw(data: &[u8]) -> IResult<&[u8], JsonPrimitive> {
    double(data).map(|(rest, i)| (rest, JsonPrimitive::Double(i)))
}

fn return_integer_as_bytes(data: &[u8]) -> IResult<&[u8], &[u8]> {
    take_while1(is_digit_or_num_like)(data)
}

fn is_digit_or_num_like(c: u8) -> bool {
    is_digit(c)
        || char::from(c) == 'E'
        || char::from(c) == 'e'
        || char::from(c) == '.'
        || char::from(c) == '-'
}

fn parse_integer(data: &[u8]) -> IResult<&[u8], JsonPrimitive> {
    let negative_sign = opt(tag("-"))(data)?;
    let b = return_integer_as_bytes(negative_sign.0)?;
    let s = std::str::from_utf8(b.1)
        .map_err(|_| nom::Err::Error((b.0, nom::error::ErrorKind::AlphaNumeric)))?;
    s.parse::<i64>()
        .map(|mut num| {
            if negative_sign.1.is_some() {
                num = -num;
            }
            (b.0, JsonPrimitive::Integer(num))
        })
        .map_err(|_| nom::Err::Error((b.0, nom::error::ErrorKind::Digit)))
}

fn parse_string_raw(data: &[u8]) -> Result<String, std::str::Utf8Error> {
    std::str::from_utf8(data)
        .map(|s| s.trim_matches('\"'))
        .map(std::borrow::ToOwned::to_owned)
}

fn parse_string(data: &[u8]) -> IResult<&[u8], JsonPrimitive> {
    let q = parse_quotation(data)?;
    let v = map(
        map_res(take_while(|d| char::from(d) != '"'), parse_string_raw),
        JsonPrimitive::JSONString,
    )(q.0)
    .map_err(|e| {
        if let nom::Err::Error((_, ek)) = e {
            return nom::Err::Error((data, ek));
        }
        e
    })?;
    let last = tag("\"")(v.0).map_err(|e| {
        if let nom::Err::Error((_, ek)) = e {
            return nom::Err::Error((data, ek));
        }
        e
    })?;
    Ok((last.0, v.1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_double() {
        assert_eq!(
            parse_double(b"7.5, \"foo\": \"bar\""),
            Ok((", \"foo\": \"bar\"".as_bytes(), JsonPrimitive::Double(7.5)))
        );
        assert_eq!(
            parse_double(b"8, \"data\": [1, 2, 3, 4]"),
            Ok((
                ", \"data\": [1, 2, 3, 4]".as_bytes(),
                JsonPrimitive::Double(8.0)
            ))
        );
        assert_eq!(
            parse_double(b"\"foo\": \"bar\", \"n\": 9.5"),
            Err(nom::Err::Error((
                "\"foo\": \"bar\", \"n\": 9.5".as_bytes(),
                nom::error::ErrorKind::TakeWhile1
            )))
        );
        assert_eq!(
            parse_double(b"42."),
            Err(nom::Err::Incomplete(nom::Needed::new(1)))
        )
    }

    #[test]
    fn test_parse_integer() {
        assert_eq!(
            parse_integer(b"8, foo: bar"),
            Ok((", foo: bar".as_bytes(), JsonPrimitive::Integer(8)))
        );

        assert_eq!(
            parse_integer(b"foo: bar, "),
            Err(nom::Err::Error((
                "foo: bar, ".as_bytes(),
                nom::error::ErrorKind::TakeWhile1
            )))
        );

        assert_eq!(
            parse_integer(b"-78, foo: bar"),
            Ok((", foo: bar".as_bytes(), JsonPrimitive::Integer(-78)))
        );
        assert_eq!(
            parse_integer(b"42."),
            Err(nom::Err::Incomplete(nom::Needed::new(1)))
        )
    }

    #[test]
    fn test_parse_string() {
        assert_eq!(
            parse_string(b"\"foo\": \"bar\""),
            Ok((
                ": \"bar\"".as_bytes(),
                JsonPrimitive::JSONString("foo".to_owned())
            ))
        );
        assert_eq!(
            parse_string("\"diggity❤❤❤❤❤❤❤❤❤❤\"".as_bytes()),
            Ok((
                "".as_bytes(),
                JsonPrimitive::JSONString("diggity❤❤❤❤❤❤❤❤❤❤".to_owned())
            ))
        );

        assert_eq!(
            parse_string(b"\"\""),
            Ok(("".as_bytes(), JsonPrimitive::JSONString("".to_owned())))
        );

        assert_eq!(
            parse_string(b"\"\": null, \"\": false"),
            Ok((
                ": null, \"\": false".as_bytes(),
                JsonPrimitive::JSONString("".to_owned())
            ))
        );
    }

    #[test]
    fn test_parse_boolean() {
        assert_eq!(
            parse_boolean(b"true"),
            Ok(("".as_bytes(), JsonPrimitive::Boolean(true)))
        );
        assert_eq!(
            parse_boolean(b"false, foo: bar"),
            Ok((", foo: bar".as_bytes(), JsonPrimitive::Boolean(false)))
        );
        assert_eq!(
            parse_boolean(b"null, foo: bar"),
            Err(nom::Err::Error((
                "null, foo: bar".as_bytes(),
                nom::error::ErrorKind::Alt
            )))
        );
    }

    #[test]
    fn test_parse() {
        assert_eq!(
            parse(b"{ \"foo\": \"bar\"}"),
            Ok((" \"foo\": \"bar\"}".as_bytes(), JsonPrimitive::LeftBrace))
        );
        assert_eq!(
            parse(b" \"foo\": \"bar\"}"),
            Ok(("\"foo\": \"bar\"}".as_bytes(), JsonPrimitive::WS))
        );
        assert_eq!(
            parse(b"\"foo\": \"bar\"}"),
            Ok((
                ": \"bar\"}".as_bytes(),
                JsonPrimitive::JSONString("foo".to_owned())
            ))
        );
        assert_eq!(
            parse(b": \"bar\"}"),
            Ok((" \"bar\"}".as_bytes(), JsonPrimitive::Colon))
        );
        assert_eq!(
            parse(b" \"bar\"}"),
            Ok(("\"bar\"}".as_bytes(), JsonPrimitive::WS))
        );
        assert_eq!(
            parse(b"\"bar\"}"),
            Ok(("}".as_bytes(), JsonPrimitive::JSONString("bar".to_owned())))
        );
        assert_eq!(parse(b"}"), Ok(("".as_bytes(), JsonPrimitive::RightBrace)));
        assert_eq!(
            parse(b"[1,2,3]"),
            Ok(("1,2,3]".as_bytes(), JsonPrimitive::LeftBracket))
        );
        assert_eq!(
            parse(b"1,2,3]"),
            Ok((",2,3]".as_bytes(), JsonPrimitive::Integer(1)))
        );
        assert_eq!(
            parse(b",2,3]"),
            Ok(("2,3]".as_bytes(), JsonPrimitive::Comma))
        );
        assert_eq!(
            parse(b"]"),
            Ok(("".as_bytes(), JsonPrimitive::RightBracket))
        );
        assert_eq!(
            parse(b"9.5, foo: bar"),
            Ok((", foo: bar".as_bytes(), JsonPrimitive::Double(9.5)))
        );
        assert_eq!(
            parse(b"42."),
            Err(nom::Err::Incomplete(nom::Needed::new(1)))
        );
        assert_eq!(
            parse(b"[{ \"\": null }]"),
            Ok(("{ \"\": null }]".as_bytes(), JsonPrimitive::LeftBracket))
        );
        assert_eq!(
            parse(b"{ \"\": null }]"),
            Ok((" \"\": null }]".as_bytes(), JsonPrimitive::LeftBrace))
        );
        assert_eq!(
            parse(b" \"\": null }]"),
            Ok(("\"\": null }]".as_bytes(), JsonPrimitive::WS))
        );
        assert_eq!(
            parse(b"\"\": null }]"),
            Ok((
                ": null }]".as_bytes(),
                JsonPrimitive::JSONString("".to_owned())
            ))
        );
    }

    #[test]
    fn test_parse_start() {
        assert_eq!(
            parse_start(b"nul"),
            Err(nom::Err::Incomplete(nom::Needed::new(4)))
        );

        assert_eq!(
            parse_start(b"nuell"),
            Err(nom::Err::Error((
                "nuell".as_bytes(),
                nom::error::ErrorKind::TakeWhile1
            )))
        );

        assert_eq!(
            parse_start(b"fals"),
            Err(nom::Err::Incomplete(nom::Needed::new(5)))
        );

        assert_eq!(
            parse_start(b"flase"),
            Err(nom::Err::Error((
                "flase".as_bytes(),
                nom::error::ErrorKind::TakeWhile1
            )))
        );

        assert_eq!(
            parse_start(b"9.444555"),
            Err(nom::Err::Incomplete(nom::Needed::new(1)))
        );
    }
}
