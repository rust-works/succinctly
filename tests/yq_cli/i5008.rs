//! #5008: the head of the first document that is made of blank lines is kept by a write and by `-P`. yq slurps every
//! leading blank, comment, `%YAML` and `---` line and prints it back verbatim, so `\n\na: 1` keeps both blank lines
//! and `# h` / two blanks / `# g` keeps both between the comments. The comment index records no blank line before
//! the first comment and one at most between two, so the DOM route builds the head from the header's text (as it
//! does for a header with a CRLF break or a line of only blanks, #5053 and #6401). The head belongs to the root
//! node: a result that is a part of it (`.a`, `.[0]`) carries none. Every row was captured from yq v4.53.3.

use anyhow::Result;

use crate::run_yq_stdin;

#[test]
fn a_write_and_pretty_print_keep_the_blank_lines_of_the_head_5008() -> Result<()> {
    for (doc, filter, args, want) in [
        ("\na: 1\n", ".b = 2", &[][..], "\na: 1\nb: 2\n"),
        ("\na: 1\n", ".b = 2", &["-P"][..], "\na: 1\nb: 2\n"),
        ("\n\na: 1\n", ".b = 2", &[][..], "\n\na: 1\nb: 2\n"),
        ("\n\na: 1\n", ".b = 2", &["-P"][..], "\n\na: 1\nb: 2\n"),
        ("\n# c\na: 1\n", ".b = 2", &[][..], "\n# c\na: 1\nb: 2\n"),
        (
            "\n# c\na: 1\n",
            ".b = 2",
            &["-P"][..],
            "\n# c\na: 1\nb: 2\n",
        ),
        (
            "\n\n# c\n\na: 1\n",
            ".b = 2",
            &[][..],
            "\n\n# c\n\na: 1\nb: 2\n",
        ),
        (
            "\n\n# c\n\na: 1\n",
            ".b = 2",
            &["-P"][..],
            "\n\n# c\n\na: 1\nb: 2\n",
        ),
        (
            "# h\n\n\n# g\na: 1\n",
            ".b = 2",
            &[][..],
            "# h\n\n\n# g\na: 1\nb: 2\n",
        ),
        (
            "# h\n\n\n# g\na: 1\n",
            ".b = 2",
            &["-P"][..],
            "# h\n\n\n# g\na: 1\nb: 2\n",
        ),
        (
            "# h\n\n\n\n# g\na: 1\n",
            ".b = 2",
            &[][..],
            "# h\n\n\n\n# g\na: 1\nb: 2\n",
        ),
        (
            "# h\n\n\n\n# g\na: 1\n",
            ".b = 2",
            &["-P"][..],
            "# h\n\n\n\n# g\na: 1\nb: 2\n",
        ),
        ("\n- 1\n- 2\n", ". + [3]", &[][..], "\n- 1\n- 2\n- 3\n"),
        ("\n\n- 1\n", ".[0] = 9", &[][..], "\n\n- 9\n"),
        ("\na: 1\nb: 2\n", "del(.a)", &[][..], "\nb: 2\n"),
        ("\na:\n  b: 1\n", "sort_keys(.)", &[][..], "\na:\n  b: 1\n"),
        ("\nx\n", ".", &[][..], "\nx\n"),
        ("\n{a: 1}\n", ".b = 2", &[][..], "\n{a: 1, b: 2}\n"),
        ("\n{a: 1}\n", ".b = 2", &["-P"][..], "\na: 1\nb: 2\n"),
        ("\na: 1\n", ".a", &[][..], "1\n"),
        ("\n- 1\n- 2\n", ".[0]", &[][..], "1\n"),
        ("\n  \na: 1\n", ".b = 2", &[][..], "\n#   \na: 1\nb: 2\n"),
        (
            "\n  \na: 1\n",
            ".b = 2",
            &["-P"][..],
            "\n#   \na: 1\nb: 2\n",
        ),
        ("\n\n\na: 1\n", ".b = 2", &[][..], "\n\n\na: 1\nb: 2\n"),
        ("\n\n\na: 1\n", ".b = 2", &["-P"][..], "\n\n\na: 1\nb: 2\n"),
        (
            "\n\n# a\n# b\na: 1\n",
            ".b = 2",
            &[][..],
            "\n\n# a\n# b\na: 1\nb: 2\n",
        ),
        (
            "\n\n# a\n# b\na: 1\n",
            ".b = 2",
            &["-P"][..],
            "\n\n# a\n# b\na: 1\nb: 2\n",
        ),
    ] {
        let (stdout, code) = run_yq_stdin(filter, doc, args)?;
        assert_eq!(
            (stdout.as_str(), code),
            (want, 0),
            "{doc:?} {filter} {args:?}"
        );
    }
    Ok(())
}

#[test]
fn an_empty_input_is_not_a_blank_line_5008() -> Result<()> {
    // The empty document is made up; the line break it is read from is not the input's.
    let (out, code) = run_yq_stdin(".a = 1", "", &[])?;
    assert_eq!((out.as_str(), code), ("a: 1\n", 0));
    // A real line break is: yq keeps it.
    let (out, code) = run_yq_stdin(".a = 1", "\n", &[])?;
    assert_eq!((out.as_str(), code), ("\na: 1\n", 0));
    Ok(())
}
