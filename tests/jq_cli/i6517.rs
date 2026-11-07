//! #6517: a `$` that `(?m)` does not reach (one before an isolated `(?m)`, or outside a
//! `(?m:...)` group) still matches before a final newline, as in jq. Every expected answer was
//! captured from jq 1.7.1 (`/usr/bin/jq`).

use anyhow::Result;

use crate::run_jq_full;

const FILTER: &str = r#"$s | [[match($p; "g") | [.offset, .length]], sub($p; "X"; "g")]"#;

#[test]
fn a_dollar_outside_multi_line_mode_matches_before_a_final_newline_6517() -> Result<()> {
    for (pattern, subject, want) in [
        (r"$(?m)", "ab\n", r#"[[[2,0],[3,0]],"abX\nX"]"#),
        (r"$(?m)", "ab\n\n", r#"[[[3,0],[4,0]],"ab\nX\nX"]"#),
        (r"b$(?m)", "ab\n", r#"[[[1,1]],"aX\n"]"#),
        (r"b$(?m)|z", "ab\n", r#"[[[1,1]],"aX\n"]"#),
        (r"$(?m:)", "ab\n", r#"[[[2,0],[3,0]],"abX\nX"]"#),
        (r"(?:$)(?m)", "ab\n", r#"[[[2,0],[3,0]],"abX\nX"]"#),
        (r"(?m:a)$", "b\nb\n", r#"[[],"b\nb\n"]"#),
        (r"(?m:b$)|b$", "b\nb\n", r#"[[[0,1],[2,1]],"X\nX\n"]"#),
        (r"(?:(?m)b$)b$", "ab\n", r#"[[],"ab\n"]"#),
        (r"b$(?m)$", "b\nb\n", r#"[[[2,1]],"b\nX\n"]"#),
        (r"(?-m:b$)(?m)", "b\nb\n", r#"[[[2,1]],"b\nX\n"]"#),
        // What already agreed: no `(?m)` reaching nothing, a `$` under `(?m)`, no final newline.
        (r"$(?-m)", "ab\n", r#"[[[2,0],[3,0]],"abX\nX"]"#),
        (r"(?m)b$", "b\nb\n", r#"[[[0,1],[2,1]],"X\nX\n"]"#),
        (r"$(?m)", "ab", r#"[[[2,0]],"abX"]"#),
    ] {
        let (stdout, stderr, code) = run_jq_full(
            &["-nc", "--arg", "p", pattern, "--arg", "s", subject, FILTER],
            None,
        )?;
        assert_eq!(
            (code, stderr.as_str(), stdout.as_str()),
            (0, "", format!("{want}\n").as_str()),
            "{pattern:?} on {subject:?}"
        );
    }
    Ok(())
}
