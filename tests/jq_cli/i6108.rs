//! #6108: a group repeated `{0}` (or `{0,0}`) can never take part in a match, but Oniguruma still
//! numbers it, so jq reports a capture entry for it: offset `-1` and string `null`, or an empty string
//! at the match offset when the whole match is empty. The `regex` crate compiles `x{0}` to nothing and
//! lost the group, so the list came out short whenever no later group followed. Every expectation is
//! what `/usr/bin/jq` 1.7.1 printed for the same program.

use anyhow::Result;

use crate::run_jq_full;

fn jq(text: &str, program: &str) -> Result<String> {
    let program = format!("$s | {program}");
    let (stdout, stderr, code) = run_jq_full(&["-nc", "--arg", "s", text, &program], None)?;
    assert_eq!(code, 0, "{text:?} | {program}: {stderr}");
    Ok(stdout.trim().to_string())
}

/// The captures of every match of `pattern` in `text`, as `[offset, string]` pairs.
fn captures(text: &str, pattern: &str) -> Result<String> {
    let program = format!(
        r#"[match("{pattern}"; "g") | .captures | map([.offset, .string])]"#
    );
    jq(text, &program)
}

#[test]
fn a_zero_count_group_with_no_later_group_has_a_capture_entry() -> Result<()> {
    assert_eq!(captures("b", "(b)(a){0}")?, r#"[[[0,"b"],[-1,null]]]"#);
    assert_eq!(captures("ab", "(a){0}b")?, "[[[-1,null]]]");
    assert_eq!(
        captures("ab", "(a){0}(b)(c){0}")?,
        r#"[[[-1,null],[1,"b"],[-1,null]]]"#
    );
    // A later group already kept the list whole; it still does.
    assert_eq!(captures("ab", "(a){0}(b)")?, r#"[[[-1,null],[1,"b"]]]"#);
    Ok(())
}

#[test]
fn a_zero_count_group_in_an_empty_match_is_an_empty_string_at_the_offset() -> Result<()> {
    assert_eq!(captures("ab", "(a){0}")?, r#"[[[0,""]],[[1,""]],[[2,""]]]"#);
    assert_eq!(captures("", "(a){0}")?, r#"[[[0,""]]]"#);
    Ok(())
}

#[test]
fn the_zero_spellings_and_the_groups_inside_one_are_counted() -> Result<()> {
    for pattern in ["(a){0,0}b", "(a){00}b", "(a){0}?b", "(a){0}*b", "(a){0}+b"] {
        assert_eq!(captures("ab", pattern)?, "[[[-1,null]]]", "{pattern}");
    }
    assert_eq!(captures("ab", "(?:(a)){0}b")?, "[[[-1,null]]]");
    assert_eq!(
        captures("ab", "((a)(b)){0}b")?,
        "[[[-1,null],[-1,null],[-1,null]]]"
    );
    assert_eq!(captures("ab", "(a(b){0}){0}b")?, "[[[-1,null],[-1,null]]]");
    assert_eq!(captures("ab", "(a)|(b){0}")?, r#"[[[0,"a"],[-1,null]],[[1,""],[1,""]],[[2,""],[2,""]]]"#);
    Ok(())
}

#[test]
fn capture_and_scan_see_the_group() -> Result<()> {
    assert_eq!(jq("ab", r#"capture("(?<n>a){0}")"#)?, r#"{"n":""}"#);
    assert_eq!(jq("ab", r#"capture("(?<n>a){0}b")"#)?, r#"{"n":null}"#);
    assert_eq!(
        jq("ab", r#"capture("(?<n>a){0}(?<m>b)")"#)?,
        r#"{"n":null,"m":"b"}"#
    );
    assert_eq!(jq("ab", r#"[scan("(a){0}b")]"#)?, "[[null]]");
    Ok(())
}

/// What is not a zero-count group is left alone: a lower bound above zero, an open upper bound, a
/// group that holds no capture, an escaped parenthesis and a class.
#[test]
fn other_repeats_and_lookalikes_keep_their_lists() -> Result<()> {
    assert_eq!(captures("ab", "(a){0,1}b")?, r#"[[[0,"a"]]]"#);
    assert_eq!(captures("ab", "(a){0,}b")?, r#"[[[0,"a"]]]"#);
    assert_eq!(captures("ab", "(?:a){0}b")?, "[[]]");
    assert_eq!(captures("ab", "a{0}b")?, "[[]]");
    assert_eq!(captures("a)b", r"[)](a){0}b")?, "[[[-1,null]]]");
    assert_eq!(captures("(a){0}b", r"\\(a\\){0}b")?, "[]");
    Ok(())
}

/// The flags change the engine or the search, not the group count.
#[test]
fn flags_keep_the_group() -> Result<()> {
    for flags in ["i", "x", "gl", "ix"] {
        let program = format!(r#"[match("(a){{0}}b"; "{flags}") | .captures | length]"#);
        assert_eq!(jq("ab", &program)?, "[1]", "{flags}");
    }
    Ok(())
}
