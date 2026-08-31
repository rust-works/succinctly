//! The two JSON string-escaping conventions, each defined exactly once.
//!
//! `jq` and `mikefarah/yq` disagree about which characters a JSON string must
//! escape, so succinctly needs both — but it needs each of them *once*. This
//! module is that single definition; every writer in the crate and in the CLI
//! routes here rather than open-coding a `match` over `char`.
//!
//! See [`write_json_body_jq`] for the table both conventions are pinned to, and
//! `conventions_differ_at_exactly_three_code_points` in this module's tests for
//! the assertion that keeps them from drifting apart again (#385).

#[cfg(not(test))]
use alloc::string::String;

use core::fmt::Write;

use crate::util::simd::escape::find_json_escape;

/// Lowercase hex digits, for the `\u00xx` forms.
const HEX: &[u8; 16] = b"0123456789abcdef";

/// Write `\u00xx` for a byte in `0x00..=0xff`.
#[inline]
fn write_short_u_escape<W: Write>(out: &mut W, b: u8) -> core::fmt::Result {
    out.write_str("\\u00")?;
    out.write_char(HEX[(b >> 4) as usize] as char)?;
    out.write_char(HEX[(b & 0xf) as usize] as char)
}

/// Write `\uXXXX`, or a surrogate pair for a code point above the BMP.
#[inline]
fn write_u_escape<W: Write>(out: &mut W, c: char) -> core::fmt::Result {
    let cp = c as u32;
    if cp <= 0xFFFF {
        write_bmp_u_escape(out, cp)
    } else {
        let adjusted = cp - 0x10000;
        write_bmp_u_escape(out, 0xD800 + (adjusted >> 10))?;
        write_bmp_u_escape(out, 0xDC00 + (adjusted & 0x3FF))
    }
}

/// Write `\uXXXX` for a value that fits the BMP.
#[inline]
fn write_bmp_u_escape<W: Write>(out: &mut W, cp: u32) -> core::fmt::Result {
    out.write_str("\\u")?;
    for shift in [12, 8, 4, 0] {
        out.write_char(HEX[((cp >> shift) & 0xF) as usize] as char)?;
    }
    Ok(())
}

/// Escape `s` into `out` as a JSON string **body** — no surrounding quotes —
/// using jq's convention.
///
/// Pinned against `jq-1.7.1`; the `yq` column is what `mikefarah/yq` emits and
/// what the `tests/data/yq-golden` fixtures hold.
///
/// | character            | jq             | yq             |
/// |----------------------|----------------|----------------|
/// | `"` `\`              | `\"` `\\`      | `\"` `\\`      |
/// | `0x08` `0x0c`        | `\b` `\f`      | `` `` |
/// | `0x09` `0x0a` `0x0d` | `\t` `\n` `\r` | `\t` `\n` `\r` |
/// | other `< 0x20`       | `\u00xx`       | `\u00xx`       |
/// | `0x7f` (DEL)         | ``       | raw            |
/// | `0x80..=0x9f` (C1)   | raw            | raw            |
/// | other non-ASCII      | raw            | raw            |
///
/// The two conventions therefore differ at exactly three code points: `0x08`,
/// `0x0c` and `0x7f`.
///
/// The C1 row is the one that cost a bug (#385): `char::is_control()` is true
/// for U+0080–U+009F, so branching on it escapes characters JSON does not
/// require escaping and jq does not escape. RFC 8259 only mandates escaping
/// below U+0020; jq escapes DEL as well, which is why the predicate here is
/// `< 0x20 || == 0x7f` rather than either `is_control()` or a bare `< 0x20`.
pub fn write_json_body_jq<W: Write>(out: &mut W, s: &str) -> core::fmt::Result {
    for c in s.chars() {
        match c {
            '"' => out.write_str("\\\"")?,
            '\\' => out.write_str("\\\\")?,
            '\x08' => out.write_str("\\b")?,
            '\x0c' => out.write_str("\\f")?,
            '\n' => out.write_str("\\n")?,
            '\r' => out.write_str("\\r")?,
            '\t' => out.write_str("\\t")?,
            c if is_jq_escaped_control(c) => write_short_u_escape(out, c as u8)?,
            c => out.write_char(c)?,
        }
    }
    Ok(())
}

/// [`write_json_body_jq`], plus `\uXXXX` for every non-ASCII character — jq's
/// `--ascii-output` (`-a`) mode.
pub fn write_json_body_jq_ascii<W: Write>(out: &mut W, s: &str) -> core::fmt::Result {
    for c in s.chars() {
        match c {
            '"' => out.write_str("\\\"")?,
            '\\' => out.write_str("\\\\")?,
            '\x08' => out.write_str("\\b")?,
            '\x0c' => out.write_str("\\f")?,
            '\n' => out.write_str("\\n")?,
            '\r' => out.write_str("\\r")?,
            '\t' => out.write_str("\\t")?,
            c if is_jq_escaped_control(c) => write_short_u_escape(out, c as u8)?,
            c if !c.is_ascii() => write_u_escape(out, c)?,
            c => out.write_char(c)?,
        }
    }
    Ok(())
}

/// Escape `s` into `out` as a JSON string **body** — no surrounding quotes —
/// using yq's convention: no `\b`/`\f` short forms, and DEL left raw. See
/// [`write_json_body_jq`] for the full table.
///
/// This is the hot path for `syq -o json`, so it keeps the SIMD escape scan
/// (O3, #87): [`find_json_escape`] looks for `"`, `\` and `< 0x20`, which is
/// exactly yq's escape set, and everything between two hits is copied as one
/// span.
pub fn write_json_body_yq<W: Write>(out: &mut W, s: &str) -> core::fmt::Result {
    let bytes = s.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        let escape_pos = find_json_escape(bytes, i);

        if i < escape_pos {
            out.write_str(&s[i..escape_pos])?;
        }

        i = escape_pos;

        if i < len {
            let b = bytes[i];
            match b {
                b'"' => out.write_str("\\\"")?,
                b'\\' => out.write_str("\\\\")?,
                b'\n' => out.write_str("\\n")?,
                b'\r' => out.write_str("\\r")?,
                b'\t' => out.write_str("\\t")?,
                // `find_json_escape` only stops on the three cases above and on
                // `< 0x20`, so nothing else can reach here.
                b => write_short_u_escape(out, b)?,
            }
            i += 1;
        }
    }

    Ok(())
}

/// [`write_json_body_yq`], plus `\uXXXX` for every non-ASCII character — yq's
/// ASCII output mode.
///
/// Non-ASCII means this cannot copy spans the way [`write_json_body_yq`] does,
/// so it walks `char`s; ASCII output is not a hot path.
pub fn write_json_body_yq_ascii<W: Write>(out: &mut W, s: &str) -> core::fmt::Result {
    for c in s.chars() {
        match c {
            '"' => out.write_str("\\\"")?,
            '\\' => out.write_str("\\\\")?,
            '\n' => out.write_str("\\n")?,
            '\r' => out.write_str("\\r")?,
            '\t' => out.write_str("\\t")?,
            c if (c as u32) < 0x20 => write_short_u_escape(out, c as u8)?,
            c if !c.is_ascii() => write_u_escape(out, c)?,
            c => out.write_char(c)?,
        }
    }
    Ok(())
}

/// The controls jq escapes as `\u00xx` once the short forms are taken: C0 minus
/// `\b \f \n \r \t`, plus DEL. Deliberately **not** `char::is_control()`, which
/// also covers C1 — see [`write_json_body_jq`].
#[inline]
const fn is_jq_escaped_control(c: char) -> bool {
    (c as u32) < 0x20 || c == '\u{7f}'
}

/// Run one of this module's body writers into a fresh `String`.
///
/// The writers are generic over [`core::fmt::Write`], which is what lets the
/// streaming callers avoid an allocation; this is the convenience for callers
/// that are building a `String` anyway.
///
/// ```
/// use succinctly::jq::escape::{escape_json_body, write_json_body_jq};
/// assert_eq!(escape_json_body(write_json_body_jq, "a\u{8}b"), "a\\bb");
/// ```
pub fn escape_json_body(write: fn(&mut String, &str) -> core::fmt::Result, s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    // Writing into a `String` is infallible.
    let _ = write(&mut out, s);
    out
}

/// A [`Write`] adapter that rewrites non-ASCII characters as `\uXXXX` escapes.
///
/// Surrogate-paired above the BMP, the same convention
/// [`write_json_body_yq_ascii`] applies inside a string body; every ASCII byte
/// passes through untouched.
///
/// This is `--ascii-output` for a *streamed* JSON document (#1700), applied to
/// the finished character stream rather than inside the string writers. That
/// is sound because of a property of JSON itself, not of any particular
/// writer: **outside a string literal, JSON's grammar admits only ASCII** —
/// structural punctuation, digits, `true`/`false`/`null`, whitespace — and
/// every escape sequence a writer emits is ASCII too. So every non-ASCII
/// character in a well-formed JSON stream is content inside a string body,
/// which is exactly the set `--ascii-output` has to escape, and escaping at
/// the sink cannot reach anything else.
///
/// Two consequences worth naming:
///
/// - It is **idempotent**. Output that is already all-ASCII — the DOM path's
///   `format_json` with `ascii: true`, say — passes through byte-for-byte, so
///   wrapping a sink that some values reach by another route is harmless.
/// - It composes with any downstream stage that is itself ASCII, ANSI color
///   codes included.
///
/// Deliberately **not** an `ascii: bool` threaded through the M2 streamers,
/// which is the fix direction #1700 itself first proposed. String bytes reach
/// the sink from three independent writers — `stream_json_string` and the two
/// `stream_transcode_*_quoted_to_json` scalar transcoders, the latter pair
/// carrying ~15 inline write sites each — behind a deeper stack of call paths
/// (`stream_json_value`, `stream_json_sequence`, `stream_resolved_scalar_as_json`,
/// the mapping-key arm) that would each have to carry the flag. A flag missed
/// at any one of them is a silently wrong document. The grammar argument above
/// covers all of them at once, and leaves `stream_json_string`'s hot SIMD scan
/// untouched — #965 measured that scan's inlining as worth up to 14%, and this
/// adapter only exists on the `--ascii-output` branch, so the default path is
/// compiled exactly as before.
///
/// ```
/// use core::fmt::Write;
/// use succinctly::jq::escape::AsciiEscapeWriter;
///
/// let mut out = String::new();
/// AsciiEscapeWriter::new(&mut out).write_str("{\"k\":\"héllo 😀\"}").unwrap();
/// // Structure and ASCII content pass through untouched; the two content
/// // characters are escaped, the astral one as a surrogate pair.
/// assert_eq!(out, r#"{"k":"h\u00e9llo \ud83d\ude00"}"#);
/// ```
pub struct AsciiEscapeWriter<'a, W> {
    inner: &'a mut W,
}

impl<'a, W: Write> AsciiEscapeWriter<'a, W> {
    /// Wrap `inner`, escaping every non-ASCII character written through it.
    pub fn new(inner: &'a mut W) -> Self {
        Self { inner }
    }

    /// Reborrow the wrapped writer directly, bypassing this wrapper's own
    /// ASCII-escaping `Write` impl -- for a caller that needs one of the
    /// inner writer's own non-`Write`-trait methods (`succinctly yq`'s
    /// `ColorSink::write_result_terminator`, #1709), not a `write_str`
    /// call. Sound to bypass escaping for: every terminator this crate
    /// writes (`\0`, `\n`, or nothing) is already ASCII, so routing it
    /// through this wrapper would be a no-op anyway.
    pub fn inner_mut(&mut self) -> &mut W {
        self.inner
    }
}

impl<W: Write> Write for AsciiEscapeWriter<'_, W> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let mut rest = s;
        // ASCII runs are forwarded whole; only a non-ASCII character costs a
        // decode. The scan can only stop on a *leading* byte: a continuation
        // byte is never the first `>= 0x80` byte of valid UTF-8, so the
        // `split_at` below always lands on a character boundary.
        while let Some(pos) = rest.as_bytes().iter().position(|b| !b.is_ascii()) {
            let (ascii, tail) = rest.split_at(pos);
            if !ascii.is_empty() {
                self.inner.write_str(ascii)?;
            }
            // `tail` is non-empty by construction, so this always matches;
            // handled rather than unwrapped to keep the adapter panic-free.
            let Some(c) = tail.chars().next() else { break };
            write_u_escape(self.inner, c)?;
            rest = &tail[c.len_utf8()..];
        }
        if !rest.is_empty() {
            self.inner.write_str(rest)?;
        }
        Ok(())
    }

    /// The streamers write most structural characters one at a time, so the
    /// default `write_str`-via-stack-buffer route is worth skipping.
    fn write_char(&mut self, c: char) -> core::fmt::Result {
        if c.is_ascii() {
            self.inner.write_char(c)
        } else {
            write_u_escape(self.inner, c)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jq(s: &str) -> String {
        let mut out = String::new();
        write_json_body_jq(&mut out, s).unwrap();
        out
    }

    fn jq_ascii(s: &str) -> String {
        let mut out = String::new();
        write_json_body_jq_ascii(&mut out, s).unwrap();
        out
    }

    fn yq(s: &str) -> String {
        let mut out = String::new();
        write_json_body_yq(&mut out, s).unwrap();
        out
    }

    fn yq_ascii(s: &str) -> String {
        let mut out = String::new();
        write_json_body_yq_ascii(&mut out, s).unwrap();
        out
    }

    /// Every code point the two conventions have to agree or disagree on:
    /// all of Latin-1 (C0, printable ASCII, DEL, C1, high Latin-1), a
    /// multi-byte BMP character, a line separator, and an astral one.
    fn corpus() -> Vec<char> {
        let mut cs: Vec<char> = (0u32..=0xFF).map(|c| char::from_u32(c).unwrap()).collect();
        cs.extend(['é', '\u{2028}', '😀']);
        cs
    }

    /// The whole point of the module: one predicate, two conventions, and a
    /// test that pins the delta. Asserting only that they *agree* would pass if
    /// both writers broke the same way, so this asserts the disagreement set
    /// exactly — and the next test asserts what each side renders there.
    #[test]
    fn conventions_differ_at_exactly_three_code_points() {
        let mut differ: Vec<char> = Vec::new();
        for c in corpus() {
            let s = String::from(c);
            if jq(&s) != yq(&s) {
                differ.push(c);
            }
        }
        assert_eq!(differ, vec!['\u{8}', '\u{c}', '\u{7f}']);
    }

    #[test]
    fn the_three_differences_render_as_documented() {
        assert_eq!((jq("\u{8}"), yq("\u{8}")), ("\\b".into(), "\\u0008".into()));
        assert_eq!((jq("\u{c}"), yq("\u{c}")), ("\\f".into(), "\\u000c".into()));
        assert_eq!(
            (jq("\u{7f}"), yq("\u{7f}")),
            ("\\u007f".into(), "\u{7f}".into())
        );
    }

    /// Pinned against `jq-1.7.1`:
    ///
    /// ```console
    /// $ printf '"a\\u0001b\\u007fc\\u0085d\\u0008e\\u000cf"' | jq -r tojson
    /// "abc<C2 85>d\be\ff"
    /// ```
    #[test]
    fn jq_convention_matches_the_oracle() {
        assert_eq!(jq("a"), "a");
        assert_eq!(jq("say \"hi\""), "say \\\"hi\\\"");
        assert_eq!(jq("a\\b"), "a\\\\b");
        assert_eq!(jq("\u{8}\u{c}\n\r\t"), "\\b\\f\\n\\r\\t");
        // C0 without a short form.
        assert_eq!(
            jq("\u{0}\u{1}\u{b}\u{1b}\u{1f}"),
            "\\u0000\\u0001\\u000b\\u001b\\u001f"
        );
        // DEL is escaped; C1 is not.
        assert_eq!(jq("\u{7f}"), "\\u007f");
        assert_eq!(jq("\u{80}\u{85}\u{9f}"), "\u{80}\u{85}\u{9f}");
        // U+00A0 is past C1 and stays raw, as does everything else non-ASCII.
        assert_eq!(jq("\u{a0}café😀"), "\u{a0}café😀");
    }

    /// Pinned against `jq -a`, which escapes every non-ASCII character —
    /// including C1 — and uses surrogate pairs above the BMP.
    #[test]
    fn jq_ascii_convention_matches_the_oracle() {
        assert_eq!(jq_ascii("\u{8}\u{c}\n\r\t"), "\\b\\f\\n\\r\\t");
        assert_eq!(jq_ascii("\u{7f}"), "\\u007f");
        assert_eq!(jq_ascii("\u{85}"), "\\u0085");
        assert_eq!(jq_ascii("é"), "\\u00e9");
        assert_eq!(jq_ascii("\u{2028}"), "\\u2028");
        assert_eq!(jq_ascii("😀"), "\\ud83d\\ude00");
        assert_eq!(jq_ascii("plain"), "plain");
    }

    /// yq escapes only `"`, `\` and C0; DEL and C1 stay raw and backspace /
    /// form-feed take the long form.
    #[test]
    fn yq_convention_matches_mikefarah_yq() {
        assert_eq!(yq("\u{8}\u{c}"), "\\u0008\\u000c");
        assert_eq!(yq("\t\n\r"), "\\t\\n\\r");
        assert_eq!(yq("a\"\\b"), "a\\\"\\\\b");
        assert_eq!(yq("\u{0}\u{7}\u{b}\u{1b}"), "\\u0000\\u0007\\u000b\\u001b");
        assert_eq!(yq("\u{7f}"), "\u{7f}");
        assert_eq!(yq("\u{80}\u{85}\u{9f}"), "\u{80}\u{85}\u{9f}");
        assert_eq!(yq("café"), "café");
    }

    #[test]
    fn yq_ascii_convention_escapes_non_ascii() {
        assert_eq!(yq_ascii("\u{8}\u{c}"), "\\u0008\\u000c");
        assert_eq!(yq_ascii("\u{7f}"), "\u{7f}"); // DEL is ASCII, so it stays raw
        assert_eq!(yq_ascii("\u{85}"), "\\u0085");
        assert_eq!(yq_ascii("é"), "\\u00e9");
        assert_eq!(yq_ascii("😀"), "\\ud83d\\ude00");
    }

    /// The ASCII variants may only differ from their base convention on
    /// non-ASCII input — a check that the two pairs cannot drift either.
    #[test]
    fn ascii_variants_agree_with_their_base_on_ascii_input() {
        for c in corpus().into_iter().filter(char::is_ascii) {
            let s = String::from(c);
            assert_eq!(jq(&s), jq_ascii(&s), "jq/{c:?}");
            assert_eq!(yq(&s), yq_ascii(&s), "yq/{c:?}");
        }
    }

    /// The SIMD span-copying path in [`write_json_body_yq`] has to agree with
    /// the scalar walk for spans longer than a SIMD chunk, including when an
    /// escape lands on a chunk boundary.
    #[test]
    fn yq_span_copying_agrees_with_a_scalar_walk() {
        let scalar_ref = |s: &str| {
            let mut out = String::new();
            for c in s.chars() {
                match c {
                    '"' => out.push_str("\\\""),
                    '\\' => out.push_str("\\\\"),
                    '\n' => out.push_str("\\n"),
                    '\r' => out.push_str("\\r"),
                    '\t' => out.push_str("\\t"),
                    c if (c as u32) < 0x20 => {
                        out.push_str("\\u00");
                        out.push(HEX[((c as u32) >> 4) as usize] as char);
                        out.push(HEX[((c as u32) & 0xf) as usize] as char);
                    }
                    c => out.push(c),
                }
            }
            out
        };

        for pad in 0..96usize {
            for needle in ['"', '\\', '\n', '\u{1}', '\u{7f}', 'é'] {
                let mut s = "a".repeat(pad);
                s.push(needle);
                s.push_str(&"b".repeat(pad));
                assert_eq!(yq(&s), scalar_ref(&s), "pad={pad} needle={needle:?}");
            }
        }
    }

    #[test]
    fn escape_json_body_matches_the_writer_it_is_given() {
        for c in corpus() {
            let s = String::from(c);
            assert_eq!(escape_json_body(write_json_body_jq, &s), jq(&s), "jq/{c:?}");
            assert_eq!(escape_json_body(write_json_body_yq, &s), yq(&s), "yq/{c:?}");
        }
    }

    /// Run a whole string through [`AsciiEscapeWriter`].
    fn sink(s: &str) -> String {
        let mut out = String::new();
        AsciiEscapeWriter::new(&mut out).write_str(s).unwrap();
        out
    }

    /// The design invariant behind escaping at the sink (#1700): a body
    /// written with the *base* convention and then passed through the adapter
    /// is byte-for-byte what the dedicated ASCII writer produces. This is what
    /// lets `--ascii-output` reuse the M2 streamers untouched instead of
    /// threading a flag into all three of their string-writing routes.
    #[test]
    fn adapter_over_base_convention_equals_the_ascii_convention() {
        for c in corpus() {
            let s = String::from(c);
            assert_eq!(sink(&yq(&s)), yq_ascii(&s), "yq/{c:?}");
            assert_eq!(sink(&jq(&s)), jq_ascii(&s), "jq/{c:?}");
        }
    }

    /// Structure, punctuation and existing escapes pass through untouched:
    /// JSON's grammar admits no non-ASCII outside a string literal, which is
    /// what makes escaping at the sink equivalent to escaping in the writers.
    #[test]
    fn adapter_rewrites_only_non_ascii() {
        let structural = r#"{"a":[1,-2.5,true,false,null],"b":"x\ty"}"#;
        assert_eq!(sink(structural), structural);
        assert_eq!(sink("{\"k\":\"héllo\"}"), "{\"k\":\"h\\u00e9llo\"}");
        assert_eq!(sink("😀"), "\\ud83d\\ude00");
        assert_eq!(sink("\u{2028}"), "\\u2028");
    }

    /// Already-ASCII text survives a second pass unchanged, so a sink that
    /// also carries output from the DOM path (which escapes for itself) can
    /// never be double-escaped.
    #[test]
    fn adapter_is_idempotent() {
        for c in corpus() {
            let once = sink(&yq(&String::from(c)));
            assert_eq!(sink(&once), once, "{c:?}");
        }
    }

    /// The streamers write in many small pieces, one structural character at
    /// a time included. The adapter keeps no state between calls, so every
    /// chunking of the same text must produce the same bytes -- including the
    /// `write_char` fast path, which bypasses `write_str` entirely.
    #[test]
    fn adapter_is_chunking_independent() {
        let text = "{\"k\":\"héllo 😀 \u{2028}\",\"n\":1}";
        let whole = sink(text);
        assert_ne!(whole, text, "the sample must actually exercise escaping");

        let mut piecewise = String::new();
        {
            let mut w = AsciiEscapeWriter::new(&mut piecewise);
            for c in text.chars() {
                w.write_str(&String::from(c)).unwrap();
            }
        }
        assert_eq!(piecewise, whole);

        let mut by_char = String::new();
        {
            let mut w = AsciiEscapeWriter::new(&mut by_char);
            for c in text.chars() {
                w.write_char(c).unwrap();
            }
        }
        assert_eq!(by_char, whole);
    }
}
