/*
* Copyright 2026 Boyd Johnson
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

//! Decoding the raw string text that `Handler::handle_string` and
//! `Handler::handle_map_key` receive.

use std::borrow::Cow;

/// Why a raw JSON string could not be decoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum DecodeError {
    /// The input does not start and end with a double quote.
    NotQuoted,
    /// A raw control character (U+0000 to U+001F) or an unescaped `"`
    /// inside the string.
    UnescapedCharacter {
        /// Byte offset in the raw input.
        at: usize,
    },
    /// A backslash followed by something other than one of `"\/bfnrt` or
    /// `u` and four hex digits.
    InvalidEscape {
        /// Byte offset of the backslash in the raw input.
        at: usize,
    },
    /// A `\u` escape of a UTF-16 surrogate that is not part of a valid
    /// high/low pair. JSON grammar allows these, but they are not Unicode
    /// scalar values, so they cannot be represented in a `str`.
    LoneSurrogate {
        /// Byte offset of the backslash in the raw input.
        at: usize,
    },
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DecodeError::NotQuoted => write!(f, "string is not enclosed in double quotes"),
            DecodeError::UnescapedCharacter { at } => {
                write!(f, "unescaped control character or quote at byte {}", at)
            }
            DecodeError::InvalidEscape { at } => write!(f, "invalid escape at byte {}", at),
            DecodeError::LoneSurrogate { at } => write!(f, "lone UTF-16 surrogate at byte {}", at),
        }
    }
}

impl std::error::Error for DecodeError {}

/// Decode a raw JSON string, as delivered to `Handler::handle_string` and
/// `Handler::handle_map_key`, into its value: strip the quotes, resolve the
/// escapes, and join UTF-16 surrogate pairs.
///
/// Borrows from `raw` when the string has no escapes, so the common case
/// does not allocate.
///
/// ```
/// use yajlish::decode_str;
///
/// assert_eq!(decode_str(r#""foo""#).unwrap(), "foo");
/// assert_eq!(decode_str(r#""a\"b\nc""#).unwrap(), "a\"b\nc");
/// assert_eq!(decode_str(r#""\u00e9""#).unwrap(), "\u{e9}");
/// ```
///
/// # Errors
/// A `DecodeError` if `raw` is not a quoted JSON string, or if it contains a
/// lone surrogate escape.
pub fn decode_str(raw: &str) -> Result<Cow<'_, str>, DecodeError> {
    let bytes = raw.as_bytes();
    if bytes.len() < 2 || bytes[0] != b'"' || bytes[bytes.len() - 1] != b'"' {
        return Err(DecodeError::NotQuoted);
    }
    let body = &raw[1..raw.len() - 1];

    // Fast path: nothing to resolve.
    match body
        .bytes()
        .position(|b| b == b'\\' || b == b'"' || b < 0x20)
    {
        None => return Ok(Cow::Borrowed(body)),
        Some(i) if body.as_bytes()[i] != b'\\' => {
            return Err(DecodeError::UnescapedCharacter { at: i + 1 })
        }
        Some(_) => {}
    }

    let b = body.as_bytes();
    let mut out = String::with_capacity(body.len());
    let mut i = 0;
    // Start of the run of plain characters not yet copied to `out`.
    let mut run = 0;
    while i < b.len() {
        match b[i] {
            b'\\' => {
                out.push_str(&body[run..i]);
                let at = i + 1;
                let esc = *b.get(i + 1).ok_or(DecodeError::InvalidEscape { at })?;
                let c = match esc {
                    b'"' => '"',
                    b'\\' => '\\',
                    b'/' => '/',
                    b'b' => '\u{8}',
                    b'f' => '\u{c}',
                    b'n' => '\n',
                    b'r' => '\r',
                    b't' => '\t',
                    b'u' => {
                        let unit = hex4(b, i + 2).ok_or(DecodeError::InvalidEscape { at })?;
                        match unit {
                            0xD800..=0xDBFF => {
                                // A high surrogate must be followed by an
                                // escaped low surrogate.
                                let low = if b.get(i + 6) == Some(&b'\\')
                                    && b.get(i + 7) == Some(&b'u')
                                {
                                    hex4(b, i + 8)
                                } else {
                                    None
                                };
                                match low {
                                    Some(low @ 0xDC00..=0xDFFF) => {
                                        let scalar = 0x10000
                                            + ((u32::from(unit) - 0xD800) << 10)
                                            + (u32::from(low) - 0xDC00);
                                        i += 6;
                                        char::from_u32(scalar)
                                            .ok_or(DecodeError::LoneSurrogate { at })?
                                    }
                                    _ => return Err(DecodeError::LoneSurrogate { at }),
                                }
                            }
                            0xDC00..=0xDFFF => return Err(DecodeError::LoneSurrogate { at }),
                            _ => char::from_u32(u32::from(unit))
                                .ok_or(DecodeError::InvalidEscape { at })?,
                        }
                    }
                    _ => return Err(DecodeError::InvalidEscape { at }),
                };
                out.push(c);
                i += if esc == b'u' { 6 } else { 2 };
                run = i;
            }
            b'"' | 0x00..=0x1F => return Err(DecodeError::UnescapedCharacter { at: i + 1 }),
            _ => i += 1,
        }
    }
    out.push_str(&body[run..]);
    Ok(Cow::Owned(out))
}

/// Four hex digits at `b[at..at + 4]` as a UTF-16 code unit.
fn hex4(b: &[u8], at: usize) -> Option<u16> {
    let digits = b.get(at..at + 4)?;
    let mut v = 0u16;
    for &d in digits {
        v = v * 16 + u16::from((d as char).to_digit(16)? as u8);
    }
    Some(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(raw: &str, want: &str) {
        assert_eq!(decode_str(raw).as_deref(), Ok(want), "{:?}", raw);
    }

    #[test]
    fn test_no_escapes_borrows() {
        assert!(matches!(decode_str("\"foo\""), Ok(Cow::Borrowed("foo"))));
        assert!(matches!(decode_str("\"\""), Ok(Cow::Borrowed(""))));
        ok("\"caf\u{e9} \u{1F600}\"", "caf\u{e9} \u{1F600}");
    }

    #[test]
    fn test_all_escapes() {
        ok("\"\\\"\"", "\"");
        ok("\"\\\\\"", "\\");
        ok("\"\\/\"", "/");
        ok("\"\\b\\f\\n\\r\\t\"", "\u{8}\u{c}\n\r\t");
        ok("\"a\\nb\\tc\"", "a\nb\tc");
        ok("\"\\u0041\\u00e9\\u00E9\\u20ac\"", "A\u{e9}\u{e9}\u{20ac}");
        ok("\"\\u0000\"", "\u{0}");
        ok("\"x\\uffffy\"", "x\u{ffff}y");
    }

    #[test]
    fn test_surrogate_pairs() {
        ok("\"\\ud83d\\ude00\"", "\u{1F600}");
        ok("\"\\uD834\\uDD1E\"", "\u{1D11E}");
        ok("\"a\\uDBFF\\uDFFFb\"", "a\u{10FFFF}b");
    }

    #[test]
    fn test_lone_surrogates() {
        for raw in [
            "\"\\ud800\"",
            "\"\\udc00\"",
            "\"\\ud800x\"",
            "\"\\ud800\\n\"",
            "\"\\ud800\\u0041\"",
            "\"\\ud800\\ud800\"",
            "\"\\ude00\\ud83d\"",
        ] {
            assert!(
                matches!(decode_str(raw), Err(DecodeError::LoneSurrogate { at: 1 })),
                "{:?} -> {:?}",
                raw,
                decode_str(raw)
            );
        }
    }

    #[test]
    fn test_invalid() {
        assert_eq!(decode_str(""), Err(DecodeError::NotQuoted));
        assert_eq!(decode_str("\""), Err(DecodeError::NotQuoted));
        assert_eq!(decode_str("foo"), Err(DecodeError::NotQuoted));
        assert_eq!(decode_str("\"foo"), Err(DecodeError::NotQuoted));
        assert_eq!(
            decode_str("\"a\\x\""),
            Err(DecodeError::InvalidEscape { at: 2 })
        );
        assert_eq!(
            decode_str("\"\\u12\""),
            Err(DecodeError::InvalidEscape { at: 1 })
        );
        assert_eq!(
            decode_str("\"\\u12g4\""),
            Err(DecodeError::InvalidEscape { at: 1 })
        );
        assert_eq!(
            decode_str("\"\\\""),
            Err(DecodeError::InvalidEscape { at: 1 })
        );
        assert_eq!(
            decode_str("\"a\tb\""),
            Err(DecodeError::UnescapedCharacter { at: 2 })
        );
        assert_eq!(
            decode_str("\"a\"b\""),
            Err(DecodeError::UnescapedCharacter { at: 2 })
        );
        assert_eq!(
            decode_str("\"\\n\"b\""),
            Err(DecodeError::UnescapedCharacter { at: 3 })
        );
    }

    #[test]
    fn test_never_panics_on_short_or_odd_input() {
        for raw in [
            "\"\\u\"",
            "\"\\ud83d\\\"",
            "\"\\ud83d\\u\"",
            "\"\\ud83d\\ude0\"",
            "\"\u{e9}\\",
        ] {
            let _ = decode_str(raw);
        }
    }
}
