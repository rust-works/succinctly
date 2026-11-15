//! #7099: the first document's leading content -- what yq's header preprocessing sets aside before the document and
//! prints back in front of it -- on a write and on `-P`, which build the output from the document's comment tree.
//! A `%YAML` directive was dropped (the preamble gave up on any `%` line), a comment or `%YAML` line indented by
//! blanks lost its indent, a comment after `---` over an empty root was lost (it is no node's head), a line of only
//! blanks after `---` printed twice, and a head written to a scalar root left the header in place. Each row is a
//! live yq v4.53.3 capture.

use anyhow::Result;

use crate::run_yq_stdin;

/// (document, extra arguments, filter, yq's output).
type Row = (
    &'static str,
    &'static [&'static str],
    &'static str,
    &'static str,
);

const DIRECTIVE: &[Row] = &[
    (
        "%YAML 1.2\n---\na: 1\nb: 2\n",
        &[],
        ".a = 3",
        "%YAML 1.2\n---\na: 3\nb: 2\n",
    ),
    (
        "%YAML 1.2\n---\na: 1\nb: 2\n",
        &[],
        "del(.zz)",
        "%YAML 1.2\n---\na: 1\nb: 2\n",
    ),
    (
        "%YAML 1.2\n---\na: 1\nb: 2\n",
        &[],
        "to_entries",
        "%YAML 1.2\n---\n- key: a\n  value: 1\n- key: b\n  value: 2\n",
    ),
    (
        "%YAML 1.2\n---\na: 1\nb: 2\n",
        &["-P"],
        ".",
        "%YAML 1.2\n---\na: 1\nb: 2\n",
    ),
    (
        "%YAML 1.2\n---\na: 1\nb: 2\n",
        &["-N"],
        ".a = 3",
        "%YAML 1.2\na: 3\nb: 2\n",
    ),
    ("%YAML 1.2\n---\n5\n", &[], ". = 6", "%YAML 1.2\n---\n6\n"),
    (
        "%YAML 1.1\n---\n# h\nk: v\n",
        &[],
        "sort_keys(.)",
        "%YAML 1.1\n---\n# h\nk: v\n",
    ),
    (
        "# a\n%YAML 1.2\n---\nk: v\n",
        &[],
        ".k = 1",
        "# a\n%YAML 1.2\n---\nk: 1\n",
    ),
    ("%YAML 1.2\nk: v\n", &[], ".k = 1", "%YAML 1.2\nk: 1\n"),
    (
        "%YAML 1.2\n# c\nk: v\n",
        &[],
        ".k = 1",
        "%YAML 1.2\n# c\nk: 1\n",
    ),
    (
        "  %YAML 1.2\n---\nk: v\n",
        &[],
        ".k = 1",
        "#   %YAML 1.2\n---\nk: 1\n",
    ),
    (
        "  %YAML 1.2\nk: v\n",
        &[],
        ".k = 1",
        "#   %YAML 1.2\nk: 1\n",
    ),
    (
        "---\n%YAML 1.1\nk: v\n",
        &[],
        ".k = 1",
        "---\n%YAML 1.1\nk: 1\n",
    ),
    (
        "%YAML 1.2\r\n---\r\nk: v\r\n",
        &[],
        ".k = 1",
        "%YAML 1.2\r\n---\r\nk: 1\n",
    ),
];

const INDENTED: &[Row] = &[
    (
        "\t# tab\n# a\nk: v\n",
        &[],
        ".k = 1",
        "\t# tab\n# a\nk: 1\n",
    ),
    (
        "\t# tab\n# a\nk: v\n",
        &[],
        "to_entries",
        "\t# tab\n# a\n- key: k\n  value: v\n",
    ),
    (
        "\t# tab\n# a\nk: v\n",
        &[],
        "sort_keys(.)",
        "\t# tab\n# a\nk: v\n",
    ),
    ("\t# tab\n# a\nk: v\n", &["-P"], ".", "\t# tab\n# a\nk: v\n"),
    (
        "  # two\n- 1\n- 2\n",
        &[],
        ".[0] = 9",
        "  # two\n- 9\n- 2\n",
    ),
    (
        "---\n  # ind\nk: v\n",
        &[],
        ".k = 1",
        "---\n  # ind\nk: 1\n",
    ),
    (
        "# p\n\t# tab\n---\n\t# tab\nk: v\n",
        &[],
        ".k = 1",
        "# p\n\t# tab\n---\n\t# tab\nk: 1\n",
    ),
    (
        "---\n\t\n# h\nk: v\n",
        &[],
        ".zz = 1",
        "---\n# \t\n# h\nk: v\nzz: 1\n",
    ),
    (
        "---\n\t\n\nk: v\n",
        &[],
        ".zz = 1",
        "---\n# \t\n\nk: v\nzz: 1\n",
    ),
    (
        "\t\n%YAML 1.1\nk: v # lc\n",
        &["-P"],
        ".",
        "# \t\n%YAML 1.1\nk: v # lc\n",
    ),
    (
        "---\n%YAML 1.1\n\t# tab\nk: v\n",
        &[],
        ".k = 1",
        "---\n%YAML 1.1\n\t# tab\nk: 1\n",
    ),
    (
        "---\n%YAML 1.1\n \n# c\nk: v\n",
        &[],
        ".k = 1",
        "---\n%YAML 1.1\n#  \n# c\nk: 1\n",
    ),
    (
        "# p\r\n---\r\n%YAML 1.1\r\n# c\r\nk: v\r\n",
        &[],
        ".k = 1",
        "# p\r\n---\r\n%YAML 1.1\r\n# c\r\nk: 1\n",
    ),
];

const EMPTY_ROOT: &[Row] = &[
    ("---\n# a\n[]\n", &[], ".[0] = 9", "---\n# a\n- 9\n"),
    ("---\n# a\n[]\n", &[], "sort_keys(.)", "---\n# a\n[]\n"),
    ("---\n# a\n[]\n", &["-P"], ".", "---\n# a\n[]\n"),
    ("---\n# a\n{}\n", &[], ".a = 1", "---\n# a\na: 1\n"),
    ("---\n# a\n{}\n", &[], "del(.zz)", "---\n# a\n{}\n"),
    ("---\n# a\n\n[]\n", &[], ".[0] = 1", "---\n# a\n\n- 1\n"),
    (
        "# p\n---\n# a\n{}\n",
        &[],
        "with_entries(.)",
        "# p\n---\n# a\n{}\n",
    ),
];

const SCALAR_HEAD_WRITE: &[Row] = &[
    ("---\n# a\n5\n", &[], r#". head_comment = "x""#, "5\n"),
    ("---\n# a\n5\n", &[], r#". head_comment |= "x""#, "5\n"),
    ("---\n# a\n5\n", &[], r#". comments = "x""#, "5\n"),
    (
        "---\n# a\n5\n",
        &[],
        r#". foot_comment = "x""#,
        "---\n# a\n5\n",
    ),
    ("# p\n---\n5\n", &[], r#". head_comment = "x""#, "5\n"),
    ("# a\n5\n", &[], r#". head_comment = "x""#, "5\n"),
    ("%YAML 1.2\n---\n5\n", &[], r#". head_comment = "x""#, "5\n"),
];

fn check(rows: &[Row]) -> Result<()> {
    for (doc, args, filter, expected) in rows {
        let (stdout, code) = run_yq_stdin(filter, doc, args)?;
        assert_eq!(
            (stdout.as_str(), code),
            (*expected, 0),
            "{doc:?} {args:?} {filter}"
        );
    }
    Ok(())
}

#[test]
fn test_a_yaml_directive_is_kept_in_front_of_a_written_document_7099() -> Result<()> {
    check(DIRECTIVE)
}

#[test]
fn test_an_indented_or_blank_header_line_is_printed_as_yq_prints_it_7099() -> Result<()> {
    check(INDENTED)
}

#[test]
fn test_the_comment_after_the_marker_stays_over_an_empty_root_7099() -> Result<()> {
    check(EMPTY_ROOT)
}

#[test]
fn test_a_head_written_to_a_scalar_root_replaces_the_header_7099() -> Result<()> {
    check(SCALAR_HEAD_WRITE)
}
