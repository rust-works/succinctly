//! #6078: yq's lexer accepts several spellings of most operators (`line_?comment|lineComment`,
//! `to_?entries|toEntries`, `group_?by`, ...), and has `root` and `parents`. Every expected output
//! is what yq v4.53.3 prints for the same document and filter; a row for the underscore spelling
//! stands beside each alias so that the two are pinned to answer alike.
//!
//! The documents are block-style: a collection written in flow style loses it when it is collected
//! into a sequence here (`.a.b | sortby(.)` over `{b: [..]}`), which is older than the aliases and
//! has nothing to do with the spelling.

use anyhow::Result;

use crate::run_yq_stdin;

/// (name, document, filter, extra args, expected stdout).
const ORACLE_ROWS: &[(&str, &str, &str, &[&str], &str)] = &[
    (
        "line_lineComment",
        "a: 1 # one\n# head of b\nb: 2\n# foot of b\n\nc: 3\n",
        ".a | lineComment",
        &[],
        "one\n",
    ),
    (
        "line_linecomment",
        "a: 1 # one\n# head of b\nb: 2\n# foot of b\n\nc: 3\n",
        ".a | linecomment",
        &[],
        "one\n",
    ),
    (
        "line_line_comment",
        "a: 1 # one\n# head of b\nb: 2\n# foot of b\n\nc: 3\n",
        ".a | line_comment",
        &[],
        "one\n",
    ),
    (
        "head_headComment",
        "a: 1 # one\n# head of b\nb: 2\n# foot of b\n\nc: 3\n",
        ".b | headComment",
        &[],
        "\n",
    ),
    (
        "head_headcomment",
        "a: 1 # one\n# head of b\nb: 2\n# foot of b\n\nc: 3\n",
        ".b | headcomment",
        &[],
        "\n",
    ),
    (
        "head_head_comment",
        "a: 1 # one\n# head of b\nb: 2\n# foot of b\n\nc: 3\n",
        ".b | head_comment",
        &[],
        "\n",
    ),
    (
        "foot_footComment",
        "a: 1 # one\n# head of b\nb: 2\n# foot of b\n\nc: 3\n",
        ".b | footComment",
        &[],
        "\n",
    ),
    (
        "foot_footcomment",
        "a: 1 # one\n# head of b\nb: 2\n# foot of b\n\nc: 3\n",
        ".b | footcomment",
        &[],
        "\n",
    ),
    (
        "foot_foot_comment",
        "a: 1 # one\n# head of b\nb: 2\n# foot of b\n\nc: 3\n",
        ".b | foot_comment",
        &[],
        "\n",
    ),
    (
        "assign_line",
        "a: 1 # one\n# head of b\nb: 2\n# foot of b\n\nc: 3\n",
        ".a lineComment = \"x\"",
        &[],
        "a: 1 # x\n# head of b\nb: 2\n# foot of b\n\nc: 3\n",
    ),
    (
        "assign_head",
        "a: 1\nb: 2\nc: 3\n",
        ".b headComment = \"y\"",
        &[],
        "a: 1\nb: 2\n# y\nc: 3\n",
    ),
    (
        "assign_foot",
        "a: 1\nb: 2\nc: 3\n",
        ".c footComment = \"z\"",
        &[],
        "a: 1\nb: 2\nc: 3\n# z\n",
    ),
    (
        "assign_head_lower",
        "a: 1\nb: 2\nc: 3\n",
        ".b headcomment = \"y\"",
        &[],
        "a: 1\nb: 2\n# y\nc: 3\n",
    ),
    (
        "docindex_documentIndex",
        "a: 1\n---\nb: 2\n---\nc: 3\n",
        "documentIndex",
        &["-N"],
        "0\n1\n2\n",
    ),
    (
        "docindex_documentindex",
        "a: 1\n---\nb: 2\n---\nc: 3\n",
        "documentindex",
        &["-N"],
        "0\n1\n2\n",
    ),
    (
        "docindex_document_index",
        "a: 1\n---\nb: 2\n---\nc: 3\n",
        "document_index",
        &["-N"],
        "0\n1\n2\n",
    ),
    (
        "docindex_di",
        "a: 1\n---\nb: 2\n---\nc: 3\n",
        "di",
        &["-N"],
        "0\n1\n2\n",
    ),
    (
        "splitdoc_splitDoc",
        "- a\n- b\n",
        ".[] | splitDoc",
        &[],
        "a\n---\nb\n",
    ),
    (
        "splitdoc_splitdoc",
        "- a\n- b\n",
        ".[] | splitdoc",
        &[],
        "a\n---\nb\n",
    ),
    (
        "splitdoc_split_doc",
        "- a\n- b\n",
        ".[] | split_doc",
        &[],
        "a\n---\nb\n",
    ),
    (
        "mapvalues",
        "n:\n  - x\n  - y\nm:\n  - k: b\n    v: 2\n  - k: a\n    v: 3\n  - k: b\n    v: 1\nd:\n  p: 1\n  q: 2\n",
        ".d | mapvalues(. + 1)",
        &[],
        "p: 2\nq: 3\n",
    ),
    (
        "map_values",
        "n:\n  - x\n  - y\nm:\n  - k: b\n    v: 2\n  - k: a\n    v: 3\n  - k: b\n    v: 1\nd:\n  p: 1\n  q: 2\n",
        ".d | map_values(. + 1)",
        &[],
        "p: 2\nq: 3\n",
    ),
    (
        "groupby",
        "n:\n  - x\n  - y\nm:\n  - k: b\n    v: 2\n  - k: a\n    v: 3\n  - k: b\n    v: 1\nd:\n  p: 1\n  q: 2\n",
        ".m | groupby(.k)",
        &[],
        "- - k: b\n    v: 2\n  - k: b\n    v: 1\n- - k: a\n    v: 3\n",
    ),
    (
        "group_by",
        "n:\n  - x\n  - y\nm:\n  - k: b\n    v: 2\n  - k: a\n    v: 3\n  - k: b\n    v: 1\nd:\n  p: 1\n  q: 2\n",
        ".m | group_by(.k)",
        &[],
        "- - k: b\n    v: 2\n  - k: b\n    v: 1\n- - k: a\n    v: 3\n",
    ),
    (
        "sortby",
        "n:\n  - x\n  - y\nm:\n  - k: b\n    v: 2\n  - k: a\n    v: 3\n  - k: b\n    v: 1\nd:\n  p: 1\n  q: 2\n",
        ".m | sortby(.v)",
        &[],
        "- k: b\n  v: 1\n- k: b\n  v: 2\n- k: a\n  v: 3\n",
    ),
    (
        "sort_by",
        "n:\n  - x\n  - y\nm:\n  - k: b\n    v: 2\n  - k: a\n    v: 3\n  - k: b\n    v: 1\nd:\n  p: 1\n  q: 2\n",
        ".m | sort_by(.v)",
        &[],
        "- k: b\n  v: 1\n- k: b\n  v: 2\n- k: a\n  v: 3\n",
    ),
    (
        "uniqueby",
        "n:\n  - x\n  - y\nm:\n  - k: b\n    v: 2\n  - k: a\n    v: 3\n  - k: b\n    v: 1\nd:\n  p: 1\n  q: 2\n",
        ".m | uniqueby(.k)",
        &[],
        "- k: b\n  v: 2\n- k: a\n  v: 3\n",
    ),
    (
        "unique_by",
        "n:\n  - x\n  - y\nm:\n  - k: b\n    v: 2\n  - k: a\n    v: 3\n  - k: b\n    v: 1\nd:\n  p: 1\n  q: 2\n",
        ".m | unique_by(.k)",
        &[],
        "- k: b\n  v: 2\n- k: a\n  v: 3\n",
    ),
    (
        "sortkeys",
        "b: 1\na: {z: 1, y: 2}\n",
        "sortkeys(.)",
        &[],
        "a: {z: 1, y: 2}\nb: 1\n",
    ),
    (
        "sortKeys",
        "b: 1\na: {z: 1, y: 2}\n",
        "sortKeys(.)",
        &[],
        "a: {z: 1, y: 2}\nb: 1\n",
    ),
    (
        "to_toentries",
        "n:\n  - x\n  - y\nm:\n  - k: b\n    v: 2\n  - k: a\n    v: 3\n  - k: b\n    v: 1\nd:\n  p: 1\n  q: 2\n",
        ".d | toentries",
        &[],
        "- key: p\n  value: 1\n- key: q\n  value: 2\n",
    ),
    (
        "to_toEntries",
        "n:\n  - x\n  - y\nm:\n  - k: b\n    v: 2\n  - k: a\n    v: 3\n  - k: b\n    v: 1\nd:\n  p: 1\n  q: 2\n",
        ".d | toEntries",
        &[],
        "- key: p\n  value: 1\n- key: q\n  value: 2\n",
    ),
    (
        "to_to_entries",
        "n:\n  - x\n  - y\nm:\n  - k: b\n    v: 2\n  - k: a\n    v: 3\n  - k: b\n    v: 1\nd:\n  p: 1\n  q: 2\n",
        ".d | to_entries",
        &[],
        "- key: p\n  value: 1\n- key: q\n  value: 2\n",
    ),
    (
        "from_fromentries",
        "n:\n  - x\n  - y\nm:\n  - k: b\n    v: 2\n  - k: a\n    v: 3\n  - k: b\n    v: 1\nd:\n  p: 1\n  q: 2\n",
        ".d | to_entries | fromentries",
        &[],
        "p: 1\nq: 2\n",
    ),
    (
        "from_fromEntries",
        "n:\n  - x\n  - y\nm:\n  - k: b\n    v: 2\n  - k: a\n    v: 3\n  - k: b\n    v: 1\nd:\n  p: 1\n  q: 2\n",
        ".d | to_entries | fromEntries",
        &[],
        "p: 1\nq: 2\n",
    ),
    (
        "from_from_entries",
        "n:\n  - x\n  - y\nm:\n  - k: b\n    v: 2\n  - k: a\n    v: 3\n  - k: b\n    v: 1\nd:\n  p: 1\n  q: 2\n",
        ".d | to_entries | from_entries",
        &[],
        "p: 1\nq: 2\n",
    ),
    (
        "with_withentries",
        "n:\n  - x\n  - y\nm:\n  - k: b\n    v: 2\n  - k: a\n    v: 3\n  - k: b\n    v: 1\nd:\n  p: 1\n  q: 2\n",
        ".d | withentries(.value |= . + 1)",
        &[],
        "p: 2\nq: 3\n",
    ),
    (
        "with_withEntries",
        "n:\n  - x\n  - y\nm:\n  - k: b\n    v: 2\n  - k: a\n    v: 3\n  - k: b\n    v: 1\nd:\n  p: 1\n  q: 2\n",
        ".d | withEntries(.value |= . + 1)",
        &[],
        "p: 2\nq: 3\n",
    ),
    (
        "with_with_entries",
        "n:\n  - x\n  - y\nm:\n  - k: b\n    v: 2\n  - k: a\n    v: 3\n  - k: b\n    v: 1\nd:\n  p: 1\n  q: 2\n",
        ".d | with_entries(.value |= . + 1)",
        &[],
        "p: 2\nq: 3\n",
    ),
    (
        "fromunix_fromunix",
        "t: 1700000000\n",
        ".t | fromunix | tz(\"UTC\")",
        &[],
        "2023-11-14T22:13:20Z\n",
    ),
    (
        "fromunix_from_unix",
        "t: 1700000000\n",
        ".t | from_unix | tz(\"UTC\")",
        &[],
        "2023-11-14T22:13:20Z\n",
    ),
    (
        "tounix_tounix",
        "t: \"2024-01-01T00:00:00Z\"\n",
        ".t | tounix",
        &[],
        "1704067200\n",
    ),
    (
        "tounix_to_unix",
        "t: \"2024-01-01T00:00:00Z\"\n",
        ".t | to_unix",
        &[],
        "1704067200\n",
    ),
    (
        "a2m_array_to_map",
        "- a\n- b\n",
        "array_to_map",
        &[],
        "0: a\n1: b\n",
    ),
    (
        "a2m_array_tomap",
        "- a\n- b\n",
        "array_tomap",
        &[],
        "0: a\n1: b\n",
    ),
    (
        "a2m_arrayto_map",
        "- a\n- b\n",
        "arrayto_map",
        &[],
        "0: a\n1: b\n",
    ),
    (
        "a2m_arraytomap",
        "- a\n- b\n",
        "arraytomap",
        &[],
        "0: a\n1: b\n",
    ),
    (
        "root",
        "a:\n  b:\n    - 1\n    - c: 3\nd: x\n",
        ".a.b[1].c | root",
        &[],
        "a:\n  b:\n    - 1\n    - c: 3\nd: x\n",
    ),
    (
        "root_member",
        "a:\n  b:\n    - 1\n    - c: 3\nd: x\n",
        ".a.b[1].c | root | .d",
        &[],
        "x\n",
    ),
    (
        "root_at_root",
        "a:\n  b:\n    - 1\n    - c: 3\nd: x\n",
        "root",
        &[],
        "a:\n  b:\n    - 1\n    - c: 3\nd: x\n",
    ),
    (
        "parents",
        "a:\n  b:\n    - 1\n    - c: 3\nd: x\n",
        ".a.b[1].c | parents",
        &[],
        "- c: 3\n- - 1\n  - c: 3\n- b:\n    - 1\n    - c: 3\n- a:\n    b:\n      - 1\n      - c: 3\n  d: x\n",
    ),
    (
        "parents_length",
        "a:\n  b:\n    - 1\n    - c: 3\nd: x\n",
        ".a.b[1].c | parents | length",
        &[],
        "4\n",
    ),
    (
        "parents_root",
        "a:\n  b:\n    - 1\n    - c: 3\nd: x\n",
        "parents",
        &[],
        "[]\n",
    ),
    (
        "parents_first",
        "a:\n  b:\n    - 1\n    - c: 3\nd: x\n",
        ".a.b[1] | parents | .[0]",
        &[],
        "- 1\n- c: 3\n",
    ),
    (
        "parents_each",
        "a:\n  b:\n    - 1\n    - c: 3\nd: x\n",
        ".a.b[] | parents | length",
        &[],
        "3\n3\n",
    ),
    (
        "parents_of_scalar",
        "a:\n  b:\n    - 1\n    - c: 3\nd: x\n",
        ".d | parents",
        &[],
        "- a:\n    b:\n      - 1\n      - c: 3\n  d: x\n",
    ),
    (
        "filename_stdin",
        "a:\n  b:\n    - 1\n    - c: 3\nd: x\n",
        "fileName",
        &[],
        "-\n",
    ),
    (
        "file_name_stdin",
        "a:\n  b:\n    - 1\n    - c: 3\nd: x\n",
        "file_name",
        &[],
        "-\n",
    ),
    (
        "fileindex",
        "a:\n  b:\n    - 1\n    - c: 3\nd: x\n",
        "fileindex",
        &[],
        "0\n",
    ),
    (
        "fileIndex",
        "a:\n  b:\n    - 1\n    - c: 3\nd: x\n",
        "fileIndex",
        &[],
        "0\n",
    ),
];

#[test]
fn operator_spellings_and_root_and_parents_match_yq_6078() -> Result<()> {
    for &(name, document, filter, args, expected) in ORACLE_ROWS {
        let (output, code) = run_yq_stdin(filter, document, args)?;
        assert_eq!(code, 0, "{name}");
        assert_eq!(output, expected, "{name}: `{filter}` over {document:?}");
    }
    Ok(())
}

/// jq has none of these spellings: in jq mode they stay undefined functions.
#[test]
fn jq_mode_keeps_the_alias_spellings_undefined_6078() -> Result<()> {
    for (filter, arity) in [
        ("groupby(.)", 1),
        ("sortby(.)", 1),
        ("toentries", 0),
        ("withEntries(.)", 1),
        ("mapvalues(.)", 1),
        ("fromunix", 0),
        ("parents", 0),
        ("root", 0),
    ] {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_succinctly"))
            .args(["jq", "-n", filter])
            .output()?;
        let name = filter.split('(').next().expect("a name");
        assert!(!output.status.success(), "{filter}");
        let stderr = String::from_utf8(output.stderr)?;
        assert!(
            stderr.contains(&format!("{name}/{arity} is not defined")),
            "{filter}: {stderr:?}"
        );
    }
    Ok(())
}

/// A function the program defines under one of these names (possible with `--jq-extensions`) is
/// the one called, as it is for every other name.
#[test]
fn a_defined_function_keeps_the_name_6078() -> Result<()> {
    for (filter, expected) in [
        ("def root: 5; root", "5\n"),
        ("def parents: 6; parents", "6\n"),
        ("def toEntries: 7; toEntries", "7\n"),
        ("def groupby(f): 8; groupby(.a)", "8\n"),
    ] {
        let (output, code) = run_yq_stdin(filter, "a: {b: 1}\n", &["--jq-extensions"])?;
        assert_eq!(code, 0, "{filter}");
        assert_eq!(output, expected, "{filter}");
    }
    Ok(())
}
