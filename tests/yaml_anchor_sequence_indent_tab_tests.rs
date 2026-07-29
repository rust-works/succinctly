//! A tab, not just a space, separates a same-indent block sequence's `-` from
//! its content when deciding whether that sequence is an anchored key's value
//! (#434).
//!
//! A block sequence may sit at its parent mapping key's own indent (YAML
//! allows this compact form). `parse_mapping_entry`'s `is_sequence_at_same_indent`
//! check (`src/yaml/parser.rs`) decides, while still parsing the key's line,
//! whether a same-indent `-` on the next line is that sequence - and it
//! matched `Some(b' ' | b'\n' | b'\r') | None`, missing `b'\t'`, while its own
//! doc comment says *"Same test as `parse_compact_mapping_entry`, which had it
//! right"* - but `parse_compact_mapping_entry`'s copy already included the
//! tab and this one didn't.
//!
//! This only matters when the key is anchored: without an anchor, the normal
//! dispatch loop resolves the sequence as continuation content regardless.
//! With an anchor, the parser must decide immediately (before the sequence
//! is parsed) whether to eagerly emit a null node for the anchor to point at.
//! Before the fix, a tab after the same-indent `-` made that decision wrongly:
//! `key: &a\n-\tx\n-\ty\n` resolved to `{"key":null}` instead of
//! `{"key":["x","y"]}` - the anchor's value went to the wrong node, the exact
//! "anchor dangling" failure the function's own comment describes, just
//! reached by a tab instead of the previously-fixed case.
//!
//! Run with: `cargo test --test yaml_anchor_sequence_indent_tab_tests`

use succinctly::yaml::{YamlError, YamlIndex, YamlValue};

/// Render as `succinctly yq -o json '.'` does, one JSON value per document.
/// Mirrors `yaml_to_json_documents` in `tests/yaml_test_suite.rs`.
fn to_json(yaml: &[u8]) -> Result<String, YamlError> {
    let index = YamlIndex::build(yaml)?;
    let root = index.root(yaml);
    Ok(match root.value() {
        YamlValue::Sequence(mut elements) => {
            let mut docs = Vec::new();
            while let Some((cursor, rest)) = elements.uncons_cursor() {
                docs.push(cursor.to_json());
                elements = rest;
            }
            docs.join(" ")
        }
        _ => root.to_json_document(),
    })
}

/// Renders a byte string with any non-ASCII-printable bytes visible in
/// assertion output.
struct Text<'a>(&'a [u8]);

impl core::fmt::Debug for Text<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{:?}", String::from_utf8_lossy(self.0))
    }
}

#[track_caller]
fn assert_json(yaml: &[u8], expected: &str, why: &str) {
    let actual =
        to_json(yaml).unwrap_or_else(|e| panic!("{:?} failed to parse: {e} — {why}", Text(yaml)));
    assert_eq!(actual, expected, "input {:?} — {why}", Text(yaml));
}

#[test]
fn a_tab_after_a_same_indent_sequence_dash_does_not_dangle_the_anchor() {
    assert_json(
        b"key: &a\n-\tx\n-\ty\n",
        r#"{"key":["x","y"]}"#,
        "the #434 repro: resolved to {\"key\":null} before the fix",
    );
}

/// The control: without an anchor, the sequence already resolved correctly
/// regardless of this check - pinning that the fix does not change this case.
#[test]
fn a_same_indent_sequence_without_an_anchor_is_unaffected() {
    assert_json(
        b"key:\n-\tx\n-\ty\n",
        r#"{"key":["x","y"]}"#,
        "no anchor means the dispatch loop resolves the sequence either way",
    );
}

/// The space form already worked and must produce the same document as the
/// tab form.
#[test]
fn spelling_the_dash_separator_with_a_space_gives_the_same_document() {
    let tabbed = to_json(b"key: &a\n-\tx\n-\ty\n").expect("the tab form parses");
    let spaced = to_json(b"key: &a\n- x\n- y\n").expect("the space form parses");
    assert_eq!(
        tabbed, spaced,
        "the tab and space forms should be the same document"
    );
}
