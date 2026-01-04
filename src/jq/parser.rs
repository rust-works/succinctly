//! Parser for jq-like query expressions.
//!
//! Supports a subset of jq syntax:
//! - `.` - identity
//! - `.foo` - field access
//! - `.[0]` - array index
//! - `.[]` - iterate
//! - `.[2:5]` - slice
//! - `.foo.bar` - chained access
//! - `.foo?` - optional (returns null if missing)

#[cfg(not(test))]
use alloc::format;
#[cfg(not(test))]
use alloc::string::{String, ToString};
#[cfg(not(test))]
use alloc::vec;

use super::expr::Expr;

/// Error that occurs during parsing.
#[derive(Debug, Clone, PartialEq)]
pub struct ParseError {
    pub message: String,
    pub position: usize,
}

impl ParseError {
    fn new(message: impl Into<String>, position: usize) -> Self {
        ParseError {
            message: message.into(),
            position,
        }
    }
}

impl core::fmt::Display for ParseError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "parse error at position {}: {}",
            self.position, self.message
        )
    }
}

/// Parser state.
struct Parser<'a> {
    input: &'a str,
    pos: usize,
}

impl<'a> Parser<'a> {
    fn new(input: &'a str) -> Self {
        Parser { input, pos: 0 }
    }

    /// Peek at the current character without consuming it.
    fn peek(&self) -> Option<char> {
        self.input[self.pos..].chars().next()
    }

    /// Consume and return the current character.
    fn next(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.pos += c.len_utf8();
        Some(c)
    }

    /// Skip whitespace.
    fn skip_ws(&mut self) {
        while let Some(c) = self.peek() {
            if c.is_whitespace() {
                self.next();
            } else {
                break;
            }
        }
    }

    /// Check if we're at the end of input.
    fn is_eof(&self) -> bool {
        self.pos >= self.input.len()
    }

    /// Consume a specific character or return error.
    fn expect(&mut self, expected: char) -> Result<(), ParseError> {
        self.skip_ws();
        match self.peek() {
            Some(c) if c == expected => {
                self.next();
                Ok(())
            }
            Some(c) => Err(ParseError::new(
                format!("expected '{}', found '{}'", expected, c),
                self.pos,
            )),
            None => Err(ParseError::new(
                format!("expected '{}', found end of input", expected),
                self.pos,
            )),
        }
    }

    /// Parse an identifier (field name).
    fn parse_ident(&mut self) -> Result<String, ParseError> {
        let start = self.pos;

        // First character must be alphabetic or underscore
        match self.peek() {
            Some(c) if c.is_alphabetic() || c == '_' => {
                self.next();
            }
            Some(c) => {
                return Err(ParseError::new(
                    format!("expected identifier, found '{}'", c),
                    self.pos,
                ));
            }
            None => {
                return Err(ParseError::new(
                    "expected identifier, found end of input",
                    self.pos,
                ));
            }
        }

        // Subsequent characters can be alphanumeric or underscore
        while let Some(c) = self.peek() {
            if c.is_alphanumeric() || c == '_' {
                self.next();
            } else {
                break;
            }
        }

        Ok(self.input[start..self.pos].to_string())
    }

    /// Parse a number (positive or negative integer).
    fn parse_number(&mut self) -> Result<i64, ParseError> {
        let start = self.pos;

        // Optional negative sign
        if self.peek() == Some('-') {
            self.next();
        }

        // Must have at least one digit
        match self.peek() {
            Some(c) if c.is_ascii_digit() => {
                self.next();
            }
            _ => {
                return Err(ParseError::new("expected digit", self.pos));
            }
        }

        // Consume remaining digits
        while let Some(c) = self.peek() {
            if c.is_ascii_digit() {
                self.next();
            } else {
                break;
            }
        }

        self.input[start..self.pos]
            .parse()
            .map_err(|_| ParseError::new("invalid number", start))
    }

    /// Parse a bracket expression: `[0]`, `[]`, `[1:3]`, etc.
    fn parse_bracket(&mut self) -> Result<Expr, ParseError> {
        self.expect('[')?;
        self.skip_ws();

        // Empty brackets = iterate
        if self.peek() == Some(']') {
            self.next();
            return Ok(Expr::Iterate);
        }

        // Check for slice starting with ':'
        if self.peek() == Some(':') {
            self.next();
            self.skip_ws();

            if self.peek() == Some(']') {
                // `[:]` - full slice (same as iterate)
                self.next();
                return Ok(Expr::Iterate);
            }

            // `[:n]` - slice from start to n
            let end = self.parse_number()?;
            self.skip_ws();
            self.expect(']')?;
            return Ok(Expr::Slice {
                start: None,
                end: Some(end),
            });
        }

        // Parse first number
        let first = self.parse_number()?;
        self.skip_ws();

        match self.peek() {
            Some(']') => {
                // `[n]` - simple index
                self.next();
                Ok(Expr::Index(first))
            }
            Some(':') => {
                // `[n:]` or `[n:m]` - slice
                self.next();
                self.skip_ws();

                if self.peek() == Some(']') {
                    // `[n:]` - slice from n to end
                    self.next();
                    Ok(Expr::Slice {
                        start: Some(first),
                        end: None,
                    })
                } else {
                    // `[n:m]` - slice from n to m
                    let second = self.parse_number()?;
                    self.skip_ws();
                    self.expect(']')?;
                    Ok(Expr::Slice {
                        start: Some(first),
                        end: Some(second),
                    })
                }
            }
            Some(c) => Err(ParseError::new(
                format!("expected ']' or ':', found '{}'", c),
                self.pos,
            )),
            None => Err(ParseError::new(
                "expected ']' or ':', found end of input",
                self.pos,
            )),
        }
    }

    /// Parse a bracket expression and check for optional marker.
    fn parse_bracket_with_optional(&mut self) -> Result<Expr, ParseError> {
        let expr = self.parse_bracket()?;
        self.skip_ws();
        if self.peek() == Some('?') {
            self.next();
            Ok(Expr::Optional(expr.into()))
        } else {
            Ok(expr)
        }
    }

    /// Parse a single term (field, bracket, or identity).
    fn parse_term(&mut self) -> Result<Expr, ParseError> {
        self.skip_ws();

        let expr = match self.peek() {
            Some('[') => return self.parse_bracket_with_optional(),
            Some(c) if c.is_alphabetic() || c == '_' => {
                let name = self.parse_ident()?;
                Expr::Field(name)
            }
            Some(c) => {
                return Err(ParseError::new(
                    format!("unexpected character '{}'", c),
                    self.pos,
                ));
            }
            None => {
                return Err(ParseError::new("unexpected end of input", self.pos));
            }
        };

        // Check for optional marker
        self.skip_ws();
        if self.peek() == Some('?') {
            self.next();
            Ok(Expr::Optional(expr.into()))
        } else {
            Ok(expr)
        }
    }

    /// Parse a complete expression.
    fn parse_expr(&mut self) -> Result<Expr, ParseError> {
        self.skip_ws();

        // Must start with '.'
        self.expect('.')?;
        self.skip_ws();

        // Check for just '.' (identity)
        if self.is_eof() || self.peek() == Some('|') {
            return Ok(Expr::Identity);
        }

        // Check for '[]' or '[n]' directly after '.'
        if self.peek() == Some('[') {
            let first = self.parse_bracket_with_optional()?;
            return self.parse_chain(first);
        }

        // Parse first term (field name)
        let first = self.parse_term()?;
        self.parse_chain(first)
    }

    /// Parse a chain of terms following the first one.
    fn parse_chain(&mut self, first: Expr) -> Result<Expr, ParseError> {
        let mut exprs = vec![first];

        loop {
            self.skip_ws();

            match self.peek() {
                Some('.') => {
                    self.next();
                    self.skip_ws();

                    // Check for bracket after dot
                    if self.peek() == Some('[') {
                        exprs.push(self.parse_bracket_with_optional()?);
                    } else {
                        exprs.push(self.parse_term()?);
                    }
                }
                Some('[') => {
                    exprs.push(self.parse_bracket_with_optional()?);
                }
                _ => break,
            }
        }

        Ok(Expr::pipe(exprs))
    }
}

/// Parse a jq expression string into an AST.
///
/// # Examples
///
/// ```
/// use succinctly::jq::parse;
///
/// // Identity
/// let expr = parse(".").unwrap();
///
/// // Field access
/// let expr = parse(".foo").unwrap();
///
/// // Chained access
/// let expr = parse(".foo.bar[0]").unwrap();
///
/// // Iteration
/// let expr = parse(".items[]").unwrap();
/// ```
pub fn parse(input: &str) -> Result<Expr, ParseError> {
    let mut parser = Parser::new(input);
    let expr = parser.parse_expr()?;

    // Ensure we consumed all input
    parser.skip_ws();
    if !parser.is_eof() {
        return Err(ParseError::new(
            format!("unexpected character '{}'", parser.peek().unwrap()),
            parser.pos,
        ));
    }

    Ok(expr)
}

#[cfg(test)]
mod tests {
    use super::{Expr, parse};

    #[test]
    fn test_identity() {
        assert_eq!(parse(".").unwrap(), Expr::Identity);
        assert_eq!(parse(" . ").unwrap(), Expr::Identity);
    }

    #[test]
    fn test_field_access() {
        assert_eq!(parse(".foo").unwrap(), Expr::Field("foo".into()));
        assert_eq!(parse(".foo_bar").unwrap(), Expr::Field("foo_bar".into()));
        assert_eq!(parse(".foo123").unwrap(), Expr::Field("foo123".into()));
        assert_eq!(parse("._private").unwrap(), Expr::Field("_private".into()));
    }

    #[test]
    fn test_index() {
        assert_eq!(parse(".[0]").unwrap(), Expr::Index(0));
        assert_eq!(parse(".[42]").unwrap(), Expr::Index(42));
        assert_eq!(parse(".[-1]").unwrap(), Expr::Index(-1));
        assert_eq!(parse(".[ 0 ]").unwrap(), Expr::Index(0));
    }

    #[test]
    fn test_iterate() {
        assert_eq!(parse(".[]").unwrap(), Expr::Iterate);
        assert_eq!(parse(".[ ]").unwrap(), Expr::Iterate);
    }

    #[test]
    fn test_slice() {
        assert_eq!(
            parse(".[1:3]").unwrap(),
            Expr::Slice {
                start: Some(1),
                end: Some(3)
            }
        );
        assert_eq!(
            parse(".[1:]").unwrap(),
            Expr::Slice {
                start: Some(1),
                end: None
            }
        );
        assert_eq!(
            parse(".[:3]").unwrap(),
            Expr::Slice {
                start: None,
                end: Some(3)
            }
        );
    }

    #[test]
    fn test_optional() {
        assert_eq!(
            parse(".foo?").unwrap(),
            Expr::Optional(Box::new(Expr::Field("foo".into())))
        );
    }

    #[test]
    fn test_chained() {
        assert_eq!(
            parse(".foo.bar").unwrap(),
            Expr::Pipe(vec![Expr::Field("foo".into()), Expr::Field("bar".into()),])
        );

        assert_eq!(
            parse(".foo[0]").unwrap(),
            Expr::Pipe(vec![Expr::Field("foo".into()), Expr::Index(0),])
        );

        assert_eq!(
            parse(".foo.bar[0].baz").unwrap(),
            Expr::Pipe(vec![
                Expr::Field("foo".into()),
                Expr::Field("bar".into()),
                Expr::Index(0),
                Expr::Field("baz".into()),
            ])
        );

        assert_eq!(
            parse(".users[].name").unwrap(),
            Expr::Pipe(vec![
                Expr::Field("users".into()),
                Expr::Iterate,
                Expr::Field("name".into()),
            ])
        );
    }

    #[test]
    fn test_errors() {
        assert!(parse("").is_err());
        assert!(parse("foo").is_err()); // missing leading dot
        assert!(parse(".[").is_err()); // unclosed bracket
        assert!(parse(".[abc]").is_err()); // invalid index
        assert!(parse(".123").is_err()); // field starting with number
    }
}
