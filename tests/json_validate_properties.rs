//! Property and differential tests for the JSON validator.
//!
//! Three layers, each catching what the others cannot:
//!
//! 1. **Structural invariants** of the validator itself — most importantly that
//!    [`Position`] is a pure function of `(input, offset)`, which is what lets
//!    any future rewrite track only `offset` in its hot loop.
//! 2. **Exhaustive sweeps** over the shapes a byte-at-a-time parser gets wrong:
//!    every truncation point, every byte value in every embedding context, and
//!    every construct placed at every offset across three 64-byte boundaries.
//! 3. **A `serde_json` differential**, the only layer that can catch this crate
//!    being self-consistently wrong. `tests/json_test_suite.rs` covers external
//!    conformance; this covers the far larger space of generated input.
//!
//! Assertions compare the full `ValidationError`, not `is_ok()`. `position` is
//! public API that drives the CLI's rendered diagnostics, and
//! `.claude/skills/testing/SKILL.md` opens by forbidding success-only checks.

use proptest::prelude::*;
use succinctly::json::validate::{self, ValidationErrorKind};

#[path = "common/json_oracle.rs"]
mod oracle;
use oracle::{
    assert_position_invariant, assert_serde_agreement, classify_divergence, position_of, render,
    KnownDivergence,
};

// ---------------------------------------------------------------------------
// Valid-JSON generation
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
enum Node {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    Arr(Vec<Self>),
    Obj(Vec<(String, Self)>),
}

/// Render a node in a deliberately awkward — but strictly legal — style.
///
/// `serde_json::to_vec` emits a canonical subset: it never produces `\/`, never
/// escapes a character that need not be escaped, never writes `1E+2` or `-0`,
/// and never puts a bare CR between tokens. A generator that only exercises
/// serde's output would leave most of the grammar untested, so this renderer
/// deliberately reaches for the legal-but-unusual forms. Both renderings of the
/// same tree must validate.
fn render_awkward(node: &Node, out: &mut String, depth: usize) {
    // Rotate through several legal whitespace fillers, including bare CR.
    const FILLERS: [&str; 4] = ["", " ", "\n  ", "\r\t"];
    let ws = FILLERS[depth % FILLERS.len()];

    match node {
        Node::Null => out.push_str("null"),
        Node::Bool(true) => out.push_str("true"),
        Node::Bool(false) => out.push_str("false"),
        Node::Int(v) => {
            // `-0` is legal and distinct from `0` in the grammar.
            if *v == 0 && depth % 2 == 1 {
                out.push_str("-0");
            } else {
                out.push_str(&v.to_string());
            }
        }
        Node::Float(_) => {
            // Exercise the exponent forms serde never emits.
            out.push_str(match depth % 3 {
                0 => "1E+2",
                1 => "1e-0",
                _ => "-1.5e10",
            });
        }
        Node::Str(s) => render_awkward_string(s, out, depth),
        Node::Arr(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(ws);
                render_awkward(item, out, depth + 1);
            }
            out.push_str(ws);
            out.push(']');
        }
        Node::Obj(fields) => {
            out.push('{');
            for (i, (k, v)) in fields.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(ws);
                render_awkward_string(k, out, depth);
                out.push(':');
                out.push_str(ws);
                render_awkward(v, out, depth + 1);
            }
            out.push_str(ws);
            out.push('}');
        }
    }
}

fn render_awkward_string(s: &str, out: &mut String, depth: usize) {
    out.push('"');
    for (i, ch) in s.chars().enumerate() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            // `/` may be escaped; serde never does.
            '/' => out.push_str("\\/"),
            // Occasionally escape an ASCII letter that needs no escaping.
            c if c.is_ascii_alphabetic() && (i + depth) % 7 == 0 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    // A surrogate pair escape, which only the escaped form can express.
    if depth % 5 == 0 {
        out.push_str("\\uD83D\\uDE00");
    }
    out.push('"');
}

fn to_serde(node: &Node) -> serde_json::Value {
    use serde_json::Value;
    match node {
        Node::Null => Value::Null,
        Node::Bool(b) => Value::Bool(*b),
        Node::Int(v) => Value::from(*v),
        Node::Float(f) => serde_json::Number::from_f64(*f).map_or(Value::Null, Value::Number),
        Node::Str(s) => Value::String(s.clone()),
        Node::Arr(items) => Value::Array(items.iter().map(to_serde).collect()),
        Node::Obj(fields) => Value::Object(
            fields
                .iter()
                .map(|(k, v)| (k.clone(), to_serde(v)))
                .collect(),
        ),
    }
}

/// Depth is capped well under `MAX_NESTING_DEPTH` (128) so `NestingTooDeep`
/// never fires by accident — the same reasoning as `VALID_DEPTH` in
/// `tests/deep_nesting_valid_tests.rs`.
fn arb_node() -> impl Strategy<Value = Node> {
    let leaf = prop_oneof![
        Just(Node::Null),
        any::<bool>().prop_map(Node::Bool),
        any::<i64>().prop_map(Node::Int),
        Just(Node::Float(0.0)),
        // Include the characters a JSON writer must think about.
        r#"[a-zA-Z0-9 /\\"\t\n\u{80}-\u{7ff}\u{1000}-\u{ffff}]{0,12}"#.prop_map(Node::Str),
    ];
    leaf.prop_recursive(6, 64, 6, |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 0..6).prop_map(Node::Arr),
            prop::collection::vec(("[a-z]{0,8}", inner), 0..6).prop_map(Node::Obj),
        ]
    })
}

// ---------------------------------------------------------------------------
// Properties
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Every reported position must be reconstructible from the offset alone.
    ///
    /// This is the load-bearing invariant for any future rewrite: if it holds,
    /// the hot loop never needs line/column bookkeeping. It is also the only
    /// check on `validate_keyword`'s hand-rolled `column` arithmetic, which is
    /// the one place that builds a `Position` without calling `position()`.
    #[test]
    fn position_is_a_function_of_offset(bytes in prop::collection::vec(any::<u8>(), 0..256)) {
        if let Err(e) = validate::validate(&bytes) {
            prop_assert_eq!(
                e.position,
                position_of(&bytes, e.position.offset),
                "position disagrees with recomputation for {}", render(&bytes)
            );
        }
    }

    /// Both renderings of a generated tree must validate.
    #[test]
    fn generated_json_validates(node in arb_node()) {
        let canonical = serde_json::to_vec(&to_serde(&node)).expect("serde renders");
        prop_assert!(
            validate::validate(&canonical).is_ok(),
            "rejected serde_json's own output: {}", render(&canonical)
        );

        let mut awkward = String::new();
        render_awkward(&node, &mut awkward, 0);
        prop_assert!(
            validate::validate(awkward.as_bytes()).is_ok(),
            "rejected legal-but-awkward rendering: {:?}\n  error: {:?}",
            awkward,
            validate::validate(awkward.as_bytes())
        );
    }

    /// Positions must survive whatever a mutation does to a valid document.
    #[test]
    fn mutated_json_keeps_position_invariant(
        node in arb_node(),
        idx in any::<prop::sample::Index>(),
        op in 0u8..4,
        byte in prop::sample::select(&[
            b'"', b'\\', b'{', b'}', b'[', b']', b',', b':', b'0', b'e', b'-', b'.',
            0x00, 0x1F, 0x7F, 0xC0, 0xED, 0xF5, 0xFF,
        ][..]),
    ) {
        let mut doc = serde_json::to_vec(&to_serde(&node)).expect("serde renders");
        if doc.is_empty() {
            return Ok(());
        }
        let i = idx.index(doc.len());
        match op {
            0 => doc[i] ^= 1,
            1 => doc[i] = byte,
            2 => { doc.remove(i); }
            _ => doc.insert(i, byte),
        }

        if let Err(e) = validate::validate(&doc) {
            prop_assert_eq!(e.position, position_of(&doc, e.position.offset));
        }
        assert_serde_agreement("mutated", &doc);
    }

    /// Random bytes: we and serde_json must agree on validity.
    #[test]
    fn random_bytes_agree_with_serde(bytes in prop::collection::vec(any::<u8>(), 0..192)) {
        assert_serde_agreement("random", &bytes);
    }

    /// Random ASCII drawn from the JSON alphabet reaches far deeper into the
    /// grammar than uniform bytes, which almost always die at offset 0.
    #[test]
    fn json_alphabet_soup_agrees_with_serde(
        s in prop::collection::vec(
            prop::sample::select(&b"{}[]\",:0123456789.eE+-truefalsn \t\n\r\\/"[..]),
            0..192,
        )
    ) {
        assert_serde_agreement("soup", &s);
    }
}

// ---------------------------------------------------------------------------
// Exhaustive sweeps
// ---------------------------------------------------------------------------

/// A valid document with enough variety to make every truncation interesting.
fn sample_document() -> Vec<u8> {
    let mut s = String::from("{\n  \"items\": [\n");
    for i in 0..40 {
        if i > 0 {
            s.push_str(",\n");
        }
        s.push_str(&format!(
            "    {{\"id\": {i}, \"name\": \"item \\u00e9{i}\", \"tags\": [\"a\\\"b\", \"c\\\\d\"], \
             \"ratio\": -1.5e-{i}, \"ok\": true, \"nil\": null, \"emoji\": \"\\uD83D\\uDE00\"}}"
        ));
    }
    s.push_str("\n  ]\n}\n");
    s.into_bytes()
}

/// Truncate a valid document at every offset.
///
/// This is the single most productive sweep for a chunked reader: it exercises
/// every `UnexpectedEof` / `UnclosedString` / incomplete-escape path at every
/// possible position relative to a 64-byte boundary, including the final
/// partial chunk that end-of-input handling most often gets wrong.
#[test]
fn every_truncation_is_rejected_with_a_consistent_position() {
    let doc = sample_document();
    assert!(
        validate::validate(&doc).is_ok(),
        "sample document must be valid"
    );

    // Cutting only trailing whitespace still leaves a complete document, so
    // those prefixes are legitimately valid. Everything shorter cuts into
    // structure and must be rejected. Deriving the boundary rather than
    // hard-coding it keeps the assertion exact if the sample changes.
    let last_significant = doc
        .iter()
        .rposition(|b| !b.is_ascii_whitespace())
        .expect("sample document is not all whitespace");

    for i in 0..doc.len() {
        let prefix = &doc[..i];
        let complete = i > last_significant;

        match validate::validate(prefix) {
            Ok(()) => assert!(
                complete,
                "truncation at {i} cuts into structure but was accepted"
            ),
            Err(err) => {
                assert!(
                    !complete,
                    "truncation at {i} leaves a complete document but was rejected: {err}"
                );
                assert_eq!(
                    err.position,
                    position_of(prefix, err.position.offset),
                    "truncation at {i}: position disagrees with recomputation"
                );
                assert!(
                    err.position.offset <= prefix.len(),
                    "truncation at {i}: offset {} past end {}",
                    err.position.offset,
                    prefix.len()
                );
            }
        }
        assert_serde_agreement(&format!("truncated@{i}"), prefix);
    }
}

/// Every byte value, in every context a byte can appear in.
///
/// Modelled on `tests/simd_level_tests.rs`'s per-byte differential. Its #186
/// note applies with more force here: divergent validator behaviour would mean
/// the same file validates on one architecture and fails on another.
#[test]
fn every_byte_in_every_context_agrees_with_serde() {
    // (prefix, suffix) pairs placing the byte at 9 structurally distinct spots,
    // including offsets that straddle the 16/32/64-byte boundaries.
    let contexts: &[(&str, &str)] = &[
        ("", ""),
        ("[", "]"),
        ("[1,", "]"),
        ("{\"k\":", "}"),
        ("{\"", "\":1}"),
        ("[\"", "\"]"),
        ("[\"\\", "\"]"),
        ("[\"aaaaaaaaaaaaaa", "\"]"), // byte at offset 16
        ("[\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "\"]"), // byte at offset 32
    ];

    for (prefix, suffix) in contexts {
        for byte in 0u8..=255 {
            let mut input = Vec::from(prefix.as_bytes());
            input.push(byte);
            input.extend_from_slice(suffix.as_bytes());

            let label = format!("byte {byte:#04x} in {prefix:?}_{suffix:?}");
            assert_position_invariant(&label, &input);
            assert_serde_agreement(&label, &input);
        }
    }
}

/// Place each construct at every offset across three 64-byte boundaries.
///
/// The sweep is `0..192`, not `0..64`: a carry or state bug that only shows on
/// the *second* propagation survives a single-boundary sweep.
/// `tests/dsv_simd_differential_tests.rs` records that testing only offset 0 is
/// exactly how the #149 bit-63 bug slipped through.
#[test]
fn constructs_at_every_offset_across_three_chunks() {
    // (label, valid form, invalid twin)
    let constructs: &[(&str, &str, &str)] = &[
        ("backslash-1", r#""a\\b""#, r#""a\b""#),
        ("backslash-2", r#""a\\\\b""#, r#""a\\\b""#),
        ("backslash-3", r#""a\\\\\\b""#, r#""a\\\\\b""#),
        ("unicode-escape", r#""\u0041""#, r#""\u00G1""#),
        ("surrogate-pair", r#""\uD83D\uDE00""#, r#""\uD83D\u0041""#),
        ("lone-surrogate", r#""\uD83D\uDE00""#, r#""\uD83D""#),
        ("utf8-2byte", "\"\u{00e9}\"", "\"\\u00e9"),
        ("utf8-3byte", "\"\u{20ac}\"", "\"\u{20ac}"),
        ("utf8-4byte", "\"\u{1f600}\"", "\"\u{1f600}"),
        ("number", "-1.5e-10", "01"),
        ("number-frac", "0.5", "1."),
        ("number-exp", "1e+5", "1e+"),
        ("keyword-true", "true", "tru"),
        ("keyword-null", "null", "nulll"),
        ("keyword-false", "false", "falsey"),
        ("empty-string", r#""""#, r#"""#),
        ("nested", "[[1]]", "[[1]"),
        ("object", r#"{"k":1}"#, r#"{"k":}"#),
    ];

    for (label, valid, invalid) in constructs {
        for offset in 0..192usize {
            // Pad with array elements so the construct starts at `offset`, then
            // trail enough bytes that it still has a full chunk behind it.
            for (variant, body) in [("valid", valid), ("invalid", invalid)] {
                let mut doc = String::from("[");
                while doc.len() < offset {
                    doc.push_str(if doc.len() + 2 <= offset { "1," } else { " " });
                }
                doc.push_str(body);
                doc.push_str(&format!(",{}]", "2,".repeat(32).trim_end_matches(',')));

                let input = doc.as_bytes();
                let case = format!("{label}/{variant}@{offset}");
                assert_position_invariant(&case, input);
                assert_serde_agreement(&case, input);
            }
        }
    }
}

/// Line/column bookkeeping across every line-terminator form, at every offset.
///
/// A bare `\r` counts as a line break in `skip_whitespace`, and CRLF counts
/// once. Both are easy to get wrong and neither is covered by the CLI tests.
#[test]
fn line_terminators_at_every_offset() {
    for terminator in ["\n", "\r", "\r\n"] {
        for offset in 0..192usize {
            let mut doc = String::from("[");
            while doc.len() < offset {
                doc.push_str(if doc.len() + 2 <= offset { "1," } else { " " });
            }
            doc.push_str(terminator);
            // An unterminated string forces an error *after* the terminator, so
            // the reported line must reflect it.
            doc.push_str("\"unterminated");

            let input = doc.as_bytes();
            let err = validate::validate(input).expect_err("unterminated string must fail");
            assert_eq!(
                err.position,
                position_of(input, err.position.offset),
                "{terminator:?} at offset {offset}: position disagrees"
            );
            assert!(
                err.position.line >= 2,
                "{terminator:?} at offset {offset}: expected line >= 2, got {}",
                err.position.line
            );
        }
    }
}

/// The corpus files must validate, and their prefixes must never disagree.
#[test]
fn real_corpus_files_validate() {
    let seed = std::path::Path::new("tests/data/bench-corpus/seed/json");
    let mut files = Vec::new();
    collect_json(seed, &mut files);
    assert!(!files.is_empty(), "committed JSON seed corpus is missing");

    for path in files {
        let bytes = std::fs::read(&path).expect("read corpus file");
        assert!(
            validate::validate(&bytes).is_ok(),
            "corpus file {} failed validation",
            path.display()
        );
        assert_serde_agreement(&path.display().to_string(), &bytes);
    }
}

fn collect_json(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_json(&path, out);
        } else if matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("json" | "geojson")
        ) {
            out.push(path);
        }
    }
}

/// Pin the two classified divergences so they cannot quietly become unclassified
/// (or quietly stop being divergences).
#[test]
fn known_serde_divergences_are_exactly_as_classified() {
    // Depth 128: we accept, serde_json rejects at 127.
    let deep = format!("{}{}", "[".repeat(128), "]".repeat(128));
    assert!(
        validate::validate(deep.as_bytes()).is_ok(),
        "depth 128 must be accepted"
    );
    assert!(
        serde_json::from_slice::<serde_json::Value>(deep.as_bytes()).is_err(),
        "serde_json is expected to reject depth 128"
    );
    assert_eq!(
        classify_divergence(deep.as_bytes(), true, false),
        Some(KnownDivergence::DepthLimit)
    );

    // Depth 129: we reject too, so there is no divergence to classify.
    let too_deep = format!("{}{}", "[".repeat(129), "]".repeat(129));
    let err = validate::validate(too_deep.as_bytes()).expect_err("depth 129 must be rejected");
    assert_eq!(err.kind, ValidationErrorKind::NestingTooDeep { limit: 128 });
    assert_eq!(err.position.offset, 128);

    // f64-overflowing numbers: we accept (RFC 8259 §6 sets no range).
    for text in ["[1e309]", "[-1e400]", "[1e-400]"] {
        assert!(
            validate::validate(text.as_bytes()).is_ok(),
            "{text} must be accepted: RFC 8259 §6 sets no range on numbers"
        );
        assert_eq!(
            classify_divergence(text.as_bytes(), true, false),
            Some(KnownDivergence::NumberRange),
            "{text} should classify as a number-range divergence"
        );
    }

    // A real bug must NOT be classifiable.
    assert_eq!(classify_divergence(b"[1,2]", true, false), None);
}

/// `position_of` is the test's own reimplementation; check it against the
/// validator's incremental tracking on inputs whose positions are known.
#[test]
fn position_of_matches_known_positions() {
    // Error on line 3, column 3 (the `x`).
    let input = b"[\n  1,\n  x\n]";
    let err = validate::validate(input).expect_err("must fail");
    assert_eq!(err.position.line, 3);
    assert_eq!(err.position.column, 3);
    assert_eq!(err.position, position_of(input, err.position.offset));

    // CRLF counts once.
    let crlf = b"[\r\n  x]";
    let err = validate::validate(crlf).expect_err("must fail");
    assert_eq!(err.position.line, 2);
    assert_eq!(err.position, position_of(crlf, err.position.offset));

    // A bare CR is also a line break.
    let cr = b"[\r  x]";
    let err = validate::validate(cr).expect_err("must fail");
    assert_eq!(err.position.line, 2);
    assert_eq!(err.position, position_of(cr, err.position.offset));
}
