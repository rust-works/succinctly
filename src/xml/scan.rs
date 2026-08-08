//! Scalar XML semi-index scanner.
//!
//! Single-pass, non-validating scanner producing the interest-bits (IB),
//! balanced-parentheses (BP), and per-node kind data that
//! [`XmlIndex`](super::light::XmlIndex) needs. Scalar only, matching
//! `docs/STYLE_GUIDE.md`'s guidance and this repo's P5-P8 rejected-SIMD
//! history: a fast path is only worth adding once a real workload calls
//! for it.
//!
//! Every input byte gets exactly one IB bit (dense, like JSON's IB); BP
//! bits are only written at node boundaries (sparse, event-driven): an
//! element start writes `1`, its matching end writes `0`, and an attribute
//! or text/CDATA node writes a leaf `10` — exactly JSON's leaf-value
//! convention (`src/json/standard.rs`).
//!
//! Not validated: element nesting (end tags are matched positionally, not
//! by name), full XML grammar for names, DTD internal subsets beyond
//! bracket-depth tracking. This mirrors the project's semi-indexing
//! philosophy ("Semi-indexing performs minimal validation", `CLAUDE.md`).

#[cfg(not(test))]
use alloc::vec::Vec;

use crate::json::BitWriter;

/// The kind of an indexed XML node.
///
/// Used to interpret the raw bytes at a node's `text_position` and to
/// synthesize its `DocumentField` key (`src/xml/light.rs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum XmlNodeKind {
    /// An element (container). `text_position` points at the first byte of
    /// the tag name, immediately after `<`.
    Element,
    /// An attribute (leaf). `text_position` points at the first byte of the
    /// attribute name; the value is located by scanning forward past `=`
    /// and the opening quote (both unambiguous per XML grammar).
    Attribute,
    /// A text run or CDATA section (leaf). `text_position` points at the
    /// first content byte; `len` is the exact byte length of the content —
    /// stored explicitly, rather than re-derived by scanning for a
    /// terminator, because a CDATA run may itself contain a literal `<`.
    ///
    /// `raw` distinguishes the two sources: `false` for a plain text run
    /// (entity references like `&amp;` are decoded lazily on `.as_str()`),
    /// `true` for a CDATA section (content is literal per the XML spec —
    /// `&amp;` inside `<![CDATA[...]]>` must NOT be decoded).
    Text { len: u32, raw: bool },
}

/// Output of [`build_semi_index`]: the raw index data `XmlIndex::build`
/// wraps.
#[derive(Debug)]
pub(crate) struct SemiIndex {
    pub(crate) ib: Vec<u64>,
    pub(crate) bp: Vec<u64>,
    pub(crate) bp_len: usize,
    pub(crate) kinds: Vec<XmlNodeKind>,
}

/// A scan failure. Non-validating: only failures that would otherwise
/// panic or produce a nonsensical index are reported (unmatched tags,
/// truncated input, no root element).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum XmlScanError {
    /// Input ended while a tag, comment, attribute value, or element was
    /// still open.
    UnexpectedEof,
    /// An end tag (`</...>`) appeared with no matching open element.
    UnmatchedEndTag,
    /// No start tag was found anywhere in the input.
    NoRootElement,
    /// A start or end tag was malformed (e.g. missing `=`, missing quote,
    /// missing `>`).
    MalformedTag,
    /// Input exceeds `u32::MAX` bytes, which the IB rank directory cannot
    /// address (matches `JsonIndex`'s #188 limit).
    InputTooLarge,
}

impl core::fmt::Display for XmlScanError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let msg = match self {
            Self::UnexpectedEof => "unexpected end of input",
            Self::UnmatchedEndTag => "end tag with no matching open element",
            Self::NoRootElement => "no root element found",
            Self::MalformedTag => "malformed tag",
            Self::InputTooLarge => "input exceeds maximum supported size (u32::MAX bytes)",
        };
        f.write_str(msg)
    }
}

#[cfg(feature = "std")]
impl std::error::Error for XmlScanError {}

/// True for a byte that ends an element or attribute name span. Shared with
/// `light.rs`'s lazy name scanning (`tag_name`/`attr_name`) so the two stay
/// in lockstep — see `CLAUDE.md`'s note on duplicated predicates diverging
/// silently (#106).
#[inline]
pub(super) fn is_name_end(b: u8) -> bool {
    b.is_ascii_whitespace() || b == b'>' || b == b'/' || b == b'='
}

struct Scanner<'a> {
    xml: &'a [u8],
    i: usize,
    ib: BitWriter,
    ib_pos: usize,
    bp: BitWriter,
    bp_len: usize,
    kinds: Vec<XmlNodeKind>,
    depth: usize,
    root_found: bool,
    root_closed: bool,
}

impl<'a> Scanner<'a> {
    fn new(xml: &'a [u8]) -> Self {
        let word_capacity = xml.len().div_ceil(64);
        Self {
            xml,
            i: 0,
            ib: BitWriter::with_capacity(word_capacity),
            ib_pos: 0,
            bp: BitWriter::with_capacity(word_capacity),
            bp_len: 0,
            kinds: Vec::new(),
            depth: 0,
            root_found: false,
            root_closed: false,
        }
    }

    #[inline]
    fn byte(&self, pos: usize) -> Option<u8> {
        self.xml.get(pos).copied()
    }

    #[inline]
    fn starts_with(&self, pat: &[u8]) -> bool {
        self.xml[self.i..].starts_with(pat)
    }

    /// Find the start of the next occurrence of `pat` at or after `from`.
    fn find(&self, from: usize, pat: &[u8]) -> Option<usize> {
        if pat.is_empty() || from > self.xml.len() || pat.len() > self.xml.len() - from {
            return None;
        }
        self.xml[from..]
            .windows(pat.len())
            .position(|w| w == pat)
            .map(|p| from + p)
    }

    /// Mark a node-start byte in IB and record its kind. Advances the IB
    /// writer's position to `pos + 1`, filling the gap since the last mark
    /// with zeros.
    fn mark(&mut self, pos: usize, kind: XmlNodeKind) {
        debug_assert!(pos >= self.ib_pos);
        self.ib.write_zeros(pos - self.ib_pos);
        self.ib.write_1();
        self.ib_pos = pos + 1;
        self.kinds.push(kind);
    }

    /// Write a leaf BP node (`10`): open immediately followed by close.
    fn bp_leaf(&mut self) {
        self.bp.write_1();
        self.bp.write_0();
        self.bp_len += 2;
    }

    fn bp_open(&mut self) {
        self.bp.write_1();
        self.bp_len += 1;
    }

    fn bp_close(&mut self) {
        self.bp.write_0();
        self.bp_len += 1;
    }

    /// Skip a `<!-- ... -->` comment, `<?...?>` declaration/PI, or
    /// `<![CDATA[...]]>` section is handled separately (it's indexed, not
    /// skipped); this only handles the "skip to a fixed end marker"
    /// shape used by comments and declarations.
    fn skip_to(&mut self, end_marker: &[u8]) -> Result<(), XmlScanError> {
        let end = self
            .find(self.i, end_marker)
            .ok_or(XmlScanError::UnexpectedEof)?;
        self.i = end + end_marker.len();
        Ok(())
    }

    /// Skip a `<!DOCTYPE ...>` (or other `<!...>` markup declaration),
    /// tracking `[`/`]` depth so a `>` inside an internal subset doesn't
    /// end the declaration early.
    fn skip_doctype(&mut self) -> Result<(), XmlScanError> {
        let mut bracket_depth = 0u32;
        loop {
            match self.byte(self.i) {
                None => return Err(XmlScanError::UnexpectedEof),
                Some(b'[') => {
                    bracket_depth += 1;
                    self.i += 1;
                }
                Some(b']') => {
                    bracket_depth = bracket_depth.saturating_sub(1);
                    self.i += 1;
                }
                Some(b'>') if bracket_depth == 0 => {
                    self.i += 1;
                    return Ok(());
                }
                Some(_) => self.i += 1,
            }
        }
    }

    fn skip_whitespace(&mut self) {
        while let Some(b) = self.byte(self.i) {
            if b.is_ascii_whitespace() {
                self.i += 1;
            } else {
                break;
            }
        }
    }

    /// Scan one start tag beginning at `self.i` (positioned on `<`).
    /// Handles the tag name, attributes, and self-closing `/>`, updating
    /// `self.depth` and emitting BP/IB/kinds as it goes.
    fn scan_start_tag(&mut self) -> Result<(), XmlScanError> {
        let name_start = self.i + 1;
        self.mark(name_start, XmlNodeKind::Element);
        self.bp_open();
        self.depth += 1;

        let mut j = name_start;
        while let Some(b) = self.byte(j) {
            if is_name_end(b) {
                break;
            }
            j += 1;
        }
        if j == name_start {
            return Err(XmlScanError::MalformedTag);
        }
        self.i = j;

        loop {
            self.skip_whitespace();
            match self.byte(self.i) {
                None => return Err(XmlScanError::UnexpectedEof),
                Some(b'/') => {
                    if self.byte(self.i + 1) != Some(b'>') {
                        return Err(XmlScanError::MalformedTag);
                    }
                    self.depth -= 1;
                    self.bp_close();
                    self.i += 2;
                    if self.depth == 0 {
                        self.root_closed = true;
                    }
                    return Ok(());
                }
                Some(b'>') => {
                    self.i += 1;
                    return Ok(());
                }
                Some(_) => self.scan_attribute()?,
            }
        }
    }

    /// Scan one `name="value"` (or `name='value'`) attribute at `self.i`.
    fn scan_attribute(&mut self) -> Result<(), XmlScanError> {
        let name_start = self.i;
        self.mark(name_start, XmlNodeKind::Attribute);
        self.bp_leaf();

        let mut j = name_start;
        while let Some(b) = self.byte(j) {
            if is_name_end(b) {
                break;
            }
            j += 1;
        }
        if j == name_start {
            return Err(XmlScanError::MalformedTag);
        }
        self.i = j;
        self.skip_whitespace();
        if self.byte(self.i) != Some(b'=') {
            return Err(XmlScanError::MalformedTag);
        }
        self.i += 1;
        self.skip_whitespace();
        let quote = self.byte(self.i);
        if quote != Some(b'"') && quote != Some(b'\'') {
            return Err(XmlScanError::MalformedTag);
        }
        self.i += 1;
        let quote = quote.unwrap();
        loop {
            match self.byte(self.i) {
                None => return Err(XmlScanError::UnexpectedEof),
                Some(b) if b == quote => {
                    self.i += 1;
                    return Ok(());
                }
                Some(_) => self.i += 1,
            }
        }
    }

    /// Scan `</name>` at `self.i`. Names are not validated against their
    /// opening tag (non-validating semi-indexing, `CLAUDE.md`) — any end
    /// tag closes the innermost open element.
    fn scan_end_tag(&mut self) -> Result<(), XmlScanError> {
        if self.depth == 0 {
            return Err(XmlScanError::UnmatchedEndTag);
        }
        let close = self
            .find(self.i + 2, b">")
            .ok_or(XmlScanError::UnexpectedEof)?;
        self.depth -= 1;
        self.bp_close();
        self.i = close + 1;
        if self.depth == 0 {
            self.root_closed = true;
        }
        Ok(())
    }

    /// Scan a `<![CDATA[...]]>` section at `self.i`, indexing its content
    /// as a `Text` leaf (raw bytes, no entity re-decoding — per XML spec,
    /// CDATA content is literal).
    fn scan_cdata(&mut self) -> Result<(), XmlScanError> {
        let start = self.i + 9; // past "<![CDATA["
        let end = self
            .find(start, b"]]>")
            .ok_or(XmlScanError::UnexpectedEof)?;
        if end > start && self.depth > 0 {
            let len = (end - start) as u32;
            self.mark(start, XmlNodeKind::Text { len, raw: true });
            self.bp_leaf();
        }
        self.i = end + 3;
        Ok(())
    }

    /// Scan a plain (non-tag) text run starting at `self.i`, up to the
    /// next `<`. Whitespace-only runs are not indexed (decision #3 in the
    /// milestone plan: mixed-content whitespace preservation is out of
    /// scope).
    fn scan_text(&mut self) -> Result<(), XmlScanError> {
        let start = self.i;
        let mut j = start;
        while let Some(b) = self.byte(j) {
            if b == b'<' {
                break;
            }
            j += 1;
        }
        if self.depth > 0 && self.xml[start..j].iter().any(|b| !b.is_ascii_whitespace()) {
            let len = (j - start) as u32;
            self.mark(start, XmlNodeKind::Text { len, raw: false });
            self.bp_leaf();
        }
        self.i = j;
        Ok(())
    }

    fn run(mut self) -> Result<SemiIndex, XmlScanError> {
        if u32::try_from(self.xml.len()).is_err() {
            return Err(XmlScanError::InputTooLarge);
        }

        while self.i < self.xml.len() {
            if self.byte(self.i) == Some(b'<') {
                if self.starts_with(b"<?") {
                    self.i += 2;
                    self.skip_to(b"?>")?;
                } else if self.starts_with(b"<!--") {
                    self.i += 4;
                    self.skip_to(b"-->")?;
                } else if self.starts_with(b"<![CDATA[") {
                    self.scan_cdata()?;
                } else if self.starts_with(b"<!") {
                    self.i += 2;
                    self.skip_doctype()?;
                } else if self.starts_with(b"</") {
                    self.scan_end_tag()?;
                } else {
                    self.root_found = true;
                    self.scan_start_tag()?;
                }
            } else {
                self.scan_text()?;
            }
        }

        if self.depth != 0 {
            return Err(XmlScanError::UnexpectedEof);
        }
        if !self.root_found || !self.root_closed {
            return Err(XmlScanError::NoRootElement);
        }

        self.ib.write_zeros(self.xml.len() - self.ib_pos);

        Ok(SemiIndex {
            ib: self.ib.finish(),
            bp: self.bp.finish(),
            bp_len: self.bp_len,
            kinds: self.kinds,
        })
    }
}

/// Build an XML semi-index (IB + BP + per-node kinds) from raw bytes.
pub(crate) fn build_semi_index(xml: &[u8]) -> Result<SemiIndex, XmlScanError> {
    Scanner::new(xml).run()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn get_bit(words: &[u64], i: usize) -> bool {
        let word_idx = i / 64;
        let bit_idx = i % 64;
        word_idx < words.len() && (words[word_idx] >> bit_idx) & 1 == 1
    }

    fn ib_bits(words: &[u64], n: usize) -> alloc::string::String {
        (0..n)
            .map(|i| if get_bit(words, i) { '1' } else { '0' })
            .collect()
    }

    #[test]
    fn empty_element() {
        let semi = build_semi_index(b"<root/>").unwrap();
        // <root/>
        // 0123456
        assert_eq!(ib_bits(&semi.ib, 7), "0100000");
        assert_eq!(semi.kinds, alloc::vec![XmlNodeKind::Element]);
        assert_eq!(semi.bp_len, 2);
        assert_eq!(ib_bits(&semi.bp, 2), "10");
    }

    #[test]
    fn element_with_text() {
        let semi = build_semi_index(b"<root>hi</root>").unwrap();
        // <root>hi</root>
        // 0123456789...
        // IB set at 1 (tag name 'root') and 6 (text 'hi')
        assert!(get_bit(&semi.ib, 1));
        assert!(get_bit(&semi.ib, 6));
        assert_eq!(
            semi.kinds,
            alloc::vec![
                XmlNodeKind::Element,
                XmlNodeKind::Text { len: 2, raw: false }
            ]
        );
        assert_eq!(semi.bp_len, 4); // open root, leaf text, close root
        assert_eq!(ib_bits(&semi.bp, 4), "1100");
    }

    #[test]
    fn nested_elements() {
        let semi = build_semi_index(b"<root><foo><bar>x</bar></foo></root>").unwrap();
        assert_eq!(
            semi.kinds,
            alloc::vec![
                XmlNodeKind::Element,                     // root
                XmlNodeKind::Element,                     // foo
                XmlNodeKind::Element,                     // bar
                XmlNodeKind::Text { len: 1, raw: false }, // x
            ]
        );
    }

    #[test]
    fn attributes() {
        let semi = build_semi_index(br#"<root id="1" name="x"><child/></root>"#).unwrap();
        assert_eq!(
            semi.kinds,
            alloc::vec![
                XmlNodeKind::Element,
                XmlNodeKind::Attribute,
                XmlNodeKind::Attribute,
                XmlNodeKind::Element,
            ]
        );
    }

    #[test]
    fn declaration_comment_doctype_skipped() {
        let semi =
            build_semi_index(b"<?xml version=\"1.0\"?><!-- c --><!DOCTYPE root><root/>").unwrap();
        assert_eq!(semi.kinds, alloc::vec![XmlNodeKind::Element]);
    }

    #[test]
    fn cdata_and_entities_pass_through_as_text() {
        let semi = build_semi_index(b"<root>a &amp; b<![CDATA[<raw>]]></root>").unwrap();
        assert_eq!(
            semi.kinds,
            alloc::vec![
                XmlNodeKind::Element,
                XmlNodeKind::Text {
                    len: "a &amp; b".len() as u32,
                    raw: false
                },
                XmlNodeKind::Text {
                    len: "<raw>".len() as u32,
                    raw: true
                },
            ]
        );
    }

    #[test]
    fn repeated_siblings() {
        let semi = build_semi_index(b"<root><item/><item/><item/></root>").unwrap();
        assert_eq!(
            semi.kinds,
            alloc::vec![
                XmlNodeKind::Element,
                XmlNodeKind::Element,
                XmlNodeKind::Element,
                XmlNodeKind::Element,
            ]
        );
    }

    #[test]
    fn whitespace_only_text_not_indexed() {
        let semi = build_semi_index(b"<root>\n  <a/>\n</root>").unwrap();
        assert_eq!(
            semi.kinds,
            alloc::vec![XmlNodeKind::Element, XmlNodeKind::Element]
        );
    }

    #[test]
    fn missing_root_errors() {
        assert_eq!(
            build_semi_index(b"   ").unwrap_err(),
            XmlScanError::NoRootElement
        );
    }

    #[test]
    fn unmatched_end_tag_errors() {
        assert_eq!(
            build_semi_index(b"<root></root></root>").unwrap_err(),
            XmlScanError::UnmatchedEndTag
        );
    }

    #[test]
    fn truncated_input_errors() {
        assert_eq!(
            build_semi_index(b"<root>").unwrap_err(),
            XmlScanError::UnexpectedEof
        );
        assert_eq!(
            build_semi_index(b"<root").unwrap_err(),
            XmlScanError::UnexpectedEof
        );
    }
}
