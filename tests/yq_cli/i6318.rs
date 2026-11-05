//! #6318: a write rebuilds the document from its parts, and a `<<` merge key that names nothing to merge
//! (`- <<: *seq` where the alias is a sequence of scalars, `<<: 5`, `<<:`, `<<: hello`) was left out, so a
//! list item came back `- {}` and the key and its alias were gone from the file. yq keeps the entry as
//! written, spelling the key `!!merge <<`; this keeps it too and writes it plain, which reads back as the
//! same merge scalar. The rows pin this build's output, so a change in the spelling shows here; a merge of a
//! mapping is expanded by a write, as before (`docs/compliance/yq/limitations.md`).

use anyhow::Result;

use crate::run_yq_stdin;

/// (document, filter, expected stdout).
const ROWS: &[(&str, &str, &str)] = &[
    (
        "x: &a\n  - one\n  - two\nl:\n  - <<: *a\n  - y\n",
        ".zzkey = 1",
        "x: &a\n  - one\n  - two\nl:\n  - <<: *a\n  - y\nzzkey: 1\n",
    ),
    (
        "x: &a\n  - one\n  - two\nl:\n  - <<: [*a]\n  - y\n",
        ".zzkey = 1",
        "x: &a\n  - one\n  - two\nl:\n  - <<: [*a]\n  - y\nzzkey: 1\n",
    ),
    (
        "b:\n  <<: 5\n  k: 1\n",
        ".zzkey = 1",
        "b:\n  <<: 5\n  k: 1\nzzkey: 1\n",
    ),
    (
        "b:\n  <<:\n  k: 1\n",
        ".zzkey = 1",
        "b:\n  <<:\n  k: 1\nzzkey: 1\n",
    ),
    (
        "b: {<<: hello, k: 1}\n",
        ".zzkey = 1",
        "b: {<<: hello, k: 1}\nzzkey: 1\n",
    ),
];

#[test]
fn a_write_keeps_a_merge_key_that_names_nothing_to_merge_6318() -> Result<()> {
    for (document, filter, expected) in ROWS {
        let (stdout, code) = run_yq_stdin(filter, document, &[])?;
        assert_eq!(stdout, *expected, "{document:?} | {filter}");
        assert_eq!(code, 0, "{document:?} | {filter}");
        // And it reads back as the document it came from: the `<<` entry is still a member.
        let probe = ".. | select(type == \"!!map\") | select(has(\"<<\")) | keys";
        let (before, _) = run_yq_stdin(probe, document, &[])?;
        let (again, _) = run_yq_stdin(probe, &stdout, &[])?;
        assert!(
            before.contains("<<"),
            "{document:?}: the probe sees the entry: {before:?}"
        );
        assert_eq!(again, before, "{document:?} reads back");
    }
    Ok(())
}

/// A read that lists the mapping's members does not list the entry: `.[]` and the JSON output skip a merge
/// value that names nothing, as yq does.
#[test]
fn a_read_still_skips_a_merge_key_that_names_nothing_to_merge_6318() -> Result<()> {
    let document = "b:\n  <<: 5\n  k: 1\n";
    let (stdout, code) = run_yq_stdin(".b[]", document, &[])?;
    assert_eq!((stdout.as_str(), code), ("1\n", 0));
    let (stdout, code) = run_yq_stdin(".", document, &["-o", "json", "-I0"])?;
    assert_eq!((stdout.as_str(), code), ("{\"b\":{\"k\":1}}\n", 0));
    Ok(())
}
