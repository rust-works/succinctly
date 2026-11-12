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
//! It also costs nothing on valid input. Every caller reaches this function
//! only after a whole-input `validate_utf8` (a SIMD pass, ~1.1 ms on 8.4 MB)
//! has already failed, so a well-formed document never enters the scanner
//! below. `--seq` then substitutes each value's own bytes, and only the
//! values that fail a check of their own (#3247).
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
use crate::text::utf8::{
    substitute_invalid_utf8_jq_style, substitute_invalid_utf8_jq_style_observed,
};
use core::ops::Range;

/// Lossily decode a JSON *document* as UTF-8 the way jq 1.7.1 does, scoping
/// [`substitute_invalid_utf8_jq_style`] to each JSON string rather than to
/// the whole buffer (#1743).
///
/// Bytes outside any string are substituted too, one U+FFFD for each maximal
/// ill-formed run (`String::from_utf8_lossy`): the result has to be valid UTF-8 for the rest
/// of the pipeline to treat it as text at all. Real jq substitutes nothing
/// there: a bad byte is part of a literal token, which is a parse error
/// (`printf '{"a":\xe1\x41 1}' | jq .`), and so it is in succinctly. The
/// white space after it must survive, since jq counts its newlines and a
/// read that resumes after the fault starts from them (#5994).
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
    substitute_document(raw, &mut |_| {})
}

/// One place where [`substitute_invalid_utf8_jq_document_mapped`] changed the document: `raw` is
/// the bytes of the document, `text` what they became in the result (#5975).
///
/// Between two edits the text is the document byte for byte, so an offset there moves by the
/// edits before it alone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubstitutionEdit {
    /// The bytes of the document that were replaced.
    pub raw: Range<usize>,
    /// Where those bytes are in the result.
    pub text: Range<usize>,
}

/// The offset in the document of `at` in the text the substitution made of it.
///
/// `edits` are the ones that made the text ([`substitute_invalid_utf8_jq_document_mapped`]). An offset
/// inside a replacement is where the bytes it replaced start.
pub fn original_offset(edits: &[SubstitutionEdit], at: usize) -> usize {
    let before = edits.partition_point(|edit| edit.text.end <= at);
    match edits.get(before) {
        Some(edit) if edit.text.start <= at => edit.raw.start,
        _ => match before.checked_sub(1).map(|i| &edits[i]) {
            Some(edit) => at - edit.text.end + edit.raw.end,
            None => at,
        },
    }
}

/// [`substitute_invalid_utf8_jq_document`], with every place it changed the document, in order
/// (#5975).
///
/// A run of invalid bytes outside a string, or inside one with no escape in it, is one edit for
/// the run, which is one U+FFFD in the text. The drop quirk makes the run of a string's whole
/// tail one edit as well. A string whose escapes had to be decoded to be judged (`"\xe1\u0041"`)
/// is rewritten whole, so it is one edit for the whole body: an offset inside it has no place in
/// the document, and a position the parser stops at is never inside a string it read.
///
/// # Examples
///
/// ```
/// use succinctly::jq::utf8_document::substitute_invalid_utf8_jq_document_mapped;
///
/// // jq drops the `A` with the lead byte: two bytes of the document are one U+FFFD.
/// let (text, edits) = substitute_invalid_utf8_jq_document_mapped(b"\"\xe1A\" }");
/// assert_eq!(text, "\"\u{fffd}\" }");
/// assert_eq!((edits[0].raw.clone(), edits[0].text.clone()), (1..3, 1..4));
/// ```
pub fn substitute_invalid_utf8_jq_document_mapped(raw: &[u8]) -> (String, Vec<SubstitutionEdit>) {
    let mut edits = Vec::new();
    let text = substitute_document(raw, &mut |edit| edits.push(edit));
    (text, edits)
}

fn substitute_document(raw: &[u8], on_edit: &mut impl FnMut(SubstitutionEdit)) -> String {
    let mut out = String::with_capacity(raw.len());
    // Start of the current run of bytes outside any string.
    let mut segment_start = 0;
    let mut i = 0;

    while i < raw.len() {
        if raw[i] != b'"' {
            i += 1;
            continue;
        }

        push_segment(&mut out, &raw[segment_start..i], segment_start, on_edit);
        out.push('"');

        let body_start = i + 1;
        let (body_end, has_escape) = scan_string_body(raw, body_start);
        repair_string_body(
            &mut out,
            &raw[body_start..body_end],
            body_start,
            has_escape,
            on_edit,
        );

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

    push_segment(&mut out, &raw[segment_start..], segment_start, on_edit);
    out
}

/// Append one run of bytes from *outside* any string, substituting only if
/// it is not already valid UTF-8.
///
/// The check is not an optimisation on top of a different answer -- it *is*
/// the same answer: `String::from_utf8_lossy` returns valid input
/// byte-for-byte unchanged. Skipping it
/// skips an allocation and a second scan, and that is worth having because
/// segment count scales with the document, not with the corruption in it: a
/// file with one bad byte still has one segment per JSON string, so building
/// a `String` for each is millions of short-lived allocations on a large
/// input where a borrow would do.
fn push_segment(
    out: &mut String,
    bytes: &[u8],
    base: usize,
    on_edit: &mut impl FnMut(SubstitutionEdit),
) {
    match core::str::from_utf8(bytes) {
        Ok(s) => out.push_str(s),
        // One U+FFFD per maximal ill-formed run, never the string-scoped short-tail rule: jq
        // substitutes nothing outside a string (a bad byte is part of a literal token), so a lead
        // byte must not take the white space after it, or the newline that ends the token
        // (`\xe1\n"a" 1` lost its newline, and the read that resumed after the fault lost `"a"` and
        // `1`, #5994).
        Err(_) => push_lossy(out, bytes, base, on_edit),
    }
}

/// Append `bytes` (at `base` in the document) substituted, one edit per run replaced.
fn push_substituted(
    out: &mut String,
    bytes: &[u8],
    base: usize,
    on_edit: &mut impl FnMut(SubstitutionEdit),
) {
    let text = out.len();
    let substituted = substitute_invalid_utf8_jq_style_observed(bytes, |start, end, at| {
        on_edit(SubstitutionEdit {
            raw: base + start..base + end,
            text: text + at..text + at + '\u{FFFD}'.len_utf8(),
        });
    });
    out.push_str(&substituted);
}

/// Append `bytes` (at `base` in the document) as `String::from_utf8_lossy` reads them, one edit
/// per run it replaces (#5990, #5975).
fn push_lossy(
    out: &mut String,
    bytes: &[u8],
    base: usize,
    on_edit: &mut impl FnMut(SubstitutionEdit),
) {
    let mut at = 0;
    while let Err(error) = core::str::from_utf8(&bytes[at..]) {
        let valid = error.valid_up_to();
        let invalid = error.error_len().unwrap_or(bytes.len() - at - valid);
        // The bytes up to `valid` were just checked.
        out.push_str(&String::from_utf8_lossy(&bytes[at..at + valid]));
        let text = out.len();
        out.push('\u{FFFD}');
        on_edit(SubstitutionEdit {
            raw: base + at + valid..base + at + valid + invalid,
            text: text..text + '\u{FFFD}'.len_utf8(),
        });
        at += valid + invalid;
    }
    out.push_str(&String::from_utf8_lossy(&bytes[at..]));
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
fn repair_string_body(
    out: &mut String,
    body: &[u8],
    base: usize,
    has_escape: bool,
    on_edit: &mut impl FnMut(SubstitutionEdit),
) {
    if let Ok(s) = core::str::from_utf8(body) {
        out.push_str(s);
        return;
    }

    // jq's lexer refuses a raw control character in a string before it substitutes anything, so a
    // body holding one is a parse error at that character. The drop quirk must not take it with a
    // lead byte (`"\xe1\n"` would read as a lone U+FFFD): substitute each ill-formed run on its own
    // instead, which keeps every ASCII byte where the parser will find it (#5990).
    if body.iter().any(|&b| b < 0x20) {
        push_lossy(out, body, base, on_edit);
        return;
    }

    if has_escape {
        let mut decoded = Vec::with_capacity(body.len());
        if decode_escapes_into::<false>(body, &mut decoded).is_ok() {
            let text = out.len();
            out.push_str(&escape_json_body(
                write_json_body_jq,
                &substitute_invalid_utf8_jq_style(&decoded),
            ));
            on_edit(SubstitutionEdit {
                raw: base..base + body.len(),
                text: text..out.len(),
            });
            return;
        }
    }

    push_substituted(out, body, base, on_edit);
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

    /// #5990: a raw control character keeps its place, so the parser refuses the string as jq's
    /// lexer does; the drop quirk would have folded it into the lead byte's U+FFFD.
    #[test]
    fn a_control_character_is_never_dropped_with_a_lead_byte_5990() {
        assert_eq!(
            substitute_invalid_utf8_jq_document(b"\"\xe1\n\" 1"),
            "\"\u{fffd}\n\" 1"
        );
        assert_eq!(
            substitute_invalid_utf8_jq_document(b"{\"k\xe1\t\":1}"),
            "{\"k\u{fffd}\t\":1}"
        );
        // Without one, the quirk is unchanged.
        assert_eq!(
            substitute_invalid_utf8_jq_document(b"\"\xe1A\" 1"),
            "\"\u{fffd}\" 1"
        );
    }

    /// #5994: outside a string each ill-formed run is one U+FFFD on its own, so a lead byte keeps
    /// the white space after it; jq's parser counts that newline and ends the literal token at it.
    #[test]
    fn a_lead_byte_outside_a_string_keeps_the_white_space_after_it_5994() {
        assert_eq!(
            substitute_invalid_utf8_jq_document(b"\xe1\n\"a\" 1"),
            "\u{fffd}\n\"a\" 1"
        );
        assert_eq!(
            substitute_invalid_utf8_jq_document(b"1 \xf0\x90\n\"a\""),
            "1 \u{fffd}\n\"a\""
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

    /// #5975: the mapped substitution answers the same text as the plain one, and names the bytes
    /// each replacement stands for.
    #[test]
    fn the_mapped_substitution_names_the_bytes_each_replacement_covers_5975() {
        let edit = |raw: Range<usize>, text: Range<usize>| SubstitutionEdit { raw, text };
        for (raw, text, edits) in [
            // Outside a string: one edit per run, a lone byte and a truncated sequence alike.
            (
                &b"a\xffb\xe2\x82c"[..],
                "a\u{fffd}b\u{fffd}c",
                vec![edit(1..2, 1..4), edit(3..5, 5..8)],
            ),
            // Inside a string with no escape: the drop quirk takes the `A` with the lead byte.
            (b"\"\xe1A\" 1", "\"\u{fffd}\" 1", vec![edit(1..3, 1..4)]),
            // Inside a string with room to judge the lead byte: the `A` stays.
            (b"\"\xe1Ax\"", "\"\u{fffd}Ax\"", vec![edit(1..2, 1..4)]),
            // A string rewritten whole because an escape had to be decoded: one edit for the body.
            (
                b"\"\xe1\\u0041\" 1",
                "\"\u{fffd}\" 1",
                vec![edit(1..8, 1..4)],
            ),
            // A raw control character keeps the string out of the drop quirk (#5990): every
            // invalid run is its own replacement, as `String::from_utf8_lossy` makes them.
            (
                b"\"\xe1\n\xff\"",
                "\"\u{fffd}\n\u{fffd}\"",
                vec![edit(1..2, 1..4), edit(3..4, 5..8)],
            ),
            // Valid text has no edit.
            (b"{\"a\":\"\xc3\xa9\"}", "{\"a\":\"\u{e9}\"}", vec![]),
            // Offsets after an earlier edit move with it.
            (
                b"\"\xe1A\" \"\xff\" \xff",
                "\"\u{fffd}\" \"\u{fffd}\" \u{fffd}",
                vec![edit(1..3, 1..4), edit(6..7, 7..10), edit(9..10, 12..15)],
            ),
        ] {
            let (mapped, got) = substitute_invalid_utf8_jq_document_mapped(raw);
            assert_eq!(mapped, text, "{raw:?}");
            assert_eq!(mapped, substitute_invalid_utf8_jq_document(raw), "{raw:?}");
            assert_eq!(got, edits, "{raw:?}");
        }
    }
}
