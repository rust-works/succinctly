//! The content of a `---` line parses as the same content on a bare line (#407).
//!
//! A `---` marker introduces a document; it does not change the grammar of what
//! follows it on the line. So for any block-context node, `--- X` and a bare `X`
//! must produce the same document — that equivalence is the property this file
//! tests.
//!
//! # Why a differential rather than fixtures
//!
//! The parser had two dispatchers for block context: one for an ordinary
//! document line, and a hand-rolled partial copy for the content of a `---`
//! line. They had drifted apart in six shapes, and nothing pointed at it,
//! because every test asserted one side or the other and never both.
//!
//! The worst of the six was an anchor alone on the `---` line. The copy opened
//! an empty node for the anchor to name, and a node at document root *is* a
//! document, so `--- &x\na: 1` yielded **two** documents — `null` and
//! `{"a":1}` — with the mapping the anchor should have named left unanchored.
//! [#372](https://github.com/rust-works/succinctly/issues/372) contained the
//! fallout by leaving the anchor unrecorded, so a later `*x` at least errored
//! rather than silently resolving to that placeholder; #407 removed the copy.
//!
//! The two dispatchers are now one ([`parse_block_node`]), so these cases pass
//! by construction. They stay because that is exactly the condition a future
//! `---`-only special case would quietly undo.
//!
//! [`parse_block_node`]: https://github.com/rust-works/succinctly/blob/main/src/yaml/parser.rs

use serde_json::Value;
use succinctly::yaml::{YamlIndex, YamlValue};

/// Parse path — identical to `tests/yaml_test_suite.rs` and
/// `tests/yaml_crlf_tests.rs`, for the same reason: it is the path
/// `succinctly yq -o json '.'` actually takes.
fn documents(yaml: &str) -> Result<Vec<Value>, String> {
    let bytes = yaml.as_bytes();
    let index = YamlIndex::build(bytes).map_err(|e| e.to_string())?;
    let root = index.root(bytes);

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

    docs.iter()
        .map(|d| serde_json::from_str(d).map_err(|e| format!("{e} in {d:?}")))
        .collect()
}

fn expected(json: &[&str]) -> Vec<Value> {
    json.iter()
        .map(|d| serde_json::from_str(d).expect("expectation is valid JSON"))
        .collect()
}

/// `(content, the documents it must produce)`.
///
/// The first six rows are the shapes that diverged; the rest are the controls
/// that already agreed, kept so a regression has to show up as a *change* in
/// which rows fail rather than as a table that only covers the known bugs.
const CASES: &[(&str, &[&str])] = &[
    // An anchor whose node is on the following line. The node — not an empty
    // placeholder, and not a second document — is what the anchor names.
    ("&x\na: 1\n", &[r#"{"a":1}"#]),
    // Suite case FTA2, "Single block sequence with anchor and explicit
    // document start".
    ("&x\n- a\n", &[r#"["a"]"#]),
    // Indented, so the placeholder used to swallow the `a` as a plain-scalar
    // continuation and leave `: 1` behind as a second document.
    ("&x\n  a: 1\n", &[r#"{"a":1}"#]),
    // The `---` copy reached `looks_like_mapping_entry` before skipping the
    // anchor name, so it found the `:` *inside* the flow mapping and read the
    // whole line as one entry, yielding `{"": "1}"}`.
    ("&x {a: 1}\n", &[r#"{"a":1}"#]),
    // The copy had no `?` arm, so an explicit key fell through to the
    // plain-scalar arm as the literal `"? a"`.
    ("? a\n: b\n", &[r#"{"a":"b"}"#]),
    // The copy tested `"` *before* the mapping check, so a quoted key parsed
    // as a bare scalar and left `: 1` to become a second document.
    (r#""a": 1"#, &[r#"{"a":1}"#]),
    // Controls: shapes that already agreed.
    ("&x val\n", &[r#""val""#]),
    // A quoted scalar directly after a standalone anchor. Distinct from the
    // unquoted case above: `parse_block_node`'s `&` arm dispatches quoted and
    // unquoted content through different branches.
    (r#"&x "val""#, &[r#""val""#]),
    ("&x 'val'\n", &[r#""val""#]),
    ("&x a: b\n", &[r#"{"a":"b"}"#]),
    // The anchor really does bind to the node below it, not merely go
    // unrecorded: an alias in the next document resolves to that mapping.
    // (In the *same* document it would be a cycle — see
    // `test_build_rejects_cycle_through_anchor_alone_on_the_document_start_line`.)
    (
        "&x\na: 1\n---\nb: *x\n",
        &[r#"{"a":1}"#, r#"{"b":{"a":1}}"#],
    ),
    ("|\n  foo\n", &[r#""foo\n""#]),
    (">\n  foo\n", &[r#""foo\n""#]),
    ("{a: 1}\n", &[r#"{"a":1}"#]),
    ("[1, 2]\n", &["[1,2]"]),
    ("- a\n- b\n", &[r#"["a","b"]"#]),
    ("a: 1\n", &[r#"{"a":1}"#]),
    ("plain\n", &[r#""plain""#]),
    (r#""q""#, &[r#""q""#]),
    ("'s'", &[r#""s""#]),
    // A `---` line that carries only the marker still delimits documents.
    ("a: 1\n---\nb: 2\n", &[r#"{"a":1}"#, r#"{"b":2}"#]),
];

#[test]
fn a_document_start_line_parses_its_content_as_a_bare_line_does() {
    for (content, want) in CASES {
        let with_marker = format!("--- {content}");
        let bare = documents(content);
        let marked = documents(&with_marker);

        assert_eq!(
            bare,
            Ok(expected(want)),
            "bare {content:?} should parse as {want:?}"
        );
        assert_eq!(
            marked, bare,
            "{with_marker:?} should parse as bare {content:?} does"
        );
    }
}

/// Rejections have to agree too, but only on *whether* the input is rejected:
/// the `--- ` prefix shifts every offset the message carries by four bytes.
#[test]
fn a_document_start_line_rejects_what_a_bare_line_rejects() {
    for content in ["*nope", "&x\na: 1\nb: *x", "&x\n- 1\n- *x"] {
        let bare = documents(content);
        let marked = documents(&format!("--- {content}"));
        assert!(bare.is_err(), "bare {content:?} should be rejected");
        assert!(
            marked.is_err(),
            "--- {content:?} should be rejected like the bare form"
        );
    }
}
