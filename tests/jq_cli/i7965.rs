//! #7965 (ADR-0031): jq-mode regex compiles and matches with Oniguruma at jq 1.7.1's pin, read the
//! way jq 1.7.1's `f_match` reads it.
//!
//! Every expected answer was captured from `/usr/bin/jq` 1.7.1, except `capture` on a group that
//! took no part in an empty match, which follows jq 1.8.2 (#7520). The rows are the shape the corpus
//! of `scripts/jq-regex-corpus.py` found the earlier prototype getting wrong: after an empty match
//! jq searches again one byte later, so a match can begin inside a multibyte character. jq then
//! reports it at offset 0, counts its length from the start of the text, and reads its string from
//! the raw bytes, each stray byte a U+FFFD.

#![cfg(feature = "regex")]

use anyhow::Result;

use crate::{spawn_jq, spawn_jq_with_env};

/// `filter` on the JSON text `input`, compact: stdout and the exit code, or stderr's message when
/// it fails.
fn run(filter: &str, input: &str) -> Result<(String, i32)> {
    let (out, code) = spawn_jq(&["-c", filter], Some(input.as_bytes()))?;
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
            run(filter, input)?,
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
            run(filter, input)?,
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
            run(filter, input)?,
            (expected.to_string(), code),
            "{input} | {filter}"
        );
    }
    Ok(())
}

/// Step 6 deleted the earlier engine, and the variable that selected it (`SUCCINCTLY_REGEX_ONIG=0`)
/// now changes nothing: jq's answer (captured from `/usr/bin/jq` 1.7.1) either way.
#[test]
fn the_earlier_engine_switch_is_gone_7965() -> Result<()> {
    let (input, filter) = (r#""éſ""#, r#"[scan("(?<=(?:é|ſ))"; "g")]"#);
    let jq = r#"["","�",""]"#;
    assert_eq!(run(filter, input)?, (jq.to_string(), 0));
    let (off, code) = spawn_jq_with_env(
        &["-c", filter],
        "SUCCINCTLY_REGEX_ONIG",
        "0",
        Some(input.as_bytes()),
    )?;
    assert_eq!(code, 0);
    assert_eq!(String::from_utf8(off.stdout)?.trim_end(), jq);
    Ok(())
}

/// ADR-0031 step 5: a literal or an alternation of literals is searched without Oniguruma, with its
/// answers: the leftmost match, the first alternative there, offsets in characters. Each expected
/// value is captured from `/usr/bin/jq` 1.7.1.
#[test]
fn literal_patterns_answer_as_oniguruma_does_7965() -> Result<()> {
    let input = r#""abc ab a 日本語本 é, x;y""#;
    for (filter, expected) in [
        (
            r#"[match("a|ab|abc";"g") | [.offset, .length, .string]]"#,
            r#"[[0,1,"a"],[4,1,"a"],[7,1,"a"]]"#,
        ),
        (
            r#"[match("abc|ab|a";"g") | .string]"#,
            r#"["abc","ab","a"]"#,
        ),
        (
            r#"[match("本|語本";"g") | [.offset, .length]]"#,
            "[[10,1],[11,2]]",
        ),
        (
            r#"gsub("é|日"; "<\(.)>")"#,
            r#""abc ab a <{}>本語本 <{}>, x;y""#,
        ),
        (r#"[splits(", |;")]"#, r#"["abc ab a 日本語本 é","x","y"]"#),
        (r#"test("ab|zz")"#, "true"),
    ] {
        assert_eq!(
            run(filter, input)?,
            (expected.to_string(), 0),
            "{input} | {filter}"
        );
    }
    Ok(())
}
