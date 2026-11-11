//! #6089: `(PATH | key) = value` and `(PATH | key) |= filter` rename the mapping key of the member
//! `PATH` names, in place, keeping the key's quoting, anchor and comments; `succinctly yq` accepted
//! both and printed the document unchanged. Every expected output is what yq v4.53.3 prints.

use anyhow::Result;

use crate::run_yq_stdin;

/// (document, filter, extra arguments, expected stdout).
const ROWS: &[(&str, &str, &[&str], &str)] = &[
    // A `select` over `key` in the target reaches every matching node (#6206).
    (
        "a: {x: 1, y: {x: 2}}\n",
        r#"(.. | select(key == "x") | key) = "q""#,
        &[],
        "a: {q: 1, y: {q: 2}}\n",
    ),
    (
        "a:\n  x: 3\n  y: 4\n",
        "(.a.x | key) = \"meow\"",
        &[],
        "a:\n  meow: 3\n  y: 4\n",
    ),
    (
        "a:\n  x: 3\n  y: 4\n",
        "(.a.x | key) |= \"m_\" + .",
        &[],
        "a:\n  m_x: 3\n  y: 4\n",
    ),
    (
        "a:\n  x: 3\n  y: 4\n",
        "(.a.x | key) = 5",
        &[],
        "a:\n  5: 3\n  y: 4\n",
    ),
    (
        "a:\n  x: 3\n  y: 4\n",
        "(.a.x | key) = null",
        &[],
        "a:\n  null: 3\n  y: 4\n",
    ),
    (
        "a:\n  x: 3\n  y: 4\n",
        "(.a.x | key) = \"\"",
        &[],
        "a:\n  \"\": 3\n  y: 4\n",
    ),
    (
        "a:\n  x: 3\n  y: 4\n",
        "(.a.x | key) = 1.50",
        &[],
        "a:\n  1.50: 3\n  y: 4\n",
    ),
    (
        "a:\n  x: 3\n  y: 4\n",
        "(.a.x | key) = true",
        &[],
        "a:\n  true: 3\n  y: 4\n",
    ),
    (
        "a:\n  x: 3\n  y: 4\n",
        "(.a.x | key) = \"a b\"",
        &[],
        "a:\n  a b: 3\n  y: 4\n",
    ),
    (
        "a:\n  x: 3\n  y: 4\n",
        "(.a.x | key) = \"1\"",
        &[],
        "a:\n  \"1\": 3\n  y: 4\n",
    ),
    (
        "a:\n  x: 3\n  y: 4\n",
        "(.a.x | key) = \"true\"",
        &[],
        "a:\n  \"true\": 3\n  y: 4\n",
    ),
    (
        "a:\n  x: 3\n  y: 4\n",
        "(.a.x | key) = \"~\"",
        &[],
        "a:\n  \"~\": 3\n  y: 4\n",
    ),
    (
        "a:\n  x: 3\n  y: 4\n",
        "(.a.x | key) = \"null\"",
        &[],
        "a:\n  \"null\": 3\n  y: 4\n",
    ),
    (
        "a:\n  x: 3\n  y: 4\n",
        "(.a.x | key) = \"# c\"",
        &[],
        "a:\n  '# c': 3\n  y: 4\n",
    ),
    (
        "a:\n  x: 3\n  y: 4\n",
        "(.a.x | key) = \"a: b\"",
        &[],
        "a:\n  'a: b': 3\n  y: 4\n",
    ),
    (
        "a:\n  x: 3\n  y: 4\n",
        "(.a.x | key) = \"- a\"",
        &[],
        "a:\n  '- a': 3\n  y: 4\n",
    ),
    (
        "a:\n  x: 3\n  y: 4\n",
        "(.a.x | key) |= . + \"1\"",
        &[],
        "a:\n  x1: 3\n  y: 4\n",
    ),
    (
        "a:\n  x: 3\n  y: 4\n",
        "(.a.x | key) |= length",
        &[],
        "a:\n  1: 3\n  y: 4\n",
    ),
    (
        "a:\n  x: 3\n  y: 4\n",
        "(.a.x | key) |= 7",
        &[],
        "a:\n  7: 3\n  y: 4\n",
    ),
    (
        "a:\n  x: 3\n  y: 4\n",
        "(.a.x | key) = .a.y",
        &[],
        "a:\n  4: 3\n  y: 4\n",
    ),
    (
        "a:\n  x: 3\n  y: 4\n",
        "(.a.x | key) = .zz",
        &[],
        "a:\n  x: 3\n  y: 4\n",
    ),
    (
        "a:\n  x: 3\n  y: 4\n",
        "(.a.x | key) |= .zz",
        &[],
        "a:\n  x: 3\n  y: 4\n",
    ),
    (
        "a:\n  x: 3\n  y: 4\n",
        ".a.y = 1 | (.a.x | key) = \"z\"",
        &[],
        "a:\n  z: 3\n  y: 1\n",
    ),
    (
        "a:\n  x: 3\n  y: 4\n",
        "(.a.x | key) = \"z\" | .a.y = 1",
        &[],
        "a:\n  z: 3\n  y: 1\n",
    ),
    (
        "a:\n  x: 3\n  y: 4\n",
        "(.a.x | key) = \"z\" | .a | keys",
        &[],
        "- z\n- y\n",
    ),
    (
        "a:\n  x: 3\n  y: 4\n",
        "(.a.x | key) = \"z\" | (.a.y | key) = \"w\"",
        &[],
        "a:\n  z: 3\n  w: 4\n",
    ),
    (
        "a:\n  x: 3\n  y: 4\n",
        "(.a.x | key) = \"z\" | (.a.z | key) = \"w\"",
        &[],
        "a:\n  w: 3\n  y: 4\n",
    ),
    (
        "a:\n  x: 3\n  y: 4\n",
        "(.a.x | key) = \"z\" | .a.z",
        &[],
        "3\n",
    ),
    (
        "a:\n  x: 3\n  y: 4\n",
        ".a | (.x | key) = \"z\"",
        &[],
        "z: 3\ny: 4\n",
    ),
    (
        "a:\n  x: 3\n  y: 4\n",
        ". as $d | (.a.x | key) = \"z\"",
        &[],
        "a:\n  z: 3\n  y: 4\n",
    ),
    (
        "a:\n  x: 3\n  y: 4\n",
        "(.a[] | key) |= \"k_\" + .",
        &[],
        "a:\n  k_x: 3\n  k_y: 4\n",
    ),
    (
        "a:\n  x: 3\n  y: 4\n",
        "(.a | key) = \"A\"",
        &[],
        "A:\n  x: 3\n  y: 4\n",
    ),
    (
        "a:\n  x: 3\n  y: 4\n",
        "(.a.zz | key) = \"q\"",
        &[],
        "a:\n  x: 3\n  y: 4\n  q: null\n",
    ),
    (
        "a:\n  x: 3\n  y: 4\n",
        "(.a.x | key) = (1,2)",
        &[],
        "a:\n  2: 3\n  y: 4\n",
    ),
    (
        "a:\n  x: 3\n  y: 4\n",
        "(.a.x | key) |= (1,2)",
        &[],
        "a:\n  1: 3\n  y: 4\n",
    ),
    (
        "a:\n  x: 3\n  y: 4\n",
        "(.a.x | key) = \"x\"",
        &[],
        "a:\n  x: 3\n  y: 4\n",
    ),
    ("- a\n- b\n", "(.[0] | key) = \"z\"", &[], "- a\n- b\n"),
    ("- a\n- b\n", "(.[0] | key) |= \"z\"", &[], "- a\n- b\n"),
    (
        "x: 1\n---\nx: 2\ny: 3\n",
        "(.x | key) = \"z\"",
        &[],
        "z: 1\n---\nz: 2\ny: 3\n",
    ),
    (
        "a: [{p: 1}, {p: 2}]\n",
        "(.a[].p | key) = \"q\"",
        &[],
        "a: [{q: 1}, {q: 2}]\n",
    ),
    (
        "a: [{p: 1}, {p: 2}]\n",
        "(.a[0] | key) = \"q\"",
        &[],
        "a: [{p: 1}, {p: 2}]\n",
    ),
    (
        "a: [{p: 1}, {p: 2}]\n",
        "(.a | key) = \"q\"",
        &[],
        "q: [{p: 1}, {p: 2}]\n",
    ),
    (
        "a: {x: 3}\n",
        "(.a.x | key) = \"z\"",
        &["-o=json", "-I=0"],
        "{\"a\":{\"z\":3}}\n",
    ),
    (
        "a: {x: 1}\n",
        "(.a.x | key) = 7",
        &["-o=json", "-I=0"],
        "{\"a\":{\"7\":1}}\n",
    ),
    (
        "a: &x 1\nb: 2 # c\n# head\nc: 3\n",
        "(.b | key) = \"B\"",
        &[],
        "a: &x 1\nB: 2 # c\n# head\nc: 3\n",
    ),
    (
        "a: &x 1\nb: 2 # c\n# head\nc: 3\n",
        "(.c | key) = \"B\"",
        &[],
        "a: &x 1\nb: 2 # c\n# head\nB: 3\n",
    ),
    (
        "&k a: 1\n\"b\": 2\n",
        "(.a | key) = \"A\"",
        &[],
        "&k A: 1\n\"b\": 2\n",
    ),
    (
        "&k a: 1\n\"b\": 2\n",
        "(.b | key) = \"B\"",
        &[],
        "&k a: 1\n\"B\": 2\n",
    ),
    (
        "a:\n  x: 3 # c\n  # foot\n  y: 4\n",
        "(.a.x | key) = \"z\"",
        &[],
        "a:\n  z: 3 # c\n  # foot\n  y: 4\n",
    ),
    (
        "a:\n  # head\n  x: 3 # c\n  y: 4\n",
        "(.a.x | key) = \"z\"",
        &[],
        "a:\n  # head\n  z: 3 # c\n  y: 4\n",
    ),
    (
        "a:\n  \"x\": 3\n  'y': 4\n  ? z\n  : 5\n",
        "(.a.[] | key) |= \"k_\" + .",
        &[],
        "a:\n  \"k_x\": 3\n  'k_y': 4\n  k_z: 5\n",
    ),
    (
        "a:\n  \"x\": 3\n  'y': 4\n",
        "(.a.x | key) = \"n\"",
        &[],
        "a:\n  \"n\": 3\n  'y': 4\n",
    ),
    (
        "a:\n  \"x\": 3\n  'y': 4\n",
        "(.a.x | key) = 5",
        &[],
        "a:\n  5: 3\n  'y': 4\n",
    ),
    (
        "a:\n  \"x\": 3\n  'y': 4\n",
        "(.a.y | key) = \"1\"",
        &[],
        "a:\n  \"x\": 3\n  '1': 4\n",
    ),
    (
        "a: {x: 3}\n",
        "(.a.x | key) = \"1\"",
        &[],
        "a: {\"1\": 3}\n",
    ),
    ("a: {x: 3}\n", "(.a.x | key) = 1", &[], "a: {1: 3}\n"),
    (
        "a: {x: 3}\nb: 1\n",
        "(.a.x | key) = \"z\" | .b = 2",
        &[],
        "a: {z: 3}\nb: 2\n",
    ),
    (
        "a:\n  x: {p: 1} # xc\n  y: [1, 2]\n",
        "(.a.x | key) = \"z\"",
        &[],
        "a:\n  z: {p: 1} # xc\n  y: [1, 2]\n",
    ),
    (
        "a:\n  x: |\n    lit\n  y: 2\n",
        "(.a.x | key) = \"z\"",
        &[],
        "a:\n  z: |\n    lit\n  y: 2\n",
    ),
];

#[test]
fn test_key_assignment_renames_the_key_as_yq_does_6089() -> Result<()> {
    for &(document, filter, args, expected) in ROWS {
        let (output, code) = run_yq_stdin(filter, document, args)?;
        assert_eq!(code, 0, "{filter} on {document:?}");
        assert_eq!(output, expected, "{filter} on {document:?}");
    }
    Ok(())
}

/// What `succinctly yq` refuses where a rename cannot be written as yq writes it: a name that is
/// already a member (yq keeps both entries), a mapping or sequence, and a text with a line break
/// (yq writes a complex `? ` key). Each is an error, not a silent no-op.
#[test]
fn test_key_assignment_refuses_what_it_cannot_write_6089() -> Result<()> {
    for (document, filter, message) in [
        (
            "a: 1\nb: 2\n",
            r#"(.a | key) = "b""#,
            "cannot rename key 'a' to 'b': the mapping already has that key",
        ),
        (
            "a:\n  x: 3\n  y: 4\n",
            r#"(.a[] | key) = "k""#,
            "to 'k': the mapping already has that key",
        ),
        (
            "a:\n  x: 3\n",
            "(.a.x | key) = []",
            "a mapping key cannot be a mapping or a sequence",
        ),
        (
            "a:\n  x: 3\n",
            "(.a.x | key) = {}",
            "a mapping key cannot be a mapping or a sequence",
        ),
        (
            "a:\n  x: 3\n",
            r#"(.a.x | key) = "a\nb""#,
            "a mapping key with a line break is not supported",
        ),
    ] {
        let (stdout, stderr, code) = crate::run_yq_stdin_with_stderr(filter, document, &[])?;
        assert_ne!(code, 0, "{filter}: stdout={stdout:?}");
        assert!(stderr.contains(message), "{filter}: {stderr:?}");
    }
    Ok(())
}

/// An operator assignment to a key target was a silent no-op; it is refused (yq supports `+=`
/// there and errors for the others, which this does not model).
#[test]
fn test_key_operator_assignment_is_refused_6089() -> Result<()> {
    for filter in [
        r#"(.a.x | key) += "z""#,
        r#"(.a.x | key) -= "z""#,
        r#"(.a.x | key) *= "z""#,
        r#"(.a.x | key) //= "z""#,
    ] {
        // `//=` is not yq's operator, so it needs the flag to reach this refusal (#6981).
        let args: &[&str] = if filter.contains("//=") {
            &["--jq-extensions"]
        } else {
            &[]
        };
        let (stdout, stderr, code) = crate::run_yq_stdin_with_stderr(filter, "a: {x: 1}\n", args)?;
        assert_ne!(code, 0, "{filter}: stdout={stdout:?}");
        assert!(
            stderr.contains("(PATH | key) can only be assigned with = or |="),
            "{filter}: {stderr:?}"
        );
    }
    Ok(())
}

/// Where the rename is not as yq's (the first two columns are this tool's answer, pinned so the
/// limitations page stays true; the last is yq v4.53.3's).
const GAPS: &[(&str, &str, &str, &str)] = &[
    // A key is text here, so a number keeps no type once the mapping is read again.
    (
        "a: {x: 1}\n",
        "(.a.x | key) = 7 | .a | keys",
        "- \"7\"\n",
        "- 7\n",
    ),
    // A name computed by a filter the presentation replay does not read is a plain key.
    (
        "a:\n  x: 3\n  y: 4\n",
        "(.a.x | key) = (.a.y | tostring)",
        "a:\n  4: 3\n  y: 4\n",
        "a:\n  \"4\": 3\n  y: 4\n",
    ),
];

#[test]
fn test_key_assignment_gaps_still_differ_from_yq_6089() -> Result<()> {
    for &(document, filter, ours, yq) in GAPS {
        let (output, code) = run_yq_stdin(filter, document, &[])?;
        assert_eq!(code, 0, "{filter} on {document:?}");
        assert_eq!(output, ours, "{filter} on {document:?}");
        assert_ne!(
            ours, yq,
            "{filter}: no longer a gap, drop it from the table"
        );
    }
    Ok(())
}

/// Members renamed together are checked as a whole: names that swap are fine, though renaming one
/// at a time would meet the other's old name. yq has no `if`, so this row has no oracle; it pins the
/// mapping being rebuilt once per parent.
#[test]
fn test_key_assignment_swapping_two_names_is_not_a_collision_6089() -> Result<()> {
    let (output, code) = run_yq_stdin(
        r#"(.a[] | key) |= (if . == "x" then "y" else "x" end)"#,
        "a: {x: 1, y: 2}\n",
        &["--jq-extensions"],
    )?;
    assert_eq!(code, 0);
    assert_eq!(output, "a: {y: 1, x: 2}\n");
    Ok(())
}
