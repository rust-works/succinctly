//! A tab, not just a space, separates an explicit key/value indicator (`?`/`:`)
//! from its content (#434).
//!
//! Four sites in `src/yaml/parser.rs` looked ahead one byte past `?` or `:` to
//! decide whether it was an explicit-key/value indicator at all, and each
//! matched `Some(b' ' | b'\n' | b'\r') | None` - missing `b'\t'` - while the
//! `-` (sequence indicator) check in the exact same functions, a few lines
//! away, already matched the full four-byte set, as does the canonical shared
//! definition `is_seq_indicator_next` (`src/yaml/mod.rs`, written for #332
//! specifically to be "the one definition" after five separate copies of this
//! terminator set diverged).
//!
//! Before the fix, `?\tkey` fell through to `looks_like_mapping_entry()` and
//! was parsed as a **plain scalar** `"?\tkey"` instead of an explicit-key
//! node - a structural misparse, not a missed edge case: `?\tkey\n:\tvalue\n`
//! loaded as two unrelated top-level documents (`"?\tkey"` and
//! `{"":"value"}`) instead of the one intended mapping entry.
//!
//! Inputs are byte literals so no editor or formatter can quietly launder a
//! tab into a space.
//!
//! Run with: `cargo test --test yaml_explicit_indicator_tab_tests`

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

/// The main per-line dispatch (`parse_document_line`): a top-level explicit
/// key/value pair separated by tabs instead of spaces.
#[test]
fn a_tab_after_the_explicit_key_and_value_indicators_is_separation() {
    assert_json(
        b"?\tkey\n:\tvalue\n",
        r#"{"key":"value"}"#,
        "the #434 repro: loaded as two unrelated documents before the fix",
    );
    assert_json(
        b"?\tkey\n",
        r#"{"key":null}"#,
        "an explicit key with no value is null, same as the space form",
    );
}

/// The sequence-item-value site (`parse_sequence_item_inner`): `- ? k` with a
/// tab before the key.
#[test]
fn a_tab_after_an_explicit_key_inside_a_sequence_item_is_separation() {
    assert_json(
        b"-\t?\tk\n  :\tv\n",
        r#"[{"k":"v"}]"#,
        "explicit key as a sequence item's value, tab-separated throughout",
    );
}

/// The control: the space form already worked and must keep producing the
/// same document as the tab form.
#[test]
fn spelling_the_indicator_separator_with_a_space_gives_the_same_document() {
    for (tab_form, space_form) in [
        (&b"?\tkey\n:\tvalue\n"[..], &b"? key\n: value\n"[..]),
        (b"?\tkey\n", b"? key\n"),
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
