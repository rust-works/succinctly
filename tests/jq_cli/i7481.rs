//! #7481: a `-f` program file that opens with the shell/jq polyglot
//!
//! ```text
//! #!/bin/sh
//! # \
//! exec jq -n -f "$0" "$@"
//! 1+2
//! ```
//!
//! runs in jq 1.7.1, which drops the first three lines of such a file (`skip_shebang` in its
//! `src/main.c`; it applies to the text of a `-f` file and to nothing else). `succinctly jq` ended
//! the comment at its newline and parsed `exec jq ...` as the program. The rule is exact, so it is
//! pinned over a generated matrix captured from jq 1.7.1: a shebang line or none, a second line
//! with 0 to 4 trailing backslashes (and the other shapes of a second line), the third line, CRLF
//! line endings, a program or none behind it, and a continuation on later lines only.
//!
//! jq 1.8.2 runs the polyglot too, but by a language change (Tcl-style comment continuation
//! everywhere, jqlang/jq#2989), so there `# \` continues a comment in any program. That is NOT
//! followed: `1` / `# \` / `+2` is `3` in jq 1.7.1, and the rows with a continuation outside the
//! shebang rule keep that answer here.

use anyhow::Result;

use crate::run_jq_full;

/// (the program file, jq 1.7.1's stdout for `jq -n -c -f file`, its exit status).
const ROWS: &[(&str, &str, i32)] = &[
    (
        "#!/bin/sh\n# plain\nexec jq -n -f \"$0\" \"$@\"\n1+2\n",
        "",
        3,
    ),
    (
        "#!/bin/sh\r\n# plain\r\nexec jq -n -f \"$0\" \"$@\"\r\n1+2\n",
        "",
        3,
    ),
    (
        "#!/bin/sh\n# \\\nexec jq -n -f \"$0\" \"$@\"\n1+2\n",
        "3\n",
        0,
    ),
    (
        "#!/bin/sh\n# \\\nexec jq -n -f \"$0\" \"$@\"\n1+2",
        "3\n",
        0,
    ),
    ("#!/bin/sh\n# \\\nexec jq -n -f \"$0\" \"$@\"\n\n", "", 3),
    ("#!/bin/sh\n# \\\nexec jq -n -f \"$0\" \"$@\"\n", "", 3),
    (
        "#!/bin/sh\n# \\\nexec jq -n -f \"$0\" \"$@\"\n[1,\n2]\n",
        "[1,2]\n",
        0,
    ),
    (
        "#!/bin/sh\r\n# \\\r\nexec jq -n -f \"$0\" \"$@\"\r\n1+2\n",
        "",
        3,
    ),
    (
        "#!/bin/sh\r\n# \\\r\nexec jq -n -f \"$0\" \"$@\"\r\n1+2",
        "",
        3,
    ),
    (
        "#!/bin/sh\r\n# \\\r\nexec jq -n -f \"$0\" \"$@\"\r\n\n",
        "",
        3,
    ),
    (
        "#!/bin/sh\r\n# \\\r\nexec jq -n -f \"$0\" \"$@\"\r\n",
        "",
        3,
    ),
    (
        "#!/bin/sh\r\n# \\\r\nexec jq -n -f \"$0\" \"$@\"\r\n[1,\n2]\n",
        "",
        3,
    ),
    ("#!/bin/sh\n# \\\n# \\\n1+2\n", "3\n", 0),
    ("#!/bin/sh\r\n# \\\r\n# \\\r\n1+2\n", "3\n", 0),
    ("#!/bin/sh\n# \\\n#c\n1+2\n", "3\n", 0),
    ("#!/bin/sh\r\n# \\\r\n#c\r\n1+2\n", "3\n", 0),
    ("#!/bin/sh\n# \\\n\n1+2\n", "3\n", 0),
    ("#!/bin/sh\r\n# \\\r\n\r\n1+2\n", "", 3),
    ("#!/bin/sh\n# \\\n1\n1+2\n", "3\n", 0),
    ("#!/bin/sh\r\n# \\\r\n1\r\n1+2\n", "", 3),
    (
        "#!/bin/sh\n# \\\\\nexec jq -n -f \"$0\" \"$@\"\n1+2\n",
        "",
        3,
    ),
    ("#!/bin/sh\n# \\\\\nexec jq -n -f \"$0\" \"$@\"\n1+2", "", 3),
    ("#!/bin/sh\n# \\\\\nexec jq -n -f \"$0\" \"$@\"\n\n", "", 3),
    ("#!/bin/sh\n# \\\\\nexec jq -n -f \"$0\" \"$@\"\n", "", 3),
    (
        "#!/bin/sh\n# \\\\\nexec jq -n -f \"$0\" \"$@\"\n[1,\n2]\n",
        "",
        3,
    ),
    (
        "#!/bin/sh\r\n# \\\\\r\nexec jq -n -f \"$0\" \"$@\"\r\n1+2\n",
        "",
        3,
    ),
    (
        "#!/bin/sh\r\n# \\\\\r\nexec jq -n -f \"$0\" \"$@\"\r\n1+2",
        "",
        3,
    ),
    (
        "#!/bin/sh\r\n# \\\\\r\nexec jq -n -f \"$0\" \"$@\"\r\n\n",
        "",
        3,
    ),
    (
        "#!/bin/sh\r\n# \\\\\r\nexec jq -n -f \"$0\" \"$@\"\r\n",
        "",
        3,
    ),
    (
        "#!/bin/sh\r\n# \\\\\r\nexec jq -n -f \"$0\" \"$@\"\r\n[1,\n2]\n",
        "",
        3,
    ),
    (
        "#!/bin/sh\n# \\\\\\\nexec jq -n -f \"$0\" \"$@\"\n1+2\n",
        "",
        3,
    ),
    (
        "#!/bin/sh\n# \\\\\\\nexec jq -n -f \"$0\" \"$@\"\n1+2",
        "",
        3,
    ),
    (
        "#!/bin/sh\n# \\\\\\\nexec jq -n -f \"$0\" \"$@\"\n\n",
        "",
        3,
    ),
    ("#!/bin/sh\n# \\\\\\\nexec jq -n -f \"$0\" \"$@\"\n", "", 3),
    (
        "#!/bin/sh\n# \\\\\\\nexec jq -n -f \"$0\" \"$@\"\n[1,\n2]\n",
        "",
        3,
    ),
    (
        "#!/bin/sh\r\n# \\\\\\\r\nexec jq -n -f \"$0\" \"$@\"\r\n1+2\n",
        "",
        3,
    ),
    (
        "#!/bin/sh\r\n# \\\\\\\r\nexec jq -n -f \"$0\" \"$@\"\r\n1+2",
        "",
        3,
    ),
    (
        "#!/bin/sh\r\n# \\\\\\\r\nexec jq -n -f \"$0\" \"$@\"\r\n\n",
        "",
        3,
    ),
    (
        "#!/bin/sh\r\n# \\\\\\\r\nexec jq -n -f \"$0\" \"$@\"\r\n",
        "",
        3,
    ),
    (
        "#!/bin/sh\r\n# \\\\\\\r\nexec jq -n -f \"$0\" \"$@\"\r\n[1,\n2]\n",
        "",
        3,
    ),
    (
        "#!/bin/sh\n# \\\\\\\\\nexec jq -n -f \"$0\" \"$@\"\n1+2\n",
        "",
        3,
    ),
    (
        "#!/bin/sh\r\n# \\\\\\\\\r\nexec jq -n -f \"$0\" \"$@\"\r\n1+2\n",
        "",
        3,
    ),
    (
        "#!/bin/sh\n#\\\nexec jq -n -f \"$0\" \"$@\"\n1+2\n",
        "3\n",
        0,
    ),
    ("#!/bin/sh\n#\\\nexec jq -n -f \"$0\" \"$@\"\n1+2", "3\n", 0),
    ("#!/bin/sh\n#\\\nexec jq -n -f \"$0\" \"$@\"\n\n", "", 3),
    ("#!/bin/sh\n#\\\nexec jq -n -f \"$0\" \"$@\"\n", "", 3),
    (
        "#!/bin/sh\n#\\\nexec jq -n -f \"$0\" \"$@\"\n[1,\n2]\n",
        "[1,2]\n",
        0,
    ),
    (
        "#!/bin/sh\r\n#\\\r\nexec jq -n -f \"$0\" \"$@\"\r\n1+2\n",
        "",
        3,
    ),
    (
        "#!/bin/sh\r\n#\\\r\nexec jq -n -f \"$0\" \"$@\"\r\n1+2",
        "",
        3,
    ),
    (
        "#!/bin/sh\r\n#\\\r\nexec jq -n -f \"$0\" \"$@\"\r\n\n",
        "",
        3,
    ),
    ("#!/bin/sh\r\n#\\\r\nexec jq -n -f \"$0\" \"$@\"\r\n", "", 3),
    (
        "#!/bin/sh\r\n#\\\r\nexec jq -n -f \"$0\" \"$@\"\r\n[1,\n2]\n",
        "",
        3,
    ),
    (
        "#!/bin/sh\n#\\\\\nexec jq -n -f \"$0\" \"$@\"\n1+2\n",
        "",
        3,
    ),
    (
        "#!/bin/sh\r\n#\\\\\r\nexec jq -n -f \"$0\" \"$@\"\r\n1+2\n",
        "",
        3,
    ),
    ("#!/bin/sh\n#\nexec jq -n -f \"$0\" \"$@\"\n1+2\n", "", 3),
    (
        "#!/bin/sh\r\n#\r\nexec jq -n -f \"$0\" \"$@\"\r\n1+2\n",
        "",
        3,
    ),
    ("#!/bin/sh\nx\\\nexec jq -n -f \"$0\" \"$@\"\n1+2\n", "", 3),
    (
        "#!/bin/sh\r\nx\\\r\nexec jq -n -f \"$0\" \"$@\"\r\n1+2\n",
        "",
        3,
    ),
    (
        "#!/bin/sh\n # \\\nexec jq -n -f \"$0\" \"$@\"\n1+2\n",
        "",
        3,
    ),
    (
        "#!/bin/sh\r\n # \\\r\nexec jq -n -f \"$0\" \"$@\"\r\n1+2\n",
        "",
        3,
    ),
    ("#!/bin/sh\n\nexec jq -n -f \"$0\" \"$@\"\n1+2\n", "", 3),
    (
        "#!/bin/sh\r\n\r\nexec jq -n -f \"$0\" \"$@\"\r\n1+2\n",
        "",
        3,
    ),
    ("#!\n# plain\nexec jq -n -f \"$0\" \"$@\"\n1+2\n", "", 3),
    (
        "#!\r\n# plain\r\nexec jq -n -f \"$0\" \"$@\"\r\n1+2\n",
        "",
        3,
    ),
    ("#!\n# \\\nexec jq -n -f \"$0\" \"$@\"\n1+2\n", "3\n", 0),
    ("#!\r\n# \\\r\nexec jq -n -f \"$0\" \"$@\"\r\n1+2\n", "", 3),
    ("#!\n# \\\\\nexec jq -n -f \"$0\" \"$@\"\n1+2\n", "", 3),
    (
        "#!\r\n# \\\\\r\nexec jq -n -f \"$0\" \"$@\"\r\n1+2\n",
        "",
        3,
    ),
    ("#!\n# \\\\\\\nexec jq -n -f \"$0\" \"$@\"\n1+2\n", "", 3),
    (
        "#!\r\n# \\\\\\\r\nexec jq -n -f \"$0\" \"$@\"\r\n1+2\n",
        "",
        3,
    ),
    ("#!\n# \\\\\\\\\nexec jq -n -f \"$0\" \"$@\"\n1+2\n", "", 3),
    (
        "#!\r\n# \\\\\\\\\r\nexec jq -n -f \"$0\" \"$@\"\r\n1+2\n",
        "",
        3,
    ),
    ("#!\n#\\\nexec jq -n -f \"$0\" \"$@\"\n1+2\n", "3\n", 0),
    ("#!\r\n#\\\r\nexec jq -n -f \"$0\" \"$@\"\r\n1+2\n", "", 3),
    ("#!\n#\\\\\nexec jq -n -f \"$0\" \"$@\"\n1+2\n", "", 3),
    ("#!\r\n#\\\\\r\nexec jq -n -f \"$0\" \"$@\"\r\n1+2\n", "", 3),
    ("#!\n#\nexec jq -n -f \"$0\" \"$@\"\n1+2\n", "", 3),
    ("#!\r\n#\r\nexec jq -n -f \"$0\" \"$@\"\r\n1+2\n", "", 3),
    ("#!\nx\\\nexec jq -n -f \"$0\" \"$@\"\n1+2\n", "", 3),
    ("#!\r\nx\\\r\nexec jq -n -f \"$0\" \"$@\"\r\n1+2\n", "", 3),
    ("#!\n # \\\nexec jq -n -f \"$0\" \"$@\"\n1+2\n", "", 3),
    ("#!\r\n # \\\r\nexec jq -n -f \"$0\" \"$@\"\r\n1+2\n", "", 3),
    ("#!\n\nexec jq -n -f \"$0\" \"$@\"\n1+2\n", "", 3),
    ("#!\r\n\r\nexec jq -n -f \"$0\" \"$@\"\r\n1+2\n", "", 3),
    ("# plain\nexec jq -n -f \"$0\" \"$@\"\n1+2\n", "", 3),
    ("# plain\r\nexec jq -n -f \"$0\" \"$@\"\r\n1+2\n", "", 3),
    ("# \\\nexec jq -n -f \"$0\" \"$@\"\n1+2\n", "", 3),
    ("# \\\r\nexec jq -n -f \"$0\" \"$@\"\r\n1+2\n", "", 3),
    ("# \\\\\nexec jq -n -f \"$0\" \"$@\"\n1+2\n", "", 3),
    ("# \\\\\r\nexec jq -n -f \"$0\" \"$@\"\r\n1+2\n", "", 3),
    ("# \\\\\\\nexec jq -n -f \"$0\" \"$@\"\n1+2\n", "", 3),
    ("# \\\\\\\r\nexec jq -n -f \"$0\" \"$@\"\r\n1+2\n", "", 3),
    ("# \\\\\\\\\nexec jq -n -f \"$0\" \"$@\"\n1+2\n", "", 3),
    ("# \\\\\\\\\r\nexec jq -n -f \"$0\" \"$@\"\r\n1+2\n", "", 3),
    ("#\\\nexec jq -n -f \"$0\" \"$@\"\n1+2\n", "", 3),
    ("#\\\r\nexec jq -n -f \"$0\" \"$@\"\r\n1+2\n", "", 3),
    ("#\\\\\nexec jq -n -f \"$0\" \"$@\"\n1+2\n", "", 3),
    ("#\\\\\r\nexec jq -n -f \"$0\" \"$@\"\r\n1+2\n", "", 3),
    ("#\nexec jq -n -f \"$0\" \"$@\"\n1+2\n", "", 3),
    ("#\r\nexec jq -n -f \"$0\" \"$@\"\r\n1+2\n", "", 3),
    ("x\\\nexec jq -n -f \"$0\" \"$@\"\n1+2\n", "", 3),
    ("x\\\r\nexec jq -n -f \"$0\" \"$@\"\r\n1+2\n", "", 3),
    (" # \\\nexec jq -n -f \"$0\" \"$@\"\n1+2\n", "", 3),
    (" # \\\r\nexec jq -n -f \"$0\" \"$@\"\r\n1+2\n", "", 3),
    ("\nexec jq -n -f \"$0\" \"$@\"\n1+2\n", "", 3),
    ("\r\nexec jq -n -f \"$0\" \"$@\"\r\n1+2\n", "", 3),
    ("# x\n# plain\nexec jq -n -f \"$0\" \"$@\"\n1+2\n", "", 3),
    (
        "# x\r\n# plain\r\nexec jq -n -f \"$0\" \"$@\"\r\n1+2\n",
        "",
        3,
    ),
    ("# x\n# \\\nexec jq -n -f \"$0\" \"$@\"\n1+2\n", "", 3),
    ("# x\r\n# \\\r\nexec jq -n -f \"$0\" \"$@\"\r\n1+2\n", "", 3),
    ("# x\n# \\\\\nexec jq -n -f \"$0\" \"$@\"\n1+2\n", "", 3),
    (
        "# x\r\n# \\\\\r\nexec jq -n -f \"$0\" \"$@\"\r\n1+2\n",
        "",
        3,
    ),
    ("# x\n# \\\\\\\nexec jq -n -f \"$0\" \"$@\"\n1+2\n", "", 3),
    (
        "# x\r\n# \\\\\\\r\nexec jq -n -f \"$0\" \"$@\"\r\n1+2\n",
        "",
        3,
    ),
    ("# x\n# \\\\\\\\\nexec jq -n -f \"$0\" \"$@\"\n1+2\n", "", 3),
    (
        "# x\r\n# \\\\\\\\\r\nexec jq -n -f \"$0\" \"$@\"\r\n1+2\n",
        "",
        3,
    ),
    ("# x\n#\\\nexec jq -n -f \"$0\" \"$@\"\n1+2\n", "", 3),
    ("# x\r\n#\\\r\nexec jq -n -f \"$0\" \"$@\"\r\n1+2\n", "", 3),
    ("# x\n#\\\\\nexec jq -n -f \"$0\" \"$@\"\n1+2\n", "", 3),
    (
        "# x\r\n#\\\\\r\nexec jq -n -f \"$0\" \"$@\"\r\n1+2\n",
        "",
        3,
    ),
    ("# x\n#\nexec jq -n -f \"$0\" \"$@\"\n1+2\n", "", 3),
    ("# x\r\n#\r\nexec jq -n -f \"$0\" \"$@\"\r\n1+2\n", "", 3),
    ("# x\nx\\\nexec jq -n -f \"$0\" \"$@\"\n1+2\n", "", 3),
    ("# x\r\nx\\\r\nexec jq -n -f \"$0\" \"$@\"\r\n1+2\n", "", 3),
    ("# x\n # \\\nexec jq -n -f \"$0\" \"$@\"\n1+2\n", "", 3),
    (
        "# x\r\n # \\\r\nexec jq -n -f \"$0\" \"$@\"\r\n1+2\n",
        "",
        3,
    ),
    ("# x\n\nexec jq -n -f \"$0\" \"$@\"\n1+2\n", "", 3),
    ("# x\r\n\r\nexec jq -n -f \"$0\" \"$@\"\r\n1+2\n", "", 3),
    ("#!/bin/sh\n# x\n# \\\n1+2\n", "3\n", 0),
    ("#!/bin/sh\n1\n# \\\n2\n", "", 3),
    ("1\n# \\\n+2\n", "3\n", 0),
    ("# \\\n1+2\n", "3\n", 0),
    ("#!/bin/sh\n# a \\\nb\n1+2\n", "3\n", 0),
    ("#!/bin/sh\n# a \\\n# b \\\n1+2\n", "3\n", 0),
    ("#!/bin/sh\n#\\\n\n1+2\n", "3\n", 0),
    ("#!/bin/sh\n# \\\n\n1+2\n", "3\n", 0),
    ("#!/bin/sh\n# \\\n\n\n1+2\n", "3\n", 0),
    ("#!/bin/sh\n# \\\nexec x\n", "", 3),
    ("#!/bin/sh\n# \\\nexec x", "", 3),
    ("#!/bin/sh\n# \\\n", "", 3),
    ("#!/bin/sh\n# \\", "", 3),
    ("#!/bin/sh\n# \\\n\n", "", 3),
    ("#!/bin/sh\n# \\\n#\n1\n", "1\n", 0),
    ("#!/bin/sh\n# \\\n 1+2\n", "", 3),
    ("#!/bin/sh\n# \\\n[1,\n2]\n", "", 3),
    ("#!/bin/sh\n# \\\nx\n.a\n", "null\n", 0),
    (
        "#!/bin/sh\n# \\\nx\n$__loc__\n",
        "{\"file\":\"<top-level>\",\"line\":1}\n",
        0,
    ),
    ("#!/bin/sh\n# \\\nx\n1 +\n", "", 3),
    ("#!/bin/sh\n# \\\nx\n)\n", "", 3),
    ("#!/bin/sh\n# \\\nx\n# \\\n1+2\n", "3\n", 0),
    ("#!/bin/sh\n# \\\nx\n1 # \\\n+2\n", "3\n", 0),
    ("#!/bin/sh\n\n# \\\nx\n1\n", "", 3),
    ("\n#!/bin/sh\n# \\\nx\n1\n", "", 3),
    (" #!/bin/sh\n# \\\nx\n1\n", "", 3),
    ("#!/bin/sh\r# \\\rx\r1\r", "", 3),
    ("\u{feff}#!/bin/sh\n# \\\nx\n1\n", "", 3),
];

#[test]
fn a_shebang_polyglot_program_file_skips_its_first_three_lines_7481() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("prog.jq");
    let path = path.to_str().expect("a UTF-8 temp path");
    for &(text, stdout, status) in ROWS {
        std::fs::write(path, text)?;
        let (out, err, code) = run_jq_full(&["-n", "-c", "-f", path], None)?;
        assert_eq!(
            (out.as_str(), code),
            (stdout, status),
            "{text:?}: stderr {err}"
        );
    }
    Ok(())
}

/// What the skipped text leaves behind is the whole program: nothing left is jq's own error, and
/// the lines after the skipped three are numbered from 1 (`$__loc__`).
#[test]
fn what_follows_the_skipped_lines_is_the_whole_program_7481() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("prog.jq");
    let path = path.to_str().expect("a UTF-8 temp path");

    std::fs::write(path, "#!/bin/sh\n# \\\nexec x\n")?;
    let (out, err, code) = run_jq_full(&["-n", "-f", path], None)?;
    assert_eq!(out, "");
    assert_eq!(
        err,
        "jq: error: Top-level program not given (try \".\")\njq: 1 compile error\n"
    );
    assert_eq!(code, 3);

    std::fs::write(path, "#!/bin/sh\n# \\\nx\n$__loc__\n")?;
    let (out, err, code) = run_jq_full(&["-n", "-c", "-f", path], None)?;
    assert_eq!(
        (out.as_str(), err.as_str(), code),
        ("{\"file\":\"<top-level>\",\"line\":1}\n", "", 0)
    );
    Ok(())
}

/// `skip_shebang` runs on the text of a `-f` / `--from-file` file only: the same polyglot given as the
/// program argument is parsed whole (jq 1.7.1 fails on its `exec jq` line), and a comment that ends in
/// a backslash still ends at its newline there, so `1+2` runs.
#[test]
fn a_program_argument_is_not_shebang_skipped_7481() -> Result<()> {
    let polyglot = "#!/bin/sh\n# \\\n1+2\n";
    let (out, _err, code) = run_jq_full(&["-n", "-c", polyglot], None)?;
    assert_eq!((out.as_str(), code), ("3\n", 0));

    let polyglot = "#!/bin/sh\n# \\\nexec jq\n1+2\n";
    let (out, _err, code) = run_jq_full(&["-n", "-c", polyglot], None)?;
    assert_eq!((out.as_str(), code), ("", 3));

    let dir = tempfile::tempdir()?;
    let path = dir.path().join("prog.jq");
    let path = path.to_str().expect("a UTF-8 temp path");
    std::fs::write(path, polyglot)?;
    let (out, _err, code) = run_jq_full(&["-n", "-c", "--from-file", path], None)?;
    assert_eq!((out.as_str(), code), ("3\n", 0));
    Ok(())
}
