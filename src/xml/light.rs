//! XmlIndex/XmlCursor/XmlValue — lazy XML navigation.
//!
//! Mirrors `src/json/light.rs`'s architecture (semi-index + `Copy` cursor +
//! lazily-materialized value), scaled down to this milestone's scope:
//! element/attribute/text navigation only, no combinators, no namespaces.
//!
//! # Value model
//!
//! An XML element projects to [`DocumentValue::as_object`] only (never
//! `as_array`). Its BP children — in document order, which for XML always
//! means attributes first (they're part of the opening tag), then child
//! elements and text runs — each become one object field:
//!
//! - An attribute `id="1"` becomes a field keyed `"+@id"`.
//! - A child element `<bar>` becomes a field keyed `"bar"` (the plain tag
//!   name). Repeated same-name children resolve to the first occurrence.
//! - A text/CDATA run becomes a field keyed `"+content"`.
//!
//! The `+@` attribute-key prefix is not invented here — it matches the
//! convention already recorded in `docs/plan/yq.md` for the future
//! `@xml`/`to_xml` encoder (issue #85), so this milestone is
//! forward-compatible with it.
//!
//! Unlike JSON/YAML, a field's key is not a separate sibling node: it is
//! derived from the field's own value node (the tag name is scanned from
//! the element's own text position; the attribute name is scanned from the
//! same anchor the attribute's value is read from; the content key is a
//! constant). This is XML-specific: the "key" is always structurally
//! recoverable from the value's own span, unlike JSON/YAML's independent
//! key/value pairs.
//!
//! # Example
//!
//! ```
//! use succinctly::xml::light::XmlIndex;
//! use succinctly::jq::document::{DocumentCursor, DocumentValue};
//!
//! let xml = br#"<root><name>Alice</name></root>"#;
//! let index = XmlIndex::build(xml).unwrap();
//! let root = index.root(xml);
//!
//! let name = root.value().as_object().unwrap().find("name").unwrap();
//! assert_eq!(&*name.as_object().unwrap().find("+content").unwrap().as_str().unwrap(), "Alice");
//! ```

#[cfg(not(test))]
use alloc::{borrow::Cow, format, string::String, vec::Vec};
#[cfg(test)]
use std::borrow::Cow;

use core::cell::OnceCell;

use crate::jq::document::{
    DocumentCursor, DocumentElements, DocumentField, DocumentFields, DocumentValue,
};
use crate::trees::BalancedParens;

pub use super::scan::XmlScanError;
use super::scan::{build_semi_index, is_name_end, XmlNodeKind};

// ============================================================================
// XmlIndex: Holds the IB, BP, and per-node kind data
// ============================================================================

/// Index structures for navigating XML.
///
/// The type parameter `W` controls how the underlying IB/BP data is stored
/// (`Vec<u64>` for owned data built from XML text). Use [`XmlIndex::build`]
/// to build one.
#[derive(Clone, Debug)]
pub struct XmlIndex<W = Vec<u64>> {
    /// Interest bits - one per input byte, set at element/attribute/text
    /// node-start positions.
    ib: W,
    ib_len: usize,
    /// Cumulative popcount per word (for O(1) rank on IB).
    ib_rank: Vec<u32>,
    /// Balanced parentheses - encodes element nesting as a tree; attributes
    /// and text/CDATA runs are leaves.
    bp: BalancedParens<W>,
    /// Per-node kind, indexed by node ordinal (same indexing as IB rank).
    kinds: Vec<XmlNodeKind>,
    /// Line starts for line/column lookup, built lazily on first use — same
    /// rationale as `JsonIndex`'s (#228): most queries never need it.
    lines: OnceCell<crate::text::LineIndex>,
}

fn build_ib_rank(words: &[u64]) -> Vec<u32> {
    let mut rank = Vec::with_capacity(words.len() + 1);
    let mut cumulative: u32 = 0;
    rank.push(0);
    for &word in words {
        cumulative += word.count_ones();
        rank.push(cumulative);
    }
    rank
}

impl XmlIndex<Vec<u64>> {
    /// Build an XML index from XML text.
    ///
    /// Non-validating (like JSON/YAML's semi-indexers): element nesting is
    /// tracked positionally, not by matching end-tag names against their
    /// start tags. Returns an error for input that isn't well-formed enough
    /// to index safely (unmatched tags, truncated input, no root element) —
    /// see [`XmlScanError`].
    pub fn build(xml: &[u8]) -> Result<Self, XmlScanError> {
        let semi = build_semi_index(xml)?;
        let ib_len = xml.len();
        let ib_rank = build_ib_rank(&semi.ib);

        Ok(Self {
            ib: semi.ib,
            ib_len,
            ib_rank,
            bp: BalancedParens::new(semi.bp, semi.bp_len),
            kinds: semi.kinds,
            lines: OnceCell::new(),
        })
    }
}

impl<W: AsRef<[u64]>> XmlIndex<W> {
    /// Get a reference to the balanced parentheses.
    #[inline]
    pub fn bp(&self) -> &BalancedParens<W> {
        &self.bp
    }

    /// Create a cursor at the root element of the document.
    #[inline]
    pub fn root<'a>(&'a self, text: &'a [u8]) -> XmlCursor<'a, W> {
        XmlCursor {
            text,
            index: self,
            bp_pos: 0,
            as_key: false,
        }
    }

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

    /// Convert byte offset to 1-indexed line and column.
    #[inline]
    pub fn to_line_column(&self, offset: usize, text: &[u8]) -> (usize, usize) {
        self.ensure_lines(text).to_line_column(offset)
    }

    /// Convert 1-indexed line and column to byte offset.
    #[inline]
    pub fn to_offset(&self, line: usize, column: usize, text: &[u8]) -> Option<usize> {
        self.ensure_lines(text).to_offset(line, column)
    }

    /// select1 on the IB: position of the k-th 1-bit (0-indexed), via
    /// binary search over the rank directory. No galloping-hint variant
    /// (unlike JSON's `ib_select1_from`) — that's a sequential-access
    /// micro-optimization this milestone doesn't need.
    pub(crate) fn ib_select1(&self, k: usize) -> Option<usize> {
        let words = self.ib.as_ref();
        if words.is_empty() {
            return None;
        }
        let k32 = k as u32;
        let n = words.len();

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
        let bit_pos = crate::util::broadword::select_in_word(word, remaining as u32) as usize;
        let result = lo * 64 + bit_pos;
        if result < self.ib_len {
            Some(result)
        } else {
            None
        }
    }

    /// rank1 on the IB: count of 1-bits in `[0, pos)`.
    pub(crate) fn ib_rank1(&self, pos: usize) -> usize {
        if pos == 0 {
            return 0;
        }
        let words = self.ib.as_ref();
        let word_idx = pos / 64;
        let bit_idx = pos % 64;
        let mut count = self.ib_rank[word_idx.min(words.len())] as usize;
        if word_idx < words.len() && bit_idx > 0 {
            let mask = (1u64 << bit_idx) - 1;
            count += (words[word_idx] & mask).count_ones() as usize;
        }
        count
    }

    #[inline]
    fn kind_at_rank(&self, rank: usize) -> XmlNodeKind {
        self.kinds[rank]
    }
}

// ============================================================================
// XmlCursor: Position in the XML structure
// ============================================================================

/// A cursor pointing to a position in the XML structure.
///
/// Lightweight (a BP position integer plus borrowed references) and cheap
/// to copy, like `JsonCursor`/`YamlCursor`.
///
/// `as_key` distinguishes a cursor used as a `DocumentField::key_cursor`
/// from an ordinary navigational cursor at the exact same BP position. JSON
/// and YAML's `key_cursor` points at a *separate* document node whose
/// `.value()` naturally is the key string; XML has no such node — the key
/// is synthesized from the value node's own kind (module docs above). Two
/// distinct `XmlCursor`s at the same `bp_pos`, differing only in `as_key`,
/// let [`value`](Self::value) return the synthesized key
/// ([`XmlValue::Key`]) instead of the field's real value when navigation
/// code (`eval_generic.rs`'s `LazyKeysUnsorted` fast path) forwards a
/// `key_cursor` and later calls `.value()` on it, exactly as it does for
/// JSON/YAML.
#[derive(Debug)]
pub struct XmlCursor<'a, W = Vec<u64>> {
    text: &'a [u8],
    index: &'a XmlIndex<W>,
    bp_pos: usize,
    as_key: bool,
}

impl<W> Clone for XmlCursor<'_, W> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<W> Copy for XmlCursor<'_, W> {}

impl<'a, W: AsRef<[u64]>> XmlCursor<'a, W> {
    /// Create a cursor at a specific BP position.
    ///
    /// Useful when the position is already known, e.g. from
    /// [`crate::xml::locate`]'s offset-to-path search.
    #[inline]
    pub fn from_bp_position(index: &'a XmlIndex<W>, text: &'a [u8], bp_pos: usize) -> Self {
        Self {
            text,
            index,
            bp_pos,
            as_key: false,
        }
    }

    /// A cursor at this same position, whose [`value`](Self::value) returns
    /// the synthesized key ([`XmlValue::Key`]) instead of the field's real
    /// value. See the struct docs' `as_key` note.
    #[inline]
    fn as_key_view(&self) -> Self {
        Self {
            as_key: true,
            ..*self
        }
    }

    /// Get the position in the BP vector.
    #[inline]
    pub fn bp_position(&self) -> usize {
        self.bp_pos
    }

    /// Check if this cursor has children in the BP tree. An empty element
    /// (`<foo/>` with no attributes/children/text) is not a container by
    /// this definition, even though it's still an `Element`-kind, empty
    /// object value — this mirrors `JsonCursor::is_container`'s "has
    /// children" contract exactly (an empty `{}` isn't a container there
    /// either).
    #[inline]
    pub fn is_container(&self) -> bool {
        self.index.bp().first_child(self.bp_pos).is_some()
    }

    #[inline]
    fn rank(&self) -> usize {
        self.index.bp().rank1(self.bp_pos)
    }

    /// Get the byte position in the XML text.
    pub fn text_position(&self) -> Option<usize> {
        let rank = self.rank();
        self.index.ib_select1(rank)
    }

    #[inline]
    fn kind(&self) -> XmlNodeKind {
        self.index.kind_at_rank(self.rank())
    }

    /// Get the 1-based line number of this node's position.
    #[inline]
    pub fn line(&self) -> usize {
        let offset = self.text_position().unwrap_or(0);
        self.index.to_line_column(offset, self.text).0
    }

    /// Get the 1-based column number of this node's position.
    #[inline]
    pub fn column(&self) -> usize {
        let offset = self.text_position().unwrap_or(0);
        self.index.to_line_column(offset, self.text).1
    }

    /// Navigate to the first child.
    #[inline]
    pub fn first_child(&self) -> Option<Self> {
        let new_pos = self.index.bp().first_child(self.bp_pos)?;
        Some(Self {
            text: self.text,
            index: self.index,
            bp_pos: new_pos,
            as_key: false,
        })
    }

    /// Navigate to the next sibling.
    #[inline]
    pub fn next_sibling(&self) -> Option<Self> {
        let new_pos = self.index.bp().next_sibling(self.bp_pos)?;
        Some(Self {
            text: self.text,
            index: self.index,
            bp_pos: new_pos,
            as_key: false,
        })
    }

    /// Navigate to the parent.
    #[inline]
    pub fn parent(&self) -> Option<Self> {
        let new_pos = self.index.bp().parent(self.bp_pos)?;
        Some(Self {
            text: self.text,
            index: self.index,
            bp_pos: new_pos,
            as_key: false,
        })
    }

    /// Get the XML value at this cursor position — the synthesized key
    /// ([`XmlValue::Key`]) if this cursor came from
    /// [`as_key_view`](Self::as_key_view), otherwise the field's real value.
    pub fn value(&self) -> XmlValue<'a, W> {
        let kind = self.kind();
        if self.as_key {
            // Same synthesis as `XmlValue::key_string()`'s match arms
            // (inlined rather than shared: that trait method needs `W:
            // Clone`, which this inherent method doesn't otherwise require).
            let name = match kind {
                XmlNodeKind::Element => self.tag_name().map(Cow::Borrowed),
                XmlNodeKind::Attribute => self.attr_name().map(|n| Cow::Owned(format!("+@{n}"))),
                XmlNodeKind::Text { .. } => Some(Cow::Borrowed("+content")),
            };
            match name {
                Some(name) => XmlValue::Key(name),
                None => XmlValue::Error("missing key"),
            }
        } else {
            match kind {
                XmlNodeKind::Element => XmlValue::Element(*self),
                XmlNodeKind::Attribute => XmlValue::Attribute(*self),
                XmlNodeKind::Text { .. } => XmlValue::Text(*self),
            }
        }
    }

    /// Scan a name span (element tag name or attribute name) starting at
    /// `start`, stopping at the first `is_name_end` byte. Shared logic
    /// between `tag_name`/`attr_name` since both use the same terminator
    /// set — see `scan::is_name_end`'s doc comment.
    fn name_at(&self, start: usize) -> Option<&'a str> {
        let mut j = start;
        while j < self.text.len() && !is_name_end(self.text[j]) {
            j += 1;
        }
        core::str::from_utf8(&self.text[start..j]).ok()
    }

    /// The tag name of an `Element`-kind cursor.
    fn tag_name(&self) -> Option<&'a str> {
        debug_assert!(matches!(self.kind(), XmlNodeKind::Element));
        self.name_at(self.text_position()?)
    }

    /// The attribute name of an `Attribute`-kind cursor.
    fn attr_name(&self) -> Option<&'a str> {
        debug_assert!(matches!(self.kind(), XmlNodeKind::Attribute));
        self.name_at(self.text_position()?)
    }

    /// The raw (still-escaped), byte span of an `Attribute`-kind cursor's
    /// value, located by scanning forward from the name past `=` and the
    /// opening quote to the matching closing quote — unambiguous per XML
    /// grammar (attribute values cannot contain a literal quote of their
    /// own delimiter, nor a literal `<`).
    fn attr_value_span(&self) -> Option<(usize, usize)> {
        let name_start = self.text_position()?;
        let mut j = name_start;
        while j < self.text.len() && !is_name_end(self.text[j]) {
            j += 1;
        }
        while j < self.text.len() && self.text[j].is_ascii_whitespace() {
            j += 1;
        }
        if self.text.get(j) != Some(&b'=') {
            return None;
        }
        j += 1;
        while j < self.text.len() && self.text[j].is_ascii_whitespace() {
            j += 1;
        }
        let quote = *self.text.get(j)?;
        j += 1;
        let value_start = j;
        while j < self.text.len() && self.text[j] != quote {
            j += 1;
        }
        Some((value_start, j))
    }

    /// The raw byte span of a `Text`-kind cursor, plus whether it came from
    /// a CDATA section (`raw`, in which case entity decoding must be
    /// skipped — see `XmlNodeKind::Text`'s doc comment).
    fn text_span(&self) -> Option<(usize, usize, bool)> {
        let start = self.text_position()?;
        match self.kind() {
            XmlNodeKind::Text { len, raw } => Some((start, start + len as usize, raw)),
            _ => None,
        }
    }

    /// The byte range of this node's *value* — for `Attribute`, the quoted
    /// value span (not the name, which is what `text_position()` points
    /// at); for `Text`, its content span. `None` for `Element` (and on
    /// malformed spans), matching `crate::xml::locate::locate_offset_detailed`'s
    /// documented fallback.
    ///
    /// Public because `text_position() + as_str().len()` is NOT the
    /// attribute value's range — `text_position()` is the *name* start for
    /// attributes, so that formula would slice into the name, not the
    /// value. This exists so callers outside this module (namely
    /// `xml::locate`) can't make that mistake.
    pub fn value_byte_range(&self) -> Option<(usize, usize)> {
        match self.kind() {
            XmlNodeKind::Attribute => self.attr_value_span(),
            XmlNodeKind::Text { .. } => self.text_span().map(|(start, end, _)| (start, end)),
            XmlNodeKind::Element => None,
        }
    }

    /// Decoded string value of an `Attribute`-kind cursor.
    fn attr_value_str(&self) -> Option<Cow<'a, str>> {
        let (start, end) = self.attr_value_span()?;
        decode_entities(&self.text[start..end]).ok()
    }

    /// Decoded string value of a `Text`-kind cursor.
    fn text_str(&self) -> Option<Cow<'a, str>> {
        let (start, end, raw) = self.text_span()?;
        let bytes = &self.text[start..end];
        if raw {
            core::str::from_utf8(bytes).ok().map(Cow::Borrowed)
        } else {
            decode_entities(bytes).ok()
        }
    }

    /// Create a cursor at the specified byte offset (0-indexed).
    ///
    /// Ports `JsonCursor::cursor_at_offset`'s algorithm verbatim: resolve
    /// the IB rank at `offset`, then binary-search the BP structure for the
    /// position whose rank matches. Not a tree walk — O(log n) regardless
    /// of nesting depth.
    pub fn cursor_at_offset(&self, offset: usize) -> Option<Self> {
        if offset >= self.text.len() {
            return None;
        }

        let rank = self.index.ib_rank1(offset);
        let ib_idx = if let Some(struct_pos) = self.index.ib_select1(rank) {
            if struct_pos == offset {
                rank
            } else if rank > 0 {
                rank - 1
            } else {
                return None;
            }
        } else if rank > 0 {
            rank - 1
        } else {
            return None;
        };

        let bp = self.index.bp();
        let bp_len = bp.len();
        if bp_len == 0 {
            return None;
        }

        let mut lo = 0;
        let mut hi = bp_len;
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            let count = bp.rank1(mid + 1);
            if count <= ib_idx {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }

        if lo < bp_len && bp.rank1(lo + 1) == ib_idx + 1 {
            Some(Self {
                text: self.text,
                index: self.index,
                bp_pos: lo,
                as_key: false,
            })
        } else {
            None
        }
    }

    /// Create a cursor at the specified line and column (1-indexed).
    pub fn cursor_at_position(&self, line: usize, col: usize) -> Option<Self> {
        let offset = self.index.to_offset(line, col, self.text)?;
        self.cursor_at_offset(offset)
    }
}

// ============================================================================
// Entity decoding
// ============================================================================

/// An error decoding a text/attribute value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum XmlValueError {
    /// The raw bytes were not valid UTF-8.
    InvalidUtf8,
    /// An `&...;` sequence wasn't one of the 5 predefined entities or a
    /// numeric character reference, or was missing its terminating `;`.
    InvalidEntity,
}

/// Decode the 5 predefined XML entities (`&amp; &lt; &gt; &quot; &apos;`)
/// and numeric character references (`&#NN;`, `&#xHH;`). Zero-copy
/// (`Cow::Borrowed`) when no `&` is present, matching JSON's
/// zero-copy-unless-escaped `decode_escapes` pattern.
fn decode_entities(bytes: &[u8]) -> Result<Cow<'_, str>, XmlValueError> {
    if !bytes.contains(&b'&') {
        return core::str::from_utf8(bytes)
            .map(Cow::Borrowed)
            .map_err(|_| XmlValueError::InvalidUtf8);
    }

    let mut result = String::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'&' {
            let semi = bytes[i..]
                .iter()
                .position(|&b| b == b';')
                .map(|p| i + p)
                .ok_or(XmlValueError::InvalidEntity)?;
            let entity = &bytes[i + 1..semi];
            let ch = match entity {
                b"amp" => '&',
                b"lt" => '<',
                b"gt" => '>',
                b"quot" => '"',
                b"apos" => '\'',
                _ if entity.first() == Some(&b'#') => {
                    let (digits, radix) =
                        if entity.get(1) == Some(&b'x') || entity.get(1) == Some(&b'X') {
                            (&entity[2..], 16)
                        } else {
                            (&entity[1..], 10)
                        };
                    let digits =
                        core::str::from_utf8(digits).map_err(|_| XmlValueError::InvalidEntity)?;
                    let cp = u32::from_str_radix(digits, radix)
                        .map_err(|_| XmlValueError::InvalidEntity)?;
                    char::from_u32(cp).ok_or(XmlValueError::InvalidEntity)?
                }
                _ => return Err(XmlValueError::InvalidEntity),
            };
            result.push(ch);
            i = semi + 1;
        } else {
            let start = i;
            while i < bytes.len() && bytes[i] != b'&' {
                i += 1;
            }
            let chunk =
                core::str::from_utf8(&bytes[start..i]).map_err(|_| XmlValueError::InvalidUtf8)?;
            result.push_str(chunk);
        }
    }
    Ok(Cow::Owned(result))
}

// ============================================================================
// XmlValue: The value type
// ============================================================================

/// An XML value with lazy decoding: an element (projects to an object of
/// its attributes/children/content), an attribute value, or a text/CDATA
/// run. See the module docs for the field-key convention.
#[derive(Clone, Debug)]
pub enum XmlValue<'a, W = Vec<u64>> {
    /// An element. Always `as_object()`-only — see module docs.
    Element(XmlCursor<'a, W>),
    /// An attribute's value.
    Attribute(XmlCursor<'a, W>),
    /// A text or CDATA run's value.
    Text(XmlCursor<'a, W>),
    /// A field's synthesized key, materialized from an
    /// [`XmlCursor::as_key_view`] cursor's [`value`](XmlCursor::value) —
    /// see `XmlCursor`'s `as_key` doc note. Always a string.
    Key(Cow<'a, str>),
    /// An error encountered during navigation.
    Error(&'static str),
}

impl<'a, W: AsRef<[u64]> + Clone> DocumentValue for XmlValue<'a, W> {
    type Cursor = XmlCursor<'a, W>;
    type Fields = XmlFields<'a, W>;
    type Elements = XmlElements<'a, W>;

    #[inline]
    fn is_null(&self) -> bool {
        false
    }

    // XML has no type system: attribute/text content is always a string
    // unless a query explicitly coerces it (`tonumber`). `as_bool`/`as_i64`/
    // `as_f64` must NOT auto-parse from `as_str()` here — `to_owned()`
    // (`src/jq/eval_generic.rs`) checks them *before* `as_str()`, so an
    // attribute like `id="2"` would silently materialize as the JSON number
    // `2` instead of the string `"2"` on every full-value dump (identity,
    // `to_entries`, anything past the natively-handled builtin subset).
    // `tonumber` itself parses `as_str()` independently, so it still works.
    #[inline]
    fn as_bool(&self) -> Option<bool> {
        None
    }

    #[inline]
    fn as_i64(&self) -> Option<i64> {
        None
    }

    #[inline]
    fn as_f64(&self) -> Option<f64> {
        None
    }

    fn as_str(&self) -> Option<Cow<'_, str>> {
        match self {
            XmlValue::Attribute(cursor) => cursor.attr_value_str(),
            XmlValue::Text(cursor) => cursor.text_str(),
            XmlValue::Key(s) => Some(s.clone()),
            _ => None,
        }
    }

    fn key_string(&self) -> Option<Cow<'_, str>> {
        match self {
            XmlValue::Element(cursor) => cursor.tag_name().map(Cow::Borrowed),
            XmlValue::Attribute(cursor) => cursor.attr_name().map(|n| Cow::Owned(format!("+@{n}"))),
            XmlValue::Text(_) => Some(Cow::Borrowed("+content")),
            XmlValue::Key(s) => Some(s.clone()),
            XmlValue::Error(_) => None,
        }
    }

    fn as_object(&self) -> Option<Self::Fields> {
        match self {
            XmlValue::Element(cursor) => Some(XmlFields::from_element_cursor(*cursor)),
            _ => None,
        }
    }

    #[inline]
    fn as_array(&self) -> Option<Self::Elements> {
        None
    }

    fn type_name(&self) -> &'static str {
        match self {
            XmlValue::Element(_) => "object",
            XmlValue::Attribute(_) | XmlValue::Text(_) | XmlValue::Key(_) => "string",
            XmlValue::Error(_) => "error",
        }
    }

    #[inline]
    fn is_error(&self) -> bool {
        matches!(self, XmlValue::Error(_))
    }

    fn error_message(&self) -> Option<&'static str> {
        match self {
            XmlValue::Error(msg) => Some(msg),
            _ => None,
        }
    }
}

impl<'a, W: AsRef<[u64]> + Clone> DocumentCursor for XmlCursor<'a, W> {
    type Value = XmlValue<'a, W>;

    #[inline]
    fn value(&self) -> Self::Value {
        XmlCursor::value(self)
    }

    #[inline]
    fn first_child(&self) -> Option<Self> {
        XmlCursor::first_child(self)
    }

    #[inline]
    fn next_sibling(&self) -> Option<Self> {
        XmlCursor::next_sibling(self)
    }

    #[inline]
    fn parent(&self) -> Option<Self> {
        XmlCursor::parent(self)
    }

    #[inline]
    fn is_container(&self) -> bool {
        XmlCursor::is_container(self)
    }

    #[inline]
    fn text_position(&self) -> Option<usize> {
        XmlCursor::text_position(self)
    }

    #[inline]
    fn line(&self) -> usize {
        XmlCursor::line(self)
    }

    #[inline]
    fn column(&self) -> usize {
        XmlCursor::column(self)
    }

    #[inline]
    fn cursor_at_offset(&self, offset: usize) -> Option<Self> {
        XmlCursor::cursor_at_offset(self, offset)
    }

    #[inline]
    fn cursor_at_position(&self, line: usize, col: usize) -> Option<Self> {
        XmlCursor::cursor_at_position(self, line, col)
    }
}

// ============================================================================
// XmlFields: Immutable iteration over an element's attributes/children/content
// ============================================================================

/// Immutable "list" of an element's fields (attributes, child elements, and
/// its content), in document order.
///
/// Unlike JSON/YAML's key+value sibling pairs, each field here is a single
/// BP child node: the "key" (`DocumentField::key`) and "value" are both
/// derived from that same node's [`XmlValue`] (see module docs).
#[derive(Debug)]
pub struct XmlFields<'a, W = Vec<u64>> {
    current: Option<XmlCursor<'a, W>>,
}

impl<W> Clone for XmlFields<'_, W> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<W> Copy for XmlFields<'_, W> {}

impl<'a, W: AsRef<[u64]> + Clone> XmlFields<'a, W> {
    fn from_element_cursor(element_cursor: XmlCursor<'a, W>) -> Self {
        Self {
            current: element_cursor.first_child(),
        }
    }

    /// Check if there are no fields.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.current.is_none()
    }

    /// Get the first field and the remaining fields.
    #[allow(clippy::type_complexity)] // STYLE-0004: mirrors DocumentFields::uncons's cons-list contract (field, rest)
    pub fn uncons(&self) -> Option<(DocumentField<XmlValue<'a, W>, XmlCursor<'a, W>>, Self)> {
        let cursor = self.current?;
        let value = cursor.value();
        let key = value.clone();
        let rest = Self {
            current: cursor.next_sibling(),
        };
        Some((
            DocumentField {
                key,
                value,
                key_cursor: cursor.as_key_view(),
                value_cursor: cursor,
            },
            rest,
        ))
    }

    /// Find a field by key. First occurrence wins for repeated keys
    /// (matches JSON's policy) — see module docs on repeated same-name
    /// children.
    pub fn find(&self, name: &str) -> Option<XmlValue<'a, W>> {
        let mut fields = *self;
        while let Some((field, rest)) = fields.uncons() {
            if field.key_str().as_deref() == Some(name) {
                return Some(field.value);
            }
            fields = rest;
        }
        None
    }

    /// Find a field by key and return a cursor to its value. Same
    /// first-match semantics as [`find`](Self::find).
    pub fn find_cursor(&self, name: &str) -> Option<XmlCursor<'a, W>> {
        let mut fields = *self;
        while let Some((field, rest)) = fields.uncons() {
            if field.key_str().as_deref() == Some(name) {
                return Some(field.value_cursor);
            }
            fields = rest;
        }
        None
    }
}

impl<'a, W: AsRef<[u64]> + Clone> DocumentFields for XmlFields<'a, W> {
    type Value = XmlValue<'a, W>;
    type Cursor = XmlCursor<'a, W>;

    #[inline]
    fn uncons(&self) -> Option<(DocumentField<Self::Value, Self::Cursor>, Self)> {
        XmlFields::uncons(self)
    }

    #[inline]
    fn find(&self, name: &str) -> Option<Self::Value> {
        XmlFields::find(self, name)
    }

    #[inline]
    fn find_cursor(&self, name: &str) -> Option<Self::Cursor> {
        XmlFields::find_cursor(self, name)
    }

    #[inline]
    fn is_empty(&self) -> bool {
        XmlFields::is_empty(self)
    }
}

// ============================================================================
// XmlElements: Unpopulated `DocumentElements` impl (as_array() is always None)
// ============================================================================

/// A `DocumentElements` implementation that is never constructed with data.
///
/// [`XmlValue::as_array`] always returns `None` (elements project to
/// `as_object()` only — see module docs), but `DocumentValue::Elements`
/// still requires *some* concrete `DocumentElements` type to name; this is
/// that type, existing purely to satisfy the associated-type requirement.
#[derive(Debug)]
pub struct XmlElements<'a, W = Vec<u64>> {
    // Mirrors `XmlCursor<'a, W>`'s own field shape (rather than an
    // unrelated marker) so the implied `W: 'a` bound this type needs
    // propagates automatically wherever `XmlElements<'a, W>` is named —
    // see the E0309 this fixed.
    _marker: core::marker::PhantomData<Option<XmlCursor<'a, W>>>,
}

impl<W> Clone for XmlElements<'_, W> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<W> Copy for XmlElements<'_, W> {}

impl<'a, W: AsRef<[u64]> + Clone> DocumentElements for XmlElements<'a, W> {
    type Value = XmlValue<'a, W>;
    type Cursor = XmlCursor<'a, W>;

    fn uncons(&self) -> Option<(Self::Value, Self)> {
        None
    }

    fn uncons_cursor(&self) -> Option<(Self::Cursor, Self)> {
        None
    }

    fn get(&self, _index: usize) -> Option<Self::Value> {
        None
    }

    fn is_empty(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn navigate_nested_elements() {
        let xml = b"<root><foo><bar>x</bar></foo></root>";
        let index = XmlIndex::build(xml).unwrap();
        let root = index.root(xml);
        assert!(root.is_container());
        let foo = root.first_child().unwrap();
        assert_eq!(foo.tag_name(), Some("foo"));
        let bar = foo.first_child().unwrap();
        assert_eq!(bar.tag_name(), Some("bar"));
        let text = bar.first_child().unwrap();
        assert_eq!(text.text_str().as_deref(), Some("x"));
        assert!(text.next_sibling().is_none());
        assert_eq!(bar.parent().unwrap().bp_position(), foo.bp_position());
    }

    #[test]
    fn dot_path_navigation_via_document_value() {
        let xml = b"<root><foo><bar>x</bar></foo></root>";
        let index = XmlIndex::build(xml).unwrap();
        let root = index.root(xml).value();
        let foo = root.as_object().unwrap().find("foo").unwrap();
        let bar = foo.as_object().unwrap().find("bar").unwrap();
        let content = bar.as_object().unwrap().find("+content").unwrap();
        assert_eq!(content.as_str().as_deref(), Some("x"));
    }

    #[test]
    fn attribute_access() {
        let xml = br#"<root id="1" name="x"><child/></root>"#;
        let index = XmlIndex::build(xml).unwrap();
        let root = index.root(xml).value();
        let fields = root.as_object().unwrap();
        assert_eq!(fields.find("+@id").unwrap().as_str().as_deref(), Some("1"));
        assert_eq!(
            fields.find("+@name").unwrap().as_str().as_deref(),
            Some("x")
        );
        assert!(fields
            .find("child")
            .unwrap()
            .as_object()
            .unwrap()
            .is_empty());
    }

    #[test]
    fn repeated_children_first_wins() {
        let xml = br"<root><item>a</item><item>b</item></root>";
        let index = XmlIndex::build(xml).unwrap();
        let root = index.root(xml).value();
        let item = root.as_object().unwrap().find("item").unwrap();
        let content = item.as_object().unwrap().find("+content").unwrap();
        assert_eq!(content.as_str().as_deref(), Some("a"));
    }

    #[test]
    fn entity_and_cdata_decoding() {
        let xml = b"<root>a &amp; b<![CDATA[<raw> &amp; not-decoded]]></root>";
        let index = XmlIndex::build(xml).unwrap();
        let root = index.root(xml);
        let plain = root.first_child().unwrap();
        assert_eq!(plain.text_str().as_deref(), Some("a & b"));
        let cdata = plain.next_sibling().unwrap();
        assert_eq!(cdata.text_str().as_deref(), Some("<raw> &amp; not-decoded"));
    }

    #[test]
    fn numeric_character_references() {
        let xml = b"<root>&#65;&#x42;</root>";
        let index = XmlIndex::build(xml).unwrap();
        let root = index.root(xml);
        let text = root.first_child().unwrap();
        assert_eq!(text.text_str().as_deref(), Some("AB"));
    }

    #[test]
    fn as_object_never_also_as_array() {
        let xml = b"<root><a/></root>";
        let index = XmlIndex::build(xml).unwrap();
        let value = index.root(xml).value();
        assert!(value.as_object().is_some());
        assert!(value.as_array().is_none());
    }

    #[test]
    fn type_names_match_jq_conventions() {
        let xml = br#"<root attr="v">text</root>"#;
        let index = XmlIndex::build(xml).unwrap();
        let root = index.root(xml).value();
        assert_eq!(root.type_name(), "object");
        let fields = root.as_object().unwrap();
        assert_eq!(fields.find("+@attr").unwrap().type_name(), "string");
        assert_eq!(fields.find("+content").unwrap().type_name(), "string");
    }

    #[test]
    fn cursor_at_offset_lands_on_expected_node() {
        let xml = br#"<root id="1"><child>hi</child></root>"#;
        let index = XmlIndex::build(xml).unwrap();
        let root = index.root(xml);

        // Offset of 'c' in "child" (the child element's tag name start).
        let child_name_offset = xml.iter().position(|&b| b == b'c').unwrap();
        let cursor = root.cursor_at_offset(child_name_offset).unwrap();
        assert_eq!(cursor.tag_name(), Some("child"));

        // Offset of the text content "hi", right after "<child>" — NOT a
        // naive `windows(2).position(..)` search for b"hi", which would
        // wrongly match the "hi" inside "cHIld" first.
        let text_offset = child_name_offset + "child>".len();
        assert_eq!(&xml[text_offset..text_offset + 2], b"hi");
        let cursor = root.cursor_at_offset(text_offset).unwrap();
        assert_eq!(cursor.text_str().as_deref(), Some("hi"));
    }

    #[test]
    fn cursor_at_position_matches_cursor_at_offset() {
        let xml = b"<root>\n  <child>hi</child>\n</root>";
        let index = XmlIndex::build(xml).unwrap();
        let root = index.root(xml);
        let offset = xml.windows(2).position(|w| w == b"hi").unwrap();
        let (line, col) = index.to_line_column(offset, xml);
        let by_offset = root.cursor_at_offset(offset).unwrap();
        let by_position = root.cursor_at_position(line, col).unwrap();
        assert_eq!(by_offset.bp_position(), by_position.bp_position());
    }
}
