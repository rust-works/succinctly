//! #6534: in yq mode a string's `length` inside a loop body is read off the string. The `range`
//! loop is a `--jq-extensions` builtin, so these rows have no yq capture; what is yq's own is the
//! unit: `yq -n '"héllo😀" | length'` is `10`, the string's bytes. Each row binds `. as $w |` ahead
//! of the string, because a bare `$t` stage after a fan-out answers once in yq (#6821) and the
//! loop would not loop. Each answer below is also what the build before the change printed (it
//! reindexed the string once per round), so the rows pin that the shortcut changes the speed and
//! not the answer.

use anyhow::Result;

use crate::run_yq_stdin;

const ARGS: &[&str] = &["--jq-extensions", "-n", "-o=json", "-I=0"];

/// `(filter, output)`.
const ROWS: &[(&str, &str)] = &[
    // A computed multibyte string counts in bytes, every round.
    ("(\"héllo😀\" + \"x\") as $t | [range(3) | . as $w | ($t | length)]", "[11,11,11]\n"),
    ("(\"héllo😀\" + \"x\") as $t | [range(3) | . as $w | $t | length]", "[11,11,11]\n"),
    // The empty string is zero, not a decline.
    ("(\"\" + \"\") as $t | [range(2) | . as $w | ($t | length)]", "[0,0]\n"),
    // An ASCII string and a second binding in the same body.
    ("(\"ab\" + \"c\") as $t | (\"de\" + \"\") as $u | [range(2) | . as $w | ($t | length) + ($u | length)]", "[5,5]\n"),
    // A stage after the length is the evaluator's.
    ("(\"ab\" + \"c\") as $t | [range(2) | . as $w | ($t | length) * 2]", "[6,6]\n"),
    // A user's own `length` is called, not shadowed by the shortcut.
    ("def length: 7; (\"ab\" + \"c\") as $t | [range(2) | . as $w | ($t | length)]", "[7,7]\n"),
    ("(\"ab\" + \"c\") as $t | [range(2) | . as $w | ($t | def length: 7; length)]", "[7,7]\n"),
    // Not a string: the evaluator answers, as before.
    ("([1, 2] + [3]) as $t | [range(2) | . as $w | ($t | length)]", "[3,3]\n"),
    ("({\"a\": 1} + {\"b\": 2}) as $t | [range(2) | . as $w | ($t | length)]", "[2,2]\n"),
];

#[test]
fn test_a_strings_length_in_a_loop_body_is_read_off_the_string_6534() -> Result<()> {
    for (filter, want) in ROWS {
        let (stdout, code) = run_yq_stdin(filter, "", ARGS)?;
        assert_eq!(code, 0, "exit code of {filter:?}");
        assert_eq!(&stdout, want, "{filter:?}");
    }
    Ok(())
}
