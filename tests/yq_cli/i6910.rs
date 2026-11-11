//! #6910: yq splits an input into its front matter and body by one rule, and reads whatever it holds: the
//! text up to the first line that starts with `---` at column 0, after skipping the input's own leading
//! `---` line, is the front matter (all of the input when nothing closes it), and that line starts the
//! body, which `process` writes back as it was. An input with no front matter, an unterminated one, an
//! indented or commented opening line, and a `...` (which closes nothing) were refused here. The opening
//! `---` is the front matter's own line, written back as it was (CRLF included) when the result is the
//! front matter document. Every answer was captured from yq v4.53.3; a row is kept only when both agree.
//! Left out because they still differ: a scalar read of a front matter that has an opening `---` (yq
//! prints no `---` before it), a filter error (yq prints nothing, the body is still written here), and
//! an empty input under a write (a blank line here).

use anyhow::Result;
use std::io::Write;

use crate::{run_yq_stdin, run_yq_stdin_with_stderr};

/// (input, mode, filter, yq's stdout).
const ROWS: &[(&str, &str, &str, &str)] = &[
    ("a: 1\n", "process", ".a", "1\n"),
    ("a: 1\n", "extract", ".a", "1\n"),
    ("a: 1\n", "process", ".", "a: 1\n"),
    ("a: 1\n", "extract", ".", "a: 1\n"),
    ("a: 1\n", "process", ".a = \"N\"", "a: N\n"),
    ("a: 1\n", "process", ".b = 2", "a: 1\nb: 2\n"),
    ("a: 1\n", "process", "del(.a)", "{}\n"),
    ("---\na: 1\n---\nbody\n", "process", ".a", "1\n---\nbody\n"),
    ("---\na: 1\n---\nbody\n", "extract", ".a", "1\n"),
    (
        "---\na: 1\n---\nbody\n",
        "process",
        ".",
        "---\na: 1\n---\nbody\n",
    ),
    ("---\na: 1\n---\nbody\n", "extract", ".", "---\na: 1\n"),
    (
        "---\na: 1\n---\nbody\n",
        "process",
        ".a = \"N\"",
        "---\na: N\n---\nbody\n",
    ),
    (
        "---\na: 1\n---\nbody\n",
        "process",
        ".b = 2",
        "---\na: 1\nb: 2\n---\nbody\n",
    ),
    (
        "---\na: 1\n---\nbody\n",
        "process",
        "del(.a)",
        "---\n{}\n---\nbody\n",
    ),
    ("---\na: 1\n---\n", "process", ".a", "1\n---\n"),
    ("---\na: 1\n---\n", "extract", ".a", "1\n"),
    ("---\na: 1\n---\n", "process", ".", "---\na: 1\n---\n"),
    ("---\na: 1\n---\n", "extract", ".", "---\na: 1\n"),
    (
        "---\na: 1\n---\n",
        "process",
        ".a = \"N\"",
        "---\na: N\n---\n",
    ),
    (
        "---\na: 1\n---\n",
        "process",
        ".b = 2",
        "---\na: 1\nb: 2\n---\n",
    ),
    ("---\na: 1\n---\n", "process", "del(.a)", "---\n{}\n---\n"),
    ("---\na: 1\n", "process", ".a", "1\n"),
    ("---\na: 1\n", "extract", ".a", "1\n"),
    ("---\na: 1\n", "process", ".", "---\na: 1\n"),
    ("---\na: 1\n", "extract", ".", "---\na: 1\n"),
    ("---\na: 1\n", "process", ".a = \"N\"", "---\na: N\n"),
    ("---\na: 1\n", "process", ".b = 2", "---\na: 1\nb: 2\n"),
    ("---\na: 1\n", "process", "del(.a)", "---\n{}\n"),
    (
        "# c\n---\na: 1\n---\nbody\n",
        "process",
        ".a",
        "null\n---\na: 1\n---\nbody\n",
    ),
    ("# c\n---\na: 1\n---\nbody\n", "extract", ".a", "null\n"),
    (
        "# c\n---\na: 1\n---\nbody\n",
        "process",
        ".",
        "# c\n---\na: 1\n---\nbody\n",
    ),
    ("# c\n---\na: 1\n---\nbody\n", "extract", ".", "# c\n"),
    (
        "# c\n---\na: 1\n---\nbody\n",
        "process",
        ".a = \"N\"",
        "# c\na: N\n---\na: 1\n---\nbody\n",
    ),
    (
        "# c\n---\na: 1\n---\nbody\n",
        "process",
        ".b = 2",
        "# c\nb: 2\n---\na: 1\n---\nbody\n",
    ),
    (
        "# c\n---\na: 1\n---\nbody\n",
        "process",
        "del(.a)",
        "# c\n---\na: 1\n---\nbody\n",
    ),
    (
        "\n---\na: 1\n---\nbody\n",
        "process",
        ".a",
        "null\n---\na: 1\n---\nbody\n",
    ),
    ("\n---\na: 1\n---\nbody\n", "extract", ".a", "null\n"),
    (
        "\n---\na: 1\n---\nbody\n",
        "process",
        ".",
        "\n---\na: 1\n---\nbody\n",
    ),
    ("\n---\na: 1\n---\nbody\n", "extract", ".", "\n"),
    (
        "\n---\na: 1\n---\nbody\n",
        "process",
        ".a = \"N\"",
        "\na: N\n---\na: 1\n---\nbody\n",
    ),
    (
        "\n---\na: 1\n---\nbody\n",
        "process",
        ".b = 2",
        "\nb: 2\n---\na: 1\n---\nbody\n",
    ),
    (
        "\n---\na: 1\n---\nbody\n",
        "process",
        "del(.a)",
        "\n---\na: 1\n---\nbody\n",
    ),
    (" ---\na: 1\n---\nbody\n", "process", ".a", "1\n---\nbody\n"),
    (" ---\na: 1\n---\nbody\n", "extract", ".a", "1\n"),
    (
        " ---\na: 1\n---\nbody\n",
        "process",
        ".",
        "---\na: 1\n---\nbody\n",
    ),
    (" ---\na: 1\n---\nbody\n", "extract", ".", "---\na: 1\n"),
    (
        " ---\na: 1\n---\nbody\n",
        "process",
        ".a = \"N\"",
        "---\na: N\n---\nbody\n",
    ),
    (
        " ---\na: 1\n---\nbody\n",
        "process",
        ".b = 2",
        "---\na: 1\nb: 2\n---\nbody\n",
    ),
    (
        " ---\na: 1\n---\nbody\n",
        "process",
        "del(.a)",
        "---\n{}\n---\nbody\n",
    ),
    ("---\n---\nbody\n", "process", ".a", "null\n---\nbody\n"),
    ("---\n---\nbody\n", "extract", ".a", "null\n"),
    ("---\n---\nbody\n", "process", ".", "---\n---\nbody\n"),
    ("---\n---\nbody\n", "extract", ".", "---\n"),
    (
        "---\n---\nbody\n",
        "process",
        ".a = \"N\"",
        "---\na: N\n---\nbody\n",
    ),
    (
        "---\n---\nbody\n",
        "process",
        ".b = 2",
        "---\nb: 2\n---\nbody\n",
    ),
    ("---\n---\nbody\n", "process", "del(.a)", "---\n---\nbody\n"),
    (
        "---\r\na: 1\r\n---\r\nbody\r\n",
        "process",
        ".a",
        "1\n---\r\nbody\r\n",
    ),
    ("---\r\na: 1\r\n---\r\nbody\r\n", "extract", ".a", "1\n"),
    (
        "---\r\na: 1\r\n---\r\nbody\r\n",
        "process",
        ".",
        "---\r\na: 1\n---\r\nbody\r\n",
    ),
    (
        "---\r\na: 1\r\n---\r\nbody\r\n",
        "extract",
        ".",
        "---\r\na: 1\n",
    ),
    (
        "---\r\na: 1\r\n---\r\nbody\r\n",
        "process",
        ".a = \"N\"",
        "---\r\na: N\n---\r\nbody\r\n",
    ),
    (
        "---\r\na: 1\r\n---\r\nbody\r\n",
        "process",
        ".b = 2",
        "---\r\na: 1\nb: 2\n---\r\nbody\r\n",
    ),
    (
        "---\r\na: 1\r\n---\r\nbody\r\n",
        "process",
        "del(.a)",
        "---\r\n{}\n---\r\nbody\r\n",
    ),
    ("body only\nline2\n", "process", ".a", ""),
    ("body only\nline2\n", "extract", ".a", ""),
    ("body only\nline2\n", "process", ".", "body only line2\n"),
    ("body only\nline2\n", "extract", ".", "body only line2\n"),
    (
        "body only\nline2\n",
        "process",
        ".a = \"N\"",
        "body only line2\n",
    ),
    (
        "body only\nline2\n",
        "process",
        ".b = 2",
        "body only line2\n",
    ),
    (
        "body only\nline2\n",
        "process",
        "del(.a)",
        "body only line2\n",
    ),
    (
        "---\na: 1\n---\nbody\n---\nmore\n",
        "process",
        ".a",
        "1\n---\nbody\n---\nmore\n",
    ),
    ("---\na: 1\n---\nbody\n---\nmore\n", "extract", ".a", "1\n"),
    (
        "---\na: 1\n---\nbody\n---\nmore\n",
        "process",
        ".",
        "---\na: 1\n---\nbody\n---\nmore\n",
    ),
    (
        "---\na: 1\n---\nbody\n---\nmore\n",
        "extract",
        ".",
        "---\na: 1\n",
    ),
    (
        "---\na: 1\n---\nbody\n---\nmore\n",
        "process",
        ".a = \"N\"",
        "---\na: N\n---\nbody\n---\nmore\n",
    ),
    (
        "---\na: 1\n---\nbody\n---\nmore\n",
        "process",
        ".b = 2",
        "---\na: 1\nb: 2\n---\nbody\n---\nmore\n",
    ),
    (
        "---\na: 1\n---\nbody\n---\nmore\n",
        "process",
        "del(.a)",
        "---\n{}\n---\nbody\n---\nmore\n",
    ),
    (
        "---\na: 1 # c\n---\nbody\n---\n",
        "process",
        ".a",
        "1\n---\nbody\n---\n",
    ),
    ("---\na: 1 # c\n---\nbody\n---\n", "extract", ".a", "1\n"),
    (
        "---\na: 1 # c\n---\nbody\n---\n",
        "process",
        ".",
        "---\na: 1 # c\n---\nbody\n---\n",
    ),
    (
        "---\na: 1 # c\n---\nbody\n---\n",
        "extract",
        ".",
        "---\na: 1 # c\n",
    ),
    (
        "---\na: 1 # c\n---\nbody\n---\n",
        "process",
        ".a = \"N\"",
        "---\na: N # c\n---\nbody\n---\n",
    ),
    (
        "---\na: 1 # c\n---\nbody\n---\n",
        "process",
        ".b = 2",
        "---\na: 1 # c\nb: 2\n---\nbody\n---\n",
    ),
    (
        "---\na: 1 # c\n---\nbody\n---\n",
        "process",
        "del(.a)",
        "---\n{}\n---\nbody\n---\n",
    ),
    (
        "---\nkey:\n  ---\n---\nBody\n",
        "process",
        ".a",
        "null\n---\nBody\n",
    ),
    ("---\nkey:\n  ---\n---\nBody\n", "extract", ".a", "null\n"),
    (
        "---\nkey:\n  ---\n---\nBody\n",
        "process",
        ".",
        "---\nkey: '---'\n---\nBody\n",
    ),
    (
        "---\nkey:\n  ---\n---\nBody\n",
        "extract",
        ".",
        "---\nkey: '---'\n",
    ),
    (
        "---\nkey:\n  ---\n---\nBody\n",
        "process",
        ".a = \"N\"",
        "---\nkey: '---'\na: N\n---\nBody\n",
    ),
    (
        "---\nkey:\n  ---\n---\nBody\n",
        "process",
        ".b = 2",
        "---\nkey: '---'\nb: 2\n---\nBody\n",
    ),
    (
        "---\nkey:\n  ---\n---\nBody\n",
        "process",
        "del(.a)",
        "---\nkey: '---'\n---\nBody\n",
    ),
    (
        "---\na: 1\n---  \nbody\n",
        "process",
        ".a",
        "1\n---  \nbody\n",
    ),
    ("---\na: 1\n---  \nbody\n", "extract", ".a", "1\n"),
    (
        "---\na: 1\n---  \nbody\n",
        "process",
        ".",
        "---\na: 1\n---  \nbody\n",
    ),
    ("---\na: 1\n---  \nbody\n", "extract", ".", "---\na: 1\n"),
    (
        "---\na: 1\n---  \nbody\n",
        "process",
        ".a = \"N\"",
        "---\na: N\n---  \nbody\n",
    ),
    (
        "---\na: 1\n---  \nbody\n",
        "process",
        ".b = 2",
        "---\na: 1\nb: 2\n---  \nbody\n",
    ),
    (
        "---\na: 1\n---  \nbody\n",
        "process",
        "del(.a)",
        "---\n{}\n---  \nbody\n",
    ),
    (
        "---\na: 1\n--- # c\nbody\n",
        "process",
        ".a",
        "1\n--- # c\nbody\n",
    ),
    ("---\na: 1\n--- # c\nbody\n", "extract", ".a", "1\n"),
    (
        "---\na: 1\n--- # c\nbody\n",
        "process",
        ".",
        "---\na: 1\n--- # c\nbody\n",
    ),
    ("---\na: 1\n--- # c\nbody\n", "extract", ".", "---\na: 1\n"),
    (
        "---\na: 1\n--- # c\nbody\n",
        "process",
        ".a = \"N\"",
        "---\na: N\n--- # c\nbody\n",
    ),
    (
        "---\na: 1\n--- # c\nbody\n",
        "process",
        ".b = 2",
        "---\na: 1\nb: 2\n--- # c\nbody\n",
    ),
    (
        "---\na: 1\n--- # c\nbody\n",
        "process",
        "del(.a)",
        "---\n{}\n--- # c\nbody\n",
    ),
    (
        "---\na: 1\n----\nbody\n",
        "process",
        ".a",
        "1\n----\nbody\n",
    ),
    ("---\na: 1\n----\nbody\n", "extract", ".a", "1\n"),
    (
        "---\na: 1\n----\nbody\n",
        "process",
        ".",
        "---\na: 1\n----\nbody\n",
    ),
    ("---\na: 1\n----\nbody\n", "extract", ".", "---\na: 1\n"),
    (
        "---\na: 1\n----\nbody\n",
        "process",
        ".a = \"N\"",
        "---\na: N\n----\nbody\n",
    ),
    (
        "---\na: 1\n----\nbody\n",
        "process",
        ".b = 2",
        "---\na: 1\nb: 2\n----\nbody\n",
    ),
    (
        "---\na: 1\n----\nbody\n",
        "process",
        "del(.a)",
        "---\n{}\n----\nbody\n",
    ),
    (
        "---  \na: 1\n---\nbody\n",
        "process",
        ".a",
        "1\n---\nbody\n",
    ),
    ("---  \na: 1\n---\nbody\n", "extract", ".a", "1\n"),
    (
        "---  \na: 1\n---\nbody\n",
        "process",
        ".",
        "---\na: 1\n---\nbody\n",
    ),
    ("---  \na: 1\n---\nbody\n", "extract", ".", "---\na: 1\n"),
    (
        "---  \na: 1\n---\nbody\n",
        "process",
        ".a = \"N\"",
        "---\na: N\n---\nbody\n",
    ),
    (
        "---  \na: 1\n---\nbody\n",
        "process",
        ".b = 2",
        "---\na: 1\nb: 2\n---\nbody\n",
    ),
    (
        "---  \na: 1\n---\nbody\n",
        "process",
        "del(.a)",
        "---\n{}\n---\nbody\n",
    ),
    (
        "  ---\na: 1\n---\nbody\n",
        "process",
        ".a",
        "1\n---\nbody\n",
    ),
    ("  ---\na: 1\n---\nbody\n", "extract", ".a", "1\n"),
    (
        "  ---\na: 1\n---\nbody\n",
        "process",
        ".",
        "---\na: 1\n---\nbody\n",
    ),
    ("  ---\na: 1\n---\nbody\n", "extract", ".", "---\na: 1\n"),
    (
        "  ---\na: 1\n---\nbody\n",
        "process",
        ".a = \"N\"",
        "---\na: N\n---\nbody\n",
    ),
    (
        "  ---\na: 1\n---\nbody\n",
        "process",
        ".b = 2",
        "---\na: 1\nb: 2\n---\nbody\n",
    ),
    (
        "  ---\na: 1\n---\nbody\n",
        "process",
        "del(.a)",
        "---\n{}\n---\nbody\n",
    ),
    (
        "\t---\na: 1\n---\nbody\n",
        "process",
        ".a",
        "1\n---\nbody\n",
    ),
    ("\t---\na: 1\n---\nbody\n", "extract", ".a", "1\n"),
    (
        "\t---\na: 1\n---\nbody\n",
        "process",
        ".",
        "---\na: 1\n---\nbody\n",
    ),
    ("\t---\na: 1\n---\nbody\n", "extract", ".", "---\na: 1\n"),
    (
        "\t---\na: 1\n---\nbody\n",
        "process",
        ".a = \"N\"",
        "---\na: N\n---\nbody\n",
    ),
    (
        "\t---\na: 1\n---\nbody\n",
        "process",
        ".b = 2",
        "---\na: 1\nb: 2\n---\nbody\n",
    ),
    (
        "\t---\na: 1\n---\nbody\n",
        "process",
        "del(.a)",
        "---\n{}\n---\nbody\n",
    ),
    (
        "---\ntitle: T\ntags: [a, b]\n---\n# Heading\n\ntext with: colon\n",
        "process",
        ".a",
        "null\n---\n# Heading\n\ntext with: colon\n",
    ),
    (
        "---\ntitle: T\ntags: [a, b]\n---\n# Heading\n\ntext with: colon\n",
        "extract",
        ".a",
        "null\n",
    ),
    (
        "---\ntitle: T\ntags: [a, b]\n---\n# Heading\n\ntext with: colon\n",
        "process",
        ".",
        "---\ntitle: T\ntags: [a, b]\n---\n# Heading\n\ntext with: colon\n",
    ),
    (
        "---\ntitle: T\ntags: [a, b]\n---\n# Heading\n\ntext with: colon\n",
        "extract",
        ".",
        "---\ntitle: T\ntags: [a, b]\n",
    ),
    (
        "---\ntitle: T\ntags: [a, b]\n---\n# Heading\n\ntext with: colon\n",
        "process",
        ".a = \"N\"",
        "---\ntitle: T\ntags: [a, b]\na: N\n---\n# Heading\n\ntext with: colon\n",
    ),
    (
        "---\ntitle: T\ntags: [a, b]\n---\n# Heading\n\ntext with: colon\n",
        "process",
        ".b = 2",
        "---\ntitle: T\ntags: [a, b]\nb: 2\n---\n# Heading\n\ntext with: colon\n",
    ),
    (
        "---\ntitle: T\ntags: [a, b]\n---\n# Heading\n\ntext with: colon\n",
        "process",
        "del(.a)",
        "---\ntitle: T\ntags: [a, b]\n---\n# Heading\n\ntext with: colon\n",
    ),
    ("---\ntitle: T\n---", "process", ".a", "null\n---"),
    ("---\ntitle: T\n---", "extract", ".a", "null\n"),
    ("---\ntitle: T\n---", "process", ".", "---\ntitle: T\n---"),
    ("---\ntitle: T\n---", "extract", ".", "---\ntitle: T\n"),
    (
        "---\ntitle: T\n---",
        "process",
        ".a = \"N\"",
        "---\ntitle: T\na: N\n---",
    ),
    (
        "---\ntitle: T\n---",
        "process",
        ".b = 2",
        "---\ntitle: T\nb: 2\n---",
    ),
    (
        "---\ntitle: T\n---",
        "process",
        "del(.a)",
        "---\ntitle: T\n---",
    ),
    (
        "---\ntitle: T\n---\nno newline at end",
        "process",
        ".a",
        "null\n---\nno newline at end",
    ),
    (
        "---\ntitle: T\n---\nno newline at end",
        "extract",
        ".a",
        "null\n",
    ),
    (
        "---\ntitle: T\n---\nno newline at end",
        "process",
        ".",
        "---\ntitle: T\n---\nno newline at end",
    ),
    (
        "---\ntitle: T\n---\nno newline at end",
        "extract",
        ".",
        "---\ntitle: T\n",
    ),
    (
        "---\ntitle: T\n---\nno newline at end",
        "process",
        ".a = \"N\"",
        "---\ntitle: T\na: N\n---\nno newline at end",
    ),
    (
        "---\ntitle: T\n---\nno newline at end",
        "process",
        ".b = 2",
        "---\ntitle: T\nb: 2\n---\nno newline at end",
    ),
    (
        "---\ntitle: T\n---\nno newline at end",
        "process",
        "del(.a)",
        "---\ntitle: T\n---\nno newline at end",
    ),
    ("---\na: 1\n---\n---\n", "process", ".a", "1\n---\n---\n"),
    ("---\na: 1\n---\n---\n", "extract", ".a", "1\n"),
    (
        "---\na: 1\n---\n---\n",
        "process",
        ".",
        "---\na: 1\n---\n---\n",
    ),
    ("---\na: 1\n---\n---\n", "extract", ".", "---\na: 1\n"),
    (
        "---\na: 1\n---\n---\n",
        "process",
        ".a = \"N\"",
        "---\na: N\n---\n---\n",
    ),
    (
        "---\na: 1\n---\n---\n",
        "process",
        ".b = 2",
        "---\na: 1\nb: 2\n---\n---\n",
    ),
    (
        "---\na: 1\n---\n---\n",
        "process",
        "del(.a)",
        "---\n{}\n---\n---\n",
    ),
    (
        "---\na: 1\n---\n\n\nbody\n",
        "process",
        ".a",
        "1\n---\n\n\nbody\n",
    ),
    ("---\na: 1\n---\n\n\nbody\n", "extract", ".a", "1\n"),
    (
        "---\na: 1\n---\n\n\nbody\n",
        "process",
        ".",
        "---\na: 1\n---\n\n\nbody\n",
    ),
    ("---\na: 1\n---\n\n\nbody\n", "extract", ".", "---\na: 1\n"),
    (
        "---\na: 1\n---\n\n\nbody\n",
        "process",
        ".a = \"N\"",
        "---\na: N\n---\n\n\nbody\n",
    ),
    (
        "---\na: 1\n---\n\n\nbody\n",
        "process",
        ".b = 2",
        "---\na: 1\nb: 2\n---\n\n\nbody\n",
    ),
    (
        "---\na: 1\n---\n\n\nbody\n",
        "process",
        "del(.a)",
        "---\n{}\n---\n\n\nbody\n",
    ),
    (
        "---\n# only comment\n---\nbody\n",
        "process",
        ".a",
        "null\n---\nbody\n",
    ),
    (
        "---\n# only comment\n---\nbody\n",
        "extract",
        ".a",
        "null\n",
    ),
    (
        "---\n# only comment\n---\nbody\n",
        "process",
        ".",
        "---\n# only comment\n---\nbody\n",
    ),
    (
        "---\n# only comment\n---\nbody\n",
        "extract",
        ".",
        "---\n# only comment\n",
    ),
    (
        "---\n# only comment\n---\nbody\n",
        "process",
        ".a = \"N\"",
        "---\n# only comment\na: N\n---\nbody\n",
    ),
    (
        "---\n# only comment\n---\nbody\n",
        "process",
        ".b = 2",
        "---\n# only comment\nb: 2\n---\nbody\n",
    ),
    (
        "---\n# only comment\n---\nbody\n",
        "process",
        "del(.a)",
        "---\n# only comment\n---\nbody\n",
    ),
];

#[test]
fn front_matter_is_split_as_yq_splits_it_6910() -> Result<()> {
    for &(input, mode, filter, want) in ROWS {
        let (out, stderr, code) =
            run_yq_stdin_with_stderr(filter, input, &["--front-matter", mode])?;
        assert_eq!(code, 0, "{input:?} {mode} {filter}: {stderr}");
        assert_eq!(out, want, "{input:?} {mode} {filter}");
    }
    let _ = run_yq_stdin;
    Ok(())
}

/// (file before, filter, file after) under `-i --front-matter process`.
const IN_PLACE: &[(&str, &str, &str)] = &[
    ("a: 1\n", ".a = \"N\"", "a: N\n"),
    ("a: 1\n", "del(.a)", "{}\n"),
    (
        "---\na: 1\n---\nbody\n",
        ".a = \"N\"",
        "---\na: N\n---\nbody\n",
    ),
    ("---\na: 1\n---\nbody\n", "del(.a)", "---\n{}\n---\nbody\n"),
    ("---\na: 1\n---\n", ".a = \"N\"", "---\na: N\n---\n"),
    ("---\na: 1\n---\n", "del(.a)", "---\n{}\n---\n"),
    ("---\na: 1\n", ".a = \"N\"", "---\na: N\n"),
    ("---\na: 1\n", "del(.a)", "---\n{}\n"),
    (
        "# c\n---\na: 1\n---\nbody\n",
        ".a = \"N\"",
        "# c\na: N\n---\na: 1\n---\nbody\n",
    ),
    (
        "# c\n---\na: 1\n---\nbody\n",
        "del(.a)",
        "# c\n---\na: 1\n---\nbody\n",
    ),
    (
        "\n---\na: 1\n---\nbody\n",
        ".a = \"N\"",
        "\na: N\n---\na: 1\n---\nbody\n",
    ),
    (
        "\n---\na: 1\n---\nbody\n",
        "del(.a)",
        "\n---\na: 1\n---\nbody\n",
    ),
    (
        " ---\na: 1\n---\nbody\n",
        ".a = \"N\"",
        "---\na: N\n---\nbody\n",
    ),
    (" ---\na: 1\n---\nbody\n", "del(.a)", "---\n{}\n---\nbody\n"),
    ("---\n---\nbody\n", ".a = \"N\"", "---\na: N\n---\nbody\n"),
    ("---\n---\nbody\n", "del(.a)", "---\n---\nbody\n"),
    (
        "---\r\na: 1\r\n---\r\nbody\r\n",
        ".a = \"N\"",
        "---\r\na: N\n---\r\nbody\r\n",
    ),
    (
        "---\r\na: 1\r\n---\r\nbody\r\n",
        "del(.a)",
        "---\r\n{}\n---\r\nbody\r\n",
    ),
    ("body only\nline2\n", ".a = \"N\"", "body only line2\n"),
    ("body only\nline2\n", "del(.a)", "body only line2\n"),
    (
        "---\na: 1\n---\nbody\n---\nmore\n",
        ".a = \"N\"",
        "---\na: N\n---\nbody\n---\nmore\n",
    ),
    (
        "---\na: 1\n---\nbody\n---\nmore\n",
        "del(.a)",
        "---\n{}\n---\nbody\n---\nmore\n",
    ),
    (
        "---\na: 1 # c\n---\nbody\n---\n",
        ".a = \"N\"",
        "---\na: N # c\n---\nbody\n---\n",
    ),
    (
        "---\na: 1 # c\n---\nbody\n---\n",
        "del(.a)",
        "---\n{}\n---\nbody\n---\n",
    ),
    (
        "---\nkey:\n  ---\n---\nBody\n",
        ".a = \"N\"",
        "---\nkey: '---'\na: N\n---\nBody\n",
    ),
    (
        "---\nkey:\n  ---\n---\nBody\n",
        "del(.a)",
        "---\nkey: '---'\n---\nBody\n",
    ),
    (
        "---\na: 1\n---  \nbody\n",
        ".a = \"N\"",
        "---\na: N\n---  \nbody\n",
    ),
    (
        "---\na: 1\n---  \nbody\n",
        "del(.a)",
        "---\n{}\n---  \nbody\n",
    ),
    (
        "---\na: 1\n--- # c\nbody\n",
        ".a = \"N\"",
        "---\na: N\n--- # c\nbody\n",
    ),
    (
        "---\na: 1\n--- # c\nbody\n",
        "del(.a)",
        "---\n{}\n--- # c\nbody\n",
    ),
    (
        "---\na: 1\n----\nbody\n",
        ".a = \"N\"",
        "---\na: N\n----\nbody\n",
    ),
    (
        "---\na: 1\n----\nbody\n",
        "del(.a)",
        "---\n{}\n----\nbody\n",
    ),
    (
        "---  \na: 1\n---\nbody\n",
        ".a = \"N\"",
        "---\na: N\n---\nbody\n",
    ),
    (
        "---  \na: 1\n---\nbody\n",
        "del(.a)",
        "---\n{}\n---\nbody\n",
    ),
    (
        "  ---\na: 1\n---\nbody\n",
        ".a = \"N\"",
        "---\na: N\n---\nbody\n",
    ),
    (
        "  ---\na: 1\n---\nbody\n",
        "del(.a)",
        "---\n{}\n---\nbody\n",
    ),
    (
        "\t---\na: 1\n---\nbody\n",
        ".a = \"N\"",
        "---\na: N\n---\nbody\n",
    ),
    (
        "\t---\na: 1\n---\nbody\n",
        "del(.a)",
        "---\n{}\n---\nbody\n",
    ),
    (
        "---\ntitle: T\ntags: [a, b]\n---\n# Heading\n\ntext with: colon\n",
        ".a = \"N\"",
        "---\ntitle: T\ntags: [a, b]\na: N\n---\n# Heading\n\ntext with: colon\n",
    ),
    (
        "---\ntitle: T\ntags: [a, b]\n---\n# Heading\n\ntext with: colon\n",
        "del(.a)",
        "---\ntitle: T\ntags: [a, b]\n---\n# Heading\n\ntext with: colon\n",
    ),
    (
        "---\ntitle: T\n---",
        ".a = \"N\"",
        "---\ntitle: T\na: N\n---",
    ),
    ("---\ntitle: T\n---", "del(.a)", "---\ntitle: T\n---"),
    (
        "---\ntitle: T\n---\nno newline at end",
        ".a = \"N\"",
        "---\ntitle: T\na: N\n---\nno newline at end",
    ),
    (
        "---\ntitle: T\n---\nno newline at end",
        "del(.a)",
        "---\ntitle: T\n---\nno newline at end",
    ),
    (
        "---\na: 1\n---\n---\n",
        ".a = \"N\"",
        "---\na: N\n---\n---\n",
    ),
    ("---\na: 1\n---\n---\n", "del(.a)", "---\n{}\n---\n---\n"),
    (
        "---\na: 1\n---\n\n\nbody\n",
        ".a = \"N\"",
        "---\na: N\n---\n\n\nbody\n",
    ),
    (
        "---\na: 1\n---\n\n\nbody\n",
        "del(.a)",
        "---\n{}\n---\n\n\nbody\n",
    ),
    (
        "---\n# only comment\n---\nbody\n",
        ".a = \"N\"",
        "---\n# only comment\na: N\n---\nbody\n",
    ),
    (
        "---\n# only comment\n---\nbody\n",
        "del(.a)",
        "---\n# only comment\n---\nbody\n",
    ),
];

#[test]
fn an_in_place_front_matter_edit_leaves_the_file_as_yq_does_6910() -> Result<()> {
    for &(before, filter, after) in IN_PLACE {
        let mut file = tempfile::NamedTempFile::new()?;
        write!(file, "{before}")?;
        let path = file.path().to_string_lossy().into_owned();
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_succinctly"))
            .args(["yq", "-i", "--front-matter", "process", filter, &path])
            .stdin(std::process::Stdio::null())
            .output()?;
        assert!(output.status.success(), "{before:?} {filter}");
        assert_eq!(
            std::fs::read_to_string(&path)?,
            after,
            "{before:?} {filter}"
        );
    }
    Ok(())
}
