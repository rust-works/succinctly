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
        // Use the containers bitvector to check if this BP position has a TY entry.
        // Containers are mappings and sequences; sequence items and scalars are not.
        self.index.is_container(self.bp_pos)
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
        // Special case: the root (bp_pos=0) is always the virtual document sequence,
        // even if it's empty (no documents). Check it explicitly FIRST before
        // looking at text bytes, since the root's text_position may point to the
        // first document's content (like a flow mapping starting with '{').
        if self.bp_pos == 0 {
            // Root is always a sequence (of documents)
            return YamlValue::Sequence(YamlElements::from_sequence_cursor(*self));
        }

        // Special case: sequence item wrappers delegate to their first child's value.
        // A sequence item is a thin wrapper around the actual content:
        //   - [item1, item2]  <- sequence item wrappers contain the actual values
        // The item wrapper's first_child IS the value (could be scalar, mapping, etc.)
        if self.index.is_seq_item(self.bp_pos) {
            if let Some(child) = self.first_child() {
                return child.value();
            }
            // Empty sequence item (null)
            return YamlValue::Null;
        }

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

        // Check for anchor - if present, skip past it to get the actual value
        // Anchors look like: &anchor_name value
        let effective_text_pos = if byte == b'&' {
            self.skip_anchor_and_whitespace(text_pos)
        } else {
            text_pos
        };

        // If we're past the end after skipping anchor, this is an anchored null
        if effective_text_pos >= self.text.len() {
            return YamlValue::Null;
        }

        let byte = self.text[effective_text_pos];

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
        if byte == b'-'
            && effective_text_pos + 1 < self.text.len()
            && self.text[effective_text_pos + 1] == b' '
        {
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
        // This uses the authoritative TY bits for containers that have them.
        if self.is_container() {
            // Determine if mapping or sequence using the TY bits
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
        match self.text[effective_text_pos] {
            b'"' => YamlValue::String(YamlString::DoubleQuoted {
                text: self.text,
                start: effective_text_pos,
            }),
            b'\'' => YamlValue::String(YamlString::SingleQuoted {
                text: self.text,
                start: effective_text_pos,
            }),
            b'|' => {
                // Block literal scalar
                let (chomping, explicit_indent) = self.parse_block_header(effective_text_pos);
                YamlValue::String(YamlString::BlockLiteral {
                    text: self.text,
                    indicator_pos: effective_text_pos,
                    chomping,
                    explicit_indent,
                })
            }
            b'>' => {
                // Block folded scalar
                let (chomping, explicit_indent) = self.parse_block_header(effective_text_pos);
                YamlValue::String(YamlString::BlockFolded {
                    text: self.text,
                    indicator_pos: effective_text_pos,
                    chomping,
                    explicit_indent,
                })
            }
            _ => {
                // Unquoted (plain) scalar - may span multiple lines
                let base_indent = self.compute_base_indent_for_plain(effective_text_pos);
                let is_doc_root = self.is_document_root_scalar(effective_text_pos);
                let end = self.find_plain_scalar_end(effective_text_pos, base_indent, is_doc_root);
                YamlValue::String(YamlString::Unquoted {
                    text: self.text,
                    start: effective_text_pos,
                    end,
                    base_indent,
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

    /// Skip past an anchor definition and any following whitespace.
    ///
    /// Anchor syntax: `&name` where name can contain alphanumerics, underscores,
    /// hyphens, and colons.
    ///
    /// Returns the position of the first non-whitespace character after the anchor,
    /// or the end of text if nothing follows.
    fn skip_anchor_and_whitespace(&self, start: usize) -> usize {
        // Skip the '&'
        let mut pos = start + 1;

        // Skip anchor name characters (per YAML spec, anchors can contain
        // alphanumerics, underscores, hyphens, and also colons in some contexts)
        while pos < self.text.len() {
            match self.text[pos] {
                b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_' | b'-' | b':' => pos += 1,
                _ => break,
            }
        }

        // Skip whitespace after anchor name
        while pos < self.text.len() && (self.text[pos] == b' ' || self.text[pos] == b'\t') {
            pos += 1;
        }

        pos
    }

    /// Parse block scalar header (chomping and explicit indentation indicators).
    /// Returns (ChompingIndicator, Option<explicit_indent>).
    fn parse_block_header(&self, indicator_pos: usize) -> (ChompingIndicator, Option<u8>) {
        let mut pos = indicator_pos + 1;
        let mut chomping = ChompingIndicator::Clip;
        let mut explicit_indent = None;

        // Check next 2 characters for chomping and indent indicators
        for _ in 0..2 {
            if pos >= self.text.len() {
                break;
            }
            match self.text[pos] {
                b'-' => {
                    chomping = ChompingIndicator::Strip;
                    pos += 1;
                }
                b'+' => {
                    chomping = ChompingIndicator::Keep;
                    pos += 1;
                }
                b'1'..=b'9' => {
                    explicit_indent = Some(self.text[pos] - b'0');
                    pos += 1;
                }
                _ => break,
            }
        }
        (chomping, explicit_indent)
    }

    fn compute_line_indent_static(text: &[u8], pos: usize) -> usize {
        // Find start of line
        let mut line_start = pos;
        while line_start > 0 && text[line_start - 1] != b'\n' {
            line_start -= 1;
        }

        // Count spaces at start of line (tabs don't count as indentation in YAML)
        let mut indent = 0;
        while line_start + indent < text.len() && text[line_start + indent] == b' ' {
            indent += 1;
        }
        indent
    }

    /// Compute the base indent for plain scalar continuation.
    /// For values on their own line (after key:), this returns the key's indent.
    /// For values on the same line as the key, this returns that line's indent.
    /// Compute base indent for plain scalar continuation checking.
    /// Returns (base_indent, is_document_root) where is_document_root is true
    /// if this scalar is the document root content (right after --- or at start).
    fn compute_base_indent_for_plain(&self, value_pos: usize) -> usize {
        let (indent, _is_root) = self.compute_base_indent_and_root_flag(value_pos);
        indent
    }

    /// Check if value is at document root level (right after --- or at document start).
    fn is_document_root_scalar(&self, value_pos: usize) -> bool {
        let (_indent, is_root) = self.compute_base_indent_and_root_flag(value_pos);
        is_root
    }

    fn compute_base_indent_and_root_flag(&self, value_pos: usize) -> (usize, bool) {
        // Find start of current line
        let mut line_start = value_pos;
        while line_start > 0 && self.text[line_start - 1] != b'\n' {
            line_start -= 1;
        }

        // Check if there's a colon before us on this line (meaning key: value on same line)
        let mut has_colon_before = false;
        let mut colon_pos = 0;
        let mut scan = line_start;
        while scan < value_pos {
            if self.text[scan] == b':' {
                // Check if this colon is a key separator (followed by space/tab/newline)
                if scan + 1 < self.text.len() {
                    let next = self.text[scan + 1];
                    if next == b' ' || next == b'\t' || next == b'\n' {
                        has_colon_before = true;
                        colon_pos = scan;
                    }
                }
            }
            scan += 1;
        }

        if has_colon_before {
            // Value is on same line as key - not a document root
            // Find the effective content indent, which is the column of the key.
            // For compact mappings in sequences like "- key: value", we need to
            // find where the actual content starts (after any "- " indicators).

            // Scan backwards from the colon to find the key start
            let mut key_start = colon_pos;
            while key_start > line_start {
                let prev = self.text[key_start - 1];
                if prev == b' ' || prev == b'\t' {
                    break;
                }
                // Skip over quoted key content
                if prev == b'"' || prev == b'\'' {
                    // This is end of a quoted key - harder to handle, just use line indent
                    return (
                        Self::compute_line_indent_static(self.text, value_pos),
                        false,
                    );
                }
                key_start -= 1;
            }

            // key_start is now at the start of the key
            // The base indent is the column position of the key
            (key_start - line_start, false)
        } else {
            // Value is on its own line
            // Check if previous line is a document marker (---) or if we're at start
            if line_start == 0 {
                // No previous line, this is document root
                return (0, true);
            }

            // Go to previous line
            let mut prev_line_start = line_start - 1;
            while prev_line_start > 0 && self.text[prev_line_start - 1] != b'\n' {
                prev_line_start -= 1;
            }

            // Check if previous line is "---" (document start marker)
            let prev_line = &self.text[prev_line_start..line_start.saturating_sub(1)];
            let trimmed = prev_line
                .iter()
                .copied()
                .skip_while(|&b| b == b' ')
                .collect::<alloc::vec::Vec<u8>>();

            if trimmed.starts_with(b"---") {
                let rest = &trimmed[3..];
                if rest.is_empty() || rest.iter().all(|&b| b == b' ' || b == b'\t') {
                    // Previous line is just "---" (possibly with trailing whitespace)
                    return (0, true);
                }
            }

            // Not at document root - return previous line's indent
            (
                Self::compute_line_indent_static(self.text, prev_line_start),
                false,
            )
        }
    }

    /// Check if a position is inside a flow context (inside `[]` or `{}`).
    /// Returns true if there's an unmatched `[` or `{` before the position.
    fn is_in_flow_context(&self, pos: usize) -> bool {
        let mut depth = 0i32;
        let mut in_double_quote = false;
        let mut in_single_quote = false;
        let mut i = 0;

        while i < pos {
            let byte = self.text[i];

            if in_double_quote {
                if byte == b'\\' && i + 1 < pos {
                    i += 2; // Skip escaped character
                    continue;
                }
                if byte == b'"' {
                    in_double_quote = false;
                }
            } else if in_single_quote {
                if byte == b'\'' {
                    // Check for escaped single quote ''
                    if i + 1 < pos && self.text[i + 1] == b'\'' {
                        i += 2;
                        continue;
                    }
                    in_single_quote = false;
                }
            } else {
                match byte {
                    b'"' => in_double_quote = true,
                    b'\'' => in_single_quote = true,
                    b'[' | b'{' => depth += 1,
                    b']' | b'}' => depth -= 1,
                    _ => {}
                }
            }
            i += 1;
        }

        depth > 0
    }

    /// Find the end of a single-line unquoted scalar (stops at newline).
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

    /// Find the end of a plain (unquoted) scalar, including continuation lines.
    /// A continuation line is more indented than base_indent (in block context),
    /// or any line that doesn't start with a flow delimiter (in flow context).
    /// For document root scalars (is_doc_root=true), continuation at indent 0 is allowed.
    fn find_plain_scalar_end(&self, start: usize, base_indent: usize, is_doc_root: bool) -> usize {
        let in_flow = self.is_in_flow_context(start);
        let mut end = start;

        loop {
            // Find end of current line content
            while end < self.text.len() {
                match self.text[end] {
                    b'\n' => break,
                    b'#' => {
                        // # preceded by whitespace (space or tab) is a comment
                        if end > start && matches!(self.text[end - 1], b' ' | b'\t') {
                            break;
                        }
                        end += 1;
                    }
                    // Flow context delimiters - only terminate in flow context
                    b',' | b']' | b'}' if in_flow => break,
                    b':' => {
                        // Colon followed by space, newline, or EOF ends the scalar
                        if end + 1 >= self.text.len() {
                            break;
                        }
                        if self.text[end + 1] == b' '
                            || self.text[end + 1] == b'\n'
                            || self.text[end + 1] == b'\t'
                        {
                            break;
                        }
                        end += 1;
                    }
                    _ => end += 1,
                }
            }

            // Trim trailing whitespace (spaces and tabs) from current line
            let mut line_end = end;
            while line_end > start && matches!(self.text[line_end - 1], b' ' | b'\t') {
                line_end -= 1;
            }

            // Check if there's a continuation line
            if end >= self.text.len() || self.text[end] != b'\n' {
                // No newline, end of text or hit delimiter
                return line_end;
            }

            // Look ahead to see if next line is a continuation
            let newline_pos = end;
            end += 1; // Skip newline

            // Skip empty lines (they can be part of multiline scalar)
            let mut empty_lines_end = end;
            while empty_lines_end < self.text.len() {
                // Count indentation of this line
                let mut line_indent = 0;
                while empty_lines_end + line_indent < self.text.len()
                    && self.text[empty_lines_end + line_indent] == b' '
                {
                    line_indent += 1;
                }

                // Check what's after the indentation
                let after_indent = empty_lines_end + line_indent;
                if after_indent >= self.text.len() {
                    // EOF after spaces
                    return line_end;
                }

                match self.text[after_indent] {
                    b'\n' => {
                        // Empty line - continue looking
                        empty_lines_end = after_indent + 1;
                    }
                    b'\r' => {
                        // Handle CRLF
                        empty_lines_end = after_indent + 1;
                        if empty_lines_end < self.text.len() && self.text[empty_lines_end] == b'\n'
                        {
                            empty_lines_end += 1;
                        }
                    }
                    b'\t' => {
                        // Tabs after spaces - check if rest of line is whitespace only
                        // If so, this is an empty line for folding purposes
                        let mut check_pos = after_indent;
                        while check_pos < self.text.len()
                            && matches!(self.text[check_pos], b'\t' | b' ')
                        {
                            check_pos += 1;
                        }
                        if check_pos >= self.text.len()
                            || matches!(self.text[check_pos], b'\n' | b'\r')
                        {
                            // Line has only whitespace - treat as empty line, continue looking
                            empty_lines_end = check_pos;
                            if check_pos < self.text.len() && self.text[check_pos] == b'\r' {
                                empty_lines_end += 1;
                                if empty_lines_end < self.text.len()
                                    && self.text[empty_lines_end] == b'\n'
                                {
                                    empty_lines_end += 1;
                                }
                            } else if check_pos < self.text.len() && self.text[check_pos] == b'\n' {
                                empty_lines_end += 1;
                            }
                            // Continue the loop to check the next line
                        } else if in_flow || line_indent > base_indent {
                            // Tab followed by content, with sufficient indent - this is a continuation
                            end = empty_lines_end;
                            break;
                        } else {
                            // Tab after indent, followed by content, not enough indent - not a continuation
                            return line_end;
                        }
                    }
                    b'#' => {
                        // Comment line - not a continuation
                        return line_end;
                    }
                    b'-' => {
                        // Check for document start marker "---" at column 0
                        if line_indent == 0
                            && after_indent + 2 < self.text.len()
                            && self.text[after_indent + 1] == b'-'
                            && self.text[after_indent + 2] == b'-'
                        {
                            // Check what follows the "---"
                            let after_marker = after_indent + 3;
                            if after_marker >= self.text.len()
                                || matches!(self.text[after_marker], b' ' | b'\t' | b'\n' | b'\r')
                            {
                                // This is a document start marker - not a continuation
                                return line_end;
                            }
                            // "---" followed by content is a plain scalar continuation
                            // (like "---word1")
                        }
                        // Possible sequence indicator
                        if after_indent + 1 < self.text.len()
                            && (self.text[after_indent + 1] == b' '
                                || self.text[after_indent + 1] == b'\n')
                        {
                            // This is a sequence item - not a continuation
                            return line_end;
                        }
                        // Not a sequence indicator (dash not followed by space/newline)
                        // Check if it's a continuation based on indentation or flow context
                        // For document root scalars, allow same-level continuation at indent 0
                        if in_flow || line_indent > base_indent || (is_doc_root && line_indent == 0)
                        {
                            end = empty_lines_end;
                            break;
                        }
                        return line_end;
                    }
                    // Flow delimiters terminate the scalar regardless of indentation
                    b']' | b'}' | b',' => {
                        return line_end;
                    }
                    b'.' => {
                        // Check for document end marker "..." at column 0
                        if line_indent == 0
                            && after_indent + 2 < self.text.len()
                            && self.text[after_indent + 1] == b'.'
                            && self.text[after_indent + 2] == b'.'
                        {
                            // Check what follows the "..."
                            let after_marker = after_indent + 3;
                            if after_marker >= self.text.len()
                                || matches!(self.text[after_marker], b' ' | b'\t' | b'\n' | b'\r')
                            {
                                // This is a document end marker - not a continuation
                                return line_end;
                            }
                        }
                        // Not a document marker, check if it's a continuation
                        if in_flow || line_indent > base_indent || (is_doc_root && line_indent == 0)
                        {
                            end = empty_lines_end;
                            break;
                        }
                        return line_end;
                    }
                    _ => {
                        // Non-empty line - check indentation or flow context
                        // For document root scalars, allow same-level continuation at indent 0
                        if in_flow || line_indent > base_indent || (is_doc_root && line_indent == 0)
                        {
                            // Continuation line (in flow context, any non-delimiter continues)
                            end = empty_lines_end;
                            break;
                        } else {
                            // Not indented enough - scalar ends before this
                            return line_end;
                        }
                    }
                }
            }

            // If we fell through the empty lines loop without finding continuation
            if empty_lines_end >= self.text.len() {
                return line_end;
            }

            // Continue to include this continuation line
            // The newline and any empty lines become part of the scalar
            let _ = newline_pos; // newline_pos marks where content ended
        }
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
    /// A YAML null value (empty entry)
    Null,
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
        let value = if element_cursor.is_container() {
            // This is a container (mapping or sequence) - use it directly
            element_cursor.value()
        } else if let Some(text_pos) = element_cursor.text_position() {
            // Not a container - check if it's a block sequence item wrapper
            if text_pos < element_cursor.text.len()
                && element_cursor.text[text_pos] == b'-'
                && text_pos + 1 < element_cursor.text.len()
                && element_cursor.text[text_pos + 1] == b' '
            {
                // This is a block sequence item marker `- `
                if let Some(child) = element_cursor.first_child() {
                    // Block sequence item with content - unwrap to get the actual value
                    child.value()
                } else {
                    // Empty sequence item (like `- # comment` or `- ` with nothing)
                    // This should be null
                    YamlValue::Null
                }
            } else {
                // Scalar value
                element_cursor.value()
            }
        } else {
            element_cursor.value()
        };
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
    /// Unquoted (plain) string - may span multiple lines
    Unquoted {
        text: &'a [u8],
        start: usize,
        end: usize,
        /// Base indentation level for detecting continuation lines
        base_indent: usize,
    },
    /// Block literal scalar (`|`): preserves newlines
    BlockLiteral {
        text: &'a [u8],
        indicator_pos: usize,
        chomping: ChompingIndicator,
        /// Explicit indentation indicator (1-9), or None for auto-detect
        explicit_indent: Option<u8>,
    },
    /// Block folded scalar (`>`): folds newlines to spaces
    BlockFolded {
        text: &'a [u8],
        indicator_pos: usize,
        chomping: ChompingIndicator,
        /// Explicit indentation indicator (1-9), or None for auto-detect
        explicit_indent: Option<u8>,
    },
}

impl<'a> YamlString<'a> {
    /// Returns true if this is an unquoted (plain) scalar.
    /// Unquoted scalars like `null`, `~`, or empty values should be treated
    /// as YAML null, while quoted or block scalars should remain strings.
    pub fn is_unquoted(&self) -> bool {
        matches!(self, YamlString::Unquoted { .. })
    }

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
            YamlString::Unquoted {
                text, start, end, ..
            } => &text[*start..*end],
            YamlString::BlockLiteral {
                text,
                indicator_pos,
                chomping,
                explicit_indent,
            }
            | YamlString::BlockFolded {
                text,
                indicator_pos,
                chomping,
                explicit_indent,
            } => {
                let (_, content_end) = Self::find_block_content_range(
                    text,
                    *indicator_pos,
                    *chomping,
                    *explicit_indent,
                );
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
                                                        // Need decoding if contains escapes or newlines (multiline folding)
                if !bytes.contains(&b'\\') && !bytes.contains(&b'\n') && !bytes.contains(&b'\r') {
                    let s =
                        core::str::from_utf8(bytes).map_err(|_| YamlStringError::InvalidUtf8)?;
                    Ok(Cow::Borrowed(s))
                } else {
                    decode_double_quoted(bytes).map(Cow::Owned)
                }
            }
            YamlString::SingleQuoted { text, start } => {
                let end = Self::find_single_quote_end(text, *start);
                let bytes = &text[*start + 1..end - 1]; // Strip quotes
                                                        // Need decoding if contains escaped quotes or newlines (multiline folding)
                if !bytes.contains(&b'\'') && !bytes.contains(&b'\n') && !bytes.contains(&b'\r') {
                    let s =
                        core::str::from_utf8(bytes).map_err(|_| YamlStringError::InvalidUtf8)?;
                    Ok(Cow::Borrowed(s))
                } else {
                    decode_single_quoted(bytes).map(Cow::Owned)
                }
            }
            YamlString::Unquoted {
                text,
                start,
                end,
                base_indent,
            } => {
                let bytes = &text[*start..*end];
                // Need folding if contains newlines (multiline plain scalar)
                if !bytes.contains(&b'\n') && !bytes.contains(&b'\r') {
                    let s =
                        core::str::from_utf8(bytes).map_err(|_| YamlStringError::InvalidUtf8)?;
                    Ok(Cow::Borrowed(s))
                } else {
                    decode_plain_scalar(bytes, *base_indent).map(Cow::Owned)
                }
            }
            YamlString::BlockLiteral {
                text,
                indicator_pos,
                chomping,
                explicit_indent,
            } => decode_block_literal(text, *indicator_pos, *chomping, *explicit_indent),
            YamlString::BlockFolded {
                text,
                indicator_pos,
                chomping,
                explicit_indent,
            } => decode_block_folded(text, *indicator_pos, *chomping, *explicit_indent),
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
        explicit_indent: Option<u8>,
    ) -> (usize, usize) {
        // Compute the base indent for block scalar termination.
        // This is the indent of the mapping key (for `key: |`) or the line indent.
        // For `- key: |`, the key is at indent 2 (after `- `), not 0.
        let base_indent = Self::compute_key_indent(text, indicator_pos);

        // Check if we're on a document-start line (--- or %directive)
        // In this case, content can be at indent 0 (zero-indented block scalar)
        let on_document_start_line = Self::is_document_start_line(text, indicator_pos);

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

        // Determine content indentation:
        // - If explicit_indent is specified (e.g., >2), use base_indent + explicit_indent
        // - Otherwise, auto-detect from first non-empty line
        let content_indent = if let Some(indent) = explicit_indent {
            // Explicit indentation: content is at base_indent + N spaces
            base_indent + indent as usize
        } else {
            // Auto-detect from first non-empty line
            // For zero-indented block scalars (after ---), content at indent 0 is valid
            match Self::detect_block_indent(text, pos) {
                Some(indent) if indent > base_indent || (on_document_start_line && indent == 0) => {
                    indent
                }
                Some(_) | None => {
                    // Content is not more indented than indicator line = empty block scalar
                    // Per YAML spec: for keep chomping on empty block scalar, content is "\n"
                    // We need to find any trailing newlines starting from content_start
                    if chomping == ChompingIndicator::Keep {
                        // Find all trailing newlines/empty lines from content_start
                        let mut end = content_start;
                        while end < text.len() {
                            // Count spaces
                            let mut spaces = 0;
                            while end + spaces < text.len() && text[end + spaces] == b' ' {
                                spaces += 1;
                            }
                            // Check if it's an empty line or EOF
                            if end + spaces >= text.len() {
                                break;
                            }
                            match text[end + spaces] {
                                b'\n' => {
                                    end += spaces + 1;
                                }
                                b'\r' => {
                                    end += spaces + 1;
                                    if end < text.len() && text[end] == b'\n' {
                                        end += 1;
                                    }
                                }
                                _ => {
                                    // Non-empty line at same or lower indent = end of block
                                    break;
                                }
                            }
                        }
                        // If no trailing newlines found but we're at EOF, the content
                        // area includes the newline that terminated the indicator line.
                        // Check if there was a newline before content_start.
                        if end == content_start && content_start > 0 {
                            let prev = content_start - 1;
                            if text[prev] == b'\n'
                                || (text[prev] == b'\r'
                                    && (content_start >= text.len()
                                        || text[content_start] != b'\n'))
                            {
                                // The indicator line ended with a newline - include it
                                // by adjusting content_start back to include that newline
                                return (prev, content_start);
                            }
                        }
                        return (content_start, end);
                    } else {
                        return (content_start, content_start);
                    }
                }
            }
        };

        // Find end of block scalar content
        let mut last_content_end = pos;
        let mut has_content = false;

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
                    // Empty line - include in content area
                    pos += 1;
                }
                b'\r' => {
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
                    has_content = true;

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
                // Clip: Include exactly one trailing newline if there was content
                if has_content {
                    // Include one newline after last content
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
            ChompingIndicator::Keep => pos, // Include all trailing newlines
        };

        (content_start, content_end)
    }

    /// Compute the indentation of the mapping key associated with a block scalar.
    /// For `key: |`, returns the indent of `key`.
    /// For `- key: |`, returns indent 2 (after `- `), not 0.
    /// For `- |` (direct block scalar in sequence), returns 0.
    fn compute_key_indent(text: &[u8], indicator_pos: usize) -> usize {
        // Find start of line
        let mut line_start = indicator_pos;
        while line_start > 0 && text[line_start - 1] != b'\n' {
            line_start -= 1;
        }

        // Scan forward to find the key's indent
        // Skip leading spaces
        let mut pos = line_start;
        while pos < text.len() && text[pos] == b' ' {
            pos += 1;
        }

        let line_indent = pos - line_start;

        // Check if we start with `-` (sequence item indicator)
        if pos < text.len() && text[pos] == b'-' {
            // Check if followed by space (block sequence indicator)
            if pos + 1 < text.len() && (text[pos + 1] == b' ' || text[pos + 1] == b'\n') {
                // Check if there's a `:` between `-` and the indicator
                // If so, it's `- key: |` and we should return line_indent + 2
                // If not, it's `- |` and we should return line_indent
                let has_colon = text[(pos + 2)..indicator_pos].contains(&b':');
                if has_colon {
                    return line_indent + 2;
                } else {
                    return line_indent;
                }
            }
        }

        // Otherwise, key indent is the line's leading spaces
        line_indent
    }

    /// Check if the given position is on a document-start line (begins with ---).
    /// This allows zero-indented block scalars per YAML spec.
    fn is_document_start_line(text: &[u8], pos: usize) -> bool {
        // Find start of line
        let mut line_start = pos;
        while line_start > 0 && text[line_start - 1] != b'\n' {
            line_start -= 1;
        }

        // Check if line starts with "---" (optionally preceded by spaces)
        let mut check_pos = line_start;
        while check_pos < text.len() && text[check_pos] == b' ' {
            check_pos += 1;
        }

        // Check for "---"
        if check_pos + 2 < text.len()
            && text[check_pos] == b'-'
            && text[check_pos + 1] == b'-'
            && text[check_pos + 2] == b'-'
        {
            return true;
        }

        false
    }

    /// Detect content indentation from first non-empty line.
    /// Returns None if no content lines found, Some(indent) otherwise.
    /// Note: indent can be 0 for content at column 0.
    fn detect_block_indent(text: &[u8], start: usize) -> Option<usize> {
        let mut pos = start;

        loop {
            if pos >= text.len() {
                return None;
            }

            // Count spaces
            let mut indent = 0;
            while pos < text.len() && text[pos] == b' ' {
                indent += 1;
                pos += 1;
            }

            if pos >= text.len() {
                return None;
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
                // Note: '#' is NOT treated as a comment here because this function
                // is used for block scalar content detection, where '#' is content.
                _ => {
                    return Some(indent);
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

/// Decode a double-quoted YAML string, handling escapes and line folding.
///
/// Line folding rules for double-quoted strings:
/// - A single line break becomes a space (unless escaped with \)
/// - Multiple consecutive line breaks: first becomes space, rest become \n
/// - Leading whitespace on continuation lines is trimmed
/// - `\` at end of line escapes the line break entirely (no space added)
fn decode_double_quoted(bytes: &[u8]) -> Result<String, YamlStringError> {
    let mut result = String::with_capacity(bytes.len());
    let mut i = 0;

    while i < bytes.len() {
        match bytes[i] {
            b'\\' => {
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
                    b'\n' => {
                        // Escaped line break - skip it entirely (no space added)
                        // Also skip leading whitespace on next line
                        i += 1;
                        while i < bytes.len() && matches!(bytes[i], b' ' | b'\t') {
                            i += 1;
                        }
                        continue; // Don't increment i again at end of loop
                    }
                    b'\r' => {
                        // Escaped line break (CRLF variant)
                        i += 1;
                        if i < bytes.len() && bytes[i] == b'\n' {
                            i += 1;
                        }
                        while i < bytes.len() && matches!(bytes[i], b' ' | b'\t') {
                            i += 1;
                        }
                        continue;
                    }
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
                        result
                            .push(char::from_u32(codepoint).ok_or(YamlStringError::InvalidEscape)?);
                        i += 4;
                    }
                    b'U' => {
                        // \UNNNNNNNN - 8 hex digits
                        if i + 8 >= bytes.len() {
                            return Err(YamlStringError::InvalidEscape);
                        }
                        let hex = &bytes[i + 1..i + 9];
                        let codepoint = parse_hex(hex)?;
                        result
                            .push(char::from_u32(codepoint).ok_or(YamlStringError::InvalidEscape)?);
                        i += 8;
                    }
                    _ => return Err(YamlStringError::InvalidEscape),
                }
                i += 1;
            }
            b'\r' | b'\n' => {
                // Line folding: handle newlines
                i = fold_quoted_line_break(bytes, i, &mut result);
            }
            _ => {
                // Regular content - copy until we hit escape or newline
                let start = i;
                while i < bytes.len() && !matches!(bytes[i], b'\\' | b'\n' | b'\r') {
                    i += 1;
                }
                let chunk = core::str::from_utf8(&bytes[start..i])
                    .map_err(|_| YamlStringError::InvalidUtf8)?;
                result.push_str(chunk);
            }
        }
    }

    Ok(result)
}

/// Decode a single-quoted YAML string, handling '' escapes and line folding.
///
/// Line folding rules for single-quoted strings:
/// - A single line break becomes a space
/// - Multiple consecutive line breaks: first becomes space, rest become \n
/// - Leading whitespace on continuation lines is trimmed
fn decode_single_quoted(bytes: &[u8]) -> Result<String, YamlStringError> {
    let mut result = String::with_capacity(bytes.len());
    let mut i = 0;

    while i < bytes.len() {
        match bytes[i] {
            b'\'' if i + 1 < bytes.len() && bytes[i + 1] == b'\'' => {
                // '' -> '
                result.push('\'');
                i += 2;
            }
            b'\r' | b'\n' => {
                // Line folding
                i = fold_quoted_line_break(bytes, i, &mut result);
            }
            _ => {
                // Regular content
                let start = i;
                while i < bytes.len()
                    && !matches!(bytes[i], b'\n' | b'\r')
                    && !(bytes[i] == b'\'' && i + 1 < bytes.len() && bytes[i + 1] == b'\'')
                {
                    i += 1;
                }
                let chunk = core::str::from_utf8(&bytes[start..i])
                    .map_err(|_| YamlStringError::InvalidUtf8)?;
                result.push_str(chunk);
            }
        }
    }

    Ok(result)
}

/// Handle line folding for quoted strings.
/// Returns the new position after processing the line break(s).
///
/// Rules:
/// - Trim trailing whitespace from the previous line (already in result)
/// - Skip the line break
/// - Count consecutive empty lines (they become \n)
/// - Skip leading whitespace on the continuation line
/// - Add a space for the line break (or \n for each empty line)
fn fold_quoted_line_break(bytes: &[u8], mut i: usize, result: &mut String) -> usize {
    // Trim trailing whitespace from what we've accumulated
    while result.ends_with(' ') || result.ends_with('\t') {
        result.pop();
    }

    // Skip the first line break
    if bytes[i] == b'\r' {
        i += 1;
        if i < bytes.len() && bytes[i] == b'\n' {
            i += 1;
        }
    } else if bytes[i] == b'\n' {
        i += 1;
    }

    // Count empty lines (lines with only whitespace)
    let mut empty_lines = 0;
    loop {
        // Skip whitespace at start of line
        while i < bytes.len() && matches!(bytes[i], b' ' | b'\t') {
            i += 1;
        }

        // Check if this is an empty line
        if i < bytes.len() && (bytes[i] == b'\n' || bytes[i] == b'\r') {
            empty_lines += 1;
            // Skip the line break
            if bytes[i] == b'\r' {
                i += 1;
                if i < bytes.len() && bytes[i] == b'\n' {
                    i += 1;
                }
            } else {
                i += 1;
            }
        } else {
            // Non-empty line - we're done counting empty lines
            // i is now positioned after the leading whitespace
            break;
        }
    }

    // Output the folded content
    if empty_lines == 0 {
        // Single line break -> space
        result.push(' ');
    } else {
        // Empty lines -> newlines (first line break becomes space, rest become \n)
        // Actually per YAML spec: empty lines preserve as \n each
        for _ in 0..empty_lines {
            result.push('\n');
        }
    }

    i
}

/// Decode a plain (unquoted) scalar with line folding.
///
/// Rules for plain scalars:
/// - Continuation lines must be more indented than base_indent
/// - Single line breaks between non-empty lines become spaces (folding)
/// - Empty lines (only whitespace) become literal newlines
/// - Leading whitespace on continuation lines is stripped (up to a reasonable amount)
/// - Trailing whitespace on each line is stripped
fn decode_plain_scalar(bytes: &[u8], base_indent: usize) -> Result<String, YamlStringError> {
    let mut result = String::with_capacity(bytes.len());
    let mut i = 0;

    while i < bytes.len() {
        match bytes[i] {
            b'\r' | b'\n' => {
                // Line folding
                i = fold_plain_line_break(bytes, i, base_indent, &mut result);
            }
            _ => {
                // Regular content - read until end of line
                let start = i;
                while i < bytes.len() && !matches!(bytes[i], b'\n' | b'\r') {
                    i += 1;
                }
                // Trim trailing whitespace
                let mut end = i;
                while end > start && matches!(bytes[end - 1], b' ' | b'\t') {
                    end -= 1;
                }
                let chunk = core::str::from_utf8(&bytes[start..end])
                    .map_err(|_| YamlStringError::InvalidUtf8)?;
                result.push_str(chunk);
            }
        }
    }

    Ok(result)
}

/// Handle line folding for plain scalars.
/// Returns the new position after processing the line break(s).
///
/// Rules:
/// - Skip the line break
/// - Count consecutive empty lines (they become \n)
/// - Skip leading whitespace on the continuation line (indentation)
/// - Add a space for the line break (or \n for each empty line)
fn fold_plain_line_break(
    bytes: &[u8],
    mut i: usize,
    base_indent: usize,
    result: &mut String,
) -> usize {
    // Skip the first line break
    if bytes[i] == b'\r' {
        i += 1;
        if i < bytes.len() && bytes[i] == b'\n' {
            i += 1;
        }
    } else if bytes[i] == b'\n' {
        i += 1;
    }

    // Count empty lines (lines with only whitespace)
    let mut empty_lines = 0;
    loop {
        // Skip leading whitespace (spaces for indentation)
        let line_start = i;
        while i < bytes.len() && bytes[i] == b' ' {
            i += 1;
        }
        let line_indent = i - line_start;

        // Check if this is an empty line
        if i < bytes.len() && (bytes[i] == b'\n' || bytes[i] == b'\r') {
            empty_lines += 1;
            // Skip the line break
            if bytes[i] == b'\r' {
                i += 1;
                if i < bytes.len() && bytes[i] == b'\n' {
                    i += 1;
                }
            } else {
                i += 1;
            }
        } else if i >= bytes.len() {
            // End of content
            break;
        } else if bytes[i] == b'\t' {
            // Tab after spaces - check if the rest of line is whitespace
            // If so, this is an empty line for folding purposes
            let mut check = i;
            while check < bytes.len() && matches!(bytes[check], b'\t' | b' ') {
                check += 1;
            }
            if check >= bytes.len() || matches!(bytes[check], b'\n' | b'\r') {
                // Line is all whitespace - treat as empty line
                empty_lines += 1;
                i = check;
                if i < bytes.len() && bytes[i] == b'\r' {
                    i += 1;
                    if i < bytes.len() && bytes[i] == b'\n' {
                        i += 1;
                    }
                } else if i < bytes.len() && bytes[i] == b'\n' {
                    i += 1;
                }
                // Continue looking for more empty lines
            } else {
                // Tab followed by content - skip the tabs as indentation
                while i < bytes.len() && bytes[i] == b'\t' {
                    i += 1;
                }
                break;
            }
        } else {
            // Non-empty line with content
            // For plain scalars, we've already included all continuation lines
            // in the bytes range, so we just need to strip the indentation
            // The content indent must be > base_indent for this to be a continuation
            if line_indent > base_indent {
                // Valid continuation - position is after the indentation
                break;
            } else {
                // This shouldn't happen if find_plain_scalar_end worked correctly
                // but handle it gracefully
                break;
            }
        }
    }

    // Output the folded content
    if empty_lines == 0 {
        // Single line break -> space
        if !result.is_empty() {
            result.push(' ');
        }
    } else {
        // Empty lines -> newlines
        for _ in 0..empty_lines {
            result.push('\n');
        }
    }

    i
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
    explicit_indent: Option<u8>,
) -> Result<Cow<'a, str>, YamlStringError> {
    let (content_start, content_end) =
        YamlString::find_block_content_range(text, indicator_pos, chomping, explicit_indent);

    if content_start >= content_end {
        return Ok(Cow::Borrowed(""));
    }

    let content = &text[content_start..content_end];

    // Determine indentation to strip:
    // - If explicit_indent is specified, use base_indent + explicit_indent
    // - Otherwise, auto-detect from first non-empty line
    let base_indent = YamlString::compute_key_indent(text, indicator_pos);
    let indent = if let Some(ei) = explicit_indent {
        base_indent + ei as usize
    } else {
        YamlString::detect_block_indent(text, content_start).unwrap_or(0)
    };
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

    // Apply chomping at the end
    match chomping {
        ChompingIndicator::Strip => {
            while result.ends_with('\n') {
                result.pop();
            }
        }
        ChompingIndicator::Clip => {
            // Clip: Ensure exactly one trailing newline
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

/// Decode a folded block scalar (folds newlines to spaces).
fn decode_block_folded<'a>(
    text: &'a [u8],
    indicator_pos: usize,
    chomping: ChompingIndicator,
    explicit_indent: Option<u8>,
) -> Result<Cow<'a, str>, YamlStringError> {
    let (content_start, content_end) =
        YamlString::find_block_content_range(text, indicator_pos, chomping, explicit_indent);

    if content_start >= content_end {
        return Ok(Cow::Borrowed(""));
    }

    let content = &text[content_start..content_end];

    // Determine indentation to strip:
    // - If explicit_indent is specified, use base_indent + explicit_indent
    // - Otherwise, auto-detect from first non-empty line
    let base_indent = YamlString::compute_key_indent(text, indicator_pos);
    let indent = if let Some(ei) = explicit_indent {
        base_indent + ei as usize
    } else {
        YamlString::detect_block_indent(text, content_start).unwrap_or(0)
    };

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
            // A line is "more indented" if it has extra spaces OR starts with a tab
            // Per YAML spec 8.1.3: "Lines starting with white space characters (more-indented lines) are not folded."
            let is_more_indented =
                line_indent > indent || (pos < content.len() && content[pos] == b'\t');

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

    #[test]
    fn test_multiline_double_quoted_simple() {
        // Simple line folding: newline becomes space
        let yaml = b"key: \"line one\n  line two\"";
        let index = YamlIndex::build(yaml).unwrap();
        let root = index.root(yaml);
        if let YamlValue::Sequence(docs) = root.value() {
            let doc = docs.into_iter().next().unwrap();
            if let YamlValue::Mapping(fields) = doc {
                let field = fields.into_iter().next().unwrap();
                let value = field.value();
                if let YamlValue::String(s) = value {
                    assert_eq!(&*s.as_str().unwrap(), "line one line two");
                } else {
                    panic!("expected string value");
                }
            } else {
                panic!("expected mapping");
            }
        } else {
            panic!("expected sequence");
        }
    }

    #[test]
    fn test_multiline_double_quoted_empty_line() {
        // Empty line becomes newline
        let yaml = b"key: \"line one\n\n  line two\"";
        let index = YamlIndex::build(yaml).unwrap();
        let root = index.root(yaml);
        if let YamlValue::Sequence(docs) = root.value() {
            let doc = docs.into_iter().next().unwrap();
            if let YamlValue::Mapping(fields) = doc {
                let field = fields.into_iter().next().unwrap();
                let value = field.value();
                if let YamlValue::String(s) = value {
                    assert_eq!(&*s.as_str().unwrap(), "line one\nline two");
                } else {
                    panic!("expected string value");
                }
            } else {
                panic!("expected mapping");
            }
        } else {
            panic!("expected sequence");
        }
    }

    #[test]
    fn test_multiline_double_quoted_escaped_newline() {
        // Escaped newline: no space added
        let yaml = b"key: \"line one\\\n  line two\"";
        let index = YamlIndex::build(yaml).unwrap();
        let root = index.root(yaml);
        if let YamlValue::Sequence(docs) = root.value() {
            let doc = docs.into_iter().next().unwrap();
            if let YamlValue::Mapping(fields) = doc {
                let field = fields.into_iter().next().unwrap();
                let value = field.value();
                if let YamlValue::String(s) = value {
                    assert_eq!(&*s.as_str().unwrap(), "line oneline two");
                } else {
                    panic!("expected string value");
                }
            } else {
                panic!("expected mapping");
            }
        } else {
            panic!("expected sequence");
        }
    }

    #[test]
    fn test_multiline_single_quoted_simple() {
        // Simple line folding in single-quoted strings
        let yaml = b"key: 'line one\n  line two'";
        let index = YamlIndex::build(yaml).unwrap();
        let root = index.root(yaml);
        if let YamlValue::Sequence(docs) = root.value() {
            let doc = docs.into_iter().next().unwrap();
            if let YamlValue::Mapping(fields) = doc {
                let field = fields.into_iter().next().unwrap();
                let value = field.value();
                if let YamlValue::String(s) = value {
                    assert_eq!(&*s.as_str().unwrap(), "line one line two");
                } else {
                    panic!("expected string value");
                }
            } else {
                panic!("expected mapping");
            }
        } else {
            panic!("expected sequence");
        }
    }

    #[test]
    fn test_multiline_single_quoted_with_escaped_quote() {
        // Single-quoted with '' escape and line folding
        let yaml = b"key: 'it''s\n  working'";
        let index = YamlIndex::build(yaml).unwrap();
        let root = index.root(yaml);
        if let YamlValue::Sequence(docs) = root.value() {
            let doc = docs.into_iter().next().unwrap();
            if let YamlValue::Mapping(fields) = doc {
                let field = fields.into_iter().next().unwrap();
                let value = field.value();
                if let YamlValue::String(s) = value {
                    assert_eq!(&*s.as_str().unwrap(), "it's working");
                } else {
                    panic!("expected string value");
                }
            } else {
                panic!("expected mapping");
            }
        } else {
            panic!("expected sequence");
        }
    }

    #[test]
    fn test_sequence_entry_content_on_next_line() {
        // Sequence entry with content on the next line parses without error
        // (structure verification is done by the official YAML test suite tests 229Q and M6YH)
        let yaml = b"-\n  name: Mark McGwire\n  hr: 65";
        let result = YamlIndex::build(yaml);
        assert!(result.is_ok(), "Failed to parse: {:?}", result.err());
    }

    #[test]
    fn test_sequence_entry_nested_sequence_on_next_line() {
        // Nested sequence on next line parses without error
        let yaml = b"-\n - inner1\n - inner2";
        let result = YamlIndex::build(yaml);
        assert!(result.is_ok(), "Failed to parse: {:?}", result.err());
    }
}
