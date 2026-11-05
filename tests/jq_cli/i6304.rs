//! #6304: jq 1.7.1 finds no match for `"xc" | match("(?=c*c).+")` although the lookahead holds at
//! offset 1, and the first column of `GAPS` is what `/usr/bin/jq` printed. jq tries a greedy `.+` or
//! `.*` that follows the pattern's leading lookahead only at the start of the subject and after a
//! line break, unless the pattern has a literal to search for (`(?=c+).+`); that is what
//! Oniguruma's `.*`-anchored search does when its optimizer finds no literal, which is the reading
//! these rows fit, not something read from its source. Reproducing it means modelling the
//! optimizer, which this tool does not do: it finds the match wherever it is. The last column is
//! this tool's answer, pinned so `docs/compliance/jq/limitations.md` stays true. `AGREE` is the
//! neighbours that agree with jq, so the gap is bounded on both sides.

use anyhow::Result;

use crate::run_jq_full;

fn match_on(text: &str, pattern: &str, flags: &str) -> Result<String> {
    let program = r#"$s | try ([match($p; $f) | [.offset, .string]]) catch "ERR""#;
    let (stdout, stderr, code) = run_jq_full(
        &[
            "-nc", "--arg", "s", text, "--arg", "p", pattern, "--arg", "f", flags, program,
        ],
        None,
    )?;
    assert_eq!(code, 0, "{text:?} ~ {pattern:?} [{flags}]: {stderr}");
    Ok(stdout.trim().to_string())
}

/// (text, pattern, flags, jq 1.7.1, this tool).
const GAPS: &[(&str, &str, &str, &str, &str)] = &[
    ("xc", r"(?=c*c).+", "", "[]", r#"[[1,"c"]]"#),
    ("xc", r"(?=c*c).*", "", "[]", r#"[[1,"c"]]"#),
    (
        "aaac",
        r"(?=(?:[bc]*c??|){2,}c).+",
        "",
        "[]",
        r#"[[3,"c"]]"#,
    ),
    ("xxc", r"(?=a*c).+", "", "[]", r#"[[2,"c"]]"#),
    ("xc", r"(?=c*c)(?:.)*", "", "[]", r#"[[1,"c"]]"#),
    ("xc", r"(?=c*c)(.+)", "", "[]", r#"[[1,"c"]]"#),
    ("xc", r"(?:(?=c*c)).+", "", "[]", r#"[[1,"c"]]"#),
    // a later line start is still tried
    (
        "xc\nc",
        r"(?=c*c).+",
        "g",
        r#"[[3,"c"]]"#,
        r#"[[1,"c"],[3,"c"]]"#,
    ),
    ("x\nxc", r"(?=c*c).+", "", "[]", r#"[[3,"c"]]"#),
];

#[test]
fn test_a_dot_plus_after_a_leading_lookahead_is_tried_everywhere_here_6304() -> Result<()> {
    for &(text, pattern, flags, jq, ours) in GAPS {
        assert_eq!(
            match_on(text, pattern, flags)?,
            ours,
            "{text:?} ~ {pattern:?} [{flags}]"
        );
        assert_ne!(
            jq, ours,
            "{text:?} ~ {pattern:?}: the gap is closed, update the limitations page"
        );
    }
    Ok(())
}

/// Shapes that agree with jq: a lookahead with a literal to search for, a dot that is not a greedy
/// star, a dot-all group, a first match at a line start, a pattern with something before the
/// lookahead, and a lookahead that is not first.
const AGREE: &[(&str, &str, &str, &str)] = &[
    ("xc", r"(?=c+).+", "", r#"[[1,"c"]]"#),
    ("xc", r"(?=c*c).", "", r#"[[1,"c"]]"#),
    ("xc", r"(?=c*c).{1,3}", "", r#"[[1,"c"]]"#),
    ("xc", r"(?=c*c).+?", "", r#"[[1,"c"]]"#),
    ("xc", r"(?=c*c)(?s:.+)", "", r#"[[1,"c"]]"#),
    ("xc", r"(?!x)(?=c*c).+", "", r#"[[1,"c"]]"#),
    ("xc", r"(?=c*c)\w+", "", r#"[[1,"c"]]"#),
    ("x\nc", r"(?=c*c).+", "", r#"[[2,"c"]]"#),
    ("c", r"(?=c*c).+", "", r#"[[0,"c"]]"#),
    ("cx", r"(?=c*c).+", "", r#"[[0,"cx"]]"#),
    ("xc", r"(?=c*c)(?=c).+", "", r#"[[1,"c"]]"#),
    ("xc", r"(?:^|x)(?=c*c).+", "", r#"[[0,"xc"]]"#),
    ("xc", r"(?=.*c).+", "", r#"[[0,"xc"]]"#),
];

#[test]
fn test_the_neighbours_of_the_dot_plus_gap_agree_with_jq_6304() -> Result<()> {
    for &(text, pattern, flags, expected) in AGREE {
        assert_eq!(
            match_on(text, pattern, flags)?,
            expected,
            "{text:?} ~ {pattern:?} [{flags}]"
        );
    }
    Ok(())
}
