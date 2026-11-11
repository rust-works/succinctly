//! #6969: under `--eval-all`, the result of an arithmetic operator keeps where its left operand was
//! read, so `document_index` and `file_index` of `. * {"z":1}` are each result's own document and
//! file, as in yq. They were `0` for every result: the pairing was built as a node with no origin
//! (a comparison's boolean is such a node, and still has none). This holds for `+` and `*`, the
//! merge flags (`*+`, `*d`, `*n`), a constructed left operand (`{"z":1} * .`), and a number built
//! from a read (`.x + 1`). Every row is a live yq v4.53.3 capture (`yq ea -o=json -I0`), kept only
//! where both agree.
//!
//! Left out because they differ for another reason (`docs/compliance/yq/limitations.md`): `line` of
//! a computed node, the result count of `. as $d | $d * {...}`, and a map merge or concat keeping
//! `EvaluateTogether`, so that `. * {"z":1} | [.]` collects every result in yq.

use anyhow::Result;
use std::io::Write;
use tempfile::NamedTempFile;

use crate::run_yq_files;

/// (file set, filter, expected stdout). `md` is one file of two documents, `two` is two files, the
/// second of two documents.
const ROWS: &[(&str, &str, &str)] = &[
    ("md", ". * {\"z\":1} | document_index", "0\n0\n1\n1\n"),
    ("md", ". * {\"z\":1} | file_index", "0\n0\n0\n0\n"),
    ("md", ". + {\"z\":1} | document_index", "0\n0\n1\n1\n"),
    ("md", ". + {\"z\":1} | file_index", "0\n0\n0\n0\n"),
    ("md", "{\"z\":1} * . | document_index", "0\n0\n1\n1\n"),
    ("md", "{\"z\":1} * . | file_index", "0\n0\n0\n0\n"),
    ("md", ". * . | document_index", "0\n0\n1\n1\n"),
    ("md", ". * . | file_index", "0\n0\n0\n0\n"),
    ("md", ". + . | document_index", "0\n0\n1\n1\n"),
    ("md", ". + . | file_index", "0\n0\n0\n0\n"),
    ("md", ".x * 2 | document_index", "0\n0\n1\n1\n"),
    ("md", ".x * 2 | file_index", "0\n0\n0\n0\n"),
    ("md", ".x + 1 | document_index", "0\n0\n1\n1\n"),
    ("md", ".x + 1 | file_index", "0\n0\n0\n0\n"),
    ("md", ".x + .y | document_index", "0\n0\n1\n1\n"),
    ("md", ".x + .y | file_index", "0\n0\n0\n0\n"),
    ("md", ". *+ {\"z\":1} | document_index", "0\n0\n1\n1\n"),
    ("md", ". *+ {\"z\":1} | file_index", "0\n0\n0\n0\n"),
    ("md", ". *d {\"z\":1} | document_index", "0\n0\n1\n1\n"),
    ("md", ". *d {\"z\":1} | file_index", "0\n0\n0\n0\n"),
    ("md", ". *n {\"z\":1} | document_index", "0\n0\n1\n1\n"),
    ("md", ". *n {\"z\":1} | file_index", "0\n0\n0\n0\n"),
    (
        "md",
        ". * {\"z\":1} * {\"w\":2} | document_index",
        "0\n0\n0\n0\n1\n1\n1\n1\n",
    ),
    (
        "md",
        ". * {\"z\":1} * {\"w\":2} | file_index",
        "0\n0\n0\n0\n0\n0\n0\n0\n",
    ),
    ("md", "({\"z\":1} * .) | document_index", "0\n0\n1\n1\n"),
    ("md", "({\"z\":1} * .) | file_index", "0\n0\n0\n0\n"),
    ("md", ". == . | document_index", "0\n0\n0\n0\n"),
    ("md", ". == . | file_index", "0\n0\n0\n0\n"),
    ("md", ". * {\"z\":1} | .z | document_index", "0\n0\n1\n1\n"),
    ("md", ". * {\"z\":1} | .z | file_index", "0\n0\n0\n0\n"),
    ("md", "[. * {\"z\":1}] | document_index", "0\n"),
    ("md", "[. * {\"z\":1}] | file_index", "0\n"),
    (
        "md",
        "select(.x == 1) * {\"z\":1} | document_index",
        "0\n0\n",
    ),
    ("md", "select(.x == 1) * {\"z\":1} | file_index", "0\n0\n"),
    ("md", ". // {\"z\":1} | document_index", "0\n1\n"),
    ("md", ". // {\"z\":1} | file_index", "0\n0\n"),
    ("md", ".a * 2 | document_index", ""),
    ("md", ".a * 2 | file_index", ""),
    ("md", ".a + 1 | document_index", "0\n0\n"),
    ("md", ".a + 1 | file_index", "0\n0\n"),
    (
        "two",
        ". * {\"z\":1} | document_index",
        "0\n0\n0\n0\n0\n0\n1\n1\n1\n",
    ),
    (
        "two",
        ". * {\"z\":1} | file_index",
        "0\n0\n0\n1\n1\n1\n1\n1\n1\n",
    ),
    (
        "two",
        ". + {\"z\":1} | document_index",
        "0\n0\n0\n0\n0\n0\n1\n1\n1\n",
    ),
    (
        "two",
        ". + {\"z\":1} | file_index",
        "0\n0\n0\n1\n1\n1\n1\n1\n1\n",
    ),
    (
        "two",
        "{\"z\":1} * . | document_index",
        "0\n0\n0\n0\n0\n0\n1\n1\n1\n",
    ),
    (
        "two",
        "{\"z\":1} * . | file_index",
        "0\n0\n0\n1\n1\n1\n1\n1\n1\n",
    ),
    (
        "two",
        ". * . | document_index",
        "0\n0\n0\n0\n0\n0\n1\n1\n1\n",
    ),
    ("two", ". * . | file_index", "0\n0\n0\n1\n1\n1\n1\n1\n1\n"),
    (
        "two",
        ". + . | document_index",
        "0\n0\n0\n0\n0\n0\n1\n1\n1\n",
    ),
    ("two", ". + . | file_index", "0\n0\n0\n1\n1\n1\n1\n1\n1\n"),
    ("two", ".x * 2 | document_index", ""),
    ("two", ".x * 2 | file_index", ""),
    ("two", ".x + 1 | document_index", "0\n0\n0\n"),
    ("two", ".x + 1 | file_index", "0\n0\n0\n"),
    ("two", ".x + .y | document_index", ""),
    ("two", ".x + .y | file_index", ""),
    (
        "two",
        ". *+ {\"z\":1} | document_index",
        "0\n0\n0\n0\n0\n0\n1\n1\n1\n",
    ),
    (
        "two",
        ". *+ {\"z\":1} | file_index",
        "0\n0\n0\n1\n1\n1\n1\n1\n1\n",
    ),
    (
        "two",
        ". *d {\"z\":1} | document_index",
        "0\n0\n0\n0\n0\n0\n1\n1\n1\n",
    ),
    (
        "two",
        ". *d {\"z\":1} | file_index",
        "0\n0\n0\n1\n1\n1\n1\n1\n1\n",
    ),
    (
        "two",
        ". *n {\"z\":1} | document_index",
        "0\n0\n0\n0\n0\n0\n1\n1\n1\n",
    ),
    (
        "two",
        ". *n {\"z\":1} | file_index",
        "0\n0\n0\n1\n1\n1\n1\n1\n1\n",
    ),
    (
        "two",
        ". * {\"z\":1} * {\"w\":2} | document_index",
        "0\n0\n0\n0\n0\n0\n0\n0\n0\n0\n0\n0\n0\n0\n0\n0\n0\n0\n1\n1\n1\n1\n1\n1\n1\n1\n1\n",
    ),
    (
        "two",
        ". * {\"z\":1} * {\"w\":2} | file_index",
        "0\n0\n0\n0\n0\n0\n0\n0\n0\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n",
    ),
    (
        "two",
        "({\"z\":1} * .) | document_index",
        "0\n0\n0\n0\n0\n0\n1\n1\n1\n",
    ),
    (
        "two",
        "({\"z\":1} * .) | file_index",
        "0\n0\n0\n1\n1\n1\n1\n1\n1\n",
    ),
    (
        "two",
        ". == . | document_index",
        "0\n0\n0\n0\n0\n0\n0\n0\n0\n",
    ),
    ("two", ". == . | file_index", "0\n0\n0\n0\n0\n0\n0\n0\n0\n"),
    (
        "two",
        ". * {\"z\":1} | .z | document_index",
        "0\n0\n0\n0\n0\n0\n1\n1\n1\n",
    ),
    (
        "two",
        ". * {\"z\":1} | .z | file_index",
        "0\n0\n0\n1\n1\n1\n1\n1\n1\n",
    ),
    ("two", "[. * {\"z\":1}] | document_index", "0\n"),
    ("two", "[. * {\"z\":1}] | file_index", "0\n"),
    ("two", "select(.x == 1) * {\"z\":1} | document_index", ""),
    ("two", "select(.x == 1) * {\"z\":1} | file_index", ""),
    ("two", ". // {\"z\":1} | document_index", "0\n0\n1\n"),
    ("two", ". // {\"z\":1} | file_index", "0\n1\n1\n"),
    (
        "two",
        ".a * 2 | document_index",
        "0\n0\n0\n0\n0\n0\n1\n1\n1\n",
    ),
    ("two", ".a * 2 | file_index", "0\n0\n0\n1\n1\n1\n1\n1\n1\n"),
    (
        "two",
        ".a + 1 | document_index",
        "0\n0\n0\n0\n0\n0\n1\n1\n1\n",
    ),
    ("two", ".a + 1 | file_index", "0\n0\n0\n1\n1\n1\n1\n1\n1\n"),
    ("md", ".x - 1 | document_index", "0\n0\n1\n1\n"),
    ("md", ".x - 1 | file_index", "0\n0\n0\n0\n"),
    ("md", ".x / 2 | document_index", "0\n0\n1\n1\n"),
    ("md", ".x / 2 | file_index", "0\n0\n0\n0\n"),
    ("md", ".x % 2 | document_index", "0\n0\n1\n1\n"),
    ("md", ".x % 2 | file_index", "0\n0\n0\n0\n"),
    ("md", ".x - .y | document_index", "0\n0\n1\n1\n"),
    ("md", ".x - .y | file_index", "0\n0\n0\n0\n"),
    ("md", ".x / .y | document_index", "0\n0\n1\n1\n"),
    ("md", ".x / .y | file_index", "0\n0\n0\n0\n"),
    ("md", ".x * .y | document_index", "0\n0\n1\n1\n"),
    ("md", ".x * .y | file_index", "0\n0\n0\n0\n"),
    // A left operand that builds a fresh node (a literal, `length`) has no origin: all `0`.
    ("two", "10 + .x | document_index", "0\n0\n0\n"),
    ("two", "10 + .x | file_index", "0\n0\n0\n"),
    (
        "two",
        "\"a\" + \"b\" | document_index",
        "0\n0\n0\n0\n0\n0\n0\n0\n0\n",
    ),
    (
        "two",
        "\"a\" + \"b\" | file_index",
        "0\n0\n0\n0\n0\n0\n0\n0\n0\n",
    ),
    (
        "two",
        "1 + 2 | document_index",
        "0\n0\n0\n0\n0\n0\n0\n0\n0\n",
    ),
    ("two", "1 + 2 | file_index", "0\n0\n0\n0\n0\n0\n0\n0\n0\n"),
    (
        "two",
        "length + 1 | document_index",
        "0\n0\n0\n0\n0\n0\n0\n0\n0\n",
    ),
    (
        "two",
        "length + 1 | file_index",
        "0\n0\n0\n0\n0\n0\n0\n0\n0\n",
    ),
    (
        "two",
        ".x + 1 + 2 | document_index",
        "0\n0\n0\n0\n0\n0\n0\n0\n0\n",
    ),
    (
        "two",
        ".x + 1 + 2 | file_index",
        "0\n0\n0\n0\n0\n0\n0\n0\n0\n",
    ),
    ("two", "select(.x == 1) + 5 | document_index", "0\n0\n0\n"),
    ("two", "select(.x == 1) + 5 | file_index", "0\n0\n0\n"),
    ("two", "(.x) + 1 | document_index", "0\n0\n0\n"),
    ("two", "(.x) + 1 | file_index", "0\n0\n0\n"),
];

fn file(text: &str) -> Result<NamedTempFile> {
    let mut f = NamedTempFile::new()?;
    f.write_all(text.as_bytes())?;
    Ok(f)
}

#[test]
fn test_arithmetic_result_keeps_the_left_operands_origin_under_eval_all_6969() -> Result<()> {
    let md = file("x: 1\ny: 2\n---\nx: 3\ny: 4\n")?;
    let (f1, f2) = (file("a: 1\n")?, file("a: 2\n---\na: 3\n")?);
    for (set, filter, expected) in ROWS {
        let files = match *set {
            "md" => vec![md.path()],
            _ => vec![f1.path(), f2.path()],
        };
        let (stdout, stderr, code) =
            run_yq_files(filter, &files, &["--eval-all", "-o=json", "-I=0"])?;
        assert_eq!(
            (stdout.as_str(), code),
            (*expected, 0),
            "{set} | {filter}: {stderr}"
        );
    }
    Ok(())
}

/// `filename` follows the same origin: each merged result names the file of its left operand.
#[test]
fn test_merge_result_names_the_file_it_was_read_from_6969() -> Result<()> {
    let (f1, f2) = (file("a: 1\n")?, file("a: 2\n---\na: 3\n")?);
    let (stdout, stderr, code) = run_yq_files(
        ". * {\"z\":1} | filename",
        &[f1.path(), f2.path()],
        &["--eval-all", "-o=json", "-I=0"],
    )?;
    let (n1, n2) = (
        serde_json::to_string(&f1.path().to_string_lossy())?,
        serde_json::to_string(&f2.path().to_string_lossy())?,
    );
    let expected = [&n1, &n1, &n1, &n2, &n2, &n2, &n2, &n2, &n2]
        .iter()
        .map(|name| format!("{name}\n"))
        .collect::<String>();
    assert_eq!((stdout, code), (expected, 0), "{stderr}");
    Ok(())
}
