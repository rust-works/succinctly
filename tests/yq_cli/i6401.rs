//! #6401: a head that yq's header slurp leaves to go-yaml prints back as yq prints it. Captured from yq v4.53.3:
//!
//! - a line of only blanks in the head of the first document is a comment to go-yaml, written `# ` plus the
//!   line (`⇥` becomes `# ⇥`), wherever in the head it sits (before or after a comment, a `---` or a
//!   `%YAML` line) and not at all once a document has started (`a: 1` / blanks / `b: 2` prints no comment);
//! - a head that ends mid-line gets the line break yq writes after it (`%YAML 1.2`, `# c`);
//! - a `---` ending that head gets a further empty line, kept under `-N` where the marker is not.

use anyhow::Result;

use crate::run_yq_stdin;

#[test]
fn a_line_of_blanks_in_the_head_prints_as_a_comment_6401() -> Result<()> {
    for (doc, want) in [
        ("\t\n", "# \t\n"),
        (" \n", "#  \n"),
        ("\t\na: 1\n", "# \t\na: 1\n"),
        (" \na: 1\n", "#  \na: 1\n"),
        ("# c\n\t\n", "# c\n# \t\n"),
        ("# c\n\t\na: 1\n", "# c\n# \t\na: 1\n"),
        ("\t\n# c\na: 1\n", "# \t\n# c\na: 1\n"),
        ("\t\n---\na: 1\n", "# \t\n---\na: 1\n"),
        ("---\n\t\na: 1\n", "---\n# \t\na: 1\n"),
        ("---\n \na: 1\n", "---\n#  \na: 1\n"),
        ("  \n\t\n\na: 1\n", "#   \n# \t\n\na: 1\n"),
        ("  \n# c\n\na: 1\n", "#   \n# c\n\na: 1\n"),
        ("\t\n  # c\na: 1\n", "# \t\n  # c\na: 1\n"),
        (" \n%YAML 1.2\na: 1\n", "#  \n%YAML 1.2\na: 1\n"),
        ("\n \n\nx\n", "\n#  \n\nx\n"),
        ("\n\t\n", "\n# \t\n"),
        ("# c\n  \na: 1\n", "# c\n#   \na: 1\n"),
        (" \r\na: 1\r\n", "#  \r\na: 1\n"),
        ("# c\r\n \r\n", "# c\r\n#  \r\n"),
        (" ", "#  \n"),
        ("\t", "# \t\n"),
    ] {
        let (stdout, code) = run_yq_stdin(".", doc, &[])?;
        assert_eq!((stdout.as_str(), code), (want, 0), "{doc:?}");
    }
    Ok(())
}

#[test]
fn a_line_of_blanks_after_the_head_is_dropped_6401() -> Result<()> {
    // Not in the head: go-yaml has no comment to attach it to, and so does yq.
    for (doc, want) in [
        ("a: 1\n  \nb: 2\n", "a: 1\nb: 2\n"),
        ("a:\n  - x\n  \n  - y\n", "a:\n  - x\n  - y\n"),
        ("a: 1\n---\n \nb: 2\n", "a: 1\n---\nb: 2\n"),
        ("a: 1\n# c\n  \nb: 2\n", "a: 1\n# c\n\nb: 2\n"),
    ] {
        let (stdout, code) = run_yq_stdin(".", doc, &[])?;
        assert_eq!((stdout.as_str(), code), (want, 0), "{doc:?}");
    }
    Ok(())
}

#[test]
fn a_head_that_ends_mid_line_gets_a_line_break_6401() -> Result<()> {
    for (doc, want) in [
        ("%YAML 1.2", "%YAML 1.2\n"),
        ("\t # c", "\t # c\n"),
        ("# c", "# c\n"),
        ("# a\n\n# b", "# a\n\n# b\n"),
        ("---\n# d", "---\n# d\n"),
        ("%YAML 1.2\r", "%YAML 1.2\r\n"),
    ] {
        let (stdout, code) = run_yq_stdin(".", doc, &[])?;
        assert_eq!((stdout.as_str(), code), (want, 0), "{doc:?}");
    }
    Ok(())
}

#[test]
fn a_head_ending_in_a_bare_marker_gets_an_empty_line_after_it_6401() -> Result<()> {
    for (doc, want) in [
        ("---\n# d\n---", "---\n# d\n---\n\n"),
        ("---", "---\n\n"),
        ("---\n---", "---\n---\n\n"),
        ("# c\n---", "# c\n---\n\n"),
        ("%YAML 1.2\n---", "%YAML 1.2\n---\n\n"),
    ] {
        let (stdout, code) = run_yq_stdin(".", doc, &[])?;
        assert_eq!((stdout.as_str(), code), (want, 0), "{doc:?}");
    }
    // `-N` drops the marker and keeps the empty line.
    for (doc, want) in [
        ("---", "\n"),
        ("# c\n---", "# c\n\n"),
        ("---\n \n---", "#  \n\n"),
        ("\t\na: 1\n", "# \t\na: 1\n"),
    ] {
        let (stdout, code) = run_yq_stdin(".", doc, &["-N"])?;
        assert_eq!((stdout.as_str(), code), (want, 0), "{doc:?}");
    }
    Ok(())
}
