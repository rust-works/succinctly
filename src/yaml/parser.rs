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
use super::line_break::{is_line_break, line_break_len};
use super::simd;
use crate::text;

/// Node type in the YAML structure tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeType {
    /// Mapping (object-like): key-value pairs
    Mapping,
    /// Sequence (array-like): ordered list
    Sequence,
    /// Scalar value (string, number, etc.)
    #[allow(dead_code)] // STYLE-0005: parser style variant retained for completeness
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
    #[allow(dead_code)] // STYLE-0005: retained field (block style) for future use
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
    /// End positions for scalars. For each BP open, stores the end byte offset.
    /// For containers, stores 0 (containers don't have a text end position).
    pub bp_to_text_end: Vec<u32>,
    /// Container marker bits: 1 if this BP position has a TY entry (is a mapping or sequence).
    /// Used to compute correct TY index from BP position.
    pub containers: Vec<u64>,
    /// Number of valid bits in IB (= input length)
    #[allow(dead_code)] // STYLE-0005: index metadata field retained
    pub ib_len: usize,
    /// Number of valid bits in BP
    pub bp_len: usize,
    /// Number of valid bits in TY (= number of container opens)
    #[allow(dead_code)] // STYLE-0005: index metadata field retained
    pub ty_len: usize,
    /// Anchor definitions: anchor name → BP position of the anchored value
    pub anchors: BTreeMap<String, usize>,
    /// Alias references: BP position of alias → target BP position (resolved at parse time)
    pub aliases: BTreeMap<usize, usize>,
    /// Explicit source tags: BP position → raw tag text (see [`YamlIndex::get_tag`](super::index::YamlIndex::get_tag))
    pub tags: BTreeMap<usize, String>,
    /// Trailing same-line comments: BP position of the node the comment trails →
    /// `(start, end)` byte range of the raw comment text, starting at `#` and
    /// running to end of line (exclusive of the line break, inclusive of any
    /// trailing whitespace). Only line comments (issue #710) — standalone
    /// head/foot comments are not captured.
    pub line_comments: BTreeMap<usize, (u32, u32)>,
}

/// Maximum nesting depth for recursively-parsed constructs (flow collections
/// and inline `- - x` sequence-item chains). Bounds parser stack growth on
/// pathological input like `[`×20000, which otherwise aborts the process with
/// a stack overflow (#152). Real documents nest ~30-50 levels;
/// `tests/deep_nesting_valid_tests.rs` pins depth 100 as must-parse, so this
/// cap must stay above that.
const MAX_NESTING_DEPTH: usize = 128;

/// Parser state for the YAML-lite oracle.
///
/// `HAS_CR` says whether the input contains a carriage return anywhere.
/// [`build_semi_index`] answers that with one SIMD pass before parsing and picks
/// the matching monomorphization, so the LF-only specialization can drop every
/// `\r` arm the CRLF correctness fix added and keep the pre-#324 codegen (#340).
///
/// The gate is a *performance* knob, not a correctness one, in one direction
/// only. `HAS_CR == true` is always correct — it is exactly the #324 parser. It
/// is `HAS_CR == false` that carries the obligation, and the precheck discharges
/// it: no `\r` in the input means no CR arm can ever be reached, whatever the
/// context. So a site left un-gated is merely a missed optimization, while a
/// wrongly-`false` gate would corrupt the parse — which is why the flag is
/// derived from the bytes rather than from any parser-state heuristic.
struct Parser<'a, const HAS_CR: bool> {
    input: &'a [u8],
    pos: usize,

    // Index builders
    ib_words: Vec<u64>,
    bp_words: Vec<u64>,
    ty_words: Vec<u64>,
    /// Container marker bits - marks BP positions that have TY entries (mappings/sequences)
    container_words: Vec<u64>,
    bp_pos: usize,
    ty_pos: usize,

    // Direct BP-to-text mapping
    bp_to_text: Vec<u32>,
    /// End positions for scalars (start is in bp_to_text, end is here)
    bp_to_text_end: Vec<u32>,

    // Indentation tracking
    indent_stack: Vec<usize>,

    // Node type stack (to track if we're in mapping or sequence)
    type_stack: Vec<NodeType>,
    /// Cached current type for branchless access (avoids Option unwrapping in hot paths)
    current_type: Option<NodeType>,

    // Anchor and alias tracking
    /// Anchors collected during parsing: name → bp_pos of anchored value
    anchors: BTreeMap<String, usize>,
    /// Aliases collected during parsing: bp_pos → target bp_pos (resolved at parse time)
    aliases: BTreeMap<usize, usize>,
    /// Explicit source tags collected during parsing: bp_pos → raw tag text
    tags: BTreeMap<usize, String>,
    /// `bp_pos` of the node an anchor or tag was just scanned for, if that
    /// node hasn't opened yet. An alias node opening at this same `bp_pos`
    /// would mean the property was meant for it — invalid per the YAML 1.2
    /// grammar (an alias node carries no properties of its own), and
    /// rejected by real yq/PyYAML (#1374).
    ///
    /// Never needs clearing: `bp_pos` only ever increases (every
    /// [`Self::write_bp_open`]/[`Self::write_bp_close`] call increments it),
    /// so a stale value can only equal the position of the exact node the
    /// property was scanned for — never any later node.
    pending_property_bp: Option<usize>,
    /// Trailing same-line comments collected during parsing: bp_pos of the
    /// owning node → `(start, end)` byte range of the raw `#...` comment text.
    /// See [`SemiIndex::line_comments`].
    line_comments: BTreeMap<usize, (u32, u32)>,

    /// A comment trailing a `&anchor`/`!tag` whose value is deferred to a
    /// later line, not yet attached to any node (#784).
    ///
    /// Real yq attaches such a comment to whatever node the deferred value
    /// resolves to — its first child if one follows, or (if the value turns
    /// out null) the next sibling entirely — never to the anchor's own key
    /// line. That target doesn't exist yet at the point the comment is
    /// scanned, so [`Self::defer_line_comment`] stashes the byte range here
    /// and [`Self::take_pending_head_comment`] claims it once the next
    /// primary node (a mapping key or sequence item) actually opens.
    pending_head_comment: Option<(u32, u32)>,

    // Document tracking
    /// Whether we're currently inside a document
    in_document: bool,
    /// `bp_pos` recorded by [`Self::start_document`]. If [`Self::end_document`]
    /// finds `bp_pos` unchanged, the document had no content (e.g. `---\n...\n`
    /// or a directive-only document immediately followed by EOF) and gets a
    /// synthesized null node, mirroring [`Self::close_pending_explicit_key`]
    /// (#225).
    document_start_bp_pos: usize,

    // Explicit key tracking
    /// Depth (`indent_stack.len()`) of the mapping holding an explicit key that
    /// still needs a value, or `None`. The key gets a null value if no `: ` follows.
    ///
    /// A depth rather than a bool because a complex key can itself be an open
    /// container: `? k: v` makes the whole `k: v` the key, so the mapping on top of
    /// the stack is the *key*, and the owner — the mapping the null belongs to — is
    /// the one below it.
    pending_explicit_key: Option<usize>,

    /// Current depth of recursively-parsed constructs, capped at
    /// [`MAX_NESTING_DEPTH`] to bound call-stack growth
    nesting_depth: usize,

    /// One flag per `bp_to_text_end` slot, set for block sequence-item wrapper nodes.
    ///
    /// `YamlCursor::value` tells a childless wrapper from a plain scalar that merely
    /// begins `- ` by whether an end position was recorded (#332). That rests on the
    /// parser *never* recording one for a wrapper — an invariant the code held only by
    /// accident of left-to-right parsing, whose failure mode is silent: `- a` would
    /// start decoding as the string `"- a"`. Debug builds assert it in
    /// [`Self::set_bp_text_end`]; release builds carry nothing.
    #[cfg(debug_assertions)]
    wrapper_slots: Vec<bool>,

    /// The end position most recently recorded by [`Self::set_bp_text_end`].
    ///
    /// The other half of the same invariant: a wrapper stores no end of its own, so
    /// under the compact `EndPositions` variant it *inherits* this value, and that is
    /// what `YamlCursor::value` measures against the wrapper's own text position.
    /// Asserted in [`Self::write_bp_open_seq_item`].
    #[cfg(debug_assertions)]
    last_recorded_end: usize,

    /// The raw BP bit position ([`Self::bp_pos`] before increment) most
    /// recently assigned by [`Self::write_bp_open_at`].
    ///
    /// `bp_to_text_end.len() - 1` is *not* this value in general — it counts
    /// opens only, while `bp_pos` counts opens and closes together, so the
    /// two diverge by the number of closes that happened before this node's
    /// own open. [`Self::set_bp_text_end`]'s trailing-comment capture
    /// (#710) needs the real bit position, since that's what every
    /// `YamlCursor` method (`self.bp_pos`) keys its lookups on.
    last_open_bp_pos: usize,
}

impl<'a, const HAS_CR: bool> Parser<'a, HAS_CR> {
    fn new(input: &'a [u8]) -> Self {
        debug_assert!(
            HAS_CR || !crate::util::simd::escape::contains_cr(input),
            "Parser::<false> built for input containing a carriage return: the \
             LF-only specialization would silently mis-parse it (#340)"
        );
        let ib_words = vec![0u64; input.len().div_ceil(64).max(1)];
        let bp_words = vec![0u64; input.len().div_ceil(32).max(1)]; // ~2x IB for BP
        let ty_words = vec![0u64; input.len().div_ceil(64).max(1)];
        let container_words = vec![0u64; input.len().div_ceil(32).max(1)]; // Same size as BP

        // Estimate BP opens: ~1 structural element per 8 bytes of input
        let estimated_opens = input.len().div_ceil(8).max(1);

        // Pre-allocate indent/type stacks for typical nesting depths
        let mut indent_stack = Vec::with_capacity(32);
        indent_stack.push(0); // Start at indent 0

        Self {
            input,
            pos: 0,
            ib_words,
            bp_words,
            ty_words,
            container_words,
            bp_pos: 0,
            ty_pos: 0,
            bp_to_text: Vec::with_capacity(estimated_opens),
            bp_to_text_end: Vec::with_capacity(estimated_opens),
            indent_stack,
            type_stack: Vec::with_capacity(32),
            current_type: None,
            anchors: BTreeMap::new(),
            aliases: BTreeMap::new(),
            tags: BTreeMap::new(),
            pending_property_bp: None,
            line_comments: BTreeMap::new(),
            pending_head_comment: None,
            in_document: false,
            document_start_bp_pos: 0,
            pending_explicit_key: None,
            nesting_depth: 0,
            #[cfg(debug_assertions)]
            wrapper_slots: Vec::with_capacity(estimated_opens),
            #[cfg(debug_assertions)]
            last_recorded_end: 0,
            last_open_bp_pos: 0,
        }
    }

    /// Enter a recursively-parsed construct, erroring past [`MAX_NESTING_DEPTH`].
    /// Callers must decrement `nesting_depth` when the construct's frame exits.
    #[inline]
    fn enter_nested(&mut self) -> Result<(), YamlError> {
        if self.nesting_depth >= MAX_NESTING_DEPTH {
            return Err(YamlError::NestingTooDeep {
                offset: self.pos,
                limit: MAX_NESTING_DEPTH,
            });
        }
        self.nesting_depth += 1;
        Ok(())
    }

    /// Push a type onto the type stack and update the cached current type.
    #[inline]
    fn push_type(&mut self, node_type: NodeType) {
        self.type_stack.push(node_type);
        self.current_type = Some(node_type);
    }

    /// Pop a type from the type stack and update the cached current type.
    #[inline]
    fn pop_type(&mut self) -> Option<NodeType> {
        let popped = self.type_stack.pop();
        self.current_type = self.type_stack.last().copied();
        popped
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
    #[allow(dead_code)] // STYLE-0005: helper for an experimental SIMD path
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
        self.last_open_bp_pos = self.bp_pos;
        // Record the text position for this BP open
        self.bp_to_text.push(text_pos as u32);
        // Placeholder for end position (will be set by set_bp_text_end for scalars)
        self.bp_to_text_end.push(0);
        #[cfg(debug_assertions)]
        self.wrapper_slots.push(false);
        self.bp_pos += 1;
    }

    /// Open a block sequence-item wrapper node.
    ///
    /// Distinct from [`Self::write_bp_open`] only in debug builds, where it records the
    /// slot so [`Self::set_bp_text_end`] can assert no end is ever stored for it — the
    /// invariant `YamlCursor::value` relies on to tell an empty item from a plain scalar
    /// beginning `- ` (#332) — and asserts the other half of that invariant here.
    #[inline]
    fn write_bp_open_seq_item(&mut self) {
        self.write_bp_open();
        #[cfg(debug_assertions)]
        {
            // Storing no end is only half of what the reader needs. Under the compact
            // `EndPositions` variant the wrapper's slot is zero-filled from the last end
            // recorded *before* it, and `value()` reads `end <= text_pos` as "no extent
            // of its own — empty item, null". So that inherited end must never sit past
            // the `-`. It cannot today: every end is recorded at or before `self.pos`,
            // and `self.pos` only advances. Assert it because the failure is silent —
            // an empty `- ` would start reading as the string `"-"` — and because the
            // wrapper's text position is a parameter of `write_bp_open_at`, not a
            // constant.
            let text_pos = self.bp_to_text.last().copied().unwrap_or_default() as usize;
            debug_assert!(
                self.last_recorded_end <= text_pos,
                "sequence-item wrapper at text {text_pos} inherits the end position {} \
                 of an earlier node; YamlCursor::value reads an end past the wrapper as \
                 the plain scalar `- …` rather than an empty item (#332)",
                self.last_recorded_end
            );
            if let Some(last) = self.wrapper_slots.last_mut() {
                *last = true;
            }
        }
    }

    /// Set the end text position for the most recently opened BP node.
    /// Call this before write_bp_close for scalar nodes.
    ///
    /// "Most recently opened" means the last slot *pushed*, not the node the caller is
    /// about to close. If the content just parsed opened BP nodes of its own — a nested
    /// flow container, an alias — this writes the innermost of those instead, corrupting
    /// its extent (#332). Such nodes need no end anyway: `value()` recognises them from
    /// their leading `[`, `{` or `*`. So don't call this after them.
    #[inline]
    fn set_bp_text_end(&mut self, end_pos: usize) {
        self.set_bp_text_end_position(end_pos);
        // The node whose end was just recorded is the one that owns a
        // trailing same-line comment, if `self.pos` (left wherever scanning
        // for this value stopped) has nothing but inline whitespace before
        // a `#`. Every scalar-parsing call site invokes `set_bp_text_end`
        // immediately after its own scan loop returns, with no intervening
        // advance, so `self.pos` still reflects exactly where that scan
        // stopped (#710).
        let owner_bp_pos = self.last_open_bp_pos;
        self.maybe_capture_line_comment(owner_bp_pos);
    }

    /// Record `end_pos` as the text end for the node currently open, without
    /// attempting trailing-comment capture (#710). Block scalars call this
    /// directly: their own trailing comment, if any, was already captured
    /// explicitly on the header line (`| # text`) before content was
    /// consumed, and by the time content parsing finishes `self.pos` sits at
    /// the start of a following line — possibly several blank lines, or a
    /// comment line belonging to the next sibling, past the block region
    /// (`consume_block_scalar_content`/`detect_block_content_indent` both
    /// advance `self.pos` past it). `set_bp_text_end`'s same-line capture
    /// would misattribute that unrelated following comment to this block
    /// scalar; skipping it here is always safe since `self.pos` is at column
    /// 0 of a fresh line, never mid-line, so there is no legitimate
    /// same-line comment left to find.
    #[inline]
    fn set_bp_text_end_position(&mut self, end_pos: usize) {
        #[cfg(debug_assertions)]
        debug_assert!(
            self.wrapper_slots.last() != Some(&true),
            "recorded an end position for a block sequence-item wrapper at text {}; \
             YamlCursor::value would decode it as the plain scalar `- …` instead of \
             unwrapping to its content (#332)",
            self.bp_to_text.last().copied().unwrap_or_default()
        );
        if let Some(last) = self.bp_to_text_end.last_mut() {
            *last = end_pos as u32;
        }
        #[cfg(debug_assertions)]
        {
            self.last_recorded_end = end_pos;
        }
    }

    /// Scan for a trailing same-line comment starting at `self.pos`, if it's
    /// separated from a `#` only by inline whitespace (spaces/tabs) on the
    /// current line. Does not consume any input or record anything —
    /// [`Self::maybe_capture_line_comment`] and [`Self::defer_line_comment`]
    /// are the two storage-specific callers; kept as one scan so a future fix
    /// to what counts as a trailing comment can't be applied to one and
    /// silently missed in the other (the exact "duplicated predicates
    /// diverge silently" risk this project has hit before).
    #[inline]
    fn scan_trailing_comment(&self) -> Option<(u32, u32)> {
        let mut p = self.pos;
        while p < self.input.len() && Self::is_inline_whitespace(self.input[p]) {
            p += 1;
        }
        if p < self.input.len() && self.input[p] == b'#' {
            let start = p;
            while p < self.input.len() && !Self::is_break(self.input[p]) {
                p += 1;
            }
            Some((start as u32, p as u32))
        } else {
            None
        }
    }

    /// Capture a trailing same-line comment for `owner_bp_pos`. Callers
    /// still run their own `skip_to_eol`/`skip_newlines` afterward to
    /// actually advance past it.
    ///
    /// Uses `entry().or_insert()` rather than unconditional `insert()` so a
    /// node captured explicitly at a more specific point (e.g. a block
    /// scalar's header-line comment, captured before its content is parsed)
    /// is never clobbered by a later, spurious call for the same bp_pos.
    #[inline]
    fn maybe_capture_line_comment(&mut self, owner_bp_pos: usize) {
        if let Some(range) = self.scan_trailing_comment() {
            self.line_comments.entry(owner_bp_pos).or_insert(range);
        }
    }

    /// Like [`Self::maybe_capture_line_comment`], but the owning node doesn't
    /// exist yet: stash the comment's byte range in
    /// [`Self::pending_head_comment`] instead of `line_comments` directly.
    /// [`Self::take_pending_head_comment`] attaches it once that node opens
    /// (#784).
    ///
    /// Never overwrites an already-outstanding pending comment: a nested
    /// anchor can itself defer a second comment before the first is ever
    /// claimed (`- &x # c1\n  - &y # c2\n    b: 1\n`, the outer sequence
    /// item's `c1` still pending when the inner item's own `&y` defers
    /// `c2`) — first-deferred-first-claimed keeps that outer comment alive
    /// long enough for a consumption site to reach it, rather than silently
    /// destroying it. This can't be told apart from "genuinely nothing
    /// pending" by [`Self::drop_stale_pending_head_comment`]'s once-per-line
    /// check, since the clobber (if it happened) would occur *before* that
    /// check ever runs.
    #[inline]
    fn defer_line_comment(&mut self) {
        if self.pending_head_comment.is_some() {
            return;
        }
        self.pending_head_comment = self.scan_trailing_comment();
    }

    /// Claim a comment deferred by [`Self::defer_line_comment`] for
    /// `owner_bp_pos`, if one is pending (#784).
    ///
    /// Called at whichever bp_pos the *renderer* actually reads a trailing
    /// comment from for that shape of node, mirroring the bp each call
    /// site's own ordinary (non-deferred) same-line comment already
    /// attaches to: a mapping key (`parse_mapping_entry`,
    /// `parse_compact_mapping_entry` — both read from the key regardless of
    /// whether the value is inline or deferred further), a plain scalar
    /// (`parse_block_node`'s catch-all arm), or a plain-scalar sequence
    /// item's own scalar (`parse_sequence_item_inner`, after `parse_value`
    /// returns — a sequence item's *wrapper* bp is never read, only its
    /// content's). Never call this for a node the deferred value's own
    /// null-value fallback opens inline, or the comment would misattach to
    /// the anchor's own empty value instead of floating to the next
    /// sibling, matching real yq's own behavior.
    ///
    /// **Ordering matters**: always call this *after* any ordinary
    /// same-line capture for the same `owner_bp_pos` has already had its
    /// chance to run (e.g. after `maybe_capture_line_comment`/
    /// `set_bp_text_end`'s own call for that bp), never before. Both use the
    /// same `entry().or_insert()` idiom, so whichever runs first wins the
    /// slot — calling this first would silently destroy a node's own
    /// genuine trailing comment in favor of an unrelated floated one instead
    /// of just leaving the floated one to be dropped (#784 review).
    #[inline]
    fn take_pending_head_comment(&mut self, owner_bp_pos: usize) {
        if let Some(range) = self.pending_head_comment.take() {
            self.line_comments.entry(owner_bp_pos).or_insert(range);
        }
    }

    /// Drop a [`Self::pending_head_comment`] that survived one full
    /// document-line dispatch completely untouched — neither consumed by
    /// [`Self::take_pending_head_comment`] nor replaced by a fresh
    /// [`Self::defer_line_comment`] call chained off that same line's own
    /// anchor (#784). `before` is the value observed prior to the dispatch;
    /// comparing by value, not just presence, is what tells "untouched" apart
    /// from "consumed, then a new one queued in its place" when a line
    /// chains two deferred anchors (`a: &x # c1` immediately followed by
    /// `b: &y # c2`) — a boolean "was something pending" flag can't
    /// distinguish those, and would wipe out `c2` before it ever got a
    /// chance to attach.
    #[inline]
    fn drop_stale_pending_head_comment(&mut self, before: Option<(u32, u32)>) {
        if before.is_some() && self.pending_head_comment == before {
            self.pending_head_comment = None;
        }
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

    /// Whether the mapping on top of `indent_stack` right now is the owner
    /// of `pending_explicit_key` — i.e. the frame that would receive the
    /// key's value if one were written this instant. A complex key can
    /// itself be an open container (`? k: v` leaves the key mapping above
    /// its owner), so this is only true once the stack has unwound back to
    /// exactly the owner's own recorded depth.
    ///
    /// Shared by every pending-key-aware pop site (#106: one definition,
    /// not a copy of this same equality test re-derived at each call site).
    fn pending_explicit_key_owns_current_frame(&self) -> bool {
        self.pending_explicit_key == Some(self.indent_stack.len())
    }

    /// Close a pending explicit key by adding a null value node.
    /// Call this when a new key or end of mapping is encountered without an explicit value.
    ///
    /// The null goes to the mapping that *owns* the key, which is only the mapping on
    /// top of the stack when the depths match (see
    /// [`Self::pending_explicit_key_owns_current_frame`]) — writing the owner's null
    /// into a still-open complex key would give the key a stray third child and consume
    /// the pending state before the real `: value` line arrives.
    ///
    /// Every caller that wants a null synthesized funnels through here rather than
    /// repeating the ownership test at the call site: three copies of one predicate is
    /// how #106 happened. [`Self::close_same_indent_sequence_before_mapping_entry`]
    /// checks the same predicate directly instead, since it must clear the flag
    /// *without* synthesizing a null — the key there already has a real value (the
    /// sequence just closed), not a missing one.
    fn close_pending_explicit_key(&mut self) {
        if self.pending_explicit_key_owns_current_frame() {
            // Add a null value node (empty open/close pair)
            // Use input.len() as the text position to indicate "no text" / null value
            self.write_bp_open_at(self.input.len());
            self.write_bp_close();
            self.pending_explicit_key = None;
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
            self.pos += 1;
        }
    }

    /// Advance position by multiple bytes.
    #[inline]
    fn advance_by(&mut self, count: usize) {
        self.pos = (self.pos + count).min(self.input.len());
    }

    /// Compute line number at current position (1-indexed).
    /// Only called on error paths, so we pay the cost only when needed.
    #[inline]
    fn current_line(&self) -> usize {
        // Count line breaks from start to current position, by counting the
        // bytes each break *ends* at: a break of width 1 ends where it starts,
        // so a CRLF — width 2 at its `\r` — is counted once, at its `\n` (#324).
        //
        // `line_break_len` reads `self.input`, not the truncated prefix: when
        // the cursor sits on the LF of a CRLF, that CR's partner is one byte
        // past the prefix, and measuring within the prefix would read it as a
        // lone CR and report a line too many.
        let breaks = (0..self.pos)
            .filter(|&i| line_break_len(self.input, i) == 1)
            .count();
        breaks + 1
    }

    /// Is `b` a YAML line break, under this parser's `HAS_CR` specialization?
    ///
    /// Not a second spelling of [`is_line_break`] — #341 deleted that and was
    /// right to. The `HAS_CR` arm *delegates* to it, so the rule still has one
    /// definition; what this adds is the compile-time switch #340 needs. On a
    /// document with no `\r` anywhere the `\r` compare is not merely never taken,
    /// it is not emitted, and this is the per-byte test in the inlined scalar
    /// loop where that matters.
    #[inline]
    fn is_break(b: u8) -> bool {
        if HAS_CR {
            is_line_break(b)
        } else {
            b == b'\n'
        }
    }

    /// A space or tab - YAML's "inline whitespace", separation within a
    /// line rather than between them. Shared by [`Self::skip_inline_whitespace`]
    /// (which consumes it) and [`Self::maybe_capture_line_comment`] (which
    /// only peeks past it, since its caller still needs to consume the run
    /// itself).
    #[inline]
    fn is_inline_whitespace(b: u8) -> bool {
        matches!(b, b' ' | b'\t')
    }

    /// Width in bytes of the line break at `pos`, under this parser's `HAS_CR`
    /// specialization: [`line_break_len`] when a `\r` may be present, and
    /// otherwise the one-byte LF answer it collapses to.
    #[inline]
    fn break_len_at(&self, pos: usize) -> usize {
        if HAS_CR {
            line_break_len(self.input, pos)
        } else {
            usize::from(self.input.get(pos) == Some(&b'\n'))
        }
    }

    /// Is the current position at a line break?
    #[inline]
    fn at_break(&self) -> bool {
        self.peek().is_some_and(is_line_break)
    }

    /// Is `b` whitespace or a line break?
    #[inline]
    fn is_ws_or_break(b: u8) -> bool {
        matches!(b, b' ' | b'\t') || Self::is_break(b)
    }

    /// Does `next` terminate an indicator character — whitespace, a line break,
    /// or end of input?
    ///
    /// This is the lookahead that separates a *structural* `-`/`?`/`:` from the
    /// first byte of a plain scalar. It had seventeen inline `matches!` copies
    /// before #340; one definition means the `HAS_CR` gate has a single place to
    /// live and the copies cannot drift apart (#106). It also keeps the test off
    /// its own source line, which a multi-line `matches!` in a match guard turns
    /// into a coverage region that never reports as executed.
    ///
    /// This is the parser's spelling of the terminator set [`super::is_seq_indicator_next`]
    /// gives the *reader* (#332). It cannot simply call it: the whole point of #340 is that
    /// the `\r` arm here becomes compile-time gated, and the reader — which indexes into
    /// text rather than scanning it under a `HAS_CR` specialization — has no such gate. The
    /// two are therefore pinned by `is_ws_break_or_eoi_agrees_with_is_seq_indicator_next`
    /// rather than by the compiler; change one acceptance set and that test fails.
    #[inline]
    fn is_ws_break_or_eoi(next: Option<u8>) -> bool {
        match next {
            Some(b) => Self::is_ws_or_break(b),
            None => true,
        }
    }

    /// Consume the line break at the current position, if any.
    ///
    /// The one deliberate restatement of [`line_break_len`]. Dispatching on the
    /// byte keeps the overwhelmingly common LF case at one bounds-checked load
    /// and an increment — exactly what the `if peek() == Some(b'\n') { advance() }`
    /// it replaced cost. Routing it through `advance_by(line_break_len(..))`
    /// instead doubled the bounds checks on a per-line path.
    ///
    /// Because it is a copy, it is pinned to the shared definition by
    /// `skip_line_break_agrees_with_line_break_len` rather than by the compiler
    /// (#341). Change one and that test fails.
    #[inline]
    fn skip_line_break(&mut self) {
        if !HAS_CR {
            // Exactly the `if peek() == Some(b'\n') { advance() }` this replaced.
            if self.input.get(self.pos) == Some(&b'\n') {
                self.pos += 1;
            }
            return;
        }
        match self.input.get(self.pos) {
            Some(b'\n') => self.pos += 1,
            Some(b'\r') => {
                self.pos += 1;
                if self.input.get(self.pos) == Some(&b'\n') {
                    self.pos += 1;
                }
            }
            _ => {}
        }
    }

    /// Skip whitespace on the current line (spaces and tabs, not newlines).
    #[inline]
    fn skip_inline_whitespace(&mut self) {
        while self.pos < self.input.len() && Self::is_inline_whitespace(self.input[self.pos]) {
            self.pos += 1;
        }
    }

    /// Skip spaces only (not tabs) with hybrid scalar/SIMD approach.
    /// Returns number of spaces skipped.
    #[inline]
    fn skip_spaces_simd(&mut self) -> usize {
        // Fast path for short runs (0-8 spaces) - avoid SIMD overhead
        let mut count = 0;
        while count < 8 && self.pos < self.input.len() && self.input[self.pos] == b' ' {
            self.pos += 1;
            count += 1;
        }

        // If we found non-space within 8 bytes, we're done
        if count < 8 || self.pos >= self.input.len() {
            return count;
        }

        // For longer runs (>= 8 spaces), use SIMD from current position
        let remaining = super::simd::count_leading_spaces(self.input, self.pos);
        self.pos += remaining;
        count + remaining
    }

    /// Find next newline using SIMD acceleration.
    /// Returns offset from current position, or None if not found.
    #[inline]
    #[allow(dead_code)] // STYLE-0005: helper for an experimental SIMD path
    fn find_next_newline_simd(&self) -> Option<usize> {
        super::simd::find_newline(self.input, self.pos)
    }

    /// SIMD fast-path for skipping regular characters in unquoted values.
    /// Returns the number of bytes that can be safely skipped, or None if
    /// a potential terminator was found immediately.
    #[cfg(all(target_arch = "x86_64", not(feature = "scalar-yaml")))]
    #[inline]
    fn skip_unquoted_simd(&self, _value_start: usize) -> Option<usize> {
        // Use classify_yaml_chars to scan 32 bytes at once
        if let Some(class) = super::simd::classify_yaml_chars::<HAS_CR>(self.input, self.pos) {
            // Line break, colon, or hash — see `plain_scalar_terminators`, which
            // both this and the ARM variant below share so the two cannot drift
            // on what ends a scalar (#185). `\r` counts: it is a YAML 1.2 §5.4
            // line break, and without it the classifier skips straight over a
            // lone CR and swallows the rest of the document into one scalar
            // (#324).
            //
            // The accessor takes the same `HAS_CR` the classifier did. Under
            // `false` the classifier leaves `carriage_returns` zero, so the OR
            // would be a no-op — but that promise is one the optimizer cannot
            // verify across the `#[target_feature]` call, so the const gates
            // both ends and the compare never reaches the instruction stream
            // (#340).
            let terminators = class.plain_scalar_terminators::<HAS_CR>();

            if terminators == 0 {
                // No structural characters in the classified chunk — skip
                // exactly the bytes the classifier scanned (32 with AVX2, 16
                // with SSE2). Deriving the width from input length alone
                // assumed AVX2: after a 16-byte SSE2 classify it skipped 32,
                // swallowing newlines/colons in bytes 16..31 on non-AVX2 CPUs
                // (#193).
                return Some(class.width);
            }

            // Found a potential terminator - find its position
            let first_pos = terminators.trailing_zeros() as usize;

            // If it's at position 0, we can't skip anything
            if first_pos == 0 {
                return None;
            }

            // We can safely skip up to the terminator position
            Some(first_pos)
        } else {
            None
        }
    }

    /// Broadword fast-path for skipping regular characters in unquoted values (ARM64).
    /// Uses pure u64 arithmetic instead of NEON movemask emulation for better performance.
    /// Returns the number of bytes that can be safely skipped, or None if
    /// a potential terminator was found immediately.
    ///
    /// NOTE: Currently disabled - benchmarks showed neutral to slight regression.
    /// Kept for future investigation. See P4 analysis in docs/parsing/yaml.md.
    #[cfg(all(target_arch = "aarch64", not(feature = "scalar-yaml")))]
    #[inline]
    #[allow(dead_code)] // STYLE-0005: helper for an experimental SIMD path
    fn skip_unquoted_simd(&self, _value_start: usize) -> Option<usize> {
        // Use broadword classify to scan 16 bytes at once (two 8-byte chunks)
        if let Some(class) = super::simd::classify_yaml_chars_16(self.input, self.pos) {
            // The same set the live x86 path uses: line break (LF or CR), colon,
            // hash. This called a `value_terminators()` that also included
            // spaces until #185, so re-enabling this path would have stopped the
            // skip at every space for no reason.
            let terminators = class.plain_scalar_terminators();

            if terminators == 0 {
                // No structural characters in this 16-byte chunk - safe to skip all
                return Some(16);
            }

            // Found a potential terminator - find its position
            let first_pos = terminators.trailing_zeros() as usize;

            // If it's at position 0, we can't skip anything
            if first_pos == 0 {
                return None;
            }

            // We can safely skip up to the terminator position
            Some(first_pos)
        } else {
            None
        }
    }

    /// Count leading spaces (indentation) at start of a line.
    fn count_indent(&self) -> Result<usize, YamlError> {
        // Use SIMD-accelerated space counting
        let count = super::simd::count_leading_spaces(self.input, self.pos);

        // Check for tab at the position after spaces
        let next_pos = self.pos + count;
        if next_pos < self.input.len() && self.input[next_pos] == b'\t' {
            // Tab after spaces - check context
            // If we haven't seen any spaces and hit a tab at start of line,
            // that's tab indentation (error). But tab after spaces is content.
            if count == 0 {
                return Err(YamlError::TabIndentation {
                    line: self.current_line(),
                    offset: next_pos,
                });
            }
            // Tab after spaces is start of content, indent count is correct
        }
        Ok(count)
    }

    /// Is the cursor sitting on a tab that is *indentation* rather than separation?
    ///
    /// `count_indent` counts spaces only, so after `advance_by(indent)` the cursor
    /// lands on a tab whenever one follows the leading spaces. YAML forbids a tab in
    /// indentation, but a tab is only indentation when block structure follows —
    /// hence [`super::line_is_structural`], shared with the strict validator, rather
    /// than a bare "is there a tab" (#173).
    ///
    /// `line_start` is where this line's leading spaces began. The check matters
    /// because `parse_document_line` is not always entered at a line start:
    /// `parse_explicit_key` returns mid-line for `? k: v` (see
    /// docs/compliance/yaml/limitations.md), and the flow and quoted scanners stop
    /// just past their closing delimiter, so the main loop can re-derive an "indent"
    /// from a mid-line cursor. A tab in the middle of a line is never indentation.
    ///
    /// Only reachable with `indent >= 1`: `count_indent` already returns
    /// `Err(TabIndentation)` for a tab at column 0, so `Ok(0)` implies no tab here.
    #[inline]
    fn tab_indents_block_structure(&self, line_start: usize) -> bool {
        self.peek() == Some(b'\t')
            && (line_start == 0 || is_line_break(self.input[line_start - 1]))
            && super::line_is_structural(self.input, self.pos)
    }

    /// Skip `s-separate-in-line` whitespace still sitting on the cursor after
    /// the leading indent has already been consumed by `advance_by`.
    ///
    /// `count_indent` only counts spaces, so a tab immediately after them is
    /// left on the cursor. When that tab is legal separation (not indenting
    /// block structure — see `tab_indents_block_structure`, which this reuses
    /// so the two never disagree), it is not part of the following node and
    /// must be skipped before a caller dispatches on `self.peek()` or records
    /// a node's start position. Otherwise the tab becomes the node's first
    /// byte: a plain scalar picks up a leading `\t` (`DK95/00`), and a quoted
    /// node is pushed off its opening quote and misread as a mapping (#381).
    ///
    /// A tab that *does* indent block structure is left in place for
    /// `tab_indents_block_structure`'s caller to reject.
    #[inline]
    fn skip_separation_whitespace(&mut self, line_start: usize) {
        loop {
            match self.peek() {
                Some(b' ') => self.advance(),
                Some(b'\t') if !self.tab_indents_block_structure(line_start) => self.advance(),
                _ => break,
            }
        }
    }

    /// Get the current column position (0-based).
    /// This counts characters from the start of the current line.
    fn current_column(&self) -> usize {
        // Find the start of the current line
        let mut line_start = self.pos;
        while line_start > 0 && !Self::is_break(self.input[line_start - 1]) {
            line_start -= 1;
        }
        self.pos - line_start
    }

    /// Build a `YamlError::UnexpectedCharacter` for the byte at `offset`
    /// (#1187): every one of this file's 8 call sites built this variant by
    /// hand, and 7 of the 8 used `self.peek().map_or('\0', |b| b as char)`
    /// -- a Latin-1 cast, not a real UTF-8 decode, so a multi-byte character
    /// at the offending position rendered as mojibake instead of itself.
    /// Decodes via [`text::utf8::decode_char_at`] instead, which (#1422)
    /// generalized this function's own three-way fallback logic into a
    /// shared primitive `yaml/validate.rs` and `json/validate.rs` also use,
    /// against the byte slice starting at `offset` (not just the one byte
    /// `self.peek()`/`self.input[offset]` would give, which isn't enough to
    /// decode a multi-byte sequence).
    ///
    /// Takes `offset` explicitly rather than reading `self.pos`: the one
    /// site with a genuinely different call shape
    /// ([`Self::reject_trailing_flow_content`]) finds the offending byte
    /// via a local scan cursor, before `self.pos` itself has advanced past
    /// it.
    ///
    /// The `None`/true-EOF fallback inside `decode_char_at` carries no
    /// coverage from any test reaching *this* function, before or after
    /// #1187's introduction of it: two of the 8 call sites
    /// (`parse_flow_sequence_inner`/`parse_flow_mapping_inner`'s comma
    /// checks) have their own dedicated `self.peek().is_none()` ->
    /// `YamlError::UnexpectedEof` guard immediately before reaching this
    /// call, and the remaining key-colon sites are only ever entered once a
    /// `looks_like_*_entry`-style lookahead has already confirmed a `:`
    /// exists later in the input, making true EOF before finding *some*
    /// byte structurally unlikely there too -- not verified with the same
    /// rigor as `parse_block_scalar_header`'s catch-all below, so kept as a
    /// real (not `unreachable!()`) fallback rather than a proven-dead one.
    fn err_unexpected_char(&self, offset: usize, context: &'static str) -> YamlError {
        YamlError::UnexpectedCharacter {
            offset,
            char: text::utf8::decode_char_at(self.input, offset),
            context,
        }
    }

    /// Check if at end of meaningful content on this line.
    fn at_line_end(&self) -> bool {
        let mut i = self.pos;
        while i < self.input.len() {
            match self.input[i] {
                b'#' => return true, // Comment starts
                b' ' => i += 1,
                b if Self::is_break(b) => return true,
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
            Some(b'{' | b'[') => return false,
            // Empty key: `:` at start followed by whitespace/newline/EOF
            Some(b':') => {
                let next = self.peek_at(1);
                if Self::is_ws_break_or_eoi(next) {
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
                } else if Self::is_break(self.input[i]) {
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
                return Self::is_ws_break_or_eoi(next);
            }
            return false;
        }

        // Scan for `: ` pattern in unquoted key
        while i < self.input.len() {
            match self.input[i] {
                b':' => {
                    // Check what follows the colon
                    let next = if i + 1 < self.input.len() {
                        Some(self.input[i + 1])
                    } else {
                        None
                    };
                    if Self::is_ws_break_or_eoi(next) {
                        return true;
                    }
                    i += 1; // Colon not followed by whitespace, continue
                }
                // Line ended without finding `: `
                b if Self::is_break(b) => return false,
                // Note: " and ' in the middle of a key are allowed (e.g., bla"keks: foo)
                // Continue scanning past them.
                _ => i += 1,
            }
        }
        false
    }

    /// Check whether the node property/properties (`&anchor`, `!tag`, or both,
    /// in either order) at the current position prefix a mapping entry rather
    /// than the node itself, as in `&a k: v` or `!!str k: v`. In that shape
    /// the property binds to the *key*, so the caller must leave it for the
    /// mapping parser instead of consuming it (see `parse_mapping_entry`,
    /// which records the key's own BP position) — generalizes what was
    /// `anchor_prefixes_mapping_entry` (anchor-only) before #224 gave tags
    /// the same "prefixes a key" ambiguity anchors already had.
    ///
    /// Assumes `self.peek()` is `Some(b'&')` or `Some(b'!')`. Restores
    /// `self.pos` before returning.
    fn node_properties_prefix_mapping_entry(&mut self) -> bool {
        let saved_pos = self.pos;
        loop {
            match self.peek() {
                Some(b'&') => {
                    // Skip `&` and the anchor name. Deliberately uses the
                    // scanner rather than `parse_anchor_name`, which errors on
                    // an empty name and would turn this speculative lookahead
                    // into a hard parse failure on `- & x: y`.
                    self.pos = super::simd::parse_anchor_name(self.input, self.pos + 1);
                    self.skip_inline_whitespace();
                }
                Some(b'!') => {
                    // Permissive twin of `parse_tag`, for the same reason:
                    // a malformed tag must not turn this speculative
                    // lookahead into a hard parse failure.
                    let (end, _) = scan_tag_extent(self.input, self.pos);
                    self.pos = end;
                    self.skip_inline_whitespace();
                }
                _ => break,
            }
        }
        // `looks_like_mapping_entry` does not stop at `#`, so a trailing comment
        // containing `: ` would otherwise read as a mapping entry.
        let result = !self.at_line_end() && self.looks_like_mapping_entry();
        self.pos = saved_pos;
        result
    }

    /// Skip to end of line (handles comments).
    ///
    /// Stops *before* the line break, whichever of the three forms it is, so
    /// callers can consume it with `skip_line_break` (#324).
    #[inline]
    fn skip_to_eol(&mut self) {
        while self.pos < self.input.len() && !Self::is_break(self.input[self.pos]) {
            self.pos += 1;
        }
    }

    /// Skip newline and empty/comment lines.
    fn skip_newlines(&mut self) {
        while let Some(b) = self.peek() {
            if Self::is_break(b) {
                self.skip_line_break();
            } else if b == b'#' {
                // Comment line
                self.skip_to_eol();
            } else if b == b' ' {
                // Check if rest of line is whitespace or comment
                let start = self.pos;
                self.skip_inline_whitespace();
                if self.at_break() || self.peek() == Some(b'#') || self.peek().is_none() {
                    if self.peek() == Some(b'#') {
                        self.skip_to_eol();
                    }
                    continue;
                }
                // Non-empty content - back up
                self.pos = start;
                break;
            } else {
                break;
            }
        }
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
        // Must be followed by white space (space or tab), a line break, or EOF.
        // The tab is not optional: `doc_marker_char` (validate.rs), the strict
        // validator's copy of this same check, already includes it, and
        // dropping it here meant `---\tfoo` was silently parsed as content
        // instead of a document boundary (#434).
        Self::is_ws_break_or_eoi(self.peek_at(3))
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
        // Must be followed by white space (space or tab), a line break, or EOF.
        // See `is_document_start` for why the tab isn't optional (#434).
        Self::is_ws_break_or_eoi(self.peek_at(3))
    }

    /// Skip past a document marker (`---` or `...`).
    /// Does NOT skip content after the marker - that should be parsed.
    fn skip_document_marker(&mut self) {
        // Skip the 3-character marker
        self.advance();
        self.advance();
        self.advance();
        // Skip trailing separation white space after the marker, if present -
        // both space and tab, matching `parse_inline_document_value`'s own
        // leading-whitespace skip right after this call (#434).
        while matches!(self.peek(), Some(b' ' | b'\t')) {
            self.advance();
        }
    }

    /// Check if there's parseable content on the current line (not just whitespace/comment).
    fn has_content_on_line(&self) -> bool {
        let mut i = 0;
        loop {
            match self.peek_at(i) {
                Some(b' ' | b'\t') => i += 1,
                Some(b'\n' | b'\r' | b'#') | None => return false,
                _ => return true,
            }
        }
    }

    /// Parse content after a document marker on the same line (e.g., `--- >` or `--- value`).
    ///
    /// The content of a `---` line is an ordinary block-context node that
    /// happens to start mid-line, so it goes through the same
    /// [`Self::parse_block_node`] every other line does. It had its own partial
    /// copy of that dispatch until #407, and the two had drifted apart in six
    /// shapes — most visibly `--- &x` with its node on the next line, which
    /// split one document into two.
    ///
    /// The indent is 0 rather than one re-derived from the cursor:
    /// [`Self::skip_document_marker`] consumes a run of trailing spaces and
    /// tabs, so `count_indent` would read `---···&x` as indent 3 (or worse
    /// with a tab in the run), when the node is at document root either way.
    fn parse_inline_document_value(&mut self) -> Result<(), YamlError> {
        // Skip leading whitespace
        self.skip_inline_whitespace();

        // The one arm that is genuinely the `---` line's own. `parse_block_node`
        // has no `|`/`>` arm: at document root a block scalar falls to its
        // plain-scalar arm, and `YamlCursor::value` re-reads the header from
        // the node's text (`parse_block_header`) when the value is
        // materialized. Calling `parse_block_scalar` directly keeps `--- |` on
        // the path it has always taken.
        //
        // A tag is resolved the same way an anchor already is here: this
        // dispatch has no arm of its own for either, so `--- !!str x` (like
        // `--- &a x`) falls through to `parse_block_node(0)`, whose combined
        // `&`/`!` arm consumes it before dispatching the value (#224). That
        // also means a tag immediately before `|`/`>` on this line
        // (`--- !!str |`) takes the same generic-scalar path an anchor in
        // that position already does, rather than this function's dedicated
        // block-scalar arm — parity with the anchor case, not a new gap.
        if matches!(self.peek(), Some(b'|' | b'>')) {
            return self.parse_block_scalar(0);
        }

        self.parse_block_node(0)
    }

    /// Start a new document within the virtual root sequence.
    /// This doesn't open a container - the document IS its content.
    fn start_document(&mut self) {
        self.in_document = true;
        self.document_start_bp_pos = self.bp_pos;
        // A comment deferred by an anchor in a *previous* document (#784)
        // has no node left to attach to once a new document starts -
        // `parse_document_line`'s own one-line grace period already covers
        // ordinary lines, but a document boundary can be reached via
        // `parse_inline_document_value` instead, which doesn't go through
        // that backstop.
        self.pending_head_comment = None;
    }

    /// End the current document, closing any open containers.
    fn end_document(&mut self) {
        if !self.in_document {
            return;
        }

        // An explicit `---`/`...` boundary always produces a document, even
        // with no content before the next boundary or EOF (#225's `MUS6/02-06`,
        // `6ZKB`, `9DXL`, `W4TN`) - give it a null value, the same way
        // `close_pending_explicit_key` synthesizes null for a key with no
        // value. `bp_pos` unchanged since `start_document` means nothing was
        // written for this document: any container open, or scalar/anchor
        // write, would have advanced it already.
        if self.bp_pos == self.document_start_bp_pos {
            self.write_bp_open_at(self.input.len());
            self.write_bp_close();
        }

        // Close any remaining open containers within the document
        // The virtual root is at indent_stack[0], so close everything above it
        while self.indent_stack.len() > 1 {
            // If we're closing the mapping that owns a pending explicit key, give the
            // key its null first. `close_pending_explicit_key` owns that test.
            self.close_pending_explicit_key();
            self.indent_stack.pop();
            self.pop_type();
            self.write_bp_close();
        }

        self.in_document = false;
    }

    /// Skip `%`-directive lines (`%YAML 1.2`, `%TAG ...`, or any reserved
    /// directive) preceding a document's `---` marker (#225).
    ///
    /// Recognized only at column 0 while `!self.in_document` — the same
    /// gating condition the strict validator (`validate.rs`) uses. The
    /// directive's name and parameters are fully discarded: this loader is
    /// non-validating (see the module doc), so it does not need to
    /// distinguish `%YAML`/`%TAG` from a reserved directive like `%FOO` —
    /// all three are simply consumed here without emitting any content,
    /// which also means a misspelled directive name (e.g. `%YAM`, `%YAMLL`)
    /// is skipped exactly like a well-formed one, with no name matching at
    /// all.
    fn skip_directives(&mut self) {
        loop {
            self.skip_directive_gap_whitespace();
            if self.in_document || self.peek() != Some(b'%') {
                break;
            }
            self.skip_to_eol();
            self.skip_line_break();
        }
    }

    /// Skip blank lines (including tab-only ones) and comment lines between
    /// directives, or between a directive and the following `---` (`DK95/07`,
    /// #225).
    ///
    /// Deliberately its own copy of [`Self::skip_newlines`] rather than a
    /// shared call: a tab-only line here is blank, because there is no
    /// indentation to speak of in a pre-document gap, but `skip_newlines`'s
    /// other callers depend on a bare tab breaking immediately so
    /// `count_indent` can reject it as invalid indentation inside a
    /// document's actual content. Folding the two together silently stopped
    /// rejecting that case (`Y79Y/000`).
    fn skip_directive_gap_whitespace(&mut self) {
        while let Some(b) = self.peek() {
            if is_line_break(b) {
                self.skip_line_break();
            } else if b == b'#' {
                self.skip_to_eol();
            } else if b == b' ' || b == b'\t' {
                let start = self.pos;
                self.skip_inline_whitespace();
                if self.at_break() || self.peek() == Some(b'#') || self.peek().is_none() {
                    if self.peek() == Some(b'#') {
                        self.skip_to_eol();
                    }
                    continue;
                }
                self.pos = start;
                break;
            } else {
                break;
            }
        }
    }

    /// Parse a double-quoted string.
    ///
    /// Uses SIMD fast-path to skip to the next quote or backslash.
    fn parse_double_quoted(&mut self) -> Result<usize, YamlError> {
        let start = self.pos;
        self.advance(); // Skip opening quote

        loop {
            // SIMD fast-path: find next quote or backslash
            if let Some(offset) = simd::find_quote_or_escape(self.input, self.pos, self.input.len())
            {
                // Skip to the found character
                self.advance_by(offset);

                // Now process the found character
                match self.peek() {
                    Some(b'"') => {
                        self.advance();
                        return Ok(self.pos - start);
                    }
                    Some(b'\\') => {
                        self.advance(); // Skip backslash
                        if self.peek().is_some() {
                            self.advance(); // Skip escaped char
                        } else {
                            return Err(YamlError::UnexpectedEof {
                                context: "escape sequence in string",
                            });
                        }
                    }
                    _ => {
                        // Should not happen since we found quote or backslash
                        self.advance();
                    }
                }
            } else {
                // No quote or backslash found - string is unclosed
                return Err(YamlError::UnclosedQuote {
                    start_offset: start,
                    quote_type: '"',
                });
            }
        }
    }

    /// Parse a single-quoted string.
    ///
    /// Uses SIMD fast-path to skip to the next single quote.
    fn parse_single_quoted(&mut self) -> Result<usize, YamlError> {
        let start = self.pos;
        self.advance(); // Skip opening quote

        loop {
            // SIMD fast-path: find next single quote
            if let Some(offset) = simd::find_single_quote(self.input, self.pos, self.input.len()) {
                // Skip to the found quote
                self.advance_by(offset);

                // Check for escaped quote ('')
                if self.peek_at(1) == Some(b'\'') {
                    self.advance();
                    self.advance();
                } else {
                    self.advance();
                    return Ok(self.pos - start);
                }
            } else {
                // No quote found - string is unclosed
                return Err(YamlError::UnclosedQuote {
                    start_offset: start,
                    quote_type: '\'',
                });
            }
        }
    }

    /// Parse an unquoted scalar value with a minimum indentation requirement.
    /// Handles multiline plain scalars - continues on lines more indented than start_indent.
    /// When `is_doc_root` is true, same-indent lines continue the scalar (YAML spec 7.4).
    fn parse_unquoted_value_with_indent(&mut self, start_indent: usize) -> usize {
        self.parse_unquoted_value_with_indent_impl(start_indent, false)
    }

    /// Parse an unquoted scalar at document root level.
    /// At document root, same-indent lines continue the scalar (YAML spec 7.4).
    fn parse_unquoted_value_doc_root(&mut self, start_indent: usize) -> usize {
        self.parse_unquoted_value_with_indent_impl(start_indent, true)
    }

    fn parse_unquoted_value_with_indent_impl(
        &mut self,
        start_indent: usize,
        is_doc_root: bool,
    ) -> usize {
        let start = self.pos;
        // Track the actual end of content (before newlines we skip)
        let mut content_end = start;

        loop {
            let line_start = self.pos;
            // Parse content on current line
            // Use inline scalar loop for common case, SIMD for long runs
            while let Some(b) = self.peek() {
                match b {
                    b'#' => {
                        // # is only a comment if preceded by whitespace (space or tab)
                        if self.pos > start && matches!(self.input[self.pos - 1], b' ' | b'\t') {
                            break;
                        }
                        self.advance();
                    }
                    b':' => {
                        // Colon followed by whitespace, a line break, or EOF ends
                        // the value (could be a key). In value context, colons in
                        // URLs etc. are allowed. The EOF case matters: without
                        // it, a colon as the last byte of the document (no trailing
                        // newline) was absorbed as content instead of terminating
                        // the value, while `find_scalar_end` (the locate-path copy
                        // of this same boundary) already stopped there - eval and
                        // locate disagreed on the same node (#434, same shape as
                        // #370).
                        if Self::is_ws_break_or_eoi(self.peek_at(1)) {
                            break;
                        }
                        self.advance();
                    }
                    b if Self::is_break(b) => break,
                    _ => {
                        // SIMD/broadword fast-path: skip long runs of regular characters
                        // Only use SIMD if we have enough remaining bytes to justify overhead
                        #[cfg(all(target_arch = "x86_64", not(feature = "scalar-yaml")))]
                        if self.input.len() - self.pos >= 32 {
                            if let Some(skip) = self.skip_unquoted_simd(start) {
                                self.advance_by(skip);
                                continue;
                            }
                        }
                        // ARM64 broadword disabled - see P4 analysis in docs/parsing/yaml.md
                        // #[cfg(target_arch = "aarch64")]
                        // if self.input.len() - self.pos >= 16 {
                        //     if let Some(skip) = self.skip_unquoted_simd(start) {
                        //         self.advance_by(skip);
                        //         continue;
                        //     }
                        // }
                        self.advance();
                    }
                }
            }

            // Only update content_end if we parsed content on this line
            // (Skip if we just returned from an empty line continuation)
            if self.pos > line_start {
                content_end = self.pos;
            }

            // Check if we can continue to next line
            if !self.at_break() {
                break;
            }

            // Look ahead to see if next line is a continuation
            let mut lookahead = self.pos + self.break_len_at(self.pos); // Skip the line break
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
            if Self::is_break(next_char) || next_char == b'\t' {
                // Check if rest of line is whitespace
                let mut check_pos = lookahead;
                while check_pos < self.input.len() && matches!(self.input[check_pos], b' ' | b'\t')
                {
                    check_pos += 1;
                }
                if check_pos >= self.input.len() || Self::is_break(self.input[check_pos]) {
                    // Empty line - skip it and continue
                    self.skip_line_break(); // Skip the current line break
                                            // Skip to end of empty line
                    while matches!(self.peek(), Some(b' ' | b'\t')) {
                        self.advance();
                    }
                    continue;
                }
                // Tab followed by content - for document root scalars, this is
                // a continuation per YAML spec example 7.12 "Plain Lines". The
                // tabs become part of the folded content (converted to space).
                //
                // Gated on `is_doc_root`, not `start_indent == 0`: a mapping
                // value or sequence item at indent 0 also has `start_indent ==
                // 0` but is not the document root, so a tab there indents
                // whatever structure follows and must not be folded into
                // content (#371, #432).
                if is_doc_root && next_char == b'\t' {
                    // Continue to next line - this is a valid continuation
                    self.skip_line_break();
                    // Skip leading whitespace (tabs are content, but we're at the scalar's level)
                    while matches!(self.peek(), Some(b' ' | b'\t')) {
                        self.advance();
                    }
                    continue;
                }
                // Reaching here means `next_char == b'\t'` and `is_doc_root` is
                // false (the two branches above already `continue`d for every
                // other combination). This scan works off local `lookahead`/
                // `next_indent` variables rather than the cursor, so it can't
                // reuse `tab_indents_block_structure` directly - `line_is_structural`
                // is the shared primitive underneath both. Without this check the
                // generic "more indented, so continue" rule below only compares
                // space-counts and can't see the tab, so it folds whatever
                // structure follows (a sequence item, mapping entry, ...) into
                // the scalar instead of stopping so the per-line dispatcher can
                // reject the tab as indentation (#371, #432).
                if super::line_is_structural(self.input, lookahead) {
                    break;
                }
            }

            // Continuation requires more indent than where scalar started,
            // EXCEPT at document root (start_indent == 0) where same-indent is allowed.
            // Next line shouldn't start block structure or be a comment.
            //
            // A `- ` on a continuation line is ordinary scalar content at ANY indent
            // greater than start_indent - per YAML 1.2 `nb-ns-plain-in-line`, once a
            // plain scalar's first line has begun, a leading `-` on a later line is
            // never re-tested as a sequence indicator. It's only block structure when
            // it's at or before start_indent, which (since indent_allows_continuation
            // already requires next_indent > start_indent outside doc-root) can only
            // happen at document root, where it means a `- ` reappearing at column 0
            // is a genuine new top-level sequence item, not scalar content.
            //
            // AB8U (`next_indent == start_indent + 1`) is just the narrowest case of
            // "greater than start_indent" - it was once mishandled as the only correct
            // one, wrongly stopping continuation for any deeper indent too (#484).
            let is_sequence_indicator = next_char == b'-'
                && lookahead + 1 < self.input.len()
                && matches!(self.input[lookahead + 1], b' ' | b'\t');
            let sequence_indicator_is_block_structure =
                is_sequence_indicator && next_indent <= start_indent;

            // A `---`/`...` document marker at true document root must end the
            // scalar rather than fold into it as content (#225): an implicit
            // first document with no explicit leading `---` (a bare scalar
            // like `Document`) would otherwise swallow the marker that starts
            // or ends the next document. Scoped to `is_doc_root` only - inside
            // a container, `indent_allows_continuation` already requires
            // `next_indent > start_indent`, which a column-0 marker can't
            // satisfy, so this never fires there.
            let is_document_marker = is_doc_root
                && next_indent == 0
                && lookahead + 2 < self.input.len()
                && matches!(&self.input[lookahead..lookahead + 3], b"---" | b"...")
                && matches!(
                    self.input.get(lookahead + 3),
                    Some(b' ' | b'\t' | b'\n' | b'\r') | None
                );

            // At document root, same-indent continues the scalar (YAML spec 7.4).
            // Inside containers, must be more indented than start.
            let indent_allows_continuation = is_doc_root || next_indent > start_indent;

            if indent_allows_continuation
                && next_char != b'#'
                && !sequence_indicator_is_block_structure
                && !is_document_marker
                && !(next_char == b':'
                    && Self::is_ws_break_or_eoi(self.input.get(lookahead + 1).copied()))
            {
                // Continue to next line
                self.skip_line_break();
                // Skip leading whitespace
                while matches!(self.peek(), Some(b' ' | b'\t')) {
                    self.advance();
                }
            } else {
                // Not a continuation - stop here
                break;
            }
        }

        // Trim trailing whitespace from content_end. `\r` is a line break, never
        // scalar content, so it is trimmed too: the SIMD classifier can skip a
        // CR to land on the LF that follows it, leaving the CR inside the
        // extent, which is what made `a: 1\r\n` resolve as the string `"1 "`
        // rather than the number 1 (#324).
        let mut end = content_end;
        while end > start && matches!(self.input[end - 1], b' ' | b'\t' | b'\r') {
            end -= 1;
        }

        // Return absolute end position (not length)
        end
    }

    /// Parse an unquoted key (stops at colon+space).
    fn parse_unquoted_key(&mut self) -> Result<usize, YamlError> {
        let start = self.pos;

        while let Some(b) = self.peek() {
            match b {
                b':' => {
                    // Check for colon + whitespace or colon + line break
                    if Self::is_ws_break_or_eoi(self.peek_at(1)) {
                        break;
                    }
                    // Colon not followed by whitespace is part of the key
                    // (e.g., "key::" or URLs like "http://example.com")
                    self.advance();
                }
                b'#' => {
                    // # starts a comment only after s-separate-in-line, and
                    // s-white is a space *or* a tab — the same set #370 fixed
                    // for the trailing trim below (#410). Otherwise it's part
                    // of the key (e.g., "a#b: value").
                    if self.pos > start && matches!(self.input[self.pos - 1], b' ' | b'\t') {
                        return Err(YamlError::KeyWithoutValue {
                            offset: start,
                            line: self.current_line(),
                        });
                    }
                    self.advance();
                }
                b if Self::is_break(b) => {
                    // Key without colon
                    return Err(YamlError::KeyWithoutValue {
                        offset: start,
                        line: self.current_line(),
                    });
                }
                _ => {
                    // SIMD/broadword fast-path: skip long runs of regular characters
                    #[cfg(all(target_arch = "x86_64", not(feature = "scalar-yaml")))]
                    if self.input.len() - self.pos >= 32 {
                        if let Some(skip) = self.skip_unquoted_simd(start) {
                            self.advance_by(skip);
                            continue;
                        }
                    }
                    // ARM64 broadword disabled - see P4 analysis in docs/parsing/yaml.md
                    // #[cfg(target_arch = "aarch64")]
                    // if self.input.len() - self.pos >= 16 {
                    //     if let Some(skip) = self.skip_unquoted_simd(start) {
                    //         self.advance_by(skip);
                    //         continue;
                    //     }
                    // }
                    self.advance();
                }
            }
        }

        // Trim trailing whitespace. `\r` goes with it for the same reason as in
        // `parse_unquoted_value_with_indent_impl`: it is a line break, so it is
        // never part of the key (#324). A tab goes with it because the white
        // space between a key and its `:` is `s-separate-in-line`, which
        // `ns-s-implicit-yaml-key` leaves outside the key — the same reason a
        // trailing space is trimmed, and this set is why `a\t: 1` used to load
        // as {"a\t":1} (#370).
        let mut end = self.pos;
        while end > start && matches!(self.input[end - 1], b' ' | b'\t' | b'\r') {
            end -= 1;
        }

        // Empty key is valid in YAML (e.g., `: value`)
        // Return absolute end position
        Ok(end)
    }

    /// Close containers that are at higher indent levels.
    fn close_deeper_indents(&mut self, new_indent: usize) {
        while self.indent_stack.len() > 1 {
            let current_indent = *self.indent_stack.last().unwrap();
            // Only close containers that are DEEPER than the new indent.
            // Containers at the same level should stay open so new entries
            // can be added to them.
            if current_indent > new_indent {
                // If we're closing the mapping that owns a pending explicit key, give
                // the key its null first. `close_pending_explicit_key` owns that test —
                // testing `current_type == Mapping` here would fire on a complex key
                // that is itself a mapping (`? k: v`), which is not the owner.
                self.close_pending_explicit_key();
                self.indent_stack.pop();
                self.pop_type();
                self.write_bp_close();
            } else {
                break;
            }
        }
    }

    /// Whether a sequence frame at `indent_stack[frame_idx]` should still hold
    /// an item at `indent`: either an exact indent match (the ordinary case)
    /// or, leniently, an out-of-range indent strictly between the sequence
    /// and whatever encloses it.
    ///
    /// A block sequence continuation at such an indent is invalid YAML (`yq`
    /// and the strict validator both reject it), but closing the sequence
    /// here — the plain `close_deeper_indents` behaviour — pops it back to
    /// its enclosing mapping and then reopens a second, untagged sequence as
    /// a sibling *child* of that mapping instead of a value under any key.
    /// That extra child throws off the mapping's key/value pairing for
    /// everything after it, silently dropping the item and corrupting the
    /// next entry into a phantom empty key (#485). Treating the out-of-range
    /// indent as still belonging to the open sequence avoids that, the same
    /// "parse the obvious extension" policy #325 used for `a: - x`.
    ///
    /// Shared by `close_deeper_indents_for_sequence_item` (decides whether to
    /// stop closing) and `parse_sequence_item_inner` (decides whether to
    /// reuse the sequence rather than opening a new one) so the two
    /// can't drift out of sync (the #106 lesson: duplicated predicates
    /// diverge silently).
    fn sequence_frame_reaches(&self, frame_idx: usize, indent: usize) -> bool {
        let frame_indent = self.indent_stack[frame_idx];
        indent == frame_indent
            || (frame_idx > 0 && indent > self.indent_stack[frame_idx - 1] && indent < frame_indent)
    }

    /// Close deeper containers before parsing a `-` sequence item at
    /// `new_indent`, tolerating an out-dented continuation via
    /// [`Self::sequence_frame_reaches`] instead of popping the sequence
    /// (#485).
    ///
    /// Only `parse_sequence_item_inner` uses this variant of
    /// `close_deeper_indents`: the tolerance is specific to the sequence
    /// getting a new *item*, and does not generalize to other callers (a
    /// mapping key at the same out-of-range indent has no sequence-item
    /// wrapper to land in, and would produce a different, wrong shape).
    fn close_deeper_indents_for_sequence_item(&mut self, new_indent: usize) {
        while self.indent_stack.len() > 1 {
            let top_idx = self.indent_stack.len() - 1;
            let current_indent = self.indent_stack[top_idx];
            if current_indent <= new_indent {
                break;
            }
            if self.type_stack[top_idx] == NodeType::Sequence
                && self.sequence_frame_reaches(top_idx, new_indent)
            {
                // Out-dented continuation - keep this sequence open.
                break;
            }
            self.close_pending_explicit_key();
            self.indent_stack.pop();
            self.pop_type();
            self.write_bp_close();
        }
    }

    /// Shared shape behind [`Self::compact_mapping_gap_reaches`],
    /// [`Self::sequence_item_gap_reaches`], and
    /// [`Self::mapping_under_mapping_gap_reaches`]: `indent` falls between
    /// the top two stack frames' own recorded indents, with the top frame
    /// of type `child` directly on top of a frame of type `parent`. One
    /// definition so the three call sites' bound-inclusivity and
    /// frame-type choices can't silently drift apart from each other (the
    /// #106 lesson — duplicated predicates diverge silently).
    fn frame_gap_reaches(
        &self,
        indent: usize,
        parent: NodeType,
        child: NodeType,
        inclusive_lower: bool,
        inclusive_upper: bool,
    ) -> bool {
        let len = self.indent_stack.len();
        if len < 2 || self.type_stack[len - 1] != child || self.type_stack[len - 2] != parent {
            return false;
        }
        let lower = self.indent_stack[len - 2];
        let upper = self.indent_stack[len - 1];
        let lower_ok = if inclusive_lower {
            indent >= lower
        } else {
            indent > lower
        };
        let upper_ok = if inclusive_upper {
            indent <= upper
        } else {
            indent < upper
        };
        lower_ok && upper_ok
    }

    /// Whether `indent` falls anywhere from an open compact mapping's own
    /// recorded indent down through its enclosing sequence item's virtual
    /// indent (inclusive on both ends) — e.g. `-   a: hello\n  b: 2\n`, where
    /// the compact mapping opened by `a` sits at indent 4 (the real column
    /// of `a`) but `b`'s line is indented only 2, between that and the
    /// item's own virtual indent 1.
    ///
    /// Real yq rejects this input outright as inconsistently indented, but
    /// the plain indent comparison `close_deeper_indents`/`parse_mapping_entry`
    /// would otherwise apply treats it as *closing* the compact mapping and
    /// opening a second, untagged mapping as a sibling directly under the
    /// same `SequenceItem` — which structurally expects only one value
    /// child, so every field after the first inconsistent line is silently
    /// dropped by the JSON serializer (#885).
    ///
    /// The lower bound is inclusive (`>=`, not `>`) because the ordinary,
    /// single-space-after-dash case (the common real-world shape, not just
    /// this file's extra-spaced headline repro) lands exactly *at* the
    /// item's own virtual indent — `close_deeper_indents` never closes a
    /// frame at an indent equal to its own recorded value elsewhere in this
    /// file either (`current_indent > new_indent` gates every close), so
    /// treating equality as "still inside" here is consistent with that,
    /// not a special case invented for this function.
    ///
    /// Deliberately narrow to a `Mapping` frame directly on top of a
    /// `SequenceItem` frame: in that shape there's exactly one possible
    /// enclosing scope for the line to belong to, which is what makes this
    /// tolerance unambiguous. Two known, deliberately out-of-scope siblings
    /// of this same failure shape, not yet fixed:
    /// - A `-` continuation landing in the same gap (#900) — unlike a
    ///   `key: value` line, a bare item has no obvious "add it to the open
    ///   mapping" interpretation.
    /// - Mapping-under-mapping nesting (#901) — needs its own correctness
    ///   argument for whether the same reasoning still holds when the frame
    ///   below isn't a `SequenceItem` (and thus not guaranteed to be the
    ///   line's only possible enclosing scope) before extending this check
    ///   to it.
    fn compact_mapping_gap_reaches(&self, indent: usize) -> bool {
        self.frame_gap_reaches(
            indent,
            NodeType::SequenceItem,
            NodeType::Mapping,
            true,
            true,
        )
    }

    /// Normalize `indent` to the open compact mapping's own recorded indent
    /// when [`Self::compact_mapping_gap_reaches`] holds, so
    /// `parse_mapping_entry` adds an entry to that mapping instead of
    /// closing it — the same normalization `parse_sequence_item_inner`
    /// already does for an out-dented sequence continuation (#485). A no-op
    /// otherwise.
    fn resolve_compact_mapping_gap_indent(&self, indent: usize) -> usize {
        if self.compact_mapping_gap_reaches(indent) {
            *self.indent_stack.last().unwrap()
        } else {
            indent
        }
    }

    /// Whether an explicit value's (already #885-gap-resolved) `indent`
    /// matches the mapping frame opened by its own pending `?`, if one is
    /// pending (#1010). YAML ties both the explicit-key and explicit-value
    /// productions to the same indentation parameter `n`
    /// (c-l-block-map-explicit-key/value(n)), so any deviation -- not just a
    /// dedent -- is ambiguous. This is a different shape than the dedent
    /// ambiguity [`Self::mapping_under_mapping_gap_reaches`] detects: that
    /// predicate's `indent >= indent_stack[top]` short-circuit is correct for
    /// `parse_mapping_entry`'s "deeper indent opens a new nested mapping"
    /// semantics, but would wrongly wave through an over-indented `:` here,
    /// since a `:` value line never opens a new frame the way an ordinary
    /// `key: value` line does. Confirmed live against real yq: `? k\n : v\n`
    /// (`:` one column past `?`) and deeper misalignments both error ("did
    /// not find expected key"); only exact alignment, or no pending key at
    /// all (a bare `: value` with no `?`, a separate and pre-existing
    /// out-of-scope shape), passes.
    ///
    /// `owner_depth` is `parse_explicit_key`'s own `indent_stack.len()`,
    /// captured immediately after the owning mapping frame is opened/reused
    /// and before any key-content parsing that could push extra frames (a
    /// sequence or compact-mapping key) -- so `indent_stack[owner_depth - 1]`
    /// always resolves to the owner's own recorded indent, never a deeper
    /// frame the key's own content pushed. `close_pending_explicit_key`
    /// clears `pending_explicit_key` before that frame can ever be popped
    /// (it only fires when `indent_stack.len() == owner_depth` exactly), so
    /// `owner_depth - 1` is always in bounds whenever this observes `Some`.
    fn explicit_value_matches_key_indent(&self, indent: usize) -> bool {
        match self.pending_explicit_key {
            Some(owner_depth) => {
                debug_assert!(
                    owner_depth >= 1 && owner_depth <= self.indent_stack.len(),
                    "pending_explicit_key must name a frame still on indent_stack"
                );
                self.indent_stack[owner_depth - 1] == indent
            }
            None => true,
        }
    }

    /// The #901 sibling of [`Self::compact_mapping_gap_reaches`]: whether
    /// `indent` would land ambiguously if [`Self::close_deeper_indents`]
    /// popped every frame deeper than it -- the surviving (landing) frame's
    /// own recorded indent doesn't exactly match `indent`, so the line
    /// belongs to neither the frame(s) that would be popped nor the one it
    /// lands in unambiguously.
    ///
    /// Originally scoped to only the top two stack frames (both `Mapping`)
    /// — #958 found the identical silent-data-loss shape still reproduced,
    /// unfixed, whenever the ambiguity wasn't between an *adjacent* pair:
    /// a still-open intervening frame of any type (e.g. a `Sequence`
    /// deferred as a mapping value) masked the top-two check entirely, and
    /// a non-adjacent ancestor 3+ levels up was never examined since the
    /// check only ever looked at `len-1`/`len-2`. Generalized here to walk
    /// the whole stack instead, mirroring `close_deeper_indents`'s own
    /// popping condition (`current_indent > new_indent`) exactly rather
    /// than re-deriving a narrower approximation of it — so the two can't
    /// silently drift apart the way #106 warns about.
    ///
    /// Index 0 is the permanent virtual-root sentinel (`indent_stack[0]`
    /// set to `usize::MAX` in `parse`, never popped) — landing there is
    /// still correctly flagged whenever `indent` doesn't match it (which
    /// no real indent ever can), not specially exempted: a dedent past an
    /// indented top-level document's own established indentation is
    /// itself an error in real YAML (confirmed live: `  a: 1\n  b: 2\nc:
    /// 3\n` raises "did not find expected <document start>"), and the
    /// walk never legitimately reaches index 0 for the ordinary "indented
    /// top-level document, exact-match sibling" case (`  a: 1\n  b: 2\n`)
    /// -- that stops at the earlier `indent >= indent_stack[top]`
    /// short-circuit instead, since a sibling's indent always matches the
    /// top-level frame's own recorded indent exactly.
    ///
    /// Deliberately not restricted to a `Mapping` landing frame: verified
    /// live that real yq rejects a mapping-entry-shaped line landing in an
    /// *open sequence's* gap the same way (`a:\n  - x\n c: 1\n` errors,
    /// `did not find expected key`) — succinctly had the identical
    /// silent-data-loss bug there too before this fix, unrelated to
    /// #901/#958's own named repros but the same root cause. An ordinary
    /// sibling key landing exactly at a closed sequence's own indent
    /// (`a:\n  - x\n  - y\nb: 1\n`) stays unaffected, since it matches the
    /// landing frame's indent exactly.
    ///
    /// `for_sequence_item` must be `true` only for the `-` dispatch arm in
    /// `parse_block_node`, which closes through
    /// [`Self::close_deeper_indents_for_sequence_item`] rather than plain
    /// `close_deeper_indents` — that closer has its own additional
    /// tolerance (#325/#485: an out-dented `-` continuation of an *open
    /// sequence*, checked via [`Self::sequence_frame_reaches`] at each
    /// frame the walk would otherwise pop through), which a `key: value`
    /// line landing in the same gap does **not** share (confirmed live:
    /// `a:\n  - x\n c: 1\n` errors in real yq, but `a:\n  - x\n - y\n`
    /// -- a `-` continuation at the identical out-of-range indent --
    /// parses cleanly). Every other call site passes `false`.
    fn mapping_under_mapping_gap_reaches(&self, indent: usize, for_sequence_item: bool) -> bool {
        if self.indent_stack.len() < 2 {
            return false;
        }
        // Defer to the top-two-frame-specific tolerances (#885's compact
        // mapping continuation, #900's sequence-item continuation) when
        // either applies: both are deliberate, oracle-verified "obvious
        // extension" readings for a SequenceItem/Mapping pairing
        // specifically (see their own doc comments), normalized by their
        // own callers just after this check runs -- not a case with no
        // valid reading at all, the way every other gap this function
        // catches is. Checked before the walk below since this predicate
        // is now general enough to otherwise also flag them (it no longer
        // restricts itself to a Mapping-under-Mapping frame pairing).
        if self.compact_mapping_gap_reaches(indent) || self.sequence_item_gap_reaches(indent) {
            return false;
        }
        let top = self.indent_stack.len() - 1;
        // `close_deeper_indents`/`close_deeper_indents_for_sequence_item`
        // only ever pop (current_indent > new_indent) -- going deeper or
        // staying flat at the current top frame never pops anything, so
        // there's no landing-frame question to ask at all: it's either a
        // new nested frame (handled by the caller opening one) or an
        // ordinary sibling of the frame already open, both fine.
        if indent >= self.indent_stack[top] {
            return false;
        }
        let mut landing_idx = top;
        while landing_idx > 0 && self.indent_stack[landing_idx] > indent {
            // #325/#485's out-dented-sequence-continuation tolerance,
            // mirrored from `close_deeper_indents_for_sequence_item`'s own
            // per-frame check -- only reachable from the `-` dispatch arm.
            if for_sequence_item
                && self.type_stack[landing_idx] == NodeType::Sequence
                && self.sequence_frame_reaches(landing_idx, indent)
            {
                return false;
            }
            landing_idx -= 1;
        }
        self.indent_stack[landing_idx] != indent
    }

    /// Shared `Err` construction for every indentation-ambiguity predicate in
    /// this cluster of the file (#106: one definition instead of each call
    /// site re-deriving it) -- `ok` is the specific predicate's own verdict;
    /// this only owns the error's shape, at the current position.
    fn require_consistent_indentation(&self, ok: bool) -> Result<(), YamlError> {
        if ok {
            Ok(())
        } else {
            Err(YamlError::InconsistentIndentation {
                offset: self.pos,
                line: self.current_line(),
            })
        }
    }

    /// Check-and-error wrapper around [`Self::mapping_under_mapping_gap_reaches`],
    /// via [`Self::require_consistent_indentation`] -- the same check-and-error,
    /// `?`-friendly shape [`Self::reject_trailing_flow_content`] already uses
    /// for an unrelated check in this file. `for_sequence_item` forwards to
    /// [`Self::mapping_under_mapping_gap_reaches`] -- see its own doc
    /// comment for which call site must pass `true`.
    fn check_mapping_under_mapping_gap(
        &self,
        indent: usize,
        for_sequence_item: bool,
    ) -> Result<(), YamlError> {
        self.require_consistent_indentation(
            !self.mapping_under_mapping_gap_reaches(indent, for_sequence_item),
        )
    }

    /// Whether a `-` sequence-item line's `indent` falls in the *lower*
    /// portion of the gap [`Self::compact_mapping_gap_reaches`] detects
    /// (#900) -- deliberately **excluding** the upper bound (`indent`
    /// exactly matching the open compact mapping's own indent), unlike that
    /// function's inclusive range.
    ///
    /// The exact-match indent is genuinely ambiguous rather than a case with
    /// no valid reading at all: `parse_compact_mapping_entry` leaves the
    /// mapping's key value deferred specifically when the *next* line is a
    /// `-` at that exact indent, so the ordinary `need_new_sequence` logic
    /// below can open the sequence as that key's own value (`- k: &a\n  -
    /// 1\n` -> `{k: [1]}`, `test_compact_entry_trailing_anchor_targets_its_collection`)
    /// -- a real, already-correct, already-tested YAML shape ("a block
    /// sequence may sit at its key's own indent"), not a bug. Whether that
    /// key is deferred or already resolved is not visible from
    /// `indent_stack`/`type_stack` alone, so this predicate can't
    /// distinguish an exact-match orphaned `-` (still a bug, just not fixed
    /// here) from the legitimate deferred-value case -- excluding the exact
    /// match entirely is what keeps this fix from firing on the latter.
    /// Every indent strictly below the mapping's own has no such reading
    /// (confirmed against the real, deferred-value-priority behavior above):
    /// `parse_compact_mapping_entry` only takes the "leave deferred" branch
    /// for an exact indent match, so a lesser indent already resolves the
    /// key (to `null`) before this is even reached, making it structurally
    /// identical to the already-resolved-key case #900 reports.
    fn sequence_item_gap_reaches(&self, indent: usize) -> bool {
        self.frame_gap_reaches(
            indent,
            NodeType::SequenceItem,
            NodeType::Mapping,
            true,
            false,
        )
    }

    /// Normalize a `-` sequence-item line's `indent` when
    /// [`Self::sequence_item_gap_reaches`] holds (#900), to the recorded
    /// indent of the *outer* sequence -- the frame two levels below the
    /// open compact mapping (`[.., Sequence, SequenceItem, Mapping]`,
    /// always present: a `SequenceItem` is only ever pushed onto an
    /// already-open `Sequence`, so indexing `len - 3` here can't panic).
    ///
    /// Feeding this normalized value into `close_deeper_indents_for_sequence_item`
    /// and the `need_new_sequence`/`sequence_frame_reaches` check below closes
    /// through both the compact mapping and its enclosing item and reuses the
    /// outer sequence, instead of the plain indent comparison's behavior:
    /// closing only the mapping, then opening a *second*, untagged sequence
    /// nested inside the still-open item -- which the JSON serializer treats
    /// as an extra, ignored child, silently dropping this item (#900). A
    /// no-op otherwise.
    fn resolve_sequence_item_gap_indent(&self, indent: usize) -> usize {
        if self.sequence_item_gap_reaches(indent) {
            self.indent_stack[self.indent_stack.len() - 3]
        } else {
            indent
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
                self.pop_type();
                self.write_bp_close();

                // The sequence just closed may itself have been the
                // pending explicit key's own implicit value (`? k\n-
                // item\n`, at the same indent as the key). Clear the
                // stale flag now that the key's real value is fully
                // closed -- unlike every other pop site's
                // `close_pending_explicit_key()` call, no null is
                // synthesized here: the key already received a real
                // value, so this isn't the "encountered without an
                // explicit value" case that helper handles. Left stale,
                // a later mapping entry at this same indent would be
                // silently misattributed to the closed key instead of
                // starting fresh (#1040). Shares
                // `pending_explicit_key_owns_current_frame`'s predicate
                // with `close_pending_explicit_key` rather than
                // re-deriving it here (#106).
                if self.pending_explicit_key_owns_current_frame() {
                    self.pending_explicit_key = None;
                }
            }
        }
    }

    /// Parse a sequence item (starts with `- `).
    fn parse_sequence_item(&mut self, indent: usize) -> Result<(), YamlError> {
        self.enter_nested()?;
        let result = self.parse_sequence_item_inner(indent);
        self.nesting_depth -= 1;
        result
    }

    fn parse_sequence_item_inner(&mut self, indent: usize) -> Result<(), YamlError> {
        // A `-` landing strictly below an open compact mapping's own indent
        // (but still within its enclosing sequence item's range) has no
        // "just add it to the open compact mapping" interpretation the way
        // a `key: value` continuation does (#885) -- a bare sequence-item
        // marker can't become a mapping entry. The obvious extension here
        // instead closes through both the compact mapping and its enclosing
        // sequence item -- unambiguous, since a `SequenceItem` always has
        // exactly one enclosing `Sequence` -- and treats this as the next
        // item of that *outer* sequence, the same "parse the obvious
        // extension" policy #325/#485/#885 already use elsewhere. See
        // `sequence_item_gap_reaches` for why the upper bound is excluded
        // (#900).
        let indent = self.resolve_sequence_item_gap_indent(indent);

        let _item_start = self.pos;

        // Mark the `-` position
        self.set_ib();

        // First close any deeper containers. This might reveal an existing sequence
        // at this indent level that we can reuse. Uses the sequence-item-aware
        // variant so an out-dented continuation reuses the open sequence instead
        // of being closed out from under it (#485).
        self.close_deeper_indents_for_sequence_item(indent);

        // Now check if we need to open a new sequence (check AFTER closing)
        // Normally, sequence items must be at the exact same indent as the sequence.
        // However, for nested sequences created by `- - item` pattern, items can be
        // at greater indent because the nested sequence's indent is virtual. And an
        // out-dented continuation (#485) can be at *lesser* indent, as long as it's
        // still within the range `close_deeper_indents_for_sequence_item` tolerated.
        //
        // We need a new sequence if there's no sequence on the stack, or the item
        // indent doesn't fall within the current sequence's range (exact match, or
        // the same out-of-range gap `sequence_frame_reaches` already tolerated).
        let need_new_sequence = self.current_type != Some(NodeType::Sequence)
            || !self.sequence_frame_reaches(self.indent_stack.len() - 1, indent);

        if need_new_sequence {
            // Open new sequence
            self.write_bp_open();
            self.write_ty(true); // 1 = sequence
            self.indent_stack.push(indent);
            self.push_type(NodeType::Sequence);
        }

        // Normalize to the sequence's own recorded indent for everything below:
        // the item's virtual child indent, a compact mapping's indent, and the
        // structure indent passed to `parse_value`. A no-op for the ordinary
        // exact-match case and for a sequence just opened above (both already
        // equal `indent`), but load-bearing for an out-dented continuation that
        // reused the sequence (#485) - without it, a later sibling line whose
        // indent falls between this out-dented `indent` and the sequence's real
        // indent would look like "more indented than this item's own value" and
        // fold into it as scalar continuation text instead of being recognized
        // as the next sequence item.
        let indent = *self.indent_stack.last().unwrap();

        // Open the sequence item node
        self.write_bp_open_seq_item();
        // Deliberately not a `take_pending_head_comment` call site (#784):
        // this wrapper's own bp is never what the emitter reads for a
        // trailing comment on this line — a plain-scalar item's line
        // comment lives on the *scalar's* own bp (see the `take_pending_head_comment`
        // call after `parse_value` below), and any other continuation
        // (compact-mapping key, a nested `parse_mapping_entry`/
        // `parse_sequence_item` reached via a fresh line dispatch) claims it
        // at its own, already-instrumented key/wrapper-open site instead.

        // Skip `- `
        self.advance(); // -
        self.skip_inline_whitespace();

        // A node property (`&anchor` and/or `!tag`, either order) prefixes the
        // item's value, so consume it here — before the dispatch below, which
        // would otherwise test its predicates against the property text
        // instead of the value and mis-classify the item (#328; tags follow
        // the same rule, #224).
        //
        // The exception is `- &a k: v` / `- !!str k: v`, where the property
        // binds to the *key* of the compact mapping (matching yq); that shape
        // is left for parse_compact_mapping_entry to record.
        let prefixed_item = matches!(self.peek(), Some(b'&' | b'!'))
            && !self.node_properties_prefix_mapping_entry();
        let had_property = if prefixed_item {
            self.parse_node_properties()?
        } else {
            false
        };

        // Track the sequence item on the stack so close_deeper_indents can close it.
        // We use indent + 1 as a virtual indent - any content at indent > indent
        // is considered part of this item.
        //
        // NOTE: This means for `- foo`, the item is at virtual indent 1, so content
        // at indent 2 would be part of the item. But the sequence itself is at indent 0.
        self.indent_stack.push(indent + 1);
        self.push_type(NodeType::SequenceItem);

        // Check what follows
        if self.at_line_end() {
            // A trailing comment here belongs to this item's own deferred
            // value, not to the item's dash line (#784, same shape as
            // `parse_mapping_entry`'s anchor case) - only when there was a
            // property to defer it past; a bare `- # comment` has no anchor
            // to blame the deferral on and is a separate, untouched gap.
            if had_property {
                self.defer_line_comment();
            }

            // An anchor records the *next* BP position as its target, and a
            // tag is looked up *at* that position (`self.tags`), so a
            // property-prefixed item whose value turns out to be null needs
            // an explicit empty node for either to resolve against —
            // otherwise the anchor/tag would land on a sibling's open, on a
            // close bit, or (for a tag) nowhere at all, silently dropping it
            // (corpus case LE5A: `- !!str` with nothing after must resolve
            // to `""`, not `null` — #224). parse_mapping_entry does the same
            // for `key: &a` / `key: !!str` with no value.
            if had_property && self.following_value_is_null(indent) {
                self.set_ib();
                self.write_bp_open();
                self.write_bp_close();
            }

            // Content is on the next line(s) at greater indentation.
            // Leave the item open - subsequent content at indent > this item's
            // indent will be parsed as the item's value. The item will be closed
            // by close_deeper_indents when we see content at indent <= sequence indent.
            return Ok(());
        }

        // Check for nested sequence: `- - item` (sequence item containing a sequence)
        if self.peek() == Some(b'-') && Self::is_ws_break_or_eoi(self.peek_at(1)) {
            // Nested sequence - the item value is another sequence.
            // Use the actual column position of the nested `-` as the indent.
            // This ensures that subsequent items at the same column (like `- d` after `- c`)
            // will be correctly recognized as siblings in the same sequence.
            let nested_indent = self.current_column();
            self.parse_sequence_item(nested_indent)?;
            // Don't close the outer item - it will be closed when we return
            // to a lower indent level.
        } else if self.peek() == Some(b'?')
            // Tab included: the sibling `-` check just above uses the same
            // 4-way terminator set; this one dropped the tab, so `?\tkey`
            // fell through to being parsed as a plain scalar instead of an
            // explicit key (#434).
            && matches!(self.peek_at(1), Some(b' ' | b'\t' | b'\n' | b'\r') | None)
        {
            // Explicit key as the item's value: `- ? k` / `  : v` (#339).
            //
            // Routed through the same `parse_explicit_key` the mapping-level
            // dispatch uses rather than a fourth copy of the decision (#106) —
            // that path is already correct at top level and in mapping-value
            // position, and reaching it is the whole fix. Without this arm the
            // `?` reaches `parse_value` as a plain scalar and the `: v` line
            // becomes a phantom sibling element.
            //
            // Ordered *before* `looks_like_mapping_entry`, matching the main
            // loop's dispatch order in `parse_document_line`: the `?` indicator
            // wins over a `: ` later on the line.
            //
            // `current_column()` — the `?`'s own column — is the mapping's
            // indent, not `indent + 2`, so `-   ? k` / `    : v` lines up. It is
            // always >= `indent + 2`, which is why `parse_explicit_key`'s
            // opening `close_deeper_indents` cannot close the item we just
            // pushed at virtual indent `indent + 1`.
            self.parse_explicit_key(self.current_column())?;
            // Don't close anything - mapping and item are closed by
            // close_deeper_indents when we see content at lower indent.
        } else if self.looks_like_mapping_entry() {
            // Check for compact mapping: `- key: value`
            // This is a mapping entry directly as the sequence item value
            // The sequence item contains a mapping.
            //
            // The key's real column, not a hardcoded `indent + 2` - `- `
            // (dash + whitespace) is already skipped by this point, so
            // `self.current_column()` is exact regardless of spacing. A
            // fixed `indent + 2` assumed exactly one space after the dash:
            // with more than one (`-   a: hello`), every entry after the
            // compact mapping's first field folded into that first field's
            // own value instead of being recognized as its own entry (#877).
            let compact_indent = self.current_column();
            self.parse_compact_mapping_entry(compact_indent)?;
            // Don't close anything - mapping and item will be closed by
            // close_deeper_indents when we see content at lower indent.
        } else {
            // Parse the item value normally
            // Pass structure indent for block scalars (content must be > this)
            let bp_pos_before_value = self.bp_pos;
            self.parse_value(indent)?;
            // Claim a comment deferred by an earlier anchor's deferred value
            // (#784): a plain-scalar item's own trailing comment lives on
            // the scalar's own bp (matching the ordinary, non-deferred
            // `- 1 # comment` case, which `set_bp_text_end` inside
            // `parse_value` already attaches there).
            //
            // Only for a genuine plain/quoted scalar, though: `parse_value`
            // dispatching to a flow collection (`[1, 2]`/`{a: 1}`) opens one
            // bp per element *after* the collection's own, leaving
            // `self.last_open_bp_pos` pointing at the collection's *last
            // inner element* rather than the collection or this item - a
            // comment claimed there landed between the last element and the
            // closing bracket, corrupting the emitted structure (#1081
            // review). A plain scalar opens exactly one bp (at
            // `bp_pos_before_value`, since `write_bp_open` records
            // `last_open_bp_pos` before incrementing `bp_pos`); anything
            // that opened more than that fails this check and safely drops
            // the floated comment instead of misattaching it.
            if self.last_open_bp_pos == bp_pos_before_value {
                self.take_pending_head_comment(self.last_open_bp_pos);
            }
            // Close the sequence item for simple values
            self.indent_stack.pop();
            self.pop_type();
            self.write_bp_close();
        }

        Ok(())
    }

    /// Look ahead from the end of the current line to decide whether the value
    /// that would continue on the following lines exists at all.
    ///
    /// Deliberately reuses `skip_newlines`, which also skips indented comment
    /// lines: a hand-rolled blank-line scan would disagree with what the main
    /// loop does next, and every failure mode of the anchored-item null node is
    /// exactly that disagreement.
    ///
    /// There is no same-indent-sequence exception: callers that need one (a
    /// block sequence may sit at its parent key's indent) test for it
    /// themselves. Restores `self.pos` before returning.
    fn following_value_is_null(&mut self, indent: usize) -> bool {
        let saved_pos = self.pos;
        self.skip_to_eol();
        self.skip_newlines();
        let is_null = self.peek().is_none() || self.count_indent().unwrap_or(0) <= indent;
        self.pos = saved_pos;
        is_null
    }

    /// Parse a compact mapping entry within a sequence item.
    /// This handles `- key: value` where the mapping is inline with the sequence item.
    fn parse_compact_mapping_entry(&mut self, indent: usize) -> Result<(), YamlError> {
        // Open a mapping for this compact entry
        self.write_bp_open();
        self.write_ty(false); // 0 = mapping
        self.indent_stack.push(indent);
        self.push_type(NodeType::Mapping);

        // Mark key position
        self.set_ib();

        // Open key node
        self.write_bp_open();

        // Check for a property on the key (`- &a k: v` / `- !!str k: v`) -
        // record it pointing to this key.
        self.record_key_properties()?;

        // Parse the key
        let key_end = match self.peek() {
            Some(b'"') => {
                self.parse_double_quoted()?;
                self.pos
            }
            Some(b'\'') => {
                self.parse_single_quoted()?;
                self.pos
            }
            // Alias as key (`- *a: v`), sharing the block mapping's site.
            Some(b'*') => self.record_key_alias()?,
            _ => self.parse_unquoted_key()?,
        };
        self.set_bp_text_end(key_end);

        // Close key node
        self.write_bp_close();

        // Expect colon
        if self.peek() != Some(b':') {
            return Err(
                self.err_unexpected_char(self.pos, "expected ':' after key in compact mapping")
            );
        }
        self.advance(); // Skip ':'

        // Skip space after colon
        self.skip_inline_whitespace();

        // An anchor prefixes the value rather than being it, so it has to be
        // consumed *before* asking where the value is: with `&a` last on the
        // line the value is on the *next* line. Deciding first sent `- k: &a` /
        // `    b: 1` down the inline path, where `parse_inline_value`'s
        // multi-line plain-scalar rule read the nested block as one folded
        // scalar — `{"k":"b"}` for that input, and `{"k":null}` for the
        // sequence form (#406).
        //
        // Every other block-context value site already orders the two this way
        // — `parse_mapping_entry`, `parse_sequence_item_inner`,
        // `parse_explicit_value` — which is why the block form `k: &a` /
        // `  b: 1` was always right. This was the last one that did not.
        //
        // `- k: &a 1` also never registered the anchor before this, so a later
        // `*a` resolved to nothing (#372). Tags follow the same rule (#224).
        let had_property = if !self.at_line_end() {
            self.parse_node_properties()?
        } else {
            false
        };

        // Parse value
        if self.at_line_end() {
            if had_property {
                // An anchor/tag caused this deferral - the comment belongs
                // to the deferred value, not this key's own line
                // (#784/#1078, matching `parse_mapping_entry`'s identical
                // split below) - defer it rather than capturing it here;
                // `take_pending_head_comment` claims it wherever the next
                // primary node opens.
                self.defer_line_comment();
            } else {
                // No anchor - a trailing comment here is this key's own
                // (issue #765) - `self.last_open_bp_pos` still holds the key
                // node's bp_pos here since no value node has been opened yet
                // (nothing opens a BP node between the key's own close above
                // and this point). This mirrors `parse_mapping_entry`'s
                // identical capture just below - missing here left a
                // block-sequence item's *first* field (the only mapping
                // entry parsed by this function rather than
                // `parse_mapping_entry`) silently dropping its own key
                // comment (#785).
                //
                // Falls back to a comment an *earlier*, unrelated anchor's
                // deferred value floated onto this key (#784) only if this
                // key's own line had nothing of its own - run the fallback
                // second so a genuine same-line comment always wins the slot
                // (#1081 review: consuming a floated comment eagerly at
                // key-open, before this real capture had a chance to run,
                // silently destroyed it).
                self.maybe_capture_line_comment(self.last_open_bp_pos);
                self.take_pending_head_comment(self.last_open_bp_pos);
            }
            self.skip_to_eol();

            // Look ahead to determine if this is a null value or a nested structure
            self.skip_newlines();
            if self.peek().is_none() {
                // EOF - null value: emit empty value node
                self.set_ib();
                self.write_bp_open();
                self.write_bp_close();
            } else {
                let next_indent = self.count_indent().unwrap_or(0);

                // Check if next content is a sequence indicator
                let saved_pos = self.pos;
                self.advance_by(next_indent);
                let is_sequence_indicator =
                    matches!(self.peek(), Some(b'-')) && Self::is_ws_break_or_eoi(self.peek_at(1));
                self.pos = saved_pos;

                if next_indent < indent || (next_indent == indent && !is_sequence_indicator) {
                    // Next line is at lower indent, or same indent but not a sequence
                    // - null value: emit empty value node
                    self.set_ib();
                    self.write_bp_open();
                    self.write_bp_close();
                }
                // Otherwise, value is a nested structure - main loop will handle it
            }
            // An anchor consumed above needs a node to name, and both arms
            // provide one: the null arms emit that empty node unconditionally,
            // and in the nested-structure arm the next BP write is the
            // container's own open (`close_deeper_indents` closes only
            // *strictly* deeper containers, so it cannot slip a close in
            // first). Hence no `following_value_is_null` call here, unlike
            // `parse_sequence_item_inner` and `parse_explicit_value` where the
            // placeholder is conditional. `test_every_anchor_targets_an_open_bit`
            // is the whole-corpus guard on that.
        } else if self.try_dispatch_flow_or_block_value(indent, false)? {
            // Flow sequence/mapping value (`- a: [1, 2]` / `- a: {x: 1}`), or
            // block scalar value (`- a: |`) — handled. Missing this used to
            // leave every flow-value fallback through the scalar arm's
            // `parse_inline_value`, which treats `{`/`[`/`,`/`}`/`]` as
            // ordinary plain-scalar content instead of structure: a flow
            // *array* value gets consumed whole as one bogus scalar token
            // (its real content lost — reads back as `[]`), and a flow
            // *mapping* value is worse, since the scalar scanner stops at its
            // first inner `key:`+space and strands `self.pos` mid-line,
            // corrupting everything parsed after it too (#864). A block
            // scalar hit the same class of corruption for the same reason:
            // the plain-scalar scanner doesn't know it's inside literal
            // content, so a body line shaped like `key: value` or starting
            // with `#` (legal in a block literal, meaningless as YAML there)
            // stopped the scan early (confirmed via `git worktree` bisection
            // against the pre-#864 fallback during review).
        } else {
            // Inline value (`parse_node_properties` ran above)
            match self.peek() {
                Some(b'*') => {
                    // `parse_alias` opens and closes its own node. `- k: *a` never
                    // became an alias node at all before #372, and was swallowed
                    // into the plain scalar below.
                    self.parse_alias()?;
                }
                Some(b'-') if Self::is_ws_break_or_eoi(self.peek_at(1)) => {
                    // Block sequence indicator inline with a compact mapping's own
                    // value (`- a: - x`): the same invalid-but-common shape #325
                    // fixed for a top-level mapping value, one level deeper. This
                    // arm was missing when #325 landed, so `- a: - x` still fell
                    // through to `parse_inline_value` below and read back as the
                    // scalar `"- x"` instead of the sequence `["x"]` — inconsistent
                    // with the sibling fix in `parse_mapping_entry`.
                    //
                    // `self.current_column()` (not `indent + 2`), matching every
                    // other site that opens this arm, so a continuation line whose
                    // `-` sits at the same column joins this sequence.
                    let seq_indent = self.current_column();
                    self.parse_sequence_item(seq_indent)?;
                }
                _ => {
                    // Open value node
                    self.set_ib();
                    self.write_bp_open();
                    let end_pos = self.parse_inline_value(indent)?;
                    self.set_bp_text_end(end_pos);
                    // Claim a comment deferred by an earlier anchor's
                    // deferred value (#784), if this value's own line had no
                    // comment of its own - run after `set_bp_text_end`'s own
                    // capture just above so a genuine same-line comment
                    // always wins the slot. Needed for a deferred value
                    // whose first line is itself a compact mapping with an
                    // inline scalar (`a: &anc # comment\n  - key: 1`) - `1`
                    // is this exact arm.
                    self.take_pending_head_comment(self.last_open_bp_pos);
                    self.write_bp_close();
                }
            }
        }

        // Don't close the mapping here - leave it open so subsequent lines
        // at compatible indent levels can add more entries. The mapping will
        // be closed by close_deeper_indents when we return to a lower indent.

        Ok(())
    }

    /// Parse a mapping key-value pair.
    fn parse_mapping_entry(&mut self, indent: usize) -> Result<(), YamlError> {
        let _entry_start = self.pos;

        // A sibling landing strictly between an open mapping's own indent
        // and its parent mapping's indent has no unambiguous owner (#901) —
        // real yq rejects it outright, so this must too rather than
        // silently misattributing it. Checked before the #885 normalization
        // below since the two shapes are mutually exclusive on the parent
        // frame's type (SequenceItem vs. Mapping).
        self.check_mapping_under_mapping_gap(indent, false)?;

        // Normalize an inconsistently-indented continuation of an open
        // compact mapping to the mapping's own indent, so the
        // close/need-new-mapping decision below adds an entry to it instead
        // of closing it (#885).
        let indent = self.resolve_compact_mapping_gap_indent(indent);

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
        let need_new_mapping = self.current_type != Some(NodeType::Mapping)
            || self.indent_stack.last().copied() != Some(indent);

        if need_new_mapping {
            // Open new mapping (virtual - no IB bit, children will have IB)
            self.write_bp_open();
            self.write_ty(false); // 0 = mapping
            self.indent_stack.push(indent);
            self.push_type(NodeType::Mapping);
        }

        // Mark key position
        self.set_ib();

        // Open key node
        self.write_bp_open();

        // Check for a property on the key - record it pointing to this key BP
        self.record_key_properties()?;

        // Parse the key - check for empty key first (colon at start)
        let key_end = if self.peek() == Some(b':') {
            // Empty key - check that it's followed by proper terminator
            let next = self.peek_at(1);
            if Self::is_ws_break_or_eoi(next) {
                // Empty key case - key length is 0, don't advance yet
                self.pos
            } else {
                // Colon followed by something else - not an empty key
                self.parse_unquoted_key()?
            }
        } else {
            // Parse the key
            match self.peek() {
                Some(b'"') => {
                    self.parse_double_quoted()?;
                    self.pos
                }
                Some(b'\'') => {
                    self.parse_single_quoted()?;
                    self.pos
                }
                // Alias as key (`*a: v`), sharing the compact mapping's site.
                Some(b'*') => self.record_key_alias()?,
                _ => self.parse_unquoted_key()?,
            }
        };
        self.set_bp_text_end(key_end);

        // Close key node
        self.write_bp_close();

        // Skip optional whitespace between key and colon (e.g., 'key' : value)
        self.skip_inline_whitespace();

        // Expect colon
        if self.peek() != Some(b':') {
            return Err(self.err_unexpected_char(self.pos, "expected ':' after key"));
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
            // Value is on next line - check what kind of value. Capture a
            // trailing comment on the key's own line first (issue #765) -
            // `self.last_open_bp_pos` still holds the key node's bp_pos here
            // since no value node has been opened yet (nothing opens a BP
            // node between the key's own close above and this point).
            //
            // Falls back to a comment an *earlier*, unrelated anchor's
            // deferred value floated onto this key (#784) only if this
            // key's own line had nothing of its own - run the fallback
            // second so a genuine same-line comment always wins the slot
            // (#1081 review: consuming a floated comment eagerly at
            // key-open, before this real capture had a chance to run,
            // silently destroyed it).
            self.maybe_capture_line_comment(self.last_open_bp_pos);
            self.take_pending_head_comment(self.last_open_bp_pos);
            self.skip_to_eol();

            // Look ahead to see what the next content line looks like
            self.skip_newlines();
            if self.peek().is_none() {
                // EOF - null value: emit empty value node
                self.set_ib();
                self.write_bp_open();
                self.write_bp_close();
                return Ok(());
            }

            // Count indentation of next line
            let next_indent = self.count_indent().unwrap_or(0);

            // Check what's at the next line's content position
            let saved_pos = self.pos;
            self.advance_by(next_indent);
            let next_char = self.peek();
            let is_sequence_indicator =
                matches!(next_char, Some(b'-')) && Self::is_ws_break_or_eoi(self.peek_at(1));
            self.pos = saved_pos;

            if next_indent < indent {
                // Next line is at lower indent - definitely null value
                self.set_ib();
                self.write_bp_open();
                self.write_bp_close();
                return Ok(());
            }

            if next_indent == indent && !is_sequence_indicator {
                // Next line is at same indent but NOT a sequence - null value
                // (If it were a sequence, the sequence is the value of this key)
                self.set_ib();
                self.write_bp_open();
                self.write_bp_close();
                return Ok(());
            }

            // Re-advance to content position for the remaining checks
            self.advance_by(next_indent);
            // A tab here indents whatever block structure follows (most often
            // a sequence item) - reject it before the dispatch below, which
            // otherwise misses it: the `Some(b'-')` arm doesn't match while
            // the tab is still on the cursor, so it fell through to the
            // plain-scalar arm and folded the dash and all into a string
            // instead of raising this error (#432).
            if self.tab_indents_block_structure(saved_pos) {
                return Err(YamlError::TabIndentation {
                    line: self.current_line(),
                    offset: self.pos,
                });
            }
            // A tab left over from `advance_by` (which only accounts for
            // spaces) is legal separation here, not content - skip it so the
            // dispatch below, and any node it opens, land on the node's real
            // first byte (#381).
            self.skip_separation_whitespace(saved_pos);

            // Check if this is a nested structure or a plain scalar value
            match self.peek() {
                Some(b'-') if Self::is_ws_break_or_eoi(self.peek_at(1)) => {
                    // Sequence - will be handled by main loop
                    self.pos = saved_pos;
                    return Ok(());
                }
                // Tab included: the sibling `-` check just above uses the
                // same 4-way terminator set (#434).
                Some(b'?') if Self::is_ws_break_or_eoi(self.peek_at(1)) => {
                    // Explicit key - will be handled by main loop
                    self.pos = saved_pos;
                    return Ok(());
                }
                Some(b'{' | b'[' | b'|' | b'>') => {
                    // Flow/block structure - will be handled by main loop
                    self.pos = saved_pos;
                    return Ok(());
                }
                Some(b'#') => {
                    // Comment - will be handled by main loop
                    self.pos = saved_pos;
                    return Ok(());
                }
                Some(b'&' | b'*' | b'!') => {
                    // Anchor, alias, or tag on its own line - will be handled
                    // by main loop (parse_block_node, which calls
                    // parse_node_properties). Without `!` here a tagged
                    // deferred value fell to the `_` catch-all below and was
                    // parsed inline instead, bypassing property consumption
                    // (#224, #664 root cause 3).
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
                    // Scalar value - parse it here with key's indent as base.
                    // Quoted nodes need their own reader: the unquoted scanner
                    // stops at the first `: ` it sees, including one inside
                    // the quotes, stranding the cursor mid-string and
                    // corrupting whatever follows on the document.
                    self.set_ib();
                    self.write_bp_open();
                    let end_pos = match self.peek() {
                        Some(b'"') => {
                            self.parse_double_quoted()?;
                            self.pos
                        }
                        Some(b'\'') => {
                            self.parse_single_quoted()?;
                            self.pos
                        }
                        _ => self.parse_unquoted_value_with_indent(indent),
                    };
                    self.set_bp_text_end(end_pos);
                    self.write_bp_close();
                    return Ok(());
                }
            }
        }

        {
            // Consume any leading `&anchor` and/or `!tag` - they prefix the
            // actual value.
            self.parse_node_properties()?;

            // After anchor, check if value continues on next line
            if self.at_line_end() {
                // A trailing comment here belongs to the deferred value, not
                // to this key's own line (#784, distinct from #765's
                // no-anchor case) - defer it rather than dropping it;
                // `take_pending_head_comment` claims it wherever the next
                // primary node opens.
                self.defer_line_comment();

                // Need to check if the next line has content for this value,
                // or if the value is null (same or lower indent on next line)
                self.skip_to_eol();

                // Save position to look ahead
                let saved_pos = self.pos;

                // Look at next content line
                self.skip_newlines();
                if self.peek().is_none() {
                    // EOF - value is null, create explicit null node for anchor
                    self.pos = saved_pos;
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
                    // Skip past indent spaces to check what follows (SIMD accelerated)
                    self.skip_spaces_simd();
                    matches!(self.peek(), Some(b'-')) && Self::is_ws_break_or_eoi(self.peek_at(1))
                };
                // Restore position after checking
                self.pos = pos_before_check;

                // A block sequence may sit at its parent key's indent, so it is
                // only that key's value when the indents are *equal*. One at a
                // lower indent belongs to an outer container, and treating it
                // as this key's value left the key's anchor dangling
                // (`m:\n  b: &b\n- x`). Same test as
                // `parse_compact_mapping_entry`, which had it right - though
                // it wasn't quite: that version already included the tab in
                // its terminator set and this one didn't, so `-\tx` at the
                // key's indent still dangled the anchor (#434).
                if next_indent < indent || (next_indent == indent && !is_sequence_at_same_indent) {
                    // Next line is at same or lower indent and not a sequence - value is null
                    // Create explicit null node for anchor to point to
                    self.pos = saved_pos;
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
            if self.try_dispatch_flow_or_block_value(indent, false)? {
                return Ok(());
            }
            match self.peek() {
                Some(b'-') if Self::is_ws_break_or_eoi(self.peek_at(1)) => {
                    // Block sequence indicator inline with the mapping key (`a: - x`).
                    // This is invalid YAML (test-suite case 5U3A: a block sequence may
                    // not begin on the same line as its parent mapping key), and the
                    // opt-in strict validator rejects it. But the loader does minimal
                    // validation by design, so it parses the obvious extension rather
                    // than silently dropping the item's content. See #325.
                    //
                    // `parse_sequence_item` handles everything: the nesting guard, the
                    // sequence container, the item wrapper, and the three item shapes
                    // (`a: - - x`, `a: - k: v`, `a: - x`). No BP node has been opened
                    // for the value yet, so it must open both itself.
                    //
                    // The indent is the actual column of the `-`, not `indent + 2`, so
                    // that continuation lines whose `-` sits at the same column join
                    // this sequence instead of opening a nested one:
                    //
                    // ```yaml
                    // key: - a    # `-` at column 5 -> sequence at indent 5
                    //      - b    # same column -> same sequence
                    // ```
                    let seq_indent = self.current_column();
                    self.parse_sequence_item(seq_indent)?;
                }
                _ => {
                    // Scalar value - wrap in BP
                    self.set_ib();
                    self.write_bp_open();
                    let end_pos = self.parse_inline_value(indent)?;
                    self.set_bp_text_end(end_pos);
                    // Claim a comment deferred by an earlier anchor's
                    // deferred value (#784), if this value's own line had no
                    // comment of its own - run after `set_bp_text_end`'s own
                    // capture (called just above) so a genuine same-line
                    // comment always wins the slot. This is what makes the
                    // issue's own repro work: `a: &anc # comment\n  b: 1`
                    // resolves `a`'s deferred value to this exact scalar arm
                    // for key `b`'s own value `1`, not a nested structure.
                    self.take_pending_head_comment(self.last_open_bp_pos);
                    self.write_bp_close();
                }
            }
        }

        Ok(())
    }

    /// Parse an explicit key (`? key`).
    /// The key can be any value: scalar, sequence, or mapping.
    fn parse_explicit_key(&mut self, indent: usize) -> Result<(), YamlError> {
        // Same ambiguous-gap error as parse_mapping_entry (#901).
        self.check_mapping_under_mapping_gap(indent, false)?;

        // Same gap-tolerant normalization as parse_mapping_entry (#885):
        // this function has the identical close/need-new-mapping shape.
        let indent = self.resolve_compact_mapping_gap_indent(indent);

        // Close any deeper containers
        self.close_deeper_indents(indent);

        // If there's a pending explicit key without value, close it with null
        self.close_pending_explicit_key();

        // Close a sequence that was a *previous* explicit key's same-indent
        // implicit value, exactly as `parse_mapping_entry` does for an
        // ordinary `key:` entry (#1040) -- `? k\n- item\n? k2\n: v2\n`
        // reaches this function for `? k2`, not `parse_mapping_entry`, so
        // without this call the still-open sequence stays the top frame,
        // `need_new_mapping` below sees `current_type == Sequence` and
        // wrongly nests `k2` as a second element of `k`'s array instead of
        // opening a sibling mapping entry.
        self.close_same_indent_sequence_before_mapping_entry(indent);

        // Check if we need to open a new mapping
        let need_new_mapping = self.current_type != Some(NodeType::Mapping)
            || self.indent_stack.last().copied() != Some(indent);

        if need_new_mapping {
            // Open new mapping
            self.write_bp_open();
            self.write_ty(false); // 0 = mapping
            self.indent_stack.push(indent);
            self.push_type(NodeType::Mapping);
        }

        // The mapping that owns this key, recorded now while it is unambiguously the
        // top of the stack: parsing the key may push containers of its own (a
        // sequence, or the compact mapping of `? k: v`) that must not receive the
        // owner's null value.
        let owner_depth = self.indent_stack.len();

        // Skip `?`
        self.advance();

        // Skip whitespace/comments after `?`
        self.skip_inline_whitespace();

        // Check for a property before the key
        self.parse_node_properties()?;

        // Check what the key is
        if self.at_line_end() {
            // Key content might be on next line(s), or this could be an empty key
            // Save position for potential empty key emission (after the `?`)
            let key_pos = self.pos;
            self.skip_to_eol();

            // Look ahead to see what's on the next line
            self.skip_newlines();
            if self.peek().is_none() {
                // EOF - empty key (null) with implicit null value
                // Emit empty key node using the position after `?`
                self.pos = key_pos;
                self.set_ib();
                self.write_bp_open();
                self.write_bp_close();
                self.pending_explicit_key = Some(owner_depth);
                return Ok(());
            }

            let next_indent = self.count_indent().unwrap_or(0);

            // Check if next line starts with `:` at same indent (explicit value)
            // or has other content at same/lower indent (meaning empty key)
            let saved_pos = self.pos;
            self.advance_by(next_indent);
            let next_char = self.peek();
            self.pos = saved_pos;

            if next_indent <= indent {
                // Next content is at same or lower indent
                if next_char == Some(b':')
                    && (next_indent == indent)
                    && Self::is_ws_break_or_eoi(
                        self.input.get(saved_pos + next_indent + 1).copied(),
                    )
                {
                    // `: value` at same indent - empty key (null), value follows
                    // Emit empty key node using the position after `?`
                    self.pos = key_pos;
                    self.set_ib();
                    self.write_bp_open();
                    self.write_bp_close();
                    // Restore position for main loop to process `: value`
                    self.pos = saved_pos;
                    self.pending_explicit_key = Some(owner_depth);
                    return Ok(());
                }
                // Other content at same/lower indent - empty key (null) with implicit null value
                // Emit empty key node using the position after `?`
                self.pos = key_pos;
                self.set_ib();
                self.write_bp_open();
                self.write_bp_close();
                // Restore position for main loop to process next content
                self.pos = saved_pos;
                self.pending_explicit_key = Some(owner_depth);
                return Ok(());
            }

            // Content at deeper indent - that's the key content
            // Let the main loop parse it (don't restore position - stay at content start)
            return Ok(());
        }

        // Mark key position
        self.set_ib();

        // Parse the key value inline
        match self.peek() {
            // Tab included: every sibling `-` check in this file uses the same
            // 4-way terminator set (kept in sync with `super::is_seq_indicator_next`
            // by `is_ws_break_or_eoi_agrees_with_is_seq_indicator_next`), but this
            // site still hand-rolled a 3-way match missing the tab, so
            // `? -\ta\n  -\tb\n: value` fell through to being parsed as a plain
            // scalar key `-\ta` instead of a sequence key (#434).
            Some(b'-') if Self::is_ws_break_or_eoi(self.peek_at(1)) => {
                // Sequence as key - open key node and let sequence parsing continue
                // The key will be a sequence
                self.write_bp_open();
                self.write_ty(true); // sequence
                self.indent_stack.push(indent + 2); // Indent for sequence content
                self.push_type(NodeType::Sequence);

                // Parse first sequence item inline
                self.write_bp_open_seq_item(); // item node
                self.advance(); // skip `-`
                self.skip_inline_whitespace();

                if !self.at_line_end() {
                    // Parse item value
                    if self.looks_like_mapping_entry() {
                        // The key's real column, not a hardcoded `indent + 3`
                        // - `- ` is already skipped above, so `self.pos` sits
                        // at the key. `indent + 3` was wrong even for the
                        // ordinary single-space case (`? - a: 1`): `?` + ` `
                        // + `-` + ` ` is 4 columns, not 3, so every field
                        // after the compact mapping's first silently landed
                        // at the wrong indent (#877 follow-up).
                        self.parse_compact_mapping_entry(self.current_column())?;
                    } else {
                        self.parse_value(indent + 2)?;
                    }
                }
                self.write_bp_close(); // close item
            }
            Some(b'[') => {
                // Flow sequence as key. #902: this dispatch is a 5th,
                // deliberately-unmerged copy of `try_dispatch_flow_or_block_
                // value`'s `[`/`{` arms (it wraps the key in its own
                // `write_bp_open`/`write_bp_close` pair, which that shared
                // helper can't represent without double-wrapping the BP
                // tree) -- it never gained #878's trailing-content
                // validation either, so `? [1, 2] extra\n: v\n` silently
                // dropped the real `: v` line instead of erroring the way
                // real yq does. `reject_trailing_flow_content` still
                // correctly permits a genuine same-line `? [1, 2]: value`
                // (confirmed live: real yq reads that as the compact-
                // mapping-keyed-by-flow-collection shape, not trailing
                // garbage -- same ambiguity `try_dispatch_flow_or_block_
                // value`'s doc comment already covers for the value
                // position, just mirrored here at the key position).
                self.write_bp_open();
                self.parse_flow_sequence()?;
                self.reject_trailing_flow_content(true)?;
                self.write_bp_close();
            }
            Some(b'{') => {
                // Flow mapping as key -- see the `[` arm above (#902).
                self.write_bp_open();
                self.parse_flow_mapping()?;
                self.reject_trailing_flow_content(true)?;
                self.write_bp_close();
            }
            Some(b'|' | b'>') => {
                // Block scalar as key
                self.parse_block_scalar(indent)?;
            }
            _ if self.looks_like_mapping_entry() => {
                // `? k: v` — a value indicator on this line means the node after `? `
                // is a *compact block mapping* that is itself the key (YAML 1.2 §8.2.2:
                // c-l-block-map-explicit-key -> s-l+block-indented -> ns-l-compact-mapping).
                // The entry therefore has a complex key and no value, which is why yq
                // renders it `""` and null (#346).
                //
                // Routed through the same `parse_compact_mapping_entry` the `- k: v`
                // sequence-item path uses rather than a second copy of the decision
                // (#106). Ordered after the `-`, `[`, `{` and block-scalar arms so
                // those spellings — whose lines can carry a `: ` that is not this
                // line's value indicator — keep their handling.
                //
                // The key content's own column, not `indent`, is the mapping's indent:
                // a continuation line joins the key at the key's column (`? k: v` then
                // `  j: u`), while the `: ` value indicator aligns with the `?`.
                //
                // Leaving that mapping open is what keeps the parser off the mid-line
                // exit this used to take: `parse_unquoted_value_with_indent` stopped at
                // the `: `, and the main loop then re-derived the line's indent from
                // mid-line as 0 and closed the mapping it should have been filling.
                self.parse_compact_mapping_entry(self.current_column())?;
            }
            Some(b'*') => {
                // Alias as key (`? *a`). As in the flow-mapping key path,
                // `parse_alias` opens and closes its own node. Without this arm
                // the alias fell to the plain-scalar arm below, which produced
                // an empty key whether or not the anchor existed (#372).
                //
                // Below the mapping-entry arm above, so `? *a: v` is read as a
                // compact mapping whose key is the alias — the same reading
                // `parse_compact_mapping_entry` already gives `- *a: v`. This
                // arm is the alias that is the whole key, with its `:` (if any)
                // on a later line.
                self.parse_alias()?;
            }
            Some(b'"') => {
                // Double-quoted key
                self.write_bp_open();
                self.parse_double_quoted()?;
                self.set_bp_text_end(self.pos);
                self.write_bp_close();
            }
            Some(b'\'') => {
                // Single-quoted key
                self.write_bp_open();
                self.parse_single_quoted()?;
                self.set_bp_text_end(self.pos);
                self.write_bp_close();
            }
            _ => {
                // Unquoted scalar key
                self.write_bp_open();
                let end_pos = self.parse_unquoted_value_with_indent(indent);
                self.set_bp_text_end(end_pos);
                self.write_bp_close();
            }
        }

        // Mark that we have an explicit key waiting for a value
        self.pending_explicit_key = Some(owner_depth);

        Ok(())
    }

    /// Parse an explicit value (`: value` after explicit key).
    fn parse_explicit_value(&mut self, indent: usize) -> Result<(), YamlError> {
        // Same ambiguous-gap error as parse_mapping_entry (#901).
        self.check_mapping_under_mapping_gap(indent, false)?;

        // Same gap-tolerant normalization as parse_mapping_entry (#885): an
        // inconsistently-indented `:` must not close the pending key's own
        // compact mapping out from under it.
        let indent = self.resolve_compact_mapping_gap_indent(indent);

        // Same ambiguous-column error for a `:` that doesn't match its own
        // `?` (#1010) -- checked against the #885-resolved `indent` above,
        // not the raw column, so that normalization's tolerance stays intact.
        self.require_consistent_indentation(self.explicit_value_matches_key_indent(indent))?;

        // Close deeper structures, but keep the mapping at this indent open
        self.close_deeper_indents(indent + 1);

        // This value is for the pending explicit key
        self.pending_explicit_key = None;

        // Skip `:`
        self.advance();

        // Skip whitespace after `:`
        self.skip_inline_whitespace();

        // Check for a property (anchor and/or tag)
        let had_property = self.parse_node_properties()?;

        // Check if value is on this line or next
        if self.at_line_end() {
            // An anchor/tag caused this deferral - the comment belongs to
            // the deferred value, not this line (#784, same shape as
            // `parse_mapping_entry`'s/`parse_sequence_item_inner`'s
            // identical split) - defer it rather than dropping it;
            // `take_pending_head_comment` claims it wherever the next
            // primary node opens. No no-anchor equivalent here (unlike
            // `parse_mapping_entry`'s #765 capture) - a bare `: # comment`
            // with no property has never captured its own trailing comment
            // at all, a separate, pre-existing gap outside this issue's scope.
            if had_property {
                self.defer_line_comment();
            }

            // A property on a value that turns out to be null needs an
            // explicit node to resolve against, or it dangles on whatever BP
            // bit comes next - a close, or the open of an unrelated node.
            // `? e` / `: &a` then `z: *a` resolved the alias to the *key*
            // `z`, and inside a sequence the anchor landed on the alias's
            // own open and tripped a spurious cycle rejection. A tag with
            // nothing after it (`? e` / `: !!str`) needs the same treatment
            // so it resolves to `""` rather than being dropped (#224).
            if had_property && self.following_value_is_null(indent) {
                self.set_ib();
                self.write_bp_open();
                self.write_bp_close();
            }

            // Value is on next line(s) or null
            self.skip_to_eol();
            return Ok(());
        }

        // Parse the value. The node after `: ` may itself be a compact
        // mapping whose *key* is a flow collection (`: [a, b]: value`),
        // mirroring the scalar-keyed case the `looks_like_mapping_entry` arm
        // below already handles (`: b: c`) — real YAML accepts both
        // (confirmed live against real `yq`: `? k\n: [a, b]: value\n` parses
        // as `{"k":{"":"value"}}`). `try_dispatch_flow_or_block_value`'s own
        // trailing-content check (#902) permits exactly this `:`-following
        // shape, so it doesn't reject real, non-garbage input before
        // `looks_like_mapping_entry` below gets a chance to run on it.
        if self.try_dispatch_flow_or_block_value(indent, true)? {
            return Ok(());
        }
        match self.peek() {
            Some(b'-') if Self::is_ws_break_or_eoi(self.peek_at(1)) => {
                // Sequence as value. Routed through the shared sequence-item
                // parser rather than an inlined copy of its dispatch, so anchor
                // handling (#328) and every future fix land here too — this was
                // the third divergent copy of this decision (#106). That shared
                // parser opens the item wrapper via `write_bp_open_seq_item`, so
                // the #332 wrapper-end invariant is enforced here too.
                self.parse_sequence_item(self.current_column())?;
            }
            _ if self.looks_like_mapping_entry() => {
                // `: b: c` — the mirror of the key-side arm above: the node after `: `
                // may equally be a compact block mapping starting on the same line, and
                // stopping the scalar at the inner `: ` left the parser mid-line with
                // the same consequences (#346). Corpus case V9D5
                // (`- ? earth: blue` / `  : moon: white`) needs both arms.
                self.parse_compact_mapping_entry(self.current_column())?;
            }
            Some(b'"') => {
                self.set_ib();
                self.write_bp_open();
                self.parse_double_quoted()?;
                self.set_bp_text_end(self.pos);
                self.write_bp_close();
            }
            Some(b'\'') => {
                self.set_ib();
                self.write_bp_open();
                self.parse_single_quoted()?;
                self.set_bp_text_end(self.pos);
                self.write_bp_close();
            }
            Some(b'*') => {
                // Alias as value
                self.parse_alias()?;
            }
            _ => {
                self.set_ib();
                self.write_bp_open();
                let end_pos = self.parse_unquoted_value_with_indent(indent);
                self.set_bp_text_end(end_pos);
                self.write_bp_close();
            }
        }

        Ok(())
    }

    /// Parse an inline scalar value (on the same line as the key).
    /// Returns the end position of the scalar content.
    fn parse_inline_value(&mut self, min_indent: usize) -> Result<usize, YamlError> {
        let end = match self.peek() {
            Some(b'"') => {
                self.parse_double_quoted()?;
                self.pos
            }
            Some(b'\'') => {
                self.parse_single_quoted()?;
                self.pos
            }
            _ => self.parse_unquoted_value_with_indent(min_indent),
        };
        Ok(end)
    }

    /// Parse a value (could be scalar or nested structure).
    fn parse_value(&mut self, min_indent: usize) -> Result<(), YamlError> {
        // Check for a property first - it prefixes the actual value. Callers
        // that already consumed one (e.g. parse_sequence_item_inner) leave
        // nothing here, so this is a no-op re-check rather than a second
        // consumption.
        self.parse_node_properties()?;

        // Check for alias - this IS the value (no value follows)
        if self.peek() == Some(b'*') {
            return self.parse_alias();
        }

        // A flow collection reached here may turn out to be an implicit
        // mapping *key* rather than a standalone value — see
        // `try_dispatch_flow_or_block_value`'s doc comment for why its own
        // trailing-content check permits that shape rather than rejecting it.
        if self.try_dispatch_flow_or_block_value(min_indent, true)? {
            return Ok(());
        }

        match self.peek() {
            Some(b'"') => {
                self.set_ib();
                self.write_bp_open();
                self.parse_double_quoted()?;
                self.set_bp_text_end(self.pos);
                self.write_bp_close();
            }
            Some(b'\'') => {
                self.set_ib();
                self.write_bp_open();
                self.parse_single_quoted()?;
                self.set_bp_text_end(self.pos);
                self.write_bp_close();
            }
            Some(b'-') if Self::is_ws_break_or_eoi(self.peek_at(1)) => {
                // Inline sequence item - this creates a nested sequence.
                //
                // This arm used to be a no-op on the theory that "the caller already
                // opened a BP node for us". The caller opens a *wrapper*, but nothing
                // was ever emitted inside it, so the item's content was silently
                // discarded and the node resolved to null (#325). Delegate instead,
                // exactly as `parse_sequence_item_inner` does for `- - x`.
                //
                // Reached when a `-` gets past the callers' own dash checks, which
                // happens once an anchor stands between the two: `- &a &b - x`.
                // `parse_sequence_item_inner` sees `&`, not `-`, so it falls through
                // to `parse_value`, whose anchor prologue consumes only the first
                // anchor and leaves the cursor on the second. Rare, but real — see
                // `test_double_anchor_before_nested_dash_keeps_content`.
                //
                // The guard goes through `is_seq_indicator_next`, so it accepts
                // `\n`/`\r`/end-of-input as well as space/tab, matching the reader's
                // seq-item predicate (`light.rs`, `index.rs`). The old hand-written
                // space/tab-only spelling here was the asymmetry #325 called out: it
                // let a bare trailing `-` reach the scalar arm, where the reader then
                // classified the resulting node as an item wrapper with no child.
                let seq_indent = self.current_column();
                self.parse_sequence_item(seq_indent)?;
            }
            _ => {
                self.set_ib();
                self.write_bp_open();
                let end_pos = self.parse_unquoted_value_with_indent(min_indent);
                self.set_bp_text_end(end_pos);
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

        // Skip a leading anchor or alias: an aliased key IS the key (`*x: v`,
        // #409) and an anchored key just prefixes it (`&x k: v`, corpus case
        // CN3R) - what determines whether this item is a pair is what
        // follows the name, not the indicator. Uses the same scanner the
        // real anchor/alias parse uses, so this can't drift from what will
        // actually be consumed (#106).
        if i < self.input.len() && (self.input[i] == b'&' || self.input[i] == b'*') {
            let is_alias = self.input[i] == b'*';
            let name_start = i + 1;
            let name_end = simd::parse_anchor_name(self.input, name_start);
            if name_end == name_start {
                // Empty name - not a valid anchor/alias; let the real parse
                // report it.
                return false;
            }
            i = name_end;
            while i < self.input.len() && matches!(self.input[i], b' ' | b'\t') {
                i += 1;
            }
            if is_alias {
                // The alias name is the whole key; nothing else to scan.
                return i < self.input.len() && self.input[i] == b':';
            }
            // An anchor prefixes the actual key, which still needs to be
            // scanned below - it may be quoted, a container, or a plain
            // scalar.
        }

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
                b',' | b']' | b'}' | b'\n' | b'\r' => return false,
                b':' => {
                    let next = if i + 1 < self.input.len() {
                        Some(self.input[i + 1])
                    } else {
                        None
                    };
                    // In flow context, colon must be followed by space, or flow indicator
                    return matches!(next, Some(b' ' | b'\t' | b',' | b']' | b'}') | None);
                }
                _ => i += 1,
            }
        }
        false
    }

    /// Check if we're looking at an explicit key indicator `?` in flow context
    fn looks_like_explicit_flow_key(&self) -> bool {
        self.peek() == Some(b'?') && Self::is_ws_break_or_eoi(self.peek_at(1))
    }

    /// Parse the key node of an explicit flow entry, with the `? ` indicator already
    /// consumed and the key's BP node already open.
    ///
    /// Records the key's own text end. Nested flow containers open and end BP nodes of
    /// their own, so none is recorded for them — doing so would clobber the innermost
    /// of them (#332).
    ///
    /// One definition for the flow-sequence and flow-mapping sites. They were separate
    /// and diverged: the mapping one planted the interest bit on the `?` *before*
    /// consuming it, folding the indicator and its space into the key text (#402).
    fn parse_explicit_flow_key_node(&mut self) -> Result<(), YamlError> {
        match self.peek() {
            Some(b'{') => {
                self.parse_flow_mapping()?;
            }
            Some(b'[') => {
                self.parse_flow_sequence()?;
            }
            Some(b':') => {
                // Empty key (null) - ?: means null key
                // Don't consume anything, write empty node
                self.set_bp_text_end(self.pos);
            }
            Some(b',' | b']' | b'}') => {
                // Empty key (null) - ? followed by terminator
                self.set_bp_text_end(self.pos);
            }
            _ => {
                let key_end = self.parse_explicit_flow_key_scalar()?;
                self.set_bp_text_end(key_end);
            }
        }
        Ok(())
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
        self.parse_explicit_flow_key_node()?;
        self.write_bp_close();

        // Skip whitespace before possible colon
        self.skip_flow_whitespace();

        // Check for colon (explicit value indicator)
        if self.peek() == Some(b':') {
            self.advance();
            self.skip_flow_whitespace();

            // Parse value (if present before , or ])
            if !matches!(self.peek(), Some(b',' | b']' | b'}') | None) {
                self.set_ib();
                self.write_bp_open();
                match self.peek() {
                    // Nested flow containers need no end of their own (#332)
                    Some(b'[') => {
                        self.parse_flow_sequence()?;
                    }
                    Some(b'{') => {
                        self.parse_flow_mapping()?;
                    }
                    _ => {
                        let end = self.parse_flow_scalar()?;
                        self.set_bp_text_end(end);
                    }
                }
                self.write_bp_close();
            } else {
                // Empty value (null)
                self.set_ib();
                self.write_bp_open();
                // Null has no text end
                self.write_bp_close();
            }
        } else {
            // No colon - value is null
            self.write_bp_open_at(self.input.len());
            // Null has no text end
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

        // Parse key - shares `parse_flow_key` with the flow-mapping key site,
        // so an anchor or alias prefixing this key (`[&x k: 1, *x: 2]`,
        // #409) binds via `record_key_anchor` / `record_key_alias` to this
        // already-open key node, rather than `parse_anchor`/`parse_alias`'s
        // "next node opened" semantics binding to the wrong node.
        self.set_ib();
        self.write_bp_open();
        if !self.parse_flow_key()? {
            // A complex key (a nested flow container) opened its own BP nodes
            // and carries its own end; recording one here would clobber the
            // innermost of them (#332).
            self.set_bp_text_end(self.pos);
        }
        self.write_bp_close();

        // Skip whitespace before colon
        self.skip_inline_whitespace();

        // Expect and skip colon
        if self.peek() != Some(b':') {
            return Err(
                self.err_unexpected_char(self.pos, "expected ':' in implicit flow mapping entry")
            );
        }
        self.advance();
        self.skip_flow_whitespace();

        // Parse value (if present before , or ])
        if !matches!(self.peek(), Some(b',' | b']' | b'}') | None) {
            self.set_ib();
            self.write_bp_open();
            match self.peek() {
                // Nested flow containers need no end of their own (#332)
                Some(b'[') => {
                    self.parse_flow_sequence()?;
                }
                Some(b'{') => {
                    self.parse_flow_mapping()?;
                }
                _ => {
                    let val_end = self.parse_flow_scalar()?;
                    self.set_bp_text_end(val_end);
                }
            }
            self.write_bp_close();
        } else {
            // Empty value (null)
            self.set_ib();
            self.write_bp_open();
            // Null has no text end
            self.write_bp_close();
        }

        // Close implicit mapping
        self.write_bp_close();

        Ok(())
    }

    /// Dispatches a flow collection or block scalar starting a block-context
    /// value, shared by every site that can find one there:
    /// `parse_compact_mapping_entry`, `parse_mapping_entry`,
    /// `parse_explicit_value`, `parse_value`. Each used to hand-roll an
    /// identical copy of this 3-arm table — the same bug class recurring
    /// (#325, #372, #406, #224, #785, #864): a hand-copied dispatch missing
    /// an arm, or a stale ordering bug, that a sibling copy already had
    /// (#876). Deliberately narrow: only `[`/`{`/`|`/`>` are shared here —
    /// each site's `-`/quoted/alias/plain-scalar arms differ enough in
    /// content and relative ordering (see each call site) that folding them
    /// in too would trade one duplication bug class for a worse one.
    ///
    /// `permit_colon_terminator` gates #902's widening of #878's validation
    /// (see `reject_trailing_flow_content`): `true` wherever the flow
    /// collection might turn out to be an *implicit mapping key* rather
    /// than a standalone value — real YAML allows `[a, b]: value` /
    /// `{a: 1}: value`, where `:` legitimately follows the closing
    /// delimiter (confirmed against the YAML test suite's own "Implicit
    /// Flow Mapping Key" case). Two of the four callers are that ambiguous:
    /// `parse_value` (reached for a sequence item's value once
    /// `looks_like_mapping_entry` has already ruled out a *scalar*-keyed
    /// compact mapping, and for document-root/deferred content starting
    /// with `[`/`{`) and `parse_explicit_value` (its own value position can
    /// equally be a compact mapping keyed by a flow collection — confirmed
    /// live against real `yq`: `? k\n: [a, b]: value\n` parses as
    /// `{"k":{"":"value"}}`). The other two callers
    /// (`parse_compact_mapping_entry`, `parse_mapping_entry`) only ever
    /// reach this helper *after* an unambiguous `key:` has already been
    /// parsed, where a following `:` cannot be anything but real trailing
    /// garbage — confirmed live: real `yq` rejects `key1: [a, b]: value2`
    /// outright, which an earlier version of this fix (before review
    /// caught it) wrongly stopped rejecting by making the `:` exception
    /// unconditional at every call site instead of gating it here.
    ///
    /// Returns `Ok(true)` if a flow collection or block scalar was found and
    /// fully consumed (the caller does nothing further for this value), or
    /// `Ok(false)` if the current byte isn't one of the four (the caller
    /// falls through to its own remaining arms).
    fn try_dispatch_flow_or_block_value(
        &mut self,
        indent: usize,
        permit_colon_terminator: bool,
    ) -> Result<bool, YamlError> {
        match self.peek() {
            Some(b'[') => {
                self.parse_flow_sequence()?;
                self.reject_trailing_flow_content(permit_colon_terminator)?;
                Ok(true)
            }
            Some(b'{') => {
                self.parse_flow_mapping()?;
                self.reject_trailing_flow_content(permit_colon_terminator)?;
                Ok(true)
            }
            Some(b'|' | b'>') => {
                self.parse_block_scalar(indent)?;
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    /// #878: real YAML (confirmed against real `yq`) treats content after a
    /// flow collection's closing `]`/`}` on the same line — other than
    /// whitespace or a comment — as a hard parse error. Silently continuing
    /// instead (this loader's previous behavior) doesn't trade validation
    /// rigor for permissiveness the way CLAUDE.md's documented
    /// minimal-validation trade-off intends: it corrupts the rest of the
    /// document, dropping every sibling field that follows with no error at
    /// all.
    ///
    /// Deliberately not `at_line_end()`: that helper only treats a plain
    /// space as skippable inline whitespace, not a tab, unlike this file's
    /// own `is_inline_whitespace` (space *or* tab). Every existing caller of
    /// `at_line_end()` only uses it for soft branching, where a stray tab
    /// merely picks a slightly different (but still correct) path — this is
    /// the first caller turning a wrong `false` into a hard error, which
    /// made the gap observable: confirmed live, `at_line_end()` would
    /// false-positive-reject `a: [1, 2]\t# comment`, which real `yq` accepts.
    ///
    /// `permit_colon_terminator` (#902): when `true`, a real mapping-value
    /// indicator (`:` followed by whitespace/break/EOF — the same
    /// disambiguation `looks_like_mapping_entry` uses for a scalar key) is
    /// ALSO a permitted terminator, alongside `#`/whitespace/break/EOF.
    /// When `false`, a `:` here is ordinary trailing garbage like any other
    /// byte, matching #878's original unconditional rejection. This has to
    /// stay a caller-supplied flag, not something this function can infer
    /// from the bytes alone: `[a, b]: value` and `key1: [a, b]: value2` are
    /// byte-identical *after* the closing delimiter, and only the caller
    /// knows whether its own position could legitimately be an implicit
    /// mapping key (see `try_dispatch_flow_or_block_value`'s doc comment
    /// for exactly which callers pass which value, and why an earlier
    /// version of this fix that made the exception unconditional
    /// everywhere silently reopened #878's own corruption bug at the two
    /// callers that need `false`). Confirmed live that a colon with no
    /// following whitespace (`[1, 2]:value`) is never this exception,
    /// regardless of the flag — it still errors the same as any other
    /// trailing garbage either way.
    ///
    /// Only called for `[`/`{` — a block scalar (`|`/`>`) has no equivalent
    /// same-line trailing-content shape to reject, since its own terminator
    /// is a dedent, not a delimiter byte a stray token could follow.
    ///
    /// Consumes the inline whitespace it validates (advancing `self.pos` to
    /// the comment/break/EOI it finds) rather than only checking it (#1186):
    /// this function previously left `self.pos` sitting on the flow
    /// collection's own closing delimiter even after confirming a run of
    /// inline whitespace (space *or* tab) followed. A tab specifically was
    /// then left unconsumed for the caller's line-oriented loop
    /// (`skip_newlines`, which only recognizes a leading *space* as
    /// "possibly a blank/comment line", not a tab) to re-enter
    /// `parse_document_line` mid-line, where `count_indent` -- which
    /// assumes it's only ever called at a genuine line start -- misread the
    /// tab as indentation. Consuming it here instead means the caller's
    /// `self.pos` already sits at the line break (or EOI) by the time
    /// control returns, so that mid-line reentry never happens. The `:`
    /// terminator arm deliberately does *not* advance `self.pos` -- #902's
    /// own compact-mapping-key parsing still needs to see the `:` itself.
    fn reject_trailing_flow_content(
        &mut self,
        permit_colon_terminator: bool,
    ) -> Result<(), YamlError> {
        let mut i = self.pos;
        while i < self.input.len() {
            match self.input[i] {
                b'#' => {
                    self.pos = i;
                    return Ok(());
                }
                b if Self::is_inline_whitespace(b) => i += 1,
                b if Self::is_break(b) => {
                    self.pos = i;
                    return Ok(());
                }
                b':' if permit_colon_terminator
                    && Self::is_ws_break_or_eoi(self.input.get(i + 1).copied()) =>
                {
                    return Ok(());
                }
                _ => break,
            }
        }
        if i >= self.input.len() {
            self.pos = i;
            return Ok(());
        }
        Err(self.err_unexpected_char(i, "after a flow collection's closing delimiter"))
    }

    /// Parse a flow sequence: `[item1, item2, ...]`
    fn parse_flow_sequence(&mut self) -> Result<(), YamlError> {
        self.enter_nested()?;
        let result = self.parse_flow_sequence_inner();
        self.nesting_depth -= 1;
        result
    }

    fn parse_flow_sequence_inner(&mut self) -> Result<(), YamlError> {
        // Mark the `[` position
        self.set_ib();

        // Open sequence container
        let container_bp_pos = self.bp_pos;
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
                    return Err(
                        self.err_unexpected_char(self.pos, "expected ',' or ']' in flow sequence")
                    );
                }
                self.advance(); // Skip `,`
                self.skip_flow_whitespace();

                // Allow trailing comma
                if self.peek() == Some(b']') {
                    break;
                }
            }
            first = false;

            // Check for explicit key `? key : value` in flow context
            if self.looks_like_explicit_flow_key() {
                self.parse_explicit_flow_mapping_entry()?;
            } else if self.looks_like_flow_mapping_entry() {
                // Handles all key forms, including a leading anchor or alias
                // (`[&x k: 1, *x: 2]`, #409), { and [, quoted, and plain scalar
                // keys. This is an implicit single-pair mapping: [ key : value ]
                self.parse_implicit_flow_mapping_entry()?;
            } else {
                // Not a key:value pair - a property (anchor and/or tag) or an
                // alias here prefixes a standalone value instead
                // (`[&x a, *x]`, `[!!str a]`), so it's consumed only now that
                // the pair check has ruled that out.
                self.parse_flow_node_properties()?;
                if self.peek() == Some(b'*') {
                    self.parse_alias()?;
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
                            let end = self.parse_flow_scalar()?;
                            self.set_bp_text_end(end);
                            self.write_bp_close();
                        }
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

        // A trailing comment right after `]` belongs to this sequence as a
        // whole (#710), e.g. `a: [1, 2, 3] # comment`.
        self.maybe_capture_line_comment(container_bp_pos);

        // Close sequence
        self.write_bp_close();

        Ok(())
    }

    /// Parse a flow mapping: `{key: value, ...}`
    fn parse_flow_mapping(&mut self) -> Result<(), YamlError> {
        self.enter_nested()?;
        let result = self.parse_flow_mapping_inner();
        self.nesting_depth -= 1;
        result
    }

    fn parse_flow_mapping_inner(&mut self) -> Result<(), YamlError> {
        // Mark the `{` position
        self.set_ib();

        // Open mapping container
        let container_bp_pos = self.bp_pos;
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
                    return Err(
                        self.err_unexpected_char(self.pos, "expected ',' or '}' in flow mapping")
                    );
                }
                self.advance(); // Skip `,`
                self.skip_flow_whitespace();

                // Allow trailing comma
                if self.peek() == Some(b'}') {
                    break;
                }
            }
            first = false;

            // `? ` is a node marker, not key text: consume it *before* the interest bit
            // is planted, or the indicator and the space after it land inside the key's
            // span. The flow-sequence path has always done it in this order (#402).
            let explicit = self.looks_like_explicit_flow_key();
            if explicit {
                self.advance();
                self.skip_flow_whitespace();
            }

            // Parse key
            self.set_ib();
            self.write_bp_open();
            if explicit {
                // Records its own end, or none at all for a nested flow container.
                self.parse_explicit_flow_key_node()?;
            } else if !self.parse_flow_key()? {
                // A complex key (a nested flow container) opened its own BP nodes and
                // carries its own end; recording one here would clobber the innermost of
                // them (#332).
                self.set_bp_text_end(self.pos);
            }
            self.write_bp_close();

            self.skip_flow_whitespace();

            // Check for colon - if missing, value is implicitly null
            if self.peek() == Some(b':') {
                self.advance(); // Skip `:`
                self.skip_flow_whitespace();

                // Parse value - check for a property or alias first
                // Check for a property prefix on the value
                self.parse_flow_node_properties()?;

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
                            let end = self.parse_flow_scalar()?;
                            self.set_bp_text_end(end);
                            self.write_bp_close();
                        }
                    }
                }
            } else if matches!(self.peek(), Some(b',' | b'}')) {
                // Key without colon/value - emit empty value (implicit null)
                self.set_ib();
                self.write_bp_open();
                // Null has no text end
                self.write_bp_close();
            } else {
                return Err(self.err_unexpected_char(
                    self.pos,
                    "expected ':', ',' or '}' after key in flow mapping",
                ));
            }

            self.skip_flow_whitespace();
        }

        // Skip `}`
        if self.peek() == Some(b'}') {
            self.set_ib();
            self.advance();
        }

        // A trailing comment right after `}` belongs to this mapping as a
        // whole (#710), e.g. `a: {b: 1} # comment`.
        self.maybe_capture_line_comment(container_bp_pos);

        // Close mapping
        self.write_bp_close();

        Ok(())
    }

    /// Parse an *implicit* key in flow context.
    /// Keys can be scalars, flow sequences, or flow mappings (complex keys).
    ///
    /// Shared by both implicit-key sites: a flow mapping's own entries
    /// (`{k: v}`) and a flow sequence's implicit single-pair-mapping entry
    /// (`[k: v]`, since #409 — it used to hand-roll a narrower version of
    /// this match with no anchor/alias arms).
    ///
    /// The explicit `? key` form is not handled here: its indicator has to be consumed
    /// before the caller plants the key's interest bit, so the caller dispatches it to
    /// [`Self::parse_explicit_flow_key_node`] instead (#402).
    ///
    /// Returns `true` when the key opened BP nodes of its own — a nested flow container.
    /// The caller must **not** record an end position in that case — see
    /// [`Self::set_bp_text_end`].
    fn parse_flow_key(&mut self) -> Result<bool, YamlError> {
        // Check for a property (anchor and/or tag, either order) on the key.
        // The caller already opened the key's BP node, so this records
        // `bp_pos - 1`; using `parse_anchor` here bound the anchor to the
        // key's close bit, and aliases to it resolved to the *value* instead
        // of the key (corpus case CN3R). Tags follow the same convention
        // (#224).
        loop {
            match self.peek() {
                Some(b'&') => self.record_key_anchor()?,
                Some(b'!') => self.record_key_tag()?,
                _ => break,
            }
            self.skip_flow_whitespace();
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
                return Ok(true);
            }
            Some(b'{') => {
                // Flow mapping as key (complex key)
                self.parse_flow_mapping()?;
                return Ok(true);
            }
            // Alias as key (`{*a: v}`, and via this same function `[*a: v]`
            // since #409), sharing the block and compact mappings' site. The
            // caller already opened the key's BP node, so the edge binds to
            // that node; `parse_alias` here opened a *second* one, which
            // took the edge and left the key with no extent, so a resolving alias
            // still rendered as `""` while a miss went through `parse_alias`'s
            // lookup and errored — the one key position that stayed inconsistent
            // after #372 (#405). Returning `false` lets the caller record the
            // end, which is the `self.pos` the helper returns.
            Some(b'*') => {
                self.record_key_alias()?;
            }
            _ => {
                self.parse_flow_unquoted_key()?;
            }
        }
        Ok(false)
    }

    /// Parse an unquoted key in flow context.
    /// Stops at `:`, `,`, `}`, `]`, or whitespace before those.
    /// Handles multiline keys (continues across newlines with proper indentation).
    fn parse_flow_unquoted_key(&mut self) -> Result<usize, YamlError> {
        // #224 (was #369's reject-only gate): consume a tag at the start of a
        // flow key. `parse_flow_key` is the only caller, from two call sites
        // (the flow-mapping key and, since #409, the flow-sequence
        // implicit-entry key); it already loops over `&`/`!` and lands here
        // with `pos` at the key's first byte, so this is a no-op re-check in
        // practice — kept so this function stays self-sufficient the way
        // `parse_flow_scalar` and `parse_explicit_flow_unquoted_key` are.
        self.record_flow_tag()?;

        let start = self.pos;

        while let Some(b) = self.peek() {
            match b {
                b':' | b',' | b'}' | b']' => break,
                b'#' => {
                    // # starts a comment only after s-separate-in-line (space or
                    // tab), same rule as the block-key and flow-value arms
                    // (#410, `parse_flow_unquoted_value`). A comment here means
                    // the key never reached its `:`, so this errors the same way
                    // the block-key path does rather than folding the comment
                    // text into the key (#437). Otherwise `#` is ordinary key
                    // content (e.g. `a#b: value`).
                    if self.pos > start && matches!(self.input[self.pos - 1], b' ' | b'\t') {
                        return Err(YamlError::KeyWithoutValue {
                            offset: start,
                            line: self.current_line(),
                        });
                    }
                    self.advance();
                }
                b'\n' | b'\r' => {
                    // Multiline key - check if next line continues the key
                    // Skip the line break (CRLF counts as the one break it is)
                    let mut lookahead = self.pos + self.break_len_at(self.pos);
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
                    while matches!(self.peek(), Some(b' ' | b'\t')) {
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
        // Return absolute end position
        Ok(end)
    }

    /// Parse a scalar value in flow context (string or unquoted).
    fn parse_flow_scalar(&mut self) -> Result<usize, YamlError> {
        // #224 (was #369's reject-only gate): consume a tag at the start of a
        // flow node, the way block context does. Without this the `_` arm
        // below reads `!!str x` as plain scalar *content*, yielding the
        // string "!!str x" instead of resolving the tag. `pos` is at a node
        // start here — anchors, aliases and nested containers are all
        // dispatched by the caller before this point, though not on every
        // path (some callers, like the flow-sequence `[k: v]` shorthand's
        // value, don't strip an anchor first) — so this stays self-sufficient
        // rather than assuming the caller always did it, the same reason
        // `record_key_tag` exists alongside `parse_node_properties`. A `!`
        // inside content is never seen by this check.
        self.record_flow_tag()?;

        let end = match self.peek() {
            Some(b'"') => {
                self.parse_double_quoted()?;
                self.pos
            }
            Some(b'\'') => {
                self.parse_single_quoted()?;
                self.pos
            }
            _ => self.parse_flow_unquoted_value(),
        };
        Ok(end)
    }

    /// Parse an unquoted value in flow context.
    /// Stops at `,`, `}`, `]`, `#` (comment), or newline.
    /// Returns the absolute end position (with trailing whitespace trimmed).
    fn parse_flow_unquoted_value(&mut self) -> usize {
        let start = self.pos;

        while let Some(b) = self.peek() {
            match b {
                b',' | b'}' | b']' => break,
                b'#' => {
                    // # is a comment if preceded by whitespace
                    if self.pos > start && matches!(self.input[self.pos - 1], b' ' | b'\t') {
                        break;
                    }
                    self.advance();
                }
                b'\n' | b'\r' => {
                    // Multiline value - check if next line continues the value
                    // Skip the line break (CRLF counts as the one break it is)
                    let mut lookahead = self.pos + self.break_len_at(self.pos);
                    // Skip leading whitespace on next line
                    while lookahead < self.input.len()
                        && matches!(self.input[lookahead], b' ' | b'\t')
                    {
                        lookahead += 1;
                    }
                    // Check what follows
                    if lookahead >= self.input.len()
                        || matches!(self.input[lookahead], b',' | b'}' | b']' | b'#')
                    {
                        // Delimiter, comment, or EOF - stop value here
                        break;
                    }
                    // Continue parsing on next line
                    self.advance(); // Skip \r if present
                    if self.peek() == Some(b'\n') {
                        self.advance();
                    }
                    // Skip leading whitespace
                    while matches!(self.peek(), Some(b' ' | b'\t')) {
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

        end
    }

    /// Parse an explicit flow key scalar.
    /// Unlike implicit flow keys which stop at `:`, explicit keys stop at `: ` (colon+space)
    /// because the colon is part of the explicit value syntax.
    fn parse_explicit_flow_key_scalar(&mut self) -> Result<usize, YamlError> {
        let end = match self.peek() {
            Some(b'"') => {
                self.parse_double_quoted()?;
                self.pos
            }
            Some(b'\'') => {
                self.parse_single_quoted()?;
                self.pos
            }
            _ => self.parse_explicit_flow_unquoted_key()?,
        };
        Ok(end)
    }

    /// Parse an explicit unquoted key in flow context.
    /// Stops at `: ` (colon followed by whitespace) or flow delimiters, but NOT at bare `:`.
    ///
    /// A `:` ends the key only when a blank, a line break or end-of-input follows it.
    /// `:` before a flow indicator (`,`, `}`, `]`) is ordinary key content and the scan
    /// stops at the indicator instead, so `[? k :, x]` keys on `k :`. That is `yq`'s
    /// rule, and it is a **deliberate divergence from YAML 1.2 §7.3.3**, under which the
    /// colon would be the value indicator and the key just `k` — see spec example 7.3
    /// (corpus case FRK4), which `yq` rejects outright. Chosen for `yq` agreement in
    /// #402; `test_yaml_flow_explicit_key_colon_before_a_flow_indicator_is_content` is
    /// the only guard, since FRK4 is a parses-only corpus case.
    fn parse_explicit_flow_unquoted_key(&mut self) -> Result<usize, YamlError> {
        // #224 (was #369's reject-only gate): the `? key : value` form in flow
        // context is a fourth way to reach a plain scalar at node start.
        // `parse_explicit_flow_key_node` dispatches here with nothing
        // stripped first (it has no anchor/tag handling of its own), so this
        // is where a leading tag genuinely gets consumed for this form.
        self.record_flow_tag()?;

        let start = self.pos;

        while let Some(b) = self.peek() {
            match b {
                b',' | b'}' | b']' => break,
                b'#' => {
                    // Same rule as `parse_flow_unquoted_key` (#437): a `#`
                    // preceded by a space or tab starts a comment, so the key
                    // never reached its value and this errors instead of
                    // folding the comment text into the key. Otherwise `#` is
                    // ordinary key content.
                    if self.pos > start && matches!(self.input[self.pos - 1], b' ' | b'\t') {
                        return Err(YamlError::KeyWithoutValue {
                            offset: start,
                            line: self.current_line(),
                        });
                    }
                    self.advance();
                }
                b':' => {
                    // Only stop at `: ` or `:\n` or `:` at end. A flow indicator after
                    // the colon does *not* stop the key (#402) — `,`/`}`/`]` end it on
                    // their own arm, one byte later, with the colon kept.
                    if Self::is_ws_break_or_eoi(self.peek_at(1)) {
                        break;
                    }
                    // Colon not followed by space - include it in the key
                    self.advance();
                }
                b'\n' | b'\r' => {
                    // Multiline key - check if next line continues the key
                    // Skip the line break (CRLF counts as the one break it is)
                    let mut lookahead = self.pos + self.break_len_at(self.pos);
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
                        && Self::is_ws_or_break(self.input[lookahead + 1])
                    {
                        // Explicit value indicator - stop key here
                        break;
                    }
                    // A `:` before a flow indicator on the next line is *not* a value
                    // indicator — the key continues onto that line and keeps the colon,
                    // so `[? k\n  :, x]` keys on `k :` (#402).
                    // Continue parsing on next line
                    self.advance(); // Skip newline char(s)
                    if self.peek() == Some(b'\n') {
                        self.advance();
                    }
                    // Skip leading whitespace
                    while matches!(self.peek(), Some(b' ' | b'\t')) {
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
                                // Check if colon is followed by space/end. A flow
                                // indicator after it keeps the key going (#402).
                                let after_colon = if lookahead + 1 < self.input.len() {
                                    Some(self.input[lookahead + 1])
                                } else {
                                    None
                                };
                                if Self::is_ws_break_or_eoi(after_colon) {
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
                            _ => {
                                // Continue with the key. A line break lands here too:
                                // walking the blanks hands the decision to the `\n`/`\r`
                                // arm, which is the one that knows whether the next line
                                // continues the key. Stopping here instead made a space
                                // before the break abort the parse — `[? k \n  x : v]`
                                // errored while `[? k\n  x : v]` was fine (#402).
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

        // Return absolute end position
        Ok(end)
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
            // Defensive and carries no coverage: this function is only ever
            // reached via `parse_block_scalar`, and all three call sites of
            // *that* (lines ~1313, ~3278, ~3935) already gate on
            // `matches!(self.peek(), Some(b'|' | b'>'))` immediately before
            // calling, with nothing in between that advances `self.pos` --
            // `set_ib()`/`write_bp_open()` touch only the interest-bit/BP
            // bitvectors. Kept as a backstop for any future caller that
            // dispatches here without that guarantee (same reasoning as the
            // two defensive arms in the main dispatch loop, see
            // `parse_documents`'s own `Some(b'#')`/`Some(b'\n' | b'\r')`
            // comment).
            _ => {
                return Err(
                    self.err_unexpected_char(self.pos, "expected block scalar indicator (| or >)")
                );
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

        // Scan ahead to find first non-empty line
        loop {
            if self.peek().is_none() {
                // EOF - empty block scalar
                self.pos = saved_pos;
                return None;
            }

            // Count spaces at start of line (SIMD accelerated)
            let indent = self.skip_spaces_simd();

            // Check what's on this line
            match self.peek() {
                Some(b'\n' | b'\r') => {
                    // Empty line - skip and continue
                    self.skip_line_break();
                }
                Some(b'#') => {
                    // Comment line - skip to end
                    self.skip_to_eol();
                    self.skip_line_break();
                }
                None => {
                    // EOF
                    self.pos = saved_pos;
                    return None;
                }
                _ => {
                    // Found content - restore position and return indent
                    self.pos = saved_pos;

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
        // Use SIMD to quickly find where the block scalar ends
        let block_end = simd::find_block_scalar_end(self.input, self.pos, content_indent)
            .unwrap_or(self.input.len());

        // Now we need to walk through the content to find:
        // 1. last_content_end - position after last non-empty line
        // 2. trailing_newline_start - where trailing newlines begin
        let mut last_content_end = self.pos;
        let mut trailing_newline_start = self.pos;

        while self.pos < block_end {
            let line_start = self.pos;

            // Count spaces at start of line (SIMD accelerated)
            let _line_indent = self.skip_spaces_simd();

            // Check what's on this line
            match self.peek() {
                Some(b'\n' | b'\r') => {
                    // Empty line - part of trailing newlines
                    trailing_newline_start = line_start;
                    self.skip_line_break();
                }
                None => {
                    break; // EOF
                }
                _ => {
                    // This is a content line - skip to end
                    self.skip_to_eol();
                    last_content_end = self.pos;
                    trailing_newline_start = self.pos;
                    self.skip_line_break();
                }
            }
        }

        // Position should now be at block_end
        self.pos = block_end;

        // Return position based on chomping
        match chomping {
            ChompingIndicator::Strip => last_content_end,
            ChompingIndicator::Clip => {
                // Include one trailing line break if there was content — two
                // bytes wide when that break is a CRLF (#324)
                if last_content_end > 0 && trailing_newline_start > last_content_end {
                    last_content_end + self.break_len_at(last_content_end)
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

        // Skip to end of indicator line, capturing a trailing comment as this
        // block scalar's own line comment first (#710) — `self.last_open_bp_pos`
        // is still this node's bp_pos here since no content/children are
        // parsed yet.
        self.maybe_capture_line_comment(self.last_open_bp_pos);
        self.skip_to_eol();
        self.skip_line_break();

        // Determine content indentation
        let content_indent = if header.explicit_indent > 0 {
            base_indent + header.explicit_indent as usize
        } else {
            // Auto-detect from first content line
            match self.detect_block_content_indent(base_indent) {
                Some(indent) => indent,
                None => {
                    // Empty block scalar. `self.pos` sits at the start of the
                    // line following the header (see `detect_block_content_indent`),
                    // so a `#` there belongs to a following comment/sibling
                    // line, not this scalar — use the no-capture variant
                    // (#710, see `set_bp_text_end_position`'s doc comment).
                    self.set_bp_text_end_position(self.pos);
                    self.write_bp_close();
                    return Ok(());
                }
            }
        };

        // Consume content lines
        let content_end = self.consume_block_scalar_content(content_indent, header.chomping);

        // Close the block scalar node. `self.pos` has been advanced past the
        // block region (potentially past blank lines or a following
        // sibling's comment line) by `consume_block_scalar_content`, so use
        // the no-capture variant — the block's own trailing comment, if any,
        // was already captured on the header line above (#710, see
        // `set_bp_text_end_position`'s doc comment).
        self.set_bp_text_end_position(content_end);
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

        // Use SIMD to find the end of the anchor name (P4 optimization)
        let end = simd::parse_anchor_name(self.input, start);
        self.pos = end;

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
        self.pending_property_bp = Some(self.bp_pos);

        Ok(name)
    }

    /// Record an anchor that prefixes a mapping key whose BP node is already
    /// open, as in `&a k: v`.
    ///
    /// The anchor names the key, so its target is `bp_pos - 1` — the open just
    /// written — not `bp_pos`, which `parse_anchor` would record and which lands
    /// on the key's *close* for a scalar key. Callers that have not yet opened
    /// the anchored node want `parse_anchor` instead.
    ///
    /// One definition for all three key-anchor sites (block, compact and flow
    /// mappings): they diverged before, and the flow one was silently binding
    /// anchors to the value.
    fn record_key_anchor(&mut self) -> Result<(), YamlError> {
        debug_assert_eq!(self.peek(), Some(b'&'));
        self.advance();
        let name = self.parse_anchor_name()?;
        self.skip_inline_whitespace();
        self.anchors.insert(name, self.bp_pos - 1);
        self.pending_property_bp = Some(self.bp_pos - 1);
        Ok(())
    }

    /// Record an alias used as a mapping key whose BP node is already open, as
    /// in `*a: v`, and return the key's text end.
    ///
    /// The alias *is* the key, so the edge is recorded from `bp_pos - 1` — the
    /// open already written — exactly as [`Self::record_key_anchor`] binds an
    /// anchor to it. Callers that have not opened a node yet want
    /// [`Self::parse_alias`], which opens and closes one of its own.
    ///
    /// The end returned is the byte after the name, with no whitespace skipped
    /// — unlike [`Self::record_key_anchor`], where the key text *follows* the
    /// indicator — so a key written `*a : v` has an extent of exactly `*a`.
    ///
    /// One definition for all three key-alias sites (block, compact and flow
    /// mappings), as [`Self::record_key_anchor`] already was for the three
    /// key-*anchor* sites: they were separate copies, only one of which
    /// resolved the alias at all, so `- *a: v` silently produced an empty key
    /// (#372) and `{*a: v}` bound the edge to a node below the key (#405).
    fn record_key_alias(&mut self) -> Result<usize, YamlError> {
        debug_assert_eq!(self.peek(), Some(b'*'));
        // Offset of the `*`, so an unresolved alias can point at itself.
        let alias_start = self.pos;
        self.advance();
        // Alias names follow the anchor-name rules.
        let name = self.parse_anchor_name()?;
        // An anchor/tag scanned for this same (already-open) key node just
        // above means the property was meant for this alias - invalid, since
        // an alias node carries no properties of its own (#1374).
        if self.pending_property_bp == Some(self.bp_pos - 1) {
            return Err(YamlError::PropertyOnAlias {
                offset: alias_start,
                name,
            });
        }
        match self.anchors.get(&name) {
            Some(&target_bp_pos) => {
                self.aliases.insert(self.bp_pos - 1, target_bp_pos);
            }
            // #372, as in `parse_alias`: a miss here rendered the key as the
            // empty string rather than erroring.
            None => {
                return Err(YamlError::UnknownAnchor {
                    offset: alias_start,
                    name,
                });
            }
        }
        Ok(self.pos)
    }

    /// Parse an alias reference (`*name`).
    /// Creates a leaf node in the BP tree pointing to the aliased value.
    fn parse_alias(&mut self) -> Result<(), YamlError> {
        // An anchor/tag scanned for the node about to open here means the
        // property was meant for this alias - invalid, since an alias node
        // carries no properties of its own (#1374). Check before opening the
        // node so the rejected alias never enters the BP tree.
        if self.pending_property_bp == Some(self.bp_pos) {
            // Offset of the `*`, so the error points at the alias itself.
            let alias_start = self.pos;
            self.advance();
            let name = self.parse_anchor_name()?;
            return Err(YamlError::PropertyOnAlias {
                offset: alias_start,
                name,
            });
        }

        // Mark alias position
        self.set_ib();
        self.write_bp_open();

        // Offset of the `*`, so an unresolved alias can point at itself (#372).
        let alias_start = self.pos;

        // Consume `*`
        self.advance();

        // Parse anchor name
        let name = self.parse_anchor_name()?;

        // Resolve alias to anchor at parse time
        // This ensures we get the anchor definition that was active at this point
        let alias_bp_pos = self.bp_pos - 1;
        match self.anchors.get(&name) {
            Some(&target_bp_pos) => {
                self.aliases.insert(alias_bp_pos, target_bp_pos);
            }
            // #372: an unresolved alias used to be dropped on the floor, which
            // left the node with nothing to resolve to and rendered it as
            // `null`. An alias must name a *previous* anchor (YAML 1.2 §7.1),
            // so a miss — forward reference or simply undefined — is invalid
            // input, not a value.
            None => {
                return Err(YamlError::UnknownAnchor {
                    offset: alias_start,
                    name,
                });
            }
        }

        // Close the alias node
        self.set_bp_text_end(self.pos);
        self.write_bp_close();

        Ok(())
    }

    // =========================================================================
    // Tag parsing (#224)
    // =========================================================================

    /// Parse a tag (`!`, `!!suffix`, `!suffix`, `!handle!suffix`, or
    /// `!<verbatim>`) at the current position, returning its raw text
    /// (including the leading `!`/`!!`/`!<...>`).
    ///
    /// Does not record it anywhere or skip trailing whitespace — callers
    /// combine this with anchor handling and decide the right `bp_pos`
    /// convention (see [`Self::parse_node_properties`] and
    /// [`Self::record_key_tag`]), the same split `parse_anchor_name` has from
    /// `parse_anchor`/`record_key_anchor`.
    fn parse_tag(&mut self) -> Result<String, YamlError> {
        let start = self.pos;
        let (end, ok) = scan_tag_extent(self.input, start);
        if !ok {
            return Err(YamlError::InvalidTag {
                offset: start,
                reason: "unterminated verbatim tag (missing closing '>')",
            });
        }
        self.pos = end;
        core::str::from_utf8(&self.input[start..end])
            .map(str::to_string)
            .map_err(|_| YamlError::InvalidUtf8 { offset: start })
    }

    /// Consume any combination of a leading `&anchor` and/or `!tag` before a
    /// node not yet opened, in either order (YAML 1.2's node properties,
    /// `c-ns-properties`), skipping whitespace after each. Anchors are
    /// recorded via [`Self::parse_anchor`] as before; the tag, if present, is
    /// recorded into `self.tags` at the current `self.bp_pos` — the position
    /// the node that follows will occupy once opened, the same assumption
    /// `parse_anchor` alone already made.
    ///
    /// Returns whether any property was present. Some callers need that to
    /// synthesize an empty node for an otherwise-null value: an anchor
    /// records the *next* BP position as its target, and a tag is looked up
    /// *at* that position (`self.tags`), so either would otherwise land on a
    /// sibling's open, on a close bit, or nowhere at all — silently dropping
    /// a bare `!!str` with no value instead of resolving it to `""` (corpus
    /// case LE5A, #224).
    ///
    /// Callers that have already opened the node (a mapping key) want
    /// [`Self::record_key_properties`] instead.
    fn parse_node_properties(&mut self) -> Result<bool, YamlError> {
        let mut had_property = false;
        loop {
            match self.peek() {
                Some(b'&') => {
                    self.parse_anchor()?;
                    had_property = true;
                }
                Some(b'!') => {
                    let tag = self.parse_tag()?;
                    self.tags.insert(self.bp_pos, tag);
                    self.pending_property_bp = Some(self.bp_pos);
                    self.skip_inline_whitespace();
                    had_property = true;
                }
                _ => break,
            }
        }
        Ok(had_property)
    }

    /// Flow-context twin of [`Self::parse_node_properties`]: same loop and
    /// `bp_pos` convention, but skips flow whitespace (which, unlike inline
    /// whitespace, crosses line breaks) after each property instead of only
    /// same-line whitespace.
    ///
    /// Without this, `k: !!seq\n  [a, b]` inside a flow mapping left `pos` on
    /// the line break after the tag, so the `[`/`{` check the caller makes
    /// next saw `\n` instead and fell to the scalar arm, absorbing the
    /// literal `[a, b]` as unquoted text (corpus case EHF6). The same gap
    /// already existed for a bare anchor in this position — `parse_anchor`'s
    /// own internal skip is inline-only too — so this fixes both together
    /// rather than leaving tags inconsistent with anchors.
    fn parse_flow_node_properties(&mut self) -> Result<bool, YamlError> {
        let mut had_property = false;
        loop {
            match self.peek() {
                Some(b'&') => {
                    self.parse_anchor()?;
                    self.skip_flow_whitespace();
                    had_property = true;
                }
                Some(b'!') => {
                    let tag = self.parse_tag()?;
                    self.tags.insert(self.bp_pos, tag);
                    self.pending_property_bp = Some(self.bp_pos);
                    self.skip_flow_whitespace();
                    had_property = true;
                }
                _ => break,
            }
        }
        Ok(had_property)
    }

    /// Record a tag that prefixes a mapping key whose BP node is already
    /// open, as in `!!str k: v`. Mirrors [`Self::record_key_anchor`]'s
    /// `bp_pos - 1` convention: the key's open was already written, so the
    /// tag names *that* position, not the position a fresh [`Self::parse_tag`]
    /// caller (not yet opened) would use.
    fn record_key_tag(&mut self) -> Result<(), YamlError> {
        debug_assert_eq!(self.peek(), Some(b'!'));
        let tag = self.parse_tag()?;
        self.skip_inline_whitespace();
        self.tags.insert(self.bp_pos - 1, tag);
        self.pending_property_bp = Some(self.bp_pos - 1);
        Ok(())
    }

    /// Consume any combination of a leading `&anchor` and/or `!tag` prefixing
    /// a mapping key whose BP node is already open, in either order — the
    /// already-open twin of [`Self::parse_node_properties`], combining
    /// [`Self::record_key_anchor`] and [`Self::record_key_tag`].
    fn record_key_properties(&mut self) -> Result<(), YamlError> {
        loop {
            match self.peek() {
                Some(b'&') => self.record_key_anchor()?,
                Some(b'!') => self.record_key_tag()?,
                _ => break,
            }
        }
        Ok(())
    }

    /// Consume a leading `!tag` when the enclosing node's BP is already open
    /// (`bp_pos - 1`), recording it and skipping flow whitespace after. The
    /// flow-context, value-or-key-agnostic twin of [`Self::record_key_tag`],
    /// shared by [`Self::parse_flow_unquoted_key`], [`Self::parse_flow_scalar`],
    /// and [`Self::parse_explicit_flow_unquoted_key`] — each of those
    /// independently re-checks at its own entry so property consumption never
    /// leaves a stale check behind, the same reason `check_unsupported` used
    /// to be called at all three (#369).
    fn record_flow_tag(&mut self) -> Result<(), YamlError> {
        if self.peek() == Some(b'!') {
            let tag = self.parse_tag()?;
            self.tags.insert(self.bp_pos - 1, tag);
            self.skip_flow_whitespace();
        }
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
        self.push_type(NodeType::Sequence);
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
        self.pop_type();
        self.write_bp_close();

        // Truncate over-allocated bitvectors to actual used length.
        // Parser pre-allocates worst-case (e.g., bp_words at input.len()/32 words)
        // but actual usage is typically much smaller (e.g., 1-2% for sparse YAML).
        let bp_word_count = self.bp_pos.div_ceil(64).max(1);
        let ty_word_count = self.ty_pos.div_ceil(64).max(1);

        let mut bp = core::mem::take(&mut self.bp_words);
        bp.truncate(bp_word_count);
        bp.shrink_to_fit();

        let mut ty = core::mem::take(&mut self.ty_words);
        ty.truncate(ty_word_count);
        ty.shrink_to_fit();

        let mut containers = core::mem::take(&mut self.container_words);
        containers.truncate(bp_word_count);
        containers.shrink_to_fit();

        let mut bp_to_text = core::mem::take(&mut self.bp_to_text);
        bp_to_text.shrink_to_fit();

        let mut bp_to_text_end = core::mem::take(&mut self.bp_to_text_end);
        bp_to_text_end.shrink_to_fit();

        Ok(SemiIndex {
            ib: core::mem::take(&mut self.ib_words),
            bp,
            ty,
            bp_to_text,
            bp_to_text_end,
            containers,
            ib_len: self.input.len(),
            bp_len: self.bp_pos,
            ty_len: self.ty_pos,
            anchors: core::mem::take(&mut self.anchors),
            aliases: core::mem::take(&mut self.aliases),
            tags: core::mem::take(&mut self.tags),
            line_comments: core::mem::take(&mut self.line_comments),
        })
    }

    /// Parse all documents in the stream.
    fn parse_documents(&mut self) -> Result<(), YamlError> {
        // Skip any `%YAML`/`%TAG`/reserved directive lines before the first
        // document (#225).
        self.skip_directives();

        // Skip leading `---` if present (optional for first doc)
        if self.is_document_start() {
            self.skip_document_marker();
            // An explicit `---` always starts a document, content or not
            // (#225) - `end_document` synthesizes null if nothing follows.
            self.start_document();

            // Check for inline content after `---` (e.g., `--- >` or `--- value`)
            if self.has_content_on_line() {
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

        // Start first document if not already started. Guarded on
        // `!is_document_end()`: with no leading `---` at all, an immediate
        // `...` (optionally after only comments/blank lines) means there was
        // never a document to start - `HWV9`/`QT73` expect zero documents,
        // not a phantom null one from `end_document`'s synthesis (#225).
        if !self.in_document && !self.is_document_end() {
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

                // A directive can recur here for the next document, e.g.
                // after a `...` end marker (#225).
                self.skip_directives();

                // Check for another document or EOF (a directive-only tail
                // can itself exhaust the input).
                if self.peek().is_none() {
                    break;
                }

                // If there's a document start marker, skip it and check for inline content
                if self.is_document_start() {
                    self.skip_document_marker();
                    // Unconditional, as above: `---` always starts a document (#225).
                    self.start_document();
                    if self.has_content_on_line() {
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
                // Unconditional, as above: `---` always starts a document (#225).
                self.start_document();

                // Check for inline content after `---` (e.g., `--- >` or `--- value`)
                if self.has_content_on_line() {
                    self.parse_inline_document_value()?;
                } else {
                    self.skip_newlines();
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
        // Snapshot for `drop_stale_pending_head_comment` below (#784): a
        // comment deferred by an anchor on an *earlier* line gets exactly
        // one line's worth of grace to be claimed by this dispatch.
        let pending_head_comment_before = self.pending_head_comment;

        // Count indentation - but handle tabs specially for flow structures
        let indent = match self.count_indent() {
            Ok(n) => n,
            Err(YamlError::TabIndentation { .. }) => {
                // Tabs found - check if this leads to a flow structure
                // Skip all leading whitespace (tabs and spaces)
                while matches!(self.peek(), Some(b' ' | b'\t')) {
                    self.advance();
                }
                // If it's a flow structure, that's allowed
                match self.peek() {
                    Some(b'{' | b'[') => {
                        self.close_deeper_indents(0);
                        self.parse_value(0)?;
                        // Move to next line if we haven't already
                        self.skip_line_break();
                        self.drop_stale_pending_head_comment(pending_head_comment_before);
                        return Ok(());
                    }
                    _ => {
                        // Not a flow structure - re-report the tab error
                        return Err(YamlError::TabIndentation {
                            line: self.current_line(),
                            offset: self.pos,
                        });
                    }
                }
            }
            Err(e) => return Err(e),
        };

        // Skip to content
        let line_start = self.pos;
        self.advance_by(indent);

        // A tab *after* the leading spaces is indentation — and so illegal — only when
        // block structure follows. Before a plain scalar it is separation and legal
        // (DK95/00 `foo:\n \tbar`, UV7Q `x:\n - x\n  \tx`), which is why the test is
        // `line_is_structural` and not "is there a tab" (#173).
        if self.tab_indents_block_structure(line_start) {
            return Err(YamlError::TabIndentation {
                line: self.current_line(),
                offset: self.pos,
            });
        }

        // The tab above was ruled out as (illegal) indentation, so it's
        // separation - skip it, and any more of it, so the dispatch below
        // lands on the line's real first byte instead of the tab (#381).
        self.skip_separation_whitespace(line_start);

        self.parse_block_node(indent)?;

        // Move to next line if we haven't already
        self.skip_line_break();
        self.drop_stale_pending_head_comment(pending_head_comment_before);

        Ok(())
    }

    /// Dispatch the block-context node that begins at the cursor, treating
    /// `indent` as its indentation level.
    ///
    /// One definition for both ways into block context: an ordinary document
    /// line ([`Self::parse_document_line`], which derives `indent` from the
    /// line's leading spaces) and the content of a `---` line
    /// ([`Self::parse_inline_document_value`], always at document root, so
    /// `indent` is 0).
    ///
    /// The `---` line used to carry its own copy of this match, missing arms
    /// and ordering the ones it did have differently, so six shapes parsed one
    /// way on a bare line and another after `---`. `--- &x` with its node on
    /// the next line was the worst of them: the copy opened an empty node for
    /// the anchor to name, and a node at document root *is* a document, so one
    /// document became two (#407).
    fn parse_block_node(&mut self, indent: usize) -> Result<(), YamlError> {
        // close_deeper_indents will handle closing any SequenceItem entries
        // when we return to a lower indent level

        // Check what kind of content this is
        match self.peek() {
            Some(b'-') if Self::is_ws_break_or_eoi(self.peek_at(1)) => {
                // Same ambiguous-gap error as parse_mapping_entry (#901,
                // #959) - a sequence item has no unambiguous owner here
                // either, distinct from #900's SequenceItem-under-Mapping
                // tolerance (`sequence_item_gap_reaches`), which
                // `parse_sequence_item` still applies below this check for
                // its own, different gap shape. `for_sequence_item: true`
                // since this arm closes via
                // `close_deeper_indents_for_sequence_item`, which has its
                // own additional #325/#485 out-dented-continuation
                // tolerance this check must not flag as ambiguous.
                self.check_mapping_under_mapping_gap(indent, true)?;
                self.parse_sequence_item(indent)?;
            }
            // Tab included in both arms below: the sibling `-` arm just above
            // uses the same 4-way terminator set, and `?`/`:` dropping the
            // tab meant `?\tkey` / `:\tvalue` fell through to
            // `looks_like_mapping_entry()` instead of being recognized here
            // (#434).
            Some(b'?') if Self::is_ws_break_or_eoi(self.peek_at(1)) => {
                // Explicit key indicator
                self.parse_explicit_key(indent)?;
            }
            Some(b':') if Self::is_ws_break_or_eoi(self.peek_at(1)) => {
                // Explicit value indicator (value for previous explicit key)
                self.parse_explicit_value(indent)?;
            }
            // These two arms are defensive and carry no coverage: `parse_documents`
            // calls `skip_newlines` before each line, and that consumes comment
            // lines and blank lines (including whitespace-only ones) itself, so a
            // `#` or a line break is never the first byte here. Kept as a backstop
            // for any future caller that dispatches a line without pre-skipping.
            Some(b'#') => {
                // Comment line - skip
                self.skip_to_eol();
            }
            Some(b'\n' | b'\r') => {
                // Empty line
                self.skip_line_break();
            }
            Some(b'{' | b'[') => {
                // Flow mapping or sequence at document root
                // Same ambiguous-gap error as parse_mapping_entry (#901, #959).
                self.check_mapping_under_mapping_gap(indent, false)?;
                self.close_deeper_indents(indent);
                self.parse_value(indent)?;
            }
            Some(b'&' | b'!') => {
                // Anchor and/or tag (either order) - check if this is
                // `&anchor key: value` / `!!str key: value` (property on a
                // mapping key). In that case, let parse_mapping_entry handle
                // it so it points to the key, not the mapping container.
                //
                // This is the shared block-context dispatcher — every "value
                // deferred to the next line" case from parse_sequence_item,
                // parse_mapping_entry, parse_compact_mapping_entry,
                // parse_explicit_key, and parse_explicit_value eventually
                // lands here, so fixing property consumption in this one arm
                // closes most of #664's audit at once (#224).
                //
                // Look ahead to see if this is a `key:` pattern
                if self.node_properties_prefix_mapping_entry() {
                    // Let parse_mapping_entry handle the property, including
                    // its own close_deeper_indents — moved here (rather than
                    // closing unconditionally up front) so a #885 gap-tolerant
                    // indent normalization inside parse_mapping_entry isn't
                    // preempted by an unconditional close running first.
                    self.parse_mapping_entry(indent)?;
                } else {
                    // Same ambiguous-gap error as parse_mapping_entry (#901, #959).
                    self.check_mapping_under_mapping_gap(indent, false)?;
                    self.close_deeper_indents(indent);
                    // Consume any leading `&anchor` and/or `!tag`, in either
                    // order, for non-mapping-key cases
                    self.parse_node_properties()?;
                    // Check what follows
                    match self.peek() {
                        Some(b'\n' | b'\r') | None => {
                            // Property with value on next line - will be parsed in next iteration
                        }
                        // Keep this guard on one line: rustfmt splitting the
                        // `matches!` across lines gives the opening line its own
                        // coverage region that never reports as executed, even
                        // though the arm body does.
                        Some(b'-') if Self::is_ws_break_or_eoi(self.peek_at(1)) => {
                            // Property before block sequence on same line
                            self.parse_sequence_item(indent)?;
                        }
                        Some(b'{' | b'[') => {
                            // Property before flow collection
                            self.parse_value(indent)?;
                        }
                        Some(b'*') => {
                            // Property before alias (`&a *b`) at document-root
                            // level - invalid (an alias node carries no
                            // properties of its own); route through
                            // `parse_alias`, whose `pending_property_bp`
                            // check rejects it (#1374). Real yq accepts this
                            // specific shape but emits corrupted output
                            // rather than erroring - reject uniformly
                            // instead, per docs/compliance/yq/limitations.md.
                            self.parse_alias()?;
                        }
                        _ => {
                            // Scalar value - only doc_root if not inside a container
                            self.set_ib();
                            self.write_bp_open();
                            // type_stack.len() == 1 means we're inside only the virtual root sequence
                            let is_truly_doc_root = self.type_stack.len() <= 1;
                            let end_pos = match self.peek() {
                                Some(b'"') => {
                                    self.parse_double_quoted()?;
                                    self.pos
                                }
                                Some(b'\'') => {
                                    self.parse_single_quoted()?;
                                    self.pos
                                }
                                _ => {
                                    if is_truly_doc_root {
                                        self.parse_unquoted_value_doc_root(indent)
                                    } else {
                                        self.parse_unquoted_value_with_indent(indent)
                                    }
                                }
                            };
                            self.set_bp_text_end(end_pos);
                            // Claim a comment deferred by an earlier anchor's
                            // deferred value (#784), if this scalar's own
                            // line had no comment of its own - run after
                            // `set_bp_text_end`'s own capture just above so
                            // a genuine same-line comment always wins the
                            // slot. This is the property-then-scalar
                            // sibling of the plain-scalar arm below (e.g. a
                            // chained `&y hello` continuing an outer
                            // deferral).
                            self.take_pending_head_comment(self.last_open_bp_pos);
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
                    // Same ambiguous-gap error as parse_mapping_entry (#901, #959).
                    self.check_mapping_under_mapping_gap(indent, false)?;
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
                    // Same ambiguous-gap error as parse_mapping_entry (#901,
                    // #959) - the original repro this issue was filed
                    // against.
                    self.check_mapping_under_mapping_gap(indent, false)?;
                    // Scalar value - either bare document scalar or value in a container
                    self.close_deeper_indents(indent);
                    self.set_ib();
                    self.write_bp_open();
                    // Only use doc_root mode if we're not inside any container
                    // type_stack.len() == 1 means we're inside only the virtual root sequence
                    let is_truly_doc_root = self.type_stack.len() <= 1;
                    let end_pos = match self.peek() {
                        Some(b'"') => {
                            self.parse_double_quoted()?;
                            self.pos
                        }
                        Some(b'\'') => {
                            self.parse_single_quoted()?;
                            self.pos
                        }
                        _ => {
                            if is_truly_doc_root {
                                self.parse_unquoted_value_doc_root(indent)
                            } else {
                                self.parse_unquoted_value_with_indent(indent)
                            }
                        }
                    };
                    self.set_bp_text_end(end_pos);
                    // Claim a comment deferred by an earlier anchor's
                    // deferred value (#784), if this scalar's own line had
                    // no comment of its own - run after `set_bp_text_end`'s
                    // own capture just above so a genuine same-line comment
                    // always wins the slot. This is the single most common
                    // shape a deferred anchor value resolves to: a plain or
                    // quoted scalar folded onto the next line with no
                    // container and no property of its own (`a: &anc #
                    // comment\n  hello`).
                    self.take_pending_head_comment(self.last_open_bp_pos);
                    self.write_bp_close();
                }
            }
            None => {}
        }

        Ok(())
    }
}

/// Scan a tag's raw extent starting at a `!` in `bytes[start..]` (must be
/// `b'!'`), returning `(end, ok)`.
///
/// `ok` is `false` only for a verbatim tag (`!<...>`) that never finds its
/// closing `>` before whitespace, a line break, or end of input — every
/// other shape (`!`, `!!suffix`, `!suffix`, `!handle!suffix`) always
/// succeeds, since a short or empty suffix is still a valid (if unusual)
/// tag. A bare `!` alone is the YAML 1.2 non-specific tag.
///
/// A suffix ends at whitespace, a line break, end of input, or a flow
/// indicator (`,[]{}`) — excluded from `ns-tag-char` everywhere per YAML 1.2
/// §5.5, not just inside flow collections, so this one scan serves both
/// block and flow context.
///
/// A free function (not a `Parser` method) so `light.rs`'s decode-time value
/// reader can share it too: a mapping key's BP node is opened *before* its
/// property is consumed (`record_key_tag`'s `bp_pos - 1` convention), so the
/// key's recorded text span still starts at the tag, exactly as it already
/// does for anchors — `YamlCursor::value`'s `skip_anchor_and_whitespace`
/// exists for the same reason and this is its tag counterpart.
///
/// Permissive by design: shared by the strict [`Parser::parse_tag`] (which
/// turns `ok == false` into `InvalidTag`) and the speculative
/// [`Parser::node_properties_prefix_mapping_entry`] lookahead, which cannot
/// error mid-peek — the same reason `parse_anchor_name` has a permissive
/// twin in [`simd::parse_anchor_name`].
pub(crate) fn scan_tag_extent(bytes: &[u8], start: usize) -> (usize, bool) {
    debug_assert_eq!(bytes.get(start), Some(&b'!'));
    let mut i = start + 1;

    if bytes.get(i) == Some(&b'<') {
        i += 1;
        loop {
            match bytes.get(i) {
                Some(b'>') => return (i + 1, true),
                Some(b) if !matches!(b, b' ' | b'\t' | b'\n' | b'\r') => i += 1,
                _ => return (i, false), // whitespace/break/EOF before `>`
            }
        }
    }

    if bytes.get(i) == Some(&b'!') {
        i += 1; // secondary handle `!!`
    } else {
        let word_start = i;
        while matches!(bytes.get(i), Some(b) if b.is_ascii_alphanumeric() || *b == b'-') {
            i += 1;
        }
        if i > word_start && bytes.get(i) == Some(&b'!') {
            i += 1; // named handle `!handle!`
        }
    }

    while matches!(
        bytes.get(i),
        Some(b) if !matches!(b, b' ' | b'\t' | b'\n' | b'\r' | b',' | b'[' | b']' | b'{' | b'}')
    ) {
        i += 1;
    }

    (i, true)
}

/// Build a semi-index from YAML input.
///
/// # Errors
///
/// Returns [`YamlError::InputTooLarge`] for inputs over `u32::MAX` bytes
/// (just under 4 GiB): the semi-index stores text positions as `u32` (#188).
/// Other variants report malformed YAML. (Pathological YAML can also push the
/// BP bit count past `u32::MAX` before the text does; `BalancedParens`
/// asserts its own ceiling as a loud backstop.)
pub fn build_semi_index(input: &[u8]) -> Result<SemiIndex, YamlError> {
    // Text positions (bp_to_text/bp_to_text_end and the select samples and
    // rank arrays derived from them) are stored as u32, so inputs past
    // u32::MAX bytes would silently truncate offsets (#188). Every position
    // written is <= input.len() (including the input.len() sentinel for null
    // nodes, an exact fit at the maximum), so this single guard makes all
    // downstream u32 casts safe.
    if u32::try_from(input.len()).is_err() {
        return Err(YamlError::InputTooLarge { len: input.len() });
    }
    // One SIMD pass to pick the parser's `HAS_CR` monomorphization (#340). LF-only
    // documents — the overwhelming majority — then parse with every `\r` arm the
    // #324 correctness fix added compiled out. The scan reads the input straight
    // through at 16-32 bytes per iteration and leaves it warm in cache for the
    // parse that follows.
    if crate::util::simd::escape::contains_cr(input) {
        Parser::<true>::new(input).parse()
    } else {
        Parser::<false>::new(input).parse()
    }
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

    /// #1186: unlike the genuine-indentation tab above, a tab that sits
    /// between a flow collection's closing delimiter and a trailing
    /// comment on the *same* line must not be misread as indentation on a
    /// (nonexistent) next line.
    #[test]
    fn test_tab_before_trailing_comment_after_flow_collection_accepted_1186() {
        let yaml = b"a: [1, 2]\t# trailing comment\n";
        let result = build_semi_index(yaml);
        assert!(result.is_ok(), "{result:?}");
    }

    /// #1186 regression guard: a naive fix (widening the general
    /// blank/comment-line skip loop, `skip_newlines`, to treat a leading
    /// tab like a space unconditionally) broke this unrelated, official
    /// YAML Test Suite case (`Y79Y/000`, "Tabs in various contexts") --
    /// a block scalar's own content line containing *only* a tab must
    /// still be rejected, since a tab is never valid indentation there
    /// either. The real fix is scoped to `reject_trailing_flow_content`
    /// alone (only reachable after a flow collection's own closing
    /// delimiter), which never touches block-scalar content at all.
    #[test]
    fn test_tab_only_block_scalar_content_line_still_rejected_1186() {
        let yaml = b"foo: |\n\t\nbar: 1\n";
        let result = build_semi_index(yaml);
        assert!(
            matches!(result, Err(YamlError::TabIndentation { .. })),
            "{result:?}"
        );
    }

    /// #173: a tab following the leading spaces was treated as start-of-content, so
    /// this loaded as `{"a":{"\tb":1}}` — the tab folded into the key — instead of
    /// being rejected as indentation.
    #[test]
    fn regression_issue_173_tab_after_spaces_before_a_mapping_key_is_rejected() {
        let err = build_semi_index(b"a:\n \tb: 1\n").unwrap_err();
        assert!(
            matches!(err, YamlError::TabIndentation { line: 2, offset: 4 }),
            "expected the tab at line 2 offset 4, got {err:?}"
        );
    }

    /// DK95/06. The conformance harness already counted this as rejected because the
    /// opt-in validator caught it; after #173 the default loader catches it too.
    #[test]
    fn regression_issue_173_dk95_06_is_rejected_by_the_loader_not_only_the_validator() {
        let err = build_semi_index(b"foo:\n  a: 1\n  \tb: 2\n").unwrap_err();
        assert!(
            matches!(
                err,
                YamlError::TabIndentation {
                    line: 3,
                    offset: 14
                }
            ),
            "expected the tab at line 3 offset 14, got {err:?}"
        );
    }

    /// #410: the comment guard in `parse_unquoted_key` tested only for a preceding
    /// space, so `a\t# c: d` loaded the comment text into the key as `{"a\t# c":"d"}`
    /// instead of erroring the same way `a # c: d` already did.
    #[test]
    fn regression_issue_410_tab_before_hash_starts_a_comment_in_a_key() {
        let err = build_semi_index(b"a\t# c: d\n").unwrap_err();
        assert!(
            matches!(err, YamlError::KeyWithoutValue { line: 1, offset: 0 }),
            "expected key-without-value at line 1 offset 0, got {err:?}"
        );
    }

    /// #437: `parse_flow_unquoted_key` had no `#` arm at all, so a comment inside
    /// a flow-mapping key folded into the key text instead of erroring, unlike
    /// its block-key (#410) and flow-value siblings which both treat a
    /// whitespace-preceded `#` as a comment.
    #[test]
    fn regression_issue_437_space_before_hash_starts_a_comment_in_a_flow_key() {
        let err = build_semi_index(b"{a # b: c}\n").unwrap_err();
        assert!(
            matches!(err, YamlError::KeyWithoutValue { line: 1, offset: 1 }),
            "expected key-without-value at line 1 offset 1, got {err:?}"
        );
    }

    /// #437: same gap, tab variant.
    #[test]
    fn regression_issue_437_tab_before_hash_starts_a_comment_in_a_flow_key() {
        let err = build_semi_index(b"{a\t# b: c}\n").unwrap_err();
        assert!(
            matches!(err, YamlError::KeyWithoutValue { line: 1, offset: 1 }),
            "expected key-without-value at line 1 offset 1, got {err:?}"
        );
    }

    /// #437: a `#` not preceded by whitespace stays ordinary key content, in flow
    /// context exactly as it already does in block context (`a#b: value`).
    /// Content is pinned via the CLI in
    /// `hash_without_preceding_space_is_flow_key_content` (tests/yq_cli_tests.rs).
    #[test]
    fn hash_without_preceding_space_in_a_flow_key_still_parses() {
        let result = build_semi_index(b"{a#b: c}\n");
        assert!(
            result.is_ok(),
            "expected a#b to parse as key content: {result:?}"
        );
    }

    /// #437: the explicit `? key : value` flow form (`parse_explicit_flow_unquoted_key`)
    /// has the same shape as the implicit key parser and was missing the same `#` arm.
    #[test]
    fn regression_issue_437_space_before_hash_starts_a_comment_in_an_explicit_flow_key() {
        let err = build_semi_index(b"{? a # b : c}\n").unwrap_err();
        assert!(
            matches!(err, YamlError::KeyWithoutValue { line: 1, offset: 3 }),
            "expected key-without-value at line 1 offset 3, got {err:?}"
        );
    }

    /// A tab before a sequence entry is indentation just as much as one before a
    /// key. Starting at offset 0 also exercises the `line_start == 0` arm of
    /// `tab_indents_block_structure`.
    #[test]
    fn tab_after_spaces_before_a_sequence_entry_is_rejected() {
        assert!(matches!(
            build_semi_index(b"  \t- a\n"),
            Err(YamlError::TabIndentation { .. })
        ));
    }

    /// Q5MG and 6CA3: a root flow node's leading separation may contain tabs. These
    /// go through `count_indent`'s column-0 arm and its flow recovery, both of which
    /// #173 left untouched — this pins that.
    #[test]
    fn a_flow_node_after_a_leading_tab_is_still_accepted() {
        assert!(build_semi_index(b"\t{}\n").is_ok());
        assert!(build_semi_index(b"\t[\n\t]\n").is_ok());
    }

    /// A quoted scalar after the tab is a *node*, so the tab is separation — the `:`
    /// inside the quotes is content, not a value indicator. Rejecting these would be
    /// a false positive on valid YAML (same production as DK95/00), which is why
    /// `line_is_structural` skips quoted spans.
    #[test]
    fn a_quoted_scalar_after_the_tab_is_not_indentation() {
        assert!(build_semi_index(b"a:\n \t\"x: y\"\n").is_ok());
        assert!(build_semi_index(b"a:\n \t'x: y'\n").is_ok());
        // A quoted *key*, though, is a mapping entry and the tab really is indentation.
        assert!(matches!(
            build_semi_index(b"a:\n \t\"b\": 1\n"),
            Err(YamlError::TabIndentation { .. })
        ));
    }

    /// `parse_document_line` is not always entered at a line start — the flow scanner
    /// stops just past `]`, leaving the cursor mid-line, and the main loop then
    /// re-derives an "indent" from there. The tab below is separation in the middle
    /// of a line, never indentation, so it must not be reported as such.
    #[test]
    fn a_mid_line_tab_is_not_reported_as_indentation() {
        // Two root nodes: malformed either way. What matters is *which* complaint.
        assert!(!matches!(
            build_semi_index(b"[1] \tfoo: bar\n"),
            Err(YamlError::TabIndentation { .. })
        ));
    }

    #[test]
    fn test_flow_sequence() {
        let yaml = b"items: [1, 2, 3]";
        let result = build_semi_index(yaml);
        assert!(result.is_ok(), "Flow sequence should parse: {result:?}");
    }

    #[test]
    fn test_flow_mapping() {
        let yaml = b"person: {name: Alice, age: 30}";
        let result = build_semi_index(yaml);
        assert!(result.is_ok(), "Flow mapping should parse: {result:?}");
    }

    #[test]
    fn test_flow_nested() {
        let yaml = b"data: {users: [{name: Alice}, {name: Bob}]}";
        let result = build_semi_index(yaml);
        assert!(result.is_ok(), "Nested flow should parse: {result:?}");
    }

    #[test]
    fn test_flow_with_strings() {
        let yaml = b"items: [\"hello\", 'world', plain]";
        let result = build_semi_index(yaml);
        assert!(result.is_ok(), "Flow with strings should parse: {result:?}");
    }

    #[test]
    fn test_flow_trailing_comma() {
        let yaml = b"items: [1, 2, 3,]";
        let result = build_semi_index(yaml);
        assert!(
            result.is_ok(),
            "Flow with trailing comma should parse: {result:?}"
        );
    }

    #[test]
    fn test_flow_empty_sequence() {
        let yaml = b"items: []";
        let result = build_semi_index(yaml);
        assert!(
            result.is_ok(),
            "Empty flow sequence should parse: {result:?}"
        );
    }

    #[test]
    fn test_flow_empty_mapping() {
        let yaml = b"data: {}";
        let result = build_semi_index(yaml);
        assert!(
            result.is_ok(),
            "Empty flow mapping should parse: {result:?}"
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
        assert!(result.is_ok(), "Block literal should parse: {result:?}");
    }

    #[test]
    fn test_block_folded_basic() {
        let yaml = b"text: >\n  line1\n  line2\n";
        let result = build_semi_index(yaml);
        assert!(result.is_ok(), "Block folded should parse: {result:?}");
    }

    #[test]
    fn test_block_literal_strip() {
        let yaml = b"text: |-\n  content\n";
        let result = build_semi_index(yaml);
        assert!(
            result.is_ok(),
            "Block literal strip should parse: {result:?}"
        );
    }

    #[test]
    fn test_block_literal_keep() {
        let yaml = b"text: |+\n  content\n\n\n";
        let result = build_semi_index(yaml);
        assert!(
            result.is_ok(),
            "Block literal keep should parse: {result:?}"
        );
    }

    #[test]
    fn test_block_folded_strip() {
        let yaml = b"text: >-\n  content\n";
        let result = build_semi_index(yaml);
        assert!(
            result.is_ok(),
            "Block folded strip should parse: {result:?}"
        );
    }

    #[test]
    fn test_block_folded_keep() {
        let yaml = b"text: >+\n  content\n\n";
        let result = build_semi_index(yaml);
        assert!(result.is_ok(), "Block folded keep should parse: {result:?}");
    }

    #[test]
    fn test_block_explicit_indent() {
        let yaml = b"text: |2\n  content\n";
        let result = build_semi_index(yaml);
        assert!(
            result.is_ok(),
            "Block with explicit indent should parse: {result:?}"
        );
    }

    #[test]
    fn test_block_explicit_indent_with_chomping() {
        let yaml = b"text: |2-\n  content\n";
        let result = build_semi_index(yaml);
        assert!(
            result.is_ok(),
            "Block with explicit indent and chomping should parse: {result:?}"
        );
    }

    #[test]
    fn test_block_empty() {
        let yaml = b"text: |\nnext: value\n";
        let result = build_semi_index(yaml);
        assert!(
            result.is_ok(),
            "Empty block scalar should parse: {result:?}"
        );
    }

    #[test]
    fn test_block_in_sequence() {
        let yaml = b"- |\n  item\n- value\n";
        let result = build_semi_index(yaml);
        assert!(
            result.is_ok(),
            "Block scalar in sequence should parse: {result:?}"
        );
    }

    #[test]
    fn test_block_with_nested_indent() {
        let yaml = b"code: |\n  def foo():\n    return 42\n";
        let result = build_semi_index(yaml);
        assert!(
            result.is_ok(),
            "Block with nested indent should parse: {result:?}"
        );
    }

    #[test]
    fn test_block_multiple() {
        let yaml = b"one: |\n  first\ntwo: |\n  second\n";
        let result = build_semi_index(yaml);
        assert!(
            result.is_ok(),
            "Multiple block scalars should parse: {result:?}"
        );
    }

    #[test]
    fn test_block_with_comment() {
        let yaml = b"text: | # this is a comment\n  content\n";
        let result = build_semi_index(yaml);
        assert!(
            result.is_ok(),
            "Block scalar with comment should parse: {result:?}"
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
            "explicit document start should parse: {result:?}"
        );
    }

    #[test]
    fn test_two_documents() {
        // Two documents separated by `---`
        let yaml = b"---\nname: Alice\n---\nname: Bob";
        let result = build_semi_index(yaml);
        assert!(result.is_ok(), "two documents should parse: {result:?}");
    }

    #[test]
    fn test_document_end_marker() {
        // Document end marker `...` followed by new document
        let yaml = b"---\nname: Alice\n...\n---\nname: Bob";
        let result = build_semi_index(yaml);
        assert!(
            result.is_ok(),
            "document with end marker should parse: {result:?}"
        );
    }

    #[test]
    fn test_document_end_at_eof() {
        // Document end marker at EOF
        let yaml = b"name: Alice\n...";
        let result = build_semi_index(yaml);
        assert!(
            result.is_ok(),
            "document end at EOF should parse: {result:?}"
        );
    }

    #[test]
    fn test_mixed_document_types() {
        // First document is sequence, second is mapping
        let yaml = b"---\n- item1\n- item2\n---\nkey: value";
        let result = build_semi_index(yaml);
        assert!(
            result.is_ok(),
            "mixed document types should parse: {result:?}"
        );
    }

    #[test]
    fn test_empty_between_markers() {
        // Empty content between document markers
        let yaml = b"---\n---\nname: Alice";
        let result = build_semi_index(yaml);
        assert!(result.is_ok(), "empty document should parse: {result:?}");
    }

    #[test]
    fn test_document_marker_in_flow() {
        // `---` inside a quoted string should not be treated as marker
        let yaml = b"text: \"---\"";
        let result = build_semi_index(yaml);
        assert!(
            result.is_ok(),
            "quoted document marker should parse: {result:?}"
        );
    }

    #[test]
    fn test_three_documents() {
        let yaml = b"---\na: 1\n---\nb: 2\n---\nc: 3";
        let result = build_semi_index(yaml);
        assert!(result.is_ok(), "three documents should parse: {result:?}");
    }

    // =========================================================================
    // Directive tests (#225)
    // =========================================================================

    /// Collects one JSON string per document via the same `uncons_cursor` /
    /// `to_json` path `tests/yaml_test_suite.rs` and `succinctly yq -o json`
    /// both use, so these tests exercise the real read path rather than just
    /// `build_semi_index(..).is_ok()`.
    fn documents(yaml: &[u8]) -> Vec<String> {
        use crate::yaml::{YamlIndex, YamlValue};

        let index = YamlIndex::build(yaml).expect("parse failed");
        let root = index.root(yaml);
        let mut docs = Vec::new();
        match root.value() {
            YamlValue::Sequence(mut elements) => {
                while let Some((cursor, rest)) = elements.uncons_cursor() {
                    docs.push(cursor.to_json());
                    elements = rest;
                }
            }
            _ => docs.push(root.to_json_document()),
        }
        docs
    }

    #[test]
    fn test_yaml_directive_before_single_document() {
        // 27NA: a `%YAML` directive must not absorb the following `---`.
        let docs = documents(b"%YAML 1.2\n--- text\n");
        assert_eq!(docs, vec!["\"text\""]);
    }

    #[test]
    fn test_reserved_directive_ignored() {
        // 2LFX/6LVF: an unknown directive is dropped, not emitted as content.
        let docs = documents(
            b"%FOO  bar baz # Should be ignored\n              # with a warning.\n---\n\"foo\"\n",
        );
        assert_eq!(docs, vec!["\"foo\""]);
    }

    #[test]
    fn test_document_root_scalar_does_not_swallow_next_marker() {
        // A bare scalar with no explicit leading `---` (an implicit first
        // document) used to fold a following `---`/`...` into itself as
        // content, the same underlying gap directives exposed (#225):
        //     $ printf 'Document\n---\nname: Bob\n' | succinctly yq '.'
        //     "Document --- name"      # expected: "Document", then {"name": "Bob"}
        let docs = documents(b"Document\n---\nname: Bob\n");
        assert_eq!(docs, vec!["\"Document\"", "{\"name\":\"Bob\"}"]);
    }

    #[test]
    fn test_misspelled_directive_name_still_skipped() {
        // MUS6/05, MUS6/06: the loader never inspects the directive name, so
        // `%YAM`/`%YAMLL` are skipped exactly like a well-formed `%YAML`.
        assert_eq!(documents(b"%YAM 1.1\n---\n"), vec!["null"]);
        assert_eq!(documents(b"%YAMLL 1.1\n---\n"), vec!["null"]);
    }

    #[test]
    fn test_directive_recurs_after_document_end() {
        // 6ZKB: an implicit first document, an explicit empty second document,
        // and a directive recurring before the third document's `---` - all in
        // one stream.
        let yaml = b"Document\n---\n# Empty\n...\n%YAML 1.2\n---\nmatches %: 20\n";
        assert_eq!(
            documents(yaml),
            vec!["\"Document\"", "null", "{\"matches %\":20}"]
        );
    }

    #[test]
    fn test_explicit_document_start_with_no_content_is_null() {
        // An explicit `---` always starts a document, even with nothing
        // before EOF or the next marker (MUS6/02-06's shape, minus the
        // directive).
        assert_eq!(documents(b"---\n"), vec!["null"]);
    }

    #[test]
    fn test_bare_end_marker_with_nothing_before_it_is_empty_stream() {
        // HWV9/QT73: an end marker with no preceding `---` and no content
        // never started a document, so it must not synthesize a phantom one.
        assert_eq!(documents(b"...\n"), Vec::<String>::new());
        assert_eq!(documents(b"# comment\n...\n"), Vec::<String>::new());
    }

    #[test]
    fn test_question_mark_in_value() {
        // Question mark should be allowed in plain scalar values
        let yaml = b"- a?string";
        let result = build_semi_index(yaml);
        assert!(
            result.is_ok(),
            "question mark in value should parse: {result:?}"
        );
    }

    #[test]
    fn test_question_mark_in_key() {
        // Question mark should be allowed in plain scalar keys
        let yaml = b"key?: value";
        let result = build_semi_index(yaml);
        assert!(
            result.is_ok(),
            "question mark in key should parse: {result:?}"
        );
    }

    #[test]
    fn test_question_mark_in_flow_key() {
        // Question mark in flow mapping key
        let yaml = b"{key?: value}";
        let result = build_semi_index(yaml);
        assert!(
            result.is_ok(),
            "question mark in flow key should parse: {result:?}"
        );
    }

    #[test]
    fn test_question_mark_in_flow_value() {
        // Question mark in flow mapping value
        let yaml = b"{key: value?}";
        let result = build_semi_index(yaml);
        assert!(
            result.is_ok(),
            "question mark in flow value should parse: {result:?}"
        );
    }

    #[test]
    fn test_question_marks_full() {
        // Full JR7V test case - question marks in various contexts
        let yaml = b"- a?string\n- another ? string\n- key: value?\n- [a?string]\n- [another ? string]\n- {key: value? }\n- {key: value?}\n- {key?: value }";
        let result = build_semi_index(yaml);
        assert!(
            result.is_ok(),
            "question marks test should parse: {result:?}"
        );
    }

    #[test]
    fn test_compact_mapping_in_sequence() {
        // This is `- key: value` - a compact mapping within a sequence item
        let yaml = b"- key: value";
        let result = build_semi_index(yaml);
        assert!(
            result.is_ok(),
            "compact mapping in sequence should parse: {result:?}"
        );
    }

    #[test]
    fn test_flow_mapping_in_sequence() {
        // Flow mapping inside sequence item with various spacing patterns
        let yaml = b"- { one : two , three: four , }\n- {five: six,seven : eight}";
        let result = build_semi_index(yaml);
        assert!(
            result.is_ok(),
            "flow mapping in sequence should parse: {result:?}"
        );
    }

    #[test]
    fn test_double_colon_plain_scalar() {
        // ::vector is a plain scalar, not a mapping entry
        let yaml = b"- ::vector";
        let result = build_semi_index(yaml);
        assert!(
            result.is_ok(),
            "double colon plain scalar should parse: {result:?}"
        );
    }

    #[test]
    fn test_quoted_key_mapping() {
        // Quoted keys with special characters
        let yaml = b"\"foo\": bar\n'single': value";
        let result = build_semi_index(yaml);
        assert!(
            result.is_ok(),
            "quoted key mapping should parse: {result:?}"
        );
    }

    #[test]
    fn test_empty_key_in_flow_sequence() {
        // CFD4: [ : empty key ]
        let yaml = b"- [ : empty key ]";
        let result = build_semi_index(yaml);
        assert!(
            result.is_ok(),
            "empty key in flow sequence should parse: {result:?}"
        );
    }

    #[test]
    fn test_explicit_empty_key() {
        // Test: `?\n: value` should parse as {null: "value"}
        use crate::jq::document::DocumentValue;
        use crate::jq::eval_generic::to_owned;
        use crate::yaml::light::YamlValue;
        use crate::yaml::YamlIndex;

        let yaml = b"?\n: value\n";
        let index = YamlIndex::build(yaml).expect("parse failed");

        // Debug: print BP structure
        eprintln!("BP len: {}", index.bp().len());
        for i in 0..index.bp().len() {
            let is_open = index.bp().is_open(i);
            eprintln!("BP {}: {}", i, if is_open { "OPEN" } else { "CLOSE" });
        }

        // Get the first document
        let doc_cursor = index.root(yaml).first_child().expect("no document");
        eprintln!("Doc value: {:?}", doc_cursor.value());

        // Check that it's a mapping
        match doc_cursor.value() {
            YamlValue::Mapping(fields) => {
                let mut count = 0;
                for field in fields {
                    let key_val = field.key();
                    eprintln!("Field: key={:?}, value={:?}", key_val, field.value());
                    // Test as_str on the key
                    eprintln!("  key.as_str() = {:?}", key_val.as_str());
                    eprintln!("  key.is_null() = {:?}", key_val.is_null());
                    count += 1;
                }
                eprintln!("Field count: {count}");
                assert!(
                    count > 0,
                    "mapping should have at least one field, but has {count}"
                );

                // Now test to_owned conversion
                eprintln!("\n=== Testing to_owned conversion ===");
                let owned = to_owned(&doc_cursor.value());
                eprintln!("to_owned result: {owned:?}");
            }
            other => panic!("expected mapping, got {other:?}"),
        }
    }

    // #152: pathological nesting must return an error, not abort the process
    // with a stack overflow. The passing of these tests is itself the proof
    // (an overflow would kill the test harness).

    #[test]
    fn test_deep_flow_sequence_errors_instead_of_aborting() {
        let yaml = format!("a: {}{}", "[".repeat(20_000), "]".repeat(20_000));
        let result = build_semi_index(yaml.as_bytes());
        assert!(matches!(result, Err(YamlError::NestingTooDeep { .. })));
    }

    #[test]
    fn test_deep_flow_mapping_errors_instead_of_aborting() {
        let yaml = "{".repeat(20_000);
        let result = build_semi_index(yaml.as_bytes());
        assert!(matches!(result, Err(YamlError::NestingTooDeep { .. })));
    }

    #[test]
    fn test_deep_alternating_flow_errors_instead_of_aborting() {
        let yaml = "[{".repeat(10_000);
        let result = build_semi_index(yaml.as_bytes());
        assert!(matches!(result, Err(YamlError::NestingTooDeep { .. })));
    }

    #[test]
    fn test_deep_inline_sequence_items_error_instead_of_aborting() {
        let yaml = format!("{}x", "- ".repeat(20_000));
        let result = build_semi_index(yaml.as_bytes());
        assert!(matches!(result, Err(YamlError::NestingTooDeep { .. })));
    }

    #[test]
    fn test_nesting_at_cap_parses() {
        let yaml = format!(
            "a: {}{}",
            "[".repeat(MAX_NESTING_DEPTH),
            "]".repeat(MAX_NESTING_DEPTH)
        );
        assert!(build_semi_index(yaml.as_bytes()).is_ok());
    }

    #[test]
    fn test_nesting_one_past_cap_errors() {
        let yaml = format!(
            "a: {}{}",
            "[".repeat(MAX_NESTING_DEPTH + 1),
            "]".repeat(MAX_NESTING_DEPTH + 1)
        );
        let result = build_semi_index(yaml.as_bytes());
        // The 129th `[` sits after the 3-byte `a: ` prefix and 128 accepted `[`s.
        assert!(matches!(
            result,
            Err(YamlError::NestingTooDeep { offset, limit })
                if limit == MAX_NESTING_DEPTH && offset == 3 + MAX_NESTING_DEPTH
        ));
    }

    #[test]
    fn test_nesting_too_deep_display() {
        let err = YamlError::NestingTooDeep {
            offset: 131,
            limit: 128,
        };
        assert_eq!(
            err.to_string(),
            "nesting depth exceeds limit of 128 at offset 131"
        );
    }
    /// The other #106 guard: [`Parser::is_ws_break_or_eoi`] is the parser's
    /// spelling of the terminator set [`super::super::is_seq_indicator_next`]
    /// gives the reader (#332), and the two are pinned by this test rather than
    /// by the compiler — see the doc comment for why the parser cannot just call
    /// it. Exhaustive over every byte plus end of input, so a divergence in the
    /// acceptance set cannot hide in an untested byte.
    ///
    /// `Parser::<true>` is the unconditional rule and must match the reader
    /// outright. `Parser::<false>` is the #340 specialization, and the only
    /// licence it has is to drop `\r` — a document the precheck routed there
    /// contains none, so the answer is unobservable. Pinning both directions
    /// keeps the gate from quietly widening into some other byte.
    #[test]
    fn is_ws_break_or_eoi_agrees_with_is_seq_indicator_next() {
        for next in (0..=u8::MAX).map(Some).chain(core::iter::once(None)) {
            let reader = crate::yaml::is_seq_indicator_next(next);

            assert_eq!(
                Parser::<true>::is_ws_break_or_eoi(next),
                reader,
                "{next:?}: parser and reader disagree on the indicator terminator set"
            );
            assert_eq!(
                Parser::<false>::is_ws_break_or_eoi(next),
                reader && next != Some(b'\r'),
                "{next:?}: the HAS_CR gate changed something other than the `\\r` arm"
            );
        }
    }

    /// The #106 guard: `skip_line_break` is the one place in the crate that
    /// restates [`line_break_len`] instead of calling it, for the measured
    /// reason on its doc comment. A restated predicate diverges silently, so
    /// pin the two together over every break form and every position — the
    /// widths must agree byte for byte.
    #[test]
    fn skip_line_break_agrees_with_line_break_len() {
        // CR-bearing inputs only reach the `HAS_CR == true` parser, which is what
        // `build_semi_index`'s precheck guarantees.
        skip_line_break_agrees::<true>(&[
            b"a\r\nb\rc\nd",
            b"\r\n",
            b"\n\r",
            b"\r\r\n",
            b"\n\n",
            b"a\r",
            b"a\n",
            b"x",
            b"",
        ]);
        // The #340 specialization has to hold the same property against its own
        // narrowed `break_len_at`, or its `skip_line_break` fast path would be a
        // third spelling rather than a second one.
        skip_line_break_agrees::<false>(&[b"a\nb\nc", b"\n\n", b"a\n", b"\n", b"x", b""]);
    }

    /// Shared body of the #106 guard, over both `HAS_CR` monomorphizations.
    ///
    /// Under `HAS_CR` the expected width is tied to [`line_break_len`] itself, so
    /// the hand-rolled `skip_line_break`, the `break_len_at` dispatcher, and the
    /// shared definition are all pinned to each other.
    fn skip_line_break_agrees<const HAS_CR: bool>(inputs: &[&[u8]]) {
        for input in inputs {
            for pos in 0..=input.len() {
                let want = Parser::<HAS_CR>::new(input).break_len_at(pos);
                if HAS_CR {
                    assert_eq!(
                        want,
                        line_break_len(input, pos),
                        "{input:?} @ {pos}: break_len_at and line_break_len disagree"
                    );
                }
                let mut parser = Parser::<HAS_CR>::new(input);
                parser.pos = pos;
                parser.skip_line_break();
                assert_eq!(
                    parser.pos - pos,
                    want,
                    "{input:?} @ {pos} (HAS_CR={HAS_CR}): \
                     skip_line_break and break_len_at disagree"
                );
            }
        }
    }

    /// `skip_line_break` consumes exactly one break, never half of a CRLF and
    /// never a byte when the cursor is not on one.
    #[test]
    fn skip_line_break_consumes_exactly_one_break() {
        let mut parser = Parser::<true>::new(b"\r\n\r\nx");
        parser.skip_line_break();
        assert_eq!(parser.pos, 2, "first CRLF consumed whole");
        parser.skip_line_break();
        assert_eq!(parser.pos, 4, "second CRLF consumed whole");
        parser.skip_line_break();
        assert_eq!(parser.pos, 4, "`x` is not a break, so nothing moves");

        let mut lone = Parser::<true>::new(b"\r\rx");
        lone.skip_line_break();
        assert_eq!(lone.pos, 1, "a lone CR is a complete break");
        assert!(lone.at_break());
        lone.skip_line_break();
        assert_eq!(lone.pos, 2);
        assert!(!lone.at_break());
    }

    /// `current_line` counts breaks, and a CRLF is one of them — not two.
    #[test]
    fn current_line_counts_a_crlf_once() {
        // b"a\r\nb\r\nc"
        //   0 1 2 3 4 5 6
        let mut parser = Parser::<true>::new(b"a\r\nb\r\nc");
        assert_eq!(parser.current_line(), 1);
        parser.pos = 3;
        assert_eq!(parser.current_line(), 2, "`b` is on line 2");
        // Sitting on the LF of the second CRLF is still line 2: that CR's
        // partner lies past the counted prefix and must not read as a lone CR.
        parser.pos = 5;
        assert_eq!(
            parser.current_line(),
            2,
            "the LF of a CRLF is not a new line"
        );
        parser.pos = 6;
        assert_eq!(parser.current_line(), 3, "`c` is on line 3");

        let mut lone = Parser::<true>::new(b"a\rb\rc");
        lone.pos = 4;
        assert_eq!(lone.current_line(), 3, "lone CRs each start a line");
    }

    /// The two `HAS_CR` monomorphizations must agree on every CR-free document.
    ///
    /// This is the test that pins the #340 gating. `HAS_CR == true` is the #324
    /// parser verbatim, so it is the oracle here; `HAS_CR == false` is the
    /// specialization, and the only way it can be wrong is by gating a site whose
    /// `\r` arm was doing something *other* than handling a carriage return.
    /// Nothing else in the suite would catch that: `tests/yaml_crlf_tests.rs` and
    /// the CRLF reruns in `tests/yq_golden_tests.rs` exercise inputs that all
    /// contain a `\r`, which is precisely the set that takes the `true` path.
    #[test]
    fn both_monomorphizations_agree_on_cr_free_input() {
        // 4096 `a`s, so the plain-scalar case clears the 32-byte SIMD classifier
        // threshold by a wide margin rather than only just.
        let long_scalar = format!("long: {}\n", "a".repeat(4096));
        let cases: &[&str] = &[
            "",
            "\n",
            "name: Alice\nage: 30\n",
            "person:\n  name: Alice\n  address:\n    city: Sydney\n",
            "- one\n- two\n- three\n",
            "- - nested\n  - items\n",
            "quoted: \"has: colon\"\nsingle: 'and ''escaped'' quotes'\n",
            "flow_map: {a: 1, b: 2}\nflow_seq: [1, 2, [3, 4]]\n",
            "literal: |\n  line one\n  line two\nfolded: >\n  folded one\n\n  folded two\n",
            "? explicit key\n: explicit value\n",
            "base: &anchor\n  a: 1\nderived: *anchor\n",
            "---\ndoc: one\n---\ndoc: two\n...\n",
            "# leading comment\nkey: value # trailing comment\n\n\nafter: blanks\n",
            "empty:\nnull_value: ~\nbool: true\nnum: 1.5\n",
            "url: http://example.com/path\ntime: 12:30:00\n",
            "  indented_root: value\n",
            "no trailing newline: here",
            "plain\nmultiline\nscalar\n",
            "outer:\n- a\n- b\n",
            &long_scalar,
        ];

        for case in cases {
            let input = case.as_bytes();
            assert!(!input.contains(&b'\r'), "fixture must be CR-free: {case:?}");

            let with = Parser::<true>::new(input).parse();
            let without = Parser::<false>::new(input).parse();

            match (with, without) {
                (Ok(a), Ok(b)) => {
                    assert_eq!(a.ib, b.ib, "ib differs for {case:?}");
                    assert_eq!(a.bp, b.bp, "bp differs for {case:?}");
                    assert_eq!(a.ty, b.ty, "ty differs for {case:?}");
                    assert_eq!(
                        a.bp_to_text, b.bp_to_text,
                        "bp_to_text differs for {case:?}"
                    );
                    assert_eq!(
                        a.bp_to_text_end, b.bp_to_text_end,
                        "bp_to_text_end differs for {case:?}"
                    );
                    assert_eq!(
                        a.containers, b.containers,
                        "containers differs for {case:?}"
                    );
                    assert_eq!(a.ib_len, b.ib_len, "ib_len differs for {case:?}");
                    assert_eq!(a.bp_len, b.bp_len, "bp_len differs for {case:?}");
                    assert_eq!(a.ty_len, b.ty_len, "ty_len differs for {case:?}");
                    assert_eq!(a.anchors, b.anchors, "anchors differ for {case:?}");
                    assert_eq!(a.aliases, b.aliases, "aliases differ for {case:?}");
                }
                (Err(a), Err(b)) => {
                    assert_eq!(a.to_string(), b.to_string(), "errors differ for {case:?}");
                }
                (a, b) => panic!("acceptance differs for {case:?}: {a:?} vs {b:?}"),
            }
        }
    }

    /// A verbatim tag (`!<...>`) that hits whitespace before its closing `>`
    /// is malformed - `scan_tag_extent` bails out at the space rather than
    /// treating it as part of the tag, and `parse_tag` turns that into
    /// `InvalidTag`.
    #[test]
    fn verbatim_tag_unterminated_before_whitespace_is_invalid_tag() {
        let err = build_semi_index(b"a: !<foo bar\n").unwrap_err();
        assert!(
            matches!(
                err,
                YamlError::InvalidTag {
                    offset: 3,
                    reason: "unterminated verbatim tag (missing closing '>')",
                }
            ),
            "expected InvalidTag at offset 3, got {err:?}"
        );
    }

    /// Same malformed-verbatim-tag case, but the input ends before the
    /// closing `>` is ever found - `scan_tag_extent`'s scan loop must reject
    /// end-of-input the same way it rejects whitespace, not run off the end
    /// of the buffer.
    #[test]
    fn verbatim_tag_unterminated_at_eof_is_invalid_tag() {
        let err = build_semi_index(b"a: !<foo").unwrap_err();
        assert!(
            matches!(
                err,
                YamlError::InvalidTag {
                    offset: 3,
                    reason: "unterminated verbatim tag (missing closing '>')",
                }
            ),
            "expected InvalidTag at offset 3, got {err:?}"
        );
    }

    /// A multi-line plain scalar must stop folding when the next line is a
    /// lone `: ` (explicit-key value indicator), rather than swallowing it as
    /// scalar content - the guard in
    /// `parse_unquoted_value_with_indent_impl` that checks
    /// `next_char == b':' && is_ws_break_or_eoi(..)` alongside
    /// `indent_allows_continuation`.
    ///
    /// The extra indentation on both lines matters for actually exercising
    /// that guard: `indent_allows_continuation` only becomes `true` (and the
    /// `&&`-chain only reaches the colon check) when the next line is
    /// indented *past* the key's `start_indent` (here, past the mapping's
    /// indent of 0) - a bare `"? a\n: 1\n"` never reaches the check at all,
    /// since `next_indent (0) > start_indent (0)` is already false and the
    /// rest of the condition short-circuits. `"  : 1"` is still less
    /// indented than the key `a` itself (column 4), matching the YAML-1.2
    /// corpus shape (id `35KP`, "Tags for Root Objects") that used to
    /// exercise this before this PR's tag-support change routed that corpus
    /// case through a different path (#224).
    ///
    /// #1010: this `:` (column 2) doesn't actually align with its own `?`
    /// (column 0) either, which real yq rejects ("did not find expected
    /// key") -- confirmed live. Before #1010's fix, `parse_explicit_value`
    /// had no alignment check at all, so this silently resolved to `{"a":1}`
    /// once the guard above stopped the scalar from swallowing `: 1`; the
    /// guard's own job (stopping the scalar there rather than absorbing the
    /// line as text) is unchanged and still exercised here, but the
    /// now-separated `: 1` line is correctly rejected instead of silently
    /// accepted.
    #[test]
    fn multiline_plain_scalar_stops_before_explicit_value_indicator() {
        let yaml: &[u8] = b"?   a\n  : 1\n";
        let err = crate::yaml::YamlIndex::build(yaml).unwrap_err();
        assert!(
            matches!(err, YamlError::InconsistentIndentation { line: 2, .. }),
            "explicit key 'a's scalar must stop before ': 1' (not swallow it into the key text), \
             and the resulting misaligned ':' must then be rejected, not silently paired with 'a'; got {err:?}"
        );
    }

    /// `parse_value`'s `Some(b'-') if is_ws_break_or_eoi(..)` arm — an
    /// inline-sequence-item fallback for a `-` that reaches `parse_value`
    /// itself rather than being intercepted by a caller's own copy of the
    /// same check — is still reachable after this PR replaced single
    /// `parse_anchor()` calls with looping `parse_node_properties()` calls in
    /// both `parse_value` and `parse_sequence_item_inner`.
    ///
    /// That refactor *does* make the arm dead for the scenario the arm's own
    /// comment names (`- &a &b - x`, a sequence item): `parse_sequence_item_inner`
    /// now consumes *both* leading anchors in one loop and its own `Some(b'-')`
    /// check (just above the property loop's result) catches the inner dash
    /// directly, never delegating to `parse_value` at all - confirmed by
    /// coverage (`- &a &b - x\n` alone does not hit this arm).
    ///
    /// It stays reachable through a different caller: `parse_explicit_key`'s
    /// "sequence as key" arm (`? - ...`) skips only the *first* `-` and then
    /// calls `parse_value` unconditionally for whatever follows, with no
    /// dash-precheck of its own (unlike `parse_sequence_item_inner` and
    /// `parse_mapping_entry`). A second `-` there - anchored or not - lands
    /// on this exact arm. Confirmed via `cargo llvm-cov`: this input hits
    /// parser.rs:2741-2742, `"- &a &b - x\n"` alone does not.
    #[test]
    fn explicit_key_double_anchored_nested_dash_reaches_parse_value_dash_arm() {
        let yaml: &[u8] = b"? - &a &b - x\n: v\n";
        let index = crate::yaml::YamlIndex::build(yaml).expect("should parse");
        let root = index.root(yaml);
        let doc0 = match root.value() {
            crate::yaml::YamlValue::Sequence(mut docs) => docs.next().expect("one document"),
            other => panic!("expected root sequence, got {other:?}"),
        };
        let fields = match doc0 {
            crate::yaml::YamlValue::Mapping(fields) => fields,
            other => panic!("expected mapping, got {other:?}"),
        };
        let (field, rest) = fields.uncons().expect("mapping has one field");
        assert!(rest.is_empty(), "mapping must have exactly one field");

        // Key: a sequence (from `? - ...`) whose single element is itself a
        // sequence (the nested `- x`, opened by this PR's arm) containing "x".
        match field.key() {
            crate::yaml::YamlValue::Sequence(mut outer) => {
                let inner = outer.next().expect("outer sequence has one element");
                assert!(
                    outer.next().is_none(),
                    "outer sequence has only one element"
                );
                match inner {
                    crate::yaml::YamlValue::Sequence(mut inner_seq) => {
                        let x = inner_seq.next().expect("inner sequence has one element");
                        assert!(
                            inner_seq.next().is_none(),
                            "inner sequence has only one element"
                        );
                        match x {
                            crate::yaml::YamlValue::String(s) => {
                                assert_eq!(s.as_str().unwrap(), "x");
                            }
                            other => panic!("expected string 'x', got {other:?}"),
                        }
                    }
                    other => panic!("expected nested sequence key, got {other:?}"),
                }
            }
            other => panic!("expected sequence key, got {other:?}"),
        }

        // Value: the explicit value `v` on the following line.
        match field.value_cursor().value() {
            crate::yaml::YamlValue::String(s) => assert_eq!(s.as_str().unwrap(), "v"),
            other => panic!("expected string 'v' value, got {other:?}"),
        }
    }
}
