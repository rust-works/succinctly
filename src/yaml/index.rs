//! YAML index structures for efficient navigation.
//!
//! Holds the semi-index (IB, BP, TY) and provides rank/select operations.

#[cfg(not(test))]
use alloc::{string::String, vec::Vec};

#[cfg(not(test))]
use alloc::collections::BTreeMap;

#[cfg(test)]
use std::collections::BTreeMap;

use core::cell::OnceCell;

use crate::trees::{BalancedParens, WithCsPoppy};
use crate::util::broadword::select_in_word;

use super::advance_positions::{build_cumulative_rank, OpenPositions};
use super::end_positions::EndPositions;
use super::error::YamlError;
use super::light::YamlCursor;
use super::parser::build_semi_index;
use super::starts_seq_entry;

/// Index structures for navigating YAML.
///
/// The type parameter `W` controls how the underlying data is stored:
/// - `Vec<u64>` for owned data (built from YAML text)
/// - `&[u64]` for borrowed data (e.g., from mmap)
///
/// Unlike JSON, YAML uses [`WithCsPoppy`] for the balanced parentheses structure
/// because YAML has more BP opens than IB bits (containers don't have IB bits),
/// requiring efficient select1 queries for offset-to-BP lookups.
#[derive(Clone, Debug)]
pub struct YamlIndex<W = Vec<u64>> {
    /// Interest bits - marks positions of structural elements
    ib: W,
    /// Number of valid bits in IB
    ib_len: usize,
    /// Cumulative popcount for IB. O(1) rank via single array lookup.
    ib_rank: Vec<u32>,
    /// Balanced parentheses - encodes the YAML structure as a tree.
    /// Uses WithCsPoppy for O(1) select1 queries needed by find_bp_at_text_pos.
    bp: BalancedParens<W, WithCsPoppy>,
    /// Type bits - 0 = mapping, 1 = sequence at each container position
    ty: W,
    /// Number of valid bits in TY
    #[allow(dead_code)] // STYLE-0005: index metadata field retained for parity with ib_len
    ty_len: usize,
    /// Memory-efficient BP open index to text position mapping.
    /// Uses Advance Index encoding for ~1.5× compression when positions are monotonic,
    /// otherwise falls back to `Vec<u32>` for non-monotonic cases (explicit keys, etc.).
    open_positions: OpenPositions,
    /// End positions for scalars using memory-efficient encoding.
    /// Uses Advance Index encoding for ~5× compression when end positions are monotonic,
    /// otherwise falls back to `Vec<u32>`.
    bp_to_text_end: EndPositions,
    /// Container markers - 1 if BP position has a TY entry (is a mapping or sequence)
    containers: W,
    /// Cumulative popcount for containers. O(1) rank via single array lookup.
    containers_rank: Vec<u32>,
    /// Anchor definitions: anchor name → BP position of the anchored value
    anchors: BTreeMap<String, usize>,
    /// Reverse anchor mapping: BP position → anchor name (for metadata access)
    bp_to_anchor: BTreeMap<usize, String>,
    /// Alias references: BP position of alias → target BP position (resolved at parse time)
    aliases: BTreeMap<usize, usize>,
    /// Explicit source tags: BP position → raw tag text (`"!!str"`, `"!custom"`, etc.).
    ///
    /// Unlike anchors, a tag has no forward name→position map: nothing
    /// resolves to a tag by reference the way an alias resolves to an
    /// anchor, so this is a single side table, not three (#224).
    tags: BTreeMap<usize, String>,
    /// Trailing same-line comments: BP position of the owning node → `(start, end)`
    /// byte range of the raw `#...` comment text (issue #710).
    line_comments: BTreeMap<usize, (u32, u32)>,
    /// Line starts for line/column lookup (built lazily on first use).
    /// Only needed by `to_line_column()` and `to_offset()` (used by the
    /// `yq-locate` CLI and the `at_position` jq builtin).
    lines: OnceCell<crate::text::LineIndex>,
    /// Set via [`Self::mark_json_sourced`] when this index's text is
    /// JSON, not genuine YAML, run through this parser only because JSON
    /// is a syntactic subset of YAML's flow grammar (#996).
    ///
    /// Read by [`YamlCursor`]'s M2 streaming formatters
    /// (`stream_resolved_scalar_as_json`/`stream_yaml_string_value`) to
    /// switch a float scalar's rendering from YAML's literal-spelling
    /// preservation to real yq's JSON-input convention (always
    /// re-serialize through `f64`, matching [`format_float_yq`] /
    /// bare-`Display` -- never preserve `1.50`'s trailing zero or `1e2`'s
    /// exponent notation). Carried on the index itself (read via
    /// `YamlCursor`'s existing `&YamlIndex` reference) rather than as a
    /// parameter threaded through every streaming call site, since the
    /// M2 formatters recurse through the entire `Expr`-independent value
    /// tree and back through `DocumentCursor`'s trait methods (shared
    /// with the unrelated `JsonCursor` implementor) -- a field on the one
    /// struct already in scope everywhere it's needed avoids a much
    /// larger, cross-crate signature change for the same effect.
    canonicalize_numbers: bool,
}

/// Build cumulative popcount index for IB.
#[inline]
fn build_ib_rank(words: &[u64]) -> Vec<u32> {
    build_cumulative_rank(words)
}

/// Build cumulative popcount index for containers.
#[inline]
fn build_containers_rank(words: &[u64]) -> Vec<u32> {
    build_cumulative_rank(words)
}

impl YamlIndex<Vec<u64>> {
    /// Build a YAML index from YAML text.
    ///
    /// This parses the YAML to build the interest bits (IB), balanced
    /// parentheses (BP), and type bits (TY) index structures.
    ///
    /// The newline index for line/column lookup is built lazily on first
    /// call to `to_line_column()` or `to_offset()`.
    ///
    /// # Errors
    ///
    /// Returns [`YamlError::InputTooLarge`] for inputs over `u32::MAX` bytes
    /// (just under 4 GiB): the semi-index stores text positions as `u32`
    /// (#188). Other variants report malformed YAML.
    pub fn build(yaml: &[u8]) -> Result<Self, YamlError> {
        let semi = build_semi_index(yaml)?;

        let ib_len = yaml.len();
        let ib_rank = build_ib_rank(&semi.ib);
        let containers_rank = build_containers_rank(&semi.containers);

        // Build reverse anchor mapping (bp_pos → anchor_name)
        let bp_to_anchor: BTreeMap<usize, String> = semi
            .anchors
            .iter()
            .map(|(name, &bp_pos)| (bp_pos, name.clone()))
            .collect();

        // Convert bp_to_text to compact storage when positions are monotonic
        let open_positions = OpenPositions::build(&semi.bp_to_text, ib_len);

        let index = Self {
            ib: semi.ib,
            ib_len,
            ib_rank,
            bp: BalancedParens::new_with_cspoppy(semi.bp, semi.bp_len),
            ty: semi.ty,
            ty_len: semi.ty_len,
            open_positions,
            bp_to_text_end: EndPositions::build(&semi.bp_to_text_end, ib_len),
            containers: semi.containers,
            containers_rank,
            anchors: semi.anchors,
            bp_to_anchor,
            aliases: semi.aliases,
            tags: semi.tags,
            line_comments: semi.line_comments,
            lines: OnceCell::new(),
            canonicalize_numbers: false,
        };
        index.validate_alias_acyclicity()?;
        Ok(index)
    }
}

impl<W: AsRef<[u64]>> YamlIndex<W> {
    /// Create a YAML index from pre-existing IB, BP, TY, and bp_to_text data.
    ///
    /// This is useful for loading serialized index data.
    #[allow(clippy::too_many_arguments)] // STYLE-0004: constructor threads each index array; a builder would hide the 1:1 field mapping
    pub fn from_parts(
        ib: W,
        ib_len: usize,
        bp: W,
        bp_len: usize,
        ty: W,
        ty_len: usize,
        bp_to_text: Vec<u32>,
        bp_to_text_end: Vec<u32>,
        containers: W,
        anchors: BTreeMap<String, usize>,
        aliases: BTreeMap<usize, usize>,
        tags: BTreeMap<usize, String>,
    ) -> Self {
        let ib_rank = build_ib_rank(ib.as_ref());
        let containers_rank = build_containers_rank(containers.as_ref());

        // Build reverse anchor mapping
        let bp_to_anchor: BTreeMap<usize, String> = anchors
            .iter()
            .map(|(name, &bp_pos)| (bp_pos, name.clone()))
            .collect();

        // Convert bp_to_text to compact storage when positions are monotonic
        let open_positions = OpenPositions::build(&bp_to_text, ib_len);

        Self {
            ib,
            ib_len,
            ib_rank,
            bp: BalancedParens::from_words_with_cspoppy(bp, bp_len),
            ty,
            ty_len,
            open_positions,
            bp_to_text_end: EndPositions::build(&bp_to_text_end, ib_len),
            containers,
            containers_rank,
            anchors,
            bp_to_anchor,
            aliases,
            tags,
            // `from_parts` predates line-comment tracking and has no caller
            // in this codebase today; a future caller that needs it can be
            // given an explicit parameter then.
            line_comments: BTreeMap::new(),
            lines: OnceCell::new(),
            canonicalize_numbers: false,
        }
    }

    /// Get the text byte offset for a BP position.
    ///
    /// The BP position must be at an open parenthesis (1-bit).
    /// Returns `None` if the position is invalid.
    #[inline]
    pub fn bp_to_text_pos(&self, bp_pos: usize) -> Option<usize> {
        // We need the index of this open in bp_to_text
        // That's the count of 1-bits in BP before bp_pos (inclusive)
        // which is rank1(bp_pos + 1) - 1 if the bit at bp_pos is 1
        // Actually it's simpler: rank1(bp_pos) gives count before,
        // so the index is rank1(bp_pos) if the bit is 1
        let open_idx = self.bp.rank1(bp_pos);
        self.open_positions.get(open_idx).map(|pos| pos as usize)
    }

    /// Get the text end byte offset for a BP position.
    ///
    /// For scalars, returns the end position. For nodes the parser recorded no end for
    /// — containers, nulls, sequence-item wrappers — the result depends on the storage
    /// variant: `None`, or an *earlier* node's end. Never a later node's, so
    /// `end > bp_to_text_pos(bp_pos)` is a sound test for "this node has an extent of its
    /// own" (see the `EndPositions::get` contract in `yaml::end_positions`, and #332).
    #[inline]
    pub fn bp_to_text_end_pos(&self, bp_pos: usize) -> Option<usize> {
        let open_idx = self.bp.rank1(bp_pos);
        self.bp_to_text_end.get(open_idx)
    }

    /// Compute the open index for a BP position (rank1).
    ///
    /// This is the index into the open_positions and end_positions arrays.
    /// Use with `text_pos_by_open_idx` and `text_end_pos_by_open_idx` to
    /// avoid redundant rank1 computation when both positions are needed.
    #[inline]
    pub fn bp_to_open_idx(&self, bp_pos: usize) -> usize {
        self.bp.rank1(bp_pos)
    }

    /// Get the text byte offset given a precomputed open index.
    #[inline]
    pub fn text_pos_by_open_idx(&self, open_idx: usize) -> Option<usize> {
        self.open_positions.get(open_idx).map(|pos| pos as usize)
    }

    /// Get the text end byte offset given a precomputed open index.
    #[inline]
    pub fn text_end_pos_by_open_idx(&self, open_idx: usize) -> Option<usize> {
        self.bp_to_text_end.get(open_idx)
    }

    /// Marks this index's text as JSON-sourced (#996) -- see the private
    /// `canonicalize_numbers` field's own doc comment for what this
    /// changes. Callers building an index over JSON bytes (JSON is a
    /// syntactic subset of YAML's flow grammar, so `YamlIndex::build`
    /// parses it fine) call this once, right after `build`, before
    /// streaming through any [`YamlCursor`] derived from it.
    pub fn mark_json_sourced(&mut self) {
        self.canonicalize_numbers = true;
    }

    /// Whether [`Self::mark_json_sourced`] was called on this index.
    #[inline]
    pub(crate) fn canonicalize_numbers(&self) -> bool {
        self.canonicalize_numbers
    }

    /// Get a reference to the interest bits words.
    #[inline]
    pub fn ib(&self) -> &[u64] {
        self.ib.as_ref()
    }

    /// Get the number of valid bits in IB.
    #[inline]
    pub fn ib_len(&self) -> usize {
        self.ib_len
    }

    /// Get a reference to the balanced parentheses.
    #[inline]
    pub fn bp(&self) -> &BalancedParens<W, WithCsPoppy> {
        &self.bp
    }

    /// Get a reference to the type bits.
    #[inline]
    pub fn ty(&self) -> &[u64] {
        self.ty.as_ref()
    }

    /// Get the number of valid bits in TY.
    #[inline]
    pub fn ty_len(&self) -> usize {
        self.ty_len
    }

    /// Create a cursor at the root of the YAML document.
    #[inline]
    pub fn root<'a>(&'a self, text: &'a [u8]) -> YamlCursor<'a, W> {
        YamlCursor::new(self, text, 0)
    }

    /// Check if the container at the given TY index is a sequence.
    ///
    /// Returns `true` for sequence, `false` for mapping.
    #[inline]
    pub fn is_sequence_at(&self, ty_idx: usize) -> bool {
        if ty_idx >= self.ty_len {
            return false;
        }
        let word_idx = ty_idx / 64;
        let bit_idx = ty_idx % 64;
        let ty_words = self.ty.as_ref();
        if word_idx < ty_words.len() {
            (ty_words[word_idx] >> bit_idx) & 1 == 1
        } else {
            false
        }
    }

    /// Check if the container at the given BP position is a sequence.
    ///
    /// This converts the BP position to a TY index and checks the type bit.
    /// Only valid for BP positions that are container opens (1-bits).
    ///
    /// Returns `true` for sequence, `false` for mapping.
    #[inline]
    pub fn is_sequence_at_bp(&self, bp_pos: usize) -> bool {
        // The TY index equals the number of container opens before this position.
        // Use the containers bitvector to count how many BP opens before bp_pos
        // actually have TY entries (are mappings or sequences).
        let ty_idx = self.count_containers_before(bp_pos);
        self.is_sequence_at(ty_idx)
    }

    /// Check if a BP position is a container (has a TY entry).
    ///
    /// Containers are mappings and sequences. Other BP nodes (scalars, sequence items)
    /// don't have TY entries.
    #[inline]
    pub fn is_container(&self, bp_pos: usize) -> bool {
        let word_idx = bp_pos / 64;
        let bit_idx = bp_pos % 64;
        let container_words = self.containers.as_ref();
        if word_idx < container_words.len() {
            (container_words[word_idx] >> bit_idx) & 1 == 1
        } else {
            false
        }
    }

    /// Count containers at BP positions 0..bp_pos.
    ///
    /// This gives the TY index for a BP position that is a container.
    /// Uses a cumulative popcount index for O(1) lookup.
    #[inline]
    pub fn count_containers_before(&self, bp_pos: usize) -> usize {
        let container_words = self.containers.as_ref();
        if container_words.is_empty() {
            return 0;
        }

        let word_idx = bp_pos / 64;
        let bit_idx = bp_pos % 64;

        // Use cumulative rank for full words (single array lookup)
        let mut count = self.containers_rank[word_idx.min(container_words.len())] as usize;

        // Add partial word bits
        if word_idx < container_words.len() && bit_idx > 0 {
            let mask = (1u64 << bit_idx) - 1;
            count += (container_words[word_idx] & mask).count_ones() as usize;
        }

        count
    }

    /// Check if a BP position corresponds to an alias.
    #[inline]
    pub fn is_alias(&self, bp_pos: usize) -> bool {
        self.aliases.contains_key(&bp_pos)
    }

    /// Whether this document has any alias (`*name`) references at all.
    ///
    /// Used to skip alias-sensitive post-processing entirely for the
    /// overwhelmingly common case of a document with no aliases (#711).
    #[inline]
    pub fn has_aliases(&self) -> bool {
        !self.aliases.is_empty()
    }

    /// Check if a BP position is a sequence item wrapper.
    ///
    /// Sequence items have BP open/close but no TY entry.
    /// They are wrapper nodes around the item's content.
    /// Derives this from text (starts with `- `) rather than storing a bitvector.
    ///
    /// Unlike [`YamlCursor::value`] this does *not* discriminate a childless wrapper from
    /// a plain scalar that merely begins `- ` (see #332). It does not need to: the only
    /// caller, `locate::path_to_bp`, asks about a node's *ancestors*, and such a scalar is
    /// always a BP leaf — a flow scalar is opened and closed with nothing pushed in
    /// between. Adding the end-position lookup here would be unreachable code paid for on
    /// every ancestor of every backwards walk.
    #[inline]
    pub fn is_seq_item(&self, text: &[u8], bp_pos: usize) -> bool {
        // Fast path: containers (mappings/sequences) are never seq_items
        if self.is_container(bp_pos) {
            return false;
        }

        // Get text position for this BP node
        let Some(text_pos) = self.bp_to_text_pos(bp_pos) else {
            return false;
        };

        starts_seq_entry(text, text_pos)
    }

    /// Debug helper to get open position for a given open index.
    #[cfg(test)]
    pub fn debug_open_position_at(&self, open_idx: usize) -> Option<u32> {
        self.open_positions.get(open_idx)
    }

    /// Get a reference to the OpenPositions structure.
    ///
    /// This maps BP open index to text byte offset using memory-efficient
    /// Advance Index encoding when positions are monotonic.
    #[inline]
    pub fn open_positions(&self) -> &OpenPositions {
        &self.open_positions
    }

    /// Find the BP position for the deepest node starting at a given text position.
    ///
    /// Uses the Advance Index to find the last open_idx with matching text position,
    /// then O(1) select1 to convert to BP position.
    /// Returns None if no match found.
    pub fn find_bp_at_text_pos(&self, text_pos: usize) -> Option<usize> {
        if self.open_positions.is_empty() {
            return None;
        }

        // Use AdvancePositions reverse lookup to find the last open at this position
        let last_open_idx = self.open_positions.find_last_open_at_text_pos(text_pos)?;

        // Convert open_idx to bp_pos using O(1) select1
        // select1(k) returns the position of the k-th 1-bit (0-indexed)
        self.bp.select1(last_open_idx)
    }

    /// Get the target BP position for an alias at the given BP position.
    ///
    /// Returns `None` if the position is not an alias.
    #[inline]
    pub fn get_alias_target(&self, bp_pos: usize) -> Option<usize> {
        self.aliases.get(&bp_pos).copied()
    }

    /// Get the anchor name that an alias at the given BP position references.
    ///
    /// Returns `None` if the position is not an alias.
    /// This is equivalent to yq's `alias` function.
    #[inline]
    pub fn get_alias_anchor_name(&self, bp_pos: usize) -> Option<&str> {
        let target_bp_pos = self.aliases.get(&bp_pos)?;
        self.bp_to_anchor
            .get(target_bp_pos)
            .map(alloc::string::String::as_str)
    }

    /// Get the BP position of an anchor by name.
    ///
    /// Returns `None` if the anchor is not defined.
    #[inline]
    pub fn get_anchor_bp_pos(&self, anchor_name: &str) -> Option<usize> {
        self.anchors.get(anchor_name).copied()
    }

    /// Get the anchor name for a BP position.
    ///
    /// Returns `None` if the position does not have an anchor.
    #[inline]
    pub fn get_anchor_name(&self, bp_pos: usize) -> Option<&str> {
        self.bp_to_anchor
            .get(&bp_pos)
            .map(alloc::string::String::as_str)
    }

    /// Get the explicit source tag for a BP position (`"!!str"`, `"!custom"`,
    /// `"!<tag:example.com,2000:foo>"`, etc.).
    ///
    /// Returns `None` if the node has no explicit tag. Distinct from
    /// [`YamlCursor::tag`](super::light::YamlCursor::tag), which returns an
    /// *inferred* type label derived from the value's shape rather than the
    /// source text.
    #[inline]
    pub fn get_tag(&self, bp_pos: usize) -> Option<&str> {
        self.tags.get(&bp_pos).map(alloc::string::String::as_str)
    }

    /// Get the raw trailing-comment byte range for a BP position, if the
    /// node at that position has a same-line comment (issue #710).
    ///
    /// The range starts at `#` and runs to end of line, exclusive of the
    /// line break — the caller slices it out of the original source text
    /// and strips the leading `#`/space at the point of use, mirroring how
    /// [`YamlCursor::style`](super::light::YamlCursor::style) reads from
    /// already-retained text rather than a stored string.
    #[inline]
    pub fn get_line_comment(&self, bp_pos: usize) -> Option<(u32, u32)> {
        self.line_comments.get(&bp_pos).copied()
    }

    /// Resolve an alias at the given BP position to a cursor pointing to
    /// the anchored value.
    ///
    /// Returns `None` if:
    /// - The position is not an alias
    /// - The referenced anchor is not defined
    pub fn resolve_alias<'a>(&'a self, bp_pos: usize, text: &'a [u8]) -> Option<YamlCursor<'a, W>> {
        let target_bp_pos = self.aliases.get(&bp_pos)?;
        Some(YamlCursor::new(self, text, *target_bp_pos))
    }

    /// Test-only: repoint the alias at `alias_bp_pos` at `target_bp_pos`,
    /// bypassing every check the parser applies.
    ///
    /// `YamlIndex::build` can no longer produce an alias whose target is
    /// itself an alias -- `&anchor *alias` was the only spelling that did,
    /// and #1374 rejects it at parse time -- so the multi-hop and
    /// depth-ceiling paths of `YamlCursor::resolve_alias_chain` and
    /// `resolve_alias_target_cursor` (kept as defense in depth against
    /// exactly such a hand-built index) are reachable only through an index
    /// rewired like this. Pointing an alias at *itself* yields an unbounded
    /// chain in one node, which is how the ceiling tests reach
    /// `MAX_ALIAS_CHAIN_DEPTH` without building 65,537 nodes.
    #[cfg(test)]
    pub(crate) fn rewire_alias_target(&mut self, alias_bp_pos: usize, target_bp_pos: usize) {
        debug_assert!(
            self.aliases.contains_key(&alias_bp_pos),
            "not an alias node"
        );
        self.aliases.insert(alias_bp_pos, target_bp_pos);
    }

    /// Reject documents whose aliases would make an anchored value contain
    /// itself (issue #153: unbounded recursion when materializing).
    ///
    /// Aliases resolve at parse time against anchors defined earlier in the
    /// text, so alias edges only point backward. Any materialization cycle
    /// must therefore include at least one alias whose target node is an
    /// ancestor of (or equal to) the alias node in the BP tree; rejecting
    /// exactly those edges rejects exactly the cyclic documents.
    ///
    /// The reported anchor name comes from `bp_to_anchor` and may be empty if
    /// the anchor was redefined after the cycle-forming definition; the
    /// offset still pinpoints the offending alias.
    fn validate_alias_acyclicity(&self) -> Result<(), YamlError> {
        for (&alias_bp, &target_bp) in &self.aliases {
            // Ancestor test: target opens at-or-before the alias and closes
            // after it. A target on a close bit (degenerate empty anchored
            // value) can never be an ancestor, so `find_close` returning
            // `None` for it must not count as a cycle.
            let is_cycle = target_bp == alias_bp
                || (target_bp < alias_bp
                    && self.bp.is_open(target_bp)
                    && self
                        .bp
                        .find_close(target_bp)
                        .is_some_and(|close| close > alias_bp));
            if is_cycle {
                return Err(YamlError::AliasCycle {
                    offset: self.bp_to_text_pos(alias_bp).unwrap_or(0),
                    name: self
                        .bp_to_anchor
                        .get(&target_bp)
                        .cloned()
                        .unwrap_or_default(),
                });
            }
        }
        Ok(())
    }

    /// Create a cursor at the given BP position.
    ///
    /// This is useful for navigating to a specific position in the index.
    #[inline]
    pub fn cursor_at<'a>(&'a self, bp_pos: usize, text: &'a [u8]) -> YamlCursor<'a, W> {
        YamlCursor::new(self, text, bp_pos)
    }

    /// Perform select1 with a hint for the starting word index.
    ///
    /// Uses exponential search (galloping) from the hint, which is optimal for
    /// sequential access patterns.
    #[inline]
    pub fn ib_select1_from(&self, k: usize, hint: usize) -> Option<usize> {
        let words = self.ib.as_ref();
        if words.is_empty() {
            return None;
        }

        let k32 = k as u32;
        let n = words.len();

        // #40: count `ib_rank` probes so this path's cost can be compared with
        // the word-scan sites. Starts at 1 for the `hint_rank` probe below.
        #[cfg(feature = "select-stats")]
        let mut probes = 1usize;

        // Clamp hint to valid range
        let hint = hint.min(n.saturating_sub(1));

        // Check if hint is already past k
        let hint_rank = self.ib_rank[hint + 1];
        let lo;
        let hi;

        if hint_rank <= k32 {
            // k is at or after hint - search forward with exponential expansion
            let mut bound = 1usize;
            let mut prev = hint;

            loop {
                #[cfg(feature = "select-stats")]
                {
                    probes += 1;
                }
                let next = (hint + bound).min(n);
                if next >= n || self.ib_rank[next + 1] > k32 {
                    lo = prev;
                    hi = next;
                    break;
                }
                prev = next;
                bound *= 2;
            }
        } else {
            // k is before hint - search backward with exponential expansion
            let mut bound = 1usize;
            let mut prev = hint;

            loop {
                #[cfg(feature = "select-stats")]
                {
                    probes += 1;
                }
                let next = hint.saturating_sub(bound);
                if next == 0 || self.ib_rank[next + 1] <= k32 {
                    lo = next;
                    hi = prev;
                    break;
                }
                prev = next;
                bound *= 2;
            }
        }

        // Binary search within [lo, hi]
        let mut lo = lo;
        let mut hi = hi;
        while lo < hi {
            #[cfg(feature = "select-stats")]
            {
                probes += 1;
            }
            let mid = lo + (hi - lo) / 2;
            if self.ib_rank[mid + 1] <= k32 {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }

        #[cfg(feature = "select-stats")]
        crate::util::select_stats::record(
            crate::util::select_stats::Site::YamlIbSelectFrom,
            probes,
        );

        if lo >= n {
            return None;
        }

        // Now lo is the word index, and ib_rank[lo] is count before this word
        let remaining = k - self.ib_rank[lo] as usize;
        let word = words[lo];
        let bit_pos = select_in_word(word, remaining as u32) as usize;
        let result = lo * 64 + bit_pos;

        if result < self.ib_len {
            Some(result)
        } else {
            None
        }
    }

    /// Perform select1 on the IB using pure binary search.
    #[inline]
    pub fn ib_select1(&self, k: usize) -> Option<usize> {
        let words = self.ib.as_ref();
        if words.is_empty() {
            return None;
        }

        let k32 = k as u32;
        let n = words.len();

        // Binary search over all words
        let mut lo = 0usize;
        let mut hi = n;
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            if self.ib_rank[mid + 1] <= k32 {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }

        if lo >= n {
            return None;
        }

        let remaining = k - self.ib_rank[lo] as usize;
        let word = words[lo];
        let bit_pos = select_in_word(word, remaining as u32) as usize;
        let result = lo * 64 + bit_pos;

        if result < self.ib_len {
            Some(result)
        } else {
            None
        }
    }

    /// Perform rank1 on the IB (count 1-bits in [0, pos)).
    pub fn ib_rank1(&self, pos: usize) -> usize {
        if pos == 0 {
            return 0;
        }

        let words = self.ib.as_ref();
        let word_idx = pos / 64;
        let bit_idx = pos % 64;

        // Use cumulative rank for full words (single array lookup)
        let mut count = self.ib_rank[word_idx.min(words.len())] as usize;

        // Add partial word
        if word_idx < words.len() && bit_idx > 0 {
            let mask = (1u64 << bit_idx) - 1;
            count += (words[word_idx] & mask).count_ones() as usize;
        }

        count
    }

    /// Convert a byte offset to (line, column) using the line index.
    ///
    /// Lines and columns are 1-based to match common editor conventions.
    /// Returns (1, offset+1) if the text has no line terminators.
    ///
    /// The line index is built lazily on first call from `text`.
    ///
    /// # Example
    ///
    /// ```
    /// use succinctly::yaml::YamlIndex;
    ///
    /// let yaml = b"name: Alice\nage: 30";
    /// let index = YamlIndex::build(yaml).unwrap();
    ///
    /// // Position 0 is line 1, column 1
    /// assert_eq!(index.to_line_column(0, yaml), (1, 1));
    ///
    /// // Position 12 is line 2, column 1 (right after the newline)
    /// assert_eq!(index.to_line_column(12, yaml), (2, 1));
    /// ```
    #[inline]
    pub fn to_line_column(&self, offset: usize, text: &[u8]) -> (usize, usize) {
        self.ensure_lines(text).to_line_column(offset)
    }

    /// Convert (line, column) to a byte offset using the line index.
    ///
    /// Lines and columns are 1-based to match common editor conventions.
    /// Returns None if the line/column is out of bounds.
    ///
    /// The line index is built lazily on first call from `text`.
    ///
    /// # Example
    ///
    /// ```
    /// use succinctly::yaml::YamlIndex;
    ///
    /// let yaml = b"name: Alice\nage: 30";
    /// let index = YamlIndex::build(yaml).unwrap();
    ///
    /// // Line 1, column 1 is offset 0
    /// assert_eq!(index.to_offset(1, 1, yaml), Some(0));
    ///
    /// // Line 2, column 1 is offset 12 (first char after newline)
    /// assert_eq!(index.to_offset(2, 1, yaml), Some(12));
    /// ```
    #[inline]
    pub fn to_offset(&self, line: usize, column: usize, text: &[u8]) -> Option<usize> {
        self.ensure_lines(text).to_offset(line, column)
    }

    /// Lazily build and cache the line index from the source text.
    ///
    /// The first caller's `text` wins for the lifetime of the index, so a
    /// later call with different text silently reads the first one's line
    /// map. The debug assertion catches the cheap half of that mistake.
    #[inline]
    fn ensure_lines(&self, text: &[u8]) -> &crate::text::LineIndex {
        let lines = self
            .lines
            .get_or_init(|| crate::text::LineIndex::build(text));

        debug_assert_eq!(
            lines.text_len(),
            text.len(),
            "line index was built from different text ({} bytes) than this call passed ({} bytes)",
            lines.text_len(),
            text.len()
        );

        lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_simple_mapping() {
        let yaml = b"name: Alice";
        let index = YamlIndex::build(yaml);
        assert!(index.is_ok());
    }

    #[test]
    fn test_build_simple_sequence() {
        let yaml = b"- item1\n- item2";
        let index = YamlIndex::build(yaml);
        assert!(index.is_ok());
    }

    // ------------------------------------------------------------------------
    // Trailing line comment capture (#710)
    // ------------------------------------------------------------------------

    /// Helper: build an index, navigate to `.<key>`'s value cursor via the
    /// virtual-document-root -> mapping -> field path every YAML document
    /// shares, and return its raw `#`-prefixed line comment (if any).
    fn field_line_comment_raw(yaml: &[u8], key: &str) -> Option<String> {
        use crate::yaml::light::YamlValue;

        let index = YamlIndex::build(yaml).expect("valid YAML");
        let root = index.root(yaml);
        let YamlValue::Sequence(docs) = root.value() else {
            panic!("root is always the virtual document sequence");
        };
        let (doc_cursor, _) = docs.uncons_cursor().expect("at least one document");
        let YamlValue::Mapping(fields) = doc_cursor.value() else {
            panic!("expected a mapping document");
        };
        for field in fields {
            if let YamlValue::String(k) = field.key() {
                if k.raw_bytes() == key.as_bytes() {
                    return field
                        .value_cursor()
                        .line_comment_raw()
                        .map(alloc::string::ToString::to_string);
                }
            }
        }
        None
    }

    #[test]
    fn test_line_comment_captured_on_plain_scalar() {
        let yaml = b"a: 1 # keep this\nb: 2\n";
        assert_eq!(
            field_line_comment_raw(yaml, "a").as_deref(),
            Some("# keep this")
        );
        assert_eq!(field_line_comment_raw(yaml, "b"), None);
    }

    /// Helper: like [`field_line_comment_raw`], but via the checked getter
    /// that distinguishes "no comment" from "comment present but not valid
    /// UTF-8" (issue #797).
    fn field_line_comment_checked(
        yaml: &[u8],
        key: &str,
    ) -> Result<Option<String>, core::str::Utf8Error> {
        use crate::yaml::light::YamlValue;

        let index = YamlIndex::build(yaml).expect("valid YAML");
        let root = index.root(yaml);
        let YamlValue::Sequence(docs) = root.value() else {
            panic!("root is always the virtual document sequence");
        };
        let (doc_cursor, _) = docs.uncons_cursor().expect("at least one document");
        let YamlValue::Mapping(fields) = doc_cursor.value() else {
            panic!("expected a mapping document");
        };
        for field in fields {
            if let YamlValue::String(k) = field.key() {
                if k.raw_bytes() == key.as_bytes() {
                    return field
                        .value_cursor()
                        .line_comment_checked()
                        .map(|opt| opt.map(alloc::string::ToString::to_string));
                }
            }
        }
        Ok(None)
    }

    #[test]
    fn test_line_comment_checked_distinguishes_absent_from_invalid_utf8_797() {
        // Absent: no comment at all.
        assert_eq!(field_line_comment_checked(b"a: 1\nb: 2\n", "a"), Ok(None));
        // Present and valid - stripped form, like `line_comment` (not the
        // raw `#`-prefixed form `line_comment_raw` returns).
        assert_eq!(
            field_line_comment_checked(b"a: 1 # keep this\n", "a"),
            Ok(Some("keep this".to_string()))
        );
        // Present but not valid UTF-8 - must be `Err`, not silently `Ok(None)`
        // like the tolerant `line_comment_raw` getter.
        assert!(field_line_comment_checked(b"a: 1 # caf\xE9\n", "a").is_err());
        // A key that isn't the first field - exercises the helper's
        // skip-and-continue loop, not just its immediate-match return.
        assert_eq!(
            field_line_comment_checked(b"a: 1\nb: 2 # keep this\n", "b"),
            Ok(Some("keep this".to_string()))
        );
        // A key that's absent entirely - the helper's own fallback, as
        // opposed to a present-but-commentless value (the "Absent" case
        // above).
        assert_eq!(field_line_comment_checked(b"a: 1\n", "z"), Ok(None));
        // A preceding field with a non-scalar (explicit complex) key -
        // `field.key()` returns `YamlValue::Sequence`, not `String`, so the
        // helper's `if let YamlValue::String(k) = field.key()` pattern
        // match itself fails and skips the field, distinct from the
        // scalar-key-that-just-doesn't-match case above.
        assert_eq!(
            field_line_comment_checked(b"? [1, 2]\n: 1\na: 2 # keep this\n", "a"),
            Ok(Some("keep this".to_string()))
        );
    }

    #[test]
    fn test_line_comment_captured_on_quoted_scalar() {
        let yaml = b"a: \"hello\" # quoted\n";
        assert_eq!(
            field_line_comment_raw(yaml, "a").as_deref(),
            Some("# quoted")
        );
    }

    #[test]
    fn test_line_comment_captured_on_flow_collection_close() {
        let yaml = b"a: [1, 2, 3] # flow comment\n";
        assert_eq!(
            field_line_comment_raw(yaml, "a").as_deref(),
            Some("# flow comment")
        );
    }

    #[test]
    fn test_line_comment_captured_on_flow_mapping_close() {
        let yaml = b"a: {b: 1} # flow mapping comment\n";
        assert_eq!(
            field_line_comment_raw(yaml, "a").as_deref(),
            Some("# flow mapping comment")
        );
    }

    #[test]
    fn test_line_comment_captured_on_block_scalar_header() {
        let yaml = b"a: | # header comment\n  line one\n  line two\n";
        assert_eq!(
            field_line_comment_raw(yaml, "a").as_deref(),
            Some("# header comment")
        );
    }

    #[test]
    fn test_line_comment_not_preceded_by_space_is_not_a_comment() {
        // '#' immediately after non-whitespace content is part of the scalar,
        // not a comment start (pre-existing rule, #437/#410).
        let yaml = b"a: 1#notacomment\n";
        assert_eq!(field_line_comment_raw(yaml, "a"), None);
    }

    #[test]
    fn test_line_comment_no_comment_returns_none() {
        let yaml = b"a: 1\nb: 2\n";
        assert_eq!(field_line_comment_raw(yaml, "a"), None);
        assert_eq!(field_line_comment_raw(yaml, "b"), None);
    }

    #[test]
    fn test_line_comment_field_not_found_returns_none() {
        // Every other case above queries a key that's actually present, so
        // `field_line_comment_raw`'s loop always returns from inside the
        // `for` — this is the only test that lets it run out of fields and
        // fall through to the `None` after the loop.
        let yaml = b"a: 1 # keep this\n";
        assert_eq!(field_line_comment_raw(yaml, "nonexistent"), None);
    }

    #[test]
    fn test_line_comment_on_sequence_items() {
        let yaml = b"a:\n  - 1 # first\n  - 2 # second\n";
        let index = YamlIndex::build(yaml).expect("valid YAML");
        let root = index.root(yaml);
        use crate::yaml::light::YamlValue;
        let YamlValue::Sequence(docs) = root.value() else {
            panic!("root is always the virtual document sequence");
        };
        let (doc_cursor, _) = docs.uncons_cursor().expect("at least one document");
        let YamlValue::Mapping(fields) = doc_cursor.value() else {
            panic!("expected a mapping document");
        };
        let (field, _) = fields.uncons().expect("field 'a'");
        let YamlValue::Sequence(elements) = field.value_cursor().value() else {
            panic!("expected a sequence value");
        };
        let (first, rest) = elements.uncons_cursor().expect("first element");
        assert_eq!(first.line_comment_raw(), Some("# first"));
        let (second, _) = rest.uncons_cursor().expect("second element");
        assert_eq!(second.line_comment_raw(), Some("# second"));
    }

    // ------------------------------------------------------------------------
    // Key-scoped trailing line comment capture (#765)
    // ------------------------------------------------------------------------

    /// Helper: the twin of [`field_line_comment_raw`], reading the *key*
    /// cursor's line comment instead of the value cursor's.
    fn field_key_line_comment_raw(yaml: &[u8], key: &str) -> Option<String> {
        use crate::yaml::light::YamlValue;

        let index = YamlIndex::build(yaml).expect("valid YAML");
        let root = index.root(yaml);
        let YamlValue::Sequence(docs) = root.value() else {
            panic!("root is always the virtual document sequence");
        };
        let (doc_cursor, _) = docs.uncons_cursor().expect("at least one document");
        let YamlValue::Mapping(fields) = doc_cursor.value() else {
            panic!("expected a mapping document");
        };
        for field in fields {
            if let YamlValue::String(k) = field.key() {
                if k.raw_bytes() == key.as_bytes() {
                    return field
                        .key_cursor()
                        .line_comment_raw()
                        .map(alloc::string::ToString::to_string);
                }
            }
        }
        None
    }

    #[test]
    fn test_key_line_comment_captured_when_value_deferred_to_nested_mapping_765() {
        let yaml = b"a: # comment on key\n  b: 1\n";
        assert_eq!(
            field_key_line_comment_raw(yaml, "a").as_deref(),
            Some("# comment on key")
        );
        // The value cursor (what `.a | line_comment` navigates to) must NOT
        // see it - it's scoped to the key, matching real yq's getters.
        assert_eq!(field_line_comment_raw(yaml, "a"), None);
    }

    #[test]
    fn test_key_line_comment_captured_when_value_deferred_to_nested_sequence_765() {
        let yaml = b"a: # comment on key\n  - 1\n  - 2\n";
        assert_eq!(
            field_key_line_comment_raw(yaml, "a").as_deref(),
            Some("# comment on key")
        );
        assert_eq!(field_line_comment_raw(yaml, "a"), None);
    }

    #[test]
    fn test_key_line_comment_none_when_absent_765() {
        let yaml = b"a:\n  b: 1\n";
        assert_eq!(field_key_line_comment_raw(yaml, "a"), None);
    }

    /// Helper: like [`field_key_line_comment_raw`], but for a field of the
    /// mapping that is itself the value of a block-sequence item's *first*
    /// element - the only mapping entry parsed by `parse_compact_mapping_entry`
    /// rather than `parse_mapping_entry` (issue #785).
    fn seq_item_field_key_line_comment_raw(yaml: &[u8], key: &str) -> Option<String> {
        use crate::yaml::light::YamlValue;

        let index = YamlIndex::build(yaml).expect("valid YAML");
        let root = index.root(yaml);
        let YamlValue::Sequence(docs) = root.value() else {
            panic!("root is always the virtual document sequence");
        };
        let (doc_cursor, _) = docs.uncons_cursor().expect("at least one document");
        let YamlValue::Sequence(items) = doc_cursor.value() else {
            panic!("expected a sequence document");
        };
        let (item_cursor, _) = items.uncons_cursor().expect("at least one item");
        let YamlValue::Mapping(fields) = item_cursor.value() else {
            panic!(
                "expected item to be a mapping, got {:?}",
                item_cursor.value()
            );
        };
        for field in fields {
            if let YamlValue::String(k) = field.key() {
                if k.raw_bytes() == key.as_bytes() {
                    return field
                        .key_cursor()
                        .line_comment_raw()
                        .map(alloc::string::ToString::to_string);
                }
            }
        }
        None
    }

    #[test]
    fn test_key_line_comment_captured_for_seq_item_first_field_785() {
        // The compact block-sequence form (`- key: value`, the item's
        // first field sharing the dash's own line) parses that first
        // field through a different function than an ordinary mapping
        // entry (`parse_compact_mapping_entry` vs `parse_mapping_entry`) -
        // a #765 regression this pins directly (issue #785).
        let yaml = b"- a: # comment on key\n    b: 1\n";
        assert_eq!(
            seq_item_field_key_line_comment_raw(yaml, "a").as_deref(),
            Some("# comment on key")
        );
    }

    #[test]
    fn test_key_line_comment_captured_for_seq_item_second_field_785() {
        // A non-first field of the same mapping already went through
        // `parse_mapping_entry` before #785 - included as a control case
        // confirming the first-field fix doesn't disturb it.
        let yaml = b"- x: 1\n  a: # comment on key\n    b: 2\n";
        assert_eq!(
            seq_item_field_key_line_comment_raw(yaml, "a").as_deref(),
            Some("# comment on key")
        );
    }

    #[test]
    fn test_key_line_comment_not_captured_for_same_line_value_765() {
        // A comment trailing a same-line value belongs to the value (#710),
        // not the key - the new #765 capture point is only reached when the
        // value is deferred to a following line.
        let yaml = b"a: 1 # keep this\nb: 2\n";
        assert_eq!(field_key_line_comment_raw(yaml, "a"), None);
        assert_eq!(
            field_line_comment_raw(yaml, "a").as_deref(),
            Some("# keep this")
        );
    }

    // ------------------------------------------------------------------------
    // Anchor targeting (#328)
    // ------------------------------------------------------------------------

    /// Every anchor must name a node.
    ///
    /// `parse_anchor` records `bp_pos` — the *next* BP bit — as the anchor's
    /// target, betting that the anchored node's open comes next. `bp_pos`
    /// counts closes as well as opens, so any parse path that emits a close
    /// first, or none at all, leaves the anchor pointing at a close bit or at a
    /// node that is not the anchored one. The reader then resolves aliases to
    /// an unrelated node and emits a well-formed but wrong document, which is
    /// exactly how #328 stayed invisible.
    ///
    /// Checking the invariant over the whole corpus catches that class on
    /// shapes no example test would think to write. This is a unit test rather
    /// than an integration one because `anchors` is private.
    #[test]
    fn test_every_anchor_targets_an_open_bit() {
        const CORPUS: &str = include_str!("../../tests/data/yaml-test-suite-2022-01-17.json");
        let cases: Vec<serde_json::Value> =
            serde_json::from_str(CORPUS).expect("corpus is valid JSON");

        let mut checked = 0usize;
        let mut anchors_seen = 0usize;
        let mut violations: Vec<String> = Vec::new();
        for case in &cases {
            let yaml = case["yaml"].as_str().expect("case has yaml");
            // Invalid documents are the reject harness's business, not ours.
            let Ok(index) = YamlIndex::build(yaml.as_bytes()) else {
                continue;
            };
            checked += 1;
            let id = case["id"].as_str().unwrap_or("<no id>");
            for (name, &bp_pos) in &index.anchors {
                anchors_seen += 1;
                if bp_pos >= index.bp.len() || !index.bp.is_open(bp_pos) {
                    violations.push(format!("{id}: anchor {name:?} at bp {bp_pos}"));
                }
            }
        }

        assert!(
            violations.is_empty(),
            "{} anchor(s) do not target a node open:\n  {}",
            violations.len(),
            violations.join("\n  ")
        );

        // Guard the guard: a corpus that stopped parsing, or an anchor map that
        // stopped being populated, would make the loop above vacuously true.
        assert!(checked > 300, "only {checked} cases parsed; corpus broken?");
        assert!(
            anchors_seen > 50,
            "only {anchors_seen} anchors seen; anchor recording broken?"
        );
    }

    /// Issue #328: the anchor on a sequence item names the item's collection
    /// value, so an alias to it resolves to that collection.
    #[test]
    fn test_anchored_sequence_item_targets_its_collection() {
        for (yaml, kind_is_seq) in [
            (&b"- &m\n    k: v\n- *m\n"[..], false),
            (&b"- &m\n    - a\n- *m\n"[..], true),
            (&b"- &m {k: v}\n- *m\n"[..], false),
            (&b"- &m [a]\n- *m\n"[..], true),
        ] {
            let index = YamlIndex::build(yaml).expect("must parse");
            let target = index.get_anchor_bp_pos("m").expect("anchor m recorded");
            assert!(
                index.bp.is_open(target),
                "anchor must target an open bit for {:?}",
                core::str::from_utf8(yaml).unwrap()
            );
            assert_eq!(
                index.is_sequence_at_bp(target),
                kind_is_seq,
                "anchor must target the collection itself for {:?}",
                core::str::from_utf8(yaml).unwrap()
            );
        }
    }

    // ------------------------------------------------------------------------
    // Anchor at the end of a compact mapping entry's line (#406)
    // ------------------------------------------------------------------------

    /// Issue #406: the anchor on a compact mapping entry whose value is on the
    /// next line names that value's collection.
    ///
    /// The sibling of `test_anchored_sequence_item_targets_its_collection` for
    /// the one block-context site that decided where the value was *before*
    /// consuming the anchor. It targeted an open bit even when it was wrong —
    /// the open of the collapsed plain scalar — so the corpus invariant above
    /// could not see this; only the node's *kind* distinguishes the two.
    #[test]
    fn test_compact_entry_trailing_anchor_targets_its_collection() {
        for (yaml, kind_is_seq) in [
            (&b"- k: &a\n    b: 1\n"[..], false),
            (&b"- k: &a\n    - 1\n"[..], true),
            // A block sequence may sit at the entry's own indent.
            (&b"- k: &a\n  - 1\n"[..], true),
            (&b"- k: &a\n    [1, 2]\n"[..], true),
        ] {
            let index = YamlIndex::build(yaml).expect("must parse");
            let target = index.get_anchor_bp_pos("a").expect("anchor a recorded");
            assert!(
                index.bp.is_open(target),
                "anchor must target an open bit for {:?}",
                core::str::from_utf8(yaml).unwrap()
            );
            assert!(
                index.is_container(target),
                "anchor must target the collection, not a collapsed scalar, for {:?}",
                core::str::from_utf8(yaml).unwrap()
            );
            assert_eq!(
                index.is_sequence_at_bp(target),
                kind_is_seq,
                "anchor must target the collection itself for {:?}",
                core::str::from_utf8(yaml).unwrap()
            );
        }
    }

    /// The other side of #406: with no value on the following lines the entry
    /// is null, and the anchor names the explicit empty node the null arms emit
    /// rather than dangling on a sibling's open or a close bit.
    #[test]
    fn test_compact_entry_trailing_anchor_on_null_targets_an_empty_node() {
        for yaml in [
            &b"- k: &a\n"[..],
            &b"- k: &a\n- b\n"[..],
            &b"- k: &a\n  j: 2\n"[..],
        ] {
            let index = YamlIndex::build(yaml).expect("must parse");
            let target = index.get_anchor_bp_pos("a").expect("anchor a recorded");
            assert!(
                index.bp.is_open(target),
                "anchor must target an open bit for {:?}",
                core::str::from_utf8(yaml).unwrap()
            );
            assert!(
                !index.is_container(target),
                "a null entry's anchor must not name a container for {:?}",
                core::str::from_utf8(yaml).unwrap()
            );
        }
    }

    // ------------------------------------------------------------------------
    // Open-position storage selection (#327)
    // ------------------------------------------------------------------------

    /// What actually selects the `OpenPositions::Dense` fallback.
    ///
    /// The fallback exists for non-monotonic open positions, and the docs on
    /// `AdvancePositions` attribute that to "explicit keys". Measured against
    /// the YAML test suite (6 of 362 parsed cases reach Dense), the trigger is
    /// narrower: a **valueless** explicit key — `? a` with no `: value` — that
    /// is followed by another mapping entry. An ordinary `? key` / `: value`
    /// pair stays monotonic and uses the compact encoding, as do block-scalar
    /// and sequence keys.
    ///
    /// This matters beyond the assertion: the benchmark suite's
    /// `explicit-keys` pattern emits valueless keys specifically so this
    /// representation is exercised end-to-end (#327). If the trigger changes,
    /// that pattern needs to change with it.
    #[test]
    fn test_valueless_explicit_key_selects_dense_open_positions() {
        for yaml in [
            &b"? a\n? b\nc: 1\n"[..],
            &b"? a\nc: 1\n"[..],
            &b"? a\n? b\n"[..],
            // #346: a same-line complex key is valueless too, so it reaches the
            // same fallback once another entry follows its null sentinel
            &b"? k: v\nj: u\n"[..],
        ] {
            let index = YamlIndex::build(yaml).expect("valueless explicit keys must parse");
            assert!(
                !index.open_positions.is_compact(),
                "a valueless explicit key followed by another entry should force \
                 the Dense fallback, in:\n{}",
                String::from_utf8_lossy(yaml)
            );
        }
    }

    #[test]
    fn test_monotonic_documents_use_compact_open_positions() {
        for yaml in [
            // Ordinary block mapping
            &b"key one: value one\nkey two: value two\n"[..],
            // Explicit key *with* a value: still monotonic
            &b"? a\n: 1\n? b\n: 2\n"[..],
            // A single valueless explicit key, with nothing after it
            &b"? a\n"[..],
            // Block-scalar and sequence keys
            &b"? |\n  block key\n: v\n"[..],
            &b"outer:\n  ? - a\n  : b\n"[..],
            // #346: a same-line complex key with nothing after it - the null
            // sentinel sorts last, so the opens stay monotonic
            &b"? k: v\n"[..],
            // ...and one that does take a value stays monotonic throughout
            &b"? k: v\n: w\n"[..],
        ] {
            let index = YamlIndex::build(yaml).expect("must parse");
            assert!(
                index.open_positions.is_compact(),
                "monotonic open positions should use the Advance Index encoding, in:\n{}",
                String::from_utf8_lossy(yaml)
            );
        }
    }

    // ------------------------------------------------------------------------
    // Alias cycle validation tests (#153)
    // ------------------------------------------------------------------------

    /// Assert that `yaml` is rejected with `AliasCycle` naming `expected_name`
    /// at the byte offset of `alias_text` within `yaml`.
    fn expect_alias_cycle(yaml: &str, expected_name: &str, alias_text: &str) {
        let expected_offset = yaml.find(alias_text).expect("alias text present in input");
        let Err(err) = YamlIndex::build(yaml.as_bytes()) else {
            panic!("cyclic alias must be rejected at build time");
        };
        match err {
            YamlError::AliasCycle { offset, name } => {
                assert_eq!(name, expected_name);
                assert_eq!(offset, expected_offset);
            }
            other => panic!("expected AliasCycle, got {other:?}"),
        }
    }

    #[test]
    fn test_build_rejects_issue_153_alias_cycle() {
        expect_alias_cycle("a: &anchor\n  self: *anchor", "anchor", "*anchor");
    }

    /// Assert that `yaml` is rejected with `PropertyOnAlias` naming
    /// `expected_name` at the byte offset of `alias_text` within `yaml`
    /// (#1374). `expect_err` + `assert_eq!` on the whole error value rather
    /// than `expect_alias_cycle`'s `let Err(..) else { panic!() }` / `other
    /// => panic!()` shape: those two arms can never execute in a passing
    /// test, so each caller written that way adds two permanently
    /// uncovered lines (the shape this helper replaced was flagged for
    /// exactly that).
    fn expect_property_on_alias(yaml: &str, expected_name: &str, alias_text: &str) {
        let expected_offset = yaml.find(alias_text).expect("alias text present in input");
        let err = YamlIndex::build(yaml.as_bytes())
            .expect_err("anchor-on-alias must be rejected at build time");
        assert_eq!(
            err,
            YamlError::PropertyOnAlias {
                offset: expected_offset,
                name: expected_name.to_string(),
            }
        );
    }

    /// `&x *x` is anchor-on-alias (#1374, invalid YAML -- an alias node
    /// carries no properties of its own) as well as a would-be self-cycle;
    /// the property check in `parse_alias` rejects it before the cycle
    /// checker (`validate_alias_acyclicity`) ever runs, so the error here is
    /// `PropertyOnAlias`, not `AliasCycle` — unlike this file's other
    /// `expect_alias_cycle` cases, which all cycle through a *nested*
    /// structure rather than decorating the alias itself.
    #[test]
    fn test_build_rejects_direct_self_alias() {
        expect_property_on_alias("a: &x *x", "x", "*x");
    }

    /// Same shape as `test_build_rejects_direct_self_alias` but with the
    /// property+alias at document-root level rather than as a mapping
    /// value, exercising `parse_block_node`'s own `Some(b'*')` dispatch
    /// arm (#1374) rather than the value-position one.
    #[test]
    fn test_build_rejects_document_root_self_alias() {
        expect_property_on_alias("&x *x", "x", "*x");
    }

    #[test]
    fn test_build_rejects_grandparent_alias_cycle() {
        expect_alias_cycle("a: &x\n  b:\n    c: *x", "x", "*x");
    }

    #[test]
    fn test_build_rejects_flow_alias_cycle() {
        expect_alias_cycle("a: &x {b: *x}", "x", "*x");
    }

    #[test]
    fn test_build_rejects_cycle_in_second_document() {
        expect_alias_cycle("ok: 1\n---\na: &x\n  b: *x", "x", "*x");
    }

    /// An anchor alone on the `---` line names the document's root node, which
    /// starts on the *next* line, so an alias to it from inside that node is a
    /// cycle like any other.
    ///
    /// These two were `UnknownAnchor` cases until #407. The `---` line had its
    /// own dispatch, which opened an empty node for the anchor to bind to and
    /// left the real root as a second document; #372 kept the anchor
    /// unrecorded rather than let an alias resolve to that placeholder and
    /// render as `null`. With the dispatch shared the anchor binds properly,
    /// and both now behave exactly as their `---`-less equivalents already did.
    /// (`yq` v4.53.3 still reports `unknown anchor 'x'` for the first and reads
    /// the second as the plain scalar `1 - *x`; it also renders the mapping
    /// form as `{"":1}`, so it is not the oracle for this shape.)
    #[test]
    fn test_build_rejects_cycle_through_anchor_alone_on_the_document_start_line() {
        expect_alias_cycle("--- &x\na: 1\nb: *x", "x", "*x");
        expect_alias_cycle("--- &x\n- 1\n- *x", "x", "*x");
    }

    #[test]
    fn test_build_allows_sibling_alias() {
        assert!(YamlIndex::build(b"a: &x 1\nb: *x").is_ok());
    }

    #[test]
    fn test_build_allows_alias_to_closed_nested_target() {
        assert!(YamlIndex::build(b"a:\n  b: &x 1\nc: *x").is_ok());
    }

    #[test]
    fn test_build_allows_merge_key_alias() {
        assert!(YamlIndex::build(b"base: &b\n  x: 1\nitem:\n  <<: *b\n  y: 2").is_ok());
    }

    #[test]
    fn test_build_allows_cross_document_alias() {
        assert!(YamlIndex::build(b"a: &x 1\n---\nb: *x").is_ok());
    }

    // ------------------------------------------------------------------------
    // Unknown-anchor rejection tests (#372)
    // ------------------------------------------------------------------------

    /// Assert that `yaml` is rejected with `UnknownAnchor` naming
    /// `expected_name` at the byte offset of `alias_text` within `yaml`.
    ///
    /// The offset assertion is what stops a site from reporting a plausible but
    /// unrelated position: every alias below must point at its own `*`.
    #[track_caller]
    fn expect_unknown_anchor(yaml: &str, expected_name: &str, alias_text: &str) {
        let expected_offset = yaml.find(alias_text).expect("alias text present in input");
        let Err(err) = YamlIndex::build(yaml.as_bytes()) else {
            panic!("alias to an unknown anchor must be rejected at build time, in: {yaml:?}");
        };
        match err {
            YamlError::UnknownAnchor { offset, name } => {
                assert_eq!(name, expected_name, "in: {yaml:?}");
                assert_eq!(offset, expected_offset, "in: {yaml:?}");
            }
            other => panic!("expected UnknownAnchor, got {other:?}, in: {yaml:?}"),
        }
    }

    /// Every position an alias can appear in must reject an anchor that is not
    /// in scope, and name it.
    ///
    /// #372: a lookup miss used to be dropped, leaving the node with nothing to
    /// resolve to — it rendered as `null`, or as an empty string in the four
    /// key positions. Aliases reach the anchor table through two sites
    /// (`parse_alias` and `record_key_alias`), so this is table-driven rather
    /// than one case per site: a new position that forgets to resolve fails
    /// here.
    #[test]
    fn test_build_rejects_alias_to_unknown_anchor_in_every_position() {
        // (input, anchor name, the alias text whose offset must be reported)
        let cases: &[(&str, &str, &str)] = &[
            // Values.
            ("bad: *nope", "nope", "*nope"),
            ("- k: *nope", "nope", "*nope"),
            ("a: [*nope]", "nope", "*nope"),
            ("a: {k: *nope}", "nope", "*nope"),
            ("- *nope", "nope", "*nope"),
            ("--- *nope", "nope", "*nope"),
            // Keys.
            ("*nope: v", "nope", "*nope"),
            ("- *nope: v", "nope", "*nope"),
            ("? *nope\n: v", "nope", "*nope"),
            // The flow-mapping key reached this rejection through
            // `parse_alias` before #405 routed it through `record_key_alias`
            // like the other three, so the offset it reports is the helper's
            // rather than `parse_alias`'s. Both are the `*`, and this pins that.
            ("{*nope: v}", "nope", "*nope"),
            // The flow-*sequence* implicit single-pair-mapping key didn't
            // reach this rejection at all before #409: the sequence loop
            // consumed a leading `*` as a standalone item before the pair
            // check ever ran, so `[*nope: v]` errored `expected ',' or ']'`
            // rather than naming the unknown anchor.
            ("[*nope: v]", "nope", "*nope"),
            // A forward reference is a miss like any other: YAML 1.2 §7.1
            // requires an alias to name a *previous* anchor, so the anchor
            // below is not in scope at the alias.
            ("a: *x\nb: &x 5", "x", "*x"),
            // The anchor exists but was never in scope for this alias.
            ("a: &x 1\nb: *y", "y", "*y"),
        ];
        for (yaml, name, alias_text) in cases {
            expect_unknown_anchor(yaml, name, alias_text);
        }
    }

    #[test]
    fn test_build_allows_empty_anchored_value_alias() {
        // `&x` anchors an empty value; the recorded anchor BP position may
        // land on a close bit or a later sibling node, never an ancestor of
        // the alias, so this must not be reported as a cycle.
        assert!(YamlIndex::build(b"a: &x\nb: *x").is_ok());
    }

    #[test]
    fn test_root_cursor() {
        let yaml = b"name: Alice";
        let index = YamlIndex::build(yaml).unwrap();
        let root = index.root(yaml);
        assert_eq!(root.bp_position(), 0);
    }

    #[test]
    fn test_is_seq_item_derived_from_text() {
        // Sequence-item wrappers are recovered from the leading `- ` in the text
        // rather than a stored bitvector.
        let yaml = b"- apple\n- banana\n";
        let index = YamlIndex::build(yaml).unwrap();

        // The two block-sequence item wrappers are detected as seq items.
        let detected = (0..index.bp().len())
            .filter(|&bp| index.is_seq_item(yaml, bp))
            .count();
        assert!(
            detected >= 2,
            "expected the block-sequence item wrappers to be detected, got {detected}"
        );

        // Fast path: containers (the document-root sequence at bp 0) are never
        // seq items, even though the text at their position may start with `-`.
        assert!(index.is_container(0));
        assert!(!index.is_seq_item(yaml, 0));

        // Defensive guard: a BP position with no text mapping (e.g. the final
        // closing paren) has `bp_to_text_pos() == None` and must return false
        // without indexing into the text.
        let last = index.bp().len() - 1;
        assert_eq!(index.bp_to_text_pos(last), None);
        assert!(!index.is_seq_item(yaml, last));
    }

    #[test]
    fn test_is_seq_item_ignores_dash_without_space() {
        // A scalar that merely starts with `-` (e.g. a negative number) is NOT a
        // sequence-item wrapper: the `-` must be followed by whitespace or EOF.
        // This exercises the non-matching arm of the `- ` check (see #106).
        let yaml = b"n: -5\n";
        let index = YamlIndex::build(yaml).unwrap();

        let detected = (0..index.bp().len())
            .filter(|&bp| index.is_seq_item(yaml, bp))
            .count();
        assert_eq!(
            detected, 0,
            "a `-N` scalar must not be detected as a sequence item"
        );
    }

    // ------------------------------------------------------------------------
    // Newline index tests
    // ------------------------------------------------------------------------

    #[test]
    fn test_newline_index_single_line() {
        let yaml = b"name: Alice";
        let index = YamlIndex::build(yaml).unwrap();

        // All positions on line 1
        assert_eq!(index.to_line_column(0, yaml), (1, 1)); // 'n'
        assert_eq!(index.to_line_column(5, yaml), (1, 6)); // ' '
        assert_eq!(index.to_line_column(10, yaml), (1, 11)); // 'e'

        // Reverse lookup
        assert_eq!(index.to_offset(1, 1, yaml), Some(0));
        assert_eq!(index.to_offset(1, 6, yaml), Some(5));
        assert_eq!(index.to_offset(2, 1, yaml), None); // No line 2
    }

    #[test]
    fn test_newline_index_multi_line_unix() {
        let yaml = b"name: Alice\nage: 30";
        let index = YamlIndex::build(yaml).unwrap();

        // Line 1: positions 0-10 (name: Alice)
        assert_eq!(index.to_line_column(0, yaml), (1, 1)); // 'n'
        assert_eq!(index.to_line_column(10, yaml), (1, 11)); // 'e'
        assert_eq!(index.to_line_column(11, yaml), (1, 12)); // '\n'

        // Line 2: positions 12-18 (age: 30)
        assert_eq!(index.to_line_column(12, yaml), (2, 1)); // 'a'
        assert_eq!(index.to_line_column(17, yaml), (2, 6)); // '3'
        assert_eq!(index.to_line_column(18, yaml), (2, 7)); // '0'

        // Reverse lookup
        assert_eq!(index.to_offset(1, 1, yaml), Some(0));
        assert_eq!(index.to_offset(2, 1, yaml), Some(12));
        assert_eq!(index.to_offset(2, 6, yaml), Some(17));
    }

    #[test]
    fn test_newline_index_sequence() {
        let yaml = b"- item1\n- item2\n- item3";
        let index = YamlIndex::build(yaml).unwrap();

        // Line 1: positions 0-6 (- item1)
        assert_eq!(index.to_line_column(0, yaml), (1, 1));
        assert_eq!(index.to_line_column(7, yaml), (1, 8)); // '\n'

        // Line 2: positions 8-14 (- item2)
        assert_eq!(index.to_line_column(8, yaml), (2, 1));

        // Line 3: positions 16-22 (- item3)
        assert_eq!(index.to_line_column(16, yaml), (3, 1));

        // Reverse lookup
        assert_eq!(index.to_offset(1, 1, yaml), Some(0));
        assert_eq!(index.to_offset(2, 1, yaml), Some(8));
        assert_eq!(index.to_offset(3, 1, yaml), Some(16));
    }

    #[test]
    fn test_newline_index_crlf() {
        let yaml = b"name: Alice\r\nage: 30";
        let index = YamlIndex::build(yaml).unwrap();

        // Line 1: positions 0-10 (name: Alice)
        assert_eq!(index.to_line_column(0, yaml), (1, 1));
        assert_eq!(index.to_line_column(10, yaml), (1, 11));

        // Line 2: positions 13-19 (age: 30) - after \r\n
        assert_eq!(index.to_line_column(13, yaml), (2, 1));
        assert_eq!(index.to_offset(2, 1, yaml), Some(13));
    }

    #[test]
    fn test_newline_index_classic_mac_cr() {
        let yaml = b"name: Alice\rage: 30";
        let index = YamlIndex::build(yaml).unwrap();

        // Line 2: position 12 (age: 30) - after \r
        assert_eq!(index.to_line_column(12, yaml), (2, 1));
        assert_eq!(index.to_offset(2, 1, yaml), Some(12));
    }

    #[test]
    fn test_newline_index_invalid_inputs() {
        let yaml = b"name: Alice\nage: 30";
        let index = YamlIndex::build(yaml).unwrap();

        assert_eq!(index.to_offset(0, 1, yaml), None); // line 0 invalid
        assert_eq!(index.to_offset(1, 0, yaml), None); // column 0 invalid
    }

    #[test]
    fn test_to_offset_rejects_positions_past_the_end() {
        // Before #228 this returned Some(offset) for offsets past the text,
        // unlike JsonIndex::to_offset. The shared LineIndex bounds-checks both.
        let yaml = b"name: Alice\nage: 30";
        let index = YamlIndex::build(yaml).unwrap();

        assert_eq!(index.to_offset(2, 7, yaml), Some(18), "last byte");
        assert_eq!(index.to_offset(2, 8, yaml), None, "one past the end");
        assert_eq!(index.to_offset(2, 999, yaml), None, "far past the end");
        assert_eq!(index.to_offset(3, 1, yaml), None, "no line 3");
    }

    #[test]
    fn test_newline_index_round_trip() {
        let yaml = b"users:\n  - name: Alice\n    age: 30\n  - name: Bob\n    age: 25";
        let index = YamlIndex::build(yaml).unwrap();

        // Test round-trip: offset -> line/column -> offset
        for offset in 0..yaml.len() {
            let (line, col) = index.to_line_column(offset, yaml);
            let result = index.to_offset(line, col, yaml);
            assert_eq!(
                result,
                Some(offset),
                "Round-trip failed for offset {offset}"
            );
        }
    }

    // ------------------------------------------------------------------------
    // from_parts (#224: tags round-trip)
    // ------------------------------------------------------------------------

    /// `from_parts` is the public constructor for loading pre-serialized index
    /// data (mirrors `SimpleJsonIndex::from_parts` in `json/simple_light.rs`).
    /// Unlike its JSON siblings it had no direct test at all - build a
    /// `YamlIndex` normally, tear it back down into the raw parts via
    /// `build_semi_index` (the same function `YamlIndex::build` itself calls),
    /// reconstruct through `from_parts`, and check the reconstructed index
    /// behaves identically. The `tags` map is this PR's new field, so the
    /// fixture carries an explicit tag (`!!str`) specifically to exercise it,
    /// not just the fields `from_parts` already had before #224.
    #[test]
    fn test_from_parts_round_trip_with_tags() {
        let yaml: &[u8] = b"a: !!str 1\n";

        let built = YamlIndex::build(yaml).expect("should parse");

        let semi = build_semi_index(yaml).expect("should parse");
        let reconstructed = YamlIndex::from_parts(
            semi.ib,
            semi.ib_len,
            semi.bp,
            semi.bp_len,
            semi.ty,
            semi.ty_len,
            semi.bp_to_text,
            semi.bp_to_text_end,
            semi.containers,
            semi.anchors,
            semi.aliases,
            semi.tags,
        );

        // Same document either way, and the tag did its job: `!!str` forces
        // the plain scalar `1` to resolve as the string "1", not the number 1.
        let expected_json = r#"[{"a":"1"}]"#;
        assert_eq!(built.root(yaml).to_json(), expected_json);
        assert_eq!(reconstructed.root(yaml).to_json(), expected_json);

        // The tag itself must survive the round trip too, not just its
        // effect on resolution - look it up by the tagged value's own BP
        // position on both indexes.
        let fields = match built.root(yaml).value() {
            crate::yaml::YamlValue::Sequence(mut docs) => match docs.next().expect("one doc") {
                crate::yaml::YamlValue::Mapping(fields) => fields,
                other => panic!("expected mapping, got {other:?}"),
            },
            other => panic!("expected root sequence, got {other:?}"),
        };
        let (field, _rest) = fields.uncons().expect("one field");
        let value_bp_pos = field.value_cursor().bp_position();

        assert_eq!(built.get_tag(value_bp_pos), Some("!!str"));
        assert_eq!(
            reconstructed.get_tag(value_bp_pos),
            Some("!!str"),
            "tags map must survive from_parts round trip"
        );
    }
}
