//! YAML parser (oracle) for Phase 3: YAML with block scalars.
//!
//! This module implements the sequential oracle that resolves YAML's
//! context-sensitive grammar and emits IB/BP/TY bits for index construction.
//!
//! # Phase 3 Scope
//!
//! - Block mappings and sequences
//! - Flow mappings `{key: value}` and sequences `[a, b, c]`
//! - Simple scalars (unquoted, double-quoted, single-quoted)
//! - **Block scalars: literal (`|`) and folded (`>`)**
//! - **Chomping modifiers: strip (`-`), keep (`+`), clip (default)**
//! - Comments (ignored in block context, not allowed in flow)
//! - Single document only

#[cfg(not(test))]
use alloc::{vec, vec::Vec};

use super::error::YamlError;

/// Node type in the YAML structure tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeType {
    /// Mapping (object-like): key-value pairs
    Mapping,
    /// Sequence (array-like): ordered list
    Sequence,
    /// Scalar value (string, number, etc.)
    #[allow(dead_code)]
    Scalar,
}

/// Block scalar style (literal or folded).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BlockStyle {
    /// Literal (`|`): preserves newlines exactly
    Literal,
    /// Folded (`>`): folds newlines to spaces
    Folded,
}

/// Chomping indicator for block scalars.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChompingIndicator {
    /// Clip (default): single trailing newline
    Clip,
    /// Strip (`-`): no trailing newlines
    Strip,
    /// Keep (`+`): preserve all trailing newlines
    Keep,
}

/// Block scalar header information.
#[derive(Debug)]
struct BlockScalarHeader {
    /// Literal or folded style (used for debugging/future extensions)
    #[allow(dead_code)]
    style: BlockStyle,
    /// Chomping behavior
    chomping: ChompingIndicator,
    /// Explicit indentation indicator (1-9), or 0 for auto-detect
    explicit_indent: u8,
}

/// Output from parsing: the semi-index structures.
#[derive(Debug)]
pub struct SemiIndex {
    /// Interest bits: marks positions of structural elements
    pub ib: Vec<u64>,
    /// Balanced parentheses: encodes tree structure
    pub bp: Vec<u64>,
    /// Type bits: 0 = mapping, 1 = sequence at each structural position
    pub ty: Vec<u64>,
    /// Direct mapping from BP open positions to text byte offsets.
    /// For each BP open (1-bit), this stores the corresponding byte offset.
    /// Containers may share position with first child.
    pub bp_to_text: Vec<u32>,
    /// Number of valid bits in IB (= input length)
    #[allow(dead_code)]
    pub ib_len: usize,
    /// Number of valid bits in BP
    pub bp_len: usize,
    /// Number of valid bits in TY (= number of container opens)
    #[allow(dead_code)]
    pub ty_len: usize,
}

/// Parser state for the YAML-lite oracle.
struct Parser<'a> {
    input: &'a [u8],
    pos: usize,
    line: usize,

    // Index builders
    ib_words: Vec<u64>,
    bp_words: Vec<u64>,
    ty_words: Vec<u64>,
    bp_pos: usize,
    ty_pos: usize,

    // Direct BP-to-text mapping
    bp_to_text: Vec<u32>,

    // Indentation tracking
    indent_stack: Vec<usize>,

    // Node type stack (to track if we're in mapping or sequence)
    type_stack: Vec<NodeType>,
}

impl<'a> Parser<'a> {
    fn new(input: &'a [u8]) -> Self {
        let ib_words = vec![0u64; input.len().div_ceil(64).max(1)];
        let bp_words = vec![0u64; input.len().div_ceil(32).max(1)]; // ~2x IB for BP
        let ty_words = vec![0u64; input.len().div_ceil(64).max(1)];

        Self {
            input,
            pos: 0,
            line: 1,
            ib_words,
            bp_words,
            ty_words,
            bp_pos: 0,
            ty_pos: 0,
            bp_to_text: Vec::new(),
            indent_stack: vec![0], // Start at indent 0
            type_stack: Vec::new(),
        }
    }

    /// Set an interest bit at the current position.
    #[inline]
    fn set_ib(&mut self) {
        let word_idx = self.pos / 64;
        let bit_idx = self.pos % 64;
        if word_idx < self.ib_words.len() {
            self.ib_words[word_idx] |= 1u64 << bit_idx;
        }
    }

    /// Set an interest bit at a specific position.
    #[inline]
    #[allow(dead_code)]
    fn set_ib_at(&mut self, pos: usize) {
        let word_idx = pos / 64;
        let bit_idx = pos % 64;
        if word_idx < self.ib_words.len() {
            self.ib_words[word_idx] |= 1u64 << bit_idx;
        }
    }

    /// Write an open parenthesis (1) to BP at the current text position.
    #[inline]
    fn write_bp_open(&mut self) {
        self.write_bp_open_at(self.pos);
    }

    /// Write an open parenthesis (1) to BP at a specific text position.
    #[inline]
    fn write_bp_open_at(&mut self, text_pos: usize) {
        let word_idx = self.bp_pos / 64;
        let bit_idx = self.bp_pos % 64;
        // Ensure capacity
        while word_idx >= self.bp_words.len() {
            self.bp_words.push(0);
        }
        self.bp_words[word_idx] |= 1u64 << bit_idx;
        // Record the text position for this BP open
        self.bp_to_text.push(text_pos as u32);
        self.bp_pos += 1;
    }

    /// Write a close parenthesis (0) to BP.
    #[inline]
    fn write_bp_close(&mut self) {
        let word_idx = self.bp_pos / 64;
        // Ensure capacity
        while word_idx >= self.bp_words.len() {
            self.bp_words.push(0);
        }
        // Close is 0, which is default, so just increment position
        self.bp_pos += 1;
    }

    /// Write a type bit: 0 = mapping, 1 = sequence.
    #[inline]
    fn write_ty(&mut self, is_sequence: bool) {
        let word_idx = self.ty_pos / 64;
        let bit_idx = self.ty_pos % 64;
        while word_idx >= self.ty_words.len() {
            self.ty_words.push(0);
        }
        if is_sequence {
            self.ty_words[word_idx] |= 1u64 << bit_idx;
        }
        self.ty_pos += 1;
    }

    /// Get current byte without advancing.
    #[inline]
    fn peek(&self) -> Option<u8> {
        self.input.get(self.pos).copied()
    }

    /// Get byte at offset from current position.
    #[inline]
    fn peek_at(&self, offset: usize) -> Option<u8> {
        self.input.get(self.pos + offset).copied()
    }

    /// Advance position by one byte.
    #[inline]
    fn advance(&mut self) {
        if self.pos < self.input.len() {
            if self.input[self.pos] == b'\n' {
                self.line += 1;
            }
            self.pos += 1;
        }
    }

    /// Skip whitespace on the current line (spaces only, not newlines).
    fn skip_inline_whitespace(&mut self) {
        while let Some(b) = self.peek() {
            if b == b' ' {
                self.advance();
            } else {
                break;
            }
        }
    }

    /// Count leading spaces (indentation) at start of a line.
    fn count_indent(&self) -> Result<usize, YamlError> {
        let mut count = 0;
        let mut i = self.pos;
        while i < self.input.len() {
            match self.input[i] {
                b' ' => {
                    count += 1;
                    i += 1;
                }
                b'\t' => {
                    return Err(YamlError::TabIndentation {
                        line: self.line,
                        offset: i,
                    });
                }
                _ => break,
            }
        }
        Ok(count)
    }

    /// Check if at end of meaningful content on this line.
    fn at_line_end(&self) -> bool {
        let mut i = self.pos;
        while i < self.input.len() {
            match self.input[i] {
                b'\n' => return true,
                b'#' => return true, // Comment starts
                b' ' => i += 1,
                _ => return false,
            }
        }
        true // EOF counts as line end
    }

    /// Skip to end of line (handles comments).
    fn skip_to_eol(&mut self) {
        while let Some(b) = self.peek() {
            if b == b'\n' {
                break;
            }
            self.advance();
        }
    }

    /// Skip newline and empty/comment lines.
    fn skip_newlines(&mut self) {
        while let Some(b) = self.peek() {
            if b == b'\n' {
                self.advance();
            } else if b == b'#' {
                // Comment line
                self.skip_to_eol();
            } else if b == b' ' {
                // Check if rest of line is whitespace or comment
                let start = self.pos;
                self.skip_inline_whitespace();
                if self.peek() == Some(b'\n') || self.peek() == Some(b'#') || self.peek().is_none()
                {
                    if self.peek() == Some(b'#') {
                        self.skip_to_eol();
                    }
                    continue;
                } else {
                    // Non-empty content - back up
                    self.pos = start;
                    break;
                }
            } else {
                break;
            }
        }
    }

    /// Check for unsupported YAML features (Phase 2: flow style is now supported).
    fn check_unsupported(&self) -> Result<(), YamlError> {
        if let Some(b) = self.peek() {
            match b {
                // Flow style is supported in Phase 2+
                b'{' | b'[' => {
                    // Allowed - will be parsed by parse_flow_*
                }
                // Block scalars are supported in Phase 3+
                b'|' | b'>' => {
                    // Allowed - will be parsed by parse_block_scalar()
                }
                b'&' | b'*' => {
                    return Err(YamlError::AnchorAliasNotSupported { offset: self.pos });
                }
                b'?' => {
                    // Check if it's explicit key (? at start of content + space)
                    if self.peek_at(1) == Some(b' ') || self.peek_at(1) == Some(b'\n') {
                        return Err(YamlError::ExplicitKeyNotSupported { offset: self.pos });
                    }
                }
                b'!' => {
                    return Err(YamlError::TagNotSupported { offset: self.pos });
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Check for document markers.
    fn check_document_marker(&self) -> Result<(), YamlError> {
        if self.pos + 2 < self.input.len() {
            let slice = &self.input[self.pos..self.pos + 3];
            if slice == b"---" || slice == b"..." {
                // Check if it's at start of line (or followed by space/newline)
                if self.peek_at(3) == Some(b' ')
                    || self.peek_at(3) == Some(b'\n')
                    || self.peek_at(3).is_none()
                {
                    return Err(YamlError::MultiDocumentNotSupported { offset: self.pos });
                }
            }
        }
        Ok(())
    }

    /// Parse a double-quoted string.
    fn parse_double_quoted(&mut self) -> Result<usize, YamlError> {
        let start = self.pos;
        self.advance(); // Skip opening quote

        while let Some(b) = self.peek() {
            match b {
                b'"' => {
                    self.advance();
                    return Ok(self.pos - start);
                }
                b'\\' => {
                    self.advance(); // Skip backslash
                    if self.peek().is_some() {
                        self.advance(); // Skip escaped char
                    } else {
                        return Err(YamlError::UnexpectedEof {
                            context: "escape sequence in string",
                        });
                    }
                }
                b'\n' => {
                    // Multi-line string - continue
                    self.advance();
                }
                _ => self.advance(),
            }
        }

        Err(YamlError::UnclosedQuote {
            start_offset: start,
            quote_type: '"',
        })
    }

    /// Parse a single-quoted string.
    fn parse_single_quoted(&mut self) -> Result<usize, YamlError> {
        let start = self.pos;
        self.advance(); // Skip opening quote

        while let Some(b) = self.peek() {
            match b {
                b'\'' => {
                    // Check for escaped quote ('')
                    if self.peek_at(1) == Some(b'\'') {
                        self.advance();
                        self.advance();
                    } else {
                        self.advance();
                        return Ok(self.pos - start);
                    }
                }
                b'\n' => {
                    // Multi-line string - continue
                    self.advance();
                }
                _ => self.advance(),
            }
        }

        Err(YamlError::UnclosedQuote {
            start_offset: start,
            quote_type: '\'',
        })
    }

    /// Parse an unquoted scalar value (stops at colon+space, newline, or comment).
    fn parse_unquoted_value(&mut self) -> usize {
        let start = self.pos;

        while let Some(b) = self.peek() {
            match b {
                b'\n' | b'#' => break,
                b':' => {
                    // Colon followed by space ends the value (could be a key)
                    // But in value context, colons in URLs etc. are allowed
                    // For Phase 1, we stop at colon + space
                    if self.peek_at(1) == Some(b' ') || self.peek_at(1) == Some(b'\n') {
                        break;
                    }
                    self.advance();
                }
                _ => self.advance(),
            }
        }

        // Trim trailing whitespace
        let mut end = self.pos;
        while end > start && self.input[end - 1] == b' ' {
            end -= 1;
        }

        end - start
    }

    /// Parse an unquoted key (stops at colon+space).
    fn parse_unquoted_key(&mut self) -> Result<usize, YamlError> {
        let start = self.pos;

        while let Some(b) = self.peek() {
            match b {
                b':' => {
                    // Check for colon + space or colon + newline
                    if self.peek_at(1) == Some(b' ')
                        || self.peek_at(1) == Some(b'\n')
                        || self.peek_at(1).is_none()
                    {
                        break;
                    }
                    // Colon without space might be part of the key (e.g., URL-like)
                    // In strict mode, we could error here
                    return Err(YamlError::ColonWithoutSpace { offset: self.pos });
                }
                b'\n' | b'#' => {
                    // Key without colon
                    return Err(YamlError::KeyWithoutValue {
                        offset: start,
                        line: self.line,
                    });
                }
                _ => self.advance(),
            }
        }

        // Trim trailing whitespace
        let mut end = self.pos;
        while end > start && self.input[end - 1] == b' ' {
            end -= 1;
        }

        if end == start {
            return Err(YamlError::UnexpectedEof { context: "key" });
        }

        Ok(end - start)
    }

    /// Close containers that are at higher indent levels.
    fn close_deeper_indents(&mut self, new_indent: usize) {
        while self.indent_stack.len() > 1 {
            let current_indent = *self.indent_stack.last().unwrap();
            if current_indent >= new_indent {
                self.indent_stack.pop();
                self.type_stack.pop();
                self.write_bp_close();
            } else {
                break;
            }
        }
    }

    /// Parse a sequence item (starts with `- `).
    fn parse_sequence_item(&mut self, indent: usize) -> Result<(), YamlError> {
        let _item_start = self.pos;

        // Mark the `-` position
        self.set_ib();

        // Check if we need to open a new sequence
        let need_new_sequence = self.type_stack.last() != Some(&NodeType::Sequence)
            || self.indent_stack.last().copied() != Some(indent);

        if need_new_sequence {
            // Close any deeper containers first
            self.close_deeper_indents(indent);

            // Open new sequence
            self.write_bp_open();
            self.write_ty(true); // 1 = sequence
            self.indent_stack.push(indent);
            self.type_stack.push(NodeType::Sequence);
        }

        // Open the sequence item node
        self.write_bp_open();

        // Skip `- `
        self.advance(); // -
        self.skip_inline_whitespace();

        self.check_unsupported()?;

        // Check what follows
        if self.at_line_end() {
            // Empty item or nested content on next line
            // The value is implicit null or will be a nested structure
            self.write_bp_close(); // Close the item
            return Ok(());
        }

        // Parse the item value
        // Pass structure indent for block scalars (content must be > this)
        self.parse_value(indent)?;

        // Close the sequence item
        self.write_bp_close();

        Ok(())
    }

    /// Parse a mapping key-value pair.
    fn parse_mapping_entry(&mut self, indent: usize) -> Result<(), YamlError> {
        let _entry_start = self.pos;

        // Check if we need to open a new mapping
        let need_new_mapping = self.type_stack.last() != Some(&NodeType::Mapping)
            || self.indent_stack.last().copied() != Some(indent);

        if need_new_mapping {
            // Close any deeper containers first
            self.close_deeper_indents(indent);

            // Open new mapping (virtual - no IB bit, children will have IB)
            self.write_bp_open();
            self.write_ty(false); // 0 = mapping
            self.indent_stack.push(indent);
            self.type_stack.push(NodeType::Mapping);
        }

        // Mark key position
        self.set_ib();

        // Open key node
        self.write_bp_open();

        // Parse the key
        let _key_len = match self.peek() {
            Some(b'"') => self.parse_double_quoted()?,
            Some(b'\'') => self.parse_single_quoted()?,
            _ => self.parse_unquoted_key()?,
        };

        // Close key node
        self.write_bp_close();

        // Expect colon
        if self.peek() != Some(b':') {
            return Err(YamlError::UnexpectedCharacter {
                offset: self.pos,
                char: self.peek().map(|b| b as char).unwrap_or('\0'),
                context: "expected ':' after key",
            });
        }
        self.advance(); // Skip ':'

        // Skip space after colon
        self.skip_inline_whitespace();

        // Parse value
        if self.at_line_end() {
            // Value is on next line (nested structure or explicit null)
            // Open a placeholder value node
            self.set_ib();
            self.write_bp_open();
            self.skip_to_eol();
            self.write_bp_close();
        } else {
            self.check_unsupported()?;
            // Check for flow style or block scalar - these handle their own BP
            match self.peek() {
                Some(b'[') => {
                    self.parse_flow_sequence()?;
                }
                Some(b'{') => {
                    self.parse_flow_mapping()?;
                }
                Some(b'|') | Some(b'>') => {
                    // Block scalar - handles its own BP
                    self.parse_block_scalar(indent)?;
                }
                _ => {
                    // Scalar value - wrap in BP
                    self.set_ib();
                    self.write_bp_open();
                    self.parse_inline_value()?;
                    self.write_bp_close();
                }
            }
        }

        Ok(())
    }

    /// Parse an inline scalar value (on the same line as the key).
    fn parse_inline_value(&mut self) -> Result<(), YamlError> {
        match self.peek() {
            Some(b'"') => {
                self.parse_double_quoted()?;
            }
            Some(b'\'') => {
                self.parse_single_quoted()?;
            }
            _ => {
                self.parse_unquoted_value();
            }
        }
        Ok(())
    }

    /// Parse a value (could be scalar or nested structure).
    fn parse_value(&mut self, min_indent: usize) -> Result<(), YamlError> {
        self.check_unsupported()?;

        match self.peek() {
            Some(b'"') => {
                self.set_ib();
                self.write_bp_open();
                self.parse_double_quoted()?;
                self.write_bp_close();
            }
            Some(b'\'') => {
                self.set_ib();
                self.write_bp_open();
                self.parse_single_quoted()?;
                self.write_bp_close();
            }
            Some(b'-') if self.peek_at(1) == Some(b' ') => {
                // Inline sequence item - this creates a nested sequence
                // The caller already opened a BP node for us
            }
            Some(b'[') => {
                // Flow sequence
                self.parse_flow_sequence()?;
            }
            Some(b'{') => {
                // Flow mapping
                self.parse_flow_mapping()?;
            }
            Some(b'|') | Some(b'>') => {
                // Block scalar - handles its own BP
                self.parse_block_scalar(min_indent)?;
            }
            _ => {
                self.set_ib();
                self.write_bp_open();
                self.parse_unquoted_value();
                self.write_bp_close();
            }
        }
        Ok(())
    }

    // =========================================================================
    // Flow style parsing (Phase 2)
    // =========================================================================

    /// Skip whitespace in flow context (spaces, tabs, newlines).
    /// Unlike block context, newlines are allowed within flow constructs.
    fn skip_flow_whitespace(&mut self) {
        while let Some(b) = self.peek() {
            match b {
                b' ' | b'\t' | b'\n' | b'\r' => self.advance(),
                _ => break,
            }
        }
    }

    /// Parse a flow sequence: `[item1, item2, ...]`
    fn parse_flow_sequence(&mut self) -> Result<(), YamlError> {
        // Mark the `[` position
        self.set_ib();

        // Open sequence container
        self.write_bp_open();
        self.write_ty(true); // 1 = sequence

        // Skip `[`
        self.advance();
        self.skip_flow_whitespace();

        // Parse items
        let mut first = true;
        while self.peek() != Some(b']') {
            if self.peek().is_none() {
                return Err(YamlError::UnexpectedEof {
                    context: "flow sequence",
                });
            }

            if !first {
                // Expect comma
                if self.peek() != Some(b',') {
                    return Err(YamlError::UnexpectedCharacter {
                        offset: self.pos,
                        char: self.peek().map(|b| b as char).unwrap_or('\0'),
                        context: "expected ',' or ']' in flow sequence",
                    });
                }
                self.advance(); // Skip `,`
                self.skip_flow_whitespace();

                // Allow trailing comma
                if self.peek() == Some(b']') {
                    break;
                }
            }
            first = false;

            // Parse flow value (item) - containers handle their own BP
            match self.peek() {
                Some(b'[') => {
                    self.parse_flow_sequence()?;
                }
                Some(b'{') => {
                    self.parse_flow_mapping()?;
                }
                _ => {
                    // Scalar value - wrap in BP
                    self.set_ib();
                    self.write_bp_open();
                    self.parse_flow_scalar()?;
                    self.write_bp_close();
                }
            }
            self.skip_flow_whitespace();
        }

        // Skip `]`
        if self.peek() == Some(b']') {
            self.set_ib();
            self.advance();
        }

        // Close sequence
        self.write_bp_close();

        Ok(())
    }

    /// Parse a flow mapping: `{key: value, ...}`
    fn parse_flow_mapping(&mut self) -> Result<(), YamlError> {
        // Mark the `{` position
        self.set_ib();

        // Open mapping container
        self.write_bp_open();
        self.write_ty(false); // 0 = mapping

        // Skip `{`
        self.advance();
        self.skip_flow_whitespace();

        // Parse key-value pairs
        let mut first = true;
        while self.peek() != Some(b'}') {
            if self.peek().is_none() {
                return Err(YamlError::UnexpectedEof {
                    context: "flow mapping",
                });
            }

            if !first {
                // Expect comma
                if self.peek() != Some(b',') {
                    return Err(YamlError::UnexpectedCharacter {
                        offset: self.pos,
                        char: self.peek().map(|b| b as char).unwrap_or('\0'),
                        context: "expected ',' or '}' in flow mapping",
                    });
                }
                self.advance(); // Skip `,`
                self.skip_flow_whitespace();

                // Allow trailing comma
                if self.peek() == Some(b'}') {
                    break;
                }
            }
            first = false;

            // Parse key
            self.set_ib();
            self.write_bp_open();
            self.parse_flow_key()?;
            self.write_bp_close();

            self.skip_flow_whitespace();

            // Expect colon
            if self.peek() != Some(b':') {
                return Err(YamlError::UnexpectedCharacter {
                    offset: self.pos,
                    char: self.peek().map(|b| b as char).unwrap_or('\0'),
                    context: "expected ':' after key in flow mapping",
                });
            }
            self.advance(); // Skip `:`
            self.skip_flow_whitespace();

            // Parse value - for nested containers, they handle their own BP
            match self.peek() {
                Some(b'[') => {
                    self.parse_flow_sequence()?;
                }
                Some(b'{') => {
                    self.parse_flow_mapping()?;
                }
                _ => {
                    // Scalar value - wrap in BP
                    self.set_ib();
                    self.write_bp_open();
                    self.parse_flow_scalar()?;
                    self.write_bp_close();
                }
            }

            self.skip_flow_whitespace();
        }

        // Skip `}`
        if self.peek() == Some(b'}') {
            self.set_ib();
            self.advance();
        }

        // Close mapping
        self.write_bp_close();

        Ok(())
    }

    /// Parse a key in flow context (scalar only, no containers).
    fn parse_flow_key(&mut self) -> Result<(), YamlError> {
        match self.peek() {
            Some(b'"') => {
                self.parse_double_quoted()?;
            }
            Some(b'\'') => {
                self.parse_single_quoted()?;
            }
            _ => {
                self.parse_flow_unquoted_key()?;
            }
        }
        Ok(())
    }

    /// Parse an unquoted key in flow context.
    /// Stops at `:`, `,`, `}`, `]`, or whitespace before those.
    fn parse_flow_unquoted_key(&mut self) -> Result<usize, YamlError> {
        let start = self.pos;

        while let Some(b) = self.peek() {
            match b {
                b':' | b',' | b'}' | b']' | b'\n' | b'\r' => break,
                b' ' | b'\t' => {
                    // Check if whitespace is followed by a delimiter
                    let mut lookahead = self.pos + 1;
                    while lookahead < self.input.len() {
                        match self.input[lookahead] {
                            b' ' | b'\t' => lookahead += 1,
                            b':' | b',' | b'}' | b']' => {
                                // Whitespace before delimiter - stop here
                                break;
                            }
                            _ => {
                                // Continue with the key
                                self.advance();
                                break;
                            }
                        }
                    }
                    if lookahead == self.input.len()
                        || matches!(
                            self.input[lookahead],
                            b':' | b',' | b'}' | b']' | b'\n' | b'\r'
                        )
                    {
                        break;
                    }
                }
                _ => self.advance(),
            }
        }

        // Trim trailing whitespace
        let mut end = self.pos;
        while end > start && matches!(self.input[end - 1], b' ' | b'\t') {
            end -= 1;
        }

        if end == start {
            return Err(YamlError::UnexpectedEof {
                context: "flow key",
            });
        }

        Ok(end - start)
    }

    /// Parse a scalar value in flow context (string or unquoted).
    fn parse_flow_scalar(&mut self) -> Result<(), YamlError> {
        match self.peek() {
            Some(b'"') => {
                self.parse_double_quoted()?;
            }
            Some(b'\'') => {
                self.parse_single_quoted()?;
            }
            _ => {
                self.parse_flow_unquoted_value();
            }
        }
        Ok(())
    }

    /// Parse an unquoted value in flow context.
    /// Stops at `,`, `}`, `]`, or newline.
    fn parse_flow_unquoted_value(&mut self) -> usize {
        let start = self.pos;

        while let Some(b) = self.peek() {
            match b {
                b',' | b'}' | b']' | b'\n' | b'\r' => break,
                _ => self.advance(),
            }
        }

        // Trim trailing whitespace
        let mut end = self.pos;
        while end > start && matches!(self.input[end - 1], b' ' | b'\t') {
            end -= 1;
        }

        end - start
    }

    // =========================================================================
    // Block scalar parsing (Phase 3)
    // =========================================================================

    /// Parse the header of a block scalar (indicator + modifiers).
    /// Returns the header info and advances past the header.
    fn parse_block_scalar_header(&mut self) -> Result<BlockScalarHeader, YamlError> {
        let style = match self.peek() {
            Some(b'|') => BlockStyle::Literal,
            Some(b'>') => BlockStyle::Folded,
            _ => {
                return Err(YamlError::UnexpectedCharacter {
                    offset: self.pos,
                    char: self.peek().map(|b| b as char).unwrap_or('\0'),
                    context: "expected block scalar indicator (| or >)",
                });
            }
        };
        self.advance(); // consume indicator

        let mut chomping = ChompingIndicator::Clip;
        let mut explicit_indent: u8 = 0;

        // Parse optional modifiers (order can vary: |2- or |-2)
        for _ in 0..2 {
            match self.peek() {
                Some(b'-') => {
                    chomping = ChompingIndicator::Strip;
                    self.advance();
                }
                Some(b'+') => {
                    chomping = ChompingIndicator::Keep;
                    self.advance();
                }
                Some(c) if c.is_ascii_digit() && c != b'0' => {
                    explicit_indent = c - b'0';
                    self.advance();
                }
                _ => break,
            }
        }

        Ok(BlockScalarHeader {
            style,
            chomping,
            explicit_indent,
        })
    }

    /// Detect content indentation from the first non-empty line.
    /// Returns the indentation level, or None if block is empty.
    fn detect_block_content_indent(&mut self, base_indent: usize) -> Option<usize> {
        let saved_pos = self.pos;
        let saved_line = self.line;

        // Scan ahead to find first non-empty line
        loop {
            if self.peek().is_none() {
                // EOF - empty block scalar
                self.pos = saved_pos;
                self.line = saved_line;
                return None;
            }

            // Count spaces at start of line
            let mut indent = 0;
            while self.peek() == Some(b' ') {
                indent += 1;
                self.advance();
            }

            // Check what's on this line
            match self.peek() {
                Some(b'\n') => {
                    // Empty line - skip and continue
                    self.advance();
                    self.line += 1;
                }
                Some(b'#') => {
                    // Comment line - skip to end
                    self.skip_to_eol();
                    if self.peek() == Some(b'\n') {
                        self.advance();
                        self.line += 1;
                    }
                }
                Some(b'\r') => {
                    // Handle \r\n
                    self.advance();
                    if self.peek() == Some(b'\n') {
                        self.advance();
                    }
                    self.line += 1;
                }
                None => {
                    // EOF
                    self.pos = saved_pos;
                    self.line = saved_line;
                    return None;
                }
                _ => {
                    // Found content - restore position and return indent
                    self.pos = saved_pos;
                    self.line = saved_line;

                    if indent <= base_indent {
                        // Content must be more indented than indicator
                        return None;
                    }
                    return Some(indent);
                }
            }
        }
    }

    /// Consume block scalar content lines until indentation drops.
    /// Returns the end position of the content (before trailing newlines based on chomping).
    fn consume_block_scalar_content(
        &mut self,
        content_indent: usize,
        chomping: ChompingIndicator,
    ) -> usize {
        let mut last_content_end = self.pos;
        let mut trailing_newline_start = self.pos;

        loop {
            if self.peek().is_none() {
                break; // EOF ends block scalar
            }

            let line_start = self.pos;

            // Count spaces at start of line
            let mut line_indent = 0;
            while self.peek() == Some(b' ') {
                line_indent += 1;
                self.advance();
            }

            // Check what's on this line
            match self.peek() {
                Some(b'\n') => {
                    // Empty line - include in content (counts as trailing newline area)
                    trailing_newline_start = line_start;
                    self.advance();
                    self.line += 1;
                }
                Some(b'\r') => {
                    // Handle \r\n
                    trailing_newline_start = line_start;
                    self.advance();
                    if self.peek() == Some(b'\n') {
                        self.advance();
                    }
                    self.line += 1;
                }
                Some(b'#') if line_indent < content_indent => {
                    // Comment at lower indent - end of block
                    self.pos = line_start;
                    break;
                }
                None => {
                    break; // EOF
                }
                _ => {
                    if line_indent < content_indent {
                        // Real content at lower indent - end of block scalar
                        self.pos = line_start;
                        break;
                    }

                    // This is a content line - skip to end
                    self.skip_to_eol();
                    last_content_end = self.pos;
                    trailing_newline_start = self.pos;

                    if self.peek() == Some(b'\n') {
                        self.advance();
                        self.line += 1;
                    } else if self.peek() == Some(b'\r') {
                        self.advance();
                        if self.peek() == Some(b'\n') {
                            self.advance();
                        }
                        self.line += 1;
                    }
                }
            }
        }

        // Return position based on chomping
        match chomping {
            ChompingIndicator::Strip => last_content_end,
            ChompingIndicator::Clip => {
                // Include one trailing newline if there was content
                if last_content_end > 0 && trailing_newline_start > last_content_end {
                    last_content_end + 1 // Include one newline
                } else {
                    last_content_end
                }
            }
            ChompingIndicator::Keep => self.pos, // Include all trailing newlines
        }
    }

    /// Parse a block scalar (| or >) including all content lines.
    fn parse_block_scalar(&mut self, base_indent: usize) -> Result<(), YamlError> {
        // Mark the indicator position
        self.set_ib();
        self.write_bp_open();

        // Parse the header
        let header = self.parse_block_scalar_header()?;

        // Skip to end of indicator line (may have trailing comment)
        self.skip_to_eol();
        if self.peek() == Some(b'\n') {
            self.advance();
            self.line += 1;
        } else if self.peek() == Some(b'\r') {
            self.advance();
            if self.peek() == Some(b'\n') {
                self.advance();
            }
            self.line += 1;
        }

        // Determine content indentation
        let content_indent = if header.explicit_indent > 0 {
            base_indent + header.explicit_indent as usize
        } else {
            // Auto-detect from first content line
            match self.detect_block_content_indent(base_indent) {
                Some(indent) => indent,
                None => {
                    // Empty block scalar
                    self.write_bp_close();
                    return Ok(());
                }
            }
        };

        // Consume content lines
        let _content_end = self.consume_block_scalar_content(content_indent, header.chomping);

        // Close the block scalar node
        self.write_bp_close();

        Ok(())
    }

    /// Main parsing loop.
    fn parse(&mut self) -> Result<SemiIndex, YamlError> {
        if self.input.is_empty() {
            return Err(YamlError::EmptyInput);
        }

        // Skip initial whitespace and comments
        self.skip_newlines();

        if self.peek().is_none() {
            return Err(YamlError::EmptyInput);
        }

        // Check for document markers at start
        self.check_document_marker()?;

        // Parse the root structure
        self.parse_root()?;

        // Close any remaining open containers
        while self.indent_stack.len() > 1 {
            self.indent_stack.pop();
            self.type_stack.pop();
            self.write_bp_close();
        }

        // Close root if we opened one
        if !self.type_stack.is_empty() {
            self.write_bp_close();
        }

        Ok(SemiIndex {
            ib: self.ib_words.clone(),
            bp: self.bp_words.clone(),
            ty: self.ty_words.clone(),
            bp_to_text: self.bp_to_text.clone(),
            ib_len: self.input.len(),
            bp_len: self.bp_pos,
            ty_len: self.ty_pos,
        })
    }

    /// Parse the root structure.
    fn parse_root(&mut self) -> Result<(), YamlError> {
        while self.pos < self.input.len() {
            self.skip_newlines();

            if self.peek().is_none() {
                break;
            }

            self.check_document_marker()?;
            self.check_unsupported()?;

            // Count indentation
            let indent = self.count_indent()?;

            // Skip to content
            let _content_start = self.pos;
            for _ in 0..indent {
                self.advance();
            }

            // Check what kind of content this is
            match self.peek() {
                Some(b'-') if self.peek_at(1) == Some(b' ') => {
                    self.parse_sequence_item(indent)?;
                }
                Some(b'#') => {
                    // Comment line - skip
                    self.skip_to_eol();
                }
                Some(b'\n') => {
                    // Empty line
                    self.advance();
                }
                Some(_) => {
                    self.parse_mapping_entry(indent)?;
                }
                None => break,
            }

            // Move to next line if we haven't already
            if self.peek() == Some(b'\n') {
                self.advance();
            }
        }

        Ok(())
    }
}

/// Build a semi-index from YAML input.
pub fn build_semi_index(input: &[u8]) -> Result<SemiIndex, YamlError> {
    let mut parser = Parser::new(input);
    parser.parse()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_mapping() {
        let yaml = b"name: Alice";
        let result = build_semi_index(yaml);
        assert!(result.is_ok());
        let index = result.unwrap();
        assert!(index.bp_len > 0);
    }

    #[test]
    fn test_simple_sequence() {
        let yaml = b"- item1\n- item2";
        let result = build_semi_index(yaml);
        assert!(result.is_ok());
    }

    #[test]
    fn test_nested_mapping() {
        let yaml = b"person:\n  name: Alice\n  age: 30";
        let result = build_semi_index(yaml);
        assert!(result.is_ok());
    }

    #[test]
    fn test_double_quoted_string() {
        let yaml = b"name: \"Alice\"";
        let result = build_semi_index(yaml);
        assert!(result.is_ok());
    }

    #[test]
    fn test_single_quoted_string() {
        let yaml = b"name: 'Alice'";
        let result = build_semi_index(yaml);
        assert!(result.is_ok());
    }

    #[test]
    fn test_comment() {
        let yaml = b"# This is a comment\nname: Alice";
        let result = build_semi_index(yaml);
        assert!(result.is_ok());
    }

    #[test]
    fn test_inline_comment() {
        let yaml = b"name: Alice # inline comment";
        let result = build_semi_index(yaml);
        assert!(result.is_ok());
    }

    #[test]
    fn test_tab_indentation_error() {
        let yaml = b"name:\n\tvalue";
        let result = build_semi_index(yaml);
        assert!(matches!(result, Err(YamlError::TabIndentation { .. })));
    }

    #[test]
    fn test_flow_sequence() {
        let yaml = b"items: [1, 2, 3]";
        let result = build_semi_index(yaml);
        assert!(result.is_ok(), "Flow sequence should parse: {:?}", result);
    }

    #[test]
    fn test_flow_mapping() {
        let yaml = b"person: {name: Alice, age: 30}";
        let result = build_semi_index(yaml);
        assert!(result.is_ok(), "Flow mapping should parse: {:?}", result);
    }

    #[test]
    fn test_flow_nested() {
        let yaml = b"data: {users: [{name: Alice}, {name: Bob}]}";
        let result = build_semi_index(yaml);
        assert!(result.is_ok(), "Nested flow should parse: {:?}", result);
    }

    #[test]
    fn test_flow_with_strings() {
        let yaml = b"items: [\"hello\", 'world', plain]";
        let result = build_semi_index(yaml);
        assert!(
            result.is_ok(),
            "Flow with strings should parse: {:?}",
            result
        );
    }

    #[test]
    fn test_flow_trailing_comma() {
        let yaml = b"items: [1, 2, 3,]";
        let result = build_semi_index(yaml);
        assert!(
            result.is_ok(),
            "Flow with trailing comma should parse: {:?}",
            result
        );
    }

    #[test]
    fn test_flow_empty_sequence() {
        let yaml = b"items: []";
        let result = build_semi_index(yaml);
        assert!(
            result.is_ok(),
            "Empty flow sequence should parse: {:?}",
            result
        );
    }

    #[test]
    fn test_flow_empty_mapping() {
        let yaml = b"data: {}";
        let result = build_semi_index(yaml);
        assert!(
            result.is_ok(),
            "Empty flow mapping should parse: {:?}",
            result
        );
    }

    #[test]
    fn test_empty_input() {
        let yaml = b"";
        let result = build_semi_index(yaml);
        assert!(matches!(result, Err(YamlError::EmptyInput)));
    }

    #[test]
    fn test_whitespace_only() {
        let yaml = b"   \n\n  ";
        let result = build_semi_index(yaml);
        assert!(matches!(result, Err(YamlError::EmptyInput)));
    }

    // =========================================================================
    // Block scalar tests (Phase 3)
    // =========================================================================

    #[test]
    fn test_block_literal_basic() {
        let yaml = b"text: |\n  line1\n  line2\n";
        let result = build_semi_index(yaml);
        assert!(result.is_ok(), "Block literal should parse: {:?}", result);
    }

    #[test]
    fn test_block_folded_basic() {
        let yaml = b"text: >\n  line1\n  line2\n";
        let result = build_semi_index(yaml);
        assert!(result.is_ok(), "Block folded should parse: {:?}", result);
    }

    #[test]
    fn test_block_literal_strip() {
        let yaml = b"text: |-\n  content\n";
        let result = build_semi_index(yaml);
        assert!(
            result.is_ok(),
            "Block literal strip should parse: {:?}",
            result
        );
    }

    #[test]
    fn test_block_literal_keep() {
        let yaml = b"text: |+\n  content\n\n\n";
        let result = build_semi_index(yaml);
        assert!(
            result.is_ok(),
            "Block literal keep should parse: {:?}",
            result
        );
    }

    #[test]
    fn test_block_folded_strip() {
        let yaml = b"text: >-\n  content\n";
        let result = build_semi_index(yaml);
        assert!(
            result.is_ok(),
            "Block folded strip should parse: {:?}",
            result
        );
    }

    #[test]
    fn test_block_folded_keep() {
        let yaml = b"text: >+\n  content\n\n";
        let result = build_semi_index(yaml);
        assert!(
            result.is_ok(),
            "Block folded keep should parse: {:?}",
            result
        );
    }

    #[test]
    fn test_block_explicit_indent() {
        let yaml = b"text: |2\n  content\n";
        let result = build_semi_index(yaml);
        assert!(
            result.is_ok(),
            "Block with explicit indent should parse: {:?}",
            result
        );
    }

    #[test]
    fn test_block_explicit_indent_with_chomping() {
        let yaml = b"text: |2-\n  content\n";
        let result = build_semi_index(yaml);
        assert!(
            result.is_ok(),
            "Block with explicit indent and chomping should parse: {:?}",
            result
        );
    }

    #[test]
    fn test_block_empty() {
        let yaml = b"text: |\nnext: value\n";
        let result = build_semi_index(yaml);
        assert!(
            result.is_ok(),
            "Empty block scalar should parse: {:?}",
            result
        );
    }

    #[test]
    fn test_block_in_sequence() {
        let yaml = b"- |\n  item\n- value\n";
        let result = build_semi_index(yaml);
        assert!(
            result.is_ok(),
            "Block scalar in sequence should parse: {:?}",
            result
        );
    }

    #[test]
    fn test_block_with_nested_indent() {
        let yaml = b"code: |\n  def foo():\n    return 42\n";
        let result = build_semi_index(yaml);
        assert!(
            result.is_ok(),
            "Block with nested indent should parse: {:?}",
            result
        );
    }

    #[test]
    fn test_block_multiple() {
        let yaml = b"one: |\n  first\ntwo: |\n  second\n";
        let result = build_semi_index(yaml);
        assert!(
            result.is_ok(),
            "Multiple block scalars should parse: {:?}",
            result
        );
    }

    #[test]
    fn test_block_with_comment() {
        let yaml = b"text: | # this is a comment\n  content\n";
        let result = build_semi_index(yaml);
        assert!(
            result.is_ok(),
            "Block scalar with comment should parse: {:?}",
            result
        );
    }
}
