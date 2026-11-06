//! #6187: a plain group whose whole body is one option group of a zero-width body
//! (`(?:(?i:\K))*`, `(?:(?:(?i:\G)))*`) is a repeat of that option group, a valid target in
//! jq as #6164 made it for the group the operator follows directly. The `regex` crate and
//! `fancy-regex` refuse the repeat of a zero-width body, so the option group is made atomic one or
//! more plain groups down. A zero-width body with no option group in it (`(?:\K)*`,
//! `(?:(?:\G))*`) stays refused, as in jq. Every row was captured from jq 1.7.1
//! (`/usr/bin/jq -n`) over `"cab"`.

use anyhow::Result;

use crate::run_jq_full;

fn run(pattern: &str) -> Result<(String, String, i32)> {
    run_jq_full(
        &[
            "-nc",
            "--arg",
            "p",
            pattern,
            r#""cab" | [match($p; "g") | [.offset, .length]] | tostring"#,
        ],
        None,
    )
}

#[test]
fn test_a_plain_group_of_only_an_option_group_answers_as_jq_does_6187() -> Result<()> {
    for (pattern, expected) in [
        (r"(?:(?i:\K))*", "[[0,0],[1,0],[2,0],[3,0]]"),
        (r"(?:(?i:\K))+b", "[[2,1]]"),
        (r"(?:(?i:\K))?b", "[[2,1]]"),
        (r"(?:(?i:\K)){2}b", "[[2,1]]"),
        (r"(?:(?i:(?=a)))*b", "[[2,1]]"),
        (r"(?:(?i:(?!c)))+", "[[1,0],[2,0],[3,0]]"),
        (r"(?:(?i:(?<=a)))*b", "[[2,1]]"),
        (r"(?:(?i:\G))*", "[[0,0],[1,0],[2,0],[3,0]]"),
        (r"(?:(?:(?i:\G)))*", "[[0,0],[1,0],[2,0],[3,0]]"),
        (r"(?:(?:(?:(?-i:\K))))+b", "[[2,1]]"),
        (r"a(?:(?m:\K))*", "[[2,0]]"),
        (r"(?:(?i-m:\K\K))*b", "[[2,1]]"),
        (r"(?:(?x:\K ))+b", "[[2,1]]"),
        (r"(?:(?i:\K))*?b", "[[2,1]]"),
        (r"(?:(?i:\K)){1,2}?b", "[[2,1]]"),
        (r"(?:a(?i:\K))*", "[[0,0],[2,0],[3,0]]"),
        (r"(?:(?i:\K)a)*b", "[[1,2]]"),
        (r"(?:(?i:\K)|b)*", "[[0,0],[1,0],[2,0],[3,0]]"),
        (r"(?:(?i:\K)){1,2}b", "[[2,1]]"),
        (r"(?:(?i:\K)\))*", "[[0,0],[1,0],[2,0],[3,0]]"),
        (r"(?:(?i:\K))\)*", "[[0,0],[1,0],[2,0],[3,0]]"),
        (r"(?i:\K)\)*b", "[[2,1]]"),
        (r"(?:(?i:\K))*+", "[[0,0],[1,0],[2,0],[3,0]]"),
        (r"(?:(?:(?i:\K)|(?i:\G)))*b", "[[2,1]]"),
    ] {
        let (stdout, stderr, code) = run(pattern)?;
        assert_eq!(
            (stdout.trim_end().trim_matches('"'), code),
            (expected, 0),
            "{pattern}: stderr {stderr}"
        );
    }
    Ok(())
}

#[test]
fn test_a_zero_width_body_without_an_option_group_stays_refused_6187() -> Result<()> {
    for pattern in [r"(?:\K)*", r"(?:(?:\G))*", r"(?:(?:\K))+b", r"(?:(?=a))*"] {
        let (stdout, stderr, code) = run(pattern)?;
        assert_eq!(
            (stdout.as_str(), code),
            ("", 5),
            "{pattern}: stderr {stderr}"
        );
        assert!(stderr.contains("Regex failure"), "{pattern}: {stderr}");
    }
    Ok(())
}
