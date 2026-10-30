//! #5590: three forms jq 1.7.1 compiles that the parser rejected: a surrogate pair in a string
//! literal, a keyword spelled as a field name right after the dot, and a namespaced `def`
//! name. Every expected byte string was captured from the pinned jq 1.7.1 source build
//! (`scripts/compare-jq-build.sh 1.7.1`).

use anyhow::Result;

use crate::run_jq_full;

fn run(filter: &str, input: Option<&str>) -> Result<(String, String, i32)> {
    let args: &[&str] = if input.is_some() {
        &["-c", filter]
    } else {
        &["-n", "-c", filter]
    };
    run_jq_full(args, input)
}

#[test]
fn test_surrogate_pair_escape_is_one_character_5590() -> Result<()> {
    for (program, expected) in [
        (r#""\ud83d\ude00""#, "\"😀\"\n"),
        (r#""a\ud83d\ude00b""#, "\"a😀b\"\n"),
        // The boundaries of the pair: first and last code points of the supplementary planes.
        (r#""\ud800\udc00""#, "\"\u{10000}\"\n"),
        (r#""\udbff\udfff""#, "\"\u{10ffff}\"\n"),
        // Neighbours that are not surrogates stay as they were.
        (r#""\ud7ff\ue000""#, "\"\u{d7ff}\u{e000}\"\n"),
        // A lone low surrogate is U+FFFD.
        (r#""\ude00""#, "\"\u{fffd}\"\n"),
        // Interpolation keeps the escape too.
        (r#""\ud83d\ude00\(1)""#, "\"😀1\"\n"),
    ] {
        let (out, err, code) = run(program, None)?;
        assert_eq!((out.as_str(), code), (expected, 0), "{program}: {err}");
    }
    Ok(())
}

#[test]
fn test_unpaired_high_surrogate_is_a_compile_error_5590() -> Result<()> {
    for program in [
        r#""\ud83d""#,
        r#""\ud83dx""#,
        r#""\ud83d\u0041""#,
        r#""\ud83d\ud83d\ude00""#,
        r#""\ude00\ud83d""#,
        // An escape cut short stays an error.
        r#""\ud83d\ude0""#,
    ] {
        let (out, _, code) = run(program, None)?;
        assert_eq!((out.as_str(), code), ("", 3), "{program}");
    }
    Ok(())
}

const DOC: &str = r#"{"then":1,"elif":2,"else":3,"end":4,"as":5,"catch":6,"and":7,"or":8,"if":9,"a":{"then":10}}"#;

#[test]
fn test_keyword_after_the_dot_is_a_field_name_5590() -> Result<()> {
    for (program, expected) in [
        (".then", "1\n"),
        (".elif", "2\n"),
        (".else", "3\n"),
        (".end", "4\n"),
        (".as", "5\n"),
        (".catch", "6\n"),
        (".and", "7\n"),
        (".or", "8\n"),
        (".if", "9\n"),
        (".a.then", "10\n"),
        (".a|.then", "10\n"),
        (".and?", "7\n"),
        (".then=0|.then", "0\n"),
        ("{\"x\": .and}", "{\"x\":7}\n"),
        ("[.then,.end]", "[1,4]\n"),
        ("path(.a.then)", "[\"a\",\"then\"]\n"),
        // The word is read to its end: `.andy` is a field, not `.and` then `y`.
        (".andy", "null\n"),
        // With a gap the dot is still the identity before the keyword.
        ("if . then 1 else 2 end", "1\n"),
        (". and true", "true\n"),
        (". as $x | $x.then", "1\n"),
        ("try . catch 0 | .then", "1\n"),
        ("if .then then .else else .end end", "3\n"),
    ] {
        let (out, err, code) = run(program, Some(DOC))?;
        assert_eq!((out.as_str(), code), (expected, 0), "{program}: {err}");
    }
    Ok(())
}

#[test]
fn test_namespaced_def_name_5590() -> Result<()> {
    for (program, expected) in [
        ("def a::b: 1; a::b", "1\n"),
        ("def a::b(x): x + 1; a::b(2)", "3\n"),
        ("def a::b::c: 2; a::b::c", "2\n"),
        ("def a::b::c(x): x * 2; a::b::c(4)", "8\n"),
        ("def f: 1; def a::f: 2; [f, a::f]", "[1,2]\n"),
        ("def a::b: 1; def a::b: 2; a::b", "2\n"),
        (
            "def a::b: if . < 3 then . + 1 | a::b else . end; 0 | a::b",
            "3\n",
        ),
        ("[1 | def a::b: . + 1; a::b]", "[2]\n"),
    ] {
        let (out, err, code) = run(program, None)?;
        assert_eq!((out.as_str(), code), (expected, 0), "{program}: {err}");
    }
    // The name is the whole `a::b`: neither part alone, nor a sibling, is defined by it.
    for program in [
        "def a::b: 1; b",
        "def a::b: 1; a::c",
        "def a::b::c: 2; a::b",
        "def a::b::c: 2; a::c",
        "a::b::c",
    ] {
        let (out, _, code) = run(program, None)?;
        assert_eq!((out.as_str(), code), ("", 3), "{program}");
    }
    Ok(())
}
