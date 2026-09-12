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

/// Head/line/foot comments attached to one node, keyed by its own bp
/// position (#798). `head`/`foot` hold zero or more whole comment-only
/// lines (consecutive lines are one logical block, hence `Vec` rather than
/// `Option` -- real yq joins them with a newline either way). `line` is
/// also a `Vec` (#1085): a mapping key's trailing-comment slot can hold
/// *two* entries at once -- a comment floated onto it from an earlier
/// anchor's deferred value, and the key's own genuine same-line comment --
/// and real yq renders both (the first inline, any more as standalone
/// lines) rather than letting one silently clobber the other. Every other
/// node kind still ever gets at most one `line` entry. A node can carry
/// all three fields at once.
///
/// Every range is raw, `#` included: the reader strips a leading `"# "` at
/// the point of use, which is what makes `#\ttabbed` and a bare `#` come
/// back verbatim, matching real yq.
#[derive(Debug, Clone, Default)]
pub struct NodeComments {
    /// Standalone `#` lines directly above the node, in source order. For a
    /// mapping entry these live on its *key* node; for a sequence item, on
    /// the item's content node -- which for a same-line compact `- k: v` or
    /// nested `- - x` item is the mapping/sequence node itself, the one
    /// `.[i]` resolves to, not its first key/item (#2811). See
    /// [`Parser::record_standalone_comment`].
    pub head: Vec<(u32, u32)>,
    /// The node's own trailing same-line comment(s): `(start, end)` byte
    /// ranges of the raw comment text, each starting at `#` and running to
    /// end of line (exclusive of the line break, inclusive of any trailing
    /// whitespace), in source order. See the struct doc comment for why
    /// this can hold more than one entry (#1085).
    pub line: Vec<(u32, u32)>,
    /// Standalone `#` lines directly below the node, in source order. See
    /// [`Self::head`].
    pub foot: Vec<(u32, u32)>,
}

/// An open block sequence, as [`Parser::seq_frames`] tracks it (#1079).
///
/// This is a second stack, parallel to (not folded into) `indent_stack`/
/// `type_stack` -- tempting to simplify by widening `NodeType::Sequence`
/// into a payload-carrying variant and dropping `serial`/`frame_len`
/// instead, reading a sequence's identity straight off those two. That
/// simplification is unsound: `serial` is load-bearing, not redundant with
/// depth. Two sibling items nested inline (`- - # c1\n- - # c2\n`) each open
/// an inner sequence at the *same* stack depth with no settle checkpoint in
/// between -- an inline `- - x` item never calls `attach_head_foot_at` for
/// its own dash, it recurses straight into the nested sequence -- so by the
/// time item 1's inner sequence registers, it has already replaced item 0's
/// at that identical depth. A depth-only check reads that as "still open"
/// and never settles item 0's floated comment at all
/// (`test_seq_frame_serial_distinguishes_sibling_frames_at_the_same_depth_1079_float`
/// pins the diverging case this produces against real yq; confirmed by a
/// throwaway experimental patch during #1079's review).
#[derive(Debug, Clone, Copy)]
struct SeqFrame {
    /// Identity that survives the frame's depth being reused by a later
    /// sequence -- see [`Parser::next_seq_serial`].
    serial: u32,
    /// `indent_stack.len()` while this sequence is the top frame; only
    /// used to drop entries whose frame has since been popped.
    frame_len: usize,
    /// The column of the sequence's `-` markers: a later node at a lower
    /// column is a dedent out of it, which is when a floated comment can
    /// become a foot at all.
    indent: usize,
    /// The nearest enclosing mapping key -- the key this sequence is the
    /// value of, or the enclosing sequence's own when this one is nested in
    /// an item: the node real yq gives a floated comment to as its `foot`
    /// when the sequence closes to a following key.
    owner_key_bp: Option<usize>,
    /// Whether that key is this sequence's *own* (parent frame a mapping)
    /// rather than inherited through an item. A directly-owned sequence's
    /// floated comment is always the key's foot; an inherited one's is the
    /// next outer item's when that is what opens next.
    direct_key_value: bool,
}

/// A comment floated off an absent bare sequence item -- see
/// [`Parser::floated_item_comment`] (#1079).
#[derive(Debug, Clone, Copy)]
struct FloatedItemComment {
    /// Its byte range, as also held in [`Parser::pending_head_lines`].
    range: (u32, u32),
    /// The sequence the item belonged to, by [`SeqFrame::serial`].
    seq_serial: u32,
    /// That sequence's [`SeqFrame::indent`].
    seq_indent: usize,
    /// That sequence's [`SeqFrame::owner_key_bp`].
    owner_key_bp: Option<usize>,
    /// That sequence's [`SeqFrame::direct_key_value`].
    direct_key_value: bool,
}

/// One open block (a mapping or a block sequence) as
/// [`PendingBlock::frames`] snapshots it (#2811).
#[derive(Debug, Clone, Copy)]
struct CommentFrame {
    /// The block's indent: the column its keys / `-` markers sit at.
    indent: usize,
    /// Mapping (`true`) or block sequence (`false`).
    is_mapping: bool,
    /// For a mapping, the bp of the last key opened in it so far -- the
    /// node real yq gives a foot that lands on the mapping's *end* to.
    last_key_bp: Option<usize>,
}

/// What [`Parser::flush_pending_head_lines`] needs to know about the
/// standalone block in [`Parser::pending_head_lines`] beyond its byte
/// ranges (#2811): where it sits relative to the blocks open around it.
/// Captured when the block's first line is recorded; meaningless while
/// `pending_head_lines` is empty.
#[derive(Debug, Clone, Default)]
struct PendingBlock {
    /// Column of the block's first line (go-yaml's `start_mark.column`).
    column: usize,
    /// Indent of the innermost open block when the block was recorded --
    /// go-yaml's `parser.indent`, the reference point for "dedented".
    prev_indent: usize,
    /// The open blocks at record time, outermost first, only filled when
    /// `column < prev_indent` (the one case that consults them).
    frames: Vec<CommentFrame>,
    /// Whether this block began where a column change split it off an
    /// earlier one. go-yaml then keys it to its own position rather than to
    /// the prior node, so it is placed by column even when adjacent to what
    /// follows.
    after_split: bool,
    /// Whether a blank line separates the block from `PREV`. That detaches
    /// it from `PREV` for every placement except one: adjacent to a node
    /// that dedents out of `PREV`'s block and not at that node's column, it
    /// is still `PREV`'s foot (`    k: 1` / blank / ` # c` / `  z: 2` is
    /// `.k`'s key's foot, measured), so `PREV` itself is kept and this
    /// consulted instead.
    blank_before: bool,
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
    /// Reverse anchor mapping: BP position → its own anchor name, one entry
    /// per `&name` declaration in source order (#1353) -- see the identically
    /// named `Parser` field's doc comment for why this isn't derived from
    /// `anchors` above by inversion.
    pub bp_to_anchor: BTreeMap<usize, String>,
    /// Alias references: BP position of alias → target BP position (resolved at parse time)
    pub aliases: BTreeMap<usize, usize>,
    /// Explicit source tags: BP position → raw tag text (see [`YamlIndex::get_tag`](super::index::YamlIndex::get_tag))
    pub tags: BTreeMap<usize, String>,
    /// Head/line/foot comments, keyed by the BP position of the node they
    /// attach to. See [`NodeComments`].
    pub comments: BTreeMap<usize, NodeComments>,
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
    /// Anchors collected during parsing: name → bp_pos of anchored value.
    /// Last-wins on a redefined name, which is exactly the resolution rule
    /// an alias needs (YAML: an alias refers to the most recent preceding
    /// anchor of that name) — see `bp_to_anchor` below for the position
    /// that rule intentionally discards.
    anchors: BTreeMap<String, usize>,
    /// Reverse anchor mapping: bp_pos of the anchored value/key → its own
    /// anchor name (#1353). Populated directly at each `&name` site
    /// (`parse_anchor`/`record_key_anchor`), *not* derived from `anchors`
    /// by inversion — inverting a last-wins map would silently lose every
    /// declaration but the final one when a name is redefined, dropping
    /// `&x` from `a: &x 1\nb: &x 2\nc: *x` on re-emission even though `a`'s
    /// own declaration is still in the source and real yq keeps it.
    bp_to_anchor: BTreeMap<usize, String>,
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
    /// Head/line/foot comments collected during parsing, bp_pos of the
    /// owning node → [`NodeComments`]. See [`SemiIndex::comments`].
    comments: BTreeMap<usize, NodeComments>,

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

    /// A run of standalone `#` lines awaiting forward attachment as the
    /// *next* node's `head` (#798). Distinct from
    /// [`Self::pending_head_comment`], which despite its name carries a
    /// single *trailing* comment floated past a deferred `&anchor`/`!tag`
    /// (#784); this one carries whole comment-only lines.
    ///
    /// Lives on `Parser` rather than as a local threaded through the parse
    /// functions on purpose: `Parser` is one struct in `build_semi_index`'s
    /// frame, so a field here costs nothing per recursion level, whereas a
    /// local in a recursive function is charged at every level and has twice
    /// inverted a depth guard into a real stack overflow on this issue
    /// (#798 PR1's `parse_assignment`, PR2's `NodeMeta`).
    pending_head_lines: Vec<(u32, u32)>,
    /// The most recently opened node that can own a head/foot comment: a
    /// mapping *key* or a sequence item's *content* (#798). This is `PREV`
    /// in [`Self::record_standalone_comment`]'s attachment rule.
    ///
    /// Deliberately not [`Self::last_open_bp_pos`], which is the last bp of
    /// *any* kind: for an inline `k: v` that is the value node, while real
    /// yq measurably attaches head/foot to the key
    /// (`.b | key | foot_comment`, never `.b | foot_comment`).
    ///
    /// A blank line between it and a block no longer clears it (#2811):
    /// [`PendingBlock::blank_before`] records that instead, and
    /// [`Self::attached_prev`] is the `PREV` most placements read.
    last_head_foot_bp: Option<usize>,
    /// High-water mark of recorded standalone-comment text (#798).
    ///
    /// Three `skip_newlines` call sites rewind after a speculative lookahead
    /// (`following_value_is_null`, `parse_mapping_entry`'s anchored branch,
    /// `parse_explicit_key`), so the same comment bytes pass through the
    /// recorder more than once. Recording only ranges starting at or past
    /// this mark makes the recorder idempotent without any call site needing
    /// to know it is on a rewinding path.
    comment_watermark: u32,
    /// Whether any node that can own a head/foot comment has opened yet
    /// (#798). Distinguishes a block trailing real content (the document
    /// root's *foot*) from a comment-only document (its *head*).
    saw_head_foot_node: bool,
    /// Whether [`Self::start_document`] has run at least once (#798) --
    /// distinguishes "no previous document" (very first `start_document`
    /// call) from "previous document's root is bp 0", which is otherwise
    /// indistinguishable from the field's zero-initialized state.
    seen_any_document: bool,
    /// The root bp of whichever document [`Self::last_head_foot_bp`]'s `None`
    /// currently means "nothing has opened in the *new* document yet",
    /// rather than "a blank line detached the in-document `PREV`" (#798).
    ///
    /// Set at every [`Self::start_document`] to the document that just
    /// closed (or `None` for the very first document), and cleared the
    /// moment any node opens in the new document -- from then on an
    /// in-document blank-line detachment must fall forward as usual, not
    /// reach back across the boundary a second time. Consulted only by
    /// [`Self::flush_pending_head_lines`]'s no-`PREV` fallback.
    document_boundary_fallback_bp: Option<usize>,
    /// One entry per open block sequence, innermost last (#1079). Not
    /// popped where the frame closes -- several paths pop `indent_stack`
    /// -- but trimmed lazily against `indent_stack.len()` wherever it is
    /// read, which is only when a floated comment is being placed.
    seq_frames: Vec<SeqFrame>,
    /// Source of [`SeqFrame::serial`]; never reused, so a sequence that
    /// closed and had another opened at the same depth is told apart from
    /// one that is still open.
    next_seq_serial: u32,
    /// The most recent comment floated off a bare `- # c` item whose value
    /// turned out absent (#1079), still sitting in
    /// [`Self::pending_head_lines`] with the standalone lines. Real yq has
    /// no node for such a comment and moves it: forward onto the next item
    /// of the same sequence as its `head` (accumulating), or -- when the
    /// sequence closes past a dedent first -- the *last* one becomes a
    /// `foot` (see [`Self::settle_float_if_sequence_closed`] for whose)
    /// while the rest go forward, or at end of document all of them become
    /// the root's `foot`. The first of those is what the ordinary forward
    /// attach already does; this records what
    /// [`Self::flush_pending_head_lines`] needs for the other two.
    floated_item_comment: Option<FloatedItemComment>,
    /// Whether an absent bare `-` item, with or without a comment of its
    /// own, has been seen since the last node that owns head/foot comments
    /// opened (#1079). At end of document the pending block is then the
    /// root's foot in real yq, not `PREV`'s (`- 1\n# c\n-\n` and
    /// `- 1\n-\n# c\n` alike), whether or not a floated comment is still
    /// in it.
    block_passed_absent_item: bool,
    /// Column/indent context of the block in [`Self::pending_head_lines`]
    /// (#2811). See [`PendingBlock`].
    pending_block: PendingBlock,
    /// The bp of the most recent key opened at each `indent_stack` depth
    /// (#2811), indexed by depth; `usize::MAX` while the mapping at that
    /// depth has no key yet. Only ever read while a standalone block is
    /// being placed, but written for every key, since a standalone comment
    /// can only be recognised after the keys it might need are already
    /// open. Every mapping open resets its slot
    /// ([`Self::open_frame_key_slot`]), so a closed mapping's key is never
    /// read as a later one's at the same depth.
    frame_key_bp: Vec<usize>,
    /// A block already settled as a *foot* whose owner is the first key or
    /// scalar of the sequence item opening now (#2811): the item's own
    /// open hook runs at the container level (`- k: v`, `- - x`) before
    /// that node exists. Claimed by the next [`Self::attach_head_foot_at`]
    /// / [`Self::attach_head_foot_to_key`]; anything still here at end of
    /// input (the item turned out empty: `- -`) is the document root's
    /// foot, as in real yq.
    deferred_foot_lines: Vec<(u32, u32)>,
    /// The first node in the whole stream that can own a head/foot comment
    /// (#2811), for [`Self::first_node_takes_no_foot_quirk`].
    first_head_foot_bp: Option<usize>,
    /// Whether that node (or its inline value -- `k: v # c` keeps the
    /// comment on the value's bp) carries a trailing comment (#2811).
    first_node_has_line_comment: bool,
    /// The open index (into `bp_to_text`) of that node's own value: the
    /// node itself for a sequence item or document root, the inline value
    /// for a mapping key (#2811).
    first_node_value_idx: usize,
    /// Where that node's hook ran: the start of its own text, or of its
    /// line (#2811). Only the bytes between here and the value's start are
    /// ever scanned, and only by the quirk check itself -- scanning to the
    /// end of the line at hook time cost 4 instructions per byte of a
    /// single-line JSON document, +10% on the perf guard's `yq` row.
    first_node_pos: usize,

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

    /// Enforce JSON's stricter grammar for `-p json` input (#2279, #2778).
    ///
    /// Set only for input the caller already knows is JSON, i.e. exactly the
    /// callers that follow `YamlIndex::build` with
    /// [`mark_json_sourced`](crate::yaml::YamlIndex::mark_json_sourced).
    /// JSON is a subset of YAML's flow grammar, so the YAML parser accepts
    /// it as-is -- but it also accepts things real yq's own JSON front end
    /// rejects: `[1,]`/`[,1]`/`[1,,2]` (delimiters, #2279) and, at every
    /// scalar *value* position (flow-sequence item, flow-mapping value,
    /// block/top-level value), YAML-only spellings like `True`, `'a'`,
    /// `.5`, `+1`, `~`, or an implicit-pair inside a sequence (`[a: 1]`,
    /// #2778). `DocumentCursor::preceding_delimiter_ok`'s doc comment names
    /// the invariant this restores: *every format but JSON validates
    /// delimiters while parsing*. JSON-sourced YAML was the one case that
    /// did neither -- a YAML parse that permits what JSON forbids, feeding
    /// cursors whose delimiter checks all default to `true`.
    ///
    /// Deliberately a runtime flag rather than a third const generic: it
    /// would multiply the `HAS_CR` monomorphizations (#340) for a branch
    /// that is predictable and off the scalar-scanning hot path.
    ///
    /// **Delimiters: sequences only, never mappings.** Real yq's own object
    /// handling is *lenient* here -- it ignores punctuation inside `{}`
    /// entirely and pairs up tokens (`{"a":1,}`, `{,}`, `{"a" 1}` all
    /// parse) -- so extending the delimiter checks to flow mappings would
    /// refuse input the reference accepts.
    ///
    /// **Scalar grammar: values only, never keys.** Mapping *keys* are
    /// exempt (real yq panics on a non-string JSON key -- `{1:1}`,
    /// `{true:1}` -- which succinctly does not reproduce; see
    /// `docs/compliance/yq/limitations.md`). `json_strict_plain_scalar_ok`
    /// is the predicate; yq accepts `[01]`/`[00]`/`[1.]` (leading zeros, a
    /// bare trailing `.` survive) while rejecting `.5`/`+1`/`1e` -- not
    /// "strict JSON", the boundary is
    /// `goccy/go-json`'s own token scanner plus `strconv.ParseFloat`.
    ///
    /// **Structural, key-agnostic rejections.** `?` (explicit key), `&`/`!`
    /// (anchor/tag), `*` (alias), and `'` (single quote) have no JSON
    /// spelling at all regardless of what follows them, so `parse_block_node`
    /// rejects the byte itself at one chokepoint rather than each dispatch
    /// arm needing its own check -- an anchor/tag-prefixed value is refused
    /// before the prefix's own target is even examined, not merely left
    /// unvalidated. Two shapes bypass that chokepoint's own dispatch and
    /// carry their own identical gate instead: `--- |`/`--- >`
    /// (`parse_inline_document_value`'s dedicated fast path) and `?` inside
    /// a flow mapping (`parse_flow_mapping_inner`).
    json_strict: bool,
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
            bp_to_anchor: BTreeMap::new(),
            aliases: BTreeMap::new(),
            tags: BTreeMap::new(),
            pending_property_bp: None,
            comments: BTreeMap::new(),
            pending_head_comment: None,
            pending_head_lines: Vec::new(),
            last_head_foot_bp: None,
            comment_watermark: 0,
            saw_head_foot_node: false,
            seen_any_document: false,
            document_boundary_fallback_bp: None,
            seq_frames: Vec::new(),
            next_seq_serial: 0,
            floated_item_comment: None,
            block_passed_absent_item: false,
            pending_block: PendingBlock::default(),
            frame_key_bp: Vec::new(),
            deferred_foot_lines: Vec::new(),
            first_head_foot_bp: None,
            first_node_has_line_comment: false,
            first_node_value_idx: 0,
            first_node_pos: 0,
            in_document: false,
            document_start_bp_pos: 0,
            pending_explicit_key: None,
            nesting_depth: 0,
            json_strict: false,
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

    /// Push `range` onto `owner_bp_pos`'s `line` comments, unless that exact
    /// range is already present.
    ///
    /// Shared by [`Self::maybe_capture_line_comment`] and
    /// [`Self::take_pending_head_comment`], the two writers of this slot.
    /// The dedup check is what `entry().or_insert()` used to provide back
    /// when `line` was a single `Option` slot: a node captured explicitly at
    /// a more specific point (e.g. a block scalar's header-line comment,
    /// captured before its content is parsed) must never see a duplicate
    /// entry from a later, spurious call for the same bp_pos with the same
    /// range. It does *not* prevent a second, genuinely different range from
    /// being appended -- that's exactly the #1085 shape this slot exists to
    /// hold (a floated anchor comment and the node's own comment, in that
    /// order, at the two mapping-key call sites below).
    #[inline]
    fn push_line_comment(&mut self, owner_bp_pos: usize, range: (u32, u32)) {
        // While `PREV` is still the stream's first head/foot node, any
        // trailing comment belongs to that node or its inline value --
        // nothing else has opened yet (#2811, see
        // `first_node_takes_no_foot_quirk`).
        if self.last_head_foot_bp.is_some() && self.last_head_foot_bp == self.first_head_foot_bp {
            self.first_node_has_line_comment = true;
        }
        let line = &mut self.comments.entry(owner_bp_pos).or_default().line;
        if !line.contains(&range) {
            line.push(range);
        }
    }

    /// Capture a trailing same-line comment for `owner_bp_pos`. Callers
    /// still run their own `skip_to_eol`/`skip_newlines` afterward to
    /// actually advance past it.
    #[inline]
    fn maybe_capture_line_comment(&mut self, owner_bp_pos: usize) {
        if let Some(range) = self.scan_trailing_comment() {
            self.push_line_comment(owner_bp_pos, range);
        }
    }

    /// Record a *standalone* comment line — one whose line holds nothing but
    /// whitespace before the `#` — for later attachment as some node's
    /// `head` or `foot` (#798).
    ///
    /// **Record-only.** Never moves `self.pos` and never changes a branch:
    /// the caller's `skip_to_eol` still does the consuming. That is the whole
    /// safety argument for touching `skip_newlines` at all — nothing reads
    /// `head`/`foot` yet, so a mis-*attributed* comment cannot change any
    /// output, but a disturbed parser position would be a real regression.
    ///
    /// Two guards make this safe at every call site rather than only at the
    /// ones known to be well-behaved:
    ///
    /// - [`Self::comment_watermark`] makes it idempotent under the three
    ///   rewinding lookahead sites.
    /// - the standalone check below rejects a *trailing* comment reached
    ///   mid-line, which happens whenever `skip_newlines` is entered after a
    ///   document marker (`--- # c` leaves the cursor past the marker, and
    ///   the space arm then walks straight onto the `#`).
    ///
    /// Attachment follows the rule measured against pinned yq v4.53.3 (and
    /// read off the scanner in mikefarah's go-yaml fork yq embeds, whose
    /// `yaml_parser_scan_comments` splits a comment run into foot/head
    /// pieces and `yaml_parser_unroll_indent` repositions a block's
    /// BLOCK-END token before a comment sitting at that block's own column
    /// — confirmed live against `scannerc.go` on the `v3` branch of
    /// `github.com/mikefarah/yaml`, #2811). A block of consecutive
    /// comment lines sticks to whatever it is *not* separated from by a
    /// blank line, preferring forward -- except that a block *dedented*
    /// below the innermost open block (`col < prev_indent`, with `PREV` in
    /// that block) is placed by its column instead:
    ///
    /// ```text
    /// adjacent to NEXT (no blank line between; a blank line *before* changes nothing here):
    ///   PREV && NEXT dedents out of PREV's block && col != NEXT.col -> PREV.foot
    ///   else                                                        -> NEXT.head
    /// blank line after, or end of document:
    ///   !PREV, or a blank line before the block  -> NEXT.head, else ROOT.foot
    ///   col >= prev_indent                      -> PREV.foot
    ///   else, among the blocks NEXT closes (all of them at end of document):
    ///     the block at column `col`, then outward, first mapping -> its last key's foot
    ///     none: NEXT is a key       -> the previous key of NEXT's mapping, its foot
    ///           NEXT is a `-` item  -> the foot of the item's first key/scalar
    ///           end of document     -> ROOT.foot
    /// ```
    ///
    /// `a:` / `  b:` / `    - 1` / `# c` is `.a | key | foot_comment`, not
    /// `.a.b[0]`'s; the same comment two columns in is `.a.b`'s key, four
    /// in is the item's (all measured). A run whose later line moves to a
    /// different column below `prev_indent` is two blocks, the first of
    /// which is `PREV`'s foot.
    ///
    /// Only `blank_before` and the block's own column context are decidable
    /// here (the lines *after* the block have not been scanned yet), so this
    /// records into [`Self::pending_head_lines`] / [`Self::pending_block`]
    /// and leaves the whole decision to [`Self::flush_pending_head_lines`],
    /// which runs once the next node opens (or at EOF) and can see both
    /// sides.
    fn record_standalone_comment(&mut self, start: usize, end: usize) {
        let (start, end) = (start as u32, end as u32);
        if start < self.comment_watermark {
            return;
        }
        let Some(column) = self.standalone_column_before(start as usize) else {
            return;
        };
        self.comment_watermark = end;
        // A blank line between the pending block and this line ends that
        // block: its `blank_after` is now known to be true, so settle it
        // before starting a new one. Without this, `a: 1 / # m1 / <blank> /
        // # m2 / b: 2` would merge both blocks onto `b`, where real yq
        // splits them `m1` -> `.a`'s foot, `m2` -> `.b`'s head.
        if let Some(&(_, last_end)) = self.pending_head_lines.last() {
            if self.blank_line_between(last_end as usize, start as usize) {
                self.resolve_pending_block_backward();
            }
        }
        // So does a line at a different column below the innermost open
        // block (#2811): go-yaml ends the run there and gives what it has
        // to the prior node -- `    - 1` / `  # c` / ` # d` / `z: 2` is
        // `c` on the item, `d` on `.a`'s key, not `c\nd` on either. A line
        // that only goes *deeper* still joins the block.
        // Unlike the blank-line split, this one settles on `PREV` even
        // across a blank line above the run (`    c: 1` / blank / `    # c2`
        // / `# c3` still gives `c` the `c2`, measured); the segment it
        // starts inherits the run's own blank-line detachment.
        let mut after_split = false;
        if !self.pending_head_lines.is_empty()
            && column != self.pending_block.column
            && column < self.pending_block.prev_indent
        {
            if let Some(prev) = self.last_head_foot_bp {
                self.drain_pending_into_foot(prev);
                after_split = true;
            }
        }
        if self.pending_head_lines.is_empty() {
            let blank_before = if after_split {
                self.pending_block.blank_before
            } else {
                self.blank_line_precedes(start as usize)
            };
            self.capture_pending_block(column, after_split, blank_before);
        }
        self.pending_head_lines.push((start, end));
    }

    /// Record where the block starting now sits relative to the open blocks
    /// (#2811): see [`PendingBlock`]. The frame snapshot used to be taken
    /// only for a genuinely dedented block (`column < prev_indent`), the one
    /// case [`Self::flush_pending_head_lines`] consulted it in -- widened to
    /// `column <= prev_indent` (#2811 review) so
    /// [`Self::positional_boundary_foot_target`] also has frames to search
    /// when a block sits flush against the innermost open block's own
    /// column at a document boundary (`a:\n  b: 1\n  # c\n---\nc: 2\n`,
    /// where the comment never dedents below `b`'s own indent). Every
    /// existing consumer of `pending_block.frames`
    /// ([`Self::positional_foot_target`], called only from
    /// [`Self::settle_dedented_block`]) is reached only from call sites that
    /// independently re-check `column < prev_indent` before calling, so
    /// this widening changes nothing for them.
    fn capture_pending_block(&mut self, column: usize, after_split: bool, blank_before: bool) {
        let prev_indent = self.innermost_block_indent();
        self.pending_block.column = column;
        self.pending_block.prev_indent = prev_indent;
        self.pending_block.after_split = after_split;
        self.pending_block.blank_before = blank_before;
        self.pending_block.frames.clear();
        if column > prev_indent {
            return;
        }
        // `indent_stack[0]` is the virtual root (sentinel indent), never a
        // block; a `SequenceItem` frame is an item's virtual `indent + 1`,
        // not a block either.
        for depth in 1..self.indent_stack.len() {
            let is_mapping = match self.type_stack.get(depth) {
                Some(NodeType::Mapping) => true,
                Some(NodeType::Sequence) => false,
                _ => continue,
            };
            self.pending_block.frames.push(CommentFrame {
                indent: self.indent_stack[depth],
                is_mapping,
                last_key_bp: is_mapping
                    .then(|| self.frame_key_bp.get(depth).copied())
                    .flatten(),
            });
        }
    }

    /// The indent of the innermost open block -- go-yaml's `parser.indent`
    /// (#2811). A `SequenceItem` frame on top is its sequence's indent plus
    /// one (see `parse_sequence_item_inner`), so it reads as the sequence's;
    /// the virtual root's sentinel reads as 0.
    fn innermost_block_indent(&self) -> usize {
        let top = self.indent_stack.len() - 1;
        let indent = self.indent_stack[top];
        if top == 0 {
            return 0;
        }
        if self.type_stack.get(top) == Some(&NodeType::SequenceItem) {
            indent - 1
        } else {
            indent
        }
    }

    /// Attach the pending standalone-comment block (#798), applying the
    /// measured rule in [`Self::record_standalone_comment`]'s doc comment.
    ///
    /// `next` is the node that just opened and would own the block as its
    /// `head`, with the column its line starts at (a key's own column, a
    /// sequence item's `-` column), or `None` at end of input. Called at
    /// every point a node that can own a head comment opens — a mapping key,
    /// a sequence item's content, and a document's content node — plus once
    /// at end of parse. A no-op when nothing is pending, which is the
    /// overwhelmingly common case, so the hooks cost one `Vec::is_empty`
    /// each.
    ///
    /// #1079 state (a comment floated off an absent bare item, or the plain
    /// "an absent item was seen" flag) is settled first, since real yq's
    /// placement for such a block differs from the rule
    /// [`Self::flush_pending_head_lines_with_root`] applies in two ways
    /// handled here. The third -- forward onto the next item of the same
    /// sequence, accumulating -- is the ordinary `NEXT.head` path and falls
    /// through to it.
    fn flush_pending_head_lines(&mut self, next: Option<(usize, usize)>) {
        if self.pending_head_lines.is_empty() {
            return;
        }
        let floated = self.floated_item_comment.take();
        let passed_absent_item = core::mem::take(&mut self.block_passed_absent_item);
        let root = self.document_start_bp_pos;
        let Some((bp, column)) = next else {
            if floated.is_some() || passed_absent_item {
                // End of document: everything still pending is the root's
                // foot, whatever `PREV` is -- `a:\n  - # c\n` answers
                // `. | foot_comment`, not `.a | key | foot_comment`; `- # c\n`
                // alone has no `PREV` at all yet is a foot, not a
                // comment-only document's head; and `- 1\n# c\n-\n` is the
                // root's, not `.[0]`'s (all measured).
                self.drain_pending_into_foot(root);
                return;
            }
            return self.flush_pending_head_lines_with_root(None, root);
        };
        if let Some(floated) = floated {
            // The sequence may have closed before this node opened: then
            // the last floated comment becomes a foot -- of this node when
            // it is an item of an outer sequence (`-\n  - # c\n- 2\n` is
            // `.[1]`'s foot, measured), otherwise of the nearest enclosing
            // key (`a:\n  - # c\n  - # d\nb: 2\n` gives `.a`'s key `d` and
            // `.b`'s key `c`) -- and whatever else is pending goes forward
            // as usual.
            let is_item = self.current_type == Some(NodeType::SequenceItem);
            self.settle_float_if_sequence_closed(floated, Some((bp, column, is_item)));
            if self.pending_head_lines.is_empty() {
                return;
            }
        }
        self.flush_pending_head_lines_with_root(Some((bp, column)), root);
    }

    /// [`Self::flush_pending_head_lines`]'s #798/#2811 rule proper, run
    /// after its #1079 floated-comment pre-check -- with the document root
    /// the end-of-document fallbacks use passed explicitly, since at a
    /// `---`/`...` boundary that is the *closing* document's root, which
    /// `document_start_bp_pos` no longer names by the time the flush runs
    /// (#2811).
    fn flush_pending_head_lines_with_root(&mut self, next: Option<(usize, usize)>, root: usize) {
        if self.pending_head_lines.is_empty() {
            return;
        }
        let column = self.pending_block.column;
        let prev_indent = self.pending_block.prev_indent;
        // A blank line between the block and whatever follows detaches it
        // forward; so does having nothing follow at all. Either way it falls
        // back onto `PREV` — which `record_standalone_comment` has already
        // set to `None` if a blank line detached the block backwards too.
        let block_end = self.pending_head_lines[self.pending_head_lines.len() - 1].1 as usize;
        let detached_forward = next.is_none() || self.blank_line_between(block_end, self.pos);
        if detached_forward {
            if self.pending_block.after_split && next.is_some() {
                // A segment split off an earlier one is keyed to its own
                // position, so the node closing or opening next places it
                // whether or not a blank line detached the run from `PREV`
                // (`    # c2` / `  # c3` / blank / `z: 1` is `.a.b`'s key's
                // foot either way, measured); at end of input a detached
                // run's segment is the document's instead, below.
                self.settle_dedented_block(next, root);
                return;
            }
            if let Some(prev) = self.attached_prev() {
                // Dedented below the block `PREV` sits in, the comment is
                // placed by its column rather than by adjacency (#2811):
                // `a:` / `  b:` / `    - 1` / `# c` is `.a`'s key's foot,
                // not the item's.
                if column < prev_indent {
                    self.settle_dedented_block(next, root);
                    return;
                }
                if !self.first_node_takes_no_foot_quirk(prev, prev_indent, next.is_some()) {
                    self.drain_pending_into_foot(prev);
                    return;
                }
                // Falls through to the head/root arms below.
            } else if let Some(old_root) = self.document_boundary_fallback_bp {
                // No `PREV` in *this* document -- if nothing has opened here
                // yet because we just crossed a `---`/`...` boundary, the
                // block reaches back across it onto the document that just
                // closed, rather than forward into this one (measured against
                // pinned yq v4.53.3: `a: 1\n---\n# mid\n\nb: 2\n` puts `mid`
                // on the first document's own foot, not `.b`'s head, #798).
                self.drain_pending_into_foot(old_root);
                return;
            }
        } else if let (Some((_, next_column)), Some(prev)) = (next, self.last_head_foot_bp) {
            // Adjacent to a node that dedents out of `PREV`'s block, the
            // comment is `PREV`'s foot unless it sits at that node's own
            // column (#2811): `    - 1` / `  # c` / `d: 2` is the item's
            // foot, `# c` there is `.d`'s key's head (both measured) -- a
            // blank line above the block changes nothing here, which is
            // why `PREV` is read directly rather than via `attached_prev`.
            // A block that a column change split off an earlier one has
            // already given `PREV` that earlier block, and is placed by
            // column like a detached one (`  # c` / ` # d` / `z: 2` puts
            // `d` on `.a`'s key, measured).
            if next_column < prev_indent && column != next_column {
                if self.pending_block.after_split && column < prev_indent {
                    self.settle_dedented_block(next, root);
                } else {
                    self.drain_pending_into_foot(prev);
                }
                return;
            }
        }
        let pending = &mut self.pending_head_lines;
        match next {
            Some((bp, _)) => self.comments.entry(bp).or_default().head.append(pending),
            // Nothing follows and nothing precedes it closely enough, so the
            // block belongs to the document root -- as its *foot* normally,
            // but as its *head* when the document never opened a node at all
            // (a comment-only document, where real yq answers
            // `. | head_comment`, not `foot_comment`).
            None if self.saw_head_foot_node => self.drain_pending_into_foot(root),
            None => {
                let pending = &mut self.pending_head_lines;
                self.comments.entry(root).or_default().head.append(pending);
            }
        }
    }

    /// Place a dedented block (`column < prev_indent`) whose adjacency no
    /// longer decides its owner (#2811): it is the foot of the node
    /// [`Self::positional_foot_target`] names, of the first key/scalar of
    /// the sequence item opening now, or -- with every open block closing
    /// and none of them a mapping -- of the document root (`- x` / `- b:` /
    /// `    - 1` / `   # c` is `. | foot_comment`, measured).
    fn settle_dedented_block(&mut self, next: Option<(usize, usize)>, root: usize) {
        if let Some(target) = self.positional_foot_target(next) {
            self.drain_pending_into_foot(target);
        } else if next.is_some() {
            // The owner is the first key/scalar of the item opening now
            // (`- a:` / `    - 1` / ` # c` / blank / `- k: v` is `.[1].k`'s
            // key's foot, measured), a node the item-level hook runs ahead
            // of.
            let pending = &mut self.pending_head_lines;
            self.deferred_foot_lines.append(pending);
        } else {
            self.drain_pending_into_foot(root);
        }
    }

    /// Where a dedented block lands, or `None` when the owner is the first
    /// key/scalar of the sequence item opening now, or the document root at
    /// end of input (#2811). `next` is the node opening now with its column,
    /// or `None` at end of document, where every open block closes.
    ///
    /// Mirrors go-yaml: `unrollIndent` moves the end token of a closing
    /// block to just before a comment sitting at that block's own column,
    /// so the comment becomes that block's foot -- which a mapping hands to
    /// its last key, and a block sequence hands on to the next enclosing
    /// end (`x:` / `  a:` / `    - k: 1` / `    # c` is `.x.a`'s key's
    /// foot). At end of document the outermost block's end lands past the
    /// comment whatever its column, so the outermost block claims it when
    /// no exact match does. When no closing mapping claims it, the comment
    /// reaches the next key's own mapping, whose composer gives it to the
    /// key before that one (`a:` / `  b:` / `    - 1` / ` # c` / blank /
    /// `  e: 3` is `.a.b`'s key's foot) -- or to the key itself when it is
    /// the mapping's first (`- a:` / `    - 1` / ` # c` / blank / `-` /
    /// `  k: v` is `.[1].k`'s).
    fn positional_foot_target(&self, next: Option<(usize, usize)>) -> Option<usize> {
        let frames = &self.pending_block.frames;
        let column = self.pending_block.column;
        let next_column = next.map(|(_, c)| c);
        // `next_column.is_none_or(..)` is stable only since 1.82; the MSRV
        // is 1.73.
        let closing = |frame: &CommentFrame| next_column.map_or(true, |c| frame.indent > c);
        let start = frames
            .iter()
            .rposition(|frame| frame.indent == column && closing(frame))
            .or_else(|| next_column.is_none().then_some(0));
        if let Some(start) = start {
            let claimed = frames[..=start]
                .iter()
                .rev()
                .take_while(|frame| closing(frame))
                .find(|frame| frame.is_mapping)
                .and_then(|frame| frame.last_key_bp)
                // A mapping frame with no key yet records the
                // `open_frame_key_slot` sentinel rather than `None` (its
                // `last_key_bp` is only ever cleared to `None` for a
                // non-mapping frame); filter it out here the same way the
                // fallback arm below already does, so a mapping that opened
                // without ever yet completing a key doesn't hand a dedented
                // comment to a bp no cursor resolves to (#2811 review).
                .filter(|&bp| bp != usize::MAX);
            if claimed.is_some() {
                return claimed;
            }
        }
        let (next_bp, _) = next?;
        if self.current_type == Some(NodeType::SequenceItem) {
            return None;
        }
        // The key opening now belongs to the top frame; its slot still
        // holds the key before it, or the fresh-slot sentinel when this is
        // the mapping's first.
        let depth = self.indent_stack.len() - 1;
        match self.frame_key_bp.get(depth) {
            Some(&bp) if bp != usize::MAX => Some(bp),
            _ => Some(next_bp),
        }
    }

    /// Where a block sitting flush against a `---`/`...` marker (no blank
    /// line in between) lands, when it isn't handled by the #1079
    /// floated-comment/absent-item pre-check
    /// [`Self::flush_pending_head_lines_at_boundary`] already runs first
    /// (#2811 review). By the time this runs, `end_document` has already
    /// unwound `self.indent_stack` for the *closing* document, so -- like
    /// [`Self::positional_foot_target`] -- this reads the frame snapshot
    /// [`Self::capture_pending_block`] took while those containers were
    /// still open, not live parser state.
    ///
    /// Unlike an in-document dedent, real yq never lets a document boundary
    /// hand the block to the *outermost* container's own last key -- that
    /// specific shape is #798's older, still-correct "closing document's
    /// root foot" rule (`a: 1\n# mid\n---\nb: 2\n` stays document 0's root
    /// foot, not `.a`'s key's) -- but a container genuinely nested inside
    /// it still claims the block by column, exactly as
    /// [`Self::positional_foot_target`] already does in-document/at EOF
    /// (`a:\n  b: 1\n  # c\n---\nc: 2\n` is `.a.b`'s key foot, measured).
    /// `frames[0]` is always that outermost container, so a match there, or
    /// no match at all, both return `None` (the caller's cue to fall back
    /// to the closing document's root as before).
    ///
    /// A matching frame that is a sequence rather than a mapping (nothing
    /// in `frames` ever names a sequence *item*, only a mapping's last
    /// *key*) falls back to `PREV` instead, which already names that exact
    /// item when the block never left the sequence's own column
    /// (`a:\n  - 1\n  # c\n---\nb: 2\n` is `.a[0]`'s foot, measured) --
    /// coincides with the same frame's `last_key_bp` for a mapping match
    /// too, so this fallback is safe to apply uniformly rather than only
    /// for the sequence case.
    fn positional_boundary_foot_target(&self) -> Option<usize> {
        let frames = &self.pending_block.frames;
        let column = self.pending_block.column;
        let start = frames.iter().rposition(|frame| frame.indent == column)?;
        if start == 0 {
            return None;
        }
        frames[1..=start]
            .iter()
            .rev()
            .find(|frame| frame.is_mapping)
            .and_then(|frame| frame.last_key_bp)
            .filter(|&bp| bp != usize::MAX)
            .or_else(|| self.attached_prev())
    }

    /// Real yq never makes a detached block the foot of the stream's very
    /// first top-level node when that node's line ends in a trailing
    /// comment, a quoted scalar or a flow collection (#2811): `- 1 # c1` /
    /// `# c3` and `- "q"` / `# f` are `. | foot_comment`, and with `- 2`
    /// after a blank line the block is `.[1]`'s head -- while `- 1` /
    /// `- 2 # c1` / `# c3`, or the same first line in a second document, is
    /// the node's foot as usual, and a first node nested in a top-level
    /// item (`- k: v # c1` / `  # c3`) is too. Measured, not explained:
    /// go-yaml's comment scanner special-cases the first fetch of a stream.
    ///
    /// Gated on `prev` being [`Self::first_head_foot_bp`] -- "the stream's
    /// first head/foot-bearing node" -- not on that node's own column
    /// (#2811 review). An earlier `prev_indent != 0` guard narrowed this to
    /// a *top-level* first node only, contradicting this doc comment's own
    /// stated `- k: v # c1` / `  # c3` case: that shape's first head/foot
    /// node is `.[0].k`'s key, at column 2 (nested under the top-level `-`
    /// item), not column 0, so the guard wrongly excluded exactly the
    /// nested-first-node shape the comment already described -- confirmed
    /// live: `- k: v # c1\n  # c3\n` (nothing else follows) is
    /// `. | foot_comment`, not `.[0].k | key | foot_comment`, in real yq.
    ///
    /// `next_is_some` keeps one part of the old guard for a nested first
    /// node specifically (measured, not derived, like the rest of this
    /// function): a *top-level* first node's block still forwards onto a
    /// following node's head across a blank line as before
    /// (`- 1 # c1\n# c3\n\n- 2\n` is `.[1] | head_comment`), but a *nested*
    /// one does not -- it stays this node's own foot instead
    /// (`- k: v # c1\n  # c3\n\n- 2\n` keeps `.[0].k`'s key foot `c3`,
    /// `.[1]`'s head empty, both confirmed live). Within this function's
    /// sole call site (guarded by `detached_forward`), `next.is_some()`
    /// only holds when a blank line -- not true end of input -- is why the
    /// block detached forward, so this only distinguishes those two cases,
    /// not "was there a next node" in general.
    fn first_node_takes_no_foot_quirk(
        &self,
        prev: usize,
        prev_indent: usize,
        next_is_some: bool,
    ) -> bool {
        if self.first_head_foot_bp != Some(prev) {
            return false;
        }
        if prev_indent != 0 && next_is_some {
            return false;
        }
        if self.first_node_has_line_comment {
            return true;
        }
        // A quoted scalar or flow collection on that first line has the
        // same effect as a trailing comment (`- "q"` / `# f` and `k: [1]` /
        // `# f` are both `. | foot_comment`); a plain, anchored, tagged or
        // block scalar, a quoted *key*, or a value deferred to the next
        // line does not.
        let Some(&value_pos) = self.bp_to_text.get(self.first_node_value_idx) else {
            return false;
        };
        let value_pos = value_pos as usize;
        let same_line = value_pos >= self.first_node_pos
            && !self.input[self.first_node_pos..value_pos]
                .iter()
                .any(|&b| is_line_break(b));
        same_line && matches!(self.input[value_pos], b'"' | b'\'' | b'[' | b'{')
    }

    /// `PREV` for the pending block, unless a blank line detached the block
    /// from it (#798) -- the one placement that ignores that blank line
    /// reads [`Self::last_head_foot_bp`] directly.
    fn attached_prev(&self) -> Option<usize> {
        if self.pending_block.blank_before {
            None
        } else {
            self.last_head_foot_bp
        }
    }

    /// Move the whole pending block onto `bp`'s `foot`. The one place a
    /// pending block leaves for a foot, so it is also where a floated item
    /// comment (#1079) that was part of that block stops being pending --
    /// left set, it would claim some later, unrelated block at end of
    /// document.
    fn drain_pending_into_foot(&mut self, bp: usize) {
        self.floated_item_comment = None;
        self.block_passed_absent_item = false;
        let pending = &mut self.pending_head_lines;
        self.comments.entry(bp).or_default().foot.append(pending);
    }

    /// Whether the block sequence [`SeqFrame::serial`] names is still on the
    /// stack (#1079). Entries whose frame has been popped are dropped here;
    /// a sequence that closed and had another opened at the same depth has
    /// a different serial, so it correctly reads as closed.
    fn sequence_still_open(&mut self, serial: u32) -> bool {
        self.trim_seq_frames_to(self.indent_stack.len());
        self.seq_frames.iter().any(|frame| frame.serial == serial)
    }

    /// Drop [`Self::seq_frames`] entries deeper than `max_frame_len`: their
    /// frames were popped by one of the paths that don't touch this stack.
    fn trim_seq_frames_to(&mut self, max_frame_len: usize) {
        while self
            .seq_frames
            .last()
            .is_some_and(|frame| frame.frame_len > max_frame_len)
        {
            self.seq_frames.pop();
        }
    }

    /// The entry for the sequence just below the item frame on top of the
    /// stack, if that sequence was registered (#1079).
    fn enclosing_seq_frame(&mut self) -> Option<SeqFrame> {
        let seq_frame_len = self.indent_stack.len() - 1;
        self.trim_seq_frames_to(seq_frame_len);
        self.seq_frames
            .last()
            .copied()
            .filter(|frame| frame.frame_len == seq_frame_len)
    }

    /// A floated comment whose sequence has since closed becomes a foot
    /// (#1079), leaving the rest of the pending block to go forward. `next`
    /// is the node opening now, as (bp, column, is a sequence item), or
    /// `None` when the trigger is another absent item.
    ///
    /// Real yq's placement, measured: a node at or past the sequence's own
    /// column is not a dedent out of it, and the comment goes forward as
    /// that node's head like any other (`a:\n- # c\nb: 2\n` heads `b`);
    /// past a real dedent, the comment is the foot of the nearest enclosing
    /// key when the sequence was that key's own value (`a:\n  - # c\nb: 2\n`,
    /// and `k:\n- # c\n-\n` closing to an outer item alike), else -- the
    /// sequence nested in an item -- of the outer item opening now
    /// (`-\n  - # c\n- 2\n` is `.[1]`'s foot), or of the inherited key when
    /// a key opens instead. A no-op while the sequence is still open, or
    /// with no target at all (a root-level sequence nested in a sequence
    /// closing to nothing: real yq drops the comment outright, and keeping
    /// it pending, to go forward, is the ADR-0018 rule 4 answer to that).
    fn settle_float_if_sequence_closed(
        &mut self,
        floated: FloatedItemComment,
        next: Option<(usize, usize, bool)>,
    ) {
        if self.sequence_still_open(floated.seq_serial) {
            return;
        }
        let target = match next {
            Some((_, column, _)) if column >= floated.seq_indent => return,
            Some((bp, _, true)) if !floated.direct_key_value => Some(bp),
            _ => floated.owner_key_bp,
        };
        let Some(target) = target else {
            return;
        };
        if let Some(idx) = self
            .pending_head_lines
            .iter()
            .position(|&range| range == floated.range)
        {
            let range = self.pending_head_lines.remove(idx);
            self.comments.entry(target).or_default().foot.push(range);
        }
    }

    /// An absent item with no comment of its own (#1079): a comment
    /// floated earlier in the *same* sequence can no longer become a foot
    /// -- real yq heads the next key with it (`a:\n  - # c\n  -\nb: 2\n`).
    /// One from a nested sequence that has already closed stays pending
    /// for the next node to place (`a:\n  - b:\n      - # c\n  -\nz: 1\n`
    /// is `.a[0].b`'s key's foot, measured). The pending lines themselves
    /// stay to go forward -- and at end of document, lines pending after
    /// any absent item are the root's foot (`- 1\n-\n# c\n`, measured).
    fn end_float(&mut self) {
        if let Some(previous) = self.floated_item_comment {
            if self.sequence_still_open(previous.seq_serial) {
                self.floated_item_comment = None;
            }
        }
        self.block_passed_absent_item = true;
    }

    /// The owner key of the innermost open block sequence, for a sequence
    /// about to open inside one of its items (#1079). Called before the new
    /// sequence's own push, with the item frame on top.
    fn enclosing_seq_owner_key(&mut self) -> Option<usize> {
        self.enclosing_seq_frame()
            .and_then(|frame| frame.owner_key_bp)
    }

    /// Record the block sequence that just became the top frame (#1079).
    /// Called right after its `indent_stack`/`type_stack` push. Entries at
    /// this depth or deeper belong to sequences that have since closed
    /// through one of the paths that don't touch this stack, so they go
    /// first.
    fn register_seq_frame(
        &mut self,
        indent: usize,
        owner_key_bp: Option<usize>,
        direct_key_value: bool,
    ) {
        let frame_len = self.indent_stack.len();
        self.trim_seq_frames_to(frame_len - 1);
        let serial = self.next_seq_serial;
        self.next_seq_serial += 1;
        self.seq_frames.push(SeqFrame {
            serial,
            frame_len,
            indent,
            owner_key_bp,
            direct_key_value,
        });
    }

    /// A bare `- # c` item's trailing comment whose value turned out absent
    /// (#1079): there is no node to attach it to, so real yq moves it -- see
    /// [`Self::floated_item_comment`]. It joins the pending standalone
    /// block, which is what carries it forward, in source order: the
    /// absence lookahead has already recorded any comment-only lines that
    /// follow it (`- # c\n# h\n- 2\n` reads `c\nh` on `.[1]`).
    ///
    /// Called with the item frame on top of the stack, so the sequence is
    /// the frame below it.
    fn float_absent_item_comment(&mut self, range: (u32, u32)) {
        // An earlier float whose own sequence has closed since (this item
        // is in an outer sequence: `a:\n  - b:\n      - # c\n  - # d\n`)
        // settles onto its key's foot now, before this one takes its place
        // as the pending float -- real yq gives `.a[0].b`'s key `c` there.
        // One from this same sequence simply goes forward with the block.
        if let Some(previous) = self.floated_item_comment.take() {
            self.settle_float_if_sequence_closed(previous, None);
        }
        let Some(frame) = self.enclosing_seq_frame() else {
            return; // omni-dev: coverage tolerate-line reason="unreachable: every block-sequence open registers a frame at its own depth before any item of it can be parsed (#1079)"
        };
        let idx = self
            .pending_head_lines
            .partition_point(|&(start, _)| start < range.0);
        self.pending_head_lines.insert(idx, range);
        self.block_passed_absent_item = true;
        self.floated_item_comment = Some(FloatedItemComment {
            range,
            seq_serial: frame.serial,
            seq_indent: frame.indent,
            owner_key_bp: frame.owner_key_bp,
            direct_key_value: frame.direct_key_value,
        });
    }

    /// Settle a pending block whose forward attachment is already disproven
    /// (a blank line follows it) onto `PREV`'s foot. A no-op when `PREV` is
    /// unset, which means a blank line detached the block backwards too —
    /// it stays pending and goes forward to whatever opens next.
    fn resolve_pending_block_backward(&mut self) {
        if self.pending_head_lines.is_empty() {
            return; // omni-dev: coverage tolerate-line reason="unreachable: this function's sole caller (record_standalone_comment) only invokes it from inside a match on `pending_head_lines.last()`, so pending_head_lines is already known non-empty here (#798)"
        }
        if let Some(prev) = self.attached_prev() {
            self.drain_pending_into_foot(prev);
        }
    }

    /// Hook for "a node that can own a standalone comment just opened at
    /// [`Self::last_open_bp_pos`]" (#798): attach any pending block as its
    /// `head`, and make it the `PREV` a later block can attach its `foot` to.
    ///
    /// Called where a mapping *key* opens and where a sequence item's
    /// *content* opens — the two nodes real yq answers `head_comment`/
    /// `foot_comment` from. Not called for container opens, which own no
    /// standalone comment of their own (a comment above a nested mapping's
    /// first key belongs to that key, measured), and not for an inline
    /// `k: v`'s value node.
    ///
    /// The key's column is its mapping's indent (`indent_stack`'s top), the
    /// column a dedented standalone comment is compared against (#2811).
    fn attach_head_foot_to_key(&mut self, column: usize) {
        let depth = self.indent_stack.len() - 1;
        let bp = self.last_open_bp_pos;
        self.attach_head_foot_at(bp, column);
        if let Some(slot) = self.frame_key_bp.get_mut(depth) {
            *slot = bp;
        }
    }

    /// [`Self::attach_head_foot_to_key`] for a key whose bp is known but has
    /// not opened yet (#2811 review): `? key` dispatches to many different
    /// value kinds (sequence, flow collection, block scalar, alias, compact
    /// mapping, plain scalar), each opening the key's own bp at a different
    /// point below the dispatch, so `self.last_open_bp_pos` is not yet the
    /// key's bp when the standalone-comment hook must run, ahead of
    /// dispatch, at `self.bp_pos`. Without this, `parse_explicit_key` called
    /// [`Self::attach_head_foot_at`] directly and never wrote
    /// [`Self::frame_key_bp`], so a dedented comment after an explicit key's
    /// only entry in a mapping resolved [`Self::positional_foot_target`]'s
    /// `frame_key_bp` lookup to the `usize::MAX` sentinel and was silently
    /// dropped instead of landing on the key's foot.
    fn attach_head_foot_to_key_at(&mut self, bp: usize, column: usize) {
        let depth = self.indent_stack.len() - 1;
        self.attach_head_foot_at(bp, column);
        if let Some(slot) = self.frame_key_bp.get_mut(depth) {
            *slot = bp;
        }
    }

    /// [`Self::attach_head_foot_to_key`] for a node whose bp is known but
    /// which has not opened yet — a sequence item's content, flushed *before*
    /// `parse_value` runs so [`Self::flush_pending_head_lines`] still sees
    /// `self.pos` at the content's own line rather than past the whole value.
    /// `column` is the column the node's line starts at: the `-` for an
    /// item, not its content (#2811).
    fn attach_head_foot_at(&mut self, bp: usize, column: usize) {
        self.attach_head_foot_at_with(bp, column, true);
    }

    /// [`Self::attach_head_foot_at`] for a bare `-` item's value deferred
    /// to a later line (#1079's hook), whose bp may turn out to be a
    /// mapping or sequence: a foot deferred to "the item's first key or
    /// scalar" (#2811) is not claimed here but by that node -- the first
    /// key's own hook, or [`Self::claim_deferred_foot`] in
    /// `parse_block_node`'s scalar arms (`- a:` / `    - 1` / ` # c` /
    /// blank / `-` / `  k: v` is `.[1].k`'s key's foot, measured).
    fn attach_deferred_item_value_at(&mut self, bp: usize, column: usize) {
        self.attach_head_foot_at_with(bp, column, false);
    }

    /// A foot deferred by [`Self::settle_dedented_block`] to the sequence
    /// item's first key/scalar lands on `bp` (#2811).
    fn claim_deferred_foot(&mut self, bp: usize) {
        if !self.deferred_foot_lines.is_empty() {
            let deferred = &mut self.deferred_foot_lines;
            self.comments.entry(bp).or_default().foot.append(deferred);
        }
    }

    fn attach_head_foot_at_with(&mut self, bp: usize, column: usize, claim_deferred: bool) {
        self.flush_pending_head_lines(Some((bp, column)));
        if claim_deferred {
            self.claim_deferred_foot(bp);
        }
        self.last_head_foot_bp = Some(bp);
        self.block_passed_absent_item = false;
        self.saw_head_foot_node = true;
        if self.first_head_foot_bp.is_none() {
            self.first_head_foot_bp = Some(bp);
            // The next open either way: a key is hooked after its own open
            // and its value is the next one; an item's content or a
            // document root is hooked before its own.
            self.first_node_value_idx = self.bp_to_text.len();
            self.first_node_pos = self.pos;
        }
        // A real node has now opened in this document, so a later blank
        // line detaching *its own* `PREV` must fall forward as usual, not
        // reach back across the document boundary a second time (#798).
        self.document_boundary_fallback_bp = None;
    }

    /// The head hook for a sequence item whose value is a same-line compact
    /// mapping (`- k: v`) or nested sequence (`- - x`), run before that
    /// container opens at `bp` (#2811): real yq gives a block above such an
    /// item to the container as a whole -- go-yaml's "stem comment" -- so
    /// `.[1] | head_comment` answers directly, and `.[1].k | key |
    /// head_comment` is empty. Only the head: the key/item that opens right
    /// after is still `PREV` for a later foot, and claims any deferred foot,
    /// via its own [`Self::attach_head_foot_at`].
    ///
    /// A no-op while a comment is floated off an earlier absent bare item
    /// (#1079's [`Self::floated_item_comment`]): this hook's own bp is a
    /// stem, not a genuine resolved node, so it must not pre-empt the
    /// float's settle logic the way a real [`Self::attach_head_foot_at`]/
    /// [`Self::attach_head_foot_to_key`] call would (both set `is_item` from
    /// `self.current_type`, which reads identically true for a stem as for
    /// a genuine item content, so the float would otherwise wrongly settle
    /// onto this stem's own bp instead of the sequence's owner key --
    /// confirmed live: `k:\n  - - # c1\n  - - # c2\n` regresses `.k`'s
    /// foot from `c1` to empty without this guard). Deferring here leaves
    /// the float exactly where [`Self::float_absent_item_comment`]'s own
    /// "settle the previous float" step (or, at end of document,
    /// [`Self::flush_pending_head_lines`]'s own check) already resolves it
    /// correctly once the sequence this item recurses into registers its
    /// own frame and reaches a genuine content node.
    fn attach_item_head_at(&mut self, bp: usize, column: usize) {
        if self.floated_item_comment.is_some() {
            return;
        }
        // A block trailing an item whose value turned out absent, closed by
        // this very stem's own recursion (#2811 review): the sequence that
        // absent item belonged to (`.k[0]` in `k:\n  - -\n    # c1\n  - -
        // x\n`) has already closed with nothing to give the block to, so
        // real yq defers it forward the same way a genuine dedent's
        // unclaimed block already defers to "the item opening now's first
        // key/scalar" (`Self::settle_dedented_block`'s own `next.is_some()`
        // arm) -- as that node's *foot*, not this stem's head. Confirmed
        // live: `.k[1][0]` (the scalar `x`, resolved by the sequence this
        // stem is about to recurse into) gets `c1` as its foot, not head.
        // Without this, `flush_pending_head_lines` ran normally here and
        // (since the block sits at the same column the absent item's own
        // nested sequence had, not shallower) placed it by the ordinary
        // adjacent-node rule instead -- `.k[1][0]`'s *head*, still wrong.
        //
        // Guarded on `pending_head_lines` actually holding something: an
        // absent item with no comment of its own reaches every later stem
        // in the same sequence with the flag still set (nothing has cleared
        // it), and must fall through to the ordinary no-op flush below
        // rather than resetting the flag on an empty deferral.
        if self.block_passed_absent_item && !self.pending_head_lines.is_empty() {
            self.block_passed_absent_item = false;
            let pending = &mut self.pending_head_lines;
            self.deferred_foot_lines.append(pending);
            return;
        }
        self.flush_pending_head_lines(Some((bp, column)));
    }

    /// A scalar or flow collection about to open as a document's own
    /// content node is the node real yq answers the document's
    /// `head_comment`/`foot_comment` from (#2811): `# lead` / `42` /
    /// `# foot` is head `lead`, foot `foot` -- without this the trailing
    /// block had no `PREV` and joined the head. A no-op inside any
    /// container, where the node is a mapping value or sequence item
    /// whose owner is already decided.
    fn attach_document_root_node(&mut self, column: usize) {
        if self.type_stack.len() <= 1 {
            self.attach_head_foot_at(self.bp_pos, column);
        } else if self.current_type == Some(NodeType::SequenceItem) {
            // A bare `-` item's scalar value on its own line: the node a
            // foot deferred to "the item's first key/scalar" was waiting
            // for (#2811; the item's own hook left it unclaimed).
            self.claim_deferred_foot(self.bp_pos);
        }
    }

    /// A mapping just opened as the top frame: give it a fresh
    /// [`Self::frame_key_bp`] slot (#2811), so a stale key of an earlier
    /// mapping at this depth can never be read as one of its own. Runs once
    /// per mapping open, not per key.
    fn open_frame_key_slot(&mut self) {
        let depth = self.indent_stack.len() - 1;
        if depth < self.frame_key_bp.len() {
            self.frame_key_bp[depth] = usize::MAX;
        } else {
            self.frame_key_bp.resize(depth + 1, usize::MAX);
        }
    }

    /// [`Self::flush_pending_head_lines`], but called from
    /// [`Self::start_document`] where the pending block may instead belong
    /// to the document that's *closing*, not the one about to open (#798).
    ///
    /// Measured against pinned yq v4.53.3: a block sitting directly against
    /// a `---`/`...` boundary (no blank line between the block and the
    /// marker) stays with the closing document as its own root `foot`,
    /// rather than following the ordinary adjacent-block-attaches-forward
    /// rule onto the new document's `head` --
    /// `a: 1\n# mid\n---\nb: 2\n` puts `mid` on document 0's `foot`, not
    /// document 1's `head`. A block separated from the marker by a blank
    /// line and still attached to a `PREV` is the closing document's to
    /// place, as at its end of input: `PREV`'s foot, or by column when
    /// dedented (`a:` / `  b: 1` / `# c` / blank / `---` is `.a`'s key's
    /// foot, measured, #2811) -- this runs before `last_head_foot_bp` is
    /// reset below, so `PREV` is still the closing document's own last key.
    /// With no `PREV` it takes the ordinary path (delegated to
    /// [`Self::flush_pending_head_lines`]).
    ///
    /// `old_root_bp` is `None` for the very first document (nothing to
    /// reach back to), in which case this always defers to the ordinary
    /// path.
    ///
    /// #1079 state (a floated comment, or the plain "an absent item was
    /// seen" flag) is settled first, against the *closing* document, and
    /// takes priority over the ordinary blank-line-adjacency rule above --
    /// measured against pinned yq v4.53.3, a document boundary always fully
    /// closes any open block sequence, so unlike an in-document dedent
    /// there is no "still at the sequence's own column" case for
    /// [`Self::settle_float_if_sequence_closed`]'s column check to apply to
    /// the node opening in the *next* document; passing `next: None` skips
    /// straight to its `owner_key_bp` fallback. A floated comment still
    /// becomes that key's foot when there is one (`a:\n  - # c\n---\nb: 2\n`
    /// puts `c` on `.a`'s foot, whether or not a blank line separates it
    /// from the marker -- unlike true end of input, which collapses this
    /// same shape onto the *root*'s foot instead, `a:\n  - # c\n` measured
    /// against `.a | key | foot_comment` empty vs. `. | foot_comment` = `c`).
    /// With no owner key, or after an absent item with no comment of its
    /// own, whatever is left is the closing document's own root foot.
    fn flush_pending_head_lines_at_boundary(
        &mut self,
        next: Option<(usize, usize)>,
        old_root_bp: Option<usize>,
    ) {
        if self.pending_head_lines.is_empty() {
            return;
        }
        if let Some(old_root) = old_root_bp {
            if let Some(floated) = self.floated_item_comment.take() {
                self.settle_float_if_sequence_closed(floated, None);
            }
            if core::mem::take(&mut self.block_passed_absent_item) {
                if !self.pending_head_lines.is_empty() {
                    self.drain_pending_into_foot(old_root);
                }
                return;
            }
            if self.pending_head_lines.is_empty() {
                return;
            }
            let block_end = self.pending_head_lines[self.pending_head_lines.len() - 1].1 as usize;
            let adjacent_to_marker = !self.blank_line_between(block_end, self.pos);
            if adjacent_to_marker {
                // A block that never actually left the closing document's
                // outermost container keeps #798's flat rule (the closing
                // document's own root foot); one that sits inside a
                // container nested *within* that outermost one is instead
                // placed by column, the same dedent-aware rule
                // `flush_pending_head_lines_with_root`'s own dedent branch
                // already applies in-document/at EOF (#2811 review).
                let target = self.positional_boundary_foot_target().unwrap_or(old_root);
                self.drain_pending_into_foot(target);
                return;
            }
            if self.attached_prev().is_some() {
                self.flush_pending_head_lines_with_root(None, old_root);
                return;
            }
        }
        self.flush_pending_head_lines(next);
    }

    /// Whether a wholly blank line (empty or inline whitespace only) lies in
    /// `[from, to)`.
    ///
    /// Every caller passes `from` at the end of a comment line's text, so
    /// the first break scanned is that line's own; `to` is always mid-line
    /// (a key, a value, or just past a `---`), so a trailing partial line is
    /// never counted. Lines with content in between are not blank: an
    /// empty `-` item or a bare `-` whose value is deferred sits between a
    /// comment block and the node it heads without attaching anything
    /// itself, and counting its line break as a blank line sent the block
    /// backwards onto `PREV`'s foot (`- 1\n# h\n-\n- 3\n` is `.[2]`'s head
    /// in real yq, #1079).
    ///
    /// Deliberately not the same algorithm as `light.rs`'s cursor-side twin
    /// of the same name (#2811 review): that one only needs a total break
    /// *count* because its caller already guarantees nothing but
    /// whitespace in the range, while this one must tell "two adjacent
    /// breaks" apart from "a break, real (non-blank) content, another
    /// break" -- exactly the `- 1\n# h\n-\n- 3\n` case above, which a
    /// count-only check cannot distinguish from a genuine blank line.
    fn blank_line_between(&self, from: usize, to: usize) -> bool {
        let mut seen_content = true;
        let mut p = from;
        let end = to.min(self.input.len());
        while p < end {
            let byte = self.input[p];
            if is_line_break(byte) {
                if !seen_content {
                    return true;
                }
                seen_content = false;
                // CRLF is one break -- via the shared `line_break_len` rule
                // (`crate::text::line_break`'s module doc: "there is still
                // exactly one definition") rather than a second hand-rolled
                // copy of the same check (#2811 review). The `p + 1 < end`
                // guard is still needed on top: `line_break_len` reads
                // `self.input` unbounded by `end`, and `end` can fall
                // between a CRLF pair's two bytes.
                if line_break_len(self.input, p) == 2 && p + 1 < end {
                    p += 1;
                }
            } else if !Self::is_inline_whitespace(byte) {
                seen_content = true;
            }
            p += 1;
        }
        false
    }

    /// The column of `pos` if everything between it and the start of its
    /// line is inline whitespace — i.e. `pos` begins a *standalone* comment
    /// rather than one trailing content — else `None`. See
    /// [`Self::record_standalone_comment`]. Bytes, like `current_column`:
    /// a tab counts once, as it does in go-yaml's mark.
    fn standalone_column_before(&self, pos: usize) -> Option<usize> {
        let mut p = pos;
        while p > 0 && Self::is_inline_whitespace(self.input[p - 1]) {
            p -= 1;
        }
        (p == 0 || is_line_break(self.input[p - 1])).then_some(pos - p)
    }

    /// Whether the line before the one containing `pos` is blank (empty or
    /// whitespace only). Start of input is not a blank line.
    ///
    /// Read straight off the text rather than tracked as parser state, so it
    /// cannot be confused by a `skip_newlines` call entered mid-line — see
    /// [`Self::record_standalone_comment`].
    fn blank_line_precedes(&self, pos: usize) -> bool {
        // Step back over this line's indent to its opening break, then over
        // that break, then check whether the line before it is empty.
        let mut p = pos;
        while p > 0 && Self::is_inline_whitespace(self.input[p - 1]) {
            p -= 1;
        }
        if p == 0 {
            return false;
        }
        // `p - 1` is the break that ended the previous line. A CRLF counts
        // once: step back over the `\n` and then over a `\r` if it precedes.
        let mut q = p - 1;
        if self.input[q] == b'\n' && q > 0 && self.input[q - 1] == b'\r' {
            q -= 1;
        }
        if q == 0 {
            return false;
        }
        // Walk the previous line backwards; blank iff we reach another break
        // (or the start of input) without meeting content.
        let mut r = q;
        while r > 0 && !is_line_break(self.input[r - 1]) {
            if !Self::is_inline_whitespace(self.input[r - 1]) {
                return false;
            }
            r -= 1;
        }
        // A whitespace-only run back to the start of input is a blank line
        // only if it is non-empty; `r == q` means the previous "line" was
        // empty, which is blank either way.
        r < q || r > 0
    }

    /// Like [`Self::maybe_capture_line_comment`], but the owning node doesn't
    /// exist yet: stash the comment's byte range in
    /// [`Self::pending_head_comment`] instead of `comments` directly.
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
    /// **Ordering, at the two mapping-key call sites**
    /// (`parse_mapping_entry`, `parse_compact_mapping_entry`): call this
    /// *before* the paired `maybe_capture_line_comment` call for the same
    /// `owner_bp_pos`, not after. `line` is a `Vec` (#1085) precisely
    /// because both a floated comment and the key's own genuine comment can
    /// land on the same key, and real yq renders them in source order --
    /// the floated one (written on an *earlier* line, by construction: it
    /// was deferred from an anchor that had to appear before this key ever
    /// opened) always precedes the key's own. Calling this first yields
    /// push order `[floated, own]`, matching the oracle
    /// (`.a.b | key | line_comment` joins them `"floated\nown"`). Getting
    /// this backwards doesn't lose either comment -- both still end up in
    /// the `Vec` either way -- but silently renders them in the wrong
    /// order.
    ///
    /// (Historical note: before #1085, `line` was a single `Option` slot and
    /// this function had to run *after* `maybe_capture_line_comment` so a
    /// genuine same-line comment would win the only slot rather than being
    /// clobbered by an unrelated floated one -- #784 review. `push_line_comment`
    /// no longer clobbers anything, so that constraint is gone; only the
    /// output *order* depends on call order now.)
    #[inline]
    fn take_pending_head_comment(&mut self, owner_bp_pos: usize) {
        if let Some(range) = self.pending_head_comment.take() {
            self.push_line_comment(owner_bp_pos, range);
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

    /// Under `json_strict` (#2778), a scalar *value* position (never a key
    /// -- see `json_strict`'s doc comment) may not start with `'`: JSON has
    /// no single-quoted string spelling, so this applies unconditionally,
    /// with no leniency to reproduce.
    fn err_json_strict_single_quote(&self) -> YamlError {
        self.err_unexpected_char(self.pos, "single-quoted scalar in JSON input")
    }

    /// Under `json_strict` (#2778), validate a plain scalar *value*'s
    /// already-consumed text (`self.input[start..end]`) against
    /// [`json_strict_plain_scalar_ok`]. A no-op when `json_strict` is
    /// unset, so every call site stays cheap on the ordinary YAML path.
    fn check_json_strict_scalar(&self, start: usize, end: usize) -> Result<(), YamlError> {
        if self.json_strict && !json_strict_plain_scalar_ok(&self.input[start..end]) {
            return Err(self.err_unexpected_char(start, "YAML-only scalar in JSON input"));
        }
        Ok(())
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
                let comment_start = self.pos;
                self.skip_to_eol();
                self.record_standalone_comment(comment_start, self.pos);
            } else if b == b' ' {
                // Check if rest of line is whitespace or comment
                let start = self.pos;
                self.skip_inline_whitespace();
                if self.at_break() || self.peek() == Some(b'#') || self.peek().is_none() {
                    if self.peek() == Some(b'#') {
                        let comment_start = self.pos;
                        self.skip_to_eol();
                        self.record_standalone_comment(comment_start, self.pos);
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
            // #2778: this dedicated arm bypasses `parse_block_node` (see
            // above), so it needs its own `json_strict` gate rather than
            // inheriting one from there -- `|`/`>` have no JSON spelling
            // either way (real yq's token scanner rejects both).
            if self.json_strict {
                return Err(
                    self.err_unexpected_char(self.pos, "block scalar indicator in JSON input")
                );
            }
            return self.parse_block_scalar(0);
        }

        self.parse_block_node(0)
    }

    /// Start a new document within the virtual root sequence.
    /// This doesn't open a container - the document IS its content.
    fn start_document(&mut self) {
        self.in_document = true;
        // The document that's closing, if any -- `document_start_bp_pos`
        // still holds its root right up until the overwrite below.
        // `seen_any_document` is the only way to tell "no previous document"
        // apart from "its root happens to be bp 0" (#798).
        let old_root_bp = self.seen_any_document.then_some(self.document_start_bp_pos);
        // A foot deferred to a sequence item's first key/scalar (#2811) has
        // no such node left to claim it once the document that deferred it
        // closes at this `---`/`...` boundary with nothing having claimed
        // it in between (`end_document`, called just above by every caller
        // of `start_document` past the first, doesn't touch this field
        // either) -- left alone, it survived untouched into the new
        // document and was wrongly claimed there by whatever node opened
        // first via `claim_deferred_foot` (#2811 review). Mirrors the
        // end-of-parse flush in `parse` (just above its own `end_document`
        // call), but targets the *closing* document's root (`old_root_bp`)
        // rather than `document_start_bp_pos`, which the overwrite below
        // is about to repoint at the new document.
        if let Some(old_root) = old_root_bp {
            if !self.deferred_foot_lines.is_empty() {
                let deferred = &mut self.deferred_foot_lines;
                self.comments
                    .entry(old_root)
                    .or_default()
                    .foot
                    .append(deferred);
            }
        }
        self.seen_any_document = true;
        self.document_start_bp_pos = self.bp_pos;
        // A standalone comment block seen before this document's content
        // belongs to the document's own node, not to its first key -- real
        // yq answers `. | head_comment`, not `.a | key | head_comment`, for
        // a leading `# lead` (#798). `document_start_bp_pos` is the bp the
        // content node is about to take, and `end_document` synthesizes a
        // node there even when the document turns out empty, so the block
        // always lands on something real.
        //
        // Deliberately *before* `pending_head_comment` is cleared below:
        // that field is #784's single floated trailing comment, unrelated to
        // this block. Uses the boundary-aware flush, not the ordinary one:
        // a block still pending here may belong to the *closing* document
        // instead (see [`Self::flush_pending_head_lines_at_boundary`]).
        let root_bp = self.document_start_bp_pos;
        self.flush_pending_head_lines_at_boundary(Some((root_bp, 0)), old_root_bp);
        self.last_head_foot_bp = None;
        // #1079: `block_passed_absent_item` belongs to whichever document set
        // it. `flush_pending_head_lines_at_boundary` above already clears it
        // when it drains a pending block, but it early-returns without
        // touching any state when nothing is pending -- which is exactly the
        // case an absent bare item with no comment of its own leaves behind
        // (`end_float` sets the flag without ever pushing into
        // `pending_head_lines`). Left set, it would wrongly force a later,
        // unrelated comment in *this* document onto its own root foot at end
        // of input, the same way it correctly does for the document that
        // actually saw the absent item.
        self.block_passed_absent_item = false;
        // Primes the no-`PREV` fallback for whatever comes next in the new
        // document -- see [`Self::document_boundary_fallback_bp`].
        self.document_boundary_fallback_bp = old_root_bp;
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
                // The second whole-line-comment funnel (#798): a comment
                // before the first `---` of a document *with* directives is
                // consumed here and never reaches `skip_newlines`.
                let comment_start = self.pos;
                self.skip_to_eol();
                self.record_standalone_comment(comment_start, self.pos);
            } else if b == b' ' || b == b'\t' {
                let start = self.pos;
                self.skip_inline_whitespace();
                if self.at_break() || self.peek() == Some(b'#') || self.peek().is_none() {
                    if self.peek() == Some(b'#') {
                        let comment_start = self.pos;
                        self.skip_to_eol();
                        self.record_standalone_comment(comment_start, self.pos);
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
            // The nearest enclosing mapping key is where real yq puts a
            // comment floated off an absent item of this sequence when the
            // sequence closes past a dedent (#1079): the key whose value
            // this sequence is -- the key whose line just ended -- or, for
            // a sequence nested in a sequence item, the enclosing
            // sequence's own. Read before the push below changes
            // `current_type`. See `settle_float_if_sequence_closed`.
            let (owner_key_bp, direct_key_value) = match self.current_type {
                Some(NodeType::Mapping) => (self.last_head_foot_bp, true),
                Some(NodeType::SequenceItem) => (self.enclosing_seq_owner_key(), false),
                _ => (None, false),
            };
            // Open new sequence
            self.write_bp_open();
            self.write_ty(true); // 1 = sequence
            self.indent_stack.push(indent);
            self.push_type(NodeType::Sequence);
            self.register_seq_frame(indent, owner_key_bp, direct_key_value);
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
        // Not a `take_pending_head_comment` call site (#784): a
        // plain-scalar item's line comment lives on the *scalar's* own bp
        // (see the `take_pending_head_comment` call after `parse_value`
        // below), and any other continuation (compact-mapping key, a
        // nested `parse_mapping_entry`/`parse_sequence_item` reached via a
        // fresh line dispatch) claims it at its own, already-instrumented
        // key/wrapper-open site instead. The wrapper's own bp *is* read for
        // a bare item's (`had_property == false`) trailing comment when its
        // value is deferred to a node on a later line (#1079, below) — that
        // node hasn't opened yet, so the wrapper is the only bp available
        // to key the comment against.
        let wrapper_bp = self.last_open_bp_pos;

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
            // `parse_mapping_entry`'s anchor case). A property-prefixed
            // item defers it past the property (`defer_line_comment`,
            // #784). A bare item has no property to defer past, and where
            // the comment goes depends on whether a value follows at all
            // (#1079):
            //
            // - deferred to a node on a later line (a mapping, nested
            //   sequence, or scalar): captured against the item wrapper's
            //   own bp, which the emitter renders on the dash line (or, for
            //   a scalar, above it) -- and that node, opening next at
            //   `self.bp_pos`, is the one `.[i]` resolves to and real yq
            //   answers `head_comment` from (`# h\n-\n  x\n` is `.[1]`'s
            //   head, not `.[0]`'s foot), so any pending standalone block
            //   attaches to it here, exactly as the plain-scalar arm below
            //   does for `- x`;
            // - absent (the next content is a sibling item, a dedent, or
            //   end of input): there is no node, and real yq floats the
            //   comment -- see `float_absent_item_comment`. An absent item
            //   with no comment attaches nothing either, so a block above
            //   it reaches the next item that does exist -- and it ends an
            //   earlier floated comment's claim on the enclosing key's foot
            //   (`a:\n  - # c\n  -\nb: 2\n` heads `b`, measured).
            if had_property {
                self.defer_line_comment();
            } else {
                let trailing = self.scan_trailing_comment();
                // The lookahead also records comment-only lines between
                // this dash and the following content. Those sit *inside*
                // the deferred value and belong to its first node
                // (`-\n  # c\n  - x\n` heads `.[0][0]`, measured), so only
                // the lines already pending here are this value's own head.
                let above_dash = self.pending_head_lines.len();
                let value_is_null = self
                    .next_line_settles_value_is_null(indent)
                    .unwrap_or_else(|| self.following_value_is_null(indent));
                if value_is_null {
                    match trailing {
                        Some(range) => self.float_absent_item_comment(range),
                        None => self.end_float(),
                    }
                    // A null item is still content: a block after it at end
                    // of document is the root's foot, not a comment-only
                    // document's head (`-\n# c\n`, measured).
                    self.saw_head_foot_node = true;
                } else {
                    // The lookahead can also have *settled* the block (a
                    // blank line after it resolves it backwards), so clamp.
                    let above_dash = above_dash.min(self.pending_head_lines.len());
                    let below_dash = self.pending_head_lines.split_off(above_dash);
                    self.attach_deferred_item_value_at(self.bp_pos, indent);
                    self.pending_head_lines = below_dash;
                    if let Some(range) = trailing {
                        self.push_line_comment(wrapper_bp, range);
                    }
                }
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
            // A block above this item heads the nested sequence itself, the
            // node `.[i]` resolves to, not its first item (#2811). The
            // recursive call's first bp write is that sequence's open (the
            // outer item frame at `indent + 1 < nested_indent` closes
            // nothing first), so it opens at the current `bp_pos`.
            self.attach_item_head_at(self.bp_pos, indent);
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
            // A block above this item heads the compact mapping itself, the
            // node `.[i]` resolves to, not its first key (#2811 -- real yq
            // answers `.[1] | head_comment` for `- 1` / `# h` / `- b: 1`).
            // `parse_compact_mapping_entry`'s first bp write is the
            // mapping's open, so it opens at the current `bp_pos`.
            self.attach_item_head_at(self.bp_pos, indent);
            self.parse_compact_mapping_entry(compact_indent)?;
            // Don't close anything - mapping and item will be closed by
            // close_deeper_indents when we see content at lower indent.
        } else {
            // Parse the item value normally
            // Pass structure indent for block scalars (content must be > this)
            let bp_pos_before_value = self.bp_pos;
            // A standalone comment above a sequence item attaches to the
            // item's *content* node, which is about to open at
            // `bp_pos_before_value` -- real yq answers `.[1] | head_comment`
            // directly for one, with no `key` step (#798). Flushed before
            // `parse_value` so the blank-line check still sees `self.pos` on
            // the item's own line. The item's column is its `-`'s, not the
            // content's (#2811).
            self.attach_head_foot_at(bp_pos_before_value, indent);
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

    /// [`Self::following_value_is_null`]'s answer read straight off the
    /// next line when that line settles it by itself: ordinary content at
    /// some indent, or end of input. `None` when the next line is blank,
    /// comment-only, or tab-led -- anything `skip_newlines` has an opinion
    /// about -- so the caller falls back to the full lookahead and the two
    /// cannot disagree. Called with `self.pos` on the current line's break.
    ///
    /// Exists for the bare `-` item, the common shape in record-style
    /// documents (`-\n  name: ...` per record, #1079): the full lookahead
    /// measured +0.9% on that shape, this reads a handful of bytes.
    #[inline]
    fn next_line_settles_value_is_null(&self, indent: usize) -> Option<bool> {
        let mut p = self.pos;
        let Some(&b) = self.input.get(p) else {
            return Some(true);
        };
        if !Self::is_break(b) {
            return None;
        }
        p += 1;
        if b == b'\r' && self.input.get(p) == Some(&b'\n') {
            p += 1;
        }
        let spaces = super::simd::count_leading_spaces(self.input, p);
        match self.input.get(p + spaces) {
            Some(&b) if !Self::is_break(b) && b != b'#' && b != b'\t' => Some(spaces <= indent),
            None if spaces == 0 => Some(true),
            _ => None,
        }
    }

    /// Parse a compact mapping entry within a sequence item.
    /// This handles `- key: value` where the mapping is inline with the sequence item.
    fn parse_compact_mapping_entry(&mut self, indent: usize) -> Result<(), YamlError> {
        // Open a mapping for this compact entry
        self.write_bp_open();
        self.write_ty(false); // 0 = mapping
        self.indent_stack.push(indent);
        self.push_type(NodeType::Mapping);
        self.open_frame_key_slot();

        // Mark key position
        self.set_ib();

        // Open key node
        self.write_bp_open();
        // Standalone comments attach to a mapping entry's *key* (#798).
        self.attach_head_foot_to_key(indent);

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
                // A trailing comment here is this key's own (issue #765) -
                // `self.last_open_bp_pos` still holds the key node's bp_pos
                // here since no value node has been opened yet (nothing
                // opens a BP node between the key's own close above and
                // this point). This mirrors `parse_mapping_entry`'s
                // identical capture just below - missing here left a
                // block-sequence item's *first* field (the only mapping
                // entry parsed by this function rather than
                // `parse_mapping_entry`) silently dropping its own key
                // comment (#785).
                //
                // Also claims a comment an *earlier*, unrelated anchor's
                // deferred value floated onto this key (#784). `line` is a
                // `Vec` (#1085): both can coexist, and real yq renders them
                // in source order (floated, then this key's own), which is
                // why `take_pending_head_comment` runs *first* here - see
                // its own doc comment for the ordering rationale (this used
                // to run second, to protect a single `Option` slot; that
                // constraint no longer applies).
                self.take_pending_head_comment(self.last_open_bp_pos);
                self.maybe_capture_line_comment(self.last_open_bp_pos);
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
            self.open_frame_key_slot();
        }

        // Mark key position
        self.set_ib();

        // Open key node
        self.write_bp_open();
        // Standalone comments attach to a mapping entry's *key* (#798).
        self.attach_head_foot_to_key(indent);

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
            // trailing comment on the key's own line (issue #765) -
            // `self.last_open_bp_pos` still holds the key node's bp_pos here
            // since no value node has been opened yet (nothing opens a BP
            // node between the key's own close above and this point).
            //
            // Also claims a comment an *earlier*, unrelated anchor's
            // deferred value floated onto this key (#784). `line` is a
            // `Vec` (#1085): both can coexist, and real yq renders them in
            // source order (floated, then this key's own), which is why
            // `take_pending_head_comment` runs *first* here - see its own
            // doc comment for the ordering rationale (this used to run
            // second, to protect a single `Option` slot; that constraint no
            // longer applies).
            self.take_pending_head_comment(self.last_open_bp_pos);
            self.maybe_capture_line_comment(self.last_open_bp_pos);
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
                            if self.json_strict {
                                return Err(self.err_json_strict_single_quote());
                            }
                            self.parse_single_quoted()?;
                            self.pos
                        }
                        _ => {
                            let start = self.pos;
                            let end = self.parse_unquoted_value_with_indent(indent);
                            self.check_json_strict_scalar(start, end)?;
                            end
                        }
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
            self.open_frame_key_slot();
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

        // Standalone comments attach to the key node (#798), which every
        // arm below opens at `self.bp_pos` -- inline, on a later line, or
        // synthesized empty -- so attach before the dispatch, as
        // `parse_mapping_entry` does at its key open. Also makes this key
        // the owner of a same-indent sequence value's floated item comment
        // (`? a\n:\n  - # c\nb: 2\n` is `.a`'s key's foot, #1079).
        //
        // Goes through `attach_head_foot_to_key_at` (not `attach_head_foot_
        // at` directly) so `frame_key_bp` records this key like every other
        // key form does -- otherwise a dedented comment after this key's
        // only entry in its mapping resolves `positional_foot_target`'s
        // `frame_key_bp` lookup to the `usize::MAX` sentinel and is silently
        // dropped (#2811 review: `? a\n:\n  - 1\n# c\n`).
        self.attach_head_foot_to_key_at(self.bp_pos, indent);

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
                // This sequence *is* the key, not any key's value -- but a
                // comment floated off one of its own absent items still
                // lands on the key's own foot when the sequence closes past
                // a dedent, the same as an ordinary mapping value's would
                // (measured against pinned yq v4.53.3: `? - a\n  - # c\n:
                // v\nb: 2\n` puts `c` on the explicit key's own foot, not
                // `.b`'s head). `last_head_foot_bp` still holds this key's
                // own bp, set by `attach_head_foot_at` above, unchanged
                // since (#1079).
                self.register_seq_frame(indent + 2, self.last_head_foot_bp, true);

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
                if self.json_strict {
                    return Err(self.err_json_strict_single_quote());
                }
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
                let start = self.pos;
                let end_pos = self.parse_unquoted_value_with_indent(indent);
                self.check_json_strict_scalar(start, end_pos)?;
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
                if self.json_strict {
                    return Err(self.err_json_strict_single_quote());
                }
                self.parse_single_quoted()?;
                self.pos
            }
            _ => {
                let start = self.pos;
                let end = self.parse_unquoted_value_with_indent(min_indent);
                self.check_json_strict_scalar(start, end)?;
                end
            }
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
                if self.json_strict {
                    return Err(self.err_json_strict_single_quote());
                }
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
                let start = self.pos;
                let end_pos = self.parse_unquoted_value_with_indent(min_indent);
                self.check_json_strict_scalar(start, end_pos)?;
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

            // #2279: a leading `,` (`[,1]`, and `[,]`'s first pass) is an
            // element YAML reads as an empty scalar and JSON has no spelling
            // for at all.
            if first && self.json_strict && self.peek() == Some(b',') {
                return Err(
                    self.err_unexpected_char(self.pos, "expected value or ']' in flow sequence")
                );
            }

            if !first {
                // Expect comma
                if self.peek() != Some(b',') {
                    return Err(
                        self.err_unexpected_char(self.pos, "expected ',' or ']' in flow sequence")
                    );
                }
                let comma_pos = self.pos;
                self.advance(); // Skip `,`
                self.skip_flow_whitespace();

                // #2279: `[1,,2]` -- a second `,` where an element belongs.
                if self.json_strict && self.peek() == Some(b',') {
                    return Err(
                        self.err_unexpected_char(self.pos, "expected value in flow sequence")
                    );
                }

                // Allow trailing comma -- except for JSON-sourced input
                // (#2279), where `[1,]` is exactly what real yq rejects.
                // Reported against the offending `,`, not the `]` that
                // merely revealed it.
                if self.peek() == Some(b']') {
                    if self.json_strict {
                        return Err(
                            self.err_unexpected_char(comma_pos, "trailing ',' in JSON array")
                        );
                    }
                    break;
                }
            }
            first = false;

            // Check for explicit key `? key : value` in flow context. Both
            // lookaheads scan up to the element's own length, not O(1), so
            // they're hoisted into locals and reused below rather than
            // re-run by the json_strict check and the dispatch separately.
            let is_explicit_key = self.looks_like_explicit_flow_key();
            let is_mapping_entry = is_explicit_key || self.looks_like_flow_mapping_entry();
            if self.json_strict && is_mapping_entry {
                // #2778: a JSON array element is a value, never a `key:
                // value` pair -- real yq's front end has no concept of an
                // implicit single-pair mapping inside `[...]` at all, so
                // `[a: 1]`/`["a":1]` error before the key's own text is even
                // examined. Reported at the element's start, matching every
                // other json_strict rejection in this loop.
                return Err(self.err_unexpected_char(self.pos, "mapping entry inside JSON array"));
            }
            if is_explicit_key {
                self.parse_explicit_flow_mapping_entry()?;
            } else if is_mapping_entry {
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
                // #2778: `?` has no JSON spelling at all -- this is a
                // structural rejection of the marker itself (real yq's
                // token scanner never accepts it as a value-start byte,
                // regardless of what follows), not a key-grammar check, so
                // it stays in scope even though key *text* grammar (#2777)
                // does not.
                if self.json_strict {
                    return Err(
                        self.err_unexpected_char(self.pos, "explicit key marker in JSON input")
                    );
                }
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
                if self.json_strict {
                    return Err(self.err_json_strict_single_quote());
                }
                self.parse_single_quoted()?;
                self.pos
            }
            _ => {
                let start = self.pos;
                let end = self.parse_flow_unquoted_value();
                self.check_json_strict_scalar(start, end)?;
                end
            }
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
        // #1353: recorded per-declaration, alongside (not derived from) the
        // last-wins `anchors` map above -- see `bp_to_anchor`'s own doc
        // comment.
        self.bp_to_anchor.insert(self.bp_pos, name.clone());
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
        self.anchors.insert(name.clone(), self.bp_pos - 1);
        // #1353: see `parse_anchor`'s own matching insert above.
        self.bp_to_anchor.insert(self.bp_pos - 1, name);
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

        // A standalone comment block still pending at EOF has no following
        // node to head (#798): it becomes the last key/item's foot, or the
        // document root's foot when a blank line detached it from that too.
        self.flush_pending_head_lines(None);
        // A foot deferred to a sequence item's first key/scalar when no
        // such node ever opened (#2811): `- -` with nothing after it is an
        // empty inner item, and real yq gives the document the comment
        // (`- a:` / `    - 1` / ` # c` / blank / `- -` is `. | foot_comment`).
        if !self.deferred_foot_lines.is_empty() {
            let root = self.document_start_bp_pos;
            let deferred = &mut self.deferred_foot_lines;
            self.comments.entry(root).or_default().foot.append(deferred);
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
            bp_to_anchor: core::mem::take(&mut self.bp_to_anchor),
            aliases: core::mem::take(&mut self.aliases),
            tags: core::mem::take(&mut self.tags),
            comments: core::mem::take(&mut self.comments),
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

        // #2778: a single chokepoint for every value-start byte real yq's
        // token scanner (goccy/go-json's `skipValue`) would refuse outright
        // -- `?` (explicit key), `&`/`!` (anchor/tag, no JSON spelling at
        // all), `*` (alias), `'` (single quote) -- ahead of the per-arm
        // dispatch below, so a *new* arm added to that match can't reopen
        // this gap by omission the way the sequence-dash and bare-mapping
        // checks further down (which still need their own, narrower logic:
        // `-1` is a legal number but `- 1` is not, and a quoted key still
        // reaches its own arm) could not cover on their own. `#`/a line
        // break/EOF are not JSON tokens either, but are harmless between or
        // after a value, so they still fall through unchanged.
        if self.json_strict
            && !matches!(
                self.peek(),
                Some(
                    b'{' | b'[' | b'"' | b't' | b'f' | b'n' | b'-' | b'0'
                        ..=b'9' | b'#' | b'\n' | b'\r'
                ) | None
            )
        {
            return Err(self.err_unexpected_char(self.pos, "not a valid JSON value"));
        }

        // Check what kind of content this is
        match self.peek() {
            Some(b'-') if Self::is_ws_break_or_eoi(self.peek_at(1)) => {
                // #2778: a JSON value never starts with `- ` (a block
                // sequence dash followed by whitespace/break/EOF) -- real
                // yq's token scanner only accepts `{ [ " t f n -` or a
                // digit as a value start, and a bare `-1` (no space) is a
                // negative-number token that never reaches this arm at all
                // (it falls to the catch-all scalar arm below instead).
                if self.json_strict {
                    return Err(
                        self.err_unexpected_char(self.pos, "YAML block sequence in JSON input")
                    );
                }
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
                self.attach_document_root_node(indent);
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
                            self.attach_document_root_node(indent);
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
                            self.attach_document_root_node(indent);
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
                    self.attach_document_root_node(indent);
                    self.parse_alias()?;
                }
            }
            Some(_) => {
                // Check if this looks like a mapping entry (has `: ` on this line)
                // This handles both quoted keys ("foo": bar) and unquoted keys (foo: bar)
                if self.json_strict && self.looks_like_mapping_entry() {
                    // #2778: a bare `key: value` (no enclosing `{}`) is not
                    // a JSON value at all -- real yq's front end never
                    // reads block-mapping syntax, so it errors on `a: 1`
                    // the same way it does on any other non-`{[`t f n-digit`
                    // token start. Checked ahead of `looks_like_mapping_entry`'s
                    // own dispatch so the key's text is never even examined
                    // (matching rule 4(c): key grammar stays out of scope).
                    return Err(
                        self.err_unexpected_char(self.pos, "YAML mapping entry in JSON input")
                    );
                }
                if self.looks_like_mapping_entry() {
                    self.parse_mapping_entry(indent)?;
                } else {
                    // Same ambiguous-gap error as parse_mapping_entry (#901,
                    // #959) - the original repro this issue was filed
                    // against.
                    self.check_mapping_under_mapping_gap(indent, false)?;
                    // Scalar value - either bare document scalar or value in a container
                    self.close_deeper_indents(indent);
                    self.attach_document_root_node(indent);
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
                            if self.json_strict {
                                return Err(self.err_json_strict_single_quote());
                            }
                            self.parse_single_quoted()?;
                            self.pos
                        }
                        _ => {
                            let start = self.pos;
                            let end = if is_truly_doc_root {
                                self.parse_unquoted_value_doc_root(indent)
                            } else {
                                self.parse_unquoted_value_with_indent(indent)
                            };
                            self.check_json_strict_scalar(start, end)?;
                            end
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

/// Whether `bytes` is a plain scalar token real yq's `-p json` front end
/// (`goccy/go-json` v0.10.6) would accept, per its two-layer boundary
/// (#2778's triage plan derives this from the reference's own source):
///
/// 1. **Token scanner**: a value token may start only with a digit or `-`
///    (`.5`, `+1`, `~` never reach the parse step at all).
/// 2. **Number parse** (`strconv.ParseFloat`): optional `-`, `digits ['.'
///    digits*] | '.' digits`, optional `[eE][+-]?digits`, at least one
///    mantissa digit, leading zeros allowed, `ErrRange` on overflow.
///
/// `true`/`false`/`null` are the only non-numeric plain scalars this
/// accepts. Rust's `f64::from_str` matches Go's `ParseFloat` on every row
/// of the pinned matrix (`1.`, `-.5`, `01`, `1.e5` accepted; `1e`, `1e+`,
/// `-`, `-.`, `-e1`, `1..2`, `1e1.5` rejected) -- `is_finite()` reproduces
/// `ErrRange` since Rust returns `inf` rather than erroring on overflow.
fn json_strict_plain_scalar_ok(bytes: &[u8]) -> bool {
    if matches!(bytes, b"true" | b"false" | b"null") {
        return true;
    }
    match bytes.first() {
        Some(b'-' | b'0'..=b'9') => {}
        _ => return false,
    }
    if !bytes
        .iter()
        .all(|b| matches!(b, b'0'..=b'9' | b'.' | b'e' | b'E' | b'+' | b'-'))
    {
        return false;
    }
    let Ok(s) = core::str::from_utf8(bytes) else {
        return false;
    };
    // The finite-`f64`-parse primitive, not the core-schema dispatch around
    // it -- see `parse_float`'s doc comment for why only this much is
    // shared with `resolve_plain`.
    matches!(
        super::scalar::parse_float(s),
        super::scalar::ResolvedScalar::Float(_)
    )
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
    build_semi_index_impl(input, false)
}

/// [`build_semi_index`], enforcing JSON's stricter grammar (#2279, #2778)
/// -- for callers that already know the bytes are JSON and pair this with
/// [`YamlIndex::mark_json_sourced`](crate::yaml::YamlIndex::mark_json_sourced).
///
/// See `Parser::json_strict` for exactly what this does and does not
/// tighten (delimiters: sequences only, never mappings; scalar grammar:
/// values only, never keys, never anchor/tag-prefixed content).
///
/// # Errors
///
/// As [`build_semi_index`], plus the JSON delimiter and scalar-grammar
/// violations above, which that function accepts.
pub fn build_semi_index_json_strict(input: &[u8]) -> Result<SemiIndex, YamlError> {
    build_semi_index_impl(input, true)
}

fn build_semi_index_impl(input: &[u8], json_strict: bool) -> Result<SemiIndex, YamlError> {
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
        let mut p = Parser::<true>::new(input);
        p.json_strict = json_strict;
        p.parse()
    } else {
        let mut p = Parser::<false>::new(input);
        p.json_strict = json_strict;
        p.parse()
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

    /// #2778: `json_strict_plain_scalar_ok`'s accept/reject boundary, copied
    /// from the issue's pinned matrix (live against Homebrew `yq` v4.53.3 /
    /// `goccy/go-json` v0.10.6). Both columns matter equally -- the accepted
    /// non-RFC-8259 rows (`01`, `00`, `1.`, `-.5`, `1.e5`, `00e1`) are what
    /// makes this "not strict JSON", and every rejected row is a case where
    /// real yq's token scanner never reaches `ParseFloat` at all.
    #[test]
    fn test_json_strict_plain_scalar_ok_matrix() {
        const ACCEPT: &[&str] = &[
            "true",
            "false",
            "null",
            "0",
            "1",
            "01",
            "00",
            "1.",
            "0.",
            "00.5",
            "-01",
            "-.5",
            "-01.50e+01",
            "1.e5",
            "00e1",
            "1e-999",
            "-0",
        ];
        const REJECT: &[&str] = &[
            "a",
            "True",
            "TRUE",
            "Null",
            "NULL",
            "yes",
            "no",
            ".5",
            ".",
            ".e1",
            "+1",
            "1e",
            "0e",
            "1.e",
            "1.5e+",
            "1e+",
            "-",
            "-.",
            "--1",
            "-e1",
            "1-2",
            "1+2",
            "1..2",
            "1e1.5",
            "1e5e5",
            "1.2.3",
            "0x1A",
            "1_000",
            "NaN",
            "Infinity",
            "-Infinity",
            ".inf",
            "~",
            "nul",
            "tru",
            "",
            " ",
            "1 2",
        ];
        for s in ACCEPT {
            assert!(
                json_strict_plain_scalar_ok(s.as_bytes()),
                "{s:?} must be accepted (real yq accepts it)"
            );
        }
        for s in REJECT {
            assert!(
                !json_strict_plain_scalar_ok(s.as_bytes()),
                "{s:?} must be rejected (real yq rejects it)"
            );
        }
    }

    /// `1e999`/`2e308` overflow to `f64::INFINITY` in Rust rather than
    /// erroring the way Go's `strconv.ParseFloat` does with `ErrRange` --
    /// `is_finite()` is what turns that back into a rejection. A plain
    /// `.parse::<f64>().is_ok()` predicate would wrongly accept both.
    #[test]
    fn test_json_strict_plain_scalar_ok_rejects_overflow() {
        assert!(!json_strict_plain_scalar_ok(b"1e999"));
        assert!(!json_strict_plain_scalar_ok(b"2e308"));
        // Underflow to zero is a real, finite value in both Go and Rust --
        // accepted by real yq (`[1e-999] -> [0]`), not a rejection case.
        assert!(json_strict_plain_scalar_ok(b"1e-999"));
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

    /// The #106 guard for #1079's fast path: [`Parser::next_line_settles_value_is_null`]
    /// restates part of [`Parser::following_value_is_null`]'s answer with its own
    /// byte scan rather than calling it, for the measured performance reason on
    /// its own doc comment. Its doc comment argues the two cannot disagree, by
    /// inspection; this pins that claim the way `skip_line_break_agrees` pins
    /// `skip_line_break` against `break_len_at` -- whenever the fast path
    /// commits to an answer (`Some`), the full lookahead must agree. A `None`
    /// makes no claim and is not checked here; the caller falls back to the
    /// full lookahead in that case regardless.
    #[test]
    fn next_line_settles_value_is_null_agrees_with_following_value_is_null() {
        let cases: &[(&[u8], usize)] = &[
            (b"x", 1),        // no trailing break at all (last line, EOF)
            (b"x\n", 1),      // break then immediate EOF
            (b"x\n  y\n", 1), // deeper indent: value continues
            (b"x\n  y\n", 2), // same indent: value settles null
            (b"x\n  y\n", 3), // shallower-than-content indent
            (b"x\ny\n", 0),   // next line at indent 0
            (b"x\ny\n", 1),   // dedent past indent 1
            (b"x\n    deep\n", 0),
            (b"x\n    deep\n", 4),
            (b"x\n    deep\n", 5),
        ];
        for &(input, indent) in cases {
            // `self.pos` must sit on the current line's own break, per both
            // functions' contracts -- that's `x`'s length here in every case.
            let pos = input
                .iter()
                .position(|&b| is_line_break(b))
                .unwrap_or(input.len());

            let fast = {
                let mut p = Parser::<false>::new(input);
                p.pos = pos;
                p.next_line_settles_value_is_null(indent)
            };
            if let Some(fast) = fast {
                let mut full = Parser::<false>::new(input);
                full.pos = pos;
                let slow = full.following_value_is_null(indent);
                assert_eq!(
                    fast, slow,
                    "{input:?} @ pos {pos}, indent {indent}: fast path and full \
                     lookahead disagree"
                );
                assert_eq!(full.pos, pos, "following_value_is_null must restore pos");
            }
        }

        // The CRLF-carrying `HAS_CR == true` monomorphization has its own `\r`
        // arm in the fast path (`next_line_settles_value_is_null` steps over
        // `\r\n` as one break); pin it the same way.
        let crlf_cases: &[(&[u8], usize)] = &[
            (b"x\r\n  y\r\n", 1),
            (b"x\r\n  y\r\n", 2),
            (b"x\r\ny\r\n", 0),
        ];
        for &(input, indent) in crlf_cases {
            let pos = input
                .iter()
                .position(|&b| is_line_break(b))
                .unwrap_or(input.len());
            let fast = {
                let mut p = Parser::<true>::new(input);
                p.pos = pos;
                p.next_line_settles_value_is_null(indent)
            };
            if let Some(fast) = fast {
                let mut full = Parser::<true>::new(input);
                full.pos = pos;
                let slow = full.following_value_is_null(indent);
                assert_eq!(
                    fast, slow,
                    "{input:?} @ pos {pos}, indent {indent}: fast path and full \
                     lookahead disagree (CRLF)"
                );
            }
        }
    }
}
