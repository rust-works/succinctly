//! #5712: a string's `length`, `startswith` and `endswith` are answered from the string where
//! they are composed -- a leading stage the rest runs on, a `select` condition, an `until` or
//! `while` condition -- as they are as a lone last stage (#5691). Each row is a filter over
//! `--arg s <value>` and the answer a live jq 1.7.1 gives: `length` counts codepoints, the
//! prefix and suffix tests compare bytes.

use anyhow::Result;

use crate::run_jq_full;

const ROWS: &[(&str, &str, &str)] = &[
    ("h\u{e9}llo", "$s|length|tostring", "\"5\"\n"),
    ("h\u{e9}llo", "$s|length|.+1", "6\n"),
    ("h\u{e9}llo", "$s|select(length>3)|length", "5\n"),
    ("h\u{e9}llo", "$s|select(length>30)|length", ""),
    // The threshold sits between the codepoint count (5) and the byte count (6).
    ("h\u{e9}llo", "$s|select(length>5)|length", ""),
    ("h\u{e9}llo", "$s|select(length>4)|length", "5\n"),
    ("\u{1f600}x\u{1f600}", "$s|length|tostring", "\"3\"\n"),
    ("", "$s|select(length==0)|\"empty\"", "\"empty\"\n"),
    // jq orders every pair (a number sorts before a string), so the comparison does not raise:
    // `3 < "a"` is true and `3 > "a"` is false.
    ("abc", "$s|select(length<\"a\")|length", "3\n"),
    ("abc", "$s|select(length>\"a\")|length", ""),
    ("zzzabc", "$s|startswith(\"zzz\")|not", "false\n"),
    ("zzzabc", "$s|endswith(\"abc\")|tostring", "\"true\"\n"),
    ("zzzabc", "$s|select(startswith(\"zzz\"))|length", "6\n"),
    ("zzzabc", "$s|(startswith(\"zzz\")|tostring)", "\"true\"\n"),
    ("", "$s|until(length>=5;.+\"x\")|length", "5\n"),
    // `\u{e9}` is one codepoint, so jq stops at 5 where a byte count would stop at 3 rounds.
    ("", "$s|until(length>=5;.+\"\u{e9}\")|length", "5\n"),
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

/// The shapes the doors decline keep jq's own error: a non-string argument to `startswith`
/// raises (exit 5) whether it is composed or not.
#[test]
fn a_non_string_prefix_still_raises_when_composed_5712() -> Result<()> {
    for filter in [
        "$s|startswith(1)|not",
        "$s|select(startswith(1))|length",
        "$s|length|tostring|startswith(1)",
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-n", "--arg", "s", "abc", filter], None)?;
        assert_eq!(stdout, "", "{filter}");
        assert!(
            stderr.contains("startswith() requires string inputs"),
            "{filter}: {stderr}"
        );
        assert_eq!(code, 5, "{filter}");
    }
    Ok(())
}

/// A condition the door now answers is no longer refused at the value-tree depth limit (384):
/// the bridge wrote the whole value out and refused it (`nesting depth exceeds limit of 384`),
/// where jq answers. Every depth gives jq 1.7.1's answer, 533 once the value is deeper than jq
/// prints (it elides past 256 levels), including the depths 384 and 500 the bridge refused.
#[test]
fn a_string_condition_over_a_deep_value_answers_as_jq_does_5712() -> Result<()> {
    for (depth, want) in [
        (100, "205\n"),
        (300, "533\n"),
        (383, "533\n"),
        (384, "533\n"),
        (500, "533\n"),
    ] {
        let filter = format!(
            "def f(n): if n==0 then \"xyz\" else [f(n-1)] end; \
             f({depth}) | select(tostring | length > 5) | tostring | length"
        );
        let (stdout, stderr, code) = run_jq_full(&["-n", &filter], None)?;
        assert_eq!(
            (stdout.as_str(), stderr.as_str(), code),
            (want, "", 0),
            "depth {depth}"
        );
    }
    Ok(())
}

/// The residual shapes: a comparison of the length, `try`/`?`, a `range` body, a `reduce`, a
/// `def`, and a `select` or `if` over a string inside `[...]`, `//` or an object value. Each
/// answer is jq 1.7.1's (`length` counts codepoints; the threshold sits between the codepoint
/// count, 5, and the byte count, 6).
#[test]
fn the_residual_composed_string_length_shapes_answer_as_jq_does_5712() -> Result<()> {
    const RESIDUAL: &[(&str, &str)] = &[
        ("$s|length>5", "false\n"),
        ("$s|length>4", "true\n"),
        ("$s|length>5|not", "true\n"),
        ("$s|(length>4 and length<6)", "true\n"),
        ("$s|(length<5 or startswith(\"h\"))", "true\n"),
        ("$s|try length", "5\n"),
        ("$s|length?", "5\n"),
        ("$s|try (length>4) catch \"c\"", "true\n"),
        ("$s|length>\"a\"", "false\n"),
        ("[range(3)|($s|length)]|tojson", "\"[5,5,5]\"\n"),
        ("$s|reduce range(3) as $i (0; . + ($s|length))", "15\n"),
        ("$s|def f: length; f", "5\n"),
        ("[$s|select(length>4)]|length", "1\n"),
        ("[$s|select(length>5)]|length", "0\n"),
        ("[$s|if length>4 then 1 else 0 end]|tojson", "\"[1]\"\n"),
        ("[$s|if length>5 then 1 else 0 end]|tojson", "\"[0]\"\n"),
        ("$s|(select(length>5)|length)//0", "0\n"),
        ("$s|(select(length>4)|length)//0", "5\n"),
        ("{a:($s|select(length>4)|length)}|.a", "5\n"),
        ("[\"x\",$s]|map(select(length>4))|length", "1\n"),
        // A condition with two outputs forks the branch, as it does for a number.
        (
            "[$s|if (true,false) then 1 else 2 end]|tojson",
            "\"[1,2]\"\n",
        ),
        // A right operand that would raise is never read when the left decides.
        ("$s|(length>100 and .a)", "false\n"),
        ("$s|(length<100 or .a)", "true\n"),
    ];
    for (filter, expected) in RESIDUAL {
        let (stdout, stderr, code) =
            run_jq_full(&["-n", "--arg", "s", "h\u{e9}llo", filter], None)?;
        assert_eq!(
            (stdout.as_str(), stderr.as_str(), code),
            (*expected, "", 0),
            "{filter}"
        );
    }
    Ok(())
}

/// A comparison the door cannot settle keeps jq's own error: indexing a string raises whether the
/// comparison sits in `try` or not.
#[test]
fn a_comparison_that_raises_still_raises_over_a_string_5712() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(&["-n", "--arg", "s", "abc", "$s|.a>1"], None)?;
    assert_eq!((stdout.as_str(), code), ("", 5), "{stderr}");
    assert!(stderr.contains("Cannot index string"), "{stderr}");
    let (stdout, stderr, code) = run_jq_full(
        &["-n", "--arg", "s", "abc", "$s|try (.a>1) catch \"c\""],
        None,
    )?;
    assert_eq!((stdout.as_str(), stderr.as_str(), code), ("\"c\"\n", "", 0));
    Ok(())
}
