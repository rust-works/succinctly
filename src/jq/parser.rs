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

#[cfg(not(test))]
use alloc::collections::BTreeMap;
#[cfg(test)]
use std::collections::BTreeMap;

use super::expr::{
    ArithOp, AssignOp, Builtin, CompareOp, Expr, FormatType, Import, Include, Literal, MetaValue,
    ModuleMeta, ObjectEntry, ObjectKey, Pattern, PatternEntry, Program, StringPart,
};

/// Parser mode controls syntax differences between jq and yq.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ParserMode {
    /// Standard jq mode: `-` is subtraction, identifiers are alphanumeric + underscore
    #[default]
    Jq,
    /// yq mode: `-` allowed in identifiers (e.g., `.my-key`), matches yq behavior
    Yq,
}

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
    mode: ParserMode,
}

impl<'a> Parser<'a> {
    #[allow(dead_code)] // kept for tests and future use
    fn new(input: &'a str) -> Self {
        Parser {
            input,
            pos: 0,
            mode: ParserMode::Jq,
        }
    }

    fn with_mode(input: &'a str, mode: ParserMode) -> Self {
        Parser {
            input,
            pos: 0,
            mode,
        }
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

    /// Skip whitespace and comments.
    /// Comments start with `#` and run to the end of the line.
    fn skip_ws(&mut self) {
        loop {
            match self.peek() {
                Some(c) if c.is_whitespace() => {
                    self.next();
                }
                Some('#') => {
                    // Skip comment until end of line
                    self.next(); // consume '#'
                    while let Some(c) = self.peek() {
                        self.next();
                        if c == '\n' {
                            break;
                        }
                    }
                }
                _ => break,
            }
        }
    }

    /// Check if we're at the end of input.
    fn is_eof(&self) -> bool {
        self.pos >= self.input.len()
    }

    /// Get the 1-based line number at the current position.
    fn current_line(&self) -> usize {
        1 + self.input[..self.pos]
            .bytes()
            .filter(|&b| b == b'\n')
            .count()
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

    /// Check if after consuming a keyword, the next non-whitespace character is '('.
    /// This is used to determine if a no-arg builtin should be parsed as a builtin
    /// or as a user-defined function call.
    fn peek_after_keyword_is_paren(&self, keyword: &str) -> bool {
        let after = self.pos + keyword.len();
        // Skip whitespace after keyword
        let rest = &self.input[after..];
        for c in rest.chars() {
            if c.is_whitespace() {
                continue;
            }
            return c == '(';
        }
        false
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
        // In Yq mode, also allow hyphens (but not at the end)
        while let Some(c) = self.peek() {
            if c.is_alphanumeric() || c == '_' {
                self.next();
            } else if self.mode == ParserMode::Yq && c == '-' {
                // In Yq mode, allow hyphen if followed by valid ident char
                // Peek ahead to check if there's a valid continuation
                let remaining = &self.input[self.pos + 1..];
                if let Some(next_c) = remaining.chars().next() {
                    if next_c.is_alphanumeric() || next_c == '_' {
                        self.next(); // consume the hyphen
                    } else {
                        break;
                    }
                } else {
                    break;
                }
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

    /// Parse a string literal or string interpolation.
    /// Returns either a simple string literal or a StringInterpolation expression.
    fn parse_string_or_interpolation(&mut self) -> Result<Expr, ParseError> {
        self.expect('"')?;
        let mut parts: Vec<StringPart> = Vec::new();
        let mut current_literal = String::new();

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
                        // String interpolation: \(expr)
                        Some('(') => {
                            self.next();
                            // Save current literal if any
                            if !current_literal.is_empty() {
                                parts.push(StringPart::Literal(core::mem::take(
                                    &mut current_literal,
                                )));
                            }
                            // Parse the expression inside \(...)
                            let expr = self.parse_pipe_expr()?;
                            self.skip_ws();
                            self.expect(')')?;
                            parts.push(StringPart::Expr(Box::new(expr)));
                        }
                        Some('"') => {
                            self.next();
                            current_literal.push('"');
                        }
                        Some('\\') => {
                            self.next();
                            current_literal.push('\\');
                        }
                        Some('/') => {
                            self.next();
                            current_literal.push('/');
                        }
                        Some('n') => {
                            self.next();
                            current_literal.push('\n');
                        }
                        Some('r') => {
                            self.next();
                            current_literal.push('\r');
                        }
                        Some('t') => {
                            self.next();
                            current_literal.push('\t');
                        }
                        Some('b') => {
                            self.next();
                            current_literal.push('\x08');
                        }
                        Some('f') => {
                            self.next();
                            current_literal.push('\x0C');
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
                            current_literal.push(c);
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
                    current_literal.push(c);
                }
            }
        }

        // If no interpolations, return a simple string literal
        if parts.is_empty() {
            return Ok(Expr::Literal(Literal::String(current_literal)));
        }

        // Add final literal if any
        if !current_literal.is_empty() {
            parts.push(StringPart::Literal(current_literal));
        }

        Ok(Expr::StringInterpolation(parts))
    }

    /// Parse a simple string literal (for object keys, etc.)
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

    /// Parse a bracket expression: `[0]`, `[]`, `[1:3]`, `["key"]`, etc.
    /// This is for indexing, NOT array construction.
    fn parse_index_bracket(&mut self) -> Result<Expr, ParseError> {
        self.expect('[')?;
        self.skip_ws();

        // Empty brackets = iterate
        if self.peek() == Some(']') {
            self.next();
            return Ok(Expr::Iterate);
        }

        // Check for string key: `["key"]`
        if self.peek() == Some('"') {
            let key = self.parse_string_literal()?;
            self.skip_ws();
            self.expect(']')?;
            return Ok(Expr::Field(key));
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

            // String literal or interpolation
            Some('"') => self.parse_string_or_interpolation(),

            // Format strings: @text, @json, @uri, etc.
            Some('@') => self.parse_format_string(),

            // Number literal (starts with digit)
            Some(c) if c.is_ascii_digit() => {
                let lit = self.parse_number_literal()?;
                Ok(Expr::Literal(lit))
            }

            // Unary minus: either a negative number literal or negation of an expression
            Some('-') => {
                // Check if this is a negative number literal (- followed by digit)
                if self
                    .peek_str(2)
                    .chars()
                    .nth(1)
                    .is_some_and(|c| c.is_ascii_digit())
                {
                    let lit = self.parse_number_literal()?;
                    Ok(Expr::Literal(lit))
                } else {
                    // Unary minus: negate the following expression
                    // Implemented as (0 - expr)
                    self.next(); // consume '-'
                    let operand = self.parse_primary()?;
                    Ok(Expr::Arithmetic {
                        op: ArithOp::Sub,
                        left: Box::new(Expr::Literal(Literal::Int(0))),
                        right: Box::new(operand),
                    })
                }
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

                // Check for quoted field access `."key"`
                let mut expr = if self.peek() == Some('"') {
                    let name = self.parse_string_literal()?;
                    Expr::Field(name)
                } else {
                    // Field access `.foo`
                    let name = self.parse_ident()?;
                    Expr::Field(name)
                };

                // Check for optional
                self.skip_ws();
                if self.peek() == Some('?') {
                    self.next();
                    expr = Expr::Optional(Box::new(expr));
                }

                self.parse_postfix(expr)
            }

            // Variable reference: $varname, $__loc__, or $ENV
            Some('$') => {
                let line = self.current_line();
                self.next();
                let name = self.parse_ident()?;
                let expr = if name == "__loc__" {
                    Expr::Loc { line }
                } else if name == "ENV" {
                    Expr::Env
                } else {
                    Expr::Var(name)
                };
                self.parse_postfix(expr)
            }

            // Keywords: null, true, false, not, if, try, error, reduce, foreach, etc.
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
                } else if self.matches_keyword("reduce") {
                    self.parse_reduce_expr()
                } else if self.matches_keyword("foreach") {
                    self.parse_foreach_expr()
                } else if self.matches_keyword("limit") {
                    self.parse_limit_expr()
                } else if self.matches_keyword("until") {
                    self.parse_until_expr()
                } else if self.matches_keyword("while") {
                    self.parse_while_expr()
                } else if self.matches_keyword("repeat") {
                    self.parse_repeat_expr()
                } else if self.matches_keyword("range") {
                    self.parse_range_expr()
                } else if self.matches_keyword("first") {
                    self.parse_first_expr()
                } else if self.matches_keyword("last") {
                    self.parse_last_expr()
                } else if self.matches_keyword("def") {
                    // Phase 9: Function definition
                    self.parse_def_expr()
                } else if self.matches_keyword("label") {
                    self.parse_label_expr()
                } else if self.matches_keyword("break") {
                    self.parse_break_expr()
                } else if let Some(builtin) = self.try_parse_builtin()? {
                    // Allow postfix operations after builtins (e.g., env.PATH, keys[0])
                    self.parse_postfix(Expr::Builtin(builtin))
                } else {
                    // Phase 9: Try to parse as function call
                    self.parse_func_call_or_error()
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

    /// Parse a reduce expression.
    /// Syntax: reduce EXPR as $VAR (INIT; UPDATE)
    fn parse_reduce_expr(&mut self) -> Result<Expr, ParseError> {
        self.consume_keyword("reduce");
        self.skip_ws();

        // Parse input expression - use parse_alternative to stop before 'as'
        let input = self.parse_alternative()?;
        self.skip_ws();

        // Expect 'as'
        if !self.matches_keyword("as") {
            return Err(ParseError::new("expected 'as'", self.pos));
        }
        self.consume_keyword("as");
        self.skip_ws();

        // Parse variable name (with $)
        self.expect('$')?;
        let var = self.parse_ident()?;
        self.skip_ws();

        // Parse (init; update)
        self.expect('(')?;
        self.skip_ws();
        let init = self.parse_pipe_expr()?;
        self.skip_ws();
        self.expect(';')?;
        self.skip_ws();
        let update = self.parse_pipe_expr()?;
        self.skip_ws();
        self.expect(')')?;

        Ok(Expr::Reduce {
            input: Box::new(input),
            var,
            init: Box::new(init),
            update: Box::new(update),
        })
    }

    /// Parse a foreach expression.
    /// Syntax: foreach EXPR as $VAR (INIT; UPDATE) or foreach EXPR as $VAR (INIT; UPDATE; EXTRACT)
    fn parse_foreach_expr(&mut self) -> Result<Expr, ParseError> {
        self.consume_keyword("foreach");
        self.skip_ws();

        // Parse input expression - use parse_alternative to stop before 'as'
        let input = self.parse_alternative()?;
        self.skip_ws();

        // Expect 'as'
        if !self.matches_keyword("as") {
            return Err(ParseError::new("expected 'as'", self.pos));
        }
        self.consume_keyword("as");
        self.skip_ws();

        // Parse variable name (with $)
        self.expect('$')?;
        let var = self.parse_ident()?;
        self.skip_ws();

        // Parse (init; update[; extract])
        self.expect('(')?;
        self.skip_ws();
        let init = self.parse_pipe_expr()?;
        self.skip_ws();
        self.expect(';')?;
        self.skip_ws();
        let update = self.parse_pipe_expr()?;
        self.skip_ws();

        // Optional extract expression
        let extract = if self.peek() == Some(';') {
            self.next();
            self.skip_ws();
            Some(Box::new(self.parse_pipe_expr()?))
        } else {
            None
        };
        self.skip_ws();
        self.expect(')')?;

        Ok(Expr::Foreach {
            input: Box::new(input),
            var,
            init: Box::new(init),
            update: Box::new(update),
            extract,
        })
    }

    /// Parse a limit expression.
    /// Syntax: limit(N; EXPR)
    fn parse_limit_expr(&mut self) -> Result<Expr, ParseError> {
        self.consume_keyword("limit");
        self.skip_ws();
        self.expect('(')?;
        self.skip_ws();
        let n = self.parse_pipe_expr()?;
        self.skip_ws();
        self.expect(';')?;
        self.skip_ws();
        let expr = self.parse_pipe_expr()?;
        self.skip_ws();
        self.expect(')')?;

        Ok(Expr::Limit {
            n: Box::new(n),
            expr: Box::new(expr),
        })
    }

    /// Parse an until expression.
    /// Syntax: until(COND; UPDATE)
    fn parse_until_expr(&mut self) -> Result<Expr, ParseError> {
        self.consume_keyword("until");
        self.skip_ws();
        self.expect('(')?;
        self.skip_ws();
        let cond = self.parse_pipe_expr()?;
        self.skip_ws();
        self.expect(';')?;
        self.skip_ws();
        let update = self.parse_pipe_expr()?;
        self.skip_ws();
        self.expect(')')?;

        Ok(Expr::Until {
            cond: Box::new(cond),
            update: Box::new(update),
        })
    }

    /// Parse a while expression.
    /// Syntax: while(COND; UPDATE)
    fn parse_while_expr(&mut self) -> Result<Expr, ParseError> {
        self.consume_keyword("while");
        self.skip_ws();
        self.expect('(')?;
        self.skip_ws();
        let cond = self.parse_pipe_expr()?;
        self.skip_ws();
        self.expect(';')?;
        self.skip_ws();
        let update = self.parse_pipe_expr()?;
        self.skip_ws();
        self.expect(')')?;

        Ok(Expr::While {
            cond: Box::new(cond),
            update: Box::new(update),
        })
    }

    /// Parse a repeat expression.
    /// Syntax: repeat(EXPR)
    fn parse_repeat_expr(&mut self) -> Result<Expr, ParseError> {
        self.consume_keyword("repeat");
        self.skip_ws();
        self.expect('(')?;
        self.skip_ws();
        let expr = self.parse_pipe_expr()?;
        self.skip_ws();
        self.expect(')')?;

        Ok(Expr::Repeat(Box::new(expr)))
    }

    /// Parse a first expression.
    /// Syntax: first or first(expr)
    fn parse_first_expr(&mut self) -> Result<Expr, ParseError> {
        self.consume_keyword("first");
        self.skip_ws();
        if self.peek() == Some('(') {
            self.next();
            self.skip_ws();
            let expr = self.parse_pipe_expr()?;
            self.skip_ws();
            self.expect(')')?;
            Ok(Expr::FirstExpr(Box::new(expr)))
        } else {
            Ok(Expr::Builtin(Builtin::First))
        }
    }

    /// Parse a last expression.
    /// Syntax: last or last(expr)
    fn parse_last_expr(&mut self) -> Result<Expr, ParseError> {
        self.consume_keyword("last");
        self.skip_ws();
        if self.peek() == Some('(') {
            self.next();
            self.skip_ws();
            let expr = self.parse_pipe_expr()?;
            self.skip_ws();
            self.expect(')')?;
            Ok(Expr::LastExpr(Box::new(expr)))
        } else {
            Ok(Expr::Builtin(Builtin::Last))
        }
    }

    /// Parse a range expression.
    /// Syntax: range(N) or range(A; B) or range(A; B; STEP)
    fn parse_range_expr(&mut self) -> Result<Expr, ParseError> {
        self.consume_keyword("range");
        self.skip_ws();
        self.expect('(')?;
        self.skip_ws();
        let first = self.parse_pipe_expr()?;
        self.skip_ws();

        if self.peek() == Some(')') {
            // range(N) - from 0 to N
            self.next();
            return Ok(Expr::Range {
                from: Box::new(Expr::Literal(Literal::Int(0))),
                to: Some(Box::new(first)),
                step: None,
            });
        }

        self.expect(';')?;
        self.skip_ws();
        let second = self.parse_pipe_expr()?;
        self.skip_ws();

        if self.peek() == Some(')') {
            // range(A; B)
            self.next();
            return Ok(Expr::Range {
                from: Box::new(first),
                to: Some(Box::new(second)),
                step: None,
            });
        }

        self.expect(';')?;
        self.skip_ws();
        let step = self.parse_pipe_expr()?;
        self.skip_ws();
        self.expect(')')?;

        Ok(Expr::Range {
            from: Box::new(first),
            to: Some(Box::new(second)),
            step: Some(Box::new(step)),
        })
    }

    // =========================================================================
    // Phase 9: Variables & Definitions
    // =========================================================================

    /// Parse a pattern for destructuring.
    /// Patterns can be:
    /// - `$var` - simple variable binding
    /// - `{key: $var, ...}` - object destructuring
    /// - `[$first, $second, ...]` - array destructuring
    fn parse_pattern(&mut self) -> Result<Pattern, ParseError> {
        self.skip_ws();
        match self.peek() {
            Some('$') => {
                // Simple variable: $var
                self.next();
                let name = self.parse_ident()?;
                Ok(Pattern::Var(name))
            }
            Some('{') => {
                // Object pattern: {key: $var, ...}
                self.next();
                self.skip_ws();

                let mut entries = Vec::new();

                // Empty object pattern
                if self.peek() == Some('}') {
                    self.next();
                    return Ok(Pattern::Object(entries));
                }

                loop {
                    self.skip_ws();
                    // Parse key (must be identifier or string)
                    let key = if self.peek() == Some('"') {
                        self.parse_string_literal()?
                    } else {
                        self.parse_ident()?
                    };

                    self.skip_ws();
                    self.expect(':')?;
                    self.skip_ws();

                    // Parse the pattern for this key
                    let pattern = self.parse_pattern()?;
                    entries.push(PatternEntry { key, pattern });

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
                        _ => {
                            return Err(ParseError::new(
                                "expected ',' or '}' in pattern",
                                self.pos,
                            ));
                        }
                    }
                }

                Ok(Pattern::Object(entries))
            }
            Some('[') => {
                // Array pattern: [$first, $second, ...]
                self.next();
                self.skip_ws();

                let mut patterns = Vec::new();

                // Empty array pattern
                if self.peek() == Some(']') {
                    self.next();
                    return Ok(Pattern::Array(patterns));
                }

                loop {
                    self.skip_ws();
                    let pattern = self.parse_pattern()?;
                    patterns.push(pattern);

                    self.skip_ws();
                    match self.peek() {
                        Some(',') => {
                            self.next();
                            continue;
                        }
                        Some(']') => {
                            self.next();
                            break;
                        }
                        _ => {
                            return Err(ParseError::new(
                                "expected ',' or ']' in pattern",
                                self.pos,
                            ));
                        }
                    }
                }

                Ok(Pattern::Array(patterns))
            }
            _ => Err(ParseError::new(
                "expected pattern ($var, {key: $var}, or [$var])",
                self.pos,
            )),
        }
    }

    /// Parse a label expression.
    /// Syntax: label $name | expr
    fn parse_label_expr(&mut self) -> Result<Expr, ParseError> {
        self.consume_keyword("label");
        self.skip_ws();

        // Expect $name
        if self.peek() != Some('$') {
            return Err(ParseError::new("expected '$' after 'label'", self.pos));
        }
        self.next();
        let name = self.parse_ident()?;
        self.skip_ws();

        // Expect '|'
        if self.peek() != Some('|') {
            return Err(ParseError::new("expected '|' after label name", self.pos));
        }
        self.next();
        self.skip_ws();

        // Parse body expression
        let body = self.parse_pipe_expr()?;

        Ok(Expr::Label {
            name,
            body: Box::new(body),
        })
    }

    /// Parse a break expression.
    /// Syntax: break $name
    fn parse_break_expr(&mut self) -> Result<Expr, ParseError> {
        self.consume_keyword("break");
        self.skip_ws();

        // Expect $name
        if self.peek() != Some('$') {
            return Err(ParseError::new("expected '$' after 'break'", self.pos));
        }
        self.next();
        let name = self.parse_ident()?;

        Ok(Expr::Break(name))
    }

    /// Parse a function definition.
    /// Syntax: def NAME: BODY; or def NAME(PARAMS): BODY;
    fn parse_def_expr(&mut self) -> Result<Expr, ParseError> {
        self.consume_keyword("def");
        self.skip_ws();

        // Parse function name
        let name = self.parse_ident()?;
        self.skip_ws();

        // Parse optional parameters
        let params = if self.peek() == Some('(') {
            self.next();
            self.skip_ws();
            let mut params = Vec::new();

            if self.peek() != Some(')') {
                loop {
                    // Parameters can be $var or just var
                    if self.peek() == Some('$') {
                        self.next();
                    }
                    let param = self.parse_ident()?;
                    params.push(param);
                    self.skip_ws();

                    match self.peek() {
                        Some(';') | Some(',') => {
                            self.next();
                            self.skip_ws();
                        }
                        Some(')') => break,
                        _ => {
                            return Err(ParseError::new(
                                "expected ';', ',', or ')' in parameter list",
                                self.pos,
                            ));
                        }
                    }
                }
            }
            self.expect(')')?;
            params
        } else {
            Vec::new()
        };

        self.skip_ws();
        self.expect(':')?;
        self.skip_ws();

        // Parse function body
        let body = self.parse_pipe_expr()?;
        self.skip_ws();

        // Expect semicolon
        self.expect(';')?;
        self.skip_ws();

        // Parse the rest of the expression where this function is in scope
        let then = self.parse_pipe_expr()?;

        Ok(Expr::FuncDef {
            name,
            params,
            body: Box::new(body),
            then: Box::new(then),
        })
    }

    /// Parse a function call or return an error for unknown identifier.
    /// Function call syntax: NAME or NAME(args; args; ...) or NAMESPACE::NAME(args)
    fn parse_func_call_or_error(&mut self) -> Result<Expr, ParseError> {
        let _start_pos = self.pos;
        let name = self.parse_ident()?;
        self.skip_ws();

        // Check for namespaced call (module::func)
        if self.peek_str(2) == "::" {
            return self.parse_namespaced_call(name);
        }

        // Check for function arguments
        let args = if self.peek() == Some('(') {
            self.next();
            self.skip_ws();
            let mut args = Vec::new();

            if self.peek() != Some(')') {
                loop {
                    let arg = self.parse_pipe_expr()?;
                    args.push(arg);
                    self.skip_ws();

                    match self.peek() {
                        Some(';') => {
                            self.next();
                            self.skip_ws();
                        }
                        Some(')') => break,
                        _ => {
                            return Err(ParseError::new(
                                "expected ';' or ')' in function arguments",
                                self.pos,
                            ));
                        }
                    }
                }
            }
            self.expect(')')?;
            args
        } else {
            Vec::new()
        };

        // Return as function call - the evaluator will check if it's defined
        // Note: for known identifiers that aren't functions, we'd have returned earlier
        // So if we reach here, it's either a user-defined function call or an error
        if args.is_empty() {
            // Zero-arg function call - but this might be an unknown identifier
            // For now, treat it as a function call; the evaluator will handle errors
            Ok(Expr::FuncCall { name, args })
        } else {
            // Has arguments, definitely a function call
            Ok(Expr::FuncCall { name, args })
        }
    }

    /// Parse a format string: @text, @json, @uri, @dsv(delimiter), etc.
    fn parse_format_string(&mut self) -> Result<Expr, ParseError> {
        self.expect('@')?;

        // Parse the format name
        let format_start = self.pos;
        while self
            .peek()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '6' || c == '4')
        {
            self.next();
        }

        let format_name = &self.input[format_start..self.pos];
        let format_type = match format_name {
            "text" => FormatType::Text,
            "json" => FormatType::Json,
            "uri" => FormatType::Uri,
            "csv" => FormatType::Csv,
            "tsv" => FormatType::Tsv,
            "dsv" => {
                // @dsv(delimiter) - parse the delimiter argument
                self.skip_ws();
                self.expect('(')?;
                self.skip_ws();

                // Parse delimiter as a string literal
                if self.peek() != Some('"') {
                    return Err(ParseError::new(
                        "expected string delimiter argument for @dsv".to_string(),
                        self.pos,
                    ));
                }

                let delimiter = self.parse_string_literal()?;

                self.skip_ws();
                self.expect(')')?;
                FormatType::Dsv(delimiter)
            }
            "base64" => FormatType::Base64,
            "base64d" => FormatType::Base64d,
            "html" => FormatType::Html,
            "sh" => FormatType::Sh,
            "urid" => FormatType::Urid,
            "yaml" => FormatType::Yaml,
            "props" => FormatType::Props,
            _ => {
                return Err(ParseError::new(
                    format!("unknown format '@{}'", format_name),
                    format_start,
                ));
            }
        };

        Ok(Expr::Format(format_type))
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

        // Type filter functions (select by type)
        if self.matches_keyword("values") {
            self.consume_keyword("values");
            return Ok(Some(Builtin::Values));
        }
        if self.matches_keyword("nulls") {
            self.consume_keyword("nulls");
            return Ok(Some(Builtin::Nulls));
        }
        if self.matches_keyword("booleans") {
            self.consume_keyword("booleans");
            return Ok(Some(Builtin::Booleans));
        }
        if self.matches_keyword("numbers") {
            self.consume_keyword("numbers");
            return Ok(Some(Builtin::Numbers));
        }
        // Note: "strings" must be checked before "string" in any other context
        if self.matches_keyword("strings") {
            self.consume_keyword("strings");
            return Ok(Some(Builtin::Strings));
        }
        if self.matches_keyword("arrays") {
            self.consume_keyword("arrays");
            return Ok(Some(Builtin::Arrays));
        }
        if self.matches_keyword("objects") {
            self.consume_keyword("objects");
            return Ok(Some(Builtin::Objects));
        }
        if self.matches_keyword("iterables") {
            self.consume_keyword("iterables");
            return Ok(Some(Builtin::Iterables));
        }
        if self.matches_keyword("scalars") {
            self.consume_keyword("scalars");
            return Ok(Some(Builtin::Scalars));
        }
        // Additional numeric type filters
        if self.matches_keyword("normals") {
            self.consume_keyword("normals");
            return Ok(Some(Builtin::Normals));
        }
        if self.matches_keyword("finites") {
            self.consume_keyword("finites");
            return Ok(Some(Builtin::Finites));
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
        // Note: If followed by '(', these should be parsed as user-defined function calls
        if self.matches_keyword("add") && !self.peek_after_keyword_is_paren("add") {
            self.consume_keyword("add");
            return Ok(Some(Builtin::Add));
        }
        if self.matches_keyword("any") && !self.peek_after_keyword_is_paren("any") {
            self.consume_keyword("any");
            return Ok(Some(Builtin::Any));
        }
        if self.matches_keyword("all") && !self.peek_after_keyword_is_paren("all") {
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
        if self.matches_keyword("min") && !self.peek_after_keyword_is_paren("min") {
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
        if self.matches_keyword("max") && !self.peek_after_keyword_is_paren("max") {
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
        // Check splits before split since split is a prefix of splits
        if self.matches_keyword("splits") {
            self.consume_keyword("splits");
            self.skip_ws();
            self.expect('(')?;
            self.skip_ws();
            let re = self.parse_pipe_expr()?;
            self.skip_ws();
            if self.peek() == Some(';') {
                self.next(); // consume ';'
                self.skip_ws();
                let flags = self.parse_pipe_expr()?;
                self.skip_ws();
                self.expect(')')?;
                return Ok(Some(Builtin::SplitsFlags(Box::new(re), Box::new(flags))));
            }
            self.expect(')')?;
            return Ok(Some(Builtin::Splits(Box::new(re))));
        }
        if self.matches_keyword("split") {
            self.consume_keyword("split");
            self.skip_ws();
            self.expect('(')?;
            self.skip_ws();
            let s = self.parse_pipe_expr()?;
            self.skip_ws();
            if self.peek() == Some(';') {
                self.next(); // consume ';'
                self.skip_ws();
                let flags = self.parse_pipe_expr()?;
                self.skip_ws();
                self.expect(')')?;
                return Ok(Some(Builtin::SplitRegex(Box::new(s), Box::new(flags))));
            }
            self.expect(')')?;
            return Ok(Some(Builtin::Split(Box::new(s))));
        }
        // match function - regex matching
        if self.matches_keyword("match") {
            self.consume_keyword("match");
            self.skip_ws();
            self.expect('(')?;
            self.skip_ws();
            let re = self.parse_pipe_expr()?;
            self.skip_ws();
            if self.peek() == Some(';') {
                self.next(); // consume ';'
                self.skip_ws();
                let flags = self.parse_pipe_expr()?;
                self.skip_ws();
                self.expect(')')?;
                return Ok(Some(Builtin::MatchFlags(Box::new(re), Box::new(flags))));
            }
            self.expect(')')?;
            return Ok(Some(Builtin::Match(Box::new(re))));
        }
        // capture function - named capture groups
        if self.matches_keyword("capture") {
            self.consume_keyword("capture");
            self.skip_ws();
            self.expect('(')?;
            self.skip_ws();
            let re = self.parse_pipe_expr()?;
            self.skip_ws();
            if self.peek() == Some(';') {
                self.next(); // consume ';'
                self.skip_ws();
                let flags = self.parse_pipe_expr()?;
                self.skip_ws();
                self.expect(')')?;
                return Ok(Some(Builtin::CaptureFlags(Box::new(re), Box::new(flags))));
            }
            self.expect(')')?;
            return Ok(Some(Builtin::Capture(Box::new(re))));
        }
        // sub function - replace first match
        if self.matches_keyword("sub") {
            self.consume_keyword("sub");
            self.skip_ws();
            self.expect('(')?;
            self.skip_ws();
            let re = self.parse_pipe_expr()?;
            self.skip_ws();
            self.expect(';')?;
            self.skip_ws();
            let replacement = self.parse_pipe_expr()?;
            self.skip_ws();
            if self.peek() == Some(';') {
                self.next(); // consume ';'
                self.skip_ws();
                let flags = self.parse_pipe_expr()?;
                self.skip_ws();
                self.expect(')')?;
                return Ok(Some(Builtin::SubFlags(
                    Box::new(re),
                    Box::new(replacement),
                    Box::new(flags),
                )));
            }
            self.expect(')')?;
            return Ok(Some(Builtin::Sub(Box::new(re), Box::new(replacement))));
        }
        // gsub function - replace all matches
        if self.matches_keyword("gsub") {
            self.consume_keyword("gsub");
            self.skip_ws();
            self.expect('(')?;
            self.skip_ws();
            let re = self.parse_pipe_expr()?;
            self.skip_ws();
            self.expect(';')?;
            self.skip_ws();
            let replacement = self.parse_pipe_expr()?;
            self.skip_ws();
            if self.peek() == Some(';') {
                self.next(); // consume ';'
                self.skip_ws();
                let flags = self.parse_pipe_expr()?;
                self.skip_ws();
                self.expect(')')?;
                return Ok(Some(Builtin::GsubFlags(
                    Box::new(re),
                    Box::new(replacement),
                    Box::new(flags),
                )));
            }
            self.expect(')')?;
            return Ok(Some(Builtin::Gsub(Box::new(re), Box::new(replacement))));
        }
        // scan function - find all matches
        if self.matches_keyword("scan") {
            self.consume_keyword("scan");
            self.skip_ws();
            self.expect('(')?;
            self.skip_ws();
            let re = self.parse_pipe_expr()?;
            self.skip_ws();
            if self.peek() == Some(';') {
                self.next(); // consume ';'
                self.skip_ws();
                let flags = self.parse_pipe_expr()?;
                self.skip_ws();
                self.expect(')')?;
                return Ok(Some(Builtin::ScanFlags(Box::new(re), Box::new(flags))));
            }
            self.expect(')')?;
            return Ok(Some(Builtin::Scan(Box::new(re))));
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
        // Note: first, last are handled in parse_primary before try_parse_builtin
        // to support both first/last (no args) and first(expr)/last(expr)
        // Note: nth is handled in Phase 13 section to support both nth(n) and nth(n; expr)
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

        // Phase 6: Type Conversions
        if self.matches_keyword("tostring") {
            self.consume_keyword("tostring");
            return Ok(Some(Builtin::ToString));
        }
        if self.matches_keyword("tonumber") {
            self.consume_keyword("tonumber");
            return Ok(Some(Builtin::ToNumber));
        }
        if self.matches_keyword("tojson") {
            self.consume_keyword("tojson");
            return Ok(Some(Builtin::ToJson));
        }
        if self.matches_keyword("fromjson") {
            self.consume_keyword("fromjson");
            return Ok(Some(Builtin::FromJson));
        }

        // Phase 6: Additional String Functions
        if self.matches_keyword("explode") {
            self.consume_keyword("explode");
            return Ok(Some(Builtin::Explode));
        }
        if self.matches_keyword("implode") {
            self.consume_keyword("implode");
            return Ok(Some(Builtin::Implode));
        }
        if self.matches_keyword("test") {
            self.consume_keyword("test");
            self.skip_ws();
            self.expect('(')?;
            self.skip_ws();
            let re = self.parse_pipe_expr()?;
            self.skip_ws();
            if self.peek() == Some(';') {
                self.next(); // consume ';'
                self.skip_ws();
                let flags = self.parse_pipe_expr()?;
                self.skip_ws();
                self.expect(')')?;
                return Ok(Some(Builtin::TestFlags(Box::new(re), Box::new(flags))));
            }
            self.expect(')')?;
            return Ok(Some(Builtin::Test(Box::new(re))));
        }
        if self.matches_keyword("indices") {
            self.consume_keyword("indices");
            self.skip_ws();
            self.expect('(')?;
            self.skip_ws();
            let s = self.parse_pipe_expr()?;
            self.skip_ws();
            self.expect(')')?;
            return Ok(Some(Builtin::Indices(Box::new(s))));
        }
        // Check index before rindex since rindex contains "index"
        if self.matches_keyword("rindex") {
            self.consume_keyword("rindex");
            self.skip_ws();
            self.expect('(')?;
            self.skip_ws();
            let s = self.parse_pipe_expr()?;
            self.skip_ws();
            self.expect(')')?;
            return Ok(Some(Builtin::Rindex(Box::new(s))));
        }
        if self.matches_keyword("index") {
            self.consume_keyword("index");
            self.skip_ws();
            self.expect('(')?;
            self.skip_ws();
            let s = self.parse_pipe_expr()?;
            self.skip_ws();
            self.expect(')')?;
            return Ok(Some(Builtin::Index(Box::new(s))));
        }
        if self.matches_keyword("tojsonstream") {
            self.consume_keyword("tojsonstream");
            return Ok(Some(Builtin::ToJsonStream));
        }
        if self.matches_keyword("fromjsonstream") {
            self.consume_keyword("fromjsonstream");
            return Ok(Some(Builtin::FromJsonStream));
        }
        if self.matches_keyword("getpath") {
            self.consume_keyword("getpath");
            self.skip_ws();
            self.expect('(')?;
            self.skip_ws();
            let path = self.parse_pipe_expr()?;
            self.skip_ws();
            self.expect(')')?;
            return Ok(Some(Builtin::GetPath(Box::new(path))));
        }

        // Phase 8: Advanced Control Flow Builtins
        // recurse, recurse(f), recurse(f; cond)
        if self.matches_keyword("recurse") {
            self.consume_keyword("recurse");
            self.skip_ws();
            if self.peek() == Some('(') {
                self.next();
                self.skip_ws();
                let f = self.parse_pipe_expr()?;
                self.skip_ws();
                if self.peek() == Some(';') {
                    self.next();
                    self.skip_ws();
                    let cond = self.parse_pipe_expr()?;
                    self.skip_ws();
                    self.expect(')')?;
                    return Ok(Some(Builtin::RecurseCond(Box::new(f), Box::new(cond))));
                }
                self.expect(')')?;
                return Ok(Some(Builtin::RecurseF(Box::new(f))));
            }
            return Ok(Some(Builtin::Recurse));
        }

        // walk(f)
        if self.matches_keyword("walk") {
            self.consume_keyword("walk");
            self.skip_ws();
            self.expect('(')?;
            self.skip_ws();
            let f = self.parse_pipe_expr()?;
            self.skip_ws();
            self.expect(')')?;
            return Ok(Some(Builtin::Walk(Box::new(f))));
        }

        // isvalid(expr)
        if self.matches_keyword("isvalid") {
            self.consume_keyword("isvalid");
            self.skip_ws();
            self.expect('(')?;
            self.skip_ws();
            let expr = self.parse_pipe_expr()?;
            self.skip_ws();
            self.expect(')')?;
            return Ok(Some(Builtin::IsValid(Box::new(expr))));
        }

        // Phase 10: Path Expressions
        // path(expr) - return the path to values selected by expr
        // path (no-arg, yq) - return the current traversal path
        if self.matches_keyword("path") {
            self.consume_keyword("path");
            self.skip_ws();
            if self.peek() == Some('(') {
                // path(expr) - jq style
                self.next();
                self.skip_ws();
                let expr = self.parse_pipe_expr()?;
                self.skip_ws();
                self.expect(')')?;
                return Ok(Some(Builtin::Path(Box::new(expr))));
            }
            // path (no-arg) - yq style
            return Ok(Some(Builtin::PathNoArg));
        }
        // parent (no-arg, yq) - return the parent node
        // parent(n) (yq) - return the nth parent node
        if self.matches_keyword("parent") {
            self.consume_keyword("parent");
            self.skip_ws();
            if self.peek() == Some('(') {
                // parent(n) - nth parent
                self.next();
                self.skip_ws();
                let n = self.parse_pipe_expr()?;
                self.skip_ws();
                self.expect(')')?;
                return Ok(Some(Builtin::ParentN(Box::new(n))));
            }
            // parent (no-arg) - immediate parent
            return Ok(Some(Builtin::Parent));
        }
        // leaf_paths - must check before paths
        if self.matches_keyword("leaf_paths") {
            self.consume_keyword("leaf_paths");
            return Ok(Some(Builtin::LeafPaths));
        }
        // paths or paths(filter)
        if self.matches_keyword("paths") {
            self.consume_keyword("paths");
            self.skip_ws();
            if self.peek() == Some('(') {
                self.next();
                self.skip_ws();
                let filter = self.parse_pipe_expr()?;
                self.skip_ws();
                self.expect(')')?;
                return Ok(Some(Builtin::PathsFilter(Box::new(filter))));
            }
            return Ok(Some(Builtin::Paths));
        }
        // setpath(path; value)
        if self.matches_keyword("setpath") {
            self.consume_keyword("setpath");
            self.skip_ws();
            self.expect('(')?;
            self.skip_ws();
            let path = self.parse_pipe_expr()?;
            self.skip_ws();
            self.expect(';')?;
            self.skip_ws();
            let value = self.parse_pipe_expr()?;
            self.skip_ws();
            self.expect(')')?;
            return Ok(Some(Builtin::SetPath(Box::new(path), Box::new(value))));
        }
        // delpaths(paths)
        if self.matches_keyword("delpaths") {
            self.consume_keyword("delpaths");
            self.skip_ws();
            self.expect('(')?;
            self.skip_ws();
            let paths = self.parse_pipe_expr()?;
            self.skip_ws();
            self.expect(')')?;
            return Ok(Some(Builtin::DelPaths(Box::new(paths))));
        }
        // del(path) - delete value at path
        if self.matches_keyword("del") {
            self.consume_keyword("del");
            self.skip_ws();
            self.expect('(')?;
            self.skip_ws();
            let path = self.parse_pipe_expr()?;
            self.skip_ws();
            self.expect(')')?;
            return Ok(Some(Builtin::Del(Box::new(path))));
        }

        // Phase 10: Math Functions
        if self.matches_keyword("floor") {
            self.consume_keyword("floor");
            return Ok(Some(Builtin::Floor));
        }
        if self.matches_keyword("ceil") {
            self.consume_keyword("ceil");
            return Ok(Some(Builtin::Ceil));
        }
        if self.matches_keyword("round") {
            self.consume_keyword("round");
            return Ok(Some(Builtin::Round));
        }
        if self.matches_keyword("sqrt") {
            self.consume_keyword("sqrt");
            return Ok(Some(Builtin::Sqrt));
        }
        if self.matches_keyword("fabs") {
            self.consume_keyword("fabs");
            return Ok(Some(Builtin::Fabs));
        }
        // Logarithmic - check log10 and log2 before log
        if self.matches_keyword("log10") {
            self.consume_keyword("log10");
            return Ok(Some(Builtin::Log10));
        }
        if self.matches_keyword("log2") {
            self.consume_keyword("log2");
            return Ok(Some(Builtin::Log2));
        }
        if self.matches_keyword("log") {
            self.consume_keyword("log");
            return Ok(Some(Builtin::Log));
        }
        // Exponential - check exp10 and exp2 before exp
        if self.matches_keyword("exp10") {
            self.consume_keyword("exp10");
            return Ok(Some(Builtin::Exp10));
        }
        if self.matches_keyword("exp2") {
            self.consume_keyword("exp2");
            return Ok(Some(Builtin::Exp2));
        }
        if self.matches_keyword("exp") {
            self.consume_keyword("exp");
            return Ok(Some(Builtin::Exp));
        }
        // pow(base; exp)
        if self.matches_keyword("pow") {
            self.consume_keyword("pow");
            self.skip_ws();
            self.expect('(')?;
            self.skip_ws();
            let base = self.parse_pipe_expr()?;
            self.skip_ws();
            self.expect(';')?;
            self.skip_ws();
            let exp = self.parse_pipe_expr()?;
            self.skip_ws();
            self.expect(')')?;
            return Ok(Some(Builtin::Pow(Box::new(base), Box::new(exp))));
        }
        // Trigonometric functions - check longer names first
        if self.matches_keyword("sinh") {
            self.consume_keyword("sinh");
            return Ok(Some(Builtin::Sinh));
        }
        if self.matches_keyword("cosh") {
            self.consume_keyword("cosh");
            return Ok(Some(Builtin::Cosh));
        }
        if self.matches_keyword("tanh") {
            self.consume_keyword("tanh");
            return Ok(Some(Builtin::Tanh));
        }
        if self.matches_keyword("asinh") {
            self.consume_keyword("asinh");
            return Ok(Some(Builtin::Asinh));
        }
        if self.matches_keyword("acosh") {
            self.consume_keyword("acosh");
            return Ok(Some(Builtin::Acosh));
        }
        if self.matches_keyword("atanh") {
            self.consume_keyword("atanh");
            return Ok(Some(Builtin::Atanh));
        }
        // atan2(y; x) - must check before atan
        if self.matches_keyword("atan2") {
            self.consume_keyword("atan2");
            self.skip_ws();
            self.expect('(')?;
            self.skip_ws();
            let y = self.parse_pipe_expr()?;
            self.skip_ws();
            self.expect(';')?;
            self.skip_ws();
            let x = self.parse_pipe_expr()?;
            self.skip_ws();
            self.expect(')')?;
            return Ok(Some(Builtin::Atan2(Box::new(y), Box::new(x))));
        }
        if self.matches_keyword("asin") {
            self.consume_keyword("asin");
            return Ok(Some(Builtin::Asin));
        }
        if self.matches_keyword("acos") {
            self.consume_keyword("acos");
            return Ok(Some(Builtin::Acos));
        }
        if self.matches_keyword("atan") {
            self.consume_keyword("atan");
            return Ok(Some(Builtin::Atan));
        }
        if self.matches_keyword("sin") {
            self.consume_keyword("sin");
            return Ok(Some(Builtin::Sin));
        }
        if self.matches_keyword("cos") {
            self.consume_keyword("cos");
            return Ok(Some(Builtin::Cos));
        }
        if self.matches_keyword("tan") {
            self.consume_keyword("tan");
            return Ok(Some(Builtin::Tan));
        }

        // Phase 10: Number Classification & Constants
        // Check isinfinite, isnan, isnormal, isfinite before infinite, nan
        if self.matches_keyword("isinfinite") {
            self.consume_keyword("isinfinite");
            return Ok(Some(Builtin::IsInfinite));
        }
        if self.matches_keyword("isnan") {
            self.consume_keyword("isnan");
            return Ok(Some(Builtin::IsNan));
        }
        if self.matches_keyword("isnormal") {
            self.consume_keyword("isnormal");
            return Ok(Some(Builtin::IsNormal));
        }
        if self.matches_keyword("isfinite") {
            self.consume_keyword("isfinite");
            return Ok(Some(Builtin::IsFinite));
        }
        if self.matches_keyword("infinite") {
            self.consume_keyword("infinite");
            return Ok(Some(Builtin::Infinite));
        }
        if self.matches_keyword("nan") {
            self.consume_keyword("nan");
            return Ok(Some(Builtin::Nan));
        }

        // Phase 10: Debug
        if self.matches_keyword("debug") {
            self.consume_keyword("debug");
            self.skip_ws();
            if self.peek() == Some('(') {
                self.next();
                self.skip_ws();
                let msg = self.parse_pipe_expr()?;
                self.skip_ws();
                self.expect(')')?;
                return Ok(Some(Builtin::DebugMsg(Box::new(msg))));
            }
            return Ok(Some(Builtin::Debug));
        }

        // Phase 10: Environment
        // $ENV - the full environment object - handled via Var("ENV") after $ is parsed
        // env(VAR) - get specific environment variable (yq syntax)
        // env.VAR or env (as builtin)
        if self.matches_keyword("env") {
            self.consume_keyword("env");
            self.skip_ws();
            // Check for env(VAR) syntax - yq style
            if self.peek() == Some('(') {
                self.next(); // consume '('
                self.skip_ws();
                // Parse identifier (unquoted variable name)
                let var_name = self.parse_ident()?;
                self.skip_ws();
                self.expect(')')?;
                return Ok(Some(Builtin::EnvObject(var_name)));
            }
            return Ok(Some(Builtin::Env));
        }

        // strenv(VAR) - get environment variable as string (yq specific)
        if self.matches_keyword("strenv") {
            self.consume_keyword("strenv");
            self.skip_ws();
            self.expect('(')?;
            self.skip_ws();
            let var_name = self.parse_ident()?;
            self.skip_ws();
            self.expect(')')?;
            return Ok(Some(Builtin::StrEnv(var_name)));
        }

        // Phase 10: String functions
        // Check ltrim and rtrim before trim
        if self.matches_keyword("ltrim") {
            self.consume_keyword("ltrim");
            return Ok(Some(Builtin::Ltrim));
        }
        if self.matches_keyword("rtrim") {
            self.consume_keyword("rtrim");
            return Ok(Some(Builtin::Rtrim));
        }
        if self.matches_keyword("trim") {
            self.consume_keyword("trim");
            return Ok(Some(Builtin::Trim));
        }

        // Phase 10: Array functions
        if self.matches_keyword("transpose") {
            self.consume_keyword("transpose");
            return Ok(Some(Builtin::Transpose));
        }
        if self.matches_keyword("bsearch") {
            self.consume_keyword("bsearch");
            self.skip_ws();
            self.expect('(')?;
            self.skip_ws();
            let x = self.parse_pipe_expr()?;
            self.skip_ws();
            self.expect(')')?;
            return Ok(Some(Builtin::BSearch(Box::new(x))));
        }

        // Phase 10: Object functions
        if self.matches_keyword("modulemeta") {
            self.consume_keyword("modulemeta");
            self.skip_ws();
            self.expect('(')?;
            self.skip_ws();
            let name = self.parse_pipe_expr()?;
            self.skip_ws();
            self.expect(')')?;
            return Ok(Some(Builtin::ModuleMeta(Box::new(name))));
        }

        // pick(keys) - yq: select only specified keys from object/array
        if self.matches_keyword("pick") {
            self.consume_keyword("pick");
            self.skip_ws();
            self.expect('(')?;
            self.skip_ws();
            let keys = self.parse_pipe_expr()?;
            self.skip_ws();
            self.expect(')')?;
            return Ok(Some(Builtin::Pick(Box::new(keys))));
        }

        // omit(keys) - yq: remove specified keys from object/indices from array
        if self.matches_keyword("omit") {
            self.consume_keyword("omit");
            self.skip_ws();
            self.expect('(')?;
            self.skip_ws();
            let keys = self.parse_pipe_expr()?;
            self.skip_ws();
            self.expect(')')?;
            return Ok(Some(Builtin::Omit(Box::new(keys))));
        }

        // tag - yq: return YAML type tag (!!str, !!int, !!map, etc.)
        if self.matches_keyword("tag") {
            self.consume_keyword("tag");
            return Ok(Some(Builtin::Tag));
        }

        // anchor - yq: return anchor name if present
        if self.matches_keyword("anchor") {
            self.consume_keyword("anchor");
            return Ok(Some(Builtin::Anchor));
        }

        // style - yq: return scalar/collection style
        if self.matches_keyword("style") {
            self.consume_keyword("style");
            return Ok(Some(Builtin::Style));
        }

        // kind - yq: return node kind (scalar, seq, map)
        if self.matches_keyword("kind") {
            self.consume_keyword("kind");
            return Ok(Some(Builtin::Kind));
        }

        // key - yq: return current key when iterating
        if self.matches_keyword("key") {
            self.consume_keyword("key");
            return Ok(Some(Builtin::Key));
        }

        // line - yq: return 1-based line number
        if self.matches_keyword("line") {
            self.consume_keyword("line");
            return Ok(Some(Builtin::Line));
        }

        // column - yq: return 1-based column number
        if self.matches_keyword("column") {
            self.consume_keyword("column");
            return Ok(Some(Builtin::Column));
        }

        // document_index / di - yq: return 0-indexed document position in multi-doc stream
        if self.matches_keyword("document_index") {
            self.consume_keyword("document_index");
            return Ok(Some(Builtin::DocumentIndex));
        }
        if self.matches_keyword("di") {
            self.consume_keyword("di");
            return Ok(Some(Builtin::DocumentIndex));
        }

        // shuffle - yq: randomly shuffle array elements
        if self.matches_keyword("shuffle") {
            self.consume_keyword("shuffle");
            return Ok(Some(Builtin::Shuffle));
        }

        // pivot - yq: transpose arrays/objects
        if self.matches_keyword("pivot") {
            self.consume_keyword("pivot");
            return Ok(Some(Builtin::Pivot));
        }

        // split_doc - yq: mark output as separate YAML documents
        if self.matches_keyword("split_doc") {
            self.consume_keyword("split_doc");
            return Ok(Some(Builtin::SplitDoc));
        }

        // Phase 12: Additional builtins
        if self.matches_keyword("now") {
            self.consume_keyword("now");
            return Ok(Some(Builtin::Now));
        }
        if self.matches_keyword("abs") {
            self.consume_keyword("abs");
            return Ok(Some(Builtin::Abs));
        }
        if self.matches_keyword("builtins") {
            self.consume_keyword("builtins");
            return Ok(Some(Builtin::Builtins));
        }

        // Phase 13: Iteration control
        // limit(n; expr) - output at most n values from expr
        if self.matches_keyword("limit") {
            self.consume_keyword("limit");
            self.skip_ws();
            self.expect('(')?;
            self.skip_ws();
            let n = self.parse_pipe_expr()?;
            self.skip_ws();
            self.expect(';')?;
            self.skip_ws();
            let expr = self.parse_pipe_expr()?;
            self.skip_ws();
            self.expect(')')?;
            return Ok(Some(Builtin::Limit(Box::new(n), Box::new(expr))));
        }

        // skip(n; expr) - skip first n outputs from expr
        if self.matches_keyword("skip") {
            self.consume_keyword("skip");
            self.skip_ws();
            self.expect('(')?;
            self.skip_ws();
            let n = self.parse_pipe_expr()?;
            self.skip_ws();
            self.expect(';')?;
            self.skip_ws();
            let expr = self.parse_pipe_expr()?;
            self.skip_ws();
            self.expect(')')?;
            return Ok(Some(Builtin::Skip(Box::new(n), Box::new(expr))));
        }

        // first(expr) or first - output only the first value
        // first without args is already handled by Phase 5 Builtin::First
        // first(expr) uses stream version
        if self.matches_keyword("first") {
            self.consume_keyword("first");
            self.skip_ws();
            if self.peek() == Some('(') {
                self.next();
                self.skip_ws();
                let expr = self.parse_pipe_expr()?;
                self.skip_ws();
                self.expect(')')?;
                return Ok(Some(Builtin::FirstStream(Box::new(expr))));
            }
            // No-arg first is already handled by Phase 5 Builtin::First
            return Ok(Some(Builtin::First));
        }

        // last(expr) or last - output only the last value
        // last without args is already handled by Phase 5 Builtin::Last
        if self.matches_keyword("last") {
            self.consume_keyword("last");
            self.skip_ws();
            if self.peek() == Some('(') {
                self.next();
                self.skip_ws();
                let expr = self.parse_pipe_expr()?;
                self.skip_ws();
                self.expect(')')?;
                return Ok(Some(Builtin::LastStream(Box::new(expr))));
            }
            // No-arg last is already handled by Phase 5 Builtin::Last
            return Ok(Some(Builtin::Last));
        }

        // nth(n; expr) or nth(n) - output only the nth value (0-indexed)
        // nth(n) without second arg is already handled by Phase 5 Builtin::Nth
        if self.matches_keyword("nth") {
            self.consume_keyword("nth");
            self.skip_ws();
            self.expect('(')?;
            self.skip_ws();
            let n = self.parse_pipe_expr()?;
            self.skip_ws();
            if self.peek() == Some(';') {
                self.next();
                self.skip_ws();
                let expr = self.parse_pipe_expr()?;
                self.skip_ws();
                self.expect(')')?;
                return Ok(Some(Builtin::NthStream(Box::new(n), Box::new(expr))));
            }
            self.expect(')')?;
            // No-arg nth(n) is already handled by Phase 5 Builtin::Nth
            return Ok(Some(Builtin::Nth(Box::new(n))));
        }

        // range(n), range(from; upto), range(from; upto; by)
        if self.matches_keyword("range") {
            self.consume_keyword("range");
            self.skip_ws();
            self.expect('(')?;
            self.skip_ws();
            let first = self.parse_pipe_expr()?;
            self.skip_ws();
            if self.peek() == Some(';') {
                self.next();
                self.skip_ws();
                let second = self.parse_pipe_expr()?;
                self.skip_ws();
                if self.peek() == Some(';') {
                    self.next();
                    self.skip_ws();
                    let third = self.parse_pipe_expr()?;
                    self.skip_ws();
                    self.expect(')')?;
                    return Ok(Some(Builtin::RangeFromToBy(
                        Box::new(first),
                        Box::new(second),
                        Box::new(third),
                    )));
                }
                self.expect(')')?;
                return Ok(Some(Builtin::RangeFromTo(
                    Box::new(first),
                    Box::new(second),
                )));
            }
            self.expect(')')?;
            return Ok(Some(Builtin::Range(Box::new(first))));
        }

        // isempty(expr) - returns true if expr produces no outputs
        if self.matches_keyword("isempty") {
            self.consume_keyword("isempty");
            self.skip_ws();
            self.expect('(')?;
            self.skip_ws();
            let expr = self.parse_pipe_expr()?;
            self.skip_ws();
            self.expect(')')?;
            return Ok(Some(Builtin::IsEmpty(Box::new(expr))));
        }

        // Phase 14: Recursive traversal (extends Phase 8)
        // recurse_down is an alias for recurse
        if self.matches_keyword("recurse_down") {
            self.consume_keyword("recurse_down");
            return Ok(Some(Builtin::RecurseDown));
        }

        // Phase 15: Date/Time functions
        if self.matches_keyword("gmtime") {
            self.consume_keyword("gmtime");
            return Ok(Some(Builtin::Gmtime));
        }
        if self.matches_keyword("localtime") {
            self.consume_keyword("localtime");
            return Ok(Some(Builtin::Localtime));
        }
        if self.matches_keyword("mktime") {
            self.consume_keyword("mktime");
            return Ok(Some(Builtin::Mktime));
        }
        if self.matches_keyword("strftime") {
            self.consume_keyword("strftime");
            self.skip_ws();
            self.expect('(')?;
            self.skip_ws();
            let fmt = self.parse_pipe_expr()?;
            self.skip_ws();
            self.expect(')')?;
            return Ok(Some(Builtin::Strftime(Box::new(fmt))));
        }
        if self.matches_keyword("strptime") {
            self.consume_keyword("strptime");
            self.skip_ws();
            self.expect('(')?;
            self.skip_ws();
            let fmt = self.parse_pipe_expr()?;
            self.skip_ws();
            self.expect(')')?;
            return Ok(Some(Builtin::Strptime(Box::new(fmt))));
        }
        if self.matches_keyword("todateiso8601") {
            self.consume_keyword("todateiso8601");
            return Ok(Some(Builtin::Todateiso8601));
        }
        if self.matches_keyword("fromdateiso8601") {
            self.consume_keyword("fromdateiso8601");
            return Ok(Some(Builtin::Fromdateiso8601));
        }
        if self.matches_keyword("todate") {
            self.consume_keyword("todate");
            return Ok(Some(Builtin::Todate));
        }
        if self.matches_keyword("fromdate") {
            self.consume_keyword("fromdate");
            return Ok(Some(Builtin::Fromdate));
        }

        // Phase 21: Extended Date/Time functions (yq)
        if self.matches_keyword("from_unix") {
            self.consume_keyword("from_unix");
            return Ok(Some(Builtin::FromUnix));
        }
        if self.matches_keyword("to_unix") {
            self.consume_keyword("to_unix");
            return Ok(Some(Builtin::ToUnix));
        }
        if self.matches_keyword("tz") {
            self.consume_keyword("tz");
            self.skip_ws();
            self.expect('(')?;
            self.skip_ws();
            let zone = self.parse_pipe_expr()?;
            self.skip_ws();
            self.expect(')')?;
            return Ok(Some(Builtin::Tz(Box::new(zone))));
        }

        // Phase 17: Combinations
        if self.matches_keyword("combinations") {
            self.consume_keyword("combinations");
            self.skip_ws();
            if self.peek() == Some('(') {
                self.next(); // consume '('
                self.skip_ws();
                let n = self.parse_pipe_expr()?;
                self.skip_ws();
                self.expect(')')?;
                return Ok(Some(Builtin::CombinationsN(Box::new(n))));
            }
            return Ok(Some(Builtin::Combinations));
        }

        // Phase 18: Additional math functions
        if self.matches_keyword("trunc") {
            self.consume_keyword("trunc");
            return Ok(Some(Builtin::Trunc));
        }

        // Phase 19: Type conversion
        if self.matches_keyword("toboolean") {
            self.consume_keyword("toboolean");
            return Ok(Some(Builtin::ToBoolean));
        }

        // Phase 22: File operations (yq)
        if self.matches_keyword("load") {
            self.consume_keyword("load");
            self.skip_ws();
            self.expect('(')?;
            self.skip_ws();
            let file_expr = self.parse_pipe_expr()?;
            self.skip_ws();
            self.expect(')')?;
            return Ok(Some(Builtin::Load(Box::new(file_expr))));
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
            Some('a') if self.matches_keyword("and") || self.matches_keyword("as") => true,
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
                    } else if self.peek() == Some('"') {
                        // Quoted field access `."key"`
                        let name = self.parse_string_literal()?;
                        let mut field_expr = Expr::Field(name);

                        // Check for optional
                        self.skip_ws();
                        if self.peek() == Some('?') {
                            self.next();
                            field_expr = Expr::Optional(Box::new(field_expr));
                        }

                        chain.push(field_expr);
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
            // Check for compound assignment operators (*=, /=, %=) and don't consume them
            let peek2 = self.peek_str(2);
            if peek2 == "*=" || peek2 == "%=" {
                break;
            }
            // /= needs special handling since // is alternative
            if peek2.starts_with('/') && peek2 != "//" && peek2.ends_with('=') {
                break;
            }
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
            // Check for compound assignment operators (+= or -=) and don't consume them
            let peek2 = self.peek_str(2);
            if peek2 == "+=" || peek2 == "-=" {
                break;
            }
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
            // Check for // but not //= (alternative assignment)
            let peek3 = self.peek_str(3);
            if peek3 == "//=" {
                break;
            }
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

    /// Parse assignment expressions: `path = value`, `path |= filter`, `path += value`, etc.
    /// Assignment has higher precedence than pipe, lower than alternative.
    fn parse_assignment(&mut self) -> Result<Expr, ParseError> {
        let left = self.parse_alternative()?;
        self.skip_ws();

        // Check for assignment operators
        let peek2 = self.peek_str(2);
        let peek3 = self.peek_str(3);

        // Check for |= (update)
        if peek2 == "|=" {
            self.next(); // |
            self.next(); // =
            self.skip_ws();
            let filter = self.parse_alternative()?;
            return Ok(Expr::Update {
                path: Box::new(left),
                filter: Box::new(filter),
            });
        }

        // Check for //= (alternative assignment)
        if peek3 == "//=" {
            self.next(); // /
            self.next(); // /
            self.next(); // =
            self.skip_ws();
            let value = self.parse_alternative()?;
            return Ok(Expr::AlternativeAssign {
                path: Box::new(left),
                value: Box::new(value),
            });
        }

        // Check for compound assignments: +=, -=, *=, /=, %=
        if peek2.len() == 2 && peek2.ends_with('=') {
            let op_char = peek2.chars().next().unwrap();
            let assign_op = match op_char {
                '+' => Some(AssignOp::Add),
                '-' => Some(AssignOp::Sub),
                '*' => Some(AssignOp::Mul),
                '/' => Some(AssignOp::Div),
                '%' => Some(AssignOp::Mod),
                _ => None,
            };

            if let Some(op) = assign_op {
                self.next(); // op char
                self.next(); // =
                self.skip_ws();
                let value = self.parse_alternative()?;
                return Ok(Expr::CompoundAssign {
                    op,
                    path: Box::new(left),
                    value: Box::new(value),
                });
            }
        }

        // Check for simple assignment: =
        // But be careful not to match == (comparison)
        if self.peek() == Some('=') && self.peek_str(2) != "==" {
            self.next(); // =
            self.skip_ws();
            let value = self.parse_alternative()?;
            return Ok(Expr::Assign {
                path: Box::new(left),
                value: Box::new(value),
            });
        }

        Ok(left)
    }

    /// Parse a pipe expression: `expr | expr | ...`
    /// Also handles `as` binding: `expr as $var | body` or `expr as {pattern} | body`
    fn parse_pipe_expr(&mut self) -> Result<Expr, ParseError> {
        let first = self.parse_assignment()?;
        self.skip_ws();

        // Check for `as` binding (Phase 8: simple var, Phase 9: patterns)
        if self.matches_keyword("as") {
            self.consume_keyword("as");
            self.skip_ws();

            // Check if it's a simple $var or a pattern
            let as_expr = self.parse_as_pattern(first)?;
            return Ok(as_expr);
        }

        if self.peek() != Some('|') {
            return Ok(first);
        }

        let mut exprs = vec![first];

        while self.peek() == Some('|') {
            self.next();
            self.skip_ws();
            let next_expr = self.parse_assignment()?;
            self.skip_ws();

            // Check for `as` binding after pipe
            if self.matches_keyword("as") {
                self.consume_keyword("as");
                self.skip_ws();

                let as_expr = self.parse_as_pattern(next_expr)?;
                // Wrap what we have so far
                let so_far = if exprs.len() == 1 {
                    exprs.pop().unwrap()
                } else {
                    Expr::Pipe(exprs)
                };
                return Ok(Expr::Pipe(vec![so_far, as_expr]));
            }

            exprs.push(next_expr);
        }

        Ok(Expr::pipe(exprs))
    }

    /// Parse the pattern part of an `as` binding.
    /// Called after "as" has been consumed.
    fn parse_as_pattern(&mut self, expr: Expr) -> Result<Expr, ParseError> {
        match self.peek() {
            Some('$') => {
                // Simple variable binding: `$var`
                self.next();
                let var = self.parse_ident()?;
                self.skip_ws();
                self.expect('|')?;
                self.skip_ws();
                let body = self.parse_pipe_expr()?;
                Ok(Expr::As {
                    expr: Box::new(expr),
                    var,
                    body: Box::new(body),
                })
            }
            Some('{') | Some('[') => {
                // Pattern destructuring: `{key: $var}` or `[$first, $second]`
                let pattern = self.parse_pattern()?;
                self.skip_ws();
                self.expect('|')?;
                self.skip_ws();
                let body = self.parse_pipe_expr()?;
                Ok(Expr::AsPattern {
                    expr: Box::new(expr),
                    pattern,
                    body: Box::new(body),
                })
            }
            _ => Err(ParseError::new(
                "expected '$var' or pattern after 'as'",
                self.pos,
            )),
        }
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

    // =========================================================================
    // Module System Parsing
    // =========================================================================

    /// Parse a complete program including module directives.
    fn parse_program(&mut self) -> Result<Program, ParseError> {
        let mut program = Program::default();

        self.skip_ws();

        // Parse optional module declaration
        if self.matches_keyword("module") {
            program.module = Some(self.parse_module_declaration()?);
        }

        // Parse import and include directives
        loop {
            self.skip_ws();
            if self.matches_keyword("import") {
                program.imports.push(self.parse_import()?);
            } else if self.matches_keyword("include") {
                program.includes.push(self.parse_include()?);
            } else {
                break;
            }
        }

        // Parse the main expression (which may include function definitions at module level)
        self.skip_ws();
        if !self.is_eof() {
            program.expr = self.parse_module_body()?;
        }

        Ok(program)
    }

    /// Parse module body - handles standalone function definitions
    /// In module files, `def foo: ...; def bar: ...;` doesn't need a trailing expression
    fn parse_module_body(&mut self) -> Result<Expr, ParseError> {
        self.skip_ws();

        // Collect function definitions at module level
        let mut defs: Vec<(String, Vec<String>, Expr)> = Vec::new();

        // Parse function definitions until we hit something that isn't one
        while self.matches_keyword("def") {
            let (name, params, body) = self.parse_func_def_parts()?;
            defs.push((name, params, body));
            self.skip_ws();
        }

        // Parse the remaining expression (or use Identity if nothing left)
        let tail_expr = if self.is_eof() {
            Expr::Identity
        } else {
            self.parse_expr()?
        };

        // Wrap the tail expression with the function definitions in reverse order
        let mut result = tail_expr;
        for (name, params, body) in defs.into_iter().rev() {
            result = Expr::FuncDef {
                name,
                params,
                body: Box::new(body),
                then: Box::new(result),
            };
        }

        Ok(result)
    }

    /// Parse just the function definition parts (name, params, body) without the "then" clause
    fn parse_func_def_parts(&mut self) -> Result<(String, Vec<String>, Expr), ParseError> {
        self.consume_keyword("def");
        self.skip_ws();

        let name = self.parse_ident()?;
        self.skip_ws();

        // Parse optional parameters
        let params = if self.peek() == Some('(') {
            self.next();
            self.skip_ws();
            let mut params = Vec::new();
            while self.peek() != Some(')') {
                // Parameters can be $var or just var
                if self.peek() == Some('$') {
                    self.next();
                }
                let param = self.parse_ident()?;
                params.push(param);
                self.skip_ws();

                match self.peek() {
                    Some(';') | Some(',') => {
                        self.next();
                        self.skip_ws();
                    }
                    Some(')') => break,
                    _ => {
                        return Err(ParseError::new(
                            "expected ';', ',', or ')' in parameter list",
                            self.pos,
                        ));
                    }
                }
            }
            self.expect(')')?;
            params
        } else {
            Vec::new()
        };

        self.skip_ws();
        self.expect(':')?;
        self.skip_ws();

        // Parse function body
        let body = self.parse_pipe_expr()?;
        self.skip_ws();

        // Expect semicolon
        self.expect(';')?;

        Ok((name, params, body))
    }

    /// Parse module declaration: `module { ... };`
    fn parse_module_declaration(&mut self) -> Result<ModuleMeta, ParseError> {
        self.consume_keyword("module");
        self.skip_ws();

        let metadata = self.parse_metadata_object()?;

        self.skip_ws();
        if self.peek() != Some(';') {
            return Err(ParseError::new(
                "expected ';' after module declaration",
                self.pos,
            ));
        }
        self.next();

        Ok(ModuleMeta { metadata })
    }

    /// Parse import directive: `import "path" as name;` or `import "path" as $name;`
    fn parse_import(&mut self) -> Result<Import, ParseError> {
        self.consume_keyword("import");
        self.skip_ws();

        // Parse the path string
        let path = self.parse_string_literal()?;

        self.skip_ws();

        // Expect 'as'
        if !self.matches_keyword("as") {
            return Err(ParseError::new(
                "expected 'as' in import directive",
                self.pos,
            ));
        }
        self.consume_keyword("as");
        self.skip_ws();

        // Parse the alias (optionally prefixed with $)
        let alias = if self.peek() == Some('$') {
            self.next();
            self.parse_ident()?
        } else {
            self.parse_ident()?
        };

        self.skip_ws();

        // Parse optional metadata
        let metadata = if self.peek() == Some('{') {
            Some(self.parse_metadata_object()?)
        } else {
            None
        };

        self.skip_ws();
        if self.peek() != Some(';') {
            return Err(ParseError::new(
                "expected ';' after import directive",
                self.pos,
            ));
        }
        self.next();

        Ok(Import {
            path,
            alias,
            metadata,
        })
    }

    /// Parse include directive: `include "path";`
    fn parse_include(&mut self) -> Result<Include, ParseError> {
        self.consume_keyword("include");
        self.skip_ws();

        // Parse the path string
        let path = self.parse_string_literal()?;

        self.skip_ws();

        // Parse optional metadata
        let metadata = if self.peek() == Some('{') {
            Some(self.parse_metadata_object()?)
        } else {
            None
        };

        self.skip_ws();
        if self.peek() != Some(';') {
            return Err(ParseError::new(
                "expected ';' after include directive",
                self.pos,
            ));
        }
        self.next();

        Ok(Include { path, metadata })
    }

    /// Parse a metadata object: `{ key: value, ... }`
    fn parse_metadata_object(&mut self) -> Result<BTreeMap<String, MetaValue>, ParseError> {
        if self.peek() != Some('{') {
            return Err(ParseError::new("expected '{' for metadata", self.pos));
        }
        self.next();
        self.skip_ws();

        let mut map = BTreeMap::new();

        while self.peek() != Some('}') {
            // Parse key
            let key = self.parse_ident()?;
            self.skip_ws();

            // Expect ':'
            if self.peek() != Some(':') {
                return Err(ParseError::new("expected ':' in metadata object", self.pos));
            }
            self.next();
            self.skip_ws();

            // Parse value
            let value = self.parse_meta_value()?;
            map.insert(key, value);

            self.skip_ws();

            // Check for comma or end
            if self.peek() == Some(',') {
                self.next();
                self.skip_ws();
            } else if self.peek() != Some('}') {
                return Err(ParseError::new(
                    "expected ',' or '}' in metadata object",
                    self.pos,
                ));
            }
        }

        self.next(); // consume '}'
        Ok(map)
    }

    /// Parse a metadata value (string, number, bool, array, or object).
    fn parse_meta_value(&mut self) -> Result<MetaValue, ParseError> {
        self.skip_ws();

        match self.peek() {
            Some('"') => Ok(MetaValue::String(self.parse_string_literal()?)),
            Some('{') => Ok(MetaValue::Object(self.parse_metadata_object()?)),
            Some('[') => {
                self.next();
                self.skip_ws();
                let mut arr = Vec::new();
                while self.peek() != Some(']') {
                    arr.push(self.parse_meta_value()?);
                    self.skip_ws();
                    if self.peek() == Some(',') {
                        self.next();
                        self.skip_ws();
                    }
                }
                self.next(); // consume ']'
                Ok(MetaValue::Array(arr))
            }
            Some('t') if self.matches_keyword("true") => {
                self.consume_keyword("true");
                Ok(MetaValue::Bool(true))
            }
            Some('f') if self.matches_keyword("false") => {
                self.consume_keyword("false");
                Ok(MetaValue::Bool(false))
            }
            Some(c) if c.is_ascii_digit() || c == '-' => {
                let num_str = self.parse_number_str()?;
                let num: f64 = num_str
                    .parse()
                    .map_err(|_| ParseError::new("invalid number in metadata", self.pos))?;
                Ok(MetaValue::Number(num))
            }
            _ => Err(ParseError::new("invalid metadata value", self.pos)),
        }
    }

    /// Parse a number as a string (for metadata).
    fn parse_number_str(&mut self) -> Result<String, ParseError> {
        let start = self.pos;
        if self.peek() == Some('-') {
            self.next();
        }
        while let Some(c) = self.peek() {
            if c.is_ascii_digit() || c == '.' || c == 'e' || c == 'E' || c == '+' || c == '-' {
                self.next();
            } else {
                break;
            }
        }
        Ok(self.input[start..self.pos].to_string())
    }

    /// Parse a namespaced call: `namespace::func` or `namespace::func(args)`.
    /// Called when we've seen an identifier followed by `::`.
    fn parse_namespaced_call(&mut self, namespace: String) -> Result<Expr, ParseError> {
        // Consume '::'
        self.next();
        self.next();
        self.skip_ws();

        // Parse function name
        let name = self.parse_ident()?;
        self.skip_ws();

        // Parse optional arguments
        let args = if self.peek() == Some('(') {
            self.next();
            self.skip_ws();
            let mut args = Vec::new();
            while self.peek() != Some(')') {
                args.push(self.parse_pipe_expr()?);
                self.skip_ws();
                if self.peek() == Some(';') {
                    self.next();
                    self.skip_ws();
                }
            }
            self.next(); // consume ')'
            args
        } else {
            Vec::new()
        };

        Ok(Expr::NamespacedCall {
            namespace,
            name,
            args,
        })
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
    parse_with_mode(input, ParserMode::Jq)
}

/// Parse a jq expression with a specific parser mode.
///
/// Use `ParserMode::Yq` to allow kebab-case identifiers like `.my-key`.
pub fn parse_with_mode(input: &str, mode: ParserMode) -> Result<Expr, ParseError> {
    let mut parser = Parser::with_mode(input, mode);
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

/// Parse a complete jq program including module directives.
///
/// A jq program can optionally start with module directives:
/// - `module { metadata };` - module metadata declaration
/// - `import "path" as name;` - import a module with a namespace
/// - `include "path";` - include a module's definitions directly
///
/// Followed by the main expression (with optional function definitions).
///
/// # Examples
///
/// ```
/// use succinctly::jq::parse_program;
///
/// // Simple expression (no module directives)
/// let prog = parse_program(".foo").unwrap();
/// assert!(prog.module.is_none());
/// assert!(prog.imports.is_empty());
///
/// // With import directive
/// let prog = parse_program(r#"import "utils" as u; u::double"#).unwrap();
/// assert_eq!(prog.imports.len(), 1);
/// assert_eq!(prog.imports[0].path, "utils");
/// assert_eq!(prog.imports[0].alias, "u");
/// ```
pub fn parse_program(input: &str) -> Result<Program, ParseError> {
    parse_program_with_mode(input, ParserMode::Jq)
}

/// Parse a complete jq program with a specific parser mode.
///
/// Use `ParserMode::Yq` to allow kebab-case identifiers like `.my-key`.
pub fn parse_program_with_mode(input: &str, mode: ParserMode) -> Result<Program, ParseError> {
    let mut parser = Parser::with_mode(input, mode);
    let program = parser.parse_program()?;

    // Ensure we consumed all input
    parser.skip_ws();
    if !parser.is_eof() {
        return Err(ParseError::new(
            format!("unexpected character '{}'", parser.peek().unwrap()),
            parser.pos,
        ));
    }

    Ok(program)
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
        // Note: "foo" now parses as FuncCall{name:"foo", args:[]} - valid syntax for user functions
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

    #[test]
    fn test_quoted_field_access() {
        // Basic quoted field access
        assert_eq!(parse(".\"my-key\"").unwrap(), Expr::Field("my-key".into()));
        assert_eq!(
            parse(".\"with spaces\"").unwrap(),
            Expr::Field("with spaces".into())
        );
        assert_eq!(
            parse(".\"special@chars!\"").unwrap(),
            Expr::Field("special@chars!".into())
        );

        // Empty string key
        assert_eq!(parse(".\"\"").unwrap(), Expr::Field("".into()));

        // Quoted field in chained access
        assert_eq!(
            parse(".foo.\"bar-baz\"").unwrap(),
            Expr::Pipe(vec![
                Expr::Field("foo".into()),
                Expr::Field("bar-baz".into()),
            ])
        );

        // Multiple quoted fields chained
        assert_eq!(
            parse(".\"a-b\".\"c-d\"").unwrap(),
            Expr::Pipe(vec![Expr::Field("a-b".into()), Expr::Field("c-d".into()),])
        );

        // Mix of quoted and unquoted fields
        assert_eq!(
            parse(".foo.\"my-key\".bar").unwrap(),
            Expr::Pipe(vec![
                Expr::Field("foo".into()),
                Expr::Field("my-key".into()),
                Expr::Field("bar".into()),
            ])
        );
    }

    #[test]
    fn test_bracket_string_notation() {
        // Basic bracket string notation
        assert_eq!(
            parse(".[\"my-key\"]").unwrap(),
            Expr::Field("my-key".into())
        );
        assert_eq!(
            parse(".[\"with spaces\"]").unwrap(),
            Expr::Field("with spaces".into())
        );

        // Empty string key
        assert_eq!(parse(".[\"\"]").unwrap(), Expr::Field("".into()));

        // Bracket string in chained access
        assert_eq!(
            parse(".foo[\"bar-baz\"]").unwrap(),
            Expr::Pipe(vec![
                Expr::Field("foo".into()),
                Expr::Field("bar-baz".into()),
            ])
        );

        // Multiple bracket notations chained
        assert_eq!(
            parse(".[\"a-b\"][\"c-d\"]").unwrap(),
            Expr::Pipe(vec![Expr::Field("a-b".into()), Expr::Field("c-d".into()),])
        );

        // Mix of bracket and dot notation
        assert_eq!(
            parse(".foo[\"my-key\"].bar").unwrap(),
            Expr::Pipe(vec![
                Expr::Field("foo".into()),
                Expr::Field("my-key".into()),
                Expr::Field("bar".into()),
            ])
        );
    }

    #[test]
    fn test_optional_quoted_field() {
        // Optional quoted field access
        assert_eq!(
            parse(".\"my-key\"?").unwrap(),
            Expr::Optional(Box::new(Expr::Field("my-key".into())))
        );

        // Optional bracket string notation
        assert_eq!(
            parse(".[\"my-key\"]?").unwrap(),
            Expr::Optional(Box::new(Expr::Field("my-key".into())))
        );
    }

    #[test]
    fn test_yq_mode_kebab_case() {
        // In Yq mode, bare kebab-case identifiers are allowed
        assert_eq!(
            parse_with_mode(".my-key", ParserMode::Yq).unwrap(),
            Expr::Field("my-key".into())
        );
        assert_eq!(
            parse_with_mode(".foo-bar-baz", ParserMode::Yq).unwrap(),
            Expr::Field("foo-bar-baz".into())
        );

        // Chained kebab-case
        assert_eq!(
            parse_with_mode(".my-key.other-key", ParserMode::Yq).unwrap(),
            Expr::Pipe(vec![
                Expr::Field("my-key".into()),
                Expr::Field("other-key".into()),
            ])
        );

        // Mix of kebab-case and regular identifiers
        assert_eq!(
            parse_with_mode(".foo.my-key.bar", ParserMode::Yq).unwrap(),
            Expr::Pipe(vec![
                Expr::Field("foo".into()),
                Expr::Field("my-key".into()),
                Expr::Field("bar".into()),
            ])
        );

        // Kebab-case with array index
        assert_eq!(
            parse_with_mode(".my-key[0]", ParserMode::Yq).unwrap(),
            Expr::Pipe(vec![Expr::Field("my-key".into()), Expr::Index(0),])
        );

        // Kebab-case with optional
        assert_eq!(
            parse_with_mode(".my-key?", ParserMode::Yq).unwrap(),
            Expr::Optional(Box::new(Expr::Field("my-key".into())))
        );
    }

    #[test]
    fn test_jq_mode_kebab_case_is_subtraction() {
        // In Jq mode (default), .foo-bar is parsed as .foo minus bar
        let expr = parse(".foo-bar").unwrap();
        match expr {
            Expr::Arithmetic { left, op, right } => {
                assert_eq!(*left, Expr::Field("foo".into()));
                assert_eq!(op, ArithOp::Sub);
                // 'bar' is parsed as a bare identifier - FuncCall or Var
                match &*right {
                    Expr::FuncCall { name, .. } => {
                        assert_eq!(name, "bar");
                    }
                    Expr::Var(name) => {
                        assert_eq!(name, "bar");
                    }
                    other => panic!("expected FuncCall or Var, got {:?}", other),
                }
            }
            _ => panic!("expected subtraction, got {:?}", expr),
        }
    }

    #[test]
    fn test_comments() {
        // Inline comment at end
        assert_eq!(parse(".foo # comment").unwrap(), Expr::Field("foo".into()));

        // Comment on its own line
        assert_eq!(parse("# comment\n.foo").unwrap(), Expr::Field("foo".into()));

        // Multiple comments
        assert_eq!(
            parse("# first\n.foo # second").unwrap(),
            Expr::Field("foo".into())
        );

        // Comment between expressions
        assert_eq!(
            parse(".foo # get foo\n| .bar # then bar").unwrap(),
            Expr::Pipe(vec![Expr::Field("foo".into()), Expr::Field("bar".into()),])
        );

        // Comment with special characters
        assert_eq!(
            parse(".foo # comment with $pecial ch@rs!").unwrap(),
            Expr::Field("foo".into())
        );

        // Empty comment
        assert_eq!(
            parse(".foo #\n| .bar").unwrap(),
            Expr::Pipe(vec![Expr::Field("foo".into()), Expr::Field("bar".into()),])
        );

        // Comment-only input should fail (no expression)
        assert!(parse("# just a comment").is_err());

        // Hash in string is not a comment
        assert_eq!(
            parse("\"hello # world\"").unwrap(),
            Expr::Literal(Literal::String("hello # world".into()))
        );
    }
}
