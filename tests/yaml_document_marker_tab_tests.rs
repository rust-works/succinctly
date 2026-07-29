//! A tab, not just a space, terminates a `---`/`...` document marker (#434).
//!
//! `is_document_start`/`is_document_end` (`src/yaml/parser.rs`) required the
//! marker to be followed by `Some(b' ' | b'\n' | b'\r') | None` - missing
//! `b'\t'` - while the strict validator's equivalent, `doc_marker_char`
//! (`src/yaml/validate.rs`), already accepted all four. Since the loader (not
//! the validator) builds the semi-index `syq`/`yq` actually read from, `---\tfoo`
//! was silently parsed as ordinary scalar content instead of a document
//! boundary, confirmed against the vendored YAML Test Suite case K54U ("Tab
//! after document header", `---\tscalar` -> `"scalar"`), which this fix turns
//! from a known failure into a pass.
//!
//! `skip_document_marker` had the matching gap on the other side: it only
//! skipped one optional `b' '` after the marker, not a tab (or a run of
//! either), which the `is_document_start`/`is_document_end` fix alone would
//! have left masked but not fixed.
//!
//! Run with: `cargo test --test yaml_document_marker_tab_tests`

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
fn a_tab_after_the_document_start_marker_is_separation() {
    assert_json(
        b"---\tscalar\n",
        r#""scalar""#,
        "the YAML Test Suite K54U case: tab after document header",
    );
    assert_json(
        b"---\tfoo: 1\n",
        r#"{"foo":1}"#,
        "inline mapping after the marker, tab-separated",
    );
    assert_json(
        b"---\t\nfoo: 1\n",
        r#"{"foo":1}"#,
        "marker line has only a trailing tab, content is on the next line",
    );
}

#[test]
fn a_tab_after_the_document_end_marker_is_separation() {
    assert_json(
        b"a: 1\n...\t\n",
        r#"{"a":1}"#,
        "a tab-terminated end marker does not get folded into the prior document",
    );
}

/// The control: the space form already worked and must keep working
/// identically to the tab form.
#[test]
fn spelling_the_marker_separator_with_a_space_gives_the_same_document() {
    for (tab_form, space_form) in [
        (&b"---\tscalar\n"[..], &b"--- scalar\n"[..]),
        (b"---\tfoo: 1\n", b"--- foo: 1\n"),
    ] {
        let tabbed = to_json(tab_form).expect("the tab form parses");
        let spaced = to_json(space_form).expect("the space form parses");
        assert_eq!(
            tabbed,
            spaced,
            "{:?} and {:?} should be the same document",
            Text(tab_form),
            Text(space_form)
        );
    }
}
