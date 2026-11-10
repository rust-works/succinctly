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

use super::block_unicode;
use super::error::YamlError;
use super::line_break::{
    blank_line_between, crlf_comment_blank_between, is_line_break, line_break_len,
};
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

/// What a comment on the line of a flow collection's comma is, when it follows an element and
/// that line is not the element's own (#6334).
#[derive(Clone, Copy, PartialEq, Eq)]
enum CommaLine {
    /// The comma is on the element's line (or the comment is not on the comma's): a comment on
    /// that line is the element's line comment.
    Own,
    /// The comma starts a line and the element has a line comment already: yq reads the comment
    /// as the head of what follows.
    Head,
    /// Comments on lines of their own came before the comma: yq reads the comment as nothing.
    Drop,
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
    /// The column of the block a blank line just settled onto `PREV` as its foot, when this block is the
    /// next one of the same run (#5053): go-yaml ended the blocks at that column with the first, so this one
    /// is evaluated against the blocks still open outside it, whatever its own column (a block deeper than
    /// the first, which captures no frames of its own, takes them for this).
    follows_foot_at: Option<usize>,
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
    /// BP position of a property-bearing node → byte offset of its *first*
    /// `&anchor`/`!tag` token (#2712). See the `Parser` field of the same name.
    pub property_starts: BTreeMap<usize, usize>,
    /// Head/line/foot comments, keyed by the BP position of the node they
    /// attach to. See [`NodeComments`].
    pub comments: BTreeMap<usize, NodeComments>,
    /// Text offset of the `|`/`>` of every block scalar that holds NEL, LS or PS (#6369), sorted.
    /// Almost always empty. See the `Parser` field of the same name.
    pub unicode_block_scalars: Vec<usize>,
}

/// Maximum nesting depth for recursively-parsed constructs (flow collections
/// and inline `- - x` sequence-item chains). Bounds parser stack growth on
/// pathological input like `[`×20000, which otherwise aborts the process with
/// a stack overflow (#152). Real documents nest ~30-50 levels;
/// `tests/deep_nesting_valid_tests.rs` pins depth 100 as must-parse, so this
/// cap must stay above that.
const MAX_NESTING_DEPTH: usize = 128;

/// The markers `Parser::previous_line_key` walks past before the key a line opens with.
#[derive(Clone, Copy, PartialEq, Eq)]
enum LineMarkers {
    /// `- ` only: the markers a compact mapping inside sequence items stands behind.
    Dashes,
    /// `- `, and the `? ` and `: ` of an explicit key (#6543).
    Explicit,
}

/// The key a content line opens with, past its markers, and the last marker before it
/// (`Parser::previous_line_key`, #6540, #6543).
struct PreviousLineKey<'a> {
    column: usize,
    text: &'a [u8],
    /// The column of the last marker, and whether it is a dash.
    last_marker: Option<(usize, bool)>,
}

/// A plain scalar the scanner just read: `start..end` in the input, and the indent of the
/// innermost open block when it began (#5068).
#[derive(Clone, Copy)]
struct PlainScalarSpan {
    start: usize,
    end: usize,
    owner: usize,
}

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
    /// bp_pos of a property-bearing node → byte offset of its *first*
    /// `&anchor`/`!tag` token (#2712). yq's `line`/`column` report this
    /// position, not the value's, for a node that carries a property.
    /// Recorded under the same `bp_pos` convention as `bp_to_anchor` and
    /// `tags`, first property wins (`&x !!str 5` and `!!str &x 5` both name
    /// the first token) — see [`Self::note_property_start`].
    property_starts: BTreeMap<usize, usize>,
    /// Text offset of the `|`/`>` of every block scalar that holds NEL, LS or PS, or that one of them
    /// ends the dedent line of (#6369): go-yaml reads those scalars line by line with the three as
    /// breaks, which the decoder does only for a scalar listed here. In source order.
    unicode_block_scalars: Vec<usize>,
    /// Text offset of the header of the block scalar being parsed ([`Self::parse_block_scalar`]).
    block_header_pos: usize,
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

    /// Comments on lines of their own inside the flow collection being read, awaiting the element
    /// that follows them (its head) or the closing bracket (the last element's foot) (#6334).
    /// Every flow level drains it before it reads a nested collection, so one list serves them all.
    flow_own_comments: Vec<(u32, u32)>,

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

    /// Whether the current document's root node, a scalar or a flow collection, has been taken (#5070):
    /// nothing may follow it before the next `---` or `...`. Set when such a node is about to open at
    /// the document root, cleared by [`Self::start_document`].
    document_root_node_taken: bool,
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
    /// Where the plain scalar scanned last ended before its trailing blanks were trimmed (#6214).
    last_plain_content_end: usize,
    /// Whether the input holds a tab and a byte that starts U+2028, U+2029 or U+0085 (#6214), so a
    /// plain scalar is searched for one of them only in a document that can be refused for it: the
    /// refusal needs the tab. The tab test goes first, as a SIMD pass, and most inputs have none.
    may_hold_unicode_break: bool,

    /// Where the plain scalar scanned last began and ended, and the indent of the innermost open
    /// block when it did (#5068). The next block-context line takes it
    /// ([`Self::line_is_stray_after_a_plain_scalar`]): a line that stands after a comment that
    /// ended the scalar has nothing to attach to. Written once per plain scalar scan.
    last_plain_scalar: Option<PlainScalarSpan>,

    /// Where the plain scalar that is the whole content of the pending explicit key began (#6371):
    /// the scalar [`Self::last_plain_scalar`] holds, when `parse_explicit_key` has just read it.
    /// A line that stands after it once a comment ended it has nothing to attach to either.
    explicit_key_scalar_start: Option<usize>,

    /// Whether the pending explicit key's whole content is a node that ended on its own line, a
    /// quoted scalar or a flow collection (#6382). The line after it that is more indented than the
    /// `?` has nothing to attach to, comment or not.
    explicit_key_node_closed: bool,

    /// Whether the key named by [`Self::pending_explicit_key`] was a bare `?`
    /// with no content (`?\n- a\n:\n- c`), as opposed to a key that held a
    /// node of its own (`? k`). Only the latter rejects a same-indent `-`
    /// line (#1104): real yq never reads such a sequence as the key's
    /// implicit value, but the bare-`?` shapes (YAML test suite 6PBE) must
    /// keep parsing as they do today. Meaningful only while
    /// `pending_explicit_key` is `Some`; written only together with it, by
    /// [`Self::set_pending_explicit_key`].
    pending_explicit_key_empty: bool,

    /// The explicit keys still waiting for a value *behind* the one in
    /// [`Self::pending_explicit_key`], outermost first, as `(owner depth, empty)`
    /// pairs, the next one to be handed back last. A key whose own content is an explicit key (`? ? a\n  : b\n: c`, #6322;
    /// `? - ? a\n    : b\n: c`, whose first item opens it, #6340) opens a mapping that has a `?` of its own: the inner
    /// key's `:` has to find the inner owner, and the outer key's `:` has to find the outer one once
    /// the inner mapping has closed. [`Self::parse_nested_explicit_key`] files the outer one;
    /// `parse_explicit_key` then leaves the inner key as the pending one rather than writing its own
    /// owner over it. [`Self::clear_pending_explicit_key`] hands the next one back.
    pending_explicit_key_below: Vec<(usize, bool)>,

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

    /// Enforce real yq's own grammar for `-p json` input, on the "plain"
    /// route (#2279, #2778, #2777) -- not JSON's grammar, and not YAML's.
    ///
    /// Set only for input the caller already knows is JSON, i.e. exactly the
    /// callers that follow `YamlIndex::build` with
    /// [`mark_json_sourced`](crate::yaml::YamlIndex::mark_json_sourced).
    /// JSON is a subset of YAML's flow grammar, so the YAML parser accepts
    /// it as-is -- but real yq's own JSON front end (`goccy/go-json` v0.10.6
    /// feeding `CandidateNode.UnmarshalJSON`) is neither strict JSON nor
    /// YAML: it rejects things the YAML parser would otherwise accept
    /// (`[1,]`/`[,1]`/`[1,,2]` delimiters, #2279; YAML-only scalar
    /// spellings like `True`/`'a'`/`.5`/`+1`/`~` at any value position,
    /// #2778) while *also* accepting things neither grammar does inside a
    /// flow **mapping** specifically (#2777, below). `DocumentCursor::
    /// preceding_delimiter_ok`'s doc comment names the invariant the
    /// delimiter half restores: *every format but JSON validates delimiters
    /// while parsing*. JSON-sourced YAML was the one case that did neither.
    ///
    /// Deliberately a runtime flag rather than a third const generic: it
    /// would multiply the `HAS_CR` monomorphizations (#340) for a branch
    /// that is predictable and off the scalar-scanning hot path.
    ///
    /// **Flow sequences: strict delimiters, strict scalar grammar.** `[1,]`,
    /// `[,1]`, `[1,,2]` and a YAML-only scalar spelling all error --
    /// `parse_flow_sequence_inner` (#2279) and `check_json_strict_scalar`/
    /// `json_strict_plain_scalar_ok` (#2778, shared with every scalar
    /// *value* position, not just array elements).
    ///
    /// **Flow mappings: real yq's own token-pairing grammar, not stricter
    /// delimiters (#2777).** Real yq ignores punctuation inside `{}`
    /// entirely and pairs up tokens -- `{"a":1,}`, `{,}`, `{"a" 1}` all
    /// parse, and `{"a":1 "b":2}` (no separator between entries at all)
    /// does too. `parse_json_strict_flow_mapping_entries` reproduces this
    /// (see its own doc comment for the exact skip-before-key/skip-before-
    /// value rules and the deliberately-deferred "any bracket ends the
    /// loop" edge). A non-string key (`{1:1}`, `{true:1}`) would panic in
    /// the reference (Go interface-conversion panic, exit 2); per
    /// ADR-0018 rule 4(c) this refuses cleanly instead of reproducing the
    /// crash. Applies only to the plain (`YamlIndex`-backed) route -- the
    /// DOM-bridge route (`--inplace`/`--slurp`/`--eval-all`, `JsonIndex`-
    /// backed) still applies #1975's stricter grammar there, a known,
    /// separately-tracked gap (`docs/compliance/yq/limitations.md`).
    ///
    /// **Structural, key-agnostic rejections.** `?` (explicit key), `&`/`!`
    /// (anchor/tag), `*` (alias), and `'` (single quote) have no JSON
    /// spelling at all regardless of what follows them, so `parse_block_node`
    /// rejects the byte itself at one chokepoint rather than each dispatch
    /// arm needing its own check -- an anchor/tag-prefixed value is refused
    /// before the prefix's own target is even examined, not merely left
    /// unvalidated. One shape bypasses that chokepoint's own dispatch and
    /// carries its own identical gate instead: `--- |`/`--- >` (a
    /// block-scalar indicator right after a document marker,
    /// `parse_inline_document_value`'s dedicated fast path).
    ///
    /// A flow **mapping**'s own key/value positions (under `json_strict`)
    /// reject the same four bytes too, but via a different mechanism
    /// (#2777): `parse_json_strict_flow_mapping_entries` requires a key to
    /// start with `"` and a value to start with `{`/`[`/`"`/a
    /// `json_strict_token_end` literal, so `?`/`&`/`!`/`*`/`'` in either
    /// position simply never matches any of those shapes -- there is no
    /// dedicated gate for them there, only the absence of an arm that would
    /// accept them. (Before #2777, this position had *no* rejection at all
    /// for these bytes -- `{"a": &x 5}`/`{&x "a":1}` were silently accepted,
    /// a pre-existing gap #2777 closed as a side effect of requiring a
    /// specific key/value shape, not a deliberate new check.)
    json_strict: bool,
    /// Reads the stream as yq does, for the input `succinctly yq` reads (see
    /// [`YamlIndex::build_yq`](crate::yaml::YamlIndex::build_yq)); the library loader keeps the
    /// spec's reading. Three things differ, all from yq's header slurp, which sets aside every
    /// leading blank, comment, `%YAML` directive and `---` line before the first content line:
    ///
    /// - a run of leading `---` markers is one document (#6349): `---` / `---` / `x` is the one
    ///   document `x` where YAML reads a null document and then `x`;
    /// - a `...` met while that head is all the stream holds is an error (#6342): go-yaml is
    ///   handed the `...` as the first token of an implicit document, which has no node to hold;
    /// - a stream of only that head, directives included, is one null document (#6342).
    yq_mode: bool,
    /// Where the cursor stood when `parse_node_properties` last consumed a property, which is the next
    /// token's start (`usize::MAX` before one): a `-` met there has a property in front of it on its
    /// line, which yq refuses (#6414).
    properties_end: usize,
    /// Whether a directive other than `%YAML` was skipped (#6342): `%TAG` and the rest are not
    /// set aside by yq's slurp, so go-yaml sees them and wants a `---` after them.
    saw_non_header_directive: bool,
    /// How many documents `start_document` has started: the leading run only ever merges into the
    /// first (`a: 1` / `---` / `---` / `b: 2` is three documents in yq as in YAML).
    documents_started: usize,
    /// Where the text after the first document's latest leading `---` begins (#6349): the run only
    /// continues over a further marker when nothing but blank and comment lines lie between the two
    /// (`---` / `&a` / `---` / `x` is two documents: a property is content, though it writes no
    /// node).
    leading_run_from: usize,
    /// With `yq_mode`, the stream has no head at all (#5357): yq's slurp found nothing to set aside
    /// because the input began with a byte-order mark, so go-yaml reads every line, and no line is
    /// "more of the head" (`only_yq_header_lines_since` is never true). `---` / `---` / `x` is then
    /// a null document and `x`, as in the spec.
    yq_no_head: bool,
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
            property_starts: BTreeMap::new(),
            unicode_block_scalars: Vec::new(),
            block_header_pos: 0,
            pending_property_bp: None,
            comments: BTreeMap::new(),
            pending_head_comment: None,
            flow_own_comments: Vec::new(),
            pending_head_lines: Vec::new(),
            last_head_foot_bp: None,
            comment_watermark: 0,
            saw_head_foot_node: false,
            seen_any_document: false,
            document_root_node_taken: false,
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
            last_plain_content_end: 0,
            may_hold_unicode_break: crate::util::simd::escape::contains_byte(input, b'\t')
                && (crate::util::simd::escape::contains_byte(input, 0xC2)
                    || crate::util::simd::escape::contains_byte(input, 0xE2)),
            last_plain_scalar: None,
            explicit_key_scalar_start: None,
            explicit_key_node_closed: false,
            pending_explicit_key_empty: false,
            pending_explicit_key_below: Vec::new(),
            nesting_depth: 0,
            json_strict: false,
            yq_mode: false,
            properties_end: usize::MAX,
            saw_non_header_directive: false,
            documents_started: 0,
            leading_run_from: 0,
            yq_no_head: false,
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
        // Where the comment line recorded before this one ended, for `blank_line_precedes`.
        let previous_comment_end = self.comment_watermark as usize;
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
        //
        // Only when the pending block is not *deeper* than `PREV`'s own
        // block, though (`pending_block.column <= pending_block.prev_indent`,
        // the same signal the column-change split below gates on): a block
        // already sitting inside a not-yet-opened nested block (`a:` /
        // `  # h1` / blank / `  # h2` / `  b: 1`) is still working out which
        // forward node will own it, so an internal blank line does not yet
        // mean "settle onto `PREV`" -- real yq keeps `h1`/`h2` together as
        // one block (blank line preserved) attached to `.a.b`'s key, not
        // split with `h1` misrouted onto `.a`'s own foot (measured; #2795
        // review).
        let mut follows_foot_at = None;
        if let Some(&(_, last_end)) = self.pending_head_lines.last() {
            if self.blank_line_between(last_end as usize, start as usize)
                && self.pending_block.column <= self.pending_block.prev_indent
            {
                // Exactly one blank line: go-yaml keeps a run with two or more together (measured).
                // A CRLF ending a standalone comment line is a blank line of its own to go-yaml (#5053), but a
                // blank line the source already has does not make it a run of two: yq answers `# A` / blank /
                // `# B` the same in CRLF as in LF.
                let breaks = self.input[last_end as usize..start as usize]
                    .iter()
                    .filter(|&&b| b == b'\n')
                    .count();
                let blanks = if breaks >= 2 {
                    breaks
                } else {
                    breaks
                        + usize::from(crlf_comment_blank_between(
                            self.input,
                            last_end as usize,
                            start as usize,
                        ))
                };
                let one_blank = blanks == 2;
                // ...and the first block sits at the innermost open block's own column, which is what ends that
                // block for go-yaml (`unrollIndent`); one at an odd column leaves it open.
                if one_blank
                    && self.pending_block.column == self.pending_block.prev_indent
                    && self.attached_prev().is_some()
                {
                    follows_foot_at = Some(self.pending_block.column);
                }
                self.resolve_split_block(column);
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
                self.blank_line_precedes(start as usize, previous_comment_end)
            };
            self.capture_pending_block(column, after_split, blank_before, follows_foot_at);
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
    /// this widening changes nothing for them. A block that follows one a blank line settled
    /// (`follows_foot_at`, #5053) takes the frames at any column, for
    /// [`Self::outer_key_of_settled_foot`]; no other consumer reads a deeper block's frames.
    fn capture_pending_block(
        &mut self,
        column: usize,
        after_split: bool,
        blank_before: bool,
        follows_foot_at: Option<usize>,
    ) {
        let prev_indent = self.innermost_block_indent();
        self.pending_block.column = column;
        self.pending_block.prev_indent = prev_indent;
        self.pending_block.after_split = after_split;
        self.pending_block.blank_before = blank_before;
        self.pending_block.follows_foot_at = follows_foot_at;
        self.pending_block.frames.clear();
        if column > prev_indent && follows_foot_at.is_none() {
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
                // The node opening now is deeper than the block `PREV` is in, so it is `PREV`'s
                // first child: the comment above it, whatever its own column, is that child's head
                // across the blank line, not `PREV`'s foot (`m:` / `  # a` / blank / `  k: 1` is
                // `.m.k`'s key's head, measured against yq v4.53.3, #4099).
                let heads_first_child =
                    next.is_some_and(|(_, next_column)| next_column > prev_indent);
                // A key with no value yet (`b:` / `# c` / blank / `h: 1`): the comment is read while
                // go-yaml looks for the value, so it is the next sibling's head, not `b`'s foot (#5053).
                let heads_next_sibling = next
                    .is_some_and(|(_, next_column)| next_column == prev_indent)
                    && self.follows_valueless_key();
                if !heads_first_child
                    && !heads_next_sibling
                    && !self.first_node_takes_no_foot_quirk(prev, prev_indent, next.is_some())
                {
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
            } else if let Some((_, next_column)) = next {
                // A blank line after the run's second block changes nothing: it is still the foot of the key
                // that owns the blocks the first one closed (#5053).
                if self.defers_foot_to_next_item(next_column) {
                    self.defer_pending_foot_to_next_item();
                    return;
                }
                if let Some(owner) = self.outer_key_of_settled_foot(next_column) {
                    self.drain_pending_into_owner_foot(owner);
                    return;
                }
                if self.detached_block_to_prev_before_dedent(next_column) {
                    return;
                }
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
                } else if self.defers_foot_to_next_item(next_column) {
                    self.defer_pending_foot_to_next_item();
                } else if let Some(owner) = self.outer_key_of_settled_foot(next_column) {
                    // The run's first block already closed the blocks at its column as `PREV`'s foot, so this
                    // one is the foot of the key that owns them (#5053).
                    self.drain_pending_into_owner_foot(owner);
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
            None if self.saw_head_foot_node => {
                self.mark_blank_runs_in_pending();
                self.drain_pending_into_foot(root);
            }
            None => {
                let pending = &mut self.pending_head_lines;
                self.comments.entry(root).or_default().head.append(pending);
            }
        }
    }

    /// Place the pending blocks at a `---`/`...` boundary (a dedent to column 0): a second block of a run after
    /// a blank line is the foot of the key that owns the blocks the first closed, as in a mapping
    /// (`outer_key_of_settled_foot`, #5053); otherwise the foot of the node the column names, else the closing
    /// document's root (#2811), whose blocks keep the blank line between them (#5052).
    fn drain_pending_at_boundary(&mut self, old_root: usize) {
        let owner = self.outer_key_of_settled_foot(0);
        let target = owner
            .or_else(|| self.positional_boundary_foot_target())
            .unwrap_or(old_root);
        if owner.is_some() || target == old_root {
            self.mark_blank_runs_in_pending();
        }
        self.drain_pending_into_foot(target);
    }

    /// Drain the pending blocks into the foot of the key that owns the blocks a run's first block closed
    /// (#5053), keeping the blank line between them as a document's foot does (#5052).
    fn drain_pending_into_owner_foot(&mut self, owner: usize) {
        self.mark_blank_runs_in_pending();
        self.drain_pending_into_foot(owner);
    }

    /// The key whose value block the run's first block closed, for the block after it (#5053): the last key of
    /// the open mapping the following node (at `next_column`) belongs to, when it dedents past the closed column. `a:` /
    /// `  b: 1` / `  # A` / blank / `  # B` / `c: 2` is `# A` on `.a.b`'s key and `# B` on `.a`'s.
    fn outer_key_of_settled_foot(&self, next_column: usize) -> Option<usize> {
        let closed_at = self.pending_block.follows_foot_at?;
        // The node after this block dedents past the column the first one closed, and this block is not at or
        // left of that node's own column (a block there is placed by that column, not moved).
        if next_column >= closed_at || self.pending_block.column <= next_column {
            return None;
        }
        // A block left of the closed column, at the column of a block still open, is placed by that column:
        // the foot of that block's last key (`x:` / `  a:` / `    b: 1` / `    # A` / blank / `  # B` /
        // `c: 2` is `# B` at column 2 on `.x.a`'s key).
        let column = self.pending_block.column;
        if column < closed_at {
            if let Some(frame) = self
                .pending_block
                .frames
                .iter()
                .find(|frame| frame.indent == column)
            {
                return frame.is_mapping.then_some(frame.last_key_bp).flatten();
            }
        }
        // The mapping the following node belongs to: the key it follows owns the blocks that closed.
        self.pending_block
            .frames
            .iter()
            .find(|frame| frame.indent == next_column)
            .and_then(|frame| frame.is_mapping.then_some(frame.last_key_bp).flatten())
    }

    /// Whether the pending block is the second of a run that a blank line split, and the node opening now is
    /// an item of a sequence that the block is deeper than: yq makes it the foot of that item's first key, not of
    /// an outer key (#5053); a block at or left of the item's column is placed by that column.
    /// GitHub's default CodeQL workflow has `languages:` / block / blank / block / blank / `- name:`.
    fn defers_foot_to_next_item(&self, next_column: usize) -> bool {
        // Like `outer_key_of_settled_foot`: the node must dedent past the column the first block closed.
        self.pending_block
            .follows_foot_at
            .is_some_and(|closed_at| next_column < closed_at)
            && self.current_type == Some(NodeType::SequenceItem)
            && self.pending_block.column > next_column
            // yq drops the block before a flow collection item, which succinctly does not reproduce.
            && !self.input[self.pos.min(self.input.len())..]
                .iter()
                .find(|&&b| !matches!(b, b' ' | b'\t'))
                .is_some_and(|&b| matches!(b, b'[' | b'{'))
    }

    /// Hand the pending block to the first key of the item opening now, keeping the blank lines between its
    /// lines (see [`Self::defers_foot_to_next_item`]).
    fn defer_pending_foot_to_next_item(&mut self) {
        self.mark_blank_runs_in_pending();
        let pending = &mut self.pending_head_lines;
        self.deferred_foot_lines.append(pending);
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
            let claimed =
                Self::last_mapping_key(frames[..=start].iter().rev().take_while(|f| closing(f)));
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
        Self::last_mapping_key(frames[1..=start].iter().rev()).or_else(|| self.attached_prev())
    }

    /// The last mapping frame's key bp among `frames`, filtering the
    /// `open_frame_key_slot` sentinel (#2811 review) -- shared by
    /// [`Self::positional_foot_target`] and
    /// [`Self::positional_boundary_foot_target`], whose own backward scans
    /// differ only in which frames are in range to begin with (one takes
    /// `closing` into account and starts at index 0, the other doesn't and
    /// starts past index 0) -- everything after that range is picked is this
    /// one rule in both. A mapping frame with no key yet records the
    /// `open_frame_key_slot` sentinel rather than `None` (`last_key_bp` is
    /// only ever cleared to `None` for a non-mapping frame); filtering it out
    /// here keeps a mapping that opened without ever completing a key from
    /// handing a dedented comment to a bp no cursor resolves to.
    fn last_mapping_key<'f>(mut frames: impl Iterator<Item = &'f CommentFrame>) -> Option<usize> {
        frames
            .find(|frame| frame.is_mapping)
            .and_then(|frame| frame.last_key_bp)
            .filter(|&bp| bp != usize::MAX)
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

    /// Put one empty line between the pending comment lines wherever a blank line (or a run of them)
    /// separates them in the input (#5052): a document's foot keeps that one blank line between its
    /// blocks in yq (`a: 1` / blank / `# A` / blank / `# B` answers `. | foot_comment` as `"A\n\nB"`
    /// and prints the blank back). The empty line is an empty range at the next line's start, which
    /// the emitters write as a bare line break.
    fn mark_blank_runs_in_pending(&mut self) {
        let lines = &self.pending_head_lines;
        if lines.len() < 2 {
            return;
        }
        let mut marked = Vec::with_capacity(lines.len() * 2);
        marked.push(lines[0]);
        for pair in lines.windows(2) {
            let (end, start) = (pair[0].1, pair[1].0);
            if self.blank_line_between(end as usize, start as usize) {
                marked.push((start, start));
            }
            marked.push(pair[1]);
        }
        self.pending_head_lines = marked;
    }

    /// A pending block with a blank line above and below it, before a node that opens at `next_column`
    /// (#5056): when that node dedents out of the block `PREV` is in and the block is not at its column, it
    /// is `PREV`'s foot in yq -- #2811's rule for an adjacent block, which the blank lines around it keep
    /// from being asked (`b:` / `  s:` / `  - n: a` / `    u: x` / blank / `  # C` / blank / `d: 1` is
    /// `u`'s foot, measured). A document marker is a dedent to column 0 (#5053). Its blocks keep one blank
    /// line between them, as a document's foot does (#5052). Not when `PREV` already has a foot: a second
    /// block after it is placed by yq's own sequence-item quirks, which this does not model. Returns
    /// whether the block was placed.
    fn detached_block_to_prev_before_dedent(&mut self, next_column: usize) -> bool {
        let Some(prev) = self.last_head_foot_bp else {
            return false;
        };
        let prev_has_foot = self.comments.get(&prev).is_some_and(|c| !c.foot.is_empty());
        if next_column < self.pending_block.prev_indent
            && self.pending_block.column != next_column
            && !prev_has_foot
        {
            self.mark_blank_runs_in_pending();
            self.drain_pending_into_foot(prev);
            return true;
        }
        false
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
            return; // patchcov: coverage tolerate-line reason="unreachable: every block-sequence open registers a frame at its own depth before any item of it can be parsed (#1079)"
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

    /// [`Self::float_absent_item_comment`] for a bare `- # c` item whose value is node
    /// properties alone (`  !!str` on the next line, #2835). The item is an empty tagged or
    /// anchored node rather than an absent one, so real yq does not remember the comment as
    /// floated off an item of this sequence: it goes forward with the pending block and heads
    /// whatever opens next, even past the end of the sequence (`k:` / `  - # c` / `    !!str` /
    /// `z: 1` heads `z`, where an absent item makes it the foot of `k`).
    fn float_comment_past_empty_property_item(&mut self, range: (u32, u32)) {
        if let Some(previous) = self.floated_item_comment.take() {
            self.settle_float_if_sequence_closed(previous, None);
        }
        let idx = self
            .pending_head_lines
            .partition_point(|&(start, _)| start < range.0);
        self.pending_head_lines.insert(idx, range);
        self.block_passed_absent_item = true;
    }

    /// [`Self::resolve_pending_block_backward`] for a block that a blank line split off a later one of its run
    /// (#5053). A block dedented to the column of a block that is still open (`x:` / `  k: 1` / `# A` / blank /
    /// `# B`, or `steps:` / `  - run: x` / `  # A` / blank / `  # B`) is the foot of the last key of the nearest
    /// enclosing mapping, as go-yaml's `unrollIndent` places it when the node that follows is not yet known: a
    /// block sequence hands the block on to the mapping that holds it. It is not `PREV`, the innermost key, which
    /// would print it at `PREV`'s own column. That applies when the next block of the run is at that column too (a
    /// block after it at another column leaves the choice to the node that follows, which is not known yet). Any
    /// other block settles on `PREV`.
    fn resolve_split_block(&mut self, next_column: usize) {
        if self.pending_block.column < self.pending_block.prev_indent
            && next_column == self.pending_block.column
            && self.attached_prev().is_some()
        {
            let frames = &self.pending_block.frames;
            let target = frames
                .iter()
                .rposition(|frame| frame.indent == self.pending_block.column)
                .and_then(|at| Self::last_mapping_key(frames[..=at].iter().rev()));
            if let Some(target) = target {
                self.drain_pending_into_foot(target);
                return;
            }
        }
        self.resolve_pending_block_backward();
    }

    /// Whether the line just above the pending block is a mapping key that has no value on its line: `b:`,
    /// `b: &x` or `b: !!str` (#5053). Its value, if any, comes on a later line, so a comment directly below
    /// it is read as part of looking for that value.
    fn follows_valueless_key(&self) -> bool {
        let text = self.input;
        let mut line_start = self
            .pending_head_lines
            .first()
            .map_or(0, |&(first, _)| first) as usize;
        while line_start > 0 && !is_line_break(text[line_start - 1]) {
            line_start -= 1;
        }
        // The line above, without its break.
        let above = &text[..line_start];
        let above = above
            .strip_suffix(b"\r\n")
            .or_else(|| above.strip_suffix(b"\n"))
            .or_else(|| above.strip_suffix(b"\r"))
            .unwrap_or(above);
        let line = above.rsplit(|&b| is_line_break(b)).next().unwrap_or(&[]);
        let line = String::from_utf8_lossy(line);
        // The last token that is not a property or a trailing comment must end the key.
        let mut last = None;
        for token in line.split_ascii_whitespace() {
            if token.starts_with('#') {
                break;
            }
            if !(token.starts_with('&') || token.starts_with('!')) {
                last = Some(token);
            }
        }
        last.is_some_and(|token| token.ends_with(':'))
    }

    /// Settle a pending block whose forward attachment is already disproven
    /// (a blank line follows it) onto `PREV`'s foot -- or, with no `PREV` in
    /// the *current* document yet, onto the document that just closed
    /// across a `---`/`...` boundary (mirrors
    /// [`Self::flush_pending_head_lines_with_root`]'s own no-`PREV`
    /// fallback, #2795 review): `a: 1\n---\n# c1\n\n# c2\nb: 2\n` gives `c1`
    /// to the first document's foot and only `c2` goes forward to `.b`'s
    /// head, not both merged onto `.b` (measured against pinned yq). A
    /// genuine no-op only when neither is set, which means a blank line
    /// detached the block backwards *and* this is still the very first
    /// document -- it stays pending and goes forward to whatever opens
    /// next.
    fn resolve_pending_block_backward(&mut self) {
        if self.pending_head_lines.is_empty() {
            return; // patchcov: coverage tolerate-line reason="unreachable: this function's sole caller (record_standalone_comment) only invokes it from inside a match on `pending_head_lines.last()`, so pending_head_lines is already known non-empty here (#798)"
        }
        if let Some(prev) = self.attached_prev() {
            self.drain_pending_into_foot(prev);
        } else if let Some(old_root) = self.document_boundary_fallback_bp {
            self.drain_pending_into_foot(old_root);
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
    /// A comment floated off an earlier absent bare item of the sequence this item belongs
    /// to (#1079's [`Self::floated_item_comment`]) is taken here: the sequence is still open,
    /// so the item is its sibling and the block heads it like any adjacent block (#2835). A
    /// no-op when the float's sequence has since closed: this hook's own bp is a stem, not a
    /// genuine resolved node, so it must not pre-empt the float's settle logic the way a real
    /// [`Self::attach_head_foot_at`]/[`Self::attach_head_foot_to_key`] call would (both set
    /// `is_item` from `self.current_type`, which reads identically true for a stem as for a
    /// genuine item content, so the float would otherwise wrongly settle onto this stem's own
    /// bp instead of the sequence's owner key -- confirmed live: `k:\n  - - # c1\n  - - #
    /// c2\n` regresses `.k`'s foot from `c1` to empty without the guard). Deferring there
    /// leaves the float exactly where [`Self::float_absent_item_comment`]'s own "settle the
    /// previous float" step (or, at end of document, [`Self::flush_pending_head_lines`]'s own
    /// check) already resolves it correctly once the sequence this item recurses into
    /// registers its own frame and reaches a genuine content node.
    fn attach_item_head_at(&mut self, bp: usize, column: usize) {
        if let Some(floated) = self.floated_item_comment {
            // A float from an earlier item of the sequence this item belongs to, still open,
            // is the head of this item when it is a compact mapping or nested sequence, as
            // it is for any other item that opens next: `- # c` / `- b: 1` heads `.[1]`
            // (#2835). One from a sequence that has since closed is a foot that
            // `settle_float_if_sequence_closed` places, so this hook stands aside for it.
            if !self.sequence_still_open(floated.seq_serial) {
                return;
            }
            self.floated_item_comment = None;
            self.block_passed_absent_item = false;
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
            self.document_root_node_taken = true;
            self.attach_head_foot_at(self.bp_pos, column);
        } else if self.current_type == Some(NodeType::SequenceItem) {
            // A bare `-` item's scalar value on its own line: the node a
            // foot deferred to "the item's first key/scalar" was waiting
            // for (#2811; the item's own hook left it unclaimed).
            //
            // Comment lines between the dash and the value are this node's head (`-` / `  # f` /
            // `  z`, #4268): the bare-item arm left them pending for "the item's first node",
            // which for a mapping or a sequence is its own first key or item, and for a scalar
            // is the scalar itself, where they used to ride on to the next item.
            self.flush_pending_head_lines(Some((self.bp_pos, column)));
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
    /// A block detached from `PREV` too (a blank line on each side) is the closing document's foot as well,
    /// placed like one flush against the marker (`a: 1\n\n# A\n\n---\nb: 2\n`, #5053), when the closing document
    /// opened a node and the one after the marker has content ([`Self::next_document_has_content`]). Otherwise,
    /// and with no closing document, it takes the ordinary path (delegated to [`Self::flush_pending_head_lines`]).
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
                self.drain_pending_at_boundary(old_root);
                return;
            }
            if self.attached_prev().is_some() {
                self.flush_pending_head_lines_with_root(None, old_root);
                return;
            }
            // Detached from `PREV` by a blank line as well (#5053): yq keeps the block as the closing
            // document's foot, placed as one flush against the marker is, not as the next document's head.
            // Only when the closing document opened a node (`last_head_foot_bp`): a comment-only document
            // keeps the ordinary path, which reaches back across the marker in its own way (#798).
            if self.last_head_foot_bp.is_some() && self.next_document_has_content() {
                if self.detached_block_to_prev_before_dedent(0) {
                    return;
                }
                self.drain_pending_at_boundary(old_root);
                return;
            }
        }
        self.flush_pending_head_lines(next);
    }

    /// Whether the document opening at `self.pos` has a content line: past blank lines and comment-only
    /// lines, something other than the next `---`/`...` marker or the end of input (#5053). yq places a
    /// block that reaches back across a marker differently when the document after it is comment-only.
    fn next_document_has_content(&self) -> bool {
        let mut pos = self.pos;
        while pos < self.input.len() {
            let rest = &self.input[pos..];
            let indent = rest
                .iter()
                .take_while(|&&b| Self::is_inline_whitespace(b))
                .count();
            match rest.get(indent) {
                None => return false,
                Some(&b) if is_line_break(b) => {}
                Some(b'#') => {}
                Some(_) => {
                    return indent != 0
                        || !(Self::marker_prefix(rest, b"---")
                            || Self::marker_prefix(rest, b"..."));
                }
            }
            pos += rest
                .iter()
                .position(|&b| is_line_break(b))
                .map_or(rest.len(), |at| at + line_break_len(rest, at));
        }
        false
    }

    /// Whether a wholly blank line (empty or inline whitespace only) lies in `[from, to)`:
    /// [`blank_line_between`] over the input, shared with the cursor side in `light.rs`.
    fn blank_line_between(&self, from: usize, to: usize) -> bool {
        blank_line_between(self.input, from, to)
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
    fn blank_line_precedes(&self, pos: usize, previous_comment_end: usize) -> bool {
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
        // A CRLF after a standalone comment line is a blank line of its own to go-yaml (#5053). The line
        // must be a comment the parser recorded: a `#` line inside a block scalar is its content.
        if q == previous_comment_end {
            let prev_line_start = self.input[..q]
                .iter()
                .rposition(|&b| is_line_break(b))
                .map_or(0, |i| i + 1);
            if crlf_comment_blank_between(self.input, prev_line_start, p) {
                return true;
            }
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

    /// Write the node of a value that is empty (null), anchored at `at`: the end of its key's `:`
    /// (#6386). yq places an empty value there (`.a | line` of `a:` is the key's line), and it must be
    /// a byte no node decodes from: the reader reads a node by its first byte, so a null written at
    /// the start of the *next* line, where the lookahead leaves the cursor, read a quoted or flow key
    /// there as the value (`a:` / `"b": 1` was `{"a": "b"}`).
    fn write_null_value_at(&mut self, at: usize) {
        self.set_ib_at(at);
        self.write_bp_open_at(at);
        self.write_bp_close();
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

    /// Record that the mapping at `owner_depth` has an explicit key still
    /// waiting for its value, and whether that key was a bare `?`
    /// ([`Self::pending_explicit_key_empty`]). The one place that field pair
    /// is written, so a new site cannot set the depth and inherit a stale
    /// emptiness (#1104).
    #[inline]
    fn set_pending_explicit_key(&mut self, owner_depth: usize, empty: bool) {
        // A key still waiting in a mapping *below* this one is an outer key whose own content is
        // this mapping (`? ? ? a⏎  ? y⏎: c`, #6322): it waits behind the new one.
        if let Some(waiting) = self.pending_explicit_key {
            if waiting < owner_depth {
                self.wait_behind_pending_explicit_key(waiting, self.pending_explicit_key_empty);
            }
        }
        self.pending_explicit_key = Some(owner_depth);
        self.pending_explicit_key_empty = empty;
    }

    /// Records the explicit key of the mapping at `owner_depth` as waiting behind the pending one
    /// ([`Self::pending_explicit_key_below`]). The one place that list is added to: it is kept in
    /// order of owner depth, so the last entry is the nearest outer key whichever order the keys
    /// are found in (a nested key returns after the keys inside it, a key added to a mapping
    /// above one that waits is found before it).
    fn wait_behind_pending_explicit_key(&mut self, owner_depth: usize, empty: bool) {
        let at = self
            .pending_explicit_key_below
            .partition_point(|&(depth, _)| depth < owner_depth);
        self.pending_explicit_key_below
            .insert(at, (owner_depth, empty));
    }

    /// The pending explicit key has its value (or its null): the one behind it, if a nested
    /// key left one ([`Self::pending_explicit_key_below`]), is the one waiting now.
    #[inline]
    fn clear_pending_explicit_key(&mut self) {
        match self.pending_explicit_key_below.pop() {
            Some((owner_depth, empty)) => self.set_pending_explicit_key(owner_depth, empty),
            None => self.pending_explicit_key = None,
        }
    }

    /// Whether a `-` line at `indent` would land directly after an explicit
    /// key that held a node of its own (`? k`, `? a: b`, `? - a`, `? [a]`) in
    /// the mapping at that indent, with the key's `:` value not yet seen.
    ///
    /// Real yq rejects it ("did not find expected key"): an explicit key's
    /// value must be introduced by `:`, unlike an ordinary `key:` entry,
    /// which accepts a same-indent sequence as its implicit value (#1104).
    /// The owner is found by depth rather than as the top of the stack: a key
    /// that opens a container of its own (a compact mapping, a block
    /// sequence) leaves that frame above the owner until the `-` line closes
    /// it. A bare `?` is excluded -- its same-indent sequence is the key's
    /// own content (YAML test suite 6PBE) and must keep parsing.
    fn explicit_key_with_content_rejects_sequence_at(&self, indent: usize) -> bool {
        let Some(owner_depth) = self.pending_explicit_key else {
            return false;
        };
        !self.pending_explicit_key_empty
            && owner_depth > 0
            && self.indent_stack.get(owner_depth - 1) == Some(&indent)
            && self.type_stack.get(owner_depth - 1) == Some(&NodeType::Mapping)
            // Not the next item of a key sequence that sits at this very column (`?⏎- a⏎- b`,
            // #6370).
            && !self.key_sequence_open_at(indent)
    }

    /// The `:` of an explicit value ends the key's own sequence at the `?`'s column.
    fn close_key_sequence_at(&mut self, indent: usize) {
        while self.key_sequence_open_at(indent)
            && self.current_type == Some(NodeType::Sequence)
            && self.indent_stack.last() == Some(&indent)
        {
            self.indent_stack.pop();
            self.pop_type();
            self.write_bp_close();
        }
    }

    /// Whether a block sequence opened as the pending explicit key's content is still open at
    /// `indent`, above the mapping that owns the key (`?⏎- a⏎- b`, #6370).
    fn key_sequence_open_at(&self, indent: usize) -> bool {
        match self.pending_explicit_key {
            Some(owner_depth) => {
                self.indent_stack.len() > owner_depth
                    && self.indent_stack[owner_depth..]
                        .iter()
                        .zip(&self.type_stack[owner_depth..])
                        .any(|(&i, &t)| i == indent && t == NodeType::Sequence)
            }
            None => false,
        }
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
            self.clear_pending_explicit_key();
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
        self.line_at(self.pos)
    }

    /// The 1-indexed line `offset` is on.
    fn line_at(&self, offset: usize) -> usize {
        // Count line breaks from start to `offset`, by counting the
        // bytes each break *ends* at: a break of width 1 ends where it starts,
        // so a CRLF — width 2 at its `\r` — is counted once, at its `\n` (#324).
        //
        // `line_break_len` reads `self.input`, not the truncated prefix: when
        // the cursor sits on the LF of a CRLF, that CR's partner is one byte
        // past the prefix, and measuring within the prefix would read it as a
        // lone CR and report a line too many.
        let breaks = (0..offset)
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

    /// Width of the Unicode line break at `pos` (NEL is two bytes, LS and PS three), or 0. go-yaml
    /// ends a line at these as at `\n`; the index keeps them as scalar content (#4913).
    #[inline]
    fn unicode_break_len_at(&self, pos: usize) -> usize {
        super::block_unicode::unicode_break_len(self.input, pos)
    }

    /// A Unicode break inside a flow plain scalar (#6214): the tab rule of #6059 reads the lines after
    /// it, then the break is passed as the content it stays for the fold (#4913). One definition for
    /// the three flow scanners.
    fn pass_unicode_break_in_flow_plain(&mut self) -> Result<(), YamlError> {
        self.reject_tab_in_flow_plain_continuation(self.pos)?;
        self.advance_by(self.unicode_break_len_at(self.pos));
        Ok(())
    }

    /// Width of the line break at `pos`, ASCII or Unicode, or 0 (#6214).
    #[inline]
    fn any_break_len_at(&self, pos: usize) -> usize {
        match self.input.get(pos) {
            Some(b'\n' | b'\r') => self.break_len_at(pos).max(1),
            _ => self.unicode_break_len_at(pos),
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

    /// The number of spaces before the tab of the line at `line_start` when that line is spaces, a tab
    /// and nothing else but blanks (#6286, #6295), `None` for any other line.
    fn tab_only_line_indent(&self, line_start: usize) -> Option<usize> {
        let (indent, at) = self.tab_led_blank_prefix(line_start)?;
        (at >= self.input.len() || is_line_break(self.input[at])).then_some(indent)
    }

    /// [`Self::tab_only_line_indent`] for a line that goes on to a `#` comment after its blanks (#6342):
    /// in the head of the first document that is a comment line to yq's slurp, as a space-led one is.
    fn tab_comment_line_indent(&self, line_start: usize) -> Option<usize> {
        let (indent, at) = self.tab_led_blank_prefix(line_start)?;
        (self.input.get(at) == Some(&b'#')).then_some(indent)
    }

    /// The blanks a line opens with when they are spaces and then a tab (and any blanks after it):
    /// the number of spaces, and where the blanks end. `None` for a line with no tab among them.
    fn tab_led_blank_prefix(&self, line_start: usize) -> Option<(usize, usize)> {
        let mut at = line_start;
        while self.input.get(at) == Some(&b' ') {
            at += 1;
        }
        let indent = at - line_start;
        if self.input.get(at) != Some(&b'\t') {
            return None;
        }
        while matches!(self.input.get(at), Some(b' ' | b'\t')) {
            at += 1;
        }
        Some((indent, at))
    }

    /// Whether the last plain scalar ends at or before `line_start` with only blank lines between.
    fn plain_scalar_directly_before(&self, line_start: usize) -> bool {
        self.last_plain_scalar.is_some_and(|span| {
            span.end <= line_start
                && self.input[span.end..line_start]
                    .iter()
                    .all(|b| matches!(b, b' ' | b'\t' | b'\n' | b'\r'))
        })
    }

    /// Whether go-yaml refuses the line at `line_start`, which is spaces, a tab and nothing else, where this
    /// parser would read an empty line (#6286, #6295). The one decision for every site (`skip_newlines`,
    /// `parse_document_line`, [`Self::reject_tab_only_line_at_cursor`]), with one scan of the line.
    ///
    /// go-yaml's `scan_to_next_token` skips a tab only where a simple key is not allowed, which is never
    /// at the start of a block-context line, so it stops with `found a tab character that violates
    /// indentation` at every column. The lines it does not reach are the ones a scalar scanner takes:
    ///
    /// - after a plain scalar the scanner consumes the blank lines itself, and refuses the tab only at a
    ///   column no deeper than the block that owns the scalar (#6286);
    /// - a block scalar's content takes it as text (it never reaches `skip_newlines`);
    /// - in the head of the first document yq's header slurp takes the line away, so nothing before the
    ///   first node of the first document refuses it. After anything else (a flow collection, a quoted
    ///   scalar, a comment, an empty value, a property, a `---` that is not the first) it is refused.
    fn tab_only_line_is_refused(&self, line_start: usize) -> bool {
        let Some(indent) = self.tab_only_line_indent(line_start) else {
            return false;
        };
        match self.last_plain_scalar {
            Some(span) if self.plain_scalar_directly_before(line_start) => indent <= span.owner,
            _ => self.documents_started > 1 || self.bp_pos > self.document_start_bp_pos,
        }
    }

    /// [`Self::tab_only_line_is_refused`] for the line the cursor stopped at, for the sites that look
    /// past an empty value or key and would otherwise take the line as its blank prefix (#6295: `a:` /
    /// ` ⇥` is refused by yq, and was read as `{"a": null}`; `?` / ` ⇥` / `  a` as `{"a": null}` too).
    fn reject_tab_only_line_at_cursor(&self) -> Result<(), YamlError> {
        let at_line_start = self.pos == 0 || Self::is_break(self.input[self.pos - 1]);
        if at_line_start && self.tab_only_line_is_refused(self.pos) {
            return Err(YamlError::TabIndentation {
                line: self.current_line(),
                offset: self.pos,
            });
        }
        Ok(())
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
    /// Whether the cursor is at a `:` that is a value indicator (a blank, a break or the end follows
    /// it). With no `?` or key before it on its line it is the keyless entry yq refuses with
    /// `did not find expected key` (#6288, #6325).
    #[inline]
    fn at_bare_colon(&self) -> bool {
        self.peek() == Some(b':') && Self::is_ws_break_or_eoi(self.peek_at(1))
    }

    /// Whether a run of block sequence indicators opens at the cursor and ends at a bare `:`
    /// (`- - : x`), without moving it (#6325).
    fn dashes_end_in_bare_colon(&self) -> bool {
        let mut at = self.pos;
        loop {
            match self.input.get(at) {
                Some(b'-') if Self::is_ws_break_or_eoi(self.input.get(at + 1).copied()) => {
                    at += 1;
                    while matches!(self.input.get(at), Some(b' ' | b'\t')) {
                        at += 1;
                    }
                }
                Some(b':') => {
                    return Self::is_ws_break_or_eoi(self.input.get(at + 1).copied());
                }
                _ => return false,
            }
        }
    }

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
    /// `#[inline]`: dispatched from a dozen call sites, so leaving this
    /// out-of-line cost a real function call on every plain-scalar value
    /// under `json_strict` even on the (common) accept path.
    #[inline]
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

    /// Whether the flow collection at the cursor is an implicit mapping key:
    /// `[a, b]: v` / `{a: 1}: v` (#1188). The collection must close on the
    /// same line (YAML limits an implicit key to one) and be followed, after
    /// optional blanks, by a `:` that is itself followed by whitespace, a
    /// break or the end of input -- the same disambiguation
    /// [`Self::looks_like_mapping_entry`] applies to a scalar key, which
    /// deliberately answers `false` for `[`/`{` and is left that way.
    ///
    /// It is a second, line-bounded lexer for the quoting, nesting and comment
    /// rules `parse_flow_sequence` / `parse_flow_mapping` apply; a change to
    /// those rules has to be mirrored here, or the entry is routed wrongly.
    ///
    /// Answers `false` for anything it cannot place: under `json_strict`
    /// (`{"a": 1}: x` is two JSON values there, not a mapping), when the
    /// collection runs onto another line or past a comment, and when a quote
    /// inside it never closes. `false` is always the safe direction -- it
    /// keeps the previous behavior for that input.
    fn looks_like_flow_keyed_mapping_entry(&self) -> bool {
        if self.json_strict || !matches!(self.peek(), Some(b'[' | b'{')) {
            return false;
        }
        let input = self.input;
        let mut i = self.pos;
        let mut depth = 0usize;
        // The last non-blank byte seen: a quote only opens a string at the
        // start of a token (`[it's, b]` has no string in it).
        let mut prev = 0u8;
        // Whether blanks sit between `prev` and the byte under the cursor: a
        // `#` only starts a comment after one.
        let mut blank_before = false;
        // The first byte of the word before the current one, so that a quote
        // after a property (`[!!str "a]", b]`, `[&x "a]"]`) still opens a string.
        let mut word_first = 0u8;
        let mut prev_word_first = 0u8;
        while i < input.len() {
            let b = input[i];
            if Self::is_inline_whitespace(b) {
                blank_before = true;
                i += 1;
                continue;
            }
            if blank_before || prev == 0 || matches!(prev, b'[' | b'{' | b',' | b':' | b'?') {
                prev_word_first = word_first;
                word_first = b;
            }
            let after_property = blank_before && matches!(prev_word_first, b'&' | b'!');
            match b {
                b'"' | b'\''
                    if after_property || matches!(prev, b'[' | b'{' | b',' | b':' | b'?') =>
                {
                    i += 1;
                    loop {
                        match input.get(i) {
                            None => return false,
                            Some(&c) if Self::is_break(c) => return false,
                            // An escaped line break would carry the key onto
                            // another line, which an implicit key cannot do.
                            Some(b'\\') if b == b'"' => match input.get(i + 1) {
                                None => return false,
                                Some(&c) if Self::is_break(c) => return false,
                                Some(_) => i += 2,
                            },
                            Some(&c) if c == b => {
                                if b == b'\'' && input.get(i + 1) == Some(&b'\'') {
                                    i += 2;
                                } else {
                                    break;
                                }
                            }
                            Some(_) => i += 1,
                        }
                    }
                }
                b'[' | b'{' => depth += 1,
                b']' | b'}' => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        i += 1;
                        break;
                    }
                }
                b'#' if blank_before => return false,
                c if Self::is_break(c) => return false,
                _ => {}
            }
            prev = b;
            blank_before = false;
            i += 1;
        }
        if depth != 0 {
            return false;
        }
        while input.get(i).is_some_and(|&c| Self::is_inline_whitespace(c)) {
            i += 1;
        }
        input.get(i) == Some(&b':') && Self::is_ws_break_or_eoi(input.get(i + 1).copied())
    }

    /// Parse the flow collection that is an implicit mapping key (see
    /// [`Self::looks_like_flow_keyed_mapping_entry`]) inside the key node the
    /// caller already opened: the same wrapper-around-the-collection shape
    /// `parse_explicit_key`'s `[`/`{` arms write, which the reader already
    /// reads as `""` (#1188). The `:` is left for the caller.
    fn parse_flow_mapping_key(&mut self) -> Result<(), YamlError> {
        if self.peek() == Some(b'[') {
            self.parse_flow_sequence()?;
        } else {
            self.parse_flow_mapping()?;
        }
        self.reject_trailing_flow_content(true)
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
            // Skip optional whitespace: spaces and tabs, as `parse_mapping_entry` does
            // (`s-separate-in-line`, #5814).
            while i < self.input.len() && Self::is_inline_whitespace(self.input[i]) {
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
                // #5062: a `#` after a space or tab starts a comment (the s-separate-in-line rule
                // `parse_unquoted_key` applies, #370/#410), so a `: ` after it is the comment's
                // (`- a # TODO: x` is the scalar `a`, not a key).
                b'#' if i > self.pos && matches!(self.input[i - 1], b' ' | b'\t') => return false,
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
        // A flow collection keying the entry (`&k [a, b]: v`) counts too (#1188).
        let result = !self.at_line_end()
            && (self.looks_like_mapping_entry() || self.looks_like_flow_keyed_mapping_entry());
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

    /// [`Self::skip_to_eol`] eight bytes at a time, and whether the line it passed held a NEL, LS or PS
    /// (#6369; it noted tabs for #6287 before). Each word is tested for a break byte (exactly: the high
    /// bit of every zero byte of the word xor the pattern); the first break in a word ends the line.
    /// A word with no byte outside ASCII, which is all of the usual text, costs one more `AND`. Only
    /// a word with one is tested for the lead bytes, `0xC2` and `0xE2` (one test: they differ in bit
    /// 5 alone), and only a lead byte before the break is read, in place, for the break it may begin,
    /// so text full of U+2014 or U+00E9 reads only those bytes.
    #[inline]
    fn skip_to_eol_noting_unicode_break(&mut self, saw_unicode_break: &mut bool) {
        const NL: u64 = 0x0A0A_0A0A_0A0A_0A0A;
        const CR: u64 = 0x0D0D_0D0D_0D0D_0D0D;
        const HIGH: u64 = 0x8080_8080_8080_8080;
        const BIT5: u64 = 0x2020_2020_2020_2020;
        const LEAD: u64 = 0xE2E2_E2E2_E2E2_E2E2;
        let input = self.input;
        let mut at = self.pos;
        while at + 8 <= input.len() {
            let mut word = [0u8; 8];
            word.copy_from_slice(&input[at..at + 8]);
            let w = u64::from_le_bytes(word);
            let breaks = if HAS_CR {
                zero_bytes(w ^ NL) | zero_bytes(w ^ CR)
            } else {
                zero_bytes(w ^ NL)
            };
            if w & HIGH != 0 && !*saw_unicode_break {
                // the lead bytes below the first break, if the word has one
                let mut leads = zero_bytes((w | BIT5) ^ LEAD);
                if breaks != 0 {
                    leads &= (1u64 << (breaks.trailing_zeros() & !7)) - 1;
                }
                while leads != 0 {
                    let i = (leads.trailing_zeros() / 8) as usize;
                    if block_unicode::unicode_break_len(input, at + i) > 0 {
                        *saw_unicode_break = true;
                        break;
                    }
                    // the next flagged byte (a flag is the high bit of its byte)
                    leads &= leads - 1;
                }
            }
            if breaks != 0 {
                self.pos = at + (breaks.trailing_zeros() / 8) as usize;
                return;
            }
            at += 8;
        }
        while at < input.len() && !Self::is_break(input[at]) {
            if !*saw_unicode_break && block_unicode::unicode_break_len(input, at) > 0 {
                *saw_unicode_break = true;
            }
            at += 1;
        }
        self.pos = at;
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
            } else if let Some(at) = self.yq_indented_head_directive() {
                // #6408: an indented `%YAML` line after the head's `---` is yq's header too.
                self.pos = at;
                self.skip_to_eol();
                self.record_standalone_comment(at, self.pos);
            } else if b == b'\t'
                && (self.pos == 0 || Self::is_break(self.input[self.pos - 1]))
                && self.documents_started <= 1
                && self.bp_pos == self.document_start_bp_pos
                && (self.tab_only_line_indent(self.pos).is_some()
                    || (self.yq_mode
                        && self.tab_comment_line_indent(self.pos).is_some()
                        && self.only_yq_header_lines_before_cursor()))
            {
                // #6295: a tab-only line in the head of the first document is yq's header, which its
                // slurp takes away (`---` / `⇥` / `a: 1`): an empty line, as a space-led one is. A tab
                // before a comment is the same (#6342): the stream `⇥ # c` is one null document.
                self.skip_inline_whitespace();
            } else if b == b'%' && self.at_yq_head_yaml_directive() {
                // #6342: a `%YAML` line after the head's `---` is yq's header too, which its slurp sets
                // aside up to the first content line (`---` / `%YAML 1.2` / `a: 1`), and prints back
                // as it was written, as a comment does.
                let start = self.pos;
                self.skip_to_eol();
                self.record_standalone_comment(start, self.pos);
            } else if b == b' ' {
                // Check if rest of line is whitespace or comment
                let start = self.pos;
                // #6286: a tab-only line go-yaml refuses is left for the line dispatcher to reject.
                if (start == 0 || Self::is_break(self.input[start - 1]))
                    && self.tab_only_line_is_refused(start)
                {
                    break;
                }
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
        Self::marker_prefix(&self.input[self.pos.min(self.input.len())..], b"---")
    }

    /// Whether `text` starts with `marker` (`---` or `...`) as a document marker: followed by white space
    /// (space or tab), a line break, or EOF, and with the three bytes present. The tab is not optional:
    /// `doc_marker_char` (validate.rs), the strict validator's copy of this same check, already includes
    /// it, and dropping it meant `---\tfoo` was silently parsed as content instead of a document boundary
    /// (#434). One rule for [`Self::is_document_start`], [`Self::is_document_end`] and
    /// [`Self::next_document_has_content`].
    fn marker_prefix(text: &[u8], marker: &[u8; 3]) -> bool {
        text.len() >= 3 && text[..3] == marker[..] && Self::is_ws_break_or_eoi(text.get(3).copied())
    }

    /// Check if we're at a document end marker (`...`).
    fn is_document_end(&self) -> bool {
        Self::marker_prefix(&self.input[self.pos.min(self.input.len())..], b"...")
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
        // Every frame of the closing document has been closed, each key with its value or null.
        self.pending_explicit_key_below.clear();
        self.explicit_key_scalar_start = None;
        self.explicit_key_node_closed = false;
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
        self.documents_started += 1;
        self.document_root_node_taken = false;
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
    fn skip_directives(&mut self) -> Result<bool, YamlError> {
        self.skip_directive_run(false)
    }

    /// [`Self::skip_directives`] and whether it skipped any directive. `in_head_document`: the run sits in the head
    /// of the stream's first document, behind yq's leading `---` (#6419), where the document is still open.
    fn skip_directive_run(&mut self, in_head_document: bool) -> Result<bool, YamlError> {
        let mut run = DirectiveRun::default();
        let mut skipped = false;
        loop {
            self.skip_directive_gap_whitespace()?;
            // #6408: an indented `%YAML` in the head is yq's header too.
            if let Some(at) = self.yq_indented_head_directive() {
                self.pos = at;
                self.skip_to_eol();
                self.skip_line_break();
                continue;
            }
            if (self.in_document && !in_head_document) || self.peek() != Some(b'%') {
                break;
            }
            skipped = true;
            if !self.input[self.pos..].starts_with(b"%YAML") {
                self.saw_non_header_directive = true;
            }
            if self.yq_mode {
                self.check_yq_directive(&mut run)?;
            }
            self.skip_to_eol();
            self.skip_line_break();
        }
        Ok(skipped)
    }

    /// In yq mode, a `%` at column 0 that starts a node is a directive, and a directive ends the document as a `...`
    /// does (#6419): `a: 1` / `%TAG !e! w` / `---` / `b: 2` is two documents, whatever the document held (a mapping
    /// or sequence, a value still to come, a property without its node, a block scalar) and what follows must be a
    /// `---`. The cursor is on the `%`. Behind yq's leading `---` the directive belongs to the head, and the
    /// document goes on as `---` / `---` does (#6349).
    fn directive_ends_document(&mut self) -> Result<(), YamlError> {
        let in_head = self.documents_started == 1
            && self.bp_pos == self.document_start_bp_pos
            && self.only_yq_header_lines_since(self.leading_run_from);
        if in_head {
            self.skip_directive_run(true)?;
            self.leading_run_from = self.pos;
            if !self.is_document_start() {
                return Err(self.err_directive_without_document_start());
            }
            return Ok(());
        }
        self.end_document();
        self.skip_directives()?;
        // The loop reads the `---` as it reads any other, and starts the next document.
        if !self.is_document_start() {
            return Err(self.err_directive_without_document_start());
        }
        Ok(())
    }

    /// In yq mode, refuse a `%` directive line (the cursor is on its `%`) that go-yaml's scanner refuses (#6412).
    ///
    /// yq's header slurp sets aside only a line that starts `%YAML` (`%YAMLL` too, `%YAM` not), so every other directive reaches go-yaml:
    /// a name that is empty (`%`, `% FOO`) or not `YAML` or `TAG` (`%FOO`, any case but the exact word) is an
    /// error, and so is a `%TAG` that is not a handle (`!`, `!!` or `!word!`), a blank, a non-empty URI prefix and
    /// then nothing but a `#` comment, or that repeats a handle already given. A `%YAML` that go-yaml sees (after a
    /// `...`, or after another kind of directive) must be `1.1` alone and come once (#6419); `1.2` is `found
    /// incompatible YAML document`. The library loader keeps the
    /// YAML 1.2 reading, where an unknown directive is ignored.
    fn check_yq_directive(&self, run: &mut DirectiveRun) -> Result<(), YamlError> {
        let start = self.pos;
        // A `%YAML` line in the head of the stream, before any other directive, is yq's header (#6342): the
        // slurp takes every such line, however many, and go-yaml never sees them.
        if self.input[start..].starts_with(b"%YAML")
            && self.documents_started == 0
            && !self.saw_non_header_directive
        {
            return Ok(());
        }
        let line_end = self.input[start..]
            .iter()
            .position(|&b| Self::is_break(b))
            .map_or(self.input.len(), |n| start + n);
        let line = &self.input[start..line_end];
        let refuse = |at: usize, why: &'static str| Err(self.err_unexpected_char(start + at, why));
        let is_blank = |b: u8| b == b' ' || b == b'\t';
        let is_word = |b: u8| b.is_ascii_alphanumeric() || b == b'_' || b == b'-';
        let name_len = line[1..].iter().take_while(|&&b| is_word(b)).count();
        if name_len == 0 {
            return refuse(1, "could not find expected directive name");
        }
        let name = &line[1..=name_len];
        let blanks = |from: usize| line[from..].iter().take_while(|&&b| is_blank(b)).count();
        let mut at = 1 + name_len;
        if name == b"YAML" {
            // go-yaml reads `%YAML`, a blank, `major.minor` and then only blanks and a `#` comment, accepts
            // nothing but `1.1`, and refuses a second one for the document.
            let gap = blanks(at);
            if gap == 0 {
                return refuse(at, "did not find expected whitespace");
            }
            at += gap;
            let digits = |from: usize| {
                line[from..]
                    .iter()
                    .take_while(|&&b| b.is_ascii_digit())
                    .count()
            };
            let value = |from: usize, n: usize| {
                line[from..from + n].iter().fold(0u64, |acc, &b| {
                    acc.saturating_mul(10).saturating_add(u64::from(b - b'0'))
                })
            };
            // go-yaml reads at most two digits of each number.
            let major_len = digits(at);
            if major_len == 0 {
                return refuse(at, "did not find expected version number");
            }
            if major_len > 2 {
                return refuse(at, "found extremely long version number");
            }
            let major = value(at, major_len);
            at += major_len;
            if line.get(at) != Some(&b'.') {
                return refuse(at, "did not find expected digit or '.' character");
            }
            at += 1;
            let minor_len = digits(at);
            if minor_len == 0 {
                return refuse(at, "did not find expected version number");
            }
            if minor_len > 2 {
                return refuse(at, "found extremely long version number");
            }
            let minor = value(at, minor_len);
            at += minor_len;
            at += blanks(at);
            if at < line.len() && line[at] != b'#' {
                return refuse(at, "did not find expected comment or line break");
            }
            if run.yaml {
                return refuse(1, "found duplicate %YAML directive");
            }
            run.yaml = true;
            if (major, minor) != (1, 1) {
                return refuse(1, "found incompatible YAML document");
            }
            return Ok(());
        }
        if name != b"TAG" {
            return refuse(1, "found unknown directive name");
        }
        let gap = blanks(at);
        if gap == 0 {
            return refuse(at, "did not find expected whitespace");
        }
        at += gap;
        // The handle: `!`, `!!` or `!word!`.
        if line.get(at) != Some(&b'!') {
            return refuse(at, "did not find expected '!'");
        }
        let handle_start = at;
        at += 1;
        let word = line[at..].iter().take_while(|&&b| is_word(b)).count();
        if line.get(at + word) == Some(&b'!') {
            at += word + 1;
        } else if word != 0 {
            return refuse(at + word, "did not find expected '!'");
        }
        let handle = (start + handle_start, start + at);
        let gap = blanks(at);
        if gap == 0 {
            return refuse(at, "did not find expected whitespace");
        }
        at += gap;
        // The prefix is URI characters: alphanumerics, `-_;/?:@&=+$,.!~*'()[]{}` and `%` with two hex digits. Anything
        // else (`#`, `^`, `"`, `<`, `>`, a backtick, `|`, `\`, a control byte, a non-ASCII one) ends it.
        let mut prefix = 0;
        while let Some(&b) = line.get(at + prefix) {
            if b.is_ascii_alphanumeric() || b"-_;/?:@&=+$,.!~*'()[]{}".contains(&b) {
                prefix += 1;
            } else if b == b'%'
                && line.get(at + prefix + 1).is_some_and(u8::is_ascii_hexdigit)
                && line.get(at + prefix + 2).is_some_and(u8::is_ascii_hexdigit)
            {
                prefix += 3;
            } else {
                break;
            }
        }
        if prefix == 0 {
            return refuse(at, "did not find expected tag URI");
        }
        at += prefix;
        if at < line.len() && !is_blank(line[at]) {
            return refuse(at, "did not find expected whitespace or line break");
        }
        at += blanks(at);
        if at < line.len() && line[at] != b'#' {
            return refuse(at, "did not find expected comment or line break");
        }
        if run
            .tag_handles
            .iter()
            .any(|&(a, b)| self.input[a..b] == self.input[handle.0..handle.1])
        {
            return refuse(handle_start, "found duplicate %TAG directive");
        }
        run.tag_handles.push(handle);
        Ok(())
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
    fn skip_directive_gap_whitespace(&mut self) -> Result<(), YamlError> {
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
                // #6295: after a `...` the gap is not yq's header (its slurp only takes the head of the
                // input), so a tab-only line there is go-yaml's refused one, not an empty line.
                if self.documents_started >= 1
                    && (start == 0 || Self::is_break(self.input[start - 1]))
                    && self.tab_only_line_indent(start).is_some()
                {
                    return Err(YamlError::TabIndentation {
                        line: self.current_line(),
                        offset: start,
                    });
                }
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
        Ok(())
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
                        if let Some(escaped) = self.peek() {
                            // JSON input must not inherit YAML-only escapes,
                            // especially line continuations that discard bytes
                            // before the value is ever read (#3394).
                            if self.json_strict
                                && !matches!(
                                    escaped,
                                    b'"' | b'\\' | b'/' | b'b' | b'f' | b'n' | b'r' | b't' | b'u'
                                )
                            {
                                return Err(self.err_unexpected_char(
                                    self.pos,
                                    "escape sequence in JSON string",
                                ));
                            }
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
                    // #6286: a line holding a tab, at a column no deeper than the block that owns
                    // this scalar, is not an empty line to go-yaml: it stops the scalar and the
                    // line is refused (`found a tab character that violates indentation`).
                    // The scalar ends here and `parse_document_line` raises it.
                    if !is_doc_root
                        && next_char == b'\t'
                        && next_indent <= self.innermost_block_indent()
                    {
                        break;
                    }
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
                is_sequence_indicator && next_indent <= start_indent && !is_doc_root;

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

        self.last_plain_scalar = Some(PlainScalarSpan {
            start,
            end,
            owner: self.innermost_block_indent(),
        });

        // The extent the Unicode-break tab check (#6214) reads, kept for the caller: it must not
        // widen this function's return type, which every plain scalar pays for.
        self.last_plain_content_end = content_end;

        // Return absolute end position (not length)
        end
    }

    /// The unquoted scalar a node of its own line starts, scanned from the indent of the block that holds it
    /// (a continuation line need only be indented more than that block, as under a `key:`, #5083), or from the
    /// document's own indent at the root (#5086).
    fn parse_own_line_unquoted_value(&mut self, indent: usize, is_truly_doc_root: bool) -> usize {
        if is_truly_doc_root {
            self.parse_unquoted_value_doc_root(indent)
        } else {
            self.parse_unquoted_value_with_indent(self.innermost_block_indent())
        }
    }

    /// Parse an unquoted key (stops at colon+space).
    fn parse_unquoted_key(&mut self) -> Result<usize, YamlError> {
        let start = self.pos;
        if self.yq_mode {
            self.reject_unstartable_plain_at(self.pos)?;
        }

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

        self.reject_tab_after_unicode_break(start, self.pos)?;

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
        self.close_deeper_indents_above(new_indent, 1);
    }

    /// [`Self::close_deeper_indents`], leaving the `keep` frames at the bottom of the stack open
    /// whatever their indent: the root sentinel alone (`keep` is 1) for the ordinary close, and
    /// up to a nested key's outer owner for #6322.
    fn close_deeper_indents_above(&mut self, new_indent: usize, keep: usize) {
        while self.indent_stack.len() > keep {
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

    /// Whether an explicit value's `indent` matches the mapping frame opened by its own pending `?`, if one is
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

    /// The #901 check: whether `indent` would land ambiguously if
    /// [`Self::close_deeper_indents`] popped every frame deeper than it -- the
    /// surviving (landing) frame's own recorded indent doesn't exactly match
    /// `indent`, or it is a sequence item's virtual indent, which a line can
    /// never sit on once the item's content has closed (#6385) -- so the line
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
    /// No line gets a tolerance, because real yq v4.53.3 refuses every shape that used to be waved
    /// through: an out-dented continuation of an open sequence (`a:\n  - x\n - y\n`, which
    /// #325/#485 read as a third item), a `-` or a `key: value` line in an open compact mapping's gap
    /// (`- k: a\n - z\n` and `- k: a\n b: 1\n`, which #900 and #885 read as the next item or another
    /// entry of the mapping), and a line on a sequence item's own virtual indent (`- - a\n - b\n`,
    /// which dropped the `b`) (#5086, #6385). An error loses nothing silently, which is what those
    /// tolerances were for.
    fn mapping_under_mapping_gap_reaches(&self, indent: usize) -> bool {
        if self.indent_stack.len() < 2 {
            return false;
        }
        let top = self.indent_stack.len() - 1;
        // `close_deeper_indents` only ever pops (current_indent > new_indent) --
        // going deeper or staying flat at the current top frame never pops
        // anything, so there's no landing-frame question to ask at all: it's
        // either a new nested frame (handled by the caller opening one) or an
        // ordinary sibling of the frame already open, both fine.
        if indent >= self.indent_stack[top] {
            return false;
        }
        let mut landing_idx = top;
        while landing_idx > 0 && self.indent_stack[landing_idx] > indent {
            landing_idx -= 1;
        }
        // A line that lands on a sequence item's own virtual indent belongs to neither the item
        // nor its sequence.
        self.indent_stack[landing_idx] != indent
            || self.type_stack[landing_idx] == NodeType::SequenceItem
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
    /// for an unrelated check in this file.
    fn check_mapping_under_mapping_gap(&self, indent: usize) -> Result<(), YamlError> {
        self.require_consistent_indentation(!self.mapping_under_mapping_gap_reaches(indent))
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

                // The sequence just closed was the pending explicit key's own content
                // (`?⏎- a⏎k: w`, #6370): a non-empty key refuses a same-column sequence as its
                // value (#1104), and a bare `?` takes it as the key, so the key has no value
                // yet. It gets its null here, as at every other pop site. (Before #6370 this
                // cleared the flag with no null, because a bare `?` made the sequence the
                // key's value; left stale, a later entry at this indent was misattributed to
                // the closed key, #1040.)
                // Shares `pending_explicit_key_owns_current_frame`'s predicate with
                // `close_pending_explicit_key` rather than re-deriving it here (#106).
                self.close_pending_explicit_key();
            }
        }
    }

    /// What a mapping entry (`key:` or `? key`) at `indent` does to a block sequence still open
    /// there: closes it when it is a key's same-indent value
    /// ([`Self::close_same_indent_sequence_before_mapping_entry`]), and otherwise rejects the entry.
    /// A sequence open at that indent with no mapping to close it into has its items at that
    /// column, so the entry is neither an item nor a sibling of the sequence's key (`a:` / `  - x`
    /// / `  c: 1`). Real yq stops with `did not find expected '-' indicator`; the entry used to
    /// become one more item holding a mapping (#4737).
    fn settle_open_sequence_before_mapping_entry(
        &mut self,
        indent: usize,
    ) -> Result<(), YamlError> {
        self.close_same_indent_sequence_before_mapping_entry(indent);
        if self.current_type == Some(NodeType::Sequence)
            && self.indent_stack.last().copied() == Some(indent)
        {
            return Err(self.err_unexpected_char(
                self.pos,
                "a mapping entry cannot follow a sequence item at its indent",
            ));
        }
        Ok(())
    }

    /// Parse a sequence item (starts with `- `).
    fn parse_sequence_item(&mut self, indent: usize) -> Result<(), YamlError> {
        self.enter_nested()?;
        let result = self.parse_sequence_item_inner(indent);
        self.nesting_depth -= 1;
        result
    }

    fn parse_sequence_item_inner(&mut self, indent: usize) -> Result<(), YamlError> {
        let _item_start = self.pos;
        self.reject_misplaced_sequence_indicator()?;

        // Mark the `-` position
        self.set_ib();

        // First close any deeper containers. This might reveal an existing sequence
        // at this indent level that we can reuse. A `-` in a gap between two open
        // blocks never gets here: the dispatch arm refuses it, as yq does (#5086).
        self.close_deeper_indents(indent);

        // Now check if we need to open a new sequence (check AFTER closing).
        // Sequence items must be at the exact same indent as the sequence. For
        // nested sequences created by the `- - item` pattern, items can be at
        // greater indent because the nested sequence's indent is virtual.
        let need_new_sequence = self.current_type != Some(NodeType::Sequence)
            || indent != *self.indent_stack.last().unwrap();

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
            self.defer_item_value_to_later_lines(indent, had_property, wrapper_bp, None);
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
            // #6379: a property before the `?` (`- &t ? x`) leaves no key to open.
            if had_property {
                return Err(self.err_unexpected_char(
                    self.pos,
                    "mapping keys are not allowed in this context",
                ));
            }
            self.attach_item_head_at(self.bp_pos, indent);
            self.parse_explicit_key(self.current_column())?;
            // Don't close anything - mapping and item are closed by
            // close_deeper_indents when we see content at lower indent.
        } else if self.at_bare_colon() {
            // #6288: `- : b` has no key and no `?` before the `:`, which yq refuses (a tag or an
            // anchor first, `- !!str : b`, is an empty key it reads, and starts with neither).
            return Err(self.err_unexpected_char(self.pos, "did not find expected key"));
        } else if self.looks_like_mapping_entry() || self.looks_like_flow_keyed_mapping_entry() {
            // Check for compact mapping: `- key: value` (or `- [a, b]: value`, #1188)
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

    /// A sequence item whose value stands on the lines below its `-` (or below the properties
    /// after it): settle where the line comment, the comments between and a null value go, and
    /// leave the item open for the main loop to read the value into. The one copy of this, shared
    /// by an ordinary item and the first item of an explicit key's sequence (#6580).
    ///
    /// `indent` is the `-`'s column, `wrapper_bp` the item wrapper's own bp. `had_property` is
    /// whether properties follow the dash on its line: an ordinary item passes it, the first item
    /// of an explicit key's sequence passes `false` (properties there are read by its own #6570
    /// branch, so only a dash alone on its line reaches this). `lines_above_dash` is how many
    /// pending head lines were above the dash, when the caller's lookahead has already recorded
    /// the lines below it (#6593); `None` counts them here, before this routine's own lookahead.
    fn defer_item_value_to_later_lines(
        &mut self,
        indent: usize,
        had_property: bool,
        wrapper_bp: usize,
        lines_above_dash: Option<usize>,
    ) {
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
        //   attaches to it here, exactly as the plain-scalar arm of
        //   `parse_sequence_item_inner` does for `- x`;
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
            let above_dash = lines_above_dash.unwrap_or(self.pending_head_lines.len());
            let value_is_null = self
                .next_line_settles_value_is_null(indent)
                .unwrap_or_else(|| self.following_value_is_null(indent));
            // Properties on a line of their own name an empty node, so the item is not
            // absent, but its comment still goes forward off the dash line (#2835).
            let empty_property_item =
                !value_is_null && self.next_line_is_properties_of_a_null_value(indent);
            if value_is_null || empty_property_item {
                if empty_property_item && self.pending_head_lines.len() > above_dash {
                    // Comment lines between the dash and the properties go forward with
                    // the rest, as the dash line's own comment does below: yq heads
                    // whatever opens next with them, not placing them by the column they
                    // were written at (#4851). The whole block is reset, a line pending
                    // above the dash included, since it is one block to the placement
                    // rule and yq moves it the same way (`- 1` / `# a` / `-` / `# x` /
                    // `!!str`, measured).
                    self.pending_block = PendingBlock::default();
                }
                match (trailing, empty_property_item) {
                    (Some(range), false) => self.float_absent_item_comment(range),
                    (Some(range), true) => self.float_comment_past_empty_property_item(range),
                    (None, _) => self.end_float(),
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
        let is_null = if self.peek().is_none() {
            true
        } else {
            let next_indent = self.count_indent().unwrap_or(0);
            next_indent < indent || (next_indent == indent && !self.header_at_indent(next_indent))
        };
        self.pos = saved_pos;
        is_null
    }

    /// Whether the content line the cursor is on, at its indent `indent`, is a block scalar
    /// header (`|` or `>`). A header cannot be a key or an item, so at the indent of an empty
    /// `key:`, `:` or `-` it is that node's value, as a sequence is for a key: `a:` / `|` / `  t` is
    /// `{"a": "t\n"}` and `-` / `|` / `  t` is `["t\n"]` in yq (#6390). The one definition behind
    /// every place that tells a null value from one on the next line, so they cannot disagree.
    /// Restores `self.pos`.
    fn header_at_indent(&mut self, indent: usize) -> bool {
        let saved_pos = self.pos;
        self.advance_by(indent);
        let is_header = self.peek().is_some_and(Self::opens_block_scalar);
        self.pos = saved_pos;
        is_header
    }

    /// Whether a line that starts with `byte` is a block scalar header: `|` or `>`. The one test
    /// every lookahead for a header applies, so a change to what counts is made once (#6390).
    #[inline]
    const fn opens_block_scalar(byte: u8) -> bool {
        matches!(byte, b'|' | b'>')
    }

    /// [`Self::following_value_is_null`] for a value that a block sequence at the owner's own
    /// indent also fills: `? k` / `: &x` / `- item` is `{k: [item]}`, as `k: &x` / `- item` is
    /// (#4736). Restores `self.pos` before returning.
    fn following_value_is_null_unless_sequence_at(&mut self, indent: usize) -> bool {
        let saved_pos = self.pos;
        self.skip_to_eol();
        self.skip_newlines();
        let is_null = if self.peek().is_none() {
            true
        } else {
            let next_indent = self.count_indent().unwrap_or(0);
            let line_start = self.pos;
            self.advance_by(next_indent);
            let is_sequence =
                matches!(self.peek(), Some(b'-')) && Self::is_ws_break_or_eoi(self.peek_at(1));
            self.pos = line_start;
            next_indent < indent
                || (next_indent == indent && !is_sequence && !self.header_at_indent(next_indent))
        };
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
            // A block scalar header at the indent is the value, as `following_value_is_null` has it.
            Some(&b) if !Self::is_break(b) && b != b'#' && b != b'\t' => {
                Some(spaces < indent || (spaces == indent && !Self::opens_block_scalar(b)))
            }
            None if spaces == 0 => Some(true),
            _ => None,
        }
    }

    /// Whether the node properties that open the cursor's line, under a bare `-` or `key:` on the
    /// line above (either may carry properties of its own: `- &x`, `k: !!map`), are followed by
    /// no value (#2835). The properties name an empty node then, and real yq reads the next key
    /// or item as a sibling, not as their value. Called with `self.pos` on the line's break or
    /// its trailing comment; restores it.
    fn own_line_properties_have_no_value(&mut self, indent: usize) -> bool {
        let Some((lead, rest)) = self.previous_content_line() else {
            return false;
        };
        let Some((owner, is_dash)) = Self::deferred_value_owner(lead, rest) else {
            return false;
        };
        indent > owner && self.value_after_own_line_properties_is_null(owner, is_dash)
    }

    /// The column of the dash or key whose value the line `rest` (standing `lead` columns in)
    /// leaves for the lines below it, and whether it is a dash: a bare `-`, a chain of them
    /// (`- -`), a dash with only properties (`- &x`) or such a dash after an explicit key's marker
    /// (`: - &x`, #6565), or a key written up to its colon, with properties after it or not (`k:`,
    /// `- k: !!map`, `: k:`). A key's column is the line's indent plus the markers it opens with
    /// (`-`, and `?` and `:`, see [`Self::walk_line_markers`]); a dash is the owner only when it
    /// is the last marker, so `- ?` is owned by its `?`, which is not a dash.
    fn deferred_value_owner(lead: usize, rest: &[u8]) -> Option<(usize, bool)> {
        let (marker_len, last_marker) = Self::walk_line_markers(lead, rest, LineMarkers::Explicit);
        let tail = &rest[marker_len..];
        match last_marker {
            Some((column, true)) if only_node_properties(tail) => Some((column, true)),
            // An explicit value's `:` with nothing after it but properties (`:`, `: &x`) leaves
            // its value for the lines below as the key's `:` of `k:` does, at its own column.
            Some((column, false)) if only_node_properties(tail) && rest[column - lead] == b':' => {
                Some((column, false))
            }
            _ => line_ends_with_colon_after_properties(tail).then_some((lead + marker_len, false)),
        }
    }

    /// The markers a content line opens with, past which its key or content stands: how many bytes
    /// they take (their spacing included), and the last one's column and whether it is a dash. One
    /// walk for [`Self::previous_line_key`] and [`Self::deferred_value_owner`], so the two cannot
    /// disagree on where a key stands (#6543, #6565).
    fn walk_line_markers(
        lead: usize,
        rest: &[u8],
        markers: LineMarkers,
    ) -> (usize, Option<(usize, bool)>) {
        let mut marker_len = 0;
        let mut last_marker = None;
        while rest.get(marker_len).is_some_and(|&b| {
            b == b'-' || (markers == LineMarkers::Explicit && (b == b'?' || b == b':'))
        }) && Self::is_ws_break_or_eoi(rest.get(marker_len + 1).copied())
        {
            last_marker = Some((lead + marker_len, rest[marker_len] == b'-'));
            marker_len += 1;
            while matches!(rest.get(marker_len), Some(b' ' | b'\t')) {
                marker_len += 1;
            }
        }
        (marker_len, last_marker)
    }

    /// Whether the value that the dash or key at column `owner` leaves for the lines below it is
    /// absent once the cursor's line (its own-line properties) is past: the next content line
    /// stands no deeper than the owner, and is not a zero-indented sequence a key takes as its
    /// value (`seq:` / ` &x` / `- a`, YAML test suite SKE5). A value can stand left of its own
    /// properties as long as it is deeper than the owner (`folded:` / `   !foo` / `  >1`, M5C3),
    /// so the properties' own column says nothing. The one definition behind both the empty node
    /// the properties name and the comment that floats off the item that holds it.
    fn value_after_own_line_properties_is_null(&mut self, owner: usize, is_dash: bool) -> bool {
        if is_dash {
            self.following_value_is_null(owner)
        } else {
            self.following_value_is_null_unless_sequence_at(owner)
        }
    }

    /// Whether the next content line holds nothing but node properties (`!!str`, `&x`) and the
    /// one after it stands no deeper than the item at `indent`, so the properties name an empty
    /// value (#2835). A bare `-` item whose value is that empty node is still absent for the
    /// comment on its dash line: real yq floats it onto the next node as it does for `-` alone.
    /// Called with `self.pos` on the current line's break or its trailing comment; restores it.
    fn next_line_is_properties_of_a_null_value(&mut self, indent: usize) -> bool {
        let mut p = self.pos;
        // The first line below that is neither blank nor a comment.
        let (line_start, spaces) = loop {
            while self.input.get(p).is_some_and(|&b| !Self::is_break(b)) {
                p += 1;
            }
            if self.input.get(p).is_none() {
                return false;
            }
            p += line_break_len(self.input, p);
            let spaces = super::simd::count_leading_spaces(self.input, p);
            match self.input.get(p + spaces) {
                Some(b'#') => p += spaces,
                Some(&b) if Self::is_break(b) => p += spaces,
                _ => break (p + spaces, spaces),
            }
        };
        if spaces <= indent || !matches!(self.input.get(line_start), Some(b'&' | b'!')) {
            return false;
        }
        let line_end = self.input[line_start..]
            .iter()
            .position(|&b| Self::is_break(b))
            .map_or(self.input.len(), |at| line_start + at);
        if !only_node_properties(&self.input[line_start..line_end]) {
            return false;
        }
        let saved_pos = self.pos;
        self.pos = line_end;
        let is_null = self.value_after_own_line_properties_is_null(indent, true);
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

        // Parse the key. A flow collection keying the entry (`- [a, b]: v`)
        // is a container inside the key node, with no text range (#1188).
        let flow_key = self.looks_like_flow_keyed_mapping_entry();
        let key_end = match self.peek() {
            _ if flow_key => {
                self.parse_flow_mapping_key()?;
                self.pos
            }
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
        if !flow_key {
            self.set_bp_text_end(key_end);
        }

        // Close key node
        self.write_bp_close();

        // Skip optional whitespace between key and colon, as the block mapping's site does: a
        // quoted or alias key ends at its closing quote or name, so `- "k" : 1` and `- *a : 1`
        // (which yq writes for an alias key, #5469) reach here with the space still ahead. A
        // plain key already took its trailing space.
        self.skip_inline_whitespace();

        // Expect colon
        if self.peek() != Some(b':') {
            return Err(
                self.err_unexpected_char(self.pos, "expected ':' after key in compact mapping")
            );
        }
        self.advance(); // Skip ':'
        let colon_end = self.pos;

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
        // Where an empty value goes (#6386): at its key's colon, or at the end of its properties. Never
        // on the next line, whose first byte the reader would decode the node from.
        let null_at = if had_property { self.pos } else { colon_end };
        self.reject_explicit_key_after_implicit_key()?;

        // Parse value
        if self.at_line_end() {
            if had_property {
                // An anchor/tag caused this deferral - the comment belongs
                // to the deferred value, not this key's own line
                // (#784/#1078, matching `parse_mapping_entry`'s identical
                // split below) - defer it rather than capturing it here;
                // `take_pending_head_comment` claims it wherever the next
                // primary node opens.
                //
                // First claim a comment an *earlier* anchor's deferred value
                // floated onto this key (`a: &x # c1` / `b: &y # c2`, #1080):
                // `defer_line_comment` never overwrites a pending comment, so
                // leaving `c1` unclaimed here silently dropped `c2` and then
                // `c1` itself as stale. The key's own same-line comment
                // belongs to the deferred value, so nothing else competes for
                // the key's slot.
                self.take_pending_head_comment(self.last_open_bp_pos);
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
            self.reject_tab_only_line_at_cursor()?;
            if self.peek().is_none() {
                // EOF - null value: emit empty value node
                self.write_null_value_at(null_at);
            } else {
                let next_indent = self.count_indent().unwrap_or(0);

                // Check if next content is a sequence indicator
                let saved_pos = self.pos;
                self.advance_by(next_indent);
                let is_sequence_indicator =
                    matches!(self.peek(), Some(b'-')) && Self::is_ws_break_or_eoi(self.peek_at(1));
                // A block scalar header at the key's own indent is the key's value (see
                // `parse_mapping_entry`): `- a:` / `  |` / `    t` is `[{"a": "t\n"}]`.
                let is_block_scalar_header = self.peek().is_some_and(Self::opens_block_scalar);
                self.pos = saved_pos;

                if next_indent < indent
                    || (next_indent == indent && !is_sequence_indicator && !is_block_scalar_header)
                {
                    // Next line is at lower indent, or same indent but not a sequence
                    // - null value: emit empty value node
                    self.write_null_value_at(null_at);
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
        // silently misattributing it. A line between a sequence item and its
        // compact mapping is refused the same way (#6385).
        self.check_mapping_under_mapping_gap(indent)?;

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
        self.settle_open_sequence_before_mapping_entry(indent)?;

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

        // A flow collection keying the entry (`[a, b]: v`) is a container
        // inside the key node, with no text range of its own (#1188).
        let flow_key = self.looks_like_flow_keyed_mapping_entry();

        // Parse the key - check for empty key first (colon at start)
        let key_end = if flow_key {
            self.parse_flow_mapping_key()?;
            self.pos
        } else if self.peek() == Some(b':') {
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
        if !flow_key {
            self.set_bp_text_end(key_end);
        }

        // Close key node
        self.write_bp_close();

        // Skip optional whitespace between key and colon (e.g., 'key' : value)
        self.skip_inline_whitespace();

        // Expect colon
        if self.peek() != Some(b':') {
            return Err(self.err_unexpected_char(self.pos, "expected ':' after key"));
        }
        self.advance(); // Skip ':'
        let colon_end = self.pos;

        // Skip space after colon
        self.skip_inline_whitespace();

        // Parse value
        if self.at_line_end() {
            // Check if at EOF - if so, we need an explicit empty value node
            if self.peek().is_none() {
                // EOF after colon - emit empty value
                self.write_null_value_at(colon_end);
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
            self.reject_tab_only_line_at_cursor()?;
            if self.peek().is_none() {
                // EOF - null value: emit empty value node
                self.write_null_value_at(colon_end);
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
            // A line that starts with `|` or `>` is a block scalar header: it cannot be a key, so at the
            // key's own indent it is the key's value, as a sequence is (`a:` / `|` / `  x` is
            // `{"a": "x\n"}` in yq). It used to read only because the null written for it sat on the
            // `|` (#6386), which the reader then took for the block scalar.
            let is_block_scalar_header = next_char.is_some_and(Self::opens_block_scalar);
            self.pos = saved_pos;

            if next_indent < indent {
                // Next line is at lower indent - definitely null value
                self.write_null_value_at(colon_end);
                return Ok(());
            }

            if next_indent == indent && !is_sequence_indicator && !is_block_scalar_header {
                // Next line is at same indent but NOT a sequence or a block scalar - null value
                // (If it were a sequence, the sequence is the value of this key)
                self.write_null_value_at(colon_end);
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
                            self.reject_deeper_line_after_value()?;
                            self.pos
                        }
                        // `json_strict` still applies here: reachable via a
                        // streamed key like `true` whose value continues on the
                        // next line (`true:\n  'v'`) -- real yq's `-p json`
                        // reads this as two independent values in a stream
                        // (#2839, not a bare-mapping rejection), but each
                        // streamed value is still validated the same way any
                        // JSON value is; confirmed live, `true:\n  'v'` and
                        // `true:\n  True` both error in real yq too.
                        Some(b'\'') => {
                            if self.json_strict {
                                return Err(self.err_json_strict_single_quote());
                            }
                            self.parse_single_quoted()?;
                            self.reject_deeper_line_after_value()?;
                            self.pos
                        }
                        _ => {
                            let start = self.pos;
                            let end = self.parse_unquoted_value_with_indent(indent);
                            self.check_plain_scalar()?;
                            self.check_json_strict_scalar(start, end)?;
                            // The scan stops at a `: ` of a later line: no node of its own (#5083), as
                            // for a value on the key's line (#5064).
                            self.reject_content_after_inline_value()?;
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
            self.reject_explicit_key_after_implicit_key()?;

            // After anchor, check if value continues on next line
            if self.at_line_end() {
                // A trailing comment here belongs to the deferred value, not
                // to this key's own line (#784, distinct from #765's
                // no-anchor case) - defer it rather than dropping it;
                // `take_pending_head_comment` claims it wherever the next
                // primary node opens.
                //
                // A comment an earlier anchor's deferred value floated onto
                // this key is claimed first, for the same reason as in
                // `parse_compact_mapping_entry` (#1080).
                self.take_pending_head_comment(self.last_open_bp_pos);
                self.defer_line_comment();

                // Need to check if the next line has content for this value,
                // or if the value is null (same or lower indent on next line)
                self.skip_to_eol();

                // Save position to look ahead
                let saved_pos = self.pos;

                // Look at next content line
                self.skip_newlines();
                self.reject_tab_only_line_at_cursor()?;
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
                // A block scalar header at the key's indent is its value too (#6390).
                if next_indent < indent
                    || (next_indent == indent
                        && !is_sequence_at_same_indent
                        && !self.header_at_indent(next_indent))
                {
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

    /// Claim a comment an earlier anchor's deferred value floated onto this explicit key (`x: &anc
    /// # c` / `  ? b # own`, #4384), as `parse_mapping_entry` claims it at an implicit key's open
    /// (#1080/#784) and before the key's own comment is captured, so the two read in source order
    /// (floated, then own). Called only where the key is a scalar or empty, never for a key that is
    /// itself a collection (`? [1, 2]`, `? - p`, `? k: v`): yq drops the comment there, and the
    /// pending slot is cleared as stale after the line.
    #[inline]
    fn claim_floated_comment_for_key(&mut self) {
        if self.pending_head_comment.is_some() {
            self.take_pending_head_comment(self.bp_pos);
        }
    }

    /// Parse an explicit key (`? key`).
    /// The key can be any value: scalar, sequence, or mapping.
    fn parse_explicit_key(&mut self, indent: usize) -> Result<(), YamlError> {
        self.reject_tab_after_indicator()?;

        // Same ambiguous-gap error as parse_mapping_entry (#901).
        self.check_mapping_under_mapping_gap(indent)?;

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
        self.settle_open_sequence_before_mapping_entry(indent)?;

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

        // #6325: whether the key content opens with a bare `: ` (`? : x`), before any property
        // can stand in front of it (`? !!str : x` is an empty key yq reads).
        let mut bare_colon_key = self.at_bare_colon();

        // Check for a property before the key
        let mut key_has_properties = self.parse_node_properties()?;

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
            self.reject_tab_only_line_at_cursor()?;
            if self.peek().is_none() {
                // EOF - empty key (null) with implicit null value
                // Emit empty key node using the position after `?`
                self.pos = key_pos;
                self.set_ib();
                self.claim_floated_comment_for_key();
                self.write_bp_open();
                self.write_bp_close();
                self.set_pending_explicit_key(owner_depth, true);
                return Ok(());
            }

            let next_indent = self.count_indent().unwrap_or(0);

            // Check if next line starts with `:` at same indent (explicit value)
            // or has other content at same/lower indent (meaning empty key)
            let saved_pos = self.pos;
            self.advance_by(next_indent);
            let next_char = self.peek();
            self.pos = saved_pos;

            // A block sequence at the `?`'s own column is the key's content (YAML 1.2 lets a block
            // sequence sit at its indicator's indent; yq reads `?⏎- a⏎: v` as the key `[a]` with
            // the value `v`, #6370), not a sibling of the `?`: a `-` cannot be one.
            let key_sequence_at_indicator_column = next_indent == indent
                && next_char == Some(b'-')
                && Self::is_ws_break_or_eoi(self.input.get(saved_pos + next_indent + 1).copied());

            // A block scalar header at the `?`'s own column is the key's content too (`?⏎|⏎  t` is
            // the key `t⏎` in yq, #6553), where any other scalar there is the next entry.
            let key_block_scalar_at_indicator_column =
                next_indent == indent && matches!(next_char, Some(b'|' | b'>'));

            if next_indent <= indent
                && !key_sequence_at_indicator_column
                && !key_block_scalar_at_indicator_column
            {
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
                    self.claim_floated_comment_for_key();
                    self.write_bp_open();
                    self.write_bp_close();
                    // Restore position for main loop to process `: value`
                    self.pos = saved_pos;
                    self.set_pending_explicit_key(owner_depth, true);
                    return Ok(());
                }
                // Other content at same/lower indent - empty key (null) with implicit null value
                // Emit empty key node using the position after `?`
                self.pos = key_pos;
                self.set_ib();
                self.claim_floated_comment_for_key();
                self.write_bp_open();
                self.write_bp_close();
                // Restore position for main loop to process next content
                self.pos = saved_pos;
                self.set_pending_explicit_key(owner_depth, true);
                return Ok(());
            }

            // Content at deeper indent: that is the key content. Parsed here as if it
            // had followed the `?` on its own line, so the key is recorded as pending and
            // the entry after it is not taken for its value (#4532).
            self.advance_by(next_indent);
            // #6325: and a bare `: ` opening the key's content on the line below (`?⏎  : x`).
            bare_colon_key = self.at_bare_colon();
            // Properties on the `?` line then belong to the node on this line, which they may
            // stand before (`? !!str⏎  ? a`).
            key_has_properties = false;
            // Node properties on the content line belong to the key, as on the `?` line. When
            // they stand alone on it the key content is a further line down, which the main loop
            // reads as it always has.
            if matches!(self.peek(), Some(b'&' | b'!')) {
                let properties_start = self.pos;
                key_has_properties = self.parse_node_properties()? || key_has_properties;
                if self.at_line_end() {
                    self.pos = properties_start;
                    return Ok(());
                }
            }
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
                // `? &t - x`, `? -⇥x`: refused as everywhere else (#6414).
                self.reject_misplaced_sequence_indicator()?;
                // Sequence as key - open key node and let sequence parsing continue
                // The key will be a sequence
                self.write_bp_open();
                // The `-`'s own column: `indent + 2` for `? - a`, but a key on a line of its
                // own (`?⏎ - a⏎ - b`) or behind more blanks starts elsewhere, and its next item
                // is at that column. A hard-coded `indent + 2` only worked while a `-` between
                // two open blocks was read as the next item (#5086).
                self.write_ty(true); // sequence
                let seq_indent = self.current_column();
                self.indent_stack.push(seq_indent);
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
                self.register_seq_frame(seq_indent, self.last_head_foot_bp, true);

                // Parse first sequence item inline
                self.write_bp_open_seq_item(); // item node
                let item_wrapper_bp = self.last_open_bp_pos;
                // The item is a frame of its own, as `parse_sequence_item_inner` makes one, so that
                // a container its value opens (`? - - p`, `? - a: 1`) is closed inside it by the
                // dedent that ends the key, and not by the close of the item below (#6496).
                let item_depth = self.indent_stack.len();
                self.indent_stack.push(seq_indent + 1);
                self.push_type(NodeType::SequenceItem);
                self.advance(); // skip `-`
                self.skip_inline_whitespace();

                // #6325: `? - : x`, the same keyless entry in the key's first item.
                if self.at_bare_colon() || self.dashes_end_in_bare_colon() {
                    return Err(self.err_unexpected_char(self.pos, "did not find expected key"));
                }

                // #6570: properties alone after the dash (`? - &x`) with a value on a line below
                // are an item whose value the main loop reads there, with the checks it makes of a
                // line below a block scalar, as for any other item: the item stays open. Reading
                // them through `parse_value` below takes the properties' own line end for a plain
                // scalar over the lines that follow, which holds a block scalar header and its
                // content as text (`? - &x` / `   |` / `     t` / `` / `   u` dropped `u`). With
                // nothing below (`? - &x` / `: v`) the item is null and takes that path as before.
                let mut value_on_a_later_line = false;
                // #6580: a dash alone on its line with a value below (`? -` / `   x`, `? -` /
                // `   &x` / `   x`) is the same item as an ordinary one reads: its value is the
                // node on the lines below, not a null item followed by a second one. The item
                // stays open for the main loop, and its comments are placed by the one routine
                // an ordinary item uses.
                if self.at_line_end() {
                    // #6593: this lookahead records the comment lines below the dash as pending,
                    // as the routine's own does, so the routine is told how many lines were above
                    // the dash before it: the rest head the value's first node, not its container.
                    let above_dash = self.pending_head_lines.len();
                    let value_is_null = self
                        .next_line_settles_value_is_null(seq_indent)
                        .unwrap_or_else(|| self.following_value_is_null(seq_indent));
                    if !value_is_null {
                        self.defer_item_value_to_later_lines(
                            seq_indent,
                            false,
                            item_wrapper_bp,
                            Some(above_dash),
                        );
                        value_on_a_later_line = true;
                    }
                }
                if !value_on_a_later_line
                    && matches!(self.peek(), Some(b'&' | b'!'))
                    && !self.node_properties_prefix_mapping_entry()
                {
                    let properties_start = self.pos;
                    self.parse_node_properties()?;
                    if self.at_line_end() && !self.following_value_is_null(seq_indent) {
                        value_on_a_later_line = true;
                        // A comment after the properties goes with the value that follows, as for
                        // any property-prefixed item (#784).
                        self.defer_line_comment();
                    } else {
                        self.pos = properties_start;
                    }
                }

                if !value_on_a_later_line && !self.at_line_end() {
                    // #6379: and a property before that `?` (`? - &t ? x`) is refused.
                    if self.properties_precede_explicit_key_indicator() {
                        return Err(self.err_unexpected_char(
                            self.pos,
                            "mapping keys are not allowed in this context",
                        ));
                    }
                    // Parse item value
                    if self.peek() == Some(b'?') && Self::is_ws_break_or_eoi(self.peek_at(1)) {
                        // `? - ? x`: the first item is a compact mapping whose first entry is an
                        // explicit key of its own (#6340), as the node after `? ` is (#6322): the
                        // outer key waits behind the inner one's. A later item is dispatched by
                        // `parse_sequence_item`, which finds the outer key already waiting.
                        self.parse_nested_explicit_key(false, Some(owner_depth))?;
                    } else if self.looks_like_mapping_entry() {
                        // The key's real column, not a hardcoded `indent + 3`
                        // - `- ` is already skipped above, so `self.pos` sits
                        // at the key. `indent + 3` was wrong even for the
                        // ordinary single-space case (`? - a: 1`): `?` + ` `
                        // + `-` + ` ` is 4 columns, not 3, so every field
                        // after the compact mapping's first silently landed
                        // at the wrong indent (#877 follow-up).
                        self.parse_compact_mapping_entry(self.current_column())?;
                    } else {
                        self.parse_value(seq_indent)?;
                    }
                }
                // Nothing opened inside the item: close it now. A container it opened leaves the
                // item's frame on the stack for the dedent to close, in order.
                if !value_on_a_later_line && self.indent_stack.len() == item_depth + 1 {
                    self.indent_stack.pop();
                    self.pop_type();
                    self.write_bp_close(); // close item
                }
            }
            // `? ? a`: the node after `? ` is a compact mapping whose first entry is an explicit
            // key of its own, so the outer key *is* that mapping (#6322; yq reads `? ? a\n: c` as
            // `{"": "c"}`, where this used to read the plain scalar `? a`). The inner `?` is the
            // key now waiting for a value; the outer one waits behind it until the inner mapping
            // closes ([`Self::pending_explicit_key_below`]).
            Some(b'?') if Self::is_ws_break_or_eoi(self.peek_at(1)) => {
                self.parse_nested_explicit_key(key_has_properties, Some(owner_depth))?;
                return Ok(());
            }
            // `? [1, 2]: value`: the node after `? ` is a compact mapping keyed by
            // the flow collection, so the explicit key *is* that mapping and the
            // entry has no value (yq: `{"":null}`), exactly as `? k: v` below
            // (#1188). Ahead of the plain `[`/`{` arms, which read a collection
            // that stands alone.
            Some(b'[' | b'{') if self.looks_like_flow_keyed_mapping_entry() => {
                self.parse_compact_mapping_entry(self.current_column())?;
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
                self.explicit_key_node_closed = true;
            }
            Some(b'{') => {
                // Flow mapping as key -- see the `[` arm above (#902).
                self.write_bp_open();
                self.parse_flow_mapping()?;
                self.reject_trailing_flow_content(true)?;
                self.write_bp_close();
                self.explicit_key_node_closed = true;
            }
            Some(b'|' | b'>') => {
                // Block scalar as key
                self.claim_floated_comment_for_key();
                self.parse_block_scalar(indent)?;
                self.reject_line_deeper_than_block_after_block_scalar()?;
            }
            _ if bare_colon_key => {
                // #6325: the compact mapping in the key is an entry with no key and no `?`, which
                // yq refuses like the keyless `:` of #6288 (`? : x`, `- ? : x`, corpus M2N8/00).
                return Err(self.err_unexpected_char(self.pos, "did not find expected key"));
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
                self.claim_floated_comment_for_key();
                self.write_bp_open();
                self.parse_double_quoted()?;
                self.reject_content_after_inline_value()?;
                self.set_bp_text_end(self.pos);
                self.write_bp_close();
                self.explicit_key_node_closed = true;
            }
            Some(b'\'') => {
                // Single-quoted key
                self.claim_floated_comment_for_key();
                self.write_bp_open();
                self.parse_single_quoted()?;
                self.reject_content_after_inline_value()?;
                self.set_bp_text_end(self.pos);
                self.write_bp_close();
                self.explicit_key_node_closed = true;
            }
            _ => {
                // Unquoted scalar key
                self.claim_floated_comment_for_key();
                self.write_bp_open();
                let end_pos = self.parse_unquoted_value_with_indent(indent);
                self.explicit_key_scalar_start = self.last_plain_scalar.map(|span| span.start);
                self.check_plain_scalar()?;
                // The scan stops at a `: ` of a later line (`? a⏎  b: 1`): that is a mapping
                // inside the key's node, which yq refuses, not the entry's value (#5086). A `: `
                // on the `?` line is the compact mapping arm above.
                self.reject_content_after_inline_value()?;
                self.set_bp_text_end(end_pos);
                self.write_bp_close();
            }
        }

        // Mark that we have an explicit key waiting for a value, unless a key nested in this one's
        // own content is the one waiting (`? - ? x`, #6340): it took the value first, and this one
        // is behind it ([`Self::pending_explicit_key_below`]).
        if !self
            .pending_explicit_key
            .is_some_and(|nested| nested > owner_depth)
        {
            self.set_pending_explicit_key(owner_depth, false);
        }

        Ok(())
    }

    /// What yq refuses about the `-` at the cursor that opens a block sequence entry, whichever site
    /// reads it (#6414): a property in front of it on its line (`&t - x`, `a: &t - x`, `- !!seq - x`)
    /// is no node's property, and yq stops with `block sequence entries are not allowed in this
    /// context` wherever the property stands (a property on a line of its own above the `-` is fine,
    /// and ends at the line break, not here); and a tab among the blanks after it is no separation
    /// to go-yaml (`-⇥x`, `- ⇥x`). Yq only: the library loader keeps both readings.
    fn reject_misplaced_sequence_indicator(&self) -> Result<(), YamlError> {
        if !self.yq_mode {
            return Ok(());
        }
        if self.properties_end == self.pos {
            return Err(self.err_unexpected_char(
                self.pos,
                "block sequence entries are not allowed in this context",
            ));
        }
        self.reject_tab_after_indicator()
    }

    /// A tab among the blanks after the `?`, `:` or `-` at the cursor (#6379, #6414): go-yaml skips no tab
    /// where a simple key is allowed, which is right after any of the indicators, so it stops with `found character
    /// that cannot start any token` (`?⇥x`, `:⇥b`, `? ⇥x`, and a tab with nothing after it). The blanks
    /// may end at a `#`, which its comment scanner takes (`?⇥# c`). The one decision for the key's and
    /// the value's indicator, and for the nested positions that reach them in yq (#6340). The library loader
    /// keeps its older refusal of a tab right after the `?` of a nested key, in `parse_nested_explicit_key`.
    fn reject_tab_after_indicator(&self) -> Result<(), YamlError> {
        debug_assert!(matches!(self.peek(), Some(b'?' | b':' | b'-')));
        // Only for yq: YAML 1.2 reads the tab as separation, which the library loader keeps.
        if !self.yq_mode {
            return Ok(());
        }
        let mut at = self.pos + 1;
        let mut tab = None;
        while let Some(&byte) = self.input.get(at) {
            match byte {
                b' ' => {}
                b'\t' => {
                    tab.get_or_insert(at);
                }
                _ => break,
            }
            at += 1;
        }
        // The blanks may end at a `#` after `?` or `:`, which the comment scanner takes (`?⇥# c`); after
        // a `-` go-yaml stops at the tab all the same (`-⇥# c`, #6414).
        let comment_follows = self.peek() != Some(b'-') && self.input.get(at) == Some(&b'#');
        match tab {
            Some(tab) if !comment_follows => {
                Err(self.err_unexpected_char(tab, "found character that cannot start any token"))
            }
            _ => Ok(()),
        }
    }

    /// Whether the node properties at the cursor (`&a`, `!!t`, either order, on this line) are followed by
    /// an explicit key indicator (#6379). yq stops with `mapping keys are not allowed in this context`:
    /// a `?` after a property is no key, wherever the property stands. Restores the cursor.
    fn properties_precede_explicit_key_indicator(&mut self) -> bool {
        let saved = self.pos;
        let mut found = false;
        while matches!(self.peek(), Some(b'&' | b'!')) {
            let end = if self.peek() == Some(b'&') {
                super::simd::parse_anchor_name(self.input, self.pos + 1)
            } else {
                scan_tag_extent(self.input, self.pos).0
            };
            if end <= self.pos {
                break; // patchcov: coverage tolerate-line reason="unreachable: both scanners consume at least the `&` or `!` they start on, so `end` is past the cursor; kept so a scanner that returned early could not loop forever (#6379)"
            }
            self.pos = end;
            self.skip_inline_whitespace();
            found = self.peek() == Some(b'?') && Self::is_ws_break_or_eoi(self.peek_at(1));
        }
        self.pos = saved;
        found
    }

    /// `key: ? x`: an explicit key indicator straight after an implicit key's colon, on its line. It cannot
    /// open a mapping there (that is `key:` / `  ? x`, or `: ? x` after an explicit key's own colon), and
    /// yq stops with `mapping keys are not allowed in this context`; the line was read as the plain scalar
    /// `? x` (#6379). A property between the two (`key: &a ? x`) does not change it, and `?x` and `?? x`
    /// stay plain scalars.
    fn reject_explicit_key_after_implicit_key(&self) -> Result<(), YamlError> {
        if self.peek() == Some(b'?') && Self::is_ws_break_or_eoi(self.peek_at(1)) {
            return Err(
                self.err_unexpected_char(self.pos, "mapping keys are not allowed in this context")
            );
        }
        Ok(())
    }

    /// An explicit key that is the first entry of a compact mapping standing where a node is expected:
    /// the node after `? ` (#6322), after the `- ` that opens a key sequence (#6340) and after an
    /// explicit value's `: ` (#6340). The one place these three sites share, so a refusal added to
    /// one cannot be missing from the others.
    ///
    /// `has_properties`: a property on the line before the `?` (`? !!str ? a`, `: &t ? x`) is refused
    /// by yq (`mapping key is not allowed in this context`), which a property on a compact mapping's
    /// own first entry is not. A tab straight after the indicator is refused too, go-yaml skipping no
    /// tab where a simple key is allowed (`found character that cannot start any token`).
    ///
    /// `outer_owner_depth` is the mapping of the explicit key whose content this is, still to get its
    /// value: it waits behind the inner key ([`Self::pending_explicit_key_below`]). `None` when the
    /// content is a value, whose key has had its `:` already.
    fn parse_nested_explicit_key(
        &mut self,
        has_properties: bool,
        outer_owner_depth: Option<usize>,
    ) -> Result<(), YamlError> {
        if has_properties {
            return Err(
                self.err_unexpected_char(self.pos, "mapping key is not allowed in this context")
            );
        }
        // The library loader refuses it here as it always has (#6340); yq's own rule is
        // `reject_tab_after_indicator`, which `parse_explicit_key` applies.
        if !self.yq_mode && self.peek_at(1) == Some(b'\t') {
            return Err(self
                .err_unexpected_char(self.pos + 1, "found character that cannot start any token"));
        }
        self.parse_explicit_key(self.current_column())?;
        if let Some(owner_depth) = outer_owner_depth {
            self.wait_behind_pending_explicit_key(owner_depth, false);
        }
        Ok(())
    }

    /// Parse an explicit value (`: value` after explicit key).
    fn parse_explicit_value(&mut self, indent: usize) -> Result<(), YamlError> {
        self.reject_tab_after_indicator()?;

        // Same ambiguous-gap error as parse_mapping_entry (#901).
        self.check_mapping_under_mapping_gap(indent)?;

        // #6322: a `:` shallower than the nested key still waiting for its value (`? ? a\n: c`)
        // is the outer key's: the inner mapping closes first, its key taking a null, and the outer
        // key is the one waiting. Only when a nested key waits: for any other key a shallower `:`
        // is misaligned with its `?`, which the check below refuses.
        while let Some(&(outer_owner, _)) = self.pending_explicit_key_below.last() {
            if !self.pending_explicit_key.is_some_and(|owner_depth| {
                self.indent_stack
                    .get(owner_depth - 1)
                    .is_some_and(|&at| at > indent)
            }) {
                break;
            }
            let open = self.indent_stack.len();
            self.close_deeper_indents_above(indent, outer_owner);
            if self.indent_stack.len() == open {
                break;
            }
        }

        // Same ambiguous-column error for a `:` that doesn't match its own
        // `?` (#1010).
        self.require_consistent_indentation(self.explicit_value_matches_key_indent(indent))?;

        // Close deeper structures, but keep the mapping at this indent open. Everything the
        // key pushed is deeper than the `?` (its content starts past it), so a key sequence
        // left open at the `?`'s column + 1 is closed too (#5086), and so is one at the `?`'s
        // own column (`?⏎- a⏎: v`, #6370), which is no deeper than it.
        self.close_deeper_indents(indent);
        self.close_key_sequence_at(indent);

        // This value is for the pending explicit key
        self.clear_pending_explicit_key();

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
            // A block sequence at the `:`'s own indent is the value, as it is for an ordinary
            // `key:` entry (#4736).
            //
            // #6321: also with no property. A bare `:` whose value is null needs the same node,
            // or the key is paired with whatever the next sibling is (`? a` / `:` / `? b` / `: 1`
            // was `{a: b}`) or has no value at all and the entry is dropped (`? a` / `:` was `{}`).
            if self.following_value_is_null_unless_sequence_at(indent) {
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
        //
        // Checked first, so the collection is read as the compact mapping's key
        // rather than consumed as a value (#1188).
        if self.looks_like_flow_keyed_mapping_entry() {
            self.parse_compact_mapping_entry(self.current_column())?;
            return Ok(());
        }
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
            // `: ? x`: the node after `: ` is a compact mapping whose first entry is an explicit key
            // of its own, as the node after `- ` is (#339) and the node after `? ` is (#6322). The
            // `?` indicator wins over a `: ` later on the line, as in the main loop's dispatch.
            // Properties on the line before it (`: &t ? x`) are refused by yq (`mapping key is not
            // allowed in this context`), which a property on a compact mapping's own first entry
            // is not.
            Some(b'?') if Self::is_ws_break_or_eoi(self.peek_at(1)) => {
                self.parse_nested_explicit_key(had_property, None)?;
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
                self.reject_content_after_inline_value()?;
                self.reject_deeper_line_after_value()?;
                self.set_bp_text_end(self.pos);
                self.write_bp_close();
            }
            // No `json_strict` check needed in this match: `parse_explicit_value`
            // has exactly one caller, `parse_block_node`'s `:` explicit-value-
            // indicator arm, which the chokepoint above already refuses
            // unconditionally under `json_strict` (`:` has no JSON spelling) --
            // this function is never entered at all once `json_strict` is set
            // (#2778 review; confirmed empirically).
            Some(b'\'') => {
                self.set_ib();
                self.write_bp_open();
                self.parse_single_quoted()?;
                self.reject_content_after_inline_value()?;
                self.reject_deeper_line_after_value()?;
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
                self.check_plain_scalar()?;
                self.set_bp_text_end(end_pos);
                self.write_bp_close();
            }
        }

        Ok(())
    }

    /// Parse an inline scalar value (on the same line as the key).
    /// Returns the end position of the scalar content.
    /// No `json_strict` check needed anywhere in this match: both callers
    /// `json_strict` still applies here: `parse_compact_mapping_entry`'s own
    /// caller is dead under `json_strict` (behind the dash-rejection above),
    /// but `parse_mapping_entry`'s `key: value` form is not -- real yq's
    /// `-p json` (no `--slurp`) reads a *stream* of top-level values, so
    /// `true: 1`/`"a": 1` are two valid, independent streamed values rather
    /// than an invalid bare mapping (#2778, reverted the opposite
    /// assumption after checking live; full streaming is unimplemented,
    /// tracked separately as #2839) -- but each such value is still subject
    /// to the same grammar `goccy/go-json` enforces on any value, confirmed
    /// live: `true: 'v'`/`true: True`/`true: .5` all error on the second
    /// value in real yq too.
    fn parse_inline_value(&mut self, min_indent: usize) -> Result<usize, YamlError> {
        let quoted = matches!(self.peek(), Some(b'"' | b'\''));
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
                self.check_plain_scalar()?;
                self.check_json_strict_scalar(start, end)?;
                end
            }
        };
        self.reject_content_after_inline_value()?;
        if quoted {
            self.reject_deeper_line_after_value()?;
        }
        Ok(end)
    }

    /// A mapping value's scalar must be the last thing on its line: whatever
    /// follows it other than blanks and a comment is an error, not text to
    /// drop (#4463). `a: 1 b: 2` is yq's "mapping values are not allowed in
    /// this context" -- the plain scalar stops at the `: ` and the rest of
    /// the line used to be skipped, so `{"a":"1 b"}` came back with `2` gone
    /// and a write rewrote the line without it. Quoted values follow the same
    /// rule (`a: "x" y: z`), as do the second line of a multi-line plain
    /// scalar and a trailing `:` (`a: b:`).
    fn reject_content_after_inline_value(&self) -> Result<(), YamlError> {
        // The common case: the scalar ends the line.
        match self.peek() {
            None => return Ok(()),
            Some(b) if Self::is_break(b) => return Ok(()),
            Some(_) => {}
        }
        let mut i = self.pos;
        while self
            .input
            .get(i)
            .is_some_and(|&b| Self::is_inline_whitespace(b))
        {
            i += 1;
        }
        match self.input.get(i) {
            None => Ok(()),
            Some(&b) if Self::is_break(b) => Ok(()),
            // A comment, which only counts after a blank.
            Some(b'#') if i > self.pos || Self::is_inline_whitespace(self.input[self.pos - 1]) => {
                Ok(())
            }
            Some(_) => {
                Err(self.err_unexpected_char(i, "mapping values are not allowed in this context"))
            }
        }
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
                self.reject_content_after_inline_value()?;
                self.reject_deeper_line_after_value()?;
                self.set_bp_text_end(self.pos);
                self.write_bp_close();
            }
            // No `json_strict` check needed anywhere in this match: every
            // caller of `parse_value` that's reachable under `json_strict`
            // (`parse_block_node`'s `{`/`[` arm, always) passes a `{`/`[`
            // peek, which `try_dispatch_flow_or_block_value` above already
            // handles and returns from -- this match's own arms are reached
            // only via the anchor/dash edge cases and callers the chokepoint
            // and dash-rejection above already exclude under `json_strict`
            // (#2778 review; confirmed empirically, including the `- &a &b`
            // double-anchor path below).
            Some(b'\'') => {
                self.set_ib();
                self.write_bp_open();
                self.parse_single_quoted()?;
                self.reject_content_after_inline_value()?;
                self.reject_deeper_line_after_value()?;
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
                self.check_plain_scalar()?;
                self.set_bp_text_end(end_pos);
                self.write_bp_close();
                // The scalar stops at a `: ` and what follows is no node (#5064), so `- a` /
                // `  z: 1` is yq's "mapping values are not allowed in this context".
                self.reject_content_after_inline_value()?;
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

    /// [`Self::skip_flow_whitespace`] where a comment on the line the cursor starts on, after the
    /// element at `owner_bp_pos`, is that element's own line comment (#6334): `[a, # c` / `  b]`.
    /// A comment on a line of its own is held for the element that follows it, or for the
    /// collection's last element when the closing bracket does (`take_flow_head`,
    /// `give_flow_foot`). What a comment on the line of a comma that starts its own line is depends
    /// on `on_comma_line`.
    fn skip_flow_whitespace_noting_line_comment(
        &mut self,
        owner_bp_pos: Option<usize>,
        on_comma_line: CommaLine,
    ) {
        let mut same_line = true;
        while let Some(b) = self.peek() {
            match b {
                b' ' | b'\t' => self.advance(),
                b'\n' | b'\r' => {
                    same_line = false;
                    self.advance();
                }
                b'#' => {
                    let start = self.pos;
                    let owner = owner_bp_pos.filter(|_| same_line);
                    match (owner, on_comma_line) {
                        (Some(owner), CommaLine::Own) if same_line => {
                            self.maybe_capture_line_comment(owner);
                            self.skip_to_eol();
                        }
                        _ => {
                            self.skip_to_eol();
                            let range = (start as u32, self.pos as u32);
                            if !same_line || (on_comma_line == CommaLine::Head) {
                                self.flow_own_comments.push(range);
                            }
                        }
                    }
                    same_line = false;
                }
                _ => break,
            }
        }
    }

    /// What a comment on the line of the comma at `comma_pos` is, after the element at `owner` that
    /// ended at `element_end` (#6334): the element's line comment, unless own-line comments came
    /// before the comma (yq reads it as nothing), or the comma starts a line of its own and the
    /// element has a line comment already (the head of what follows).
    fn comma_line(&self, owner: Option<usize>, element_end: usize, comma_pos: usize) -> CommaLine {
        if !self.flow_own_comments.is_empty() {
            return CommaLine::Drop;
        }
        let starts_line = self.input[element_end.min(comma_pos)..comma_pos]
            .iter()
            .any(|&b| Self::is_break(b));
        // Looked up for a comma that starts a line only, which keeps the map off the common path.
        if starts_line
            && owner.is_some_and(|bp| self.comments.get(&bp).is_some_and(|c| !c.line.is_empty()))
        {
            CommaLine::Head
        } else {
            CommaLine::Own
        }
    }

    /// Give the comments held on lines of their own to the element about to open at `bp` as its
    /// head (#6334).
    fn take_flow_head(&mut self, bp: usize) {
        if self.flow_own_comments.is_empty() {
            return;
        }
        let held = core::mem::take(&mut self.flow_own_comments);
        let head = &mut self.comments.entry(bp).or_default().head;
        // Ranges ascend, so one at or before the last recorded was recorded already.
        for range in held {
            if head.last().map_or(true, |last| last.1 <= range.0) {
                head.push(range);
            }
        }
    }

    /// Give the comments held on lines of their own to the last element, at `bp`, as its foot
    /// when the closing bracket follows them; with no element to own them they are dropped, as yq
    /// drops them (#6334).
    fn give_flow_foot(&mut self, last: Option<usize>) {
        let held = core::mem::take(&mut self.flow_own_comments);
        let Some(bp) = last else { return };
        if held.is_empty() {
            return;
        }
        let foot = &mut self.comments.entry(bp).or_default().foot;
        for range in held {
            if foot.last().map_or(true, |last| last.1 <= range.0) {
                foot.push(range);
            }
        }
    }

    /// Check if current position starts an implicit mapping entry in flow context.
    /// Returns true if there's a `key : value` pattern (colon followed by space).
    /// This is used to detect patterns like `[ YAML : separate ]`.
    fn looks_like_flow_mapping_entry(&self) -> bool {
        let mut i = self.pos;

        // Skip leading node properties: an aliased key IS the key (`*x: v`,
        // #409), while an anchored key (`&x k: v`, corpus case CN3R) and a
        // tagged one (`!Foo {a: 1}`, #4081) just prefix it - what determines
        // whether this item is a pair is what follows the properties, not
        // the indicators. Uses the same scanners the real parse uses, so this
        // can't drift from what will actually be consumed (#106).
        while i < self.input.len() && matches!(self.input[i], b'&' | b'*' | b'!') {
            if self.input[i] == b'!' {
                let (end, ok) = scan_tag_extent(self.input, i);
                if !ok {
                    // An unterminated verbatim tag: let the real parse report it.
                    return false;
                }
                i = end;
            } else {
                let is_alias = self.input[i] == b'*';
                let name_start = i + 1;
                let name_end = simd::parse_anchor_name(self.input, name_start);
                if name_end == name_start {
                    // Empty name - not a valid anchor/alias; let the real parse
                    // report it.
                    return false;
                }
                i = name_end;
                if is_alias {
                    while i < self.input.len() && matches!(self.input[i], b' ' | b'\t') {
                        i += 1;
                    }
                    // The alias name is the whole key; nothing else to scan.
                    return i < self.input.len() && self.input[i] == b':';
                }
            }
            while i < self.input.len() && matches!(self.input[i], b' ' | b'\t') {
                i += 1;
            }
            // A property prefixes the actual key, which still needs to be
            // scanned below - it may be quoted, a container, or a plain
            // scalar (or another property).
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
            // After the flow, check for colon - can be adjacent (no space required),
            // or follow spaces on the same line, as after a quoted key (`[{a: 1} : v]`,
            // #4081: it parsed with a tag before it only because the scan below took
            // the `: ` inside the braces for the pair's colon).
            while i < self.input.len() && matches!(self.input[i], b' ' | b'\t') {
                i += 1;
            }
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
                    // In flow context, colon must be followed by a blank, a line break,
                    // end-of-input or a flow indicator to be the pair's colon. The break
                    // is the same stop `parse_flow_unquoted_value` makes (#1960), so the
                    // two agree on `[a:\n 1]`; any other `:` is key content (`[x:y: 1]`).
                    if Self::is_ws_break_or_eoi(next) {
                        return true;
                    }
                    // #4558: a `:` straight before a flow indicator is key content in yq, not
                    // the pair's colon (`[a:]` is the one item `a:`, `{a:}` the key `a:`), so
                    // the item is not a pair.
                    if matches!(next, Some(b',' | b']' | b'}')) {
                        return false;
                    }
                    i += 1;
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

    /// A flow node that is not a key: node properties (`&a`, `!tag`), then an
    /// alias, a flow collection or a scalar. Used for a flow sequence's standalone
    /// item and for the value of a `key: value` pair (the `:` and the whitespace
    /// after it already consumed).
    ///
    /// One definition for a flow sequence's items, a flow mapping's values and both
    /// single-pair forms of a flow sequence (`[k: v]`, `[? k : v]`). The pair forms were copies that opened a
    /// BP node for the value and then called `parse_flow_sequence` /
    /// `parse_flow_mapping`, which open one of their own: a sequence value came
    /// back wrapped in a second sequence and a mapping value came back empty, and
    /// an alias value read as a plain scalar (#4091). A flow collection carries
    /// its own node and needs no wrapper (#332); only a scalar is wrapped.
    fn parse_flow_value_node(&mut self) -> Result<(), YamlError> {
        // A property prefix on the value
        self.parse_flow_node_properties()?;

        // A standalone alias
        if self.peek() == Some(b'*') {
            return self.parse_alias();
        }
        match self.peek() {
            Some(b'[') => self.parse_flow_sequence(),
            Some(b'{') => self.parse_flow_mapping(),
            _ => {
                // Scalar value - wrap in BP
                self.set_ib();
                self.write_bp_open();
                let end = self.parse_flow_scalar()?;
                self.set_bp_text_end(end);
                self.write_bp_close();
                Ok(())
            }
        }
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
            // `[? k : ? v]`: refused by yq as `[k: ? v]` is (#6379).
            self.reject_explicit_key_after_implicit_key()?;

            // Parse value (if present before , or ])
            if !matches!(self.peek(), Some(b',' | b']' | b'}') | None) {
                self.parse_flow_value_node()?;
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
        let key_start = self.pos;
        self.set_ib();
        self.write_bp_open();
        if !self.parse_flow_key()? {
            // A complex key (a nested flow container) opened its own BP nodes
            // and carries its own end; recording one here would clobber the
            // innermost of them (#332).
            self.set_bp_text_end(self.pos);
        }
        self.write_bp_close();

        // #6208: a simple key has no line break between its first character and its `:`
        // (`[{x⏎}: 1]`, `["x⏎ y": 1]`, `[[a,⏎ b]: 1]`), as in a flow mapping's key; the break
        // before the `:` (`["x"⏎ : 1]`) is refused below, where only blanks may precede it.
        let key_end = self.pos;

        // Skip whitespace before colon
        self.skip_inline_whitespace();

        if self.peek() == Some(b':')
            && self.input[key_start..key_end]
                .iter()
                .any(|&b| matches!(b, b'\n' | b'\r'))
        {
            return Err(self.err_unexpected_char(key_start, "expected ',' or ']' after a key"));
        }

        // Expect and skip colon
        if self.peek() != Some(b':') {
            return Err(
                self.err_unexpected_char(self.pos, "expected ':' in implicit flow mapping entry")
            );
        }
        self.advance();
        self.skip_flow_whitespace();
        self.reject_explicit_key_after_implicit_key()?;

        // Parse value (if present before , or ])
        if !matches!(self.peek(), Some(b',' | b']' | b'}') | None) {
            self.parse_flow_value_node()?;
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
                self.reject_deeper_line_after_value()?;
                Ok(true)
            }
            Some(b'{') => {
                self.parse_flow_mapping()?;
                self.reject_trailing_flow_content(permit_colon_terminator)?;
                self.reject_deeper_line_after_value()?;
                Ok(true)
            }
            Some(b'|' | b'>') => {
                self.parse_block_scalar(indent)?;
                self.reject_line_deeper_than_block_after_block_scalar()?;
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

        // Skip `[`; a comment on a line of its own after it heads the first element (#6334), one
        // on its line is dropped. JSON holds none.
        self.advance();
        if self.json_strict {
            self.skip_flow_whitespace();
        } else {
            self.skip_flow_whitespace_noting_line_comment(None, CommaLine::Own);
        }

        // Parse items
        let mut first = true;
        // The element the cursor is after, which a comment on the same line belongs to (#6334).
        let mut prev_element: Option<usize> = None;
        // The last element read and where it ended, for a foot comment and for a comma that starts
        // a line (#6334).
        let mut last_element: Option<usize> = None;
        let mut element_end = self.pos;
        while self.peek() != Some(b']') {
            if self.peek().is_none() {
                return Err(YamlError::UnexpectedEof {
                    context: "flow sequence",
                });
            }

            // #2279: a leading `,` (`[,1]`, and `[,]`'s first pass) is an
            // element YAML reads as an empty scalar and JSON has no spelling
            // for at all. Real yq (go-yaml) rejects it for YAML input too --
            // `,` cannot start a flow node (#2886) -- so this is not gated on
            // `json_strict`.
            if first && self.peek() == Some(b',') {
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
                let on_comma_line = self.comma_line(prev_element, element_end, comma_pos);
                self.advance(); // Skip `,`
                self.skip_flow_whitespace_noting_line_comment(prev_element, on_comma_line);

                // #2279: `[1,,2]` -- a second `,` where an element belongs
                // (#2886: in YAML input too, as in go-yaml).
                if self.peek() == Some(b',') {
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
                    // yq reads comments on lines of their own before a trailing comma as nothing at
                    // all, and with them those after the comma (#6334).
                    if on_comma_line == CommaLine::Drop {
                        self.flow_own_comments.clear();
                    }
                    break;
                }
            }
            first = false;
            prev_element = None;

            // #4558: an item that opens with a `:` and a blank (`[: x]`, `[a, : x]`) is an empty
            // key yq rejects ("did not find expected node content"); a `:` straight before
            // anything else is item text (`[:]`, `[:x]`).
            if self.peek() == Some(b':') && Self::is_ws_break_or_eoi(self.peek_at(1)) {
                return Err(self.err_unexpected_char(self.pos, "expected value in flow sequence"));
            }

            // Check for explicit key `? key : value` in flow context. Both
            // lookaheads scan up to the element's own length, not O(1), so
            // they're hoisted into locals and reused below rather than
            // re-run by the json_strict check and the dispatch separately.
            let element_bp = self.bp_pos;
            let is_explicit_key = self.looks_like_explicit_flow_key();
            let is_mapping_entry = is_explicit_key || self.looks_like_flow_mapping_entry();
            // A single pair is a mapping of its own, whose key owns the comments (#6334).
            let marked_bp = element_bp + usize::from(is_mapping_entry);
            self.take_flow_head(marked_bp);
            last_element = Some(marked_bp);
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
                self.parse_flow_value_node()?;
                prev_element = Some(element_bp);
            }
            element_end = self.pos;
            self.skip_flow_whitespace_noting_line_comment(prev_element, CommaLine::Own);
        }
        self.give_flow_foot(last_element);

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

        // Skip `{`; as after `[`, a comment on a line of its own heads the first entry (#6334).
        self.advance();
        if self.json_strict {
            self.skip_flow_whitespace();
        } else {
            self.skip_flow_whitespace_noting_line_comment(None, CommaLine::Own);
        }

        // #2777: real yq's own JSON-sourced object grammar is not YAML's
        // flow-mapping grammar at all -- see `parse_json_strict_flow_mapping_entries`'s
        // own doc comment for the derivation and why it is a wholly separate
        // loop rather than more `if self.json_strict` branches threaded
        // through the one below.
        if self.json_strict {
            self.parse_json_strict_flow_mapping_entries()?;
        } else {
            self.parse_yaml_flow_mapping_entries()?;
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

    /// #2777: real yq's own JSON-sourced object grammar, traced from
    /// `goccy/go-json`'s `Stream.Token()`/`PrepareForDecode` (the machinery
    /// `CandidateNode.UnmarshalJSON` actually uses for an object body) --
    /// not YAML's flow-mapping grammar at all, which is why this is a
    /// wholly separate loop rather than more `if self.json_strict` branches
    /// threaded through [`Self::parse_yaml_flow_mapping_entries`]:
    ///
    /// - Before each key, whitespace and *any* run of `,`/`:` is skipped --
    ///   both are pure separators to yq's token scanner here, so `{,}`,
    ///   `{,"a":1}`, `{"a":1,,"b":2}`, `{:1}` (the leading `:` is itself
    ///   skipped as a separator, leaving a bare numeric "key" -- see below)
    ///   all collapse the same way.
    /// - A key must be a JSON string (`"..."`); yq's own code path does an
    ///   unchecked cast of the decoded token to `string`, so a
    ///   well-formed *non-string* JSON literal there (`{1:1}`, `{true:1}`,
    ///   `{null:1}`) is a Go interface-conversion **panic** (exit 2) in the
    ///   reference. Per ADR-0018 rule 4(c) ("matching would take the host
    ///   process down"), this reproduces the *rejection*, as a clean parse
    ///   error, not the crash. Anything that isn't even a well-formed JSON
    ///   token at all (`{abc:1}`, `{'a':1}`) is an ordinary scanner-level
    ///   error in the reference too (`invalid character ... as token`), so
    ///   it gets an ordinary parse error here as well.
    /// - After the key, whitespace and *at most one* `,`/`:` is skipped --
    ///   `PrepareForDecode`'s own, stricter rule -- so `{"a" 1}`/`{"a":1}`/
    ///   `{"a",1}` all reach the value, but `{"a"::1}`/`{"a",,1}` (two
    ///   separators) do not.
    /// - The value must then start on a genuine JSON value byte: `{`, `[`,
    ///   `"`, or a token matched by [`Self::json_strict_token_end`]
    ///   (number/`true`/`false`/`null`) -- reusing the exact same
    ///   token-boundary rule the key check above uses, since both are the
    ///   same underlying scanner in the reference. A quoted-string or
    ///   container value goes through the ordinary flow-value machinery;
    ///   nested arrays keep their own, unrelated strict `[...]` delimiter
    ///   rules (#2279/#2778) unchanged -- yq's object leniency has no array
    ///   counterpart (`json.Unmarshal`'s ordinary strict slice decoder
    ///   parses a nested array, confirmed live: `{"a":[1 2]}` errors).
    ///
    /// **Deliberately not reproduced**: the reference's token scanner
    /// treats *every* bracket character (`{`, `}`, `[`, `]`) as a `Delim`
    /// that ends the *current* object's key-reading loop the instant one
    /// is seen -- including a `]` or a second `{`/`[` where a key was
    /// expected, with no requirement that it be *this* object's own closing
    /// brace (confirmed live: `{"a":1 ]}` -> `{"a":1}`; `{[1]:2}` -> `{}`).
    /// What (if anything) is left unconsumed in that case is then read as
    /// the *next* top-level document by yq's own stream evaluator, whose
    /// own trailing-content handling is itself inconsistent (some leftover
    /// shapes silently produce no error at all; others, differing only in
    /// which byte is left over, print a stream-decode error to stderr while
    /// still emitting the correct value and exiting 0). Reproducing that
    /// would mean modelling a second, top-level-only decoder loop with the
    /// same inconsistency, for a shape (a bare bracket standing in for a
    /// key, with no quotes at all) that essentially never occurs in
    /// real-world JSON -- deferred, and this parser instead reports a
    /// ordinary parse error for a `{`/`[`/`]` found where a key is expected,
    /// always failing the whole document cleanly rather than silently
    /// producing a truncated one.
    fn parse_json_strict_flow_mapping_entries(&mut self) -> Result<(), YamlError> {
        loop {
            // Skip whitespace and any run of `,`/`:` before a key, then
            // decide whether one is even there to parse.
            loop {
                self.skip_flow_whitespace();
                match self.peek() {
                    Some(b',' | b':') => self.advance(),
                    Some(b'}') => return Ok(()),
                    None => {
                        return Err(YamlError::UnexpectedEof {
                            context: "flow mapping",
                        });
                    }
                    _ => break,
                }
            }

            // Parse key: must be a JSON string.
            match self.peek() {
                Some(b'"') => self.parse_json_strict_quoted_node()?,
                _ => {
                    return Err(if self.json_strict_token_end().is_some() {
                        self.err_unexpected_char(
                            self.pos,
                            "non-string JSON object key (would panic in real yq)",
                        )
                    } else {
                        self.err_unexpected_char(self.pos, "invalid character as JSON object key")
                    });
                }
            }

            // Skip whitespace and at most one `,`/`:` before the value.
            self.skip_flow_whitespace();
            if matches!(self.peek(), Some(b',' | b':')) {
                self.advance();
                self.skip_flow_whitespace();
            }

            // Parse value.
            match self.peek() {
                Some(b'[') => {
                    self.parse_flow_sequence()?;
                }
                Some(b'{') => {
                    self.parse_flow_mapping()?;
                }
                Some(b'"') => self.parse_json_strict_quoted_node()?,
                _ => {
                    let start = self.pos;
                    let Some(end) = self.json_strict_token_end() else {
                        return Err(
                            self.err_unexpected_char(self.pos, "expected value in JSON object")
                        );
                    };
                    self.check_json_strict_scalar(start, end)?;
                    // `write_bp_open` records `self.pos` as the node's start
                    // -- it must run before `self.pos` moves to `end`, the
                    // same order every other scalar-value arm here follows.
                    self.set_ib();
                    self.write_bp_open();
                    self.pos = end;
                    self.set_bp_text_end(end);
                    self.write_bp_close();
                }
            }
        }
    }

    /// Parse a double-quoted string as a complete, self-contained BP node
    /// -- `set_ib`/`write_bp_open`/`parse_double_quoted`/`set_bp_text_end`/
    /// `write_bp_close`, in the order `set_bp_text_end` requires (recording
    /// `self.pos` as the node's start before `parse_double_quoted` moves it
    /// to the string's end -- `parse_double_quoted` returns the token's own
    /// *length*, `self.pos - start`, not an absolute position, so its
    /// return value is deliberately discarded here in favor of `self.pos`).
    /// Shared by [`Self::parse_json_strict_flow_mapping_entries`]'s key and
    /// value positions, the only two contexts that reach a JSON string.
    fn parse_json_strict_quoted_node(&mut self) -> Result<(), YamlError> {
        self.set_ib();
        self.write_bp_open();
        self.parse_double_quoted()?;
        self.set_bp_text_end(self.pos);
        self.write_bp_close();
        Ok(())
    }

    /// Where a JSON literal token (`true`/`false`/`null`/a number) would end
    /// if scanned from the current position, without consuming it -- the
    /// token-boundary rule real yq's own scanner uses (#2777), which stops
    /// well short of YAML's own unquoted-scalar scanner
    /// ([`Self::parse_flow_unquoted_value`]): at the first byte that cannot
    /// continue the token, not at the next flow delimiter. `None` means the
    /// current position cannot start such a token at all. The returned
    /// range is not yet validated as a *well-formed* number (`1.2.3`,
    /// `--5`) -- callers combine this with [`Self::check_json_strict_scalar`]
    /// for that, the same predicate [`Self::parse_flow_scalar`] already
    /// uses for ordinary scalar values.
    fn json_strict_token_end(&self) -> Option<usize> {
        let rest = self.input.get(self.pos..)?;
        if rest.starts_with(b"true") {
            return Some(self.pos + 4);
        }
        if rest.starts_with(b"false") {
            return Some(self.pos + 5);
        }
        if rest.starts_with(b"null") {
            return Some(self.pos + 4);
        }
        let mut end = self.pos;
        while end < self.input.len() && is_json_strict_number_byte(self.input[end]) {
            end += 1;
        }
        if end == self.pos {
            None
        } else {
            Some(end)
        }
    }

    /// The ordinary YAML flow-mapping entry loop (`parse_flow_mapping_inner`
    /// before #2777 split it out) -- unchanged behavior, still used for
    /// genuine YAML input and reused as-is for a JSON-sourced flow
    /// *sequence's* nested object values are not affected since arrays
    /// always route into [`Self::parse_flow_sequence_inner`], not here.
    fn parse_yaml_flow_mapping_entries(&mut self) -> Result<(), YamlError> {
        // Parse key-value pairs
        let mut first = true;
        // The value the cursor is after, which a comment on the same line belongs to (#6334).
        let mut prev_value: Option<usize> = None;
        // The last entry's key and where the entry ended, as in a flow sequence (#6334).
        let mut last_key: Option<usize> = None;
        let mut entry_end = self.pos;
        while self.peek() != Some(b'}') {
            if self.peek().is_none() {
                return Err(YamlError::UnexpectedEof {
                    context: "flow mapping",
                });
            }

            // #2886: `,` cannot start a flow node, so `{,}` / `{,a: 1}` have no
            // entry for it to separate; go-yaml (real yq) rejects them, where
            // this used to read an empty-string key with a null value.
            if first && self.peek() == Some(b',') {
                return Err(self.err_unexpected_char(self.pos, "expected key in flow mapping"));
            }

            if !first {
                // Expect comma
                if self.peek() != Some(b',') {
                    return Err(
                        self.err_unexpected_char(self.pos, "expected ',' or '}' in flow mapping")
                    );
                }
                let on_comma_line = self.comma_line(prev_value, entry_end, self.pos);
                self.advance(); // Skip `,`
                self.skip_flow_whitespace_noting_line_comment(prev_value, on_comma_line);

                // `{a: 1,,b: 2}` -- a second `,` where an entry belongs.
                if self.peek() == Some(b',') {
                    return Err(self.err_unexpected_char(self.pos, "expected key in flow mapping"));
                }

                // Allow trailing comma
                if self.peek() == Some(b'}') {
                    if on_comma_line == CommaLine::Drop {
                        self.flow_own_comments.clear();
                    }
                    break;
                }
            }
            let first_entry = first;
            first = false;
            prev_value = None;

            // `? ` is a node marker, not key text: consume it *before* the interest bit
            // is planted, or the indicator and the space after it land inside the key's
            // span. The flow-sequence path has always done it in this order (#402).
            //
            // This function only ever runs with `json_strict` unset (#2777's
            // dispatch in `parse_flow_mapping_inner` routes `json_strict`
            // input to `parse_json_strict_flow_mapping_entries` instead,
            // whose own key dispatch already rejects `?` -- it isn't `"`,
            // and it never scans as a `json_strict_token_end` literal
            // either), so the `json_strict`-gated rejection #2778 added here
            // is unreachable; removed rather than left as dead code.
            let explicit = self.looks_like_explicit_flow_key();
            if explicit {
                self.advance();
                self.skip_flow_whitespace();
            } else if self.peek() == Some(b':')
                && (first_entry || Self::is_ws_break_or_eoi(self.peek_at(1)))
            {
                // #4558: yq rejects an empty key -- the first entry opening with `:` (`{: x}`,
                // `{:x}`, `{:}`), or any entry opening with `:` and a blank (`{a: 1, : x}`). A
                // later entry opening with `:` and text is that key (`{a: 1, :x}` is `":x"`).
                // An empty key is spelled `? : x`, handled above.
                return Err(self.err_unexpected_char(self.pos, "expected key in flow mapping"));
            }

            // Parse key
            let key_start = self.pos;
            self.set_ib();
            // The entry's comments on lines of their own are its key's head, as in a block mapping
            // (#6334).
            self.take_flow_head(self.bp_pos);
            last_key = Some(self.bp_pos);
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
                // #6208: an implicit key is a simple key, which has no line break between its
                // first character and its `:` (`{x⏎ y: 1}`, `{x⏎: 1}`, `{"x"⏎ : 1}`, a
                // multi-line quoted or nested-collection key); yq refuses it as `did not
                // find expected ',' or '}'`. Without a `:` the same text is a multi-line plain
                // scalar entry (`{x⏎ y}`), and the explicit `? key` form may span lines.
                if !explicit
                    && self.input[key_start..self.pos]
                        .iter()
                        .any(|&b| matches!(b, b'\n' | b'\r'))
                {
                    return Err(
                        self.err_unexpected_char(key_start, "expected ',' or '}' after a key")
                    );
                }
                self.advance(); // Skip `:`
                                // Comments on lines of their own between the colon and the value are held like any
                                // other: yq gives them to the next entry (or the value's first element), and drops
                                // one on the colon's line (#6334).
                self.skip_flow_whitespace_noting_line_comment(None, CommaLine::Own);
                self.reject_explicit_key_after_implicit_key()?;

                // A value that is absent (`{a: , b: 1}`) has no node for a comment to follow.
                let has_value = !matches!(self.peek(), Some(b',' | b'}'));
                let value_bp = self.bp_pos;
                self.parse_flow_value_node()?;
                prev_value = has_value.then_some(value_bp);
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

            entry_end = self.pos;
            self.skip_flow_whitespace_noting_line_comment(prev_value, CommaLine::Own);
        }
        self.give_flow_foot(last_key);

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
        if self.yq_mode {
            self.reject_unstartable_flow_plain_at(self.pos)?;
        }

        while let Some(b) = self.peek() {
            match b {
                b',' | b'}' | b']' => break,
                b':' if Self::is_ws_break_or_eoi(self.peek_at(1)) => break,
                // #4558: a `:` straight before a flow indicator is part of the key, as in
                // yq (`{a:}` is the key `a:` with no value), like any other `:` that is not
                // followed by a blank.
                b':' => self.advance(),
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
                // #6214: a Unicode break (NEL, LS, PS) ends a line for go-yaml as `\n` does, so the tab
                // rule reads the lines after it. It stays content for the fold (#4913).
                0xC2 | 0xE2 if self.unicode_break_len_at(self.pos) > 0 => {
                    self.pass_unicode_break_in_flow_plain()?;
                }
                b'\n' | b'\r' => {
                    self.reject_tab_in_flow_plain_continuation(self.pos)?;
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
                    // #4558: a `:` is the pair's indicator only when a blank, a break or the end
                    // follows it; before a flow indicator it is key content (`{a :}` is `a :`).
                    let colon_ends_key = |at: usize| {
                        self.input[at] == b':'
                            && Self::is_ws_break_or_eoi(self.input.get(at + 1).copied())
                    };
                    while lookahead < self.input.len() {
                        match self.input[lookahead] {
                            b' ' | b'\t' => lookahead += 1,
                            b':' if !colon_ends_key(lookahead) => {
                                self.advance();
                                break;
                            }
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
                        || (self.input[lookahead] == b':' && colon_ends_key(lookahead))
                        || matches!(self.input[lookahead], b',' | b'}' | b']' | b'\n' | b'\r')
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
                let end = self.parse_flow_unquoted_value()?;
                self.check_json_strict_scalar(start, end)?;
                end
            }
        };
        Ok(end)
    }

    /// Parse an unquoted value in flow context.
    /// Stops at `,`, `}`, `]`, `#` (comment), `:` followed by a blank, a line break or
    /// end-of-input, or newline.
    ///
    /// The `:` stop is `yq`'s rule and YAML 1.2's `ns-plain-char`: `{a: 1 b: 2}` ends the
    /// scalar at `1 b`, and the caller then raises `expected ',' or '}'` on the `:` where
    /// it used to swallow `b: 2` into `a`'s value — which a write (`.a = 9`) then dropped
    /// from the file (#1960). A `:` before a flow indicator is content, exactly as in
    /// [`Self::parse_explicit_flow_unquoted_key`] (#402): `{a: 1:, b: 2}` keeps `"1:"`.
    /// Returns the absolute end position (with trailing whitespace trimmed).
    fn parse_flow_unquoted_value(&mut self) -> Result<usize, YamlError> {
        let start = self.pos;
        if self.yq_mode {
            self.reject_unstartable_flow_plain_at(self.pos)?;
        }

        while let Some(b) = self.peek() {
            match b {
                b',' | b'}' | b']' => break,
                b':' if Self::is_ws_break_or_eoi(self.peek_at(1)) => break,
                b'#' => {
                    // # is a comment if preceded by whitespace
                    if self.pos > start && matches!(self.input[self.pos - 1], b' ' | b'\t') {
                        break;
                    }
                    self.advance();
                }
                // #6214: a Unicode break (NEL, LS, PS) ends a line for go-yaml as `\n` does, so the tab
                // rule reads the lines after it. It stays content for the fold (#4913).
                0xC2 | 0xE2 if self.unicode_break_len_at(self.pos) > 0 => {
                    self.pass_unicode_break_in_flow_plain()?;
                }
                b'\n' | b'\r' => {
                    self.reject_tab_in_flow_plain_continuation(self.pos)?;
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
                    // A continuation line opening with `: ` is a value indicator, not
                    // scalar content (#1960): `{a: 1\n b\n : 2}` is an error in yq.
                    if self.input[lookahead] == b':'
                        && Self::is_ws_break_or_eoi(self.input.get(lookahead + 1).copied())
                    {
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

        Ok(end)
    }

    /// The block indent a tab in a plain scalar's continuation must stay clear of (#6059): the
    /// enclosing block's. `None` for a collection at the document root, which has no block indent
    /// to violate.
    fn tab_indent_limit(&self) -> Option<usize> {
        (self.indent_stack.len() > 1).then(|| self.innermost_block_indent())
    }

    /// #6059: go-yaml refuses a plain scalar that continues on a line whose leading blanks hold a tab at
    /// a column below the enclosing block's indent plus one (`found a tab character that violates
    /// indentation`), in a flow collection as in block context. `break_pos` is the line break after the
    /// scalar's text; a collection at the document root has no block indent to violate.
    fn reject_tab_in_flow_plain_continuation(&self, break_pos: usize) -> Result<(), YamlError> {
        let Some(limit) = self.tab_indent_limit() else {
            return Ok(());
        };
        // The line after the break, and the one after it when that one is empty: the scanners skip an
        // empty line's break along with the previous one, so its successor is never a break they stop
        // at. Every other blank line is a break of its own, which checks its successor in turn.
        let mut line = break_pos;
        for _ in 0..2 {
            let width = self.any_break_len_at(line);
            if width == 0 {
                break;
            }
            let start = line + width;
            let mut at = start;
            while self.input.get(at) == Some(&b' ') {
                at += 1;
            }
            if self.input.get(at) == Some(&b'\t') && at - start <= limit {
                return Err(YamlError::TabIndentation {
                    line: self.current_line(),
                    offset: at,
                });
            }
            line = start;
        }
        Ok(())
    }

    /// The checks of the plain scalar the scanner just read: the byte it started on (#6402), then the
    /// Unicode-break tab (#6214). Every site that reads a block plain scalar calls this one.
    #[inline(always)]
    fn check_plain_scalar(&self) -> Result<(), YamlError> {
        if self.yq_mode {
            if let Some(span) = self.last_plain_scalar {
                self.reject_unstartable_plain_at(span.start)?;
            }
        }
        self.check_unicode_break_tab()
    }

    /// #6402: go-yaml's scanner starts no plain scalar, key or value, block or flow, on `%`, `@` or a
    /// backtick (`found character that cannot start any token`), and a `%` at column 0 is read as a
    /// directive before that, wherever it stands: a line after a document has started
    /// (`a: 1⏎%TAG ! x:`, `!!str⏎%YAML 1.2`) is an error in yq, where this read it as content.
    /// A `%` that continues a plain scalar is content and never reaches here.
    fn reject_unstartable_plain_at(&self, at: usize) -> Result<(), YamlError> {
        if matches!(self.input.get(at), Some(b'%' | b'@' | b'`')) {
            return Err(self.err_unexpected_char(at, "found character that cannot start any token"));
        }
        Ok(())
    }

    /// [`Self::reject_unstartable_plain_at`] for a scalar in flow context, where a `-` followed by
    /// a blank is the block sequence indicator that go-yaml refuses (`block sequence entries are not
    /// allowed in this context`) and no plain scalar starts with (`[- x]`, `[a, - x]`, `{a: - x}`),
    /// where this read `- x` as the text (#6413).
    fn reject_unstartable_flow_plain_at(&self, at: usize) -> Result<(), YamlError> {
        self.reject_unstartable_plain_at(at)?;
        if self.input.get(at) == Some(&b'-')
            && Self::is_ws_break_or_eoi(self.input.get(at + 1).copied())
        {
            return Err(self.err_unexpected_char(
                at,
                "block sequence entries are not allowed in this context",
            ));
        }
        Ok(())
    }

    /// #6214: the Unicode-break tab check for the plain scalar the scanner just read. Inlined to one
    /// flag test, so a document without a tab pays nothing but that; the search is out of line.
    #[inline(always)]
    fn check_unicode_break_tab(&self) -> Result<(), YamlError> {
        if !self.may_hold_unicode_break {
            return Ok(());
        }
        match self.last_plain_scalar {
            Some(span) => {
                self.reject_tab_after_unicode_break(span.start, self.last_plain_content_end)
            }
            None => Ok(()),
        }
    }

    /// #6214: the same refusal for a line that a U+2028, U+2029 or U+0085 ends inside a block plain
    /// scalar or key (`input[start..end]`). The scalar decoders fold those as line breaks (#4913) and
    /// go-yaml scans them as such, so the blanks after one are a continuation line's, but the block
    /// scanners take them for text. The flow scanners stop at the break and call
    /// [`Self::pass_unicode_break_in_flow_plain`] instead.
    fn reject_tab_after_unicode_break(&self, start: usize, end: usize) -> Result<(), YamlError> {
        if !self.may_hold_unicode_break {
            return Ok(());
        }
        let Some(limit) = self.tab_indent_limit() else {
            return Ok(());
        };
        let text = &self.input[start..end.min(self.input.len())];
        let mut at = 0;
        while at < text.len() {
            let width = match text[at..] {
                [0xE2, 0x80, 0xA8 | 0xA9, ..] => 3,
                [0xC2, 0x85, ..] => 2,
                _ => {
                    at += 1;
                    continue;
                }
            };
            let line = at + width;
            let mut tab = line;
            while text.get(tab) == Some(&b' ') {
                tab += 1;
            }
            if text.get(tab) == Some(&b'\t') && tab - line <= limit {
                return Err(YamlError::TabIndentation {
                    line: self.line_at(start + tab),
                    offset: start + tab,
                });
            }
            at = line;
        }
        Ok(())
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
        if self.yq_mode {
            self.reject_unstartable_flow_plain_at(self.pos)?;
        }

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
                // #6214: a Unicode break (NEL, LS, PS) ends a line for go-yaml as `\n` does, so the tab
                // rule reads the lines after it. It stays content for the fold (#4913).
                0xC2 | 0xE2 if self.unicode_break_len_at(self.pos) > 0 => {
                    self.pass_unicode_break_in_flow_plain()?;
                }
                b'\n' | b'\r' => {
                    self.reject_tab_in_flow_plain_continuation(self.pos)?;
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

        // Parse optional modifiers (order can vary: |2- or |-2), one of each kind: a second
        // chomping indicator, a second digit and a `0` are left for the check below, which
        // refuses them (#6348).
        let mut chomping_seen = false;
        for _ in 0..2 {
            match self.peek() {
                Some(b'-') if !chomping_seen => {
                    chomping = ChompingIndicator::Strip;
                    chomping_seen = true;
                    self.advance();
                }
                Some(b'+') if !chomping_seen => {
                    chomping = ChompingIndicator::Keep;
                    chomping_seen = true;
                    self.advance();
                }
                Some(c) if c.is_ascii_digit() && c != b'0' && explicit_indent == 0 => {
                    explicit_indent = c - b'0';
                    self.advance();
                }
                _ => break,
            }
        }

        // The header holds nothing else: after the indicators only blanks, a comment and the line
        // break (#6348). yq stops at anything more with `did not find expected comment or line
        // break` (`a: |  x`, `a: |x`, `a: |--`, `a: |0`, `a: |"q"`); the text was dropped and the
        // lines below read as the content.
        let mut after = self.pos;
        while self
            .input
            .get(after)
            .copied()
            .is_some_and(Self::is_inline_whitespace)
        {
            after += 1;
        }
        // NEL, LS and PS end the header line as `\n` does (#6820): the line is over there, and what
        // follows is the scalar's own first line, which `skip_block_header_line` passes the break for.
        if !matches!(self.input.get(after), None | Some(b'\n' | b'\r' | b'#'))
            && self.unicode_break_len_at(after) == 0
        {
            return Err(self.err_unexpected_char(
                after,
                "did not find expected comment or line break after a block scalar header",
            ));
        }

        Ok(BlockScalarHeader {
            style,
            chomping,
            explicit_indent,
        })
    }

    /// After a complete quoted or flow value (`k: "a"`, `- [1]`), the next line with content must not sit deeper
    /// than the block that holds it either (#5086): the value cannot be continued, and a line still deeper than its
    /// block belongs to nothing. yq stops there with `did not find expected key` (`... '-' indicator` in a
    /// sequence); it was read as a key of its own or dropped (`k:` / `  "a"` / `  z: 1` is `{"k":"a"}`). Not at the
    /// document root (#5070), and not when something other than blanks and a comment follows the value on its line:
    /// that is [`Self::reject_content_after_inline_value`]'s.
    fn reject_deeper_line_after_value(&mut self) -> Result<(), YamlError> {
        if self.type_stack.len() <= 1 {
            return Ok(());
        }
        // The common case, read without moving the cursor: the value ends its line and the next line starts
        // with content at or left of the block's indent.
        let mut i = self.pos;
        while matches!(self.input.get(i), Some(b' ' | b'\t')) {
            i += 1;
        }
        if matches!(self.input.get(i), Some(b'\n' | b'\r')) {
            let mut j = i + self.break_len_at(i).max(1);
            let line = j;
            while self.input.get(j) == Some(&b' ') {
                j += 1;
            }
            if let Some(&c) = self.input.get(j) {
                if !matches!(c, b'\n' | b'\r' | b'\t' | b'#')
                    && j - line <= self.innermost_block_indent()
                {
                    return Ok(());
                }
            }
        }
        let saved_pos = self.pos;
        self.skip_inline_whitespace();
        if self.peek() == Some(b'#') {
            self.skip_to_eol();
        }
        let result = if self.at_break() || self.peek().is_none() {
            self.reject_line_deeper_than_block_after_block_scalar()
        } else {
            Ok(())
        };
        self.pos = saved_pos;
        result
    }

    /// After a block scalar that is a value (`a: |`, `- |`), the next line with content must not sit deeper than the
    /// block that holds the scalar (#5078): the scalar ended at a line indented less than its content, and a line
    /// that is still deeper than its parent belongs to nothing. yq stops there with `did not find expected key`
    /// (`did not find expected '-' indicator` in a sequence); it was read as a key (`a: |` / `   x` / `  y` / `b: 1`
    /// is `{"a":"x\n","y":"b"}`). Blank and comment lines do not count, and a tab is left to the tab-indentation error.
    fn reject_line_deeper_than_block_after_block_scalar(&mut self) -> Result<(), YamlError> {
        let parent = self.innermost_block_indent();
        let saved_pos = self.pos;
        let result = loop {
            let line_start = self.pos;
            let indent = self.skip_spaces_simd();
            match self.peek() {
                None => break Ok(()),
                Some(b'\n' | b'\r') => self.skip_line_break(),
                Some(b'#') => {
                    self.skip_to_eol();
                    self.skip_line_break();
                }
                // A tab-only line is blank. A tab before content is the tab-indentation error's to
                // report, with its own message.
                Some(b'\t') => {
                    let blank = loop {
                        match self.peek() {
                            Some(b' ' | b'\t') => self.advance(),
                            None | Some(b'\n' | b'\r') => break true,
                            Some(_) => break false,
                        }
                    };
                    if !blank {
                        break Ok(());
                    }
                }
                Some(_) if indent > parent => {
                    let in_sequence = self.type_stack.last() == Some(&NodeType::SequenceItem);
                    break Err(self.err_unexpected_char(
                        line_start + indent,
                        if in_sequence {
                            "did not find expected '-' indicator"
                        } else {
                            "did not find expected key"
                        },
                    ));
                }
                Some(_) => break Ok(()),
            }
        };
        self.pos = saved_pos;
        result
    }

    /// Whether the first non-blank line below a block scalar's header holds content indented less than
    /// `content_indent`, so the scalar has no content of its own (#5067). End of input is not such a line.
    fn first_block_content_line_is_dedented(
        &mut self,
        content_indent: usize,
    ) -> Result<bool, YamlError> {
        let saved_pos = self.pos;
        let dedented = loop {
            let indent = self.skip_spaces_simd();
            match self.peek() {
                None => break false,
                Some(b'\n' | b'\r') => self.skip_line_break(),
                // NEL, LS and PS end an empty line as `\n` does (#6369).
                Some(0xC2 | 0xE2) if self.unicode_break_len_at(self.pos) > 0 => {
                    self.advance_by(self.unicode_break_len_at(self.pos));
                }
                // A tab below the indent the header gave, on a leading line or on the first content one,
                // is the error go-yaml raises while it eats the indentation (#6372; the auto-detected
                // indent has the same rule in `detect_block_content_indent`, #6295).
                Some(b'\t') if indent < content_indent => {
                    return Err(YamlError::TabIndentation {
                        line: self.current_line(),
                        offset: self.pos,
                    });
                }
                Some(_) => break indent < content_indent,
            }
        };
        self.pos = saved_pos;
        Ok(dedented)
    }

    /// Detect content indentation from the first non-empty line, or from the widest blank line
    /// before it when that is wider. Returns the indentation level, or None if block is empty.
    fn detect_block_content_indent(
        &mut self,
        base_indent: usize,
    ) -> Result<Option<usize>, YamlError> {
        let saved_pos = self.pos;
        // The widest blank line before the first content line (#6381).
        let mut widest_blank = 0;

        // Scan ahead to find first non-empty line
        loop {
            if self.peek().is_none() {
                // EOF - empty block scalar
                self.pos = saved_pos;
                return Ok(None);
            }

            // Count spaces at start of line (SIMD accelerated)
            let indent = self.skip_spaces_simd();

            // Check what's on this line
            match self.peek() {
                Some(b'\n' | b'\r') => {
                    // Empty line - skip and continue
                    widest_blank = widest_blank.max(indent);
                    self.skip_line_break();
                }
                // NEL, LS and PS end an empty line as `\n` does (#6369).
                Some(0xC2 | 0xE2) if self.unicode_break_len_at(self.pos) > 0 => {
                    widest_blank = widest_blank.max(indent);
                    self.advance_by(self.unicode_break_len_at(self.pos));
                }
                // #6295: the scalar's indent is not known yet, and go-yaml refuses a tab right after
                // the spaces of any line it scans for it, blank or not (`found a tab character where
                // an indentation space is expected`): ` \t` before the first content line is no
                // empty line, and a first content line that starts with a tab has no indent.
                Some(b'\t') => {
                    return Err(YamlError::TabIndentation {
                        line: self.current_line(),
                        offset: self.pos,
                    });
                }
                // A comment line at or left of the parent's indent ends a block scalar that has no content
                // yet, as go-yaml reads it (#6843): the scalar is empty and the lines after the comment
                // are the document's, so a deeper one is refused as stray (`a: |` / `#x` / `  y`). Only
                // the document's root node reads past it (`--- |` / `#x` / `  y` is `"y\n"`).
                Some(b'#') if indent <= base_indent && self.type_stack.len() > 1 => {
                    self.pos = saved_pos;
                    return Ok(None);
                }
                Some(b'#') if indent <= base_indent => {
                    // Comment line at or below the parent's indent - skip to end. A `#` line indented deeper
                    // is the first content line (#5073) and falls to the arm below: it sets the indent, so a
                    // deeper line after it is more content, not the scalar's indent.
                    self.skip_to_eol();
                    self.skip_line_break();
                }
                None => {
                    // EOF
                    self.pos = saved_pos;
                    return Ok(None);
                }
                _ => {
                    // Found content - restore position and return indent
                    self.pos = saved_pos;

                    if indent <= base_indent {
                        // Content must be more indented than indicator
                        return Ok(None);
                    }
                    // go-yaml's indent is the widest of the leading blank lines and the first content
                    // line (`max_indent` in `yaml_parser_scan_block_scalar_breaks`), so a blank line
                    // wider than the content makes the content line a dedent that ends the scalar
                    // (#6381).
                    return Ok(Some(indent.max(widest_blank)));
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
    ) -> Result<usize, YamlError> {
        // Use SIMD to quickly find where the block scalar ends
        let block_end = simd::find_block_scalar_end(self.input, self.pos, content_indent)
            .unwrap_or(self.input.len());
        let region_start = self.pos;
        let mut saw_unicode_break = false;

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
                    self.skip_to_eol_noting_unicode_break(&mut saw_unicode_break);
                    last_content_end = self.pos;
                    trailing_newline_start = self.pos;
                    self.skip_line_break();
                }
            }
        }

        // The line that ends the scalar by being dedented: go-yaml eats its spaces as the scalar's
        // indentation, and a tab after them is the error (#6372). A space-only dedent is the ordinary end.
        let mut after_spaces = block_end;
        while self.input.get(after_spaces) == Some(&b' ') {
            after_spaces += 1;
        }

        // go-yaml ends a line at NEL, LS and PS as at `\n`; the kernel and the walk above do not (#6369).
        // A scalar holding one is read again, line by line, the way go-yaml reads it, and listed so the
        // decoder reads it so too. Almost none does: the walk says whether it passed one, looking only
        // at the bytes that could start one. The line that ends the scalar counts too: spaces and a
        // break are an empty line to go-yaml, which the scalar then runs past.
        if saw_unicode_break || self.unicode_break_len_at(after_spaces) > 0 {
            self.unicode_block_scalars.push(self.block_header_pos);
            self.pos = region_start;
            return self.consume_block_scalar_content_unicode(content_indent, chomping);
        }

        if self.input.get(after_spaces) == Some(&b'\t') {
            self.pos = after_spaces;
            return Err(YamlError::TabIndentation {
                line: self.current_line(),
                offset: after_spaces,
            });
        }

        // Position should now be at block_end
        self.pos = block_end;

        // Return position based on chomping
        Ok(match chomping {
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
        })
    }

    /// [`Self::consume_block_scalar_content`] for a scalar that holds NEL, LS or PS (#6369): every one
    /// of the three ends a line as `\n` does, so the spaces after it are indentation, a dedent after
    /// it ends the scalar (in the middle of what a `\n`-only reader sees as one line), a tab below the
    /// indent is refused (#6287), and a line of spaces and a break is an empty line. The end is
    /// [`block_unicode::scan_block_end`]'s; the walk after it is the byte walk's with those breaks.
    #[cold]
    fn consume_block_scalar_content_unicode(
        &mut self,
        content_indent: usize,
        chomping: ChompingIndicator,
    ) -> Result<usize, YamlError> {
        let start = self.pos;
        let block_end = match block_unicode::scan_block_end(self.input, start, content_indent) {
            Ok(end) => end,
            Err(tab) => {
                self.pos = tab;
                return Err(YamlError::TabIndentation {
                    line: self.current_line(),
                    offset: tab,
                });
            }
        };

        let mut last_content_end = start;
        let mut trailing_newline_start = start;
        while self.pos < block_end {
            let line_start = self.pos;
            let _line_indent = self.skip_spaces_simd();
            let break_len = block_unicode::any_break_len(self.input, self.pos);
            if break_len > 0 {
                trailing_newline_start = line_start;
                self.advance_by(break_len);
            } else if self.pos >= self.input.len() {
                break;
            } else {
                while self.pos < block_end
                    && block_unicode::any_break_len(self.input, self.pos) == 0
                {
                    self.pos += 1;
                }
                last_content_end = self.pos;
                trailing_newline_start = self.pos;
                self.advance_by(block_unicode::any_break_len(self.input, self.pos));
            }
        }

        self.pos = block_end;
        Ok(match chomping {
            ChompingIndicator::Strip => last_content_end,
            ChompingIndicator::Clip => {
                if last_content_end > 0 && trailing_newline_start > last_content_end {
                    last_content_end + block_unicode::any_break_len(self.input, last_content_end)
                } else {
                    last_content_end
                }
            }
            ChompingIndicator::Keep => self.pos,
        })
    }

    /// After an empty block scalar, pass the empty lines below it that end in NEL, LS or PS (#6369): go-yaml
    /// reads each as an empty line, which the line dispatcher would otherwise take for stray structure
    /// (`a: |` / `  <PS>` / `b: 1`). Spaces and an ASCII break are passed with them but only kept when a
    /// Unicode break was among them, so the common empty scalar leaves the cursor where it was; such a
    /// scalar is listed like any that holds one.
    #[cold]
    fn skip_unicode_empty_lines(&mut self) {
        let mut at = self.pos;
        let mut keep = self.pos;
        loop {
            while self.input.get(at) == Some(&b' ') {
                at += 1;
            }
            let unicode = self.unicode_break_len_at(at);
            let ascii = if HAS_CR {
                line_break_len(self.input, at)
            } else {
                usize::from(self.input.get(at) == Some(&b'\n'))
            };
            if unicode > 0 {
                at += unicode;
                keep = at;
            } else if ascii > 0 {
                at += ascii;
            } else {
                break;
            }
        }
        if keep > self.pos {
            // Listed, so `|+` keeps those lines: each is its own break (`  <PS>` / `b: 1` is `"\u2029\n"`).
            self.unicode_block_scalars.push(self.block_header_pos);
        }
        self.pos = keep;
    }

    /// Pass the rest of a block scalar's header line and its break (#6820). go-yaml ends the line at NEL,
    /// LS and PS as at `\n`, so `a: |<NEL>  x` has its content on the same physical line; such a scalar
    /// is listed like one that holds a Unicode break in its body (#6369), so the decoders end the header
    /// at the same place. The common header ends right here, at the `\n`, and costs one look at a byte.
    #[inline]
    fn skip_block_header_line(&mut self) {
        if !matches!(self.peek(), None | Some(b'\n' | b'\r')) {
            // Blanks and a comment follow the indicators: a short walk, once per scalar.
            let mut at = self.pos;
            while at < self.input.len() && block_unicode::any_break_len(self.input, at) == 0 {
                at += 1;
            }
            if matches!(self.input.get(at), Some(0xC2 | 0xE2)) {
                self.unicode_block_scalars.push(self.block_header_pos);
            }
            self.pos = at;
        }
        if let Some(&b) = self.input.get(self.pos) {
            if matches!(b, b'\n' | b'\r') {
                self.skip_line_break();
            } else {
                self.advance_by(block_unicode::any_break_len(self.input, self.pos));
            }
        }
    }

    /// Parse a block scalar (| or >) including all content lines.
    fn parse_block_scalar(&mut self, base_indent: usize) -> Result<(), YamlError> {
        self.block_header_pos = self.pos;
        // Mark the indicator position
        self.set_ib();
        self.write_bp_open();

        // Parse the header
        let header = self.parse_block_scalar_header()?;

        // Skip to end of indicator line, capturing a trailing comment as this
        // block scalar's own line comment first (#710) — `self.last_open_bp_pos`
        // is still this node's bp_pos here since no content/children are
        // parsed yet.
        // A comment deferred past properties on the line above (#4851) is claimed first, so it
        // precedes the header's own, as on a mapping key (see `take_pending_head_comment`).
        self.take_pending_head_comment(self.last_open_bp_pos);
        if let Some((start, end)) = self.scan_trailing_comment() {
            // The comment ends at NEL, LS and PS as it does at `\n` (#6820): `a: | # c<LS>  x` has the
            // comment `# c` and the text `x`. A comment holds a lead byte only when it holds non-ASCII
            // text, so the walk below is a byte compare for the usual one.
            let end = (start as usize..end as usize)
                .find(|&at| {
                    matches!(self.input[at], 0xC2 | 0xE2) && self.unicode_break_len_at(at) > 0
                })
                .map_or(end, |at| at as u32);
            self.push_line_comment(self.last_open_bp_pos, (start, end));
        }
        self.skip_block_header_line();

        // Determine content indentation
        let content_indent = if header.explicit_indent > 0 {
            let content_indent = base_indent + header.explicit_indent as usize;
            if self.first_block_content_line_is_dedented(content_indent)? {
                // An explicit indent with nothing indented that far below the header is an empty
                // scalar, as with an auto-detected one: `|2` / `---` is not a scalar that swallows
                // the marker (#5067). `find_block_scalar_end` never looks at the first line.
                self.set_bp_text_end_position(self.pos);
                self.write_bp_close();
                self.skip_unicode_empty_lines();
                return Ok(());
            }
            content_indent
        } else {
            // Auto-detect from first content line
            match self.detect_block_content_indent(base_indent)? {
                Some(indent) => indent,
                None => {
                    // Empty block scalar. `self.pos` sits at the start of the
                    // line following the header (see `detect_block_content_indent`),
                    // so a `#` there belongs to a following comment/sibling
                    // line, not this scalar — use the no-capture variant
                    // (#710, see `set_bp_text_end_position`'s doc comment).
                    self.set_bp_text_end_position(self.pos);
                    self.write_bp_close();
                    self.skip_unicode_empty_lines();
                    return Ok(());
                }
            }
        };

        // Consume content lines
        let content_end = self.consume_block_scalar_content(content_indent, header.chomping)?;

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

    /// Record `start` as the offset of the first property token of the node
    /// at `bp`, unless an earlier property of the same node already did
    /// (#2712). Called with `self.pos` *before* the property is consumed.
    #[inline]
    fn note_property_start(&mut self, bp: usize, start: usize) {
        self.property_starts.entry(bp).or_insert(start);
    }

    /// Parse an anchor definition (`&name`).
    /// Records the anchor and returns, expecting the value to follow.
    fn parse_anchor(&mut self) -> Result<String, YamlError> {
        let start = self.pos;
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
        self.note_property_start(self.bp_pos, start);
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
        let start = self.pos;
        self.advance();
        let name = self.parse_anchor_name()?;
        self.skip_inline_whitespace();
        self.anchors.insert(name.clone(), self.bp_pos - 1);
        // #1353: see `parse_anchor`'s own matching insert above.
        self.bp_to_anchor.insert(self.bp_pos - 1, name);
        self.note_property_start(self.bp_pos - 1, start);
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
        // #6298: a verbatim tag (`!<...>`) ends at its `>`, and yq's scanner wants whitespace, a
        // line break or the end of the input right after it (`a: !<tag:x>x`, `!<tag:x>"q"`,
        // `!<tag:x>#c`: `did not find expected whitespace or line break`).
        if is_verbatim_tag(self.input, start)
            && self.input.get(end).is_some_and(|&b| !is_tag_blank(b))
        {
            return Err(YamlError::InvalidTag {
                offset: end,
                reason: "a verbatim tag must be followed by whitespace or a line break",
            });
        }
        // #6293: a tag runs to the next space, and a flow indicator right after it is part of that
        // run for yq's scanner, which then refuses the character (`[!!str]`, `{a: !!str}`,
        // `[!!str, 1]`, `a: !!str]`: `found character ']' that is not allowed in a YAML tag`). The
        // extent stops at the indicator for the lookaheads, which only want to know where the tag
        // ends; the parse refuses it. A space first (`[!!str ]`) is fine.
        if matches!(self.input.get(end), Some(b',' | b'[' | b']' | b'{' | b'}')) {
            return Err(YamlError::InvalidTag {
                offset: end,
                reason: "a flow indicator may not follow a tag without a space",
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
                    let start = self.pos;
                    let tag = self.parse_tag()?;
                    self.tags.insert(self.bp_pos, tag);
                    self.note_property_start(self.bp_pos, start);
                    self.pending_property_bp = Some(self.bp_pos);
                    self.skip_inline_whitespace();
                    had_property = true;
                }
                _ => break,
            }
        }
        if had_property {
            self.properties_end = self.pos;
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
                    let start = self.pos;
                    let tag = self.parse_tag()?;
                    self.tags.insert(self.bp_pos, tag);
                    self.note_property_start(self.bp_pos, start);
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
        let start = self.pos;
        let tag = self.parse_tag()?;
        self.skip_inline_whitespace();
        self.tags.insert(self.bp_pos - 1, tag);
        self.note_property_start(self.bp_pos - 1, start);
        self.pending_property_bp = Some(self.bp_pos - 1);
        Ok(())
    }

    /// Consume any combination of a leading `&anchor` and/or `!tag` prefixing
    /// a mapping key whose BP node is already open, in either order — the
    /// already-open twin of [`Self::parse_node_properties`], combining
    /// [`Self::record_key_anchor`] and [`Self::record_key_tag`].
    fn record_key_properties(&mut self) -> Result<(), YamlError> {
        let mut had_property = false;
        loop {
            match self.peek() {
                Some(b'&') => self.record_key_anchor()?,
                Some(b'!') => self.record_key_tag()?,
                _ => break,
            }
            had_property = true;
        }
        if had_property {
            self.properties_end = self.pos;
            // `&t - a: b`: what the property stands before is a block sequence entry, not a key, which yq
            // refuses (#6414); the key would read as the plain text `- a`.
            if self.peek() == Some(b'-') && Self::is_ws_break_or_eoi(self.peek_at(1)) {
                self.reject_misplaced_sequence_indicator()?;
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
            let start = self.pos;
            let tag = self.parse_tag()?;
            self.tags.insert(self.bp_pos - 1, tag);
            self.note_property_start(self.bp_pos - 1, start);
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
        } else {
            // The whole input is comments and/or blank lines, with no
            // `---`/`...` marker anywhere (#2795 review): real yq still
            // yields exactly one (null) document for this, e.g. a lone
            // `# c1` line prints back verbatim under `.` and answers `1`
            // under a literal filter, rather than the stream producing no
            // documents at all (verified against pinned yq). Synthesizing
            // one here, rather than leaving `pending_head_lines` to attach
            // to the virtual root sequence itself, gives the comment(s) a
            // real document node to be this document's own head -- exactly
            // `start_document`'s own "a leading `# lead` belongs to the
            // document's own node" rule, and `end_document` synthesizes the
            // null value since nothing gets written for it. Distinct from
            // the genuinely-zero-document case `parse_documents` still
            // handles on its own (#225's HWV9/QT73: an explicit marker with
            // nothing before it) -- that case requires a marker to exist at
            // all, which this branch's `peek().is_none()` guard already
            // rules out.
            //
            // Real yq's YAML-target printer suppresses the value line
            // entirely for a document with no explicit content (`# c\n`
            // under `.` prints just `# c`, no `null`) -- both tools agree
            // it's `!!null` under `. | type`, and JSON output already
            // agrees (`null` on both). This was a pre-existing emitter gap,
            // not new here (an explicit `---` alone with nothing after it
            // had the identical `---\nnull` bug already, unrelated to
            // comments), now closed on the streaming route by
            // `YamlCursor::is_header_only_document` (`src/yaml/light.rs`)
            // gating the identity path's own per-document terminator in
            // `yq_runner.rs`'s `stream_cursor!` macro (#2795 review) --
            // `# c\n`/`---\n` now round-trip through `.` byte-for-byte.
            // The DOM route (`-P`/a write) closes it the same way for a
            // different reason: `OwnedValue` cannot tell this synthesized
            // absence from a `null`/`~` keyword, so the root's node
            // metadata records the empty spelling (#3028) and
            // `output_value` skips the terminator for a header-only root
            // (#2875).
            self.start_document();
            self.end_document();
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
            property_starts: core::mem::take(&mut self.property_starts),
            comments: core::mem::take(&mut self.comments),
            unicode_block_scalars: {
                let mut listed = core::mem::take(&mut self.unicode_block_scalars);
                listed.sort_unstable();
                listed.dedup();
                listed
            },
        })
    }

    /// Parse all documents in the stream.
    fn parse_documents(&mut self) -> Result<(), YamlError> {
        // Skip any `%YAML`/`%TAG`/reserved directive lines before the first
        // document (#225).
        self.skip_directives()?;
        if self.yq_mode && self.saw_non_header_directive && !self.is_document_start() {
            return Err(self.err_directive_without_document_start());
        }

        // Skip leading `---` if present (optional for first doc)
        if self.is_document_start() {
            self.skip_document_marker();
            self.leading_run_from = self.pos;
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
            if self.yq_mode && !self.in_document && self.documents_started == 0 {
                self.yq_directive_only_stream();
            }
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
                if self.yq_mode && self.yq_head_holds_nothing_before_end_marker() {
                    return Err(
                        self.err_unexpected_char(self.pos, "did not find expected node content")
                    );
                }
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
                let skipped = self.skip_directives()?;

                // Check for another document or EOF (a directive-only tail
                // can itself exhaust the input).
                if self.peek().is_none() {
                    // yq wants a `---` after a directive (#6419).
                    if self.yq_mode && skipped {
                        return Err(self.err_directive_without_document_start());
                    }
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

                // Content here has no `---` before it: a document after a `...` needs its own
                // start marker, and yq stops with `did not find expected <document start>`
                // where this read a bare document (#5089).
                if self.peek().is_some() && !self.is_document_end() && !self.in_document {
                    return Err(self
                        .err_unexpected_char(self.pos, "did not find expected <document start>"));
                }
                continue;
            }

            // Check for document start marker (new document)
            if self.is_document_start() {
                // #6349: a marker while the first document has written nothing (only comments and
                // blank lines since its own `---`) is more of yq's header, not another document.
                if self.yq_mode
                    && self.in_document
                    && self.documents_started == 1
                    && self.bp_pos == self.document_start_bp_pos
                    && self.only_yq_header_lines_since(self.leading_run_from)
                {
                    self.skip_document_marker();
                    self.leading_run_from = self.pos;
                    if self.has_content_on_line() {
                        self.parse_inline_document_value()?;
                    } else {
                        self.skip_newlines();
                    }
                    continue;
                }
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

            // #6419: a directive met inside a document ends it.
            if self.yq_mode
                && self.in_document
                && self.peek() == Some(b'%')
                && (self.pos == 0 || Self::is_break(self.input[self.pos - 1]))
            {
                self.directive_ends_document()?;
                continue;
            }

            // Parse document content
            self.parse_document_line()?;
        }

        Ok(())
    }

    /// Whether the cursor is on a `%YAML` line in the head of a yq stream, after the first document's
    /// leading `---` (#6342): yq's slurp sets it aside like a comment (before the `---` it is a
    /// directive, which `skip_directives` skips).
    fn at_yq_head_yaml_directive(&self) -> bool {
        self.yq_mode
            && self.documents_started == 1
            && self.in_document
            && self.bp_pos == self.document_start_bp_pos
            && (self.pos == 0 || Self::is_break(self.input[self.pos - 1]))
            && starts_yq_yaml_directive(&self.input[self.pos..])
            && self.only_yq_header_lines_before_cursor()
    }

    /// Whether everything before the cursor, from the start of the stream or of the first document's
    /// last leading `---`, is a line yq's slurp sets aside (#6342). A node property (`!!str`) writes no
    /// node here, but it is content to go-yaml, which then refuses a tab-led line after it where the
    /// slurp would have taken it.
    fn only_yq_header_lines_before_cursor(&self) -> bool {
        // A directive other than `%YAML` is not set aside, so the lines after it are go-yaml's.
        !self.saw_non_header_directive
            && self.only_yq_header_lines_since(if self.documents_started == 0 {
                0
            } else {
                self.leading_run_from
            })
    }

    /// Whether a `...` at the cursor is the first token go-yaml would see (#6342): yq's header slurp
    /// has set aside everything before it (blank and comment lines, `%YAML` directives and a run of
    /// leading `---` markers), so the marker opens an implicit document and has no node to close.
    /// `a: 1` / `---` / `...` is not this: that `---` is not in the head, go-yaml reads it, and the
    /// document it starts is empty by the spec. Nor is `!!str` / `...`: a property is a node, an
    /// empty scalar, though it writes none here, so the head is read as text.
    ///
    /// A directive other than `%YAML` stays with go-yaml, whose explicit document start then makes
    /// the `...` an empty document's end, so it is not this either.
    fn yq_head_holds_nothing_before_end_marker(&self) -> bool {
        if self.saw_non_header_directive {
            return false;
        }
        match self.documents_started {
            0 => !self.in_document,
            1 => {
                self.in_document
                    && self.bp_pos == self.document_start_bp_pos
                    && self.only_yq_header_lines_since(self.leading_run_from)
            }
            _ => false,
        }
    }

    /// The refusal for a directive other than `%YAML` that no `---` follows (#6342): yq's slurp
    /// leaves `%TAG` and the rest to go-yaml, which wants a document start after them.
    fn err_directive_without_document_start(&self) -> YamlError {
        const CONTEXT: &str = "did not find expected <document start>";
        if self.peek().is_some() {
            self.err_unexpected_char(self.pos, CONTEXT)
        } else {
            YamlError::UnexpectedEof { context: CONTEXT }
        }
    }

    /// The end of a yq stream with no document and no marker (#6342). yq's slurp sets aside a
    /// `%YAML` line like a comment, so a stream of only those is the one null document its comments
    /// and directives are the head of, as a stream of only comments is (`parse`).
    fn yq_directive_only_stream(&mut self) {
        debug_assert!(!self.saw_non_header_directive);
        self.start_document();
        self.end_document();
    }

    /// Whether `input[..at]` ends in nothing but spaces and tabs back to the start of its line.
    fn only_indent_before(&self, at: usize) -> bool {
        let mut start = at;
        while start > 0 && matches!(self.input[start - 1], b' ' | b'\t') {
            start -= 1;
        }
        start == 0 || Self::is_break(self.input[start - 1])
    }

    /// The offset of the `%` of an indented `%YAML` line (a space or tab before it) that the cursor, at
    /// the start of its line, is on in the head of a yq stream (#6408): yq's slurp trims the line before
    /// it looks for `%YA`, so it sets it aside as it does a column-0 one (#6342), and prints it back as
    /// a comment (`# ` and the line, indent included). The head is the lines before the first
    /// `---` or content, or between a leading `---` and the first content line.
    fn yq_indented_head_directive(&self) -> Option<usize> {
        if !self.yq_mode || !(self.pos == 0 || Self::is_break(self.input[self.pos - 1])) {
            return None;
        }
        let mut at = self.pos;
        while matches!(self.input.get(at), Some(b' ' | b'\t')) {
            at += 1;
        }
        let in_head = match self.documents_started {
            0 => !self.in_document,
            1 => self.in_document && self.bp_pos == self.document_start_bp_pos,
            _ => false,
        };
        (at > self.pos
            && in_head
            && starts_yq_yaml_directive(&self.input[at..])
            && self.only_yq_header_lines_before_cursor())
        .then_some(at)
    }

    /// Whether `input[from..self.pos]` holds nothing but the lines yq's header slurp sets aside:
    /// blanks, line breaks, `#` comments and `%YAML` directive lines (#6342). It is the text between
    /// a leading `---` and the next one when the second is more of yq's header (#6349). A node
    /// property (`&a`, `!!str`) is not header, though it writes no node of its own.
    fn only_yq_header_lines_since(&self, from: usize) -> bool {
        if self.yq_no_head {
            return false;
        }
        let mut i = from;
        let end = self.pos.min(self.input.len());
        while i < end {
            match self.input[i] {
                b' ' | b'\t' | b'\n' | b'\r' => i += 1,
                b'#' => {
                    while i < end && !matches!(self.input[i], b'\n' | b'\r') {
                        i += 1;
                    }
                }
                b'%' if starts_yq_yaml_directive(&self.input[i..])
                    && self.only_indent_before(i) =>
                {
                    while i < end && !matches!(self.input[i], b'\n' | b'\r') {
                        i += 1;
                    }
                }
                _ => return false,
            }
        }
        true
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

        // The cursor is past the line's spaces: only a line whose next byte is the tab can be the one.
        if self.peek() == Some(b'\t') && self.tab_only_line_is_refused(line_start) {
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

    /// Whether a `-` line at `indent` lands directly in a block mapping open at that indent whose
    /// latest entry already has its value (`a: 1` / `- x`, a nested mapping or sequence that ended
    /// on the line before, a block scalar). A same-indent `-` is legal there only as the
    /// implicit value of a `key:` still waiting for one, so anywhere else real yq stops with
    /// `did not find expected key` and the line must not be dropped (#4530).
    ///
    /// The line that owns the pending value has to sit at the mapping's own indent: a `-` that
    /// dedents past a nested `b:` back into its parent mapping (`a:` / `  b:` / `- x`) is the
    /// same error. A line that opens with its own `-` is a compact mapping inside a sequence item,
    /// whose key column is not the line's indent, so it is judged on its ending alone. An
    /// explicit key's owner mapping is left to
    /// [`Self::explicit_key_with_content_rejects_sequence_at`], where a bare `?` legitimately
    /// takes the sequence as its own content (YAML test suite 6PBE).
    fn sequence_follows_completed_mapping_entry_at(&self, indent: usize) -> bool {
        let Some(mut landing) = self.indent_stack.len().checked_sub(1) else {
            return false;
        };
        while landing > 0 && self.indent_stack[landing] > indent {
            landing -= 1;
        }
        if self.indent_stack[landing] != indent || self.type_stack[landing] != NodeType::Mapping {
            return false;
        }
        if self
            .pending_explicit_key
            .is_some_and(|owner_depth| owner_depth == landing + 1)
        {
            return false;
        }
        !self.previous_content_line_defers_a_value_at(indent)
    }

    /// [`Self::previous_content_line_defers_its_value`] for a mapping entry in particular: the
    /// previous content line ends in the `:` of a key (node properties and a comment aside) and
    /// that key stands at `indent` ([`Self::sequence_follows_completed_mapping_entry_at`]). The
    /// key's column is the line's indent plus the `- ` markers it opens with, so the compact
    /// mapping of a sequence item (`- k:`) stands where its continuation lines do.
    fn previous_content_line_defers_a_value_at(&self, indent: usize) -> bool {
        self.previous_line_key(LineMarkers::Dashes)
            .is_some_and(|key| {
                key.column == indent && line_ends_with_colon_after_properties(key.text)
            })
    }

    /// The key the previous content line opens with, past the `markers` that precede it: its
    /// column (the line's indent plus the markers, as their spacing leaves it), the text from the
    /// key on, and the last marker (its column, and whether it is a dash). The walk is the one
    /// definition of where a key stands for
    /// [`Self::previous_content_line_defers_a_value_at`] (`LineMarkers::Dashes`: a `?` or `:` is
    /// not walked past, as before) and [`Self::header_is_left_of_its_owner`]
    /// (`LineMarkers::Explicit`, #6540, #6543).
    fn previous_line_key(&self, markers: LineMarkers) -> Option<PreviousLineKey<'_>> {
        let (lead, rest) = self.previous_content_line()?;
        let (marker_len, last_marker) = Self::walk_line_markers(lead, rest, markers);
        Some(PreviousLineKey {
            column: lead + marker_len,
            text: &rest[marker_len..],
            last_marker,
        })
    }

    /// Whether the nearest earlier line that is neither blank nor a comment leaves its value for
    /// the next line: a key written up to its colon (`a:`, `- k:`), with node properties after
    /// the colon or not (`a: &x`, #4947), or a dash with nothing after it but node properties and a
    /// comment (`-`, `- -`, `- &x`, #6550: [`Self::deferred_value_owner`] reads all three). The
    /// only place a block scalar header on a line of its own can belong (#4259); a header anywhere
    /// else is not a value slot and keeps the path it always took.
    fn previous_content_line_defers_its_value(&self) -> bool {
        self.previous_content_line().is_some_and(|(lead, rest)| {
            line_ends_with_colon_before_comment(rest)
                || line_ends_with_colon_after_properties(rest)
                || Self::deferred_value_owner(lead, rest).is_some()
        })
    }

    /// Whether the line above leaves its value to a dash or a key that stands to the right of
    /// `indent` (#6540, #6543): a block scalar header on a line of its own there is left of the
    /// owner it would be the value of, so it is no value slot, and yq refuses it. The owner is a
    /// key written up to its colon (`- k:`, `- - k: &y`, `? k:`, `: - k:`), at the line's indent
    /// plus the markers it opens with, or the last marker of a line that ends in one (`- -`,
    /// `- - &x`, `? -`, a bare `?`). A header at the owner's own column (`- k:` / `  |`, `- -` /
    /// `  |`) or to its right is its value. The markers are `-`, and `?` and `:` of an explicit
    /// key. yq words the refusal by the last marker: `did not find expected '-' indicator` after
    /// a dash, `did not find expected key` in a mapping.
    fn header_is_left_of_its_owner(&self, indent: usize) -> Option<&'static str> {
        let key = self.previous_line_key(LineMarkers::Explicit)?;
        let owner = match key.last_marker {
            Some((column, _)) if only_node_properties(key.text) => column,
            _ if line_ends_with_colon_before_comment(key.text)
                || line_ends_with_colon_after_properties(key.text) =>
            {
                key.column
            }
            _ => return None,
        };
        (indent < owner).then_some(match key.last_marker {
            Some((_, true)) => "did not find expected '-' indicator",
            _ => "did not find expected key",
        })
    }

    /// Whether the line above the cursor's that leaves its value for the lines below is a dash (`-`,
    /// `- &x`, a chain of them) at column `indent`: a block scalar header at that column is its
    /// value (#6390), where a header deeper than it always was.
    fn dash_above_owns_header_at(&self, indent: usize) -> bool {
        self.previous_content_line()
            .and_then(|(lead, rest)| Self::deferred_value_owner(lead, rest))
            .is_some_and(|(owner, is_dash)| is_dash && owner == indent)
    }

    /// The nearest line above the cursor's that holds content, as its count of leading spaces and
    /// the rest of it: a blank line, a comment line and node properties on a line of their own
    /// (`&x`, part of the value that follows, #4271) are walked past. `None` at the top of the
    /// input.
    fn previous_content_line(&self) -> Option<(usize, &[u8])> {
        let mut line_start = self.pos;
        while line_start > 0 && !is_line_break(self.input[line_start - 1]) {
            line_start -= 1;
        }
        let mut end = line_start;
        while end > 0 {
            let mut prev_end = end - 1;
            if prev_end > 0 && self.input[prev_end] == b'\n' && self.input[prev_end - 1] == b'\r' {
                prev_end -= 1;
            }
            let mut prev_start = prev_end;
            while prev_start > 0 && !is_line_break(self.input[prev_start - 1]) {
                prev_start -= 1;
            }
            end = prev_start;
            let line = &self.input[prev_start..prev_end];
            let lead = line.iter().take_while(|&&b| b == b' ').count();
            let rest = &line[lead..];
            if rest.is_empty() || rest[0] == b'#' || only_node_properties(rest) {
                continue;
            }
            return Some((lead, rest));
        }
        None
    }

    /// Whether a scalar, alias or flow collection that opens a line at `indent` has nothing to
    /// attach to (#6343): the nearest block mapping or sequence at that indent has no value waiting
    /// for a node on a line of its own. A mapping value, a key's content and a continuation all
    /// stand more indented than the key, so a node level with the key (or with the `-` markers) is
    /// what yq refuses with `while scanning a simple key` (`a: |` / `  x` / `stray`, `k: &a` / `q`,
    /// `? k` / `q`), where it was dropped or read as a value. A bare `?` does not take it as its
    /// key's content either (`?` / `q` is refused; only a block sequence may stand level with a
    /// `?`, YAML test suite 6PBE). The document root is not judged here.
    ///
    /// A block scalar header is the content of the entry above it when that entry is a sequence
    /// item (`- &x` / `|`) or a key still waiting for its value, as yq reads it; after a completed
    /// mapping entry (`a: |` / `  x` / `|`) it is stray like any other node.
    fn node_has_no_owner_at_container_indent(&self, indent: usize) -> bool {
        if self.type_stack.len() <= 1 {
            return false;
        }
        let Some(mut landing) = self.indent_stack.len().checked_sub(1) else {
            return false;
        };
        while landing > 0 && self.indent_stack[landing] > indent {
            landing -= 1;
        }
        let Some(&kind) = self.type_stack.get(landing) else {
            return false;
        };
        if self.indent_stack[landing] != indent
            || !matches!(kind, NodeType::Mapping | NodeType::Sequence)
        {
            return false;
        }
        let header = matches!(self.peek(), Some(b'|' | b'>'));
        !(header && (kind == NodeType::Sequence || self.previous_content_line_defers_its_value()))
    }

    /// Refuse the node at the cursor when it has no owner at `indent`
    /// ([`Self::node_has_no_owner_at_container_indent`]).
    fn reject_node_without_owner(&self, indent: usize) -> Result<(), YamlError> {
        if self.node_has_no_owner_at_container_indent(indent) {
            return Err(self.err_stray_line());
        }
        Ok(())
    }

    /// The error for a line with nothing to attach to (#5068).
    fn err_stray_line(&self) -> YamlError {
        self.err_unexpected_char(
            self.pos,
            "a line that is neither a key, a sequence item nor a continuation",
        )
    }

    /// Whether the line at the cursor, at `indent`, stands after the plain scalar `ended` and has
    /// nothing to attach to there (#5068): `- a # c` / `  z`, `k: a` / `z`, `- a` / `  # c` /
    /// `  z: 1`. A plain scalar takes every following line more indented than its block until a
    /// comment ends it, so a bare line left over is neither a continuation nor a node of its own,
    /// and a key or an item more indented than the block holding the value has no parent: yq
    /// stops with `did not find expected '-' indicator` / `could not find expected ':'`, where
    /// the line read as a further item or was dropped. Only inside a container, and not under an
    /// explicit key; the document root is [`Self::reject_content_after_inline_value`]'s and
    /// #5070's.
    ///
    /// A scalar on a line of its own (the value of the `key:` or `-` above it) goes on line by
    /// line at the indent it began at, so a bare line after it is a continuation unless a comment
    /// ended it or it is no more indented than the block.
    fn line_is_stray_after_a_plain_scalar(
        &self,
        ended: Option<PlainScalarSpan>,
        indent: usize,
        bare_scalar: bool,
    ) -> bool {
        let Some(scalar) = ended else {
            return false;
        };
        // The key's own plain scalar is judged by `line_is_stray_after_an_explicit_key_scalar`,
        // not here. A scalar inside a container the key opened (`? - a` / `    # c` / `    x`,
        // `? - - a`, `? - k: a`) is an item or a value like any other, and a line left over after
        // it is stray in the same way (#6580).
        let inside_key_container = self
            .pending_explicit_key
            .is_some_and(|owner_depth| self.indent_stack.len() > owner_depth);
        if self.type_stack.len() <= 1
            || (self.pending_explicit_key.is_some() && !inside_key_container)
        {
            return false;
        }
        if !bare_scalar && indent <= scalar.owner {
            return false;
        }
        let Some(comment) = self.only_comments_between(scalar.end) else {
            return false; // patchcov: coverage tolerate-line reason="defensive: the scalar is taken at the first line start after it, so only a comment or blank line lies between it and the line (flow, quoted and block-scalar neighbours probed, #5068)"
        };
        let line_start = self.input[..scalar.start]
            .iter()
            .rposition(|&b| is_line_break(b))
            .map_or(0, |at| at + 1);
        // Node properties before the scalar (`&a v`) leave it on a line of its own.
        let on_its_own_line = only_node_properties(&self.input[line_start..scalar.start]);
        let stray = if on_its_own_line {
            if comment {
                bare_scalar || indent > scalar.owner
            } else {
                bare_scalar && indent <= scalar.owner
            }
        } else {
            bare_scalar || indent > scalar.owner
        };
        stray && !self.document_root_is_a_bare_plain_scalar()
    }

    /// Whether the line at the cursor, at `indent`, stands after the plain scalar `ended` that is the
    /// whole content of the pending explicit key beginning at `key_scalar`, a comment having ended
    /// it, and is more indented than the `?` without being the `:` of the key's value (#6371): yq
    /// stops with `did not find expected key`, where the line read as the entry's value and the
    /// `: v` line after it was lost. A line at the `?`'s column is the `:` or the next key, which
    /// the dispatch below judges.
    fn line_is_stray_after_an_explicit_key_scalar(
        &self,
        ended: Option<PlainScalarSpan>,
        key_scalar: usize,
        indent: usize,
    ) -> bool {
        let Some(scalar) = ended.filter(|scalar| scalar.start == key_scalar) else {
            return false;
        };
        self.line_is_beyond_the_pending_explicit_key(indent)
            && self.only_comments_between(scalar.end) == Some(true)
    }

    /// Whether the line at the cursor, at `indent`, is content more indented than the `?` of the
    /// pending explicit key and is not the `:` of its value (#6371, #6382). A `:` that is followed
    /// by a blank is the value indicator, which the dispatch judges; `:x` is a plain scalar.
    fn line_is_beyond_the_pending_explicit_key(&self, indent: usize) -> bool {
        let Some(owner_depth) = self.pending_explicit_key else {
            return false;
        };
        let value_indicator =
            self.peek() == Some(b':') && Self::is_ws_break_or_eoi(self.peek_at(1));
        !matches!(self.peek(), Some(b'#' | b'\n' | b'\r') | None)
            && !value_indicator
            && self
                .indent_stack
                .get(owner_depth - 1)
                .is_some_and(|&at| indent > at)
    }

    /// Whether nothing but a comment, blank lines and comment lines lies between `from` and the
    /// line the cursor is on, and if so whether a comment does.
    fn only_comments_between(&self, from: usize) -> Option<bool> {
        let mut line_start = self.pos;
        while line_start > 0 && !is_line_break(self.input[line_start - 1]) {
            line_start -= 1;
        }
        let mut comment = false;
        let mut i = from;
        // The rest of the scalar's own line.
        while matches!(self.input.get(i), Some(b' ' | b'\t')) {
            i += 1;
        }
        if self.input.get(i) == Some(&b'#') {
            comment = true;
            while self.input.get(i).is_some_and(|&b| !is_line_break(b)) {
                i += 1;
            }
        }
        while i < line_start {
            match self.input.get(i) {
                Some(b'\n' | b'\r') => i += 1,
                Some(b' ' | b'\t') => i += 1,
                Some(b'#') => {
                    comment = true;
                    while self.input.get(i).is_some_and(|&b| !is_line_break(b)) {
                        i += 1;
                    }
                }
                _ => return None, // patchcov: coverage tolerate-line reason="defensive: see the caller's let-else, where the same invariant leaves only comments and blank lines here (#5068)"
            }
        }
        (i == line_start).then_some(comment)
    }

    /// Whether the document the cursor is in opens with a bare plain scalar (#5068): its first
    /// content line, after the `---` marker's own or the lines above, is neither a key, an item
    /// nor any other kind of node. Everything after such a root is that scalar's continuation, and
    /// the parser's reading of a continuation line that opens like an item (`- x`) is not to be
    /// judged as container content.
    fn document_root_is_a_bare_plain_scalar(&self) -> bool {
        let mut line_start = self.pos;
        while line_start > 0 && !is_line_break(self.input[line_start - 1]) {
            line_start -= 1;
        }
        let mut first_content: Option<&[u8]> = None;
        let mut end = line_start;
        while end > 0 {
            let mut prev_end = end - 1;
            if prev_end > 0 && self.input[prev_end] == b'\n' && self.input[prev_end - 1] == b'\r' {
                prev_end -= 1;
            }
            let mut prev_start = prev_end;
            while prev_start > 0 && !is_line_break(self.input[prev_start - 1]) {
                prev_start -= 1;
            }
            end = prev_start;
            let line = &self.input[prev_start..prev_end];
            if Self::marker_prefix(line, b"---") || Self::marker_prefix(line, b"...") {
                let inline = line[3..]
                    .iter()
                    .position(|b| !matches!(b, b' ' | b'\t'))
                    .map(|at| &line[3 + at..]);
                if let Some(inline) = inline.filter(|rest| !rest.is_empty() && rest[0] != b'#') {
                    first_content = Some(inline);
                }
                break;
            }
            let lead = line.iter().take_while(|&&b| b == b' ').count();
            let rest = &line[lead..];
            if !(rest.is_empty() || rest[0] == b'#') {
                first_content = Some(rest);
            }
        }
        first_content.is_some_and(line_opens_a_bare_plain_scalar)
    }

    /// Whether the cursor is at the `|`/`>` header of a block scalar that is the document's root
    /// node (#5067): outside every container, where the plain-scalar scanner would otherwise read
    /// it and end at a `: ` in its content. Its content is measured against the document, so the
    /// callers pass base indent 0, as the `--- |` line does.
    fn at_document_root_block_scalar(&self) -> bool {
        self.type_stack.len() <= 1 && !self.json_strict && matches!(self.peek(), Some(b'|' | b'>'))
    }

    /// A block scalar whose header sits on a line of its own (#4259, and after node properties
    /// #4271), with the gap and close checks already made by the caller: its content has to be
    /// indented more than the block that owns it, which can be no more than the header itself.
    fn parse_own_line_block_scalar(&mut self, indent: usize) -> Result<(), YamlError> {
        self.attach_document_root_node(indent);
        let parent = self.innermost_block_indent();
        self.parse_block_scalar(parent)?;
        self.reject_line_deeper_than_block_after_block_scalar()
    }

    /// Whether the node properties that open the cursor's line, parsed from `start`, repeat a
    /// kind (a tag, an anchor) that the lines above them already gave the same node (#4851): the
    /// properties-only lines directly above, and the trailing properties of the dash or key line
    /// that left its value for these lines (`k: !!map`, `- &x`). A tag and an anchor split
    /// across lines are fine in either order. A line above that does not defer its value to the
    /// cursor's line names a different node, and nothing is repeated.
    fn own_line_properties_repeat_a_kind(&self, start: usize) -> bool {
        fn kinds(properties: &[u8], seen: &mut [bool; 2]) -> [bool; 2] {
            let mut found = [false; 2];
            for token in properties.split(|&b| b == b' ' || b == b'\t') {
                match token.first() {
                    Some(b'!') => found[0] = true,
                    Some(b'&') => found[1] = true,
                    _ => {}
                }
            }
            for (slot, hit) in seen.iter_mut().zip(found) {
                *slot |= hit;
            }
            found
        }
        let mut above = [false; 2];
        let current = kinds(&self.input[start..self.pos], &mut [false; 2]);
        let mut end = start;
        while end > 0 && self.input[end - 1] == b' ' {
            end -= 1;
        }
        while end > 0 {
            let mut prev_end = end - 1;
            if prev_end > 0 && self.input[prev_end] == b'\n' && self.input[prev_end - 1] == b'\r' {
                prev_end -= 1;
            }
            let mut prev_start = prev_end;
            while prev_start > 0 && !is_line_break(self.input[prev_start - 1]) {
                prev_start -= 1;
            }
            end = prev_start;
            let line = &self.input[prev_start..prev_end];
            let lead = line.iter().take_while(|&&b| b == b' ').count();
            let rest = &line[lead..];
            if rest.is_empty() || rest[0] == b'#' {
                continue;
            }
            let (_, properties) = split_trailing_properties(rest);
            if only_node_properties(rest) {
                kinds(properties, &mut above);
                continue;
            }
            if Self::deferred_value_owner(lead, rest).is_none() {
                return false;
            }
            // The owner's own trailing properties.
            kinds(properties, &mut above);
            break;
        }
        (current[0] && above[0]) || (current[1] && above[1])
    }

    /// Whether nothing but spaces sits between the start of the cursor's line and `start`, so
    /// the node properties parsed from `start` open the line (#4271).
    fn line_opens_with_properties_only(&self, start: usize) -> bool {
        let mut line_start = start;
        while line_start > 0 && !is_line_break(self.input[line_start - 1]) {
            line_start -= 1;
        }
        self.input[line_start..start].iter().all(|&b| b == b' ')
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

        // Nothing follows a document's scalar or flow-collection root before the next `---` or
        // `...`: yq stops with `did not find expected <document start>` where the line read as a
        // further document (#5070).
        if self.document_root_node_taken
            && self.type_stack.len() <= 1
            && !matches!(self.peek(), Some(b'#' | b'\n' | b'\r') | None)
        {
            return Err(
                self.err_unexpected_char(self.pos, "did not find expected <document start>")
            );
        }

        // A key, an item or a node of any other kind more indented than the block that holds the
        // plain scalar on the line above has nothing to attach to once a comment ended it (#5068).
        let ended = self.last_plain_scalar.take();
        if ended.is_some()
            && !matches!(self.peek(), Some(b'#' | b'\n' | b'\r') | None)
            && self.line_is_stray_after_a_plain_scalar(ended, indent, false)
        {
            return Err(self.err_stray_line());
        }
        // #6371: and the line after an explicit key's plain scalar that a comment ended, more
        // indented than the `?` and not the `:` of its value (`? a⏎  b # c⏎  d⏎: v`).
        if let Some(key_scalar) = self.explicit_key_scalar_start.take() {
            if self.line_is_stray_after_an_explicit_key_scalar(ended, key_scalar, indent) {
                return Err(self.err_stray_line());
            }
        }
        // #6382: and the line after one whose content is a quoted scalar or a flow collection, which
        // no comment is needed to end (`? "a"⏎  b⏎: v`).
        if core::mem::take(&mut self.explicit_key_node_closed)
            && self.line_is_beyond_the_pending_explicit_key(indent)
        {
            return Err(self.err_stray_line());
        }

        // #6540, #6543: a block scalar header on a line of its own, left of the dash or key above
        // that it would be the value of, is no value slot: yq refuses it. Judged here, ahead of
        // the arms, so a header that no arm below takes as an own-line value (the line above is a
        // chain of dashes, or ends in `?`) is refused too.
        if matches!(self.peek(), Some(b'|' | b'>'))
            && !self.json_strict
            && self.type_stack.len() > 1
        {
            if let Some(what) = self.header_is_left_of_its_owner(indent) {
                return Err(self.err_unexpected_char(self.pos, what));
            }
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
                // Same ambiguous-gap error as parse_mapping_entry (#901, #959), and
                // stricter: a `-` in a gap between two open blocks is refused whatever
                // it follows, as yq refuses it (#5086).
                self.check_mapping_under_mapping_gap(indent)?;
                // An explicit key's value must be introduced by `:`, so a
                // same-indent `-` line right after a key that held a node
                // is an error, not that key's implicit value (#1104) --
                // see `explicit_key_with_content_rejects_sequence_at`.
                if self.explicit_key_with_content_rejects_sequence_at(indent) {
                    return Err(self.err_unexpected_char(
                        self.pos,
                        "a sequence cannot be an explicit key's implicit value",
                    ));
                }
                // The same line of an ordinary `key: value` entry: the `-` has nothing to be
                // the value of, so it is an error, not a line to drop (#4530).
                if self.sequence_follows_completed_mapping_entry_at(indent) {
                    return Err(self.err_unexpected_char(
                        self.pos,
                        "a sequence cannot follow a completed mapping entry",
                    ));
                }
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
                // #6288: yq reads a `:` line only as the value of a `?` key that is waiting for one;
                // with none waiting (`: a`, `a: 1⏎: b`, a second `:` after the value) it stops with
                // `did not find expected key`, where this used to drop or keep the value.
                if self.pending_explicit_key.is_none() {
                    return Err(self.err_unexpected_char(self.pos, "did not find expected key"));
                }
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
            // A flow collection keying a mapping entry (`[a, b]: v`) is that
            // entry, not a value of its own: routed to the same entry parser
            // a scalar key takes, so a following sibling joins the mapping and
            // a nested occurrence opens one (#1188).
            Some(b'{' | b'[') if self.looks_like_flow_keyed_mapping_entry() => {
                self.parse_mapping_entry(indent)?;
            }
            Some(b'{' | b'[') => {
                // Flow mapping or sequence at document root
                // Same ambiguous-gap error as parse_mapping_entry (#901, #959).
                self.check_mapping_under_mapping_gap(indent)?;
                self.reject_node_without_owner(indent)?;
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
                    // closing unconditionally up front).
                    self.parse_mapping_entry(indent)?;
                } else {
                    // Same ambiguous-gap error as parse_mapping_entry (#901, #959).
                    self.check_mapping_under_mapping_gap(indent)?;
                    // #6413: properties on a line of their own name the node that follows, which
                    // needs an owner like any other (`k: |` / `!!map`, `a: 1` / `&a`, `- a` / `!!str`).
                    self.reject_node_without_owner(indent)?;
                    self.close_deeper_indents(indent);
                    // Consume any leading `&anchor` and/or `!tag`, in either
                    // order, for non-mapping-key cases
                    let properties_start = self.pos;
                    self.parse_node_properties()?;
                    // A tag or an anchor already given to this node on an earlier line
                    // cannot be given again (#4851): real yq stops with `did not find
                    // expected key` / `'-' indicator` on `a:` / `  !!str` / `  !!int`.
                    if self.type_stack.len() > 1
                        && self.line_opens_with_properties_only(properties_start)
                    {
                        let repeats = self.own_line_properties_repeat_a_kind(properties_start);
                        self.require_consistent_indentation(!repeats)?;
                    }
                    // Check what follows
                    match self.peek() {
                        // #6379: `&t ? x`: the properties are this node's, and a `?` after them opens
                        // no key.
                        Some(b'?') if Self::is_ws_break_or_eoi(self.peek_at(1)) => {
                            return Err(self.err_unexpected_char(
                                self.pos,
                                "mapping keys are not allowed in this context",
                            ));
                        }
                        Some(b'\n' | b'\r' | b'#') | None => {
                            // A comment after the properties is deferred past them (#4851), as
                            // it is for a property-prefixed item on the dash's own line (#784):
                            // the properties end the line and the node they name is below.
                            //
                            // Property with value on next line - will be parsed in next iteration,
                            // unless there is none (#2835): the properties open a line of their
                            // own under a bare `-` or `key:`, and the next content line stands
                            // left of them (or the input ends). That value is an empty node, and
                            // the properties need one to name -- a tag is looked up at the next
                            // BP position and an anchor records it, so without it they land on a
                            // sibling's open, or on nothing, and a following mapping key is read
                            // as the value (`a:` / `  !!str` / `b: 1` lost `b`).
                            if self.peek() == Some(b'#') {
                                self.defer_line_comment();
                                self.skip_to_eol();
                            }
                            if self.type_stack.len() > 1
                                && self.line_opens_with_properties_only(properties_start)
                                && self.own_line_properties_have_no_value(indent)
                            {
                                self.set_ib();
                                self.write_bp_open();
                                self.write_bp_close();
                            }
                        }
                        // #4271: the properties open a line of their own and the block scalar
                        // header follows them (`&x |`, `!!str >`), under a `-` or `key:` that
                        // left its value for this line -- the same slot, and the same
                        // parent-relative content indent, as the bare header below (#4259).
                        Some(b'|' | b'>')
                            if !self.json_strict
                                && self.type_stack.len() > 1
                                && self.line_opens_with_properties_only(properties_start)
                                && self.previous_content_line_defers_its_value() =>
                        {
                            self.parse_own_line_block_scalar(indent)?;
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
                            // A bare line led by node properties is judged like one without
                            // (#5068).
                            if self.line_is_stray_after_a_plain_scalar(ended, indent, true) {
                                return Err(self.err_stray_line());
                            }
                            self.attach_document_root_node(indent);
                            // The block scalar of a document-root node with properties
                            // (`&x |`, `!!str >`) takes the same parser (#5067).
                            if self.at_document_root_block_scalar() {
                                return self.parse_block_scalar(0);
                            }
                            self.set_ib();
                            self.write_bp_open();
                            // type_stack.len() == 1 means we're inside only the virtual root sequence
                            let is_truly_doc_root = self.type_stack.len() <= 1;
                            let end_pos = match self.peek() {
                                Some(b'"') => {
                                    self.parse_double_quoted()?;
                                    self.reject_deeper_line_after_value()?;
                                    self.pos
                                }
                                Some(b'\'') => {
                                    self.parse_single_quoted()?;
                                    self.reject_deeper_line_after_value()?;
                                    self.pos
                                }
                                _ => {
                                    let end = self
                                        .parse_own_line_unquoted_value(indent, is_truly_doc_root);
                                    self.check_plain_scalar()?;
                                    self.reject_content_after_inline_value()?;
                                    end
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
                    self.check_mapping_under_mapping_gap(indent)?;
                    self.reject_node_without_owner(indent)?;
                    // Standalone alias value
                    self.close_deeper_indents(indent);
                    self.attach_document_root_node(indent);
                    self.parse_alias()?;
                }
            }
            // #4259: a block scalar whose header sits on a line of its own, below the `-` or
            // `key:` that owns it (`-` / `  >` / `  text`). Its content has to be indented
            // more than that *parent* block, which may be no more than the header itself
            // (`a:` / `  >` / `  text`), where the plain-scalar path below measures it
            // against the header's own line and cut the scalar short. Inside a container
            // only: the document root has its own handling (`>` at column 0).
            Some(b'|' | b'>')
                if !self.json_strict
                    && self.type_stack.len() > 1
                    && (self.previous_content_line_defers_its_value()
                        || self.dash_above_owns_header_at(indent)) =>
            {
                self.check_mapping_under_mapping_gap(indent)?;
                // #6390: a header at the column of the bare `-` above is that item's value, so the
                // item stays open for it (`-` / `|` / `  t` is `["t\n"]`).
                let keeps_item = self.dash_above_owns_header_at(indent);
                self.close_deeper_indents(if keeps_item { indent + 1 } else { indent });
                self.parse_own_line_block_scalar(indent)?;
            }
            Some(_) => {
                // Check if this looks like a mapping entry (has `: ` on this line)
                // This handles both quoted keys ("foo": bar) and unquoted keys (foo: bar)
                //
                // #2778 considered rejecting this outright under `json_strict`
                // (bare `key: value`, no enclosing `{}`, looks like it should
                // have no JSON spelling) -- reverted (confirmed live against
                // yq v4.53.3): `-p json` without `--slurp` reads a *stream* of
                // concatenated top-level JSON values the way `jq` does
                // (`printf 'true: 1' | yq -p json '.'` prints `true` then `1`,
                // exit 0 -- the colon is simply not a value's own problem, since
                // the decoder reads `true` as a complete value and starts a new
                // read afterward). A leading token that itself starts a value
                // (`"..."`, `true`/`false`/`null`, a digit/`-`) is not something
                // this single-document parser can rightly refuse just because a
                // `:` follows; the chokepoint above still refuses a token that
                // is not a legal value start regardless (`a: 1` still errors,
                // since bare `a` was never a valid token). Full multi-value
                // streaming without `--slurp` is a real, separate gap this
                // parser doesn't support at all -- tracked as #2839, out of
                // scope here.
                if self.looks_like_mapping_entry() {
                    self.parse_mapping_entry(indent)?;
                } else {
                    // Same ambiguous-gap error as parse_mapping_entry (#901,
                    // #959) - the original repro this issue was filed
                    // against.
                    self.check_mapping_under_mapping_gap(indent)?;
                    // A bare line left over after a plain scalar is no node (#5068).
                    if self.line_is_stray_after_a_plain_scalar(ended, indent, true) {
                        return Err(self.err_stray_line());
                    }
                    self.reject_node_without_owner(indent)?;
                    // Scalar value - either bare document scalar or value in a container
                    self.close_deeper_indents(indent);
                    self.attach_document_root_node(indent);
                    // A document-root block scalar is read by the block-scalar parser, as
                    // `--- |` is, not by the plain-scalar scanner below: that scan ends at a
                    // `: ` in the content and takes what follows as a further node (#5067).
                    if self.at_document_root_block_scalar() {
                        return self.parse_block_scalar(0);
                    }
                    self.set_ib();
                    self.write_bp_open();
                    // Only use doc_root mode if we're not inside any container
                    // type_stack.len() == 1 means we're inside only the virtual root sequence
                    let is_truly_doc_root = self.type_stack.len() <= 1;
                    let end_pos = match self.peek() {
                        Some(b'"') => {
                            self.parse_double_quoted()?;
                            self.reject_deeper_line_after_value()?;
                            self.pos
                        }
                        // No `json_strict` check needed on this quote arm: the
                        // chokepoint at the top of this function already
                        // refuses `'` as a value-start byte, so `self.peek()`
                        // can never be `'` here once `json_strict` is set
                        // (#2778 review; confirmed empirically). The unquoted
                        // arm below still needs its own check -- `t`/`f`/`n`/
                        // digit/`-` all pass the chokepoint but still need
                        // full scalar-grammar validation (`True` vs `true`).
                        Some(b'\'') => {
                            self.parse_single_quoted()?;
                            self.reject_deeper_line_after_value()?;
                            self.pos
                        }
                        _ => {
                            let start = self.pos;
                            let end = self.parse_own_line_unquoted_value(indent, is_truly_doc_root);
                            self.check_plain_scalar()?;
                            self.check_json_strict_scalar(start, end)?;
                            self.reject_content_after_inline_value()?;
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

/// Whether `line` is, ignoring a trailing ` #` comment and the whitespace before it, a key
/// written up to its colon (`k:`, `- k:`).
pub(crate) fn line_ends_with_colon_before_comment(line: &[u8]) -> bool {
    let code_end = (0..line.len())
        .find(|&i| line[i] == b'#' && (i == 0 || matches!(line[i - 1], b' ' | b'\t')))
        .unwrap_or(line.len());
    let mut end = code_end;
    while end > 0 && matches!(line[end - 1], b' ' | b'\t') {
        end -= 1;
    }
    end > 0 && line[end - 1] == b':'
}

/// Whether every whitespace-separated token of `bytes`, a trailing ` #` comment aside, is a node
/// property (`&anchor`, `!tag`, `!!str`, `!<verbatim>`) -- and not a key that merely begins with
/// one (`&x:`). True for an empty slice (#4271).
pub(crate) fn only_node_properties(bytes: &[u8]) -> bool {
    let code_end = (0..bytes.len())
        .find(|&i| bytes[i] == b'#' && (i == 0 || matches!(bytes[i - 1], b' ' | b'\t')))
        .unwrap_or(bytes.len());
    bytes[..code_end]
        .split(|&b| b == b' ' || b == b'\t')
        .filter(|token| !token.is_empty())
        .all(|token| matches!(token[0], b'&' | b'!') && token[token.len() - 1] != b':')
}

/// `bytes` without its trailing spaces and tabs.
fn trim_spaces_end(bytes: &[u8]) -> &[u8] {
    let len = bytes
        .iter()
        .rposition(|&b| !matches!(b, b' ' | b'\t'))
        .map_or(0, |at| at + 1);
    &bytes[..len]
}

/// `line` (already without its leading spaces) split into the code before its node properties and
/// those properties: a trailing comment is set aside, then the `&anchor` / `!tag` tokens at the
/// end of what is left (`k: &x !!seq`, `- &x`), up to the first token that is not one. The code
/// comes back without the whitespace before the properties. A `#` opens a comment only outside a
/// quoted key (`"x # y":`), a quote opens one only at the start of a token (`it's` does not), and
/// a property never starts inside one, so `"a: !x":` has none.
pub(crate) fn split_trailing_properties(line: &[u8]) -> (&[u8], &[u8]) {
    let mut quote = None;
    let mut code_end = line.len();
    // The first byte past the last closing quote: no property token starts before it.
    let mut outside_from = 0;
    let mut i = 0;
    while i < line.len() {
        let b = line[i];
        match quote {
            Some(b'"') if b == b'\\' => i += 1,
            // `''` inside a single-quoted scalar is an escaped quote, not its end (#6392): the
            // ` # ` after it is still quoted text.
            Some(b'\'') if b == b'\'' && line.get(i + 1) == Some(&b'\'') => i += 1,
            Some(q) if b == q => {
                quote = None;
                outside_from = i + 1;
            }
            Some(_) => {}
            None if matches!(b, b'"' | b'\'')
                && (i == 0 || matches!(line[i - 1], b' ' | b'\t')) =>
            {
                quote = Some(b);
            }
            None if b == b'#' && (i == 0 || matches!(line[i - 1], b' ' | b'\t')) => {
                code_end = i;
                break;
            }
            None => {}
        }
        i += 1;
    }
    let mut code = trim_spaces_end(&line[..code_end]);
    let mut properties_start = code.len();
    loop {
        let separator = code.iter().rposition(|&b| matches!(b, b' ' | b'\t'));
        let token_start = separator.map_or(0, |at| at + 1);
        let token = &code[token_start..];
        // The first token of the line has nothing before it to make it a property of a key, so
        // it is one only if it does not end in the key's colon (`&x:`).
        let is_property = matches!(token.first(), Some(b'&' | b'!'))
            && (separator.is_some() || token.last() != Some(&b':'));
        if token_start < outside_from || !is_property {
            break;
        }
        properties_start = token_start;
        code = trim_spaces_end(&code[..token_start]);
        if separator.is_none() {
            break;
        }
    }
    let properties = &line[properties_start.min(code_end)..code_end];
    (code, properties)
}

/// Whether `line` (already without its leading spaces) ends in a `:` once a trailing comment and
/// any node properties after it (`k: &x`, `k: !!seq`) are set aside -- a key waiting for its value
/// (#4530).
fn line_ends_with_colon_after_properties(line: &[u8]) -> bool {
    split_trailing_properties(line).0.last() == Some(&b':')
}

/// Whether `rest` starts a `%YAML` directive line, which yq's header slurp sets aside in the head of a
/// stream (#6342, and indented #6408). One definition for the parser and the header writer in `light.rs`.
pub(super) fn starts_yq_yaml_directive(rest: &[u8]) -> bool {
    rest.starts_with(b"%YAML")
}

/// What [`Parser::check_yq_directive`] remembers across one run of directive lines (#6412, #6419): go-yaml refuses a
/// `%TAG` handle given twice and a second `%YAML` for one document.
#[derive(Default)]
struct DirectiveRun {
    /// The `%TAG` handles met so far, as `input` offsets.
    tag_handles: Vec<(usize, usize)>,
    /// Whether a `%YAML` that go-yaml sees (not one of yq's header) has been met.
    yaml: bool,
}

/// Whether `line` (already without its leading spaces) opens a document whose root node is a bare
/// plain scalar: not a key, an item, a flow collection, a quoted or block scalar, an alias or an
/// explicit-key line, once node properties are set aside (#5068).
fn line_opens_a_bare_plain_scalar(line: &[u8]) -> bool {
    let mut at = 0;
    while matches!(line.get(at), Some(b'&' | b'!')) {
        while line.get(at).is_some_and(|&b| !matches!(b, b' ' | b'\t')) {
            at += 1;
        }
        while matches!(line.get(at), Some(b' ' | b'\t')) {
            at += 1;
        }
    }
    let (code, _) = split_trailing_properties(&line[at..]);
    let Some(&first) = code.first() else {
        return false;
    };
    !(matches!(
        first,
        b'[' | b'{' | b'"' | b'\'' | b'|' | b'>' | b'*' | b'%' | b'#' | b'@' | b'`'
    ) || (matches!(first, b'-' | b'?' | b':')
        && code.get(1).map_or(true, |&b| matches!(b, b' ' | b'\t')))
        || code.last() == Some(&b':')
        || code
            .windows(2)
            .any(|w| w[0] == b':' && matches!(w[1], b' ' | b'\t')))
}

/// A byte that ends a tag's run of characters: a blank or a line break. One definition for
/// [`scan_tag_extent`] and for [`Parser::parse_tag`]'s check of what follows a tag (#6298).
#[inline]
fn is_tag_blank(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r')
}

/// Whether the tag at `start` (a `!`) is written verbatim, `!<...>`.
#[inline]
fn is_verbatim_tag(bytes: &[u8], start: usize) -> bool {
    bytes.get(start + 1) == Some(&b'<')
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

    if is_verbatim_tag(bytes, start) {
        i += 1;
        loop {
            match bytes.get(i) {
                Some(b'>') => return (i + 1, true),
                Some(&b) if !is_tag_blank(b) => i += 1,
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
        Some(&b) if !is_tag_blank(b) && !matches!(b, b',' | b'[' | b']' | b'{' | b'}')
    ) {
        i += 1;
    }

    (i, true)
}

/// Is `b` a byte that can appear inside a JSON-strict number token -- a
/// digit, `.`, `e`/`E`, or `+`/`-` (#2777/#2778)? Shared by
/// [`Parser::json_strict_token_end`] (finding a candidate number token's
/// extent, before it is known to be well-formed) and
/// [`json_strict_plain_scalar_ok`] (validating that extent), kept as one
/// definition so a future change to what counts as a number byte cannot
/// update one without the other.
#[inline]
fn is_json_strict_number_byte(b: u8) -> bool {
    matches!(b, b'0'..=b'9' | b'.' | b'e' | b'E' | b'+' | b'-')
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
#[inline]
fn json_strict_plain_scalar_ok(bytes: &[u8]) -> bool {
    if matches!(bytes, b"true" | b"false" | b"null") {
        return true;
    }
    match bytes.first() {
        Some(b'-' | b'0'..=b'9') => {}
        _ => return false,
    }
    if !bytes.iter().all(|b| is_json_strict_number_byte(*b)) {
        return false;
    }
    // Every byte just passed the charset check above, a strict subset of
    // ASCII, so this always succeeds -- `unsafe` is disallowed crate-wide
    // (`-D unsafe-code`), so this stays a real (cheap, ASCII-fast-pathed)
    // check rather than a skippable one.
    let Ok(s) = core::str::from_utf8(bytes) else {
        return false; // patchcov: coverage tolerate-line reason="unreachable: every byte here already passed the `[0-9.eE+-]` charset check above, a strict subset of ASCII, so `str::from_utf8` can never fail (#2778)"
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
    build_semi_index_impl(input, BuildMode::default())
}

/// [`build_semi_index`] for the input `succinctly yq` reads: a run of leading `---` markers is one
/// document, as in yq (#6349). See `Parser::yq_mode`.
///
/// # Errors
///
/// As [`build_semi_index`].
pub fn build_semi_index_yq(input: &[u8]) -> Result<SemiIndex, YamlError> {
    build_semi_index_impl(
        input,
        BuildMode {
            yq_mode: true,
            ..BuildMode::default()
        },
    )
}

/// [`build_semi_index_yq`] for text whose UTF-8 byte-order mark was dropped (#5357): yq's header
/// slurp finds no head behind a mark, so no line is read as more of it. See `Parser::yq_no_head`.
///
/// # Errors
///
/// As [`build_semi_index`].
pub fn build_semi_index_yq_no_head(input: &[u8]) -> Result<SemiIndex, YamlError> {
    build_semi_index_impl(
        input,
        BuildMode {
            yq_mode: true,
            yq_no_head: true,
            ..BuildMode::default()
        },
    )
}

/// [`build_semi_index`], enforcing real yq's own `-p json` grammar (#2279,
/// #2778, #2777) -- for callers that already know the bytes are JSON and
/// pair this with
/// [`YamlIndex::mark_json_sourced`](crate::yaml::YamlIndex::mark_json_sourced).
///
/// See `Parser::json_strict` for exactly what this does and does not
/// tighten: strict delimiters and scalar grammar in flow *sequences*
/// (#2279, #2778), but real yq's own lenient token-*pairing* grammar --
/// not stricter delimiters -- in flow *mappings* (#2777).
///
/// # Errors
///
/// As [`build_semi_index`], plus the JSON delimiter and scalar-grammar
/// violations above, which that function accepts.
pub fn build_semi_index_json_strict(input: &[u8]) -> Result<SemiIndex, YamlError> {
    build_semi_index_impl(
        input,
        BuildMode {
            json_strict: true,
            ..BuildMode::default()
        },
    )
}

/// The reading a build asks of the parser; named fields so two flags cannot be swapped.
#[derive(Clone, Copy, Default)]
struct BuildMode {
    /// `Parser::json_strict`.
    json_strict: bool,
    /// `Parser::yq_mode`.
    yq_mode: bool,
    /// `Parser::yq_no_head`.
    yq_no_head: bool,
}

fn build_semi_index_impl(input: &[u8], mode: BuildMode) -> Result<SemiIndex, YamlError> {
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
        p.json_strict = mode.json_strict;
        p.yq_mode = mode.yq_mode;
        p.yq_no_head = mode.yq_no_head;
        p.parse()
    } else {
        let mut p = Parser::<false>::new(input);
        p.json_strict = mode.json_strict;
        p.yq_mode = mode.yq_mode;
        p.yq_no_head = mode.yq_no_head;
        p.parse()
    }
}

/// The high bit of every zero byte of `x`, exactly: no byte above a zero one is flagged, so the
/// lowest flag is the first zero byte (#6287, #6369).
#[inline(always)]
fn zero_bytes(x: u64) -> u64 {
    const LOW7: u64 = 0x7F7F_7F7F_7F7F_7F7F;
    !(((x & LOW7) + LOW7) | x | LOW7)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// #6349: `build_yq` reads a run of leading `---` markers as one document, as yq's header slurp
    /// does; `build` keeps the YAML spec's reading (a null document per marker), and a marker
    /// after the first content line or after an empty later document is a document in both.
    #[test]
    fn test_a_run_of_leading_markers_is_one_document_for_yq_only_6349() {
        for (yaml, yq_docs, spec_docs) in [
            (&b"---\n---\nx\n"[..], 1, 2),
            (b"---\n---\n", 1, 2),
            (b"---\n---\n---\nx\n", 1, 3),
            (b"---\n# c\n---\nx\n", 1, 2),
            (b"---\n--- x\ny\n", 1, 2),
            (b"---\n---\nx\n---\ny\n", 2, 3),
            // Not a leading run: the same in both.
            (b"x\n---\n---\ny\n", 3, 3),
            (b"---\nx\n---\n---\ny\n", 3, 3),
            (b"---\nx\n", 1, 1),
            // A property between the markers is content, though it writes no node.
            (b"---\n&a\n---\nx\n", 2, 2),
            (b"---\n!!str\n---\nx\n", 2, 2),
        ] {
            let count = |index: crate::yaml::YamlIndex| index.document_starts().len();
            assert_eq!(
                count(crate::yaml::YamlIndex::build_yq(yaml).unwrap()),
                yq_docs,
                "build_yq {:?}",
                String::from_utf8_lossy(yaml)
            );
            assert_eq!(
                count(crate::yaml::YamlIndex::build(yaml).unwrap()),
                spec_docs,
                "build {:?}",
                String::from_utf8_lossy(yaml)
            );
        }
    }

    /// #5357: behind a byte-order mark yq's header slurp finds no head, so no line is "more of the
    /// head": a run of leading `---` markers is a null document per marker, as the YAML spec reads
    /// it (the document counts of `build`), and the index has no header. A `...` that opens the
    /// stream is still the first token go-yaml sees, which has no node to close, so it stays an
    /// error; the rest of yq's reading (a tab, a directive without its `---`) is `build_yq`'s.
    #[test]
    fn test_after_a_bom_the_leading_markers_are_not_a_head_5357() {
        for yaml in [
            &b"---\n---\nx\n"[..],
            b"---\n---\n",
            b"---\n---\n---\nx\n",
            b"---\n# c\n---\nx\n",
            b"---\n--- x\ny\n",
            b"---\nx\n",
            b"x\n---\n---\ny\n",
        ] {
            let docs = |index: crate::yaml::YamlIndex| index.document_starts().len();
            assert_eq!(
                docs(crate::yaml::YamlIndex::build_yq_after_bom(yaml).unwrap()),
                docs(crate::yaml::YamlIndex::build(yaml).unwrap()),
                "{:?}",
                String::from_utf8_lossy(yaml)
            );
        }
        // The head's other rules go with it: `...` after a leading `---` closes an empty document
        // here, where `build_yq` reads the `---` as head and has nothing for `...` to close.
        let yaml = b"---\n...\n---\nx\n";
        let docs = |index: crate::yaml::YamlIndex| index.document_starts().len();
        assert_eq!(
            docs(crate::yaml::YamlIndex::build_yq_after_bom(yaml).unwrap()),
            2
        );
        assert!(crate::yaml::YamlIndex::build_yq(yaml).is_err());
        // Still an error with nothing before it.
        for yaml in [&b"...\n"[..], b"\n...\n", b"# c\n...\n"] {
            assert!(
                crate::yaml::YamlIndex::build_yq_after_bom(yaml).is_err(),
                "{:?}",
                String::from_utf8_lossy(yaml)
            );
        }
    }

    /// #6369: the eight-bytes-a-step walk stops where the plain skip does, and its flag is exactly
    /// whether a NEL, LS or PS lies in the line it passed, under both `HAS_CR` specializations (a
    /// CR is a break only under `true`). The pieces are the bytes the word tests could confuse for
    /// each other: the lead bytes with and without their continuation, an em dash, and a byte one
    /// bit from a break, at every offset in a word and across word ends.
    #[test]
    fn skip_to_eol_noting_unicode_break_agrees_with_the_plain_skip_6369() {
        fn check<const HAS_CR: bool>(bytes: &[u8], from: usize) {
            let mut plain = Parser::<HAS_CR>::new(bytes);
            plain.pos = from;
            plain.skip_to_eol();
            let mut fast = Parser::<HAS_CR>::new(bytes);
            fast.pos = from;
            let mut saw = false;
            fast.skip_to_eol_noting_unicode_break(&mut saw);
            assert_eq!(
                fast.pos, plain.pos,
                "{bytes:?} from {from} (HAS_CR {HAS_CR})"
            );
            let want = (from..plain.pos).any(|i| block_unicode::unicode_break_len(bytes, i) > 0);
            assert_eq!(saw, want, "{bytes:?} from {from} (HAS_CR {HAS_CR})");
        }
        let pieces: [&[u8]; 14] = [
            b"a",
            b"x",
            b" ",
            b"\t",
            b"\n",
            b"\r",
            &[0xC2],
            &[0xC2, 0x85],
            &[0xE2],
            &[0xE2, 0x80],
            &[0xE2, 0x80, 0xA8],
            &[0xE2, 0x80, 0xA9],
            &[0xE2, 0x80, 0x94],
            &[0x8A],
        ];
        let mut seed = 0x2545_F491_4F6C_DD1Du64;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let mut with_break = 0;
        for _ in 0..6000 {
            let mut bytes: Vec<u8> = Vec::new();
            for _ in 0..(next() % 14) {
                // mostly plain text, so each piece lands at every alignment
                if next() % 3 == 0 {
                    bytes.extend_from_slice(pieces[(next() % pieces.len() as u64) as usize]);
                } else {
                    bytes.push(b'x');
                }
            }
            for from in 0..=bytes.len() {
                check::<true>(&bytes, from);
                // `Parser::<false>` is built for input without a CR only
                if !bytes.contains(&b'\r') {
                    check::<false>(&bytes, from);
                }
            }
            with_break += usize::from(
                (0..bytes.len()).any(|i| block_unicode::unicode_break_len(&bytes, i) > 0),
            );
        }
        assert!(with_break > 500, "only {with_break} inputs held a break");
    }

    /// #6348: a block scalar header holds one chomping character, one digit 1-9, blanks, a comment
    /// and the line break. Anything else is refused at the parser, whatever the caller, in every
    /// position a block scalar can open: this pins the loop's own one-of-each guards apart from the
    /// post-check that backs them.
    #[test]
    fn test_a_block_scalar_header_with_anything_else_is_refused_6348() {
        for yaml in [
            &b"a: |  x\n  y\n"[..],
            b"a: |x\n  y\n",
            b"a: |--\n  x\n",
            b"a: |-+\n  x\n",
            b"a: |+-\n  x\n",
            b"a: |22\n  x\n",
            b"a: |0\n  x\n",
            b"a: |-1-\n  x\n",
            b"a: |1-1\n  x\n",
            b"a: !!str |  x\n  y\n",
            b"a: &x |  x\n  y\n",
            b"- |  x\n  y\n",
            b"- &a |--\n  y\n",
            b"? |0\n  x\n: 1\n",
            b"|  x\n  y\n",
            b"--- |  x\n  y\n",
            b"a:\n  b: |  q\n    r\n",
        ] {
            assert!(
                crate::yaml::YamlIndex::build(yaml).is_err(),
                "{:?} should be refused",
                String::from_utf8_lossy(yaml)
            );
        }
    }

    /// #6348: the refusal's controls: indicators in either order, blanks, a tab and a comment (with
    /// or without a space before `#`, as yq reads it) before the break all still parse.
    #[test]
    fn test_a_block_scalar_header_of_indicators_blanks_and_a_comment_still_parses_6348() {
        for yaml in [
            &b"a: |1-\n  x\n"[..],
            b"a: |-1\n  x\n",
            b"a: |+2\n   x\n",
            b"a: |#c\n  x\n",
            b"a: | #c\n  x\n",
            b"a: |\t# c\n  x\n",
            b"a: |  \n  x\n",
            b"a: |\t\n  x\n",
            b"a: !!str |-\n  x\n",
            b"a: &x |#c\n  x\n",
            b"- >+\n  x\n\n",
            b"a: |",
        ] {
            assert!(
                crate::yaml::YamlIndex::build(yaml).is_ok(),
                "{:?} should parse",
                String::from_utf8_lossy(yaml)
            );
        }
    }

    /// #5062: a `: ` inside a trailing comment does not make a plain scalar a mapping key. yq v4.53.3
    /// reads each of these; they were rejected as `key without value`. `a#b: c` (no space before `#`)
    /// is still the key `a#b`.
    #[test]
    fn test_comment_colon_after_a_plain_scalar_is_not_a_key_5062() {
        for yaml in [
            &b"- a # b: c\n"[..],
            b"a # b: c\n",
            b"k:\n  a # b: c\n",
            b"- - a # b: c\n",
            b"- a\t# b: c\n",
            b"- &x a # b: c\n",
            b"- !t a # b: c\n",
            b"? a # b: c\n",
            b"- ubuntu-22.04-arm # TODO: update to 24.04 when https://github.com/x/y solved\n",
        ] {
            assert!(
                build_semi_index(yaml).is_ok(),
                "{:?}",
                String::from_utf8_lossy(yaml)
            );
        }
        let index = crate::yaml::YamlIndex::build(b"- a#b: c\n").unwrap();
        let root = index.root(b"- a#b: c\n");
        let crate::yaml::light::YamlValue::Sequence(docs) = root.value() else {
            panic!("virtual document sequence");
        };
        let (doc, _) = docs.uncons_cursor().unwrap();
        let crate::yaml::light::YamlValue::Sequence(items) = doc.value() else {
            panic!("a sequence document");
        };
        let (item, _) = items.uncons_cursor().unwrap();
        assert!(matches!(
            item.value(),
            crate::yaml::light::YamlValue::Mapping(_)
        ));
    }

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

    /// Escape introducer boundary captured against yq v4.53.3 (#3394).
    #[test]
    fn test_json_strict_escape_byte_matrix_3394() {
        for byte in 0..=u8::MAX {
            // Exercise both quoted mapping keys and values. A raw CR outside
            // the string selects HAS_CR even when the escape byte is not CR.
            for prefix in [b"".as_slice(), b"\r".as_slice()] {
                for (head, tail) in [
                    (b"{\"a\":\"x\\".as_slice(), b"y\"}".as_slice()),
                    (b"{\"x\\".as_slice(), b"y\":1}".as_slice()),
                ] {
                    let mut input = prefix.to_vec();
                    input.extend_from_slice(head);
                    input.push(byte);
                    if byte == b'u' {
                        input.extend_from_slice(b"0061");
                    }
                    input.extend_from_slice(tail);
                    let result = build_semi_index_json_strict(&input);
                    let valid = matches!(
                        byte,
                        b'"' | b'\\' | b'/' | b'b' | b'f' | b'n' | b'r' | b't' | b'u'
                    );
                    if valid {
                        // Decoding/output of accepted escapes is checked by
                        // the CLI regression; here we test the parser boundary.
                        assert!(result.is_ok(), "byte {byte:#04x}: {result:?}");
                    } else {
                        assert!(
                            matches!(result, Err(YamlError::UnexpectedCharacter {
                            offset,
                            context: "escape sequence in JSON string",
                            ..
                        }) if offset == prefix.len() + head.len()),
                            "byte {byte:#04x}: {result:?}"
                        );
                    }
                }
            }
        }
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
    /// space, so `a\t# c: d` loaded the comment text into the key as `{"a\t# c":"d"}`.
    /// A tab starts the comment as a space does. Since #5062 the `: ` inside it no longer
    /// makes the line a mapping entry at all, so it is the scalar `a`, as in yq v4.53.3
    /// (it was rejected as `key without value` until then).
    #[test]
    fn regression_issue_410_tab_before_hash_starts_a_comment_in_a_key() {
        for yaml in [&b"a\t# c: d\n"[..], b"a # c: d\n"] {
            let index = crate::yaml::YamlIndex::build(yaml).expect("a scalar with a comment");
            let root = index.root(yaml);
            let crate::yaml::light::YamlValue::Sequence(docs) = root.value() else {
                panic!("virtual document sequence");
            };
            let (doc, _) = docs.uncons_cursor().unwrap();
            let crate::yaml::light::YamlValue::String(s) = doc.value() else {
                panic!("expected the scalar `a`, not a mapping");
            };
            assert_eq!(s.as_str().unwrap(), "a");
        }
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
    fn yq_mode_refuses_a_directive_go_yaml_refuses_6412() {
        // The library loader skips them all; the yq build refuses what go-yaml's scanner does.
        for bad in [
            &b"%FOO\n---\na: 1\n"[..],
            b"%\n---\na: 1\n",
            b"%TAG\n---\na: 1\n",
            b"%TAG ! x: y\n---\na: 1\n",
            b"%TAG ! x:\n%TAG ! y:\n---\na: 1\n",
            b"%YAM\n---\na: 1\n",
        ] {
            assert!(build_semi_index(bad).is_ok(), "{bad:?}");
            assert!(build_semi_index_yq(bad).is_err(), "{bad:?}");
        }
        for good in [
            &b"%TAG ! x:\n---\na: 1\n"[..],
            b"%TAG !e! x # c\n%TAG !! y\n---\na: 1\n",
            b"%YAMLL 1.2\n---\na: 1\n",
        ] {
            assert!(build_semi_index_yq(good).is_ok(), "{good:?}");
        }
    }

    #[test]
    fn yq_mode_checks_a_yaml_directive_go_yaml_sees_and_a_directive_ends_a_document_6419() {
        // After a `...` or another directive a `%YAML` is go-yaml's: `1.1` alone, once.
        for bad in [
            &b"a: 1\n...\n%YAML 1.2\n---\nb: 1\n"[..],
            b"a: 1\n...\n%YAML 1.1\n%YAML 1.1\n---\nb: 1\n",
            b"a: 1\n...\n%YAML 1.1 x\n---\nb: 1\n",
            b"%TAG ! x:\n%YAML 1.2\n---\na: 1\n",
            b"a: 1\n...\n%TAG !e! w\n",
        ] {
            assert!(build_semi_index(bad).is_ok(), "{bad:?}");
            assert!(build_semi_index_yq(bad).is_err(), "{bad:?}");
        }
        // In the head, before any other directive, the lines are yq's header.
        assert!(build_semi_index_yq(b"%YAML 1.2\n%YAML 9.9\n---\na: 1\n").is_ok());
        // A directive after content ends the document, and a `---` follows.
        for ok in [
            &b"a: 1\n%TAG !e! w\n---\nb: 2\n"[..],
            b"!!str\n%TAG !e! w\n---\nb: 2\n",
            b"a: |\n  x\n%YAML 1.1\n---\nb: 2\n",
            b"---\n%TAG !e! w\n---\nb: 2\n",
        ] {
            assert!(build_semi_index_yq(ok).is_ok(), "{ok:?}");
        }
        for bad in [&b"a: 1\n%TAG !e! w\n"[..], b"a: 1\n%TAG !e! w\nb: 2\n"] {
            assert!(build_semi_index_yq(bad).is_err(), "{bad:?}");
        }
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

    /// #2886: `,` cannot start a flow node, so a flow collection with a stray
    /// comma (no entry before it) is an error in YAML input, as in go-yaml
    /// (`did not find expected node content`) -- not an empty-string key or a
    /// null element. A trailing comma stays legal.
    #[test]
    fn test_flow_stray_comma_rejected_2886() {
        for yaml in [
            "{,}",
            "{,a: 1}",
            "{a: 1,,b: 2}",
            "{a: 1, , }",
            "x: {,}",
            "[,]",
            "[,1]",
            "[1,,2]",
            "[1, , ]",
            "x: [,]",
            "- {a: [,]}",
        ] {
            assert!(
                build_semi_index(yaml.as_bytes()).is_err(),
                "{yaml:?} should be rejected"
            );
        }
        for yaml in [
            "{a: 1,}",
            "{a: 1, b: 2,}",
            "[1,]",
            "[1, 2, ]",
            "{}",
            "[]",
            "{a, b}",
            "[a, b]",
            "{ a: 1 , b: 2 }",
        ] {
            assert!(
                build_semi_index(yaml.as_bytes()).is_ok(),
                "{yaml:?} should still parse"
            );
        }
    }

    #[test]
    fn test_empty_key_in_flow_sequence() {
        // CFD4: [ : empty key ] is valid YAML 1.2, and real yq rejects it ("did not find expected
        // node content"), so `succinctly yq` follows yq (#4558). A `:` glued to its text is an item.
        let yaml = b"- [ : empty key ]";
        let result = build_semi_index(yaml);
        assert!(
            result.is_err(),
            "an empty key in a flow sequence is rejected, as yq does: {result:?}"
        );
        assert!(build_semi_index(b"- [:x]").is_ok());
        assert!(build_semi_index(b"- [:]").is_ok());
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
                let owned = to_owned::<crate::jq::YqSemantics, _>(&doc_cursor.value());
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
            // A block scalar header at the indent is the value, not a null (#6390).
            (b"x\n  |\n    t\n", 2),
            (b"x\n  >\n    t\n", 2),
            (b"x\n  |\n    t\n", 3),
            (b"x\n  |\n    t\n", 1),
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

    /// #6390: every lookahead that tells a null value from one on the next line reads a block scalar
    /// header at the owner's indent as the value, and one above or below it as it reads any other
    /// line: the full lookahead, the one that makes an exception for a sequence at the owner's
    /// indent, and the fast path for a bare dash. They share `Parser::header_at_indent`, and this
    /// pins that none of them has its own copy of the answer.
    #[test]
    fn every_lookahead_reads_a_block_scalar_header_at_the_owners_indent_as_a_value_6390() {
        for header in ["|", ">", "|-", ">+"] {
            for (indent, header_indent, null) in [
                (2usize, 2usize, false), // at the owner's indent: its value
                (2, 4, false),           // deeper: its value, as it always was
                (2, 0, true),            // shallower: the owner's value is absent
            ] {
                let input = format!(
                    "x\n{}{header}\n{}t\n",
                    " ".repeat(header_indent),
                    " ".repeat(header_indent + 2)
                );
                let input = input.as_bytes();
                let pos = 1; // on `x`'s own break
                let parser = || {
                    let mut p = Parser::<false>::new(input);
                    p.pos = pos;
                    p
                };
                assert_eq!(
                    parser().following_value_is_null(indent),
                    null,
                    "{input:?} {indent}"
                );
                assert_eq!(
                    parser().following_value_is_null_unless_sequence_at(indent),
                    null,
                    "{input:?} {indent}"
                );
                assert_eq!(
                    parser().next_line_settles_value_is_null(indent),
                    Some(null),
                    "{input:?} {indent}"
                );
            }
        }
        // A plain line at the owner's indent is still a null, and the pos is restored.
        let mut p = Parser::<false>::new(b"x\ny\n");
        p.pos = 1;
        assert!(p.following_value_is_null(0));
        assert!(p.following_value_is_null_unless_sequence_at(0));
        assert_eq!(p.pos, 1);
    }

    /// #4081: the flow-sequence lookahead that decides whether an entry is an
    /// implicit single-pair mapping skips node properties -- an anchor, an alias,
    /// and a tag in either order -- and decides on what follows them. A tag
    /// before a flow collection is a tagged element (`!Foo {a: 1}`), not a pair
    /// whose key is `!Foo {a`.
    #[test]
    fn flow_pair_lookahead_skips_node_properties_4081() {
        // (entry text starting at the element, is it an implicit pair?)
        for (entry, is_pair) in [
            ("!Foo {a: 1}]", false),
            ("!Foo [a: 1]]", false),
            ("&x !Foo {a: 1}]", false),
            ("!Foo &x {a: 1}]", false),
            ("!<tag:x,y> {a: 1}]", false),
            ("!!str {a: 1}]", false),
            ("!Foo a]", false),
            ("{a: 1}]", false),
            ("!Foo k: v]", true),
            ("!Foo \"k\": v]", true),
            ("&x !Foo k: v]", true),
            ("!Foo &x k: v]", true),
            ("!Foo {a: 1}: v]", true),
            ("!Foo {a: 1} : v]", true),
            ("{a: 1} : v]", true),
            ("[a]\t: v]", true),
            ("{a: 1} x]", false),
            ("!Foo [a]: v]", true),
            ("&x k: v]", true),
            ("*x: v]", true),
            ("k: v]", true),
        ] {
            let parser = Parser::<false>::new(entry.as_bytes());
            assert_eq!(parser.looks_like_flow_mapping_entry(), is_pair, "{entry:?}");
        }
        // An unterminated verbatim tag is the real parse's to report.
        let parser = Parser::<false>::new(b"!<tag:x {a: 1}]");
        assert!(!parser.looks_like_flow_mapping_entry());
        // And the documents parse end to end.
        for doc in [
            "a: [!Foo {a: 1}]\n",
            "[x, !Foo {a: 1}]\n",
            "a: [!!str null, !Foo {a: 1}]\n",
            "[&x !Foo {a: 1}, *x]\n",
        ] {
            assert!(
                crate::yaml::YamlIndex::build(doc.as_bytes()).is_ok(),
                "{doc:?}"
            );
        }
    }

    /// #4851: a tag, or an anchor, given to a node on one line cannot be given again on a later
    /// line of its own (real yq: `did not find expected key` / `'-' indicator`), whether the
    /// first sits on the owner's line or on a properties-only line, and across blank lines,
    /// comments and CRLF breaks. A tag with an anchor, in either order, and two nodes with one
    /// property each are fine.
    #[test]
    fn own_line_properties_repeating_a_kind_are_refused_4851() {
        for doc in [
            "a:\n  !!str\n  !!int\nb: 1\n",
            "a:\n  !!str\n    !!int\n    v\n",
            "a:\n  &x\n  &y\n  v\n",
            "a: !!str\n  !!int\nb: 1\n",
            "a: &x\n  &y\nb: 1\n",
            "a: !!map # c\n  !!str\n",
            "k: &x !!map\n  &y\n  b: 1\n",
            "- !x &y\n  !z\n  v\n",
            "- !!str\n  !!int\n- 2\n",
            "-\n  !!str\n  !!int\n  v\n",
            "- - &x\n    &y\n    b: 1\n",
            "a:\n  !!str\n\n  !!int\nb: 1\n",
            "a:\n  !!str\n  # c\n  !!int\nb: 1\n",
            "a:\n  !!str\r\n  !!int\r\nb: 1\r\n",
            "a:\n  !!str # c\n  !!int\nb: 1\n",
        ] {
            let err = crate::yaml::YamlIndex::build(doc.as_bytes()).unwrap_err();
            assert!(
                matches!(err, YamlError::InconsistentIndentation { .. }),
                "{doc:?}: {err:?}"
            );
        }
        for doc in [
            "a:\n  !!str\n  &x\nb: 1\n",
            "a:\n  &x\n  !!str\n  v\nb: 1\n",
            "a: !!map\n  &x\n  b: 1\n",
            "a: &x\n  !!map\n  b: 1\n",
            "a:\n  !!str\nb:\n  !!str\n",
            "a: !!str\n  x\nb: !!str\n  y\n",
            "- !!str\n- !!int\n",
            "x: 1\n  !!str\n",
            "\"a b\": !!str\n  &x\n  v\n",
            "a:\n  !!str # c\n  v\nb: 1\n",
            "\"a: !x\":\n  !!str\n  v\n",
            "'k: &a':\n  &b\n  v\n",
            "- \"a: &x\":\n    &y\n    v\n",
        ] {
            assert!(
                crate::yaml::YamlIndex::build(doc.as_bytes()).is_ok(),
                "{doc:?}"
            );
        }
    }

    /// #4947: a block scalar header on a line of its own below a key that carries node
    /// properties after its colon is that key's value, so the text under it is not a key and the
    /// next sibling stays one.
    #[test]
    fn own_line_block_scalar_header_below_a_key_with_properties_4947() {
        for (doc, json) in [
            ("a: &x\n  |\n  text\nb: 1\n", r#"{"a":"text\n","b":1}"#),
            ("a: !!str\n  >\n  text\nb: 1\n", r#"{"a":"text\n","b":1}"#),
            (
                "a: !!str &x\n  |-\n  text\n  more\nb: 1\n",
                r#"{"a":"text\nmore","b":1}"#,
            ),
            (
                "k:\n  a: &x\n    |\n    text\n  b: 1\nc: 2\n",
                r#"{"k":{"a":"text\n","b":1},"c":2}"#,
            ),
        ] {
            let docs = documents(doc.as_bytes());
            assert_eq!(docs.len(), 1, "{doc:?}");
            assert_eq!(docs[0].replace(' ', ""), json, "{doc:?}");
        }
    }
}
