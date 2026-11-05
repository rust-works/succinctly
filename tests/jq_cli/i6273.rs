//! #6273: a range whose start is above its end inside a character class is `empty range in char class`
//! in jq (exit 5); the `regex` crate's own parse error was reported instead. Every row was captured
//! from jq 1.7.1 (`/usr/bin/jq -n`).

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn test_a_descending_range_in_a_class_is_refused_as_jq_refuses_it_6273() -> Result<()> {
    for pattern in [
        r"[z-a]",
        r"[:-1]",
        r"[\x{ff}-a]",
        r"[b-a]+",
        r"[z-a&&b]",
        r"(?i)[z-a]",
        r"[^z-a]",
        r"[\n-\t]",
        r"[\101-\100]",
        r"[\cB-\cA]",
        r"[é-a]",
        r"[a-c-0-\x]",
        r"[a-z0-9_:-.]",
        r"[a-zA-Z0-9\-_]+[z-a]",
        r"[\x{20ac}-\x{20ab}]",
    ] {
        let (stdout, stderr, status) =
            run_jq_full(&["-n", "--arg", "p", pattern, r#""a" | test($p)"#], None)?;
        assert_eq!((stdout.as_str(), status), ("", 5), "{pattern}: {stderr}");
        assert!(
            stderr.contains("invalid regex: empty range in char class"),
            "{pattern}: {stderr}"
        );
    }
    Ok(())
}

/// The ranges that are not descending, and the `-` that is no range, still match.
#[test]
fn test_an_ascending_range_or_a_literal_dash_in_a_class_still_matches_6273() -> Result<()> {
    for (pattern, expected) in [
        (r"[a-z]", r#"["a"]"#),
        (r"[a-a]", r#"["a"]"#),
        (r"[a-]", r#"["a","-"]"#),
        (r"[-a]", r#"["a","-"]"#),
        (r"[a-c-b]", r#"["a","-"]"#),
        (r"[a-é]", r#"["a","é"]"#),
        (r"[+--]", r#"["-",",","+"]"#),
        (r"[\--a]", r#"["a","-",":","]"]"#),
        (r"[\n-\r]", r"[]"),
        (r"[\101-\132]", r"[]"),
        (r"[\x{20ab}-\x{20ac}]", r"[]"),
        (r"[a-c-]", r#"["a","-"]"#),
        (r"[:-a]", r#"["a",":","]"]"#),
        (r"[]-a]", r#"["a","]"]"#),
    ] {
        let (stdout, stderr, status) = run_jq_full(
            &[
                "-n",
                "-r",
                "--arg",
                "p",
                pattern,
                r#""a-é\u0001:,]+ " | [match($p; "g") | .string] | tojson"#,
            ],
            None,
        )?;
        assert_eq!(
            (stdout.trim_end(), status),
            (expected, 0),
            "{pattern}: {stderr}"
        );
    }
    Ok(())
}
