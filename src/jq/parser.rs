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
//! - `.foo, .bar` - comma (multiple outputs)
//! - `[.foo, .bar]` - array construction
//! - `{foo: .bar}` - object construction
//! - `(.foo)` - parentheses for grouping
//! - `..` - recursive descent
//! - `null`, `true`, `false` - literals
//! - `"string"` - string literals
//! - `123`, `3.14` - number literals

#[cfg(not(test))]
use alloc::boxed::Box;
#[cfg(not(test))]
use alloc::format;
#[cfg(not(test))]
use alloc::string::{String, ToString};
#[cfg(not(test))]
use alloc::vec;
#[cfg(not(test))]
use alloc::vec::Vec;

use super::expr::{ArithOp, Builtin, CompareOp, Expr, Literal, ObjectEntry, ObjectKey};

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

    /// Peek at the next n characters.
    fn peek_str(&self, n: usize) -> &str {
        let end = (self.pos + n).min(self.input.len());
        &self.input[self.pos..end]
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

    /// Check if current position matches a keyword (followed by non-ident char).
    fn matches_keyword(&self, keyword: &str) -> bool {
        if !self.input[self.pos..].starts_with(keyword) {
            return false;
        }
        // Check that keyword is not followed by alphanumeric or underscore
        let after = self.pos + keyword.len();
        if after >= self.input.len() {
            return true;
        }
        let next_char = self.input[after..].chars().next();
        !matches!(next_char, Some(c) if c.is_alphanumeric() || c == '_')
    }

    /// Consume a keyword.
    fn consume_keyword(&mut self, keyword: &str) {
        self.pos += keyword.len();
    }

    /// Parse an identifier (field name or keyword).
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

    /// Parse a number literal (integer or float).
    fn parse_number_literal(&mut self) -> Result<Literal, ParseError> {
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

        // Check for decimal point
        let mut is_float = false;
        if self.peek() == Some('.') {
            // Look ahead to ensure it's not `..` (recursive descent)
            if self.peek_str(2) != ".." {
                self.next(); // consume the dot
                is_float = true;

                // Consume fractional digits
                while let Some(c) = self.peek() {
                    if c.is_ascii_digit() {
                        self.next();
                    } else {
                        break;
                    }
                }
            }
        }

        // Check for exponent
        if matches!(self.peek(), Some('e') | Some('E')) {
            self.next();
            is_float = true;

            // Optional sign
            if matches!(self.peek(), Some('+') | Some('-')) {
                self.next();
            }

            // Exponent digits
            while let Some(c) = self.peek() {
                if c.is_ascii_digit() {
                    self.next();
                } else {
                    break;
                }
            }
        }

        let num_str = &self.input[start..self.pos];
        if is_float {
            num_str
                .parse::<f64>()
                .map(Literal::Float)
                .map_err(|_| ParseError::new("invalid number", start))
        } else {
            num_str
                .parse::<i64>()
                .map(Literal::Int)
                .map_err(|_| ParseError::new("invalid number", start))
        }
    }

    /// Parse an integer (for array indices).
    fn parse_integer(&mut self) -> Result<i64, ParseError> {
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

    /// Parse a string literal.
    fn parse_string_literal(&mut self) -> Result<String, ParseError> {
        self.expect('"')?;
        let mut result = String::new();

        loop {
            match self.peek() {
                None => {
                    return Err(ParseError::new("unterminated string", self.pos));
                }
                Some('"') => {
                    self.next();
                    break;
                }
                Some('\\') => {
                    self.next();
                    match self.peek() {
                        Some('"') => {
                            self.next();
                            result.push('"');
                        }
                        Some('\\') => {
                            self.next();
                            result.push('\\');
                        }
                        Some('/') => {
                            self.next();
                            result.push('/');
                        }
                        Some('n') => {
                            self.next();
                            result.push('\n');
                        }
                        Some('r') => {
                            self.next();
                            result.push('\r');
                        }
                        Some('t') => {
                            self.next();
                            result.push('\t');
                        }
                        Some('b') => {
                            self.next();
                            result.push('\x08');
                        }
                        Some('f') => {
                            self.next();
                            result.push('\x0C');
                        }
                        Some('u') => {
                            self.next();
                            // Parse 4 hex digits
                            let mut hex = String::new();
                            for _ in 0..4 {
                                match self.peek() {
                                    Some(c) if c.is_ascii_hexdigit() => {
                                        hex.push(c);
                                        self.next();
                                    }
                                    _ => {
                                        return Err(ParseError::new(
                                            "invalid unicode escape",
                                            self.pos,
                                        ));
                                    }
                                }
                            }
                            let code = u32::from_str_radix(&hex, 16)
                                .map_err(|_| ParseError::new("invalid unicode escape", self.pos))?;
                            let c = char::from_u32(code).ok_or_else(|| {
                                ParseError::new("invalid unicode code point", self.pos)
                            })?;
                            result.push(c);
                        }
                        Some(c) => {
                            return Err(ParseError::new(
                                format!("invalid escape sequence '\\{}'", c),
                                self.pos,
                            ));
                        }
                        None => {
                            return Err(ParseError::new("unterminated string", self.pos));
                        }
                    }
                }
                Some(c) => {
                    self.next();
                    result.push(c);
                }
            }
        }

        Ok(result)
    }

    /// Parse a bracket expression: `[0]`, `[]`, `[1:3]`, etc.
    /// This is for indexing, NOT array construction.
    fn parse_index_bracket(&mut self) -> Result<Expr, ParseError> {
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
            let end = self.parse_integer()?;
            self.skip_ws();
            self.expect(']')?;
            return Ok(Expr::Slice {
                start: None,
                end: Some(end),
            });
        }

        // Parse first number
        let first = self.parse_integer()?;
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
                    let second = self.parse_integer()?;
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

    /// Parse an index bracket and check for optional marker.
    fn parse_index_bracket_with_optional(&mut self) -> Result<Expr, ParseError> {
        let expr = self.parse_index_bracket()?;
        self.skip_ws();
        if self.peek() == Some('?') {
            self.next();
            Ok(Expr::Optional(expr.into()))
        } else {
            Ok(expr)
        }
    }

    /// Parse array construction: `[expr]` or `[expr, expr, ...]`
    fn parse_array_construction(&mut self) -> Result<Expr, ParseError> {
        self.expect('[')?;
        self.skip_ws();

        // Empty array
        if self.peek() == Some(']') {
            self.next();
            // Empty array is constructed from identity with no iteration
            return Ok(Expr::Array(Box::new(Expr::Comma(vec![]))));
        }

        // Parse the inner expression (which may be a comma expression)
        let inner = self.parse_comma_expr()?;
        self.skip_ws();
        self.expect(']')?;

        Ok(Expr::Array(Box::new(inner)))
    }

    /// Parse object construction: `{key: value, ...}`
    fn parse_object_construction(&mut self) -> Result<Expr, ParseError> {
        self.expect('{')?;
        self.skip_ws();

        let mut entries = Vec::new();

        // Empty object
        if self.peek() == Some('}') {
            self.next();
            return Ok(Expr::Object(entries));
        }

        loop {
            self.skip_ws();

            // Parse key
            let key = if self.peek() == Some('(') {
                // Dynamic key: (expr)
                self.next();
                let key_expr = self.parse_comma_expr()?;
                self.expect(')')?;
                ObjectKey::Expr(Box::new(key_expr))
            } else if self.peek() == Some('"') {
                // String key
                let s = self.parse_string_literal()?;
                ObjectKey::Literal(s)
            } else {
                // Identifier key
                let name = self.parse_ident()?;
                ObjectKey::Literal(name)
            };

            self.skip_ws();

            // Check for shorthand: `{foo}` means `{foo: .foo}`
            let value = if self.peek() == Some(':') {
                self.next();
                self.skip_ws();
                self.parse_pipe_expr()?
            } else {
                // Shorthand: key must be literal identifier
                match &key {
                    ObjectKey::Literal(name) => Expr::Field(name.clone()),
                    ObjectKey::Expr(_) => {
                        return Err(ParseError::new(
                            "dynamic key requires explicit value",
                            self.pos,
                        ));
                    }
                }
            };

            entries.push(ObjectEntry { key, value });

            self.skip_ws();
            match self.peek() {
                Some(',') => {
                    self.next();
                    continue;
                }
                Some('}') => {
                    self.next();
                    break;
                }
                Some(c) => {
                    return Err(ParseError::new(
                        format!("expected ',' or '}}', found '{}'", c),
                        self.pos,
                    ));
                }
                None => {
                    return Err(ParseError::new(
                        "expected ',' or '}', found end of input",
                        self.pos,
                    ));
                }
            }
        }

        Ok(Expr::Object(entries))
    }

    /// Parse a primary expression (atoms and parenthesized expressions).
    fn parse_primary(&mut self) -> Result<Expr, ParseError> {
        self.skip_ws();

        match self.peek() {
            // Parenthesized expression
            Some('(') => {
                self.next();
                let expr = self.parse_comma_expr()?;
                self.expect(')')?;
                let paren = Expr::Paren(Box::new(expr));
                // Check for postfix operations after parentheses
                self.parse_postfix(paren)
            }

            // Array construction
            Some('[') => self.parse_array_construction(),

            // Object construction
            Some('{') => self.parse_object_construction(),

            // String literal
            Some('"') => {
                let s = self.parse_string_literal()?;
                Ok(Expr::Literal(Literal::String(s)))
            }

            // Number literal (starts with digit or negative sign followed by digit)
            Some(c) if c.is_ascii_digit() => {
                let lit = self.parse_number_literal()?;
                Ok(Expr::Literal(lit))
            }
            Some('-')
                if self
                    .peek_str(2)
                    .chars()
                    .nth(1)
                    .is_some_and(|c| c.is_ascii_digit()) =>
            {
                let lit = self.parse_number_literal()?;
                Ok(Expr::Literal(lit))
            }

            // Dot-based expressions
            Some('.') => {
                self.next();
                self.skip_ws();

                // Check for `..` (recursive descent)
                if self.peek() == Some('.') {
                    self.next();
                    return Ok(Expr::RecursiveDescent);
                }

                // Check for `.[...]` (index/iterate)
                if self.peek() == Some('[') {
                    let first = self.parse_index_bracket_with_optional()?;
                    return self.parse_postfix(first);
                }

                // Check for identity (just `.`)
                if self.is_eof() || self.is_expr_terminator() {
                    return Ok(Expr::Identity);
                }

                // Field access `.foo`
                let name = self.parse_ident()?;
                let mut expr = Expr::Field(name);

                // Check for optional
                self.skip_ws();
                if self.peek() == Some('?') {
                    self.next();
                    expr = Expr::Optional(Box::new(expr));
                }

                self.parse_postfix(expr)
            }

            // Keywords: null, true, false, not, if, try, error, builtins
            Some(c) if c.is_alphabetic() => {
                if self.matches_keyword("null") {
                    self.consume_keyword("null");
                    Ok(Expr::Literal(Literal::Null))
                } else if self.matches_keyword("true") {
                    self.consume_keyword("true");
                    Ok(Expr::Literal(Literal::Bool(true)))
                } else if self.matches_keyword("false") {
                    self.consume_keyword("false");
                    Ok(Expr::Literal(Literal::Bool(false)))
                } else if self.matches_keyword("not") {
                    self.consume_keyword("not");
                    Ok(Expr::Not)
                } else if self.matches_keyword("if") {
                    self.parse_if_expr()
                } else if self.matches_keyword("try") {
                    self.parse_try_expr()
                } else if self.matches_keyword("error") {
                    self.parse_error_expr()
                } else if let Some(builtin) = self.try_parse_builtin()? {
                    Ok(Expr::Builtin(builtin))
                } else {
                    Err(ParseError::new(
                        "unexpected identifier, expected expression",
                        self.pos,
                    ))
                }
            }

            Some(c) => Err(ParseError::new(
                format!("unexpected character '{}', expected expression", c),
                self.pos,
            )),
            None => Err(ParseError::new("unexpected end of input", self.pos)),
        }
    }

    /// Parse an if-then-else expression.
    /// Syntax: if COND then THEN elif COND then THEN else ELSE end
    fn parse_if_expr(&mut self) -> Result<Expr, ParseError> {
        self.consume_keyword("if");
        self.skip_ws();

        // Parse condition
        let cond = self.parse_pipe_expr()?;
        self.skip_ws();

        // Expect 'then'
        if !self.matches_keyword("then") {
            return Err(ParseError::new("expected 'then'", self.pos));
        }
        self.consume_keyword("then");
        self.skip_ws();

        // Parse then branch
        let then_branch = self.parse_pipe_expr()?;
        self.skip_ws();

        // Parse elif/else/end
        let else_branch = self.parse_else_branch()?;

        Ok(Expr::If {
            cond: Box::new(cond),
            then_branch: Box::new(then_branch),
            else_branch: Box::new(else_branch),
        })
    }

    /// Parse the else branch of an if expression (handles elif chaining).
    fn parse_else_branch(&mut self) -> Result<Expr, ParseError> {
        if self.matches_keyword("elif") {
            // elif is desugared to nested if
            self.consume_keyword("elif");
            self.skip_ws();

            let cond = self.parse_pipe_expr()?;
            self.skip_ws();

            if !self.matches_keyword("then") {
                return Err(ParseError::new("expected 'then'", self.pos));
            }
            self.consume_keyword("then");
            self.skip_ws();

            let then_branch = self.parse_pipe_expr()?;
            self.skip_ws();

            let else_branch = self.parse_else_branch()?;

            Ok(Expr::If {
                cond: Box::new(cond),
                then_branch: Box::new(then_branch),
                else_branch: Box::new(else_branch),
            })
        } else if self.matches_keyword("else") {
            self.consume_keyword("else");
            self.skip_ws();

            let else_branch = self.parse_pipe_expr()?;
            self.skip_ws();

            if !self.matches_keyword("end") {
                return Err(ParseError::new("expected 'end'", self.pos));
            }
            self.consume_keyword("end");

            Ok(else_branch)
        } else if self.matches_keyword("end") {
            // No else branch - default to null
            self.consume_keyword("end");
            Ok(Expr::Literal(Literal::Null))
        } else {
            Err(ParseError::new(
                "expected 'elif', 'else', or 'end'",
                self.pos,
            ))
        }
    }

    /// Parse a try-catch expression.
    /// Syntax: try EXPR catch HANDLER
    ///         try EXPR                 (catch is implicit, suppresses errors)
    fn parse_try_expr(&mut self) -> Result<Expr, ParseError> {
        self.consume_keyword("try");
        self.skip_ws();

        // Parse the expression to try
        let expr = self.parse_primary()?;
        self.skip_ws();

        // Check for optional catch
        let catch = if self.matches_keyword("catch") {
            self.consume_keyword("catch");
            self.skip_ws();
            Some(Box::new(self.parse_primary()?))
        } else {
            None
        };

        Ok(Expr::Try {
            expr: Box::new(expr),
            catch,
        })
    }

    /// Parse an error expression.
    /// Syntax: error
    ///         error(MESSAGE)
    fn parse_error_expr(&mut self) -> Result<Expr, ParseError> {
        self.consume_keyword("error");
        self.skip_ws();

        // Check for optional message in parentheses
        let msg = if self.peek() == Some('(') {
            self.next();
            self.skip_ws();
            let msg_expr = self.parse_pipe_expr()?;
            self.skip_ws();
            self.expect(')')?;
            Some(Box::new(msg_expr))
        } else {
            None
        };

        Ok(Expr::Error(msg))
    }

    /// Try to parse a builtin function.
    /// Returns Some(Builtin) if a builtin was parsed, None if not a builtin.
    fn try_parse_builtin(&mut self) -> Result<Option<Builtin>, ParseError> {
        // Type functions (no arguments)
        if self.matches_keyword("type") {
            self.consume_keyword("type");
            return Ok(Some(Builtin::Type));
        }
        if self.matches_keyword("isnull") {
            self.consume_keyword("isnull");
            return Ok(Some(Builtin::IsNull));
        }
        if self.matches_keyword("isboolean") {
            self.consume_keyword("isboolean");
            return Ok(Some(Builtin::IsBoolean));
        }
        if self.matches_keyword("isnumber") {
            self.consume_keyword("isnumber");
            return Ok(Some(Builtin::IsNumber));
        }
        if self.matches_keyword("isstring") {
            self.consume_keyword("isstring");
            return Ok(Some(Builtin::IsString));
        }
        if self.matches_keyword("isarray") {
            self.consume_keyword("isarray");
            return Ok(Some(Builtin::IsArray));
        }
        if self.matches_keyword("isobject") {
            self.consume_keyword("isobject");
            return Ok(Some(Builtin::IsObject));
        }

        // Length & keys (no arguments)
        if self.matches_keyword("length") {
            self.consume_keyword("length");
            return Ok(Some(Builtin::Length));
        }
        if self.matches_keyword("utf8bytelength") {
            self.consume_keyword("utf8bytelength");
            return Ok(Some(Builtin::Utf8ByteLength));
        }
        if self.matches_keyword("keys_unsorted") {
            // Check keys_unsorted before keys
            self.consume_keyword("keys_unsorted");
            return Ok(Some(Builtin::KeysUnsorted));
        }
        if self.matches_keyword("keys") {
            self.consume_keyword("keys");
            return Ok(Some(Builtin::Keys));
        }

        // has(expr) - takes an argument
        if self.matches_keyword("has") {
            self.consume_keyword("has");
            self.skip_ws();
            self.expect('(')?;
            self.skip_ws();
            let arg = self.parse_pipe_expr()?;
            self.skip_ws();
            self.expect(')')?;
            return Ok(Some(Builtin::Has(Box::new(arg))));
        }

        // Selection functions
        if self.matches_keyword("select") {
            self.consume_keyword("select");
            self.skip_ws();
            self.expect('(')?;
            self.skip_ws();
            let cond = self.parse_pipe_expr()?;
            self.skip_ws();
            self.expect(')')?;
            return Ok(Some(Builtin::Select(Box::new(cond))));
        }
        if self.matches_keyword("empty") {
            self.consume_keyword("empty");
            return Ok(Some(Builtin::Empty));
        }

        // Map functions
        if self.matches_keyword("map_values") {
            // Check map_values before map
            self.consume_keyword("map_values");
            self.skip_ws();
            self.expect('(')?;
            self.skip_ws();
            let f = self.parse_pipe_expr()?;
            self.skip_ws();
            self.expect(')')?;
            return Ok(Some(Builtin::MapValues(Box::new(f))));
        }
        if self.matches_keyword("map") {
            self.consume_keyword("map");
            self.skip_ws();
            self.expect('(')?;
            self.skip_ws();
            let f = self.parse_pipe_expr()?;
            self.skip_ws();
            self.expect(')')?;
            return Ok(Some(Builtin::Map(Box::new(f))));
        }

        // Reduction functions (no arguments)
        if self.matches_keyword("add") {
            self.consume_keyword("add");
            return Ok(Some(Builtin::Add));
        }
        if self.matches_keyword("any") {
            self.consume_keyword("any");
            return Ok(Some(Builtin::Any));
        }
        if self.matches_keyword("all") {
            self.consume_keyword("all");
            return Ok(Some(Builtin::All));
        }
        if self.matches_keyword("min_by") {
            // Check min_by before min
            self.consume_keyword("min_by");
            self.skip_ws();
            self.expect('(')?;
            self.skip_ws();
            let f = self.parse_pipe_expr()?;
            self.skip_ws();
            self.expect(')')?;
            return Ok(Some(Builtin::MinBy(Box::new(f))));
        }
        if self.matches_keyword("min") {
            self.consume_keyword("min");
            return Ok(Some(Builtin::Min));
        }
        if self.matches_keyword("max_by") {
            // Check max_by before max
            self.consume_keyword("max_by");
            self.skip_ws();
            self.expect('(')?;
            self.skip_ws();
            let f = self.parse_pipe_expr()?;
            self.skip_ws();
            self.expect(')')?;
            return Ok(Some(Builtin::MaxBy(Box::new(f))));
        }
        if self.matches_keyword("max") {
            self.consume_keyword("max");
            return Ok(Some(Builtin::Max));
        }

        // in(obj) - takes an argument (note: "in" is also sometimes used differently in jq)
        // We parse it with required parentheses
        if self.matches_keyword("in") {
            self.consume_keyword("in");
            self.skip_ws();
            self.expect('(')?;
            self.skip_ws();
            let obj = self.parse_pipe_expr()?;
            self.skip_ws();
            self.expect(')')?;
            return Ok(Some(Builtin::In(Box::new(obj))));
        }

        // Phase 5: String Functions
        if self.matches_keyword("ascii_downcase") {
            self.consume_keyword("ascii_downcase");
            return Ok(Some(Builtin::AsciiDowncase));
        }
        if self.matches_keyword("ascii_upcase") {
            self.consume_keyword("ascii_upcase");
            return Ok(Some(Builtin::AsciiUpcase));
        }
        if self.matches_keyword("ltrimstr") {
            self.consume_keyword("ltrimstr");
            self.skip_ws();
            self.expect('(')?;
            self.skip_ws();
            let s = self.parse_pipe_expr()?;
            self.skip_ws();
            self.expect(')')?;
            return Ok(Some(Builtin::Ltrimstr(Box::new(s))));
        }
        if self.matches_keyword("rtrimstr") {
            self.consume_keyword("rtrimstr");
            self.skip_ws();
            self.expect('(')?;
            self.skip_ws();
            let s = self.parse_pipe_expr()?;
            self.skip_ws();
            self.expect(')')?;
            return Ok(Some(Builtin::Rtrimstr(Box::new(s))));
        }
        if self.matches_keyword("startswith") {
            self.consume_keyword("startswith");
            self.skip_ws();
            self.expect('(')?;
            self.skip_ws();
            let s = self.parse_pipe_expr()?;
            self.skip_ws();
            self.expect(')')?;
            return Ok(Some(Builtin::Startswith(Box::new(s))));
        }
        if self.matches_keyword("endswith") {
            self.consume_keyword("endswith");
            self.skip_ws();
            self.expect('(')?;
            self.skip_ws();
            let s = self.parse_pipe_expr()?;
            self.skip_ws();
            self.expect(')')?;
            return Ok(Some(Builtin::Endswith(Box::new(s))));
        }
        if self.matches_keyword("split") {
            self.consume_keyword("split");
            self.skip_ws();
            self.expect('(')?;
            self.skip_ws();
            let s = self.parse_pipe_expr()?;
            self.skip_ws();
            self.expect(')')?;
            return Ok(Some(Builtin::Split(Box::new(s))));
        }
        if self.matches_keyword("join") {
            self.consume_keyword("join");
            self.skip_ws();
            self.expect('(')?;
            self.skip_ws();
            let s = self.parse_pipe_expr()?;
            self.skip_ws();
            self.expect(')')?;
            return Ok(Some(Builtin::Join(Box::new(s))));
        }
        if self.matches_keyword("contains") {
            self.consume_keyword("contains");
            self.skip_ws();
            self.expect('(')?;
            self.skip_ws();
            let b = self.parse_pipe_expr()?;
            self.skip_ws();
            self.expect(')')?;
            return Ok(Some(Builtin::Contains(Box::new(b))));
        }
        if self.matches_keyword("inside") {
            self.consume_keyword("inside");
            self.skip_ws();
            self.expect('(')?;
            self.skip_ws();
            let b = self.parse_pipe_expr()?;
            self.skip_ws();
            self.expect(')')?;
            return Ok(Some(Builtin::Inside(Box::new(b))));
        }

        // Phase 5: Array Functions
        if self.matches_keyword("first") {
            self.consume_keyword("first");
            return Ok(Some(Builtin::First));
        }
        if self.matches_keyword("last") {
            self.consume_keyword("last");
            return Ok(Some(Builtin::Last));
        }
        if self.matches_keyword("nth") {
            self.consume_keyword("nth");
            self.skip_ws();
            self.expect('(')?;
            self.skip_ws();
            let n = self.parse_pipe_expr()?;
            self.skip_ws();
            self.expect(')')?;
            return Ok(Some(Builtin::Nth(Box::new(n))));
        }
        if self.matches_keyword("reverse") {
            self.consume_keyword("reverse");
            return Ok(Some(Builtin::Reverse));
        }
        // Check flatten with depth before plain flatten
        if self.matches_keyword("flatten") {
            self.consume_keyword("flatten");
            self.skip_ws();
            if self.peek() == Some('(') {
                self.next();
                self.skip_ws();
                let depth = self.parse_pipe_expr()?;
                self.skip_ws();
                self.expect(')')?;
                return Ok(Some(Builtin::FlattenDepth(Box::new(depth))));
            }
            return Ok(Some(Builtin::Flatten));
        }
        if self.matches_keyword("group_by") {
            self.consume_keyword("group_by");
            self.skip_ws();
            self.expect('(')?;
            self.skip_ws();
            let f = self.parse_pipe_expr()?;
            self.skip_ws();
            self.expect(')')?;
            return Ok(Some(Builtin::GroupBy(Box::new(f))));
        }
        // Check unique_by before unique
        if self.matches_keyword("unique_by") {
            self.consume_keyword("unique_by");
            self.skip_ws();
            self.expect('(')?;
            self.skip_ws();
            let f = self.parse_pipe_expr()?;
            self.skip_ws();
            self.expect(')')?;
            return Ok(Some(Builtin::UniqueBy(Box::new(f))));
        }
        if self.matches_keyword("unique") {
            self.consume_keyword("unique");
            return Ok(Some(Builtin::Unique));
        }
        // Check sort_by before sort
        if self.matches_keyword("sort_by") {
            self.consume_keyword("sort_by");
            self.skip_ws();
            self.expect('(')?;
            self.skip_ws();
            let f = self.parse_pipe_expr()?;
            self.skip_ws();
            self.expect(')')?;
            return Ok(Some(Builtin::SortBy(Box::new(f))));
        }
        if self.matches_keyword("sort") {
            self.consume_keyword("sort");
            return Ok(Some(Builtin::Sort));
        }

        // Phase 5: Object Functions
        if self.matches_keyword("to_entries") {
            self.consume_keyword("to_entries");
            return Ok(Some(Builtin::ToEntries));
        }
        if self.matches_keyword("from_entries") {
            self.consume_keyword("from_entries");
            return Ok(Some(Builtin::FromEntries));
        }
        if self.matches_keyword("with_entries") {
            self.consume_keyword("with_entries");
            self.skip_ws();
            self.expect('(')?;
            self.skip_ws();
            let f = self.parse_pipe_expr()?;
            self.skip_ws();
            self.expect(')')?;
            return Ok(Some(Builtin::WithEntries(Box::new(f))));
        }

        Ok(None)
    }

    /// Check if current character is an expression terminator (ends a primary expression).
    /// This includes structural characters and infix operators.
    fn is_expr_terminator(&self) -> bool {
        match self.peek() {
            // Structural terminators
            Some(',') | Some(')') | Some(']') | Some('}') | Some('|') | Some(':') | Some(';') => {
                true
            }
            // Arithmetic operators
            Some('+') | Some('-') | Some('*') | Some('/') | Some('%') => true,
            // Comparison operators
            Some('=') | Some('!') | Some('<') | Some('>') => true,
            // Keywords that follow expressions
            Some('a') if self.matches_keyword("and") => true,
            Some('o') if self.matches_keyword("or") => true,
            // Conditional keywords
            Some('t') if self.matches_keyword("then") => true,
            Some('e')
                if self.matches_keyword("elif")
                    || self.matches_keyword("else")
                    || self.matches_keyword("end") =>
            {
                true
            }
            Some('c') if self.matches_keyword("catch") => true,
            _ => false,
        }
    }

    /// Parse postfix operations (field access, indexing) after a primary expression.
    fn parse_postfix(&mut self, mut expr: Expr) -> Result<Expr, ParseError> {
        let mut chain = vec![expr];

        loop {
            self.skip_ws();

            match self.peek() {
                Some('.') => {
                    self.next();
                    self.skip_ws();

                    // Check for bracket after dot
                    if self.peek() == Some('[') {
                        chain.push(self.parse_index_bracket_with_optional()?);
                    } else {
                        // Field access
                        let name = self.parse_ident()?;
                        let mut field_expr = Expr::Field(name);

                        // Check for optional
                        self.skip_ws();
                        if self.peek() == Some('?') {
                            self.next();
                            field_expr = Expr::Optional(Box::new(field_expr));
                        }

                        chain.push(field_expr);
                    }
                }
                Some('[') => {
                    chain.push(self.parse_index_bracket_with_optional()?);
                }
                _ => break,
            }
        }

        if chain.len() == 1 {
            expr = chain.pop().unwrap();
        } else {
            expr = Expr::Pipe(chain);
        }

        Ok(expr)
    }

    /// Parse multiplicative expressions: `expr * expr`, `expr / expr`, `expr % expr`
    fn parse_multiplicative(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_primary()?;

        loop {
            self.skip_ws();
            let op = match self.peek() {
                Some('*') => ArithOp::Mul,
                Some('/') => {
                    // Check it's not `//` (alternative operator)
                    if self.peek_str(2) == "//" {
                        break;
                    }
                    ArithOp::Div
                }
                Some('%') => ArithOp::Mod,
                _ => break,
            };
            self.next();
            self.skip_ws();
            let right = self.parse_primary()?;
            left = Expr::Arithmetic {
                op,
                left: Box::new(left),
                right: Box::new(right),
            };
        }

        Ok(left)
    }

    /// Parse additive expressions: `expr + expr`, `expr - expr`
    fn parse_additive(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_multiplicative()?;

        loop {
            self.skip_ws();
            let op = match self.peek() {
                Some('+') => ArithOp::Add,
                Some('-') => {
                    // Make sure it's not a negative number (handled in primary)
                    ArithOp::Sub
                }
                _ => break,
            };
            self.next();
            self.skip_ws();
            let right = self.parse_multiplicative()?;
            left = Expr::Arithmetic {
                op,
                left: Box::new(left),
                right: Box::new(right),
            };
        }

        Ok(left)
    }

    /// Parse comparison expressions: `==`, `!=`, `<`, `<=`, `>`, `>=`
    fn parse_comparison(&mut self) -> Result<Expr, ParseError> {
        let left = self.parse_additive()?;
        self.skip_ws();

        let op = match self.peek_str(2) {
            "==" => CompareOp::Eq,
            "!=" => CompareOp::Ne,
            "<=" => CompareOp::Le,
            ">=" => CompareOp::Ge,
            s if s.starts_with('<') => CompareOp::Lt,
            s if s.starts_with('>') => CompareOp::Gt,
            _ => return Ok(left),
        };

        // Consume the operator
        match op {
            CompareOp::Eq | CompareOp::Ne | CompareOp::Le | CompareOp::Ge => {
                self.next();
                self.next();
            }
            CompareOp::Lt | CompareOp::Gt => {
                self.next();
            }
        }

        self.skip_ws();
        let right = self.parse_additive()?;

        Ok(Expr::Compare {
            op,
            left: Box::new(left),
            right: Box::new(right),
        })
    }

    /// Parse `and` expressions: `expr and expr`
    fn parse_and(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_comparison()?;

        loop {
            self.skip_ws();
            if !self.matches_keyword("and") {
                break;
            }
            self.consume_keyword("and");
            self.skip_ws();
            let right = self.parse_comparison()?;
            left = Expr::And(Box::new(left), Box::new(right));
        }

        Ok(left)
    }

    /// Parse `or` expressions: `expr or expr`
    fn parse_or(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_and()?;

        loop {
            self.skip_ws();
            if !self.matches_keyword("or") {
                break;
            }
            self.consume_keyword("or");
            self.skip_ws();
            let right = self.parse_and()?;
            left = Expr::Or(Box::new(left), Box::new(right));
        }

        Ok(left)
    }

    /// Parse alternative expressions: `expr // expr`
    fn parse_alternative(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_or()?;

        loop {
            self.skip_ws();
            if self.peek_str(2) != "//" {
                break;
            }
            self.next();
            self.next();
            self.skip_ws();
            let right = self.parse_or()?;
            left = Expr::Alternative(Box::new(left), Box::new(right));
        }

        Ok(left)
    }

    /// Parse a pipe expression: `expr | expr | ...`
    fn parse_pipe_expr(&mut self) -> Result<Expr, ParseError> {
        let first = self.parse_alternative()?;
        self.skip_ws();

        if self.peek() != Some('|') {
            return Ok(first);
        }

        let mut exprs = vec![first];

        while self.peek() == Some('|') {
            self.next();
            self.skip_ws();
            exprs.push(self.parse_alternative()?);
            self.skip_ws();
        }

        Ok(Expr::pipe(exprs))
    }

    /// Parse a comma expression: `expr, expr, ...`
    /// This is the lowest precedence operator.
    fn parse_comma_expr(&mut self) -> Result<Expr, ParseError> {
        let first = self.parse_pipe_expr()?;
        self.skip_ws();

        if self.peek() != Some(',') {
            return Ok(first);
        }

        let mut exprs = vec![first];

        while self.peek() == Some(',') {
            self.next();
            self.skip_ws();
            exprs.push(self.parse_pipe_expr()?);
            self.skip_ws();
        }

        Ok(Expr::comma(exprs))
    }

    /// Parse a complete expression (entry point).
    fn parse_expr(&mut self) -> Result<Expr, ParseError> {
        self.parse_comma_expr()
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
///
/// // Comma (multiple outputs)
/// let expr = parse(".foo, .bar").unwrap();
///
/// // Array construction
/// let expr = parse("[.foo, .bar]").unwrap();
///
/// // Object construction
/// let expr = parse("{name: .name, age: .age}").unwrap();
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
    use super::*;

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
    fn test_comma() {
        assert_eq!(
            parse(".foo, .bar").unwrap(),
            Expr::Comma(vec![Expr::Field("foo".into()), Expr::Field("bar".into()),])
        );

        assert_eq!(
            parse(".a, .b, .c").unwrap(),
            Expr::Comma(vec![
                Expr::Field("a".into()),
                Expr::Field("b".into()),
                Expr::Field("c".into()),
            ])
        );
    }

    #[test]
    fn test_pipe_operator() {
        assert_eq!(
            parse(". | .foo").unwrap(),
            Expr::Pipe(vec![Expr::Identity, Expr::Field("foo".into()),])
        );
    }

    #[test]
    fn test_array_construction() {
        // Empty array
        let empty = parse("[]").unwrap();
        assert!(matches!(empty, Expr::Array(_)));

        // Array with elements
        let arr = parse("[.foo, .bar]").unwrap();
        match arr {
            Expr::Array(inner) => {
                assert!(matches!(*inner, Expr::Comma(_)));
            }
            _ => panic!("expected Array"),
        }

        // Array with iteration
        let iter_arr = parse("[.items[]]").unwrap();
        assert!(matches!(iter_arr, Expr::Array(_)));
    }

    #[test]
    fn test_object_construction() {
        // Empty object
        let empty = parse("{}").unwrap();
        assert_eq!(empty, Expr::Object(vec![]));

        // Object with entries
        let obj = parse("{name: .name, age: .age}").unwrap();
        match obj {
            Expr::Object(entries) => {
                assert_eq!(entries.len(), 2);
                assert_eq!(entries[0].key, ObjectKey::Literal("name".into()));
                assert_eq!(entries[1].key, ObjectKey::Literal("age".into()));
            }
            _ => panic!("expected Object"),
        }

        // Object shorthand
        let shorthand = parse("{foo, bar}").unwrap();
        match shorthand {
            Expr::Object(entries) => {
                assert_eq!(entries.len(), 2);
                assert_eq!(entries[0].key, ObjectKey::Literal("foo".into()));
                assert_eq!(entries[0].value, Expr::Field("foo".into()));
            }
            _ => panic!("expected Object"),
        }

        // Dynamic key
        let dynamic = parse("{(.key): .value}").unwrap();
        match dynamic {
            Expr::Object(entries) => {
                assert_eq!(entries.len(), 1);
                assert!(matches!(entries[0].key, ObjectKey::Expr(_)));
            }
            _ => panic!("expected Object"),
        }
    }

    #[test]
    fn test_literals() {
        assert_eq!(parse("null").unwrap(), Expr::Literal(Literal::Null));
        assert_eq!(parse("true").unwrap(), Expr::Literal(Literal::Bool(true)));
        assert_eq!(parse("false").unwrap(), Expr::Literal(Literal::Bool(false)));
        assert_eq!(parse("42").unwrap(), Expr::Literal(Literal::Int(42)));
        assert_eq!(parse("-123").unwrap(), Expr::Literal(Literal::Int(-123)));
        assert_eq!(parse("2.5").unwrap(), Expr::Literal(Literal::Float(2.5)));
        assert_eq!(
            parse("\"hello\"").unwrap(),
            Expr::Literal(Literal::String("hello".into()))
        );
        assert_eq!(
            parse("\"hello\\nworld\"").unwrap(),
            Expr::Literal(Literal::String("hello\nworld".into()))
        );
    }

    #[test]
    fn test_recursive_descent() {
        assert_eq!(parse("..").unwrap(), Expr::RecursiveDescent);
    }

    #[test]
    fn test_parentheses() {
        let paren = parse("(.foo)").unwrap();
        match paren {
            Expr::Paren(inner) => {
                assert_eq!(*inner, Expr::Field("foo".into()));
            }
            _ => panic!("expected Paren"),
        }

        // Nested parentheses
        let nested = parse("((.foo))").unwrap();
        assert!(matches!(nested, Expr::Paren(_)));
    }

    #[test]
    fn test_complex_expressions() {
        // Comma inside array
        let expr = parse("[.a, .b, .c]").unwrap();
        assert!(matches!(expr, Expr::Array(_)));

        // Pipe inside parentheses
        let expr = parse("(.foo | .bar)").unwrap();
        assert!(matches!(expr, Expr::Paren(_)));

        // Object with complex values
        let expr = parse("{items: [.a, .b]}").unwrap();
        assert!(matches!(expr, Expr::Object(_)));
    }

    #[test]
    fn test_errors() {
        assert!(parse("").is_err());
        assert!(parse("foo").is_err()); // missing leading dot for field access
        assert!(parse(".[").is_err()); // unclosed bracket
        assert!(parse(".[abc]").is_err()); // invalid index
        assert!(parse(".123").is_err()); // field starting with number
        assert!(parse("{").is_err()); // unclosed brace
        assert!(parse("[").is_err()); // unclosed bracket
        assert!(parse("\"unterminated").is_err()); // unterminated string
    }

    #[test]
    fn test_arithmetic() {
        // Addition
        let expr = parse(".a + .b").unwrap();
        assert!(matches!(
            expr,
            Expr::Arithmetic {
                op: ArithOp::Add,
                ..
            }
        ));

        // Subtraction
        let expr = parse(".a - .b").unwrap();
        assert!(matches!(
            expr,
            Expr::Arithmetic {
                op: ArithOp::Sub,
                ..
            }
        ));

        // Multiplication
        let expr = parse(".a * .b").unwrap();
        assert!(matches!(
            expr,
            Expr::Arithmetic {
                op: ArithOp::Mul,
                ..
            }
        ));

        // Division
        let expr = parse(".a / .b").unwrap();
        assert!(matches!(
            expr,
            Expr::Arithmetic {
                op: ArithOp::Div,
                ..
            }
        ));

        // Modulo
        let expr = parse(".a % .b").unwrap();
        assert!(matches!(
            expr,
            Expr::Arithmetic {
                op: ArithOp::Mod,
                ..
            }
        ));

        // Operator precedence: * before +
        let expr = parse(".a + .b * .c").unwrap();
        match expr {
            Expr::Arithmetic {
                op: ArithOp::Add,
                right,
                ..
            } => {
                assert!(matches!(
                    *right,
                    Expr::Arithmetic {
                        op: ArithOp::Mul,
                        ..
                    }
                ));
            }
            _ => panic!("expected Add with Mul on right"),
        }

        // Literals
        let expr = parse("1 + 2").unwrap();
        assert!(matches!(
            expr,
            Expr::Arithmetic {
                op: ArithOp::Add,
                ..
            }
        ));
    }

    #[test]
    fn test_comparison() {
        // Equality
        let expr = parse(".a == .b").unwrap();
        assert!(matches!(
            expr,
            Expr::Compare {
                op: CompareOp::Eq,
                ..
            }
        ));

        // Inequality
        let expr = parse(".a != .b").unwrap();
        assert!(matches!(
            expr,
            Expr::Compare {
                op: CompareOp::Ne,
                ..
            }
        ));

        // Less than
        let expr = parse(".a < .b").unwrap();
        assert!(matches!(
            expr,
            Expr::Compare {
                op: CompareOp::Lt,
                ..
            }
        ));

        // Less than or equal
        let expr = parse(".a <= .b").unwrap();
        assert!(matches!(
            expr,
            Expr::Compare {
                op: CompareOp::Le,
                ..
            }
        ));

        // Greater than
        let expr = parse(".a > .b").unwrap();
        assert!(matches!(
            expr,
            Expr::Compare {
                op: CompareOp::Gt,
                ..
            }
        ));

        // Greater than or equal
        let expr = parse(".a >= .b").unwrap();
        assert!(matches!(
            expr,
            Expr::Compare {
                op: CompareOp::Ge,
                ..
            }
        ));
    }

    #[test]
    fn test_boolean_operators() {
        // AND
        let expr = parse(".a and .b").unwrap();
        assert!(matches!(expr, Expr::And(_, _)));

        // OR
        let expr = parse(".a or .b").unwrap();
        assert!(matches!(expr, Expr::Or(_, _)));

        // NOT
        let expr = parse("not").unwrap();
        assert!(matches!(expr, Expr::Not));

        // Chained: and before or
        let expr = parse(".a or .b and .c").unwrap();
        match expr {
            Expr::Or(_, right) => {
                assert!(matches!(*right, Expr::And(_, _)));
            }
            _ => panic!("expected Or with And on right"),
        }
    }

    #[test]
    fn test_alternative() {
        let expr = parse(".foo // \"default\"").unwrap();
        assert!(matches!(expr, Expr::Alternative(_, _)));

        // Chained alternatives
        let expr = parse(".a // .b // .c").unwrap();
        match expr {
            Expr::Alternative(left, _) => {
                assert!(matches!(*left, Expr::Alternative(_, _)));
            }
            _ => panic!("expected nested Alternative"),
        }
    }

    #[test]
    fn test_mixed_operators() {
        // Complex expression: comparison in alternative
        let expr = parse(".a > 0 // false").unwrap();
        assert!(matches!(expr, Expr::Alternative(_, _)));

        // Pipe with operators
        let expr = parse(".a | . + 1").unwrap();
        assert!(matches!(expr, Expr::Pipe(_)));

        // Boolean with comparison
        let expr = parse(".a > 0 and .b < 10").unwrap();
        assert!(matches!(expr, Expr::And(_, _)));
    }

    // Phase 3 tests: Conditionals and Control Flow

    #[test]
    fn test_if_then_else() {
        // Basic if-then-else
        let expr = parse("if .a then .b else .c end").unwrap();
        assert!(matches!(expr, Expr::If { .. }));

        // If with complex condition
        let expr = parse("if .x > 0 then \"positive\" else \"non-positive\" end").unwrap();
        match expr {
            Expr::If { cond, .. } => {
                assert!(matches!(
                    *cond,
                    Expr::Compare {
                        op: CompareOp::Gt,
                        ..
                    }
                ));
            }
            _ => panic!("expected If"),
        }

        // Nested if
        let expr = parse("if .a then if .b then 1 else 2 end else 3 end").unwrap();
        match expr {
            Expr::If { then_branch, .. } => {
                assert!(matches!(*then_branch, Expr::If { .. }));
            }
            _ => panic!("expected If"),
        }
    }

    #[test]
    fn test_if_elif() {
        // if-elif-else
        let expr = parse("if .a then 1 elif .b then 2 else 3 end").unwrap();
        match expr {
            Expr::If { else_branch, .. } => {
                // elif is desugared to nested if
                assert!(matches!(*else_branch, Expr::If { .. }));
            }
            _ => panic!("expected If"),
        }

        // Multiple elif
        let expr = parse("if .a then 1 elif .b then 2 elif .c then 3 else 4 end").unwrap();
        match expr {
            Expr::If { else_branch, .. } => match *else_branch {
                Expr::If { else_branch, .. } => {
                    assert!(matches!(*else_branch, Expr::If { .. }));
                }
                _ => panic!("expected nested If"),
            },
            _ => panic!("expected If"),
        }
    }

    #[test]
    fn test_if_no_else() {
        // if without else should default to null
        let expr = parse("if .a then .b end").unwrap();
        match expr {
            Expr::If { else_branch, .. } => {
                assert!(matches!(*else_branch, Expr::Literal(Literal::Null)));
            }
            _ => panic!("expected If"),
        }
    }

    #[test]
    fn test_try_catch() {
        // try with catch
        let expr = parse("try .foo catch \"default\"").unwrap();
        match expr {
            Expr::Try { catch, .. } => {
                assert!(catch.is_some());
            }
            _ => panic!("expected Try"),
        }

        // try without catch
        let expr = parse("try .foo").unwrap();
        match expr {
            Expr::Try { catch, .. } => {
                assert!(catch.is_none());
            }
            _ => panic!("expected Try"),
        }

        // try with complex expression
        let expr = parse("try .missing? catch null").unwrap();
        assert!(matches!(expr, Expr::Try { .. }));
    }

    #[test]
    fn test_error() {
        // error without message
        let expr = parse("error").unwrap();
        match expr {
            Expr::Error(msg) => {
                assert!(msg.is_none());
            }
            _ => panic!("expected Error"),
        }

        // error with message
        let expr = parse("error(\"something went wrong\")").unwrap();
        match expr {
            Expr::Error(msg) => {
                assert!(msg.is_some());
            }
            _ => panic!("expected Error"),
        }

        // error with expression message
        let expr = parse("error(.message)").unwrap();
        match expr {
            Expr::Error(msg) => {
                assert!(msg.is_some());
            }
            _ => panic!("expected Error"),
        }
    }

    #[test]
    fn test_control_flow_in_expressions() {
        // if in array construction
        let expr = parse("[if .a then 1 else 2 end]").unwrap();
        assert!(matches!(expr, Expr::Array(_)));

        // try in pipe
        let expr = parse(".foo | try . catch null").unwrap();
        assert!(matches!(expr, Expr::Pipe(_)));

        // if with arithmetic
        let expr = parse("if .x > 0 then .x * 2 else .x end").unwrap();
        assert!(matches!(expr, Expr::If { .. }));
    }
}
