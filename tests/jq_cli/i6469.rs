//! #6469: a pattern with lazy repeats is read in three steps (`onig_loop::READINGS`): the repeats
//! and the loops inside them written, the repeats alone, then neither. A give-up inside one lazy
//! repeat now moves only that repeat to its next reading, so a repeat that keeps its text no longer
//! costs the pattern's other lazy repeats their write-out. Every expected answer was captured from
//! jq 1.7.1 (`/usr/bin/jq`).

use anyhow::Result;

use crate::run_jq_full;

const FILTER: &str = r#"$s | [match($p; "g") | .captures | map(.string)]"#;

fn captures(pattern: &str, subject: &str) -> Result<String> {
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

/// The lazy repeat that keeps its text, `(?:^x*|(?:(y*)|x)+(?:(y)|x?){0,2})*?`, before, after and
/// apart from one that is written, `(?:(?:(b*)|a){0,2}?)*?`.
#[test]
fn a_lazy_repeat_that_keeps_its_text_leaves_the_others_written_6469() -> Result<()> {
    for (pattern, subject, want) in [
        (
            r"(?:^x*|(?:(y*)|x)+(?:(y)|x?){0,2})*?(?:(?:(b*)|a){0,2}?)*?c",
            "aac",
            "[[null,null,null]]\n",
        ),
        (
            r"(?:(?:(b*)|a){0,2}?)*?(?:^x*|(?:(y*)|x)+(?:(y)|x?){0,2})*?c",
            "aac",
            "[[null,null,null]]\n",
        ),
        (
            r"(?:(?:(b*)|a){0,2}?)*?c(?:^x*|(?:(y*)|x)+(?:(y)|x?){0,2})*?",
            "aacx",
            "[[null,null,null]]\n",
        ),
    ] {
        assert_eq!(
            captures(pattern, subject)?,
            want,
            "{pattern:?} on {subject:?}"
        );
    }
    Ok(())
}

/// Nine repeats that keep their text take ten readings, under `MOST_READS` (16), and answer as jq
/// does. Seventeen take more than it allows, so the pattern moves on as a whole past it and the last
/// group is `""` where jq's is `null`: the limitation `docs/compliance/jq/limitations.md` records.
#[test]
fn past_the_most_readings_the_pattern_moves_on_as_a_whole_6469() -> Result<()> {
    let kept = r"(?:^x*|(?:(y*)|x)+(?:(y)|x?){0,2})*?";
    let pattern = |copies: usize| format!("{}(?:(?:(b*)|a){{0,2}}?)*?c", kept.repeat(copies));
    let nulls = |count: usize| ["null"].repeat(count).join(",");
    assert_eq!(
        captures(&pattern(9), "aac")?,
        format!("[[{}]]\n", nulls(19))
    );
    assert_eq!(
        captures(&pattern(17), "aac")?,
        format!("[[{},\"\"]]\n", nulls(34))
    );
    Ok(())
}

/// What the readings already answered as jq does: the written repeat alone, a subject where the
/// written repeat's group is set, and #6427's repeat whose loops are read as written.
#[test]
fn the_readings_still_answer_what_they_did_6469() -> Result<()> {
    for (pattern, subject, want) in [
        (r"(?:(?:(b*)|a){0,2}?)*?c", "aac", "[[null]]\n"),
        (
            r"(?:^x*|(?:(y*)|x)+(?:(y)|x?){0,2})*?(?:(?:(b*)|a){0,2}?)*?c",
            "abac",
            "[[null,null,\"b\"]]\n",
        ),
        (
            r"(?:^b*|(?:(a*)|b)+(?:(a)|b?){0,2}){1,2}?$",
            "abba",
            "[[\"\",null],[\"\",\"\"]]\n",
        ),
    ] {
        assert_eq!(
            captures(pattern, subject)?,
            want,
            "{pattern:?} on {subject:?}"
        );
    }
    Ok(())
}
