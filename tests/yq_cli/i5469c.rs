//! #5469: an alias used as a mapping key (`*a : 2`) stays an alias, where it used to be printed
//! as the anchor's text (`xy: 2`). yq writes a space before a block mapping's `:` and none in a
//! flow mapping (`{*a: 1}`). The streaming route and the DOM route (a write, `-P`) both keep it,
//! and on a write the alias is held to the anchor-soundness rules a value alias is: a key alias
//! whose anchor is no longer written before it, or no longer holds the key's text, is printed as
//! that text. The matching rows were captured from yq v4.53.3; the divergences are recorded in
//! `docs/compliance/yq/limitations.md`.

use anyhow::Result;

use crate::run_yq_stdin_with_stderr;

const DOC: &str = "b: &a xy\nc:\n  z: 1\n  *a : 2\ne: {*a : 1, k: *a}\n";

/// `(filter, extra args, what yq prints)` for [`DOC`].
const ROWS: &[(&str, &[&str], &str)] = &[
    (
        ".",
        &[],
        "b: &a xy\nc:\n  z: 1\n  *a : 2\ne: {*a: 1, k: *a}\n",
    ),
    (
        ".zz = 1",
        &[],
        "b: &a xy\nc:\n  z: 1\n  *a : 2\ne: {*a: 1, k: *a}\nzz: 1\n",
    ),
    (
        ".c.zz = 3",
        &[],
        "b: &a xy\nc:\n  z: 1\n  *a : 2\n  zz: 3\ne: {*a: 1, k: *a}\n",
    ),
    (
        "sort_keys(.c)",
        &[],
        "b: &a xy\nc:\n  *a : 2\n  z: 1\ne: {*a: 1, k: *a}\n",
    ),
    (
        ".c |= with_entries(.)",
        &[],
        "b: &a xy\nc:\n  z: 1\n  *a : 2\ne: {*a: 1, k: *a}\n",
    ),
    (
        ".c |= (to_entries | from_entries)",
        &[],
        "b: &a xy\nc:\n  z: 1\n  *a : 2\ne: {*a: 1, k: *a}\n",
    ),
    (
        ".",
        &["-P"],
        "b: &a xy\nc:\n  z: 1\n  *a : 2\ne:\n  *a : 1\n  k: *a\n",
    ),
    // The streaming route writes a subtree's aliases as yq does, a key's as a value's.
    (".c", &[], "z: 1\n*a : 2\n"),
    (".e", &[], "{*a: 1, k: *a}\n"),
    // JSON output resolves it.
    (
        ".",
        &["-o=json", "-I0"],
        "{\"b\":\"xy\",\"c\":{\"z\":1,\"xy\":2},\"e\":{\"xy\":1,\"k\":\"xy\"}}\n",
    ),
];

#[test]
fn test_an_alias_used_as_a_mapping_key_stays_an_alias_5469() -> Result<()> {
    for (filter, args, want) in ROWS {
        let (stdout, stderr, code) = run_yq_stdin_with_stderr(filter, DOC, args)?;
        assert_eq!(
            (stdout.as_str(), code),
            (*want, 0),
            "{filter:?} {args:?}: {stderr:?}"
        );
    }
    Ok(())
}

/// A key alias's anchor holding a scalar that is not a string reads back as that scalar's text,
/// so it survives a write (yq v4.53.3).
#[test]
fn test_an_alias_key_to_a_number_survives_a_write_5469() -> Result<()> {
    let (stdout, stderr, code) =
        run_yq_stdin_with_stderr(".zz = 1", "b: &a 1\nc:\n  *a : 2\n", &[])?;
    assert_eq!(
        (stdout.as_str(), code),
        ("b: &a 1\nc:\n  *a : 2\nzz: 1\n", 0),
        "{stderr:?}"
    );
    Ok(())
}

/// A key alias whose target is longer than go-yaml's 128-byte simple-key limit is still `*a`,
/// not a `? ` explicit key (yq v4.53.3), on both routes.
#[test]
fn test_an_alias_key_to_a_long_string_is_not_an_explicit_key_5469() -> Result<()> {
    let long = "x".repeat(200);
    let doc = format!("b: &a {long}\nc:\n  *a : 2\n");
    for (filter, tail) in [(".", ""), (".zz = 1", "zz: 1\n")] {
        let (stdout, stderr, code) = run_yq_stdin_with_stderr(filter, &doc, &[])?;
        assert_eq!(
            (stdout, code),
            (format!("b: &a {long}\nc:\n  *a : 2\n{tail}"), 0),
            "{filter:?}: {stderr:?}"
        );
    }
    Ok(())
}

/// The recorded divergences: yq keeps `*a` in each of these, which then either names no anchor
/// (`del(.b)`) or reads back as a different key (`.b = "q"`, where yq's key follows the anchor
/// and succinctly's stays the text it read). succinctly prints the key's own text instead.
#[test]
fn test_an_alias_key_that_would_not_read_back_prints_its_text_5469() -> Result<()> {
    for (filter, want) in [
        ("del(.b)", "c:\n  z: 1\n  xy: 2\ne: {xy: 1, k: xy}\n"),
        (
            ".b = \"q\"",
            "b: &a q\nc:\n  z: 1\n  xy: 2\ne: {xy: 1, k: *a}\n",
        ),
    ] {
        let (stdout, stderr, code) = run_yq_stdin_with_stderr(filter, DOC, &[])?;
        assert_eq!((stdout.as_str(), code), (want, 0), "{filter:?}: {stderr:?}");
    }
    // An alias to a sequence: succinctly reads the key as `''`, which `*a` would not read back as.
    let (stdout, stderr, code) =
        run_yq_stdin_with_stderr(".zz = 1", "b: &a [1]\nc:\n  *a : 2\n", &[])?;
    assert_eq!(
        (stdout.as_str(), code),
        ("b: &a [1]\nc:\n  '': 2\nzz: 1\n", 0),
        "{stderr:?}"
    );
    Ok(())
}

/// A space between a sequence item's alias or quoted key and its `:` (`- *a : 1`, `- "k" : 1`),
/// which yq writes for an alias key and reads back, used to be a parse error in a compact
/// mapping: its key parser left the space ahead of the colon check, which the block mapping's
/// own parser skips. Captured from yq v4.53.3.
#[test]
fn test_a_space_before_a_compact_mapping_colon_parses_5469() -> Result<()> {
    for (doc, args, want) in [
        (
            "b: &a xy\nc:\n- *a : 1\n",
            &["-o=json", "-I0"][..],
            "{\"b\":\"xy\",\"c\":[{\"xy\":1}]}\n",
        ),
        (
            "b: &a xy\nc:\n- *a : 1\n",
            &[][..],
            "b: &a xy\nc:\n  - *a : 1\n",
        ),
        (
            "c:\n- \"k\" : 1\n- 'j'  : 2\n",
            &["-o=json", "-I0"][..],
            "{\"c\":[{\"k\":1},{\"j\":2}]}\n",
        ),
        (
            "c:\n- \"k\" : 1\n- 'j'  : 2\n",
            &[][..],
            "c:\n  - \"k\": 1\n  - 'j': 2\n",
        ),
    ] {
        let (stdout, stderr, code) = run_yq_stdin_with_stderr(".", doc, args)?;
        assert_eq!(
            (stdout.as_str(), code),
            (want, 0),
            "{doc:?} {args:?}: {stderr:?}"
        );
    }
    // `-P` writes a flow mapping's alias key in a sequence item as `- *a : 1`, which reads back.
    let (written, stderr, code) =
        run_yq_stdin_with_stderr(".", "b: &a xy\nc:\n- {*a : 1}\n", &["-P"])?;
    assert_eq!(
        (written.as_str(), code),
        ("b: &a xy\nc:\n  - *a : 1\n", 0),
        "{stderr:?}"
    );
    let (reread, stderr, code) = run_yq_stdin_with_stderr(".", &written, &["-o=json", "-I0"])?;
    assert_eq!(
        (reread.as_str(), code),
        ("{\"b\":\"xy\",\"c\":[{\"xy\":1}]}\n", 0),
        "{stderr:?}"
    );
    Ok(())
}
