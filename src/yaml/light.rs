//! YamlCursor - Lazy YAML navigation using the semi-index.
//!
//! This module provides a cursor-based API for navigating YAML structures
//! without fully parsing the YAML text. Values are only decoded when explicitly
//! requested.

#[cfg(not(test))]
use alloc::{borrow::Cow, string::String, vec::Vec};

#[cfg(test)]
use std::borrow::Cow;

use super::index::YamlIndex;

// ============================================================================
// YamlCursor: Position in the YAML structure
// ============================================================================

/// A cursor pointing to a position in the YAML structure.
///
/// Cursors are lightweight (just a position integer) and cheap to copy.
/// Navigation methods return new cursors without mutation.
#[derive(Debug)]
pub struct YamlCursor<'a, W = Vec<u64>> {
    /// The original YAML text
    text: &'a [u8],
    /// Reference to the index
    index: &'a YamlIndex<W>,
    /// Position in the BP vector (0 = root)
    bp_pos: usize,
}

impl<'a, W> Clone for YamlCursor<'a, W> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<'a, W> Copy for YamlCursor<'a, W> {}

impl<'a, W: AsRef<[u64]>> YamlCursor<'a, W> {
    /// Create a new cursor at the given BP position.
    #[inline]
    pub fn new(index: &'a YamlIndex<W>, text: &'a [u8], bp_pos: usize) -> Self {
        Self {
            text,
            index,
            bp_pos,
        }
    }

    /// Get the position in the BP vector.
    #[inline]
    pub fn bp_position(&self) -> usize {
        self.bp_pos
    }

    /// Check if this cursor points to a container (mapping or sequence).
    #[inline]
    pub fn is_container(&self) -> bool {
        self.index.bp().first_child(self.bp_pos).is_some()
    }

    /// Get the byte position in the YAML text.
    ///
    /// Uses the direct BP-to-text mapping for O(1) lookup.
    pub fn text_position(&self) -> Option<usize> {
        self.index.bp_to_text_pos(self.bp_pos)
    }

    /// Navigate to the first child.
    #[inline]
    pub fn first_child(&self) -> Option<YamlCursor<'a, W>> {
        let new_pos = self.index.bp().first_child(self.bp_pos)?;
        Some(YamlCursor {
            text: self.text,
            index: self.index,
            bp_pos: new_pos,
        })
    }

    /// Navigate to the next sibling.
    #[inline]
    pub fn next_sibling(&self) -> Option<YamlCursor<'a, W>> {
        let new_pos = self.index.bp().next_sibling(self.bp_pos)?;
        Some(YamlCursor {
            text: self.text,
            index: self.index,
            bp_pos: new_pos,
        })
    }

    /// Navigate to the parent.
    #[inline]
    pub fn parent(&self) -> Option<YamlCursor<'a, W>> {
        let new_pos = self.index.bp().parent(self.bp_pos)?;
        Some(YamlCursor {
            text: self.text,
            index: self.index,
            bp_pos: new_pos,
        })
    }

    /// Get the YAML value at this cursor position.
    pub fn value(&self) -> YamlValue<'a, W> {
        let Some(text_pos) = self.text_position() else {
            return YamlValue::Error("invalid cursor position");
        };

        if text_pos >= self.text.len() {
            return YamlValue::Error("text position out of bounds");
        }

        // Check for flow containers by looking at the text
        // (empty flow containers may not have children, so check text first)
        let byte = self.text[text_pos];
        if byte == b'[' {
            // Flow sequence
            return YamlValue::Sequence(YamlElements::from_sequence_cursor(*self));
        }
        if byte == b'{' {
            // Flow mapping
            return YamlValue::Mapping(YamlFields::from_mapping_cursor(*self));
        }

        // Check if this is a container by looking at the BP structure
        if self.is_container() {
            // Determine if mapping or sequence based on the text
            match byte {
                b'-' => {
                    // Block sequence starts with `-`
                    return YamlValue::Sequence(YamlElements::from_sequence_cursor(*self));
                }
                _ => {
                    // Block mapping (key: value)
                    return YamlValue::Mapping(YamlFields::from_mapping_cursor(*self));
                }
            }
        }

        // Scalar value
        match self.text[text_pos] {
            b'"' => YamlValue::String(YamlString::DoubleQuoted {
                text: self.text,
                start: text_pos,
            }),
            b'\'' => YamlValue::String(YamlString::SingleQuoted {
                text: self.text,
                start: text_pos,
            }),
            b'|' => {
                // Block literal scalar
                let chomping = self.parse_chomping_indicator(text_pos);
                YamlValue::String(YamlString::BlockLiteral {
                    text: self.text,
                    indicator_pos: text_pos,
                    chomping,
                })
            }
            b'>' => {
                // Block folded scalar
                let chomping = self.parse_chomping_indicator(text_pos);
                YamlValue::String(YamlString::BlockFolded {
                    text: self.text,
                    indicator_pos: text_pos,
                    chomping,
                })
            }
            _ => {
                // Unquoted scalar
                let end = self.find_scalar_end(text_pos);
                YamlValue::String(YamlString::Unquoted {
                    text: self.text,
                    start: text_pos,
                    end,
                })
            }
        }
    }

    /// Parse chomping indicator from block scalar header.
    fn parse_chomping_indicator(&self, indicator_pos: usize) -> ChompingIndicator {
        let mut pos = indicator_pos + 1;
        // Check next 2 characters for chomping indicator
        for _ in 0..2 {
            if pos >= self.text.len() {
                break;
            }
            match self.text[pos] {
                b'-' => return ChompingIndicator::Strip,
                b'+' => return ChompingIndicator::Keep,
                b'0'..=b'9' => pos += 1, // Skip explicit indent
                _ => break,
            }
        }
        ChompingIndicator::Clip
    }

    /// Find the end of an unquoted scalar.
    fn find_scalar_end(&self, start: usize) -> usize {
        let mut end = start;
        while end < self.text.len() {
            match self.text[end] {
                // Block context delimiters
                b'\n' | b'#' => break,
                // Flow context delimiters
                b',' | b']' | b'}' => break,
                b':' => {
                    // Colon followed by space ends the scalar
                    if end + 1 < self.text.len()
                        && (self.text[end + 1] == b' ' || self.text[end + 1] == b'\n')
                    {
                        break;
                    }
                    end += 1;
                }
                _ => end += 1,
            }
        }
        // Trim trailing whitespace
        while end > start && self.text[end - 1] == b' ' {
            end -= 1;
        }
        end
    }

    /// Get children of this cursor for traversal.
    #[inline]
    pub fn children(&self) -> YamlChildren<'a, W> {
        YamlChildren {
            current: self.first_child(),
        }
    }

    /// Get the raw bytes for this YAML value.
    pub fn raw_bytes(&self) -> Option<&'a [u8]> {
        let start = self.text_position()?;
        let end = if self.is_container() {
            // For containers, find the closing position
            let close_bp = self.index.bp().find_close(self.bp_pos)?;
            let close_rank = self.index.bp().rank1(close_bp);
            self.index.ib_select1_from(close_rank, close_rank / 8)? + 1
        } else {
            // For scalars, find the value end
            match self.text.get(start)? {
                b'"' => self.find_double_quote_end(start),
                b'\'' => self.find_single_quote_end(start),
                _ => self.find_scalar_end(start),
            }
        };
        Some(&self.text[start..end.min(self.text.len())])
    }

    fn find_double_quote_end(&self, start: usize) -> usize {
        let mut i = start + 1;
        while i < self.text.len() {
            match self.text[i] {
                b'"' => return i + 1,
                b'\\' => i += 2,
                _ => i += 1,
            }
        }
        self.text.len()
    }

    fn find_single_quote_end(&self, start: usize) -> usize {
        let mut i = start + 1;
        while i < self.text.len() {
            if self.text[i] == b'\'' {
                if i + 1 < self.text.len() && self.text[i + 1] == b'\'' {
                    i += 2; // Escaped single quote
                } else {
                    return i + 1;
                }
            } else {
                i += 1;
            }
        }
        self.text.len()
    }
}

// ============================================================================
// YamlChildren: Iterator over children
// ============================================================================

/// Iterator over child cursors.
#[derive(Debug)]
pub struct YamlChildren<'a, W = Vec<u64>> {
    current: Option<YamlCursor<'a, W>>,
}

impl<'a, W> Clone for YamlChildren<'a, W> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<'a, W> Copy for YamlChildren<'a, W> {}

impl<'a, W: AsRef<[u64]>> Iterator for YamlChildren<'a, W> {
    type Item = YamlCursor<'a, W>;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        let cursor = self.current?;
        self.current = cursor.next_sibling();
        Some(cursor)
    }
}

// ============================================================================
// YamlValue: The value type
// ============================================================================

/// A YAML value with lazy decoding.
#[derive(Clone, Debug)]
pub enum YamlValue<'a, W = Vec<u64>> {
    /// A YAML string (various quote styles)
    String(YamlString<'a>),
    /// A YAML mapping (object-like)
    Mapping(YamlFields<'a, W>),
    /// A YAML sequence (array-like)
    Sequence(YamlElements<'a, W>),
    /// An error encountered during navigation
    Error(&'static str),
}

// ============================================================================
// YamlFields: Immutable iteration over mapping fields
// ============================================================================

/// Immutable "list" of YAML mapping fields.
#[derive(Debug)]
pub struct YamlFields<'a, W = Vec<u64>> {
    /// Cursor pointing to the current field key, or None if exhausted
    key_cursor: Option<YamlCursor<'a, W>>,
}

impl<'a, W> Clone for YamlFields<'a, W> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<'a, W> Copy for YamlFields<'a, W> {}

impl<'a, W: AsRef<[u64]>> YamlFields<'a, W> {
    /// Create a new YamlFields from a mapping cursor.
    pub fn from_mapping_cursor(mapping_cursor: YamlCursor<'a, W>) -> Self {
        Self {
            key_cursor: mapping_cursor.first_child(),
        }
    }

    /// Check if there are no more fields.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.key_cursor.is_none()
    }

    /// Get the first field and the remaining fields.
    pub fn uncons(&self) -> Option<(YamlField<'a, W>, YamlFields<'a, W>)> {
        let key_cursor = self.key_cursor?;
        let value_cursor = key_cursor.next_sibling()?;

        let rest = YamlFields {
            key_cursor: value_cursor.next_sibling(),
        };

        let field = YamlField {
            key_cursor,
            value_cursor,
        };

        Some((field, rest))
    }

    /// Find a field by name.
    pub fn find(&self, name: &str) -> Option<YamlValue<'a, W>> {
        let mut fields = *self;
        while let Some((field, rest)) = fields.uncons() {
            if let YamlValue::String(key) = field.key() {
                if key.as_str().ok()? == name {
                    return Some(field.value());
                }
            }
            fields = rest;
        }
        None
    }
}

impl<'a, W: AsRef<[u64]>> Iterator for YamlFields<'a, W> {
    type Item = YamlField<'a, W>;

    fn next(&mut self) -> Option<Self::Item> {
        let (field, rest) = self.uncons()?;
        *self = rest;
        Some(field)
    }
}

// ============================================================================
// YamlField: A single key-value pair
// ============================================================================

/// A single field in a YAML mapping.
#[derive(Debug)]
pub struct YamlField<'a, W = Vec<u64>> {
    key_cursor: YamlCursor<'a, W>,
    value_cursor: YamlCursor<'a, W>,
}

impl<'a, W> Clone for YamlField<'a, W> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<'a, W> Copy for YamlField<'a, W> {}

impl<'a, W: AsRef<[u64]>> YamlField<'a, W> {
    /// Get the field key.
    #[inline]
    pub fn key(&self) -> YamlValue<'a, W> {
        self.key_cursor.value()
    }

    /// Get the field value.
    #[inline]
    pub fn value(&self) -> YamlValue<'a, W> {
        self.value_cursor.value()
    }

    /// Get the value cursor directly.
    #[inline]
    pub fn value_cursor(&self) -> YamlCursor<'a, W> {
        self.value_cursor
    }
}

// ============================================================================
// YamlElements: Immutable iteration over sequence elements
// ============================================================================

/// Immutable "list" of YAML sequence elements.
#[derive(Debug)]
pub struct YamlElements<'a, W = Vec<u64>> {
    /// Cursor pointing to the current element, or None if exhausted
    element_cursor: Option<YamlCursor<'a, W>>,
}

impl<'a, W> Clone for YamlElements<'a, W> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<'a, W> Copy for YamlElements<'a, W> {}

impl<'a, W: AsRef<[u64]>> YamlElements<'a, W> {
    /// Create a new YamlElements from a sequence cursor.
    pub fn from_sequence_cursor(sequence_cursor: YamlCursor<'a, W>) -> Self {
        Self {
            element_cursor: sequence_cursor.first_child(),
        }
    }

    /// Check if there are no more elements.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.element_cursor.is_none()
    }

    /// Get the first element and the remaining elements.
    pub fn uncons(&self) -> Option<(YamlValue<'a, W>, YamlElements<'a, W>)> {
        let element_cursor = self.element_cursor?;

        let rest = YamlElements {
            element_cursor: element_cursor.next_sibling(),
        };

        // For block sequences, element_cursor points to the item wrapper node (at `-`).
        // We need to navigate to the item's first child to get the actual value.
        // If the item wrapper has a child, use that; otherwise use the wrapper itself
        // (for empty items or items where the value is implicitly null).
        let value_cursor = element_cursor.first_child().unwrap_or(element_cursor);
        let value = value_cursor.value();
        Some((value, rest))
    }

    /// Get element by index.
    pub fn get(&self, index: usize) -> Option<YamlValue<'a, W>> {
        let mut cursor = self.element_cursor?;
        for _ in 0..index {
            cursor = cursor.next_sibling()?;
        }
        // Navigate to item's child to get actual value
        let value_cursor = cursor.first_child().unwrap_or(cursor);
        Some(value_cursor.value())
    }
}

impl<'a, W: AsRef<[u64]>> Iterator for YamlElements<'a, W> {
    type Item = YamlValue<'a, W>;

    fn next(&mut self) -> Option<Self::Item> {
        let (elem, rest) = self.uncons()?;
        *self = rest;
        Some(elem)
    }
}

// ============================================================================
// YamlString: Lazy string decoding
// ============================================================================

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

/// A YAML string value with different encoding styles.
#[derive(Clone, Debug)]
pub enum YamlString<'a> {
    /// Double-quoted string (escapes need decoding)
    DoubleQuoted { text: &'a [u8], start: usize },
    /// Single-quoted string (' needs unescaping)
    SingleQuoted { text: &'a [u8], start: usize },
    /// Unquoted string (raw bytes)
    Unquoted {
        text: &'a [u8],
        start: usize,
        end: usize,
    },
    /// Block literal scalar (`|`): preserves newlines
    BlockLiteral {
        text: &'a [u8],
        indicator_pos: usize,
        chomping: ChompingIndicator,
    },
    /// Block folded scalar (`>`): folds newlines to spaces
    BlockFolded {
        text: &'a [u8],
        indicator_pos: usize,
        chomping: ChompingIndicator,
    },
}

impl<'a> YamlString<'a> {
    /// Get the raw bytes of the string (including quotes if applicable).
    pub fn raw_bytes(&self) -> &'a [u8] {
        match self {
            YamlString::DoubleQuoted { text, start } => {
                let end = Self::find_double_quote_end(text, *start);
                &text[*start..end]
            }
            YamlString::SingleQuoted { text, start } => {
                let end = Self::find_single_quote_end(text, *start);
                &text[*start..end]
            }
            YamlString::Unquoted { text, start, end } => &text[*start..*end],
            YamlString::BlockLiteral {
                text,
                indicator_pos,
                chomping,
            }
            | YamlString::BlockFolded {
                text,
                indicator_pos,
                chomping,
            } => {
                let (_, content_end) =
                    Self::find_block_content_range(text, *indicator_pos, *chomping);
                &text[*indicator_pos..content_end]
            }
        }
    }

    /// Decode the string value.
    ///
    /// Returns a `Cow::Borrowed` for strings without escapes,
    /// or a `Cow::Owned` for strings that need escape decoding.
    pub fn as_str(&self) -> Result<Cow<'a, str>, YamlStringError> {
        match self {
            YamlString::DoubleQuoted { text, start } => {
                let end = Self::find_double_quote_end(text, *start);
                let bytes = &text[*start + 1..end - 1]; // Strip quotes
                if !bytes.contains(&b'\\') {
                    let s =
                        core::str::from_utf8(bytes).map_err(|_| YamlStringError::InvalidUtf8)?;
                    Ok(Cow::Borrowed(s))
                } else {
                    decode_double_quoted_escapes(bytes).map(Cow::Owned)
                }
            }
            YamlString::SingleQuoted { text, start } => {
                let end = Self::find_single_quote_end(text, *start);
                let bytes = &text[*start + 1..end - 1]; // Strip quotes
                if !bytes.contains(&b'\'') {
                    let s =
                        core::str::from_utf8(bytes).map_err(|_| YamlStringError::InvalidUtf8)?;
                    Ok(Cow::Borrowed(s))
                } else {
                    decode_single_quoted_escapes(bytes).map(Cow::Owned)
                }
            }
            YamlString::Unquoted { text, start, end } => {
                let bytes = &text[*start..*end];
                let s = core::str::from_utf8(bytes).map_err(|_| YamlStringError::InvalidUtf8)?;
                Ok(Cow::Borrowed(s))
            }
            YamlString::BlockLiteral {
                text,
                indicator_pos,
                chomping,
            } => decode_block_literal(text, *indicator_pos, *chomping),
            YamlString::BlockFolded {
                text,
                indicator_pos,
                chomping,
            } => decode_block_folded(text, *indicator_pos, *chomping),
        }
    }

    fn find_double_quote_end(text: &[u8], start: usize) -> usize {
        let mut i = start + 1;
        while i < text.len() {
            match text[i] {
                b'"' => return i + 1,
                b'\\' => i += 2,
                _ => i += 1,
            }
        }
        text.len()
    }

    fn find_single_quote_end(text: &[u8], start: usize) -> usize {
        let mut i = start + 1;
        while i < text.len() {
            if text[i] == b'\'' {
                if i + 1 < text.len() && text[i + 1] == b'\'' {
                    i += 2;
                } else {
                    return i + 1;
                }
            } else {
                i += 1;
            }
        }
        text.len()
    }

    /// Find content range for a block scalar.
    /// Returns (content_start, content_end).
    fn find_block_content_range(
        text: &[u8],
        indicator_pos: usize,
        chomping: ChompingIndicator,
    ) -> (usize, usize) {
        // Skip indicator and modifiers to find newline
        let mut pos = indicator_pos + 1;
        while pos < text.len() && text[pos] != b'\n' {
            pos += 1;
        }
        if pos >= text.len() {
            return (pos, pos); // Empty block scalar
        }
        pos += 1; // Skip newline

        let content_start = pos;

        // Determine content indentation from first non-empty line
        let content_indent = Self::detect_block_indent(text, pos);
        if content_indent == 0 {
            return (content_start, content_start); // Empty block scalar
        }

        // Find end of block scalar content
        let mut last_content_end = pos;
        let mut trailing_newline_start = pos;

        while pos < text.len() {
            let line_start = pos;

            // Count spaces at start of line
            let mut line_indent = 0;
            while pos < text.len() && text[pos] == b' ' {
                line_indent += 1;
                pos += 1;
            }

            // Check what's on this line
            if pos >= text.len() {
                break;
            }

            match text[pos] {
                b'\n' => {
                    // Empty line
                    trailing_newline_start = line_start;
                    pos += 1;
                }
                b'\r' => {
                    trailing_newline_start = line_start;
                    pos += 1;
                    if pos < text.len() && text[pos] == b'\n' {
                        pos += 1;
                    }
                }
                _ => {
                    if line_indent < content_indent {
                        // Dedent - end of block
                        pos = line_start;
                        break;
                    }

                    // Content line - skip to end
                    while pos < text.len() && text[pos] != b'\n' && text[pos] != b'\r' {
                        pos += 1;
                    }
                    last_content_end = pos;
                    trailing_newline_start = pos;

                    if pos < text.len() {
                        if text[pos] == b'\r' {
                            pos += 1;
                            if pos < text.len() && text[pos] == b'\n' {
                                pos += 1;
                            }
                        } else if text[pos] == b'\n' {
                            pos += 1;
                        }
                    }
                }
            }
        }

        // Apply chomping
        let content_end = match chomping {
            ChompingIndicator::Strip => last_content_end,
            ChompingIndicator::Clip => {
                if last_content_end < text.len()
                    && trailing_newline_start >= last_content_end
                    && trailing_newline_start < text.len()
                {
                    // Include one newline
                    let mut end = last_content_end;
                    if end < text.len() && text[end] == b'\n' {
                        end += 1;
                    } else if end < text.len() && text[end] == b'\r' {
                        end += 1;
                        if end < text.len() && text[end] == b'\n' {
                            end += 1;
                        }
                    }
                    end
                } else {
                    last_content_end
                }
            }
            ChompingIndicator::Keep => pos,
        };

        (content_start, content_end)
    }

    /// Detect content indentation from first non-empty line.
    fn detect_block_indent(text: &[u8], start: usize) -> usize {
        let mut pos = start;

        loop {
            if pos >= text.len() {
                return 0;
            }

            // Count spaces
            let mut indent = 0;
            while pos < text.len() && text[pos] == b' ' {
                indent += 1;
                pos += 1;
            }

            if pos >= text.len() {
                return 0;
            }

            match text[pos] {
                b'\n' => {
                    pos += 1;
                }
                b'\r' => {
                    pos += 1;
                    if pos < text.len() && text[pos] == b'\n' {
                        pos += 1;
                    }
                }
                b'#' => {
                    // Comment line - skip
                    while pos < text.len() && text[pos] != b'\n' {
                        pos += 1;
                    }
                    if pos < text.len() {
                        pos += 1;
                    }
                }
                _ => {
                    return indent;
                }
            }
        }
    }
}

/// Errors that can occur during string decoding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum YamlStringError {
    /// Invalid UTF-8 in string
    InvalidUtf8,
    /// Invalid escape sequence
    InvalidEscape,
}

impl core::fmt::Display for YamlStringError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            YamlStringError::InvalidUtf8 => write!(f, "invalid UTF-8 in string"),
            YamlStringError::InvalidEscape => write!(f, "invalid escape sequence"),
        }
    }
}

/// Decode escape sequences in a double-quoted YAML string.
fn decode_double_quoted_escapes(bytes: &[u8]) -> Result<String, YamlStringError> {
    let mut result = String::with_capacity(bytes.len());
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] == b'\\' {
            if i + 1 >= bytes.len() {
                return Err(YamlStringError::InvalidEscape);
            }
            i += 1;
            match bytes[i] {
                b'0' => result.push('\0'),
                b'a' => result.push('\x07'), // bell
                b'b' => result.push('\x08'), // backspace
                b't' | b'\t' => result.push('\t'),
                b'n' => result.push('\n'),
                b'v' => result.push('\x0B'), // vertical tab
                b'f' => result.push('\x0C'), // form feed
                b'r' => result.push('\r'),
                b'e' => result.push('\x1B'), // escape
                b' ' => result.push(' '),
                b'"' => result.push('"'),
                b'/' => result.push('/'),
                b'\\' => result.push('\\'),
                b'N' => result.push('\u{0085}'), // next line
                b'_' => result.push('\u{00A0}'), // non-breaking space
                b'L' => result.push('\u{2028}'), // line separator
                b'P' => result.push('\u{2029}'), // paragraph separator
                b'x' => {
                    // \xNN - 2 hex digits
                    if i + 2 >= bytes.len() {
                        return Err(YamlStringError::InvalidEscape);
                    }
                    let hex = &bytes[i + 1..i + 3];
                    let val = parse_hex(hex)?;
                    if val <= 0x7F {
                        result.push(val as u8 as char);
                    } else {
                        result.push(
                            char::from_u32(val as u32).ok_or(YamlStringError::InvalidEscape)?,
                        );
                    }
                    i += 2;
                }
                b'u' => {
                    // \uNNNN - 4 hex digits
                    if i + 4 >= bytes.len() {
                        return Err(YamlStringError::InvalidEscape);
                    }
                    let hex = &bytes[i + 1..i + 5];
                    let codepoint = parse_hex(hex)? as u32;
                    result.push(char::from_u32(codepoint).ok_or(YamlStringError::InvalidEscape)?);
                    i += 4;
                }
                b'U' => {
                    // \UNNNNNNNN - 8 hex digits
                    if i + 8 >= bytes.len() {
                        return Err(YamlStringError::InvalidEscape);
                    }
                    let hex = &bytes[i + 1..i + 9];
                    let codepoint = parse_hex(hex)?;
                    result.push(char::from_u32(codepoint).ok_or(YamlStringError::InvalidEscape)?);
                    i += 8;
                }
                _ => return Err(YamlStringError::InvalidEscape),
            }
            i += 1;
        } else {
            // Regular UTF-8 byte
            let start = i;
            while i < bytes.len() && bytes[i] != b'\\' {
                i += 1;
            }
            let chunk =
                core::str::from_utf8(&bytes[start..i]).map_err(|_| YamlStringError::InvalidUtf8)?;
            result.push_str(chunk);
        }
    }

    Ok(result)
}

/// Decode escape sequences in a single-quoted YAML string.
fn decode_single_quoted_escapes(bytes: &[u8]) -> Result<String, YamlStringError> {
    let mut result = String::with_capacity(bytes.len());
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] == b'\'' && i + 1 < bytes.len() && bytes[i + 1] == b'\'' {
            // '' -> '
            result.push('\'');
            i += 2;
        } else {
            let start = i;
            while i < bytes.len()
                && !(bytes[i] == b'\'' && i + 1 < bytes.len() && bytes[i + 1] == b'\'')
            {
                i += 1;
            }
            let chunk =
                core::str::from_utf8(&bytes[start..i]).map_err(|_| YamlStringError::InvalidUtf8)?;
            result.push_str(chunk);
        }
    }

    Ok(result)
}

/// Parse hex digits into a u32.
fn parse_hex(hex: &[u8]) -> Result<u32, YamlStringError> {
    let mut value = 0u32;
    for &b in hex {
        let digit = match b {
            b'0'..=b'9' => b - b'0',
            b'a'..=b'f' => b - b'a' + 10,
            b'A'..=b'F' => b - b'A' + 10,
            _ => return Err(YamlStringError::InvalidEscape),
        };
        value = value * 16 + digit as u32;
    }
    Ok(value)
}

/// Decode a literal block scalar (preserves newlines).
fn decode_block_literal<'a>(
    text: &'a [u8],
    indicator_pos: usize,
    chomping: ChompingIndicator,
) -> Result<Cow<'a, str>, YamlStringError> {
    let (content_start, content_end) =
        YamlString::find_block_content_range(text, indicator_pos, chomping);

    if content_start >= content_end {
        return Ok(Cow::Borrowed(""));
    }

    let content = &text[content_start..content_end];

    // Detect the common indentation to strip
    let indent = YamlString::detect_block_indent(text, content_start);
    if indent == 0 {
        // No indentation to strip - just convert to string
        let s = core::str::from_utf8(content).map_err(|_| YamlStringError::InvalidUtf8)?;
        return Ok(Cow::Borrowed(s));
    }

    // Build result by stripping indent from each line
    let mut result = String::with_capacity(content.len());
    let mut pos = 0;

    while pos < content.len() {
        // Count and skip indentation
        let mut line_indent = 0;
        while pos + line_indent < content.len() && content[pos + line_indent] == b' ' {
            line_indent += 1;
        }

        // Strip the common indent (up to `indent` spaces)
        let skip = line_indent.min(indent);
        pos += skip;

        // Find end of line
        let line_start = pos;
        while pos < content.len() && content[pos] != b'\n' && content[pos] != b'\r' {
            pos += 1;
        }

        // Append line content
        let line = core::str::from_utf8(&content[line_start..pos])
            .map_err(|_| YamlStringError::InvalidUtf8)?;
        result.push_str(line);

        // Handle line ending
        if pos < content.len() {
            if content[pos] == b'\r' {
                pos += 1;
                result.push('\n');
                if pos < content.len() && content[pos] == b'\n' {
                    pos += 1;
                }
            } else if content[pos] == b'\n' {
                pos += 1;
                result.push('\n');
            }
        }
    }

    Ok(Cow::Owned(result))
}

/// Decode a folded block scalar (folds newlines to spaces).
fn decode_block_folded<'a>(
    text: &'a [u8],
    indicator_pos: usize,
    chomping: ChompingIndicator,
) -> Result<Cow<'a, str>, YamlStringError> {
    let (content_start, content_end) =
        YamlString::find_block_content_range(text, indicator_pos, chomping);

    if content_start >= content_end {
        return Ok(Cow::Borrowed(""));
    }

    let content = &text[content_start..content_end];

    // Detect the common indentation to strip
    let indent = YamlString::detect_block_indent(text, content_start);

    // Build result by folding newlines
    let mut result = String::with_capacity(content.len());
    let mut pos = 0;
    let mut prev_was_blank = false;
    let mut prev_was_more_indented = false;
    let mut first_line = true;

    while pos < content.len() {
        // Count indentation
        let mut line_indent = 0;
        while pos + line_indent < content.len() && content[pos + line_indent] == b' ' {
            line_indent += 1;
        }

        // Check if this is a blank line
        let is_blank = pos + line_indent >= content.len()
            || content[pos + line_indent] == b'\n'
            || content[pos + line_indent] == b'\r';

        if is_blank {
            // Blank line = paragraph break
            if !result.is_empty() && !result.ends_with('\n') {
                result.push('\n');
            }
            result.push('\n');
            prev_was_blank = true;
            prev_was_more_indented = false;

            // Skip to next line
            pos += line_indent;
            if pos < content.len() {
                if content[pos] == b'\r' {
                    pos += 1;
                    if pos < content.len() && content[pos] == b'\n' {
                        pos += 1;
                    }
                } else if content[pos] == b'\n' {
                    pos += 1;
                }
            }
        } else {
            // Strip the common indent
            let skip = line_indent.min(indent);
            pos += skip;

            // Check if more indented (relative to content indent)
            let is_more_indented = line_indent > indent;

            // Find end of line
            let line_start = pos;
            while pos < content.len() && content[pos] != b'\n' && content[pos] != b'\r' {
                pos += 1;
            }

            // Add joining character
            if !first_line && !result.is_empty() && !result.ends_with('\n') {
                if is_more_indented || prev_was_more_indented || prev_was_blank {
                    result.push('\n');
                } else {
                    result.push(' ');
                }
            }

            // Append line content
            let line = core::str::from_utf8(&content[line_start..pos])
                .map_err(|_| YamlStringError::InvalidUtf8)?;
            result.push_str(line);

            prev_was_blank = false;
            prev_was_more_indented = is_more_indented;
            first_line = false;

            // Skip line ending
            if pos < content.len() {
                if content[pos] == b'\r' {
                    pos += 1;
                    if pos < content.len() && content[pos] == b'\n' {
                        pos += 1;
                    }
                } else if content[pos] == b'\n' {
                    pos += 1;
                }
            }
        }
    }

    // Apply chomping at the end
    match chomping {
        ChompingIndicator::Strip => {
            while result.ends_with('\n') {
                result.pop();
            }
        }
        ChompingIndicator::Clip => {
            while result.ends_with("\n\n") {
                result.pop();
            }
            if !result.is_empty() && !result.ends_with('\n') {
                result.push('\n');
            }
        }
        ChompingIndicator::Keep => {
            // Keep all trailing newlines as-is
        }
    }

    Ok(Cow::Owned(result))
}

// ============================================================================
// YamlNumber: Lazy number parsing
// ============================================================================

/// A YAML number that hasn't been parsed yet.
#[derive(Clone, Copy, Debug)]
pub struct YamlNumber<'a> {
    text: &'a [u8],
    start: usize,
    end: usize,
}

impl<'a> YamlNumber<'a> {
    /// Create a new YamlNumber.
    pub fn new(text: &'a [u8], start: usize, end: usize) -> Self {
        Self { text, start, end }
    }

    /// Get the raw bytes of the number.
    pub fn raw_bytes(&self) -> &'a [u8] {
        &self.text[self.start..self.end]
    }

    /// Parse as i64.
    pub fn as_i64(&self) -> Result<i64, YamlNumberError> {
        let bytes = self.raw_bytes();
        let s = core::str::from_utf8(bytes).map_err(|_| YamlNumberError::InvalidUtf8)?;
        s.parse().map_err(|_| YamlNumberError::InvalidNumber)
    }

    /// Parse as f64.
    pub fn as_f64(&self) -> Result<f64, YamlNumberError> {
        let bytes = self.raw_bytes();
        let s = core::str::from_utf8(bytes).map_err(|_| YamlNumberError::InvalidUtf8)?;
        s.parse().map_err(|_| YamlNumberError::InvalidNumber)
    }
}

/// Errors that can occur during number parsing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum YamlNumberError {
    /// Invalid UTF-8 in number
    InvalidUtf8,
    /// Invalid number format
    InvalidNumber,
}

impl core::fmt::Display for YamlNumberError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            YamlNumberError::InvalidUtf8 => write!(f, "invalid UTF-8 in number"),
            YamlNumberError::InvalidNumber => write!(f, "invalid number format"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::yaml::YamlIndex;

    #[test]
    fn test_simple_mapping_navigation() {
        let yaml = b"name: Alice";
        let index = YamlIndex::build(yaml).unwrap();
        let root = index.root(yaml);

        // Root should be a mapping
        match root.value() {
            YamlValue::Mapping(fields) => {
                assert!(!fields.is_empty());
            }
            _ => panic!("expected mapping"),
        }
    }

    #[test]
    fn test_double_quoted_string() {
        let yaml = b"name: \"Alice\"";
        let index = YamlIndex::build(yaml).unwrap();
        let root = index.root(yaml);

        // Root should be at position 0
        assert_eq!(root.text_position(), Some(0));

        // Root should be a mapping
        if let YamlValue::Mapping(fields) = root.value() {
            assert!(!fields.is_empty());
            if let Some((field, _rest)) = fields.uncons() {
                // Key should be "name"
                if let YamlValue::String(k) = field.key() {
                    assert_eq!(&*k.as_str().unwrap(), "name");
                } else {
                    panic!("expected string key");
                }
                // Value should be "Alice"
                if let YamlValue::String(v) = field.value() {
                    assert_eq!(&*v.as_str().unwrap(), "Alice");
                } else {
                    panic!("expected string value");
                }
            }
        } else {
            panic!("expected mapping");
        }
    }

    #[test]
    fn test_single_quoted_string() {
        let yaml = b"name: 'Alice'";
        let index = YamlIndex::build(yaml).unwrap();
        let root = index.root(yaml);

        if let YamlValue::Mapping(fields) = root.value() {
            if let Some(YamlValue::String(s)) = fields.find("name") {
                assert_eq!(&*s.as_str().unwrap(), "Alice");
            }
        }
    }

    #[test]
    fn test_unquoted_string() {
        let yaml = b"name: Alice";
        let index = YamlIndex::build(yaml).unwrap();
        let root = index.root(yaml);

        if let YamlValue::Mapping(fields) = root.value() {
            if let Some(YamlValue::String(s)) = fields.find("name") {
                assert_eq!(&*s.as_str().unwrap(), "Alice");
            }
        }
    }

    #[test]
    fn test_escape_double_quote() {
        let s = YamlString::DoubleQuoted {
            text: b"\"hello\\nworld\"",
            start: 0,
        };
        assert_eq!(&*s.as_str().unwrap(), "hello\nworld");
    }

    #[test]
    fn test_escape_single_quote() {
        let s = YamlString::SingleQuoted {
            text: b"'it''s'",
            start: 0,
        };
        assert_eq!(&*s.as_str().unwrap(), "it's");
    }

    // =========================================================================
    // Flow style navigation tests (Phase 2)
    // =========================================================================

    #[test]
    fn test_flow_sequence_navigation() {
        let yaml = b"items: [1, 2, 3]";
        let index = YamlIndex::build(yaml).unwrap();
        let root = index.root(yaml);

        // Root should be a mapping with "items" key
        if let YamlValue::Mapping(fields) = root.value() {
            if let Some(YamlValue::Sequence(elements)) = fields.find("items") {
                let items: Vec<_> = elements.collect();
                assert_eq!(items.len(), 3);

                // Check first element
                if let YamlValue::String(s) = &items[0] {
                    assert_eq!(&*s.as_str().unwrap(), "1");
                } else {
                    panic!("expected string value for item");
                }
            } else {
                panic!("expected sequence for items");
            }
        } else {
            panic!("expected mapping");
        }
    }

    #[test]
    fn test_flow_mapping_navigation() {
        let yaml = b"person: {name: Alice, age: 30}";
        let index = YamlIndex::build(yaml).unwrap();
        let root = index.root(yaml);

        // Root should be a mapping with "person" key
        if let YamlValue::Mapping(fields) = root.value() {
            if let Some(YamlValue::Mapping(person_fields)) = fields.find("person") {
                // Check name
                if let Some(YamlValue::String(s)) = person_fields.find("name") {
                    assert_eq!(&*s.as_str().unwrap(), "Alice");
                } else {
                    panic!("expected name field");
                }

                // Check age
                if let Some(YamlValue::String(s)) = person_fields.find("age") {
                    assert_eq!(&*s.as_str().unwrap(), "30");
                } else {
                    panic!("expected age field");
                }
            } else {
                panic!("expected mapping for person");
            }
        } else {
            panic!("expected mapping");
        }
    }

    #[test]
    fn test_flow_nested_navigation() {
        let yaml = b"data: {users: [{name: Alice}, {name: Bob}]}";
        let index = YamlIndex::build(yaml).unwrap();
        let root = index.root(yaml);

        if let YamlValue::Mapping(fields) = root.value() {
            if let Some(YamlValue::Mapping(data_fields)) = fields.find("data") {
                if let Some(YamlValue::Sequence(users)) = data_fields.find("users") {
                    let items: Vec<_> = users.collect();
                    assert_eq!(items.len(), 2, "expected 2 users");

                    // Check first user
                    if let YamlValue::Mapping(user_fields) = &items[0] {
                        if let Some(YamlValue::String(s)) = user_fields.find("name") {
                            assert_eq!(&*s.as_str().unwrap(), "Alice");
                        }
                    }

                    // Check second user
                    if let YamlValue::Mapping(user_fields) = &items[1] {
                        if let Some(YamlValue::String(s)) = user_fields.find("name") {
                            assert_eq!(&*s.as_str().unwrap(), "Bob");
                        }
                    }
                } else {
                    panic!("expected users sequence");
                }
            } else {
                panic!("expected data mapping");
            }
        } else {
            panic!("expected mapping");
        }
    }

    #[test]
    fn test_flow_empty_sequence_navigation() {
        let yaml = b"items: []";
        let index = YamlIndex::build(yaml).unwrap();
        let root = index.root(yaml);

        if let YamlValue::Mapping(fields) = root.value() {
            if let Some(YamlValue::Sequence(elements)) = fields.find("items") {
                let items: Vec<_> = elements.collect();
                assert_eq!(items.len(), 0, "expected empty sequence");
            } else {
                panic!("expected sequence for items");
            }
        } else {
            panic!("expected mapping");
        }
    }

    #[test]
    fn test_flow_empty_mapping_navigation() {
        let yaml = b"data: {}";
        let index = YamlIndex::build(yaml).unwrap();
        let root = index.root(yaml);

        if let YamlValue::Mapping(fields) = root.value() {
            if let Some(YamlValue::Mapping(data_fields)) = fields.find("data") {
                assert!(data_fields.is_empty(), "expected empty mapping");
            } else {
                panic!("expected mapping for data");
            }
        } else {
            panic!("expected mapping");
        }
    }

    // =========================================================================
    // Block scalar navigation tests (Phase 3)
    // =========================================================================

    #[test]
    fn test_block_literal_navigation() {
        let yaml = b"text: |\n  Line 1\n  Line 2\n";
        let index = YamlIndex::build(yaml).unwrap();
        let root = index.root(yaml);

        if let YamlValue::Mapping(fields) = root.value() {
            if let Some(YamlValue::String(s)) = fields.find("text") {
                // Block literal should preserve newlines
                let decoded = s.as_str().unwrap();
                assert!(
                    decoded.contains("Line 1") && decoded.contains("Line 2"),
                    "block literal should contain both lines, got: {:?}",
                    decoded
                );
            } else {
                panic!("expected string for text");
            }
        } else {
            panic!("expected mapping");
        }
    }

    #[test]
    fn test_block_folded_navigation() {
        let yaml = b"text: >\n  First part\n  second part\n";
        let index = YamlIndex::build(yaml).unwrap();
        let root = index.root(yaml);

        if let YamlValue::Mapping(fields) = root.value() {
            if let Some(YamlValue::String(s)) = fields.find("text") {
                // Block folded should fold newlines to spaces
                let decoded = s.as_str().unwrap();
                assert!(
                    decoded.contains("First part") && decoded.contains("second part"),
                    "block folded should contain both parts, got: {:?}",
                    decoded
                );
            } else {
                panic!("expected string for text");
            }
        } else {
            panic!("expected mapping");
        }
    }

    #[test]
    fn test_block_scalar_in_sequence() {
        // First test a simple sequence without block scalars to establish baseline
        let yaml_simple = b"- item1\n- item2\n";
        let index_simple = YamlIndex::build(yaml_simple).unwrap();
        let root_simple = index_simple.root(yaml_simple);

        if let YamlValue::Sequence(elements) = root_simple.value() {
            let items: Vec<_> = elements.collect();
            assert_eq!(items.len(), 2, "simple: expected 2 items");
            // Check these work
            if let YamlValue::String(s) = &items[0] {
                assert_eq!(&*s.as_str().unwrap(), "item1");
            } else {
                panic!("simple: expected string for item1, got: {:?}", items[0]);
            }
        } else {
            panic!("simple: expected sequence");
        }

        // Now test with block scalar
        let yaml = b"- |\n  item\n- value\n";
        let index = YamlIndex::build(yaml).unwrap();
        let root = index.root(yaml);

        if let YamlValue::Sequence(elements) = root.value() {
            let items: Vec<_> = elements.collect();
            assert_eq!(items.len(), 2, "expected 2 items, got items: {:?}", items);

            // First item is block literal
            if let YamlValue::String(s) = &items[0] {
                let decoded = s.as_str().unwrap();
                assert!(
                    decoded.contains("item"),
                    "first item should contain 'item', got: {:?}",
                    decoded
                );
            } else {
                panic!("expected string for first item, got: {:?}", items[0]);
            }

            // Second item is regular value
            if let YamlValue::String(s) = &items[1] {
                assert_eq!(&*s.as_str().unwrap(), "value");
            } else {
                panic!("expected string for second item");
            }
        } else {
            panic!("expected sequence, got: {:?}", root.value());
        }
    }
}
