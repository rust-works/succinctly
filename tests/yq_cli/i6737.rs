//! #6737: the cursor streamers write a source plain scalar the way yq's emitter does when it cannot
//! stay plain: one that starts like a document marker (`---`, `... x`, `---x`) in single quotes, a
//! lone `-` inside `[...]` or `{...}` in single quotes, and one the source folded across blank
//! lines as a literal block (`|-`) in block context, where the streamers wrote a double-quoted
//! string with `\n` escapes. Every expected output is what yq v4.53.3 prints for the same input.

use anyhow::Result;

use crate::run_yq_stdin;

#[test]
fn test_a_plain_scalar_yq_quotes_or_blocks_is_written_as_yq_writes_it_6737() -> Result<()> {
    for (input, filter, expected) in [
        ("k: ---\n", ".", "k: '---'\n"),
        ("- ---\n", ".", "- '---'\n"),
        ("[---]\n", ".", "['---']\n"),
        ("{k: ---}\n", ".", "{k: '---'}\n"),
        ("a:\n  - k: ---\n", ".", "a:\n  - k: '---'\n"),
        ("k: --- x\n", ".", "k: '--- x'\n"),
        ("- --- x\n", ".", "- '--- x'\n"),
        ("[--- x]\n", ".", "['--- x']\n"),
        ("{k: --- x}\n", ".", "{k: '--- x'}\n"),
        ("a:\n  - k: --- x\n", ".", "a:\n  - k: '--- x'\n"),
        ("k: ---x\n", ".", "k: '---x'\n"),
        ("- ---x\n", ".", "- '---x'\n"),
        ("[---x]\n", ".", "['---x']\n"),
        ("{k: ---x}\n", ".", "{k: '---x'}\n"),
        ("a:\n  - k: ---x\n", ".", "a:\n  - k: '---x'\n"),
        ("k: ...\n", ".", "k: '...'\n"),
        ("- ...\n", ".", "- '...'\n"),
        ("[...]\n", ".", "['...']\n"),
        ("{k: ...}\n", ".", "{k: '...'}\n"),
        ("a:\n  - k: ...\n", ".", "a:\n  - k: '...'\n"),
        ("k: ... x\n", ".", "k: '... x'\n"),
        ("- ... x\n", ".", "- '... x'\n"),
        ("[... x]\n", ".", "['... x']\n"),
        ("{k: ... x}\n", ".", "{k: '... x'}\n"),
        ("a:\n  - k: ... x\n", ".", "a:\n  - k: '... x'\n"),
        ("k: ...x\n", ".", "k: '...x'\n"),
        ("- ...x\n", ".", "- '...x'\n"),
        ("[...x]\n", ".", "['...x']\n"),
        ("{k: ...x}\n", ".", "{k: '...x'}\n"),
        ("a:\n  - k: ...x\n", ".", "a:\n  - k: '...x'\n"),
        ("- -\n", ".", "- -\n"),
        ("[-]\n", ".", "['-']\n"),
        ("{k: -}\n", ".", "{k: '-'}\n"),
        ("k: a\n  \n    b\n", ".", "k: |-\n  a\n  b\n"),
        ("- a\n  \n    b\n", ".", "- |-\n  a\n  b\n"),
        ("k: a\n  \n  \n    b\n", ".", "k: |-\n  a\n\n  b\n"),
        ("- a\n  \n  \n    b\n", ".", "- |-\n  a\n\n  b\n"),
        (
            "k:\n  - a\n\n    b\n  - c\n",
            ".",
            "k:\n  - |-\n    a\n    b\n  - c\n",
        ),
        (
            "a: x\nb: first\n\n  second\n\n  third\nc: y\n",
            ".",
            "a: x\nb: |-\n  first\n  second\n  third\nc: y\n",
        ),
        ("a: ---\nb: 1\n", ".a", "---\n"),
        ("- ---\n- ...\n- a\n\n  b\n", ".[]", "---\n...\na\nb\n"),
        ("a: [---, '-', -]\n", ".a[]", "---\n-\n-\n"),
        (
            "a: ---\nb: [..., x]\n",
            ".. | select(tag==\"!!str\")",
            "---\n...\nx\n",
        ),
        ("# h\nk: a\n\n  b\n", ".", "# h\nk: |-\n  a\n  b\n"),
        ("# h\n- a\n\n  b\n# f\n", ".", "# h\n- |-\n  a\n  b\n# f\n"),
        (
            "k: a\n\n  b\n# foot\nz: 1\n",
            ".",
            "k: |-\n  a\n  b\n# foot\nz: 1\n",
        ),
        (
            "# lead\na: ---\n# between\nb: ...\n",
            ".",
            "# lead\na: '---'\n# between\nb: '...'\n",
        ),
    ] {
        let (output, exit_code) = run_yq_stdin(filter, input, &[])?;
        assert_eq!(exit_code, 0, "filter: {filter} on {input:?}");
        assert_eq!(output, expected, "filter: {filter} on {input:?}");
    }
    Ok(())
}

/// Rows of the same families that agree with yq (an anchor and a tag on the folded scalar, a wider indent,
/// a sibling flow collection), and the ones that keep the quoted spelling because the document has a
/// trailing same-line comment, where yq writes the comment on the block's header
/// (`docs/compliance/yq/limitations.md`): the second list is succinctly yq's own answer, not yq's.
#[test]
fn test_a_folded_plain_scalar_next_to_other_shapes_and_comments_6737() -> Result<()> {
    for (input, extra, expected) in [
        ("k: &a x\n\n  y\n", &[][..], "k: &a |-\n  x\n  y\n"),
        ("k: !!str x\n\n  y\n", &[][..], "k: !!str |-\n  x\n  y\n"),
        (
            "k:\n  n: x\n\n    y\n",
            &["-I4"][..],
            "k:\n    n: |-\n        x\n        y\n",
        ),
        ("- - x\n\n    y\n", &["-I4"][..], "- - |-\n    x\n    y\n"),
        (
            "k: [a, b]\nm: x\n\n  y\n",
            &[][..],
            "k: [a, b]\nm: |-\n  x\n  y\n",
        ),
        ("k: a\n  b # c\nz: 1\n", &[][..], "k: a b # c\nz: 1\n"),
    ] {
        let (output, exit_code) = run_yq_stdin(".", input, extra)?;
        assert_eq!(exit_code, 0, "{input:?} {extra:?}");
        assert_eq!(output, expected, "{input:?} {extra:?}");
    }
    for (input, expected) in [
        ("a: x\n\n  y\nb: 1 # c\n", "a: \"x\\ny\"\nb: 1 # c\n"),
        ("k: # c\n  a\n\n  b\n", "k: \"a\\nb\" # c\n"),
        ("k: a\n\n  b # c\n", "k: \"a\\nb\" # c\n"),
    ] {
        let (output, exit_code) = run_yq_stdin(".", input, &[])?;
        assert_eq!(exit_code, 0, "{input:?}");
        assert_eq!(output, expected, "{input:?}");
    }
    Ok(())
}
