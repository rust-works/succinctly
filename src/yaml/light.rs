#![allow(clippy::items_after_test_module)]
//! YamlCursor - Lazy YAML navigation using the semi-index.
//!
//! This module provides a cursor-based API for navigating YAML structures
//! without fully parsing the YAML text. Values are only decoded when explicitly
//! requested.

#[cfg(not(test))]
use alloc::{borrow::Cow, string::String, string::ToString, vec::Vec};

#[cfg(test)]
use std::borrow::Cow;
#[cfg(test)]
use std::string::ToString;

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

    /// Get the text end byte offset for this node.
    ///
    /// For scalars, returns the end position. For containers, returns 0.
    #[inline]
    pub fn text_end_position(&self) -> Option<usize> {
        self.index.bp_to_text_end_pos(self.bp_pos)
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

        // Compute open_idx once — reused for both text_pos and text_end_pos
        // to avoid a redundant rank1 in the unquoted scalar path.
        let open_idx = self.index.bp_to_open_idx(self.bp_pos);
        let Some(text_pos) = self.index.text_pos_by_open_idx(open_idx) else {
            return YamlValue::Error("invalid cursor position");
        };

        // text_pos == self.text.len() is used as a sentinel for null values
        // (e.g., explicit keys without values: `? key` with no `: value`)
        if text_pos >= self.text.len() {
            return YamlValue::Null;
        }

        // Check if this is a container FIRST - this takes priority over text-based detection.
        // This is important for mappings where the key is an alias (e.g., `*alias : value`),
        // which would otherwise be detected as an alias by looking at the leading `*`.
        if self.is_container() {
            // Determine if mapping or sequence using the TY bits
            if self.index.is_sequence_at_bp(self.bp_pos) {
                return YamlValue::Sequence(YamlElements::from_sequence_cursor(*self));
            } else {
                return YamlValue::Mapping(YamlFields::from_mapping_cursor(*self));
            }
        }

        // Check for alias (only for non-container nodes)
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
                // Use pre-computed end position for O(1) lookup.
                // Reuses open_idx computed above to skip redundant rank1.
                let end = self
                    .index
                    .text_end_pos_by_open_idx(open_idx)
                    .filter(|&e| e >= effective_text_pos)
                    .unwrap_or(effective_text_pos);

                // Compute base_indent only if needed for multi-line scalar decoding
                // For single-line scalars (no newlines), base_indent doesn't matter
                let base_indent = if end > effective_text_pos
                    && self.text[effective_text_pos..end].contains(&b'\n')
                {
                    // Multi-line scalar - compute base indent from line start
                    self.compute_scalar_base_indent(effective_text_pos)
                } else {
                    // Single-line scalar - base_indent is unused
                    0
                };

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

    /// Compute base indent for multi-line plain scalar decoding.
    /// Returns the indent of the current line (spaces at start of line).
    /// This is a simplified version used when we already have the end position.
    fn compute_scalar_base_indent(&self, value_pos: usize) -> usize {
        // Find start of current line
        let mut line_start = value_pos;
        while line_start > 0 && self.text[line_start - 1] != b'\n' {
            line_start -= 1;
        }

        // Compute line indent (spaces at start of line)
        Self::compute_line_indent_static(self.text, line_start)
    }

    /// Compute the base indent for plain scalar continuation.
    /// For values on their own line (after key:), this returns the key's indent.
    /// For values on the same line as the key, this returns that line's indent.
    /// Compute base indent for plain scalar continuation checking.
    /// Returns (base_indent, is_document_root) where is_document_root is true
    /// if this scalar is the document root content (right after --- or at start).
    #[allow(dead_code)]
    fn compute_base_indent_and_root_flag(&self, value_pos: usize) -> (usize, bool) {
        // Find start of current line
        let mut line_start = value_pos;
        while line_start > 0 && self.text[line_start - 1] != b'\n' {
            line_start -= 1;
        }

        // Compute line indent (spaces at start of line)
        let line_indent = Self::compute_line_indent_static(self.text, line_start);

        // Check for explicit key indicator `? `, explicit value indicator `: `,
        // and sequence indicator `- ` before the value on this line.
        // Also check for key separators (`:` followed by space/tab/newline).
        let mut last_seq_col = None;
        let mut explicit_indicator_col = None;
        let mut last_key_separator_pos = None;
        let mut scan = line_start + line_indent; // Skip leading whitespace
        while scan < value_pos {
            if self.text[scan] == b'?' {
                // Check if this is an explicit key indicator (followed by space/tab/newline)
                if scan + 1 < self.text.len()
                    && matches!(self.text[scan + 1], b' ' | b'\t' | b'\n' | b'\r')
                {
                    explicit_indicator_col = Some(scan - line_start);
                }
            }
            if self.text[scan] == b':' {
                // Check if this colon is a key separator (followed by space/tab/newline)
                if scan + 1 < self.text.len() && matches!(self.text[scan + 1], b' ' | b'\t' | b'\n')
                {
                    // Check if this is an explicit value indicator at start of content
                    if scan == line_start + line_indent {
                        explicit_indicator_col = Some(scan - line_start);
                    } else {
                        // This is a key separator for `key: value`
                        last_key_separator_pos = Some(scan);
                    }
                }
            }
            if self.text[scan] == b'-' {
                // Check if this is a sequence indicator (followed by space/tab)
                if scan + 1 < self.text.len() && matches!(self.text[scan + 1], b' ' | b'\t') {
                    // This is a sequence indicator at column (scan - line_start)
                    last_seq_col = Some(scan - line_start);
                }
            }
            scan += 1;
        }

        // If there's a key separator after the sequence indicator, use the key:value logic
        // (handled below in has_colon_before). This handles `- key: value` patterns.
        // Only use sequence indicator if there's no key separator between it and the value.
        let seq_col_before_key = match (last_seq_col, last_key_separator_pos) {
            (Some(seq_col), Some(key_sep_pos)) => {
                // Check if sequence indicator is before the key separator
                let seq_pos = line_start + seq_col;
                if seq_pos < key_sep_pos {
                    // Key separator is after sequence indicator - don't use seq_col
                    None
                } else {
                    Some(seq_col)
                }
            }
            (Some(seq_col), None) => Some(seq_col),
            _ => None,
        };

        if let Some(seq_col) = seq_col_before_key {
            return (seq_col, false);
        }
        if let Some(exp_col) = explicit_indicator_col {
            return (exp_col, false);
        }

        // Check if there's a colon before us on this line (meaning key: value on same line)
        let mut has_colon_before = false;
        let mut colon_pos = 0;
        scan = line_start;
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
            // (Sequence indicators were already handled above)

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
    #[allow(dead_code)]
    fn is_in_flow_context(&self, pos: usize) -> bool {
        // Find start of line containing pos
        let mut line_start = pos;
        while line_start > 0 && self.text[line_start - 1] != b'\n' {
            line_start -= 1;
        }

        // Fast path: check if current line has any flow markers at all.
        // If no [ or { on this line, and we're not continuing from a previous line's
        // flow context, we can skip the expensive scan.
        let mut has_flow_marker_on_line = false;
        for i in line_start..pos {
            match self.text[i] {
                b'[' | b'{' => {
                    has_flow_marker_on_line = true;
                    break;
                }
                _ => {}
            }
        }

        // If no flow markers on this line, check if we need to look back.
        // A multiline flow context would have a flow marker on a previous line.
        if !has_flow_marker_on_line {
            // Quick check: scan back at most 256 bytes for any [ or {
            // If there are no flow openers in the recent context, we're not in flow.
            let quick_start = line_start.saturating_sub(256);
            let mut found_opener = false;
            for i in quick_start..line_start {
                if matches!(self.text[i], b'[' | b'{') {
                    found_opener = true;
                    break;
                }
            }
            if !found_opener {
                return false;
            }
        }

        // Full scan needed - there might be flow context
        // Scan back up to 4KB or start of text
        let scan_start = if line_start > 4096 {
            let mut start = line_start.saturating_sub(4096);
            while start > 0 && self.text[start - 1] != b'\n' {
                start -= 1;
            }
            start
        } else {
            0
        };

        let mut depth = 0i32;
        let mut in_double_quote = false;
        let mut in_single_quote = false;
        let mut i = scan_start;

        while i < pos {
            let byte = self.text[i];

            if in_double_quote {
                if byte == b'\\' && i + 1 < pos {
                    i += 2;
                    continue;
                }
                if byte == b'"' {
                    in_double_quote = false;
                }
            } else if in_single_quote {
                if byte == b'\'' {
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
    #[allow(dead_code)]
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
                        } else if in_flow
                            || line_indent > base_indent
                            || (is_doc_root && line_indent == 0)
                        {
                            // Tab followed by content, with sufficient indent - this is a continuation.
                            // For document root scalars (base_indent=0), tabs at start of line
                            // are valid continuation per YAML spec example 7.12 "Plain Lines".
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
                                || self.text[after_indent + 1] == b'\t'
                                || self.text[after_indent + 1] == b'\n')
                        {
                            // This looks like a sequence indicator `- `.
                            // It's only a real sequence indicator if the indent matches
                            // the base indent. If the `- ` is more indented, it's part
                            // of the scalar content (like AB8U test case).
                            if line_indent == base_indent || (is_doc_root && line_indent == 0) {
                                // This is a sequence item at correct indent - not a continuation
                                return line_end;
                            }
                            // `- ` at greater indent is scalar content - continue below
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
                    b'?' => {
                        // Check for explicit key indicator "?" at start of line
                        // `? ` or `?\n` or `?` at EOF is an explicit key indicator
                        if after_indent + 1 >= self.text.len()
                            || matches!(self.text[after_indent + 1], b' ' | b'\t' | b'\n' | b'\r')
                        {
                            // This is an explicit key indicator - not a continuation
                            return line_end;
                        }
                        // `?` followed by content is a plain scalar continuation
                        // Check if it's a continuation based on indentation or flow context
                        if in_flow || line_indent > base_indent || (is_doc_root && line_indent == 0)
                        {
                            end = empty_lines_end;
                            break;
                        }
                        return line_end;
                    }
                    b':' => {
                        // Check for explicit value indicator ":" at start of line
                        // `: ` or `:\n` or `:` at EOF is an explicit value indicator
                        if after_indent + 1 >= self.text.len()
                            || matches!(self.text[after_indent + 1], b' ' | b'\t' | b'\n' | b'\r')
                        {
                            // This is an explicit value indicator - not a continuation
                            return line_end;
                        }
                        // `:` followed by content is a plain scalar continuation
                        // Check if it's a continuation based on indentation or flow context
                        if in_flow || line_indent > base_indent || (is_doc_root && line_indent == 0)
                        {
                            end = empty_lines_end;
                            break;
                        }
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

    /// Convert this YAML value directly to a JSON string.
    ///
    /// This streams directly from the YAML cursor to JSON output without
    /// building an intermediate DOM. Much more efficient than converting
    /// to OwnedValue first.
    ///
    /// Note: This outputs the raw structure including the document wrapper array.
    /// For yq-style output (unwrapping single documents), use `to_json_document()`.
    pub fn to_json(&self) -> String {
        let mut output = String::new();
        self.write_json_to(&mut output);
        output
    }

    /// Convert YAML to JSON, unwrapping single documents.
    ///
    /// If the root is a single-document array `[doc]`, returns just `doc` as JSON.
    /// If there are multiple documents, returns the array `[doc1, doc2, ...]`.
    /// This matches yq's behavior.
    pub fn to_json_document(&self) -> String {
        let mut output = String::new();

        // Check if this is the root document array with a single document
        if self.bp_pos == 0 {
            if let YamlValue::Sequence(elements) = self.value() {
                let mut iter = elements.into_iter();
                if let Some(first) = iter.next() {
                    if iter.next().is_none() {
                        // Single document - output it directly without array wrapper
                        write_yaml_value_as_json(&mut output, first);
                        return output;
                    }
                }
            }
        }

        // Multiple documents or not at root - output as-is
        self.write_json_to(&mut output);
        output
    }

    /// Stream YAML as JSON directly to a writer without intermediate String allocation.
    ///
    /// This is the most memory-efficient way to output JSON for large YAML files.
    /// Uses `core::fmt::Write` for `no_std` compatibility.
    ///
    /// # Example
    /// ```ignore
    /// use std::io::Write;
    /// let mut output = Vec::new();
    /// // Wrap std::io::Write in a fmt::Write adapter
    /// struct FmtWriter<W>(W);
    /// impl<W: std::io::Write> core::fmt::Write for FmtWriter<W> {
    ///     fn write_str(&mut self, s: &str) -> core::fmt::Result {
    ///         self.0.write_all(s.as_bytes()).map_err(|_| core::fmt::Error)
    ///     }
    /// }
    /// cursor.stream_json(&mut FmtWriter(&mut output)).unwrap();
    /// ```
    pub fn stream_json<Out: core::fmt::Write>(&self, out: &mut Out) -> core::fmt::Result {
        self.stream_json_value(out)
    }

    /// Stream YAML as JSON, unwrapping single documents (matches yq behavior).
    ///
    /// If the root is a single-document array `[doc]`, outputs just `doc` as JSON.
    /// If there are multiple documents, outputs the array `[doc1, doc2, ...]`.
    pub fn stream_json_document<Out: core::fmt::Write>(&self, out: &mut Out) -> core::fmt::Result {
        // Check if this is the root document array with a single document
        if self.bp_pos == 0 {
            if let YamlValue::Sequence(elements) = self.value() {
                let mut iter = elements.into_iter();
                if let Some(first) = iter.next() {
                    if iter.next().is_none() {
                        // Single document - output it directly without array wrapper
                        return stream_yaml_value_as_json(out, first);
                    }
                }
            }
        }

        // Multiple documents or not at root - output as-is
        self.stream_json_value(out)
    }

    /// Stream this cursor's value as YAML.
    ///
    /// - `indent_spaces`: Spaces per indentation level (0 for flow style)
    ///
    /// This enables M2.5 streaming optimization for YAML output format,
    /// allowing navigation query results to be written directly without
    /// materializing OwnedValue.
    pub fn stream_yaml<Out: core::fmt::Write>(
        &self,
        out: &mut Out,
        indent_spaces: usize,
    ) -> core::fmt::Result {
        self.stream_yaml_value(out, 0, indent_spaces)
    }

    /// Stream YAML, unwrapping single documents (matches yq behavior).
    ///
    /// If the root is a single-document array `[doc]`, outputs just `doc` as YAML.
    pub fn stream_yaml_document<Out: core::fmt::Write>(
        &self,
        out: &mut Out,
        indent_spaces: usize,
    ) -> core::fmt::Result {
        // Check if this is the root document array with a single document
        if self.bp_pos == 0 {
            if let YamlValue::Sequence(elements) = self.value() {
                if let Some((cursor, rest)) = elements.uncons_cursor() {
                    if rest.is_empty() {
                        // Single document - output it directly without array wrapper
                        return cursor.stream_yaml_value(out, 0, indent_spaces);
                    }
                }
            }
        }

        // Multiple documents or not at root - output as-is
        self.stream_yaml_value(out, 0, indent_spaces)
    }

    /// Internal: stream this cursor's value as YAML.
    fn stream_yaml_value<Out: core::fmt::Write>(
        &self,
        out: &mut Out,
        current_indent: usize,
        indent_spaces: usize,
    ) -> core::fmt::Result {
        match self.value() {
            YamlValue::Null => out.write_str("null"),
            YamlValue::String(s) => stream_yaml_string_value(out, &s),
            YamlValue::Mapping(fields) => {
                if fields.is_empty() {
                    return out.write_str("{}");
                }
                if indent_spaces == 0 {
                    // Flow style
                    out.write_char('{')?;
                    let mut first = true;
                    for field in fields {
                        if !first {
                            out.write_str(", ")?;
                        }
                        first = false;
                        // Write key
                        if let YamlValue::String(s) = field.key() {
                            stream_yaml_string_value(out, &s)?;
                        } else {
                            out.write_str("\"\"")?;
                        }
                        out.write_str(": ")?;
                        field.value_cursor().stream_yaml_value(out, 0, 0)?;
                    }
                    out.write_char('}')
                } else {
                    // Block style
                    let mut first = true;
                    for field in fields {
                        if !first {
                            out.write_char('\n')?;
                            write_yaml_indent(out, current_indent)?;
                        }
                        first = false;
                        // Write key
                        if let YamlValue::String(s) = field.key() {
                            stream_yaml_string_value(out, &s)?;
                        } else {
                            out.write_str("\"\"")?;
                        }
                        out.write_char(':')?;
                        // Check if value needs newline
                        let value = field.value_cursor();
                        if is_yaml_cursor_container(&value) {
                            out.write_char('\n')?;
                            write_yaml_indent(out, current_indent + indent_spaces)?;
                            value.stream_yaml_value(
                                out,
                                current_indent + indent_spaces,
                                indent_spaces,
                            )?;
                        } else {
                            out.write_char(' ')?;
                            value.stream_yaml_value(
                                out,
                                current_indent + indent_spaces,
                                indent_spaces,
                            )?;
                        }
                    }
                    Ok(())
                }
            }
            YamlValue::Sequence(elements) => {
                if elements.is_empty() {
                    return out.write_str("[]");
                }
                if indent_spaces == 0 {
                    // Flow style
                    out.write_char('[')?;
                    let mut first = true;
                    let mut elems = elements;
                    while let Some((cursor, rest)) = elems.uncons_cursor() {
                        if !first {
                            out.write_str(", ")?;
                        }
                        first = false;
                        cursor.stream_yaml_value(out, 0, 0)?;
                        elems = rest;
                    }
                    out.write_char(']')
                } else {
                    // Block style
                    let mut first = true;
                    let mut elems = elements;
                    while let Some((cursor, rest)) = elems.uncons_cursor() {
                        if !first {
                            out.write_char('\n')?;
                            write_yaml_indent(out, current_indent)?;
                        }
                        first = false;
                        out.write_str("- ")?;
                        // Check if value needs newline
                        if is_yaml_cursor_container(&cursor) {
                            out.write_char('\n')?;
                            write_yaml_indent(out, current_indent + indent_spaces)?;
                            cursor.stream_yaml_value(
                                out,
                                current_indent + indent_spaces,
                                indent_spaces,
                            )?;
                        } else {
                            cursor.stream_yaml_value(
                                out,
                                current_indent + indent_spaces,
                                indent_spaces,
                            )?;
                        }
                        elems = rest;
                    }
                    Ok(())
                }
            }
            YamlValue::Alias { target, .. } => {
                if let Some(target_cursor) = target {
                    target_cursor.stream_yaml_value(out, current_indent, indent_spaces)
                } else {
                    out.write_str("null")
                }
            }
            YamlValue::Error(_) => out.write_str("null"),
        }
    }

    /// Internal: stream this cursor's value as JSON.
    fn stream_json_value<Out: core::fmt::Write>(&self, out: &mut Out) -> core::fmt::Result {
        match self.value() {
            YamlValue::Null => out.write_str("null"),
            YamlValue::String(s) => {
                // Direct transcoding optimization
                match stream_yaml_string_to_json(out, &s) {
                    Ok(true) => Ok(()), // Written directly as JSON string
                    Ok(false) => {
                        // Unquoted/block scalar - need type detection
                        if let Ok(str_val) = s.as_str() {
                            stream_yaml_scalar_as_json(out, &str_val)
                        } else {
                            out.write_str("null")
                        }
                    }
                    Err(_) => out.write_str("null"),
                }
            }
            YamlValue::Mapping(fields) => {
                out.write_char('{')?;
                let mut first = true;
                for field in fields {
                    if !first {
                        out.write_char(',')?;
                    }
                    first = false;

                    // Write key
                    if let YamlValue::String(s) = field.key() {
                        match stream_yaml_string_to_json(out, &s) {
                            Ok(true) => {} // Written directly
                            Ok(false) | Err(_) => {
                                if let Ok(key_str) = s.as_str() {
                                    stream_json_string(out, &key_str)?;
                                } else {
                                    out.write_str("\"\"")?;
                                }
                            }
                        }
                    } else {
                        out.write_str("\"\"")?;
                    }

                    out.write_char(':')?;
                    field.value_cursor().stream_json_value(out)?;
                }
                out.write_char('}')
            }
            YamlValue::Sequence(elements) => {
                out.write_char('[')?;
                let mut first = true;
                for elem in elements {
                    if !first {
                        out.write_char(',')?;
                    }
                    first = false;
                    stream_yaml_value_as_json(out, elem)?;
                }
                out.write_char(']')
            }
            YamlValue::Alias { target, .. } => {
                if let Some(target_cursor) = target {
                    target_cursor.stream_json_value(out)
                } else {
                    out.write_str("null")
                }
            }
            YamlValue::Error(_) => out.write_str("null"),
        }
    }

    /// Write this YAML value as JSON to a string buffer.
    fn write_json_to(&self, output: &mut String) {
        match self.value() {
            YamlValue::Null => output.push_str("null"),
            YamlValue::String(s) => {
                // Direct transcoding optimization (avoids intermediate allocation for quoted strings)
                match write_yaml_string_to_json(output, &s) {
                    Ok(true) => {} // Written directly as JSON string
                    Ok(false) => {
                        // Unquoted/block scalar - need type detection, fall back to original path
                        if let Ok(str_val) = s.as_str() {
                            write_yaml_scalar_as_json(output, &str_val);
                        } else {
                            output.push_str("null");
                        }
                    }
                    Err(_) => output.push_str("null"),
                }
            }
            YamlValue::Mapping(fields) => {
                output.push('{');
                let mut first = true;
                for field in fields {
                    if !first {
                        output.push(',');
                    }
                    first = false;

                    // Write key - try direct transcoding first
                    if let YamlValue::String(s) = field.key() {
                        match write_yaml_string_to_json(output, &s) {
                            Ok(true) => {} // Written directly
                            Ok(false) | Err(_) => {
                                // Fall back to original path
                                if let Ok(key_str) = s.as_str() {
                                    write_json_string(output, &key_str);
                                } else {
                                    output.push_str("\"\"");
                                }
                            }
                        }
                    } else {
                        output.push_str("\"\"");
                    }

                    output.push(':');

                    // Write value by recursing on the cursor
                    field.value_cursor().write_json_to(output);
                }
                output.push('}');
            }
            YamlValue::Sequence(elements) => {
                output.push('[');
                let mut first = true;
                for elem in elements {
                    if !first {
                        output.push(',');
                    }
                    first = false;

                    // For sequences, we get YamlValue not cursor, so need to handle inline
                    write_yaml_value_as_json(output, elem);
                }
                output.push(']');
            }
            YamlValue::Alias { target, .. } => {
                if let Some(target_cursor) = target {
                    target_cursor.write_json_to(output);
                } else {
                    output.push_str("null");
                }
            }
            YamlValue::Error(_) => output.push_str("null"),
        }
    }

    // =========================================================================
    // YAML Metadata Access
    // =========================================================================

    /// Get the anchor name for this node, if any.
    ///
    /// Returns the anchor name (without the `&` prefix) if this node was
    /// defined with an anchor, e.g., `&myanchor value` returns `"myanchor"`.
    ///
    /// Note: For alias nodes (`*name`), this returns `None`. Use `alias()` to
    /// get the referenced anchor name for alias nodes.
    ///
    /// # Example
    /// ```ignore
    /// let yaml = b"default: &def value";
    /// let index = YamlIndex::build(yaml).unwrap();
    /// // Navigate to the anchored value and call .anchor()
    /// ```
    #[inline]
    pub fn anchor(&self) -> Option<&str> {
        self.index.get_anchor_name(self.bp_pos)
    }

    /// Get the anchor name that this alias references.
    ///
    /// Returns the anchor name (without the `*` prefix) if this node is an alias,
    /// e.g., `*myanchor` returns `Some("myanchor")`.
    ///
    /// Returns `None` if this node is not an alias. This matches yq's `alias` function.
    ///
    /// # Example
    /// ```ignore
    /// let yaml = b"default: &def value\nref: *def";
    /// let index = YamlIndex::build(yaml).unwrap();
    /// // Navigate to ref and call .alias() to get "def"
    /// ```
    #[inline]
    pub fn alias(&self) -> Option<&str> {
        self.index.get_alias_anchor_name(self.bp_pos)
    }

    /// Check if this node is an alias.
    #[inline]
    pub fn is_alias(&self) -> bool {
        self.index.is_alias(self.bp_pos)
    }

    /// Get the 1-based line number of this node's position in the YAML text.
    ///
    /// Returns the line number where this value starts. Line numbers are 1-based
    /// to match common editor conventions and yq's `line` function.
    ///
    /// # Example
    ///
    /// ```
    /// use succinctly::yaml::YamlIndex;
    ///
    /// let yaml = b"name: Alice\nage: 30";
    /// let index = YamlIndex::build(yaml).unwrap();
    /// let root = index.root(yaml);
    ///
    /// // Root mapping is on line 1
    /// assert_eq!(root.line(), 1);
    /// ```
    #[inline]
    pub fn line(&self) -> usize {
        let offset = self.text_position().unwrap_or(0);
        let (line, _column) = self.index.to_line_column(offset);
        line
    }

    /// Get the 1-based column number of this node's position in the YAML text.
    ///
    /// Returns the column number where this value starts. Column numbers are 1-based
    /// to match common editor conventions and yq's `column` function.
    ///
    /// # Example
    ///
    /// ```
    /// use succinctly::yaml::YamlIndex;
    ///
    /// let yaml = b"name: Alice\nage: 30";
    /// let index = YamlIndex::build(yaml).unwrap();
    /// let root = index.root(yaml);
    ///
    /// // Root mapping starts at column 1
    /// assert_eq!(root.column(), 1);
    /// ```
    #[inline]
    pub fn column(&self) -> usize {
        let offset = self.text_position().unwrap_or(0);
        let (_line, column) = self.index.to_line_column(offset);
        column
    }

    /// Get the 0-indexed document position in a multi-document stream.
    ///
    /// Returns the document index (0 for first document, 1 for second, etc.)
    /// for multi-document YAML files. For single-document files, returns 0.
    ///
    /// This navigates up to find the document-level ancestor (direct child of
    /// the implicit root sequence), then counts preceding siblings to determine
    /// the document index.
    ///
    /// # Example
    ///
    /// ```ignore
    /// use succinctly::yaml::YamlIndex;
    ///
    /// let yaml = b"---\nname: Alice\n---\nname: Bob";
    /// let index = YamlIndex::build(yaml).unwrap();
    /// let root = index.root(yaml);
    ///
    /// // Navigate to first document's content
    /// if let Some(doc1) = root.first_child() {
    ///     assert_eq!(doc1.document_index(), Some(0));
    /// }
    /// ```
    pub fn document_index(&self) -> Option<usize> {
        // If we're at the root (bp_pos=0), there's no document index
        if self.bp_pos == 0 {
            return None;
        }

        // Find the document-level ancestor (direct child of root)
        // Navigate up until parent is root (bp_pos=0)
        let mut doc_ancestor = *self;
        while let Some(parent) = doc_ancestor.parent() {
            if parent.bp_pos == 0 {
                // doc_ancestor is now the document-level node
                break;
            }
            doc_ancestor = parent;
        }

        // Count preceding siblings to get the document index
        // Start from root's first child and count until we reach doc_ancestor
        let root = YamlCursor::new(self.index, self.text, 0);
        let mut current = root.first_child()?;
        let mut index = 0;

        loop {
            if current.bp_pos == doc_ancestor.bp_pos {
                return Some(index);
            }
            match current.next_sibling() {
                Some(next) => {
                    current = next;
                    index += 1;
                }
                None => {
                    // Shouldn't happen if doc_ancestor is valid, but safety fallback
                    return Some(0);
                }
            }
        }
    }

    /// Get the style of this node.
    ///
    /// Returns the YAML style indicator:
    /// - `""` for block style (the default for most values)
    /// - `"flow"` for flow mappings `{}` or sequences `[]`
    /// - `"double"` for double-quoted strings
    /// - `"single"` for single-quoted strings
    /// - `"literal"` for literal block scalars `|`
    /// - `"folded"` for folded block scalars `>`
    ///
    /// Note: This is a simplified implementation that infers style from
    /// the text representation rather than preserving original metadata.
    pub fn style(&self) -> &'static str {
        let Some(text_pos) = self.text_position() else {
            return "";
        };

        if text_pos >= self.text.len() {
            return "";
        }

        // Skip anchor if present
        let effective_pos = if self.text[text_pos] == b'&' {
            self.skip_anchor_and_whitespace(text_pos)
        } else {
            text_pos
        };

        if effective_pos >= self.text.len() {
            return "";
        }

        match self.text[effective_pos] {
            b'"' => "double",
            b'\'' => "single",
            b'|' => "literal",
            b'>' => "folded",
            b'{' | b'[' => "flow",
            _ => "",
        }
    }

    /// Get the YAML type tag for this node.
    ///
    /// Returns the inferred YAML type tag based on the value type:
    /// - `"!!null"` for null values
    /// - `"!!bool"` for boolean values (true/false)
    /// - `"!!int"` for integer values
    /// - `"!!float"` for floating-point values
    /// - `"!!str"` for string values
    /// - `"!!seq"` for sequences (arrays)
    /// - `"!!map"` for mappings (objects)
    ///
    /// Note: This returns inferred tags. Explicit tags in the YAML source
    /// (like `!mytag`) are not currently preserved.
    pub fn tag(&self) -> &'static str {
        match self.value() {
            YamlValue::Null => "!!null",
            YamlValue::String(s) => {
                // Try to infer type from unquoted scalars
                if s.is_unquoted() {
                    if let Ok(str_val) = s.as_str() {
                        let s = str_val.as_ref();
                        // Check for null
                        if matches!(s, "null" | "~" | "") {
                            return "!!null";
                        }
                        // Check for bool
                        if matches!(s, "true" | "True" | "TRUE" | "false" | "False" | "FALSE") {
                            return "!!bool";
                        }
                        // Check for int
                        if s.parse::<i64>().is_ok() {
                            return "!!int";
                        }
                        // Check for float (including special values)
                        if s.parse::<f64>().is_ok()
                            || matches!(
                                s,
                                ".inf"
                                    | ".Inf"
                                    | ".INF"
                                    | "-.inf"
                                    | "-.Inf"
                                    | "-.INF"
                                    | ".nan"
                                    | ".NaN"
                                    | ".NAN"
                            )
                        {
                            return "!!float";
                        }
                    }
                }
                "!!str"
            }
            YamlValue::Mapping(_) => "!!map",
            YamlValue::Sequence(_) => "!!seq",
            YamlValue::Alias { target, .. } => {
                // Return tag of target
                if let Some(t) = target {
                    t.tag()
                } else {
                    "!!null"
                }
            }
            YamlValue::Error(_) => "!!null",
        }
    }

    /// Get the kind of this node.
    ///
    /// Returns the structural kind of the node:
    /// - `"scalar"` for scalar values (strings, numbers, booleans, null)
    /// - `"seq"` for sequences (arrays)
    /// - `"map"` for mappings (objects)
    /// - `"alias"` for unexploded alias references (`*name`)
    ///
    /// Note: This matches yq's `kind` function which reports aliases as a distinct kind.
    pub fn kind(&self) -> &'static str {
        // Check if this is an alias first (before resolving the value)
        if self.is_alias() {
            return "alias";
        }
        match self.value() {
            YamlValue::Null | YamlValue::String(_) => "scalar",
            YamlValue::Sequence(_) => "seq",
            YamlValue::Mapping(_) => "map",
            YamlValue::Alias { .. } => "alias",
            YamlValue::Error(_) => "scalar",
        }
    }

    /// Get a reference to the underlying YAML index.
    #[inline]
    pub fn index(&self) -> &YamlIndex<W> {
        self.index
    }

    /// Get a reference to the underlying YAML text.
    #[inline]
    pub fn text(&self) -> &'a [u8] {
        self.text
    }

    /// Create a cursor at the specified byte offset (0-indexed).
    ///
    /// Returns `None` if:
    /// - The offset is out of bounds
    /// - The offset doesn't correspond to a valid node
    ///
    /// This enables position-based navigation in jq queries via `at_offset(n)`.
    ///
    /// Note: For consistency with how yq evaluates expressions, if the offset
    /// lands on the document root sequence wrapper in a single-document file,
    /// this returns the document content instead of the wrapper.
    pub fn cursor_at_offset(&self, offset: usize) -> Option<YamlCursor<'a, W>> {
        if offset >= self.text.len() {
            return None;
        }

        // Get the rank at this position (count of IB bits in [0, offset))
        let rank = self.index.ib_rank1(offset);

        // Determine which structural element contains this offset
        let struct_text_pos = if let Some(struct_pos) = self.index.ib_select1(rank) {
            if struct_pos == offset {
                // We're exactly at a structural position - use this one
                struct_pos
            } else if rank > 0 {
                // We're inside a value - the containing node started at the previous IB bit
                self.index.ib_select1(rank - 1)?
            } else {
                return None;
            }
        } else if rank > 0 {
            self.index.ib_select1(rank - 1)?
        } else {
            return None;
        };

        // Find the BP position for this text position using binary search
        let bp_pos = self.index.find_bp_at_text_pos(struct_text_pos)?;

        let cursor = YamlCursor {
            text: self.text,
            index: self.index,
            bp_pos,
        };

        // For consistency with yq evaluation: if we land on the document root
        // sequence wrapper (bp_pos == 0, which is a sequence), navigate to
        // the first document's content. This matches how yq '.' works.
        if bp_pos == 0 && self.index.is_sequence_at_bp(0) {
            // Navigate to first document content (skip sequence wrapper)
            cursor.first_child()
        } else {
            Some(cursor)
        }
    }

    /// Create a cursor at the specified line and column (1-indexed).
    ///
    /// Returns `None` if:
    /// - Line or column is 0
    /// - The position is out of bounds
    /// - The position doesn't correspond to a valid node
    ///
    /// This enables position-based navigation in jq queries via `at_position(line; col)`.
    pub fn cursor_at_position(&self, line: usize, col: usize) -> Option<YamlCursor<'a, W>> {
        // Convert line/column to byte offset
        let offset = self.index.to_offset(line, col)?;

        // Use cursor_at_offset to find the node
        self.cursor_at_offset(offset)
    }
}

/// Write a YAML value as JSON (for sequence elements where we don't have a cursor).
fn write_yaml_value_as_json<W: AsRef<[u64]>>(output: &mut String, value: YamlValue<'_, W>) {
    match value {
        YamlValue::Null => output.push_str("null"),
        YamlValue::String(s) => {
            // Direct transcoding optimization (avoids intermediate allocation for quoted strings)
            match write_yaml_string_to_json(output, &s) {
                Ok(true) => {} // Written directly as JSON string
                Ok(false) => {
                    // Unquoted/block scalar - need type detection, fall back to original path
                    if let Ok(str_val) = s.as_str() {
                        write_yaml_scalar_as_json(output, &str_val);
                    } else {
                        output.push_str("null");
                    }
                }
                Err(_) => output.push_str("null"),
            }
        }
        YamlValue::Mapping(fields) => {
            output.push('{');
            let mut first = true;
            for field in fields {
                if !first {
                    output.push(',');
                }
                first = false;

                // Write key - try direct transcoding first
                if let YamlValue::String(s) = field.key() {
                    match write_yaml_string_to_json(output, &s) {
                        Ok(true) => {} // Written directly
                        Ok(false) | Err(_) => {
                            // Fall back to original path
                            if let Ok(key_str) = s.as_str() {
                                write_json_string(output, &key_str);
                            } else {
                                output.push_str("\"\"");
                            }
                        }
                    }
                } else {
                    output.push_str("\"\"");
                }

                output.push(':');
                field.value_cursor().write_json_to(output);
            }
            output.push('}');
        }
        YamlValue::Sequence(elements) => {
            output.push('[');
            let mut first = true;
            for elem in elements {
                if !first {
                    output.push(',');
                }
                first = false;
                write_yaml_value_as_json(output, elem);
            }
            output.push(']');
        }
        YamlValue::Alias { target, .. } => {
            if let Some(target_cursor) = target {
                target_cursor.write_json_to(output);
            } else {
                output.push_str("null");
            }
        }
        YamlValue::Error(_) => output.push_str("null"),
    }
}

/// Write a string with JSON escaping.
///
/// Uses a fast path for ASCII-only strings without special characters,
/// which is the common case for YAML data.
fn write_json_string(output: &mut String, s: &str) {
    output.push('"');

    let bytes = s.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        // Find the next byte that needs escaping
        let start = i;
        while i < bytes.len() {
            let b = bytes[i];
            // Check for bytes that need escaping: ", \, control chars (0x00-0x1F)
            if b == b'"' || b == b'\\' || b < 0x20 {
                break;
            }
            i += 1;
        }

        // Copy the safe span directly
        if start < i {
            // SAFETY: We're copying valid UTF-8 bytes that don't contain
            // any multi-byte sequence starters that would be split
            output.push_str(&s[start..i]);
        }

        // Handle the escape character if any
        if i < bytes.len() {
            let b = bytes[i];
            match b {
                b'"' => output.push_str("\\\""),
                b'\\' => output.push_str("\\\\"),
                b'\n' => output.push_str("\\n"),
                b'\r' => output.push_str("\\r"),
                b'\t' => output.push_str("\\t"),
                b if b < 0x20 => {
                    // Control character - use \uXXXX
                    output.push_str("\\u00");
                    const HEX: &[u8; 16] = b"0123456789abcdef";
                    output.push(HEX[(b >> 4) as usize] as char);
                    output.push(HEX[(b & 0xf) as usize] as char);
                }
                _ => unreachable!(),
            }
            i += 1;
        }
    }

    output.push('"');
}

// ============================================================================
// Direct YAML→JSON Transcoding (Zero-Copy for Escaped Strings)
// ============================================================================

/// Write a JSON character escape sequence to output.
/// Helper for direct transcoding functions.
#[inline]
fn write_json_escape(output: &mut String, ch: char) {
    match ch {
        '"' => output.push_str("\\\""),
        '\\' => output.push_str("\\\\"),
        '\n' => output.push_str("\\n"),
        '\r' => output.push_str("\\r"),
        '\t' => output.push_str("\\t"),
        c if (c as u32) < 0x20 => {
            // Control character - use \uXXXX
            output.push_str("\\u00");
            const HEX: &[u8; 16] = b"0123456789abcdef";
            let b = c as u8;
            output.push(HEX[(b >> 4) as usize] as char);
            output.push(HEX[(b & 0xf) as usize] as char);
        }
        c if (c as u32) >= 0x80 && (c as u32) < 0x100 => {
            // Extended ASCII (0x80-0xFF) - write as \u00XX
            output.push_str("\\u00");
            const HEX: &[u8; 16] = b"0123456789abcdef";
            let b = c as u8;
            output.push(HEX[(b >> 4) as usize] as char);
            output.push(HEX[(b & 0xf) as usize] as char);
        }
        c if (c as u32) >= 0x100 => {
            // Unicode - write as \uXXXX or surrogate pair
            let cp = c as u32;
            if cp <= 0xFFFF {
                output.push_str("\\u");
                const HEX: &[u8; 16] = b"0123456789abcdef";
                output.push(HEX[((cp >> 12) & 0xF) as usize] as char);
                output.push(HEX[((cp >> 8) & 0xF) as usize] as char);
                output.push(HEX[((cp >> 4) & 0xF) as usize] as char);
                output.push(HEX[(cp & 0xF) as usize] as char);
            } else {
                // Surrogate pair for characters > 0xFFFF
                let adjusted = cp - 0x10000;
                let high = 0xD800 + (adjusted >> 10);
                let low = 0xDC00 + (adjusted & 0x3FF);
                output.push_str("\\u");
                const HEX: &[u8; 16] = b"0123456789abcdef";
                output.push(HEX[((high >> 12) & 0xF) as usize] as char);
                output.push(HEX[((high >> 8) & 0xF) as usize] as char);
                output.push(HEX[((high >> 4) & 0xF) as usize] as char);
                output.push(HEX[(high & 0xF) as usize] as char);
                output.push_str("\\u");
                output.push(HEX[((low >> 12) & 0xF) as usize] as char);
                output.push(HEX[((low >> 8) & 0xF) as usize] as char);
                output.push(HEX[((low >> 4) & 0xF) as usize] as char);
                output.push(HEX[(low & 0xF) as usize] as char);
            }
        }
        c => output.push(c),
    }
}

/// Transcode a double-quoted YAML string directly to JSON output.
/// Avoids intermediate String allocation by decoding YAML escapes and
/// re-encoding as JSON escapes in a single pass.
fn transcode_double_quoted_to_json(
    output: &mut String,
    bytes: &[u8],
) -> Result<(), YamlStringError> {
    output.push('"');
    let mut i = 0;

    while i < bytes.len() {
        match bytes[i] {
            b'\\' => {
                if i + 1 >= bytes.len() {
                    return Err(YamlStringError::InvalidEscape);
                }
                i += 1;
                match bytes[i] {
                    // YAML escapes that map directly to JSON escapes
                    b'n' => output.push_str("\\n"),
                    b'r' => output.push_str("\\r"),
                    b't' | b'\t' => output.push_str("\\t"),
                    b'"' => output.push_str("\\\""),
                    b'\\' => output.push_str("\\\\"),
                    b'/' => output.push('/'),
                    b' ' => output.push(' '),
                    // YAML escapes that need JSON \uXXXX encoding
                    b'0' => output.push_str("\\u0000"),
                    b'a' => output.push_str("\\u0007"), // bell
                    b'b' => output.push_str("\\u0008"), // backspace
                    b'v' => output.push_str("\\u000b"), // vertical tab
                    b'f' => output.push_str("\\u000c"), // form feed
                    b'e' => output.push_str("\\u001b"), // escape
                    b'N' => output.push_str("\\u0085"), // next line
                    b'_' => output.push_str("\\u00a0"), // non-breaking space
                    b'L' => output.push_str("\\u2028"), // line separator
                    b'P' => output.push_str("\\u2029"), // paragraph separator
                    b'\n' => {
                        // Escaped line break - skip entirely
                        i += 1;
                        while i < bytes.len() && matches!(bytes[i], b' ' | b'\t') {
                            i += 1;
                        }
                        continue;
                    }
                    b'\r' => {
                        // Escaped CRLF
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
                        if val < 0x20 || val == 0x22 || val == 0x5C {
                            // Control char, quote, or backslash - escape it
                            write_json_escape(
                                output,
                                char::from_u32(val as u32).unwrap_or('\u{FFFD}'),
                            );
                        } else if val <= 0x7F {
                            output.push(val as u8 as char);
                        } else {
                            write_json_escape(
                                output,
                                char::from_u32(val as u32).unwrap_or('\u{FFFD}'),
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
                        let ch = char::from_u32(codepoint).ok_or(YamlStringError::InvalidEscape)?;
                        write_json_escape(output, ch);
                        i += 4;
                    }
                    b'U' => {
                        // \UNNNNNNNN - 8 hex digits
                        if i + 8 >= bytes.len() {
                            return Err(YamlStringError::InvalidEscape);
                        }
                        let hex = &bytes[i + 1..i + 9];
                        let codepoint = parse_hex(hex)?;
                        let ch = char::from_u32(codepoint).ok_or(YamlStringError::InvalidEscape)?;
                        write_json_escape(output, ch);
                        i += 8;
                    }
                    _ => return Err(YamlStringError::InvalidEscape),
                }
                i += 1;
            }
            b'\r' | b'\n' => {
                // Line folding: handle newlines - fold to space or preserve empty lines
                i = transcode_fold_line_break_to_json(bytes, i, output);
            }
            b'"' => {
                // Quote inside string needs escaping
                output.push_str("\\\"");
                i += 1;
            }
            b if b < 0x20 => {
                // Control character needs escaping
                write_json_escape(output, b as char);
                i += 1;
            }
            _ => {
                // Regular content - copy until we hit special char
                let start = i;
                while i < bytes.len() {
                    let b = bytes[i];
                    if matches!(b, b'\\' | b'\n' | b'\r' | b'"') || b < 0x20 {
                        break;
                    }
                    i += 1;
                }
                // Copy the safe span
                let chunk = core::str::from_utf8(&bytes[start..i])
                    .map_err(|_| YamlStringError::InvalidUtf8)?;
                output.push_str(chunk);
            }
        }
    }

    output.push('"');
    Ok(())
}

/// Transcode a single-quoted YAML string directly to JSON output.
fn transcode_single_quoted_to_json(
    output: &mut String,
    bytes: &[u8],
) -> Result<(), YamlStringError> {
    output.push('"');
    let mut i = 0;

    while i < bytes.len() {
        match bytes[i] {
            b'\'' if i + 1 < bytes.len() && bytes[i + 1] == b'\'' => {
                // '' -> ' (no JSON escaping needed for single quote)
                output.push('\'');
                i += 2;
            }
            b'\r' | b'\n' => {
                // Line folding
                i = transcode_fold_line_break_to_json(bytes, i, output);
            }
            b'"' => {
                // Quote needs escaping in JSON
                output.push_str("\\\"");
                i += 1;
            }
            b'\\' => {
                // Backslash needs escaping in JSON
                output.push_str("\\\\");
                i += 1;
            }
            b if b < 0x20 => {
                // Control character needs escaping
                write_json_escape(output, b as char);
                i += 1;
            }
            _ => {
                // Regular content - copy until we hit special char
                let start = i;
                while i < bytes.len() {
                    let b = bytes[i];
                    if matches!(b, b'\n' | b'\r' | b'"' | b'\\') || b < 0x20 {
                        break;
                    }
                    // Check for '' escape
                    if b == b'\'' && i + 1 < bytes.len() && bytes[i + 1] == b'\'' {
                        break;
                    }
                    i += 1;
                }
                let chunk = core::str::from_utf8(&bytes[start..i])
                    .map_err(|_| YamlStringError::InvalidUtf8)?;
                output.push_str(chunk);
            }
        }
    }

    output.push('"');
    Ok(())
}

/// Handle line folding during direct transcoding.
/// Returns the new position after processing the line break(s).
fn transcode_fold_line_break_to_json(bytes: &[u8], mut i: usize, output: &mut String) -> usize {
    // Trim trailing whitespace from output
    while output.ends_with(' ') || output.ends_with('\t') {
        output.pop();
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

    // Count empty lines
    let mut empty_lines = 0;
    loop {
        // Skip whitespace at start of line
        while i < bytes.len() && matches!(bytes[i], b' ' | b'\t') {
            i += 1;
        }

        // Check if this is an empty line
        if i < bytes.len() && (bytes[i] == b'\n' || bytes[i] == b'\r') {
            empty_lines += 1;
            if bytes[i] == b'\r' {
                i += 1;
                if i < bytes.len() && bytes[i] == b'\n' {
                    i += 1;
                }
            } else {
                i += 1;
            }
        } else {
            break;
        }
    }

    // Output the folded result
    if empty_lines == 0 {
        // Single line break -> space
        output.push(' ');
    } else {
        // Empty lines -> newlines (per YAML spec, no leading space)
        for _ in 0..empty_lines {
            output.push_str("\\n");
        }
    }

    i
}

/// Write a YamlString directly to JSON output, avoiding intermediate allocation
/// for strings that need escape decoding.
///
/// Returns true if the string was written as a JSON string (quoted),
/// false if it should be treated as a scalar for type detection.
fn write_yaml_string_to_json(
    output: &mut String,
    s: &YamlString<'_>,
) -> Result<bool, YamlStringError> {
    match s {
        YamlString::DoubleQuoted { text, start } => {
            let end = YamlString::find_double_quote_end(text, *start);
            let bytes = &text[*start + 1..end - 1]; // Strip quotes

            // Check if we need decoding (has escapes or newlines)
            if !bytes.contains(&b'\\') && !bytes.contains(&b'\n') && !bytes.contains(&b'\r') {
                // Fast path: no decoding needed, just JSON-escape
                let s = core::str::from_utf8(bytes).map_err(|_| YamlStringError::InvalidUtf8)?;
                write_json_string(output, s);
            } else {
                // Transcode directly: YAML escapes → JSON escapes
                transcode_double_quoted_to_json(output, bytes)?;
            }
            Ok(true) // Always a string, no type detection
        }
        YamlString::SingleQuoted { text, start } => {
            let end = YamlString::find_single_quote_end(text, *start);
            let bytes = &text[*start + 1..end - 1]; // Strip quotes

            // Check if we need decoding (has '' or newlines)
            if !bytes.contains(&b'\'') && !bytes.contains(&b'\n') && !bytes.contains(&b'\r') {
                // Fast path: no decoding needed, just JSON-escape
                let s = core::str::from_utf8(bytes).map_err(|_| YamlStringError::InvalidUtf8)?;
                write_json_string(output, s);
            } else {
                // Transcode directly
                transcode_single_quoted_to_json(output, bytes)?;
            }
            Ok(true) // Always a string, no type detection
        }
        YamlString::Unquoted { .. }
        | YamlString::BlockLiteral { .. }
        | YamlString::BlockFolded { .. } => {
            // For unquoted and block scalars, we need type detection
            // Return false to signal caller should use the original path
            Ok(false)
        }
    }
}

/// Fast integer to string formatting without allocation.
/// Writes the integer directly to the output string.
#[inline]
fn write_i64(output: &mut String, mut n: i64) {
    if n == 0 {
        output.push('0');
        return;
    }

    if n < 0 {
        output.push('-');
        // Handle MIN value specially to avoid overflow
        if n == i64::MIN {
            output.push_str("9223372036854775808");
            return;
        }
        n = -n;
    }

    // Buffer for digits (max 19 digits for i64)
    let mut buf = [0u8; 20];
    let mut i = buf.len();

    while n > 0 {
        i -= 1;
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
    }

    // SAFETY: buf contains only ASCII digits
    output.push_str(unsafe { core::str::from_utf8_unchecked(&buf[i..]) });
}

/// Fast f64 to string formatting.
/// For simple cases, writes directly; falls back to to_string() for edge cases.
#[inline]
fn write_f64(output: &mut String, f: f64) {
    // Check for integer-like floats (no fractional part)
    if f.fract() == 0.0 && f.abs() < 9007199254740992.0 {
        // Can represent exactly as i64
        write_i64(output, f as i64);
    } else {
        // Fall back to standard formatting for non-integer floats
        output.push_str(&f.to_string());
    }
}

/// Fast YAML scalar to JSON conversion.
///
/// This function is optimized to quickly identify the type of a YAML scalar
/// by checking the first byte before doing more expensive parsing.
#[inline]
fn write_yaml_scalar_as_json(output: &mut String, str_val: &str) {
    let bytes = str_val.as_bytes();

    // Empty string -> null
    if bytes.is_empty() {
        output.push_str("null");
        return;
    }

    let first = bytes[0];

    // Fast path: check first byte to determine possible types
    match first {
        // Could be null or number
        b'n' => {
            if str_val == "null" {
                output.push_str("null");
                return;
            }
        }
        // Could be "~" (null)
        b'~' => {
            if bytes.len() == 1 {
                output.push_str("null");
                return;
            }
        }
        // Could be true/True/TRUE
        b't' => {
            if str_val == "true" {
                output.push_str("true");
                return;
            }
        }
        b'T' => {
            if str_val == "True" || str_val == "TRUE" {
                output.push_str("true");
                return;
            }
        }
        // Could be false/False/FALSE
        b'f' => {
            if str_val == "false" {
                output.push_str("false");
                return;
            }
        }
        b'F' => {
            if str_val == "False" || str_val == "FALSE" {
                output.push_str("false");
                return;
            }
        }
        // Could be .inf, .nan, etc.
        b'.' => match str_val {
            ".inf" | ".Inf" | ".INF" | ".nan" | ".NaN" | ".NAN" => {
                output.push_str("null");
                return;
            }
            _ => {}
        },
        // Could be negative number or -.inf
        b'-' => {
            if bytes.len() > 1 {
                match bytes[1] {
                    b'.' => {
                        if str_val == "-.inf" || str_val == "-.Inf" || str_val == "-.INF" {
                            output.push_str("null");
                            return;
                        }
                    }
                    b'0'..=b'9' => {
                        // Likely a negative number - try parsing
                        if let Ok(n) = str_val.parse::<i64>() {
                            write_i64(output, n);
                            return;
                        }
                        if let Ok(f) = str_val.parse::<f64>() {
                            if !f.is_nan() && !f.is_infinite() {
                                write_f64(output, f);
                                return;
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        // Digits - likely a number
        b'0'..=b'9' => {
            if let Ok(n) = str_val.parse::<i64>() {
                write_i64(output, n);
                return;
            }
            if let Ok(f) = str_val.parse::<f64>() {
                if !f.is_nan() && !f.is_infinite() {
                    write_f64(output, f);
                    return;
                }
            }
        }
        // Could be +number or +.inf
        b'+' => {
            if bytes.len() > 1 && bytes[1].is_ascii_digit() {
                if let Ok(n) = str_val.parse::<i64>() {
                    write_i64(output, n);
                    return;
                }
                if let Ok(f) = str_val.parse::<f64>() {
                    if !f.is_nan() && !f.is_infinite() {
                        write_f64(output, f);
                        return;
                    }
                }
            }
        }
        _ => {}
    }

    // It's a string - write with JSON escaping
    write_json_string(output, str_val);
}

// ============================================================================
// Streaming JSON Output (core::fmt::Write based)
// ============================================================================

/// Stream a YAML value as JSON (for sequence elements where we don't have a cursor).
fn stream_yaml_value_as_json<W: AsRef<[u64]>, Out: core::fmt::Write>(
    out: &mut Out,
    value: YamlValue<'_, W>,
) -> core::fmt::Result {
    match value {
        YamlValue::Null => out.write_str("null"),
        YamlValue::String(s) => match stream_yaml_string_to_json(out, &s) {
            Ok(true) => Ok(()),
            Ok(false) => {
                if let Ok(str_val) = s.as_str() {
                    stream_yaml_scalar_as_json(out, &str_val)
                } else {
                    out.write_str("null")
                }
            }
            Err(_) => out.write_str("null"),
        },
        YamlValue::Mapping(fields) => {
            out.write_char('{')?;
            let mut first = true;
            for field in fields {
                if !first {
                    out.write_char(',')?;
                }
                first = false;

                if let YamlValue::String(s) = field.key() {
                    match stream_yaml_string_to_json(out, &s) {
                        Ok(true) => {}
                        Ok(false) | Err(_) => {
                            if let Ok(key_str) = s.as_str() {
                                stream_json_string(out, &key_str)?;
                            } else {
                                out.write_str("\"\"")?;
                            }
                        }
                    }
                } else {
                    out.write_str("\"\"")?;
                }

                out.write_char(':')?;
                field.value_cursor().stream_json_value(out)?;
            }
            out.write_char('}')
        }
        YamlValue::Sequence(elements) => {
            out.write_char('[')?;
            let mut first = true;
            for elem in elements {
                if !first {
                    out.write_char(',')?;
                }
                first = false;
                stream_yaml_value_as_json(out, elem)?;
            }
            out.write_char(']')
        }
        YamlValue::Alias { target, .. } => {
            if let Some(target_cursor) = target {
                target_cursor.stream_json_value(out)
            } else {
                out.write_str("null")
            }
        }
        YamlValue::Error(_) => out.write_str("null"),
    }
}

/// Stream a string with JSON escaping.
fn stream_json_string<Out: core::fmt::Write>(out: &mut Out, s: &str) -> core::fmt::Result {
    out.write_char('"')?;

    let bytes = s.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        let start = i;
        while i < bytes.len() {
            let b = bytes[i];
            if b == b'"' || b == b'\\' || b < 0x20 {
                break;
            }
            i += 1;
        }

        if start < i {
            out.write_str(&s[start..i])?;
        }

        if i < bytes.len() {
            let b = bytes[i];
            match b {
                b'"' => out.write_str("\\\"")?,
                b'\\' => out.write_str("\\\\")?,
                b'\n' => out.write_str("\\n")?,
                b'\r' => out.write_str("\\r")?,
                b'\t' => out.write_str("\\t")?,
                b if b < 0x20 => {
                    out.write_str("\\u00")?;
                    const HEX: &[u8; 16] = b"0123456789abcdef";
                    out.write_char(HEX[(b >> 4) as usize] as char)?;
                    out.write_char(HEX[(b & 0xf) as usize] as char)?;
                }
                _ => unreachable!(),
            }
            i += 1;
        }
    }

    out.write_char('"')
}

/// Stream a JSON character escape sequence.
#[inline]
fn stream_json_escape<Out: core::fmt::Write>(out: &mut Out, ch: char) -> core::fmt::Result {
    match ch {
        '"' => out.write_str("\\\""),
        '\\' => out.write_str("\\\\"),
        '\n' => out.write_str("\\n"),
        '\r' => out.write_str("\\r"),
        '\t' => out.write_str("\\t"),
        c if (c as u32) < 0x20 => {
            out.write_str("\\u00")?;
            const HEX: &[u8; 16] = b"0123456789abcdef";
            let b = c as u8;
            out.write_char(HEX[(b >> 4) as usize] as char)?;
            out.write_char(HEX[(b & 0xf) as usize] as char)
        }
        c if (c as u32) >= 0x80 && (c as u32) < 0x100 => {
            out.write_str("\\u00")?;
            const HEX: &[u8; 16] = b"0123456789abcdef";
            let b = c as u8;
            out.write_char(HEX[(b >> 4) as usize] as char)?;
            out.write_char(HEX[(b & 0xf) as usize] as char)
        }
        c if (c as u32) >= 0x100 => {
            let cp = c as u32;
            if cp <= 0xFFFF {
                out.write_str("\\u")?;
                const HEX: &[u8; 16] = b"0123456789abcdef";
                out.write_char(HEX[((cp >> 12) & 0xF) as usize] as char)?;
                out.write_char(HEX[((cp >> 8) & 0xF) as usize] as char)?;
                out.write_char(HEX[((cp >> 4) & 0xF) as usize] as char)?;
                out.write_char(HEX[(cp & 0xF) as usize] as char)
            } else {
                // Surrogate pair
                let adjusted = cp - 0x10000;
                let high = 0xD800 + (adjusted >> 10);
                let low = 0xDC00 + (adjusted & 0x3FF);
                out.write_str("\\u")?;
                const HEX: &[u8; 16] = b"0123456789abcdef";
                out.write_char(HEX[((high >> 12) & 0xF) as usize] as char)?;
                out.write_char(HEX[((high >> 8) & 0xF) as usize] as char)?;
                out.write_char(HEX[((high >> 4) & 0xF) as usize] as char)?;
                out.write_char(HEX[(high & 0xF) as usize] as char)?;
                out.write_str("\\u")?;
                out.write_char(HEX[((low >> 12) & 0xF) as usize] as char)?;
                out.write_char(HEX[((low >> 8) & 0xF) as usize] as char)?;
                out.write_char(HEX[((low >> 4) & 0xF) as usize] as char)?;
                out.write_char(HEX[(low & 0xF) as usize] as char)
            }
        }
        c => out.write_char(c),
    }
}

/// Stream transcode a double-quoted YAML string to JSON.
fn stream_transcode_double_quoted_to_json<Out: core::fmt::Write>(
    out: &mut Out,
    bytes: &[u8],
) -> Result<(), YamlStringError> {
    out.write_char('"')
        .map_err(|_| YamlStringError::InvalidUtf8)?;
    let mut i = 0;

    while i < bytes.len() {
        match bytes[i] {
            b'\\' => {
                if i + 1 >= bytes.len() {
                    return Err(YamlStringError::InvalidEscape);
                }
                i += 1;
                match bytes[i] {
                    b'n' => out
                        .write_str("\\n")
                        .map_err(|_| YamlStringError::InvalidUtf8)?,
                    b'r' => out
                        .write_str("\\r")
                        .map_err(|_| YamlStringError::InvalidUtf8)?,
                    b't' | b'\t' => out
                        .write_str("\\t")
                        .map_err(|_| YamlStringError::InvalidUtf8)?,
                    b'"' => out
                        .write_str("\\\"")
                        .map_err(|_| YamlStringError::InvalidUtf8)?,
                    b'\\' => out
                        .write_str("\\\\")
                        .map_err(|_| YamlStringError::InvalidUtf8)?,
                    b'/' => out
                        .write_char('/')
                        .map_err(|_| YamlStringError::InvalidUtf8)?,
                    b' ' => out
                        .write_char(' ')
                        .map_err(|_| YamlStringError::InvalidUtf8)?,
                    b'0' => out
                        .write_str("\\u0000")
                        .map_err(|_| YamlStringError::InvalidUtf8)?,
                    b'a' => out
                        .write_str("\\u0007")
                        .map_err(|_| YamlStringError::InvalidUtf8)?,
                    b'b' => out
                        .write_str("\\u0008")
                        .map_err(|_| YamlStringError::InvalidUtf8)?,
                    b'v' => out
                        .write_str("\\u000b")
                        .map_err(|_| YamlStringError::InvalidUtf8)?,
                    b'f' => out
                        .write_str("\\u000c")
                        .map_err(|_| YamlStringError::InvalidUtf8)?,
                    b'e' => out
                        .write_str("\\u001b")
                        .map_err(|_| YamlStringError::InvalidUtf8)?,
                    b'N' => out
                        .write_str("\\u0085")
                        .map_err(|_| YamlStringError::InvalidUtf8)?,
                    b'_' => out
                        .write_str("\\u00a0")
                        .map_err(|_| YamlStringError::InvalidUtf8)?,
                    b'L' => out
                        .write_str("\\u2028")
                        .map_err(|_| YamlStringError::InvalidUtf8)?,
                    b'P' => out
                        .write_str("\\u2029")
                        .map_err(|_| YamlStringError::InvalidUtf8)?,
                    b'\n' => {
                        i += 1;
                        while i < bytes.len() && matches!(bytes[i], b' ' | b'\t') {
                            i += 1;
                        }
                        continue;
                    }
                    b'\r' => {
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
                        if i + 2 >= bytes.len() {
                            return Err(YamlStringError::InvalidEscape);
                        }
                        let hex = &bytes[i + 1..i + 3];
                        let val = parse_hex(hex)?;
                        let ch = char::from_u32(val as u32).unwrap_or('\u{FFFD}');
                        if val < 0x20 || val == 0x22 || val == 0x5C {
                            stream_json_escape(out, ch)
                                .map_err(|_| YamlStringError::InvalidUtf8)?;
                        } else if val <= 0x7F {
                            out.write_char(val as u8 as char)
                                .map_err(|_| YamlStringError::InvalidUtf8)?;
                        } else {
                            stream_json_escape(out, ch)
                                .map_err(|_| YamlStringError::InvalidUtf8)?;
                        }
                        i += 2;
                    }
                    b'u' => {
                        if i + 4 >= bytes.len() {
                            return Err(YamlStringError::InvalidEscape);
                        }
                        let hex = &bytes[i + 1..i + 5];
                        let codepoint = parse_hex(hex)? as u32;
                        let ch = char::from_u32(codepoint).ok_or(YamlStringError::InvalidEscape)?;
                        stream_json_escape(out, ch).map_err(|_| YamlStringError::InvalidUtf8)?;
                        i += 4;
                    }
                    b'U' => {
                        if i + 8 >= bytes.len() {
                            return Err(YamlStringError::InvalidEscape);
                        }
                        let hex = &bytes[i + 1..i + 9];
                        let codepoint = parse_hex(hex)?;
                        let ch = char::from_u32(codepoint).ok_or(YamlStringError::InvalidEscape)?;
                        stream_json_escape(out, ch).map_err(|_| YamlStringError::InvalidUtf8)?;
                        i += 8;
                    }
                    _ => return Err(YamlStringError::InvalidEscape),
                }
                i += 1;
            }
            b'\r' | b'\n' => {
                i = stream_transcode_fold_line_break(bytes, i, out)?;
            }
            b'"' => {
                out.write_str("\\\"")
                    .map_err(|_| YamlStringError::InvalidUtf8)?;
                i += 1;
            }
            b if b < 0x20 => {
                stream_json_escape(out, b as char).map_err(|_| YamlStringError::InvalidUtf8)?;
                i += 1;
            }
            _ => {
                let start = i;
                while i < bytes.len() {
                    let b = bytes[i];
                    if matches!(b, b'\\' | b'\n' | b'\r' | b'"') || b < 0x20 {
                        break;
                    }
                    i += 1;
                }
                let chunk = core::str::from_utf8(&bytes[start..i])
                    .map_err(|_| YamlStringError::InvalidUtf8)?;
                out.write_str(chunk)
                    .map_err(|_| YamlStringError::InvalidUtf8)?;
            }
        }
    }

    out.write_char('"')
        .map_err(|_| YamlStringError::InvalidUtf8)?;
    Ok(())
}

/// Stream transcode a single-quoted YAML string to JSON.
fn stream_transcode_single_quoted_to_json<Out: core::fmt::Write>(
    out: &mut Out,
    bytes: &[u8],
) -> Result<(), YamlStringError> {
    out.write_char('"')
        .map_err(|_| YamlStringError::InvalidUtf8)?;
    let mut i = 0;

    while i < bytes.len() {
        match bytes[i] {
            b'\'' if i + 1 < bytes.len() && bytes[i + 1] == b'\'' => {
                out.write_char('\'')
                    .map_err(|_| YamlStringError::InvalidUtf8)?;
                i += 2;
            }
            b'\r' | b'\n' => {
                i = stream_transcode_fold_line_break(bytes, i, out)?;
            }
            b'"' => {
                out.write_str("\\\"")
                    .map_err(|_| YamlStringError::InvalidUtf8)?;
                i += 1;
            }
            b'\\' => {
                out.write_str("\\\\")
                    .map_err(|_| YamlStringError::InvalidUtf8)?;
                i += 1;
            }
            b if b < 0x20 => {
                stream_json_escape(out, b as char).map_err(|_| YamlStringError::InvalidUtf8)?;
                i += 1;
            }
            _ => {
                let start = i;
                while i < bytes.len() {
                    let b = bytes[i];
                    if matches!(b, b'\'' | b'\n' | b'\r' | b'"' | b'\\') || b < 0x20 {
                        break;
                    }
                    i += 1;
                }
                let chunk = core::str::from_utf8(&bytes[start..i])
                    .map_err(|_| YamlStringError::InvalidUtf8)?;
                out.write_str(chunk)
                    .map_err(|_| YamlStringError::InvalidUtf8)?;
            }
        }
    }

    out.write_char('"')
        .map_err(|_| YamlStringError::InvalidUtf8)?;
    Ok(())
}

/// Stream fold line break for quoted strings.
fn stream_transcode_fold_line_break<Out: core::fmt::Write>(
    bytes: &[u8],
    mut i: usize,
    out: &mut Out,
) -> Result<usize, YamlStringError> {
    let mut newlines = 0;

    while i < bytes.len() && matches!(bytes[i], b'\r' | b'\n') {
        if bytes[i] == b'\r' {
            i += 1;
            if i < bytes.len() && bytes[i] == b'\n' {
                i += 1;
            }
        } else {
            i += 1;
        }
        newlines += 1;

        while i < bytes.len() && matches!(bytes[i], b' ' | b'\t') {
            i += 1;
        }
    }

    if newlines == 1 {
        out.write_char(' ')
            .map_err(|_| YamlStringError::InvalidUtf8)?;
    } else {
        for _ in 1..newlines {
            out.write_str("\\n")
                .map_err(|_| YamlStringError::InvalidUtf8)?;
        }
    }

    Ok(i)
}

/// Stream YAML string to JSON output.
/// Returns true if written as a quoted string, false if type detection needed.
fn stream_yaml_string_to_json<Out: core::fmt::Write>(
    out: &mut Out,
    s: &YamlString<'_>,
) -> Result<bool, YamlStringError> {
    match s {
        YamlString::DoubleQuoted { text, start } => {
            let end = YamlString::find_double_quote_end(text, *start);
            let bytes = &text[*start + 1..end - 1];

            if !bytes.contains(&b'\\') && !bytes.contains(&b'\n') && !bytes.contains(&b'\r') {
                let s = core::str::from_utf8(bytes).map_err(|_| YamlStringError::InvalidUtf8)?;
                stream_json_string(out, s).map_err(|_| YamlStringError::InvalidUtf8)?;
            } else {
                stream_transcode_double_quoted_to_json(out, bytes)?;
            }
            Ok(true)
        }
        YamlString::SingleQuoted { text, start } => {
            let end = YamlString::find_single_quote_end(text, *start);
            let bytes = &text[*start + 1..end - 1];

            if !bytes.contains(&b'\'') && !bytes.contains(&b'\n') && !bytes.contains(&b'\r') {
                let s = core::str::from_utf8(bytes).map_err(|_| YamlStringError::InvalidUtf8)?;
                stream_json_string(out, s).map_err(|_| YamlStringError::InvalidUtf8)?;
            } else {
                stream_transcode_single_quoted_to_json(out, bytes)?;
            }
            Ok(true)
        }
        YamlString::Unquoted { .. }
        | YamlString::BlockLiteral { .. }
        | YamlString::BlockFolded { .. } => Ok(false),
    }
}

/// Stream YAML scalar as JSON with type detection.
fn stream_yaml_scalar_as_json<Out: core::fmt::Write>(
    out: &mut Out,
    str_val: &str,
) -> core::fmt::Result {
    let bytes = str_val.as_bytes();

    if bytes.is_empty() {
        return out.write_str("null");
    }

    let first = bytes[0];

    match first {
        b'n' => {
            if str_val == "null" {
                return out.write_str("null");
            }
        }
        b'~' => {
            if bytes.len() == 1 {
                return out.write_str("null");
            }
        }
        b't' => {
            if str_val == "true" {
                return out.write_str("true");
            }
        }
        b'T' => {
            if str_val == "True" || str_val == "TRUE" {
                return out.write_str("true");
            }
        }
        b'f' => {
            if str_val == "false" {
                return out.write_str("false");
            }
        }
        b'F' => {
            if str_val == "False" || str_val == "FALSE" {
                return out.write_str("false");
            }
        }
        b'.' => match str_val {
            ".inf" | ".Inf" | ".INF" | ".nan" | ".NaN" | ".NAN" => {
                return out.write_str("null");
            }
            _ => {}
        },
        b'-' => {
            if bytes.len() > 1 {
                match bytes[1] {
                    b'.' => {
                        if str_val == "-.inf" || str_val == "-.Inf" || str_val == "-.INF" {
                            return out.write_str("null");
                        }
                    }
                    b'0'..=b'9' => {
                        if let Ok(n) = str_val.parse::<i64>() {
                            return write!(out, "{}", n);
                        }
                        if let Ok(f) = str_val.parse::<f64>() {
                            if !f.is_nan() && !f.is_infinite() {
                                return write!(out, "{}", f);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        b'0'..=b'9' => {
            if let Ok(n) = str_val.parse::<i64>() {
                return write!(out, "{}", n);
            }
            if let Ok(f) = str_val.parse::<f64>() {
                if !f.is_nan() && !f.is_infinite() {
                    return write!(out, "{}", f);
                }
            }
        }
        b'+' => {
            if bytes.len() > 1 && bytes[1].is_ascii_digit() {
                if let Ok(n) = str_val.parse::<i64>() {
                    return write!(out, "{}", n);
                }
                if let Ok(f) = str_val.parse::<f64>() {
                    if !f.is_nan() && !f.is_infinite() {
                        return write!(out, "{}", f);
                    }
                }
            }
        }
        _ => {}
    }

    stream_json_string(out, str_val)
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

    /// Get the cursor for the first element and the remaining elements.
    ///
    /// This returns the cursor directly, which is useful when you need to
    /// call cursor methods like `to_json()` without going through YamlValue.
    pub fn uncons_cursor(&self) -> Option<(YamlCursor<'a, W>, YamlElements<'a, W>)> {
        let element_cursor = self.element_cursor?;

        let rest = YamlElements {
            element_cursor: element_cursor.next_sibling(),
        };

        // For block sequences, navigate to the actual value cursor
        if element_cursor.is_container() {
            Some((element_cursor, rest))
        } else if let Some(text_pos) = element_cursor.text_position() {
            if text_pos < element_cursor.text.len()
                && element_cursor.text[text_pos] == b'-'
                && text_pos + 1 < element_cursor.text.len()
                && (element_cursor.text[text_pos + 1] == b' '
                    || element_cursor.text[text_pos + 1] == b'\t')
            {
                // Block sequence item - return child cursor if exists
                if let Some(child) = element_cursor.first_child() {
                    Some((child, rest))
                } else {
                    // Empty item - return element cursor (will produce null)
                    Some((element_cursor, rest))
                }
            } else {
                Some((element_cursor, rest))
            }
        } else {
            Some((element_cursor, rest))
        }
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
                && (element_cursor.text[text_pos + 1] == b' '
                    || element_cursor.text[text_pos + 1] == b'\t')
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
                // EOF after spaces - if indent >= content_indent, these spaces are content
                // (a line with only spaces that ends at EOF)
                if line_indent >= content_indent {
                    // The spaces themselves are content (after stripping indent)
                    // pos is at EOF, last_content_end should be set to include this line
                    last_content_end = pos;
                    has_content = true;
                }
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
            // Check if followed by space or tab (block sequence indicator)
            if pos + 1 < text.len()
                && (text[pos + 1] == b' ' || text[pos + 1] == b'\t' || text[pos + 1] == b'\n')
            {
                // Check if there's a `:` between `-` and the indicator
                // If so, it's `- key: |` and we should return line_indent + 2
                // If not, it's `- |` and we should return line_indent
                let has_colon = text
                    .get((pos + 2)..indicator_pos)
                    .is_some_and(|slice| slice.contains(&b':'));
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
        // Empty block scalar - but with keep chomping, we should preserve
        // the implicit final line break (YAML spec section 8.1.1.2).
        // Even when the block has no content, there's an implicit line break
        // at the end of the indicator line that keep chomping preserves.
        if chomping == ChompingIndicator::Keep {
            return Ok(Cow::Borrowed("\n"));
        }
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
        // Empty block scalar - but with keep chomping, we should preserve
        // the implicit final line break (YAML spec section 8.1.1.2).
        if chomping == ChompingIndicator::Keep {
            return Ok(Cow::Borrowed("\n"));
        }
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

// ============================================================================
// Document trait implementations
// ============================================================================

use crate::jq::document::{
    DocumentCursor, DocumentElements, DocumentField, DocumentFields, DocumentValue,
};

impl<'a, W: AsRef<[u64]> + Clone> DocumentCursor for YamlCursor<'a, W> {
    type Value = YamlValue<'a, W>;

    #[inline]
    fn value(&self) -> Self::Value {
        YamlCursor::value(self)
    }

    #[inline]
    fn first_child(&self) -> Option<Self> {
        YamlCursor::first_child(self)
    }

    #[inline]
    fn next_sibling(&self) -> Option<Self> {
        YamlCursor::next_sibling(self)
    }

    #[inline]
    fn parent(&self) -> Option<Self> {
        YamlCursor::parent(self)
    }

    #[inline]
    fn is_container(&self) -> bool {
        YamlCursor::is_container(self)
    }

    #[inline]
    fn text_position(&self) -> Option<usize> {
        YamlCursor::text_position(self)
    }

    #[inline]
    fn line(&self) -> usize {
        YamlCursor::line(self)
    }

    #[inline]
    fn column(&self) -> usize {
        YamlCursor::column(self)
    }

    #[inline]
    fn document_index(&self) -> Option<usize> {
        YamlCursor::document_index(self)
    }

    #[inline]
    fn cursor_at_offset(&self, offset: usize) -> Option<Self> {
        YamlCursor::cursor_at_offset(self, offset)
    }

    #[inline]
    fn cursor_at_position(&self, line: usize, col: usize) -> Option<Self> {
        YamlCursor::cursor_at_position(self, line, col)
    }

    #[inline]
    fn stream_json<Out: core::fmt::Write>(&self, out: &mut Out) -> core::fmt::Result {
        YamlCursor::stream_json(self, out)
    }

    #[inline]
    fn stream_yaml<Out: core::fmt::Write>(
        &self,
        out: &mut Out,
        indent_spaces: usize,
    ) -> core::fmt::Result {
        YamlCursor::stream_yaml(self, out, indent_spaces)
    }

    #[inline]
    fn is_falsy(&self) -> bool {
        // A value is falsy if it's null or false
        match self.value() {
            YamlValue::Null => true,
            YamlValue::String(s) if s.is_unquoted() => {
                if let Ok(str_val) = s.as_str() {
                    matches!(
                        str_val.as_ref(),
                        "null" | "~" | "" | "false" | "False" | "FALSE"
                    )
                } else {
                    false
                }
            }
            _ => false,
        }
    }
}

impl<'a, W: AsRef<[u64]> + Clone> DocumentValue for YamlValue<'a, W> {
    type Cursor = YamlCursor<'a, W>;
    type Fields = YamlFields<'a, W>;
    type Elements = YamlElements<'a, W>;

    fn is_null(&self) -> bool {
        match self {
            YamlValue::Null => true,
            YamlValue::String(s) if s.is_unquoted() => {
                // YAML null values: null, ~, empty string
                if let Ok(str_val) = s.as_str() {
                    matches!(str_val.as_ref(), "null" | "~" | "")
                } else {
                    false
                }
            }
            YamlValue::Alias { target, .. } => {
                // Resolve alias and check target
                target.map(|t| t.value().is_null()).unwrap_or(true) // Unresolved alias treated as null
            }
            _ => false,
        }
    }

    fn as_bool(&self) -> Option<bool> {
        match self {
            YamlValue::String(s) if s.is_unquoted() => {
                let str_val = s.as_str().ok()?;
                match str_val.as_ref() {
                    "true" | "True" | "TRUE" => Some(true),
                    "false" | "False" | "FALSE" => Some(false),
                    _ => None,
                }
            }
            YamlValue::Alias { target, .. } => target.and_then(|t| t.value().as_bool()),
            _ => None,
        }
    }

    fn as_i64(&self) -> Option<i64> {
        match self {
            YamlValue::String(s) if s.is_unquoted() => {
                let str_val = s.as_str().ok()?;
                str_val.parse().ok()
            }
            YamlValue::Alias { target, .. } => target.and_then(|t| t.value().as_i64()),
            _ => None,
        }
    }

    fn as_f64(&self) -> Option<f64> {
        match self {
            YamlValue::String(s) if s.is_unquoted() => {
                let str_val = s.as_str().ok()?;
                // Handle special YAML float values
                match str_val.as_ref() {
                    ".inf" | ".Inf" | ".INF" => Some(f64::INFINITY),
                    "-.inf" | "-.Inf" | "-.INF" => Some(f64::NEG_INFINITY),
                    ".nan" | ".NaN" | ".NAN" => Some(f64::NAN),
                    _ => str_val.parse().ok(),
                }
            }
            YamlValue::Alias { target, .. } => target.and_then(|t| t.value().as_f64()),
            _ => None,
        }
    }

    fn as_str(&self) -> Option<Cow<'_, str>> {
        match self {
            YamlValue::String(s) => s.as_str().ok(),
            YamlValue::Alias { target, .. } => {
                // For aliases, we need to return an owned string since the
                // target value is created temporarily
                target.and_then(|t| {
                    if let YamlValue::String(s) = t.value() {
                        s.as_str().ok().map(|cow| Cow::Owned(cow.into_owned()))
                    } else {
                        None
                    }
                })
            }
            _ => None,
        }
    }

    fn as_object(&self) -> Option<Self::Fields> {
        match self {
            YamlValue::Mapping(fields) => Some(*fields),
            YamlValue::Alias { target, .. } => target.and_then(|t| t.value().as_object()),
            _ => None,
        }
    }

    fn as_array(&self) -> Option<Self::Elements> {
        match self {
            YamlValue::Sequence(elements) => Some(*elements),
            YamlValue::Alias { target, .. } => target.and_then(|t| t.value().as_array()),
            _ => None,
        }
    }

    fn type_name(&self) -> &'static str {
        match self {
            YamlValue::Null => "null",
            YamlValue::String(s) => {
                // Determine effective type based on content
                if s.is_unquoted() {
                    if let Ok(str_val) = s.as_str() {
                        match str_val.as_ref() {
                            "null" | "~" | "" => return "null",
                            "true" | "True" | "TRUE" | "false" | "False" | "FALSE" => {
                                return "boolean"
                            }
                            _ => {
                                // Check if it's a number
                                if str_val.parse::<i64>().is_ok() || str_val.parse::<f64>().is_ok()
                                {
                                    return "number";
                                }
                            }
                        }
                    }
                }
                "string"
            }
            YamlValue::Mapping(_) => "object",
            YamlValue::Sequence(_) => "array",
            YamlValue::Alias { target, .. } => {
                target.map(|t| t.value().type_name()).unwrap_or("null")
            }
            YamlValue::Error(_) => "error",
        }
    }

    fn is_error(&self) -> bool {
        matches!(self, YamlValue::Error(_))
    }

    fn error_message(&self) -> Option<&'static str> {
        match self {
            YamlValue::Error(msg) => Some(msg),
            _ => None,
        }
    }
}

impl<'a, W: AsRef<[u64]> + Clone> DocumentFields for YamlFields<'a, W> {
    type Value = YamlValue<'a, W>;
    type Cursor = YamlCursor<'a, W>;

    fn uncons(&self) -> Option<(DocumentField<Self::Value, Self::Cursor>, Self)> {
        let (field, rest) = YamlFields::uncons(self)?;
        Some((
            DocumentField {
                key: field.key(),
                value: field.value(),
                value_cursor: field.value_cursor(),
            },
            rest,
        ))
    }

    fn find(&self, name: &str) -> Option<Self::Value> {
        YamlFields::find(self, name)
    }

    fn is_empty(&self) -> bool {
        YamlFields::is_empty(self)
    }
}

impl<'a, W: AsRef<[u64]> + Clone> DocumentElements for YamlElements<'a, W> {
    type Value = YamlValue<'a, W>;
    type Cursor = YamlCursor<'a, W>;

    fn uncons(&self) -> Option<(Self::Value, Self)> {
        YamlElements::uncons(self)
    }

    fn uncons_cursor(&self) -> Option<(Self::Cursor, Self)> {
        YamlElements::uncons_cursor(self)
    }

    fn get(&self, index: usize) -> Option<Self::Value> {
        YamlElements::get(self, index)
    }

    fn is_empty(&self) -> bool {
        YamlElements::is_empty(self)
    }
}

// ============================================================================
// YAML Streaming Helpers
// ============================================================================

/// Check if a cursor points to a non-empty container.
fn is_yaml_cursor_container<W: AsRef<[u64]>>(cursor: &YamlCursor<'_, W>) -> bool {
    match cursor.value() {
        YamlValue::Mapping(fields) => !fields.is_empty(),
        YamlValue::Sequence(elements) => !elements.is_empty(),
        _ => false,
    }
}

/// Write indentation spaces.
fn write_yaml_indent<Out: core::fmt::Write>(out: &mut Out, spaces: usize) -> core::fmt::Result {
    for _ in 0..spaces {
        out.write_char(' ')?;
    }
    Ok(())
}

/// Stream a YAML string value with smart quoting.
fn stream_yaml_string_value<Out: core::fmt::Write>(
    out: &mut Out,
    s: &YamlString<'_>,
) -> core::fmt::Result {
    // Try to get the string value
    let str_val = match s.as_str() {
        Ok(v) => v,
        Err(_) => return out.write_str("\"\""),
    };

    // For quoted strings, preserve the quoting style
    match s {
        YamlString::DoubleQuoted { .. } => stream_yaml_double_quoted(out, &str_val),
        YamlString::SingleQuoted { .. } => stream_yaml_single_quoted(out, &str_val),
        _ => {
            // Unquoted or block - use smart quoting based on content
            if needs_yaml_quoting(&str_val) {
                stream_yaml_double_quoted(out, &str_val)
            } else {
                out.write_str(&str_val)
            }
        }
    }
}

/// Check if a string needs quoting in YAML.
fn needs_yaml_quoting(s: &str) -> bool {
    if s.is_empty() {
        return true;
    }

    let bytes = s.as_bytes();

    // Check first character - indicators that require quoting
    let first = bytes[0];
    if matches!(
        first,
        b'-' | b'?'
            | b':'
            | b','
            | b'['
            | b']'
            | b'{'
            | b'}'
            | b'#'
            | b'&'
            | b'*'
            | b'!'
            | b'|'
            | b'>'
            | b'\''
            | b'"'
            | b'%'
            | b'@'
            | b'`'
    ) {
        return true;
    }

    // Check for leading/trailing whitespace
    if bytes[0] == b' ' || bytes[bytes.len() - 1] == b' ' {
        return true;
    }

    // Check for special values that look like YAML keywords
    let lower = s.to_lowercase();
    if matches!(
        lower.as_str(),
        "null" | "~" | "true" | "false" | "yes" | "no" | "on" | "off" | ".inf" | "-.inf" | ".nan"
    ) {
        return true;
    }

    // Check if it looks like a number
    if looks_like_yaml_number(s) {
        return true;
    }

    // Check for characters that need escaping
    for b in bytes {
        if *b < 0x20 || *b == b':' || *b == b'#' {
            return true;
        }
    }

    false
}

/// Check if a string looks like a number.
fn looks_like_yaml_number(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }

    let bytes = s.as_bytes();
    let mut i = 0;

    // Optional sign
    if bytes[i] == b'-' || bytes[i] == b'+' {
        i += 1;
        if i >= bytes.len() {
            return false;
        }
    }

    // Must have at least one digit
    if !bytes[i].is_ascii_digit() {
        return false;
    }

    // Check remaining characters
    let mut has_dot = false;
    let mut has_exp = false;
    while i < bytes.len() {
        match bytes[i] {
            b'0'..=b'9' => {}
            b'.' if !has_dot && !has_exp => has_dot = true,
            b'e' | b'E' if !has_exp => {
                has_exp = true;
                // Optional sign after exponent
                if i + 1 < bytes.len() && (bytes[i + 1] == b'-' || bytes[i + 1] == b'+') {
                    i += 1;
                }
            }
            _ => return false,
        }
        i += 1;
    }

    true
}

/// Stream a double-quoted YAML string with proper escaping.
fn stream_yaml_double_quoted<Out: core::fmt::Write>(out: &mut Out, s: &str) -> core::fmt::Result {
    out.write_char('"')?;

    for ch in s.chars() {
        match ch {
            '"' => out.write_str("\\\"")?,
            '\\' => out.write_str("\\\\")?,
            '\n' => out.write_str("\\n")?,
            '\r' => out.write_str("\\r")?,
            '\t' => out.write_str("\\t")?,
            c if (c as u32) < 0x20 => {
                // Control characters as \xNN
                let b = c as u8;
                out.write_str("\\x")?;
                const HEX: &[u8; 16] = b"0123456789abcdef";
                out.write_char(HEX[(b >> 4) as usize] as char)?;
                out.write_char(HEX[(b & 0xf) as usize] as char)?;
            }
            c => out.write_char(c)?,
        }
    }

    out.write_char('"')
}

/// Stream a single-quoted YAML string with proper escaping.
fn stream_yaml_single_quoted<Out: core::fmt::Write>(out: &mut Out, s: &str) -> core::fmt::Result {
    out.write_char('\'')?;

    for ch in s.chars() {
        if ch == '\'' {
            // Single quotes are escaped by doubling
            out.write_str("''")?;
        } else {
            out.write_char(ch)?;
        }
    }

    out.write_char('\'')
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
    // Comprehensive as_str() escape tests (old decode path)
    // =========================================================================
    // These tests verify the decode_double_quoted and decode_single_quoted
    // functions handle all YAML escape sequences correctly.

    #[test]
    fn test_decode_double_quoted_tab_escape() {
        let s = YamlString::DoubleQuoted {
            text: b"\"hello\\tworld\"",
            start: 0,
        };
        assert_eq!(&*s.as_str().unwrap(), "hello\tworld");
    }

    #[test]
    fn test_decode_double_quoted_carriage_return_escape() {
        let s = YamlString::DoubleQuoted {
            text: b"\"line\\rbreak\"",
            start: 0,
        };
        assert_eq!(&*s.as_str().unwrap(), "line\rbreak");
    }

    #[test]
    fn test_decode_double_quoted_backslash_escape() {
        let s = YamlString::DoubleQuoted {
            text: b"\"path\\\\to\\\\file\"",
            start: 0,
        };
        assert_eq!(&*s.as_str().unwrap(), "path\\to\\file");
    }

    #[test]
    fn test_decode_double_quoted_quote_escape() {
        let s = YamlString::DoubleQuoted {
            text: b"\"say \\\"hello\\\"\"",
            start: 0,
        };
        assert_eq!(&*s.as_str().unwrap(), "say \"hello\"");
    }

    #[test]
    fn test_decode_double_quoted_null_escape() {
        let s = YamlString::DoubleQuoted {
            text: b"\"null\\0char\"",
            start: 0,
        };
        assert_eq!(&*s.as_str().unwrap(), "null\0char");
    }

    #[test]
    fn test_decode_double_quoted_bell_escape() {
        let s = YamlString::DoubleQuoted {
            text: b"\"bell\\achar\"",
            start: 0,
        };
        assert_eq!(&*s.as_str().unwrap(), "bell\x07char");
    }

    #[test]
    fn test_decode_double_quoted_backspace_escape() {
        let s = YamlString::DoubleQuoted {
            text: b"\"back\\bspace\"",
            start: 0,
        };
        assert_eq!(&*s.as_str().unwrap(), "back\x08space");
    }

    #[test]
    fn test_decode_double_quoted_formfeed_escape() {
        let s = YamlString::DoubleQuoted {
            text: b"\"form\\ffeed\"",
            start: 0,
        };
        assert_eq!(&*s.as_str().unwrap(), "form\x0Cfeed");
    }

    #[test]
    fn test_decode_double_quoted_vertical_tab_escape() {
        let s = YamlString::DoubleQuoted {
            text: b"\"vert\\vtab\"",
            start: 0,
        };
        assert_eq!(&*s.as_str().unwrap(), "vert\x0Btab");
    }

    #[test]
    fn test_decode_double_quoted_escape_char_escape() {
        let s = YamlString::DoubleQuoted {
            text: b"\"esc\\echar\"",
            start: 0,
        };
        assert_eq!(&*s.as_str().unwrap(), "esc\x1Bchar");
    }

    #[test]
    fn test_decode_double_quoted_space_escape() {
        let s = YamlString::DoubleQuoted {
            text: b"\"spaced\\ word\"",
            start: 0,
        };
        assert_eq!(&*s.as_str().unwrap(), "spaced word");
    }

    #[test]
    fn test_decode_double_quoted_slash_escape() {
        let s = YamlString::DoubleQuoted {
            text: b"\"path\\/to\\/file\"",
            start: 0,
        };
        assert_eq!(&*s.as_str().unwrap(), "path/to/file");
    }

    #[test]
    fn test_decode_double_quoted_next_line_escape() {
        let s = YamlString::DoubleQuoted {
            text: b"\"next\\Nline\"",
            start: 0,
        };
        assert_eq!(&*s.as_str().unwrap(), "next\u{0085}line");
    }

    #[test]
    fn test_decode_double_quoted_nbsp_escape() {
        let s = YamlString::DoubleQuoted {
            text: b"\"non\\_break\"",
            start: 0,
        };
        assert_eq!(&*s.as_str().unwrap(), "non\u{00A0}break");
    }

    #[test]
    fn test_decode_double_quoted_line_separator_escape() {
        let s = YamlString::DoubleQuoted {
            text: b"\"line\\Lsep\"",
            start: 0,
        };
        assert_eq!(&*s.as_str().unwrap(), "line\u{2028}sep");
    }

    #[test]
    fn test_decode_double_quoted_paragraph_separator_escape() {
        let s = YamlString::DoubleQuoted {
            text: b"\"para\\Psep\"",
            start: 0,
        };
        assert_eq!(&*s.as_str().unwrap(), "para\u{2029}sep");
    }

    #[test]
    fn test_decode_double_quoted_hex_escape() {
        let s = YamlString::DoubleQuoted {
            text: b"\"hex\\x41char\"", // \x41 = 'A'
            start: 0,
        };
        assert_eq!(&*s.as_str().unwrap(), "hexAchar");
    }

    #[test]
    fn test_decode_double_quoted_hex_control_escape() {
        let s = YamlString::DoubleQuoted {
            text: b"\"ctrl\\x07char\"", // \x07 = bell
            start: 0,
        };
        assert_eq!(&*s.as_str().unwrap(), "ctrl\x07char");
    }

    #[test]
    fn test_decode_double_quoted_unicode_4digit_escape() {
        let s = YamlString::DoubleQuoted {
            text: b"\"euro\\u20ACsign\"", // € = U+20AC
            start: 0,
        };
        assert_eq!(&*s.as_str().unwrap(), "euro€sign");
    }

    #[test]
    fn test_decode_double_quoted_unicode_8digit_escape() {
        let s = YamlString::DoubleQuoted {
            text: b"\"emoji\\U0001F600face\"", // 😀 = U+1F600
            start: 0,
        };
        assert_eq!(&*s.as_str().unwrap(), "emoji😀face");
    }

    #[test]
    fn test_decode_double_quoted_multiple_escapes() {
        let s = YamlString::DoubleQuoted {
            text: b"\"line1\\nline2\\ttabbed\\\\slash\"",
            start: 0,
        };
        assert_eq!(&*s.as_str().unwrap(), "line1\nline2\ttabbed\\slash");
    }

    #[test]
    fn test_decode_double_quoted_escaped_newline_continuation() {
        // Backslash followed by actual newline = line continuation (no space)
        let s = YamlString::DoubleQuoted {
            text: b"\"line one\\\n  line two\"",
            start: 0,
        };
        assert_eq!(&*s.as_str().unwrap(), "line oneline two");
    }

    #[test]
    fn test_decode_double_quoted_escaped_crlf_continuation() {
        let s = YamlString::DoubleQuoted {
            text: b"\"line one\\\r\n  line two\"",
            start: 0,
        };
        assert_eq!(&*s.as_str().unwrap(), "line oneline two");
    }

    #[test]
    fn test_decode_single_quoted_double_quote() {
        // " in single-quoted is literal (no escaping needed in YAML)
        let s = YamlString::SingleQuoted {
            text: b"'say \"hello\"'",
            start: 0,
        };
        assert_eq!(&*s.as_str().unwrap(), "say \"hello\"");
    }

    #[test]
    fn test_decode_single_quoted_backslash() {
        // \ in single-quoted is literal (not an escape char)
        let s = YamlString::SingleQuoted {
            text: b"'path\\to\\file'",
            start: 0,
        };
        assert_eq!(&*s.as_str().unwrap(), "path\\to\\file");
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

    // =========================================================================
    // Direct transcoding tests (to_json_document path)
    // =========================================================================
    // These tests verify that the direct YAML→JSON transcoding produces
    // semantically identical output to the old as_str() + JSON encoding path.
    //
    // Both paths should produce valid JSON that decodes to the same string value,
    // though they may use different escape formats (e.g., \u0085 vs raw character).

    /// Helper: Get JSON output via to_json_document (uses new transcoding path)
    fn get_json_via_transcode(yaml: &[u8]) -> String {
        let index = YamlIndex::build(yaml).unwrap();
        let root = index.root(yaml);
        root.to_json_document()
    }

    /// Helper: Get JSON output via as_str() + manual encoding (old decode path)
    fn get_json_via_decode(yaml: &[u8]) -> String {
        let index = YamlIndex::build(yaml).unwrap();
        let root = index.root(yaml);

        // Extract first document's value
        match first_doc(root) {
            YamlValue::String(s) => {
                let decoded = s.as_str().unwrap();
                // Manually JSON-encode like the old path did
                let mut out = String::from("\"");
                for ch in decoded.chars() {
                    match ch {
                        '"' => out.push_str("\\\""),
                        '\\' => out.push_str("\\\\"),
                        '\n' => out.push_str("\\n"),
                        '\r' => out.push_str("\\r"),
                        '\t' => out.push_str("\\t"),
                        '\x08' => out.push_str("\\b"),
                        '\x0c' => out.push_str("\\f"),
                        c if (c as u32) < 0x20 => {
                            out.push_str(&format!("\\u{:04x}", c as u32));
                        }
                        c => out.push(c),
                    }
                }
                out.push('"');
                out
            }
            YamlValue::Mapping(fields) => {
                // For mappings, extract all string values and compare
                let mut pairs = Vec::new();
                for field in fields.into_iter() {
                    if let YamlValue::String(k) = field.key() {
                        if let YamlValue::String(v) = field.value() {
                            let key_str = k.as_str().unwrap();
                            let val_str = v.as_str().unwrap();
                            pairs.push(format!("\"{}\":\"{}\"", key_str, val_str));
                        }
                    }
                }
                format!("{{{}}}", pairs.join(","))
            }
            _ => panic!("expected string or mapping"),
        }
    }

    /// Parse a JSON string and return the decoded content (strips quotes, unescapes).
    /// Used to compare semantic equivalence of different JSON escape formats.
    fn json_string_to_rust_string(json: &str) -> String {
        assert!(
            json.starts_with('"') && json.ends_with('"'),
            "not a JSON string: {}",
            json
        );
        let inner = &json[1..json.len() - 1];
        let mut result = String::new();
        let mut chars = inner.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '\\' {
                match chars.next() {
                    Some('n') => result.push('\n'),
                    Some('r') => result.push('\r'),
                    Some('t') => result.push('\t'),
                    Some('b') => result.push('\x08'),
                    Some('f') => result.push('\x0c'),
                    Some('"') => result.push('"'),
                    Some('\\') => result.push('\\'),
                    Some('/') => result.push('/'),
                    Some('u') => {
                        let hex: String = chars.by_ref().take(4).collect();
                        let codepoint = u32::from_str_radix(&hex, 16).unwrap();
                        // Handle surrogate pairs
                        if (0xD800..=0xDBFF).contains(&codepoint) {
                            if chars.next() == Some('\\') && chars.next() == Some('u') {
                                let low_hex: String = chars.by_ref().take(4).collect();
                                let low = u32::from_str_radix(&low_hex, 16).unwrap();
                                let full = 0x10000 + ((codepoint - 0xD800) << 10) + (low - 0xDC00);
                                result.push(char::from_u32(full).unwrap());
                            }
                        } else {
                            result.push(char::from_u32(codepoint).unwrap());
                        }
                    }
                    Some(c) => panic!("unknown escape: \\{}", c),
                    None => panic!("trailing backslash"),
                }
            } else {
                result.push(c);
            }
        }
        result
    }

    /// Assert that two JSON strings decode to the same Rust string value.
    /// This allows for different escape formats (e.g., \u0085 vs raw char).
    fn assert_json_semantically_equal(transcoded: &str, decoded: &str, context: &str) {
        let transcoded_value = json_string_to_rust_string(transcoded);
        let decoded_value = json_string_to_rust_string(decoded);
        assert_eq!(
            transcoded_value, decoded_value,
            "{}: JSON strings decode to different values.\n  transcoded JSON: {}\n  decoded JSON: {}\n  transcoded value: {:?}\n  decoded value: {:?}",
            context, transcoded, decoded, transcoded_value, decoded_value
        );
    }

    #[test]
    fn test_transcode_double_quoted_simple() {
        let yaml = b"\"hello world\"";
        let json = get_json_via_transcode(yaml);
        assert_eq!(json, "\"hello world\"");
    }

    #[test]
    fn test_transcode_double_quoted_newline_escape() {
        let yaml = b"\"hello\\nworld\"";
        let transcoded = get_json_via_transcode(yaml);
        let decoded = get_json_via_decode(yaml);
        assert_eq!(transcoded, decoded, "transcoded vs decoded mismatch");
        assert_eq!(transcoded, "\"hello\\nworld\"");
    }

    #[test]
    fn test_transcode_double_quoted_tab_escape() {
        let yaml = b"\"hello\\tworld\"";
        let transcoded = get_json_via_transcode(yaml);
        let decoded = get_json_via_decode(yaml);
        assert_eq!(transcoded, decoded);
        assert_eq!(transcoded, "\"hello\\tworld\"");
    }

    #[test]
    fn test_transcode_double_quoted_backslash_escape() {
        let yaml = b"\"path\\\\to\\\\file\"";
        let transcoded = get_json_via_transcode(yaml);
        let decoded = get_json_via_decode(yaml);
        assert_eq!(transcoded, decoded);
        assert_eq!(transcoded, "\"path\\\\to\\\\file\"");
    }

    #[test]
    fn test_transcode_double_quoted_quote_escape() {
        let yaml = b"\"say \\\"hello\\\"\"";
        let transcoded = get_json_via_transcode(yaml);
        let decoded = get_json_via_decode(yaml);
        assert_eq!(transcoded, decoded);
        assert_eq!(transcoded, "\"say \\\"hello\\\"\"");
    }

    #[test]
    fn test_transcode_double_quoted_null_escape() {
        let yaml = b"\"null\\0char\"";
        let transcoded = get_json_via_transcode(yaml);
        let decoded = get_json_via_decode(yaml);
        assert_eq!(transcoded, decoded);
        assert_eq!(transcoded, "\"null\\u0000char\"");
    }

    #[test]
    fn test_transcode_double_quoted_bell_escape() {
        let yaml = b"\"bell\\achar\"";
        let transcoded = get_json_via_transcode(yaml);
        let decoded = get_json_via_decode(yaml);
        assert_eq!(transcoded, decoded);
        assert_eq!(transcoded, "\"bell\\u0007char\"");
    }

    #[test]
    fn test_transcode_double_quoted_backspace_escape() {
        let yaml = b"\"back\\bspace\"";
        let transcoded = get_json_via_transcode(yaml);
        let decoded = get_json_via_decode(yaml);
        // Both \b and \u0008 are valid JSON for backspace - verify semantic equivalence
        assert_json_semantically_equal(&transcoded, &decoded, "backspace escape");
    }

    #[test]
    fn test_transcode_double_quoted_formfeed_escape() {
        let yaml = b"\"form\\ffeed\"";
        let transcoded = get_json_via_transcode(yaml);
        let decoded = get_json_via_decode(yaml);
        // Both \f and \u000c are valid JSON for form feed - verify semantic equivalence
        assert_json_semantically_equal(&transcoded, &decoded, "formfeed escape");
    }

    #[test]
    fn test_transcode_double_quoted_vertical_tab_escape() {
        let yaml = b"\"vert\\vtab\"";
        let transcoded = get_json_via_transcode(yaml);
        let decoded = get_json_via_decode(yaml);
        assert_eq!(transcoded, decoded);
        assert_eq!(transcoded, "\"vert\\u000btab\"");
    }

    #[test]
    fn test_transcode_double_quoted_escape_escape() {
        let yaml = b"\"esc\\echar\"";
        let transcoded = get_json_via_transcode(yaml);
        let decoded = get_json_via_decode(yaml);
        assert_eq!(transcoded, decoded);
        assert_eq!(transcoded, "\"esc\\u001bchar\"");
    }

    #[test]
    fn test_transcode_double_quoted_next_line_escape() {
        let yaml = b"\"next\\Nline\"";
        let transcoded = get_json_via_transcode(yaml);
        let decoded = get_json_via_decode(yaml);
        // Transcoding uses \u0085, old path uses raw char - verify semantic equivalence
        assert_json_semantically_equal(&transcoded, &decoded, "next line escape");
        assert_eq!(transcoded, "\"next\\u0085line\"");
    }

    #[test]
    fn test_transcode_double_quoted_nbsp_escape() {
        let yaml = b"\"non\\_break\"";
        let transcoded = get_json_via_transcode(yaml);
        let decoded = get_json_via_decode(yaml);
        // Transcoding uses \u00a0, old path uses raw NBSP - verify semantic equivalence
        assert_json_semantically_equal(&transcoded, &decoded, "nbsp escape");
        assert_eq!(transcoded, "\"non\\u00a0break\"");
    }

    #[test]
    fn test_transcode_double_quoted_line_separator_escape() {
        let yaml = b"\"line\\Lsep\"";
        let transcoded = get_json_via_transcode(yaml);
        let decoded = get_json_via_decode(yaml);
        // Transcoding uses \u2028, old path uses raw char - verify semantic equivalence
        assert_json_semantically_equal(&transcoded, &decoded, "line separator escape");
        assert_eq!(transcoded, "\"line\\u2028sep\"");
    }

    #[test]
    fn test_transcode_double_quoted_paragraph_separator_escape() {
        let yaml = b"\"para\\Psep\"";
        let transcoded = get_json_via_transcode(yaml);
        let decoded = get_json_via_decode(yaml);
        // Transcoding uses \u2029, old path uses raw char - verify semantic equivalence
        assert_json_semantically_equal(&transcoded, &decoded, "paragraph separator escape");
        assert_eq!(transcoded, "\"para\\u2029sep\"");
    }

    #[test]
    fn test_transcode_double_quoted_hex_escape() {
        let yaml = b"\"hex\\x41char\""; // \x41 = 'A'
        let transcoded = get_json_via_transcode(yaml);
        let decoded = get_json_via_decode(yaml);
        assert_eq!(transcoded, decoded);
        assert_eq!(transcoded, "\"hexAchar\"");
    }

    #[test]
    fn test_transcode_double_quoted_hex_control_escape() {
        let yaml = b"\"ctrl\\x07char\""; // \x07 = bell
        let transcoded = get_json_via_transcode(yaml);
        let decoded = get_json_via_decode(yaml);
        assert_eq!(transcoded, decoded);
        assert_eq!(transcoded, "\"ctrl\\u0007char\"");
    }

    #[test]
    fn test_transcode_double_quoted_unicode_4digit_escape() {
        let yaml = b"\"euro\\u20ACsign\""; // € = U+20AC
        let transcoded = get_json_via_transcode(yaml);
        let decoded = get_json_via_decode(yaml);
        // Transcoding uses \u20ac, old path uses raw € - verify semantic equivalence
        assert_json_semantically_equal(&transcoded, &decoded, "unicode 4-digit escape");
        assert_eq!(transcoded, "\"euro\\u20acsign\"");
    }

    #[test]
    fn test_transcode_double_quoted_unicode_8digit_escape() {
        let yaml = b"\"emoji\\U0001F600face\""; // 😀 = U+1F600
        let transcoded = get_json_via_transcode(yaml);
        let decoded = get_json_via_decode(yaml);
        // Transcoding uses surrogate pair \ud83d\ude00, old path uses raw 😀
        // Verify semantic equivalence
        assert_json_semantically_equal(&transcoded, &decoded, "unicode 8-digit escape");
        assert_eq!(transcoded, "\"emoji\\ud83d\\ude00face\"");
    }

    #[test]
    fn test_transcode_double_quoted_multiple_escapes() {
        let yaml = b"\"line1\\nline2\\ttabbed\\\\slash\"";
        let transcoded = get_json_via_transcode(yaml);
        let decoded = get_json_via_decode(yaml);
        assert_eq!(transcoded, decoded);
        assert_eq!(transcoded, "\"line1\\nline2\\ttabbed\\\\slash\"");
    }

    #[test]
    fn test_transcode_double_quoted_multiline_folding() {
        // Line break in double-quoted string folds to space
        let yaml = b"\"hello\nworld\"";
        let transcoded = get_json_via_transcode(yaml);
        let decoded = get_json_via_decode(yaml);
        assert_eq!(transcoded, decoded);
        assert_eq!(transcoded, "\"hello world\"");
    }

    #[test]
    fn test_transcode_double_quoted_multiline_empty_lines() {
        // Empty lines become literal newlines (per YAML spec, no leading space)
        let yaml = b"\"hello\n\nworld\"";
        let transcoded = get_json_via_transcode(yaml);
        let decoded = get_json_via_decode(yaml);
        assert_eq!(transcoded, decoded);
        assert_eq!(transcoded, "\"hello\\nworld\"");
    }

    #[test]
    fn test_transcode_single_quoted_simple() {
        let yaml = b"'hello world'";
        let json = get_json_via_transcode(yaml);
        assert_eq!(json, "\"hello world\"");
    }

    #[test]
    fn test_transcode_single_quoted_escape() {
        // '' in single-quoted = literal '
        let yaml = b"'it''s working'";
        let transcoded = get_json_via_transcode(yaml);
        let decoded = get_json_via_decode(yaml);
        assert_eq!(transcoded, decoded);
        assert_eq!(transcoded, "\"it's working\"");
    }

    #[test]
    fn test_transcode_single_quoted_double_quote() {
        // " in single-quoted needs JSON escaping
        let yaml = b"'say \"hello\"'";
        let transcoded = get_json_via_transcode(yaml);
        let decoded = get_json_via_decode(yaml);
        assert_eq!(transcoded, decoded);
        assert_eq!(transcoded, "\"say \\\"hello\\\"\"");
    }

    #[test]
    fn test_transcode_single_quoted_backslash() {
        // \ in single-quoted needs JSON escaping (not a YAML escape!)
        let yaml = b"'path\\to\\file'";
        let transcoded = get_json_via_transcode(yaml);
        let decoded = get_json_via_decode(yaml);
        assert_eq!(transcoded, decoded);
        assert_eq!(transcoded, "\"path\\\\to\\\\file\"");
    }

    #[test]
    fn test_transcode_single_quoted_multiline_folding() {
        // Line break in single-quoted string folds to space
        let yaml = b"'hello\nworld'";
        let transcoded = get_json_via_transcode(yaml);
        let decoded = get_json_via_decode(yaml);
        assert_eq!(transcoded, decoded);
        assert_eq!(transcoded, "\"hello world\"");
    }

    #[test]
    fn test_transcode_single_quoted_multiline_empty_lines() {
        // Empty lines become literal newlines (per YAML spec, no leading space)
        let yaml = b"'hello\n\nworld'";
        let transcoded = get_json_via_transcode(yaml);
        let decoded = get_json_via_decode(yaml);
        assert_eq!(transcoded, decoded);
        assert_eq!(transcoded, "\"hello\\nworld\"");
    }

    #[test]
    fn test_transcode_in_mapping_context() {
        // Test transcoding works correctly in mapping values
        let yaml = b"key: \"value\\nwith\\nnewlines\"";
        let transcoded = get_json_via_transcode(yaml);
        assert!(transcoded.contains("value\\nwith\\nnewlines"));
    }

    #[test]
    fn test_transcode_slash_escape() {
        // \/ is valid YAML escape for /
        let yaml = b"\"path\\/to\\/file\"";
        let transcoded = get_json_via_transcode(yaml);
        let decoded = get_json_via_decode(yaml);
        assert_eq!(transcoded, decoded);
        assert_eq!(transcoded, "\"path/to/file\"");
    }

    #[test]
    fn test_transcode_space_escape() {
        // \  (backslash space) is valid YAML escape for space
        let yaml = b"\"spaced\\ word\"";
        let transcoded = get_json_via_transcode(yaml);
        let decoded = get_json_via_decode(yaml);
        assert_eq!(transcoded, decoded);
        assert_eq!(transcoded, "\"spaced word\"");
    }

    #[test]
    fn test_transcode_carriage_return_escape() {
        let yaml = b"\"line\\rbreak\"";
        let transcoded = get_json_via_transcode(yaml);
        let decoded = get_json_via_decode(yaml);
        assert_eq!(transcoded, decoded);
        assert_eq!(transcoded, "\"line\\rbreak\"");
    }

    #[test]
    fn test_transcode_crlf_multiline() {
        // CRLF line endings should fold to space
        let yaml = b"\"hello\r\nworld\"";
        let transcoded = get_json_via_transcode(yaml);
        let decoded = get_json_via_decode(yaml);
        assert_eq!(transcoded, decoded);
        assert_eq!(transcoded, "\"hello world\"");
    }

    #[test]
    fn test_transcode_escaped_newline_continuation() {
        // Backslash-newline is a line continuation (no space added)
        // This is the escaped literal newline, not \n escape sequence
        let yaml = b"\"line one\\\n  line two\"";
        let transcoded = get_json_via_transcode(yaml);
        let decoded = get_json_via_decode(yaml);
        assert_eq!(transcoded, decoded);
        assert_eq!(transcoded, "\"line oneline two\"");
    }

    #[test]
    fn test_transcode_escaped_crlf_continuation() {
        // Backslash-CRLF is also a line continuation
        let yaml = b"\"line one\\\r\n  line two\"";
        let transcoded = get_json_via_transcode(yaml);
        let decoded = get_json_via_decode(yaml);
        assert_eq!(transcoded, decoded);
        assert_eq!(transcoded, "\"line oneline two\"");
    }

    #[test]
    fn test_transcode_tab_escape_literal() {
        // \<tab> is valid YAML escape (same as \t)
        let yaml = b"\"tab\\\there\"";
        let transcoded = get_json_via_transcode(yaml);
        let decoded = get_json_via_decode(yaml);
        assert_eq!(transcoded, decoded);
        assert_eq!(transcoded, "\"tab\\there\"");
    }

    // =========================================================================
    // Additional coverage tests: ensure old as_str tests have transcode equivalents
    // =========================================================================

    #[test]
    fn test_transcode_matches_old_multiline_double_quoted_simple() {
        // Equivalent to test_multiline_double_quoted_simple but via transcode
        let yaml = b"key: \"line one\n  line two\"";
        let transcoded = get_json_via_transcode(yaml);
        // Should contain "line one line two" (folded)
        assert!(
            transcoded.contains("line one line two"),
            "got: {}",
            transcoded
        );
    }

    #[test]
    fn test_transcode_matches_old_multiline_double_quoted_empty_line() {
        // Equivalent to test_multiline_double_quoted_empty_line but via transcode
        let yaml = b"key: \"line one\n\n  line two\"";
        let transcoded = get_json_via_transcode(yaml);
        // Should contain "line one\nline two" (empty line becomes newline)
        assert!(
            transcoded.contains("line one\\nline two"),
            "got: {}",
            transcoded
        );
    }

    #[test]
    fn test_transcode_matches_old_multiline_single_quoted_simple() {
        // Equivalent to test_multiline_single_quoted_simple but via transcode
        let yaml = b"key: 'line one\n  line two'";
        let transcoded = get_json_via_transcode(yaml);
        // Should contain "line one line two" (folded)
        assert!(
            transcoded.contains("line one line two"),
            "got: {}",
            transcoded
        );
    }

    #[test]
    fn test_transcode_matches_old_multiline_single_quoted_with_escaped_quote() {
        // Equivalent to test_multiline_single_quoted_with_escaped_quote but via transcode
        let yaml = b"key: 'it''s\n  working'";
        let transcoded = get_json_via_transcode(yaml);
        // Should contain "it's working"
        assert!(transcoded.contains("it's working"), "got: {}", transcoded);
    }

    // =========================================================================
    // YAML Metadata Tests
    // =========================================================================

    #[test]
    fn test_anchor_metadata() {
        let yaml = b"default: &myanchor value\nref: *myanchor";
        let index = YamlIndex::build(yaml).unwrap();
        let root = index.root(yaml);

        // Navigate to the anchored value
        if let YamlValue::Mapping(fields) = first_doc(root) {
            // Find the "default" field which has the anchor
            for field in fields {
                if let YamlValue::String(k) = field.key() {
                    let key = k.as_str().unwrap();
                    if key.as_ref() == "default" {
                        // Get the cursor for the value
                        let cursor = field.value_cursor();
                        // Check the anchor name
                        assert_eq!(cursor.anchor(), Some("myanchor"));
                        return;
                    }
                }
            }
        }
        panic!("Could not find anchored value");
    }

    #[test]
    fn test_style_double_quoted() {
        let yaml = b"key: \"value\"";
        let index = YamlIndex::build(yaml).unwrap();
        let root = index.root(yaml);

        if let YamlValue::Mapping(fields) = first_doc(root) {
            if let Some(field) = fields.into_iter().next() {
                let cursor = field.value_cursor();
                assert_eq!(cursor.style(), "double");
                return;
            }
        }
        panic!("Could not find value");
    }

    #[test]
    fn test_style_single_quoted() {
        let yaml = b"key: 'value'";
        let index = YamlIndex::build(yaml).unwrap();
        let root = index.root(yaml);

        if let YamlValue::Mapping(fields) = first_doc(root) {
            if let Some(field) = fields.into_iter().next() {
                let cursor = field.value_cursor();
                assert_eq!(cursor.style(), "single");
                return;
            }
        }
        panic!("Could not find value");
    }

    #[test]
    fn test_style_literal_block() {
        let yaml = b"key: |\n  line1\n  line2";
        let index = YamlIndex::build(yaml).unwrap();
        let root = index.root(yaml);

        if let YamlValue::Mapping(fields) = first_doc(root) {
            if let Some(field) = fields.into_iter().next() {
                let cursor = field.value_cursor();
                assert_eq!(cursor.style(), "literal");
                return;
            }
        }
        panic!("Could not find value");
    }

    #[test]
    fn test_style_folded_block() {
        let yaml = b"key: >\n  line1\n  line2";
        let index = YamlIndex::build(yaml).unwrap();
        let root = index.root(yaml);

        if let YamlValue::Mapping(fields) = first_doc(root) {
            if let Some(field) = fields.into_iter().next() {
                let cursor = field.value_cursor();
                assert_eq!(cursor.style(), "folded");
                return;
            }
        }
        panic!("Could not find value");
    }

    #[test]
    fn test_style_flow_sequence() {
        let yaml = b"key: [a, b, c]";
        let index = YamlIndex::build(yaml).unwrap();
        let root = index.root(yaml);

        if let YamlValue::Mapping(fields) = first_doc(root) {
            if let Some(field) = fields.into_iter().next() {
                let cursor = field.value_cursor();
                assert_eq!(cursor.style(), "flow");
                return;
            }
        }
        panic!("Could not find value");
    }

    #[test]
    fn test_style_flow_mapping() {
        let yaml = b"key: {a: 1, b: 2}";
        let index = YamlIndex::build(yaml).unwrap();
        let root = index.root(yaml);

        if let YamlValue::Mapping(fields) = first_doc(root) {
            if let Some(field) = fields.into_iter().next() {
                let cursor = field.value_cursor();
                assert_eq!(cursor.style(), "flow");
                return;
            }
        }
        panic!("Could not find value");
    }

    #[test]
    fn test_style_plain() {
        let yaml = b"key: plain_value";
        let index = YamlIndex::build(yaml).unwrap();
        let root = index.root(yaml);

        if let YamlValue::Mapping(fields) = first_doc(root) {
            if let Some(field) = fields.into_iter().next() {
                let cursor = field.value_cursor();
                // Plain scalars have empty style
                assert_eq!(cursor.style(), "");
                return;
            }
        }
        panic!("Could not find value");
    }

    #[test]
    fn test_tag_string() {
        let yaml = b"key: hello";
        let index = YamlIndex::build(yaml).unwrap();
        let root = index.root(yaml);

        if let YamlValue::Mapping(fields) = first_doc(root) {
            if let Some(field) = fields.into_iter().next() {
                let cursor = field.value_cursor();
                assert_eq!(cursor.tag(), "!!str");
                return;
            }
        }
        panic!("Could not find value");
    }

    #[test]
    fn test_tag_int() {
        let yaml = b"key: 42";
        let index = YamlIndex::build(yaml).unwrap();
        let root = index.root(yaml);

        if let YamlValue::Mapping(fields) = first_doc(root) {
            if let Some(field) = fields.into_iter().next() {
                let cursor = field.value_cursor();
                assert_eq!(cursor.tag(), "!!int");
                return;
            }
        }
        panic!("Could not find value");
    }

    #[test]
    fn test_tag_bool() {
        let yaml = b"key: true";
        let index = YamlIndex::build(yaml).unwrap();
        let root = index.root(yaml);

        if let YamlValue::Mapping(fields) = first_doc(root) {
            if let Some(field) = fields.into_iter().next() {
                let cursor = field.value_cursor();
                assert_eq!(cursor.tag(), "!!bool");
                return;
            }
        }
        panic!("Could not find value");
    }

    #[test]
    fn test_tag_null() {
        let yaml = b"key: null";
        let index = YamlIndex::build(yaml).unwrap();
        let root = index.root(yaml);

        if let YamlValue::Mapping(fields) = first_doc(root) {
            if let Some(field) = fields.into_iter().next() {
                let cursor = field.value_cursor();
                assert_eq!(cursor.tag(), "!!null");
                return;
            }
        }
        panic!("Could not find value");
    }

    #[test]
    fn test_tag_float() {
        let yaml = b"key: 3.14";
        let index = YamlIndex::build(yaml).unwrap();
        let root = index.root(yaml);

        if let YamlValue::Mapping(fields) = first_doc(root) {
            if let Some(field) = fields.into_iter().next() {
                let cursor = field.value_cursor();
                assert_eq!(cursor.tag(), "!!float");
                return;
            }
        }
        panic!("Could not find value");
    }

    #[test]
    fn test_tag_seq() {
        let yaml = b"key:\n  - a\n  - b";
        let index = YamlIndex::build(yaml).unwrap();
        let root = index.root(yaml);

        // Navigate: root -> doc item -> mapping -> value (which is a sequence)
        if let YamlValue::Mapping(fields) = first_doc(root) {
            if let Some(field) = fields.into_iter().next() {
                let cursor = field.value_cursor();
                assert_eq!(cursor.tag(), "!!seq");
                return;
            }
        }
        panic!("Could not find sequence");
    }

    #[test]
    fn test_tag_map() {
        let yaml = b"outer:\n  inner: value";
        let index = YamlIndex::build(yaml).unwrap();
        let root = index.root(yaml);

        // Navigate: root -> doc item -> outer mapping -> value (which is inner mapping)
        if let YamlValue::Mapping(fields) = first_doc(root) {
            if let Some(field) = fields.into_iter().next() {
                let cursor = field.value_cursor();
                assert_eq!(cursor.tag(), "!!map");
                return;
            }
        }
        panic!("Could not find nested mapping");
    }

    #[test]
    fn test_kind_scalar() {
        let yaml = b"key: value";
        let index = YamlIndex::build(yaml).unwrap();
        let root = index.root(yaml);

        if let YamlValue::Mapping(fields) = first_doc(root) {
            if let Some(field) = fields.into_iter().next() {
                let cursor = field.value_cursor();
                assert_eq!(cursor.kind(), "scalar");
                return;
            }
        }
        panic!("Could not find value");
    }

    #[test]
    fn test_kind_seq() {
        let yaml = b"key:\n  - a\n  - b";
        let index = YamlIndex::build(yaml).unwrap();
        let root = index.root(yaml);

        // Navigate: root -> doc item -> mapping -> value (which is a sequence)
        if let YamlValue::Mapping(fields) = first_doc(root) {
            if let Some(field) = fields.into_iter().next() {
                let cursor = field.value_cursor();
                assert_eq!(cursor.kind(), "seq");
                return;
            }
        }
        panic!("Could not find sequence");
    }

    #[test]
    fn test_kind_map() {
        let yaml = b"outer:\n  inner: value";
        let index = YamlIndex::build(yaml).unwrap();
        let root = index.root(yaml);

        // Navigate: root -> doc item -> outer mapping -> value (which is inner mapping)
        if let YamlValue::Mapping(fields) = first_doc(root) {
            if let Some(field) = fields.into_iter().next() {
                let cursor = field.value_cursor();
                assert_eq!(cursor.kind(), "map");
                return;
            }
        }
        panic!("Could not find nested mapping");
    }

    #[test]
    fn test_no_anchor() {
        let yaml = b"key: value";
        let index = YamlIndex::build(yaml).unwrap();
        let root = index.root(yaml);

        if let YamlValue::Mapping(fields) = first_doc(root) {
            if let Some(field) = fields.into_iter().next() {
                let cursor = field.value_cursor();
                // No anchor, should return None
                assert_eq!(cursor.anchor(), None);
                return;
            }
        }
        panic!("Could not find value");
    }

    #[test]
    fn test_anchor_on_anchored_value() {
        let yaml = b"default: &myanchor value";
        let index = YamlIndex::build(yaml).unwrap();
        let root = index.root(yaml);

        if let YamlValue::Mapping(fields) = first_doc(root) {
            if let Some(field) = fields.into_iter().next() {
                let cursor = field.value_cursor();
                // Anchored value should return the anchor name
                assert_eq!(cursor.anchor(), Some("myanchor"));
                // Not an alias
                assert!(!cursor.is_alias());
                assert_eq!(cursor.alias(), None);
                return;
            }
        }
        panic!("Could not find value");
    }

    #[test]
    fn test_alias_returns_anchor_name() {
        let yaml = b"default: &myanchor value\nref: *myanchor";
        let index = YamlIndex::build(yaml).unwrap();
        let root = index.root(yaml);

        if let YamlValue::Mapping(mut fields) = first_doc(root) {
            // Skip first field (default), get second field (ref)
            fields.next();
            if let Some(field) = fields.next() {
                let cursor = field.value_cursor();
                // Alias should return the referenced anchor name
                assert!(cursor.is_alias());
                assert_eq!(cursor.alias(), Some("myanchor"));
                // Alias does not have its own anchor (no &name on the alias itself)
                assert_eq!(cursor.anchor(), None);
                return;
            }
        }
        panic!("Could not find alias");
    }

    #[test]
    fn test_kind_alias() {
        let yaml = b"default: &myanchor value\nref: *myanchor";
        let index = YamlIndex::build(yaml).unwrap();
        let root = index.root(yaml);

        if let YamlValue::Mapping(mut fields) = first_doc(root) {
            // Skip first field (default), get second field (ref)
            fields.next();
            if let Some(field) = fields.next() {
                let cursor = field.value_cursor();
                // yq returns "alias" for alias nodes
                assert_eq!(cursor.kind(), "alias");
                return;
            }
        }
        panic!("Could not find alias");
    }

    #[test]
    fn test_alias_on_non_alias() {
        let yaml = b"key: value";
        let index = YamlIndex::build(yaml).unwrap();
        let root = index.root(yaml);

        if let YamlValue::Mapping(fields) = first_doc(root) {
            if let Some(field) = fields.into_iter().next() {
                let cursor = field.value_cursor();
                // Not an alias
                assert!(!cursor.is_alias());
                assert_eq!(cursor.alias(), None);
                return;
            }
        }
        panic!("Could not find value");
    }

    #[test]
    fn test_alias_to_sequence() {
        let yaml = b"items: &list\n  - a\n  - b\nref: *list";
        let index = YamlIndex::build(yaml).unwrap();
        let root = index.root(yaml);

        if let YamlValue::Mapping(mut fields) = first_doc(root) {
            // First field: items (anchored sequence)
            if let Some(field) = fields.next() {
                let cursor = field.value_cursor();
                assert_eq!(cursor.anchor(), Some("list"));
                assert_eq!(cursor.kind(), "seq");
            }
            // Second field: ref (alias to sequence)
            if let Some(field) = fields.next() {
                let cursor = field.value_cursor();
                assert!(cursor.is_alias());
                assert_eq!(cursor.alias(), Some("list"));
                assert_eq!(cursor.kind(), "alias");
                return;
            }
        }
        panic!("Could not find alias");
    }

    #[test]
    fn test_alias_to_mapping() {
        let yaml = b"defaults: &defaults\n  host: localhost\nproduction: *defaults";
        let index = YamlIndex::build(yaml).unwrap();
        let root = index.root(yaml);

        if let YamlValue::Mapping(mut fields) = first_doc(root) {
            // First field: defaults (anchored mapping)
            if let Some(field) = fields.next() {
                let cursor = field.value_cursor();
                assert_eq!(cursor.anchor(), Some("defaults"));
                assert_eq!(cursor.kind(), "map");
            }
            // Second field: production (alias to mapping)
            if let Some(field) = fields.next() {
                let cursor = field.value_cursor();
                assert!(cursor.is_alias());
                assert_eq!(cursor.alias(), Some("defaults"));
                assert_eq!(cursor.kind(), "alias");
                return;
            }
        }
        panic!("Could not find alias");
    }

    // =========================================================================
    // Line and Column Tests
    // =========================================================================

    #[test]
    fn test_line_basic() {
        let yaml = b"name: Alice\nage: 30";
        let index = YamlIndex::build(yaml).unwrap();
        let root = index.root(yaml);

        // Root mapping is on line 1
        assert_eq!(root.line(), 1);
    }

    #[test]
    fn test_line_multiline() {
        let yaml = b"first: one\nsecond: two\nthird: three";
        let index = YamlIndex::build(yaml).unwrap();
        let root = index.root(yaml);

        if let YamlValue::Mapping(mut fields) = first_doc(root) {
            // First field value: line 1
            if let Some(field) = fields.next() {
                let cursor = field.value_cursor();
                assert_eq!(cursor.line(), 1);
            }
            // Second field value: line 2
            if let Some(field) = fields.next() {
                let cursor = field.value_cursor();
                assert_eq!(cursor.line(), 2);
            }
            // Third field value: line 3
            if let Some(field) = fields.next() {
                let cursor = field.value_cursor();
                assert_eq!(cursor.line(), 3);
            }
        }
    }

    #[test]
    fn test_column_basic() {
        let yaml = b"name: Alice";
        let index = YamlIndex::build(yaml).unwrap();
        let root = index.root(yaml);

        // Root mapping starts at column 1
        assert_eq!(root.column(), 1);

        if let YamlValue::Mapping(mut fields) = first_doc(root) {
            if let Some(field) = fields.next() {
                // The value "Alice" starts at column 7 (after "name: ")
                let cursor = field.value_cursor();
                assert_eq!(cursor.column(), 7);
            }
        }
    }

    #[test]
    fn test_column_sequence() {
        let yaml = b"- first\n- second";
        let index = YamlIndex::build(yaml).unwrap();
        let root = index.root(yaml);

        if let YamlValue::Sequence(elements) = first_doc(root) {
            // The cursor for a sequence element points to the element start position
            // which is where the "-" indicator is, at column 1
            let mut cursor = elements.element_cursor.unwrap();
            assert_eq!(cursor.column(), 1);
            assert_eq!(cursor.line(), 1);

            // Second element is on line 2, also at column 1 (the "-" position)
            cursor = cursor.next_sibling().unwrap();
            assert_eq!(cursor.column(), 1);
            assert_eq!(cursor.line(), 2);
        }
    }

    #[test]
    fn test_line_and_column_nested() {
        let yaml = b"outer:\n  inner: value";
        let index = YamlIndex::build(yaml).unwrap();
        let root = index.root(yaml);

        if let YamlValue::Mapping(mut fields) = first_doc(root) {
            if let Some(outer_field) = fields.next() {
                // outer's value (a mapping) is on line 2, column 3
                let outer_cursor = outer_field.value_cursor();
                assert_eq!(outer_cursor.line(), 2);
                assert_eq!(outer_cursor.column(), 3);

                if let YamlValue::Mapping(mut inner_fields) = outer_field.value() {
                    if let Some(inner_field) = inner_fields.next() {
                        // inner's value "value" is on line 2, column 10
                        let inner_cursor = inner_field.value_cursor();
                        assert_eq!(inner_cursor.line(), 2);
                        assert_eq!(inner_cursor.column(), 10);
                    }
                }
            }
        }
    }

    #[test]
    fn test_line_flow_collection() {
        let yaml = b"items: [a, b, c]";
        let index = YamlIndex::build(yaml).unwrap();
        let root = index.root(yaml);

        if let YamlValue::Mapping(mut fields) = first_doc(root) {
            if let Some(field) = fields.next() {
                // Flow collection starts at column 8
                let cursor = field.value_cursor();
                assert_eq!(cursor.line(), 1);
                assert_eq!(cursor.column(), 8);
            }
        }
    }

    #[test]
    fn test_line_block_scalar() {
        let yaml = b"text: |\n  line one\n  line two";
        let index = YamlIndex::build(yaml).unwrap();
        let root = index.root(yaml);

        if let YamlValue::Mapping(mut fields) = first_doc(root) {
            if let Some(field) = fields.next() {
                // Block scalar indicator is at column 7
                let cursor = field.value_cursor();
                assert_eq!(cursor.line(), 1);
                assert_eq!(cursor.column(), 7);
            }
        }
    }
}
