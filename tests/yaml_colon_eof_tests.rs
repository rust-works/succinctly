//! A colon at absolute end-of-input ends a plain scalar the same way one
//! followed by white space does (#434).
//!
//! `parse_unquoted_value_with_indent_impl`'s colon-terminator check
//! (`src/yaml/parser.rs`) matched `Some(b' ' | b'\t' | b'\n' | b'\r')` but had
//! no `None` arm, so a colon as the very last byte of the document (no
//! trailing newline) was absorbed as scalar content instead of ending the
//! value. `find_scalar_end` (`src/yaml/light.rs`), the cursor's independent
//! re-derivation of a scalar's extent used by `at_offset`/`syq-locate`,
//! already had an explicit EOF check (added for #370's `a:\t1` case) - so
//! `syq` printed `abc:` for `key: abc:` while `syq-locate` on that same node
//! reported a byte range covering only `abc`, the same eval/locate
//! divergence shape as #370, just reached via absolute EOF instead of a
//! missing tab.
//!
//! Succinctly is a lenient semi-indexer (see `docs/architecture/semi-indexing.md`
//! on its "minimal validation" trade-off), not a full validator: a bare colon
//! with nothing to its right cannot legally start a nested node in this
//! position, and a strict parser (`yq`, PyYAML) rejects the document outright
//! ("mapping values are not allowed in this context"). Succinctly does not
//! raise that error; the fix instead makes its two derivations of the value's
//! extent - the parser's and the cursor's - agree on where the value ends,
//! rather than the parser alone treating the colon as content.
//!
//! Inputs are byte literals so no editor or formatter can quietly add a
//! trailing newline that hides the EOF condition under test.
//!
//! Run with: `cargo test --test yaml_colon_eof_tests`

use succinctly::yaml::{locate_offset_detailed, YamlError, YamlIndex, YamlValue};

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

/// The eval half: a colon at absolute EOF (no trailing newline) does not
/// become part of the value.
#[test]
fn a_colon_at_absolute_eof_ends_the_value() {
    assert_json(
        b"key: abc:",
        r#"{"key":"abc"}"#,
        "the #434 repro: loaded as {\"key\":\"abc:\"} before the fix",
    );
    assert_json(
        b"key: abc",
        r#"{"key":"abc"}"#,
        "the control: no trailing colon at all",
    );
    assert_json(
        b"key: http://example.com",
        r#"{"key":"http://example.com"}"#,
        "a colon NOT at EOF, and not followed by white space, stays content",
    );
}

/// The `syq-locate`/`at_offset` half: the byte range for the value must agree
/// with what `syq` prints for the same node - `find_scalar_end` already
/// stopped before the trailing colon, so before the fix these two derivations
/// disagreed on the same span.
#[test]
fn the_located_byte_range_excludes_a_colon_at_absolute_eof() {
    let yaml: &[u8] = b"key: abc:";
    let index = YamlIndex::build(yaml).expect("parses");
    let found = locate_offset_detailed(&index, yaml, 5)
        .unwrap_or_else(|| panic!("offset 5 of {:?} located nothing", Text(yaml)));
    let (start, end) = found.byte_range;
    assert_eq!(
        &yaml[start..end],
        b"abc",
        "offset 5 of {:?}: range {:?} covers {:?}",
        Text(yaml),
        found.byte_range,
        Text(&yaml[start..end])
    );
}
