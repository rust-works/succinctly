//! #6157: `\K` is an assertion to Oniguruma as `\G` and `\b` are, so a repeated group whose body is
//! a bare `\K` (or has one as an alternative) is refused with `target of repeat operator is
//! invalid`, as #6127 made it for the others. A `\K` inside a sequence of several nodes is an
//! ordinary repeat target. Every row was captured from jq 1.7.1 (`/usr/bin/jq -n`).

use anyhow::Result;

use crate::run_jq_full;

fn filter(pattern: &str) -> String {
    format!(
        r#""ab" | test("{}")"#,
        pattern.replace('\\', "\\\\").replace('"', "\\\"")
    )
}

#[test]
fn test_a_repeated_group_that_is_a_bare_k_is_refused_6157() -> Result<()> {
    for pattern in [
        r"(?:a|\K)*",
        r"(?:\K|a)+",
        r"(?:\K)*",
        r"(?:\K)+",
        r"(?:\K)?",
        r"(?:\K){0}",
        r"(?:\K){1,2}",
        r"(?:\K)*?",
        r"(?:\K)*+",
        r"(?:a|\K|b)*",
        r"(?:(?:\K)|c)*",
        r"(?:a|(?:\K))*",
        r"(?:a|(?:b|\K))+",
        r"(?=\K)*",
        r"(?:\K|\G)*",
        r"\K*",
        r"\K+",
        r"\K?",
        r"\K{2}",
        r"a\K*b",
        r"x(?:\K|y){2,3}",
        r"(?<=a(?:\K|b)*)",
        r"(?<!\K)+",
        r"(?!\K)*",
        r"(?<=\K)*",
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-nc", &filter(pattern)], None)?;
        assert_eq!(code, 5, "{pattern}: stdout={stdout:?} stderr={stderr:?}");
        assert_eq!(stdout, "", "{pattern}");
        assert!(
            stderr.contains("Regex failure: target of repeat operator is invalid"),
            "{pattern}: {stderr:?}"
        );
    }
    Ok(())
}

#[test]
fn test_a_repeated_group_that_holds_k_among_several_nodes_still_answers_6157() -> Result<()> {
    for (pattern, expected) in [
        (r"(?:a\K|b)*", "true"),
        (r"(\K)*", "true"),
        (r"(?<n>\K)*", "true"),
        (r"(?:\K\K)*", "true"),
        (r"(?:\Ka)*", "true"),
        (r"(?:a|\K\K)*", "true"),
        (r"(?>\K)*", "true"),
        (r"(?:\Kb|a)+", "true"),
        (r"(?:\K){,2}", "false"),
        (r"(?:a\K)?", "true"),
        (r"(?<=\K)", "true"),
        (r"(?<=a\K)", "true"),
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-nc", &filter(pattern)], None)?;
        assert_eq!(
            (stdout.trim_end(), code),
            (expected, 0),
            "{pattern}: stderr={stderr:?}"
        );
    }
    Ok(())
}
