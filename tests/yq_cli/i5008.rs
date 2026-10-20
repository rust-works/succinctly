//! #5008: yq's default `--header-preprocess` slurps the empty lines at the front of an input and prints them
//! back before the first document, on every route. The DOM routes (`-P`, a write, a navigated or computed
//! result) dropped them when no comment or `---` marker came with them. A header that holds a comment, and a
//! carriage return in an empty line, are not covered (recorded in the yq limitations).
//!
//! Every output is captured from yq v4.53.3.

use anyhow::Result;

use crate::run_yq_stdin;

/// (filter, extra args, document, expected output).
const ROWS: &[(&str, &[&str], &str, &str)] = &[
    (".", &[], "\na: 1\n", "\na: 1\n"),
    (".", &["-P"], "\na: 1\n", "\na: 1\n"),
    (".", &["-P"], "\n\na: 1\n", "\n\na: 1\n"),
    (".a = 2", &[], "\n\na: 1\n", "\n\na: 2\n"),
    (".a |= 3", &["-P"], "\na: 1\n", "\na: 3\n"),
    ("del(.a)", &[], "\na: 1\n", "\n{}\n"),
    (".", &["-P"], "\n- 1\n", "\n- 1\n"),
    (".", &["-P"], "\n\n42\n", "\n\n42\n"),
    (".", &["-P"], "\n\"x\"\n", "\nx\n"),
    (".", &[], "\nk: [1, 2]\n", "\nk: [1, 2]\n"),
    (".", &["-o=json", "-I0"], "\na: 1\n", "{\"a\":1}\n"),
    (".", &["-N"], "\n\na: 1\n", "\n\na: 1\n"),
    (".a", &[], "\na: 1\n", "1\n"),
    (".", &[], "\n\na: 1\n---\nb: 2\n", "\n\na: 1\n---\nb: 2\n"),
    (
        ".",
        &["-P"],
        "\n\na: 1\n---\nb: 2\n",
        "\n\na: 1\n---\nb: 2\n",
    ),
    (".", &[], "\n\n---\na: 1\n", "\n\n---\na: 1\n"),
];

#[test]
fn test_5008_leading_empty_lines_are_printed_back_on_every_route() -> Result<()> {
    for (filter, args, input, expected) in ROWS {
        let (out, code) = run_yq_stdin(filter, input, args)?;
        assert_eq!(code, 0, "{filter} {args:?} {input:?}");
        assert_eq!(out, *expected, "{filter} {args:?} {input:?}");
    }
    Ok(())
}
