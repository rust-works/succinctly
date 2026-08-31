//! jq's own *timing* for U+FFFD substitution in a non-UTF-8 JSON document
//! (#1743).
//!
//! [`crate::text::utf8::substitute_invalid_utf8_jq_style`] reproduces jq
//! 1.7.1's substitution *algorithm*, including the end-of-buffer drop quirk
//! #1717 found: when a multi-byte lead byte cannot be completed before its
//! buffer ends (`len - pos < seq_len`), jq collapses the entire remaining
//! tail into one U+FFFD, dropping every byte in it.
//!
//! That algorithm is granularity-independent -- it only ever asks how many
//! bytes remain in the slice it was handed -- so *what* you hand it decides
//! whether the quirk fires where jq fires it. Real jq substitutes inside
//! `jv_string_sized`, which its lexer calls once per JSON string with that
//! string's own decoded bytes. Handing the function a whole file instead
//! makes "the buffer's own end" the file's last byte, which in a realistic
//! multi-field document essentially never coincides with jq's actual
//! trigger point:
//!
//! ```text
//! $ printf '{"a":"\xe1\x41"}' | jq -c '.a'      # jq scopes to the string: "\u{fffd}"
//! $ printf '{"a":"\xe1\x41"}' | sjq -c '.a'     # whole-file scope, pre-#1743: "\u{fffd}A"
//! ```
//!
//! [`substitute_invalid_utf8_jq_document`] closes that by segmenting the
//! document into JSON strings and everything else, and scoping the
//! substitution to each string.
//!
//! # Why this stays a buffer rewrite
//!
//! The obvious alternative -- defer substitution to string materialisation,
//! where jq does it -- would cost the invariant
//! [docs/plan/decode-failure-routing.md] was built around: *after the input
//! boundary's pass, `as_str()` can only fail on an escape problem*. That is
//! what lets `StandardJson::as_str` borrow out of the document, lets the
//! printer echo a raw string span verbatim, and lets the raw-identity fast
//! path exist. Keeping the rewrite preserves all of it; only *how* the
//! replacement buffer is built changes.
//!
//! It also costs nothing on valid input. Both callers reach this function
//! only after a whole-input `validate_utf8` (a SIMD pass, ~1.1 ms on 8.4 MB)
//! has already failed, so a well-formed document never enters the scanner
//! below.
//!
//! # Why a byte scanner is sound here
//!
//! Finding string boundaries in text that is *not* valid UTF-8 sounds
//! fragile and is not: UTF-8 is self-synchronising, so `"` (0x22) and `\`
//! (0x5C) can never occur inside a multi-byte sequence, valid or otherwise.
//! A byte-level quote/backslash scan therefore agrees with jq's own
//! byte-oriented lexer by construction, whatever garbage the document
//! carries. No semi-index, and no JSON structure beyond string boundaries,
//! is needed -- so nothing here interacts with the parse-lazily design.

use alloc::string::String;
use alloc::vec::Vec;

use crate::jq::escape::{escape_json_body, write_json_body_jq};
use crate::json::light::decode_escapes_into;
use crate::text::utf8::substitute_invalid_utf8_jq_style;

/// Lossily decode a JSON *document* as UTF-8 the way jq 1.7.1 does, scoping
/// [`substitute_invalid_utf8_jq_style`] to each JSON string rather than to
/// the whole buffer (#1743).
///
/// Bytes outside any string are substituted too, per contiguous segment:
/// the result has to be valid UTF-8 for the rest of the pipeline to treat
/// it as text at all. Real jq rejects a document with a bad byte outside a
/// string (`printf '{"a":\xe1\x41 1}' | jq .` is a parse error), and so
/// does succinctly, so the exact replacement chosen there is only ever
/// visible in an error message.
///
/// Only for JSON-shaped input. `--raw-input` is line-scoped (#1742, handled
/// by its own caller), `--raw-input --slurp` is genuinely whole-buffer in
/// real jq, and DSV input is not JSON at all -- see `jq_runner`'s call
/// sites for the gate.
///
/// # Examples
///
/// ```
/// use succinctly::jq::utf8_document::substitute_invalid_utf8_jq_document;
///
/// // Scoped to the string, so the trailing 'A' is dropped -- as in real jq --
/// // even though the document continues past it.
/// assert_eq!(
///     substitute_invalid_utf8_jq_document(b"{\"a\":\"\xe1\x41\",\"b\":1}"),
///     "{\"a\":\"\u{fffd}\",\"b\":1}",
/// );
///
/// // A string with enough bytes left to judge the lead byte on its own
/// // merits keeps the rescanned byte, exactly as before.
/// assert_eq!(
///     substitute_invalid_utf8_jq_document(b"{\"a\":\"\xe1\x41x\"}"),
///     "{\"a\":\"\u{fffd}Ax\"}",
/// );
/// ```
pub fn substitute_invalid_utf8_jq_document(raw: &[u8]) -> String {
    let mut out = String::with_capacity(raw.len());
    // Start of the current run of bytes outside any string.
    let mut segment_start = 0;
    let mut i = 0;

    while i < raw.len() {
        if raw[i] != b'"' {
            i += 1;
            continue;
        }

        push_segment(&mut out, &raw[segment_start..i]);
        out.push('"');

        let body_start = i + 1;
        let (body_end, has_escape) = scan_string_body(raw, body_start);
        repair_string_body(&mut out, &raw[body_start..body_end], has_escape);

        if body_end < raw.len() {
            // The closing quote, consumed here so the scan resumes outside
            // the string rather than re-entering it on the same byte.
            out.push('"');
            i = body_end + 1;
        } else {
            // Unterminated string: the document is a parse error either way,
            // and there is no closing quote to emit.
            i = raw.len();
        }
        segment_start = i;
    }

    push_segment(&mut out, &raw[segment_start..]);
    out
}

/// Append one run of bytes from *outside* any string, substituting only if
/// it is not already valid UTF-8.
///
/// The check is not an optimisation on top of a different answer -- it *is*
/// the same answer: [`substitute_invalid_utf8_jq_style`] returns valid input
/// byte-for-byte unchanged, pinned by #1247's own guard test. Skipping it
/// skips an allocation and a second scan, and that is worth having because
/// segment count scales with the document, not with the corruption in it: a
/// file with one bad byte still has one segment per JSON string, so building
/// a `String` for each is millions of short-lived allocations on a large
/// input where a borrow would do.
fn push_segment(out: &mut String, bytes: &[u8]) {
    match core::str::from_utf8(bytes) {
        Ok(s) => out.push_str(s),
        Err(_) => out.push_str(&substitute_invalid_utf8_jq_style(bytes)),
    }
}

/// The offset of the string's closing quote (or `raw.len()` if it is
/// unterminated) and whether the body contains a backslash escape, in one
/// scan -- the quote scan has to recognise every backslash anyway in order
/// to skip what it escapes, so reporting it is free.
fn scan_string_body(raw: &[u8], body_start: usize) -> (usize, bool) {
    let mut i = body_start;
    let mut has_escape = false;
    while i < raw.len() {
        match raw[i] {
            b'"' => return (i, has_escape),
            b'\\' => {
                has_escape = true;
                i += 2;
            }
            _ => i += 1,
        }
    }
    (raw.len(), has_escape)
}

/// Rewrite one JSON string's body (between the quotes, escapes still in
/// their source spelling) so that decoding it yields what real jq's
/// `jv_string_sized` would have produced for the same string.
///
/// Three cases, in decreasing order of how often they run:
///
/// 1. **Already valid UTF-8** -- copied verbatim. Most strings in a document
///    with one bad byte are fine, and copying keeps their escape spelling
///    untouched.
/// 2. **Invalid, no escapes** -- substituted directly. Raw body and decoded
///    string are the same bytes here, so this is exact, and it needs no
///    re-escaping: the body provably contains no `"` (the scan stopped at
///    one) and no `\` (that is what `has_escape` being false means), and
///    substitution only ever replaces non-ASCII bytes with U+FFFD, so it
///    cannot introduce either.
/// 3. **Invalid, with escapes** -- decoded to bytes, substituted, re-escaped.
///    Required for exactness because jq's rule is scoped to the *decoded*
///    string, and escapes only ever shrink it: `"\xe1\u0041"` decodes to two
///    bytes, one short of the three the `0xE1` lead declares, so real jq
///    collapses it to a bare `"\u{fffd}"` where the seven-byte raw span
///    would not have. Re-escaping via [`write_json_body_jq`] normalises the
///    spelling of any *other* escape in that string (`\u0041` becomes `A`),
///    which is what succinctly's own printer does to every string on output
///    anyway.
///
/// A body whose escapes cannot be decoded at all (a bad escape character, a
/// lone surrogate) falls back to case 2's raw-span substitution rather than
/// inventing a decoding: such a document is a parse error downstream, and
/// this keeps the bytes that error is reported against unchanged. That
/// fallback is the one route into case 2 whose body *does* carry a `\`, so
/// case 2's own proof does not cover it -- but the string still cannot be
/// broken open: substitution never emits a `"`, and the byte after a
/// surviving `\` is either a U+FFFD or a byte copied from a body the scan
/// already proved holds no unescaped quote. The result is an invalid escape
/// either way, which is exactly the error the caller wants reported.
fn repair_string_body(out: &mut String, body: &[u8], has_escape: bool) {
    if let Ok(s) = core::str::from_utf8(body) {
        out.push_str(s);
        return;
    }

    if has_escape {
        let mut decoded = Vec::with_capacity(body.len());
        if decode_escapes_into::<false>(body, &mut decoded).is_ok() {
            out.push_str(&escape_json_body(
                write_json_body_jq,
                &substitute_invalid_utf8_jq_style(&decoded),
            ));
            return;
        }
    }

    out.push_str(&substitute_invalid_utf8_jq_style(body));
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;

    /// The gap #1743 names: jq scopes the #1717 drop to the string, not the
    /// file, so content after the string does not rescue the dropped byte.
    #[test]
    fn drop_quirk_is_scoped_to_the_string_not_the_document_1743() {
        assert_eq!(
            substitute_invalid_utf8_jq_document(b"{\"a\":\"\xe1\x41\"}"),
            "{\"a\":\"\u{fffd}\"}"
        );
        assert_eq!(
            substitute_invalid_utf8_jq_document(b"{\"a\":\"\xe1\x41\",\"b\":[1,2,3],\"c\":\"x\"}"),
            "{\"a\":\"\u{fffd}\",\"b\":[1,2,3],\"c\":\"x\"}"
        );
    }

    /// Every string is judged on its own bytes, so two identically-corrupt
    /// strings collapse identically regardless of position in the document.
    #[test]
    fn each_string_is_scoped_independently_1743() {
        assert_eq!(
            substitute_invalid_utf8_jq_document(b"{\"a\":\"\xe1\x41\",\"b\":\"\xf0\x90\x41\"}"),
            "{\"a\":\"\u{fffd}\",\"b\":\"\u{fffd}\"}"
        );
    }

    /// Object keys go through the same lexer path in jq, so they collapse too.
    #[test]
    fn object_keys_are_scoped_like_values_1743() {
        assert_eq!(
            substitute_invalid_utf8_jq_document(b"{\"\xe1\x41\":1,\"b\":2}"),
            "{\"\u{fffd}\":1,\"b\":2}"
        );
    }

    /// With `seq_len` bytes actually present inside the string, jq falls back
    /// to WHATWG-style rescan-at-the-bad-byte -- the rescanned byte is kept.
    /// This is the half of the rule the document-scoped version already got
    /// right, and must not regress into an over-eager collapse.
    #[test]
    fn headroom_inside_the_string_still_keeps_the_rescanned_byte_1743() {
        assert_eq!(
            substitute_invalid_utf8_jq_document(b"{\"a\":\"\xe1\x41x\"}"),
            "{\"a\":\"\u{fffd}Ax\"}"
        );
        assert_eq!(
            substitute_invalid_utf8_jq_document(b"{\"a\":\"X\xe1\x41Y\"}"),
            "{\"a\":\"X\u{fffd}AY\"}"
        );
    }

    /// jq substitutes over the *decoded* string, so an escape -- which is
    /// always shorter decoded than spelled -- can push a lead byte over the
    /// `len - pos < seq_len` line that its raw span would clear.
    /// `"\xe1\u0041"` is 7 raw bytes but 2 decoded, one short of the three
    /// the `0xE1` lead declares.
    #[test]
    fn substitution_is_scoped_to_the_decoded_string_not_the_raw_span_1743() {
        assert_eq!(
            substitute_invalid_utf8_jq_document(b"{\"a\":\"\xe1\\u0041\"}"),
            "{\"a\":\"\u{fffd}\"}"
        );
        assert_eq!(
            substitute_invalid_utf8_jq_document(b"{\"a\":\"\xf0\x90\\u0041\"}"),
            "{\"a\":\"\u{fffd}\"}"
        );
        // Same shape via a non-`\u` escape: `\n` is two bytes spelled, one
        // decoded.
        assert_eq!(
            substitute_invalid_utf8_jq_document(b"{\"a\":\"\xe1\\n\"}"),
            "{\"a\":\"\u{fffd}\"}"
        );
    }

    /// Re-escaping a repaired string must keep it readable as the same JSON
    /// string -- a surviving quote/backslash has to come back out escaped.
    #[test]
    fn repaired_string_with_escapes_is_re_escaped_1743() {
        // `\"` survives the substitution and must not terminate the string.
        assert_eq!(
            substitute_invalid_utf8_jq_document(b"{\"a\":\"x\\\"y\xe1\x41z\"}"),
            "{\"a\":\"x\\\"y\u{fffd}Az\"}"
        );
        assert_eq!(
            substitute_invalid_utf8_jq_document(b"{\"a\":\"x\\\\y\xe1\x41z\"}"),
            "{\"a\":\"x\\\\y\u{fffd}Az\"}"
        );
    }

    /// A body whose escapes cannot be decoded keeps today's raw-span
    /// substitution: the document is a parse error either way, and inventing
    /// a decoding would change the bytes that error names. A lone surrogate
    /// is the live example -- succinctly currently echoes it rather than
    /// rejecting it as jq does, and that separate divergence must not shift
    /// here.
    #[test]
    fn undecodable_escapes_fall_back_to_raw_span_substitution_1743() {
        assert_eq!(
            substitute_invalid_utf8_jq_document(b"{\"a\":\"\\ud800\xe1\x41\"}"),
            "{\"a\":\"\\ud800\u{fffd}\"}"
        );
        assert_eq!(
            substitute_invalid_utf8_jq_document(b"{\"a\":\"\\q\xe1\x41\"}"),
            "{\"a\":\"\\q\u{fffd}\"}"
        );
    }

    /// Strings that are already valid UTF-8 are copied through untouched,
    /// escape spelling included -- only the corrupt one is rewritten.
    #[test]
    fn valid_strings_keep_their_escape_spelling_1743() {
        assert_eq!(
            substitute_invalid_utf8_jq_document(b"{\"a\":\"\\u0041\\/\",\"b\":\"\xe1\x41\"}"),
            "{\"a\":\"\\u0041\\/\",\"b\":\"\u{fffd}\"}"
        );
    }

    /// The already-agreeing shapes from #1617 are unchanged by re-scoping:
    /// a never-valid lead stays one U+FFFD per byte, a structurally valid
    /// overlong collapses to one for the whole sequence.
    #[test]
    fn existing_1617_shapes_are_unchanged_1743() {
        assert_eq!(
            substitute_invalid_utf8_jq_document(b"{\"a\":\"\xff\xfe\"}"),
            "{\"a\":\"\u{fffd}\u{fffd}\"}"
        );
        assert_eq!(
            substitute_invalid_utf8_jq_document(b"{\"a\":\"\xe0\x80\x80\"}"),
            "{\"a\":\"\u{fffd}\"}"
        );
    }

    /// Bytes outside any string are still substituted -- the result has to
    /// be valid UTF-8 for the rest of the pipeline. Both tools reject such a
    /// document, so only the error message ever sees this.
    #[test]
    fn bytes_outside_a_string_are_still_substituted_1743() {
        let out = substitute_invalid_utf8_jq_document(b"{\"a\":\xff 1}");
        assert!(out.contains('\u{fffd}'), "{out}");
        assert!(!out.contains('\u{0}'), "{out}");
    }

    /// A valid document is returned byte-identical: the scanner must be a
    /// pure pass-through when there is nothing to repair. (Callers gate on
    /// `validate_utf8` first, so this is a safety net, not the hot path.)
    #[test]
    fn valid_document_round_trips_unchanged_1743() {
        for doc in [
            &br#"{"a":"hi","b":[1,2,{"c":null}],"d":"\u00e9\t\\"}"#[..],
            &br#"["\"quoted\"","tab\there"]"#[..],
            &b"{}"[..],
            &b""[..],
            "\u{65e5}\u{672c}\u{8a9e}".as_bytes(),
        ] {
            assert_eq!(
                substitute_invalid_utf8_jq_document(doc),
                String::from_utf8(doc.to_vec()).unwrap(),
                "{}",
                String::from_utf8_lossy(doc)
            );
        }
    }

    /// An unterminated string must not lose its content or emit a closing
    /// quote it never saw.
    #[test]
    fn unterminated_string_is_repaired_without_inventing_a_quote_1743() {
        assert_eq!(
            substitute_invalid_utf8_jq_document(b"{\"a\":\"\xe1\x41"),
            "{\"a\":\"\u{fffd}".to_string()
        );
    }
}
