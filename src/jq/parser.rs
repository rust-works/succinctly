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
    ArithOp, AssignOp, Builtin, CompareOp, Expr, FormatType, FuncDefBound, Import, Include,
    Literal, MergeFlags, MetaValue, ModuleMeta, NumberKey, ObjectEntry, ObjectKey, Pattern,
    PatternEntry, Program, StringPart,
};
use super::value::{parse_i64_or_f64, NumberRepr};

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
        Self {
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

/// The contents of an index bracket, before a target is attached.
///
/// A constant key (`[0]`, `["k"]`, `[1:3]`, `[]`) is a complete chain element on
/// its own. A computed key is not: `E[K]` evaluates `K` against the input to the
/// *whole* postfix chain, so it needs `E` as an explicit target rather than a
/// preceding position in a flat [`Expr::Pipe`]. Only the caller knows the chain,
/// hence this two-state return. See [`Expr::IndexExpr`].
enum Bracket {
    /// `[]`, `["k"]`, `[0]`, `[1:3]` — carries any `?` wrapper itself.
    Static(Expr),
    /// `[expr]` with a computed key; the caller supplies the target.
    Dynamic { key: Expr, optional: bool },
    /// `[start:end]` with at least one computed bound; the caller supplies
    /// the target. See [`Expr::SliceExpr`].
    DynamicSlice {
        start: Option<Expr>,
        end: Option<Expr>,
        optional: bool,
    },
}

/// Attach a parsed bracket to the postfix chain built so far.
///
/// A dynamic key captures the chain as its `target`, because the key is
/// evaluated against the chain's *input* — `.a[.k]` reads `.k` from the chain
/// input, not from `.a`. A dynamic slice bound is the same rule applied to
/// `.a[.k1:.k2]`.
fn push_bracket(chain: &mut Vec<Expr>, bracket: Bracket) {
    match bracket {
        Bracket::Static(expr) => chain.push(expr),
        Bracket::Dynamic { key, optional } => {
            let target = Expr::pipe(core::mem::take(chain));
            let node = Expr::index_by(target, key);
            chain.push(if optional { node.optional() } else { node });
        }
        Bracket::DynamicSlice {
            start,
            end,
            optional,
        } => {
            let target = Expr::pipe(core::mem::take(chain));
            let node = Expr::slice_by(target, start, end);
            chain.push(if optional { node.optional() } else { node });
        }
    }
}

/// Maximum recursion depth for a single `Pattern` AST (`as $pattern`
/// destructuring, including `?//` alternative patterns). Exceeding it
/// returns a clean [`ParseError`] instead of recursing further -- deeply
/// nested pattern syntax (`{a: {a: {a: ...}}}`) is reachable from query
/// text alone, with no input document needed, so an unguarded recursion is
/// a process-abort hazard fully within a client's control (#1240). `Pattern`
/// is exclusively constructed by [`Parser::parse_pattern`] (see that type's
/// own doc comment), so this bound transitively covers every downstream
/// `Pattern` tree-walker too.
const MAX_PATTERN_DEPTH: usize = 256;

/// Maximum recursion depth for a single `Expr` AST. Exceeding it returns a
/// clean [`ParseError`] instead of recursing further.
///
/// Same hazard as [`MAX_PATTERN_DEPTH`], reached through a different
/// construct (#1156): a filter like `-----...5`, `(((...5...)))` or
/// `try try try ... 5` is buildable from query text alone, with no input
/// document, so an unguarded recursion is a process-abort a client controls
/// outright. Measured on `main` before this guard, `parse()` alone aborted
/// with SIGABRT -- before any evaluation -- at 1835 nested parens in a
/// release build and 366 in a debug one; unary minus and `try` were higher
/// (8272/1157 and 6278/1003). 256 sits below the tightest of those with
/// room to spare, and matches [`MAX_PATTERN_DEPTH`] rather than inventing a
/// second number.
///
/// Guarding [`Parser::parse_primary`] alone is sufficient: it is the single
/// point every nesting construct descends through -- parens re-enter via
/// `parse_expr`, and unary minus and `try` recurse into it directly. Chained
/// binary operators, pipes and comma chains are *not* affected, because
/// those parse with a `while` loop rather than recursion, so a thousand-stage
/// pipe never nests more than one level deep here.
///
/// Real jq fails gracefully rather than crashing on the same inputs
/// (`jq: error: memory exhausted`, exit 3, at around 10000 nested parens),
/// so matching its exact limit is not the point -- not aborting is.
///
/// **This bounds recursion depth, not stack bytes.** 256 is comfortably
/// safe on the 8 MiB main thread the CLI runs on, in debug and release
/// alike. A caller that drives the parser from a much smaller stack can
/// still overflow below this limit: measured in debug on a 2 MiB thread
/// (cargo's own test-harness size), nested parens and object constructors
/// abort at around 96 levels, `try` at around 256, and unary minus not at
/// all. The guard is still a strict improvement there -- without it *every*
/// depth aborts -- but it is not a stack-size guarantee, which is why the
/// tests below pin the limit on an explicitly sized thread rather than
/// relying on whatever the harness provides. [`MAX_PATTERN_DEPTH`] carries
/// the same caveat; the two deliberately share one number.
const MAX_EXPR_DEPTH: usize = 256;

/// Parser state.
struct Parser<'a> {
    input: &'a str,
    pos: usize,
    mode: ParserMode,
    /// Whether yq mode accepts jq-only surface real yq's lexer rejects
    /// (`paths`, `getpath`, `limit`, `gsub`/`scan`/`splits`, etc.), gated
    /// behind `--jq-extensions` (#1512). Ignored in jq mode, which always
    /// accepts this surface.
    jq_extensions: bool,
    /// Current `Pattern` recursion depth; see [`MAX_PATTERN_DEPTH`].
    pattern_depth: usize,
    /// Current `Expr` recursion depth; see [`MAX_EXPR_DEPTH`].
    expr_depth: usize,
}

impl<'a> Parser<'a> {
    #[allow(dead_code)] // STYLE-0005: kept for tests and future use
    fn new(input: &'a str) -> Self {
        Parser {
            input,
            pos: 0,
            mode: ParserMode::Jq,
            jq_extensions: false,
            pattern_depth: 0,
            expr_depth: 0,
        }
    }

    fn with_mode_and_extensions(input: &'a str, mode: ParserMode, jq_extensions: bool) -> Self {
        Parser {
            input,
            pos: 0,
            mode,
            jq_extensions,
            pattern_depth: 0,
            expr_depth: 0,
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
                format!("expected '{expected}', found '{c}'"),
                self.pos,
            )),
            None => Err(ParseError::new(
                format!("expected '{expected}', found end of input"),
                self.pos,
            )),
        }
    }

    /// Consumes and discards any further `; expr` arguments past the
    /// arity real yq's evaluator actually reads, in yq mode only -- shared
    /// by `sub(re; s; flags[, ...])` (#1122) and `split(re; flags[, ...])`
    /// (#1439), the two builtins confirmed live against yq v4.53.3 to have
    /// this same fixed-AST-slot-past-arity-N upstream Go bug shape. Each
    /// caller parses its own required arguments first and calls this only
    /// once positioned at the (possibly absent) first extra `;`; the
    /// parsed-and-discarded expressions are never evaluated, matching how
    /// neither builtin's yq-mode evaluator arm reads past its own last
    /// used argument either. A no-op in jq mode, and a no-op if there is
    /// no further `;` to consume -- callers don't need their own mode
    /// check before calling this.
    fn parse_yq_arity_leniency_tail(&mut self) -> Result<(), ParseError> {
        if self.mode == ParserMode::Yq {
            while self.peek() == Some(';') {
                self.next();
                self.skip_ws();
                self.parse_expr()?;
                self.skip_ws();
            }
        }
        Ok(())
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

    /// Real yq's lexer has no token for jq-only surface like `paths`,
    /// `getpath`, `limit`, `gsub`/`scan`/`splits`, etc. -- gated behind
    /// `--jq-extensions` (#1512) so yq mode matches that rejection by
    /// default, the same way `?//` is gated to jq mode only in
    /// `parse_pattern_alternatives` below. Call sites must invoke this
    /// before consuming the keyword, so a rejected keyword's position is
    /// still where the caller expects.
    fn reject_unless_jq_extensions(&self, name: &str) -> Result<(), ParseError> {
        if self.mode == ParserMode::Yq && !self.jq_extensions {
            return Err(ParseError::new(
                format!(
                    "\"{name}\" is not part of yq's syntax; pass --jq-extensions to enable \
                     succinctly's jq-compatible builtin surface"
                ),
                self.pos,
            ));
        }
        Ok(())
    }

    /// Unlike [`reject_unless_jq_extensions`], `--jq-extensions` never
    /// rescues this rejection: `input`/`inputs`/`input_line_number` (#1507)
    /// need real per-document driver-loop coordination yq mode's
    /// cursor-native document loop doesn't have (see
    /// `input_builtins_unsupported_in_yq_mode`, `src/jq/eval.rs`), unlike
    /// every other jq-only builtin the flag actually unlocks. Routing them
    /// through `reject_unless_jq_extensions` instead of this function once
    /// reopened the exact bug #1507 fixed, one layer down: passing the flag
    /// let the keyword parse, so an *unreached* branch (`if false then
    /// input else . end`) went right back to silently succeeding, since the
    /// eval-time-only dispatch check in `eval.rs` never runs for a branch
    /// that's never evaluated. Rejecting unconditionally here, regardless
    /// of the flag, is reachability-independent by construction -- the same
    /// property that made the flag-gated fix work for every *other* jq-only
    /// builtin in the first place.
    fn reject_in_yq_mode(&self, name: &str) -> Result<(), ParseError> {
        if self.mode == ParserMode::Yq {
            return Err(ParseError::new(
                format!("{name} is not supported in yq mode"),
                self.pos,
            ));
        }
        Ok(())
    }

    /// Scan a run of yq merge-flag characters (`+ ? n d c`, any order,
    /// repeats allowed) starting at the current position. Matches real yq's
    /// lexer, which greedily consumes these characters with no regard for
    /// what follows (e.g. `*nan` tokenizes as flag `n` + leftover `an`, a
    /// lex error in real yq too — not a jq `nan` literal).
    fn scan_merge_flags(&mut self) -> MergeFlags {
        let mut flags = MergeFlags::default();
        while let Some(c) = self.peek() {
            match c {
                '+' => flags.append_arrays = true,
                '?' => flags.only_existing = true,
                'n' => flags.only_new = true,
                'd' => flags.deep_merge_arrays = true,
                'c' => flags.clobber_tags = true,
                _ => break,
            }
            self.next();
        }
        flags
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
                    format!("expected identifier, found '{c}'"),
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
        if self.peek() == Some('.') {
            // Look ahead to ensure it's not `..` (recursive descent)
            if self.peek_str(2) != ".." {
                self.next(); // consume the dot

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
        if matches!(self.peek(), Some('e' | 'E')) {
            self.next();

            // Optional sign
            if matches!(self.peek(), Some('+' | '-')) {
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
        let Some(repr) = parse_i64_or_f64(num_str) else {
            return Err(ParseError::new("invalid number", start));
        };
        // #1035: keep the literal's own source spelling (e.g. `1.500`,
        // `1e2`) instead of immediately collapsing it to a freshly-formatted
        // f64/i64 -- matches how document-parsed numbers already preserve
        // their spelling via `OwnedValue::NumberLiteral`. Only for
        // RFC-8259-valid spellings, though: jq's own filter grammar is
        // looser than JSON's (`1.`/`007` are valid jq number tokens jq
        // itself reformats to `1`/`7`), and echoing one of those verbatim
        // through `NumberLiteral` would leak invalid JSON out through
        // `@json`/string interpolation -- `from_number_bytes` gates on the
        // identical check for document numbers, for the identical reason
        // (see its own doc comment).
        if crate::json::validate::is_valid_number(num_str.as_bytes()) {
            Ok(Literal::NumberLiteral(repr, num_str.to_string()))
        } else {
            Ok(match repr {
                NumberRepr::Int(i) => Literal::int(i),
                NumberRepr::Float(f) => Literal::float(f),
            })
        }
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
                            let expr = self.parse_expr()?;
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
                                format!("invalid escape sequence '\\{c}'"),
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
                                format!("invalid escape sequence '\\{c}'"),
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

    /// Parse a bracket expression: `[0]`, `[]`, `[1:3]`, `["key"]`, `[$k]`, etc.
    /// This is for indexing, NOT array construction.
    ///
    /// Bracket contents parse at comma precedence, matching jq's `'[' Exp ']'`,
    /// so `.[1,2]` is a two-output generator. A key that folds to a constant
    /// becomes [`Expr::Field`] or [`Expr::Index`] (see [`Self::fold_index_key`])
    /// and stays a self-contained chain element; anything else is returned as
    /// [`Bracket::Dynamic`] for the caller to attach a target to.
    fn parse_index_bracket(&mut self) -> Result<Bracket, ParseError> {
        self.expect('[')?;
        self.skip_ws();

        // Empty brackets = iterate
        if self.peek() == Some(']') {
            self.next();
            return Ok(Bracket::Static(Expr::Iterate));
        }

        // Check for slice starting with ':'
        if self.peek() == Some(':') {
            self.next();
            self.skip_ws();

            if self.peek() == Some(']') {
                // `[:]` - full slice, returns the whole array as a single value
                self.next();
                return Ok(Bracket::Static(Expr::Slice {
                    start: None,
                    end: None,
                    start_key: None,
                    end_key: None,
                }));
            }

            // `[:n]` - slice from start to n
            let end = self.parse_slice_bound()?;
            self.skip_ws();
            self.expect(']')?;
            return Ok(Self::finish_slice(None, Some(end)));
        }

        // Parse the bracket contents as a full expression. `:` can never be
        // consumed by an expression here — object construction needs `{`, and a
        // namespaced call needs `::` — so it reliably marks a slice.
        let key = self.parse_expr()?;
        self.skip_ws();

        match self.peek() {
            Some(']') => {
                self.next();
                Ok(match Self::fold_index_key(&key) {
                    Some(folded) => Bracket::Static(folded),
                    None => Bracket::Dynamic {
                        key,
                        optional: false,
                    },
                })
            }
            Some(':') => {
                // `[n:]` or `[n:m]` - slice.
                self.next();
                self.skip_ws();

                if self.peek() == Some(']') {
                    // `[n:]` - slice from n to end
                    self.next();
                    Ok(Self::finish_slice(Some(key), None))
                } else {
                    // `[n:m]` - slice from n to m
                    let second = self.parse_slice_bound()?;
                    self.skip_ws();
                    self.expect(']')?;
                    Ok(Self::finish_slice(Some(key), Some(second)))
                }
            }
            Some(c) => Err(ParseError::new(
                format!("expected ']' or ':', found '{c}'"),
                self.pos,
            )),
            None => Err(ParseError::new(
                "expected ']' or ':', found end of input",
                self.pos,
            )),
        }
    }

    /// Decide whether a slice's bounds are both constant (the existing
    /// [`Expr::Slice`] fast path) or need runtime
    /// evaluation ([`Expr::SliceExpr`], attached to a target by the caller —
    /// see [`Bracket::DynamicSlice`]). Each bound folds independently, so
    /// `.[1:.k]` (one literal, one dynamic) still becomes dynamic.
    ///
    /// #1326: a bound also carries its own `NumberKey` through
    /// [`Self::fold_slice_bound`] when it's float-spelled, into
    /// [`Expr::Slice`]'s own `start_key`/`end_key` -- each bound
    /// independently, the same rule [`Self::fold_index_key`] applies to an
    /// index's own `key` (#1088).
    fn finish_slice(start: Option<Expr>, end: Option<Expr>) -> Bracket {
        let start_folded = start.as_ref().and_then(Self::fold_slice_bound);
        let end_folded = end.as_ref().and_then(Self::fold_slice_bound);
        let start_dynamic = start.is_some() && start_folded.is_none();
        let end_dynamic = end.is_some() && end_folded.is_none();
        if start_dynamic || end_dynamic {
            Bracket::DynamicSlice {
                start,
                end,
                optional: false,
            }
        } else {
            let (start_i64, start_key) = start_folded.unzip();
            let (end_i64, end_key) = end_folded.unzip();
            Bracket::Static(Expr::Slice {
                start: start_i64,
                end: end_i64,
                start_key: start_key.flatten(),
                end_key: end_key.flatten(),
            })
        }
    }

    /// Fold a constant bracket key into the static chain element it denotes.
    ///
    /// This is what keeps `.foo.bar[0]` on exactly the AST it has always had:
    /// only a genuinely computed key becomes an [`Expr::IndexExpr`]. `null`,
    /// `true` and `{}` deliberately do *not* fold — they must reach the
    /// evaluator so the error is jq's runtime `Cannot index …`, not a parse
    /// error (issue #360).
    fn fold_index_key(key: &Expr) -> Option<Expr> {
        match key {
            Expr::Paren(inner) => Self::fold_index_key(inner),
            Expr::Literal(Literal::String(s)) => Some(Expr::Field(s.clone())),
            Expr::Literal(Literal::Int(i)) => Some(Expr::Index { idx: *i, key: None }),
            // Every number written in filter source arrives as
            // `Literal::NumberLiteral` (#1035), so this one arm covers the
            // whole grammar; `Literal::Float`/`Literal::Int` are for
            // internally-synthesized literals, which are spliced into an
            // already-parsed AST and so never reach the parser's own fold.
            //
            // `.[1.0]` is an integer index; `.[1.7]` is not, and must go
            // through the evaluator to truncate the way jq does. The upper
            // bound is a strict `<`, not `<=`: `i64::MAX as f64` rounds *up*
            // to `2^63` (`i64::MAX` isn't exactly representable as `f64`),
            // so a `<=` check let `.[2^63]` through and `as i64` silently
            // saturated it to `i64::MAX` -- one past what was actually
            // written (#1061). `i64::MIN as f64` has no such rounding
            // (`-2^63` is an exact power of two), so the lower bound stays
            // `>=`.
            //
            // #1062: the literal's `NumberRepr` is already parsed and
            // carried on the node itself, so this reads it directly instead
            // of re-running `parse_i64_or_f64` on the source text.
            //
            // #1088: the *set* of keys that fold here is unchanged -- only
            // the variant is. An `Int` repr still folds to a plain
            // [`Expr::Index`], since its own source text renders identically
            // to the `i64` and the hot `.foo.bar[0]` path must not move. A
            // `Float` repr carries its spelling in [`Expr::Index`]'s own
            // `key` instead, because jq echoes it back verbatim:
            // `path(.[2.00])` is `[2.00]` and `path(.[1e10])` is `[1E+10]`.
            // `idx` is the identical `i64` either way.
            Expr::Literal(Literal::NumberLiteral(repr, text)) => match *repr {
                NumberRepr::Int(i) => Some(Expr::Index { idx: i, key: None }),
                NumberRepr::Float(f)
                    if f.fract() == 0.0 && f >= i64::MIN as f64 && f < i64::MAX as f64 =>
                {
                    Some(Expr::Index {
                        idx: f as i64,
                        key: Some(NumberKey::Literal(f, text.as_str().into())),
                    })
                }
                NumberRepr::Float(_) => None,
            },
            // #1035: a jq-mode negative float/exponent index/slice-bound
            // literal (e.g. `.[-1.0]`) parses as `-1 * <positive literal>`
            // (see `parse_primary_inner`'s negative-literal split), not a
            // bare `Literal` -- see through that specific shape too, or
            // every such key silently loses this fast path and downgrades
            // to a runtime `IndexExpr`/`DynamicSlice`.
            //
            // #1061: negating `fold_index_key(right)`'s own result can
            // overflow `i64` -- not just for the float/exponent spelling of
            // `i64::MIN`'s magnitude (`2^63`, one past `i64::MAX`, so it
            // reaches here rather than folding directly), but for *any*
            // producer of `Expr::index(i64::MIN)`, including a bare
            // `Literal::Int(i64::MIN)` on `right` (reachable from ordinary
            // jq source: `left` need not come from the negative-literal
            // split at all, just any `-1 * <literal>` spelling, and a
            // leading-zero integer like `-01` parses to `Literal::Int(-1)`
            // directly rather than jq's number grammar rejecting it).
            // `checked_neg` covers every such producer uniformly: it falls
            // through to `None` (a runtime `IndexExpr`, which computes the
            // same value correctly via ordinary float arithmetic) instead
            // of panicking in a debug build or silently wrapping in release.
            Expr::Arithmetic {
                op: ArithOp::Mul(_),
                left,
                right,
            } if matches!(**left, Expr::Literal(Literal::Int(-1))) => {
                match Self::fold_index_key(right)? {
                    Expr::Index { idx, key } => idx.checked_neg().map(|idx| Expr::Index {
                        idx,
                        // #1088: negation is exactly the operation that
                        // destroys jq's number-literal preservation, so a
                        // key that got this far drops to a bare `f64` --
                        // which is *why* `path(.[-1.0])` is `[-1]` while
                        // `path(.[1.0])` is `[1.0]`. Nothing index-specific
                        // is happening; `jq -n '-1.0'` already prints `-1`.
                        // See [`NumberKey`]'s own doc comment.
                        //
                        // A key-*less* index stays key-less: `map` leaves
                        // `None` alone rather than synthesizing a spelling
                        // for a plain `.[-1]`. Keeping those two directions
                        // distinct is the whole content of this arm.
                        //
                        // Negating the truncated `idx` and truncating the
                        // negated value agree, because the truncation is
                        // toward zero.
                        key: key.map(|k| NumberKey::Float(-k.value())),
                    }),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// Fold a slice bound to the `i64` every navigation step uses, seeing
    /// through parens -- and, alongside it, the [`NumberKey`] a
    /// float-spelled bound needs to keep its own spelling in `path()`
    /// output (#1326, following #1088's identical rule for an index's own
    /// `key`).
    ///
    /// [`Self::fold_index_key`] answers both halves at once: the `i64` every
    /// navigation step uses, and the key to preserve alongside it, which is
    /// `None` for an integer-spelled bound. `finish_slice` hands each bound's
    /// key straight to [`Expr::Slice`]'s matching field.
    fn fold_slice_bound(key: &Expr) -> Option<(i64, Option<NumberKey>)> {
        match Self::fold_index_key(key)? {
            Expr::Index { idx, key } => Some((idx, key)),
            _ => None,
        }
    }

    /// Parse one slice bound.
    ///
    /// Both bounds go through here so they accept the same spellings. Parsing
    /// only the *first* as an expression would let `.[(1):3]` compile while
    /// `.[1:(3)]` did not — an asymmetry with no grammar behind it. The
    /// caller (`Self::finish_slice`) decides whether the parsed expression
    /// folds to a literal or needs runtime evaluation.
    fn parse_slice_bound(&mut self) -> Result<Expr, ParseError> {
        self.parse_expr()
    }

    /// Parse an index bracket and check for optional marker.
    fn parse_index_bracket_with_optional(&mut self) -> Result<Bracket, ParseError> {
        let bracket = self.parse_index_bracket()?;
        self.skip_ws();
        if self.peek() == Some('?') {
            self.next();
            Ok(match bracket {
                Bracket::Static(expr) => Bracket::Static(Expr::Optional(expr.into())),
                Bracket::Dynamic { key, .. } => Bracket::Dynamic {
                    key,
                    optional: true,
                },
                Bracket::DynamicSlice { start, end, .. } => Bracket::DynamicSlice {
                    start,
                    end,
                    optional: true,
                },
            })
        } else {
            Ok(bracket)
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
        let inner = self.parse_expr()?;
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
                let key_expr = self.parse_expr()?;
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
                // jq's `ExpD`, not `Exp`: the `,` here separates entries, so a
                // value must stop at it or `{a: 1, b: 2}` reads `1, b` as one
                // value. Use `(...)` to fan a value out: `{a: (1,2)}`.
                self.parse_pipe_no_comma()?
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
                        format!("expected ',' or '}}', found '{c}'"),
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

    /// Parse a primary expression (atoms and parenthesized expressions), then
    /// check for a trailing `?` (jq's postfix `try` shorthand), which applies
    /// to any Term - not just path expressions. Field/bracket access already
    /// consume their own narrower `?` inline (see
    /// `parse_index_bracket_with_optional` and the dot-field branch below),
    /// so by the time control reaches here any such `?` is already gone;
    /// this only wraps a `?` still left over the whole term.
    fn parse_primary(&mut self) -> Result<Expr, ParseError> {
        self.expr_depth += 1;
        let result = if self.expr_depth > MAX_EXPR_DEPTH {
            Err(ParseError::new(
                format!("expression nesting exceeds depth limit of {MAX_EXPR_DEPTH}"),
                self.pos,
            ))
        } else {
            self.parse_primary_optional()
        };
        // Decremented on the error path too, so a caller that recovers from a
        // `ParseError` higher up does not see a permanently inflated depth.
        self.expr_depth -= 1;
        result
    }

    /// Account for left-nesting built by a binary-operator loop.
    ///
    /// `parse_additive`, `parse_and` and their siblings iterate rather than
    /// recurse, so [`Parser::parse_primary`]'s guard never sees them -- but
    /// each iteration still wraps the accumulated `left` in another node, so
    /// `1 + 1 + ... + 1` builds a chain-length-deep tree the evaluator and
    /// `Drop` have to walk later. Measured on `main`, that aborted with
    /// SIGABRT at 6206 terms in a release build and 596 in a debug one --
    /// the same hazard class as the prefix constructs, just reached by
    /// iteration, and trivially constructible either way.
    ///
    /// `parse_comparison` deliberately has no counter: jq's comparison
    /// operators are non-associative, so it parses at most one of them and
    /// cannot build a chain at all.
    ///
    /// `extra` is added to the current nesting depth rather than replacing
    /// it, so a chain written inside parentheses is charged for both.
    fn check_expr_nesting(&self, extra: usize) -> Result<(), ParseError> {
        if self.expr_depth + extra > MAX_EXPR_DEPTH {
            return Err(ParseError::new(
                format!("expression nesting exceeds depth limit of {MAX_EXPR_DEPTH}"),
                self.pos,
            ));
        }
        Ok(())
    }

    /// The real `parse_primary` body, entered only through the depth-checked
    /// wrapper above -- every recursive descent goes back through
    /// `self.parse_primary()`, not this function, so the counter sees every
    /// nesting level.
    fn parse_primary_optional(&mut self) -> Result<Expr, ParseError> {
        let expr = self.parse_primary_inner()?;
        self.skip_ws();
        if self.peek() == Some('?') {
            self.next();
            Ok(Expr::Optional(Box::new(expr)))
        } else {
            Ok(expr)
        }
    }

    fn parse_primary_inner(&mut self) -> Result<Expr, ParseError> {
        self.skip_ws();

        match self.peek() {
            // Parenthesized expression
            Some('(') => {
                self.next();
                let expr = self.parse_expr()?;
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
                    match lit {
                        // #1035: jq's own grammar treats a leading `-` as
                        // unary negation, not part of the number token --
                        // confirmed against jq 1.7.1, where `-1.500`/`-1e2`
                        // print `-1.5`/`-100` (fidelity collapses through
                        // the implied negation) while `1.500`/`1e2` print
                        // unchanged. yq is the opposite: real yq never
                        // collapses a negative literal's fidelity either
                        // (`-1.500`/`-1e2` print back unchanged), so this
                        // rewrite is jq-only. A plain negative *integer* has
                        // no alternate spelling to lose (no fraction, no
                        // exponent), so folding `-` into the token there
                        // stays unobservable in jq mode either way -- and
                        // it's what preserves succinctly's
                        // i64::MIN-exact-literal capability (see
                        // `test_large_integer_literal_falls_back_to_float`),
                        // which routing through arithmetic would degrade to
                        // a lossy float, unlike jq's double-only model.
                        Literal::NumberLiteral(repr, text)
                            if self.mode == ParserMode::Jq && text.contains(['.', 'e', 'E']) =>
                        {
                            // Multiply rather than subtract from zero:
                            // `0.0 - 0.0` is IEEE-754 positive zero,
                            // silently losing the sign of `-0.0`/`-0e0`
                            // (jq itself prints `-0` for these); `-1.0 *
                            // 0.0` correctly preserves it.
                            //
                            // #1062: `repr` is `text`'s (negative) parsed
                            // value; the split-off literal's own text has
                            // the sign stripped, so its repr is `-repr`
                            // (the positive magnitude), not a re-parse.
                            // `Int` is unreachable here: the guard above
                            // requires `.`/`e`/`E` in `text`, which
                            // `parse_i64_or_f64` never resolves to an
                            // `Int` -- `unreachable!()` rather than a
                            // silently-wrapping fallback, so a future
                            // change to that dispatch rule that breaks the
                            // invariant fails loudly here instead of
                            // producing a wrong-signed value.
                            let stripped_repr = match repr {
                                NumberRepr::Int(_) => unreachable!(
                                    "a '.'/'e'/'E'-containing literal never parses as NumberRepr::Int"
                                ),
                                NumberRepr::Float(f) => NumberRepr::Float(-f),
                            };
                            Ok(Expr::Arithmetic {
                                op: ArithOp::Mul(MergeFlags::default()),
                                left: Box::new(Expr::Literal(Literal::Int(-1))),
                                right: Box::new(Expr::Literal(Literal::NumberLiteral(
                                    stripped_repr,
                                    text[1..].to_string(),
                                ))),
                            })
                        }
                        lit => Ok(Expr::Literal(lit)),
                    }
                } else {
                    // Unary minus: negate the following expression.
                    //
                    // `Expr::Negate`, not `(0 - expr)` (#1056): IEEE-754
                    // `0.0 - 0.0` is positive zero, silently losing the sign
                    // of a zero-valued operand (`-.a` on `0.0` dropped jq's
                    // `-0` to plain `0`). Also not `(-1 * expr)` (an earlier
                    // draft of this fix, reverted after code review): `*`
                    // has its own string-repetition and null-passthrough
                    // semantics that have nothing to do with negation, so
                    // `-"abc"`/`-null` silently returned `null` instead of
                    // erroring.
                    self.next(); // consume '-'
                    let operand = self.parse_primary()?;
                    Ok(Expr::Negate(Box::new(operand)))
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

                // Check for `.[...]` (index/iterate). A leading bracket indexes
                // the identity, so a dynamic key gets `.` as its target.
                if self.peek() == Some('[') {
                    let bracket = self.parse_index_bracket_with_optional()?;
                    let mut chain = vec![Expr::Identity];
                    push_bracket(&mut chain, bracket);
                    let first = match chain.len() {
                        // A static bracket leaves `[Identity, elem]`; the
                        // Identity is redundant as a chain head.
                        2 => chain.pop().unwrap(),
                        _ => Expr::pipe(chain),
                    };
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
                let keyword_start = self.pos;
                if self.matches_keyword("null") {
                    self.consume_keyword("null");
                    self.zero_arity_or_wrong_arity_call(keyword_start, Expr::Literal(Literal::Null))
                } else if self.matches_keyword("true") {
                    self.consume_keyword("true");
                    self.zero_arity_or_wrong_arity_call(
                        keyword_start,
                        Expr::Literal(Literal::Bool(true)),
                    )
                } else if self.matches_keyword("false") {
                    self.consume_keyword("false");
                    self.zero_arity_or_wrong_arity_call(
                        keyword_start,
                        Expr::Literal(Literal::Bool(false)),
                    )
                } else if self.matches_keyword("not") {
                    self.consume_keyword("not");
                    self.zero_arity_or_wrong_arity_call(keyword_start, Expr::Not)
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
                    self.reject_unless_jq_extensions("limit")?;
                    self.parse_limit_expr()
                } else if self.matches_keyword("until") {
                    self.parse_until_expr()
                } else if self.matches_keyword("while") {
                    self.parse_while_expr()
                } else if self.matches_keyword("repeat") {
                    self.parse_repeat_expr()
                } else if self.matches_keyword("range") {
                    self.reject_unless_jq_extensions("range")?;
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
                    // #2110: a following `(<args>)` means this was actually
                    // a wrong-arity call, not a bare builtin reference --
                    // `zero_arity_or_wrong_arity_call` rewinds and re-parses
                    // it as one instead of falling into postfix parsing,
                    // which would just reject the `(` as a stray token; it
                    // also applies postfix itself (e.g. `env.PATH`,
                    // `keys[0]`) on whichever `Expr` it ends up returning.
                    self.zero_arity_or_wrong_arity_call(keyword_start, Expr::Builtin(builtin))
                } else {
                    // Phase 9: Try to parse as function call
                    self.parse_func_call_or_error()
                }
            }

            Some(c) => Err(ParseError::new(
                format!("unexpected character '{c}', expected expression"),
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
        let cond = self.parse_expr()?;
        self.skip_ws();

        // Expect 'then'
        if !self.matches_keyword("then") {
            return Err(ParseError::new("expected 'then'", self.pos));
        }
        self.consume_keyword("then");
        self.skip_ws();

        // Parse then branch
        let then_branch = self.parse_expr()?;
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

            let cond = self.parse_expr()?;
            self.skip_ws();

            if !self.matches_keyword("then") {
                return Err(ParseError::new("expected 'then'", self.pos));
            }
            self.consume_keyword("then");
            self.skip_ws();

            let then_branch = self.parse_expr()?;
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

            let else_branch = self.parse_expr()?;
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
            let msg_expr = self.parse_expr()?;
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

        // Parse binding pattern -- a bare `$var`, a full destructuring
        // pattern (#1201), or `?//`-separated alternatives (#1365; the
        // evaluator side retries an errored UPDATE with the accumulator
        // rolled back to its pre-UPDATE value, see `eval_reduce`).
        let patterns = self.parse_pattern_alternatives()?;

        // Parse (init; update)
        self.expect('(')?;
        self.skip_ws();
        let init = self.parse_expr()?;
        self.skip_ws();
        self.expect(';')?;
        self.skip_ws();
        let update = self.parse_expr()?;
        self.skip_ws();
        self.expect(')')?;

        Ok(Expr::Reduce {
            input: Box::new(input),
            patterns,
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

        // Parse binding pattern -- a bare `$var`, a full destructuring
        // pattern (#1201), or `?//`-separated alternatives (#1365; see
        // `eval_reduce`'s doc comment for the accumulator-rollback retry
        // semantics `eval_foreach` shares).
        let patterns = self.parse_pattern_alternatives()?;

        // Parse (init; update[; extract])
        self.expect('(')?;
        self.skip_ws();
        let init = self.parse_expr()?;
        self.skip_ws();
        self.expect(';')?;
        self.skip_ws();
        let update = self.parse_expr()?;
        self.skip_ws();

        // Optional extract expression
        let extract = if self.peek() == Some(';') {
            self.next();
            self.skip_ws();
            Some(Box::new(self.parse_expr()?))
        } else {
            None
        };
        self.skip_ws();
        self.expect(')')?;

        Ok(Expr::Foreach {
            input: Box::new(input),
            patterns,
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
        // `n` deliberately stays restricted to non-comma: real jq's `limit`
        // is defined with the `$n` parameter convention, where a
        // comma-valued `n` re-invokes the whole builtin once per output.
        // That fanout isn't implemented here, so accepting a comma would
        // parse but silently misbehave — worse than today's parse error.
        let n = self.parse_pipe_no_comma()?;
        self.skip_ws();
        self.expect(';')?;
        self.skip_ws();
        let expr = self.parse_expr()?;
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
        let cond = self.parse_expr()?;
        self.skip_ws();
        self.expect(';')?;
        self.skip_ws();
        let update = self.parse_expr()?;
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
        let cond = self.parse_expr()?;
        self.skip_ws();
        self.expect(';')?;
        self.skip_ws();
        let update = self.parse_expr()?;
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
        let expr = self.parse_expr()?;
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
            let expr = self.parse_expr()?;
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
            let expr = self.parse_expr()?;
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
        let first = self.parse_expr()?;
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
        let second = self.parse_expr()?;
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
        let step = self.parse_expr()?;
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
        self.pattern_depth += 1;
        let result = if self.pattern_depth > MAX_PATTERN_DEPTH {
            Err(ParseError::new(
                format!("pattern nesting exceeds depth limit of {MAX_PATTERN_DEPTH}"),
                self.pos,
            ))
        } else {
            self.parse_pattern_inner()
        };
        self.pattern_depth -= 1;
        result
    }

    /// The real `parse_pattern` body, entered only through the depth-checked
    /// wrapper above -- every recursive call goes back through
    /// `self.parse_pattern()`, not this function, so the depth counter sees
    /// every nesting level.
    fn parse_pattern_inner(&mut self) -> Result<Pattern, ParseError> {
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
                    // `{$a}` shorthand: real jq desugars a bare `$var` entry
                    // to `a: $var` -- the variable's own name doubles as
                    // both the key to match and the binding pattern, with
                    // no `:` at all. Checked before the identifier/string-
                    // key branch below since `$` can't start either of
                    // those.
                    //
                    // `{$a: Pattern}` (#1204): the same key, but with an
                    // explicit `:` and its own pattern to further
                    // destructure the matched value -- `$a` still binds the
                    // whole value under its own name (same as the bare
                    // shorthand), *and* `Pattern` binds again, independently,
                    // against that same value. Desugared here into *two*
                    // ordinary entries sharing one key, rather than a new
                    // `Pattern` variant: `Pattern::Object`'s own evaluation
                    // already re-fetches the value once per entry regardless
                    // of key uniqueness (real jq's own object patterns
                    // already tolerate a repeated key the same way --
                    // confirmed live), so two same-key entries reproduce
                    // `{$a: Pattern}`'s exact semantics with no new
                    // AST shape or evaluator match arm needed. Peeking past
                    // the identifier for `:` is required to tell the two
                    // `$`-led shapes apart.
                    if self.peek() == Some('$') {
                        self.next();
                        let name = self.parse_ident()?;
                        self.skip_ws();
                        if self.peek() == Some(':') {
                            self.next();
                            self.skip_ws();
                            let nested = self.parse_pattern()?;
                            entries.push(PatternEntry {
                                key: name.clone(),
                                pattern: Pattern::Var(name.clone()),
                            });
                            entries.push(PatternEntry {
                                key: name,
                                pattern: nested,
                            });
                        } else {
                            entries.push(PatternEntry {
                                key: name.clone(),
                                pattern: Pattern::Var(name),
                            });
                        }
                    } else {
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
                    }

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
        let body = self.parse_expr()?;

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
                        Some(';' | ',') => {
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
        let body = self.parse_expr()?;
        self.skip_ws();

        // Expect semicolon
        self.expect(';')?;
        self.skip_ws();

        // Parse the rest of the expression where this function is in scope
        let then = self.parse_expr()?;

        Ok(Expr::FuncDef {
            name,
            params,
            body: Box::new(body),
            then: Box::new(then),
            bound: FuncDefBound::default(),
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
                    // Each `;`-separated slot is a full expression — comma
                    // included (#155) — matching jq's `Arg: Exp` grammar.
                    let arg = self.parse_expr()?;
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
                    format!("unknown format '@{format_name}'"),
                    format_start,
                ));
            }
        };

        Ok(Expr::Format(format_type))
    }

    /// A zero-arity keyword (`null`/`true`/`false`/`not`, or anything
    /// [`try_parse_builtin`] recognizes) immediately followed by `(<args>)`
    /// (at least one argument), **in jq mode**, is a wrong-arity call, not a
    /// stray token -- real jq resolves it at compile time via name/arity
    /// resolution ("`length/1 is not defined`", the same class #1473/#2037
    /// already give a genuinely-undefined name), not as a raw parser
    /// rejection of the `(` (#2110). Whitespace before `(` is allowed too,
    /// matching real jq (`length (1)` errors identically to `length(1)`).
    ///
    /// yq mode never rewinds, regardless of argument count: real yq (v4.53.3)
    /// has no name/arity resolution vocabulary at all -- confirmed live,
    /// `null(1)` and `length(1)` both give the identical generic "bad
    /// expression, please check expression syntax" -- so jq's "X/N is not
    /// defined" wording would be a *new*, jq-flavored divergence in yq mode,
    /// not a fix. yq mode keeps the pre-#2110 generic parse-error rejection
    /// unchanged.
    ///
    /// An *empty* `()` is deliberately excluded from the rewind: real jq's
    /// grammar has no zero-argument parenthesized call shape at all, for
    /// any name -- confirmed live, `def f: 1; f()` is a syntax error
    /// ("unexpected ')'") in real jq exactly like `length()`/
    /// `undefinedname()` are, not a name/arity resolution error. Rewinding
    /// `length()` the same way as `length(1)` would ask
    /// [`Self::parse_func_call_or_error`] to answer a question it isn't --
    /// that function's own empty-arg-list leniency (`FuncCall { args: [] }`)
    /// exists for the ordinary "just call `foo`" case, not this one, and for
    /// any name [`resolve.rs`]'s builtin roster tracks at arity 0 for an
    /// unrelated reason (a real jq builtin succinctly doesn't implement,
    /// e.g. `cbrt`) it would let compile-time resolution wrongly accept the
    /// call, deferring the failure to a confusing runtime "undefined
    /// function" error instead. Leaving `()` alone here restores exactly
    /// the pre-#2110 behavior (a raw parser rejection of the stray `(`) --
    /// not a match for jq's own wording, but not a new divergence either,
    /// and never worse than what `main` already did.
    ///
    /// `start_pos` must be the position *before* the keyword was consumed:
    /// on a hit, this rewinds there and re-parses the same text through
    /// [`Self::parse_func_call_or_error`], which treats it as an ordinary
    /// (name, args) call for the resolver to reject by arity -- exactly as
    /// if the keyword recognizer had never matched at all. `if`-style syntax
    /// keywords are not `Expr::FuncCall`-shaped in real jq either (`if(1)`
    /// is a genuine syntax error there, not `if/1 is not defined`), so only
    /// call this from a site that already recognized a true zero-arity
    /// *builtin* or literal (`null`/`true`/`false`/`not`, or anything
    /// [`try_parse_builtin`] recognizes) -- never from `if`/`try`/`reduce`/
    /// `def`/etc.
    ///
    /// Postfix (`.field`, `[idx]`) is applied uniformly to whichever `Expr`
    /// this returns, matching the sibling call site inside
    /// [`Self::parse_primary_inner`] that already needed it for a bare
    /// builtin (`env.PATH`) -- earlier revisions of this fix only wrapped
    /// that one site, which left a wrong-arity `null`/`true`/`false`/`not`
    /// call followed by postfix syntax (`null(1).foo`) with the *same* raw
    /// parser rejection this whole function exists to replace, and, worse,
    /// broke postfix chaining after a legitimately name/arity-*resolved*
    /// call too (`def null(x): {foo:x}; null(1).foo` must parse -- real jq
    /// accepts it and returns `1` -- not just fail cleanly).
    ///
    /// [`try_parse_builtin`]: Self::try_parse_builtin
    /// [`resolve.rs`]: super::resolve
    fn zero_arity_or_wrong_arity_call(
        &mut self,
        start_pos: usize,
        parsed: Expr,
    ) -> Result<Expr, ParseError> {
        let after_keyword = self.pos;
        self.skip_ws();
        if self.peek() == Some('(') {
            let paren_pos = self.pos;
            self.next();
            self.skip_ws();
            let has_arg = self.peek() != Some(')');
            self.pos = paren_pos;
            // yq mode: real yq has no name/arity resolution vocabulary at
            // all -- confirmed live (yq v4.53.3), `null(1)` and `length(1)`
            // both give the identical generic "bad expression, please
            // check expression syntax" regardless of name or arity. Real
            // jq's own "X/N is not defined" wording this rewind produces
            // would be a *new*, jq-flavored divergence in yq mode, not a
            // fix -- so only jq mode rewinds; yq mode falls through to the
            // pre-#2110 generic parse-error rejection below, unchanged.
            if has_arg && self.mode == ParserMode::Jq {
                self.pos = start_pos;
                let call = self.parse_func_call_or_error()?;
                return self.parse_postfix(call);
            }
        }
        self.pos = after_keyword;
        self.parse_postfix(parsed)
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
            // In Yq mode, `keys` returns document order (like yq) instead of sorted (like jq)
            // Use `keys_unsorted` in jq mode to get document order
            return Ok(Some(if self.mode == ParserMode::Yq {
                Builtin::KeysUnsorted
            } else {
                Builtin::Keys
            }));
        }

        // has(expr) - takes an argument
        if self.matches_keyword("has") {
            self.consume_keyword("has");
            self.skip_ws();
            self.expect('(')?;
            self.skip_ws();
            let arg = self.parse_expr()?;
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
            let cond = self.parse_expr()?;
            self.skip_ws();
            self.expect(')')?;
            return Ok(Some(Builtin::Select(Box::new(cond))));
        }
        if self.matches_keyword("empty") {
            self.consume_keyword("empty");
            return Ok(Some(Builtin::Empty));
        }

        // Process control (#791)
        if self.matches_keyword("halt_error") {
            // Check halt_error before halt so "halt_error" isn't parsed as
            // "halt" followed by a stray "_error".
            self.consume_keyword("halt_error");
            self.skip_ws();
            if self.peek() == Some('(') {
                self.next();
                self.skip_ws();
                let code = self.parse_expr()?;
                self.skip_ws();
                self.expect(')')?;
                return Ok(Some(Builtin::HaltErrorCode(Box::new(code))));
            }
            return Ok(Some(Builtin::HaltError));
        }
        if self.matches_keyword("halt") {
            self.consume_keyword("halt");
            return Ok(Some(Builtin::Halt));
        }
        if self.matches_keyword("stderr") {
            self.consume_keyword("stderr");
            return Ok(Some(Builtin::Stderr));
        }

        // Map functions
        if self.matches_keyword("map_values") {
            // Check map_values before map
            self.consume_keyword("map_values");
            self.skip_ws();
            self.expect('(')?;
            self.skip_ws();
            let f = self.parse_expr()?;
            self.skip_ws();
            self.expect(')')?;
            return Ok(Some(Builtin::MapValues(Box::new(f))));
        }
        if self.matches_keyword("map") {
            self.consume_keyword("map");
            self.skip_ws();
            self.expect('(')?;
            self.skip_ws();
            let f = self.parse_expr()?;
            self.skip_ws();
            self.expect(')')?;
            return Ok(Some(Builtin::Map(Box::new(f))));
        }

        // Reduction functions (no arguments)
        // Note: If followed by '(', these should be parsed as user-defined function calls
        if self.matches_keyword("add") && !self.peek_after_keyword_is_paren("add") {
            self.reject_unless_jq_extensions("add")?;
            self.consume_keyword("add");
            return Ok(Some(Builtin::Add));
        }
        // any, any(cond), any(gen; cond)
        if self.matches_keyword("any") {
            self.consume_keyword("any");
            self.skip_ws();
            if self.peek() == Some('(') {
                self.next();
                self.skip_ws();
                let f = self.parse_expr()?;
                self.skip_ws();
                if self.peek() == Some(';') {
                    self.next();
                    self.skip_ws();
                    let cond = self.parse_expr()?;
                    self.skip_ws();
                    self.expect(')')?;
                    return Ok(Some(Builtin::AnyCond(Box::new(f), Box::new(cond))));
                }
                self.expect(')')?;
                return Ok(Some(Builtin::AnyF(Box::new(f))));
            }
            return Ok(Some(Builtin::Any));
        }
        // all, all(cond), all(gen; cond)
        if self.matches_keyword("all") {
            self.consume_keyword("all");
            self.skip_ws();
            if self.peek() == Some('(') {
                self.next();
                self.skip_ws();
                let f = self.parse_expr()?;
                self.skip_ws();
                if self.peek() == Some(';') {
                    self.next();
                    self.skip_ws();
                    let cond = self.parse_expr()?;
                    self.skip_ws();
                    self.expect(')')?;
                    return Ok(Some(Builtin::AllCond(Box::new(f), Box::new(cond))));
                }
                self.expect(')')?;
                return Ok(Some(Builtin::AllF(Box::new(f))));
            }
            return Ok(Some(Builtin::All));
        }
        if self.matches_keyword("min_by") {
            // Check min_by before min
            self.reject_unless_jq_extensions("min_by")?;
            self.consume_keyword("min_by");
            self.skip_ws();
            self.expect('(')?;
            self.skip_ws();
            let f = self.parse_expr()?;
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
            self.reject_unless_jq_extensions("max_by")?;
            self.consume_keyword("max_by");
            self.skip_ws();
            self.expect('(')?;
            self.skip_ws();
            let f = self.parse_expr()?;
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
            let obj = self.parse_expr()?;
            self.skip_ws();
            self.expect(')')?;
            return Ok(Some(Builtin::In(Box::new(obj))));
        }

        // IN(s) - true if any output of s equals the current value
        // IN(src; s) - true if any output of src equals any output of s
        if self.matches_keyword("IN") {
            self.reject_unless_jq_extensions("IN")?;
            self.consume_keyword("IN");
            self.skip_ws();
            self.expect('(')?;
            self.skip_ws();
            let first = self.parse_expr()?;
            self.skip_ws();
            if self.peek() == Some(';') {
                self.next();
                self.skip_ws();
                let s = self.parse_expr()?;
                self.skip_ws();
                self.expect(')')?;
                return Ok(Some(Builtin::UpperInSrc(Box::new(first), Box::new(s))));
            }
            self.expect(')')?;
            return Ok(Some(Builtin::UpperIn(Box::new(first))));
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
            self.reject_unless_jq_extensions("ltrimstr")?;
            self.consume_keyword("ltrimstr");
            self.skip_ws();
            self.expect('(')?;
            self.skip_ws();
            let s = self.parse_expr()?;
            self.skip_ws();
            self.expect(')')?;
            return Ok(Some(Builtin::Ltrimstr(Box::new(s))));
        }
        if self.matches_keyword("rtrimstr") {
            self.reject_unless_jq_extensions("rtrimstr")?;
            self.consume_keyword("rtrimstr");
            self.skip_ws();
            self.expect('(')?;
            self.skip_ws();
            let s = self.parse_expr()?;
            self.skip_ws();
            self.expect(')')?;
            return Ok(Some(Builtin::Rtrimstr(Box::new(s))));
        }
        if self.matches_keyword("startswith") {
            self.reject_unless_jq_extensions("startswith")?;
            self.consume_keyword("startswith");
            self.skip_ws();
            self.expect('(')?;
            self.skip_ws();
            let s = self.parse_expr()?;
            self.skip_ws();
            self.expect(')')?;
            return Ok(Some(Builtin::Startswith(Box::new(s))));
        }
        if self.matches_keyword("endswith") {
            self.reject_unless_jq_extensions("endswith")?;
            self.consume_keyword("endswith");
            self.skip_ws();
            self.expect('(')?;
            self.skip_ws();
            let s = self.parse_expr()?;
            self.skip_ws();
            self.expect(')')?;
            return Ok(Some(Builtin::Endswith(Box::new(s))));
        }
        // Check splits before split since split is a prefix of splits
        if self.matches_keyword("splits") {
            self.reject_unless_jq_extensions("splits")?;
            self.consume_keyword("splits");
            self.skip_ws();
            self.expect('(')?;
            self.skip_ws();
            let re = self.parse_expr()?;
            self.skip_ws();
            if self.peek() == Some(';') {
                self.next(); // consume ';'
                self.skip_ws();
                let flags = self.parse_expr()?;
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
            let s = self.parse_expr()?;
            self.skip_ws();
            if self.peek() == Some(';') {
                self.next(); // consume ';'
                self.skip_ws();
                let flags = self.parse_expr()?;
                self.skip_ws();
                // yq mode accepts (and ignores) any further `; expr`
                // arguments at arity 3+ -- confirmed live against yq
                // v4.53.3: `split("x"; "y"; "z")` parses and behaves
                // identically to arity 2 (#1439). See
                // `parse_yq_arity_leniency_tail`'s own doc comment for why
                // this is shared with `sub` below rather than a second
                // hand-copied loop.
                self.parse_yq_arity_leniency_tail()?;
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
            let re = self.parse_expr()?;
            self.skip_ws();
            if self.peek() == Some(';') {
                self.next(); // consume ';'
                self.skip_ws();
                let flags = self.parse_expr()?;
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
            let re = self.parse_expr()?;
            self.skip_ws();
            if self.peek() == Some(';') {
                self.next(); // consume ';'
                self.skip_ws();
                let flags = self.parse_expr()?;
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
            let re = self.parse_expr()?;
            self.skip_ws();
            self.expect(';')?;
            self.skip_ws();
            let replacement = self.parse_expr()?;
            self.skip_ws();
            if self.peek() == Some(';') {
                self.next(); // consume ';'
                self.skip_ws();
                let flags = self.parse_expr()?;
                self.skip_ws();
                // yq mode accepts (and ignores) any further `; expr`
                // arguments -- confirmed live against yq v4.53.3: arity 4+
                // parses and behaves identically to arity 3 (#1122), unlike
                // jq, where `sub/4` is a hard "not defined" compile error.
                // See `parse_yq_arity_leniency_tail`'s own doc comment for
                // why this is shared with `split` above rather than a
                // second hand-copied loop (#1439 review).
                self.parse_yq_arity_leniency_tail()?;
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
            self.reject_unless_jq_extensions("gsub")?;
            self.consume_keyword("gsub");
            self.skip_ws();
            self.expect('(')?;
            self.skip_ws();
            let re = self.parse_expr()?;
            self.skip_ws();
            self.expect(';')?;
            self.skip_ws();
            let replacement = self.parse_expr()?;
            self.skip_ws();
            if self.peek() == Some(';') {
                self.next(); // consume ';'
                self.skip_ws();
                let flags = self.parse_expr()?;
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
            self.reject_unless_jq_extensions("scan")?;
            self.consume_keyword("scan");
            self.skip_ws();
            self.expect('(')?;
            self.skip_ws();
            let re = self.parse_expr()?;
            self.skip_ws();
            if self.peek() == Some(';') {
                self.next(); // consume ';'
                self.skip_ws();
                let flags = self.parse_expr()?;
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
            let s = self.parse_expr()?;
            self.skip_ws();
            self.expect(')')?;
            return Ok(Some(Builtin::Join(Box::new(s))));
        }
        if self.matches_keyword("contains") {
            self.consume_keyword("contains");
            self.skip_ws();
            self.expect('(')?;
            self.skip_ws();
            let b = self.parse_expr()?;
            self.skip_ws();
            self.expect(')')?;
            return Ok(Some(Builtin::Contains(Box::new(b))));
        }
        if self.matches_keyword("inside") {
            self.reject_unless_jq_extensions("inside")?;
            self.consume_keyword("inside");
            self.skip_ws();
            self.expect('(')?;
            self.skip_ws();
            let b = self.parse_expr()?;
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
                let depth = self.parse_expr()?;
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
            let f = self.parse_expr()?;
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
            let f = self.parse_expr()?;
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
            let f = self.parse_expr()?;
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
            let f = self.parse_expr()?;
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
            self.reject_unless_jq_extensions("implode")?;
            self.consume_keyword("implode");
            return Ok(Some(Builtin::Implode));
        }
        if self.matches_keyword("test") {
            self.consume_keyword("test");
            self.skip_ws();
            self.expect('(')?;
            self.skip_ws();
            let re = self.parse_expr()?;
            self.skip_ws();
            if self.peek() == Some(';') {
                self.next(); // consume ';'
                self.skip_ws();
                let flags = self.parse_expr()?;
                self.skip_ws();
                self.expect(')')?;
                return Ok(Some(Builtin::TestFlags(Box::new(re), Box::new(flags))));
            }
            self.expect(')')?;
            return Ok(Some(Builtin::Test(Box::new(re))));
        }
        if self.matches_keyword("indices") {
            self.reject_unless_jq_extensions("indices")?;
            self.consume_keyword("indices");
            self.skip_ws();
            self.expect('(')?;
            self.skip_ws();
            let s = self.parse_expr()?;
            self.skip_ws();
            self.expect(')')?;
            return Ok(Some(Builtin::Indices(Box::new(s))));
        }
        // Check index before rindex since rindex contains "index"
        if self.matches_keyword("rindex") {
            self.reject_unless_jq_extensions("rindex")?;
            self.consume_keyword("rindex");
            self.skip_ws();
            self.expect('(')?;
            self.skip_ws();
            let s = self.parse_expr()?;
            self.skip_ws();
            self.expect(')')?;
            return Ok(Some(Builtin::Rindex(Box::new(s))));
        }
        if self.matches_keyword("index") {
            self.reject_unless_jq_extensions("index")?;
            self.consume_keyword("index");
            self.skip_ws();
            self.expect('(')?;
            self.skip_ws();
            let s = self.parse_expr()?;
            self.skip_ws();
            self.expect(')')?;
            return Ok(Some(Builtin::Index(Box::new(s))));
        }
        // INDEX(idx_expr) - build an object keyed by idx_expr from `.[]`
        // INDEX(stream; idx_expr) - build an object keyed by idx_expr from stream
        if self.matches_keyword("INDEX") {
            self.reject_unless_jq_extensions("INDEX")?;
            self.consume_keyword("INDEX");
            self.skip_ws();
            self.expect('(')?;
            self.skip_ws();
            let first = self.parse_expr()?;
            self.skip_ws();
            if self.peek() == Some(';') {
                self.next();
                self.skip_ws();
                let idx_expr = self.parse_expr()?;
                self.skip_ws();
                self.expect(')')?;
                return Ok(Some(Builtin::UpperIndexStream(
                    Box::new(first),
                    Box::new(idx_expr),
                )));
            }
            self.expect(')')?;
            return Ok(Some(Builtin::UpperIndex(Box::new(first))));
        }
        if self.matches_keyword("tojsonstream") {
            self.consume_keyword("tojsonstream");
            return Ok(Some(Builtin::ToJsonStream));
        }
        if self.matches_keyword("fromjsonstream") {
            self.consume_keyword("fromjsonstream");
            return Ok(Some(Builtin::FromJsonStream));
        }
        if self.matches_keyword("tostream") {
            self.reject_unless_jq_extensions("tostream")?;
            self.consume_keyword("tostream");
            return Ok(Some(Builtin::ToStream));
        }
        if self.matches_keyword("fromstream") {
            self.reject_unless_jq_extensions("fromstream")?;
            self.consume_keyword("fromstream");
            self.skip_ws();
            self.expect('(')?;
            self.skip_ws();
            let f = self.parse_expr()?;
            self.skip_ws();
            self.expect(')')?;
            return Ok(Some(Builtin::FromStream(Box::new(f))));
        }
        if self.matches_keyword("truncate_stream") {
            self.reject_unless_jq_extensions("truncate_stream")?;
            self.consume_keyword("truncate_stream");
            self.skip_ws();
            self.expect('(')?;
            self.skip_ws();
            let f = self.parse_expr()?;
            self.skip_ws();
            self.expect(')')?;
            return Ok(Some(Builtin::TruncateStream(Box::new(f))));
        }
        if self.matches_keyword("getpath") {
            self.reject_unless_jq_extensions("getpath")?;
            self.consume_keyword("getpath");
            self.skip_ws();
            self.expect('(')?;
            self.skip_ws();
            let path = self.parse_expr()?;
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
                let f = self.parse_expr()?;
                self.skip_ws();
                if self.peek() == Some(';') {
                    self.next();
                    self.skip_ws();
                    let cond = self.parse_expr()?;
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
            self.reject_unless_jq_extensions("walk")?;
            self.consume_keyword("walk");
            self.skip_ws();
            self.expect('(')?;
            self.skip_ws();
            let f = self.parse_expr()?;
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
            let expr = self.parse_expr()?;
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
                let expr = self.parse_expr()?;
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
                let n = self.parse_expr()?;
                self.skip_ws();
                self.expect(')')?;
                return Ok(Some(Builtin::ParentN(Box::new(n))));
            }
            // parent (no-arg) - immediate parent
            return Ok(Some(Builtin::Parent));
        }
        // leaf_paths - must check before paths
        if self.matches_keyword("leaf_paths") {
            self.reject_unless_jq_extensions("leaf_paths")?;
            self.consume_keyword("leaf_paths");
            return Ok(Some(Builtin::LeafPaths));
        }
        // paths or paths(filter)
        if self.matches_keyword("paths") {
            self.reject_unless_jq_extensions("paths")?;
            self.consume_keyword("paths");
            self.skip_ws();
            if self.peek() == Some('(') {
                self.next();
                self.skip_ws();
                let filter = self.parse_expr()?;
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
            let path = self.parse_expr()?;
            self.skip_ws();
            self.expect(';')?;
            self.skip_ws();
            let value = self.parse_expr()?;
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
            let paths = self.parse_expr()?;
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
            let path = self.parse_expr()?;
            self.skip_ws();
            self.expect(')')?;
            return Ok(Some(Builtin::Del(Box::new(path))));
        }

        // Phase 10: Math Functions
        if self.matches_keyword("floor") {
            self.reject_unless_jq_extensions("floor")?;
            self.consume_keyword("floor");
            return Ok(Some(Builtin::Floor));
        }
        if self.matches_keyword("ceil") {
            self.reject_unless_jq_extensions("ceil")?;
            self.consume_keyword("ceil");
            return Ok(Some(Builtin::Ceil));
        }
        if self.matches_keyword("round") {
            self.reject_unless_jq_extensions("round")?;
            self.consume_keyword("round");
            return Ok(Some(Builtin::Round));
        }
        if self.matches_keyword("sqrt") {
            self.reject_unless_jq_extensions("sqrt")?;
            self.consume_keyword("sqrt");
            return Ok(Some(Builtin::Sqrt));
        }
        if self.matches_keyword("fabs") {
            self.reject_unless_jq_extensions("fabs")?;
            self.consume_keyword("fabs");
            return Ok(Some(Builtin::Fabs));
        }
        // Logarithmic - check log10 and log2 before log
        if self.matches_keyword("log10") {
            self.reject_unless_jq_extensions("log10")?;
            self.consume_keyword("log10");
            return Ok(Some(Builtin::Log10));
        }
        if self.matches_keyword("log2") {
            self.reject_unless_jq_extensions("log2")?;
            self.consume_keyword("log2");
            return Ok(Some(Builtin::Log2));
        }
        if self.matches_keyword("log") {
            self.reject_unless_jq_extensions("log")?;
            self.consume_keyword("log");
            return Ok(Some(Builtin::Log));
        }
        // Exponential - check exp10 and exp2 before exp
        if self.matches_keyword("exp10") {
            self.reject_unless_jq_extensions("exp10")?;
            self.consume_keyword("exp10");
            return Ok(Some(Builtin::Exp10));
        }
        if self.matches_keyword("exp2") {
            self.reject_unless_jq_extensions("exp2")?;
            self.consume_keyword("exp2");
            return Ok(Some(Builtin::Exp2));
        }
        if self.matches_keyword("exp") {
            self.reject_unless_jq_extensions("exp")?;
            self.consume_keyword("exp");
            return Ok(Some(Builtin::Exp));
        }
        // pow(base; exp)
        if self.matches_keyword("pow") {
            self.reject_unless_jq_extensions("pow")?;
            self.consume_keyword("pow");
            self.skip_ws();
            self.expect('(')?;
            self.skip_ws();
            let base = self.parse_expr()?;
            self.skip_ws();
            self.expect(';')?;
            self.skip_ws();
            let exp = self.parse_expr()?;
            self.skip_ws();
            self.expect(')')?;
            return Ok(Some(Builtin::Pow(Box::new(base), Box::new(exp))));
        }
        // Trigonometric functions - check longer names first
        if self.matches_keyword("sinh") {
            self.reject_unless_jq_extensions("sinh")?;
            self.consume_keyword("sinh");
            return Ok(Some(Builtin::Sinh));
        }
        if self.matches_keyword("cosh") {
            self.reject_unless_jq_extensions("cosh")?;
            self.consume_keyword("cosh");
            return Ok(Some(Builtin::Cosh));
        }
        if self.matches_keyword("tanh") {
            self.reject_unless_jq_extensions("tanh")?;
            self.consume_keyword("tanh");
            return Ok(Some(Builtin::Tanh));
        }
        if self.matches_keyword("asinh") {
            self.reject_unless_jq_extensions("asinh")?;
            self.consume_keyword("asinh");
            return Ok(Some(Builtin::Asinh));
        }
        if self.matches_keyword("acosh") {
            self.reject_unless_jq_extensions("acosh")?;
            self.consume_keyword("acosh");
            return Ok(Some(Builtin::Acosh));
        }
        if self.matches_keyword("atanh") {
            self.reject_unless_jq_extensions("atanh")?;
            self.consume_keyword("atanh");
            return Ok(Some(Builtin::Atanh));
        }
        // atan2(y; x) - must check before atan
        if self.matches_keyword("atan2") {
            self.reject_unless_jq_extensions("atan2")?;
            self.consume_keyword("atan2");
            self.skip_ws();
            self.expect('(')?;
            self.skip_ws();
            let y = self.parse_expr()?;
            self.skip_ws();
            self.expect(';')?;
            self.skip_ws();
            let x = self.parse_expr()?;
            self.skip_ws();
            self.expect(')')?;
            return Ok(Some(Builtin::Atan2(Box::new(y), Box::new(x))));
        }
        if self.matches_keyword("asin") {
            self.reject_unless_jq_extensions("asin")?;
            self.consume_keyword("asin");
            return Ok(Some(Builtin::Asin));
        }
        if self.matches_keyword("acos") {
            self.reject_unless_jq_extensions("acos")?;
            self.consume_keyword("acos");
            return Ok(Some(Builtin::Acos));
        }
        if self.matches_keyword("atan") {
            self.reject_unless_jq_extensions("atan")?;
            self.consume_keyword("atan");
            return Ok(Some(Builtin::Atan));
        }
        if self.matches_keyword("sin") {
            self.reject_unless_jq_extensions("sin")?;
            self.consume_keyword("sin");
            return Ok(Some(Builtin::Sin));
        }
        if self.matches_keyword("cos") {
            self.reject_unless_jq_extensions("cos")?;
            self.consume_keyword("cos");
            return Ok(Some(Builtin::Cos));
        }
        if self.matches_keyword("tan") {
            self.reject_unless_jq_extensions("tan")?;
            self.consume_keyword("tan");
            return Ok(Some(Builtin::Tan));
        }

        // Phase 10: Number Classification & Constants
        // Check isinfinite, isnan, isnormal, isfinite before infinite, nan
        if self.matches_keyword("isinfinite") {
            self.reject_unless_jq_extensions("isinfinite")?;
            self.consume_keyword("isinfinite");
            return Ok(Some(Builtin::IsInfinite));
        }
        if self.matches_keyword("isnan") {
            self.reject_unless_jq_extensions("isnan")?;
            self.consume_keyword("isnan");
            return Ok(Some(Builtin::IsNan));
        }
        if self.matches_keyword("isnormal") {
            self.reject_unless_jq_extensions("isnormal")?;
            self.consume_keyword("isnormal");
            return Ok(Some(Builtin::IsNormal));
        }
        if self.matches_keyword("isfinite") {
            self.reject_unless_jq_extensions("isfinite")?;
            self.consume_keyword("isfinite");
            return Ok(Some(Builtin::IsFinite));
        }
        if self.matches_keyword("infinite") {
            self.reject_unless_jq_extensions("infinite")?;
            self.consume_keyword("infinite");
            return Ok(Some(Builtin::Infinite));
        }
        if self.matches_keyword("nan") {
            self.reject_unless_jq_extensions("nan")?;
            self.consume_keyword("nan");
            return Ok(Some(Builtin::Nan));
        }

        // Phase 10: Debug
        if self.matches_keyword("debug") {
            self.reject_unless_jq_extensions("debug")?;
            self.consume_keyword("debug");
            self.skip_ws();
            if self.peek() == Some('(') {
                self.next();
                self.skip_ws();
                let msg = self.parse_expr()?;
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
            self.reject_unless_jq_extensions("bsearch")?;
            self.consume_keyword("bsearch");
            self.skip_ws();
            self.expect('(')?;
            self.skip_ws();
            let x = self.parse_expr()?;
            self.skip_ws();
            self.expect(')')?;
            return Ok(Some(Builtin::BSearch(Box::new(x))));
        }

        // Phase 10: Object functions
        // modulemeta - real jq's builtin is arity 0 (takes the module name
        // via `.`, not a paren'd argument); succinctly previously required
        // `modulemeta(...)` here, inverted from jq's actual grammar (#2035).
        if self.matches_keyword("modulemeta") {
            self.consume_keyword("modulemeta");
            return Ok(Some(Builtin::ModuleMeta));
        }

        // pick(keys) - yq: select only specified keys from object/array
        if self.matches_keyword("pick") {
            self.consume_keyword("pick");
            self.skip_ws();
            self.expect('(')?;
            self.skip_ws();
            let keys = self.parse_expr()?;
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
            let keys = self.parse_expr()?;
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

        // line_comment - yq: return trailing same-line comment text, or "" (#710)
        if self.matches_keyword("line_comment") {
            self.consume_keyword("line_comment");
            return Ok(Some(Builtin::LineComment));
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

        // file_index / fileIndex / fi - succinctly extension (#715): return
        // 0-indexed origin file position within an `--eval-all` combined
        // evaluation. `fileIndex` (camelCase) is real yq's own spelling;
        // `file_index` mirrors this codebase's `document_index` convention.
        if self.matches_keyword("file_index") {
            self.consume_keyword("file_index");
            return Ok(Some(Builtin::FileIndex));
        }
        if self.matches_keyword("fileIndex") {
            self.consume_keyword("fileIndex");
            return Ok(Some(Builtin::FileIndex));
        }
        if self.matches_keyword("fi") {
            self.consume_keyword("fi");
            return Ok(Some(Builtin::FileIndex));
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
            self.reject_unless_jq_extensions("abs")?;
            self.consume_keyword("abs");
            return Ok(Some(Builtin::Abs));
        }
        if self.matches_keyword("builtins") {
            self.consume_keyword("builtins");
            return Ok(Some(Builtin::Builtins));
        }
        if self.matches_keyword("inputs") {
            self.reject_in_yq_mode("inputs")?;
            self.consume_keyword("inputs");
            return Ok(Some(Builtin::Inputs));
        }
        if self.matches_keyword("input_line_number") {
            self.reject_in_yq_mode("input_line_number")?;
            self.consume_keyword("input_line_number");
            return Ok(Some(Builtin::InputLineNumber));
        }
        if self.matches_keyword("input") {
            self.reject_in_yq_mode("input")?;
            self.consume_keyword("input");
            return Ok(Some(Builtin::Input));
        }

        // Phase 13: Iteration control
        // limit(n; expr) - output at most n values from expr
        if self.matches_keyword("limit") {
            self.consume_keyword("limit");
            self.skip_ws();
            self.expect('(')?;
            self.skip_ws();
            let n = self.parse_pipe_no_comma()?;
            self.skip_ws();
            self.expect(';')?;
            self.skip_ws();
            let expr = self.parse_expr()?;
            self.skip_ws();
            self.expect(')')?;
            return Ok(Some(Builtin::Limit(Box::new(n), Box::new(expr))));
        }

        // skip(n; expr) - skip first n outputs from expr
        if self.matches_keyword("skip") {
            self.reject_unless_jq_extensions("skip")?;
            self.consume_keyword("skip");
            self.skip_ws();
            self.expect('(')?;
            self.skip_ws();
            // `n` deliberately stays restricted to non-comma — same rationale
            // as `limit`'s `n`: real jq's `$n` parameter convention isn't
            // implemented here.
            let n = self.parse_pipe_no_comma()?;
            self.skip_ws();
            self.expect(';')?;
            self.skip_ws();
            let expr = self.parse_expr()?;
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
                let expr = self.parse_expr()?;
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
                let expr = self.parse_expr()?;
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
            self.reject_unless_jq_extensions("nth")?;
            self.consume_keyword("nth");
            self.skip_ws();
            self.expect('(')?;
            self.skip_ws();
            // `n` deliberately stays restricted to non-comma — same rationale
            // as `limit`'s `n` above: real jq's `$n` parameter convention
            // (per-output fanout) isn't implemented here.
            let n = self.parse_pipe_no_comma()?;
            self.skip_ws();
            if self.peek() == Some(';') {
                self.next();
                self.skip_ws();
                let expr = self.parse_expr()?;
                self.skip_ws();
                self.expect(')')?;
                return Ok(Some(Builtin::NthStream(Box::new(n), Box::new(expr))));
            }
            self.expect(')')?;
            // No-arg nth(n) is already handled by Phase 5 Builtin::Nth
            return Ok(Some(Builtin::Nth(Box::new(n))));
        }

        // Note: `range(...)` is handled earlier in parse_primary via
        // parse_range_expr (Expr::Range), so it never reaches this builtin path.

        // isempty(expr) - returns true if expr produces no outputs
        if self.matches_keyword("isempty") {
            self.reject_unless_jq_extensions("isempty")?;
            self.consume_keyword("isempty");
            self.skip_ws();
            self.expect('(')?;
            self.skip_ws();
            let expr = self.parse_expr()?;
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
        //
        // #1907: `gmtime`/`localtime`/`mktime` are real yq's lexer rejecting
        // these outright (confirmed live, v4.53.3: "invalid input text
        // ...") -- jq-only surface, gated like the neighboring
        // `strftime`/`strptime` below, not real yq's own
        // `from_unix`/`to_unix`/`tz(...)` (Phase 21 below, left ungated
        // since real yq does accept those).
        if self.matches_keyword("gmtime") {
            self.reject_unless_jq_extensions("gmtime")?;
            self.consume_keyword("gmtime");
            return Ok(Some(Builtin::Gmtime));
        }
        if self.matches_keyword("localtime") {
            self.reject_unless_jq_extensions("localtime")?;
            self.consume_keyword("localtime");
            return Ok(Some(Builtin::Localtime));
        }
        if self.matches_keyword("mktime") {
            self.reject_unless_jq_extensions("mktime")?;
            self.consume_keyword("mktime");
            return Ok(Some(Builtin::Mktime));
        }
        if self.matches_keyword("strftime") {
            self.reject_unless_jq_extensions("strftime")?;
            self.consume_keyword("strftime");
            self.skip_ws();
            self.expect('(')?;
            self.skip_ws();
            let fmt = self.parse_expr()?;
            self.skip_ws();
            self.expect(')')?;
            return Ok(Some(Builtin::Strftime(Box::new(fmt))));
        }
        if self.matches_keyword("strptime") {
            self.reject_unless_jq_extensions("strptime")?;
            self.consume_keyword("strptime");
            self.skip_ws();
            self.expect('(')?;
            self.skip_ws();
            let fmt = self.parse_expr()?;
            self.skip_ws();
            self.expect(')')?;
            return Ok(Some(Builtin::Strptime(Box::new(fmt))));
        }
        // #1907: same jq-only gating as gmtime/localtime/mktime above --
        // confirmed live (v4.53.3) real yq's lexer rejects all four of
        // these too.
        if self.matches_keyword("todateiso8601") {
            self.reject_unless_jq_extensions("todateiso8601")?;
            self.consume_keyword("todateiso8601");
            return Ok(Some(Builtin::Todateiso8601));
        }
        if self.matches_keyword("fromdateiso8601") {
            self.reject_unless_jq_extensions("fromdateiso8601")?;
            self.consume_keyword("fromdateiso8601");
            return Ok(Some(Builtin::Fromdateiso8601));
        }
        if self.matches_keyword("todate") {
            self.reject_unless_jq_extensions("todate")?;
            self.consume_keyword("todate");
            return Ok(Some(Builtin::Todate));
        }
        if self.matches_keyword("fromdate") {
            self.reject_unless_jq_extensions("fromdate")?;
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
            let zone = self.parse_expr()?;
            self.skip_ws();
            self.expect(')')?;
            return Ok(Some(Builtin::Tz(Box::new(zone))));
        }

        // Phase 17: Combinations
        if self.matches_keyword("combinations") {
            self.reject_unless_jq_extensions("combinations")?;
            self.consume_keyword("combinations");
            self.skip_ws();
            if self.peek() == Some('(') {
                self.next(); // consume '('
                self.skip_ws();
                let n = self.parse_expr()?;
                self.skip_ws();
                self.expect(')')?;
                return Ok(Some(Builtin::CombinationsN(Box::new(n))));
            }
            return Ok(Some(Builtin::Combinations));
        }

        // Phase 18: Additional math functions
        if self.matches_keyword("trunc") {
            self.reject_unless_jq_extensions("trunc")?;
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
            let file_expr = self.parse_expr()?;
            self.skip_ws();
            self.expect(')')?;
            return Ok(Some(Builtin::Load(Box::new(file_expr))));
        }

        // Phase 23: Position-based navigation (succinctly extension)
        // at_offset(n) - jump to node at byte offset n (0-indexed)
        if self.matches_keyword("at_offset") {
            self.consume_keyword("at_offset");
            self.skip_ws();
            self.expect('(')?;
            self.skip_ws();
            let offset_expr = self.parse_expr()?;
            self.skip_ws();
            self.expect(')')?;
            return Ok(Some(Builtin::AtOffset(Box::new(offset_expr))));
        }

        // at_position(line; col) - jump to node at line/column (1-indexed)
        if self.matches_keyword("at_position") {
            self.consume_keyword("at_position");
            self.skip_ws();
            self.expect('(')?;
            self.skip_ws();
            let line_expr = self.parse_expr()?;
            self.skip_ws();
            self.expect(';')?;
            self.skip_ws();
            let col_expr = self.parse_expr()?;
            self.skip_ws();
            self.expect(')')?;
            return Ok(Some(Builtin::AtPosition(
                Box::new(line_expr),
                Box::new(col_expr),
            )));
        }

        Ok(None)
    }

    /// Check if current character is an expression terminator (ends a primary expression).
    /// This includes structural characters and infix operators.
    fn is_expr_terminator(&self) -> bool {
        match self.peek() {
            // Structural terminators
            Some(',' | ')' | ']' | '}' | '|' | ':' | ';' | '?') => true,
            // Arithmetic operators
            Some('+' | '-' | '*' | '/' | '%') => true,
            // Comparison operators
            Some('=' | '!' | '<' | '>') => true,
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
                        let bracket = self.parse_index_bracket_with_optional()?;
                        push_bracket(&mut chain, bracket);
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
                    let bracket = self.parse_index_bracket_with_optional()?;
                    push_bracket(&mut chain, bracket);
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
        // Each iteration wraps `left` in another node, so this chain's own
        // length is AST depth even though the loop never recurses (#1156).
        let mut chain_depth = 0usize;

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
                Some('*') => {
                    self.next(); // consume '*'
                                 // yq merge-flag suffixes (*+, *?, *n, *d, *c, combinable)
                                 // are only recognized in yq mode — real jq has no such
                                 // syntax, so jq mode leaves them for the next parse step
                                 // to reject, same as today.
                    let flags = if self.mode == ParserMode::Yq {
                        self.scan_merge_flags()
                    } else {
                        MergeFlags::default()
                    };
                    ArithOp::Mul(flags)
                }
                Some('/') => {
                    // Check it's not `//` (alternative operator)
                    if self.peek_str(2) == "//" {
                        break;
                    }
                    self.next();
                    ArithOp::Div
                }
                Some('%') => {
                    self.next();
                    ArithOp::Mod
                }
                _ => break,
            };
            self.skip_ws();
            let right = self.parse_primary()?;
            chain_depth += 1;
            self.check_expr_nesting(chain_depth)?;
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
        // Each iteration wraps `left` in another node, so this chain's own
        // length is AST depth even though the loop never recurses (#1156).
        let mut chain_depth = 0usize;

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
            chain_depth += 1;
            self.check_expr_nesting(chain_depth)?;
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
        // Each iteration wraps `left` in another node, so this chain's own
        // length is AST depth even though the loop never recurses (#1156).
        let mut chain_depth = 0usize;

        loop {
            self.skip_ws();
            if !self.matches_keyword("and") {
                break;
            }
            self.consume_keyword("and");
            self.skip_ws();
            let right = self.parse_comparison()?;
            chain_depth += 1;
            self.check_expr_nesting(chain_depth)?;
            left = Expr::And(Box::new(left), Box::new(right));
        }

        Ok(left)
    }

    /// Parse `or` expressions: `expr or expr`
    fn parse_or(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_and()?;
        // Each iteration wraps `left` in another node, so this chain's own
        // length is AST depth even though the loop never recurses (#1156).
        let mut chain_depth = 0usize;

        loop {
            self.skip_ws();
            if !self.matches_keyword("or") {
                break;
            }
            self.consume_keyword("or");
            self.skip_ws();
            let right = self.parse_and()?;
            chain_depth += 1;
            self.check_expr_nesting(chain_depth)?;
            left = Expr::Or(Box::new(left), Box::new(right));
        }

        Ok(left)
    }

    /// Parse alternative expressions: `expr // expr`
    fn parse_alternative(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_or()?;
        // Each iteration wraps `left` in another node, so this chain's own
        // length is AST depth even though the loop never recurses (#1156).
        let mut chain_depth = 0usize;

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
            chain_depth += 1;
            self.check_expr_nesting(chain_depth)?;
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

        // Check for *= (merge assignment), with optional yq merge-flag
        // suffixes after the '=': *=, *=+, *=?, *=n, *=d, *=c, combinable
        // (e.g. *=+d). Flags come after '=', never before it — `.a *+= .b`
        // is not the append-merge-assign spelling in real yq either.
        if peek2 == "*=" {
            self.next(); // *
            self.next(); // =
            let flags = if self.mode == ParserMode::Yq {
                self.scan_merge_flags()
            } else {
                MergeFlags::default()
            };
            self.skip_ws();
            let value = self.parse_alternative()?;
            return Ok(Expr::CompoundAssign {
                op: AssignOp::Mul(flags),
                path: Box::new(left),
                value: Box::new(value),
            });
        }

        // Check for compound assignments: +=, -=, /=, %=
        if peek2.len() == 2 && peek2.ends_with('=') {
            let op_char = peek2.chars().next().unwrap();
            let assign_op = match op_char {
                '+' => Some(AssignOp::Add),
                '-' => Some(AssignOp::Sub),
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

    /// Parse a complete expression — jq's `Exp`, and this grammar's entry point.
    ///
    /// The precedence order below `Exp` is, loosest first:
    ///
    /// ```text
    /// parse_expr        Exp   := parse_pipe_expr
    /// parse_pipe_expr         := parse_comma_expr ( '|' parse_comma_expr )*
    /// parse_comma_expr        := parse_binding    ( ',' parse_binding    )*
    /// parse_binding           := parse_assignment [ "as" Patterns '|' parse_expr ]
    /// parse_obj_val     ExpD  := parse_binding    ( '|' parse_binding    )*
    /// ```
    ///
    /// `|` is the *loosest* operator and `,` binds tighter, matching jq's
    /// `parser.y`, which declares `%right '|'` before `%left ','`. Having these
    /// the wrong way round made `1,2,3 | . * 2` mean `1, 2, (3 | . * 2)` and
    /// print `1 2 6` instead of `2 4 6` — silent data loss, since every
    /// comma branch but the last lost its transformation (#462).
    fn parse_expr(&mut self) -> Result<Expr, ParseError> {
        self.parse_pipe_expr()
    }

    /// Parse a pipe expression: `stage | stage | ...`, where each stage is a
    /// comma list.
    fn parse_pipe_expr(&mut self) -> Result<Expr, ParseError> {
        let first = self.parse_comma_expr()?;
        self.skip_ws();

        if self.peek() != Some('|') {
            return Ok(first);
        }

        let mut exprs = vec![first];

        while self.peek() == Some('|') {
            self.next();
            self.skip_ws();
            exprs.push(self.parse_comma_expr()?);
            self.skip_ws();
        }

        Ok(Expr::pipe(exprs))
    }

    /// Parse one pipe stage: `expr, expr, ...`.
    fn parse_comma_expr(&mut self) -> Result<Expr, ParseError> {
        let first = self.parse_binding()?;
        self.skip_ws();

        if self.peek() != Some(',') {
            return Ok(first);
        }

        let mut exprs = vec![first];

        while self.peek() == Some(',') {
            self.next();
            self.skip_ws();
            exprs.push(self.parse_binding()?);
            self.skip_ws();
        }

        Ok(Expr::comma(exprs))
    }

    /// Parse one comma operand: an expression, optionally followed by an `as`
    /// binding (`expr as $var | body`, `expr as {pattern} | body`).
    ///
    /// `as` belongs *below* the comma, not beside the pipe, because its body
    /// swallows everything to its right: jq reads `1,2 as $x | $x | .+10` as
    /// `1, (2 as $x | $x | .+10)`, printing `1` then `12` — not
    /// `(1,2) as $x | ...`. Binding it above the comma would capture the whole
    /// comma list as the bound expression.
    fn parse_binding(&mut self) -> Result<Expr, ParseError> {
        let expr = self.parse_assignment()?;
        self.skip_ws();

        // Phase 8: simple var; Phase 9: patterns.
        if self.matches_keyword("as") {
            self.consume_keyword("as");
            self.skip_ws();
            return self.parse_as_pattern(expr);
        }

        Ok(expr)
    }

    /// Parse a single pattern, plus any `?//`-separated alternatives (jq
    /// mode only). Always returns at least one pattern -- the caller
    /// decides what a single-element result means: `parse_as_pattern`
    /// below still special-cases a bare `$var` into the simpler
    /// `Expr::As` node; `reduce`/`foreach` (#1365) have no such fast path
    /// and always keep the `Vec<Pattern>` shape.
    ///
    /// `?//` is real jq syntax (since jq 1.6) but real yq's own parser
    /// rejects it outright ("lexer: invalid input text", confirmed live
    /// against yq v4.53.3) -- gated to jq mode only so yq mode keeps
    /// erroring on it the same way, rather than silently accepting
    /// broader syntax than the oracle it's meant to match.
    fn parse_pattern_alternatives(&mut self) -> Result<Vec<Pattern>, ParseError> {
        let first_pattern = self.parse_pattern()?;
        self.skip_ws();
        let mut patterns = vec![first_pattern];
        if self.mode == ParserMode::Jq {
            while self.peek_str(3) == "?//" {
                self.next();
                self.next();
                self.next();
                self.skip_ws();
                patterns.push(self.parse_pattern()?);
                self.skip_ws();
            }
        }
        Ok(patterns)
    }

    /// Parse the pattern part of an `as` binding, including any
    /// `?//`-separated alternatives (`. as [$a] ?// {$a} | ...`).
    /// Called after "as" has been consumed.
    fn parse_as_pattern(&mut self, expr: Expr) -> Result<Expr, ParseError> {
        let mut patterns = self.parse_pattern_alternatives()?;
        self.expect('|')?;
        self.skip_ws();
        let body = self.parse_expr()?;

        // No `?//` alternatives: keep the simpler, pre-existing `Expr::As`
        // shape for a bare `$var` pattern (every other `Expr::As` call site
        // in this file is unaffected by `?//`'s addition), and
        // `Expr::AsPattern` with a single-element `patterns` for `{...}`/
        // `[...]`, exactly as before this feature existed.
        match patterns.as_slice() {
            [Pattern::Var(_)] => {
                let Pattern::Var(var) = patterns.pop().expect("checked len == 1 above") else {
                    unreachable!("matched Pattern::Var above")
                };
                Ok(Expr::As {
                    expr: Box::new(expr),
                    var,
                    body: Box::new(body),
                })
            }
            _ => Ok(Expr::AsPattern {
                expr: Box::new(expr),
                patterns,
                body: Box::new(body),
            }),
        }
    }

    /// Parse a pipe expression that stops at a `,` — deliberately *not* a full
    /// [`Self::parse_expr`].
    ///
    /// Exactly two kinds of position want this, and no others:
    ///
    /// 1. **Object-construction values**, jq's `ExpD` production. Inside
    ///    `{...}` a `,` separates entries, so `{a: 1, b: 2}` must not read
    ///    `1, b` as one value.
    /// 2. **The `n` of `limit`/`skip`/`nth`**, where the restriction is this
    ///    crate's rather than jq's: jq's `$n` parameter convention fans the
    ///    whole call out once per output of `n`, which is not implemented here,
    ///    so `n` stays single-valued instead of silently taking one branch.
    ///
    /// Every *other* body that once called this — `if` branches, `def` bodies,
    /// `reduce`/`foreach` slots, `label` bodies, string interpolation — is a
    /// full `Exp` in jq and now says so (#462).
    fn parse_pipe_no_comma(&mut self) -> Result<Expr, ParseError> {
        let first = self.parse_binding()?;
        self.skip_ws();

        if self.peek() != Some('|') {
            return Ok(first);
        }

        let mut exprs = vec![first];

        while self.peek() == Some('|') {
            self.next();
            self.skip_ws();
            exprs.push(self.parse_binding()?);
            self.skip_ws();
        }

        Ok(Expr::pipe(exprs))
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
                bound: FuncDefBound::default(),
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
                    Some(';' | ',') => {
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
        let body = self.parse_expr()?;
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
                // Full expression per `;`-separated slot (#155).
                args.push(self.parse_expr()?);
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
/// Use `ParserMode::Yq` to allow kebab-case identifiers like `.my-key`. In
/// `Yq` mode, jq-only builtins real yq's lexer rejects (`paths`, `getpath`,
/// `limit`, `gsub`/`scan`/`splits`, etc.) are rejected too; use
/// [`parse_with_mode_and_extensions`] to accept them (#1512).
pub fn parse_with_mode(input: &str, mode: ParserMode) -> Result<Expr, ParseError> {
    parse_with_mode_and_extensions(input, mode, false)
}

/// Parse a jq expression with a specific parser mode, optionally accepting
/// jq-only builtins real yq's lexer rejects (`--jq-extensions`, #1512).
///
/// `jq_extensions` is ignored in `ParserMode::Jq`, which always accepts this
/// surface.
pub fn parse_with_mode_and_extensions(
    input: &str,
    mode: ParserMode,
    jq_extensions: bool,
) -> Result<Expr, ParseError> {
    let mut parser = Parser::with_mode_and_extensions(input, mode, jq_extensions);
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
/// Use `ParserMode::Yq` to allow kebab-case identifiers like `.my-key`. In
/// `Yq` mode, jq-only builtins real yq's lexer rejects (`paths`, `getpath`,
/// `limit`, `gsub`/`scan`/`splits`, etc.) are rejected too; use
/// [`parse_program_with_mode_and_extensions`] to accept them (#1512).
pub fn parse_program_with_mode(input: &str, mode: ParserMode) -> Result<Program, ParseError> {
    parse_program_with_mode_and_extensions(input, mode, false)
}

/// Parse a complete jq program with a specific parser mode, optionally
/// accepting jq-only builtins real yq's lexer rejects (`--jq-extensions`,
/// #1512).
///
/// `jq_extensions` is ignored in `ParserMode::Jq`, which always accepts this
/// surface.
pub fn parse_program_with_mode_and_extensions(
    input: &str,
    mode: ParserMode,
    jq_extensions: bool,
) -> Result<Program, ParseError> {
    let mut parser = Parser::with_mode_and_extensions(input, mode, jq_extensions);
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
        assert_eq!(parse(".[0]").unwrap(), Expr::index(0));
        assert_eq!(parse(".[42]").unwrap(), Expr::index(42));
        assert_eq!(parse(".[-1]").unwrap(), Expr::index(-1));
        assert_eq!(parse(".[ 0 ]").unwrap(), Expr::index(0));
    }

    #[test]
    fn test_iterate() {
        assert_eq!(parse(".[]").unwrap(), Expr::Iterate);
        assert_eq!(parse(".[ ]").unwrap(), Expr::Iterate);
    }

    #[test]
    fn test_slice() {
        assert_eq!(parse(".[1:3]").unwrap(), Expr::slice(Some(1), Some(3)));
        assert_eq!(parse(".[1:]").unwrap(), Expr::slice(Some(1), None));
        assert_eq!(parse(".[:3]").unwrap(), Expr::slice(None, Some(3)));
        // `[:]` is a full slice (returns the whole array), not an iterate
        assert_eq!(parse(".[:]").unwrap(), Expr::slice(None, None));
    }

    #[test]
    fn test_optional() {
        assert_eq!(
            parse(".foo?").unwrap(),
            Expr::Optional(Box::new(Expr::Field("foo".into())))
        );
    }

    /// jq accepts postfix `?` after any Term, not just a path expression
    /// (#367). These cover the forms that used to fail with "unexpected
    /// character '?'" because only the dot-field and index-bracket
    /// productions checked for a trailing `?`.
    #[test]
    fn test_optional_after_builtin() {
        assert_eq!(
            parse("length?").unwrap(),
            Expr::Optional(Box::new(Expr::Builtin(Builtin::Length)))
        );
        assert_eq!(
            parse("keys?").unwrap(),
            Expr::Optional(Box::new(Expr::Builtin(Builtin::Keys)))
        );
        assert_eq!(
            parse("tonumber?").unwrap(),
            Expr::Optional(Box::new(Expr::Builtin(Builtin::ToNumber)))
        );
    }

    #[test]
    fn test_optional_after_parenthesized_expr() {
        assert_eq!(
            parse("(.a)?").unwrap(),
            Expr::Optional(Box::new(Expr::Paren(Box::new(Expr::Field("a".into())))))
        );
        assert_eq!(
            parse("(1)?").unwrap(),
            Expr::Optional(Box::new(Expr::Paren(Box::new(Expr::Literal(
                Literal::number_literal("1".to_string())
            )))))
        );
    }

    #[test]
    fn test_optional_after_function_call() {
        match parse("first(.[])?").unwrap() {
            Expr::Optional(inner) => assert!(matches!(*inner, Expr::FirstExpr(_))),
            other => panic!("expected Expr::Optional, got {other:?}"),
        }
        match parse(r#"getpath(["a"])?"#).unwrap() {
            Expr::Optional(inner) => {
                assert!(matches!(*inner, Expr::Builtin(Builtin::GetPath(_))));
            }
            other => panic!("expected Expr::Optional, got {other:?}"),
        }
        match parse(r#"setpath(["a"];1)?"#).unwrap() {
            Expr::Optional(inner) => {
                assert!(matches!(*inner, Expr::Builtin(Builtin::SetPath(_, _))));
            }
            other => panic!("expected Expr::Optional, got {other:?}"),
        }
    }

    #[test]
    fn test_optional_after_variable_and_identity() {
        assert_eq!(
            parse("$x?").unwrap(),
            Expr::Optional(Box::new(Expr::Var("x".into())))
        );
        assert_eq!(
            parse(".?").unwrap(),
            Expr::Optional(Box::new(Expr::Identity))
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
            Expr::Pipe(vec![Expr::Field("foo".into()), Expr::index(0),])
        );

        assert_eq!(
            parse(".foo.bar[0].baz").unwrap(),
            Expr::Pipe(vec![
                Expr::Field("foo".into()),
                Expr::Field("bar".into()),
                Expr::index(0),
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

    /// `|` is the loosest operator and `,` binds tighter, so a pipe stage is a
    /// comma list — not the other way round (#462).
    ///
    /// These assert the AST *shape*, because the failure this pins was silent
    /// at the value level: `1,2,3 | . * 2` still produced three outputs, just
    /// with the first two untransformed.
    #[test]
    fn test_comma_binds_tighter_than_pipe() {
        let int = |i: i64| Expr::Literal(Literal::number_literal(i.to_string()));

        // (1,2) | 3 — not 1, (2 | 3)
        assert_eq!(
            parse("1,2 | 3").unwrap(),
            Expr::Pipe(vec![Expr::Comma(vec![int(1), int(2)]), int(3)])
        );

        // Every stage is a comma list, so both sides group.
        assert_eq!(
            parse("1,2 | 3,4").unwrap(),
            Expr::Pipe(vec![
                Expr::Comma(vec![int(1), int(2)]),
                Expr::Comma(vec![int(3), int(4)]),
            ])
        );

        // A pipe of three stages stays flat, with only the comma stage nested.
        assert_eq!(
            parse(".a | 1,2 | .b").unwrap(),
            Expr::Pipe(vec![
                Expr::Field("a".into()),
                Expr::Comma(vec![int(1), int(2)]),
                Expr::Field("b".into()),
            ])
        );

        // Explicit parens were the old workaround; they must still mean the
        // same thing they always did.
        assert_eq!(
            parse("(1,2) | 3").unwrap(),
            Expr::Pipe(vec![
                Expr::Paren(Box::new(Expr::Comma(vec![int(1), int(2)]))),
                int(3),
            ])
        );

        // A comma with no pipe is still a bare comma — no spurious Pipe wrapper.
        assert_eq!(parse("1,2").unwrap(), Expr::Comma(vec![int(1), int(2)]));
    }

    /// `as` binds below the comma: its body swallows the rest of the
    /// expression, so only the *last* comma operand is bound (#462).
    #[test]
    fn test_as_binds_inside_comma_operand() {
        let int = |i: i64| Expr::Literal(Literal::number_literal(i.to_string()));

        // 1, (2 as $x | $x) — not (1,2) as $x | $x
        assert_eq!(
            parse("1,2 as $x | $x").unwrap(),
            Expr::Comma(vec![
                int(1),
                Expr::As {
                    expr: Box::new(int(2)),
                    var: "x".into(),
                    body: Box::new(Expr::Var("x".into())),
                },
            ])
        );

        // The binding body is a full expression, comma included.
        assert_eq!(
            parse("1 as $x | 2,3").unwrap(),
            Expr::As {
                expr: Box::new(int(1)),
                var: "x".into(),
                body: Box::new(Expr::Comma(vec![int(2), int(3)])),
            }
        );
    }

    /// Object values are jq's `ExpD`, not `Exp`: the `,` inside `{...}`
    /// separates entries and must not be swallowed by a value (#462).
    #[test]
    fn test_object_value_stops_at_comma() {
        let entries = match parse("{a: 1, b: 2}").unwrap() {
            Expr::Object(entries) => entries,
            other => panic!("expected an object, got {other:?}"),
        };
        assert_eq!(entries.len(), 2, "the `,` must separate two entries");
        assert_eq!(
            entries[0].value,
            Expr::Literal(Literal::number_literal("1".to_string()))
        );
        assert_eq!(
            entries[1].value,
            Expr::Literal(Literal::number_literal("2".to_string()))
        );

        // Parens are how a value fans out, and they still work.
        let entries = match parse("{a: (1,2)}").unwrap() {
            Expr::Object(entries) => entries,
            other => panic!("expected an object, got {other:?}"),
        };
        assert_eq!(entries.len(), 1);
        assert!(matches!(entries[0].value, Expr::Paren(_)));

        // A pipe inside a value is still accepted, and still stops at the `,`.
        let entries = match parse("{a: .x | .y, b: 2}").unwrap() {
            Expr::Object(entries) => entries,
            other => panic!("expected an object, got {other:?}"),
        };
        assert_eq!(entries.len(), 2);
        assert!(matches!(entries[0].value, Expr::Pipe(_)));
    }

    /// The bodies that jq spells as a full `Exp` accept a bare comma. Before
    /// #462 these were parsed one level too tight and rejected it outright.
    #[test]
    fn test_comma_accepted_in_exp_bodies() {
        assert!(parse("if true then 1,2 else 3 end").is_ok());
        assert!(parse("if true then 1 elif false then 2,3 else 4 end").is_ok());
        assert!(parse("if false then 1 else 2,3 end").is_ok());
        assert!(parse("def f: 1,2; f").is_ok());
        assert!(parse("label $out | 1,2").is_ok());
        assert!(parse("[1,2] as [$a,$b] | $a,$b").is_ok());
        assert!(parse(r#""\(1,2)""#).is_ok());
        assert!(parse("range(1,2; 4)").is_ok());
        assert!(parse("first(1,2)").is_ok());
        assert!(parse("last(1,2)").is_ok());
        assert!(parse("error(1,2)").is_ok());
        assert!(parse("repeat(1,2)").is_ok());
    }

    /// `reduce`/`foreach`'s init/update/extract and `until`/`while`'s
    /// cond/update now accept a bare comma like the other `Exp` bodies above
    /// — the evaluator implements jq's real fanout/fold rules for each slot
    /// (`eval_owned_expr_fork`/`finish_fork`, eval.rs), so #534's interim
    /// parse-error mitigation (this test used to assert these `is_err()`) is
    /// lifted.
    #[test]
    fn test_comma_accepted_in_reduce_foreach_until_while() {
        assert!(parse("reduce .[] as $x (0; .+$x, .)").is_ok());
        assert!(parse("reduce .[] as $x (0,1; .+$x)").is_ok());
        assert!(parse("foreach .[] as $x (0; .+$x, .)").is_ok());
        assert!(parse("foreach .[] as $x (0,1; .+$x)").is_ok());
        assert!(parse("foreach .[] as $x (0; .+$x; ., .*2)").is_ok());
        assert!(parse("until(.>1; .+1,.)").is_ok());
        assert!(parse("until(.>1,.>2; .+1)").is_ok());
        assert!(parse("while(.<3; .+1,.)").is_ok());
        assert!(parse("while(.<3,.<5; .+1)").is_ok());
    }

    /// #1201: `reduce`/`foreach`'s `as` clause parses a full destructuring
    /// pattern (object or array), not just a bare `$var` -- confirms the
    /// resulting AST actually carries a non-`Pattern::Var` pattern (a bare
    /// `is_ok()` check alone wouldn't distinguish "parsed the pattern
    /// correctly" from "silently produced some other, wrong AST shape").
    #[test]
    fn test_reduce_foreach_accept_full_pattern_1201() {
        let reduce_obj = parse("reduce .[] as {a: $a} (0; . + $a)").unwrap();
        assert!(matches!(
            reduce_obj,
            Expr::Reduce {
                ref patterns,
                ..
            } if matches!(patterns.as_slice(), [Pattern::Object(_)])
        ));

        let reduce_arr = parse("reduce .[] as [$a, $b] (0; . + $a + $b)").unwrap();
        assert!(matches!(
            reduce_arr,
            Expr::Reduce {
                ref patterns,
                ..
            } if matches!(patterns.as_slice(), [Pattern::Array(_)])
        ));

        let foreach_obj = parse("foreach .[] as {a: $a} (0; . + $a; .)").unwrap();
        assert!(matches!(
            foreach_obj,
            Expr::Foreach {
                ref patterns,
                ..
            } if matches!(patterns.as_slice(), [Pattern::Object(_)])
        ));

        // A bare `$var` still parses to `Pattern::Var`, not a regression.
        let reduce_var = parse("reduce .[] as $x (0; . + $x)").unwrap();
        assert!(matches!(
            reduce_var,
            Expr::Reduce { ref patterns, .. }
                if matches!(patterns.as_slice(), [Pattern::Var(v)] if v == "x")
        ));
    }

    #[test]
    fn test_comma_in_call_arguments() {
        // Builtin single-arg filter position: sort_by(.a,.b) (#155).
        assert_eq!(
            parse("sort_by(.a,.b)").unwrap(),
            Expr::Builtin(Builtin::SortBy(Box::new(Expr::Comma(vec![
                Expr::Field("a".into()),
                Expr::Field("b".into()),
            ]))))
        );

        // first(expr) with a comma-generator argument.
        assert_eq!(
            parse("first(1,2,3)").unwrap(),
            Expr::FirstExpr(Box::new(Expr::Comma(vec![
                Expr::Literal(Literal::number_literal("1".to_string())),
                Expr::Literal(Literal::number_literal("2".to_string())),
                Expr::Literal(Literal::number_literal("3".to_string())),
            ])))
        );

        // limit(n; expr) — the `expr` slot accepts comma.
        assert_eq!(
            parse("[limit(2;1,2,3,4)]").unwrap(),
            Expr::Array(Box::new(Expr::Limit {
                n: Box::new(Expr::Literal(Literal::number_literal("2".to_string()))),
                expr: Box::new(Expr::Comma(vec![
                    Expr::Literal(Literal::number_literal("1".to_string())),
                    Expr::Literal(Literal::number_literal("2".to_string())),
                    Expr::Literal(Literal::number_literal("3".to_string())),
                    Expr::Literal(Literal::number_literal("4".to_string())),
                ])),
            }))
        );

        // Deliberate carve-out (#155): `n` in limit/skip/nth stays
        // restricted to non-comma, since this codebase doesn't implement
        // real jq's `$n` per-output fanout convention for these builtins.
        assert!(parse("limit(1,2; .)").is_err());
        assert!(parse("skip(1,2; .)").is_err());
        assert!(parse("nth(1,2; .)").is_err());

        // User-defined single-parameter function called with a comma
        // argument: def f(x): x; f(1,2).
        assert!(parse("def f(x): x; f(1,2)").is_ok());

        // Namespaced call argument position.
        assert!(parse("ns::f(1,2)").is_ok());
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
        assert_eq!(
            parse("42").unwrap(),
            Expr::Literal(Literal::number_literal("42".to_string()))
        );
        assert_eq!(
            parse("-123").unwrap(),
            Expr::Literal(Literal::number_literal("-123".to_string()))
        );
        assert_eq!(
            parse("2.5").unwrap(),
            Expr::Literal(Literal::number_literal("2.5".to_string()))
        );
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
    fn test_negative_float_literal_splits_into_positive_repr_1062() {
        // A negative float/exponent literal parses as `-1 * <positive
        // literal>` in jq mode (the sign is folded into unary negation, not
        // kept as part of the number token -- see the parser's own comment
        // above this rewrite). #1062 computes the split-off literal's
        // `NumberRepr` by negating the *original* (negative) repr rather
        // than re-parsing the sign-stripped text; this pins that both the
        // repr and the text agree on the positive magnitude.
        let Expr::Arithmetic { right, .. } = parse("-1.500").unwrap() else {
            panic!("expected an Arithmetic node");
        };
        assert_eq!(
            *right,
            Expr::Literal(Literal::NumberLiteral(
                NumberRepr::Float(1.5),
                "1.500".to_string()
            ))
        );

        let Expr::Arithmetic { right, .. } = parse("-1e2").unwrap() else {
            panic!("expected an Arithmetic node");
        };
        assert_eq!(
            *right,
            Expr::Literal(Literal::NumberLiteral(
                NumberRepr::Float(100.0),
                "1e2".to_string()
            ))
        );
    }

    #[test]
    fn test_large_integer_literal_falls_back_to_float() {
        // Literals beyond i64 range degrade to floats like jq (issue #166),
        // but #1035 keeps the literal's own source spelling rather than
        // immediately collapsing it to a freshly-formatted f64 -- the value
        // still *evaluates* as a float (see the `from_number_literal`
        // conversion), only the AST node's stored text is unaffected here.
        assert_eq!(
            parse("9999999999999999999").unwrap(),
            Expr::Literal(Literal::number_literal("9999999999999999999".to_string()))
        );
        assert_eq!(
            parse("-9999999999999999999").unwrap(),
            Expr::Literal(Literal::number_literal("-9999999999999999999".to_string()))
        );
        // One past the boundary in each direction.
        assert_eq!(
            parse("9223372036854775808").unwrap(),
            Expr::Literal(Literal::number_literal("9223372036854775808".to_string()))
        );
        assert_eq!(
            parse("-9223372036854775809").unwrap(),
            Expr::Literal(Literal::number_literal("-9223372036854775809".to_string()))
        );
        // Boundary values stay exact integers.
        assert_eq!(
            parse("9223372036854775807").unwrap(),
            Expr::Literal(Literal::number_literal("9223372036854775807".to_string()))
        );
        assert_eq!(
            parse("-9223372036854775808").unwrap(),
            Expr::Literal(Literal::number_literal("-9223372036854775808".to_string()))
        );
    }

    /// A bare exponent marker with no digits after it (`1e`) is not a
    /// number Rust's own `f64`/`i64` parsers accept either -- confirmed
    /// against jq 1.7.1, which also rejects it with a parse error.
    #[test]
    fn test_bare_exponent_marker_with_no_digits_is_a_parse_error() {
        assert!(parse("1e").is_err());
        assert!(parse("1e+").is_err());
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
        assert!(parse(".[1 2]").is_err()); // missing ']' after the key
        assert!(parse(".123").is_err()); // field starting with number
        assert!(parse("{").is_err()); // unclosed brace
        assert!(parse("[").is_err()); // unclosed bracket
        assert!(parse("\"unterminated").is_err()); // unterminated string
    }

    /// `ParseError`'s `Display` impl is what the CLI prints for every real
    /// syntax error (`jq_runner.rs`'s `eprintln!("jq: compile error: {e}")`),
    /// but nothing exercised it directly: the only source of a genuine parse
    /// error in the golden corpus was #534's now-lifted restriction against
    /// a comma in reduce/foreach/until/while's init/update/cond/extract
    /// slots (`test_comma_accepted_in_reduce_foreach_until_while` above).
    #[test]
    fn test_parse_error_display_includes_position_and_message() {
        let err = parse(".[").unwrap_err();
        assert_eq!(
            err.to_string(),
            format!("parse error at position {}: {}", err.position, err.message)
        );
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
                op: ArithOp::Mul(_),
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
                        op: ArithOp::Mul(_),
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
        assert_eq!(parse(".\"\"").unwrap(), Expr::Field(String::new()));

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

    /// A constant key must still fold to the static chain element it always
    /// produced, so the hot `.foo.bar[0]` path and every `Field`/`Index` match
    /// site are untouched by #360.
    ///
    /// #1088 changed what a float-spelled key *carries*, never whether it
    /// folds: `.[1.0]` is still a static component, now with the spelling
    /// `path()` has to report. An integer-spelled key is untouched -- there
    /// is nothing to preserve, so its `key` stays `None`, and `.[0]`
    /// staying `Expr::index(0)` is what keeps the hot path hot.
    #[test]
    fn test_index_key_constant_folding() {
        assert_eq!(parse(".[0]").unwrap(), Expr::index(0));
        assert_eq!(parse(".[-1]").unwrap(), Expr::index(-1));
        assert_eq!(
            parse(".[1.0]").unwrap(),
            Expr::Index {
                idx: 1,
                key: Some(NumberKey::Literal(1.0, "1.0".into())),
            }
        );
        // `1e0` is the same value spelled differently, and jq echoes the
        // spelling, so the two must not collapse onto one component.
        assert_eq!(
            parse(".[1e0]").unwrap(),
            Expr::Index {
                idx: 1,
                key: Some(NumberKey::Literal(1.0, "1e0".into())),
            }
        );
        assert_eq!(parse(".[\"a\"]").unwrap(), Expr::Field("a".into()));
        // Parenthesised constants fold too — `.[("a")]` is `.a`.
        assert_eq!(parse(".[(\"a\")]").unwrap(), Expr::Field("a".into()));
        assert_eq!(parse(".[ \"a\" ]").unwrap(), Expr::Field("a".into()));

        // A fractional index cannot be a static component (jq truncates it at
        // evaluation), so it stays dynamic.
        assert!(matches!(parse(".[1.7]").unwrap(), Expr::IndexExpr { .. }));
        // `null`/`true`/`{}` must reach the evaluator to produce jq's runtime
        // `Cannot index …`, so they must not fold into a parse error.
        for src in [".[null]", ".[true]", ".[{}]"] {
            assert!(
                matches!(parse(src).unwrap(), Expr::IndexExpr { .. }),
                "{src} should parse as a computed index"
            );
        }
    }

    /// Both slice bounds accept the same spellings.
    ///
    /// Parsing only the first bound as an expression let `.[(1):3]` compile
    /// while `.[1:(3)]` did not — an asymmetry with no grammar behind it, and
    /// invisible unless a test writes the same constant on both sides.
    #[test]
    fn test_slice_bounds_accept_the_same_spellings() {
        let expected = Expr::slice(Some(1), Some(3));
        for src in [".[1:3]", ".[(1):3]", ".[1:(3)]"] {
            assert_eq!(parse(src).unwrap(), expected, "`{src}`");
        }
        // #1326: a float-spelled bound (even a whole-valued one) keeps its
        // own spelling in `path()` output, so it folds with that bound's
        // key populated -- only the unaffected bound stays `None`.
        assert_eq!(
            parse(".[1.0:3]").unwrap(),
            Expr::Slice {
                start: Some(1),
                end: Some(3),
                start_key: Some(NumberKey::Literal(1.0, "1.0".into())),
                end_key: None,
            }
        );
        assert_eq!(
            parse(".[1:3.0]").unwrap(),
            Expr::Slice {
                start: Some(1),
                end: Some(3),
                start_key: None,
                end_key: Some(NumberKey::Literal(3.0, "3.0".into())),
            }
        );

        // Open bounds too, including the negative that only the `[:n]` branch
        // ever sees.
        for (src, expected) in [
            (".[:-2]", (None, Some(-2))),
            (".[:(-2)]", (None, Some(-2))),
            (".[-2:]", (Some(-2), None)),
        ] {
            let (start, end) = expected;
            assert_eq!(parse(src).unwrap(), Expr::slice(start, end), "`{src}`");
        }

        // A bound that does not fold to a literal becomes `Expr::SliceExpr`
        // instead of a parse error (#499), on either side, and independently
        // of whether the other bound folds.
        assert_eq!(
            parse(".[$a:1]").unwrap(),
            Expr::slice_by(
                Expr::Identity,
                Some(Expr::Var("a".into())),
                Some(Expr::Literal(Literal::number_literal("1".to_string()))),
            )
        );
        assert_eq!(
            parse(".[1:$b]").unwrap(),
            Expr::slice_by(
                Expr::Identity,
                Some(Expr::Literal(Literal::number_literal("1".to_string()))),
                Some(Expr::Var("b".into())),
            )
        );
        assert_eq!(
            parse(".[:$b]").unwrap(),
            Expr::slice_by(Expr::Identity, None, Some(Expr::Var("b".into())))
        );
    }

    /// #1035: a negative float/exponent index or slice bound still folds to
    /// the static `Expr::Index`/`Expr::Slice` fast path, not a runtime
    /// `IndexExpr`/`DynamicSlice` -- the jq-mode negative-literal split
    /// (`-1.0` -> `-1 * 1.0`) must not defeat `fold_index_key`'s constant
    /// folding, which only sees through `Expr::Literal`/`Expr::Paren`
    /// unless taught this specific `Arithmetic` shape too.
    #[test]
    fn test_1035_negative_float_index_and_slice_bound_still_fold_to_static() {
        // #1088: negation is what destroys jq's literal preservation, so a
        // negated key folds to a bare `NumberKey::Float` -- which is exactly
        // why `path(.[-1.0])` is `[-1]` while `path(.[1.0])` is `[1.0]`.
        assert_eq!(
            parse(".[-1.0]").unwrap(),
            Expr::Index {
                idx: -1,
                key: Some(NumberKey::Float(-1.0)),
            }
        );
        assert_eq!(
            parse(".[-1e0]").unwrap(),
            Expr::Index {
                idx: -1,
                key: Some(NumberKey::Float(-1.0)),
            }
        );
        // #1326: the slice-bound sibling of the index case above --
        // negation destroys the *literal spelling* the same way, but the
        // bound still keeps a `NumberKey::Float` rather than dropping to
        // plain `Expr::Slice` (`path(.[-3.0:-1.0])` is `[{"start":-3,
        // "end":-1}]`, not `.0`-suffixed, because a *computed* whole-valued
        // double renders bare in jq's own convention -- confirmed live,
        // matching real jq exactly).
        assert_eq!(
            parse(".[-3.0:-1.0]").unwrap(),
            Expr::Slice {
                start: Some(-3),
                end: Some(-1),
                start_key: Some(NumberKey::Float(-3.0)),
                end_key: Some(NumberKey::Float(-1.0)),
            }
        );
        // A non-integral negative float still can't fold -- same as the
        // positive case, it must go through the evaluator to truncate the
        // way jq does.
        assert!(matches!(parse(".[-1.5]").unwrap(), Expr::IndexExpr { .. }));
    }

    /// #1061: `i64::MAX as f64` rounds *up* to `2^63` (`i64::MAX` itself isn't
    /// exactly representable as `f64`), so a `<=` bound let `.[2^63]` fold to
    /// `Expr::index(i64::MAX)` -- silently one lower than what was written,
    /// with no error or truncation notice. The fix must reject the whole
    /// `[2^63, +inf)` range (both the nearest-representable spellings, since
    /// `9223372036854775807.0` and `9223372036854775808.0` round to the same
    /// `f64`) while still folding every value strictly below it, through both
    /// the plain-float and the source-text-preserving `NumberLiteral` arms.
    #[test]
    fn test_1061_i64_max_boundary_no_longer_off_by_one() {
        assert!(matches!(
            parse(".[9223372036854775808.0]").unwrap(),
            Expr::IndexExpr { .. }
        ));
        // Same `f64` bit pattern as the value above (`i64::MAX` rounds up to
        // `2^63`), so it must be rejected too, not just the round-numbered
        // spelling.
        assert!(matches!(
            parse(".[9223372036854775807.0]").unwrap(),
            Expr::IndexExpr { .. }
        ));
        // Exponent-notation spelling goes through the `NumberLiteral` arm,
        // not the plain-`Float` arm -- both need the same fix.
        assert!(matches!(
            parse(".[9223372036854775808e0]").unwrap(),
            Expr::IndexExpr { .. }
        ));

        // A value strictly below the boundary still takes the fast path
        // (carrying a `key` since #1088 -- same `idx`, plus the spelling
        // `path()` reports).
        assert_eq!(
            parse(".[9223372036854774784.0]").unwrap(),
            Expr::Index {
                idx: 9223372036854774784,
                key: Some(NumberKey::Literal(
                    9223372036854774784.0,
                    "9223372036854774784.0".into()
                )),
            }
        );
        assert_eq!(
            parse(".[9223372036854774784e0]").unwrap(),
            Expr::Index {
                idx: 9223372036854774784,
                key: Some(NumberKey::Literal(
                    9223372036854774784.0,
                    "9223372036854774784e0".into()
                )),
            }
        );
    }

    /// #1061 (review follow-up): negating `fold_index_key(right)`'s result in
    /// the `-1 * <literal>` desugar arm has no `i64::MIN` guard on its own --
    /// `i64::MIN`'s magnitude (`2^63`) is one larger than `i64::MAX`'s, so
    /// `-i64::MIN` overflows `i64`. That's reachable two ways: the intended
    /// negative-float-literal spelling (`.[-9223372036854775808.0]`, whose
    /// magnitude is a `NumberLiteral` that parses to exactly `i64::MAX as
    /// f64`), and separately, unrelated to any float spelling at all, a bare
    /// `Literal::Int(i64::MIN)` reaching this arm as `right` (ordinary jq
    /// multiplication with a `Literal::Int(-1)` on the left -- reachable from
    /// real source via a leading-zero integer like `-01`, since that spelling
    /// fails JSON's number grammar and falls back to a bare `Literal::Int`
    /// rather than the source-text-preserving `NumberLiteral`). Both must
    /// fall through to a runtime `IndexExpr` instead of folding to a wrong
    /// value (previously a silent two's-complement wrap in release builds,
    /// or a debug-build panic).
    #[test]
    fn test_1061_negating_i64_min_does_not_overflow() {
        assert!(matches!(
            parse(".[-9223372036854775808.0]").unwrap(),
            Expr::IndexExpr { .. }
        ));
        assert!(matches!(
            parse(".[-9223372036854775808e0]").unwrap(),
            Expr::IndexExpr { .. }
        ));
        // The `right` operand need not come from the negative-literal split
        // at all -- any `-1 * <literal>` shape matches the same arm.
        assert!(matches!(
            parse(".[-01 * -9223372036854775808]").unwrap(),
            Expr::IndexExpr { .. }
        ));

        // Every other magnitude on this path still negates correctly --
        // one `f64` ULP below the boundary above, so distinct from it
        // (`9223372036854775807.0` isn't usable here: it rounds to the same
        // `f64` as `2^63` and collapses onto the boundary case rather than
        // testing a separate value). `test_1035_...` above already covers
        // the ordinary case through the intended desugar path (`.[-1.0]`
        // etc.), so this only needs the one large-but-safe magnitude.
        assert_eq!(
            parse(".[-9223372036854774784.0]").unwrap(),
            Expr::Index {
                idx: -9223372036854774784,
                key: Some(NumberKey::Float(-9223372036854774784.0)),
            }
        );
    }

    /// #1061: `finish_slice`'s bounds fold through `fold_slice_bound`, which
    /// itself delegates to `fold_index_key` -- so a slice bound at either
    /// boundary must inherit the same fix as a bare index, not just be
    /// exercised at the index call site.
    #[test]
    fn test_1061_boundary_fix_applies_to_slice_bounds_too() {
        // A slice's upper bound at `2^63` can't fold, same as an index.
        assert!(matches!(
            parse(".[1:9223372036854775808.0]").unwrap(),
            Expr::SliceExpr { .. }
        ));
        // A slice's lower bound at `i64::MIN`'s magnitude no longer
        // overflows on negation, same as an index.
        assert!(matches!(
            parse(".[-9223372036854775808.0:]").unwrap(),
            Expr::SliceExpr { .. }
        ));
    }

    /// The nesting shape is what encodes jq's key scoping: every key in a
    /// postfix chain is evaluated against the chain's input, which a flat
    /// `Pipe` cannot express (it is also what an explicit `|` lowers to).
    #[test]
    fn test_dynamic_index_shapes() {
        assert_eq!(
            parse(".[$k]").unwrap(),
            Expr::index_by(Expr::Identity, Expr::Var("k".into()))
        );

        // `.a[.k]`: the key hangs off the node, not off `.a`.
        assert_eq!(
            parse(".a[.k]").unwrap(),
            Expr::index_by(Expr::Field("a".into()), Expr::Field("k".into()))
        );

        // `.[.k][.k]` nests, so both keys see the same input.
        assert_eq!(
            parse(".[.k][.k]").unwrap(),
            Expr::index_by(
                Expr::index_by(Expr::Identity, Expr::Field("k".into())),
                Expr::Field("k".into())
            )
        );

        // `.a[.k].b[.j]`: the second target is the whole chain so far.
        assert_eq!(
            parse(".a[.k].b[.j]").unwrap(),
            Expr::index_by(
                Expr::Pipe(vec![
                    Expr::index_by(Expr::Field("a".into()), Expr::Field("k".into())),
                    Expr::Field("b".into()),
                ]),
                Expr::Field("j".into())
            )
        );

        // Bracket contents parse at comma precedence (#155).
        assert_eq!(
            parse(".[1,2]").unwrap(),
            Expr::index_by(
                Expr::Identity,
                Expr::Comma(vec![
                    Expr::Literal(Literal::number_literal("1".to_string())),
                    Expr::Literal(Literal::number_literal("2".to_string())),
                ])
            )
        );

        // `?` wraps the whole node, not the key.
        assert_eq!(
            parse(".[$k]?").unwrap(),
            Expr::index_by(Expr::Identity, Expr::Var("k".into())).optional()
        );
    }

    /// Same nesting shape as `test_dynamic_index_shapes`, for slice bounds
    /// that don't fold to a literal (#499): `SliceExpr` carries its own
    /// `target` for the same reason `IndexExpr` does — a bound is evaluated
    /// against the chain's input, not against the target's output.
    #[test]
    fn test_dynamic_slice_shapes() {
        assert_eq!(
            parse(".[$a:$b]").unwrap(),
            Expr::slice_by(
                Expr::Identity,
                Some(Expr::Var("a".into())),
                Some(Expr::Var("b".into())),
            )
        );

        // `.a[.k1:.k2]`: the bounds hang off the node, not off `.a`.
        assert_eq!(
            parse(".a[.k1:.k2]").unwrap(),
            Expr::slice_by(
                Expr::Field("a".into()),
                Some(Expr::Field("k1".into())),
                Some(Expr::Field("k2".into())),
            )
        );

        // `.a[.k1:.k2].b[.j1:.j2]`: the second target is the whole chain so far.
        assert_eq!(
            parse(".a[.k1:.k2].b[.j1:.j2]").unwrap(),
            Expr::slice_by(
                Expr::Pipe(vec![
                    Expr::slice_by(
                        Expr::Field("a".into()),
                        Some(Expr::Field("k1".into())),
                        Some(Expr::Field("k2".into())),
                    ),
                    Expr::Field("b".into()),
                ]),
                Some(Expr::Field("j1".into())),
                Some(Expr::Field("j2".into())),
            )
        );

        // `?` wraps the whole node, not a bound.
        assert_eq!(
            parse(".[$a:$b]?").unwrap(),
            Expr::slice_by(
                Expr::Identity,
                Some(Expr::Var("a".into())),
                Some(Expr::Var("b".into())),
            )
            .optional()
        );

        // A dynamic bound composes with an `IndexExpr` target and vice versa.
        assert_eq!(
            parse(".[.k][$a:$b]").unwrap(),
            Expr::slice_by(
                Expr::index_by(Expr::Identity, Expr::Field("k".into())),
                Some(Expr::Var("a".into())),
                Some(Expr::Var("b".into())),
            )
        );
    }

    /// `.a.[k]` — jq's older spelling, where a dot precedes the bracket — takes
    /// a separate branch of the postfix loop from `.a[k]`. Both have to attach
    /// the bracket to the chain the same way, or a computed key written with
    /// the dot would silently read its key from `.a` instead of from the input.
    #[test]
    fn test_dot_before_bracket_matches_the_bare_bracket() {
        for (dotted, bare) in [
            (".a.[0]", ".a[0]"),
            (r#".a.["k"]"#, r#".a["k"]"#),
            (".a.[.k]", ".a[.k]"),
            (".a.[$k]?", ".a[$k]?"),
            (".a.[]", ".a[]"),
        ] {
            assert_eq!(
                parse(dotted).unwrap(),
                parse(bare).unwrap(),
                "`{dotted}` should parse identically to `{bare}`"
            );
        }
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
        assert_eq!(parse(".[\"\"]").unwrap(), Expr::Field(String::new()));

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
            Expr::Pipe(vec![Expr::Field("my-key".into()), Expr::index(0),])
        );

        // Kebab-case with optional
        assert_eq!(
            parse_with_mode(".my-key?", ParserMode::Yq).unwrap(),
            Expr::Optional(Box::new(Expr::Field("my-key".into())))
        );
    }

    #[test]
    fn test_yq_merge_flags_non_assign() {
        // Plain `*` still parses with default (all-false) flags.
        assert_eq!(
            parse_with_mode(".a * .b", ParserMode::Yq).unwrap(),
            Expr::Arithmetic {
                op: ArithOp::Mul(MergeFlags::default()),
                left: Box::new(Expr::Field("a".into())),
                right: Box::new(Expr::Field("b".into())),
            }
        );

        // Single flags.
        let cases: &[(&str, MergeFlags)] = &[
            (
                "*+",
                MergeFlags {
                    append_arrays: true,
                    ..Default::default()
                },
            ),
            (
                "*?",
                MergeFlags {
                    only_existing: true,
                    ..Default::default()
                },
            ),
            (
                "*n",
                MergeFlags {
                    only_new: true,
                    ..Default::default()
                },
            ),
            (
                "*d",
                MergeFlags {
                    deep_merge_arrays: true,
                    ..Default::default()
                },
            ),
            (
                "*c",
                MergeFlags {
                    clobber_tags: true,
                    ..Default::default()
                },
            ),
        ];
        for (op, expected_flags) in cases {
            let expr = parse_with_mode(&format!(".a {op} .b"), ParserMode::Yq).unwrap();
            assert_eq!(
                expr,
                Expr::Arithmetic {
                    op: ArithOp::Mul(*expected_flags),
                    left: Box::new(Expr::Field("a".into())),
                    right: Box::new(Expr::Field("b".into())),
                },
                "parsing '.a {op} .b'"
            );
        }

        // Combined flags, and order/duplicates don't matter.
        let combined = MergeFlags {
            append_arrays: true,
            deep_merge_arrays: true,
            ..Default::default()
        };
        for op in ["*+d", "*d+", "*++dd"] {
            let expr = parse_with_mode(&format!(".a {op} .b"), ParserMode::Yq).unwrap();
            assert_eq!(
                expr,
                Expr::Arithmetic {
                    op: ArithOp::Mul(combined),
                    left: Box::new(Expr::Field("a".into())),
                    right: Box::new(Expr::Field("b".into())),
                },
                "parsing '.a {op} .b'"
            );
        }

        // All five flags combined.
        let expr = parse_with_mode(".a *+?ndc .b", ParserMode::Yq).unwrap();
        assert_eq!(
            expr,
            Expr::Arithmetic {
                op: ArithOp::Mul(MergeFlags {
                    append_arrays: true,
                    only_existing: true,
                    only_new: true,
                    deep_merge_arrays: true,
                    clobber_tags: true,
                }),
                left: Box::new(Expr::Field("a".into())),
                right: Box::new(Expr::Field("b".into())),
            }
        );
    }

    #[test]
    fn test_yq_merge_flags_assign() {
        // Plain `*=` still parses with default (all-false) flags.
        assert_eq!(
            parse_with_mode(".a *= .b", ParserMode::Yq).unwrap(),
            Expr::CompoundAssign {
                op: AssignOp::Mul(MergeFlags::default()),
                path: Box::new(Expr::Field("a".into())),
                value: Box::new(Expr::Field("b".into())),
            }
        );

        // Flags go after '=', e.g. `*=+`, `*=nd`, order/duplicates don't matter.
        let combined = MergeFlags {
            only_new: true,
            deep_merge_arrays: true,
            ..Default::default()
        };
        for op in ["*=nd", "*=dn", "*=nnd"] {
            let expr = parse_with_mode(&format!(".a {op} .b"), ParserMode::Yq).unwrap();
            assert_eq!(
                expr,
                Expr::CompoundAssign {
                    op: AssignOp::Mul(combined),
                    path: Box::new(Expr::Field("a".into())),
                    value: Box::new(Expr::Field("b".into())),
                },
                "parsing '.a {op} .b'"
            );
        }

        // `*+=` (flags BEFORE '=') is NOT the assign spelling — real yq
        // doesn't recognize it either (it's a dyadic-operator arity error
        // there). We should at least fail to parse it as a merge-assign;
        // exact error text need not match real yq.
        assert!(parse_with_mode(".a *+= .b", ParserMode::Yq).is_err());
    }

    #[test]
    fn test_yq_merge_flags_unknown_char_is_error() {
        // An unrecognized flag character is a lex/parse error, same as real yq.
        assert!(parse_with_mode(".a *=x .b", ParserMode::Yq).is_err());
        assert!(parse_with_mode(".a *x .b", ParserMode::Yq).is_err());
    }

    #[test]
    fn test_jq_mode_rejects_merge_flags() {
        // Real jq has no merge-flag syntax at all — jq mode must not
        // recognize these tokens, regardless of what yq mode does.
        assert!(parse(".a *+ .b").is_err());
        assert!(parse(".a *=n .b").is_err());

        // Plain `*`/`*=` still work in jq mode, with default (all-false) flags.
        assert_eq!(
            parse(".a * .b").unwrap(),
            Expr::Arithmetic {
                op: ArithOp::Mul(MergeFlags::default()),
                left: Box::new(Expr::Field("a".into())),
                right: Box::new(Expr::Field("b".into())),
            }
        );
        assert_eq!(
            parse(".a *= .b").unwrap(),
            Expr::CompoundAssign {
                op: AssignOp::Mul(MergeFlags::default()),
                path: Box::new(Expr::Field("a".into())),
                value: Box::new(Expr::Field("b".into())),
            }
        );
    }

    #[test]
    fn test_yq_mode_keys_returns_document_order() {
        // In yq mode, `keys` returns document order (like yq), not sorted (like jq)
        // This matches yq's behavior where key order is preserved
        assert_eq!(
            parse_with_mode("keys", ParserMode::Yq).unwrap(),
            Expr::Builtin(Builtin::KeysUnsorted)
        );

        // In jq mode (default), `keys` returns sorted order
        assert_eq!(parse("keys").unwrap(), Expr::Builtin(Builtin::Keys));

        // keys_unsorted is available in both modes for explicit document order
        assert_eq!(
            parse_with_mode("keys_unsorted", ParserMode::Yq).unwrap(),
            Expr::Builtin(Builtin::KeysUnsorted)
        );
        assert_eq!(
            parse("keys_unsorted").unwrap(),
            Expr::Builtin(Builtin::KeysUnsorted)
        );
    }

    /// #1512: yq mode rejects jq-only builtins real yq's lexer lacks by
    /// default, and `parse_program_with_mode_and_extensions(.., true)`
    /// (the `--jq-extensions` CLI flag's parser-level counterpart) accepts
    /// them. jq mode is unaffected either way -- it already accepts this
    /// whole surface unconditionally, gate or no gate.
    ///
    /// `input`/`inputs`/`input_line_number` (#1507) are deliberately *not*
    /// in this list: unlike everything here, `--jq-extensions` never makes
    /// them parse in yq mode -- see
    /// `test_yq_input_builtins_rejected_in_yq_mode_regardless_of_extensions_1507`
    /// below.
    #[test]
    fn test_yq_mode_rejects_jq_only_builtins_unless_jq_extensions_1512() {
        let cases: &[(&str, &str)] = &[
            ("IN", "IN(1)"),
            ("ltrimstr", r#"ltrimstr("a")"#),
            ("splits", r#"splits(",")"#),
            ("gsub", r#"gsub("a";"b")"#),
            ("scan", r#"scan("a")"#),
            ("tostream", "tostream"),
            ("fromstream", "fromstream(.)"),
            ("truncate_stream", "truncate_stream(.)"),
            ("getpath", "getpath([])"),
            ("leaf_paths", "leaf_paths"),
            ("paths", "paths"),
            ("isnan", "isnan"),
            ("infinite", "infinite"),
            ("debug", "debug"),
            ("isempty", "isempty(empty)"),
            ("limit", "limit(1; .)"),
            ("skip", "skip(1; .)"),
            ("asin", "0.5 | asin"),
            ("acosh", "1.5 | acosh"),
            ("abs", "(-1.5) | abs"),
            ("trunc", "1.5 | trunc"),
            ("isinfinite", "1 | isinfinite"),
            ("nan", "nan"),
        ];

        for (name, filter) in cases {
            let rejected = parse_program_with_mode(filter, ParserMode::Yq);
            assert!(
                rejected.is_err(),
                "`{name}` should be rejected in yq mode by default, got: {rejected:?}"
            );
            let message = &rejected.unwrap_err().message;
            assert!(
                message.contains("--jq-extensions"),
                "`{name}`'s rejection message should mention --jq-extensions, got: {message}"
            );

            assert!(
                parse_program_with_mode_and_extensions(filter, ParserMode::Yq, true).is_ok(),
                "`{name}` should parse in yq mode once --jq-extensions is enabled"
            );

            assert!(
                parse_program_with_mode(filter, ParserMode::Jq).is_ok(),
                "`{name}` should parse in jq mode regardless of the yq-only gate"
            );
        }
    }

    /// #1507: `input`/`inputs`/`input_line_number` are the one jq-only
    /// class `--jq-extensions` does not unlock, unlike every name in
    /// [`test_yq_mode_rejects_jq_only_builtins_unless_jq_extensions_1512`]
    /// above. Routing them through that same flag-gated mechanism once
    /// reopened #1507's own bug one layer down: passing the flag let the
    /// keyword parse, so an *unreached* branch went right back to silently
    /// succeeding, since `input_builtins_unsupported_in_yq_mode`
    /// (`src/jq/eval.rs`) only fires when the builtin is actually
    /// evaluated. Rejecting in the parser unconditionally, regardless of
    /// the flag, closes that gap by construction.
    #[test]
    fn test_yq_input_builtins_rejected_in_yq_mode_regardless_of_extensions_1507() {
        for (name, filter) in [
            ("input", "input"),
            ("inputs", "inputs"),
            ("input_line_number", "input_line_number"),
        ] {
            for extensions in [false, true] {
                let rejected =
                    parse_program_with_mode_and_extensions(filter, ParserMode::Yq, extensions);
                assert!(
                    rejected.is_err(),
                    "`{name}` should be rejected in yq mode (extensions={extensions}), got: {rejected:?}"
                );
                let message = &rejected.unwrap_err().message;
                assert!(
                    message.contains("not supported in yq mode"),
                    "`{name}`'s rejection message should say it's not supported in yq mode, got: {message}"
                );
                assert!(
                    !message.contains("--jq-extensions"),
                    "`{name}`'s rejection should not suggest --jq-extensions, since it never helps: {message}"
                );
            }

            assert!(
                parse_program_with_mode(filter, ParserMode::Jq).is_ok(),
                "`{name}` should still parse in jq mode"
            );
        }
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
                    other => panic!("expected FuncCall or Var, got {other:?}"),
                }
            }
            _ => panic!("expected subtraction, got {expr:?}"),
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

    #[test]
    fn test_parse_error_paths() {
        // Unknown @format.
        assert!(parse("@foobar")
            .unwrap_err()
            .message
            .contains("unknown format"));
        // Invalid escape inside a string literal.
        assert!(parse(r#""\q""#)
            .unwrap_err()
            .message
            .contains("invalid escape"));
        // Unexpected character where an expression is expected.
        assert!(parse("%").is_err());
        // Array index/slice: expected ']' or ':'.
        assert!(parse(".[1 2]").is_err());
        // Object: expected ',' or '}'.
        assert!(parse("{a: 1 c: 2}").is_err());
        // Grouping: expected ')', found another char.
        assert!(parse("(.foo]").is_err());
        // Grouping: expected ')', found end of input.
        assert!(parse("(.foo").is_err());
    }

    #[test]
    fn test_parse_number_exponent_sign() {
        // Exercises the optional +/- sign branch in exponent parsing.
        assert!(parse("1e+5").is_ok());
        assert!(parse("1e-5").is_ok());
    }

    /// A `Pattern` nested past `MAX_PATTERN_DEPTH` returns a clean
    /// `ParseError` instead of overflowing the stack -- regression test for
    /// #1240 (`. as {a: {a: {a: ...}}} | ...` reachable from query text
    /// alone, no input document needed).
    #[test]
    fn test_pattern_depth_limit_returns_parse_error_not_overflow_1240() {
        let n = MAX_PATTERN_DEPTH + 10;
        let object_pattern = format!("{}$x{}", "{a: ".repeat(n), "}".repeat(n));
        let err = parse(&format!(". as {object_pattern} | $x"))
            .expect_err("over-depth object pattern must be a clean parse error");
        assert!(
            err.message.contains("depth limit"),
            "unexpected error: {}",
            err.message
        );

        let array_pattern = format!("{}$x{}", "[".repeat(n), "]".repeat(n));
        let err = parse(&format!(". as {array_pattern} | $x"))
            .expect_err("over-depth array pattern must be a clean parse error");
        assert!(
            err.message.contains("depth limit"),
            "unexpected error: {}",
            err.message
        );
    }

    /// Every prefix-recursive construct is capped, not just one (#1156).
    ///
    /// Before this guard `parse()` aborted the process with SIGABRT --
    /// before any evaluation ran -- on all of these. Each is buildable from
    /// query text alone with no input document, so the crash was fully
    /// within a client's control.
    /// Runs the body on a thread with a stack big enough for
    /// [`MAX_EXPR_DEPTH`] levels. Cargo's harness gives each test 2 MiB,
    /// which is smaller than the 8 MiB main thread the CLI actually uses and
    /// too small for the heaviest constructs at this limit -- see
    /// [`MAX_EXPR_DEPTH`]'s own note. Pinning the stack here keeps these
    /// tests measuring the guard rather than the harness.
    fn with_parser_stack(f: impl FnOnce() + Send + 'static) {
        std::thread::Builder::new()
            .stack_size(8 * 1024 * 1024)
            .spawn(f)
            .expect("spawn")
            .join()
            .expect("depth-guarded parse must not abort");
    }

    #[test]
    fn test_expr_depth_limit_returns_parse_error_not_overflow_1156() {
        with_parser_stack(|| {
            let n = MAX_EXPR_DEPTH + 10;
            for (label, src) in [
                ("unary minus", format!("{}5", "-".repeat(n))),
                ("parens", format!("{}5{}", "(".repeat(n), ")".repeat(n))),
                ("try", format!("{}5", "try ".repeat(n))),
                ("array", format!("{}{}", "[".repeat(n), "]".repeat(n))),
                ("object", format!("{}5{}", "{a: ".repeat(n), "}".repeat(n))),
            ] {
                let err = parse(&src).expect_err("over-depth input must be a clean parse error");
                assert!(
                    err.message.contains("depth limit"),
                    "{label}: unexpected error: {}",
                    err.message
                );
            }
        });
    }

    /// The same cap reaches chains built by *iteration*, not just recursion.
    ///
    /// `parse_additive` and friends loop rather than recurse, so
    /// `parse_primary`'s own counter never sees them -- but `1 + 1 + ... + 1`
    /// still builds a chain-length-deep tree. Left unguarded it aborted at
    /// 6206 terms in release and 596 in debug, so a guard that only covered
    /// the recursive constructs would have been trivially bypassable.
    #[test]
    fn test_binary_chain_depth_limit_returns_parse_error_1156() {
        let n = MAX_EXPR_DEPTH + 10;
        for op in ["+", "*", "and", "or", "//"] {
            let src = vec!["1"; n].join(&format!(" {op} "));
            let err =
                parse(&src).expect_err("over-length binary chain must be a clean parse error");
            assert!(
                err.message.contains("depth limit"),
                "{op}: unexpected error: {}",
                err.message
            );
        }
    }

    /// Control: `pipe` and `comma` build a flat `Vec`, not a nested tree, so
    /// they must stay uncapped -- a long pipeline is ordinary jq, and
    /// charging it against a nesting budget would be a real regression.
    #[test]
    fn test_flat_chains_are_not_charged_against_expr_depth_1156() {
        let n = MAX_EXPR_DEPTH * 4;
        parse(&vec![".a"; n].join(" | ")).expect("a long pipeline is not nesting");
        parse(&format!("[{}]", vec!["1"; n].join(", "))).expect("a long comma list is not nesting");
    }

    /// Control: just under the limit still parses, for the recursive and the
    /// iterative shape alike.
    #[test]
    fn test_expr_depth_just_under_limit_still_parses_1156() {
        with_parser_stack(|| {
            let n = MAX_EXPR_DEPTH - 1;
            parse(&format!("{}5{}", "(".repeat(n), ")".repeat(n)))
                .expect("under-limit parens parse");
            parse(&format!("{}5", "-".repeat(n))).expect("under-limit unary minus parses");
            parse(&vec!["1"; n].join(" + ")).expect("under-limit chain parses");
        });
    }

    /// Control: a pattern nested just under the limit still parses fine --
    /// the guard doesn't clip ordinary (if unusually deep) real-world usage.
    #[test]
    fn test_pattern_depth_just_under_limit_still_parses_1240() {
        let n = MAX_PATTERN_DEPTH - 1;
        let object_pattern = format!("{}$x{}", "{a: ".repeat(n), "}".repeat(n));
        assert!(parse(&format!(". as {object_pattern} | $x")).is_ok());
    }

    /// `Parser::new` (kept for tests and future use, per its own
    /// `#[allow(dead_code)]`; `parse`/`parse_with_mode` both go through
    /// `Parser::with_mode_and_extensions` instead) initializes
    /// `pattern_depth` to `0` the same as `with_mode_and_extensions`, and a
    /// parser built through it can still parse a pattern -- #1240's new
    /// field didn't leave this alternate constructor broken.
    #[test]
    fn test_parser_new_initializes_pattern_depth_1240() {
        let mut parser = Parser::new(". as {$a} | $a");
        parser.parse_expr().expect("Parser::new must still parse");
    }
}
