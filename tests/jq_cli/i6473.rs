//! #6473: a loop body with two or more ways that match nothing only where an anchor holds, with
//! consuming ways between and after them, is written way by way (`onig_loop::exits_repeat`): at
//! each head Oniguruma leaves at the first exit that holds there, which depends only on the anchors
//! that hold at the head (`\b` or `\B`, and `\A` at the first head), so each kind of head is written
//! behind its anchors with its own ways before and after the exit. Every expected answer was
//! captured from jq 1.7.1 (`/usr/bin/jq`).

use anyhow::Result;

use crate::run_jq_full;

const FILTER: &str = r#"$s | [match($p; "g") | [.offset, .string, (.captures | map(.string))]]"#;

fn matches(pattern: &str, subject: &str) -> Result<String> {
    let (stdout, stderr, code) = run_jq_full(
        &["-nc", "--arg", "p", pattern, "--arg", "s", subject, FILTER],
        None,
    )?;
    assert_eq!(
        (code, stderr.as_str()),
        (0, ""),
        "{pattern:?} on {subject:?}"
    );
    Ok(stdout)
}

#[test]
fn a_loop_with_several_exits_leaves_where_oniguruma_does_6473() -> Result<()> {
    for (pattern, subject, want) in [
        // a lazy unbounded repeat, a dropped last exit after a start anchor's (the issue's first row).
        (r"(\Aa*|\B){2,}?", "a b", "[[0,\"\",[\"\"]]]\n"),
        // a lazy bounded repeat, three exits.
        (r"((?:a|b*)\A|\ba*|\bb){2,3}?", " ab", "[[0,\"\",[\"\"]],[1,\"\",[\"\"]],[3,\"\",[\"\"]]]\n"),
        // a greedy unbounded repeat past its minimum, an exit that holds nowhere (`\B\b`) and one at the start beside `\b`.
        (r"(?:\B\b|\w|\b\A){2,}", "a b", "[[0,\"\",[]]]\n"),
        // a greedy unbounded repeat with no minimum, an exit at the start beside a group.
        (r"(?:^(\w*)|(?:a|)|b())+", " ab", "[[0,\"\",[\"\",\"\"]],[1,\"a\",[null,null]],[2,\"\",[\"\",\"\"]],[3,\"\",[\"\",\"\"]]]\n"),
        // a greedy bounded repeat with a start exit and a word exit.
        (r"(^(?:a|)|^|(?:a|b*)\b){2}", "aab", "[[0,\"\",[\"\"]],[2,\"b\",[\"\"]],[3,\"\",[\"\"]]]\n"),
        // a greedy repeat past a minimum of two, three exits with consuming ways between them.
        (r"(?:(?:a|b*)\B|(?:a|b*)\b|b?\b){2,}", "aab", "[[0,\"aa\",[]],[2,\"\",[]],[3,\"\",[]]]\n"),
        // a prefix before the loop, an empty way between two word exits.
        (r"x?(?:()(a*)|\B\b|(a|b)\w)+", "abab", "[[0,\"a\",[\"\",\"\",null]],[1,\"\",[\"\",\"\",\"\"]],[2,\"a\",[\"\",\"\",null]],[3,\"\",[\"\",\"\",\"\"]],[4,\"\",[\"\",\"\",\"\"]]]\n"),
        // a lazy bounded repeat before a literal.
        (r"(\b|(.*?)|b){2,5}?c", "abc", "[[0,\"abc\",[\"b\",\"b\"]]]\n"),
    ] {
        assert_eq!(matches(pattern, subject)?, want, "{pattern:?} on {subject:?}");
    }
    Ok(())
}

/// jq's `$` also holds before a newline that ends the text, which a way here can consume, so the
/// loop keeps its text, which the `regex` crate already answers as jq does.
#[test]
fn a_dollar_exit_beside_a_way_that_takes_a_newline_keeps_its_text_6473() -> Result<()> {
    assert_eq!(
        matches(r"(?:\b^|$\B|(?:a|\n*)\z){2,3}", "\n")?,
        "[[0,\"\",[]],[1,\"\",[]]]\n"
    );
    Ok(())
}

/// Where the way-by-way form is not written (a `$` exit beside a way that may take a newline, a loop
/// inside a lazy repeat that keeps its text), a body with #6424's dropped last exit keeps that form,
/// and anything else keeps its text: both already answer as jq does.
#[test]
fn a_loop_the_listing_declines_keeps_what_it_had_6473() -> Result<()> {
    for (pattern, subject, want) in [
        // `$` beside `\W`, which may take a newline: the form of #6424.
        (r"(?:$\b|\W|\A\B){2}", " ", "[[0,\"\",[]]]\n"),
        // the same inside a counted repeat of three, before `$`.
        (
            r"\b(?:\W|(?:\b$)|\w+|\A\B){3}$",
            "a\n",
            "[[0,\"a\",[]],[1,\"\",[]]]\n",
        ),
        // inside an outer lazy repeat, which keeps the inner loop's own form.
        (r"(?:(?:\A\B| |a+|\b\b){2})+?b", "ab", "[]\n"),
        // a lazy loop inside a greedy one, before `\z`.
        (
            r"((?:(\B|.|\b\b){2,}?)*)\z",
            " a",
            "[[0,\" a\",[\" a\",\"\"]],[2,\"\",[\"\",\"\"]]]\n",
        ),
        // a class range from below a newline takes it.
        (
            r"(?:\A\B|$\B|[\x{1}-a])*",
            " \n",
            "[[0,\"\",[]],[1,\"\",[]],[2,\"\",[]]]\n",
        ),
        // the same with a range around the newline.
        (
            r"(?:\A\B|$\B|[\x{9}-\x{b}])*",
            " \n",
            "[[0,\"\",[]],[1,\"\",[]],[2,\"\",[]]]\n",
        ),
    ] {
        assert_eq!(
            matches(pattern, subject)?,
            want,
            "{pattern:?} on {subject:?}"
        );
    }
    Ok(())
}
