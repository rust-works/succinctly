//! #5563: a global `l` search (`ONIG_OPTION_FIND_LONGEST`, #5530) looks for the longest match at
//! or after where it starts, which reads the rest of the text, so one per match was quadratic in
//! the matches. One pass over the starts now serves every search of a text, and the answers are
//! the ones the per-search scan gave. Expected values were captured from jq 1.7.1 (`/usr/bin/jq`).

use std::time::{Duration, Instant};

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn test_global_longest_matches_are_the_ones_jq_finds_5563() -> Result<()> {
    for (text, pattern, flags, expected) in [
        ("ab ab abb a", "a|ab+", "gl", "[[6,3],[10,1]]"),
        ("abaa", "a|aa|aaa", "gl", "[[2,2]]"),
        ("éé a éa", "é+|a", "gl", "[[0,2],[5,1],[6,1]]"),
        ("aab", "(?<=a)[ab]", "gl", "[[1,1],[2,1]]"),
        ("aabbab", "\\Ga|b+", "gl", "[[2,2],[4,1],[5,1]]"),
        ("aabédb ", "a|ab+", "gln", "[[1,2]]"),
        ("abcabcd", "(a|ab)(c|bcd)?", "gl", "[[3,4]]"),
        ("xaxax", "x*|a+", "gln", "[[0,1],[1,1],[2,1],[3,1],[4,1]]"),
        ("ab ab abb a", "a|ab+", "l", "[[6,3]]"),
        ("ab\nab abb", "a|ab+", "gl", "[[6,3]]"),
        ("abab", "ab|a|abc", "gl", "[[0,2],[2,2]]"),
    ] {
        let (stdout, stderr, code) = run_jq_full(
            &[
                "-nc",
                "--arg",
                "t",
                text,
                "--arg",
                "p",
                pattern,
                "--arg",
                "f",
                flags,
                "[$t | match($p; $f) | [.offset, .length]]",
            ],
            None,
        )?;
        assert_eq!(code, 0, "{text:?} {pattern:?} {flags:?}: {stderr:?}");
        assert_eq!(
            stdout.trim_end(),
            expected,
            "{text:?} {pattern:?} {flags:?}"
        );
    }
    Ok(())
}

#[test]
fn test_every_global_builtin_reads_the_same_matches_5563() -> Result<()> {
    for (program, expected) in [
        (r#"gsub("a|ab+"; "<\(.)>"; "l")"#, r#""ab ab <{}> <{}>""#),
        (r#"[scan("a|ab+"; "l")]"#, r#"["abb","a"]"#),
        (r#"[splits("a|ab+"; "l")]"#, r#"["ab ab "," ",""]"#),
        (r#"sub("a|ab+"; "X"; "gl")"#, r#""ab ab X X""#),
        (r#"[match("a|ab+"; "gln") | .string]"#, r#"["abb","a"]"#),
        (r#"[match("a|ab+"; "l") | .string]"#, r#"["abb"]"#),
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-Rc", program], Some("ab ab abb a\n"))?;
        assert_eq!(code, 0, "{program}: {stderr:?}");
        assert_eq!(stdout.trim_end(), expected, "{program}");
    }
    Ok(())
}

/// The ab-repeated text from the issue: `"ab "` 20,000 times. Each of its 20,000 matches used to
/// read the rest of the text again (155 s in a debug build, 4.2 s in jq); one pass takes about a
/// second. The bound is far above that and far below the quadratic cost, so it fails only if the
/// loop is quadratic again.
#[test]
fn test_global_longest_search_is_linear_in_the_matches_5563() -> Result<()> {
    let text = "ab ".repeat(20_000);
    let started = Instant::now();
    let (stdout, stderr, code) =
        run_jq_full(&["-Rsc", r#"[match("a|ab+"; "gl")] | length"#], Some(&text))?;
    let took = started.elapsed();
    assert_eq!((stdout.as_str(), code), ("20000\n", 0), "{stderr:?}");
    assert!(
        took < Duration::from_secs(40),
        "{took:?}: the global `l` loop is quadratic again"
    );
    Ok(())
}
