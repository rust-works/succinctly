//! #8062: the spelled-key presentation branches #7999 added and left without a yq-captured test
//! (#8011's coverage report). A spelled key is a bracket key written `0x1` or `01`: on a mapping it
//! names the key of that text, on a sequence the slot of its value. Every row is a live yq v4.53.3
//! capture, run over a file and over stdin, and the documents carry comments, quoting and anchors so
//! that a wrong node shows up in the output. A document with an alias in it takes the alias pass
//! (`propagate_assign_alias_marks`), which reads the nodes a write copies by walking the comment
//! tree rather than a path.

use std::io::Write;

use anyhow::Result;
use tempfile::NamedTempFile;

use crate::{run_yq_file, run_yq_stdin};

const SEQ_ALIAS: &str =
    "a: &x 1 # a\nb: *x # b\ns:\n  - zero # z0\n  - &y \"one\" # o1\n  - *y # y2\nm:\n  k: v\n";
const COMMA_COPY: &str = "m:\n  \"0x1\": five # f\n  \"01\": &a seven # z\n  \"1\": six # s\ns:\n  - &t first # t0\n  - \"second\" # t1\nr: *a\nq: *t\n";
const MAP_KEYS: &str = "m:\n  # head\n  \"0x1\": five # f\n  '1': six # s\n  \"01\": &a seven # z\n  0x2: two # t\nr: *a\n";
const MAP_FLOW: &str = "m: {\"0x1\": 5, \"1\": 6, \"01\": 7} # fm\nq: x # tail\n";
const ENTRIES: &str = "s:\n  - x # x0\n  - - \"a\" # a0\n    - 'b' # b0\n    - &c cc # c0\n";

/// Every row over a file and over stdin, the plain `eval` route, which keeps comments and style.
fn check(rows: &[(&str, &str, &str)]) -> Result<()> {
    for (filter, input, want) in rows {
        let mut file = NamedTempFile::new()?;
        file.write_all(input.as_bytes())?;
        let path = file.path().to_str().expect("utf-8 temp path");
        let (out, code) = run_yq_file(filter, path, &[])?;
        assert_eq!((out.as_str(), code), (*want, 0), "{filter} over {input:?}");
        let (out, code) = run_yq_stdin(filter, input, &[])?;
        assert_eq!(
            (out.as_str(), code),
            (*want, 0),
            "{filter} over stdin {input:?}"
        );
    }
    Ok(())
}

/// A copy from a spelled sequence slot in a document with an alias takes the slot's own comment,
/// quoting and alias, as the static `.s[1]` does: `0x2` and `02` are the slot `*y` is, so the copy is
/// the alias, where `01` and `0x0` are the slots `&y "one"` and `zero` are.
#[test]
fn test_alias_pass_reads_a_spelled_sequence_slot_8062() -> Result<()> {
    check(&[
        (".c = .s[01]", SEQ_ALIAS, "a: &x 1 # a\nb: *x # b\ns:\n  - zero # z0\n  - &y \"one\" # o1\n  - *y # y2\nm:\n  k: v\nc: \"one\" # o1\n"),
        (".c = .s[0x2]", SEQ_ALIAS, "a: &x 1 # a\nb: *x # b\ns:\n  - zero # z0\n  - &y \"one\" # o1\n  - *y # y2\nm:\n  k: v\nc: *y # y2\n"),
        (".c = .s[02]", SEQ_ALIAS, "a: &x 1 # a\nb: *x # b\ns:\n  - zero # z0\n  - &y \"one\" # o1\n  - *y # y2\nm:\n  k: v\nc: *y # y2\n"),
        (".c = .s[0x0]", SEQ_ALIAS, "a: &x 1 # a\nb: *x # b\ns:\n  - zero # z0\n  - &y \"one\" # o1\n  - *y # y2\nm:\n  k: v\nc: zero # z0\n"),
        (".m.k = .s[02]", SEQ_ALIAS, "a: &x 1 # a\nb: *x # b\ns:\n  - zero # z0\n  - &y \"one\" # o1\n  - *y # y2\nm:\n  k: *y # y2\n"),
        (".s[00] = .s[0x1]", SEQ_ALIAS, "a: &x 1 # a\nb: *x # b\ns:\n  - \"one\" # o1\n  - &y \"one\" # o1\n  - *y # y2\nm:\n  k: v\n"),
    ])
}

/// A spelled key under a node the document lacks reads as `null` and copies as
/// `null` (a `+=` of it adds nothing), in a document with an alias.
#[test]
fn test_alias_pass_reads_a_spelled_key_under_an_absent_node_8062() -> Result<()> {
    check(&[
        (".c = .zz.k[0x1]", SEQ_ALIAS, "a: &x 1 # a\nb: *x # b\ns:\n  - zero # z0\n  - &y \"one\" # o1\n  - *y # y2\nm:\n  k: v\nc: null\n"),
        (".c = .zz.k[01]", SEQ_ALIAS, "a: &x 1 # a\nb: *x # b\ns:\n  - zero # z0\n  - &y \"one\" # o1\n  - *y # y2\nm:\n  k: v\nc: null\n"),
        (".c = .m.zz[0x1]", SEQ_ALIAS, "a: &x 1 # a\nb: *x # b\ns:\n  - zero # z0\n  - &y \"one\" # o1\n  - *y # y2\nm:\n  k: v\nc: null\n"),
        (".c += .zz.k[0x1]", SEQ_ALIAS, "a: &x 1 # a\nb: *x # b\ns:\n  - zero # z0\n  - &y \"one\" # o1\n  - *y # y2\nm:\n  k: v\nc: null\n"),
    ])
}

/// A write to a comma of spelled keys, parenthesized, copies the source's comment into each target
/// the document names, as the static `(.m["0x1"], .m["01"]) = .s[0]` does: the keys resolve as one
/// comma, not as a path through it, which names no node.
#[test]
fn test_alias_pass_resolves_a_comma_of_spelled_keys_8062() -> Result<()> {
    check(&[
        ("(.m[0x1], .m[01]) = .s[0]", COMMA_COPY, "m:\n  \"0x1\": first # t0\n  \"01\": &a first # t0\n  \"1\": six # s\ns:\n  - &t first # t0\n  - \"second\" # t1\nr: *a\nq: *t\n"),
        ("(.m[0x1], .m[01]) = .s[1]", COMMA_COPY, "m:\n  \"0x1\": \"second\" # t1\n  \"01\": &a \"second\" # t1\n  \"1\": six # s\ns:\n  - &t first # t0\n  - \"second\" # t1\nr: *a\nq: *t\n"),
        ("(.m[0x1], .s[01]) = .m[01]", COMMA_COPY, "m:\n  \"0x1\": seven # z\n  \"01\": &a seven # z\n  \"1\": six # s\ns:\n  - &t first # t0\n  - \"seven\" # z\nr: *a\nq: *t\n"),
        ("((.m[0x1], .m[01])) = .s[0]", COMMA_COPY, "m:\n  \"0x1\": first # t0\n  \"01\": &a first # t0\n  \"1\": six # s\ns:\n  - &t first # t0\n  - \"second\" # t1\nr: *a\nq: *t\n"),
        ("(.m[0x1], .m[01]) += .s[0]", COMMA_COPY, "m:\n  \"0x1\": fivefirst # f\n  \"01\": &a sevenfirst # z\n  \"1\": six # s\ns:\n  - &t first # t0\n  - \"second\" # t1\nr: *a\nq: *t\n"),
        ("(.m[0x1], .m[01]) |= \"z\"", COMMA_COPY, "m:\n  \"0x1\": z # f\n  \"01\": &a z # z\n  \"1\": six # s\ns:\n  - &t first # t0\n  - \"second\" # t1\nr: *a\nq: *t\n"),
    ])
}

/// A key renamed through a spelled key renames the key of that text and nothing else, and keeps the
/// key's quoting, the document's comments and its anchors (#6089).
#[test]
fn test_key_rename_through_a_spelled_key_8062() -> Result<()> {
    check(&[
        ("(.m[0x1] | key) = \"q\"", MAP_KEYS, "m:\n  # head\n  \"q\": five # f\n  '1': six # s\n  \"01\": &a seven # z\n  0x2: two # t\nr: *a\n"),
        ("(.m[0x2] | key) = \"q\"", MAP_KEYS, "m:\n  # head\n  \"0x1\": five # f\n  '1': six # s\n  \"01\": &a seven # z\n  q: two # t\nr: *a\n"),
        ("(.m[01] | key) |= \"q\"", MAP_KEYS, "m:\n  # head\n  \"0x1\": five # f\n  '1': six # s\n  \"q\": &a seven # z\n  0x2: two # t\nr: *a\n"),
        ("(.m | .[0x1] | key) = \"q\"", MAP_KEYS, "m:\n  # head\n  \"q\": five # f\n  '1': six # s\n  \"01\": &a seven # z\n  0x2: two # t\nr: *a\n"),
        ("(.m[0x1] | key) = \"new key\"", MAP_KEYS, "m:\n  # head\n  \"new key\": five # f\n  '1': six # s\n  \"01\": &a seven # z\n  0x2: two # t\nr: *a\n"),
        ("(.m[0x1] | key) = \"q\" | .z = 1", MAP_KEYS, "m:\n  # head\n  \"q\": five # f\n  '1': six # s\n  \"01\": &a seven # z\n  0x2: two # t\nr: *a\nz: 1\n"),
        ("(.m[0x1] | key) = \"q\"", MAP_FLOW, "m: {\"q\": 5, \"1\": 6, \"01\": 7} # fm\nq: x # tail\n"),
        ("(.m[01] | key) |= \"r\"", MAP_FLOW, "m: {\"0x1\": 5, \"1\": 6, \"r\": 7} # fm\nq: x # tail\n"),
    ])
}

/// A sequence reached through a spelled key and reshaped keeps each element's comments, quoting and
/// anchors, as the static `.s[1] | to_entries` does.
#[test]
fn test_reshape_of_a_sequence_behind_a_spelled_key_8062() -> Result<()> {
    check(&[
        (".s[01] | to_entries", ENTRIES, "- key: 0\n  value: \"a\" # a0\n- key: 1\n  value: 'b' # b0\n- key: 2\n  value: &c cc # c0\n"),
        (".s[0x1] | to_entries", ENTRIES, "- key: 0\n  value: \"a\" # a0\n- key: 1\n  value: 'b' # b0\n- key: 2\n  value: &c cc # c0\n"),
        (".s[-01] | to_entries", ENTRIES, "- key: 0\n  value: \"a\" # a0\n- key: 1\n  value: 'b' # b0\n- key: 2\n  value: &c cc # c0\n"),
        (".s[01] | to_entries | .[1]", ENTRIES, "key: 1\nvalue: 'b' # b0\n"),
    ])
}
