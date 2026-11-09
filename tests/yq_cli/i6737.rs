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
