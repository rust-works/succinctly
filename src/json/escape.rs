//! JSON string escaping, shared by every output path in the crate.
//!
//! Before #91 there were eleven copies of this logic — in `jq/value.rs`,
//! `jq/eval.rs`, `jq/lazy.rs`, `jq/stream.rs`, `yaml/light.rs`, and four in the
//! CLI's `output.rs` — spread across three mutually incompatible sets of
//! semantics, only two of which were SIMD-accelerated. This module is the single
//! implementation they all now call.
//!
//! ## Two layers
//!
//! Escaping splits cleanly in two, and the split is worth preserving:
//!
//! - **Scanners** (`crate::util::simd::escape`, re-exported below) answer *where
//!   is the next byte I have to look at?* — format-agnostic SIMD machinery that
//!   chews 16–32 bytes per step.
//! - **Escapers** (this module) answer *what do I emit for it?* — the
//!   convention-specific part.
//!
//! The payoff is not primarily the SIMD. Real JSON strings are short (p50 = 7
//! bytes, p90 = 11 — see `docs/benchmarks/corpus-shape.md`), so most never reach
//! a SIMD kernel at all. The win is what adopting a scanner *forces*: copying
//! safe spans with one `write_str` instead of pushing a `char` at a time, and
//! emitting `\u00xx` from a nibble table instead of allocating a `String` per
//! control character via `format!`.
//!
//! ## Conventions
//!
//! [`EscapeStyle`] selects between jq's and yq's control-character rules, which
//! genuinely differ — both are pinned against their respective upstream oracles:
//!
//! | | `Jq` | `Yq` |
//! |---|---|---|
//! | backspace / form feed | `\b` / `\f` | `` / `` |
//! | DEL (`0x7F`) | `` | raw |
//! | C1 block (U+0080–U+009F) | raw | raw |
//! | `"` `\` `\n` `\r` `\t` | short forms | short forms |
//! | other C0 controls | `\u00xx` | `\u00xx` |
//!
//! Orthogonally, `ascii` escapes every non-ASCII character as `\uXXXX` (astral
//! characters as a UTF-16 surrogate pair), matching `jq --ascii-output` and
//! `yq`'s ASCII mode. In that mode the C1 row above stops mattering: those
//! characters are non-ASCII, so both styles escape them either way.
//!
//! # Examples
//!
//! ```
//! use succinctly::json::escape::{quoted_to_string, EscapeStyle};
//!
//! // jq spells backspace with the short form; yq spells it out in full.
//! assert_eq!(quoted_to_string("a\u{8}b", EscapeStyle::Jq, false), "\"a\\bb\"");
//! assert_eq!(quoted_to_string("a\u{8}b", EscapeStyle::Yq, false), "\"a\\u0008b\"");
//!
//! // ASCII mode escapes everything above U+007F, surrogate-pairing astrals.
//! assert_eq!(quoted_to_string("é", EscapeStyle::Jq, true), "\"\\u00e9\"");
//! assert_eq!(quoted_to_string("😀", EscapeStyle::Jq, true), "\"\\ud83d\\ude00\"");
//! ```

#[cfg(not(test))]
use alloc::string::String;

use core::fmt;

pub use crate::util::simd::escape::{find_ascii_escape, find_jq_escape, find_json_escape};

/// Which tool's control-character convention a JSON string escaper follows.
///
/// See the module docs for the full table. The two differ only on backspace,
/// form feed, and DEL; everything else is common to both.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EscapeStyle {
    /// jq's rules: `\b` / `\f` short escapes and DEL as ``.
    ///
    /// Verified against jq 1.7.1, including that it emits the C1 block raw —
    /// see the `escape_del_and_c1` golden case.
    Jq,
    /// yq's rules: backspace and form feed as `` / ``, DEL raw.
    ///
    /// Matches `mikefarah/yq`; pinned by the yq golden suite and #262.
    Yq,
}

/// Lowercase nibbles, matching `format!("{:04x}")`.
const HEX: &[u8; 16] = b"0123456789abcdef";

/// Write `\u00XX` for a byte known to be `< 0x20` or DEL.
#[inline]
fn write_hex_byte<W: fmt::Write>(out: &mut W, b: u8) -> fmt::Result {
    out.write_str("\\u00")?;
    out.write_char(HEX[(b >> 4) as usize] as char)?;
    out.write_char(HEX[(b & 0xf) as usize] as char)
}

/// Write `\uXXXX` for a BMP code unit.
#[inline]
fn write_hex_u16<W: fmt::Write>(out: &mut W, u: u32) -> fmt::Result {
    out.write_str("\\u")?;
    out.write_char(HEX[((u >> 12) & 0xf) as usize] as char)?;
    out.write_char(HEX[((u >> 8) & 0xf) as usize] as char)?;
    out.write_char(HEX[((u >> 4) & 0xf) as usize] as char)?;
    out.write_char(HEX[(u & 0xf) as usize] as char)
}

/// Write a character as `\uXXXX`, or as a UTF-16 surrogate pair above the BMP.
#[inline]
fn write_unicode_escape<W: fmt::Write>(out: &mut W, c: char) -> fmt::Result {
    let cp = c as u32;
    if cp <= 0xFFFF {
        write_hex_u16(out, cp)
    } else {
        let adjusted = cp - 0x10000;
        write_hex_u16(out, 0xD800 + (adjusted >> 10))?;
        write_hex_u16(out, 0xDC00 + (adjusted & 0x3FF))
    }
}

/// The escaping loop, monomorphized over the two style axes.
///
/// Const generics rather than runtime flags so each of the four combinations
/// compiles to a straight-line loop with its own scanner call and no branching on
/// style — the `if ASCII` / `if JQ` tests below const-fold away entirely.
fn write_body_impl<W: fmt::Write, const JQ: bool, const ASCII: bool>(
    out: &mut W,
    s: &str,
) -> fmt::Result {
    let bytes = s.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        // Const-folded: exactly one of these survives per monomorphization.
        let pos = if ASCII {
            find_ascii_escape(bytes, i)
        } else if JQ {
            find_jq_escape(bytes, i)
        } else {
            find_json_escape(bytes, i)
        };

        debug_assert!(
            s.is_char_boundary(pos),
            "escape scanner returned a non-boundary index"
        );

        // The whole point: one bulk copy of everything that needs no escaping.
        if i < pos {
            out.write_str(&s[i..pos])?;
        }
        i = pos;
        if i == len {
            break;
        }

        let b = bytes[i];
        match b {
            b'"' => out.write_str("\\\"")?,
            b'\\' => out.write_str("\\\\")?,
            b'\n' => out.write_str("\\n")?,
            b'\r' => out.write_str("\\r")?,
            b'\t' => out.write_str("\\t")?,
            0x08 if JQ => out.write_str("\\b")?,
            0x0C if JQ => out.write_str("\\f")?,
            _ if b < 0x20 => write_hex_byte(out, b)?,
            0x7F => {
                // Only the jq and ASCII scanners stop here. jq escapes DEL; yq
                // leaves it raw even in ASCII mode, so under `Yq` this is a
                // tolerated false-positive stop.
                if JQ {
                    write_hex_byte(out, b)?;
                } else {
                    out.write_char(0x7F as char)?;
                }
            }
            _ => {
                // Reached only under ASCII, whose scanner stops on every byte
                // >= 0x7F. `pos` is a char boundary, so this always decodes.
                let c = s[i..].chars().next().expect("stop is a char boundary");
                write_unicode_escape(out, c)?;
                // Advance by the FULL character. A one-byte advance would resume
                // mid-character and the next scan would return a continuation-byte
                // index — see the caller contract on `find_ascii_escape`.
                i += c.len_utf8();
                continue;
            }
        }
        i += 1;
    }

    Ok(())
}

/// Write the escaped *body* of `s` — no surrounding quotes — to `out`.
///
/// # Examples
///
/// ```
/// use succinctly::json::escape::{write_body, EscapeStyle};
///
/// let mut s = String::new();
/// write_body(&mut s, "a\"b", EscapeStyle::Jq, false).unwrap();
/// assert_eq!(s, r#"a\"b"#);
/// ```
#[inline]
pub fn write_body<W: fmt::Write>(
    out: &mut W,
    s: &str,
    style: EscapeStyle,
    ascii: bool,
) -> fmt::Result {
    match (style, ascii) {
        (EscapeStyle::Jq, false) => write_body_impl::<W, true, false>(out, s),
        (EscapeStyle::Jq, true) => write_body_impl::<W, true, true>(out, s),
        (EscapeStyle::Yq, false) => write_body_impl::<W, false, false>(out, s),
        (EscapeStyle::Yq, true) => write_body_impl::<W, false, true>(out, s),
    }
}

/// Write `s` as a complete JSON string literal, quotes included.
///
/// # Examples
///
/// ```
/// use succinctly::json::escape::{write_quoted, EscapeStyle};
///
/// let mut s = String::new();
/// write_quoted(&mut s, "a\nb", EscapeStyle::Jq, false).unwrap();
/// assert_eq!(s, r#""a\nb""#);
/// ```
#[inline]
pub fn write_quoted<W: fmt::Write>(
    out: &mut W,
    s: &str,
    style: EscapeStyle,
    ascii: bool,
) -> fmt::Result {
    out.write_char('"')?;
    write_body(out, s, style, ascii)?;
    out.write_char('"')
}

/// Return the escaped body of `s`, without surrounding quotes.
///
/// # Examples
///
/// ```
/// use succinctly::json::escape::{body_to_string, EscapeStyle};
///
/// assert_eq!(body_to_string("tab\there", EscapeStyle::Jq, false), r"tab\there");
/// ```
pub fn body_to_string(s: &str, style: EscapeStyle, ascii: bool) -> String {
    let mut out = String::with_capacity(s.len());
    // Writing into a String is infallible.
    let _ = write_body(&mut out, s, style, ascii);
    out
}

/// Return `s` as a complete JSON string literal, quotes included.
///
/// # Examples
///
/// ```
/// use succinctly::json::escape::{quoted_to_string, EscapeStyle};
///
/// assert_eq!(quoted_to_string("hi", EscapeStyle::Yq, false), r#""hi""#);
/// ```
pub fn quoted_to_string(s: &str, style: EscapeStyle, ascii: bool) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    // Writing into a String is infallible.
    let _ = write_quoted(&mut out, s, style, ascii);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use EscapeStyle::{Jq, Yq};

    // ------------------------------------------------------------------------
    // Frozen scalar escapers.
    //
    // Verbatim copies of the four per-char implementations that lived in
    // src/bin/succinctly/output.rs before #91, captured 2026-07. They are the
    // differential oracle for the rewrite: NEVER "fix" or modernize them. Where
    // the new escaper deliberately disagrees, the difference is enumerated in
    // DELIBERATE_DIVERGENCES below rather than smoothed away here.
    // ------------------------------------------------------------------------

    fn frozen_jq(s: &str) -> String {
        let mut result = String::with_capacity(s.len());
        for c in s.chars() {
            match c {
                '"' => result.push_str("\\\""),
                '\\' => result.push_str("\\\\"),
                '\x08' => result.push_str("\\b"),
                '\x0C' => result.push_str("\\f"),
                '\n' => result.push_str("\\n"),
                '\r' => result.push_str("\\r"),
                '\t' => result.push_str("\\t"),
                c if c.is_control() => {
                    result.push_str(&format!("\\u{:04x}", c as u32));
                }
                c => result.push(c),
            }
        }
        result
    }

    fn frozen_jq_ascii(s: &str) -> String {
        let mut result = String::with_capacity(s.len());
        for c in s.chars() {
            match c {
                '"' => result.push_str("\\\""),
                '\\' => result.push_str("\\\\"),
                '\x08' => result.push_str("\\b"),
                '\x0C' => result.push_str("\\f"),
                '\n' => result.push_str("\\n"),
                '\r' => result.push_str("\\r"),
                '\t' => result.push_str("\\t"),
                c if c.is_control() => {
                    result.push_str(&format!("\\u{:04x}", c as u32));
                }
                c if !c.is_ascii() => {
                    let code = c as u32;
                    if code <= 0xFFFF {
                        result.push_str(&format!("\\u{code:04x}"));
                    } else {
                        let adjusted = code - 0x10000;
                        let high = 0xD800 + (adjusted >> 10);
                        let low = 0xDC00 + (adjusted & 0x3FF);
                        result.push_str(&format!("\\u{high:04x}\\u{low:04x}"));
                    }
                }
                c => result.push(c),
            }
        }
        result
    }

    fn frozen_yq(s: &str) -> String {
        let mut result = String::with_capacity(s.len());
        for c in s.chars() {
            match c {
                '"' => result.push_str("\\\""),
                '\\' => result.push_str("\\\\"),
                '\n' => result.push_str("\\n"),
                '\r' => result.push_str("\\r"),
                '\t' => result.push_str("\\t"),
                c if (c as u32) < 0x20 => {
                    result.push_str(&format!("\\u{:04x}", c as u32));
                }
                c => result.push(c),
            }
        }
        result
    }

    fn frozen_yq_ascii(s: &str) -> String {
        let mut result = String::with_capacity(s.len());
        for c in s.chars() {
            match c {
                '"' => result.push_str("\\\""),
                '\\' => result.push_str("\\\\"),
                '\n' => result.push_str("\\n"),
                '\r' => result.push_str("\\r"),
                '\t' => result.push_str("\\t"),
                c if (c as u32) < 0x20 => {
                    result.push_str(&format!("\\u{:04x}", c as u32));
                }
                c if !c.is_ascii() => {
                    let code = c as u32;
                    if code <= 0xFFFF {
                        result.push_str(&format!("\\u{code:04x}"));
                    } else {
                        let adjusted = code - 0x10000;
                        let high = 0xD800 + (adjusted >> 10);
                        let low = 0xDC00 + (adjusted & 0x3FF);
                        result.push_str(&format!("\\u{high:04x}\\u{low:04x}"));
                    }
                }
                c => result.push(c),
            }
        }
        result
    }

    fn frozen(style: EscapeStyle, ascii: bool) -> fn(&str) -> String {
        match (style, ascii) {
            (Jq, false) => frozen_jq,
            (Jq, true) => frozen_jq_ascii,
            (Yq, false) => frozen_yq,
            (Yq, true) => frozen_yq_ascii,
        }
    }

    const MODES: [(EscapeStyle, bool); 4] = [(Jq, false), (Jq, true), (Yq, false), (Yq, true)];

    /// The C1 control block, U+0080..=U+009F.
    fn is_c1(c: char) -> bool {
        ('\u{80}'..='\u{9f}').contains(&c)
    }

    /// Where the new escaper deliberately disagrees with the frozen one.
    ///
    /// Exactly one divergence, approved in #91: jq 1.7.1 emits the C1 block raw
    /// in UTF-8 output, while `char::is_control()` (which the frozen escaper
    /// used) treats U+0080..U+009F as controls and escaped them. ASCII mode is
    /// unaffected — there C1 is escaped as non-ASCII either way.
    ///
    /// Anything NOT described here must match the frozen escaper byte for byte.
    fn is_deliberate_divergence(style: EscapeStyle, ascii: bool, c: char) -> bool {
        style == Jq && !ascii && is_c1(c)
    }

    /// Inputs exercising every path: chunk edges, the scalar remainder, each
    /// predicate's distinguishing bytes, and multibyte characters straddling
    /// boundaries.
    fn corpus() -> Vec<String> {
        let mut out: Vec<String> = Vec::new();

        out.push(String::new());
        // Every single byte value that is a valid char on its own.
        for b in 0u32..=255 {
            out.push(char::from_u32(b).unwrap().to_string());
        }
        // The C1 block and the Latin-1 punctuation just above it, alone and
        // embedded — the band that separates jq's rules from the frozen ones.
        for cp in 0x80u32..=0xBF {
            let c = char::from_u32(cp).unwrap();
            out.push(c.to_string());
            out.push(format!("abc{c}def"));
            out.push(format!("{c}{c}{c}"));
        }
        // Astral characters -> surrogate pairs.
        for c in ['\u{10000}', '\u{1F600}', '\u{10FFFF}', '\u{FFFF}'] {
            out.push(c.to_string());
            out.push(format!("pad{c}pad"));
        }
        // Each interesting character at every offset across two SIMD chunks, so
        // it lands inside the 16/32-byte loops, in the tails, and straddling the
        // edges. Multibyte cases also split a character across a chunk boundary.
        for e in [
            '"', '\\', '\n', '\t', '\u{8}', '\u{c}', '\u{0}', '\u{7f}', '\u{85}', '\u{a0}', 'é',
            '😀',
        ] {
            for pos in 0..=64usize {
                let mut s = "a".repeat(pos);
                s.push(e);
                s.push_str(&"a".repeat(64 - pos));
                out.push(s);
            }
        }
        // Escape only in the scalar remainder, past both chunk loops.
        for len in [17usize, 23, 31, 33, 40, 47] {
            let mut s = "b".repeat(len - 1);
            s.push('"');
            out.push(s);
        }
        // No escapes at all (the common real-world case), and all escapes.
        for len in [0usize, 1, 7, 11, 15, 16, 17, 31, 32, 33, 64, 1024] {
            out.push("x".repeat(len));
        }
        out.push("\"\\\n\t\u{8}\u{c}\u{7f}".repeat(20));
        out.push("café au lait — naïve, £5, «quoted»".into());
        out.push("日本語のテキスト 😀 mixed with ASCII".into());

        out
    }

    /// The core differential: the SIMD escaper must reproduce the frozen
    /// per-char escaper byte for byte, except where #91 says otherwise.
    #[test]
    fn matches_frozen_scalar_escapers() {
        for s in corpus() {
            for (style, ascii) in MODES {
                let got = body_to_string(&s, style, ascii);
                let want = frozen(style, ascii)(&s);
                if s.chars().any(|c| is_deliberate_divergence(style, ascii, c)) {
                    continue;
                }
                assert_eq!(
                    got, want,
                    "mismatch for {s:?} under {style:?}/ascii={ascii}"
                );
            }
        }
    }

    /// The one approved divergence, asserted from both sides so it can never
    /// change silently: C1 raw under jq, still escaped by the frozen escaper.
    #[test]
    fn c1_block_is_raw_under_jq_and_escaped_by_the_frozen_escaper() {
        for cp in 0x80u32..=0x9F {
            let c = char::from_u32(cp).unwrap();
            let s = c.to_string();

            assert_eq!(
                body_to_string(&s, Jq, false),
                s,
                "jq must emit U+{cp:04X} raw"
            );
            assert_eq!(
                frozen_jq(&s),
                format!("\\u{cp:04x}"),
                "the frozen escaper must still escape U+{cp:04X}"
            );

            // yq always emitted C1 raw; ASCII mode always escapes it. Neither moved.
            assert_eq!(body_to_string(&s, Yq, false), s);
            assert_eq!(body_to_string(&s, Jq, true), format!("\\u{cp:04x}"));
            assert_eq!(body_to_string(&s, Yq, true), format!("\\u{cp:04x}"));
        }
    }

    /// Completeness: if the escaper rewrites a character, the scanner must stop
    /// at its first byte. This is the direction the scanner's own exhaustive
    /// parity test cannot see — that one compares a scanner to its own
    /// reference, never to the escaper it exists to serve. It is what catches a
    /// predicate that forgets DEL.
    #[test]
    fn scanner_stops_at_every_character_the_escaper_rewrites() {
        let mut buf = [0u8; 4];
        for cp in 0u32..=0x10FFFF {
            let Some(c) = char::from_u32(cp) else {
                continue;
            };
            let s: &str = c.encode_utf8(&mut buf);
            for (style, ascii) in MODES {
                let rewritten = body_to_string(s, style, ascii) != *s;
                if !rewritten {
                    continue;
                }
                let pos = match (style, ascii) {
                    (_, true) => find_ascii_escape(s.as_bytes(), 0),
                    (Jq, false) => find_jq_escape(s.as_bytes(), 0),
                    (Yq, false) => find_json_escape(s.as_bytes(), 0),
                };
                assert_eq!(
                    pos, 0,
                    "scanner walks past U+{cp:04X}, which {style:?}/ascii={ascii} rewrites"
                );
            }
        }
    }

    /// Soundness of the tolerated false-positive stop: yq's ASCII mode stops at
    /// DEL (the shared `>= 0x7F` predicate) but must emit it unchanged.
    #[test]
    fn false_positive_stops_round_trip_unchanged() {
        assert_eq!(find_ascii_escape(b"\x7f", 0), 0, "expected a stop at DEL");
        assert_eq!(body_to_string("\u{7f}", Yq, true), "\u{7f}");
        assert_eq!(
            body_to_string("a\u{7f}b\u{7f}c", Yq, true),
            "a\u{7f}b\u{7f}c"
        );
        // ...while jq escapes it in both modes.
        assert_eq!(body_to_string("\u{7f}", Jq, false), "\\u007f");
        assert_eq!(body_to_string("\u{7f}", Jq, true), "\\u007f");
        // yq's unicode mode never even stops there.
        assert_eq!(body_to_string("\u{7f}", Yq, false), "\u{7f}");
    }

    /// Style-specific behaviour, pinned directly rather than via the frozen
    /// escapers, so the table in the module docs stays honest.
    #[test]
    fn style_table_holds() {
        assert_eq!(body_to_string("\u{8}\u{c}", Jq, false), "\\b\\f");
        assert_eq!(body_to_string("\u{8}\u{c}", Yq, false), "\\u0008\\u000c");
        assert_eq!(body_to_string("\u{0}\u{1f}", Jq, false), "\\u0000\\u001f");
        assert_eq!(body_to_string("\u{0}\u{1f}", Yq, false), "\\u0000\\u001f");
        assert_eq!(body_to_string("\"\\\n\r\t", Jq, false), "\\\"\\\\\\n\\r\\t");
        assert_eq!(body_to_string("\"\\\n\r\t", Yq, false), "\\\"\\\\\\n\\r\\t");
        // ASCII mode: BMP, astral, and a character that needs no escaping.
        assert_eq!(body_to_string("é", Jq, true), "\\u00e9");
        assert_eq!(body_to_string("😀", Jq, true), "\\ud83d\\ude00");
        assert_eq!(body_to_string("a", Jq, true), "a");
    }

    /// Surrogate pairs, cross-checked against `char::encode_utf16` rather than
    /// against hand-computed constants.
    #[test]
    fn astral_escapes_match_encode_utf16() {
        let mut units = [0u16; 2];
        for cp in [0x10000u32, 0x1F600, 0x2070E, 0x10FFFF] {
            let c = char::from_u32(cp).unwrap();
            let encoded = c.encode_utf16(&mut units);
            let want: String = encoded.iter().map(|u| format!("\\u{u:04x}")).collect();
            assert_eq!(body_to_string(&c.to_string(), Jq, true), want);
            assert_eq!(body_to_string(&c.to_string(), Yq, true), want);
        }
    }

    /// `write_quoted` is `write_body` plus the delimiters, and the two
    /// `*_to_string` helpers agree with their writer forms.
    #[test]
    fn quoted_and_writer_forms_agree() {
        for s in ["", "plain", "a\"b", "é😀", "\u{7f}\u{85}"] {
            for (style, ascii) in MODES {
                let body = body_to_string(s, style, ascii);
                assert_eq!(quoted_to_string(s, style, ascii), format!("\"{body}\""));

                let mut w = String::new();
                write_body(&mut w, s, style, ascii).unwrap();
                assert_eq!(w, body);

                let mut w = String::new();
                write_quoted(&mut w, s, style, ascii).unwrap();
                assert_eq!(w, format!("\"{body}\""));
            }
        }
    }

    /// Escaper output must always be valid JSON that round-trips — the property
    /// the per-char escapers could satisfy only by accident.
    #[test]
    fn output_is_always_valid_json_that_round_trips() {
        for s in corpus() {
            for (style, ascii) in MODES {
                let json = quoted_to_string(&s, style, ascii);
                let parsed: String = serde_json::from_str(&json).unwrap_or_else(|e| {
                    panic!("invalid JSON for {s:?} under {style:?}/ascii={ascii}: {json:?}: {e}")
                });
                assert_eq!(parsed, s, "round-trip lost data for {s:?}");
                if ascii {
                    assert!(json.is_ascii(), "ascii mode emitted non-ASCII for {s:?}");
                }
            }
        }
    }

    /// A string needing no escaping must come back byte-identical, and cost no
    /// more than its own length in capacity.
    #[test]
    fn escape_free_strings_pass_through() {
        let s = "the quick brown fox jumps over the lazy dog 0123456789";
        for (style, ascii) in MODES {
            assert_eq!(body_to_string(s, style, ascii), s);
        }
    }
}
