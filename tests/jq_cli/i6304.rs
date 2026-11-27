//! #6304: jq 1.7.1 finds no match for `"xc" | match("(?=c*c).+")` although the lookahead holds at
//! offset 1. jq tries a greedy `.+` or `.*` that follows the pattern's leading lookahead only at
//! the start of the subject and after a line break, unless the pattern has a literal to search for
//! (`(?=c+).+`); that is what Oniguruma's `.*`-anchored search does when its optimizer finds no
//! literal, which is the reading these rows fit, not something read from its source. The earlier
//! engine did not model the optimizer and found the match wherever it was, a recorded divergence;
//! on Oniguruma these rows match jq 1.7.1 (#7965). Every expectation is what `/usr/bin/jq` 1.7.1
//! printed. `AGREE` is the neighbours that always agreed with jq.

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

/// (text, pattern, flags, jq 1.7.1): the rows the earlier engine answered with a match.
const ANCHORED: &[(&str, &str, &str, &str)] = &[
    ("xc", r"(?=c*c).+", "", "[]"),
    ("xc", r"(?=c*c).*", "", "[]"),
    ("aaac", r"(?=(?:[bc]*c??|){2,}c).+", "", "[]"),
    ("xxc", r"(?=a*c).+", "", "[]"),
    ("xc", r"(?=c*c)(?:.)*", "", "[]"),
    ("xc", r"(?=c*c)(.+)", "", "[]"),
    ("xc", r"(?:(?=c*c)).+", "", "[]"),
    // a later line start is still tried
    ("xc\nc", r"(?=c*c).+", "g", r#"[[3,"c"]]"#),
    ("x\nxc", r"(?=c*c).+", "", "[]"),
];

#[test]
fn test_a_dot_plus_after_a_leading_lookahead_is_tried_at_line_starts_6304() -> Result<()> {
    for &(text, pattern, flags, expected) in ANCHORED {
        assert_eq!(
            match_on(text, pattern, flags)?,
            expected,
            "{text:?} ~ {pattern:?} [{flags}]"
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
