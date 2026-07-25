//! Strict, opt-in YAML validator.
//!
//! `succinctly` is a non-validating YAML loader by design: [`YamlIndex::build`]
//! records structure, not grammar conformance, and accepts many malformed
//! documents (see `docs/compliance/yaml/limitations.md`). This module is the
//! opt-in counterpart, mirroring [`crate::json::validate`]: a separate pass, run
//! *before* indexing, that rejects invalid YAML. The default indexing path does
//! not link it and is structurally incapable of regressing because of it.
//!
//! Like the JSON validator this is a plain scalar scanner — `no_std`, with no
//! allocation on the success path. It is **not** a full YAML grammar checker:
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

use core::fmt;

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
    /// Invalid UTF-8 sequence.
    InvalidUtf8,
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
            Self::InvalidUtf8 => write!(f, "invalid UTF-8 sequence"),
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
    /// a sequence entry, a mapping entry, or a plain/quoted scalar. Quoted
    /// regions are skipped so a `:` inside a quoted key is not miscounted.
    fn line_kind(&self) -> LineKind {
        let mut i = self.offset;
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
                b'"' => {
                    i += 1;
                    while let Some(&c) = self.input.get(i) {
                        i += 1;
                        if c == b'\\' {
                            i += 1;
                        } else if c == b'"' || c == b'\n' || c == b'\r' {
                            break;
                        }
                    }
                }
                b'\'' => {
                    i += 1;
                    while let Some(&c) = self.input.get(i) {
                        if c == b'\'' {
                            i += 1;
                            if self.input.get(i) == Some(&b'\'') {
                                i += 1;
                                continue;
                            }
                            break;
                        }
                        if c == b'\n' || c == b'\r' {
                            break;
                        }
                        i += 1;
                    }
                }
                b'#' if i > self.offset && matches!(self.input.get(i - 1), Some(b' ' | b'\t')) => {
                    break
                }
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
    fn line_is_structural(&self) -> bool {
        let mut i = self.offset;
        while matches!(self.input.get(i), Some(b' ' | b'\t')) {
            i += 1;
        }
        // Sequence item: `-` followed by whitespace or end of line.
        if self.input.get(i) == Some(&b'-')
            && matches!(
                self.input.get(i + 1),
                None | Some(b' ' | b'\t' | b'\n' | b'\r')
            )
        {
            return true;
        }
        // Mapping entry: a `:` value indicator somewhere on the line.
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
            self.scan_content_line()
        }
    }

    /// Scan the significant content of a line (after leading indent), consuming
    /// scalars, flow collections, and comments to the line break.
    fn scan_content_line(&mut self) -> Result<(), YamlValidationError> {
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
                Some(b'&') if self.at_quote_start() => {
                    suppress_nested = true;
                    self.scan_anchor()?;
                }
                Some(b'*') if self.at_quote_start() => {
                    suppress_nested = true;
                    self.scan_anchor_name();
                }
                Some(b'?') => {
                    suppress_nested = true;
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
                }
                Some(b'\'') if self.at_quote_start() => {
                    let min = if seen_value_indicator {
                        self.line_indent + 1
                    } else {
                        0
                    };
                    let multiline = self.scan_single_quoted(min)?;
                    self.check_after_block_quoted(multiline)?;
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
                    let mut k = self.offset;
                    while matches!(self.input.get(k), Some(b' ' | b'\t')) {
                        k += 1;
                    }
                    if !suppress_nested
                        && self.input.get(k) == Some(&b'-')
                        && matches!(self.input.get(k + 1), Some(b' ' | b'\t' | b'\n' | b'\r'))
                    {
                        return Err(self.error(YamlValidationErrorKind::TrailingContent));
                    }
                }
                Some(b'[' | b'{') if self.at_quote_start() => {
                    self.scan_flow()?;
                    self.check_after_top_level_flow()?;
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
                Some(_) => {
                    self.advance();
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
        self.scan_anchor_name(); // consumes `&`/`*` and the name
        self.skip_spaces_and_tabs();
        match self.peek() {
            Some(b'*') => Err(self.error(YamlValidationErrorKind::AnchorOnAlias)),
            Some(b'-') if matches!(self.peek_at(1), None | Some(b' ' | b'\t' | b'\n' | b'\r')) => {
                Err(self.error(YamlValidationErrorKind::MisplacedAnchor))
            }
            _ => Ok(()),
        }
    }

    /// Consume an anchor or alias token: the `&`/`*` sigil and its name (any
    /// non-whitespace, non-flow-indicator bytes; `:` is a legal name character).
    fn scan_anchor_name(&mut self) {
        self.advance(); // `&` or `*`
        while !matches!(
            self.peek(),
            None | Some(b' ' | b'\t' | b'\n' | b'\r' | b',' | b'[' | b']' | b'{' | b'}')
        ) {
            self.advance();
        }
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
            b'x' => self.scan_hex_escape(2),
            b'u' => self.scan_hex_escape(4),
            b'U' => self.scan_hex_escape(8),
            other => Err(self.error(YamlValidationErrorKind::InvalidEscape {
                sequence: other as char,
            })),
        }
    }

    /// Validate `n` hex digits following `\x`/`\u`/`\U`. Positioned on `x`/`u`/`U`.
    fn scan_hex_escape(&mut self, n: usize) -> Result<(), YamlValidationError> {
        self.advance(); // consume x/u/U
        for _ in 0..n {
            match self.peek() {
                Some(c) if c.is_ascii_hexdigit() => {
                    self.advance();
                }
                _ => {
                    return Err(self.error(YamlValidationErrorKind::InvalidEscape { sequence: 'x' }))
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
        match self.peek() {
            Some(b'\r') => {
                self.offset += 1;
                if self.peek() == Some(b'\n') {
                    self.offset += 1;
                }
                self.line += 1;
                self.column = 1;
            }
            Some(b'\n') => {
                self.offset += 1;
                self.line += 1;
                self.column = 1;
            }
            _ => {}
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
    fn accepts_valid_escapes() {
        assert!(validate(b"a: \"tab\\tnl\\n hex \\x41 \\u0041 \\U00000041\"\n").is_ok());
        assert!(validate(b"a: \"q \\\" s \\/ b \\\\ nbsp \\_ next \\N\"\n").is_ok());
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
        assert!(matches!(
            kind(b"word1  # comment\nword2\n"),
            TrailingContent
        )); // BS4K
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
            InvalidUtf8,
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
}
