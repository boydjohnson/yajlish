//! Parser for selector strings such as `d.features[2].name`.
//!
//! The grammar: `d`, then any mix of `.key` (one or more characters other
//! than `.` and `[`) and `[n]` (decimal digits). Whitespace before each
//! piece is skipped.

use crate::Selector;

/// Why a selector string could not be parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectorParseError {
    /// Byte offset in the selector string.
    pub at: usize,
    /// What was expected there.
    pub msg: &'static str,
}

impl std::fmt::Display for SelectorParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} at byte {}", self.msg, self.at)
    }
}

impl std::error::Error for SelectorParseError {}

/// Parses a whole selector: `d` followed by keys and indices.
#[derive(Debug, Default)]
pub struct SelectorParser;

/// Parses a single `.key`.
#[derive(Debug, Default)]
pub struct IdentifierTokenParser;

/// Parses a single `[n]`.
#[derive(Debug, Default)]
pub struct ArrayTokenParser;

impl SelectorParser {
    /// Constructor.
    #[must_use]
    pub fn new() -> Self {
        SelectorParser
    }

    /// Parse `input` into the list of selectors after the leading `d`.
    ///
    /// # Errors
    /// A `SelectorParseError` naming the offset of the first problem.
    pub fn parse(&self, input: &str) -> Result<Vec<Selector>, SelectorParseError> {
        let mut c = Cursor { input, pos: 0 };
        c.skip_ws();
        if !c.rest().starts_with('d') {
            return c.err("expected `d`");
        }
        c.pos += 1;
        let mut out = vec![];
        loop {
            c.skip_ws();
            match c.rest().as_bytes().first() {
                None => return Ok(out),
                Some(b'.') => out.push(c.identifier()?),
                Some(b'[') => out.push(c.index()?),
                Some(_) => return c.err("expected `.key` or `[index]`"),
            }
        }
    }
}

impl IdentifierTokenParser {
    /// Constructor.
    #[must_use]
    pub fn new() -> Self {
        IdentifierTokenParser
    }

    /// Parse exactly one `.key`.
    ///
    /// # Errors
    /// A `SelectorParseError` if `input` is anything else.
    pub fn parse(&self, input: &str) -> Result<Selector, SelectorParseError> {
        let mut c = Cursor { input, pos: 0 };
        c.skip_ws();
        let sel = c.identifier()?;
        c.end(sel)
    }
}

impl ArrayTokenParser {
    /// Constructor.
    #[must_use]
    pub fn new() -> Self {
        ArrayTokenParser
    }

    /// Parse exactly one `[n]`.
    ///
    /// # Errors
    /// A `SelectorParseError` if `input` is anything else, or the index
    /// does not fit in `usize`.
    pub fn parse(&self, input: &str) -> Result<Selector, SelectorParseError> {
        let mut c = Cursor { input, pos: 0 };
        c.skip_ws();
        let sel = c.index()?;
        c.end(sel)
    }
}

struct Cursor<'a> {
    input: &'a str,
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn rest(&self) -> &'a str {
        &self.input[self.pos..]
    }

    fn err<T>(&self, msg: &'static str) -> Result<T, SelectorParseError> {
        Err(SelectorParseError { at: self.pos, msg })
    }

    fn skip_ws(&mut self) {
        let rest = self.rest();
        self.pos += rest.len() - rest.trim_start().len();
    }

    fn end(mut self, sel: Selector) -> Result<Selector, SelectorParseError> {
        self.skip_ws();
        if self.rest().is_empty() {
            Ok(sel)
        } else {
            self.err("unexpected trailing input")
        }
    }

    /// `.` then one or more characters other than `.` and `[`.
    fn identifier(&mut self) -> Result<Selector, SelectorParseError> {
        if !self.rest().starts_with('.') {
            return self.err("expected `.`");
        }
        self.pos += 1;
        let rest = self.rest();
        let len = rest.find(['.', '[']).unwrap_or(rest.len());
        if len == 0 {
            return self.err("expected a key after `.`");
        }
        self.pos += len;
        Ok(Selector::Identifier(rest[..len].to_owned()))
    }

    /// `[`, decimal digits, `]`.
    fn index(&mut self) -> Result<Selector, SelectorParseError> {
        if !self.rest().starts_with('[') {
            return self.err("expected `[`");
        }
        self.pos += 1;
        let rest = self.rest();
        let len = rest.bytes().take_while(u8::is_ascii_digit).count();
        if len == 0 {
            return self.err("expected digits after `[`");
        }
        let index = rest[..len]
            .parse::<usize>()
            .map_err(|_| SelectorParseError {
                at: self.pos,
                msg: "Index too big",
            })?;
        self.pos += len;
        if !self.rest().starts_with(']') {
            return self.err("expected `]`");
        }
        self.pos += 1;
        Ok(Selector::Index(index))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_errors() {
        let p = SelectorParser::new();
        assert_eq!(p.parse("").unwrap_err().at, 0);
        assert_eq!(p.parse("x.foo").unwrap_err().at, 0);
        assert_eq!(p.parse("d.").unwrap_err().at, 2);
        assert_eq!(p.parse("d..a").unwrap_err().at, 2);
        assert_eq!(p.parse("d[]").unwrap_err().at, 2);
        assert_eq!(p.parse("d[1").unwrap_err().at, 3);
        assert_eq!(p.parse("d[x]").unwrap_err().at, 2);
        assert_eq!(p.parse("dx").unwrap_err().at, 1);
        assert_eq!(
            p.parse("d[99999999999999999999999]").unwrap_err().msg,
            "Index too big"
        );
        assert!(IdentifierTokenParser::new().parse(".a[1]").is_err());
        assert!(ArrayTokenParser::new().parse("[1].a").is_err());
    }

    #[test]
    fn test_whitespace_and_unicode() {
        assert_eq!(
            SelectorParser::new().parse(" d [1] .caf\u{e9} x"),
            Ok(vec![
                Selector::Index(1),
                Selector::Identifier("caf\u{e9} x".into())
            ])
        );
    }
}
