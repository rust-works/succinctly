//! YAML parser (oracle) for Phase 5: YAML with multi-document streams.
//!
//! This module implements the sequential oracle that resolves YAML's
//! context-sensitive grammar and emits IB/BP/TY bits for index construction.
//!
//! # Phase 5 Scope
//!
//! - Block mappings and sequences
//! - Flow mappings `{key: value}` and sequences `[a, b, c]`
//! - Simple scalars (unquoted, double-quoted, single-quoted)
//! - Block scalars: literal (`|`) and folded (`>`)
//! - Chomping modifiers: strip (`-`), keep (`+`), clip (default)
//! - Anchors (`&name`) and aliases (`*name`)
//! - Comments (ignored in block context, not allowed in flow)
//! - **Multi-document streams (`---` and `...` markers)**
//!
//! # Document Wrapping
//!
//! All YAML documents are wrapped in a virtual root sequence for consistent API:
//! - Single-document files become 1-element arrays
//! - Multi-document files become N-element arrays
//! - Paths use `.[0].key` instead of `.key`

#[cfg(not(test))]
use alloc::{
    collections::BTreeMap,
    string::{String, ToString},
    vec,
    vec::Vec,
};

#[cfg(test)]
use std::collections::BTreeMap;

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
    /// Sequence item (tracks open items awaiting their value)
    SequenceItem,
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
    /// Sequence item marker bits: 1 if this BP position is a sequence item wrapper.
    /// Sequence items have BP open/close but no TY entry.
    pub seq_items: Vec<u64>,
    /// Container marker bits: 1 if this BP position has a TY entry (is a mapping or sequence).
    /// Used to compute correct TY index from BP position.
    pub containers: Vec<u64>,
    /// Number of valid bits in IB (= input length)
    #[allow(dead_code)]
    pub ib_len: usize,
    /// Number of valid bits in BP
    pub bp_len: usize,
    /// Number of valid bits in TY (= number of container opens)
    #[allow(dead_code)]
    pub ty_len: usize,
    /// Anchor definitions: anchor name → BP position of the anchored value
    pub anchors: BTreeMap<String, usize>,
    /// Alias references: BP position of alias → target BP position (resolved at parse time)
    pub aliases: BTreeMap<usize, usize>,
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
    seq_item_words: Vec<u64>,
    /// Container marker bits - marks BP positions that have TY entries (mappings/sequences)
    container_words: Vec<u64>,
    bp_pos: usize,
    ty_pos: usize,

    // Direct BP-to-text mapping
    bp_to_text: Vec<u32>,

    // Indentation tracking
    indent_stack: Vec<usize>,

    // Node type stack (to track if we're in mapping or sequence)
    type_stack: Vec<NodeType>,

    // Anchor and alias tracking
    /// Anchors collected during parsing: name → bp_pos of anchored value
    anchors: BTreeMap<String, usize>,
    /// Aliases collected during parsing: bp_pos → target bp_pos (resolved at parse time)
    aliases: BTreeMap<usize, usize>,

    // Document tracking
    /// Whether we're currently inside a document
    in_document: bool,

    // Explicit key tracking
    /// Whether we have a pending explicit key that needs a value (null if not followed by `:`)
    pending_explicit_key: bool,
}

impl<'a> Parser<'a> {
    fn new(input: &'a [u8]) -> Self {
        let ib_words = vec![0u64; input.len().div_ceil(64).max(1)];
        let bp_words = vec![0u64; input.len().div_ceil(32).max(1)]; // ~2x IB for BP
        let ty_words = vec![0u64; input.len().div_ceil(64).max(1)];
        let seq_item_words = vec![0u64; input.len().div_ceil(32).max(1)]; // Same size as BP
        let container_words = vec![0u64; input.len().div_ceil(32).max(1)]; // Same size as BP

        Self {
            input,
            pos: 0,
            line: 1,
            ib_words,
            bp_words,
            ty_words,
            seq_item_words,
            container_words,
            bp_pos: 0,
            ty_pos: 0,
            bp_to_text: Vec::new(),
            indent_stack: vec![0], // Start at indent 0
            type_stack: Vec::new(),
            anchors: BTreeMap::new(),
            aliases: BTreeMap::new(),
            in_document: false,
            pending_explicit_key: false,
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

    /// Mark the current BP position as a sequence item.
    /// Call this BEFORE write_bp_open for sequence items.
    #[inline]
    fn mark_seq_item(&mut self) {
        let bp_pos = self.bp_pos;
        let word_idx = bp_pos / 64;
        let bit_idx = bp_pos % 64;
        while word_idx >= self.seq_item_words.len() {
            self.seq_item_words.push(0);
        }
        self.seq_item_words[word_idx] |= 1u64 << bit_idx;
    }

    /// Mark current BP position as a container (has TY entry).
    /// Call this BEFORE write_bp_open for containers.
    #[inline]
    #[allow(dead_code)]
    fn mark_container(&mut self) {
        let bp_pos = self.bp_pos;
        let word_idx = bp_pos / 64;
        let bit_idx = bp_pos % 64;
        while word_idx >= self.container_words.len() {
            self.container_words.push(0);
        }
        self.container_words[word_idx] |= 1u64 << bit_idx;
    }

    /// Close a pending explicit key by adding a null value node.
    /// Call this when a new key or end of mapping is encountered without an explicit value.
    fn close_pending_explicit_key(&mut self) {
        if self.pending_explicit_key {
            // Add a null value node (empty open/close pair)
            // Use input.len() as the text position to indicate "no text" / null value
            self.write_bp_open_at(self.input.len());
            self.write_bp_close();
            self.pending_explicit_key = false;
        }
    }

    /// Write a type bit: 0 = mapping, 1 = sequence.
    /// Also marks the current BP position as a container.
    #[inline]
    fn write_ty(&mut self, is_sequence: bool) {
        // Mark this BP position as a container (bp_pos - 1 because write_bp_open already incremented)
        let container_bp_pos = self.bp_pos - 1;
        let word_idx = container_bp_pos / 64;
        let bit_idx = container_bp_pos % 64;
        while word_idx >= self.container_words.len() {
            self.container_words.push(0);
        }
        self.container_words[word_idx] |= 1u64 << bit_idx;

        // Write the TY bit
        let ty_word_idx = self.ty_pos / 64;
        let ty_bit_idx = self.ty_pos % 64;
        while ty_word_idx >= self.ty_words.len() {
            self.ty_words.push(0);
        }
        if is_sequence {
            self.ty_words[ty_word_idx] |= 1u64 << ty_bit_idx;
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

    /// Skip whitespace on the current line (spaces and tabs, not newlines).
    fn skip_inline_whitespace(&mut self) {
        while let Some(b) = self.peek() {
            if b == b' ' || b == b'\t' {
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
                    // Tab after spaces - check context
                    // If we haven't seen any spaces and hit a tab at start of line,
                    // that's tab indentation (error). But tab after spaces is content.
                    if count == 0 {
                        return Err(YamlError::TabIndentation {
                            line: self.line,
                            offset: i,
                        });
                    }
                    // Tab after spaces is start of content, stop counting indent
                    break;
                }
                _ => break,
            }
        }
        Ok(count)
    }

    /// Get the current column position (0-based).
    /// This counts characters from the start of the current line.
    fn current_column(&self) -> usize {
        // Find the start of the current line
        let mut line_start = self.pos;
        while line_start > 0 && self.input[line_start - 1] != b'\n' {
            line_start -= 1;
        }
        self.pos - line_start
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

    /// Check if current position starts a key-value pair (compact mapping).
    /// Returns true if there's a `:` followed by space/tab/newline/EOF on this line.
    /// Also returns true for empty key case (`:` at start).
    fn looks_like_mapping_entry(&self) -> bool {
        // If we're at a flow structure, it's not a compact mapping
        match self.peek() {
            Some(b'{') | Some(b'[') => return false,
            // Empty key: `:` at start followed by whitespace/newline/EOF
            Some(b':') => {
                let next = self.peek_at(1);
                if matches!(next, Some(b' ') | Some(b'\t') | Some(b'\n') | None) {
                    return true;
                }
                // Colon not followed by whitespace - continue checking
            }
            _ => {}
        }

        let mut i = self.pos;

        // If starting with a quote, skip the quoted string first
        if i < self.input.len() && (self.input[i] == b'"' || self.input[i] == b'\'') {
            let quote = self.input[i];
            i += 1;
            while i < self.input.len() {
                if self.input[i] == quote {
                    // Check for escaped quote in single-quoted strings
                    if quote == b'\'' && i + 1 < self.input.len() && self.input[i + 1] == b'\'' {
                        i += 2; // Skip ''
                        continue;
                    }
                    i += 1; // Skip closing quote
                    break;
                } else if self.input[i] == b'\\' && quote == b'"' {
                    i += 2; // Skip escape sequence in double-quoted
                } else if self.input[i] == b'\n' {
                    return false; // Unclosed quote
                } else {
                    i += 1;
                }
            }
            // After quoted key, check for `: `
            // Skip optional whitespace
            while i < self.input.len() && self.input[i] == b' ' {
                i += 1;
            }
            if i < self.input.len() && self.input[i] == b':' {
                let next = if i + 1 < self.input.len() {
                    Some(self.input[i + 1])
                } else {
                    None
                };
                return matches!(next, Some(b' ') | Some(b'\t') | Some(b'\n') | None);
            }
            return false;
        }

        // Scan for `: ` pattern in unquoted key
        while i < self.input.len() {
            match self.input[i] {
                b'\n' => return false, // Line ended without finding `: `
                b':' => {
                    // Check what follows the colon
                    let next = if i + 1 < self.input.len() {
                        Some(self.input[i + 1])
                    } else {
                        None
                    };
                    match next {
                        Some(b' ') | Some(b'\t') | Some(b'\n') | None => return true,
                        _ => i += 1, // Colon not followed by whitespace, continue
                    }
                }
                // Note: " and ' in the middle of a key are allowed (e.g., bla"keks: foo)
                // Continue scanning past them.
                _ => i += 1,
            }
        }
        false
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

    /// Check for unsupported YAML features (Phase 4: anchors and aliases now supported).
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
                // Anchors and aliases are supported in Phase 4+
                b'&' | b'*' => {
                    // Allowed - will be parsed by parse_anchor() or parse_alias()
                }
                // Explicit keys (`?`) are now supported
                b'?' => {}
                b'!' => {
                    return Err(YamlError::TagNotSupported { offset: self.pos });
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Check if we're at a document start marker (`---`).
    fn is_document_start(&self) -> bool {
        if self.pos + 2 >= self.input.len() {
            return false;
        }
        let slice = &self.input[self.pos..self.pos + 3];
        if slice != b"---" {
            return false;
        }
        // Must be followed by space, newline, or EOF
        self.peek_at(3) == Some(b' ') || self.peek_at(3) == Some(b'\n') || self.peek_at(3).is_none()
    }

    /// Check if we're at a document end marker (`...`).
    fn is_document_end(&self) -> bool {
        if self.pos + 2 >= self.input.len() {
            return false;
        }
        let slice = &self.input[self.pos..self.pos + 3];
        if slice != b"..." {
            return false;
        }
        // Must be followed by space, newline, or EOF
        self.peek_at(3) == Some(b' ') || self.peek_at(3) == Some(b'\n') || self.peek_at(3).is_none()
    }

    /// Skip past a document marker (`---` or `...`).
    /// Does NOT skip content after the marker - that should be parsed.
    fn skip_document_marker(&mut self) {
        // Skip the 3-character marker
        self.advance();
        self.advance();
        self.advance();
        // Skip trailing space after marker if present
        if self.peek() == Some(b' ') {
            self.advance();
        }
    }

    /// Check if there's parseable content on the current line (not just whitespace/comment).
    fn has_content_on_line(&self) -> bool {
        let mut i = 0;
        loop {
            match self.peek_at(i) {
                Some(b' ') | Some(b'\t') => i += 1,
                Some(b'\n') | Some(b'\r') | Some(b'#') | None => return false,
                _ => return true,
            }
        }
    }

    /// Parse content after a document marker on the same line (e.g., `--- >` or `--- value`).
    fn parse_inline_document_value(&mut self) -> Result<(), YamlError> {
        // Skip leading whitespace
        while self.peek() == Some(b' ') || self.peek() == Some(b'\t') {
            self.advance();
        }

        match self.peek() {
            Some(b'|') | Some(b'>') => {
                // Block scalar
                self.parse_block_scalar(0)?;
            }
            Some(b'"') => {
                // Quoted string
                self.set_ib();
                self.write_bp_open();
                self.parse_double_quoted()?;
                self.write_bp_close();
            }
            Some(b'\'') => {
                // Single-quoted string
                self.set_ib();
                self.write_bp_open();
                self.parse_single_quoted()?;
                self.write_bp_close();
            }
            Some(b'[') | Some(b'{') => {
                // Flow collection
                self.parse_value(0)?;
            }
            Some(b'-')
                if self.peek_at(1) == Some(b' ')
                    || self.peek_at(1) == Some(b'\t')
                    || self.peek_at(1) == Some(b'\n')
                    || self.peek_at(1).is_none() =>
            {
                // Block sequence item
                self.parse_sequence_item(0)?;
            }
            Some(_) if self.looks_like_mapping_entry() => {
                // Mapping entry
                self.parse_mapping_entry(0)?;
            }
            Some(_) => {
                // Plain scalar
                self.set_ib();
                self.write_bp_open();
                self.parse_unquoted_value_with_indent(0);
                self.write_bp_close();
            }
            None => {}
        }

        Ok(())
    }

    /// Start a new document within the virtual root sequence.
    /// This doesn't open a container - the document IS its content.
    fn start_document(&mut self) {
        self.in_document = true;
    }

    /// End the current document, closing any open containers.
    fn end_document(&mut self) {
        if !self.in_document {
            return;
        }

        // Close any remaining open containers within the document
        // The virtual root is at indent_stack[0], so close everything above it
        while self.indent_stack.len() > 1 {
            // If we're closing a mapping that has a pending explicit key, close it first
            if self.type_stack.last() == Some(&NodeType::Mapping) {
                self.close_pending_explicit_key();
            }
            self.indent_stack.pop();
            self.type_stack.pop();
            self.write_bp_close();
        }

        self.in_document = false;
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

    /// Parse an unquoted scalar value with a minimum indentation requirement.
    /// Handles multiline plain scalars - continues on lines more indented than start_indent.
    fn parse_unquoted_value_with_indent(&mut self, start_indent: usize) -> usize {
        let start = self.pos;

        loop {
            // Parse content on current line
            while let Some(b) = self.peek() {
                match b {
                    b'\n' => break,
                    b'#' => {
                        // # is only a comment if preceded by whitespace
                        if self.pos > start && self.input[self.pos - 1] == b' ' {
                            break;
                        }
                        self.advance();
                    }
                    b':' => {
                        // Colon followed by whitespace ends the value (could be a key)
                        // But in value context, colons in URLs etc. are allowed
                        if self.peek_at(1) == Some(b' ')
                            || self.peek_at(1) == Some(b'\t')
                            || self.peek_at(1) == Some(b'\n')
                        {
                            break;
                        }
                        self.advance();
                    }
                    _ => self.advance(),
                }
            }

            // Check if we can continue to next line
            if self.peek() != Some(b'\n') {
                break;
            }

            // Look ahead to see if next line is a continuation
            let mut lookahead = self.pos + 1; // Skip \n
            let mut next_indent = 0;

            // Count indentation on next line (only spaces count as indent in YAML)
            while lookahead < self.input.len() && self.input[lookahead] == b' ' {
                next_indent += 1;
                lookahead += 1;
            }

            // Check what comes after the indent
            if lookahead >= self.input.len() {
                // EOF - stop here
                break;
            }

            let next_char = self.input[lookahead];

            // If empty line (just whitespace then newline), skip it and continue
            // This includes lines with only spaces, tabs, or a mix
            if next_char == b'\n' || next_char == b'\t' {
                // Check if rest of line is whitespace
                let mut check_pos = lookahead;
                while check_pos < self.input.len() && matches!(self.input[check_pos], b' ' | b'\t')
                {
                    check_pos += 1;
                }
                if check_pos >= self.input.len()
                    || self.input[check_pos] == b'\n'
                    || self.input[check_pos] == b'\r'
                {
                    // Empty line - skip it and continue
                    self.advance(); // Skip current \n
                                    // Skip to end of empty line
                    while matches!(self.peek(), Some(b' ') | Some(b'\t')) {
                        self.advance();
                    }
                    continue;
                }
                // Tab followed by content - for document root scalars (start_indent == 0),
                // this is a continuation per YAML spec example 7.12 "Plain Lines".
                // The tabs become part of the folded content (converted to space).
                if start_indent == 0 && next_char == b'\t' {
                    // Continue to next line - this is a valid continuation
                    self.advance(); // Skip \n
                                    // Skip leading whitespace (tabs are content, but we're at the scalar's level)
                    while matches!(self.peek(), Some(b' ') | Some(b'\t')) {
                        self.advance();
                    }
                    continue;
                }
            }

            // Continuation requires more indent than where scalar started
            // and next line shouldn't start block structure or be a comment.
            //
            // For sequence indicators `- `, they're only block structure if at a
            // "proper" indent level. A `- ` at indent just 1 greater than start_indent
            // (like ` - ` when start_indent is 0) is scalar content, not a sequence.
            // This handles cases like AB8U where the `- ` is at an invalid indent.
            let is_sequence_indicator = next_char == b'-'
                && lookahead + 1 < self.input.len()
                && matches!(self.input[lookahead + 1], b' ' | b'\t');
            let sequence_indicator_is_block_structure = is_sequence_indicator
                && (next_indent <= start_indent || next_indent >= start_indent + 2);

            if next_indent > start_indent
                && next_char != b'#'
                && !sequence_indicator_is_block_structure
                && !(next_char == b':'
                    && lookahead + 1 < self.input.len()
                    && matches!(self.input[lookahead + 1], b' ' | b'\n'))
            {
                // Continue to next line
                self.advance(); // Skip \n
                                // Skip leading whitespace
                while matches!(self.peek(), Some(b' ') | Some(b'\t')) {
                    self.advance();
                }
            } else {
                // Not a continuation - stop here
                break;
            }
        }

        // Trim trailing whitespace
        let mut end = self.pos;
        while end > start && matches!(self.input[end - 1], b' ' | b'\t') {
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
                    // Check for colon + whitespace or colon + newline
                    if self.peek_at(1) == Some(b' ')
                        || self.peek_at(1) == Some(b'\t')
                        || self.peek_at(1) == Some(b'\n')
                        || self.peek_at(1).is_none()
                    {
                        break;
                    }
                    // Colon not followed by whitespace is part of the key
                    // (e.g., "key::" or URLs like "http://example.com")
                    self.advance();
                }
                b'\n' => {
                    // Key without colon
                    return Err(YamlError::KeyWithoutValue {
                        offset: start,
                        line: self.line,
                    });
                }
                b'#' => {
                    // # is only a comment if preceded by whitespace
                    // Otherwise it's part of the key (e.g., "a#b: value")
                    if self.pos > start && self.input[self.pos - 1] == b' ' {
                        return Err(YamlError::KeyWithoutValue {
                            offset: start,
                            line: self.line,
                        });
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

        // Empty key is valid in YAML (e.g., `: value`)
        Ok(end - start)
    }

    /// Close containers that are at higher indent levels.
    fn close_deeper_indents(&mut self, new_indent: usize) {
        while self.indent_stack.len() > 1 {
            let current_indent = *self.indent_stack.last().unwrap();
            // Only close containers that are DEEPER than the new indent.
            // Containers at the same level should stay open so new entries
            // can be added to them.
            if current_indent > new_indent {
                // If we're closing a mapping that has a pending explicit key, close it first
                if self.type_stack.last() == Some(&NodeType::Mapping) {
                    self.close_pending_explicit_key();
                }
                self.indent_stack.pop();
                self.type_stack.pop();
                self.write_bp_close();
            } else {
                break;
            }
        }
    }

    /// Close a sequence that was the value of a previous mapping entry.
    ///
    /// When we're about to add a new entry to a mapping at indent N, and the top
    /// of the stack is a Sequence at indent N with a Mapping below it also at
    /// indent N, the Sequence was the value of a previous entry and must be closed.
    ///
    /// YAML allows sequences-as-values to start at the same indent as the key:
    /// ```yaml
    /// foo:      # key at indent 0
    /// - item    # sequence value at indent 0  <- allowed
    /// bar:      # new key at indent 0 - closes the sequence
    /// ```
    fn close_same_indent_sequence_before_mapping_entry(&mut self, indent: usize) {
        // Check if we have a Sequence at same indent as a Mapping below it
        if self.indent_stack.len() >= 2 && self.type_stack.len() >= 2 {
            let top_idx = self.indent_stack.len() - 1;
            let top_indent = self.indent_stack[top_idx];
            let top_type = self.type_stack[top_idx];
            let below_indent = self.indent_stack[top_idx - 1];
            let below_type = self.type_stack[top_idx - 1];

            // If Sequence at indent N is on top of Mapping at indent N,
            // and we're adding a mapping entry at indent N, close the Sequence
            if top_type == NodeType::Sequence
                && below_type == NodeType::Mapping
                && top_indent == indent
                && below_indent == indent
            {
                self.indent_stack.pop();
                self.type_stack.pop();
                self.write_bp_close();
            }
        }
    }

    /// Parse a sequence item (starts with `- `).
    fn parse_sequence_item(&mut self, indent: usize) -> Result<(), YamlError> {
        let _item_start = self.pos;

        // Mark the `-` position
        self.set_ib();

        // First close any deeper containers. This might reveal an existing sequence
        // at this indent level that we can reuse.
        self.close_deeper_indents(indent);

        // Now check if we need to open a new sequence (check AFTER closing)
        // Normally, sequence items must be at the exact same indent as the sequence.
        // However, for nested sequences created by `- - item` pattern, items can be
        // at greater indent because the nested sequence's indent is virtual.
        //
        // We need a new sequence if:
        // 1. There's no sequence on the stack, OR
        // 2. The item indent doesn't match the sequence indent
        let need_new_sequence = self.type_stack.last() != Some(&NodeType::Sequence)
            || self.indent_stack.last().copied() != Some(indent);

        if need_new_sequence {
            // Open new sequence
            self.write_bp_open();
            self.write_ty(true); // 1 = sequence
            self.indent_stack.push(indent);
            self.type_stack.push(NodeType::Sequence);
        }

        // Open the sequence item node
        self.mark_seq_item();
        self.write_bp_open();

        // Skip `- `
        self.advance(); // -
        self.skip_inline_whitespace();

        self.check_unsupported()?;

        // Track the sequence item on the stack so close_deeper_indents can close it.
        // We use indent + 1 as a virtual indent - any content at indent > indent
        // is considered part of this item.
        //
        // NOTE: This means for `- foo`, the item is at virtual indent 1, so content
        // at indent 2 would be part of the item. But the sequence itself is at indent 0.
        self.indent_stack.push(indent + 1);
        self.type_stack.push(NodeType::SequenceItem);

        // Check what follows
        if self.at_line_end() {
            // Content is on the next line(s) at greater indentation.
            // Leave the item open - subsequent content at indent > this item's
            // indent will be parsed as the item's value. The item will be closed
            // by close_deeper_indents when we see content at indent <= sequence indent.
            return Ok(());
        }

        // Check for nested sequence: `- - item` (sequence item containing a sequence)
        if self.peek() == Some(b'-')
            && matches!(
                self.peek_at(1),
                Some(b' ') | Some(b'\t') | Some(b'\n') | Some(b'\r') | None
            )
        {
            // Nested sequence - the item value is another sequence.
            // Use the actual column position of the nested `-` as the indent.
            // This ensures that subsequent items at the same column (like `- d` after `- c`)
            // will be correctly recognized as siblings in the same sequence.
            let nested_indent = self.current_column();
            self.parse_sequence_item(nested_indent)?;
            // Don't close the outer item - it will be closed when we return
            // to a lower indent level.
        } else if self.looks_like_mapping_entry() {
            // Check for compact mapping: `- key: value`
            // This is a mapping entry directly as the sequence item value
            // The sequence item contains a mapping.
            // Use indent + 2 so that entries at actual indent >= indent+2 are
            // considered part of this mapping.
            let compact_indent = indent + 2;
            self.parse_compact_mapping_entry(compact_indent)?;
            // Don't close anything - mapping and item will be closed by
            // close_deeper_indents when we see content at lower indent.
        } else {
            // Parse the item value normally
            // Pass structure indent for block scalars (content must be > this)
            self.parse_value(indent)?;
            // Close the sequence item for simple values
            self.indent_stack.pop();
            self.type_stack.pop();
            self.write_bp_close();
        }

        Ok(())
    }

    /// Parse a compact mapping entry within a sequence item.
    /// This handles `- key: value` where the mapping is inline with the sequence item.
    fn parse_compact_mapping_entry(&mut self, indent: usize) -> Result<(), YamlError> {
        // Open a mapping for this compact entry
        self.write_bp_open();
        self.write_ty(false); // 0 = mapping
        self.indent_stack.push(indent);
        self.type_stack.push(NodeType::Mapping);

        // Mark key position
        self.set_ib();

        // Open key node
        self.write_bp_open();

        // Parse the key
        match self.peek() {
            Some(b'"') => {
                self.parse_double_quoted()?;
            }
            Some(b'\'') => {
                self.parse_single_quoted()?;
            }
            _ => {
                self.parse_unquoted_key()?;
            }
        }

        // Close key node
        self.write_bp_close();

        // Expect colon
        if self.peek() != Some(b':') {
            return Err(YamlError::UnexpectedCharacter {
                offset: self.pos,
                char: self.peek().map(|b| b as char).unwrap_or('\0'),
                context: "expected ':' after key in compact mapping",
            });
        }
        self.advance(); // Skip ':'

        // Skip space after colon
        self.skip_inline_whitespace();

        // Parse value
        if self.at_line_end() {
            // Value is on next line or implicit null
            self.skip_to_eol();
        } else {
            // Inline value
            self.check_unsupported()?;

            // Open value node
            self.set_ib();
            self.write_bp_open();
            self.parse_inline_value(indent)?;
            self.write_bp_close();
        }

        // Don't close the mapping here - leave it open so subsequent lines
        // at compatible indent levels can add more entries. The mapping will
        // be closed by close_deeper_indents when we return to a lower indent.

        Ok(())
    }

    /// Parse a mapping key-value pair.
    fn parse_mapping_entry(&mut self, indent: usize) -> Result<(), YamlError> {
        let _entry_start = self.pos;

        // First close any containers that are deeper than our indent level.
        // This ensures we return to the appropriate context before deciding
        // whether to open a new mapping or add to an existing one.
        self.close_deeper_indents(indent);

        // If there's a pending explicit key without value, close it with null
        self.close_pending_explicit_key();

        // Close any sequence that was the value of a previous mapping entry.
        // This handles:
        //   foo:
        //   - item  <- sequence at same indent as mapping
        //   bar:    <- new entry closes the sequence
        self.close_same_indent_sequence_before_mapping_entry(indent);

        // Now check if we need to open a new mapping
        let need_new_mapping = self.type_stack.last() != Some(&NodeType::Mapping)
            || self.indent_stack.last().copied() != Some(indent);

        if need_new_mapping {
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

        // Check for anchor on key - record it pointing to this key BP
        // The key BP was just opened, so bp_pos is now one past the key's position
        if self.peek() == Some(b'&') {
            // Consume `&`
            self.advance();
            // Parse anchor name
            let name = self.parse_anchor_name()?;
            // Skip whitespace after anchor name
            self.skip_inline_whitespace();
            // Record anchor pointing to the key (bp_pos - 1, since we just opened the key BP)
            self.anchors.insert(name, self.bp_pos - 1);
        }

        // Parse the key - check for empty key first (colon at start)
        if self.peek() == Some(b':') {
            // Empty key - check that it's followed by proper terminator
            let next = self.peek_at(1);
            if matches!(next, Some(b' ') | Some(b'\t') | Some(b'\n') | None) {
                // Empty key case - key length is 0, don't advance yet
            } else {
                // Colon followed by something else - not an empty key
                self.parse_unquoted_key()?;
            }
        } else {
            // Parse the key
            match self.peek() {
                Some(b'"') => {
                    self.parse_double_quoted()?;
                }
                Some(b'\'') => {
                    self.parse_single_quoted()?;
                }
                Some(b'*') => {
                    // Alias as key - parse alias name
                    // Skip `*`
                    self.advance();
                    // Parse alias name (same rules as anchor names)
                    let alias_name = self.parse_anchor_name()?;
                    // Record the alias reference
                    // Look up the anchor's BP position
                    if let Some(&target_bp_pos) = self.anchors.get(&alias_name) {
                        self.aliases.insert(self.bp_pos - 1, target_bp_pos);
                    }
                }
                _ => {
                    self.parse_unquoted_key()?;
                }
            }
        }

        // Close key node
        self.write_bp_close();

        // Skip optional whitespace between key and colon (e.g., 'key' : value)
        self.skip_inline_whitespace();

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
            // Check if at EOF - if so, we need an explicit empty value node
            if self.peek().is_none() {
                // EOF after colon - emit empty value
                self.set_ib();
                self.write_bp_open();
                self.write_bp_close();
                return Ok(());
            }
            // Value is on next line - check what kind of value
            self.skip_to_eol();

            // Look ahead to see what the next content line looks like
            self.skip_newlines();
            if self.peek().is_none() {
                // EOF - null value
                return Ok(());
            }

            // Count indentation of next line
            let next_indent = self.count_indent().unwrap_or(0);
            if next_indent <= indent {
                // Next line is at same or lower indent - null value
                return Ok(());
            }

            // Skip to the content position
            let saved_pos = self.pos;
            for _ in 0..next_indent {
                self.advance();
            }

            // Check if this is a nested structure or a plain scalar value
            match self.peek() {
                Some(b'-')
                    if matches!(
                        self.peek_at(1),
                        Some(b' ') | Some(b'\t') | Some(b'\n') | None
                    ) =>
                {
                    // Sequence - will be handled by main loop
                    self.pos = saved_pos;
                    return Ok(());
                }
                Some(b'?') if matches!(self.peek_at(1), Some(b' ') | Some(b'\n') | None) => {
                    // Explicit key - will be handled by main loop
                    self.pos = saved_pos;
                    return Ok(());
                }
                Some(b'{') | Some(b'[') | Some(b'|') | Some(b'>') => {
                    // Flow/block structure - will be handled by main loop
                    self.pos = saved_pos;
                    return Ok(());
                }
                Some(b'#') => {
                    // Comment - will be handled by main loop
                    self.pos = saved_pos;
                    return Ok(());
                }
                Some(b'&') | Some(b'*') => {
                    // Anchor or alias on its own line - will be handled by main loop
                    self.pos = saved_pos;
                    return Ok(());
                }
                _ => {
                    // Check if this looks like a mapping entry
                    if self.looks_like_mapping_entry() {
                        // Nested mapping - will be handled by main loop
                        self.pos = saved_pos;
                        return Ok(());
                    }
                    // Plain scalar value - parse it here with key's indent as base
                    self.set_ib();
                    self.write_bp_open();
                    self.parse_unquoted_value_with_indent(indent);
                    self.write_bp_close();
                    return Ok(());
                }
            }
        }

        {
            self.check_unsupported()?;

            // Check for anchor first - it prefixes the actual value
            let anchor_name = if self.peek() == Some(b'&') {
                Some(self.parse_anchor()?)
            } else {
                None
            };

            // After anchor, check if value continues on next line
            if self.at_line_end() {
                // Need to check if the next line has content for this value,
                // or if the value is null (same or lower indent on next line)
                self.skip_to_eol();

                // Save position to look ahead
                let saved_pos = self.pos;
                let saved_line = self.line;

                // Look at next content line
                self.skip_newlines();
                if self.peek().is_none() {
                    // EOF - value is null, create explicit null node for anchor
                    self.pos = saved_pos;
                    self.line = saved_line;
                    self.set_ib();
                    self.write_bp_open();
                    self.write_bp_close();
                    return Ok(());
                }

                let next_indent = self.count_indent().unwrap_or(0);

                // Check if next line is a sequence at same indent as key
                // Sequences can be at same indent as their parent mapping key
                let pos_before_check = self.pos;
                let is_sequence_at_same_indent = {
                    // Skip past indent spaces to check what follows
                    while self.peek() == Some(b' ') {
                        self.advance();
                    }
                    matches!(self.peek(), Some(b'-'))
                        && matches!(self.peek_at(1), Some(b' ') | Some(b'\n') | None)
                };
                // Restore position after checking
                self.pos = pos_before_check;

                if next_indent <= indent && !is_sequence_at_same_indent {
                    // Next line is at same or lower indent and not a sequence - value is null
                    // Create explicit null node for anchor to point to
                    self.pos = saved_pos;
                    self.line = saved_line;
                    self.set_ib();
                    self.write_bp_open();
                    self.write_bp_close();
                    return Ok(());
                }

                // Value is on next line (nested structure or same-indent sequence)
                // Position is at start of content line for main loop to parse
                return Ok(());
            }

            // Check for alias - this IS the value
            if self.peek() == Some(b'*') {
                self.parse_alias()?;
                return Ok(());
            }

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
                    self.parse_inline_value(indent)?;
                    self.write_bp_close();
                }
            }

            // Suppress unused warning for anchor_name
            let _ = anchor_name;
        }

        Ok(())
    }

    /// Parse an explicit key (`? key`).
    /// The key can be any value: scalar, sequence, or mapping.
    fn parse_explicit_key(&mut self, indent: usize) -> Result<(), YamlError> {
        // Close any deeper containers
        self.close_deeper_indents(indent);

        // If there's a pending explicit key without value, close it with null
        self.close_pending_explicit_key();

        // Check if we need to open a new mapping
        let need_new_mapping = self.type_stack.last() != Some(&NodeType::Mapping)
            || self.indent_stack.last().copied() != Some(indent);

        if need_new_mapping {
            // Open new mapping
            self.write_bp_open();
            self.write_ty(false); // 0 = mapping
            self.indent_stack.push(indent);
            self.type_stack.push(NodeType::Mapping);
        }

        // Skip `?`
        self.advance();

        // Skip whitespace/comments after `?`
        self.skip_inline_whitespace();

        // Check for anchor before key
        if self.peek() == Some(b'&') {
            let _ = self.parse_anchor()?;
            self.skip_inline_whitespace();
        }

        // Check what the key is
        if self.at_line_end() {
            // Key is on next line(s) - it could be a complex structure
            // For now, the next line will parse as the key content
            // and the `:` line will provide the value
            self.skip_to_eol();
            return Ok(());
        }

        // Mark key position
        self.set_ib();

        // Parse the key value inline
        match self.peek() {
            Some(b'-')
                if matches!(
                    self.peek_at(1),
                    Some(b' ') | Some(b'\n') | Some(b'\r') | None
                ) =>
            {
                // Sequence as key - open key node and let sequence parsing continue
                // The key will be a sequence
                self.write_bp_open();
                self.write_ty(true); // sequence
                self.indent_stack.push(indent + 2); // Indent for sequence content
                self.type_stack.push(NodeType::Sequence);

                // Parse first sequence item inline
                self.write_bp_open(); // item node
                self.advance(); // skip `-`
                self.skip_inline_whitespace();

                if !self.at_line_end() {
                    // Parse item value
                    if self.looks_like_mapping_entry() {
                        self.parse_compact_mapping_entry(indent + 3)?;
                    } else {
                        self.parse_value(indent + 2)?;
                    }
                }
                self.write_bp_close(); // close item
            }
            Some(b'[') => {
                // Flow sequence as key
                self.write_bp_open();
                self.parse_flow_sequence()?;
                self.write_bp_close();
            }
            Some(b'{') => {
                // Flow mapping as key
                self.write_bp_open();
                self.parse_flow_mapping()?;
                self.write_bp_close();
            }
            Some(b'|') | Some(b'>') => {
                // Block scalar as key
                self.parse_block_scalar(indent)?;
            }
            Some(b'"') => {
                // Double-quoted key
                self.write_bp_open();
                self.parse_double_quoted()?;
                self.write_bp_close();
            }
            Some(b'\'') => {
                // Single-quoted key
                self.write_bp_open();
                self.parse_single_quoted()?;
                self.write_bp_close();
            }
            _ => {
                // Unquoted scalar key
                self.write_bp_open();
                self.parse_unquoted_value_with_indent(indent);
                self.write_bp_close();
            }
        }

        // Mark that we have an explicit key waiting for a value
        self.pending_explicit_key = true;

        Ok(())
    }

    /// Parse an explicit value (`: value` after explicit key).
    fn parse_explicit_value(&mut self, indent: usize) -> Result<(), YamlError> {
        // Close deeper structures, but keep the mapping at this indent open
        self.close_deeper_indents(indent + 1);

        // This value is for the pending explicit key
        self.pending_explicit_key = false;

        // Skip `:`
        self.advance();

        // Skip whitespace after `:`
        self.skip_inline_whitespace();

        // Check for anchor
        if self.peek() == Some(b'&') {
            let _ = self.parse_anchor()?;
            self.skip_inline_whitespace();
        }

        // Check if value is on this line or next
        if self.at_line_end() {
            // Value is on next line(s) or null
            self.skip_to_eol();
            return Ok(());
        }

        // Parse the value
        match self.peek() {
            Some(b'-')
                if matches!(
                    self.peek_at(1),
                    Some(b' ') | Some(b'\t') | Some(b'\n') | Some(b'\r') | None
                ) =>
            {
                // Sequence as value
                self.write_bp_open();
                self.write_ty(true); // sequence
                self.indent_stack.push(indent + 2);
                self.type_stack.push(NodeType::Sequence);

                // Parse first sequence item
                self.write_bp_open(); // item node
                self.advance(); // skip `-`
                self.skip_inline_whitespace();

                if !self.at_line_end() {
                    if self.looks_like_mapping_entry() {
                        self.parse_compact_mapping_entry(indent + 3)?;
                    } else {
                        self.parse_value(indent + 2)?;
                    }
                }
                self.write_bp_close(); // close item
            }
            Some(b'[') => {
                self.parse_flow_sequence()?;
            }
            Some(b'{') => {
                self.parse_flow_mapping()?;
            }
            Some(b'|') | Some(b'>') => {
                self.parse_block_scalar(indent)?;
            }
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
            Some(b'*') => {
                // Alias as value
                self.parse_alias()?;
            }
            _ => {
                self.set_ib();
                self.write_bp_open();
                self.parse_unquoted_value_with_indent(indent);
                self.write_bp_close();
            }
        }

        Ok(())
    }

    /// Parse an inline scalar value (on the same line as the key).
    fn parse_inline_value(&mut self, min_indent: usize) -> Result<(), YamlError> {
        match self.peek() {
            Some(b'"') => {
                self.parse_double_quoted()?;
            }
            Some(b'\'') => {
                self.parse_single_quoted()?;
            }
            _ => {
                self.parse_unquoted_value_with_indent(min_indent);
            }
        }
        Ok(())
    }

    /// Parse a value (could be scalar or nested structure).
    fn parse_value(&mut self, min_indent: usize) -> Result<(), YamlError> {
        self.check_unsupported()?;

        // Check for anchor first - it prefixes the actual value
        if self.peek() == Some(b'&') {
            self.parse_anchor()?;
            // Now parse the actual value that follows
        }

        // Check for alias - this IS the value (no value follows)
        if self.peek() == Some(b'*') {
            return self.parse_alias();
        }

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
            Some(b'-') if self.peek_at(1) == Some(b' ') || self.peek_at(1) == Some(b'\t') => {
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
                self.parse_unquoted_value_with_indent(min_indent);
                self.write_bp_close();
            }
        }
        Ok(())
    }

    // =========================================================================
    // Flow style parsing (Phase 2)
    // =========================================================================

    /// Skip whitespace in flow context (spaces, tabs, newlines, and comments).
    /// Unlike block context, newlines are allowed within flow constructs.
    /// Comments (`# ...`) are also skipped in flow context.
    fn skip_flow_whitespace(&mut self) {
        while let Some(b) = self.peek() {
            match b {
                b' ' | b'\t' | b'\n' | b'\r' => self.advance(),
                b'#' => {
                    // Skip comment to end of line
                    self.skip_to_eol();
                }
                _ => break,
            }
        }
    }

    /// Check if current position starts an implicit mapping entry in flow context.
    /// Returns true if there's a `key : value` pattern (colon followed by space).
    /// This is used to detect patterns like `[ YAML : separate ]`.
    fn looks_like_flow_mapping_entry(&self) -> bool {
        let mut i = self.pos;

        // Skip quoted string if present
        if i < self.input.len() && (self.input[i] == b'"' || self.input[i] == b'\'') {
            let quote = self.input[i];
            i += 1;
            while i < self.input.len() {
                if self.input[i] == quote {
                    if quote == b'\'' && i + 1 < self.input.len() && self.input[i + 1] == b'\'' {
                        // Escaped single quote
                        i += 2;
                        continue;
                    }
                    i += 1; // Skip closing quote
                    break;
                } else if self.input[i] == b'\\' && quote == b'"' {
                    i += 2; // Skip escape sequence
                } else {
                    i += 1;
                }
            }
            // Skip whitespace after quoted string
            while i < self.input.len() && matches!(self.input[i], b' ' | b'\t') {
                i += 1;
            }
            // Check for colon - after quoted key, colon can be adjacent (no space required)
            if i < self.input.len() && self.input[i] == b':' {
                return true;
            }
            return false;
        }

        // Skip flow mapping or sequence if present (e.g., {JSON: like}:value or [a,b]:value)
        if i < self.input.len() && (self.input[i] == b'{' || self.input[i] == b'[') {
            let open = self.input[i];
            let close = if open == b'{' { b'}' } else { b']' };
            let mut depth = 1;
            i += 1;
            while i < self.input.len() && depth > 0 {
                match self.input[i] {
                    b'"' | b'\'' => {
                        // Skip quoted string inside the flow
                        let quote = self.input[i];
                        i += 1;
                        while i < self.input.len() {
                            if self.input[i] == quote {
                                if quote == b'\''
                                    && i + 1 < self.input.len()
                                    && self.input[i + 1] == b'\''
                                {
                                    i += 2;
                                    continue;
                                }
                                i += 1;
                                break;
                            } else if self.input[i] == b'\\' && quote == b'"' {
                                i += 2;
                            } else {
                                i += 1;
                            }
                        }
                    }
                    c if c == open => {
                        depth += 1;
                        i += 1;
                    }
                    c if c == close => {
                        depth -= 1;
                        i += 1;
                    }
                    _ => i += 1,
                }
            }
            // After the flow, check for colon - can be adjacent (no space required)
            if i < self.input.len() && self.input[i] == b':' {
                return true;
            }
            return false;
        }

        // Scan unquoted content for `: ` pattern
        while i < self.input.len() {
            match self.input[i] {
                b',' | b']' | b'}' | b'\n' => return false,
                b':' => {
                    let next = if i + 1 < self.input.len() {
                        Some(self.input[i + 1])
                    } else {
                        None
                    };
                    // In flow context, colon must be followed by space, or flow indicator
                    return matches!(
                        next,
                        Some(b' ') | Some(b'\t') | Some(b',') | Some(b']') | Some(b'}') | None
                    );
                }
                _ => i += 1,
            }
        }
        false
    }

    /// Check if we're looking at an explicit key indicator `?` in flow context
    fn looks_like_explicit_flow_key(&self) -> bool {
        self.peek() == Some(b'?')
            && matches!(
                self.peek_at(1),
                Some(b' ') | Some(b'\t') | Some(b'\n') | Some(b'\r') | None
            )
    }

    /// Parse an explicit mapping entry in flow context: `? key : value`
    /// Creates a single-pair mapping as the sequence element.
    fn parse_explicit_flow_mapping_entry(&mut self) -> Result<(), YamlError> {
        // Open implicit mapping
        self.set_ib();
        self.write_bp_open();
        self.write_ty(false); // 0 = mapping

        // Skip `?`
        self.advance();
        self.skip_flow_whitespace();

        // Parse key - can be scalar, quoted, flow mapping, or flow sequence
        self.set_ib();
        self.write_bp_open();
        match self.peek() {
            Some(b'{') => self.parse_flow_mapping()?,
            Some(b'[') => self.parse_flow_sequence()?,
            Some(b':') => {
                // Empty key (null) - ?: means null key
                // Don't consume anything, write empty node
            }
            Some(b',') | Some(b']') | Some(b'}') => {
                // Empty key (null) - ? followed by terminator
            }
            _ => self.parse_explicit_flow_key_scalar()?,
        }
        self.write_bp_close();

        // Skip whitespace before possible colon
        self.skip_flow_whitespace();

        // Check for colon (explicit value indicator)
        if self.peek() == Some(b':') {
            self.advance();
            self.skip_flow_whitespace();

            // Parse value (if present before , or ])
            if !matches!(self.peek(), Some(b',') | Some(b']') | Some(b'}') | None) {
                self.set_ib();
                self.write_bp_open();
                match self.peek() {
                    Some(b'[') => {
                        self.parse_flow_sequence()?;
                    }
                    Some(b'{') => {
                        self.parse_flow_mapping()?;
                    }
                    _ => {
                        self.parse_flow_scalar()?;
                    }
                }
                self.write_bp_close();
            } else {
                // Empty value (null)
                self.set_ib();
                self.write_bp_open();
                self.write_bp_close();
            }
        } else {
            // No colon - value is null
            self.write_bp_open_at(self.input.len());
            self.write_bp_close();
        }

        // Close implicit mapping
        self.write_bp_close();

        Ok(())
    }

    /// Parse an implicit mapping entry in flow context: `key : value`
    /// Creates a single-pair mapping as the sequence element.
    fn parse_implicit_flow_mapping_entry(&mut self) -> Result<(), YamlError> {
        // Open implicit mapping
        self.set_ib();
        self.write_bp_open();
        self.write_ty(false); // 0 = mapping

        // Parse key - can be scalar, quoted, flow mapping, or flow sequence
        self.set_ib();
        self.write_bp_open();
        match self.peek() {
            Some(b'{') => self.parse_flow_mapping()?,
            Some(b'[') => self.parse_flow_sequence()?,
            _ => self.parse_flow_key_scalar()?,
        }
        self.write_bp_close();

        // Skip whitespace before colon
        self.skip_inline_whitespace();

        // Expect and skip colon
        if self.peek() != Some(b':') {
            return Err(YamlError::UnexpectedCharacter {
                offset: self.pos,
                char: self.peek().map(|b| b as char).unwrap_or('\0'),
                context: "expected ':' in implicit flow mapping entry",
            });
        }
        self.advance();
        self.skip_flow_whitespace();

        // Parse value (if present before , or ])
        if !matches!(self.peek(), Some(b',') | Some(b']') | Some(b'}') | None) {
            self.set_ib();
            self.write_bp_open();
            match self.peek() {
                Some(b'[') => {
                    self.parse_flow_sequence()?;
                }
                Some(b'{') => {
                    self.parse_flow_mapping()?;
                }
                _ => {
                    self.parse_flow_scalar()?;
                }
            }
            self.write_bp_close();
        } else {
            // Empty value (null)
            self.set_ib();
            self.write_bp_open();
            self.write_bp_close();
        }

        // Close implicit mapping
        self.write_bp_close();

        Ok(())
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

            // Check for anchor first - it prefixes the actual value
            if self.peek() == Some(b'&') {
                self.parse_anchor()?;
            }

            // Check for alias - this IS the value
            if self.peek() == Some(b'*') {
                self.parse_alias()?;
                self.skip_flow_whitespace();
                continue;
            }

            // Check for explicit key `? key : value` in flow context
            if self.looks_like_explicit_flow_key() {
                self.parse_explicit_flow_mapping_entry()?;
            } else if self.looks_like_flow_mapping_entry() {
                // Check for implicit mapping entry (handles all key types including { and [)
                // This is an implicit single-pair mapping: [ key : value ]
                self.parse_implicit_flow_mapping_entry()?;
            } else {
                // Parse flow value (item) - containers handle their own BP
                match self.peek() {
                    Some(b'[') => {
                        self.parse_flow_sequence()?;
                    }
                    Some(b'{') => {
                        self.parse_flow_mapping()?;
                    }
                    _ => {
                        // Plain scalar value - wrap in BP
                        self.set_ib();
                        self.write_bp_open();
                        self.parse_flow_scalar()?;
                        self.write_bp_close();
                    }
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

            // Check for colon - if missing, value is implicitly null
            if self.peek() == Some(b':') {
                self.advance(); // Skip `:`
                self.skip_flow_whitespace();

                // Parse value - check for anchor or alias first
                // Check for anchor prefix on value
                let _anchor_name = if self.peek() == Some(b'&') {
                    Some(self.parse_anchor()?)
                } else {
                    None
                };

                // Check for alias (standalone value)
                if self.peek() == Some(b'*') {
                    self.parse_alias()?;
                } else {
                    // Parse the actual value - for nested containers, they handle their own BP
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
                }
            } else if matches!(self.peek(), Some(b',') | Some(b'}')) {
                // Key without colon/value - emit empty value (implicit null)
                self.set_ib();
                self.write_bp_open();
                self.write_bp_close();
            } else {
                return Err(YamlError::UnexpectedCharacter {
                    offset: self.pos,
                    char: self.peek().map(|b| b as char).unwrap_or('\0'),
                    context: "expected ':', ',' or '}' after key in flow mapping",
                });
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

    /// Parse a key in flow context.
    /// Keys can be scalars, flow sequences, or flow mappings (complex keys).
    fn parse_flow_key(&mut self) -> Result<(), YamlError> {
        // Check for anchor on key
        if self.peek() == Some(b'&') {
            let _ = self.parse_anchor()?;
            self.skip_flow_whitespace();
        }

        // Check for explicit key indicator
        if self.looks_like_explicit_flow_key() {
            // Skip `?`
            self.advance();
            self.skip_flow_whitespace();
            // Parse the actual key
            match self.peek() {
                Some(b'"') => {
                    self.parse_double_quoted()?;
                }
                Some(b'\'') => {
                    self.parse_single_quoted()?;
                }
                Some(b'[') => {
                    self.parse_flow_sequence()?;
                }
                Some(b'{') => {
                    self.parse_flow_mapping()?;
                }
                Some(b':') | Some(b',') | Some(b'}') => {
                    // Empty key (null) - don't consume anything
                }
                _ => {
                    self.parse_explicit_flow_unquoted_key()?;
                }
            }
            return Ok(());
        }

        match self.peek() {
            Some(b'"') => {
                self.parse_double_quoted()?;
            }
            Some(b'\'') => {
                self.parse_single_quoted()?;
            }
            Some(b'[') => {
                // Flow sequence as key (complex key)
                self.parse_flow_sequence()?;
            }
            Some(b'{') => {
                // Flow mapping as key (complex key)
                self.parse_flow_mapping()?;
            }
            Some(b'*') => {
                // Alias as key
                self.parse_alias()?;
            }
            _ => {
                self.parse_flow_unquoted_key()?;
            }
        }
        Ok(())
    }

    /// Parse an unquoted key in flow context.
    /// Stops at `:`, `,`, `}`, `]`, or whitespace before those.
    /// Handles multiline keys (continues across newlines with proper indentation).
    fn parse_flow_unquoted_key(&mut self) -> Result<usize, YamlError> {
        let start = self.pos;

        while let Some(b) = self.peek() {
            match b {
                b':' | b',' | b'}' | b']' => break,
                b'\n' | b'\r' => {
                    // Multiline key - check if next line continues the key
                    let mut lookahead = self.pos;
                    // Skip newline
                    if lookahead < self.input.len() && self.input[lookahead] == b'\r' {
                        lookahead += 1;
                    }
                    if lookahead < self.input.len() && self.input[lookahead] == b'\n' {
                        lookahead += 1;
                    }
                    // Skip leading whitespace on next line
                    while lookahead < self.input.len()
                        && matches!(self.input[lookahead], b' ' | b'\t')
                    {
                        lookahead += 1;
                    }
                    // Check what follows
                    if lookahead >= self.input.len()
                        || matches!(self.input[lookahead], b':' | b',' | b'}' | b']')
                    {
                        // Delimiter or EOF - stop key here
                        break;
                    }
                    // Continue parsing on next line
                    self.advance(); // Skip newline char(s)
                    if self.peek() == Some(b'\n') {
                        self.advance();
                    }
                    // Skip leading whitespace
                    while matches!(self.peek(), Some(b' ') | Some(b'\t')) {
                        self.advance();
                    }
                }
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
                            b'\n' | b'\r' => {
                                // Newline - check next line
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

        // Empty key is valid in YAML (e.g., `[ : value ]`)
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

    /// Parse a flow key (for implicit mapping entries).
    /// Like parse_flow_scalar but also stops at `: ` (colon followed by space/flow indicator).
    fn parse_flow_key_scalar(&mut self) -> Result<(), YamlError> {
        match self.peek() {
            Some(b'"') => {
                self.parse_double_quoted()?;
            }
            Some(b'\'') => {
                self.parse_single_quoted()?;
            }
            _ => {
                // Use existing parse_flow_unquoted_key which stops at `:`
                self.parse_flow_unquoted_key()?;
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
                b',' | b'}' | b']' => break,
                b'\n' | b'\r' => {
                    // Multiline value - check if next line continues the value
                    let mut lookahead = self.pos;
                    // Skip newline
                    if lookahead < self.input.len() && self.input[lookahead] == b'\r' {
                        lookahead += 1;
                    }
                    if lookahead < self.input.len() && self.input[lookahead] == b'\n' {
                        lookahead += 1;
                    }
                    // Skip leading whitespace on next line
                    while lookahead < self.input.len()
                        && matches!(self.input[lookahead], b' ' | b'\t')
                    {
                        lookahead += 1;
                    }
                    // Check what follows
                    if lookahead >= self.input.len()
                        || matches!(self.input[lookahead], b',' | b'}' | b']')
                    {
                        // Delimiter or EOF - stop value here
                        break;
                    }
                    // Continue parsing on next line
                    self.advance(); // Skip \r if present
                    if self.peek() == Some(b'\n') {
                        self.advance();
                    }
                    // Skip leading whitespace
                    while matches!(self.peek(), Some(b' ') | Some(b'\t')) {
                        self.advance();
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

        end - start
    }

    /// Parse an explicit flow key scalar.
    /// Unlike implicit flow keys which stop at `:`, explicit keys stop at `: ` (colon+space)
    /// because the colon is part of the explicit value syntax.
    fn parse_explicit_flow_key_scalar(&mut self) -> Result<(), YamlError> {
        match self.peek() {
            Some(b'"') => {
                self.parse_double_quoted()?;
            }
            Some(b'\'') => {
                self.parse_single_quoted()?;
            }
            _ => {
                self.parse_explicit_flow_unquoted_key()?;
            }
        }
        Ok(())
    }

    /// Parse an explicit unquoted key in flow context.
    /// Stops at `: ` (colon followed by whitespace) or flow delimiters, but NOT at bare `:`.
    fn parse_explicit_flow_unquoted_key(&mut self) -> Result<usize, YamlError> {
        let start = self.pos;

        while let Some(b) = self.peek() {
            match b {
                b',' | b'}' | b']' => break,
                b':' => {
                    // Only stop at `: ` or `:\n` or `:` at end
                    let next = self.peek_at(1);
                    if matches!(
                        next,
                        Some(b' ')
                            | Some(b'\t')
                            | Some(b'\n')
                            | Some(b'\r')
                            | Some(b',')
                            | Some(b'}')
                            | Some(b']')
                            | None
                    ) {
                        break;
                    }
                    // Colon not followed by space - include it in the key
                    self.advance();
                }
                b'\n' | b'\r' => {
                    // Multiline key - check if next line continues the key
                    let mut lookahead = self.pos;
                    // Skip newline
                    if lookahead < self.input.len() && self.input[lookahead] == b'\r' {
                        lookahead += 1;
                    }
                    if lookahead < self.input.len() && self.input[lookahead] == b'\n' {
                        lookahead += 1;
                    }
                    // Skip leading whitespace on next line
                    while lookahead < self.input.len()
                        && matches!(self.input[lookahead], b' ' | b'\t')
                    {
                        lookahead += 1;
                    }
                    // Check what follows
                    if lookahead >= self.input.len()
                        || matches!(self.input[lookahead], b',' | b'}' | b']')
                    {
                        // Delimiter or EOF - stop key here
                        break;
                    }
                    // Check for `: ` on next line (explicit value indicator)
                    if lookahead + 1 < self.input.len()
                        && self.input[lookahead] == b':'
                        && matches!(self.input[lookahead + 1], b' ' | b'\t' | b'\n' | b'\r')
                    {
                        // Explicit value indicator - stop key here
                        break;
                    }
                    // Check for single `:` followed by flow delimiter
                    if lookahead < self.input.len()
                        && self.input[lookahead] == b':'
                        && (lookahead + 1 >= self.input.len()
                            || matches!(self.input[lookahead + 1], b',' | b'}' | b']'))
                    {
                        break;
                    }
                    // Continue parsing on next line
                    self.advance(); // Skip newline char(s)
                    if self.peek() == Some(b'\n') {
                        self.advance();
                    }
                    // Skip leading whitespace
                    while matches!(self.peek(), Some(b' ') | Some(b'\t')) {
                        self.advance();
                    }
                }
                b' ' | b'\t' => {
                    // Check if whitespace is followed by `: ` or a delimiter
                    let mut lookahead = self.pos + 1;
                    while lookahead < self.input.len()
                        && matches!(self.input[lookahead], b' ' | b'\t')
                    {
                        lookahead += 1;
                    }
                    if lookahead < self.input.len() {
                        match self.input[lookahead] {
                            b':' => {
                                // Check if colon is followed by space/end
                                let after_colon = if lookahead + 1 < self.input.len() {
                                    Some(self.input[lookahead + 1])
                                } else {
                                    None
                                };
                                if matches!(
                                    after_colon,
                                    Some(b' ')
                                        | Some(b'\t')
                                        | Some(b'\n')
                                        | Some(b'\r')
                                        | Some(b',')
                                        | Some(b'}')
                                        | Some(b']')
                                        | None
                                ) {
                                    // Whitespace before `: ` - stop here
                                    break;
                                }
                                // Colon not followed by space - continue with the key
                                self.advance();
                            }
                            b',' | b'}' | b']' => {
                                // Whitespace before delimiter - stop here
                                break;
                            }
                            b'\n' | b'\r' => {
                                // Newline - check next line
                                break;
                            }
                            _ => {
                                // Continue with the key
                                self.advance();
                            }
                        }
                    } else {
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

        Ok(end - start)
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

    // =========================================================================
    // Anchor and alias parsing (Phase 4)
    // =========================================================================

    /// Parse an anchor name (characters after `&` or `*`).
    /// Valid anchor names: `[a-zA-Z0-9_-]+` (YAML 1.2 compliant)
    fn parse_anchor_name(&mut self) -> Result<String, YamlError> {
        let start = self.pos;

        // YAML anchor names can contain any character except flow indicators,
        // whitespace, and certain special chars.
        // Colons are allowed if not followed by whitespace (which would make it a key separator).
        while let Some(b) = self.peek() {
            match b {
                // Stop at flow indicators, whitespace, and newlines
                b' ' | b'\t' | b'\n' | b'\r' | b'[' | b']' | b'{' | b'}' | b',' => break,
                // Colon is allowed in anchor names if not followed by whitespace
                b':' => {
                    if let Some(next) = self.peek_at(1) {
                        if next == b' ' || next == b'\t' || next == b'\n' || next == b'\r' {
                            break;
                        }
                    }
                    // Colon followed by non-whitespace - part of anchor name
                    self.advance();
                }
                // High byte indicates start of multi-byte UTF-8 - consume all bytes
                0x80..=0xFF => {
                    // Multi-byte UTF-8 character - consume continuation bytes
                    self.advance();
                }
                _ => {
                    self.advance();
                }
            }
        }

        if self.pos == start {
            return Err(YamlError::InvalidAnchorName {
                offset: start,
                reason: "anchor name cannot be empty",
            });
        }

        // Convert to string
        let name = core::str::from_utf8(&self.input[start..self.pos])
            .map_err(|_| YamlError::InvalidUtf8 { offset: start })?
            .to_string();

        Ok(name)
    }

    /// Parse an anchor definition (`&name`).
    /// Records the anchor and returns, expecting the value to follow.
    fn parse_anchor(&mut self) -> Result<String, YamlError> {
        // Consume `&`
        self.advance();

        // Parse anchor name
        let name = self.parse_anchor_name()?;

        // Skip whitespace after anchor name
        self.skip_inline_whitespace();

        // Record anchor - will point to the next BP position (the value)
        // YAML allows anchor redefinition - later definitions override earlier ones
        // Store placeholder - will be updated when value BP is opened
        self.anchors.insert(name.clone(), self.bp_pos);

        Ok(name)
    }

    /// Parse an alias reference (`*name`).
    /// Creates a leaf node in the BP tree pointing to the aliased value.
    fn parse_alias(&mut self) -> Result<(), YamlError> {
        // Mark alias position
        self.set_ib();
        self.write_bp_open();

        // Consume `*`
        self.advance();

        // Parse anchor name
        let name = self.parse_anchor_name()?;

        // Resolve alias to anchor at parse time
        // This ensures we get the anchor definition that was active at this point
        let alias_bp_pos = self.bp_pos - 1;
        if let Some(&target_bp_pos) = self.anchors.get(&name) {
            self.aliases.insert(alias_bp_pos, target_bp_pos);
        }
        // Note: If anchor not found, we don't record it.
        // Forward references (alias before anchor) are not supported.

        // Close the alias node
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

        // Open virtual root sequence (wraps all documents)
        // Position 0 with text position 0
        self.write_bp_open_at(0);
        self.write_ty(true); // Root is a sequence
        self.type_stack.push(NodeType::Sequence);
        // Use usize::MAX as a sentinel indent for virtual root
        // This ensures document content at indent 0 creates its own container
        self.indent_stack[0] = usize::MAX;

        // Parse all documents (may be empty for comment-only files)
        if self.peek().is_some() {
            self.parse_documents()?;
        }

        // Close any remaining open document
        self.end_document();

        // Close virtual root sequence
        self.type_stack.pop();
        self.write_bp_close();

        Ok(SemiIndex {
            ib: self.ib_words.clone(),
            bp: self.bp_words.clone(),
            ty: self.ty_words.clone(),
            bp_to_text: self.bp_to_text.clone(),
            seq_items: self.seq_item_words.clone(),
            containers: self.container_words.clone(),
            ib_len: self.input.len(),
            bp_len: self.bp_pos,
            ty_len: self.ty_pos,
            anchors: core::mem::take(&mut self.anchors),
            aliases: core::mem::take(&mut self.aliases),
        })
    }

    /// Parse all documents in the stream.
    fn parse_documents(&mut self) -> Result<(), YamlError> {
        // Skip leading `---` if present (optional for first doc)
        if self.is_document_start() {
            self.skip_document_marker();

            // Check for inline content after `---` (e.g., `--- >` or `--- value`)
            if self.has_content_on_line() {
                self.start_document();
                self.parse_inline_document_value()?;
                // Don't skip newlines yet - let the main loop handle it
            } else {
                self.skip_newlines();
            }
        }

        // Check if file is empty after markers
        if self.peek().is_none() {
            // Empty YAML - nothing to parse
            return Ok(());
        }

        // Start first document if not already started
        if !self.in_document {
            self.start_document();
        }

        // Parse document content
        loop {
            self.skip_newlines();

            if self.peek().is_none() {
                break;
            }

            // Check for document end marker
            if self.is_document_end() {
                self.end_document();
                self.skip_document_marker();

                // Check for inline content after `...` (shouldn't normally have content)
                if !self.has_content_on_line() {
                    self.skip_newlines();
                }

                // Check for another document or EOF
                if self.peek().is_none() {
                    break;
                }

                // If there's a document start marker, skip it and check for inline content
                if self.is_document_start() {
                    self.skip_document_marker();
                    if self.has_content_on_line() {
                        self.start_document();
                        self.parse_inline_document_value()?;
                    } else {
                        self.skip_newlines();
                    }
                }

                // Start new document if there's content and not already started
                if self.peek().is_some() && !self.is_document_end() && !self.in_document {
                    self.start_document();
                }
                continue;
            }

            // Check for document start marker (new document)
            if self.is_document_start() {
                self.end_document();
                self.skip_document_marker();

                // Check for inline content after `---` (e.g., `--- >` or `--- value`)
                if self.has_content_on_line() {
                    self.start_document();
                    self.parse_inline_document_value()?;
                } else {
                    self.skip_newlines();
                    // Start new document if there's content
                    if self.peek().is_some() {
                        self.start_document();
                    }
                }
                continue;
            }

            // Parse document content
            self.parse_document_line()?;
        }

        Ok(())
    }

    /// Parse a single line of document content.
    fn parse_document_line(&mut self) -> Result<(), YamlError> {
        self.check_unsupported()?;

        // Count indentation - but handle tabs specially for flow structures
        let indent = match self.count_indent() {
            Ok(n) => n,
            Err(YamlError::TabIndentation { .. }) => {
                // Tabs found - check if this leads to a flow structure
                // Skip all leading whitespace (tabs and spaces)
                while matches!(self.peek(), Some(b' ') | Some(b'\t')) {
                    self.advance();
                }
                // If it's a flow structure, that's allowed
                match self.peek() {
                    Some(b'{') | Some(b'[') => {
                        self.close_deeper_indents(0);
                        self.parse_value(0)?;
                        // Move to next line if we haven't already
                        if self.peek() == Some(b'\n') {
                            self.advance();
                        }
                        return Ok(());
                    }
                    _ => {
                        // Not a flow structure - re-report the tab error
                        return Err(YamlError::TabIndentation {
                            line: self.line,
                            offset: self.pos,
                        });
                    }
                }
            }
            Err(e) => return Err(e),
        };

        // Skip to content
        for _ in 0..indent {
            self.advance();
        }

        // close_deeper_indents will handle closing any SequenceItem entries
        // when we return to a lower indent level

        // Check what kind of content this is
        match self.peek() {
            Some(b'-')
                if matches!(
                    self.peek_at(1),
                    Some(b' ') | Some(b'\t') | Some(b'\n') | Some(b'\r') | None
                ) =>
            {
                self.parse_sequence_item(indent)?;
            }
            Some(b'?')
                if matches!(
                    self.peek_at(1),
                    Some(b' ') | Some(b'\n') | Some(b'\r') | None
                ) =>
            {
                // Explicit key indicator
                self.parse_explicit_key(indent)?;
            }
            Some(b':')
                if matches!(
                    self.peek_at(1),
                    Some(b' ') | Some(b'\n') | Some(b'\r') | None
                ) =>
            {
                // Explicit value indicator (value for previous explicit key)
                self.parse_explicit_value(indent)?;
            }
            Some(b'#') => {
                // Comment line - skip
                self.skip_to_eol();
            }
            Some(b'\n') => {
                // Empty line
                self.advance();
            }
            Some(b'{') | Some(b'[') => {
                // Flow mapping or sequence at document root
                self.close_deeper_indents(indent);
                self.parse_value(indent)?;
            }
            Some(b'&') => {
                // Anchor - check if this is `&anchor key: value` (anchor on mapping key)
                // In that case, let parse_mapping_entry handle the anchor so it points
                // to the key, not the mapping container.
                self.close_deeper_indents(indent);

                // Look ahead to see if this is `&anchor key:` pattern
                let is_anchor_on_mapping_key = {
                    let saved_pos = self.pos;
                    // Skip `&`
                    self.advance();
                    // Skip anchor name
                    while let Some(b) = self.peek() {
                        match b {
                            b' ' | b'\t' | b'\n' | b'\r' | b'[' | b']' | b'{' | b'}' | b',' => {
                                break
                            }
                            b':' => {
                                if let Some(next) = self.peek_at(1) {
                                    if next == b' '
                                        || next == b'\t'
                                        || next == b'\n'
                                        || next == b'\r'
                                    {
                                        break;
                                    }
                                }
                                self.advance();
                            }
                            _ => self.advance(),
                        }
                    }
                    // Skip whitespace after anchor
                    while self.peek() == Some(b' ') || self.peek() == Some(b'\t') {
                        self.advance();
                    }
                    // Check if what follows looks like a mapping entry
                    let result = self.looks_like_mapping_entry();
                    self.pos = saved_pos;
                    result
                };

                if is_anchor_on_mapping_key {
                    // Let parse_mapping_entry handle the anchor
                    self.parse_mapping_entry(indent)?;
                } else {
                    // Parse anchor here for non-mapping-key cases
                    let _anchor_name = self.parse_anchor()?;
                    // Skip any whitespace after anchor
                    self.skip_inline_whitespace();
                    // Check what follows
                    match self.peek() {
                        Some(b'\n') | None => {
                            // Anchor with value on next line - will be parsed in next iteration
                        }
                        Some(b'-')
                            if matches!(
                                self.peek_at(1),
                                Some(b' ') | Some(b'\t') | Some(b'\n') | None
                            ) =>
                        {
                            // Anchor before block sequence on same line
                            self.parse_sequence_item(indent)?;
                        }
                        Some(b'{') | Some(b'[') => {
                            // Anchor before flow collection
                            self.parse_value(indent)?;
                        }
                        _ => {
                            // Scalar value
                            self.set_ib();
                            self.write_bp_open();
                            match self.peek() {
                                Some(b'"') => {
                                    self.parse_double_quoted()?;
                                }
                                Some(b'\'') => {
                                    self.parse_single_quoted()?;
                                }
                                _ => {
                                    self.parse_unquoted_value_with_indent(indent);
                                }
                            }
                            self.write_bp_close();
                        }
                    }
                }
            }
            Some(b'*') => {
                // Alias - could be a standalone value or a key in a mapping
                // Check if this is `*alias : value` pattern (alias as mapping key)
                if self.looks_like_mapping_entry() {
                    // Alias is a key - let parse_mapping_entry handle it
                    self.parse_mapping_entry(indent)?;
                } else {
                    // Standalone alias value
                    self.close_deeper_indents(indent);
                    self.parse_alias()?;
                }
            }
            Some(_) => {
                // Check if this looks like a mapping entry (has `: ` on this line)
                // This handles both quoted keys ("foo": bar) and unquoted keys (foo: bar)
                if self.looks_like_mapping_entry() {
                    self.parse_mapping_entry(indent)?;
                } else {
                    // Bare scalar document (e.g., `---\nplain value` or `---\n"quoted"`)
                    self.close_deeper_indents(indent);
                    self.set_ib();
                    self.write_bp_open();
                    match self.peek() {
                        Some(b'"') => {
                            self.parse_double_quoted()?;
                        }
                        Some(b'\'') => {
                            self.parse_single_quoted()?;
                        }
                        _ => {
                            self.parse_unquoted_value_with_indent(indent);
                        }
                    }
                    self.write_bp_close();
                }
            }
            None => {}
        }

        // Move to next line if we haven't already
        if self.peek() == Some(b'\n') {
            self.advance();
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
        // Whitespace-only is valid YAML (empty stream)
        let yaml = b"   \n\n  ";
        let result = build_semi_index(yaml);
        assert!(
            result.is_ok(),
            "Whitespace-only should parse as empty stream"
        );
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

    // =========================================================================
    // Multi-document stream tests (Phase 5)
    // =========================================================================

    #[test]
    fn test_single_document_wrapped() {
        // Single document should be wrapped in virtual root sequence
        let yaml = b"name: Alice";
        let result = build_semi_index(yaml).unwrap();
        // Root is sequence (TY bit 0 = 1)
        assert!(result.ty[0] & 1 == 1, "root should be sequence");
        // At least 2 TY bits (root sequence + document mapping)
        assert!(result.ty_len >= 2, "should have at least 2 containers");
    }

    #[test]
    fn test_explicit_document_start() {
        // Leading `---` should be handled
        let yaml = b"---\nname: Alice";
        let result = build_semi_index(yaml);
        assert!(
            result.is_ok(),
            "explicit document start should parse: {:?}",
            result
        );
    }

    #[test]
    fn test_two_documents() {
        // Two documents separated by `---`
        let yaml = b"---\nname: Alice\n---\nname: Bob";
        let result = build_semi_index(yaml);
        assert!(result.is_ok(), "two documents should parse: {:?}", result);
    }

    #[test]
    fn test_document_end_marker() {
        // Document end marker `...` followed by new document
        let yaml = b"---\nname: Alice\n...\n---\nname: Bob";
        let result = build_semi_index(yaml);
        assert!(
            result.is_ok(),
            "document with end marker should parse: {:?}",
            result
        );
    }

    #[test]
    fn test_document_end_at_eof() {
        // Document end marker at EOF
        let yaml = b"name: Alice\n...";
        let result = build_semi_index(yaml);
        assert!(
            result.is_ok(),
            "document end at EOF should parse: {:?}",
            result
        );
    }

    #[test]
    fn test_mixed_document_types() {
        // First document is sequence, second is mapping
        let yaml = b"---\n- item1\n- item2\n---\nkey: value";
        let result = build_semi_index(yaml);
        assert!(
            result.is_ok(),
            "mixed document types should parse: {:?}",
            result
        );
    }

    #[test]
    fn test_empty_between_markers() {
        // Empty content between document markers
        let yaml = b"---\n---\nname: Alice";
        let result = build_semi_index(yaml);
        assert!(result.is_ok(), "empty document should parse: {:?}", result);
    }

    #[test]
    fn test_document_marker_in_flow() {
        // `---` inside a quoted string should not be treated as marker
        let yaml = b"text: \"---\"";
        let result = build_semi_index(yaml);
        assert!(
            result.is_ok(),
            "quoted document marker should parse: {:?}",
            result
        );
    }

    #[test]
    fn test_three_documents() {
        let yaml = b"---\na: 1\n---\nb: 2\n---\nc: 3";
        let result = build_semi_index(yaml);
        assert!(result.is_ok(), "three documents should parse: {:?}", result);
    }

    #[test]
    fn test_question_mark_in_value() {
        // Question mark should be allowed in plain scalar values
        let yaml = b"- a?string";
        let result = build_semi_index(yaml);
        assert!(
            result.is_ok(),
            "question mark in value should parse: {:?}",
            result
        );
    }

    #[test]
    fn test_question_mark_in_key() {
        // Question mark should be allowed in plain scalar keys
        let yaml = b"key?: value";
        let result = build_semi_index(yaml);
        assert!(
            result.is_ok(),
            "question mark in key should parse: {:?}",
            result
        );
    }

    #[test]
    fn test_question_mark_in_flow_key() {
        // Question mark in flow mapping key
        let yaml = b"{key?: value}";
        let result = build_semi_index(yaml);
        assert!(
            result.is_ok(),
            "question mark in flow key should parse: {:?}",
            result
        );
    }

    #[test]
    fn test_question_mark_in_flow_value() {
        // Question mark in flow mapping value
        let yaml = b"{key: value?}";
        let result = build_semi_index(yaml);
        assert!(
            result.is_ok(),
            "question mark in flow value should parse: {:?}",
            result
        );
    }

    #[test]
    fn test_question_marks_full() {
        // Full JR7V test case - question marks in various contexts
        let yaml = b"- a?string\n- another ? string\n- key: value?\n- [a?string]\n- [another ? string]\n- {key: value? }\n- {key: value?}\n- {key?: value }";
        let result = build_semi_index(yaml);
        assert!(
            result.is_ok(),
            "question marks test should parse: {:?}",
            result
        );
    }

    #[test]
    fn test_compact_mapping_in_sequence() {
        // This is `- key: value` - a compact mapping within a sequence item
        let yaml = b"- key: value";
        let result = build_semi_index(yaml);
        assert!(
            result.is_ok(),
            "compact mapping in sequence should parse: {:?}",
            result
        );
    }

    #[test]
    fn test_flow_mapping_in_sequence() {
        // Flow mapping inside sequence item with various spacing patterns
        let yaml = b"- { one : two , three: four , }\n- {five: six,seven : eight}";
        let result = build_semi_index(yaml);
        assert!(
            result.is_ok(),
            "flow mapping in sequence should parse: {:?}",
            result
        );
    }

    #[test]
    fn test_double_colon_plain_scalar() {
        // ::vector is a plain scalar, not a mapping entry
        let yaml = b"- ::vector";
        let result = build_semi_index(yaml);
        assert!(
            result.is_ok(),
            "double colon plain scalar should parse: {:?}",
            result
        );
    }

    #[test]
    fn test_quoted_key_mapping() {
        // Quoted keys with special characters
        let yaml = b"\"foo\": bar\n'single': value";
        let result = build_semi_index(yaml);
        assert!(
            result.is_ok(),
            "quoted key mapping should parse: {:?}",
            result
        );
    }

    #[test]
    fn test_empty_key_in_flow_sequence() {
        // CFD4: [ : empty key ]
        let yaml = b"- [ : empty key ]";
        let result = build_semi_index(yaml);
        assert!(
            result.is_ok(),
            "empty key in flow sequence should parse: {:?}",
            result
        );
    }
}
