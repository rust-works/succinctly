//! #5675: `split("lit") | length` over a `-R` line is the number of pieces, counted from the line
//! itself instead of writing the line out as JSON, indexing it and splitting the decoded copy. The
//! answers were captured from jq 1.7.1 (`/usr/bin/jq`) over lines with no, one and many
//! separators, an empty line, an empty separator, a multi-character separator and non-ASCII text.

use anyhow::Result;

use crate::run_jq_full;

const LINES: &str =
    "a,b,c\n\n,\na,,b,\nplain\n\u{e9},\u{fc},\u{65e5}\u{672c}\naXXbXXc\nx, y, z\n,,,\n";

const CASES: &[(&str, &str)] = &[
    (r#"split(",")|length"#, "3\n0\n2\n4\n1\n3\n1\n3\n4\n"),
    (r#". | split(",") | length"#, "3\n0\n2\n4\n1\n3\n1\n3\n4\n"),
    (r#"split("")|length"#, "5\n0\n1\n5\n5\n6\n7\n7\n3\n"),
    (r#"split("XX")|length"#, "1\n0\n1\n1\n1\n1\n3\n1\n1\n"),
    (r#"split(", ")|length"#, "1\n0\n1\n1\n1\n1\n1\n3\n1\n"),
    (r#"(split(",")|length)+1"#, "4\n1\n3\n5\n2\n4\n2\n4\n5\n"),
    (
        r#"[split(",")|length]"#,
        "[3]\n[0]\n[2]\n[4]\n[1]\n[3]\n[1]\n[3]\n[4]\n",
    ),
];

#[test]
fn raw_input_split_length_answers_as_jq_does_5675() -> Result<()> {
    for (program, expected) in CASES {
        let (stdout, stderr, code) = run_jq_full(&["-R", "-c", program], Some(LINES))?;
        assert_eq!(
            (stdout.as_str(), code),
            (*expected, 0),
            "{program}: {stderr}"
        );
    }
    Ok(())
}

/// A separator the door does not take keeps its own diagnostics: a non-string, and an error.
#[test]
fn split_length_keeps_the_errors_of_a_separator_it_declines_5675() -> Result<()> {
    for program in [
        "split(1)|length",
        "split(null)|length",
        r#"split(error("x"))|length"#,
    ] {
        let (stdout, _, code) = run_jq_full(&["-R", "-c", program], Some("a,b\n"))?;
        assert_eq!((stdout.as_str(), code), ("", 5), "{program}");
    }
    Ok(())
}
