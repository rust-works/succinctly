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

    /// Check if this cursor points to a structural container (mapping or sequence).
    ///
    /// Containers are nodes that have children AND have a valid TY bit.
    /// This distinguishes real containers from item wrappers and other BP nodes.
    /// Note: for empty containers (like `[]` or `{}`), the text-based check in
    /// `value()` handles them before calling `is_container()`.
    #[inline]
    pub fn is_container(&self) -> bool {
        if self.index.bp().first_child(self.bp_pos).is_none() {
            return false;
        }
        // Check if this position has a valid TY entry
        let ty_idx = self.index.bp().rank1(self.bp_pos);
        ty_idx < self.index.ty_len()
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

        // Check for alias first (before containers)
        let byte = self.text[text_pos];
        if byte == b'*' {
            return self.parse_alias_value(text_pos);
        }

        // Check for flow containers by looking at the text
        // (empty flow containers may not have children, so check text first)
        if byte == b'[' {
            // Flow sequence
            return YamlValue::Sequence(YamlElements::from_sequence_cursor(*self));
        }
        if byte == b'{' {
            // Flow mapping
            return YamlValue::Mapping(YamlFields::from_mapping_cursor(*self));
        }

        // Check for block-style sequence (starts with '- ')
        // Block sequences are nested containers that appear as values.
        // We need to distinguish:
        // - Sequence CONTAINER: Its children are item wrappers (also at '- ')
        // - Item WRAPPER: Its child is the actual value (not at '- ')
        //
        // A sequence container has first_child at '- ', while an item wrapper
        // has first_child at the actual value position.
        if byte == b'-' && text_pos + 1 < self.text.len() && self.text[text_pos + 1] == b' ' {
            if let Some(first_child) = self.first_child() {
                if let Some(child_text_pos) = first_child.text_position() {
                    // If the child also starts with '- ', this is a sequence container
                    if child_text_pos < self.text.len()
                        && self.text[child_text_pos] == b'-'
                        && child_text_pos + 1 < self.text.len()
                        && self.text[child_text_pos + 1] == b' '
                    {
                        return YamlValue::Sequence(YamlElements::from_sequence_cursor(*self));
                    }
                }
            }
        }

        // Check if this is a container with a TY bit (structural container)
        // This must come BEFORE the heuristic checks to ensure we use the authoritative
        // TY bits for containers that have them (like document sequences).
        //
        // Special case: the root (bp_pos=0) is always the virtual document sequence,
        // even if it's empty (no documents). Check it explicitly.
        if self.bp_pos == 0 || self.is_container() {
            // Determine if mapping or sequence using the TY bits
            // (This handles virtual containers like the document root wrapper)
            if self.index.is_sequence_at_bp(self.bp_pos) {
                return YamlValue::Sequence(YamlElements::from_sequence_cursor(*self));
            } else {
                return YamlValue::Mapping(YamlFields::from_mapping_cursor(*self));
            }
        }

        // Check for block-style mapping (content that looks like a key: value)
        // This heuristic is for nodes that don't have TY bits (like item wrappers).
        // A block mapping's first child is a key node, which has a sibling (the value).
        // Key nodes don't have BP children - they're just open/close pairs.
        // We detect a mapping by:
        // 1. Has a first_child (the key)
        // 2. That first_child has a next_sibling (the value)
        // 3. The text at first_child's position is not '-' (not a sequence)
        if let Some(first_child) = self.first_child() {
            if first_child.next_sibling().is_some() {
                // First child has a sibling - this could be a mapping (key, value)
                if let Some(first_child_text_pos) = first_child.text_position() {
                    if first_child_text_pos < self.text.len() {
                        let first_byte = self.text[first_child_text_pos];
                        // If first child text doesn't start with '-', this is a mapping key
                        if first_byte != b'-' {
                            return YamlValue::Mapping(YamlFields::from_mapping_cursor(*self));
                        }
                    }
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

    /// Parse an alias value from text position.
    fn parse_alias_value(&self, text_pos: usize) -> YamlValue<'a, W> {
        // Extract anchor name from text (skip the `*`)
        let start = text_pos + 1;
        let mut end = start;
        while end < self.text.len() {
            match self.text[end] {
                b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_' | b'-' => end += 1,
                _ => break,
            }
        }

        let anchor_name = match core::str::from_utf8(&self.text[start..end]) {
            Ok(s) => s,
            Err(_) => return YamlValue::Error("invalid UTF-8 in anchor name"),
        };

        // Try to resolve the alias to its target
        let target = self.index.resolve_alias(self.bp_pos, self.text);

        YamlValue::Alias {
            anchor_name,
            target,
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
                    // Colon followed by space, newline, or EOF ends the scalar
                    if end + 1 >= self.text.len() {
                        // Colon at EOF - this is a key separator
                        break;
                    }
                    if self.text[end + 1] == b' ' || self.text[end + 1] == b'\n' {
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
    /// An alias referencing an anchored value (`*anchor_name`)
    Alias {
        /// The anchor name being referenced
        anchor_name: &'a str,
        /// Cursor to the referenced value (if resolvable)
        target: Option<YamlCursor<'a, W>>,
    },
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
        // However, for flow sequences, virtual root sequences, and document containers,
        // the element IS the value directly.
        //
        // Block sequence items are detected by:
        // 1. Text starts with `-`
        // 2. The element is NOT itself a container (containers starting with `-` are sequences)
        //
        // If the element is a container (like a nested sequence or mapping), use it directly.
        let value_cursor = if element_cursor.is_container() {
            // This is a container (mapping or sequence) - use it directly
            element_cursor
        } else if let Some(text_pos) = element_cursor.text_position() {
            // Not a container - check if it's a block sequence item wrapper
            if text_pos < element_cursor.text.len()
                && element_cursor.text[text_pos] == b'-'
                && element_cursor.first_child().is_some()
            {
                // Block sequence item with content - unwrap to get the actual value
                element_cursor.first_child().unwrap()
            } else {
                // Scalar value or empty item
                element_cursor
            }
        } else {
            element_cursor
        };
        let value = value_cursor.value();
        Some((value, rest))
    }

    /// Get element by index.
    pub fn get(&self, index: usize) -> Option<YamlValue<'a, W>> {
        let mut cursor = self.element_cursor?;
        for _ in 0..index {
            cursor = cursor.next_sibling()?;
        }
        // Same logic as uncons - containers are used directly, block sequence items are unwrapped
        let value_cursor = if cursor.is_container() {
            cursor
        } else if let Some(text_pos) = cursor.text_position() {
            if text_pos < cursor.text.len()
                && cursor.text[text_pos] == b'-'
                && cursor.first_child().is_some()
            {
                cursor.first_child().unwrap()
            } else {
                cursor
            }
        } else {
            cursor
        };
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

    /// Helper to get the first document from the root document array.
    /// All YAML documents are wrapped in a virtual root sequence.
    fn first_doc<'a, W: AsRef<[u64]> + core::fmt::Debug>(
        root: YamlCursor<'a, W>,
    ) -> YamlValue<'a, W> {
        match root.value() {
            YamlValue::Sequence(elements) => elements
                .into_iter()
                .next()
                .expect("expected at least one document"),
            other => panic!(
                "expected root to be document array (sequence), got {:?}",
                other
            ),
        }
    }

    #[test]
    fn test_simple_mapping_navigation() {
        let yaml = b"name: Alice";
        let index = YamlIndex::build(yaml).unwrap();
        let root = index.root(yaml);

        // First document should be a mapping
        match first_doc(root) {
            YamlValue::Mapping(fields) => {
                assert!(!fields.is_empty());
            }
            other => panic!("expected mapping, got {:?}", other),
        }
    }

    #[test]
    fn test_double_quoted_string() {
        let yaml = b"name: \"Alice\"";
        let index = YamlIndex::build(yaml).unwrap();
        let root = index.root(yaml);

        // Root should be at position 0
        assert_eq!(root.text_position(), Some(0));

        // First document should be a mapping
        if let YamlValue::Mapping(fields) = first_doc(root) {
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

        if let YamlValue::Mapping(fields) = first_doc(root) {
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

        if let YamlValue::Mapping(fields) = first_doc(root) {
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

        // First document should be a mapping with "items" key
        if let YamlValue::Mapping(fields) = first_doc(root) {
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

        // First document should be a mapping with "person" key
        if let YamlValue::Mapping(fields) = first_doc(root) {
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

        if let YamlValue::Mapping(fields) = first_doc(root) {
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

        if let YamlValue::Mapping(fields) = first_doc(root) {
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

        if let YamlValue::Mapping(fields) = first_doc(root) {
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

        if let YamlValue::Mapping(fields) = first_doc(root) {
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

        if let YamlValue::Mapping(fields) = first_doc(root) {
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

        if let YamlValue::Sequence(elements) = first_doc(root_simple) {
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

        if let YamlValue::Sequence(elements) = first_doc(root) {
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
            panic!("expected sequence, got: {:?}", first_doc(root));
        }
    }

    // =========================================================================
    // Anchor and alias navigation tests (Phase 4)
    // =========================================================================

    #[test]
    fn test_anchor_and_alias_basic() {
        // Basic anchor and alias
        let yaml = b"anchor: &name value\nalias: *name";
        let index = YamlIndex::build(yaml).unwrap();
        let root = index.root(yaml);

        // Get the first document cursor
        let doc_cursor = root.first_child().expect("expected document");

        if let YamlValue::Mapping(fields) = doc_cursor.value() {
            // Check anchor value
            if let Some(YamlValue::String(s)) = fields.find("anchor") {
                assert_eq!(&*s.as_str().unwrap(), "value");
            } else {
                panic!("expected string for anchor");
            }

            // Check alias
            let fields = YamlFields::from_mapping_cursor(doc_cursor);
            if let Some((_field, _rest)) = fields.uncons() {
                // Skip first field (anchor)
                let fields = _rest;
                if let Some((field, _)) = fields.uncons() {
                    // Second field is alias
                    if let YamlValue::Alias {
                        anchor_name,
                        target,
                    } = field.value()
                    {
                        assert_eq!(anchor_name, "name");
                        // Target should resolve to the anchored value
                        assert!(target.is_some(), "alias should resolve to target");
                        if let Some(target_cursor) = target {
                            if let YamlValue::String(s) = target_cursor.value() {
                                assert_eq!(&*s.as_str().unwrap(), "value");
                            } else {
                                panic!("expected string for resolved alias");
                            }
                        }
                    } else {
                        panic!("expected alias for second field value");
                    }
                }
            }
        } else {
            panic!("expected mapping");
        }
    }

    #[test]
    fn test_anchor_on_flow_mapping() {
        // Use flow style mapping since block style nested mappings have a separate issue
        let yaml = b"defaults: &defaults {key: value}\nother: *defaults";
        let index = YamlIndex::build(yaml).unwrap();
        let root = index.root(yaml);

        // Get the first document cursor
        let doc_cursor = root.first_child().expect("expected document");

        if let YamlValue::Mapping(fields) = doc_cursor.value() {
            // Check defaults mapping (flow style)
            if let Some(YamlValue::Mapping(default_fields)) = fields.find("defaults") {
                if let Some(YamlValue::String(s)) = default_fields.find("key") {
                    assert_eq!(&*s.as_str().unwrap(), "value");
                } else {
                    panic!("expected key in defaults");
                }
            } else {
                panic!("expected mapping for defaults");
            }

            // Check other (alias)
            let fields = YamlFields::from_mapping_cursor(doc_cursor);
            for field in fields {
                if let YamlValue::String(key) = field.key() {
                    if key.as_str().unwrap() == "other" {
                        if let YamlValue::Alias {
                            anchor_name,
                            target,
                        } = field.value()
                        {
                            assert_eq!(anchor_name, "defaults");
                            assert!(target.is_some());
                        } else {
                            panic!("expected alias for 'other'");
                        }
                        return;
                    }
                }
            }
            panic!("did not find 'other' field");
        } else {
            panic!("expected mapping");
        }
    }

    #[test]
    fn test_anchor_in_flow_sequence() {
        let yaml = b"items: [&first one, &second two, *first]";
        let index = YamlIndex::build(yaml).unwrap();
        let root = index.root(yaml);

        if let YamlValue::Mapping(fields) = first_doc(root) {
            if let Some(YamlValue::Sequence(elements)) = fields.find("items") {
                let items: Vec<_> = elements.collect();
                assert_eq!(items.len(), 3, "expected 3 items");

                // First item has anchor
                if let YamlValue::String(s) = &items[0] {
                    assert_eq!(&*s.as_str().unwrap(), "one");
                } else {
                    panic!("expected string for first item");
                }

                // Second item has anchor
                if let YamlValue::String(s) = &items[1] {
                    assert_eq!(&*s.as_str().unwrap(), "two");
                } else {
                    panic!("expected string for second item");
                }

                // Third item is alias
                if let YamlValue::Alias {
                    anchor_name,
                    target,
                } = &items[2]
                {
                    assert_eq!(*anchor_name, "first");
                    assert!(target.is_some());
                } else {
                    panic!("expected alias for third item, got: {:?}", items[2]);
                }
            } else {
                panic!("expected sequence for items");
            }
        } else {
            panic!("expected mapping");
        }
    }

    #[test]
    fn test_anchor_in_flow_mapping() {
        let yaml = b"data: {name: &n Alice, greeting: *n}";
        let index = YamlIndex::build(yaml).unwrap();
        let root = index.root(yaml);

        // Get the first document cursor
        let doc_cursor = root.first_child().expect("expected document");

        if let YamlValue::Mapping(fields) = doc_cursor.value() {
            if let Some(YamlValue::Mapping(data_fields)) = fields.find("data") {
                // Check name (has anchor)
                if let Some(YamlValue::String(s)) = data_fields.find("name") {
                    assert_eq!(&*s.as_str().unwrap(), "Alice");
                } else {
                    panic!("expected string for name");
                }

                // Check greeting (alias)
                // Navigate: doc_cursor -> first_child (data key) -> next_sibling (data value)
                let data_fields = YamlFields::from_mapping_cursor(
                    doc_cursor.first_child().unwrap().next_sibling().unwrap(),
                );
                for field in data_fields {
                    if let YamlValue::String(key) = field.key() {
                        if key.as_str().unwrap() == "greeting" {
                            if let YamlValue::Alias {
                                anchor_name,
                                target,
                            } = field.value()
                            {
                                assert_eq!(anchor_name, "n");
                                assert!(target.is_some());
                                return;
                            } else {
                                panic!("expected alias for greeting");
                            }
                        }
                    }
                }
                panic!("did not find greeting field");
            } else {
                panic!("expected mapping for data");
            }
        } else {
            panic!("expected mapping");
        }
    }

    #[test]
    fn test_inline_anchor_in_sequence() {
        let yaml = b"- &item value\n- *item";
        let index = YamlIndex::build(yaml).unwrap();
        let root = index.root(yaml);

        if let YamlValue::Sequence(elements) = first_doc(root) {
            let items: Vec<_> = elements.collect();
            assert_eq!(items.len(), 2, "expected 2 items");

            // First item has anchor
            if let YamlValue::String(s) = &items[0] {
                assert_eq!(&*s.as_str().unwrap(), "value");
            } else {
                panic!("expected string for first item");
            }

            // Second item is alias
            if let YamlValue::Alias {
                anchor_name,
                target,
            } = &items[1]
            {
                assert_eq!(*anchor_name, "item");
                assert!(target.is_some());
            } else {
                panic!("expected alias for second item");
            }
        } else {
            panic!("expected sequence");
        }
    }

    #[test]
    fn test_multiple_anchors() {
        let yaml = b"a: &x 1\nb: &y 2\nc: [*x, *y]";
        let index = YamlIndex::build(yaml).unwrap();
        let root = index.root(yaml);

        if let YamlValue::Mapping(fields) = first_doc(root) {
            // Check a
            if let Some(YamlValue::String(s)) = fields.find("a") {
                assert_eq!(&*s.as_str().unwrap(), "1");
            }
            // Check b
            if let Some(YamlValue::String(s)) = fields.find("b") {
                assert_eq!(&*s.as_str().unwrap(), "2");
            }
            // Check c (sequence with aliases)
            if let Some(YamlValue::Sequence(elements)) = fields.find("c") {
                let items: Vec<_> = elements.collect();
                assert_eq!(items.len(), 2);

                if let YamlValue::Alias { anchor_name, .. } = &items[0] {
                    assert_eq!(*anchor_name, "x");
                } else {
                    panic!("expected alias for first element");
                }

                if let YamlValue::Alias { anchor_name, .. } = &items[1] {
                    assert_eq!(*anchor_name, "y");
                } else {
                    panic!("expected alias for second element");
                }
            } else {
                panic!("expected sequence for c");
            }
        } else {
            panic!("expected mapping");
        }
    }

    #[test]
    fn test_undefined_alias() {
        // Alias to undefined anchor - should still parse, but target is None
        let yaml = b"bad: *undefined";
        let index = YamlIndex::build(yaml).unwrap();
        let root = index.root(yaml);

        if let YamlValue::Mapping(fields) = first_doc(root) {
            for field in fields {
                if let YamlValue::String(key) = field.key() {
                    if key.as_str().unwrap() == "bad" {
                        if let YamlValue::Alias {
                            anchor_name,
                            target,
                        } = field.value()
                        {
                            assert_eq!(anchor_name, "undefined");
                            // Target should be None because anchor doesn't exist
                            assert!(target.is_none(), "undefined alias should not resolve");
                            return;
                        } else {
                            panic!("expected alias for bad");
                        }
                    }
                }
            }
            panic!("did not find bad field");
        } else {
            panic!("expected mapping");
        }
    }

    #[test]
    fn test_block_nested_sequence() {
        // Block-style nested sequence (value on next line)
        let yaml = b"items:\n  - one\n  - two";
        let index = YamlIndex::build(yaml).unwrap();
        let root = index.root(yaml);

        if let YamlValue::Mapping(fields) = first_doc(root) {
            for field in fields {
                let key = field.key();
                let val = field.value();

                if let YamlValue::String(k) = key {
                    if k.as_str().unwrap() == "items" {
                        if let YamlValue::Sequence(elements) = val {
                            let items: Vec<_> = elements.collect();
                            assert_eq!(items.len(), 2, "expected 2 items, got: {:?}", items);
                            if let YamlValue::String(s) = &items[0] {
                                assert_eq!(&*s.as_str().unwrap(), "one");
                            }
                            return;
                        } else {
                            panic!("expected sequence for items, got: {:?}", val);
                        }
                    }
                }
            }
            panic!("did not find items field");
        } else {
            panic!("expected mapping, got: {:?}", first_doc(root));
        }
    }

    #[test]
    fn test_block_nested_mapping() {
        // Block-style nested mapping (value on next line)
        let yaml = b"person:\n  name: Alice\n  age: 30";
        let index = YamlIndex::build(yaml).unwrap();
        let root = index.root(yaml);

        if let YamlValue::Mapping(fields) = first_doc(root) {
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
    fn test_multiple_toplevel_nested_mappings() {
        // Multiple top-level keys, each with nested block-style mappings
        let yaml = b"server:\n  host: localhost\ndatabase:\n  host: db.example.com";
        let index = YamlIndex::build(yaml).unwrap();
        let root = index.root(yaml);

        // Get the first document (should be a mapping)
        if let YamlValue::Mapping(fields) = first_doc(root) {
            let all_fields: Vec<_> = fields.into_iter().collect();
            assert_eq!(
                all_fields.len(),
                2,
                "expected 2 fields (server, database), got {} fields",
                all_fields.len()
            );

            // Check field keys
            if let YamlValue::String(k) = all_fields[0].key() {
                assert_eq!(&*k.as_str().unwrap(), "server");
            } else {
                panic!("expected string key for first field");
            }

            if let YamlValue::String(k) = all_fields[1].key() {
                assert_eq!(&*k.as_str().unwrap(), "database");
            } else {
                panic!("expected string key for second field");
            }

            // Check that values are nested mappings
            if let YamlValue::Mapping(server_fields) = all_fields[0].value() {
                if let Some(YamlValue::String(s)) = server_fields.find("host") {
                    assert_eq!(&*s.as_str().unwrap(), "localhost");
                } else {
                    panic!("expected host field in server");
                }
            } else {
                panic!("expected mapping for server value");
            }

            if let YamlValue::Mapping(db_fields) = all_fields[1].value() {
                if let Some(YamlValue::String(s)) = db_fields.find("host") {
                    assert_eq!(&*s.as_str().unwrap(), "db.example.com");
                } else {
                    panic!("expected host field in database");
                }
            } else {
                panic!("expected mapping for database value");
            }
        } else {
            panic!("expected mapping");
        }
    }
}
