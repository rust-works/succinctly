//! #6833: in yq mode a call to a user `def` with no parameters whose body is a string's `length`
//! is read off the string in a loop body, as the inline `length` is (#6534). The `def` and the
//! `range` loop are `--jq-extensions` surface, so these rows have no yq capture; each answer is
//! what the build before the change printed (it reindexed the string once per round), so the rows
//! pin that the shortcut changes the speed and not the answer. Each row binds `. as $w |` ahead
//! of the string, because a bare `$t` stage after a fan-out answers once in yq (#6821).

use anyhow::Result;

use crate::run_yq_stdin;

const ARGS: &[&str] = &["--jq-extensions", "-n", "-o=json", "-I=0"];

/// `(filter, output)`.
const ROWS: &[(&str, &str)] = &[
    // A computed multibyte string counts in bytes, every round, through the call.
    (
        "def f: length; (\"héllo😀\" + \"x\") as $t | [range(3) | . as $w | ($t | f)]",
        "[11,11,11]\n",
    ),
    (
        "def f: length; (\"ab\" + \"c\") as $t | [range(2) | . as $w | $t | f]",
        "[3,3]\n",
    ),
    (
        "def f: (length); (\"ab\" + \"c\") as $t | [range(2) | . as $w | ($t | f)]",
        "[3,3]\n",
    ),
    (
        "def f: length; (\"\" + \"\") as $t | [range(2) | . as $w | ($t | f)]",
        "[0,0]\n",
    ),
    // A body that is more than the builtin, a parameter, and a call to another def are the evaluator's.
    (
        "def f: length + 1; (\"ab\" + \"c\") as $t | [range(2) | . as $w | ($t | f)]",
        "[4,4]\n",
    ),
    (
        "def f(x): length; (\"ab\" + \"c\") as $t | [range(2) | . as $w | ($t | f(1))]",
        "[3,3]\n",
    ),
    (
        "def g: length; def f: g; (\"ab\" + \"c\") as $t | [range(2) | . as $w | ($t | f)]",
        "[3,3]\n",
    ),
    // A user's own `length` is what the body calls, not the builtin.
    (
        "def length: 7; def f: length; (\"ab\" + \"c\") as $t | [range(2) | . as $w | ($t | f)]",
        "[7,7]\n",
    ),
    // Not a string: the evaluator answers, as before.
    (
        "def f: length; ([1, 2] + [3]) as $t | [range(2) | . as $w | ($t | f)]",
        "[3,3]\n",
    ),
    (
        "def f: length; ({\"a\": 1} + {\"b\": 2}) as $t | [range(2) | . as $w | ($t | f)]",
        "[2,2]\n",
    ),
];

#[test]
fn test_a_def_call_of_a_strings_length_in_a_yq_range_body_is_read_off_the_string_6833() -> Result<()>
{
    for (filter, want) in ROWS {
        let (stdout, code) = run_yq_stdin(filter, "", ARGS)?;
        assert_eq!(code, 0, "exit code of {filter:?}");
        assert_eq!(&stdout, want, "{filter:?}");
    }
    Ok(())
}
