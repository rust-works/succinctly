//! #6126: Oniguruma's Perl syntax reads a group's options one character at a time and knows only
//! `i`, `m`, `x`, `s` and `-`; jq 1.7.1 refuses every other letter (`(?U)`, `(?u:...)`, `(?a)`) with
//! `undefined group option`. The `regex` crate takes `U` (swap greed), `u` and `R` as flags, so
//! `succinctly jq` answered where jq errors, and worded the other letters as a crate parse error.
//! Every row was captured from jq 1.7.1 (`/usr/bin/jq -nc`), 364 option spellings in all.

use anyhow::Result;

use crate::run_jq_full;

/// The refusal jq prints, with this crate's `invalid regex:` prefix where jq says `Regex failure:`.
fn assert_refused(filter: &str, message: &str) -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(&["-nc", filter], None)?;
    assert_eq!(code, 5, "{filter}: stdout={stdout:?} stderr={stderr:?}");
    assert_eq!(stdout, "", "{filter}");
    assert!(
        stderr.contains(&format!("invalid regex: {message}")),
        "{filter}: {stderr:?}"
    );
    Ok(())
}

fn assert_answers(filter: &str, expected: &str) -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(&["-nc", filter], None)?;
    assert_eq!(stdout.trim_end(), expected, "{filter}: stderr={stderr:?}");
    assert_eq!(code, 0, "{filter}");
    Ok(())
}

#[test]
fn test_swap_greed_flag_is_refused_6126() -> Result<()> {
    for filter in [
        r#""a" | test("(?U)a")"#,
        r#""a" | test("(?U:a)")"#,
        r#""aa" | [match("(?U)a+") | .string]"#,
        r#""aa" | capture("(?U)(?<x>a+)")"#,
        r#""aa" | [scan("(?U)a+")]"#,
        r#""aa" | sub("(?U)a+";"X")"#,
        r#""aa" | split("(?U)a+";null)"#,
        r#""a" | test("(?-U)a")"#,
        r#""a" | test("(?iU)a")"#,
        r#""a" | test("(?i-U:a)")"#,
        r#""a" | test("(?Ui:a)")"#,
        r#""a" | test("(?i)(?U)a")"#,
        r#""a" | test("(?:(?U)a)")"#,
        r#""a" | test("(?U)a";"x")"#,
        r#""a" | test("(?x)(?U)a")"#,
    ] {
        assert_refused(filter, "undefined group option")?;
    }
    Ok(())
}

#[test]
fn test_every_other_group_option_letter_is_refused_6126() -> Result<()> {
    // `u` and `R` are `regex` crate flags; the rest the crate rejected with its own wording.
    for letter in ('a'..='z').chain('A'..='Z') {
        if "imxs".contains(letter) {
            continue;
        }
        if letter == 'R' {
            continue; // `(?R)` has its own answer below
        }
        for pattern in [
            format!("(?{letter})a"),
            format!("(?{letter}:a)"),
            format!("(?-{letter})a"),
            format!("(?i{letter})a"),
            format!("(?i-{letter}:a)"),
        ] {
            assert_refused(
                &format!(r#""a" | test("{pattern}")"#),
                "undefined group option",
            )?;
        }
    }
    for pattern in [
        "(?R:a)", "(?-R)a", "(?iR)a", "(?Ra)", "(?i=a)", "(?i!a)", "(?i<a>)", "(?-i1)a", "(?)a",
    ] {
        assert_refused(
            &format!(r#""a" | test("{pattern}")"#),
            "undefined group option",
        )?;
    }
    Ok(())
}

#[test]
fn test_a_whole_pattern_call_is_unmatched_6126() -> Result<()> {
    // `(?R)` is a call of the whole pattern in Oniguruma, which jq reports as a stray `)`.
    for pattern in ["(?R)", "(?R)a", "a(?R)", "(?R)+"] {
        assert_refused(
            &format!(r#""a" | test("{pattern}")"#),
            "unmatched close parenthesis",
        )?;
    }
    Ok(())
}

#[test]
fn test_the_options_oniguruma_knows_still_work_6126() -> Result<()> {
    for (filter, expected) in [
        (r#""A" | test("(?i)a")"#, "true"),
        (r#""A" | test("(?i:a)")"#, "true"),
        (r#""A" | test("(?i-i:a)")"#, "false"),
        (r#""A" | test("(?-i)a";"i")"#, "false"),
        (r#""a\nb" | test("a(?s).b")"#, "true"),
        // `m` is not dotall in jq's Perl syntax: `.` still stops at a newline.
        (r#""a\nb" | test("a(?m).b")"#, "false"),
        (r#""a" | test("(?x) a ")"#, "true"),
        (r#""a" | test("(?ix)  A ")"#, "true"),
        (r#""a" | test("(?imxs)a")"#, "true"),
        (r#""a" | test("(?-imx)a")"#, "true"),
        (r#""a" | test("(?i-m)a")"#, "true"),
        (r#""A" | test("(?i:a)(?-i:b)?")"#, "true"),
    ] {
        assert_answers(filter, expected)?;
    }
    Ok(())
}

#[test]
fn test_text_that_only_looks_like_a_group_option_is_unchanged_6126() -> Result<()> {
    for (filter, expected) in [
        // A class, an escaped parenthesis and a comment are not groups.
        (r#""a" | test("[(?U)]")"#, "false"),
        (r#""(?U)" | test("\\(\\?U\\)")"#, "true"),
        (r#""a" | test("(?#(?U)a")"#, "true"),
        (r#""a" | test("(?x) # (?U)\na")"#, "true"),
        (r#""a" | test("(?x:#(?U)\na)")"#, "true"),
        // Other group forms.
        (r#""a" | test("(?:a)")"#, "true"),
        (r#""a" | test("(?=a)a")"#, "true"),
        (r#""a" | test("(?!b)a")"#, "true"),
        (r#""ba" | test("(?<=b)a")"#, "true"),
        (r#""a" | test("(?>a)")"#, "true"),
        (r#""a" | capture("(?<n>a)")"#, r#"{"n":"a"}"#),
    ] {
        assert_answers(filter, expected)?;
    }
    Ok(())
}

/// A pattern that ends inside an option run is refused as before, in the crate's words: jq says
/// `end pattern in group` (see the limitations entry), and what the reader leaves alone is not
/// answered either.
#[test]
fn test_a_pattern_that_ends_inside_an_option_run_stays_refused_6126() -> Result<()> {
    for filter in [r#""a" | test("(?i")"#, r#""a" | test("a(?-")"#] {
        let (stdout, stderr, code) = run_jq_full(&["-nc", filter], None)?;
        assert_eq!(code, 5, "{filter}: stdout={stdout:?} stderr={stderr:?}");
        assert_eq!(stdout, "", "{filter}");
        assert!(stderr.contains("invalid regex:"), "{filter}: {stderr:?}");
    }
    Ok(())
}
