//! #5691: `$s | length`, `startswith("lit")` and `endswith("lit")` of a string named argument are
//! answered from the string itself, with no write-out and re-index of its text. Each row is a
//! value, a filter and the answer jq gives: `length` counts codepoints, and the prefix and suffix
//! tests compare bytes.

use anyhow::Result;

use crate::run_jq_full;

const ROWS: &[(&str, &str, &str)] = &[
    ("", "$s|length", "0\n"),
    ("abc", "$s|length", "3\n"),
    ("h\u{e9}llo", "$s|length", "5\n"),
    ("\u{65e5}\u{672c}\u{8a9e}", "$s|length", "3\n"),
    ("\u{1f600}x\u{1f600}", "$s|length", "3\n"),
    ("a\nb\t\"q\"", "$s|length", "7\n"),
    ("zzzabc", "$s|startswith(\"zzz\")", "true\n"),
    ("zzzabc", "$s|startswith(\"abc\")", "false\n"),
    ("zzzabc", "$s|endswith(\"abc\")", "true\n"),
    ("zzzabc", "$s|endswith(\"zzz\")", "false\n"),
    ("abc", "$s|startswith(\"\")", "true\n"),
    ("", "$s|endswith(\"\")", "true\n"),
    (
        "\u{65e5}\u{672c}\u{8a9e}",
        "$s|endswith(\"\u{672c}\u{8a9e}\")",
        "true\n",
    ),
    ("abc", "$s | . | length", "3\n"),
    (
        "abc",
        "[$s|length, ($s|startswith(\"a\"))]|tojson",
        "\"[3,true]\"\n",
    ),
];

#[test]
fn a_named_string_answers_length_and_prefix_tests_5691() -> Result<()> {
    for (value, filter, expected) in ROWS {
        let (stdout, stderr, code) = run_jq_full(&["-n", "--arg", "s", value, filter], None)?;
        assert_eq!(
            (stdout.as_str(), stderr.as_str(), code),
            (*expected, "", 0),
            "{filter} over {value:?}"
        );
    }
    Ok(())
}

/// The shapes the shortcut declines keep jq's own errors: a non-string argument to `startswith`
/// raises exactly as before.
#[test]
fn a_non_string_prefix_still_raises_5691() -> Result<()> {
    let (stdout, stderr, code) =
        run_jq_full(&["-n", "--arg", "s", "abc", "$s|startswith(1)"], None)?;
    assert_eq!(stdout, "");
    assert!(
        stderr.contains("startswith() requires string inputs"),
        "{stderr}"
    );
    assert_eq!(code, 5);
    Ok(())
}
