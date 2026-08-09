#![allow(unsafe_code)] // from_utf8_unchecked on validated UTF-8 in the transcoder
#![allow(clippy::items_after_test_module)] // STYLE-0004: helper items intentionally follow `mod tests` in this file
//! YamlCursor - Lazy YAML navigation using the semi-index.
//!
//! This module provides a cursor-based API for navigating YAML structures
//! without fully parsing the YAML text. Values are only decoded when explicitly
//! requested.

#[cfg(not(test))]
use alloc::vec;
#[cfg(not(test))]
use alloc::{
    borrow::Cow, collections::BTreeMap, rc::Rc, string::String, string::ToString, vec::Vec,
};

#[cfg(test)]
use std::borrow::Cow;
#[cfg(test)]
use std::collections::BTreeMap;
#[cfg(test)]
use std::rc::Rc;
#[cfg(test)]
use std::string::ToString;

use super::index::YamlIndex;
use super::line_break::{is_line_break, line_break_len, line_break_len_before};
use super::scalar::{could_be_null_or_bool, resolve_plain, resolve_tagged, ResolvedScalar};
use super::{starts_inline_seq_entry, starts_seq_entry};
use crate::util::simd::escape::find_json_escape;

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

impl<W> Clone for YamlCursor<'_, W> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<W> Copy for YamlCursor<'_, W> {}

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
    pub fn first_child(&self) -> Option<Self> {
        let new_pos = self.index.bp().first_child(self.bp_pos)?;
        Some(YamlCursor {
            text: self.text,
            index: self.index,
            bp_pos: new_pos,
        })
    }

    /// Navigate to the next sibling.
    #[inline]
    pub fn next_sibling(&self) -> Option<Self> {
        let new_pos = self.index.bp().next_sibling(self.bp_pos)?;
        Some(YamlCursor {
            text: self.text,
            index: self.index,
            bp_pos: new_pos,
        })
    }

    /// Navigate to the parent.
    #[inline]
    pub fn parent(&self) -> Option<Self> {
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

        // Check if this is a container FIRST - this takes priority over text-based detection.
        // This is important for mappings where the key is an alias (e.g., `*alias : value`),
        // which would otherwise be detected as an alias by looking at the leading `*`.
        // Note: containers have TY entries and are never seq_items.
        if self.is_container() {
            // Determine if mapping or sequence using the TY bits
            if self.index.is_sequence_at_bp(self.bp_pos) {
                return YamlValue::Sequence(YamlElements::from_sequence_cursor(*self));
            }
            return YamlValue::Mapping(YamlFields::from_mapping_cursor(*self));
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

        // Check for sequence item wrapper (non-container node starting with "- ").
        // Sequence items delegate to their first child's value. Uses the
        // already-computed text_pos to avoid redundant lookups.
        if starts_seq_entry(self.text, text_pos) {
            if let Some(child) = self.first_child() {
                return child.value();
            }

            // Childless. Two different shapes reach here (#332):
            //
            //   * a genuine empty block sequence item (`-`, `- `, `- # comment`). The
            //     parser opens the wrapper while positioned *at* the `-` and never records
            //     an end for it, so the lookup below yields `None` (dense storage) or a
            //     stale earlier scalar's end at or before the dash (compact storage).
            //
            //   * a plain scalar that merely *begins* `- `, which is what `[- x]` and
            //     `a: - x` produce. Invalid YAML — the strict validator rejects both — but
            //     the parser records a real trimmed end past the content, and dropping that
            //     content silently is a worse failure mode than preserving the text.
            //
            // So `end > text_pos` means "this node has an extent of its own". Cold path:
            // only reached when a `- `-prefixed node has no BP child, i.e. for empty
            // sequence items. Every `- x` short-circuits above with zero extra work.
            if !self.childless_seq_entry_has_own_extent(open_idx, text_pos) {
                return YamlValue::Null;
            }
            // Otherwise fall through and decode `- …` as the plain scalar it is.
        }

        // Check for alias (only for non-container nodes)
        let byte = self.text[text_pos];
        if byte == b'*' {
            return self.parse_alias_value(text_pos);
        }

        // Check for a property (anchor and/or tag) - if present, skip past it
        // to get the actual value. Anchors look like: &anchor_name value
        let effective_text_pos = if matches!(byte, b'&' | b'!') {
            self.skip_properties_and_whitespace(text_pos)
        } else {
            text_pos
        };

        // If we're past the end after skipping the property, this is a null
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

        // Note: block-style sequences (`- item`) never reach this point. Real
        // sequence containers carry TY bits and are handled by the `is_container()`
        // check above; sequence-item wrapper nodes are caught by the inline `- `
        // check after `text_pos` is computed. Both cases return before here.

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

    /// Does a childless `- `-prefixed node cover text of its own past the indicator?
    ///
    /// Split out of [`Self::value`] and marked cold so the discrimination it performs
    /// (#332) stays off the path every sequence item walks: `value` is recursive and
    /// sits under `yq`'s whole read path, and inlining this made it 1-2% slower on
    /// sequence-heavy documents even though the branch itself is never taken there.
    #[cold]
    #[inline(never)]
    fn childless_seq_entry_has_own_extent(&self, open_idx: usize, text_pos: usize) -> bool {
        self.index
            .text_end_pos_by_open_idx(open_idx)
            .is_some_and(|end| end > text_pos)
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

        // Resolve the alias to its target. For an index from `YamlIndex::build`
        // this is always `Some`: an alias naming an anchor that is not in scope
        // fails the build (#372), so no alias node reaches here unresolved.
        // `resolve_alias` stays fallible because it is public and takes an
        // arbitrary BP position, which need not be an alias at all.
        let target = self.index.resolve_alias(self.bp_pos, self.text);

        YamlValue::Alias {
            anchor_name,
            target,
        }
    }

    /// Skip past a leading `&anchor` and/or `!tag` (either order, each at
    /// most once) and any following whitespace.
    ///
    /// Anchor syntax: `&name` where name can contain alphanumerics, underscores,
    /// hyphens, and colons.
    ///
    /// A key's or value's BP node can be opened *before* its property is
    /// consumed (`record_key_tag`/`record_key_anchor`'s `bp_pos - 1`
    /// convention in `parser.rs`), so the node's recorded text span still
    /// starts at the property text — this is where that gets stripped away
    /// at decode time. `parser.rs`'s `parse_node_properties` is the
    /// parse-time analogue for nodes whose BP opens *after* property
    /// consumption (#224 generalized this from anchor-only).
    ///
    /// Returns the position of the first non-whitespace character after the
    /// last property, or the end of text if nothing follows.
    fn skip_properties_and_whitespace(&self, start: usize) -> usize {
        let mut pos = start;
        loop {
            match self.text.get(pos) {
                Some(b'&') => {
                    pos += 1;
                    // Anchor name characters (per YAML spec, anchors can
                    // contain alphanumerics, underscores, hyphens, and also
                    // colons in some contexts).
                    while pos < self.text.len() {
                        match self.text[pos] {
                            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_' | b'-' | b':' => {
                                pos += 1;
                            }
                            _ => break,
                        }
                    }
                }
                Some(b'!') => {
                    let (end, _) = super::parser::scan_tag_extent(self.text, pos);
                    pos = end;
                }
                _ => break,
            }
            while pos < self.text.len() && matches!(self.text[pos], b' ' | b'\t') {
                pos += 1;
            }
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
        while line_start > 0 && !is_line_break(text[line_start - 1]) {
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
        while line_start > 0 && !is_line_break(self.text[line_start - 1]) {
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
    #[allow(dead_code)] // STYLE-0005: alternate parse-path helper; unused in current path
    fn compute_base_indent_and_root_flag(&self, value_pos: usize) -> (usize, bool) {
        // Find start of current line
        let mut line_start = value_pos;
        while line_start > 0 && !is_line_break(self.text[line_start - 1]) {
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
                if scan + 1 < self.text.len()
                    && matches!(self.text[scan + 1], b' ' | b'\t' | b'\n' | b'\r')
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
                    if matches!(next, b' ' | b'\t' | b'\n' | b'\r') {
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

            // Step back over the break that ended the previous line — the whole
            // break, not one byte. Stepping back a fixed 1 landed on the `\n`
            // of a CRLF, so the walk below stopped dead on its `\r` and the
            // previous line came out as `---\r`, which never matched the
            // document marker (#324).
            let prev_line_end = line_start - line_break_len_before(self.text, line_start).max(1);

            // Go to the start of that line
            let mut prev_line_start = prev_line_end;
            while prev_line_start > 0 && !is_line_break(self.text[prev_line_start - 1]) {
                prev_line_start -= 1;
            }

            // Check if previous line is "---" (document start marker)
            let prev_line = &self.text[prev_line_start..prev_line_end];
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
    ///
    /// No longer dead code as of #434: `find_scalar_end` (a live path) calls
    /// this directly, in addition to the still-unused `find_plain_scalar_end`.
    fn is_in_flow_context(&self, pos: usize) -> bool {
        // Find start of line containing pos
        let mut line_start = pos;
        while line_start > 0 && !is_line_break(self.text[line_start - 1]) {
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
            while start > 0 && !is_line_break(self.text[start - 1]) {
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

    /// Find the end of a single-line unquoted scalar (stops at a line break).
    fn find_scalar_end(&self, start: usize) -> usize {
        // ns-plain-char only reserves `,`/`]`/`}` inside a flow collection; in
        // block context they're ordinary scalar content (e.g. `note: a, b`).
        // Breaking on them unconditionally made this function (used only by
        // `at_offset`/`yq-locate`) truncate any block-context scalar
        // containing one of these bytes, while `syq`/`yq` printed the value
        // in full — an eval/locate divergence in the #370 shape, just
        // reached via flow-indicator bytes instead of a missing tab (#434).
        let in_flow = self.is_in_flow_context(start);
        let mut end = start;
        while end < self.text.len() {
            match self.text[end] {
                // Block context delimiters
                b'\n' | b'\r' => break,
                b'#' => {
                    // # preceded by white space is a comment; otherwise it's
                    // part of the scalar (ns-plain-char admits a `#` that is
                    // not preceded by white space, e.g. `a#b`).
                    if end > start && matches!(self.text[end - 1], b' ' | b'\t') {
                        break;
                    }
                    end += 1;
                }
                // Flow context delimiters - only terminate in flow context
                b',' | b']' | b'}' if in_flow => break,
                b':' => {
                    // Colon followed by white space, a line break, or EOF ends
                    // the scalar. The tab is not optional: this is the same
                    // terminator set `parse_unquoted_key` uses, and dropping it
                    // here made `a:\t1` — legal YAML — report a byte range that
                    // ran to end of input (#370).
                    if end + 1 >= self.text.len() {
                        // Colon at EOF - this is a key separator
                        break;
                    }
                    if matches!(self.text[end + 1], b' ' | b'\t' | b'\n' | b'\r') {
                        break;
                    }
                    end += 1;
                }
                _ => end += 1,
            }
        }
        // Trim trailing white space, tab included: this re-derives the extent
        // the parser already trimmed, so the two must agree on what separation
        // is, or `yq-locate` reports `a\t` for a key `yq` prints as `a` (#370).
        while end > start && matches!(self.text[end - 1], b' ' | b'\t' | b'\r') {
            end -= 1;
        }
        end
    }

    /// Find the end of a plain (unquoted) scalar, including continuation lines.
    /// A continuation line is more indented than base_indent (in block context),
    /// or any line that doesn't start with a flow delimiter (in flow context).
    /// For document root scalars (is_doc_root=true), continuation at indent 0 is allowed.
    #[allow(dead_code)] // STYLE-0005: alternate parse-path helper; unused in current path
    fn find_plain_scalar_end(&self, start: usize, base_indent: usize, is_doc_root: bool) -> usize {
        let in_flow = self.is_in_flow_context(start);
        let mut end = start;

        loop {
            // Find end of current line content
            while end < self.text.len() {
                match self.text[end] {
                    b'\n' | b'\r' => break,
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
                        if matches!(self.text[end + 1], b' ' | b'\t' | b'\n' | b'\r') {
                            break;
                        }
                        end += 1;
                    }
                    _ => end += 1,
                }
            }

            // Trim trailing whitespace (spaces, tabs, and a CR that a SIMD skip
            // may have carried past) from current line
            let mut line_end = end;
            while line_end > start && matches!(self.text[line_end - 1], b' ' | b'\t' | b'\r') {
                line_end -= 1;
            }

            // Check if there's a continuation line
            if end >= self.text.len() || !is_line_break(self.text[end]) {
                // No line break, end of text or hit delimiter
                return line_end;
            }

            // Look ahead to see if next line is a continuation
            let newline_pos = end;
            end += line_break_len(self.text, end); // Skip the line break

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
                    b'\n' | b'\r' => {
                        // Empty line - continue looking. CRLF is one break.
                        empty_lines_end = after_indent + line_break_len(self.text, after_indent);
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
                        if check_pos >= self.text.len() || is_line_break(self.text[check_pos]) {
                            // Line has only whitespace - treat as empty line, continue looking
                            empty_lines_end = check_pos + line_break_len(self.text, check_pos);
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
                        }
                        // Not indented enough - scalar ends before this
                        return line_end;
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
                if let Some((first_cursor, rest)) = elements.uncons_cursor() {
                    if rest.uncons_cursor().is_none() {
                        // Single document - output it directly without array
                        // wrapper. Via the cursor, not the extracted
                        // `YamlValue`: the document's own tag lives on its
                        // cursor's `bp_pos` (#224).
                        first_cursor.write_json_to(&mut output);
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
    /// cursor.stream_json(&mut FmtWriter(&mut output), 0).unwrap();
    /// ```
    ///
    /// - `indent_spaces`: Spaces per indentation level (0 for compact)
    pub fn stream_json<Out: core::fmt::Write>(
        &self,
        out: &mut Out,
        indent_spaces: usize,
    ) -> core::fmt::Result {
        self.stream_json_value(out, 0, indent_spaces)
    }

    /// Stream YAML as JSON, unwrapping single documents (matches yq behavior).
    ///
    /// If the root is a single-document array `[doc]`, outputs just `doc` as JSON.
    /// If there are multiple documents, outputs the array `[doc1, doc2, ...]`.
    ///
    /// - `indent_spaces`: Spaces per indentation level (0 for compact)
    pub fn stream_json_document<Out: core::fmt::Write>(
        &self,
        out: &mut Out,
        indent_spaces: usize,
    ) -> core::fmt::Result {
        // Check if this is the root document array with a single document
        if self.bp_pos == 0 {
            if let YamlValue::Sequence(elements) = self.value() {
                if let Some((first_cursor, rest)) = elements.uncons_cursor() {
                    if rest.uncons_cursor().is_none() {
                        // Single document - output it directly without array
                        // wrapper. Via the cursor, not the extracted
                        // `YamlValue`: the document's own tag lives on its
                        // cursor's `bp_pos` (#224).
                        return first_cursor.stream_json_value(out, 0, indent_spaces);
                    }
                }
            }
        }

        // Multiple documents or not at root - output as-is
        self.stream_json_value(out, 0, indent_spaces)
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
            YamlValue::String(s) => {
                // YAML output (unlike JSON) has tag syntax, so an explicit
                // tag is preserved verbatim rather than dropped - matching
                // real `yq`, which re-emits `!!str 1`/`!!int "5"`/`!custom v`
                // as-is rather than only letting the tag affect resolution
                // (#224).
                if let Some(tag) = self.explicit_tag() {
                    out.write_str(tag)?;
                    out.write_char(' ')?;
                }
                stream_yaml_string_value(out, &s)
            }
            YamlValue::Mapping(fields) => {
                if fields.is_empty() {
                    return out.write_str("{}");
                }
                if indent_spaces == 0 || self.style() == "flow" {
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
                            if let Some(tag) = field.key_cursor().explicit_tag() {
                                out.write_str(tag)?;
                                out.write_char(' ')?;
                            }
                            stream_yaml_string_value(out, &s)?;
                        } else {
                            stream_yaml_nonstring_key(out, &field.key())?;
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
                            if let Some(tag) = field.key_cursor().explicit_tag() {
                                out.write_str(tag)?;
                                out.write_char(' ')?;
                            }
                            stream_yaml_string_value(out, &s)?;
                        } else {
                            stream_yaml_nonstring_key(out, &field.key())?;
                        }
                        out.write_char(':')?;
                        // Check if value needs newline
                        let value = field.value_cursor();
                        if is_yaml_cursor_container(&value) && value.style() != "flow" {
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
                if indent_spaces == 0 || self.style() == "flow" {
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
                        // Check if value needs newline. A container value
                        // gets a bare `-` (no trailing space) followed by its
                        // own indented line — matching real yq and the
                        // DOM/pretty-printer, and avoiding a stray trailing
                        // space before the newline.
                        if is_yaml_cursor_container(&cursor) && cursor.style() != "flow" {
                            out.write_char('-')?;
                            out.write_char('\n')?;
                            write_yaml_indent(out, current_indent + indent_spaces)?;
                            cursor.stream_yaml_value(
                                out,
                                current_indent + indent_spaces,
                                indent_spaces,
                            )?;
                        } else {
                            out.write_str("- ")?;
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
    ///
    /// - `indent_spaces`: Spaces per indentation level (0 for compact)
    fn stream_json_value<Out: core::fmt::Write>(
        &self,
        out: &mut Out,
        current_indent: usize,
        indent_spaces: usize,
    ) -> core::fmt::Result {
        match self.value() {
            YamlValue::Null => out.write_str("null"),
            YamlValue::String(s) => {
                // An explicit core-schema tag (`!!str`, `!!int`, …) forces
                // resolution regardless of quoting style - `!!int "5"` is the
                // number 5, not the string "5" - so it's checked before the
                // direct transcoding optimization below, which otherwise
                // always treats a quoted/block scalar as a string (#224).
                if let Some(explicit) = self.explicit_tag() {
                    if let Ok(str_val) = s.as_str() {
                        if let Some(resolved) = resolve_tagged(&str_val, explicit) {
                            return stream_resolved_scalar_as_json(out, resolved, &str_val);
                        }
                    }
                }
                // Direct transcoding optimization
                match stream_yaml_string_to_json(out, &s) {
                    Ok(true) => Ok(()), // Written directly as JSON string
                    Ok(false) => {
                        if let Ok(str_val) = s.as_str() {
                            if s.is_unquoted() {
                                // Plain scalar - resolve per the core schema
                                stream_yaml_scalar_as_json(out, &str_val)
                            } else {
                                // Block scalars are always strings
                                stream_json_string(out, &str_val)
                            }
                        } else {
                            out.write_str("null")
                        }
                    }
                    Err(_) => out.write_str("null"),
                }
            }
            YamlValue::Mapping(fields) => {
                if fields.is_empty() {
                    return out.write_str("{}");
                }
                out.write_char('{')?;
                let next_indent = current_indent + indent_spaces;
                let mut first = true;
                for field in fields {
                    if !first {
                        out.write_char(',')?;
                    }
                    first = false;
                    if indent_spaces > 0 {
                        out.write_char('\n')?;
                        write_yaml_indent(out, next_indent)?;
                    }

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
                        // Alias/complex key (#222): resolve alias-to-scalar,
                        // else "" — the entry is kept, never dropped
                        stream_json_string(out, &field.key().key_string())?;
                    }

                    out.write_str(if indent_spaces > 0 { ": " } else { ":" })?;
                    field
                        .value_cursor()
                        .stream_json_value(out, next_indent, indent_spaces)?;
                }
                if indent_spaces > 0 {
                    out.write_char('\n')?;
                    write_yaml_indent(out, current_indent)?;
                }
                out.write_char('}')
            }
            YamlValue::Sequence(elements) => {
                if elements.is_empty() {
                    return out.write_str("[]");
                }
                out.write_char('[')?;
                let next_indent = current_indent + indent_spaces;
                let mut first = true;
                let mut rest = elements;
                while let Some((cursor, next)) = rest.uncons_cursor() {
                    if !first {
                        out.write_char(',')?;
                    }
                    first = false;
                    rest = next;
                    if indent_spaces > 0 {
                        out.write_char('\n')?;
                        write_yaml_indent(out, next_indent)?;
                    }
                    // Recurse via the cursor, not `stream_yaml_value_as_json`
                    // on the extracted `YamlValue`: an element's own tag
                    // lives on its cursor's `bp_pos`, which a bare
                    // `YamlValue` has already lost (#224).
                    cursor.stream_json_value(out, next_indent, indent_spaces)?;
                }
                if indent_spaces > 0 {
                    out.write_char('\n')?;
                    write_yaml_indent(out, current_indent)?;
                }
                out.write_char(']')
            }
            YamlValue::Alias { target, .. } => {
                if let Some(target_cursor) = target {
                    target_cursor.stream_json_value(out, current_indent, indent_spaces)
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
                // An explicit core-schema tag forces resolution regardless of
                // quoting style - see the mirrored check in `stream_json_value` (#224).
                if let Some(explicit) = self.explicit_tag() {
                    if let Ok(str_val) = s.as_str() {
                        if let Some(resolved) = resolve_tagged(&str_val, explicit) {
                            write_resolved_scalar_as_json(output, resolved, &str_val);
                            return;
                        }
                    }
                }
                // Direct transcoding optimization (avoids intermediate allocation for quoted strings)
                match write_yaml_string_to_json(output, &s) {
                    Ok(true) => {} // Written directly as JSON string
                    Ok(false) => {
                        if let Ok(str_val) = s.as_str() {
                            if s.is_unquoted() {
                                // Plain scalar - resolve per the core schema
                                write_yaml_scalar_as_json(output, &str_val);
                            } else {
                                // Block scalars are always strings
                                write_json_string(output, &str_val);
                            }
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
                        // Alias/complex key (#222): resolve alias-to-scalar,
                        // else "" — the entry is kept, never dropped
                        write_json_string(output, &field.key().key_string());
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
                let mut rest = elements;
                while let Some((cursor, next)) = rest.uncons_cursor() {
                    if !first {
                        output.push(',');
                    }
                    first = false;
                    rest = next;

                    // Recurse via the cursor, not `write_yaml_value_as_json`
                    // on the extracted `YamlValue`: an element's own tag
                    // lives on its cursor's `bp_pos`, which a bare
                    // `YamlValue` has already lost (#224).
                    cursor.write_json_to(output);
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

    /// Get the explicit YAML tag written on this node in the source
    /// (`"!!str"`, `"!custom"`, `"!<tag:example.com,2000:foo>"`, …).
    ///
    /// Returns `None` if the node has no explicit tag. Distinct from
    /// [`Self::tag`], which returns an *inferred* type label derived from the
    /// value's shape rather than the source text, and never reflects an
    /// explicit tag even when one is present (#224).
    ///
    /// # Example
    /// ```ignore
    /// let yaml = b"a: !!str 1";
    /// let index = YamlIndex::build(yaml).unwrap();
    /// // Navigate to the value and call .explicit_tag() to get Some("!!str")
    /// ```
    #[inline]
    pub fn explicit_tag(&self) -> Option<&str> {
        self.index.get_tag(self.bp_pos)
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
        let (line, _column) = self.index.to_line_column(offset, self.text);
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
        let (_line, column) = self.index.to_line_column(offset, self.text);
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
    /// It skips a leading anchor (`&name`) but not an explicit tag; explicit
    /// tags are currently rejected by the parser entirely, so this is not
    /// reachable today, but would need addressing alongside tag support.
    pub fn style(&self) -> &'static str {
        let Some(text_pos) = self.text_position() else {
            return "";
        };

        if text_pos >= self.text.len() {
            return "";
        }

        // Skip a leading property (anchor and/or tag) if present
        let effective_pos = if matches!(self.text[text_pos], b'&' | b'!') {
            self.skip_properties_and_whitespace(text_pos)
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
    /// Note: This is the tag of the node's *resolved* value, not necessarily
    /// its literal source text. One of the 5 core-schema tags (`!!str`,
    /// `!!null`, `!!bool`, `!!int`, `!!float`) written explicitly in the
    /// source forces that resolution (`!!str 1` returns `"!!str"`, not the
    /// `"!!int"` the bare content would infer); any other explicit tag (a
    /// custom tag, `!!seq`, `!!map`, …) does not change it, and this falls
    /// through to inferring from the content instead. Use
    /// [`Self::explicit_tag`] for the literal source-level tag text, which is
    /// `None` unless one was actually written (#224).
    pub fn tag(&self) -> &'static str {
        match self.value() {
            YamlValue::Null => "!!null",
            YamlValue::String(s) => {
                if let Some(explicit) = self.explicit_tag() {
                    if let Ok(str_val) = s.as_str() {
                        if let Some(resolved) = resolve_tagged(&str_val, explicit) {
                            return resolved.tag();
                        }
                    }
                }
                // No explicit tag, or not one of the 5 core-schema tags -
                // infer type from plain scalars per the YAML 1.2 core
                // schema; quoted/block scalars are always strings.
                if s.is_unquoted() {
                    if let Ok(str_val) = s.as_str() {
                        return resolve_plain(&str_val).tag();
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
    pub fn cursor_at_offset(&self, offset: usize) -> Option<Self> {
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
    pub fn cursor_at_position(&self, line: usize, col: usize) -> Option<Self> {
        // Convert line/column to byte offset
        let offset = self.index.to_offset(line, col, self.text)?;

        // Use cursor_at_offset to find the node
        self.cursor_at_offset(offset)
    }
}

/// Write a YAML value as JSON from a bare `YamlValue` with no cursor of its
/// own — e.g. one already extracted via `YamlElements::uncons`/`get`.
///
/// No production call site reaches this anymore (#224): every real path
/// recurses via a cursor instead (`YamlElements::uncons_cursor` +
/// `YamlCursor::write_json_to`), since a node's own tag lives on its
/// `bp_pos`, which a bare `YamlValue` has already lost. Kept as the
/// value-only half of `test_sequence_element_accessors_agree`, which asserts
/// `uncons`/`get`/`uncons_cursor` all produce identical JSON (#332).
#[allow(dead_code)] // STYLE-0005: used in tests
fn write_yaml_value_as_json<W: AsRef<[u64]>>(output: &mut String, value: YamlValue<'_, W>) {
    match value {
        YamlValue::Null => output.push_str("null"),
        YamlValue::String(s) => {
            // Direct transcoding optimization (avoids intermediate allocation for quoted strings)
            match write_yaml_string_to_json(output, &s) {
                Ok(true) => {} // Written directly as JSON string
                Ok(false) => {
                    if let Ok(str_val) = s.as_str() {
                        if s.is_unquoted() {
                            // Plain scalar - resolve per the core schema
                            write_yaml_scalar_as_json(output, &str_val);
                        } else {
                            // Block scalars are always strings
                            write_json_string(output, &str_val);
                        }
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
                    // Alias/complex key (#222): resolve alias-to-scalar,
                    // else "" — the entry is kept, never dropped
                    write_json_string(output, &field.key().key_string());
                }

                output.push(':');
                field.value_cursor().write_json_to(output);
            }
            output.push('}');
        }
        YamlValue::Sequence(elements) => {
            output.push('[');
            let mut first = true;
            let mut rest = elements;
            while let Some((cursor, next)) = rest.uncons_cursor() {
                if !first {
                    output.push(',');
                }
                first = false;
                rest = next;
                // A child element does have a cursor even though this
                // function's own top-level `value` doesn't - use it so a
                // nested element's tag isn't lost either.
                cursor.write_json_to(output);
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
///
/// Uses SIMD (NEON on ARM64) to scan for escapable characters in 16-byte chunks,
/// with automatic scalar fallback for short strings and remainders.
fn write_json_string(output: &mut String, s: &str) {
    output.push('"');

    let bytes = s.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        // Find the next byte that needs escaping using SIMD
        // The SIMD function handles short strings internally with scalar fallback
        let escape_pos = find_json_escape(bytes, i);

        // Copy the safe span directly
        if i < escape_pos {
            // SAFETY: We're copying valid UTF-8 bytes that don't contain
            // any multi-byte sequence starters that would be split
            output.push_str(&s[i..escape_pos]);
        }

        i = escape_pos;

        // Handle the escape character if any
        if i < len {
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
                _ => {} // Not an escape character (shouldn't happen)
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
///
/// Everything >= 0x20 (including C1 controls 0x80-0x9F and higher
/// codepoints) streams as raw UTF-8, matching `stream_json_escape`'s policy
/// and yq's own output: JSON only requires escaping `"`, `\`, and C0
/// controls. An earlier version of this function additionally re-escaped
/// non-ASCII characters as `\u00XX`/`\uXXXX`, which diverged from
/// `stream_json_escape`/yq for any string reached through this non-streaming
/// transcode path (#532).
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
        c => output.push(c),
    }
}

/// Scan a run of literal spaces/tabs starting at `i`.
///
/// Returns the end of the run and whether the run is immediately followed by
/// a literal line break — in which case YAML line folding (spec 7.3) discards
/// the run. Escaped whitespace (`\t`, `\ `) never reaches this helper: escape
/// arms emit it directly, so it is always preserved as content.
#[inline]
fn scan_ws_run(bytes: &[u8], i: usize) -> (usize, bool) {
    let mut j = i;
    while j < bytes.len() && matches!(bytes[j], b' ' | b'\t') {
        j += 1;
    }
    (j, j < bytes.len() && is_line_break(bytes[j]))
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
                    // \N and \_ are C1/Latin-1 codepoints that yq streams as
                    // raw UTF-8 (not JSON-escaped) — route through
                    // write_json_escape so they get the same treatment as
                    // any other char >= 0x20 (#532).
                    b'N' => write_json_escape(output, '\u{0085}'), // next line
                    b'_' => write_json_escape(output, '\u{00a0}'), // non-breaking space
                    // \L and \P are always JSON-escaped by yq, even as raw
                    // literal bytes — not merely because write_json_escape
                    // treats them as "control" codepoints (it doesn't).
                    // Keep them hardcoded rather than routing through
                    // write_json_escape.
                    b'L' => output.push_str("\\u2028"), // line separator
                    b'P' => output.push_str("\\u2029"), // paragraph separator
                    b'\n' | b'\r' => {
                        // Escaped line break - skip entirely, CRLF included
                        i += line_break_len(bytes, i);
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
                            write_json_escape(output, char::from_u32(val).unwrap_or('\u{FFFD}'));
                        } else if val <= 0x7F {
                            output.push(val as u8 as char);
                        } else {
                            write_json_escape(output, char::from_u32(val).unwrap_or('\u{FFFD}'));
                        }
                        i += 2;
                    }
                    b'u' => {
                        // \uNNNN - 4 hex digits
                        if i + 4 >= bytes.len() {
                            return Err(YamlStringError::InvalidEscape);
                        }
                        let hex = &bytes[i + 1..i + 5];
                        let codepoint = parse_hex(hex)?;
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
            b'\t' => {
                // Literal whitespace run: folded away before a literal line
                // break, otherwise content (tabs escaped for JSON)
                let (j, at_break) = scan_ws_run(bytes, i);
                if !at_break {
                    for &b in &bytes[i..j] {
                        if b == b'\t' {
                            output.push_str("\\t");
                        } else {
                            output.push(' ');
                        }
                    }
                }
                i = j;
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
                // Trailing literal whitespace folds away before a literal
                // line break (the span can only end in spaces; tabs break it)
                let mut end = i;
                if i < bytes.len() {
                    let (j, at_break) = scan_ws_run(bytes, i);
                    if at_break {
                        while end > start && bytes[end - 1] == b' ' {
                            end -= 1;
                        }
                        i = j;
                    }
                }
                // Copy the safe span
                let chunk = core::str::from_utf8(&bytes[start..end])
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
            b'\t' => {
                // Literal whitespace run: folded away before a literal line
                // break, otherwise content (tabs escaped for JSON)
                let (j, at_break) = scan_ws_run(bytes, i);
                if !at_break {
                    for &b in &bytes[i..j] {
                        if b == b'\t' {
                            output.push_str("\\t");
                        } else {
                            output.push(' ');
                        }
                    }
                }
                i = j;
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
                // Trailing literal whitespace folds away before a literal
                // line break (the span can only end in spaces; tabs break it)
                let mut end = i;
                if i < bytes.len() {
                    let (j, at_break) = scan_ws_run(bytes, i);
                    if at_break {
                        while end > start && bytes[end - 1] == b' ' {
                            end -= 1;
                        }
                        i = j;
                    }
                }
                let chunk = core::str::from_utf8(&bytes[start..end])
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
/// Trailing literal whitespace before the break is handled by the callers
/// (folded away per YAML 7.3); escaped whitespace already in `output` is
/// content and must not be trimmed here.
fn transcode_fold_line_break_to_json(bytes: &[u8], mut i: usize, output: &mut String) -> usize {
    // Skip the first line break
    i += line_break_len(bytes, i);

    // Count empty lines
    let mut empty_lines = 0;
    loop {
        // Skip whitespace at start of line
        while i < bytes.len() && matches!(bytes[i], b' ' | b'\t') {
            i += 1;
        }

        // Check if this is an empty line
        if i < bytes.len() && is_line_break(bytes[i]) {
            empty_lines += 1;
            i += line_break_len(bytes, i);
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
        YamlString::BlockLiteral { .. } | YamlString::BlockFolded { .. } => {
            // Block scalars are always strings (yq/JSON semantics): no type
            // detection, and empty content is "" rather than null (#222)
            let decoded = s.as_str()?;
            write_json_string(output, &decoded);
            Ok(true)
        }
        YamlString::Unquoted { .. } => {
            // Plain scalars need type detection
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

/// Formats a finite `f64`, always keeping a fractional part.
///
/// A scalar that [`resolve_plain`] typed as `!!float` must not be emitted as an
/// integer: `1.0` is a float, and printing it as `1` changes its type on a
/// round trip (issue #169). Rust's `Display` for `f64` drops the `.0` from
/// whole values and never uses exponent notation, so the absence of a `.` is
/// the only case needing repair.
///
/// Shared with the streaming writer and the `yq` CLI's printers so every
/// emitter renders a float the same way.
#[must_use]
pub fn format_float_with_fraction(f: f64) -> String {
    let mut buf = f.to_string();
    if !buf.as_bytes().contains(&b'.') {
        buf.push_str(".0");
    }
    buf
}

/// Fast f64 to string formatting.
#[inline]
fn write_f64(output: &mut String, f: f64) {
    output.push_str(&format_float_with_fraction(f));
}

/// Fast YAML scalar to JSON conversion.
///
/// Resolution is delegated to [`resolve_plain`] (YAML 1.2 core schema);
/// this function only maps the resolution onto the JSON output buffer.
/// Numeric values are emitted from the parsed value, never echoed from the
/// source text (hex/octal like `0x2A` must appear as `42` in JSON).
#[inline]
fn write_yaml_scalar_as_json(output: &mut String, str_val: &str) {
    write_resolved_scalar_as_json(output, resolve_plain(str_val), str_val);
}

/// Write an already-resolved scalar as JSON. `str_val` is the original
/// source text, used only for the `Str` case (and only if `resolved` didn't
/// come from resolving it, e.g. tag-forced `!!str` on non-string content).
#[inline]
fn write_resolved_scalar_as_json(output: &mut String, resolved: ResolvedScalar, str_val: &str) {
    match resolved {
        ResolvedScalar::Null => output.push_str("null"),
        ResolvedScalar::Bool(true) => output.push_str("true"),
        ResolvedScalar::Bool(false) => output.push_str("false"),
        ResolvedScalar::Int(n) => write_i64(output, n),
        ResolvedScalar::Float(f) if f.is_finite() => write_f64(output, f),
        // JSON cannot represent the `.inf`/`.nan` family.
        ResolvedScalar::Float(_) => output.push_str("null"),
        ResolvedScalar::Str => write_json_string(output, str_val),
    }
}

// ============================================================================
// Streaming JSON Output (core::fmt::Write based)
// ============================================================================

/// Stream a YAML value as JSON from a bare `YamlValue` with no cursor of its
/// own. The streaming twin of [`write_yaml_value_as_json`] - see its doc
/// comment for why no production call site reaches this anymore (#224).
#[allow(dead_code)] // STYLE-0005: used in tests
fn stream_yaml_value_as_json<W: AsRef<[u64]>, Out: core::fmt::Write>(
    out: &mut Out,
    value: YamlValue<'_, W>,
    current_indent: usize,
    indent_spaces: usize,
) -> core::fmt::Result {
    match value {
        YamlValue::Null => out.write_str("null"),
        YamlValue::String(s) => match stream_yaml_string_to_json(out, &s) {
            Ok(true) => Ok(()),
            Ok(false) => {
                if let Ok(str_val) = s.as_str() {
                    if s.is_unquoted() {
                        // Plain scalar - resolve per the core schema
                        stream_yaml_scalar_as_json(out, &str_val)
                    } else {
                        // Block scalars are always strings
                        stream_json_string(out, &str_val)
                    }
                } else {
                    out.write_str("null")
                }
            }
            Err(_) => out.write_str("null"),
        },
        YamlValue::Mapping(fields) => {
            if fields.is_empty() {
                return out.write_str("{}");
            }
            out.write_char('{')?;
            let next_indent = current_indent + indent_spaces;
            let mut first = true;
            for field in fields {
                if !first {
                    out.write_char(',')?;
                }
                first = false;
                if indent_spaces > 0 {
                    out.write_char('\n')?;
                    write_yaml_indent(out, next_indent)?;
                }

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
                    // Alias/complex key (#222): resolve alias-to-scalar,
                    // else "" — the entry is kept, never dropped
                    stream_json_string(out, &field.key().key_string())?;
                }

                out.write_str(if indent_spaces > 0 { ": " } else { ":" })?;
                field
                    .value_cursor()
                    .stream_json_value(out, next_indent, indent_spaces)?;
            }
            if indent_spaces > 0 {
                out.write_char('\n')?;
                write_yaml_indent(out, current_indent)?;
            }
            out.write_char('}')
        }
        YamlValue::Sequence(elements) => {
            if elements.is_empty() {
                return out.write_str("[]");
            }
            out.write_char('[')?;
            let next_indent = current_indent + indent_spaces;
            let mut first = true;
            let mut rest = elements;
            while let Some((cursor, next)) = rest.uncons_cursor() {
                if !first {
                    out.write_char(',')?;
                }
                first = false;
                rest = next;
                if indent_spaces > 0 {
                    out.write_char('\n')?;
                    write_yaml_indent(out, next_indent)?;
                }
                // A child element does have a cursor even though this
                // function's own top-level `value` doesn't - use it so a
                // nested element's tag isn't lost either.
                cursor.stream_json_value(out, next_indent, indent_spaces)?;
            }
            if indent_spaces > 0 {
                out.write_char('\n')?;
                write_yaml_indent(out, current_indent)?;
            }
            out.write_char(']')
        }
        YamlValue::Alias { target, .. } => {
            if let Some(target_cursor) = target {
                target_cursor.stream_json_value(out, current_indent, indent_spaces)
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
        // Everything >= 0x20 (including C1 controls 0x80-0x9F and higher
        // codepoints) streams as raw UTF-8, matching `stream_json_string`'s
        // policy and yq's own output: JSON only requires escaping `"`, `\`,
        // and C0 controls. An earlier version of this function additionally
        // re-escaped non-ASCII characters as `\u00XX`/`\uXXXX`, which
        // diverged from `stream_json_string` and from yq for any string
        // reached through this char-by-char escape-decode path (#532).
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
                    // \N and \_ are C1/Latin-1 codepoints that yq streams as
                    // raw UTF-8 (not JSON-escaped) — route through
                    // stream_json_escape so they get the same treatment as
                    // any other char >= 0x20 (#532).
                    b'N' => stream_json_escape(out, '\u{0085}')
                        .map_err(|_| YamlStringError::InvalidUtf8)?,
                    b'_' => stream_json_escape(out, '\u{00a0}')
                        .map_err(|_| YamlStringError::InvalidUtf8)?,
                    // \L and \P are always JSON-escaped by yq, even as raw
                    // literal bytes — not merely because stream_json_escape
                    // treats them as "control" codepoints (it doesn't). Keep
                    // them hardcoded rather than routing through
                    // stream_json_escape.
                    b'L' => out
                        .write_str("\\u2028")
                        .map_err(|_| YamlStringError::InvalidUtf8)?,
                    b'P' => out
                        .write_str("\\u2029")
                        .map_err(|_| YamlStringError::InvalidUtf8)?,
                    b'\n' | b'\r' => {
                        // Escaped line break - skip entirely, CRLF included
                        i += line_break_len(bytes, i);
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
                        let ch = char::from_u32(val).unwrap_or('\u{FFFD}');
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
                        let codepoint = parse_hex(hex)?;
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
            b'\t' => {
                // Literal whitespace run: folded away before a literal line
                // break, otherwise content (tabs escaped for JSON)
                let (j, at_break) = scan_ws_run(bytes, i);
                if !at_break {
                    for &b in &bytes[i..j] {
                        if b == b'\t' {
                            out.write_str("\\t")
                                .map_err(|_| YamlStringError::InvalidUtf8)?;
                        } else {
                            out.write_char(' ')
                                .map_err(|_| YamlStringError::InvalidUtf8)?;
                        }
                    }
                }
                i = j;
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
                // Trailing literal whitespace folds away before a literal
                // line break (the span can only end in spaces; tabs break it)
                let mut end = i;
                if i < bytes.len() {
                    let (j, at_break) = scan_ws_run(bytes, i);
                    if at_break {
                        while end > start && bytes[end - 1] == b' ' {
                            end -= 1;
                        }
                        i = j;
                    }
                }
                let chunk = core::str::from_utf8(&bytes[start..end])
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
            b'\t' => {
                // Literal whitespace run: folded away before a literal line
                // break, otherwise content (tabs escaped for JSON)
                let (j, at_break) = scan_ws_run(bytes, i);
                if !at_break {
                    for &b in &bytes[i..j] {
                        if b == b'\t' {
                            out.write_str("\\t")
                                .map_err(|_| YamlStringError::InvalidUtf8)?;
                        } else {
                            out.write_char(' ')
                                .map_err(|_| YamlStringError::InvalidUtf8)?;
                        }
                    }
                }
                i = j;
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
                // Trailing literal whitespace folds away before a literal
                // line break (the span can only end in spaces; tabs break it)
                let mut end = i;
                if i < bytes.len() {
                    let (j, at_break) = scan_ws_run(bytes, i);
                    if at_break {
                        while end > start && bytes[end - 1] == b' ' {
                            end -= 1;
                        }
                        i = j;
                    }
                }
                let chunk = core::str::from_utf8(&bytes[start..end])
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

    while i < bytes.len() && is_line_break(bytes[i]) {
        i += line_break_len(bytes, i);
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
        YamlString::BlockLiteral { .. } | YamlString::BlockFolded { .. } => {
            // Block scalars are always strings (yq/JSON semantics): no type
            // detection, and empty content is "" rather than null (#222)
            let decoded = s.as_str()?;
            stream_json_string(out, &decoded).map_err(|_| YamlStringError::InvalidUtf8)?;
            Ok(true)
        }
        YamlString::Unquoted { .. } => Ok(false),
    }
}

/// Stream YAML scalar as JSON with type detection.
fn stream_yaml_scalar_as_json<Out: core::fmt::Write>(
    out: &mut Out,
    str_val: &str,
) -> core::fmt::Result {
    stream_resolved_scalar_as_json(out, resolve_plain(str_val), str_val)
}

/// Stream an already-resolved scalar as JSON. `str_val` is the original
/// source text, used only for the `Str` case (and only if `resolved` didn't
/// come from resolving it, e.g. tag-forced `!!str` on non-string content).
fn stream_resolved_scalar_as_json<Out: core::fmt::Write>(
    out: &mut Out,
    resolved: ResolvedScalar,
    str_val: &str,
) -> core::fmt::Result {
    match resolved {
        ResolvedScalar::Null => out.write_str("null"),
        ResolvedScalar::Bool(true) => out.write_str("true"),
        ResolvedScalar::Bool(false) => out.write_str("false"),
        ResolvedScalar::Int(n) => write!(out, "{n}"),
        // Not `write!(out, "{f}")`: that drops the `.0` from a whole float.
        ResolvedScalar::Float(f) if f.is_finite() => out.write_str(&format_float_with_fraction(f)),
        // JSON cannot represent the `.inf`/`.nan` family.
        ResolvedScalar::Float(_) => out.write_str("null"),
        ResolvedScalar::Str => stream_json_string(out, str_val),
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

impl<W> Clone for YamlChildren<'_, W> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<W> Copy for YamlChildren<'_, W> {}

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
///
/// The common case (`Direct`) walks the mapping's direct BP children lazily,
/// with no allocation. When the mapping contains a merge key (`<<`), fields
/// are resolved once into an ordered, deduplicated `Merged` list per YAML's
/// merge-key semantics (see [`resolve_merge_keys`]) and shared via `Rc` so
/// cloning during iteration stays O(1) rather than re-copying the list at
/// every step.
#[derive(Debug)]
enum FieldsInner<'a, W> {
    Direct(Option<YamlCursor<'a, W>>),
    Merged {
        entries: Rc<Vec<(YamlCursor<'a, W>, YamlCursor<'a, W>)>>,
        index: usize,
    },
}

impl<W> Clone for FieldsInner<'_, W> {
    fn clone(&self) -> Self {
        match self {
            FieldsInner::Direct(c) => FieldsInner::Direct(*c),
            FieldsInner::Merged { entries, index } => FieldsInner::Merged {
                entries: entries.clone(),
                index: *index,
            },
        }
    }
}

/// Immutable "list" of YAML mapping fields. See `FieldsInner` (private) for
/// the two representations this wraps.
#[derive(Debug)]
pub struct YamlFields<'a, W = Vec<u64>> {
    inner: FieldsInner<'a, W>,
}

// Manual, unconditional (no `W: Clone` bound) impl: `#[derive(Clone)]` would
// require `W: Clone`, which isn't in scope in the generic `W: AsRef<[u64]>`
// contexts this type is used in — and method resolution would then silently
// fall back to the blanket `impl Clone for &T`, cloning the *reference*
// rather than the value (a real Rust footgun caught by a type mismatch at
// the first `self.clone()` call site that reassigned the result).
impl<W> Clone for YamlFields<'_, W> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<'a, W: AsRef<[u64]>> YamlFields<'a, W> {
    /// Create a new YamlFields from a mapping cursor.
    pub fn from_mapping_cursor(mapping_cursor: YamlCursor<'a, W>) -> Self {
        let key_cursor = mapping_cursor.first_child();
        match resolve_merge_keys(key_cursor) {
            Some(entries) => Self {
                inner: FieldsInner::Merged {
                    entries: Rc::new(entries),
                    index: 0,
                },
            },
            None => Self {
                inner: FieldsInner::Direct(key_cursor),
            },
        }
    }

    /// Raw (merge-unaware) cons-list over a mapping's direct children.
    ///
    /// Used internally by [`resolve_merge_keys`] to walk a mapping's own
    /// fields, and to enumerate a merge source's own fields (which are
    /// copied verbatim, not recursively re-resolved — see that function's
    /// doc comment).
    fn raw(first_key: Option<YamlCursor<'a, W>>) -> Self {
        Self {
            inner: FieldsInner::Direct(first_key),
        }
    }

    /// Check if there are no more fields.
    #[inline]
    pub fn is_empty(&self) -> bool {
        match &self.inner {
            FieldsInner::Direct(c) => c.is_none(),
            FieldsInner::Merged { entries, index } => *index >= entries.len(),
        }
    }

    /// Get the first field and the remaining fields.
    pub fn uncons(&self) -> Option<(YamlField<'a, W>, Self)> {
        match &self.inner {
            FieldsInner::Direct(key_cursor) => {
                let key_cursor = (*key_cursor)?;
                let value_cursor = key_cursor.next_sibling()?;

                let rest = Self {
                    inner: FieldsInner::Direct(value_cursor.next_sibling()),
                };

                let field = YamlField {
                    key_cursor,
                    value_cursor,
                };

                Some((field, rest))
            }
            FieldsInner::Merged { entries, index } => {
                let &(key_cursor, value_cursor) = entries.get(*index)?;

                let rest = Self {
                    inner: FieldsInner::Merged {
                        entries: entries.clone(),
                        index: index + 1,
                    },
                };

                let field = YamlField {
                    key_cursor,
                    value_cursor,
                };

                Some((field, rest))
            }
        }
    }

    /// Find a field by name.
    ///
    /// YAML permits duplicate mapping keys; per YAML 1.2 and to match `yq`,
    /// the last matching entry wins (see issue #174). A `Merged` list is
    /// already deduplicated, so this loop is a no-op scan for it.
    pub fn find(&self, name: &str) -> Option<YamlValue<'a, W>> {
        let mut fields = self.clone();
        let mut result = None;
        while let Some((field, rest)) = fields.uncons() {
            if let YamlValue::String(key) = field.key() {
                if key.as_str().ok()? == name {
                    result = Some(field.value());
                }
            }
            fields = rest;
        }
        result
    }

    /// Find a field by name and return a cursor to its value.
    ///
    /// Same last-duplicate-key-wins semantics as [`find`](Self::find) — kept
    /// as a separate loop rather than reusing `find` so the returned cursor
    /// (needed for `line`/`column`) doesn't require re-navigating.
    pub fn find_cursor(&self, name: &str) -> Option<YamlCursor<'a, W>> {
        let mut fields = self.clone();
        let mut result = None;
        while let Some((field, rest)) = fields.uncons() {
            if let YamlValue::String(key) = field.key() {
                if key.as_str().ok()? == name {
                    result = Some(field.value_cursor());
                }
            }
            fields = rest;
        }
        result
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

/// Resolve YAML merge keys (`<<`) into an ordered, deduplicated field list.
///
/// Returns `None` when the mapping has no merge key at all, so the common
/// case stays a single forward pass with no `Vec`/`BTreeMap` allocation
/// beyond the small `seen` buffer described below. Otherwise resolves per
/// mikefarah/yq v4.53.3's actual (non-spec-compliant, but oracle-pinned —
/// see `docs/parsing/yaml.md`) behavior, verified empirically against that
/// binary:
///
/// - Fields are combined with "insert or update" semantics: a new key is
///   appended at the end; a key that already exists has its *value*
///   overwritten in place, keeping its original position. This single rule,
///   applied to the mapping's own keys in document order and interleaved
///   with each `<<`'s expansion at the point it occurs, reproduces both
///   "own key after `<<` wins" and "merge after own key wins" — yq applies
///   whichever writes last, textually, without `<<` getting special
///   priority (issue #171).
/// - `<<: [a, b, ...]` merges multiple sources. Per the merge-key spec, an
///   earlier-listed source must win value conflicts against a later one, so
///   the list is folded in *reverse*: applying `b` then `a` with the same
///   insert-or-update rule makes `a`'s value win while `b`'s unique keys
///   still claim the earlier positions (empirically confirmed: position
///   comes from whichever source is folded first, value from whichever is
///   folded last).
/// - A merge source's own fields are taken verbatim: a `<<` key nested
///   inside a source is copied as an ordinary key, not expanded recursively.
///   yq does not recurse into a merged-in mapping's own merge key.
/// - An invalid merge value (null, scalar, an alias that doesn't resolve to
///   a mapping, or a non-mapping sequence element) contributes nothing
///   rather than erroring, matching yq's silent-skip behavior.
///
/// # Why this decodes each key exactly once
///
/// A `YamlCursor::value()` call is not a free re-read: for a non-container
/// node it consults `AdvancePositions`/`CompactEndPositions`
/// (`src/yaml/advance_positions.rs`, `src/yaml/end_positions.rs`), each of
/// which keeps a single *document-wide* sequential cursor optimized for
/// monotonically increasing access (amortized O(1)). A first version of this
/// function scanned the mapping once with `.any()` to check for a merge key,
/// then — merge key or not — walked it again from the start to build the
/// result. Decoding the same key a second time is a backward jump against
/// that shared cursor, which falls back to `get_random`; that fallback
/// resets the incremental IB-scan state to the very start of the document's
/// index instead of to the resumed position, so the *next* sequential call
/// has to rescan from position zero to catch back up — O(current document
/// position), paid on every mapping regardless of whether it had a merge
/// key. Over N sibling mappings (an array of flat records — one of the most
/// common real-world YAML/JSON shapes) that turns an O(N) document walk into
/// O(N²) (#171 review).
///
/// So: walk forward exactly once, buffering each already-decoded
/// `YamlValue` (not just the cursor) in `seen` as we go. If a later field
/// turns out to be a merge key, `seen`'s entries are folded into the
/// deduplicated result using the value already in hand — never re-derived
/// via a second `.value()` call on an earlier cursor. If no merge key is
/// ever found, `seen` is simply dropped and `None` is returned; the
/// `Vec`/`BTreeMap` used for deduplication are allocated only once a merge
/// key is confirmed to exist.
fn resolve_merge_keys<'a, W: AsRef<[u64]>>(
    first_key: Option<YamlCursor<'a, W>>,
) -> Option<Vec<(YamlCursor<'a, W>, YamlCursor<'a, W>)>> {
    let mut fields = YamlFields::raw(first_key);
    let mut seen: Vec<(YamlCursor<'a, W>, YamlCursor<'a, W>, YamlValue<'a, W>)> = Vec::new();

    loop {
        let (field, rest) = fields.uncons()?;
        let key_value = field.key_cursor.value();

        if is_merge_key_value(&key_value) {
            let mut entries: Vec<(YamlCursor<'a, W>, YamlCursor<'a, W>)> =
                Vec::with_capacity(seen.len() + 1);
            let mut positions: BTreeMap<String, usize> = BTreeMap::new();

            for (key, value, key_value) in seen {
                upsert_field(&mut entries, &mut positions, key, value, &key_value);
            }
            merge_field_into(&mut entries, &mut positions, field.value_cursor);

            fields = rest;
            while let Some((field, rest)) = fields.uncons() {
                let key_value = field.key_cursor.value();
                if is_merge_key_value(&key_value) {
                    merge_field_into(&mut entries, &mut positions, field.value_cursor);
                } else {
                    upsert_field(
                        &mut entries,
                        &mut positions,
                        field.key_cursor,
                        field.value_cursor,
                        &key_value,
                    );
                }
                fields = rest;
            }

            return Some(entries);
        }

        seen.push((field.key_cursor, field.value_cursor, key_value));
        fields = rest;
    }
}

/// Expand one `<<` field's merge sources into `entries`/`positions`.
///
/// Reverse the source list so an earlier-listed source (higher merge-spec
/// priority) is applied last and wins value conflicts, while a later-listed
/// source's unique keys still claim the earlier positions (see
/// `resolve_merge_keys`'s doc comment).
fn merge_field_into<'a, W: AsRef<[u64]>>(
    entries: &mut Vec<(YamlCursor<'a, W>, YamlCursor<'a, W>)>,
    positions: &mut BTreeMap<String, usize>,
    value_cursor: YamlCursor<'a, W>,
) {
    for source in merge_sources(value_cursor).into_iter().rev() {
        for source_field in YamlFields::raw(source.first_child()) {
            let key_value = source_field.key_cursor.value();
            upsert_field(
                entries,
                positions,
                source_field.key_cursor,
                source_field.value_cursor,
                &key_value,
            );
        }
    }
}

/// Insert a field by name, or overwrite an existing entry's value in place.
///
/// `key_value` is the key's already-decoded value, passed in rather than
/// re-derived from `key` — see `resolve_merge_keys`'s doc comment for why a
/// second `YamlCursor::value()` call on an already-visited key is expensive.
fn upsert_field<'a, W: AsRef<[u64]>>(
    entries: &mut Vec<(YamlCursor<'a, W>, YamlCursor<'a, W>)>,
    positions: &mut BTreeMap<String, usize>,
    key: YamlCursor<'a, W>,
    value: YamlCursor<'a, W>,
    key_value: &YamlValue<'a, W>,
) {
    let name = key_value.key_string().into_owned();
    match positions.get(&name) {
        Some(&pos) => entries[pos] = (key, value),
        None => {
            positions.insert(name, entries.len());
            entries.push((key, value));
        }
    }
}

/// Is this key's already-decoded value YAML's merge key?
///
/// Empirically, mikefarah/yq treats a key whose *decoded* content is `<<` as
/// a merge key even when quoted (`"<<"`), so this compares decoded string
/// content rather than requiring an unquoted plain scalar.
fn is_merge_key_value<W: AsRef<[u64]>>(key_value: &YamlValue<'_, W>) -> bool {
    matches!(key_value, YamlValue::String(s) if matches!(s.as_str(), Ok(ref v) if v == "<<"))
}

/// Resolve a `<<` value to the ordered list of mapping sources it names.
///
/// Handles a direct inline mapping, an alias to a mapping, or a sequence
/// mixing either (each element resolved the same way). Anything else
/// (null, scalar, an alias to a non-mapping, or a non-mapping sequence
/// element) yields no sources — merging from it is a silent no-op, matching
/// yq rather than erroring.
fn merge_sources<W: AsRef<[u64]>>(value_cursor: YamlCursor<'_, W>) -> Vec<YamlCursor<'_, W>> {
    fn as_mapping<W: AsRef<[u64]>>(cursor: YamlCursor<'_, W>) -> bool {
        matches!(cursor.value(), YamlValue::Mapping(_))
    }

    match value_cursor.value() {
        YamlValue::Mapping(_) => vec![value_cursor],
        YamlValue::Alias {
            target: Some(target),
            ..
        } if as_mapping(target) => vec![target],
        YamlValue::Sequence(elements) => {
            let mut sources = Vec::new();
            let mut rest = elements;
            while let Some((cursor, tail)) = rest.uncons_cursor() {
                match cursor.value() {
                    YamlValue::Mapping(_) => sources.push(cursor),
                    YamlValue::Alias {
                        target: Some(target),
                        ..
                    } if as_mapping(target) => sources.push(target),
                    _ => {}
                }
                rest = tail;
            }
            sources
        }
        _ => Vec::new(),
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

impl<W> Clone for YamlField<'_, W> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<W> Copy for YamlField<'_, W> {}

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

    /// Get the key cursor directly.
    ///
    /// This allows raw-byte access to the key without decoding through
    /// `YamlValue` first.
    #[inline]
    pub fn key_cursor(&self) -> YamlCursor<'a, W> {
        self.key_cursor
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

impl<W> Clone for YamlElements<'_, W> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<W> Copy for YamlElements<'_, W> {}

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
    pub fn uncons_cursor(&self) -> Option<(YamlCursor<'a, W>, Self)> {
        let element_cursor = self.element_cursor?;

        let rest = YamlElements {
            element_cursor: element_cursor.next_sibling(),
        };

        // For a block sequence, element_cursor points at the item wrapper node (at the
        // `-`) and the value lives in its first child; unwrap so callers that use the
        // cursor *positionally* — `is_yaml_cursor_container`, which decides block-vs-inline
        // YAML layout — see the content rather than the wrapper. For flow sequences,
        // virtual root sequences, and document containers the element IS the value.
        //
        // Deliberately `starts_inline_seq_entry`, not the wider `starts_seq_entry` that
        // `value()` uses: a bare `-` stays pointed at the wrapper, which is what
        // `corpus_stats`'s bare-dash counting reads. Value decoding is the same either way.
        //
        // A childless wrapper is returned as-is: `value()` decides whether it is an empty
        // sequence item (null) or a plain scalar that merely begins `- ` (#332).
        if element_cursor.is_container() {
            Some((element_cursor, rest))
        } else if element_cursor
            .text_position()
            .is_some_and(|text_pos| starts_inline_seq_entry(element_cursor.text, text_pos))
        {
            Some((element_cursor.first_child().unwrap_or(element_cursor), rest))
        } else {
            Some((element_cursor, rest))
        }
    }

    /// Get the first element and the remaining elements.
    pub fn uncons(&self) -> Option<(YamlValue<'a, W>, Self)> {
        // Delegate rather than re-deriving the sequence-item unwrap: this used to be a
        // third open-coded copy of the `- ` test with its own acceptance set (#332).
        let (cursor, rest) = self.uncons_cursor()?;
        Some((cursor.value(), rest))
    }

    /// Get element by index.
    pub fn get(&self, index: usize) -> Option<YamlValue<'a, W>> {
        let mut cursor = self.element_cursor?;
        for _ in 0..index {
            cursor = cursor.next_sibling()?;
        }
        // `value()` performs the sequence-item unwrap itself, so no `- ` test is needed
        // here — this site used to carry one that omitted the whitespace check entirely.
        Some(cursor.value())
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

    /// How far a block scalar's content lines are indented, or `None` when the
    /// block has no content lines at all.
    ///
    /// `content_start` is the first byte after the indicator line's break.
    ///
    /// `None` covers two shapes that have to be treated alike: nothing but empty
    /// lines from here to EOF, and a first line no more indented than the block's
    /// own key — a sibling, a dedented item, or a comment — which ends the block
    /// before it starts. Either way everything inside the block is `l-empty`,
    /// whose leading whitespace is indentation and not content.
    ///
    /// One definition on purpose. [`Self::find_block_content_range`] uses it to
    /// decide where the content ends and the decoders use it to decide what to
    /// strip; while those were separate copies they disagreed, because the
    /// decoders' copy dropped the `> base_indent` test and so stripped nothing
    /// from a block that a comment line had ended (#344).
    fn block_content_indent(
        text: &[u8],
        indicator_pos: usize,
        content_start: usize,
        explicit_indent: Option<u8>,
    ) -> Option<usize> {
        // The base indent is the mapping key's (for `key: |`) or the line's.
        // For `- key: |`, the key is at indent 2 (after `- `), not 0.
        let base_indent = Self::compute_key_indent(text, indicator_pos);

        // Explicit indentation: content is at base_indent + N spaces.
        if let Some(indent) = explicit_indent {
            return Some(base_indent + indent as usize);
        }

        // Auto-detect from the first non-empty line. On a document-start line
        // (`--- |`) a zero-indented block scalar is valid, so indent 0 counts.
        match Self::detect_block_indent(text, content_start) {
            Some(indent)
                if indent > base_indent
                    || (Self::is_document_start_line(text, indicator_pos) && indent == 0) =>
            {
                Some(indent)
            }
            _ => None,
        }
    }

    /// Find content range for a block scalar.
    /// Returns (content_start, content_end).
    fn find_block_content_range(
        text: &[u8],
        indicator_pos: usize,
        chomping: ChompingIndicator,
        explicit_indent: Option<u8>,
    ) -> (usize, usize) {
        // Skip indicator and modifiers to find the end of the indicator line
        let mut pos = indicator_pos + 1;
        while pos < text.len() && !is_line_break(text[pos]) {
            pos += 1;
        }
        if pos >= text.len() {
            return (pos, pos); // Empty block scalar
        }
        pos += line_break_len(text, pos); // Skip the line break

        let content_start = pos;

        let content_indent =
            match Self::block_content_indent(text, indicator_pos, content_start, explicit_indent) {
                Some(indent) => indent,
                None => {
                    // Content is not more indented than the indicator line, so the block has
                    // no content lines. All that can remain is `l-keep-empty(n)` — a run of
                    // empty lines that only keep chomping preserves (YAML 1.2 §8.1.1.2).
                    // The break that ended the indicator line is *not* one of them: it
                    // belongs to the header's `s-b-comment`, so a block with no empty lines
                    // is `""`, not `"\n"` (#344).
                    if chomping == ChompingIndicator::Keep {
                        // Find all trailing newlines/empty lines from content_start
                        let mut end = content_start;
                        while end < text.len() {
                            // Count spaces
                            let mut spaces = 0;
                            while end + spaces < text.len() && text[end + spaces] == b' ' {
                                spaces += 1;
                            }
                            // Spaces running to EOF are not an `l-empty`: that production
                            // ends in `b-as-line-feed`, and there is no break here to feed.
                            if end + spaces >= text.len() {
                                break;
                            }
                            let break_len = line_break_len(text, end + spaces);
                            if break_len == 0 {
                                // Non-empty line at same or lower indent = end of block
                                break;
                            }
                            end += spaces + break_len;
                        }
                        // `end == content_start` means zero empty lines, which under keep
                        // is `""`. Reaching back past `content_start` to borrow the
                        // indicator line's own terminator would fabricate a break the
                        // scalar never had (#344).
                        return (content_start, end);
                    }
                    return (content_start, content_start);
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

            let break_len = line_break_len(text, pos);
            if break_len > 0 {
                // Empty line - include in content area
                pos += break_len;
            } else {
                if line_indent < content_indent {
                    // Dedent - end of block
                    pos = line_start;
                    break;
                }

                // Content line - skip to end
                while pos < text.len() && !is_line_break(text[pos]) {
                    pos += 1;
                }
                last_content_end = pos;
                has_content = true;

                pos += line_break_len(text, pos);
            }
        }

        // Apply chomping
        let content_end = match chomping {
            ChompingIndicator::Strip => last_content_end,
            ChompingIndicator::Clip => {
                // Clip: Include exactly one trailing newline if there was content
                if has_content {
                    // Include one newline after last content
                    last_content_end + line_break_len(text, last_content_end)
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
        while line_start > 0 && !is_line_break(text[line_start - 1]) {
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
                }
                return line_indent;
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
        while line_start > 0 && !is_line_break(text[line_start - 1]) {
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
                b'\n' | b'\r' => {
                    pos += line_break_len(text, pos);
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
            Self::InvalidUtf8 => write!(f, "invalid UTF-8 in string"),
            Self::InvalidEscape => write!(f, "invalid escape sequence"),
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
                    b'\n' | b'\r' => {
                        // Escaped line break - skip it entirely (no space added),
                        // CRLF included. Also skip leading whitespace on next line.
                        i += line_break_len(bytes, i);
                        while i < bytes.len() && matches!(bytes[i], b' ' | b'\t') {
                            i += 1;
                        }
                        continue; // Don't increment i again at end of loop
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
                            result.push(char::from_u32(val).ok_or(YamlStringError::InvalidEscape)?);
                        }
                        i += 2;
                    }
                    b'u' => {
                        // \uNNNN - 4 hex digits
                        if i + 4 >= bytes.len() {
                            return Err(YamlStringError::InvalidEscape);
                        }
                        let hex = &bytes[i + 1..i + 5];
                        let codepoint = parse_hex(hex)?;
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
                // Trailing literal whitespace folds away before a literal
                // line break; escaped whitespace was pushed by the escape
                // arms and is never trimmed
                let mut end = i;
                if i < bytes.len() && is_line_break(bytes[i]) {
                    while end > start && matches!(bytes[end - 1], b' ' | b'\t') {
                        end -= 1;
                    }
                }
                let chunk = core::str::from_utf8(&bytes[start..end])
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
                    && !is_line_break(bytes[i])
                    && !(bytes[i] == b'\'' && i + 1 < bytes.len() && bytes[i + 1] == b'\'')
                {
                    i += 1;
                }
                // Trailing literal whitespace folds away before a literal
                // line break
                let mut end = i;
                if i < bytes.len() && is_line_break(bytes[i]) {
                    while end > start && matches!(bytes[end - 1], b' ' | b'\t') {
                        end -= 1;
                    }
                }
                let chunk = core::str::from_utf8(&bytes[start..end])
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
/// - Skip the line break
/// - Count consecutive empty lines (they become \n)
/// - Skip leading whitespace on the continuation line
/// - Add a space for the line break (or \n for each empty line)
///
/// Trailing literal whitespace before the break is trimmed by the callers;
/// escaped whitespace already in `result` is content and must not be
/// trimmed here.
fn fold_quoted_line_break(bytes: &[u8], mut i: usize, result: &mut String) -> usize {
    // Skip the first line break
    i += line_break_len(bytes, i);

    // Count empty lines (lines with only whitespace)
    let mut empty_lines = 0;
    loop {
        // Skip whitespace at start of line
        while i < bytes.len() && matches!(bytes[i], b' ' | b'\t') {
            i += 1;
        }

        // Check if this is an empty line
        if i < bytes.len() && is_line_break(bytes[i]) {
            empty_lines += 1;
            i += line_break_len(bytes, i);
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
                while i < bytes.len() && !is_line_break(bytes[i]) {
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
    i += line_break_len(bytes, i);

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
        if i < bytes.len() && is_line_break(bytes[i]) {
            empty_lines += 1;
            i += line_break_len(bytes, i);
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
            if check >= bytes.len() || is_line_break(bytes[check]) {
                // Line is all whitespace - treat as empty line
                empty_lines += 1;
                i = check + line_break_len(bytes, check);
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
            }
            // This shouldn't happen if find_plain_scalar_end worked correctly
            // but handle it gracefully
            break;
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

/// The indent the decoders use when [`YamlString::block_content_indent`] returns
/// `None`: strip every line's leading run of spaces, however long.
///
/// Both decoders consume this through `line_indent.min(indent)` and a
/// `line_indent > indent` test, so `usize::MAX` reads as "all of it" and "never
/// more-indented" respectively — which is only the right answer because `None`
/// means the range is nothing but `l-empty` lines, whose leading whitespace is
/// indentation rather than content. That invariant is established over in
/// `find_block_content_range`, a call away, so assert it here rather than trust
/// it: were the `None` branch ever to start yielding real content, the bug would
/// be silently stripped text instead of a failing test.
fn strip_whole_indent_run(content: &[u8]) -> usize {
    debug_assert!(
        content
            .iter()
            .all(|&b| b == b' ' || b == b'\n' || b == b'\r'),
        "block_content_indent returned None over non-blank block scalar content"
    );
    usize::MAX
}

/// Decode a literal block scalar (preserves newlines).
fn decode_block_literal(
    text: &[u8],
    indicator_pos: usize,
    chomping: ChompingIndicator,
    explicit_indent: Option<u8>,
) -> Result<Cow<'_, str>, YamlStringError> {
    let (content_start, content_end) =
        YamlString::find_block_content_range(text, indicator_pos, chomping, explicit_indent);

    if content_start >= content_end {
        // No content lines and no empty lines. There is nothing for keep chomping to
        // keep either: `l-keep-empty(n)` covers the block's own empty lines, not the
        // break that ended the indicator line (YAML 1.2 §8.1.1.2, #344).
        return Ok(Cow::Borrowed(""));
    }

    let content = &text[content_start..content_end];

    // No content lines means the range is all `l-empty`, whose leading spaces are
    // indentation rather than content — strip each line's run whole. Otherwise
    // `|+` over `   \n` yields `"   \n"` where YAML 1.2 and `yq` give `"\n"`
    // (yaml-test-suite JEF9/01).
    let indent =
        YamlString::block_content_indent(text, indicator_pos, content_start, explicit_indent)
            .unwrap_or_else(|| strip_whole_indent_run(content));
    if indent == 0 && !content.contains(&b'\r') {
        // No indentation to strip and no line breaks to normalize - borrow as is.
        // Content holding a `\r` must go through the loop below, which rewrites
        // every break form to `\n` as YAML 1.2 §5.4 requires; borrowing it would
        // emit the raw CRs instead (#324).
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
        while pos < content.len() && !is_line_break(content[pos]) {
            pos += 1;
        }

        // Append line content
        let line = core::str::from_utf8(&content[line_start..pos])
            .map_err(|_| YamlStringError::InvalidUtf8)?;
        result.push_str(line);

        // Handle line ending. Every break form is rewritten to a single `\n`,
        // as YAML 1.2 §5.4 requires of the presented content.
        let break_len = line_break_len(content, pos);
        pos += break_len;
        if break_len > 0 {
            result.push('\n');
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

/// Decode a folded block scalar, applying YAML 1.2 §8.1.3 line folding.
///
/// Only *some* breaks fold. Between two content lines separated by `k` blank
/// lines:
///
/// | | `k == 0` | `k > 0` |
/// |---------------------------|----------|---------------------|
/// | neither line more-indented | `" "`   | `"\n"` × `k`        |
/// | either line more-indented  | `"\n"`  | `"\n"` × (`1 + k`)  |
///
/// That is: the break *preceding* a run of blank lines is folded away and each
/// blank line contributes the newline (`b-l-trimmed`), so N blank lines give N
/// newlines — not N+1 (#329). A more-indented line on either side suppresses
/// folding, so its break survives *in addition to* the blank lines
/// (`b-l-spaced`). Blank lines before the first content line contribute one
/// newline each.
///
/// After the last content line comes `b-chomped-last` (one newline) plus one
/// newline per trailing blank line; the chomping match at the end trims that
/// tail to strip/clip/keep. Every newline in that tail must correspond to a
/// break that is actually present in the text: a block scalar whose last line
/// runs to EOF without a break has no `b-chomped-last` to keep, and `yq` agrees
/// (`>+` over `  x` with no final newline is `"x"`, not `"x\n"`).
fn decode_block_folded(
    text: &[u8],
    indicator_pos: usize,
    chomping: ChompingIndicator,
    explicit_indent: Option<u8>,
) -> Result<Cow<'_, str>, YamlStringError> {
    let (content_start, content_end) =
        YamlString::find_block_content_range(text, indicator_pos, chomping, explicit_indent);

    if content_start >= content_end {
        // See `decode_block_literal`: an empty range is `""` under every chomping,
        // keep included (#344).
        return Ok(Cow::Borrowed(""));
    }

    let content = &text[content_start..content_end];

    // See `decode_block_literal`: `None` means every line here is `l-empty`, so
    // strip its leading whitespace whole.
    let indent =
        YamlString::block_content_indent(text, indicator_pos, content_start, explicit_indent)
            .unwrap_or_else(|| strip_whole_indent_run(content));

    // Build result by folding newlines
    let mut result = String::with_capacity(content.len());
    let mut pos = 0;
    // Blank lines seen since the last content line. Counted rather than emitted
    // eagerly: how many newlines they are worth depends on the line that
    // follows them, which has not been read yet.
    let mut pending_empty = 0usize;
    let mut prev_was_more_indented = false;
    let mut first_line = true;
    // Did the most recent content line end with a break? False when the block
    // runs to EOF unterminated, in which case there is no `b-chomped-last`.
    let mut last_line_had_break = false;

    while pos < content.len() {
        // Count indentation
        let mut line_indent = 0;
        while pos + line_indent < content.len() && content[pos + line_indent] == b' ' {
            line_indent += 1;
        }

        // Check if this is a blank line
        let is_blank =
            pos + line_indent >= content.len() || is_line_break(content[pos + line_indent]);

        if is_blank {
            // Count it only if its break is really there — trailing whitespace
            // running to EOF is not a line. The next content line decides what
            // the count is worth. `prev_was_more_indented` deliberately
            // survives a blank run: it is what distinguishes 1+k newlines
            // from k.
            pos += line_indent;
            let break_len = line_break_len(content, pos);
            pos += break_len;
            if break_len > 0 {
                pending_empty += 1;
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
            while pos < content.len() && !is_line_break(content[pos]) {
                pos += 1;
            }

            // Join to what came before (see the table on this fn)
            if first_line {
                // Leading blank lines: one newline each, no join character
                for _ in 0..pending_empty {
                    result.push('\n');
                }
            } else if pending_empty > 0 {
                // The break before the blank run only survives when folding is
                // suppressed by a more-indented line on either side
                if is_more_indented || prev_was_more_indented {
                    result.push('\n');
                }
                for _ in 0..pending_empty {
                    result.push('\n');
                }
            } else if is_more_indented || prev_was_more_indented {
                result.push('\n');
            } else {
                result.push(' ');
            }
            pending_empty = 0;

            // Append line content
            let line = core::str::from_utf8(&content[line_start..pos])
                .map_err(|_| YamlStringError::InvalidUtf8)?;
            result.push_str(line);

            prev_was_more_indented = is_more_indented;
            first_line = false;

            // Skip line ending, remembering whether there was one
            let break_len = line_break_len(content, pos);
            pos += break_len;
            last_line_had_break = break_len > 0;
        }
    }

    // Emit the tail as keep-chomping would see it: `b-chomped-last` (the break
    // that ends the final content line) plus one newline per trailing blank
    // line. The chomping match below then trims it. Without the first push,
    // `>+` loses the final line break entirely (`>+` over `a\n` yielded `"a"`
    // where yq yields `"a\n"`); without its guard, an unterminated final line
    // gains one it never had (`>+` over `a` — no final break — would yield
    // `"a\n"` where yq yields `"a"`).
    if last_line_had_break {
        result.push('\n');
    }
    for _ in 0..pending_empty {
        result.push('\n');
    }

    // Apply chomping at the end
    match chomping {
        ChompingIndicator::Strip => {
            while result.ends_with('\n') {
                result.pop();
            }
        }
        ChompingIndicator::Clip => {
            // Trim to at most one trailing newline. Never *add* one: the tail
            // above already emitted `b-chomped-last` whenever the text has it,
            // so a result not ending in `\n` here is an unterminated final line
            // that clip must leave alone (`>` over `  x` with no final break is
            // `"x"` in yq, not `"x\n"`).
            while result.ends_with("\n\n") {
                result.pop();
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
            Self::InvalidUtf8 => write!(f, "invalid UTF-8 in number"),
            Self::InvalidNumber => write!(f, "invalid number format"),
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
    fn anchor(&self) -> Option<&str> {
        YamlCursor::anchor(self)
    }

    #[inline]
    fn style(&self) -> &'static str {
        YamlCursor::style(self)
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
    fn stream_json<Out: core::fmt::Write>(
        &self,
        out: &mut Out,
        indent_spaces: usize,
    ) -> core::fmt::Result {
        YamlCursor::stream_json(self, out, indent_spaces)
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
            YamlValue::String(s) => {
                let Ok(str_val) = s.as_str() else {
                    return false;
                };
                if let Some(explicit) = self.explicit_tag() {
                    if let Some(resolved) = resolve_tagged(&str_val, explicit) {
                        return matches!(
                            resolved,
                            ResolvedScalar::Null | ResolvedScalar::Bool(false)
                        );
                    }
                }
                s.is_unquoted()
                    && could_be_null_or_bool(&str_val)
                    && matches!(
                        resolve_plain(&str_val),
                        ResolvedScalar::Null | ResolvedScalar::Bool(false)
                    )
            }
            _ => false,
        }
    }
}

impl<'a, W: AsRef<[u64]>> YamlValue<'a, W> {
    /// Mapping-key text under yq/JSON semantics (issue #222).
    ///
    /// A key is always emitted as a string. Unlike a value, a key is never
    /// type-inferred and an entry with a complex key is never dropped:
    /// - `String` → its decoded content (raw; no `null`/bool/number coercion)
    /// - `Alias` whose target resolves to a scalar string → that string
    /// - anything else (mapping, sequence, null, error, unresolved or
    ///   non-scalar alias) → `""`
    ///
    /// Never fails; returns `""` rather than erroring so keys are never lost.
    pub fn key_string(&self) -> Cow<'a, str> {
        match self {
            YamlValue::String(s) => s.as_str().unwrap_or(Cow::Borrowed("")),
            YamlValue::Alias {
                target: Some(target),
                ..
            } => {
                if let YamlValue::String(s) = target.value() {
                    s.as_str().unwrap_or(Cow::Borrowed(""))
                } else {
                    Cow::Borrowed("")
                }
            }
            _ => Cow::Borrowed(""),
        }
    }
}

// KNOWN LIMITATION (#224): the typed getters below (`is_null`, `as_bool`,
// `as_i64`, `as_f64`, `type_name`) are not tag-aware, unlike every JSON
// output path (`write_json_to`/`stream_json_value`) and the CLI's
// `yaml_to_owned_value`. A bare `YamlValue` has no `bp_pos` to look an
// explicit tag up with — only a `YamlCursor` does (`explicit_tag()`) — and
// `crate::jq::eval_generic::to_owned`, the generic evaluator's lazy
// materializer backing `select`, `==`, arithmetic, and `type` on the
// cursor-evaluator path, calls these directly on a `DocumentValue` with no
// cursor in scope. So `echo 'a: !!str 1' | succinctly yq '.a | type'` says
// `"number"`, not `"string"` — while `succinctly yq '.'`'s JSON output for
// the same input is correct. Fixing this needs either threading a cursor
// through `to_owned`'s recursion (it can: `DocumentField::value_cursor` and
// `DocumentElements::uncons_cursor` already exist) or embedding the tag in
// `YamlValue::String` itself, which fans out to ~90 pattern-match sites
// across 6 files. Scoped out of #224 as a follow-up rather than risking an
// under-tested change to a evaluator shared with JSON this late.
impl<'a, W: AsRef<[u64]> + Clone> DocumentValue for YamlValue<'a, W> {
    type Cursor = YamlCursor<'a, W>;
    type Fields = YamlFields<'a, W>;
    type Elements = YamlElements<'a, W>;

    fn is_null(&self) -> bool {
        match self {
            YamlValue::Null => true,
            YamlValue::String(s) if s.is_unquoted() => {
                // YAML null values: null, Null, NULL, ~, empty string
                if let Ok(str_val) = s.as_str() {
                    could_be_null_or_bool(&str_val)
                        && resolve_plain(&str_val) == ResolvedScalar::Null
                } else {
                    false
                }
            }
            YamlValue::Alias { target, .. } => {
                // An alias is null exactly when its target is. The `None` arm is
                // unreachable for an index from `YamlIndex::build`, which since
                // #372 refuses an alias it cannot resolve; it is kept, rather
                // than unwrapped, so a hand-built index cannot panic here.
                target.map_or(true, |t| t.value().is_null())
            }
            _ => false,
        }
    }

    fn as_bool(&self) -> Option<bool> {
        match self {
            YamlValue::String(s) if s.is_unquoted() => {
                let str_val = s.as_str().ok()?;
                if !could_be_null_or_bool(&str_val) {
                    return None;
                }
                match resolve_plain(&str_val) {
                    ResolvedScalar::Bool(b) => Some(b),
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
                match resolve_plain(&str_val) {
                    ResolvedScalar::Int(n) => Some(n),
                    _ => None,
                }
            }
            YamlValue::Alias { target, .. } => target.and_then(|t| t.value().as_i64()),
            _ => None,
        }
    }

    fn as_f64(&self) -> Option<f64> {
        match self {
            YamlValue::String(s) if s.is_unquoted() => {
                let str_val = s.as_str().ok()?;
                // Non-finite floats arise only from the `.inf`/`.nan` family
                match resolve_plain(&str_val) {
                    ResolvedScalar::Int(n) => Some(n as f64),
                    ResolvedScalar::Float(f) => Some(f),
                    _ => None,
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

    fn key_string(&self) -> Option<Cow<'_, str>> {
        // Keys are never dropped and never type-inferred (issue #222):
        // complex keys stringify to "" rather than resolving to None.
        Some(YamlValue::key_string(self))
    }

    fn as_object(&self) -> Option<Self::Fields> {
        match self {
            YamlValue::Mapping(fields) => Some(fields.clone()),
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
                // Determine effective type per the YAML 1.2 core schema
                if s.is_unquoted() {
                    if let Ok(str_val) = s.as_str() {
                        return resolve_plain(&str_val).type_name();
                    }
                }
                "string"
            }
            YamlValue::Mapping(_) => "object",
            YamlValue::Sequence(_) => "array",
            YamlValue::Alias { target, .. } => target.map_or("null", |t| t.value().type_name()),
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
                key_cursor: field.key_cursor(),
                value_cursor: field.value_cursor(),
            },
            rest,
        ))
    }

    fn find(&self, name: &str) -> Option<Self::Value> {
        YamlFields::find(self, name)
    }

    fn find_cursor(&self, name: &str) -> Option<Self::Cursor> {
        YamlFields::find_cursor(self, name)
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

/// Stream independent document cursors as a single YAML sequence (block or
/// flow style), without materializing an `OwnedValue` DOM.
///
/// Unlike `YamlCursor::stream_yaml_value`'s `Sequence` arm, the cursors
/// here need not share one `YamlIndex` — each may come from its own
/// document/source. This is what lets `--slurp` wrap multiple slurped
/// documents into one array while preserving duplicate mapping keys within
/// each (#478); an `OwnedValue`-based `IndexMap` cannot represent those.
///
/// Mirrors the `Sequence` block/flow-style rendering in `stream_yaml_value`
/// exactly (same container-vs-scalar branching, same empty-sequence `"[]"`
/// shortcut) so multi-document slurped output matches single-document M2
/// streaming byte-for-byte.
pub fn stream_yaml_sequence<'a, W, I, Out>(
    cursors: I,
    out: &mut Out,
    current_indent: usize,
    indent_spaces: usize,
) -> core::fmt::Result
where
    W: AsRef<[u64]> + 'a,
    I: IntoIterator<Item = YamlCursor<'a, W>>,
    Out: core::fmt::Write,
{
    let mut iter = cursors.into_iter().peekable();
    if iter.peek().is_none() {
        return out.write_str("[]");
    }
    if indent_spaces == 0 {
        // Flow style
        out.write_char('[')?;
        let mut first = true;
        for cursor in iter {
            if !first {
                out.write_str(", ")?;
            }
            first = false;
            cursor.stream_yaml_value(out, 0, 0)?;
        }
        out.write_char(']')
    } else {
        // Block style
        let mut first = true;
        for cursor in iter {
            if !first {
                out.write_char('\n')?;
                write_yaml_indent(out, current_indent)?;
            }
            first = false;
            if is_yaml_cursor_container(&cursor) {
                out.write_char('-')?;
                out.write_char('\n')?;
                write_yaml_indent(out, current_indent + indent_spaces)?;
                cursor.stream_yaml_value(out, current_indent + indent_spaces, indent_spaces)?;
            } else {
                out.write_str("- ")?;
                cursor.stream_yaml_value(out, current_indent + indent_spaces, indent_spaces)?;
            }
        }
        Ok(())
    }
}

/// Stream a YAML string value with smart quoting.
/// Write a non-`String` mapping key as a YAML scalar (issue #222).
///
/// Resolves an alias-to-scalar key to its content with smart quoting;
/// any other complex key (mapping, sequence, null, error, unresolved or
/// non-scalar alias) becomes `""`. The entry is kept, never dropped.
#[cold]
#[inline(never)]
fn stream_yaml_nonstring_key<W: AsRef<[u64]>, Out: core::fmt::Write>(
    out: &mut Out,
    key: &YamlValue<'_, W>,
) -> core::fmt::Result {
    let s = key.key_string();
    if needs_yaml_quoting(&s) {
        stream_yaml_double_quoted(out, &s)
    } else {
        out.write_str(&s)
    }
}

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
        YamlString::Unquoted { .. } => {
            // Source plain scalar: re-emit verbatim so both the scalar type and
            // its representation survive (`1`, `true`, `1.0`, `.5`, `yes`),
            // matching yq. Quote only when the decoded value cannot round-trip
            // as a plain scalar (empty, control chars / newlines from folded
            // multiline plains with blank lines, or a leading sequence-entry
            // indicator — `- x` emitted bare under a `- ` would read back as a
            // nested sequence, #332).
            if str_val.is_empty()
                || str_val.bytes().any(|b| b < 0x20)
                || starts_seq_entry(str_val.as_bytes(), 0)
            {
                stream_yaml_double_quoted(out, &str_val)
            } else {
                out.write_str(&str_val)
            }
        }
        _ => {
            // Block scalar - use smart quoting based on content
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
    use crate::yaml::{YamlError, YamlIndex};

    /// Helper to get the first document from the root document array.
    /// All YAML documents are wrapped in a virtual root sequence.
    fn first_doc<W: AsRef<[u64]> + core::fmt::Debug>(root: YamlCursor<'_, W>) -> YamlValue<'_, W> {
        match root.value() {
            YamlValue::Sequence(elements) => elements
                .into_iter()
                .next()
                .expect("expected at least one document"),
            other => panic!("expected root to be document array (sequence), got {other:?}"),
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
            other => panic!("expected mapping, got {other:?}"),
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
    fn test_duplicate_mapping_key_find_is_last_wins() {
        // YAML 1.2 / yq: `.a` on a mapping with a duplicate key resolves to
        // the *last* occurrence, not the first (issue #174).
        let yaml = b"a: 1\na: 2\n";
        let index = YamlIndex::build(yaml).unwrap();
        let root = index.root(yaml);

        if let YamlValue::Mapping(fields) = first_doc(root) {
            if let Some(YamlValue::String(s)) = fields.find("a") {
                assert_eq!(&*s.as_str().unwrap(), "2");
            } else {
                panic!("expected string scalar \"2\"");
            }
        } else {
            panic!("expected mapping");
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
    fn test_decode_double_quoted_hex_escape_non_ascii() {
        // \xNN with value > 0x7F takes the char::from_u32(val) path.
        let s = YamlString::DoubleQuoted {
            text: b"\"caf\\xe9\"",
            start: 0,
        };
        assert_eq!(&*s.as_str().unwrap(), "café");
    }

    #[test]
    fn test_decode_double_quoted_unicode_escape() {
        // \uNNNN 4-digit escape -> char::from_u32(codepoint).
        let s = YamlString::DoubleQuoted {
            text: b"\"caf\\u00e9\"",
            start: 0,
        };
        assert_eq!(&*s.as_str().unwrap(), "café");
        let s2 = YamlString::DoubleQuoted {
            text: b"\"\\u1234\"",
            start: 0,
        };
        assert_eq!(&*s2.as_str().unwrap(), "\u{1234}");
    }

    #[test]
    fn test_to_json_double_quoted_non_ascii_and_leading_plus() {
        // Exercises transcode_double_quoted_to_json (non-ASCII \x escape) and
        // write_yaml_scalar_as_json (leading-'+' integer and float).
        let yaml = b"s: \"caf\\xe9\"\ni: +123\nf: +1.5\n";
        let index = YamlIndex::build(yaml).unwrap();
        let json = index.root(yaml).to_json_document();
        assert!(json.contains("\"i\":123"), "got {json}");
        assert!(json.contains("\"f\":1.5"), "got {json}");
    }

    #[test]
    fn test_stream_json_double_quoted_non_ascii_and_leading_plus() {
        // Streaming counterpart of the test above: exercises
        // stream_transcode_double_quoted_to_json and stream_yaml_scalar_as_json.
        let yaml = b"s: \"caf\\xe9\"\ni: +123\nf: +1.5\n";
        let index = YamlIndex::build(yaml).unwrap();
        let mut out = String::new();
        index.root(yaml).stream_json_document(&mut out, 0).unwrap();
        assert!(out.contains("\"i\":123"), "got {out}");
        assert!(out.contains("\"f\":1.5"), "got {out}");
    }

    /// Streaming counterpart of `test_transcode_double_quoted_next_line_escape`
    /// / `test_transcode_double_quoted_nbsp_escape` — `stream_transcode_-
    /// double_quoted_to_json`'s `\N`/`\_` branches had no dedicated test at
    /// all before #532, which is how they were missed when the
    /// `stream_json_escape` policy fix landed for the `\x`/`\u` escape
    /// paths: both stream U+0085/U+00A0 as raw UTF-8, matching yq.
    #[test]
    fn test_stream_transcode_double_quoted_next_line_and_nbsp_escape() {
        let yaml = b"s: \"next\\Nline\"\nt: \"non\\_break\"\n";
        let index = YamlIndex::build(yaml).unwrap();
        let mut out = String::new();
        index.root(yaml).stream_json_document(&mut out, 0).unwrap();
        assert!(out.contains("\"s\":\"next\u{0085}line\""), "got {out}");
        assert!(out.contains("\"t\":\"non\u{00a0}break\""), "got {out}");
    }

    /// `\L`/`\P` (U+2028/U+2029) are the one pair still hardcoded rather than
    /// routed through `stream_json_escape` — yq always JSON-escapes them,
    /// even as raw literal bytes, unlike `\N`/`\_`. Pin that this streaming
    /// path still escapes them after the #532 fix.
    #[test]
    fn test_stream_transcode_double_quoted_line_and_paragraph_separator_escape() {
        let yaml = b"s: \"line\\Lsep\"\nt: \"para\\Psep\"\n";
        let index = YamlIndex::build(yaml).unwrap();
        let mut out = String::new();
        index.root(yaml).stream_json_document(&mut out, 0).unwrap();
        assert!(out.contains("\"s\":\"line\\u2028sep\""), "got {out}");
        assert!(out.contains("\"t\":\"para\\u2029sep\""), "got {out}");
    }

    /// Issue #222: mapping keys are always strings and never dropped. An
    /// alias key resolves to its anchored scalar (26DV, E76Z); a complex
    /// (sequence/mapping/null) key becomes "" but keeps its entry. Both
    /// emitter twins must produce identical JSON.
    #[test]
    fn test_mapping_keys_resolve_and_are_never_dropped() {
        let cases: &[(&[u8], &str)] = &[
            // Alias key resolves to the anchored scalar
            (
                b"top1:\n  key1: &a scalar1\ntop3:\n  *a : scalar3\n",
                "{\"top1\":{\"key1\":\"scalar1\"},\"top3\":{\"scalar1\":\"scalar3\"}}",
            ),
            // E76Z: alias keys both directions
            (b"&a a: &b b\n*b : *a\n", "{\"a\":\"b\",\"b\":\"a\"}"),
            // Complex (sequence) key -> "" but the entry survives
            (
                b"{a: [b, c], [d, e]: f}\n",
                "{\"a\":[\"b\",\"c\"],\"\":\"f\"}",
            ),
            // Explicit null key -> "" (kept)
            (b"? []\n: x\n", "{\"\":\"x\"}"),
        ];

        for (yaml, expected) in cases {
            let index = YamlIndex::build(yaml).unwrap();
            let json = index.root(yaml).to_json_document();
            let mut streamed = String::new();
            index
                .root(yaml)
                .stream_json_document(&mut streamed, 0)
                .unwrap();
            let input = core::str::from_utf8(yaml).unwrap();
            assert_eq!(json, *expected, "to_json mismatch for {input:?}");
            assert_eq!(streamed, *expected, "stream_json mismatch for {input:?}");
        }
    }

    /// `stream_json_document`/`to_json_document` unwrap a single-document
    /// root (`bp_pos == 0` with exactly one element) but must pass multi-
    /// document roots through as a JSON array, matching yq. Covers the
    /// non-single-document fallback branch in `stream_json_document`.
    #[test]
    fn test_json_document_multi_doc_wraps_in_array() {
        let yaml = b"a: 1\n---\nb: 2\n";
        let index = YamlIndex::build(yaml).unwrap();
        let root = index.root(yaml);

        let json = root.to_json_document();
        let mut streamed = String::new();
        root.stream_json_document(&mut streamed, 0).unwrap();

        assert_eq!(json, r#"[{"a":1},{"b":2}]"#);
        assert_eq!(streamed, json);
    }

    /// Issue #222: block scalars are always strings — never type-inferred
    /// (`|- 123` is "123", not 123) and never null when empty (K858,
    /// 2G84/02). Both emitter twins must agree.
    #[test]
    fn test_block_scalars_are_always_strings() {
        let cases: &[(&[u8], &str)] = &[
            (b"a: |-\n  123\n", "{\"a\":\"123\"}"),
            (b"a: |-\n  true\n", "{\"a\":\"true\"}"),
            (b"a: >-\n  null\n", "{\"a\":\"null\"}"),
            // K858: empty block scalars are "" (and keep chomping preserves \n)
            (
                b"strip: >-\n\nclip: >\n\nkeep: |+\n\n",
                "{\"strip\":\"\",\"clip\":\"\",\"keep\":\"\\n\"}",
            ),
        ];

        for (yaml, expected) in cases {
            let index = YamlIndex::build(yaml).unwrap();
            let json = index.root(yaml).to_json_document();
            let mut streamed = String::new();
            index
                .root(yaml)
                .stream_json_document(&mut streamed, 0)
                .unwrap();
            let input = core::str::from_utf8(yaml).unwrap();
            assert_eq!(json, *expected, "to_json mismatch for {input:?}");
            assert_eq!(streamed, *expected, "stream_json mismatch for {input:?}");
        }
    }

    /// Issue #344: keep chomping preserves the block's own empty lines, not the
    /// break that ended the indicator line. That break belongs to the header's
    /// `s-b-comment` (YAML 1.2 §8.1.1.2), so a block with no content lines *and*
    /// no empty lines is `""` — the two used to be conflated and every such block
    /// gained a `"\n"` it never had.
    ///
    /// The boundary is one empty line, so both sides of it are pinned here. Note
    /// `l-empty` ends in `b-as-line-feed`: a blank final line with no break is not
    /// an empty line, which is why `a: |+\n   ` is `""` while `a: |+\n   \n` is
    /// `"\n"`. Expected values are mikefarah/yq v4.53.3.
    #[test]
    fn test_content_less_keep_block_scalars_have_no_break() {
        let cases: &[(&[u8], &str)] = &[
            // Nothing at all after the header, with and without a final break
            (b"a: |+", "{\"a\":\"\"}"),
            (b"a: |+\n", "{\"a\":\"\"}"),
            (b"a: >+", "{\"a\":\"\"}"),
            (b"a: >+\n", "{\"a\":\"\"}"),
            // Explicit indentation indicator takes a different path into the
            // decoders and used to fabricate the same break (2G84/03)
            (b"a: |2+\n", "{\"a\":\"\"}"),
            (b"a: >2+\n", "{\"a\":\"\"}"),
            // The block need not be at EOF: a sibling key or a comment line ends
            // it just as well
            (b"a: |+\nb: 2\n", "{\"a\":\"\",\"b\":2}"),
            (b"a: >+\nb: 2\n", "{\"a\":\"\",\"b\":2}"),
            (b"a: |+\n# comment\n", "{\"a\":\"\"}"),
            // As a sequence item rather than a mapping value
            (b"- |+\n", "[\"\"]"),
            (b"- >+\n", "[\"\"]"),
            // Blank final line with no break: not an `l-empty`, so still ""
            (b"a: |+\n   ", "{\"a\":\"\"}"),
            // Boundary: one genuine empty line is one `\n`, and its leading
            // spaces are indentation, not content (JEF9/01)
            (b"a: |+\n\n", "{\"a\":\"\\n\"}"),
            (b"a: >+\n\n", "{\"a\":\"\\n\"}"),
            (b"a: |+\n   \n", "{\"a\":\"\\n\"}"),
            (b"a: |+\n\n\n", "{\"a\":\"\\n\\n\"}"),
            (b"a: |+\n  \n  \n", "{\"a\":\"\\n\\n\"}"),
            // Strip and clip were always right on these shapes — pin them so a
            // future fix cannot drift them the other way
            (b"a: |-\n", "{\"a\":\"\"}"),
            (b"a: |\n", "{\"a\":\"\"}"),
            (b"a: >-\n", "{\"a\":\"\"}"),
            (b"a: >\n", "{\"a\":\"\"}"),
        ];

        for (yaml, expected) in cases {
            let index = YamlIndex::build(yaml).unwrap();
            let json = index.root(yaml).to_json_document();
            let mut streamed = String::new();
            index
                .root(yaml)
                .stream_json_document(&mut streamed, 0)
                .unwrap();
            let input = core::str::from_utf8(yaml).unwrap();
            assert_eq!(json, *expected, "to_json mismatch for {input:?}");
            assert_eq!(streamed, *expected, "stream_json mismatch for {input:?}");
        }
    }

    /// Issue #222: quoted-string line folding must drop trailing *literal*
    /// whitespace before a literal line break while preserving *escaped*
    /// whitespace as content, and all three decoders — `as_str` (DOM),
    /// `to_json_document` (String twin), `stream_json_document` (stream twin)
    /// — must agree. Expected values are pinned by the YAML Test Suite
    /// (DE56, NP9H, 6WPF, 7A4E, PRH3, NAT4) and mikefarah/yq v4.53.3.
    #[test]
    fn test_quoted_fold_whitespace_three_decoders_agree() {
        let cases: &[(&[u8], &str)] = &[
            // DE56: escaped tabs (\t and \<TAB>) are content and survive the
            // fold; literal tabs/spaces before the break fold away
            (b"\"1 trailing\\t\n    tab\"", "1 trailing\t tab"),
            (b"\"2 trailing\\t  \n    tab\"", "2 trailing\t tab"),
            (b"\"3 trailing\\\t\n    tab\"", "3 trailing\t tab"),
            (b"\"4 trailing\\\t  \n    tab\"", "4 trailing\t tab"),
            (b"\"5 trailing\t\n    tab\"", "5 trailing tab"),
            (b"\"6 trailing\t  \n    tab\"", "6 trailing tab"),
            // NP9H: literal tab before an escaped line break is content
            (
                b"\"folded \nto a space,\t\n \nto a line feed, or \t\\\n \\ \tnon-content\"",
                "folded to a space,\nto a line feed, or \t \tnon-content",
            ),
            // 6WPF: flow folding with empty lines
            (b"\"\n  foo \n \n    bar\n\n  baz\n\"", " foo\nbar\nbaz "),
            // 7A4E / PRH3: trailing space folds, leading tab on the
            // continuation line is indentation
            (
                b"\" 1st non-empty\n\n 2nd non-empty \n\t3rd non-empty \"",
                " 1st non-empty\n2nd non-empty 3rd non-empty ",
            ),
            (
                b"' 1st non-empty\n\n 2nd non-empty \n\t3rd non-empty '",
                " 1st non-empty\n2nd non-empty 3rd non-empty ",
            ),
            // NAT4: whitespace-only quoted strings
            (b"'  \n  '", " "),
            (b"\"  \n  \"", " "),
            (b"'\n\n  '", "\n"),
            // Escaped space before a fold is content (yq v4.53.3)
            (b"\"x\\ \ny\"", "x  y"),
            // Mid-line literal whitespace is content (yq v4.53.3)
            (b"\"a\tb\nc\"", "a\tb c"),
            (b"'a \t\nb'", "a b"),
        ];

        for (yaml, expected) in cases {
            let index = YamlIndex::build(yaml).unwrap();

            // DOM decoder (as_str)
            match first_doc(index.root(yaml)) {
                YamlValue::String(s) => {
                    assert_eq!(
                        &*s.as_str().unwrap(),
                        *expected,
                        "as_str mismatch for {:?}",
                        core::str::from_utf8(yaml).unwrap()
                    );
                }
                other => panic!("expected string document, got {other:?}"),
            }

            // String twin vs stream twin vs independently escaped expected
            let json = index.root(yaml).to_json_document();
            let mut streamed = String::new();
            index
                .root(yaml)
                .stream_json_document(&mut streamed, 0)
                .unwrap();
            let mut expected_json = String::new();
            write_json_string(&mut expected_json, expected);
            assert_eq!(
                json,
                expected_json,
                "to_json mismatch for {:?}",
                core::str::from_utf8(yaml).unwrap()
            );
            assert_eq!(
                streamed,
                expected_json,
                "stream_json mismatch for {:?}",
                core::str::from_utf8(yaml).unwrap()
            );
        }
    }

    /// Values ≥ 16 bytes with multibyte UTF-8, long enough to enter the SIMD
    /// escape scanners (`find_json_escape`), including the 32-byte AVX2 loop.
    const MULTIBYTE_JSON_VALUES: &[&str] = &[
        "love ♥ and peace ☮", // the original #230 repro value
        "café au lait, s'il vous plaît",
        "日本語のテキストはここにあります",
        "emoji 😁 in a string long enough for a full chunk",
    ];

    fn multibyte_yaml() -> Vec<u8> {
        format!(
            "wanted: {}\nlatin: {}\ncjk: {}\nemoji: {}\n",
            MULTIBYTE_JSON_VALUES[0],
            MULTIBYTE_JSON_VALUES[1],
            MULTIBYTE_JSON_VALUES[2],
            MULTIBYTE_JSON_VALUES[3]
        )
        .into_bytes()
    }

    #[test]
    fn test_to_json_multibyte_survives_simd_escape_scan() {
        // Regression test for the x86 signed-compare bug (#150/#230): the
        // AVX2/SSE2 `find_json_escape` kernels misread bytes >= 0x80 as
        // control characters, cutting multibyte UTF-8 mid-character (a panic
        // in callers slicing on the index, or corrupt output). On x86 this
        // drives the fixed kernels through the library `to_json()` /
        // `to_json_document()` path end-to-end.
        let yaml = multibyte_yaml();
        let index = YamlIndex::build(&yaml).unwrap();
        let json = index.root(&yaml).to_json_document();
        for expect in MULTIBYTE_JSON_VALUES {
            assert!(
                json.contains(expect),
                "multibyte content lost: {expect:?} not in {json}"
            );
        }

        // `to_json` (with the document wrapper) takes the same escape path.
        let wrapped = index.root(&yaml).to_json();
        assert!(wrapped.contains(MULTIBYTE_JSON_VALUES[0]), "got {wrapped}");
    }

    #[test]
    fn test_stream_json_multibyte_survives_simd_escape_scan() {
        // Streaming counterpart of the test above (#150/#230).
        let yaml = multibyte_yaml();
        let index = YamlIndex::build(&yaml).unwrap();
        let mut out = String::new();
        index.root(&yaml).stream_json_document(&mut out, 0).unwrap();
        for expect in MULTIBYTE_JSON_VALUES {
            assert!(
                out.contains(expect),
                "multibyte content lost: {expect:?} not in {out}"
            );
        }
    }

    #[test]
    fn test_stream_yaml_plain_scalars_verbatim() {
        // #175: source plain scalars must round-trip verbatim through the YAML
        // streaming path — preserving both type (`1`, `true`) and representation
        // (`1.0`, `.5`, `yes`), matching yq. Previously they were re-quoted as
        // strings (`a: "1"`).
        let yaml = b"a: 1\nb: true\nc: hello\nd: 1.0\ne: .5\nf: yes\ng: null\n";
        let index = YamlIndex::build(yaml).unwrap();
        let mut out = String::new();
        index.root(yaml).stream_yaml_document(&mut out, 2).unwrap();
        assert_eq!(
            out,
            "a: 1\nb: true\nc: hello\nd: 1.0\ne: .5\nf: yes\ng: null"
        );
    }

    #[test]
    fn test_stream_yaml_preserves_top_level_flow_container_style() {
        // #707: a node whose source used flow style must render as flow even
        // under the CLI's normal block `indent_spaces` (2), not just when the
        // whole document is forced flow via indent_spaces=0.
        let yaml = b"a: [1, 2, 3]\nb: {c: 1, d: 2}\n";
        let index = YamlIndex::build(yaml).unwrap();
        let mut out = String::new();
        index.root(yaml).stream_yaml_document(&mut out, 2).unwrap();
        assert_eq!(out, "a: [1, 2, 3]\nb: {c: 1, d: 2}");
    }

    #[test]
    fn test_stream_yaml_preserves_flow_sequence_nested_under_block_mapping() {
        // #707: a flow-styled value nested under a block mapping key stays
        // flow and inline, not exploded onto its own indented block lines.
        let yaml = b"top:\n  a: [1, 2, 3]\n";
        let index = YamlIndex::build(yaml).unwrap();
        let mut out = String::new();
        index.root(yaml).stream_yaml_document(&mut out, 2).unwrap();
        assert_eq!(out, "top:\n  a: [1, 2, 3]");
    }

    #[test]
    fn test_stream_yaml_preserves_flow_sequence_item_nested_under_block_sequence() {
        // #707: a flow-styled sequence item nested under a block sequence
        // renders inline after `- `, not as its own nested block sequence.
        let yaml = b"a:\n  - [1, 2]\n  - 3\n";
        let index = YamlIndex::build(yaml).unwrap();
        let mut out = String::new();
        index.root(yaml).stream_yaml_document(&mut out, 2).unwrap();
        assert_eq!(out, "a:\n  - [1, 2]\n  - 3");
    }

    #[test]
    fn test_stream_yaml_preserves_flow_mapping_nested_under_block_mapping() {
        // #707: same as the sequence case above, but for a flow-styled
        // mapping value nested under a block mapping key.
        let yaml = b"a:\n  b: {x: 1, y: 2}\n  c: 3\n";
        let index = YamlIndex::build(yaml).unwrap();
        let mut out = String::new();
        index.root(yaml).stream_yaml_document(&mut out, 2).unwrap();
        assert_eq!(out, "a:\n  b: {x: 1, y: 2}\n  c: 3");
    }

    #[test]
    fn test_stream_yaml_flow_and_block_siblings_are_independent() {
        // #707: per-node style, not a single document-wide switch — one
        // sibling stays flow while the other stays block.
        let yaml = b"a: [1, 2, 3]\nb:\n  x: 1\n  y: 2\n";
        let index = YamlIndex::build(yaml).unwrap();
        let mut out = String::new();
        index.root(yaml).stream_yaml_document(&mut out, 2).unwrap();
        assert_eq!(out, "a: [1, 2, 3]\nb:\n  x: 1\n  y: 2");
    }

    #[test]
    fn test_stream_yaml_flow_in_flow_still_flow() {
        // Regression guard: flow nested inside flow (already-correct
        // pre-#707 behavior) keeps working once nodes can independently
        // opt into flow.
        let yaml = b"a: {b: [1, 2], c: {d: 3}}\n";
        let index = YamlIndex::build(yaml).unwrap();
        let mut out = String::new();
        index.root(yaml).stream_yaml_document(&mut out, 2).unwrap();
        assert_eq!(out, "a: {b: [1, 2], c: {d: 3}}");
    }

    #[test]
    fn test_stream_yaml_empty_flow_and_block_containers_unaffected() {
        // Empty containers are a special-cased early return regardless of
        // style, untouched by the #707 fix.
        let yaml = b"a: []\nb: {}\n";
        let index = YamlIndex::build(yaml).unwrap();
        let mut out = String::new();
        index.root(yaml).stream_yaml_document(&mut out, 2).unwrap();
        assert_eq!(out, "a: []\nb: {}");
    }

    #[test]
    fn test_stream_yaml_sequence_flow_style() {
        // #478: `stream_yaml_sequence` mirrors `stream_yaml_value`'s Sequence
        // flow/block duality, but its only caller (`--slurp`) always maps
        // `-I0` to indent_spaces=2 (yq_runner.rs's documented compact-YAML
        // quirk), so the flow-style (indent_spaces == 0) branch is
        // unreachable from the CLI. Exercise it directly as a library
        // caller would, combining cursors from two independent sources
        // (the reason this function exists over reusing `stream_yaml_value`'s
        // own Sequence arm, which requires one shared `YamlIndex`).
        let bytes_a = b"a: 1\n".to_vec();
        let index_a = YamlIndex::build(&bytes_a).unwrap();
        let bytes_b = b"b: 2\n".to_vec();
        let index_b = YamlIndex::build(&bytes_b).unwrap();

        let cursor_a = index_a.root(&bytes_a).first_child().unwrap();
        let cursor_b = index_b.root(&bytes_b).first_child().unwrap();

        let mut out = String::new();
        stream_yaml_sequence([cursor_a, cursor_b], &mut out, 0, 0).unwrap();
        assert_eq!(out, "[{a: 1}, {b: 2}]");
    }

    #[test]
    fn test_stream_yaml_quoted_scalars_stay_quoted() {
        // #175: quoted source scalars are genuine strings and must keep their
        // quoting style so they don't turn into numbers/booleans on re-parse.
        let yaml = b"a: \"1\"\nb: 'true'\nc: \"hello\"\n";
        let index = YamlIndex::build(yaml).unwrap();
        let mut out = String::new();
        index.root(yaml).stream_yaml_document(&mut out, 2).unwrap();
        assert_eq!(out, "a: \"1\"\nb: 'true'\nc: \"hello\"");
    }

    #[test]
    fn test_stream_yaml_block_scalar_smart_quoting() {
        // Block scalars decode to plain strings and take the smart-quoting
        // path (needs_yaml_quoting / looks_like_yaml_number): multiline
        // content and anything that would re-parse as a keyword or number
        // must be double-quoted to stay a string; everything else stays bare.
        let yaml = b"a: |-\n  hello\n  world\nb: |-\n  plain\nc: |-\n  true\nd: |-\n  123\ne: |-\n  1.5e+3\nf: |-\n  12x\ng: |-\n  1.5.5\nh: |-\n  +\ni: |-\n  +x\nj: |-\n  +5x\nk: >-\n  folded here\nl: |-\n  -foo\n";
        let index = YamlIndex::build(yaml).unwrap();
        let mut out = String::new();
        index.root(yaml).stream_yaml_document(&mut out, 2).unwrap();
        assert_eq!(
            out,
            "a: \"hello\\nworld\"\n\
             b: plain\n\
             c: \"true\"\n\
             d: \"123\"\n\
             e: \"1.5e+3\"\n\
             f: 12x\n\
             g: 1.5.5\n\
             h: +\n\
             i: +x\n\
             j: +5x\n\
             k: folded here\n\
             l: \"-foo\""
        );
    }

    #[test]
    fn test_stream_yaml_block_scalar_trailing_space_quoted() {
        // A literal block scalar preserves trailing spaces; the decoded value
        // cannot round-trip as a plain scalar and must be double-quoted.
        let yaml = b"t: |-\n  x \n";
        let index = YamlIndex::build(yaml).unwrap();
        let mut out = String::new();
        index.root(yaml).stream_yaml_document(&mut out, 2).unwrap();
        assert_eq!(out, "t: \"x \"");
    }

    #[test]
    fn test_stream_yaml_multiline_plain_folds_to_one_line() {
        // A multiline plain scalar decodes with folded spaces; verbatim
        // re-emission on one line preserves the decoded value. Blank lines
        // decode to `\n`, which cannot round-trip as a plain scalar and must
        // be double-quoted.
        let yaml = b"a: one\n  two\nb: one\n\n  two\n";
        let index = YamlIndex::build(yaml).unwrap();
        let mut out = String::new();
        index.root(yaml).stream_yaml_document(&mut out, 2).unwrap();
        assert_eq!(out, "a: one two\nb: \"one\\ntwo\"");
    }

    #[test]
    fn test_long_plain_scalar_does_not_swallow_next_line() {
        // Regression for the SSE2 chunk-width bug found by the multibyte tests
        // above (#193): `skip_unquoted_simd` assumed `classify_yaml_chars`
        // scanned 32 bytes whenever 32 remained, but on non-AVX2 x86 the SSE2
        // classifier only scans 16 — so the parser skipped past the newline of
        // any plain scalar whose line ended in bytes 16..31 of the scan window
        // and folded the next mapping entry into the value. Pure ASCII, so the
        // failure is isolated to chunk-width accounting, not UTF-8 handling.
        let yaml = b"a: love and peace forever more x\nb: x\n";
        let index = YamlIndex::build(yaml).unwrap();
        let json = index.root(yaml).to_json_document();
        assert_eq!(json, r#"{"a":"love and peace forever more x","b":"x"}"#);
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
                // Block literal preserves every line break verbatim
                assert_eq!(&*s.as_str().unwrap(), "Line 1\nLine 2\n");
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
                // Block folded folds the break between two ordinary lines to a space
                assert_eq!(&*s.as_str().unwrap(), "First part second part\n");
            } else {
                panic!("expected string for text");
            }
        } else {
            panic!("expected mapping");
        }
    }

    /// Folded block scalar line folding, YAML 1.2 §8.1.3.
    ///
    /// Every expectation below is the output of mikefarah/yq v4.53.3 on the
    /// same input, not a transcription of the spec — the two disagreed on
    /// nothing here, but yq is what `succinctly yq` claims to be a drop-in
    /// replacement for.
    ///
    /// Regression coverage for two defects that shipped together:
    ///
    /// * #329 — a blank line produced N+1 newlines where the spec and yq give
    ///   N, because the folded break *and* the blank line both emitted one.
    /// * `>+` dropped `b-chomped-last` entirely, so `>+` over `a\n` yielded
    ///   `"a"` instead of `"a\n"`.
    ///
    /// Every input here ends in a line break. The shapes that do not are in
    /// [`test_block_folded_unterminated_final_line`] — a distinction the first
    /// round of this fix got wrong precisely because nothing covered it.
    #[test]
    fn test_block_folded_line_folding() {
        // (input, expected) — `fold` is the only key in each document
        assert_folded(&[
            // No blank lines: the break folds to a space
            (b"fold: >\n  a\n  b\n", "a b\n"),
            (b"fold: >\n  a\n", "a\n"),
            // N blank lines between ordinary lines give N newlines, not N+1 (#329)
            (b"fold: >\n  a\n\n  b\n", "a\nb\n"),
            (b"fold: >\n  a\n\n\n  b\n", "a\n\nb\n"),
            (b"fold: >\n  a\n\n\n\n  b\n", "a\n\n\nb\n"),
            // A whitespace-only line is a blank line
            (b"fold: >\n  a\n  \n  b\n", "a\nb\n"),
            // Blank lines before the first content line: one newline each
            (b"fold: >\n\n  a\n", "\na\n"),
            (b"fold: >\n\n\n  a\n", "\n\na\n"),
            // A more-indented line suppresses folding on both of its breaks,
            // and that break survives *in addition to* any blank lines
            (b"fold: >\n  a\n   x\n  b\n", "a\n x\nb\n"),
            (b"fold: >\n  a\n\n   x\n", "a\n\n x\n"),
            (b"fold: >\n  a\n   x\n\n  b\n", "a\n x\n\nb\n"),
            (b"fold: >\n  a\n\n   x\n  b\n", "a\n\n x\nb\n"),
            (b"fold: >\n  a\n\n   x\n\n  b\n", "a\n\n x\n\nb\n"),
            // Chomping: strip drops the tail, clip keeps one newline, ...
            (b"fold: >-\n  a\n", "a"),
            (b"fold: >-\n  a\n\n  b\n", "a\nb"),
            (b"fold: >\n  a\n\n\n", "a\n"),
            // ... and keep preserves the final break plus every trailing blank
            (b"fold: >+\n  a\n", "a\n"),
            (b"fold: >+\n  a\n\n  b\n", "a\nb\n"),
            (b"fold: >+\n  a\n\n\n", "a\n\n\n"),
            (b"fold: >+\n  a\n   x\n\n\n", "a\n x\n\n\n"),
            // Explicit indentation indicator takes the same path
            (b"fold: >2\n  a\n\n  b\n", "a\nb\n"),
            // CRLF input folds identically
            (b"fold: >\r\n  a\r\n\r\n  b\r\n", "a\nb\n"),
            (b"fold: >\r\n  a\r\n\r\n\r\n  b\r\n", "a\n\nb\n"),
        ]);
    }

    /// A folded block whose last line runs to EOF with no line break after it.
    ///
    /// There is no `b-chomped-last` to keep in that case, so *no* chomping
    /// indicator produces a trailing newline the text never had — `>`, `>-` and
    /// `>+` over an unterminated `a` are all `"a"`. Every expectation is
    /// mikefarah/yq v4.53.3's output on the same bytes.
    ///
    /// This shape is easy to miss because it cannot be written into a fixture
    /// file that a well-behaved editor will leave alone, and every input in
    /// [`test_block_folded_line_folding`] ends in a break. The first cut of the
    /// #329 fix emitted `b-chomped-last` unconditionally and so regressed all
    /// of `>+` here while fixing the terminated cases.
    #[test]
    fn test_block_folded_unterminated_final_line() {
        assert_folded(&[
            // Clip: the folding rules are unchanged, only the tail differs
            (b"fold: >\n  a", "a"),
            (b"fold: >\n  a\n  b", "a b"),
            (b"fold: >\n  a\n\n  b", "a\nb"),
            (b"fold: >\n  a\n\n\n  b", "a\n\nb"),
            (b"fold: >\n  a\n   x", "a\n x"),
            (b"fold: >\n  a\n\n   x", "a\n\n x"),
            // Strip has nothing extra to strip
            (b"fold: >-\n  a", "a"),
            (b"fold: >-\n  a\n\n  b", "a\nb"),
            // Keep has nothing to keep — the case the guard on the tail exists for
            (b"fold: >+\n  a", "a"),
            (b"fold: >+\n  a\n\n  b", "a\nb"),
            (b"fold: >+\n  a\n   x", "a\n x"),
            // Explicit indentation indicator takes the same path
            (b"fold: >2+\n  a", "a"),
            (b"fold: >2\n  a\n\n  b", "a\nb"),
            // Trailing whitespace running to EOF is not a blank *line*: it has
            // no break, so it contributes nothing
            (b"fold: >+\n  a\n  ", "a\n"),
            (b"fold: >+\n  a\n\n  ", "a\n\n"),
            // A lone CR does terminate the line, so `b-chomped-last` survives
            (b"fold: >+\n  a\r", "a\n"),
            (b"fold: >\r\n  a\r\n\r\n  b", "a\nb"),
        ]);
    }

    /// Issue #344, folded side: with no content lines at all, keep chomping has
    /// only the block's own empty lines to keep. The break that ended the
    /// indicator line is the header's, not the scalar's, so a block with no
    /// empty lines is `""` and not `"\n"`.
    ///
    /// [`test_block_folded_unterminated_final_line`] already pins the same
    /// "a run of spaces to EOF is not a line" rule for blocks that *do* have
    /// content; these are the shapes where it is all the block consists of.
    /// Expected values are mikefarah/yq v4.53.3.
    #[test]
    fn test_block_folded_content_less_keep() {
        assert_folded(&[
            (b"fold: >+", ""),
            (b"fold: >+\n", ""),
            // Explicit indentation indicator bypasses the empty-block branch
            (b"fold: >2+\n", ""),
            // A sibling key or a comment line ends the block just as EOF does
            (b"fold: >+\nother: 1\n", ""),
            (b"fold: >+\n# comment\n", ""),
            // No break, so no `l-empty` — still ""
            (b"fold: >+\n   ", ""),
            // Boundary: a real empty line is worth one `\n`, its indentation
            // stripped
            (b"fold: >+\n\n", "\n"),
            (b"fold: >+\n   \n", "\n"),
            (b"fold: >+\n\n\n", "\n\n"),
        ]);
    }

    /// Decode the sole `fold` key of each document and compare against the
    /// pinned-yq expectation, so a new folding case is one line rather than a
    /// copied loop.
    fn assert_folded(cases: &[(&[u8], &str)]) {
        for (yaml, expected) in cases {
            let index = YamlIndex::build(yaml).unwrap();
            let root = index.root(yaml);
            let YamlValue::Mapping(fields) = first_doc(root) else {
                panic!("expected mapping for {:?}", String::from_utf8_lossy(yaml));
            };
            let Some(YamlValue::String(s)) = fields.find("fold") else {
                panic!("expected string for {:?}", String::from_utf8_lossy(yaml));
            };
            assert_eq!(
                &*s.as_str().unwrap(),
                *expected,
                "folding {:?}",
                String::from_utf8_lossy(yaml)
            );
        }
    }

    /// The folded decoder is reached through three different call paths; a fix
    /// in one must not leave the others behind. Asserts the JSON each produces
    /// for the #329 repro, terminated and unterminated.
    #[test]
    fn test_block_folded_blank_line_agrees_across_paths() {
        // (input, JSON both paths must produce) — pinned-yq output
        let cases: &[(&[u8], &str)] = &[
            (b"fold: >\n  a\n\n  b\n", r#"{"fold":"a\nb\n"}"#),
            // No final break: no `b-chomped-last`, even under keep chomping
            (b"fold: >+\n  a\n\n  b", r#"{"fold":"a\nb"}"#),
        ];

        for (yaml, expected) in cases {
            let index = YamlIndex::build(yaml).unwrap();
            let root = index.root(yaml);

            // DOM path (`to_json`) and streaming path (`stream_json`, P9)
            assert_eq!(
                root.to_json_document(),
                *expected,
                "DOM path for {:?}",
                String::from_utf8_lossy(yaml)
            );

            let mut streamed = String::new();
            root.stream_json_document(&mut streamed, 0).unwrap();
            assert_eq!(
                streamed,
                *expected,
                "streaming path for {:?}",
                String::from_utf8_lossy(yaml)
            );
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
            assert_eq!(items.len(), 2, "expected 2 items, got items: {items:?}");

            // First item is block literal
            if let YamlValue::String(s) = &items[0] {
                let decoded = s.as_str().unwrap();
                assert!(
                    decoded.contains("item"),
                    "first item should contain 'item', got: {decoded:?}"
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
                            }
                            panic!("expected alias for greeting");
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

    /// Issue #328: an anchor on a block sequence item binds to the item's
    /// value, whatever its kind. Before the fix, `- &m` followed by an indented
    /// collection was read as a multi-line plain scalar (yielding `["k"]` with
    /// the rest of the mapping leaking out as a sibling), and `- &m {...}` had
    /// its anchor swallowed into the key text so the alias resolved to nothing.
    ///
    /// Every expectation here is the output of mikefarah/yq v4.53.3, the same
    /// oracle `tests/data/yq-golden/cases/anchored_seq_item_*` is captured from.
    /// Note `- &a k: v` anchors the *key*, not the mapping — that is yq's
    /// behaviour and matches how `parse_mapping_entry` treats `&a k: v`.
    #[test]
    fn test_anchored_sequence_item_with_collection_value() {
        let cases: &[(&[u8], &str)] = &[
            // Block mapping on the following lines - the headline #328 repro
            (
                b"list:\n  - &m\n    k: v\n  - *m\n",
                "{\"list\":[{\"k\":\"v\"},{\"k\":\"v\"}]}",
            ),
            // Block sequence on the following lines
            (
                b"items:\n  - &m\n    - a\n    - b\n  - *m\n",
                "{\"items\":[[\"a\",\"b\"],[\"a\",\"b\"]]}",
            ),
            // Flow mapping on the same line - the second #328 repro
            (
                b"items:\n  - &first {id: 1}\n  - *first\n",
                "{\"items\":[{\"id\":1},{\"id\":1}]}",
            ),
            // Flow sequence on the same line
            (
                b"items:\n  - &m [1, 2]\n  - *m\n",
                "{\"items\":[[1,2],[1,2]]}",
            ),
            // Compact mapping entry: the anchor binds to the key
            (
                b"items:\n  - &a k: v\n  - *a\n",
                "{\"items\":[{\"k\":\"v\"},\"k\"]}",
            ),
            // A trailing comment containing `: ` must not read as a mapping entry
            (
                b"list:\n  - &m # note: here\n    k: v\n  - *m\n",
                "{\"list\":[{\"k\":\"v\"},{\"k\":\"v\"}]}",
            ),
            // Blank line between the anchor and its value
            (
                b"list:\n  - &m\n\n    k: v\n  - *m\n",
                "{\"list\":[{\"k\":\"v\"},{\"k\":\"v\"}]}",
            ),
            // Value at exactly indent + 1, the boundary of "is this a child?"
            (b"- &m\n k: v\n- *m\n", "[{\"k\":\"v\"},{\"k\":\"v\"}]"),
            // Null value: the anchor still needs an explicit node to point at
            (b"items:\n  - &m\n  - *m\n", "{\"items\":[null,null]}"),
            // Null value at EOF
            (b"items:\n  - &m\n", "{\"items\":[null]}"),
            // A deeper comment line is not a value
            (b"items:\n  - &m\n    # c\n", "{\"items\":[null]}"),
            // A column-0 comment does not end the item
            (
                b"list:\n  - &m\n# c\n    k: v\n  - *m\n",
                "{\"list\":[{\"k\":\"v\"},{\"k\":\"v\"}]}",
            ),
            // Already worked before the fix - guard against regression
            (
                b"items:\n  - &s val\n  - *s\n",
                "{\"items\":[\"val\",\"val\"]}",
            ),
            (
                b"items:\n  - &m |\n    line\n  - *m\n",
                "{\"items\":[\"line\\n\",\"line\\n\"]}",
            ),
            // Sequence as an explicit-key value routes through the same parser
            (b"? k\n: - &m\n    a: 1\n", "{\"k\":[{\"a\":1}]}"),
        ];

        for (yaml, expected) in cases {
            let index = YamlIndex::build(yaml).unwrap();
            let json = index.root(yaml).to_json_document();
            let mut streamed = String::new();
            index
                .root(yaml)
                .stream_json_document(&mut streamed, 0)
                .unwrap();
            let input = core::str::from_utf8(yaml).unwrap();
            assert_eq!(json, *expected, "to_json mismatch for {input:?}");
            assert_eq!(streamed, *expected, "stream_json mismatch for {input:?}");
        }
    }

    /// Issue #328: an anchored sequence item exposes the kind of the collection
    /// it names, and the alias resolves to that same collection rather than to
    /// a stray scalar.
    #[test]
    fn test_anchored_sequence_item_alias_resolves_to_collection() {
        let yaml = b"list:\n  - &m\n    k: v\n  - *m\n";
        let index = YamlIndex::build(yaml).unwrap();

        let items: Vec<_> = match first_doc(index.root(yaml)) {
            YamlValue::Mapping(fields) => {
                let field = fields.into_iter().next().expect("one field");
                match field.value_cursor.value() {
                    YamlValue::Sequence(elements) => elements.collect(),
                    other => panic!("expected sequence, got {other:?}"),
                }
            }
            other => panic!("expected mapping, got {other:?}"),
        };
        assert_eq!(items.len(), 2, "expected 2 items");

        assert!(
            matches!(&items[0], YamlValue::Mapping(_)),
            "anchored item should be a mapping"
        );
        match &items[1] {
            YamlValue::Alias {
                anchor_name,
                target,
            } => {
                assert_eq!(*anchor_name, "m");
                let target = target.as_ref().expect("alias must resolve");
                assert!(
                    matches!(target.value(), YamlValue::Mapping(_)),
                    "alias must resolve to the anchored mapping"
                );
            }
            other => panic!("expected alias, got {other:?}"),
        }
    }

    /// Issue #339: an explicit key (`? `) at block-sequence-item position is a
    /// mapping, not a plain scalar. Before the fix the item's dispatch had no
    /// `?` arm, so `- ? e` fell through to `parse_value` and read as the scalar
    /// `"? e"`, while the following `  : v` line landed in the *sequence* as a
    /// phantom sibling: `["? e","v"]` instead of `[{"e":"v"}]`. No error — a
    /// well-formed, wrong document.
    ///
    /// The fix routes the item through the same `parse_explicit_key` the
    /// mapping-level dispatch already uses, so these cases are the sequence-item
    /// spelling of shapes that were already correct at top level and in
    /// mapping-value position. Both spellings are asserted below for exactly
    /// that reason: they must not drift apart again.
    ///
    /// Every expectation is the output of mikefarah/yq v4.53.3, the same oracle
    /// `tests/data/yq-golden/cases/explicit_key_seq_item_*` is captured from.
    #[test]
    fn test_explicit_key_as_sequence_item() {
        let cases: &[(&[u8], &str)] = &[
            // The headline #339 repro, and its null-value twin
            (b"- ? e\n  : v\n", "[{\"e\":\"v\"}]"),
            (b"- ? e\n", "[{\"e\":null}]"),
            // A following item is a sibling of the mapping, not of its value
            (b"- ? e\n  : v\n- x\n", "[{\"e\":\"v\"},\"x\"]"),
            // Further entries join the item's mapping rather than starting one
            (
                b"- ? e\n  : v\n  ? f\n  : w\n",
                "[{\"e\":\"v\",\"f\":\"w\"}]",
            ),
            (b"- ? e\n  : v\n  g: h\n", "[{\"e\":\"v\",\"g\":\"h\"}]"),
            // ...and an implicit entry first, explicit second
            (b"- g: h\n  ? e\n  : v\n", "[{\"g\":\"h\",\"e\":\"v\"}]"),
            // Each item gets its own mapping
            (b"- ? e\n- ? f\n", "[{\"e\":null},{\"f\":null}]"),
            // Key content on the following line
            (b"- ?\n    e\n  : v\n", "[{\"e\":\"v\"}]"),
            // Non-plain keys reach parse_explicit_key's own dispatch
            (b"- ? \"q k\"\n  : v\n", "[{\"q k\":\"v\"}]"),
            (b"- ? 'q k'\n  : v\n", "[{\"q k\":\"v\"}]"),
            (b"- ? |\n    lit\n  : v\n", "[{\"lit\\n\":\"v\"}]"),
            (b"- ? [1, 2]\n  : v\n", "[{\"\":\"v\"}]"),
            // The mapping's indent is the `?`'s own column, not `indent + 2`
            (b"-   ? e\n    : v\n", "[{\"e\":\"v\"}]"),
            // Nested one level down: the sequence is a mapping value
            (b"k:\n  - ? e\n    : v\n", "{\"k\":[{\"e\":\"v\"}]}"),
            // A sequence item inside an explicit key's value
            (b"? k\n: - ? e\n    : v\n", "{\"k\":[{\"e\":\"v\"}]}"),
            // Anchors compose with the new arm (#328 consumes `- &a` first)
            (b"- ? &a e\n  : v\n", "[{\"e\":\"v\"}]"),
            (b"- ? e\n  : &a v\n- *a\n", "[{\"e\":\"v\"},\"v\"]"),
            // `?` without a following space is a plain scalar, as in yq
            (b"- ?foo\n", "[\"?foo\"]"),
            // #324 composition: the arm's guard admits all three YAML 1.2 §5.4
            // line breaks, so the break form cannot change the structure
            (b"- ? e\r\n  : v\r\n", "[{\"e\":\"v\"}]"),
            (b"- ? e\r  : v\r", "[{\"e\":\"v\"}]"),
            // The same shapes in the positions that already worked - these are
            // the reference the sequence-item spelling must agree with
            (b"? e\n: v\n", "{\"e\":\"v\"}"),
            (b"m:\n  ? e\n  : v\n", "{\"m\":{\"e\":\"v\"}}"),
        ];

        for (yaml, expected) in cases {
            let index = YamlIndex::build(yaml).unwrap();
            let json = index.root(yaml).to_json_document();
            let mut streamed = String::new();
            index
                .root(yaml)
                .stream_json_document(&mut streamed, 0)
                .unwrap();
            let input = core::str::from_utf8(yaml).unwrap();
            assert_eq!(json, *expected, "to_json mismatch for {input:?}");
            assert_eq!(streamed, *expected, "stream_json mismatch for {input:?}");
        }
    }

    /// Issue #339: the item wrapper must expose a *mapping*, and its key must
    /// be the key alone — the `? ` indicator is not part of the key text.
    ///
    /// `to_json_document` alone cannot show this: a scalar item whose text
    /// happened to be `{"e":"v"}` would serialize identically. This walks the
    /// cursor to prove the BP tree really is `seq( item( map( key value ) ) )`.
    #[test]
    fn test_explicit_key_sequence_item_exposes_mapping() {
        let yaml = b"- ? e\n  : v\n";
        let index = YamlIndex::build(yaml).unwrap();

        let items: Vec<_> = match first_doc(index.root(yaml)) {
            YamlValue::Sequence(elements) => elements.collect(),
            other => panic!("expected sequence, got {other:?}"),
        };
        assert_eq!(items.len(), 1, "the `: v` line must not add an element");

        let fields = match &items[0] {
            YamlValue::Mapping(fields) => fields.clone(),
            other => panic!("expected mapping, got {other:?}"),
        };
        let field = fields.into_iter().next().expect("one field");
        assert_eq!(&*field.key().as_str().unwrap(), "e", "`? ` is not key text");
        match field.value_cursor.value() {
            YamlValue::String(s) => assert_eq!(&*s.as_str().unwrap(), "v"),
            other => panic!("expected string value, got {other:?}"),
        }
    }

    /// Issue #346: `? ` and a `: ` value indicator on the *same* line. Per YAML 1.2
    /// §8.2.2 the node after `? ` is `s-l+block-indented`, which admits
    /// `ns-l-compact-mapping` — so the whole `k: v` is a *mapping used as the key*,
    /// and the entry has no value. yq renders that key `""` and the value null.
    ///
    /// Before the fix `parse_explicit_key` stopped the key scalar at the `: ` and
    /// returned mid-line. `count_indent` counts spaces forward from `self.pos` with
    /// no line-start check, so the main loop re-derived that line's indent as 0 and
    /// `parse_explicit_value` closed the mapping it should have been filling — which
    /// is why the nested spellings silently lost the value (`{"m":{"k":null}}`) while
    /// the top-level one survived intact and merely wrong (`{"k":"v"}`).
    ///
    /// Every expectation is the output of mikefarah/yq v4.53.3, the same oracle
    /// `tests/data/yq-golden/cases/explicit_key_same_line_*` is captured from.
    #[test]
    fn test_explicit_key_and_value_on_one_line() {
        let cases: &[(&[u8], &str)] = &[
            // The three positions from the issue - all were distinct corruptions
            (b"? k: v\n", "{\"\":null}"),
            (b"m:\n  ? k: v\n", "{\"m\":{\"\":null}}"),
            (b"- ? k: v\n", "[{\"\":null}]"),
            // An explicit value on the following line belongs to the complex key
            (b"? k: v\n: w\n", "{\"\":\"w\"}"),
            // A line back at the `?`'s column ends the key and is a sibling entry;
            // one at the key's own column joins the key mapping instead
            (b"? k: v\nj: u\n", "{\"\":null,\"j\":\"u\"}"),
            (b"? k: v\n  j: u\n", "{\"\":null}"),
            // The key mapping's indent is the key content's column, not the `?`'s
            (b"?   k: v\n    j: u\n: w\n", "{\"\":\"w\"}"),
            // Quoted keys reach parse_compact_mapping_entry's own key dispatch
            (b"? \"a\": v\n", "{\"\":null}"),
            (b"? 'a': v\n", "{\"\":null}"),
            // The value indicator has the identical mid-line defect
            (b"? a\n: b: c\n", "{\"a\":{\"b\":\"c\"}}"),
            (b"? a: b\n: c: d\n", "{\"\":{\"c\":\"d\"}}"),
            // YAML Test Suite case V9D5 - needs the key *and* value arms together
            (
                b"- sun: yellow\n- ? earth: blue\n  : moon: white\n",
                "[{\"sun\":\"yellow\"},{\"\":{\"moon\":\"white\"}}]",
            ),
            // A sequence key already worked; it must keep winning over the new arm,
            // whose `: ` scan would otherwise claim this line
            (b"? - a: b\n: v\n", "{\"\":\"v\"}"),
            // A following item is a sibling of the mapping, not of its value
            (b"- ? k: v\n- z\n", "[{\"\":null},\"z\"]"),
            // Two complex keys in one mapping - yq keeps both `""` entries
            (b"? k: v\n? a: b\n", "{\"\":null,\"\":null}"),
            // An empty key inside the compact mapping (corpus M2N8/00 shape)
            (b"? : x\n", "{\"\":null}"),
            // #324 composition: the break form cannot change the structure
            (b"? k: v\r\n: w\r\n", "{\"\":\"w\"}"),
            (b"? k: v\r: w\r", "{\"\":\"w\"}"),
            // Negative pins - the new arm must not claim these
            // `:` without a following space is key text, not a value indicator
            (b"? k:v\n", "{\"k:v\":null}"),
            // The multi-line spelling, which was always correct
            (b"? k\n: v\n", "{\"k\":\"v\"}"),
            // Flow-collection keys keep their existing (still yq-divergent) handling
            (b"? []\n: x\n", "{\"\":\"x\"}"),
        ];

        for (yaml, expected) in cases {
            let index = YamlIndex::build(yaml).unwrap();
            let json = index.root(yaml).to_json_document();
            let mut streamed = String::new();
            index
                .root(yaml)
                .stream_json_document(&mut streamed, 0)
                .unwrap();
            let input = core::str::from_utf8(yaml).unwrap();
            assert_eq!(json, *expected, "to_json mismatch for {input:?}");
            assert_eq!(streamed, *expected, "stream_json mismatch for {input:?}");
        }
    }

    /// Issue #172: a non-scalar (sequence/flow-mapping) explicit key at
    /// *ordinary mapping level* — not the sequence-item position #339 covers.
    ///
    /// The original repro (`? - a\n  - b\n: value`) lost the whole entry
    /// (`{}`). That was fixed as a side effect of two unrelated changes: #325
    /// rewrote `parse_explicit_key` to delegate the key side to
    /// `parse_sequence_item`, so the sequence/flow-collection key parses
    /// correctly instead of corrupting the BP tree; #429 (closing #346) fixed
    /// `parse_explicit_key`'s mid-line return, which had been making the main
    /// loop misread the following line's indentation and, in nested
    /// positions, drop the value along with the key. yq has no way to render
    /// a non-scalar key, so it collapses to `""` on both sides — these cases
    /// pin that the *value* now survives everywhere, not that the key
    /// renders meaningfully.
    ///
    /// Every expectation is the output of mikefarah/yq v4.53.3.
    #[test]
    fn test_explicit_non_scalar_key_at_mapping_level() {
        let cases: &[(&[u8], &str)] = &[
            // The headline #172 repro, and its no-value twin
            (b"? - a\n  - b\n: value\n", "{\"\":\"value\"}"),
            (b"? - a\n  - b\n", "{\"\":null}"),
            // Sibling entries around the explicit entry are unaffected
            (
                b"x: 1\n? - a\n  - b\n: value\ny: 2\n",
                "{\"x\":1,\"\":\"value\",\"y\":2}",
            ),
            // Two non-scalar-keyed entries in one mapping - yq keeps both `""`
            (b"? - a\n: v1\n? - b\n: v2\n", "{\"\":\"v1\",\"\":\"v2\"}"),
            // A flow mapping as the key, key and value on separate lines
            (b"? {a: 1}\n: value\n", "{\"\":\"value\"}"),
            // Key content (itself a block mapping) indented on the line after `?`
            (b"?\n  a: 1\n: value\n", "{\"\":\"value\"}"),
        ];

        for (yaml, expected) in cases {
            let index = YamlIndex::build(yaml).unwrap();
            let json = index.root(yaml).to_json_document();
            let mut streamed = String::new();
            index
                .root(yaml)
                .stream_json_document(&mut streamed, 0)
                .unwrap();
            let input = core::str::from_utf8(yaml).unwrap();
            assert_eq!(json, *expected, "to_json mismatch for {input:?}");
            assert_eq!(streamed, *expected, "stream_json mismatch for {input:?}");
        }
    }

    /// Issue #346: the key of `? k: v` must be a real *mapping* node.
    ///
    /// `to_json_document` cannot show this — `key_string()` renders both a mapping
    /// and an empty scalar as `""`, so a parser that emitted an empty scalar key
    /// would produce byte-identical JSON while carrying a different tree. This walks
    /// the cursor to prove the BP tree is `map( map( k v ) null )`.
    #[test]
    fn test_explicit_key_on_one_line_exposes_a_mapping_key() {
        let yaml = b"? k: v\n";
        let index = YamlIndex::build(yaml).unwrap();

        let fields = match first_doc(index.root(yaml)) {
            YamlValue::Mapping(fields) => fields,
            other => panic!("expected mapping, got {other:?}"),
        };
        let entries: Vec<_> = fields.into_iter().collect();
        assert_eq!(
            entries.len(),
            1,
            "`k: v` is one complex key, not two entries"
        );

        // The key is the compact mapping `{k: v}`, not the scalar `k`.
        let key_fields = match entries[0].key_cursor.value() {
            YamlValue::Mapping(key_fields) => key_fields,
            other => panic!("expected the key to be a mapping, got {other:?}"),
        };
        let key_entries: Vec<_> = key_fields.into_iter().collect();
        assert_eq!(
            key_entries.len(),
            1,
            "the owner's null must not land inside the key mapping"
        );
        assert_eq!(&*key_entries[0].key().as_str().unwrap(), "k");
        match key_entries[0].value_cursor.value() {
            YamlValue::String(s) => assert_eq!(&*s.as_str().unwrap(), "v"),
            other => panic!("expected string value, got {other:?}"),
        }

        // ...and the entry's own value is null - the owner's null landed on the
        // owner, not inside the key mapping as a stray third child.
        assert!(
            matches!(entries[0].value_cursor.value(), YamlValue::Null),
            "the complex key has no value"
        );
    }

    /// Issue #346: a `: value` line binds to the *entry*, never into the complex key.
    ///
    /// `key_string()` flattens any complex key to `""`, so the JSON is `{"":"w"}`
    /// whatever the tree underneath looks like — only the cursor separates "the key
    /// is `{k: v}` and the value is `w`" from a key that swallowed one of them.
    ///
    /// The discriminating case for the ownership change itself is
    /// [`test_explicit_key_on_one_line_exposes_a_mapping_key`]: at EOF `end_document`
    /// drains the stack, and keying the null on "the container being popped is a
    /// `Mapping`" writes the owner's null into the key mapping, which loses the entry
    /// entirely (`{}`). This shape survives that bug by luck — the stray child is
    /// unpaired, so the field iterator drops it — which is exactly why it is pinned
    /// separately rather than relied on.
    #[test]
    fn test_explicit_key_on_one_line_keeps_its_value_out_of_the_key() {
        let yaml = b"? k: v\n: w\n";
        let index = YamlIndex::build(yaml).unwrap();

        let fields = match first_doc(index.root(yaml)) {
            YamlValue::Mapping(fields) => fields,
            other => panic!("expected mapping, got {other:?}"),
        };
        let entries: Vec<_> = fields.into_iter().collect();
        assert_eq!(entries.len(), 1, "one entry: a complex key and `w`");

        let key_entries: Vec<_> = match entries[0].key_cursor.value() {
            YamlValue::Mapping(key_fields) => key_fields.into_iter().collect(),
            other => panic!("expected the key to be a mapping, got {other:?}"),
        };
        assert_eq!(
            key_entries.len(),
            1,
            "no null may be written into the key mapping when it is popped"
        );
        assert_eq!(&*key_entries[0].key().as_str().unwrap(), "k");

        match entries[0].value_cursor.value() {
            YamlValue::String(s) => assert_eq!(&*s.as_str().unwrap(), "w"),
            other => panic!("expected `w` as the value, got {other:?}"),
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
    fn test_undefined_alias_is_rejected() {
        // #372: this used to build successfully and leave the alias with
        // `target: None`, which rendered as `null` — an invented value for
        // input YAML 1.2 §7.1 calls invalid. It is now refused at build time,
        // as a cyclic alias always has been.
        let yaml = b"bad: *undefined";
        let err = YamlIndex::build(yaml).expect_err("undefined alias should not build");
        assert!(
            matches!(&err, YamlError::UnknownAnchor { name, .. } if name == "undefined"),
            "expected UnknownAnchor naming the anchor, got: {err:?}"
        );
    }

    #[test]
    fn test_defined_alias_still_resolves() {
        // The other side of #372: a *resolvable* alias must be untouched. This
        // is the boundary the rejection above must not cross.
        let yaml = b"good: &a 1\nref: *a";
        let index = YamlIndex::build(yaml).unwrap();
        let root = index.root(yaml);

        let YamlValue::Mapping(fields) = first_doc(root) else {
            panic!("expected mapping");
        };
        for field in fields {
            if let YamlValue::String(key) = field.key() {
                if key.as_str().unwrap() == "ref" {
                    if let YamlValue::Alias {
                        anchor_name,
                        target,
                    } = field.value()
                    {
                        assert_eq!(anchor_name, "a");
                        assert!(target.is_some(), "defined alias must resolve");
                        return;
                    }
                    panic!("expected alias for ref");
                }
            }
        }
        panic!("did not find ref field");
    }

    /// The boundary #372's rejection must not cross: wherever an anchor is in
    /// scope, the alias still resolves to its value.
    ///
    /// Rejecting a lookup miss is only safe if every anchor a document defines
    /// is actually *registered*. Two paths recorded no anchor at all, so the
    /// alias missed and — before the rejection landed — quietly yielded `null`:
    /// a compact mapping entry inside a block sequence item (`- k: &a 1`) and a
    /// document-root value (`--- &a 1`). Turning a miss into an error would
    /// have converted those into a refusal to parse valid YAML, so this asserts
    /// the resolved output rather than merely that the build succeeds.
    #[test]
    fn test_alias_resolves_wherever_the_anchor_is_in_scope() {
        // (input, expected JSON for the first document)
        let cases: &[(&[u8], &str)] = &[
            // Values.
            (b"a: &x 1\nb: *x", r#"{"a":1,"b":1}"#),
            (b"a: &x [1,2]\nb: *x", r#"{"a":[1,2],"b":[1,2]}"#),
            (b"- &x 1\n- *x", r"[1,1]"),
            (b"a: [&x 1, *x]", r#"{"a":[1,1]}"#),
            (b"a: {k: &x 1, b: *x}", r#"{"a":{"k":1,"b":1}}"#),
            // Anchor on a compact mapping entry's value, aliased from a later
            // entry of the same item, from a later item, and from outside the
            // sequence. This is the shape a Kubernetes manifest writes.
            (b"- a: &x 1\n  b: *x", r#"[{"a":1,"b":1}]"#),
            (b"- a: &x 1\n- b: *x", r#"[{"a":1},{"b":1}]"#),
            (b"l:\n  - a: &x 1\nr: *x", r#"{"l":[{"a":1}],"r":1}"#),
            // An anchor on a document-root value, the other path that recorded
            // nothing. Its node is on the `---` line here, so the anchor has
            // something to name — unlike `--- &x` alone, which names the
            // following document's placeholder and is rejected instead
            // (`test_build_rejects_alias_to_unknown_anchor_in_every_position`).
            (b"--- &x [1]", r"[1]"),
            // Keys.
            (b"&x k: 1\n*x: 2", r#"{"k":1,"k":2}"#),
            (b"- &x k: 1\n- *x: 2", r#"[{"k":1},{"k":2}]"#),
            (b"k: &x 1\n? *x\n: v", r#"{"k":1,"1":"v"}"#),
            // Flow-mapping keys (#405), the position #372 left inconsistent
            // with itself: a miss went through `parse_alias`'s lookup and
            // errored, while a hit bound the edge to the node `parse_alias`
            // opened *below* the already-open key, so the key kept no extent of
            // its own and rendered as `""`. Aliasing an anchored key and an
            // anchored value are separate targets, hence both.
            (b"{&x k: 1, *x: 2}", r#"{"k":1,"k":2}"#),
            (b"{k: &x 1, *x: 2}", r#"{"k":1,"1":2}"#),
            (b"a: {&x k: 1, *x: 2}", r#"{"a":{"k":1,"k":2}}"#),
            (b"[{&x k: 1}, {*x: 2}]", r#"[{"k":1},{"k":2}]"#),
            // Space before the `:`. Resolution is by BP position, not by
            // re-reading the alias name from text, so this isn't pinning name
            // lookup — it's a regression check that a stray space before the
            // colon still parses and resolves.
            (b"{&x k: 1, *x : 2}", r#"{"k":1,"k":2}"#),
            // Alias key with an implicit null value, which is the one flow
            // shape where the caller records the key's end and then writes an
            // empty value node.
            (b"{&x k: 1, *x}", r#"{"k":1,"k":null}"#),
            // Two aliases to one anchor: resolution is per-node, so the second
            // must not depend on the first having consumed the edge.
            (b"{&x k: 1, *x: 2, *x: 3}", r#"{"k":1,"k":2,"k":3}"#),
            // Flow-*sequence* implicit-mapping-entry keys (#409), a separate
            // bug from #405 above: the sequence loop consumed a leading `&`
            // or `*` before ever checking whether the item was a pair, so a
            // miss errored `expected ',' or ']'` instead of naming the
            // unknown anchor, and a hit either failed the same way (alias) or
            // bound to the mapping wrapper instead of the key (anchor).
            (b"[&x k: 1, *x: 2]", r#"[{"k":1},{"k":2}]"#),
            // Anchor on the key, aliased by a later plain (non-pair) item.
            (b"[&x k: 1, *x]", r#"[{"k":1},"k"]"#),
            // Anchored value in a block mapping, aliased as a flow-sequence
            // implicit-entry key nested in that mapping's value.
            (b"{a: &x 1, b: [*x: 2]}", r#"{"a":1,"b":[{"1":2}]}"#),
        ];
        for (yaml, expected) in cases {
            assert_eq!(
                first_doc_json(yaml),
                *expected,
                "in: {}",
                String::from_utf8_lossy(yaml)
            );
        }
    }

    /// An alias *is* a document (`--- *x`), the one alias position whose result
    /// the first-document helper above cannot see.
    ///
    /// Anchors carry across documents here, and `yq` v4.53.3 agrees, so the
    /// second document is the anchored value rather than `null` as it was
    /// before #372 routed this position through `parse_value`.
    #[test]
    fn test_alias_as_a_whole_document_resolves() {
        let yaml = b"a: &x 1\n--- *x";
        let index = YamlIndex::build(yaml).expect("builds");
        assert_eq!(index.root(yaml).to_json(), r#"[{"a":1},1]"#);
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
                            assert_eq!(items.len(), 2, "expected 2 items, got: {items:?}");
                            if let YamlValue::String(s) = &items[0] {
                                assert_eq!(&*s.as_str().unwrap(), "one");
                            }
                            return;
                        }
                        panic!("expected sequence for items, got: {val:?}");
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
                for field in fields {
                    if let YamlValue::String(k) = field.key() {
                        if let YamlValue::String(v) = field.value() {
                            let key_str = k.as_str().unwrap();
                            let val_str = v.as_str().unwrap();
                            pairs.push(format!("\"{key_str}\":\"{val_str}\""));
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
            "not a JSON string: {json}"
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
                    Some(c) => panic!("unknown escape: \\{c}"),
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
            "{context}: JSON strings decode to different values.\n  transcoded JSON: {transcoded}\n  decoded JSON: {decoded}\n  transcoded value: {transcoded_value:?}\n  decoded value: {decoded_value:?}"
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
        // Both paths stream U+0085 as raw UTF-8, matching yq (#532).
        assert_eq!(transcoded, decoded);
        assert_eq!(transcoded, "\"next\u{0085}line\"");
    }

    #[test]
    fn test_transcode_double_quoted_nbsp_escape() {
        let yaml = b"\"non\\_break\"";
        let transcoded = get_json_via_transcode(yaml);
        let decoded = get_json_via_decode(yaml);
        // Both paths stream U+00A0 as raw UTF-8, matching yq (#532).
        assert_eq!(transcoded, decoded);
        assert_eq!(transcoded, "\"non\u{00a0}break\"");
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
        // Both paths stream € as raw UTF-8, matching yq (#532).
        assert_eq!(transcoded, decoded);
        assert_eq!(transcoded, "\"euro€sign\"");
    }

    #[test]
    fn test_transcode_double_quoted_unicode_8digit_escape() {
        let yaml = b"\"emoji\\U0001F600face\""; // 😀 = U+1F600
        let transcoded = get_json_via_transcode(yaml);
        let decoded = get_json_via_decode(yaml);
        // Both paths stream 😀 as raw UTF-8, matching yq (#532).
        assert_eq!(transcoded, decoded);
        assert_eq!(transcoded, "\"emoji😀face\"");
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
            "got: {transcoded}"
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
            "got: {transcoded}"
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
            "got: {transcoded}"
        );
    }

    #[test]
    fn test_transcode_matches_old_multiline_single_quoted_with_escaped_quote() {
        // Equivalent to test_multiline_single_quoted_with_escaped_quote but via transcode
        let yaml = b"key: 'it''s\n  working'";
        let transcoded = get_json_via_transcode(yaml);
        // Should contain "it's working"
        assert!(transcoded.contains("it's working"), "got: {transcoded}");
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

    /// A `|+` whose content line never arrives: keep chomping still holds the
    /// break that ended the indicator line, and the scan for further trailing
    /// blank lines must stop at the next mapping key rather than swallowing it.
    /// That stop is the one place the empty-block-scalar scan asks whether it
    /// is still looking at a break, so it is measured, not assumed (#341).
    ///
    /// `yq` reads all three spellings as `{"a": "\n", "b": "c"}`.
    #[test]
    fn keep_chomped_empty_block_scalar_stops_at_the_next_key() {
        for yaml in [
            b"a: |+\n\nb: c\n".as_slice(),
            b"a: |+\r\n\r\nb: c\r\n".as_slice(),
            b"a: |+\r\rb: c\r".as_slice(),
        ] {
            let input = String::from_utf8_lossy(yaml);
            assert_eq!(doc_json(yaml), r#"[{"a":"\n","b":"c"}]"#, "input {input:?}");
        }
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

    /// Returns the tag of the value in a single-entry mapping document.
    #[track_caller]
    fn value_tag(yaml: &[u8]) -> &'static str {
        let index = YamlIndex::build(yaml).unwrap();
        let root = index.root(yaml);
        if let YamlValue::Mapping(fields) = first_doc(root) {
            if let Some(field) = fields.into_iter().next() {
                return field.value_cursor().tag();
            }
        }
        panic!("Could not find value in {:?}", core::str::from_utf8(yaml));
    }

    #[test]
    fn test_tag_core_schema_resolution() {
        // Null spellings (issue #226): Null/NULL are null, mixed case is not
        assert_eq!(value_tag(b"key: Null"), "!!null");
        assert_eq!(value_tag(b"key: NULL"), "!!null");
        assert_eq!(value_tag(b"key: NuLl"), "!!str");
        // Hex/octal ints; YAML 1.1 legacy forms stay strings
        assert_eq!(value_tag(b"key: 0x2A"), "!!int");
        assert_eq!(value_tag(b"key: 0o52"), "!!int");
        assert_eq!(value_tag(b"key: 0b101"), "!!str");
        assert_eq!(value_tag(b"key: 1_000"), "!!str");
        // Bare nan/inf require the leading dot in 1.2 core
        assert_eq!(value_tag(b"key: nan"), "!!str");
        assert_eq!(value_tag(b"key: inf"), "!!str");
        assert_eq!(value_tag(b"key: Infinity"), "!!str");
        assert_eq!(value_tag(b"key: .inf"), "!!float");
        assert_eq!(value_tag(b"key: +.inf"), "!!float");
        assert_eq!(value_tag(b"key: .nan"), "!!float");
        // Float overflow to infinity is not a core-schema float
        assert_eq!(value_tag(b"key: 1e999"), "!!str");
        assert_eq!(value_tag(b"key: .5"), "!!float");
    }

    /// Returns `to_json` for a document (exercises the buffer transcoder).
    /// The root wraps documents in an implicit sequence, hence `[...]`.
    #[track_caller]
    fn doc_json(yaml: &[u8]) -> String {
        let index = YamlIndex::build(yaml).unwrap();
        index.root(yaml).to_json()
    }

    #[test]
    fn test_to_json_core_schema_resolution() {
        assert_eq!(doc_json(b"a: Null\n"), r#"[{"a":null}]"#);
        assert_eq!(doc_json(b"a: NULL\n"), r#"[{"a":null}]"#);
        assert_eq!(doc_json(b"a: NuLl\n"), r#"[{"a":"NuLl"}]"#);
        assert_eq!(doc_json(b"a: 0x2A\n"), r#"[{"a":42}]"#);
        assert_eq!(doc_json(b"a: 0o52\n"), r#"[{"a":42}]"#);
        assert_eq!(doc_json(b"a: nan\n"), r#"[{"a":"nan"}]"#);
        assert_eq!(doc_json(b"a: Infinity\n"), r#"[{"a":"Infinity"}]"#);
        // JSON cannot represent the .inf/.nan family
        assert_eq!(doc_json(b"a: .inf\n"), r#"[{"a":null}]"#);
        assert_eq!(doc_json(b"a: +.5\n"), r#"[{"a":0.5}]"#);
        assert_eq!(doc_json(b"a: 1_000\n"), r#"[{"a":"1_000"}]"#);
        assert_eq!(doc_json(b"a: 1e999\n"), r#"[{"a":"1e999"}]"#);
    }

    #[test]
    fn test_to_json_block_scalars_stay_strings() {
        // Block scalars are always !!str; their content must never be
        // type-resolved (yq-verified behavior)
        assert_eq!(doc_json(b"a: |-\n  42\n"), r#"[{"a":"42"}]"#);
        assert_eq!(doc_json(b"a: |-\n  true\n"), r#"[{"a":"true"}]"#);
        assert_eq!(doc_json(b"a: |-\n  null\n"), r#"[{"a":"null"}]"#);
        assert_eq!(doc_json(b"a: >-\n  42\n"), r#"[{"a":"42"}]"#);
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

    // ========================================================================
    // The alternate-parse-path scanners (#324)
    //
    // The `#[allow(dead_code)]` STYLE-0005 helpers below are not on the current
    // parse path, so no integration test can reach them. They were still made
    // CR-aware, because a helper that silently keeps LF-only semantics is a
    // trap for whoever re-enables it. These tests pin that behaviour directly
    // so it cannot rot unnoticed.
    //
    // The line-break predicates they build on moved to `super::line_break` in
    // #341 and are tested there.
    // ========================================================================

    /// Build an index and hand back the root cursor, for the private scanners
    /// below that need a `YamlCursor` but not a particular node.
    fn cursor_over(yaml: &[u8]) -> (YamlIndex<Vec<u64>>, &[u8]) {
        (YamlIndex::build(yaml).expect("parses"), yaml)
    }

    #[test]
    fn find_plain_scalar_end_stops_at_every_break_form() {
        // `value` starts at offset 3 in each spelling; the scan must end at 8,
        // never carrying the break into the scalar.
        for yaml in [
            b"a: value\nb: 2\n".as_slice(),
            b"a: value\r\nb: 2\r\n".as_slice(),
            b"a: value\rb: 2\r".as_slice(),
        ] {
            let (index, text) = cursor_over(yaml);
            let root = index.root(text);
            let input = String::from_utf8_lossy(yaml);
            assert_eq!(
                root.find_plain_scalar_end(3, 0, false),
                8,
                "input {input:?}"
            );
        }
    }

    #[test]
    fn find_plain_scalar_end_folds_continuation_lines() {
        // A more-indented next line continues the scalar under every spelling.
        for yaml in [
            b"a: one\n  two\n".as_slice(),
            b"a: one\r\n  two\r\n".as_slice(),
            b"a: one\r  two\r".as_slice(),
        ] {
            let (index, text) = cursor_over(yaml);
            let root = index.root(text);
            let end = root.find_plain_scalar_end(3, 0, false);
            let input = String::from_utf8_lossy(yaml);
            let scanned = String::from_utf8_lossy(&text[3..end]);
            assert!(
                scanned.ends_with("two"),
                "input {input:?} scanned {scanned:?}"
            );
        }
    }

    /// A blank line between two content lines does not end a plain scalar —
    /// the scan steps over it and keeps going. The blank-line arm is where the
    /// scan measures a break rather than assuming one byte, so a CRLF blank
    /// line would otherwise leave it standing on the orphaned LF and read that
    /// as a second blank line (#341).
    #[test]
    fn find_plain_scalar_end_steps_over_blank_lines() {
        for yaml in [
            b"a: one\n\n  two\nb: 2\n".as_slice(),
            b"a: one\r\n\r\n  two\r\nb: 2\r\n".as_slice(),
            b"a: one\r\r  two\rb: 2\r".as_slice(),
        ] {
            let (index, text) = cursor_over(yaml);
            let root = index.root(text);
            let end = root.find_plain_scalar_end(3, 0, false);
            let input = String::from_utf8_lossy(yaml);
            let scanned = String::from_utf8_lossy(&text[3..end]);
            assert!(
                scanned.ends_with("two"),
                "input {input:?} scanned {scanned:?}"
            );
        }
    }

    /// A line holding only spaces and tabs counts as blank for folding, and the
    /// scan resumes after *its* break. Reached through the tab arm, which walks
    /// the whitespace run itself and so must measure the break at wherever that
    /// run stops rather than at the line's start (#341).
    #[test]
    fn find_plain_scalar_end_treats_a_whitespace_only_line_as_blank() {
        for yaml in [
            b"a: one\n \t \n  two\nb: 2\n".as_slice(),
            b"a: one\r\n \t \r\n  two\r\nb: 2\r\n".as_slice(),
            b"a: one\r \t \r  two\rb: 2\r".as_slice(),
        ] {
            let (index, text) = cursor_over(yaml);
            let root = index.root(text);
            let end = root.find_plain_scalar_end(3, 0, false);
            let input = String::from_utf8_lossy(yaml);
            let scanned = String::from_utf8_lossy(&text[3..end]);
            assert!(
                scanned.ends_with("two"),
                "input {input:?} scanned {scanned:?}"
            );
        }
    }

    #[test]
    fn find_plain_scalar_end_keeps_a_colon_that_is_not_a_separator() {
        // `12:30` and `http://` hold colons the scan must walk past — only a
        // colon followed by whitespace or a break ends the scalar.
        for yaml in [
            b"a: at 12:30 sharp\nb: 2\n".as_slice(),
            b"a: at 12:30 sharp\r\nb: 2\r\n".as_slice(),
            b"a: at 12:30 sharp\rb: 2\r".as_slice(),
        ] {
            let (index, text) = cursor_over(yaml);
            let root = index.root(text);
            let end = root.find_plain_scalar_end(3, 0, false);
            let input = String::from_utf8_lossy(yaml);
            assert_eq!(&text[3..end], b"at 12:30 sharp", "input {input:?}");
        }
    }

    #[test]
    fn compute_base_indent_and_root_flag_finds_the_line_start() {
        // Second line's value sits at indent 2 and is not document root. The
        // walk back to the line start has to recognise the preceding break.
        for yaml in [
            b"top:\n  key: value\n".as_slice(),
            b"top:\r\n  key: value\r\n".as_slice(),
            b"top:\r  key: value\r".as_slice(),
        ] {
            let (index, text) = cursor_over(yaml);
            let root = index.root(text);
            let value_pos = text
                .windows(5)
                .position(|w| w == b"value")
                .expect("input holds `value`");
            let (indent, is_root) = root.compute_base_indent_and_root_flag(value_pos);
            assert_eq!(
                (indent, is_root),
                (2, false),
                "input {:?}",
                String::from_utf8_lossy(yaml)
            );
        }
    }

    /// The line scan also recognises an explicit-key indicator (`? `), whose
    /// terminator set includes both break bytes.
    #[test]
    fn compute_base_indent_and_root_flag_sees_an_explicit_key_indicator() {
        for yaml in [
            b"? key\n: value\n".as_slice(),
            b"? key\r\n: value\r\n".as_slice(),
            b"? key\r: value\r".as_slice(),
        ] {
            let (index, text) = cursor_over(yaml);
            let root = index.root(text);
            let key_pos = text
                .windows(3)
                .position(|w| w == b"key")
                .expect("input holds `key`");
            // Reached at all is the point: the `?` branch is on this path only
            // when the indicator is followed by whitespace or a break.
            let (indent, _) = root.compute_base_indent_and_root_flag(key_pos);
            assert_eq!(indent, 0, "input {:?}", String::from_utf8_lossy(yaml));
        }
    }

    #[test]
    fn compute_base_indent_and_root_flag_looks_back_for_a_document_marker() {
        // A value alone on its line sends the scan back over the *previous*
        // line to look for `---`. That walk-back is a second line-start scan,
        // and a lone CR hides the boundary from an LF-only one.
        for (yaml, expect_root) in [
            (b"---\nplain\n".as_slice(), true),
            (b"---\r\nplain\r\n".as_slice(), true),
            (b"---\rplain\r".as_slice(), true),
            // No marker on the previous line: not document root.
            (b"key:\nplain\n".as_slice(), false),
            (b"key:\r\nplain\r\n".as_slice(), false),
            (b"key:\rplain\r".as_slice(), false),
        ] {
            let (index, text) = cursor_over(yaml);
            let root = index.root(text);
            let value_pos = text
                .windows(5)
                .position(|w| w == b"plain")
                .expect("input holds `plain`");
            let (_, is_root) = root.compute_base_indent_and_root_flag(value_pos);
            assert_eq!(
                is_root,
                expect_root,
                "input {:?}",
                String::from_utf8_lossy(yaml)
            );
        }
    }

    #[test]
    fn is_in_flow_context_rewinds_to_a_line_start_past_4kb() {
        // Past 4 KB into the document the scan no longer starts at byte 0: it
        // rewinds ~4 KB and then back to the enclosing line start, so it never
        // begins mid-line. That rewind is its own line-start walk, and a lone
        // CR hides the boundary from an LF-only one.
        //
        // The scan window is deliberately bounded, so the opener has to sit
        // *inside* the last 4 KB for the answer to be `true` — this pads before
        // the opener, not after it.
        for (nl, form) in [("\n", "LF"), ("\r\n", "CRLF"), ("\r", "CR")] {
            let mut yaml = String::new();
            for i in 0..400 {
                yaml.push_str(&alloc::format!("pad{i}: value{nl}"));
            }
            let opener = yaml.len();
            yaml.push_str(&alloc::format!("opener: [1,{nl}  last]{nl}"));
            let bytes = yaml.as_bytes();
            assert!(
                opener > 4096,
                "{form}: opener must sit past the 4 KB rewind"
            );

            let index = YamlIndex::build(bytes).expect("parses");
            let root = index.root(bytes);
            let inside = yaml.rfind("last").expect("holds `last`");
            assert!(
                root.is_in_flow_context(inside),
                "{form}: `last` is inside the flow sequence opened on the line before"
            );
            assert!(
                !root.is_in_flow_context(opener),
                "{form}: the opener line's own start is not yet inside the flow"
            );
        }
    }

    #[test]
    fn is_in_flow_context_sees_an_opener_on_an_earlier_line() {
        // The scan walks back over line starts; with an LF-only walk a lone CR
        // hides the line boundary and the answer flips.
        for yaml in [
            b"a: [1,\n  2]\n".as_slice(),
            b"a: [1,\r\n  2]\r\n".as_slice(),
            b"a: [1,\r  2]\r",
        ] {
            let (index, text) = cursor_over(yaml);
            let root = index.root(text);
            let inside = text.iter().position(|&b| b == b'2').expect("holds `2`");
            assert!(
                root.is_in_flow_context(inside),
                "input {:?}",
                String::from_utf8_lossy(yaml)
            );
            assert!(!root.is_in_flow_context(0), "column 0 is outside any flow");
        }
    }

    // ========================================================================
    // Sequence-entry indicator (`- `) classification — #332
    // ========================================================================

    /// Cursor at the first document. The root is always the implicit document
    /// sequence, so this is the node a `yq` filter of `.` actually sees.
    #[track_caller]
    fn first_doc_cursor<'a, W: AsRef<[u64]>>(
        index: &'a YamlIndex<W>,
        yaml: &'a [u8],
    ) -> YamlCursor<'a, W> {
        let YamlValue::Sequence(docs) = index.root(yaml).value() else {
            panic!("root is always the document sequence");
        };
        let (cursor, _rest) = docs.uncons_cursor().expect("one document");
        cursor
    }

    /// JSON for the first document, asserting the buffered (`to_json`) and streaming
    /// (`stream_json`, what `yq -o json -I0` takes) emitters agree.
    #[track_caller]
    fn first_doc_json(yaml: &[u8]) -> String {
        let index = YamlIndex::build(yaml).expect("builds");
        let cursor = first_doc_cursor(&index, yaml);

        let buffered = cursor.to_json();
        let mut streamed = String::new();
        cursor.stream_json(&mut streamed, 0).expect("streams");
        let what = String::from_utf8_lossy(yaml);
        assert_eq!(
            buffered, streamed,
            "buffered and streaming JSON differ for {what}"
        );
        buffered
    }

    /// A plain scalar beginning `- ` is only reachable where no block sequence can
    /// start: inside a flow collection, and as a same-line block mapping value. The
    /// input is invalid YAML (the strict validator rejects it) but the loader is
    /// lenient, and preserving the text beats silently dropping it (#332).
    #[test]
    fn test_dash_space_plain_scalar_content_is_preserved() {
        // Flow sequence — the shapes from the issue report
        assert_eq!(first_doc_json(b"[- x]\n"), r#"["- x"]"#);
        assert_eq!(first_doc_json(b"[- x, 1]\n"), r#"["- x",1]"#);
        assert_eq!(first_doc_json(b"[a, - b]\n"), r#"["a","- b"]"#);
        // Flow mapping value, and a flow sequence nested in a flow mapping
        assert_eq!(first_doc_json(b"{a: - x}\n"), r#"{"a":"- x"}"#);
        assert_eq!(first_doc_json(b"{a: [- x]}\n"), r#"{"a":["- x"]}"#);
        // Multi-line flow collection: the `-` is followed by a newline, so a
        // line-local text scan would misread this as a block sequence item
        assert_eq!(first_doc_json(b"[\n  - x\n]\n"), r#"["- x"]"#);
        // `- ` with nothing after it inside flow: trailing space is trimmed, so the
        // extent is the dash alone
        assert_eq!(first_doc_json(b"[- ]\n"), r#"["-"]"#);
        // Same-line block mapping value is NOT this rule's business any more: #325
        // made the parser emit a real block sequence there, exactly as the note this
        // replaces anticipated. Pinned in `test_inline_seq_as_mapping_value_*` below.
        assert_eq!(first_doc_json(b"key: - a\n"), r#"{"key":["a"]}"#);
    }

    /// A `- ` on a plain scalar's *continuation* line (as opposed to its first
    /// line, covered above) is ordinary content at any indent greater than the
    /// enclosing block's own indent, matching `yq`. The corpus's only case for
    /// this shape, AB8U, uses continuation indent exactly 1 - the parser used to
    /// treat that as the *only* valid indent and wrongly cut the scalar short
    /// (reparsing the `- ` line as a nested sequence) at indent 2 and deeper,
    /// which AB8U can never catch (#484, corpus-latent like #382 and #409).
    #[test]
    fn test_dash_continuation_at_any_indent_folds_into_plain_scalar() {
        // AB8U itself (continuation indent 1) must stay passing.
        assert_eq!(
            first_doc_json(b"- single multiline\n - sequence entry\n"),
            r#"["single multiline - sequence entry"]"#
        );
        // The bug's exact repro (indent 2), and deeper indents - none of these
        // should stop the fold.
        assert_eq!(first_doc_json(b"- x\n  - y\n"), r#"["x - y"]"#);
        assert_eq!(first_doc_json(b"- x\n   - y\n"), r#"["x - y"]"#);
        assert_eq!(first_doc_json(b"- x\n    - y\n"), r#"["x - y"]"#);
        assert_eq!(first_doc_json(b"- x\n     - y\n"), r#"["x - y"]"#);
        // Same shape as a mapping value.
        assert_eq!(
            first_doc_json(b"b:\n  - x\n    - y\n"),
            r#"{"b":["x - y"]}"#
        );
        // Regression guard: a genuinely nested sequence is unaffected, since it's
        // recognized immediately after an item's own `-`, not via continuation
        // folding.
        assert_eq!(first_doc_json(b"- - y\n"), r#"[["y"]]"#);
        assert_eq!(first_doc_json(b"- x\n- - y\n"), r#"["x",["y"]]"#);
    }

    /// A same-line block mapping value that is a bare sequence-entry indicator, with
    /// no content after it. The parser records an extent covering the `-` alone
    /// (trailing spaces are trimmed), so this decodes as the string `"-"` — where
    /// before #332 it was `null`, the same silent loss as `a: - x`.
    ///
    /// Pinned apart from the `- x` shapes because it is the one #325 is expected to
    /// flip: once the parser emits a real sequence for a same-line `- ` value, this
    /// becomes the empty item `{"a":[null]}`. `yq` rejects the input either way
    /// (`block sequence entries are not allowed in this context`).
    #[test]
    fn test_bare_dash_as_same_line_mapping_value_is_an_empty_item() {
        // Renamed and re-pinned by #325. #332 made these read back as the string
        // "-" so the byte would not be lost; #325 makes the same-line mapping value
        // a real block sequence, so the `-` is an *indicator* with an empty item
        // after it, not content. Nothing is dropped either way — the dash was never
        // a scalar to begin with (YAML 1.2 `ns-plain-first`) — and this now agrees
        // with `a: - x` on the line above it instead of splitting on whether the
        // item happens to be empty.
        assert_eq!(first_doc_json(b"a: - \n"), r#"{"a":[null]}"#);
        assert_eq!(first_doc_json(b"a: -\n"), r#"{"a":[null]}"#);
        assert_eq!(first_doc_json(b"a: -  \n"), r#"{"a":[null]}"#);
        assert_eq!(first_doc_json(b"a: -\t\n"), r#"{"a":[null]}"#);
        // The same `- ` on the *next* line is a real block sequence whose one item is
        // empty: a childless wrapper, which stays null. That is the discriminator's
        // other side, and it is why the parser must never record an end for a wrapper.
        assert_eq!(first_doc_json(b"a:\n  - \n"), r#"{"a":[null]}"#);
    }

    /// A `-` *not* followed by whitespace is an ordinary plain scalar in flow
    /// context and must keep decoding as one.
    #[test]
    fn test_dash_without_whitespace_stays_a_plain_scalar() {
        assert_eq!(first_doc_json(b"[-]\n"), r#"["-"]"#);
        assert_eq!(first_doc_json(b"{a: -}\n"), r#"{"a":"-"}"#);
        assert_eq!(first_doc_json(b"[-1, -2]\n"), "[-1,-2]");
        assert_eq!(first_doc_json(b"[-a]\n"), r#"["-a"]"#);
    }

    /// The other shape that reaches the childless `- ` branch: a genuine empty
    /// block sequence item, for which the parser records no end position. These
    /// must stay null — this is what the end-position discriminator protects.
    #[test]
    fn test_empty_block_sequence_items_are_null() {
        assert_eq!(first_doc_json(b"-\n"), "[null]");
        assert_eq!(first_doc_json(b"- \n"), "[null]");
        assert_eq!(first_doc_json(b"- # comment\n"), "[null]");
        assert_eq!(first_doc_json(b"a:\n  -\n  - y\n"), r#"{"a":[null,"y"]}"#);
        // An anchor before an inline `- ` used to leave the wrapper with neither a
        // child nor an end, so it reached this branch and read as null. #328 made
        // it a real nested sequence, so it now short-circuits at `is_container()`
        // long before the discriminator — kept as a pin that the two fixes compose.
        assert_eq!(first_doc_json(b"- &a - x\n"), r#"[["x"]]"#);
    }

    /// The empty-key arms of `parse_explicit_flow_mapping_entry`. #332 split what was
    /// one shared `set_bp_text_end(key_end)` into a call per arm so the nested-container
    /// arms could opt out; these two keep recording an end equal to the key's own start,
    /// which decodes as the empty key `""`.
    #[test]
    fn test_explicit_flow_key_with_no_key_is_empty() {
        // `?` then the value indicator — the `Some(b':')` arm
        assert_eq!(first_doc_json(b"[? : v]\n"), r#"[{"":"v"}]"#);
        // `?` then a terminator — the `Some(b',' | b']' | b'}')` arm
        assert_eq!(first_doc_json(b"[? , a]\n"), r#"[{"":null},"a"]"#);
        assert_eq!(first_doc_json(b"[? ]\n"), r#"[{"":null}]"#);
    }

    /// Ordinary block sequence items still unwrap to their content — the hot path
    /// the discriminator must not disturb.
    #[test]
    fn test_block_sequence_items_still_unwrap_to_their_content() {
        assert_eq!(first_doc_json(b"- x\n"), r#"["x"]"#);
        assert_eq!(first_doc_json(b"- x\n- y\n"), r#"["x","y"]"#);
        assert_eq!(first_doc_json(b"- - x\n"), r#"[["x"]]"#);
        assert_eq!(first_doc_json(b"- k: v\n"), r#"[{"k":"v"}]"#);
        assert_eq!(first_doc_json(b"-\n  a: b\n"), r#"[{"a":"b"}]"#);
        assert_eq!(first_doc_json(b"-\tx\n"), r#"["x"]"#);
    }

    /// `YamlElements` has three accessors and they used to open-code the `- ` test
    /// three different ways, with three different acceptance sets. They now all
    /// route through `value()`; this asserts they *agree*, so re-divergence fails a
    /// test instead of shipping (#332, and the same class of bug as #106).
    #[test]
    fn test_sequence_element_accessors_agree() {
        let inputs: &[&[u8]] = &[
            b"[- x, 1, - y]\n",
            b"[-, -1, - ]\n",
            b"- x\n- y\n- - z\n",
            b"-\n- y\n",
            b"- # c\n- y\n",
            b"a:\n  -\n  - y\n  - k: v\n",
            b"[\n  - x,\n  y\n]\n",
            b"- &a x\n- *a\n",
        ];
        for yaml in inputs {
            let index = YamlIndex::build(yaml).expect("builds");
            let doc = first_doc_cursor(&index, yaml);
            // Only sequence documents have elements to compare.
            let YamlValue::Sequence(elements) = doc.value() else {
                continue;
            };

            let mut via_uncons = Vec::new();
            let mut rest = elements;
            while let Some((value, next)) = rest.uncons() {
                via_uncons.push(write_yaml_value_as_json_string(value));
                rest = next;
            }

            let mut via_uncons_cursor = Vec::new();
            let mut rest = elements;
            while let Some((cursor, next)) = rest.uncons_cursor() {
                via_uncons_cursor.push(cursor.to_json());
                rest = next;
            }

            let via_get: Vec<String> = (0..via_uncons.len())
                .map(|i| {
                    write_yaml_value_as_json_string(
                        elements.get(i).expect("element index in range"),
                    )
                })
                .collect();

            let what = String::from_utf8_lossy(yaml);
            assert_eq!(
                via_uncons, via_uncons_cursor,
                "uncons vs uncons_cursor: {what}"
            );
            assert_eq!(via_uncons, via_get, "uncons vs get: {what}");
            assert!(
                elements.get(via_uncons.len()).is_none(),
                "get past the end returned a value for {what}"
            );
        }
    }

    /// A plain scalar that begins `- ` cannot be re-emitted bare: written under a
    /// `- ` item marker it would read back as a nested sequence. Before #332 no
    /// unquoted scalar could start that way, so this is a new obligation. The
    /// `Unquoted` branch's `starts_seq_entry` check is unconditional (not
    /// gated on "currently a block sequence item"), so it still quotes here
    /// even though these three sources are flow-style and — since #707 fixed
    /// flow-style preservation — now correctly stay flow on output instead of
    /// being forced to block. There is no block-style source spelling that
    /// parses to this same `Unquoted` value: `- - x` parses as a nested
    /// sequence, not a scalar, so the block-sequence-item collision this
    /// guards against is presently only reachable via flow-style sources.
    #[test]
    fn test_dash_space_plain_scalar_is_quoted_in_yaml_output() {
        for (yaml, expected) in [
            (&b"[- x]\n"[..], "[\"- x\"]"),
            (&b"{a: - x}\n"[..], "{a: \"- x\"}"),
            (&b"[- x, 1]\n"[..], "[\"- x\", 1]"),
        ] {
            let index = YamlIndex::build(yaml).expect("builds");
            let root = index.root(yaml);
            let mut out = String::new();
            root.stream_yaml_document(&mut out, 2).expect("streams");
            let what = String::from_utf8_lossy(yaml);
            assert_eq!(out, expected, "for {what}");

            // And the emitted YAML must read back as the same value.
            let round_tripped = first_doc_json(out.as_bytes());
            assert_eq!(
                round_tripped,
                first_doc_json(yaml),
                "round-trip changed {what}"
            );
        }
    }

    /// Helper: render a `YamlValue` as JSON, matching `to_json`'s output.
    fn write_yaml_value_as_json_string<W: AsRef<[u64]>>(value: YamlValue<'_, W>) -> String {
        let mut out = String::new();
        write_yaml_value_as_json(&mut out, value);
        out
    }

    // ========================================================================
    // Merge keys (`<<: *anchor`) — issue #171
    //
    // Expected outputs below were captured directly from mikefarah/yq v4.53.3
    // (the pinned golden oracle, `tests/data/yq-golden/YQ_VERSION`) run
    // without `--yaml-fix-merge-anchor-to-spec`, since that's the default
    // (unflagged) invocation `succinctly yq` is meant to match. That flag's
    // absence is exactly why "own key wins" isn't a fixed rule below — see
    // `resolve_merge_keys`'s doc comment for the actual rule.
    // ========================================================================

    #[test]
    fn test_merge_key_basic_expansion() {
        let yaml = b"default: &default\n  a: 1\nitem:\n  <<: *default\n  b: 2\n";
        assert_eq!(
            first_doc_json(yaml),
            r#"{"default":{"a":1},"item":{"a":1,"b":2}}"#
        );
    }

    #[test]
    fn test_merge_key_own_field_after_wins() {
        let yaml =
            b"default: &default\n  a: 1\n  b: original\nitem:\n  <<: *default\n  b: override\n";
        let index = YamlIndex::build(yaml).expect("builds");
        let YamlValue::Mapping(root) = first_doc(index.root(yaml)) else {
            panic!("root document must be a mapping");
        };
        let Some(YamlValue::Mapping(item)) = root.find("item") else {
            panic!("item must be a mapping");
        };
        assert_eq!(
            item.find("b").and_then(|v| v.as_str().map(Cow::into_owned)),
            Some("override".to_string())
        );
    }

    #[test]
    fn test_merge_key_own_field_before_loses_to_merge() {
        // Empirically surprising but matches yq's default (non-spec) behavior:
        // a merge key always overwrites in the order it's textually applied,
        // with no special priority for the mapping's own keys.
        let yaml =
            b"default: &default\n  a: 1\n  b: original\nitem:\n  b: own_first\n  <<: *default\n";
        assert_eq!(
            first_doc_json(yaml),
            r#"{"default":{"a":1,"b":"original"},"item":{"b":"original","a":1}}"#
        );
    }

    #[test]
    fn test_merge_key_multiple_sources_earlier_wins_value() {
        // `<<: [*a1, *a2]`: a1 is listed first so it must win the "common"
        // conflict, but a2's unique key still claims the earlier position.
        let yaml = b"a1: &a1\n  common: from_a1\n  x: 1\na2: &a2\n  common: from_a2\n  y: 2\nitem:\n  <<: [*a1, *a2]\n  z: 3\n";
        assert_eq!(
            first_doc_json(yaml),
            r#"{"a1":{"common":"from_a1","x":1},"a2":{"common":"from_a2","y":2},"item":{"common":"from_a1","y":2,"x":1,"z":3}}"#
        );
    }

    #[test]
    fn test_merge_key_duplicate_keys_last_wins() {
        // Two separate `<<:` entries (not a list): the later one wins on
        // conflict, same as ordinary duplicate mapping keys (#174).
        let yaml = b"a1: &a1\n  common: from_a1\n  x: 1\na2: &a2\n  common: from_a2\n  y: 2\nitem:\n  <<: *a1\n  <<: *a2\n  z: 3\n";
        assert_eq!(
            first_doc_json(yaml),
            r#"{"a1":{"common":"from_a1","x":1},"a2":{"common":"from_a2","y":2},"item":{"common":"from_a2","x":1,"y":2,"z":3}}"#
        );
    }

    #[test]
    fn test_merge_key_quoted_still_merges() {
        // yq merges even a quoted "<<" key — not spec-compliant (only a
        // plain unquoted scalar resolves to the merge type) but that's the
        // pinned oracle's actual behavior.
        let yaml = b"default: &default\n  a: 1\nitem:\n  \"<<\": *default\n  b: 2\n";
        assert_eq!(
            first_doc_json(yaml),
            r#"{"default":{"a":1},"item":{"a":1,"b":2}}"#
        );
    }

    #[test]
    fn test_merge_key_inline_mapping_value() {
        let yaml = b"item:\n  <<: {a: 1, b: 2}\n  c: 3\n";
        assert_eq!(first_doc_json(yaml), r#"{"item":{"a":1,"b":2,"c":3}}"#);
    }

    #[test]
    fn test_merge_key_invalid_value_is_silently_ignored() {
        for yaml in [
            &b"item:\n  <<:\n  b: 2\n"[..],   // null merge value
            &b"item:\n  <<: 5\n  b: 2\n"[..], // scalar merge value
        ] {
            assert_eq!(
                first_doc_json(yaml),
                r#"{"item":{"b":2}}"#,
                "for {:?}",
                String::from_utf8_lossy(yaml)
            );
        }
    }

    #[test]
    fn test_merge_key_source_own_merge_key_not_recursively_expanded() {
        // `mid` merges `base` but is itself merged verbatim into `item`: its
        // own literal (unexpanded) "<<" key is copied over, not recursively
        // resolved — matches `yq '.item'` queried directly (merge is one
        // level deep only).
        //
        // Known divergence: real yq's *whole-document* render (`yq -o json
        // '.'`) instead shows `item` fully resolved to `{"x":1,"y":2,"z":3}`
        // with no literal `<<` — an artifact of yq mutating the shared,
        // anchor-backed node in place the first time it visits `mid` during
        // top-to-bottom traversal, so a later alias copy inherits the
        // already-resolved form. That's traversal-order-dependent state
        // succinctly's semi-index deliberately doesn't replicate: every
        // lookup here is a pure, local read with no cross-node mutation, so
        // `.item` gives the same answer whether queried alone or as part of
        // `.`.
        let yaml =
            b"base: &base\n  x: 1\nmid: &mid\n  <<: *base\n  y: 2\nitem:\n  <<: *mid\n  z: 3\n";
        let index = YamlIndex::build(yaml).expect("builds");
        let YamlValue::Mapping(root) = first_doc(index.root(yaml)) else {
            panic!("root document must be a mapping");
        };

        let Some(item_value) = root.find("item") else {
            panic!("item not found");
        };
        assert_eq!(
            write_yaml_value_as_json_string(item_value),
            r#"{"<<":{"x":1},"y":2,"z":3}"#
        );

        // `mid` resolves its *own* merge key fine when read directly.
        let Some(mid_value) = root.find("mid") else {
            panic!("mid not found");
        };
        assert_eq!(
            write_yaml_value_as_json_string(mid_value),
            r#"{"x":1,"y":2}"#
        );
    }

    #[test]
    fn test_merge_key_inside_sequence_item() {
        let yaml = b"default: &default\n  a: 1\nitems:\n  - <<: *default\n    b: 2\n";
        assert_eq!(
            first_doc_json(yaml),
            r#"{"default":{"a":1},"items":[{"a":1,"b":2}]}"#
        );
    }

    #[test]
    fn test_merge_key_keys_builtin_reflects_merge() {
        let yaml = b"default: &default\n  a: 1\nitem:\n  <<: *default\n  b: 2\n";
        let index = YamlIndex::build(yaml).expect("builds");
        let YamlValue::Mapping(root) = first_doc(index.root(yaml)) else {
            panic!("root document must be a mapping");
        };
        let Some(YamlValue::Mapping(item)) = root.find("item") else {
            panic!("item must be a mapping");
        };
        assert_eq!(item.keys(), vec!["a".to_string(), "b".to_string()]);
        assert_eq!(item.len(), 2);
        assert!(!item.is_empty());
    }
    // =========================================================================
    // Inline block sequence as a mapping value (#325)
    // =========================================================================
    // `a: - x` is invalid YAML (test-suite case 5U3A: a block sequence may not
    // begin on the same line as its parent mapping key) and the opt-in strict
    // validator rejects it. The loader does minimal validation by design, so it
    // parses the obvious extension instead of silently discarding the content,
    // which is what it used to do -- `a: - x` read back as `{"a":null}`.

    // These reuse #332's `first_doc_json` above, which additionally asserts that the
    // buffered and streaming JSON writers agree, rather than adding a second helper.

    #[test]
    fn test_inline_seq_as_mapping_value_keeps_content() {
        // The #325 repro: `x` used to be silently dropped.
        assert_eq!(first_doc_json(b"a: - x\n"), r#"{"a":["x"]}"#);
        // Extra spaces before the `-` must not change the result.
        assert_eq!(first_doc_json(b"a:   - x\n"), r#"{"a":["x"]}"#);
        // A sibling entry after it still parses.
        assert_eq!(first_doc_json(b"a: - x\nb: 1\n"), r#"{"a":["x"],"b":1}"#);
        // No trailing newline.
        assert_eq!(first_doc_json(b"a: - x"), r#"{"a":["x"]}"#);
    }

    #[test]
    fn test_inline_seq_as_mapping_value_item_shapes() {
        // Nested inline sequence.
        assert_eq!(first_doc_json(b"a: - - x\n"), r#"{"a":[["x"]]}"#);
        // Compact mapping as the item.
        assert_eq!(first_doc_json(b"a: - k: v\n"), r#"{"a":[{"k":"v"}]}"#);
        // Bare `-` is an empty item, not the scalar "-": per YAML 1.2
        // `ns-plain-first` a `-` before whitespace or end-of-input is always the
        // sequence-entry indicator and never starts a plain scalar.
        assert_eq!(first_doc_json(b"a: -\n"), r#"{"a":[null]}"#);
        assert_eq!(first_doc_json(b"a: -"), r#"{"a":[null]}"#);
        assert_eq!(first_doc_json(b"a: -\r\n"), r#"{"a":[null]}"#);
    }

    #[test]
    fn test_inline_seq_continuation_line_joins_same_sequence() {
        // 5U3A. The sequence is registered at the actual column of the `-`
        // (5 here), so the next line's `-` at the same column is a sibling item
        // rather than a new nested sequence. Registering it at `indent + 2`
        // would produce {"key":[["a"],["b"]]} or worse.
        assert_eq!(
            first_doc_json(b"key: - a\n     - b\n"),
            r#"{"key":["a","b"]}"#
        );
        assert_eq!(
            first_doc_json(b"key: - a\n     - b\n     - c\n"),
            r#"{"key":["a","b","c"]}"#
        );
    }

    #[test]
    fn test_inline_seq_as_explicit_value() {
        // `parse_value`'s dash arm used to be a no-op, so the nested item's
        // content was dropped: `: - - x` read back as {"k":[null]}.
        assert_eq!(first_doc_json(b"? k\n: - - x\n"), r#"{"k":[["x"]]}"#);
        assert_eq!(first_doc_json(b"? k\n: - k2: v\n"), r#"{"k":[{"k2":"v"}]}"#);
        // Single-level explicit value was already correct; keep it that way.
        assert_eq!(first_doc_json(b"? k\n: - x\n"), r#"{"k":["x"]}"#);
    }

    #[test]
    fn test_bare_dash_as_explicit_key_is_an_empty_item() {
        // The last shape #325's acceptance criteria name, and the only one that
        // exercises the *key* side. `? -` is a non-scalar key — a one-item
        // sequence whose item is empty — reached through `parse_explicit_key`,
        // which delegates to the same `parse_sequence_item` every arm this issue
        // touched now calls. So this is the regression guard on that shared
        // helper from the one direction the value-side tests above cannot reach.
        //
        // The key renders as `""` because neither succinctly nor `yq` has a JSON
        // spelling for a non-scalar key, so both collapse it (#172); the point
        // here is that the *entry survives at all*, which is what the sibling
        // `- ` shapes used to lose. Expectations are mikefarah/yq v4.53.3, which
        // agrees byte-for-byte on every case below.
        assert_eq!(first_doc_json(b"? -\n: v\n"), r#"{"":"v"}"#);
        // No trailing newline: the `-` is the last byte of the key's line, so
        // `is_seq_indicator_next`'s end-of-input arm is what keeps it an
        // indicator rather than a scalar.
        assert_eq!(first_doc_json(b"? -\n: v"), r#"{"":"v"}"#);
        // No value at all — the key still stands, with a null value.
        assert_eq!(first_doc_json(b"? -\n"), r#"{"":null}"#);
        // Bare dash on *both* sides: an empty item as the key, and the value is
        // the `{"a":[null]}` shape from `test_inline_seq_as_mapping_value_*`.
        assert_eq!(first_doc_json(b"? -\n: -\n"), r#"{"":[null]}"#);
        // All three YAML 1.2 §5.4 line breaks reach the same reading.
        assert_eq!(first_doc_json(b"? -\r\n: v\r\n"), r#"{"":"v"}"#);
    }

    #[test]
    fn test_dash_in_mapping_value_non_sequence_cases_unchanged() {
        // A `-` NOT followed by whitespace starts a plain scalar as usual.
        assert_eq!(first_doc_json(b"a: -1\n"), r#"{"a":-1}"#);
        assert_eq!(first_doc_json(b"a: -x\n"), r#"{"a":"-x"}"#);
        // Quoting forces the scalar reading.
        assert_eq!(first_doc_json(b"a: \"- x\"\n"), r#"{"a":"- x"}"#);
        assert_eq!(first_doc_json(b"a: '- x'\n"), r#"{"a":"- x"}"#);
        // In flow context the byte after `-` is `}`, so it stays a scalar.
        assert_eq!(first_doc_json(b"{a: -}\n"), r#"{"a":"-"}"#);
    }

    #[test]
    fn test_double_anchor_before_nested_dash_keeps_content() {
        // The one shape that actually reaches `parse_value`'s dash arm. Every other
        // caller either dispatches only on `{`/`[` or intercepts the `-` first; it
        // takes an anchor between the two dashes to slip past, and *two* anchors at
        // that, because `parse_value`'s prologue consumes one before dispatching.
        // With that arm still a no-op the item's content was silently dropped.
        assert_eq!(first_doc_json(b"- &a &b - x\n"), r#"[["x"]]"#);
        // One anchor is intercepted upstream and never reaches the arm; pinned so a
        // future change to the prologue cannot silently move which path handles it.
        assert_eq!(first_doc_json(b"- &a - x\n"), r#"[["x"]]"#);
    }

    #[test]
    fn test_valid_block_sequence_spellings_still_correct() {
        // The two legal spellings must be unaffected by the lenient inline path.
        assert_eq!(first_doc_json(b"a:\n  - x\n"), r#"{"a":["x"]}"#);
        assert_eq!(first_doc_json(b"a:\n- x\n"), r#"{"a":["x"]}"#);
        assert_eq!(first_doc_json(b"a:\n  - x\n  - y\n"), r#"{"a":["x","y"]}"#);
        // Empty first item on its own line is still null.
        assert_eq!(first_doc_json(b"a:\n  -\n  - y\n"), r#"{"a":[null,"y"]}"#);
        // Top-level nested sequences.
        assert_eq!(first_doc_json(b"- - x\n- y\n"), r#"[["x"],"y"]"#);
    }

    #[test]
    fn test_inline_seq_as_compact_mapping_value_keeps_content() {
        // `parse_compact_mapping_entry`'s own inline-value branch didn't get the
        // #325 dash-sequence dispatch that its siblings (`parse_mapping_entry`,
        // `parse_explicit_value`, `parse_value`) did, so a compact mapping's own
        // same-line dash value fell through to `parse_inline_value` and read back
        // as the scalar `"- x"` instead of the sequence `["x"]` — the same bug
        // #325 fixed, one level deeper.
        assert_eq!(first_doc_json(b"- a: - x\n"), r#"[{"a":["x"]}]"#);
        // Bare dash is an empty item, matching the top-level case.
        assert_eq!(first_doc_json(b"- a: -\n"), r#"[{"a":[null]}]"#);
        // Nested inline sequence and compact mapping as the item.
        assert_eq!(first_doc_json(b"- a: - - x\n"), r#"[{"a":[["x"]]}]"#);
        assert_eq!(first_doc_json(b"- a: - k: v\n"), r#"[{"a":[{"k":"v"}]}]"#);
        // A continuation line at the same column joins the same sequence.
        assert_eq!(
            first_doc_json(b"- a: - x\n     - y\n"),
            r#"[{"a":["x","y"]}]"#
        );
        // A sibling entry in the same compact mapping still parses.
        assert_eq!(
            first_doc_json(b"- a: - x\n  b: 2\n"),
            r#"[{"a":["x"],"b":2}]"#
        );
        // A `-` not followed by whitespace, and an alias, are unaffected.
        assert_eq!(first_doc_json(b"- a: -1\n"), r#"[{"a":-1}]"#);
        assert_eq!(first_doc_json(b"- a: -x\n"), r#"[{"a":"-x"}]"#);
        assert_eq!(
            first_doc_json(b"- b: &anc 1\n- a: *anc\n"),
            r#"[{"b":1},{"a":1}]"#
        );
    }
}
