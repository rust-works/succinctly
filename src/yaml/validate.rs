//! Strict, opt-in YAML validator.
//!
//! `succinctly` is a non-validating YAML loader by design: [`YamlIndex::build`]
//! records structure, not grammar conformance, and accepts many malformed
//! documents (see `docs/compliance/yaml/limitations.md`). This module is the
//! opt-in counterpart, mirroring [`crate::json::validate`]: a separate pass, run
//! *before* indexing, that rejects invalid YAML. The default indexing path does
//! not link it and is structurally incapable of regressing because of it.
//!
//! Like the JSON validator this is a plain scalar scanner — `no_std`, and it
//! allocates on the success path only for a document that defines anchors,
//! whose names it must remember to resolve later aliases (#404). It is **not**
//! a full YAML grammar checker:
//! it walks the document permissively and rejects only the specific classes of
//! malformed input enumerated in issue #223 (the YAML Test Suite's `lax:*`
//! cases). Everything it does not recognize as invalid, it accepts — the same
//! philosophy as the loader, but with a rejection surface bolted on.
//!
//! [`YamlIndex::build`]: crate::yaml::YamlIndex::build
//!
//! # Example
//!
//! ```
//! use succinctly::yaml::validate::validate;
//!
//! assert!(validate(b"a: 1\nb: 2\n").is_ok());
//! assert!(validate(b"a: b: c\n").is_err()); // nested mapping key
//! ```

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

use super::line_break::line_break_len;

/// Position information for error reporting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Position {
    /// Byte offset (0-indexed).
    pub offset: usize,
    /// Line number (1-indexed).
    pub line: usize,
    /// Column number (1-indexed, in bytes not characters).
    pub column: usize,
}

impl fmt::Display for Position {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "line {}, column {} (offset {})",
            self.line, self.column, self.offset
        )
    }
}

/// Kinds of YAML validation errors, grouped by the grammar family they guard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum YamlValidationErrorKind {
    // --- Scalars / quoting ---
    /// Invalid escape sequence in a double-quoted string (e.g. `"\."`).
    InvalidEscape { sequence: char },
    /// A quoted scalar was not closed before end of input.
    UnclosedQuote { quote: char },
    /// Content appears immediately after a closing quote where a separator,
    /// value indicator, or line break was required (e.g. `key2: "v" trailing`).
    TrailingContentAfterScalar,
    /// A multi-line scalar was used as an implicit mapping key.
    MultilineImplicitKey,

    // --- Block scalars ---
    /// Block scalar indent indicator was `0` or more than one digit (`|0`, `|10`).
    InvalidBlockScalarIndent,
    /// Content or a glued comment followed a block scalar header on the same
    /// line (`> text`, `>#comment`).
    ContentAfterBlockScalarHeader,

    // --- Comments ---
    /// A `#` that starts a comment was not preceded by whitespace (glued to a
    /// terminated token, e.g. `"v"#c` or `c,#c`).
    CommentNotSeparated,

    // --- Flow collections ---
    /// A comma appeared where no item precedes it (leading or doubled comma).
    UnexpectedFlowComma,
    /// Two flow nodes were not separated by a comma.
    MissingFlowSeparator,
    /// A flow collection was not closed before end of input.
    UnclosedFlow { bracket: char },
    /// A closing bracket did not match the open one, or extra content followed
    /// the top-level flow close.
    UnbalancedFlow { found: char },

    // --- Indentation / structure ---
    /// A line's indentation matches no open block level.
    BadIndentation,
    /// A bare scalar followed a block collection, or a second root node
    /// appeared at document level.
    TrailingContent,
    /// A second `:` value indicator appeared on one line (`a: b: c`).
    NestedMappingKey,

    // --- Tabs ---
    /// A tab character was used where indentation is expected.
    TabInIndentation,

    // --- Anchors / aliases ---
    /// An anchor was immediately followed by an alias (`&a *b`).
    AnchorOnAlias,
    /// An anchor was placed where it cannot bind to a following node.
    MisplacedAnchor,
    /// An alias named an anchor that is not in scope (`a: *nope`), including a
    /// forward reference — YAML 1.2 §7.1 requires an alias to name a *previous*
    /// anchor, and `yq` rejects every such alias with
    /// `unknown anchor 'nope' referenced`.
    ///
    /// The lenient loader rejects this too ([`YamlError::UnknownAnchor`], #372);
    /// without this kind the *strict* validator was the laxer of the two (#404).
    ///
    /// [`YamlError::UnknownAnchor`]: super::YamlError::UnknownAnchor
    UnknownAnchor {
        /// The referenced anchor name.
        name: String,
    },

    // --- Documents / directives ---
    /// Non-comment content followed a `...` document-end marker on its line.
    ContentAfterDocumentEnd,
    /// A directive (`%YAML`/`%TAG`) appeared where it is not allowed.
    MisplacedDirective,
    /// A `%YAML`/`%TAG` directive was malformed.
    InvalidDirective,
    /// A second `%YAML` directive appeared for one document.
    DuplicateYamlDirective,
    /// A `---`/`...` document marker appeared inside an open quoted scalar.
    DocumentMarkerInScalar,

    // --- Generic (mirrors json/validate.rs) ---
    /// Unexpected character in the given context.
    UnexpectedCharacter {
        /// What was expected.
        expected: &'static str,
        /// The character found.
        found: char,
    },
    /// Unexpected end of input.
    UnexpectedEof {
        /// What was expected.
        expected: &'static str,
    },
    /// The document is not valid UTF-8 (#1242).
    ///
    /// Previously declared but never constructed: the scanner reads bytes
    /// and never decodes them, so nothing here could detect an encoding
    /// error. `validate` now runs an explicit UTF-8 pass before the grammar
    /// walk and reports through this variant, carrying the specific reason
    /// (`Utf8ErrorKind::message`) rather than one flat string.
    InvalidUtf8 {
        /// Which UTF-8 rule the byte broke.
        reason: &'static str,
    },
    /// Container nesting depth exceeded the limit.
    NestingTooDeep {
        /// The configured limit.
        limit: usize,
    },
}

impl fmt::Display for YamlValidationErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidEscape { sequence } => {
                write!(f, "invalid escape sequence '\\{sequence}'")
            }
            Self::UnclosedQuote { quote } => write!(f, "unclosed {quote} quote"),
            Self::TrailingContentAfterScalar => {
                write!(f, "unexpected content after quoted scalar")
            }
            Self::MultilineImplicitKey => {
                write!(f, "mapping key spans multiple lines")
            }
            Self::InvalidBlockScalarIndent => {
                write!(f, "invalid block scalar indentation indicator")
            }
            Self::ContentAfterBlockScalarHeader => {
                write!(f, "unexpected content after block scalar header")
            }
            Self::CommentNotSeparated => {
                write!(f, "comment must be preceded by whitespace")
            }
            Self::UnexpectedFlowComma => write!(f, "unexpected ',' in flow collection"),
            Self::MissingFlowSeparator => {
                write!(f, "missing ',' between flow collection entries")
            }
            Self::UnclosedFlow { bracket } => write!(f, "unclosed flow collection '{bracket}'"),
            Self::UnbalancedFlow { found } => {
                write!(f, "unbalanced flow collection near '{found}'")
            }
            Self::BadIndentation => write!(f, "inconsistent indentation"),
            Self::TrailingContent => write!(f, "unexpected content after block node"),
            Self::NestedMappingKey => write!(f, "nested mapping key ('a: b: c')"),
            Self::TabInIndentation => {
                write!(f, "tab character used where indentation is expected")
            }
            Self::AnchorOnAlias => write!(f, "anchor immediately followed by an alias"),
            Self::MisplacedAnchor => write!(f, "misplaced anchor"),
            Self::UnknownAnchor { name } => write!(f, "unknown anchor '{name}' referenced"),
            Self::ContentAfterDocumentEnd => {
                write!(f, "content after document-end marker '...'")
            }
            Self::MisplacedDirective => write!(f, "misplaced directive"),
            Self::InvalidDirective => write!(f, "invalid directive"),
            Self::DuplicateYamlDirective => write!(f, "duplicate %YAML directive"),
            Self::DocumentMarkerInScalar => {
                write!(f, "document marker inside an open quoted scalar")
            }
            Self::UnexpectedCharacter { expected, found } => {
                write!(f, "expected {expected}, found {found:?}")
            }
            Self::UnexpectedEof { expected } => {
                write!(f, "unexpected end of input, expected {expected}")
            }
            Self::InvalidUtf8 { reason } => write!(f, "{reason}"),
            Self::NestingTooDeep { limit } => {
                write!(f, "nesting depth exceeds limit of {limit}")
            }
        }
    }
}

/// A YAML validation error with position information.
#[derive(Debug, Clone)]
pub struct YamlValidationError {
    /// The kind of error.
    pub kind: YamlValidationErrorKind,
    /// Position where the error occurred.
    pub position: Position,
}

impl fmt::Display for YamlValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} at {}", self.kind, self.position)
    }
}

#[cfg(feature = "std")]
impl std::error::Error for YamlValidationError {}

/// Maximum flow-collection nesting depth. Matches the parser and JSON validator
/// caps so deeply nested input fails cleanly instead of overflowing the stack.
const MAX_NESTING_DEPTH: usize = 128;

/// The structural kind of a block-context line, used for root-node consistency.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LineKind {
    /// A sequence entry (`- x`).
    Seq,
    /// A mapping entry (`k: v`, `? k`, `: v`).
    Map,
    /// A plain or quoted scalar with no value indicator.
    Scalar,
}

/// Validate YAML input strictly.
///
/// Returns `Ok(())` if the input passes the checks in this module, or an error
/// with position information for the first violation found. See the module docs
/// for what is and is not checked.
///
/// # Example
///
/// ```
/// use succinctly::yaml::validate::validate;
/// assert!(validate(b"- a\n- b\n").is_ok());
/// assert!(validate(b"[ a, , b ]\n").is_err());
/// ```
pub fn validate(input: &[u8]) -> Result<(), YamlValidationError> {
    // Encoding before grammar (#1242): the scanner below reads bytes and
    // never decodes them, so without this a document with a stray non-UTF-8
    // byte validated clean and then produced a scalar nothing could decode.
    // The JSON validator has always checked this (`validate_utf8_char`); the
    // YAML one had no encoding check at all.
    if let Err(err) = crate::text::utf8::validate_utf8(input) {
        return Err(YamlValidationError {
            kind: YamlValidationErrorKind::InvalidUtf8 {
                reason: err.kind.message(),
            },
            position: Position {
                offset: err.offset,
                line: err.line,
                column: err.column,
            },
        });
    }
    Validator::new(input).validate()
}

/// A strict YAML validator with position tracking.
pub struct Validator<'a> {
    input: &'a [u8],
    offset: usize,
    line: usize,
    column: usize,
    /// Leading-space count of the physical line currently being scanned.
    line_indent: usize,
    /// Current flow-collection nesting depth, capped at [`MAX_NESTING_DEPTH`].
    nesting_depth: usize,
    /// True while inside a document body (after `---` or first content, until
    /// `...` or a new `---`). A col-0 `%` is a directive only when this is false.
    in_document: bool,
    /// A directive was seen and no `---` has followed yet.
    directive_pending: bool,
    /// A `%YAML` directive was seen for the pending document (duplicate guard).
    yaml_directive_seen: bool,
    /// Kind of the current document's root node (set by its first indent-0
    /// content line); a later indent-0 node of a different kind is a second
    /// root. Reset at each document boundary.
    root_kind: Option<LineKind>,
    /// Open block-collection frames: the indentation column of each and its
    /// kind. Used to detect dedents that land between levels and kind mismatches
    /// at an established level. Fixed-size to keep the success path allocation-free.
    frame_indent: [u32; MAX_NESTING_DEPTH],
    frame_kind: [LineKind; MAX_NESTING_DEPTH],
    frame_len: usize,
    /// The current content line carried a trailing `# comment`.
    line_had_comment: bool,
    /// A root plain scalar was closed by a trailing comment; any further content
    /// in the same document is a second root node (BS4K, EB22).
    root_scalar_done: bool,
    /// Byte spans of the anchor names defined so far, so an alias can be checked
    /// against them (#404). Empty — and unallocated — until the first anchor.
    ///
    /// Deliberately *not* cleared at a document boundary: `yq` resolves an alias
    /// against an anchor defined in an earlier document of the same stream
    /// (`a: &x 1\n---\nb: *x` loads), and the loader's anchor table is not
    /// cleared either, so clearing here would reject input both accept.
    anchors: Vec<(usize, usize)>,
    /// True while the cursor sits where a node may begin, so a `*` there is an
    /// alias rather than plain-scalar content (`a: text *star` is the string
    /// `text *star`). Only meaningful during [`Self::scan_content_tokens`].
    at_node_start: bool,
    /// The previous content line ended inside plain-scalar content, so a
    /// following scalar line at or past its indentation continues that scalar
    /// rather than starting a node.
    prev_line_open_plain: bool,
    /// Indentation of the previous content line, paired with
    /// [`Self::prev_line_open_plain`].
    prev_line_indent: usize,
}

impl<'a> Validator<'a> {
    /// Create a new validator for the given input.
    pub fn new(input: &'a [u8]) -> Self {
        Self {
            input,
            offset: 0,
            line: 1,
            column: 1,
            line_indent: 0,
            nesting_depth: 0,
            in_document: false,
            directive_pending: false,
            yaml_directive_seen: false,
            root_kind: None,
            frame_indent: [0; MAX_NESTING_DEPTH],
            frame_kind: [LineKind::Scalar; MAX_NESTING_DEPTH],
            frame_len: 0,
            line_had_comment: false,
            root_scalar_done: false,
            anchors: Vec::new(),
            at_node_start: true,
            prev_line_open_plain: false,
            prev_line_indent: 0,
        }
    }

    /// Validate the entire input.
    pub fn validate(&mut self) -> Result<(), YamlValidationError> {
        // A byte-oriented scan is enough for the lexical families (escapes,
        // block-scalar headers, quotes, comments glued to tokens, tabs). It
        // tracks just enough context to know when a `"`/`'`/`|`/`>`/`#` is
        // significant versus scalar content. Structural families are layered on
        // in later passes.
        while self.offset < self.input.len() {
            self.scan_line()?;
        }
        // A directive with no following `---` document start (9MMA `%YAML 1.2`).
        if self.directive_pending {
            return Err(self.error(YamlValidationErrorKind::MisplacedDirective));
        }
        Ok(())
    }

    /// Scan one physical line, dispatching on its first significant token.
    fn scan_line(&mut self) -> Result<(), YamlValidationError> {
        // Leading indentation (spaces only; tabs handled per-context later).
        let start = self.offset;
        self.skip_spaces();
        self.line_indent = self.offset - start;

        // At column 0, `%` begins a directive (only outside a document body) and
        // `---`/`...` are document markers.
        if self.line_indent == 0 {
            if self.peek() == Some(b'%') && !self.in_document {
                return self.handle_directive();
            }
            if let Some(marker) = self.doc_marker_char() {
                return self.handle_document_marker(marker);
            }
        }

        match self.peek() {
            None => Ok(()),
            Some(b'\n' | b'\r') => {
                self.consume_line_break();
                Ok(())
            }
            Some(b'#') => {
                self.skip_to_line_end();
                Ok(())
            }
            // A whitespace-only line (spaces and/or tabs) is blank; it carries no
            // content and does not close the directive section (DK95/07).
            _ if self.rest_of_line_is_blank() => {
                self.skip_to_line_end();
                self.consume_line_break();
                Ok(())
            }
            // A tab in the leading indentation of a structural line (a mapping
            // key or sequence item) is not valid indentation (DK95/06). A tab
            // before a plain scalar value (DK95/00 `foo:\n \tbar`) is separation,
            // not indentation, and is allowed.
            Some(b'\t') if self.line_is_structural() => {
                Err(self.error(YamlValidationErrorKind::TabInIndentation))
            }
            _ => {
                // A directive must be followed by a `---` before any content.
                if self.directive_pending {
                    return Err(self.error(YamlValidationErrorKind::MisplacedDirective));
                }
                self.in_document = true;
                self.check_root_kind()?;
                self.check_block_indent()?;
                let result = self.scan_content_line();
                // A root plain scalar terminated by a trailing comment cannot be
                // continued by a later line (BS4K, EB22).
                if self.line_indent == 0
                    && self.root_kind == Some(LineKind::Scalar)
                    && self.line_had_comment
                {
                    self.root_scalar_done = true;
                }
                result
            }
        }
    }

    /// Track block indentation and reject dedents that land between open levels
    /// (N4JP, DMG6, 4HVU) and kind mismatches at an established level (6S55). A
    /// block sequence value legitimately sits at its mapping key's indentation,
    /// so a same-indent sequence under a mapping is allowed.
    fn check_block_indent(&mut self) -> Result<(), YamlValidationError> {
        // Anchor/alias/tag property lines do not establish or test a frame.
        if matches!(self.peek(), Some(b'&' | b'*' | b'!')) {
            return Ok(());
        }
        let d = self.line_indent as u32;
        let kind = self.line_kind();

        // Dedent: drop deeper frames.
        let mut popped = false;
        while self.frame_len > 0 && self.frame_indent[self.frame_len - 1] > d {
            self.frame_len -= 1;
            popped = true;
        }

        if self.frame_len == 0 {
            if matches!(kind, LineKind::Seq | LineKind::Map) {
                self.push_frame(d, kind)?;
            }
            return Ok(());
        }

        let top_indent = self.frame_indent[self.frame_len - 1];
        let top_kind = self.frame_kind[self.frame_len - 1];
        if top_indent == d {
            // Sibling at an established level.
            match (top_kind, kind) {
                // A block sequence value sits at its mapping key's indentation.
                (LineKind::Map, LineKind::Seq) => self.push_frame(d, LineKind::Seq)?,
                // A mapping key at the sequence's indent ends the sequence value
                // and resumes the enclosing mapping (AZ63, 7ZZ5, S9E8).
                (LineKind::Seq, LineKind::Map) => self.frame_len -= 1,
                // A scalar where a sequence expects an item (6S55).
                (LineKind::Seq, LineKind::Scalar) => {
                    return Err(self.error(YamlValidationErrorKind::BadIndentation))
                }
                _ => {}
            }
        } else {
            // `top_indent < d`: a deeper line.
            if popped && matches!(kind, LineKind::Seq | LineKind::Map) {
                // A dedent that stopped between two levels (N4JP, DMG6, 4HVU).
                return Err(self.error(YamlValidationErrorKind::BadIndentation));
            }
            if matches!(kind, LineKind::Seq | LineKind::Map) {
                self.push_frame(d, kind)?;
            }
        }
        Ok(())
    }

    /// Push a block frame, capping nesting depth.
    fn push_frame(&mut self, indent: u32, kind: LineKind) -> Result<(), YamlValidationError> {
        if self.frame_len >= MAX_NESTING_DEPTH {
            return Err(self.error(YamlValidationErrorKind::NestingTooDeep {
                limit: MAX_NESTING_DEPTH,
            }));
        }
        self.frame_indent[self.frame_len] = indent;
        self.frame_kind[self.frame_len] = kind;
        self.frame_len += 1;
        Ok(())
    }

    /// At the document root (indent 0), enforce that every top-level node is
    /// compatible with the root's kind. A mapping may carry same-indent
    /// sequence values, so `root=Map` still accepts `Seq`; but a stray scalar
    /// (236B, 7MNF, 9CWY), a mapping under a sequence (BD7L), or a mapping/
    /// sequence after a plain scalar (G7JE) is a second root node.
    fn check_root_kind(&mut self) -> Result<(), YamlValidationError> {
        if self.line_indent != 0 {
            return Ok(());
        }
        // A root scalar closed by a trailing comment admits no further content.
        if self.root_scalar_done {
            return Err(self.error(YamlValidationErrorKind::TrailingContent));
        }
        // Anchor/alias/tag properties (`&a`, `*a`, `!t`) attach to the node that
        // follows; they do not themselves establish the root kind.
        if matches!(self.peek(), Some(b'&' | b'*' | b'!')) {
            return Ok(());
        }
        let kind = self.line_kind();
        match self.root_kind {
            None => self.root_kind = Some(kind),
            Some(root) => {
                let incompatible = match root {
                    // A mapping's values may be same-indent sequences.
                    LineKind::Map => kind == LineKind::Scalar,
                    LineKind::Seq => kind != LineKind::Seq,
                    LineKind::Scalar => kind != LineKind::Scalar,
                };
                if incompatible {
                    return Err(self.error(YamlValidationErrorKind::TrailingContent));
                }
            }
        }
        Ok(())
    }

    /// Classify a block-context line (from the cursor, after leading indent) as
    /// a sequence entry, a mapping entry, or a plain/quoted scalar. A quoted
    /// region is skipped via the shared [`super::quoted_span_end`] so a `:`
    /// inside one is not miscounted — and, because a value may legally follow
    /// on a later line, a quoted key or scalar that spans lines is followed to
    /// its true close rather than stopping there, unlike
    /// [`super::line_is_structural`]'s narrower question (#382). A `"`/`'`/`#`
    /// glued to preceding content is ordinary text, gated by
    /// [`super::after_separation`] — the same rule `line_is_structural` uses,
    /// which `line_kind`'s quote arms were missing until #382.
    fn line_kind(&self) -> LineKind {
        let mut i = self.offset;
        let content_start = i;
        let first = self.input.get(i).copied();
        // Sequence entry `-` / explicit key `?` / explicit value `:`.
        if matches!(first, Some(b'-'))
            && matches!(
                self.input.get(i + 1),
                None | Some(b' ' | b'\t' | b'\n' | b'\r')
            )
        {
            return LineKind::Seq;
        }
        if matches!(first, Some(b'?' | b':'))
            && matches!(
                self.input.get(i + 1),
                None | Some(b' ' | b'\t' | b'\n' | b'\r')
            )
        {
            return LineKind::Map;
        }
        // Otherwise scan for a `:` value indicator outside quoted regions.
        while let Some(&b) = self.input.get(i) {
            match b {
                b'\n' | b'\r' => break,
                b'"' | b'\'' if super::after_separation(self.input, content_start, i) => {
                    match super::quoted_span_end(self.input, i) {
                        super::QuotedSpanEnd::ClosedSameLine(end)
                        | super::QuotedSpanEnd::ClosedAcrossLines(end) => i = end,
                        super::QuotedSpanEnd::Unterminated => break,
                    }
                }
                b'#' if super::after_separation(self.input, content_start, i) => break,
                b':' if matches!(
                    self.input.get(i + 1),
                    None | Some(b' ' | b'\t' | b'\n' | b'\r')
                ) =>
                {
                    return LineKind::Map
                }
                _ => i += 1,
            }
        }
        LineKind::Scalar
    }

    /// True if the current line (from the cursor, skipping leading whitespace)
    /// is a structural node — a sequence item (`-` + whitespace) or a block
    /// mapping entry (a `: ` value indicator before end of line) — rather than a
    /// plain scalar. Used to decide whether a leading tab is illegal indentation.
    ///
    /// Delegates to the module-level definition the loader also consults, so the
    /// two agree by construction rather than by review (#173).
    fn line_is_structural(&self) -> bool {
        super::line_is_structural(self.input, self.offset)
    }

    /// True if the line begins with a block indicator (`-`/`?`/`:`) whose
    /// separating whitespace contains a tab and is followed by another block
    /// indicator or a mapping key — a tab used as block indentation
    /// (Y79Y/004-009). A tab before a plain scalar (Y79Y/010 `-\t-1`) is allowed.
    fn tab_between_indicators(&self) -> bool {
        let is_indicator = |i: usize| {
            matches!(self.input.get(i), Some(b'-' | b'?' | b':'))
                && matches!(
                    self.input.get(i + 1),
                    None | Some(b' ' | b'\t' | b'\n' | b'\r')
                )
        };
        let mut i = self.offset;
        loop {
            if !is_indicator(i) {
                return false;
            }
            // Scan the whitespace separating this indicator from the next token.
            let mut j = i + 1;
            let mut saw_tab = false;
            while let Some(&b) = self.input.get(j) {
                match b {
                    b' ' => j += 1,
                    b'\t' => {
                        saw_tab = true;
                        j += 1;
                    }
                    _ => break,
                }
            }
            match self.input.get(j) {
                None | Some(b'\n' | b'\r') => return false,
                Some(_) if saw_tab => {
                    // Tab separates the indicator from a nested block construct.
                    return is_indicator(j) || self.mapping_key_at(j);
                }
                Some(_) if is_indicator(j) => {
                    // Another indicator with no tab yet — keep looking.
                    i = j;
                }
                _ => return false,
            }
        }
    }

    /// True if the token starting at byte `i` forms a block mapping key, i.e. a
    /// `: ` value indicator appears before the end of that line.
    fn mapping_key_at(&self, mut i: usize) -> bool {
        while let Some(&b) = self.input.get(i) {
            match b {
                b'\n' | b'\r' => return false,
                b':' if matches!(
                    self.input.get(i + 1),
                    None | Some(b' ' | b'\t' | b'\n' | b'\r')
                ) =>
                {
                    return true
                }
                _ => i += 1,
            }
        }
        false
    }

    /// True if the remainder of the current line is only whitespace (spaces or
    /// tabs) up to the next line break or end of input.
    fn rest_of_line_is_blank(&self) -> bool {
        let mut i = self.offset;
        while let Some(b) = self.input.get(i) {
            match b {
                b' ' | b'\t' => i += 1,
                b'\n' | b'\r' => return true,
                _ => return false,
            }
        }
        true
    }

    /// Handle a `%YAML`/`%TAG`/other directive line at column 0 (outside a
    /// document body). Validates `%YAML` syntax and records that a document
    /// start must follow.
    fn handle_directive(&mut self) -> Result<(), YamlValidationError> {
        self.advance(); // consume '%'
        let name_start = self.offset;
        while matches!(self.peek(), Some(b'A'..=b'Z' | b'a'..=b'z')) {
            self.advance();
        }
        let is_yaml = &self.input[name_start..self.offset] == b"YAML";
        if is_yaml {
            if self.yaml_directive_seen {
                // SF5V: two %YAML directives for one document.
                return Err(self.error(YamlValidationErrorKind::DuplicateYamlDirective));
            }
            self.yaml_directive_seen = true;
            self.validate_yaml_directive()?;
        } else {
            // %TAG and other directives are accepted permissively.
            self.skip_to_line_end();
        }
        self.consume_line_break();
        self.directive_pending = true;
        Ok(())
    }

    /// Validate the tail of a `%YAML` directive: whitespace, a version token,
    /// then only whitespace and an optional space-separated comment.
    fn validate_yaml_directive(&mut self) -> Result<(), YamlValidationError> {
        self.skip_spaces_and_tabs();
        // Version token: digits and dots (`1.2`).
        while matches!(self.peek(), Some(b'0'..=b'9' | b'.')) {
            self.advance();
        }
        // MUS6/00: a `#` glued to the version (no separating space) is invalid.
        if self.peek() == Some(b'#') {
            return Err(self.error(YamlValidationErrorKind::InvalidDirective));
        }
        self.skip_spaces_and_tabs();
        match self.peek() {
            None | Some(b'\n' | b'\r' | b'#') => Ok(()),
            // H7TQ: an extra word after the version.
            Some(_) => Err(self.error(YamlValidationErrorKind::InvalidDirective)),
        }
    }

    /// Handle a `---` or `...` document marker at column 0. `marker` is `'-'`
    /// or `'.'`. For `---`, the document root may continue on the same line.
    fn handle_document_marker(&mut self, marker: char) -> Result<(), YamlValidationError> {
        self.advance();
        self.advance();
        self.advance(); // consume the three marker bytes

        if marker == '.' {
            // `...` document end. A directive with no `---` cannot be closed by
            // `...` (B63P `%YAML 1.2\n...`).
            if self.directive_pending {
                return Err(self.error(YamlValidationErrorKind::MisplacedDirective));
            }
            self.skip_spaces_and_tabs();
            match self.peek() {
                None | Some(b'\n' | b'\r' | b'#') => {}
                // 3HFZ: `... invalid` — non-comment content after the marker.
                Some(_) => return Err(self.error(YamlValidationErrorKind::ContentAfterDocumentEnd)),
            }
            self.skip_to_line_end();
            self.consume_line_break();
            self.in_document = false;
            self.directive_pending = false;
            self.yaml_directive_seen = false;
            self.root_kind = None;
            self.root_scalar_done = false;
            self.frame_len = 0;
            // A marker closes any open plain scalar; the next line starts a node.
            self.prev_line_open_plain = false;
            Ok(())
        } else {
            // `---` document start; a document root node may follow on this line.
            self.directive_pending = false;
            self.in_document = true;
            self.yaml_directive_seen = false;
            // Content after `---` on this line is the root; the marker sits at
            // column 0 but the node may be indented past it, so root-kind
            // tracking (indent-0 only) does not apply to same-line `--- x`.
            self.root_kind = None;
            self.root_scalar_done = false;
            self.frame_len = 0;
            self.prev_line_open_plain = false;
            self.scan_content_line()
        }
    }

    /// Scan the significant content of a line (after leading indent), consuming
    /// scalars, flow collections, and comments to the line break.
    ///
    /// Wraps [`Self::scan_content_tokens`] with the cross-line bookkeeping that
    /// tells a node from the continuation of the previous line's plain scalar
    /// (#404), so both call sites — a plain line and `--- x` — get it.
    fn scan_content_line(&mut self) -> Result<(), YamlValidationError> {
        self.at_node_start = !self.continues_plain_scalar();
        let result = self.scan_content_tokens();
        // A line that ended inside plain content may be continued by the next;
        // a trailing comment closes the scalar, so it cannot be.
        self.prev_line_open_plain = !self.at_node_start && !self.line_had_comment;
        self.prev_line_indent = self.line_indent;
        result
    }

    /// True if this line continues the previous line's plain scalar rather than
    /// starting a node, so a leading `*` is scalar content — `a: text\n  *x` is
    /// the string `text *x`, and `root\n*x` is `root *x`.
    ///
    /// A continuation is a scalar line at or past the open scalar's
    /// indentation. A `-`/`?`/`:` indicator or a `: ` value indicator makes the
    /// line a node instead ([`Self::line_kind`] reports those as `Seq`/`Map`),
    /// which is what keeps an alias key (`*nope: v`) checked.
    fn continues_plain_scalar(&self) -> bool {
        self.prev_line_open_plain
            && self.line_indent >= self.prev_line_indent
            && self.line_kind() == LineKind::Scalar
    }

    /// Walk one content line's tokens. See [`Self::scan_content_line`].
    fn scan_content_tokens(&mut self) -> Result<(), YamlValidationError> {
        // A tab used as indentation between block indicators (Y79Y/004-009).
        if self.tab_between_indicators() {
            return Err(self.error(YamlValidationErrorKind::TabInIndentation));
        }
        self.line_had_comment = false;
        let parent_indent = self.line_indent;
        // A block-context line may hold at most one `: ` value indicator; a
        // second means a compact nested mapping (`a: b: c`, ZCZ6 / ZL4Z). The
        // check is suppressed on lines using anchors/aliases (`&`/`*`) or
        // explicit key/value indicators (`?`/leading `:`), which legitimately
        // carry multiple colons (anchor names may contain `:`; explicit keys
        // take compact mapping values).
        let mut seen_value_indicator = false;
        let mut suppress_nested = self.peek() == Some(b':') && self.is_value_indicator();
        // Walk tokens until the line break, honoring multi-line constructs.
        loop {
            match self.peek() {
                None => return Ok(()),
                Some(b'\n' | b'\r') => {
                    self.consume_line_break();
                    return Ok(());
                }
                // An anchor is a property, not a node: whatever follows it still
                // begins one, so `at_node_start` carries through.
                Some(b'&') if self.at_quote_start() => {
                    suppress_nested = true;
                    self.scan_anchor()?;
                }
                Some(b'*') if self.at_quote_start() => {
                    suppress_nested = true;
                    self.scan_alias(self.at_node_start)?;
                    self.at_node_start = false;
                }
                Some(b'?') => {
                    suppress_nested = true;
                    // An explicit key indicator opens a node. A `?` inside a
                    // scalar does not, even when a space follows it — the
                    // `*star` in `a: what? *star` is content.
                    self.at_node_start = self.at_node_start
                        && matches!(self.peek_at(1), None | Some(b' ' | b'\t' | b'\n' | b'\r'));
                    self.advance();
                }
                Some(b'"') if self.at_quote_start() => {
                    // A value scalar's continuation lines must be indented past
                    // the key (QB6E); a key/root scalar imposes no minimum here.
                    let min = if seen_value_indicator {
                        self.line_indent + 1
                    } else {
                        0
                    };
                    let multiline = self.scan_double_quoted(min)?;
                    self.check_after_block_quoted(multiline)?;
                    self.at_node_start = false;
                }
                Some(b'\'') if self.at_quote_start() => {
                    let min = if seen_value_indicator {
                        self.line_indent + 1
                    } else {
                        0
                    };
                    let multiline = self.scan_single_quoted(min)?;
                    self.check_after_block_quoted(multiline)?;
                    self.at_node_start = false;
                }
                Some(b':') if self.is_value_indicator() => {
                    if seen_value_indicator && !suppress_nested {
                        return Err(self.error(YamlValidationErrorKind::NestedMappingKey));
                    }
                    seen_value_indicator = true;
                    self.advance();
                    // A block sequence indicator cannot follow `: ` inline on the
                    // same line (5U3A `key: - a`); the sequence must be on its own
                    // lines. `key: -1` (a scalar starting with `-`) is fine, and an
                    // explicit value (`? k\n: - v`, suppressed) legitimately may.
                    //
                    // End-of-input counts as a terminator, so a bare `a: -` with no
                    // trailing newline is rejected too (#325). Same `None | Some(..)`
                    // spelling as `scan_anchor`'s structurally identical SY6V check.
                    let mut k = self.offset;
                    while matches!(self.input.get(k), Some(b' ' | b'\t')) {
                        k += 1;
                    }
                    if !suppress_nested
                        && self.input.get(k) == Some(&b'-')
                        && matches!(
                            self.input.get(k + 1),
                            None | Some(b' ' | b'\t' | b'\n' | b'\r')
                        )
                    {
                        return Err(self.error(YamlValidationErrorKind::TrailingContent));
                    }
                    // The entry's value node follows the indicator.
                    self.at_node_start = true;
                }
                Some(b'[' | b'{') if self.at_quote_start() => {
                    self.scan_flow()?;
                    self.check_after_top_level_flow()?;
                    self.at_node_start = false;
                }
                Some(b'|' | b'>') if self.at_block_scalar_header() => {
                    self.scan_block_scalar_header()?;
                    self.skip_block_scalar_body(parent_indent);
                    return Ok(());
                }
                Some(b'#') if self.prev_is_space_or_line_start() => {
                    self.line_had_comment = true;
                    self.skip_to_line_end();
                    return Ok(());
                }
                // Separation between tokens: whatever the cursor was expecting,
                // it still is.
                Some(b' ' | b'\t') => {
                    self.advance();
                }
                // A block sequence indicator; the item's node follows it.
                Some(b'-')
                    if self.at_node_start
                        && matches!(self.peek_at(1), None | Some(b' ' | b'\t' | b'\n' | b'\r')) =>
                {
                    self.advance();
                }
                // Plain scalar content, so no node begins at the next byte.
                Some(_) => {
                    self.advance();
                    self.at_node_start = false;
                }
            }
        }
    }

    /// True if a `:` at the cursor is a block mapping value indicator: it must
    /// be followed by whitespace, a line break, or end of input. `a:b` (no
    /// space) and `http://x` are plain-scalar content, not indicators.
    fn is_value_indicator(&self) -> bool {
        matches!(self.peek_at(1), None | Some(b' ' | b'\t' | b'\n' | b'\r'))
    }

    /// True if a `"`/`'` at the cursor begins a quoted scalar rather than being
    /// content inside a plain scalar. A quoted scalar starts only where a node
    /// can begin: at line start, after whitespace, or after a flow separator.
    fn at_quote_start(&self) -> bool {
        match self.offset.checked_sub(1).and_then(|i| self.input.get(i)) {
            None => true,
            Some(b' ' | b'\t' | b'\n' | b'\r') => true,
            Some(b'[' | b'{' | b',') => true,
            Some(_) => false,
        }
    }

    /// Consume the body of a block scalar: blank lines and lines indented deeper
    /// than the header's own line. Stops before the first line at or below the
    /// parent indentation. Body content is accepted without further scanning.
    fn skip_block_scalar_body(&mut self, parent_indent: usize) {
        loop {
            // At a line start: column is 1 here and on every rewind below.
            let line_start = self.offset;
            self.skip_spaces_and_tabs();
            let indent = self.offset - line_start;
            match self.peek() {
                None => {
                    self.offset = line_start;
                    self.column = 1;
                    return;
                }
                Some(b'\n' | b'\r') => {
                    // Blank line (only whitespace) is always part of the body.
                    self.consume_line_break();
                }
                _ if indent > parent_indent => {
                    // A more-indented content line belongs to the block scalar.
                    self.skip_to_line_end();
                    self.consume_line_break();
                }
                _ => {
                    // Line at or below parent indent ends the block scalar.
                    self.offset = line_start;
                    self.column = 1;
                    return;
                }
            }
        }
    }

    /// True if a `|`/`>` at the current position begins a block scalar header:
    /// it must be followed (after optional modifiers) by end-of-line or a
    /// space-separated comment. A `|`/`>` embedded in a plain scalar (`a|b`) is
    /// not a header.
    fn at_block_scalar_header(&self) -> bool {
        // Must be preceded by whitespace or line start to be an indicator.
        if !self.prev_is_space_or_line_start() {
            return false;
        }
        let mut i = self.offset + 1;
        // Skip up to two modifier bytes (digits / + / -).
        let mut modifiers = 0;
        while modifiers < 2 {
            match self.input.get(i) {
                Some(b'-' | b'+') => {
                    i += 1;
                    modifiers += 1;
                }
                Some(c) if c.is_ascii_digit() => {
                    i += 1;
                    modifiers += 1;
                }
                _ => break,
            }
        }
        // After modifiers, only whitespace / comment / line break / EOF.
        matches!(
            self.input.get(i),
            None | Some(b' ' | b'\t' | b'\n' | b'\r' | b'#')
        )
    }

    /// Validate a block scalar header: `|`/`>` with optional chomping (`+`/`-`)
    /// and a single non-zero indent digit, then only whitespace/comment/newline.
    fn scan_block_scalar_header(&mut self) -> Result<(), YamlValidationError> {
        self.advance(); // consume | or >

        let mut saw_indent_digit = false;
        // Up to two modifiers, order-independent (`|2-` or `|-2`).
        loop {
            match self.peek() {
                Some(b'-' | b'+') => {
                    self.advance();
                }
                Some(c) if c.is_ascii_digit() => {
                    // Indent indicator must be a single digit in 1..=9.
                    if c == b'0' || saw_indent_digit {
                        return Err(self.error(YamlValidationErrorKind::InvalidBlockScalarIndent));
                    }
                    saw_indent_digit = true;
                    self.advance();
                }
                _ => break,
            }
        }

        // Only spaces/tabs, then an optional comment, then the line break.
        self.skip_spaces_and_tabs();
        match self.peek() {
            None | Some(b'\n' | b'\r') => {
                self.consume_line_break();
                Ok(())
            }
            Some(b'#') => {
                // A comment here is only valid if separated by whitespace, which
                // skip_spaces_and_tabs already required (the header consumed at
                // least the indicator, so column > indicator). `>#c` (no space)
                // is rejected: nothing was skipped.
                if !self.prev_is_space_or_line_start() {
                    return Err(self.error(YamlValidationErrorKind::ContentAfterBlockScalarHeader));
                }
                self.skip_to_line_end();
                Ok(())
            }
            Some(_) => Err(self.error(YamlValidationErrorKind::ContentAfterBlockScalarHeader)),
        }
    }

    /// Enter a nested flow collection, erroring past [`MAX_NESTING_DEPTH`].
    fn enter_nested(&mut self) -> Result<(), YamlValidationError> {
        if self.nesting_depth >= MAX_NESTING_DEPTH {
            return Err(self.error(YamlValidationErrorKind::NestingTooDeep {
                limit: MAX_NESTING_DEPTH,
            }));
        }
        self.nesting_depth += 1;
        Ok(())
    }

    /// Scan a flow collection (`[...]` or `{...}`), entered on the open bracket.
    /// Enforces comma well-formedness (no leading/doubled comma) and bracket
    /// balance. May span multiple lines.
    fn scan_flow(&mut self) -> Result<(), YamlValidationError> {
        let open = self.peek().expect("scan_flow entered on a bracket");
        let close = if open == b'[' { b']' } else { b'}' };
        self.enter_nested()?;
        self.advance(); // consume open bracket

        // `expect_item` is true right after the open bracket or a comma: a comma
        // in that state is a leading/doubled comma (9MAG, CTN5).
        let mut expect_item = true;
        loop {
            self.skip_flow_ws()?;
            match self.peek() {
                None => {
                    return Err(self.error(YamlValidationErrorKind::UnclosedFlow {
                        bracket: open as char,
                    }))
                }
                Some(c) if c == close => {
                    self.advance();
                    self.nesting_depth -= 1;
                    return Ok(());
                }
                Some(c @ (b']' | b'}')) => {
                    return Err(
                        self.error(YamlValidationErrorKind::UnbalancedFlow { found: c as char })
                    )
                }
                Some(b',') => {
                    if expect_item {
                        return Err(self.error(YamlValidationErrorKind::UnexpectedFlowComma));
                    }
                    self.advance();
                    expect_item = true;
                }
                Some(b':') => {
                    // Mapping value indicator; the value follows.
                    self.advance();
                    expect_item = false;
                }
                Some(b'[' | b'{') => {
                    self.scan_flow()?;
                    expect_item = false;
                }
                // Every arm here is reached at a node start — after `[`, `{`,
                // `,` or `:` — so a `*` is an alias, not scalar content (#404).
                // An anchor is a property: its node still follows, so
                // `expect_item` is left alone. #452: that node must not itself
                // be an alias (`[&a *a]`) — `check_after_anchor` (shared with
                // block's `scan_anchor`) rejects that, after skipping the same
                // separation `skip_flow_ws` already provides between any two
                // flow tokens.
                Some(b'&') => {
                    let name = self.scan_anchor_name();
                    self.record_anchor(name);
                    self.skip_flow_ws()?;
                    self.check_after_anchor()?;
                }
                Some(b'*') => {
                    self.scan_alias(true)?;
                    expect_item = false;
                }
                Some(b'"') => {
                    self.scan_double_quoted(0)?;
                    expect_item = false;
                }
                Some(b'\'') => {
                    self.scan_single_quoted(0)?;
                    expect_item = false;
                }
                // A bare `-` (block sequence indicator) is not a valid flow node
                // (YJV2 `[-]`, G5U8 `[-, -]`). A `-` starting a scalar like `-1`
                // is fine — only a `-` followed by a delimiter/space is bare.
                Some(b'-')
                    if matches!(
                        self.peek_at(1),
                        None | Some(b' ' | b'\t' | b'\n' | b'\r' | b',' | b']' | b'}')
                    ) =>
                {
                    return Err(self.error(YamlValidationErrorKind::UnexpectedCharacter {
                        expected: "flow node",
                        found: '-',
                    }))
                }
                Some(_) => {
                    self.scan_flow_plain();
                    expect_item = false;
                }
            }
        }
    }

    /// Skip whitespace, line breaks, and comments between flow tokens. A `#`
    /// glued to the previous token (no separating space) is rejected.
    fn skip_flow_ws(&mut self) -> Result<(), YamlValidationError> {
        loop {
            // A `---`/`...` document marker at column 0 cannot appear inside a
            // flow collection (N782 `[\n--- ,\n...\n]`).
            if self.column == 1 {
                if let Some(marker) = self.doc_marker_char() {
                    return Err(self.error(YamlValidationErrorKind::UnexpectedCharacter {
                        expected: "flow content",
                        found: marker,
                    }));
                }
            }
            match self.peek() {
                Some(b' ' | b'\t') => {
                    self.advance();
                }
                Some(b'\n' | b'\r') => {
                    self.consume_line_break();
                    self.skip_spaces();
                }
                Some(b'#') => {
                    if !self.prev_is_space_or_line_start() {
                        return Err(self.error(YamlValidationErrorKind::CommentNotSeparated));
                    }
                    self.skip_to_line_end();
                }
                _ => return Ok(()),
            }
        }
    }

    /// Consume a plain scalar inside a flow collection, stopping before the next
    /// flow delimiter (`,` `[` `]` `{` `}`), a `:` value indicator, a line break,
    /// or a space-preceded comment.
    fn scan_flow_plain(&mut self) {
        loop {
            match self.peek() {
                None | Some(b'\n' | b'\r') => return,
                Some(b',' | b'[' | b']' | b'{' | b'}') => return,
                Some(b'#') if self.prev_is_space_or_line_start() => return,
                Some(b':') => {
                    // `:` ends the scalar only when it acts as a value indicator
                    // (followed by whitespace, a flow delimiter, or end).
                    match self.peek_at(1) {
                        None | Some(b' ' | b'\t' | b'\n' | b'\r' | b',' | b']' | b'}') => return,
                        _ => self.advance(),
                    };
                }
                Some(_) => {
                    self.advance();
                }
            }
        }
    }

    /// After a top-level flow collection closes, only whitespace, a line break,
    /// a space-separated comment, or a `:` value indicator (flow collection used
    /// as a mapping key) may follow on the line. Anything else is trailing
    /// content (4H7K `] ]`) or a glued comment (9JBA `]#`).
    fn check_after_top_level_flow(&mut self) -> Result<(), YamlValidationError> {
        // A glued comment (`]#...`) is a separation error.
        if self.peek() == Some(b'#') {
            return Err(self.error(YamlValidationErrorKind::CommentNotSeparated));
        }
        self.skip_spaces_and_tabs();
        match self.peek() {
            None | Some(b'\n' | b'\r' | b'#' | b':') => Ok(()),
            Some(_) => Err(self.error(YamlValidationErrorKind::UnbalancedFlow { found: ']' })),
        }
    }

    /// After a block-context quoted scalar closes, only whitespace, a line
    /// break, a space-separated comment, or a `:` value indicator may follow.
    /// A glued `#` (SU5Z), a bare word (Q4CL/JY7Z), or — when the scalar spanned
    /// lines and is used as a key — the `:` itself (7LBH/D49Q/JKF3) is rejected.
    fn check_after_block_quoted(&mut self, multiline: bool) -> Result<(), YamlValidationError> {
        // A glued comment (`"v"#c`, no separating space) is a separation error.
        if self.peek() == Some(b'#') {
            return Err(self.error(YamlValidationErrorKind::CommentNotSeparated));
        }
        self.skip_spaces_and_tabs();
        match self.peek() {
            Some(b':') if multiline => {
                Err(self.error(YamlValidationErrorKind::MultilineImplicitKey))
            }
            None | Some(b'\n' | b'\r' | b'#' | b':') => Ok(()),
            Some(_) => Err(self.error(YamlValidationErrorKind::TrailingContentAfterScalar)),
        }
    }

    /// Scan an anchor property (`&name`) and check its placement. An anchor may
    /// not be immediately followed by an alias (`&a *b`, SR86/SU74) nor by a
    /// block sequence indicator on the same line (`&a - x`, SY6V).
    fn scan_anchor(&mut self) -> Result<(), YamlValidationError> {
        let name = self.scan_anchor_name(); // consumes `&` and the name
        self.record_anchor(name);
        self.skip_spaces_and_tabs();
        self.check_after_anchor()?;
        match self.peek() {
            Some(b'-') if matches!(self.peek_at(1), None | Some(b' ' | b'\t' | b'\n' | b'\r')) => {
                Err(self.error(YamlValidationErrorKind::MisplacedAnchor))
            }
            _ => Ok(()),
        }
    }

    /// After an anchor property, reject an immediately following alias (`&a
    /// *b`, SR86/SU74): an anchor decorates the node that follows it, and an
    /// alias is a reference, not a node an anchor can decorate. Shared by
    /// [`Self::scan_anchor`] (block context) and `scan_flow`'s `&` arm (flow
    /// context, #452) — each skips its own context's whitespace/separation
    /// before calling this, so the cursor sits on the byte right after that.
    fn check_after_anchor(&self) -> Result<(), YamlValidationError> {
        match self.peek() {
            Some(b'*') => Err(self.error(YamlValidationErrorKind::AnchorOnAlias)),
            _ => Ok(()),
        }
    }

    /// Consume an anchor or alias token — the `&`/`*` sigil and its name — and
    /// return the name's byte span, excluding the sigil.
    ///
    /// The extent is [`super::simd::parse_anchor_name`], the definition the
    /// loader scans names with, rather than a second copy here: an alias is
    /// resolved against the names the loader recorded, so a name it reads as
    /// `a` must not be read as `a:` here (`&a: 1\nb: *a` loads under `yq`).
    /// See the #106 note in `CLAUDE.md` on predicates that diverge silently.
    ///
    /// A name holds no line break, so the column advances by its length.
    fn scan_anchor_name(&mut self) -> (usize, usize) {
        self.advance(); // `&` or `*`
        let start = self.offset;
        let end = super::simd::parse_anchor_name(self.input, start);
        self.column += end - start;
        self.offset = end;
        (start, end)
    }

    /// Record an anchor name as in scope for later aliases (#404).
    ///
    /// Registration is deliberately permissive — every `&name` the scanner
    /// reaches is recorded, whether or not it truly binds a node. An extra name
    /// can only make [`Self::scan_alias`] accept more, never reject valid input.
    fn record_anchor(&mut self, (start, end): (usize, usize)) {
        if end > start {
            self.anchors.push((start, end));
        }
    }

    /// True if `input[start..end]` names an anchor already defined.
    fn anchor_in_scope(&self, start: usize, end: usize) -> bool {
        let name = &self.input[start..end];
        self.anchors.iter().any(|&(s, e)| &self.input[s..e] == name)
    }

    /// Consume an alias token (`*name`), positioned on the `*`, and — when
    /// `checked` — reject a name no anchor has defined (#404).
    ///
    /// The lookup happens *before* consuming so the error points at the `*`, as
    /// the loader's `UnknownAnchor` offset does. `checked` is false where the
    /// `*` may be plain-scalar content instead of a node (see
    /// [`Self::at_node_start`]); an empty name is left to the loader.
    fn scan_alias(&mut self, checked: bool) -> Result<(), YamlValidationError> {
        if checked {
            let start = self.offset + 1;
            let end = super::simd::parse_anchor_name(self.input, start);
            if end > start && !self.anchor_in_scope(start, end) {
                let name = String::from_utf8_lossy(&self.input[start..end]).into_owned();
                return Err(self.error(YamlValidationErrorKind::UnknownAnchor { name }));
            }
        }
        self.scan_anchor_name();
        Ok(())
    }

    /// Scan a double-quoted scalar, validating escape sequences. May span lines.
    /// Returns `true` if the scalar spanned a line break. `min_cont_indent` is
    /// the least indentation a non-blank continuation line may have (0 disables
    /// the check); a value scalar's continuations must be indented past its key.
    fn scan_double_quoted(&mut self, min_cont_indent: usize) -> Result<bool, YamlValidationError> {
        let quote_char = '"';
        self.advance(); // opening quote
        let mut multiline = false;

        loop {
            match self.peek() {
                None => {
                    return Err(
                        self.error(YamlValidationErrorKind::UnclosedQuote { quote: quote_char })
                    )
                }
                Some(b'"') => {
                    self.advance(); // closing quote
                    return Ok(multiline);
                }
                Some(b'\\') => {
                    self.scan_double_quoted_escape()?;
                }
                Some(b'\n' | b'\r') => {
                    self.consume_line_break();
                    // A `---`/`...` at column 0 ends the document; it cannot be
                    // content of an open quoted scalar (5TRB).
                    if self.doc_marker_char().is_some() {
                        return Err(self.error(YamlValidationErrorKind::DocumentMarkerInScalar));
                    }
                    // Multi-line double-quoted: continuation lines are content.
                    self.check_continuation_indent(min_cont_indent)?;
                    multiline = true;
                }
                Some(_) => {
                    self.advance();
                }
            }
        }
    }

    /// After a line break inside a multi-line scalar, consume the leading spaces
    /// and reject a non-blank continuation line indented less than `min` (QB6E).
    fn check_continuation_indent(&mut self, min: usize) -> Result<(), YamlValidationError> {
        let start = self.offset;
        self.skip_spaces();
        let indent = self.offset - start;
        if min > 0 && indent < min && !matches!(self.peek(), None | Some(b'\n' | b'\r')) {
            return Err(self.error(YamlValidationErrorKind::BadIndentation));
        }
        Ok(())
    }

    /// Validate one `\`-escape in a double-quoted scalar. Positioned on the `\`.
    fn scan_double_quoted_escape(&mut self) -> Result<(), YamlValidationError> {
        self.advance(); // consume backslash
        let esc = match self.peek() {
            None => return Err(self.error(YamlValidationErrorKind::UnclosedQuote { quote: '"' })),
            Some(b) => b,
        };
        // Valid single-char escapes (mirrors src/yaml/light.rs escape table).
        match esc {
            b'n' | b'r' | b't' | b'\t' | b'"' | b'\\' | b'/' | b' ' | b'0' | b'a' | b'b' | b'v'
            | b'f' | b'e' | b'N' | b'_' | b'L' | b'P' => {
                self.advance();
                Ok(())
            }
            b'\n' | b'\r' => {
                // Escaped line break (line continuation).
                self.consume_line_break();
                self.skip_spaces();
                Ok(())
            }
            b'x' => self.scan_hex_escape('x', 2),
            b'u' => self.scan_hex_escape('u', 4),
            b'U' => self.scan_hex_escape('U', 8),
            _other => Err(self.error(YamlValidationErrorKind::InvalidEscape {
                sequence: crate::text::utf8::decode_char_at(self.input, self.offset),
            })),
        }
    }

    /// Validate `n` hex digits following `\x`/`\u`/`\U`. Positioned on `x`/`u`/`U`,
    /// which the caller also passes as `kind` -- reported back on a bad digit
    /// (#1636) rather than a hardcoded `'x'`, since a bad `\u`/`\U` escape must
    /// name itself, not whichever kind happened to be checked first.
    fn scan_hex_escape(&mut self, kind: char, n: usize) -> Result<(), YamlValidationError> {
        self.advance(); // consume x/u/U
        for _ in 0..n {
            match self.peek() {
                Some(c) if c.is_ascii_hexdigit() => {
                    self.advance();
                }
                _ => {
                    return Err(
                        self.error(YamlValidationErrorKind::InvalidEscape { sequence: kind })
                    )
                }
            }
        }
        Ok(())
    }

    /// Scan a single-quoted scalar. `''` is an escaped quote. May span lines.
    /// Returns `true` if the scalar spanned a line break. `min_cont_indent` bounds
    /// continuation-line indentation, as in [`Self::scan_double_quoted`].
    fn scan_single_quoted(&mut self, min_cont_indent: usize) -> Result<bool, YamlValidationError> {
        self.advance(); // opening quote
        let mut multiline = false;

        loop {
            match self.peek() {
                None => {
                    return Err(self.error(YamlValidationErrorKind::UnclosedQuote { quote: '\'' }))
                }
                Some(b'\'') => {
                    if self.peek_at(1) == Some(b'\'') {
                        // Escaped quote.
                        self.advance();
                        self.advance();
                    } else {
                        self.advance(); // closing quote
                        return Ok(multiline);
                    }
                }
                Some(b'\n' | b'\r') => {
                    self.consume_line_break();
                    // A `---`/`...` at column 0 ends the document; it cannot be
                    // content of an open quoted scalar (RXY3).
                    if self.doc_marker_char().is_some() {
                        return Err(self.error(YamlValidationErrorKind::DocumentMarkerInScalar));
                    }
                    self.check_continuation_indent(min_cont_indent)?;
                    multiline = true;
                }
                Some(_) => {
                    self.advance();
                }
            }
        }
    }

    // ---- byte-level helpers ----

    /// If the cursor is at a `---` or `...` document marker (three identical
    /// bytes followed by whitespace, a line break, or end of input), return the
    /// marker byte as a char; otherwise `None`. Assumes the cursor is at a line
    /// start (column 1); the caller checks that.
    fn doc_marker_char(&self) -> Option<char> {
        let b = self.peek()?;
        if (b == b'-' || b == b'.')
            && self.peek_at(1) == Some(b)
            && self.peek_at(2) == Some(b)
            && matches!(self.peek_at(3), None | Some(b' ' | b'\t' | b'\n' | b'\r'))
        {
            Some(b as char)
        } else {
            None
        }
    }

    /// Peek at the current byte.
    #[inline]
    fn peek(&self) -> Option<u8> {
        self.input.get(self.offset).copied()
    }

    /// Peek `n` bytes ahead of the cursor.
    #[inline]
    fn peek_at(&self, n: usize) -> Option<u8> {
        self.input.get(self.offset + n).copied()
    }

    /// True if the previous byte was a space/tab or we are at the line start.
    #[inline]
    fn prev_is_space_or_line_start(&self) -> bool {
        match self.offset.checked_sub(1).and_then(|i| self.input.get(i)) {
            None => true,
            Some(b' ' | b'\t' | b'\n' | b'\r') => true,
            Some(_) => false,
        }
    }

    /// Advance one byte, tracking line/column.
    #[inline]
    fn advance(&mut self) -> Option<u8> {
        let b = *self.input.get(self.offset)?;
        self.offset += 1;
        self.column += 1;
        Some(b)
    }

    /// Consume a line break (`\n`, `\r`, or `\r\n`), resetting the column.
    fn consume_line_break(&mut self) {
        let break_len = line_break_len(self.input, self.offset);
        if break_len > 0 {
            self.offset += break_len;
            self.line += 1;
            self.column = 1;
        }
    }

    /// Skip spaces (not tabs) at the cursor.
    fn skip_spaces(&mut self) {
        while self.peek() == Some(b' ') {
            self.offset += 1;
            self.column += 1;
        }
    }

    /// Skip spaces and tabs at the cursor.
    fn skip_spaces_and_tabs(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t')) {
            self.offset += 1;
            self.column += 1;
        }
    }

    /// Skip everything up to (not including) the next line break.
    fn skip_to_line_end(&mut self) {
        while !matches!(self.peek(), None | Some(b'\n' | b'\r')) {
            self.offset += 1;
            self.column += 1;
        }
    }

    /// Current position.
    fn position(&self) -> Position {
        Position {
            offset: self.offset,
            line: self.line,
            column: self.column,
        }
    }

    /// Build an error at the current position.
    fn error(&self, kind: YamlValidationErrorKind) -> YamlValidationError {
        YamlValidationError {
            kind,
            position: self.position(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::YamlValidationErrorKind::*;
    use super::*;

    fn kind(input: &[u8]) -> YamlValidationErrorKind {
        validate(input).unwrap_err().kind
    }

    // ========================================================================
    // Accept battery — valid YAML the validator must NOT reject. These include
    // the corpus boundary cases that earlier iterations wrongly rejected, so a
    // regression fails a *named* test rather than only the corpus scoreboard.
    // ========================================================================

    #[test]
    fn accepts_basic_structures() {
        assert!(validate(b"").is_ok());
        assert!(validate(b"# just a comment\n").is_ok());
        assert!(validate(b"a: 1\nb: 2\n").is_ok());
        assert!(validate(b"- a\n- b\n").is_ok());
        assert!(validate(b"key: \"value\"\n").is_ok());
        assert!(validate(b"key: 'value'\n").is_ok());
        assert!(validate(b"nested:\n  a: 1\n  b: 2\n").is_ok());
    }

    #[test]
    fn accepts_same_indent_sequence_value() {
        // A block sequence value may sit at its mapping key's indentation.
        assert!(validate(b"one:\n- 2\n- 3\nfour: 5\n").is_ok()); // AZ63
                                                                 // Widening the 5U3A check to end-of-input (#325) must not swallow these:
                                                                 // a `-` not followed by whitespace is an ordinary plain scalar.
        assert!(validate(b"a: -1\n").is_ok());
        assert!(validate(b"a: -1").is_ok());
        assert!(validate(b"a: -x\n").is_ok());
        assert!(validate(b"a:\n  - x\n").is_ok());
        assert!(validate(b"foo:\n- 42\nbar:\n  - 44\n").is_ok()); // RLU9
        assert!(validate(b"nested sequences:\n- - - []\n- - - {}\nkey1: []\nkey2: {}\n").is_ok());
    }

    #[test]
    fn accepts_explicit_keys_and_anchors() {
        assert!(validate(b"? a\n: -\tb\n  -  -\tc\n     - d\n").is_ok()); // A2M4
        assert!(validate(b"---\n?\n- a\n- b\n:\n- c\n- d\n").is_ok()); // 6PBE
        assert!(validate(b"&sequence\n- a\n").is_ok()); // 3R3P
        assert!(validate(b"key1: &a value\nkey2: *a\n").is_ok());
        assert!(validate(b"---\nseq:\n &anchor\n- a\n- b\n").is_ok()); // SKE5
    }

    /// Issue #404: the unknown-anchor rejection must not take valid input with
    /// it. Two ways it could: a `*` that is plain-scalar content rather than an
    /// alias, and an anchor whose definition the scanner fails to register.
    /// Every case here loads under `yq` v4.53.3 (the goldens' pinned version).
    #[test]
    fn accepts_resolvable_aliases_and_stars_in_scalars() {
        // A `*` inside a scalar is content: `{"a":"text *star"}`.
        assert!(validate(b"a: text *star\n").is_ok());
        assert!(validate(b"a: rm *.tmp\n").is_ok());
        assert!(validate(b"a: what? *star\n").is_ok());
        // ... including on the continuation line of a multi-line plain scalar,
        // where the `*` opens the line: `{"a":"text *notanalias"}`.
        assert!(validate(b"a: text\n  *notanalias\n").is_ok());
        assert!(validate(b"a:\n  text\n  *notanalias\n").is_ok());
        assert!(validate(b"root\n*notanalias\n").is_ok()); // "root *notanalias"
        assert!(validate(b"a: text &amp more\n").is_ok());
        // An anchor defined before the alias, in each position defining one.
        assert!(validate(b"a: &x 1\nb: *x\n").is_ok());
        assert!(validate(b"- &x 1\n- *x\n").is_ok());
        assert!(validate(b"a: [&x 1, *x]\n").is_ok());
        assert!(validate(b"a: {k: &x 1, j: *x}\n").is_ok());
        assert!(validate(b"? &x k\n: *x\n").is_ok());
        // The name ends at a `: ` value indicator, as the loader's scan does —
        // registering `a:` here would reject the alias below.
        assert!(validate(b"&a: 1\nb: *a\n").is_ok());
        // Anchors carry across documents for `yq` and for the loader, so an
        // alias to an earlier document's anchor is not an unresolved one.
        assert!(validate(b"a: &x 1\n---\nb: *x\n").is_ok());
    }

    /// Issue #328: every shape the loader now handles must also pass the
    /// validator, or `syq --validate` would reject documents `syq` reads fine.
    ///
    /// The validator is a separate pass, so this is not implied by the parser
    /// tests; it is asserted here so a later tightening of `scan_anchor` cannot
    /// silently take these away.
    #[test]
    fn accepts_anchored_sequence_items_with_collection_values() {
        assert!(validate(b"list:\n  - &m\n    k: v\n  - *m\n").is_ok());
        assert!(validate(b"items:\n  - &m\n    - a\n    - b\n  - *m\n").is_ok());
        assert!(validate(b"items:\n  - &first {id: 1}\n  - *first\n").is_ok());
        assert!(validate(b"items:\n  - &m [1, 2]\n  - *m\n").is_ok());
        assert!(validate(b"items:\n  - &a k: v\n  - *a\n").is_ok());
        assert!(validate(b"list:\n  - &m # note: here\n    k: v\n  - *m\n").is_ok());
        assert!(validate(b"items:\n  - &m\n  - *m\n").is_ok());
        assert!(validate(b"items:\n  - &m\n").is_ok());
        assert!(validate(b"items:\n  - &m |\n    line\n  - *m\n").is_ok());
        assert!(validate(b"? k\n: - &m\n    a: 1\n").is_ok());
        assert!(validate(b"a: { &e e: f }\nb: *e\n").is_ok());
    }

    /// Issue #339: every shape the loader now reads as an explicit key at
    /// block-sequence-item position must also pass the validator, or
    /// `syq --validate` would reject documents `syq` reads fine.
    ///
    /// The validator is a separate pass with its own line classifier, so this
    /// is not implied by the parser tests — it is asserted here so a later
    /// tightening of `LineKind` detection cannot silently take these away.
    #[test]
    fn accepts_explicit_keys_as_sequence_items() {
        assert!(validate(b"- ? e\n  : v\n").is_ok());
        assert!(validate(b"- ? e\n").is_ok());
        assert!(validate(b"- ? e\n  : v\n- x\n").is_ok());
        assert!(validate(b"- ? e\n  : v\n  ? f\n  : w\n").is_ok());
        assert!(validate(b"- ? e\n  : v\n  g: h\n").is_ok());
        assert!(validate(b"- ? e\n- ? f\n").is_ok());
        assert!(validate(b"- ?\n    e\n  : v\n").is_ok());
        assert!(validate(b"- ? \"q k\"\n  : v\n").is_ok());
        assert!(validate(b"- ? |\n    lit\n  : v\n").is_ok());
        assert!(validate(b"- ? [1, 2]\n  : v\n").is_ok());
        assert!(validate(b"-   ? e\n    : v\n").is_ok());
        assert!(validate(b"k:\n  - ? e\n    : v\n").is_ok());
        assert!(validate(b"- ? &a e\n  : v\n").is_ok());
        assert!(validate(b"- ? e\n  : &a v\n- *a\n").is_ok());
        assert!(validate(b"? k\n: - ? e\n    : v\n").is_ok());
    }

    /// Issue #346: the same-line spelling `? k: v`, in which the whole `k: v` is
    /// a compact block mapping used as the key, must pass the validator too.
    ///
    /// Same reason as [`accepts_explicit_keys_as_sequence_items`]: the validator
    /// has its own `LineKind` classifier and does not inherit the parser fix, so
    /// `syq --validate` could reject documents `syq` now reads correctly.
    #[test]
    fn accepts_an_explicit_key_and_its_value_indicator_on_one_line() {
        assert!(validate(b"? k: v\n").is_ok());
        assert!(validate(b"m:\n  ? k: v\n").is_ok());
        assert!(validate(b"- ? k: v\n").is_ok());
        assert!(validate(b"? k: v\n: w\n").is_ok());
        assert!(validate(b"? k: v\n  j: u\n").is_ok());
        assert!(validate(b"? k: v\nj: u\n").is_ok());
        assert!(validate(b"?   k: v\n    j: u\n: w\n").is_ok());
        assert!(validate(b"? \"a\": v\n").is_ok());
        assert!(validate(b"? 'a': v\n").is_ok());
        assert!(validate(b"? : x\n").is_ok());
        // The value indicator carries the same spelling
        assert!(validate(b"? a\n: b: c\n").is_ok());
        assert!(validate(b"? a: b\n: c: d\n").is_ok());
        // YAML Test Suite case V9D5 - needs the key and value arms together
        assert!(validate(b"- sun: yellow\n- ? earth: blue\n  : moon: white\n").is_ok());
        assert!(validate(b"? - a: b\n: v\n").is_ok());
        assert!(validate(b"- ? k: v\n- z\n").is_ok());
        assert!(validate(b"? k: v\r\n: w\r\n").is_ok());
    }

    #[test]
    fn accepts_multiline_and_folded_scalars() {
        assert!(validate(b"a\nb\n c\nd\n\ne\n").is_ok()); // 9YRD plain multiline
        assert!(validate(b"a: b\n c\nd:\n").is_ok()); // A984
                                                      // Root multi-line double-quoted scalar folding at column 0.
        assert!(validate(b"\"folded \nto a space,\nto a line feed\"\n").is_ok());
        assert!(validate(b"key: \"a\n  b\n  c\"\n").is_ok()); // indented continuation
    }

    #[test]
    fn accepts_tabs_as_separation() {
        assert!(validate(b"-\t-1\n").is_ok()); // Y79Y/010: tab before a scalar
        assert!(validate(b"- foo:\t bar\n- - baz\n  -\tbaz\n").is_ok()); // 6BCT
        assert!(validate(b"foo:\n \tbar\n").is_ok()); // DK95/00: tab before scalar value
        assert!(validate(b"\t{}\n").is_ok()); // Q5MG: tab before root node
    }

    #[test]
    fn accepts_directives_and_document_markers() {
        assert!(validate(b"%YAML 1.2\n---\nDocument\n... # Suffix\n").is_ok()); // RTP8
        assert!(validate(b"Document\n---\n# Empty\n...\n%YAML 1.2\n---\nmatches %: 20\n").is_ok()); // 6ZKB
        assert!(validate(b"---\nscalar\n%YAML 1.2\n").is_ok()); // XLQ9: %YAML as scalar content
        assert!(validate(b"%YAML 1.2 # comment\n---\n").is_ok());
    }

    #[test]
    fn accepts_flow_collections() {
        assert!(validate(b"[a, b, c]\n").is_ok());
        assert!(validate(b"[ a, b, c, ]\n").is_ok()); // trailing comma is valid
        assert!(validate(b"{a: 1, b: 2}\n").is_ok());
        assert!(validate(b"flow: [a,\n  b,\n  c]\n").is_ok());
        assert!(validate(b"{\"foo\"\n: \"bar\"}\n").is_ok()); // 4MUZ quoted key over lines
        assert!(validate(b"[-1, -2]\n").is_ok()); // '-' starting a scalar
    }

    // ========================================================================
    // Reject battery — one representative per rule, tagged with its corpus id.
    // ========================================================================

    #[test]
    fn rejects_invalid_escapes() {
        assert!(matches!(
            kind(b"---\n\"\\.\"\n"),
            InvalidEscape { sequence: '.' }
        )); // 55WF
        assert!(matches!(
            kind(b"---\ndouble: \"quoted \\' scalar\"\n"),
            InvalidEscape { sequence: '\'' }
        )); // HRE5
    }

    #[test]
    fn rejects_invalid_escape_decodes_multibyte_char_1422() {
        // The byte after `\` can be the lead byte of a multi-byte UTF-8
        // sequence; the reported `sequence` must be the real character, not
        // a Latin-1 cast of its lead byte (#1422).
        assert!(matches!(
            kind("---\na: \"\\日\"\n".as_bytes()),
            InvalidEscape { sequence: '日' }
        ));
        assert!(matches!(
            kind("---\na: \"\\🎉\"\n".as_bytes()),
            InvalidEscape { sequence: '🎉' }
        ));
    }

    #[test]
    fn accepts_valid_escapes() {
        assert!(validate(b"a: \"tab\\tnl\\n hex \\x41 \\u0041 \\U00000041\"\n").is_ok());
        assert!(validate(b"a: \"q \\\" s \\/ b \\\\ nbsp \\_ next \\N\"\n").is_ok());
    }

    /// #1636: `scan_hex_escape` used to hardcode `sequence: 'x'` regardless
    /// of which escape kind (`\x`/`\u`/`\U`) called it, so a bad `\u`/`\U`
    /// escape was misreported as `\x`. Only `\x` itself happened to report
    /// correctly, by coincidence -- covering all three kinds here so a
    /// regression in any one of them fails a named assertion.
    #[test]
    fn rejects_bad_hex_digit_reports_the_real_escape_kind_1636() {
        assert!(matches!(
            kind(b"---\na: \"\\xZZ\"\n"),
            InvalidEscape { sequence: 'x' }
        ));
        assert!(matches!(
            kind(b"---\na: \"\\uZZZZ\"\n"),
            InvalidEscape { sequence: 'u' }
        ));
        assert!(matches!(
            kind(b"---\na: \"\\UZZZZZZZZ\"\n"),
            InvalidEscape { sequence: 'U' }
        ));
    }

    #[test]
    fn rejects_quoting_and_trailing_content() {
        assert!(matches!(
            kind(b"key: \"unterminated\n"),
            UnclosedQuote { quote: '"' }
        ));
        assert!(matches!(
            kind(b"key1: \"quoted1\"\nkey2: \"quoted2\" trailing content\n"),
            TrailingContentAfterScalar
        )); // Q4CL
        assert!(matches!(kind(b"\"a\nb\": 1\n"), MultilineImplicitKey)); // 7LBH
        assert!(matches!(kind(b"'a\nb': 1\n"), MultilineImplicitKey)); // D49Q
    }

    #[test]
    fn rejects_block_scalar_headers() {
        assert!(matches!(kind(b"--- |0\n"), InvalidBlockScalarIndent)); // 2G84/00
        assert!(matches!(kind(b"--- |10\n"), InvalidBlockScalarIndent)); // 2G84/01
        assert!(matches!(
            kind(b"block: >#comment\n  scalar\n"),
            ContentAfterBlockScalarHeader
        )); // X4QW
        assert!(matches!(
            kind(b"---\nfolded: > first line\n  second line\n"),
            ContentAfterBlockScalarHeader
        )); // S4GJ
    }

    #[test]
    fn accepts_valid_block_scalars() {
        assert!(validate(b"a: |\n  text\n  more\n").is_ok());
        assert!(validate(b"a: >2-\n  folded\n").is_ok());
        assert!(validate(b"a: | # comment\n  text\nb: c\n").is_ok());
    }

    #[test]
    fn rejects_flow_wellformedness() {
        assert!(matches!(kind(b"---\n[ , a, b, c ]\n"), UnexpectedFlowComma)); // 9MAG
        assert!(matches!(
            kind(b"---\n[ a, b, c, , ]\n"),
            UnexpectedFlowComma
        )); // CTN5
        assert!(matches!(
            kind(b"---\n[ a, b, c ] ]\n"),
            UnbalancedFlow { .. }
        )); // 4H7K
        assert!(matches!(
            kind(b"---\n[ a, b, c,#invalid\n]\n"),
            CommentNotSeparated
        )); // CVW2
        assert!(matches!(
            kind(b"[-]\n"),
            UnexpectedCharacter { found: '-', .. }
        )); // YJV2
        assert!(matches!(kind(b"[ a, b"), UnclosedFlow { bracket: '[' }));
    }

    #[test]
    fn rejects_nested_mapping_keys() {
        assert!(matches!(kind(b"a: b: c: d\n"), NestedMappingKey)); // ZCZ6
        assert!(matches!(kind(b"---\na: 'b': c\n"), NestedMappingKey)); // ZL4Z
    }

    #[test]
    fn rejects_bad_indentation() {
        assert!(matches!(
            kind(b"map:\n  key1: \"quoted1\"\n key2: \"bad indentation\"\n"),
            BadIndentation
        )); // N4JP
        assert!(matches!(
            kind(b"key:\n  ok: 1\n wrong: 2\n"),
            BadIndentation
        )); // DMG6
        assert!(matches!(
            kind(b"key:\n - bar\n - baz\n invalid\n"),
            BadIndentation
        )); // 6S55
        assert!(matches!(
            kind(b"---\nquoted: \"a\nb\nc\"\n"),
            BadIndentation
        )); // QB6E
    }

    #[test]
    fn rejects_second_root_node() {
        assert!(matches!(kind(b"foo:\n  bar\ninvalid\n"), TrailingContent)); // 236B
        assert!(matches!(
            kind(b"- item1\n- item2\ninvalid: x\n"),
            TrailingContent
        )); // BD7L
        assert!(matches!(
            kind(b"- item1\n- item2\ninvalid\n"),
            TrailingContent
        )); // TD5N
        assert!(matches!(kind(b"a\nb: 1\nc\n d: 1\n"), TrailingContent)); // G7JE
        assert!(matches!(kind(b"key: - a\n     - b\n"), TrailingContent)); // 5U3A
        assert!(matches!(kind(b"a: - x\n"), TrailingContent)); // 5U3A, single line
        assert!(matches!(kind(b"a: -\n"), TrailingContent)); // bare `-` as value
        assert!(matches!(kind(b"a: -"), TrailingContent)); // ...and at end of input (#325)
        assert!(matches!(
            kind(b"word1  # comment\nword2\n"),
            TrailingContent
        )); // BS4K

        // #382: line_kind used to scan straight through this quoted scalar's
        // own line break (absorbed inside what it wrongly treated as a fresh
        // reopened quote on line 2), landing on line 3's `c:` and reporting
        // Map for line 1 — silently accepting an incompatible second root.
        assert!(matches!(
            kind(b"\"line one\n line two\"\nc: d\n"),
            TrailingContent
        ));
        assert!(matches!(
            kind(b"'line one\n line two'\nc: d\n"),
            TrailingContent
        ));
    }

    /// #382: `line_kind`'s quote arms used to trigger on *any* `"`/`'` byte,
    /// unlike `line_is_structural`'s `after_separation` gating — so a quote
    /// glued to preceding scalar content was misread as opening a span,
    /// swallowing the line's real `:` and wrongly rejecting the entry after it.
    #[test]
    fn accepts_a_glued_quote_before_the_real_value_indicator() {
        assert!(validate(b"foo'bar: baz\nqux: quux\n").is_ok());
        assert!(validate(b"foo\"bar: baz\nqux: quux\n").is_ok());
    }

    #[test]
    fn rejects_tabs_in_indentation() {
        assert!(matches!(kind(b"-\t-\n"), TabInIndentation)); // Y79Y/004
        assert!(matches!(kind(b"?\tkey:\n"), TabInIndentation)); // Y79Y/008
        assert!(matches!(
            kind(b"foo:\n  a: 1\n  \tb: 2\n"),
            TabInIndentation
        )); // DK95/06
    }

    #[test]
    fn rejects_anchor_placement() {
        assert!(matches!(
            kind(b"key1: &a value\nkey2: &b *a\n"),
            AnchorOnAlias
        )); // SR86
        assert!(matches!(
            kind(b"&anchor - sequence entry\n"),
            MisplacedAnchor
        )); // SY6V

        // #452: `scan_flow`'s `&` arm (shared by `[...]` and `{...}`) had no
        // placement check — it recorded the anchor and read a following
        // `*alias` as an ordinary reference, which also passed the unrelated
        // #404 unknown-anchor check since the anchor was just registered into
        // scope.
        assert!(matches!(kind(b"[&a *a]\n"), AnchorOnAlias));
        assert!(matches!(kind(b"{k: &a *a}\n"), AnchorOnAlias));
    }

    /// #452: unlike block's `scan_anchor`, which skips only same-line
    /// spaces/tabs before this check, flow's equivalent runs after
    /// `skip_flow_ws`, which also crosses line breaks and comments — so an
    /// anchor and alias split across either still decorate the same node and
    /// must still be rejected, with the error still pointing at the `*`.
    #[test]
    fn rejects_anchor_on_alias_across_flow_separation() {
        for input in [&b"[&a\n*a]\n"[..], b"[&a # note\n*a]\n"] {
            let err = validate(input).unwrap_err();
            let shown = String::from_utf8_lossy(input);
            assert!(
                matches!(err.kind, AnchorOnAlias),
                "{shown:?}: {:?}",
                err.kind
            );
            assert_eq!(
                input.get(err.position.offset),
                Some(&b'*'),
                "{shown:?}: error should point at the alias sigil, not {:?}",
                err.position
            );
        }
    }

    /// Issue #404: an alias naming an anchor that is not in scope. `yq` v4.53.3
    /// rejects every case below with `unknown anchor 'nope' referenced`, and
    /// `yaml validate` is documented as the yq-conformance gate, so accepting
    /// them left a CI check green on input `yq` refuses.
    ///
    /// Table-driven over the positions an alias can occupy — a value, a
    /// sequence item, an explicit key, an implicit key, and both flow
    /// collections — because they reach the check through different scanners
    /// (`scan_content_tokens` and `scan_flow`), and a position added later that
    /// forgets to check fails here. The position is asserted too: reporting
    /// some other plausible offset would still pass a kind-only assertion.
    #[test]
    fn rejects_alias_to_unknown_anchor() {
        for input in [
            &b"a: *nope\n"[..],
            b"- *nope\n",
            b"? *nope\n: v\n",
            b"*nope: v\n",
            b"a: [*nope]\n",
            b"a: {k: *nope}\n",
            b"[*nope]\n",
            b"a: &x 1\nb: [1, *nope]\n",
        ] {
            let err = validate(input).unwrap_err();
            let shown = String::from_utf8_lossy(input);
            assert!(
                matches!(&err.kind, UnknownAnchor { name } if name == "nope"),
                "{shown:?}: {:?}",
                err.kind
            );
            assert_eq!(
                input.get(err.position.offset),
                Some(&b'*'),
                "{shown:?}: error should point at the alias sigil, not {:?}",
                err.position
            );
        }
        // A forward reference: the anchor exists, but not yet. YAML 1.2 §7.1
        // requires an alias to name a *previous* anchor.
        assert!(matches!(
            kind(b"a: *x\nb: &x 5\n"),
            UnknownAnchor { name } if name == "x"
        ));
    }

    #[test]
    fn rejects_document_and_directive_errors() {
        assert!(matches!(
            kind(b"---\nkey: value\n... invalid\n"),
            ContentAfterDocumentEnd
        )); // 3HFZ
        assert!(matches!(kind(b"%YAML 1.2\n"), MisplacedDirective)); // 9MMA
        assert!(matches!(kind(b"%YAML 1.2\n...\n"), MisplacedDirective)); // B63P
        assert!(matches!(kind(b"%YAML 1.2 foo\n---\n"), InvalidDirective)); // H7TQ
        assert!(matches!(kind(b"%YAML 1.1#...\n---\n"), InvalidDirective)); // MUS6/00
        assert!(matches!(
            kind(b"%YAML 1.2\n%YAML 1.2\n---\n"),
            DuplicateYamlDirective
        )); // SF5V
        assert!(matches!(
            kind(b"---\n\"\n---\n\"\n"),
            DocumentMarkerInScalar
        )); // 5TRB
    }

    // ========================================================================
    // Positions, nesting cap, and Display.
    // ========================================================================

    #[test]
    fn reports_error_position() {
        let err = validate(b"a: 1\nb: c: d\n").unwrap_err();
        assert_eq!(err.position.line, 2);
    }

    #[test]
    fn deep_flow_nesting_errors_instead_of_overflowing() {
        let input = "[".repeat(20_000);
        assert!(matches!(
            validate(input.as_bytes()).unwrap_err().kind,
            NestingTooDeep { .. }
        ));
    }

    #[test]
    fn deep_block_nesting_errors_instead_of_overflowing() {
        // Each level indents one more space and opens a mapping.
        let mut input = String::new();
        for i in 0..20_000 {
            for _ in 0..i {
                input.push(' ');
            }
            input.push_str("k:\n");
        }
        assert!(matches!(
            validate(input.as_bytes()).unwrap_err().kind,
            NestingTooDeep { .. }
        ));
    }

    #[test]
    fn error_display_is_nonempty_for_all_kinds() {
        let kinds = [
            InvalidEscape { sequence: 'q' },
            UnclosedQuote { quote: '"' },
            TrailingContentAfterScalar,
            MultilineImplicitKey,
            InvalidBlockScalarIndent,
            ContentAfterBlockScalarHeader,
            CommentNotSeparated,
            UnexpectedFlowComma,
            MissingFlowSeparator,
            UnclosedFlow { bracket: '[' },
            UnbalancedFlow { found: ']' },
            BadIndentation,
            TrailingContent,
            NestedMappingKey,
            TabInIndentation,
            AnchorOnAlias,
            MisplacedAnchor,
            UnknownAnchor {
                name: "nope".into(),
            },
            ContentAfterDocumentEnd,
            MisplacedDirective,
            InvalidDirective,
            DuplicateYamlDirective,
            DocumentMarkerInScalar,
            UnexpectedCharacter {
                expected: "x",
                found: 'y',
            },
            UnexpectedEof { expected: "x" },
            InvalidUtf8 {
                reason: "invalid UTF-8 lead byte",
            },
            NestingTooDeep { limit: 128 },
        ];
        for k in &kinds {
            assert!(!k.to_string().is_empty());
        }
        // Full error Display includes the position.
        let err = YamlValidationError {
            kind: NestedMappingKey,
            position: Position {
                offset: 3,
                line: 1,
                column: 4,
            },
        };
        assert!(err.to_string().contains("line 1"));
    }

    // ========================================================================
    // line_is_structural vs. line_kind agreement (#382): both skip a quoted
    // span via the shared quoted_span_end/after_separation, but ask different
    // questions of a fresh line — pin every named point of agreement and every
    // deliberate divergence, not a flat invariant (see
    // tests/yaml_tab_indentation_tests.rs for why a table, not an assertion).
    // ========================================================================

    /// `line_kind` classifies from `self.offset`; this positions a validator
    /// there directly, without driving `scan_line`'s whole state machine.
    fn line_kind_at(text: &[u8], offset: usize) -> LineKind {
        let mut v = Validator::new(text);
        v.offset = offset;
        v.line_kind()
    }

    struct QuoteScanAgreement {
        yaml: &'static [u8],
        structural: bool,
        kind: LineKind,
        why: &'static str,
    }

    const QUOTE_SCAN_AGREEMENT: &[QuoteScanAgreement] = &[
        // ---- Agree: ordinary one-line shapes. ------------------------------
        QuoteScanAgreement {
            yaml: b"a: 1\n",
            structural: true,
            kind: LineKind::Map,
            why: "plain one-line entry",
        },
        QuoteScanAgreement {
            yaml: b"- a\n",
            structural: true,
            kind: LineKind::Seq,
            why: "plain sequence entry",
        },
        QuoteScanAgreement {
            yaml: b"a\n",
            structural: false,
            kind: LineKind::Scalar,
            why: "plain scalar, no indicator",
        },
        QuoteScanAgreement {
            yaml: b"\"b\": 1\n",
            structural: true,
            kind: LineKind::Map,
            why: "quoted key",
        },
        QuoteScanAgreement {
            yaml: b"'b': 1\n",
            structural: true,
            kind: LineKind::Map,
            why: "single-quoted key",
        },
        QuoteScanAgreement {
            yaml: b"\"x: y\"\n",
            structural: false,
            kind: LineKind::Scalar,
            why: "`:` inside a quoted scalar is content, not a key",
        },
        QuoteScanAgreement {
            yaml: b"foo\"bar: baz\n",
            structural: true,
            kind: LineKind::Map,
            why: "a quote glued to content is not a delimiter (#382 gating fix)",
        },
        QuoteScanAgreement {
            yaml: b"foo'bar: baz\n",
            structural: true,
            kind: LineKind::Map,
            why: "single-quoted analogue of the row above",
        },
        // ---- Agree, now that #382 fixed the cross-line fold. ---------------
        QuoteScanAgreement {
            yaml: b"\"a\n b\": v\n",
            structural: false,
            kind: LineKind::Map,
            why: "multi-line quoted key: a node to line_is_structural (stops \
                  at the break) but a Map to line_kind, which now correctly \
                  follows the close (#382)",
        },
        QuoteScanAgreement {
            yaml: b"'a\n b': v\n",
            structural: false,
            kind: LineKind::Map,
            why: "single-quoted analogue",
        },
        QuoteScanAgreement {
            yaml: b"\"a\n b\"\n",
            structural: false,
            kind: LineKind::Scalar,
            why: "multi-line quoted scalar with no following `:`",
        },
        QuoteScanAgreement {
            yaml: b"\"ab",
            structural: false,
            kind: LineKind::Scalar,
            why: "unterminated quote, no close anywhere in the input: both \
                  scans give up and treat the line as a non-structural \
                  scalar rather than hunting past end of input",
        },
        // ---- Named, legitimate divergences (#382 leaves these alone). ------
        QuoteScanAgreement {
            yaml: b"{a: 1}\n",
            structural: false,
            kind: LineKind::Map,
            why: "flow mapping: not structural to line_is_structural (a tab \
                  before it is legal separation, Q5MG/6CA3), but line_kind \
                  has no flow-node early return and reports Map",
        },
        QuoteScanAgreement {
            yaml: b"? k\n",
            structural: false,
            kind: LineKind::Map,
            why: "leading `?`: line_kind recognizes it directly; \
                  line_is_structural only recognizes `-`, so a leading `?` \
                  falls through to its `:` scan and finds none here",
        },
    ];

    #[test]
    fn line_is_structural_and_line_kind_agree_except_where_named() {
        let mut wrong = Vec::new();
        for row in QUOTE_SCAN_AGREEMENT {
            let text = String::from_utf8_lossy(row.yaml);
            let structural = crate::yaml::line_is_structural(row.yaml, 0);
            let actual_kind = line_kind_at(row.yaml, 0);
            if structural != row.structural {
                wrong.push(format!(
                    "  {text:?}: line_is_structural = {structural}, expected {} — {}",
                    row.structural, row.why
                ));
            }
            if actual_kind != row.kind {
                wrong.push(format!(
                    "  {text:?}: line_kind = {actual_kind:?}, expected {:?} — {}",
                    row.kind, row.why
                ));
            }
        }
        assert!(wrong.is_empty(), "verdicts changed:\n{}", wrong.join("\n"));
    }
}
