//! #5712: a string's `length`, `startswith` and `endswith` are answered from the string where
//! they are composed -- a leading stage the rest runs on, a `select` condition, an `until` or
//! `while` condition -- as they are as a lone last stage (#5691). Each row is a filter over
//! `--arg s <value>` and the answer jq 1.7.1 gives: `length` counts codepoints, the prefix and
//! suffix tests compare bytes.

use anyhow::Result;

use crate::run_jq_full;

const ROWS: &[(&str, &str, &str)] = &[
    ("h\u{e9}llo", "$s|length|tostring", "\"5\"\n"),
    ("h\u{e9}llo", "$s|length|.+1", "6\n"),
    ("h\u{e9}llo", "$s|select(length>3)|length", "5\n"),
    ("h\u{e9}llo", "$s|select(length>30)|length", ""),
    ("\u{1f600}x\u{1f600}", "$s|length|tostring", "\"3\"\n"),
    ("", "$s|select(length==0)|\"empty\"", "\"empty\"\n"),
    ("zzzabc", "$s|startswith(\"zzz\")|not", "false\n"),
    ("zzzabc", "$s|endswith(\"abc\")|tostring", "\"true\"\n"),
    ("zzzabc", "$s|select(startswith(\"zzz\"))|length", "6\n"),
    ("", "$s|until(length>=5;.+\"x\")|length", "5\n"),
    (
        "",
        "$s|[while(length<3;.+\"x\")|length]|tojson",
        "\"[0,1,2]\"\n",
    ),
    (
        "abc",
        "$s|ascii_upcase|select(length>2)|startswith(\"A\")",
        "true\n",
    ),
];

#[test]
fn a_composed_string_length_and_prefix_test_answer_as_jq_does_5712() -> Result<()> {
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

/// A comparison that cannot be ordered still raises: the door answers a count, not the compare.
#[test]
fn a_string_length_compared_with_a_string_still_orders_as_before_5712() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(
        &["-n", "--arg", "s", "abc", "$s|select(length>\"a\")|length"],
        None,
    )?;
    assert_eq!((stdout.as_str(), stderr.as_str(), code), ("", "", 0));
    Ok(())
}
