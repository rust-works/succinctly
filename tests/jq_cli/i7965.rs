//! #7965 (ADR-0031 step 3): with `onig-engine` built in and `SUCCINCTLY_REGEX_ONIG=1`, jq-mode
//! regex compiles and matches with Oniguruma at jq 1.7.1's pin, read the way jq 1.7.1's `f_match`
//! reads it.
//!
//! Every expected answer was captured from `/usr/bin/jq` 1.7.1, except `capture` on a group that
//! took no part in an empty match, which follows jq 1.8.2 (#7520). The rows are the shape the corpus
//! of `scripts/jq-regex-corpus.py` found the earlier prototype getting wrong: after an empty match
//! jq searches again one byte later, so a match can begin inside a multibyte character. jq then
//! reports it at offset 0, counts its length from the start of the text, and reads its string from
//! the raw bytes, each stray byte a U+FFFD.

#![cfg(feature = "onig-engine")]

use anyhow::Result;

use crate::{spawn_jq, spawn_jq_with_env};

/// `filter` on the JSON text `input`, compact, with the engine switch at `value`: stdout and the
/// exit code, or stderr's message when it fails.
fn run(value: &str, filter: &str, input: &str) -> Result<(String, i32)> {
    let (out, code) = spawn_jq_with_env(
        &["-c", filter],
        "SUCCINCTLY_REGEX_ONIG",
        value,
        Some(input.as_bytes()),
    )?;
    let text = if code == 0 { out.stdout } else { out.stderr };
    Ok((String::from_utf8(text)?.trim_end().to_string(), code))
}

#[test]
fn a_match_that_begins_inside_a_character_is_read_as_jq_reads_it_7965() -> Result<()> {
    for (input, filter, expected) in [
        (
            r#""éſ""#,
            r#"[match("(?<=(?:é|ſ))"; "g") | [.offset, .length, .string]]"#,
            r#"[[1,0,""],[0,2,"�"],[2,0,""]]"#,
        ),
        (
            r#""ﬀff""#,
            r#"[match("\\P{L}*"; "gi") | [.offset, .length, .string]]"#,
            r#"[[0,0,""],[0,1,"��"],[1,0,""],[2,0,""],[3,0,""]]"#,
        ),
        (
            r#""é😀a""#,
            r#"[scan("\\P{L}**"; "s")]"#,
            r#"["","�😀","",""]"#,
        ),
        (
            r#""é😀a""#,
            r#"[match("(?<=(?<n>[^a]))"; "g") | .captures[0] | [.offset, .length, .string, .name]]"#,
            r#"[[1,0,"","n"],[1,1,"😀","n"],[2,0,"","n"]]"#,
        ),
    ] {
        assert_eq!(
            run("1", filter, input)?,
            (expected.to_string(), 0),
            "{input} | {filter}"
        );
    }
    Ok(())
}

#[test]
fn split_and_sub_slice_by_jq_offsets_after_such_a_match_7965() -> Result<()> {
    // The match at offset 0 begins before the one before it ended: jq's `$s[.[0]:.[1]]` and
    // `$in[.previous:$edit.offset]` are then empty, not a panic.
    for (input, filter, expected) in [
        (
            r#""abſ""#,
            r#"[splits("(?<=[ab]|ſ)")]"#,
            r#"["a","b","","",""]"#,
        ),
        (r#""ſß""#, r#"gsub("(?<=(ſ)|ß)"; "<>")"#, r#""ſ<><><>""#),
        (r#""ſß""#, r#"sub("(?<=(ſ)|ß)"; "<>"; "g")"#, r#""ſ<><><>""#),
    ] {
        assert_eq!(
            run("1", filter, input)?,
            (expected.to_string(), 0),
            "{input} | {filter}"
        );
    }
    Ok(())
}

#[test]
fn flags_refusals_and_adoptions_are_jqs_7965() -> Result<()> {
    for (input, filter, expected, code) in [
        (
            r#""aaa""#,
            r#"[match("a|aa"; "gl").string]"#,
            r#"["aa","a"]"#,
            0,
        ),
        (r#""aab""#, r#"[match("x*|a"; "n").string]"#, r#"["a"]"#, 0),
        (
            r#""ab""#,
            r#"[match("(?=(b))").captures[0].string]"#,
            r#"[""]"#,
            0,
        ),
        // jq 1.8.2's answer, adopted in #7520 (jq 1.7.1 gives `{"x":""}`).
        (r#""b""#, r#"capture("(?<x>a)?")"#, r#"{"x":null}"#, 0),
        (
            r#""a""#,
            r#"test("a"; "gz")"#,
            "jq: error (at <stdin>:0): gz is not a valid modifier string",
            5,
        ),
        (
            r#""a""#,
            r#"test("(?<n>a")"#,
            "jq: error (at <stdin>:0): Regex failure: end pattern with unmatched parenthesis",
            5,
        ),
    ] {
        assert_eq!(
            run("1", filter, input)?,
            (expected.to_string(), code),
            "{input} | {filter}"
        );
    }
    Ok(())
}

#[test]
fn the_switch_off_or_absent_keeps_the_default_engine_7965() -> Result<()> {
    let (input, filter) = (r#""éſ""#, r#"[scan("(?<=(?:é|ſ))"; "g")]"#);
    let (absent, code) = spawn_jq(&["-c", filter], Some(input.as_bytes()))?;
    assert_eq!(code, 0);
    let absent = String::from_utf8(absent.stdout)?.trim_end().to_string();
    assert_eq!(run("0", filter, input)?, (absent.clone(), 0));
    assert_ne!(run("1", filter, input)?.0, absent);
    Ok(())
}
