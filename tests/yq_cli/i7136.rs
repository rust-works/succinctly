//! #7136: yq reads a CSV/TSV cell with `parseSnippet`, which decodes **one** YAML document and never reads what
//! follows it. So a cell that opens with a flow collection is that collection when text follows it
//! (`[1-4];[5-8];9` is `["1-4"]`), a first document that ends at `---` or `...` is the cell and a later
//! document is not read (`a: 1` / `---` / `b: 2` is `{"a":1}`), and a document marker with nothing
//! to hold is a null. A first document that is a string keeps the whole text, and so does a snippet
//! whose first node does not parse (`a: [1]x`).
//!
//! The text after a complete first node is still scanned by go-yaml, two tokens of it: a `-` or `?`
//! that does not start a line, an unterminated quote, `@`, a backtick, `%`, a block scalar header
//! followed by text, and an anchor or alias name that a bracket follows are yq's error, which leaves the
//! cell its text (`[1] - x`, `[1] 'q`, `[23][89]|4[3-79]`).
//!
//! Every row was captured from yq v4.53.3 (`-p csv|tsv -o json -I=0 .`, the cell the only field under
//! a header `h`). What still differs is in `limitations.md`.

use std::io::Write;
use std::process::{Command, Stdio};

use anyhow::Result;

/// `(cell, stdout)` under `-p csv`.
const ROWS: &[(&str, &str)] = &[
    ("[1-4];[5-8];9", "[{\"h\":[\"1-4\"]}]\n"),
    ("[1]x", "[{\"h\":[1]}]\n"),
    ("[1] x", "[{\"h\":[1]}]\n"),
    ("[1];y", "[{\"h\":[1]}]\n"),
    ("[1]]", "[{\"h\":[1]}]\n"),
    ("[1],", "[{\"h\":[1]}]\n"),
    ("[a, b]x", "[{\"h\":[\"a\",\"b\"]}]\n"),
    ("&a [1]x", "[{\"h\":[1]}]\n"),
    ("{a: 1}x", "[{\"h\":{\"a\":1}}]\n"),
    ("{a: 1} x", "[{\"h\":{\"a\":1}}]\n"),
    ("[1]\nx", "[{\"h\":[1]}]\n"),
    ("[1]\n- x", "[{\"h\":[1]}]\n"),
    ("[1]\n---\n[2]", "[{\"h\":[1]}]\n"),
    ("[1] # c", "[{\"h\":[1]}]\n"),
    ("[1] [2]", "[{\"h\":[1]}]\n"),
    (
        "[23][89]|4[3-79];[89]00;N/A",
        "[{\"h\":\"[23][89]|4[3-79];[89]00;N/A\"}]\n",
    ),
    ("a: 1\n---\nb: 2", "[{\"h\":{\"a\":1}}]\n"),
    ("a: 1\n...\n", "[{\"h\":{\"a\":1}}]\n"),
    ("- a\n- b\n---\n- c", "[{\"h\":[\"a\",\"b\"]}]\n"),
    ("5\n---\n1", "[{\"h\":5}]\n"),
    ("1\n...\n2", "[{\"h\":1}]\n"),
    ("---\n---\n", "[{\"h\":null}]\n"),
    ("---", "[{\"h\":null}]\n"),
    ("--- # c", "[{\"h\":null}]\n"),
    ("# c\n---\n", "[{\"h\":null}]\n"),
    ("x\n---\ny", "[{\"h\":\"x\\n---\\ny\"}]\n"),
    ("---\na\n---\nb", "[{\"h\":\"---\\na\\n---\\nb\"}]\n"),
    ("a: 1\nb", "[{\"h\":\"a: 1\\nb\"}]\n"),
    ("a: [1]x", "[{\"h\":\"a: [1]x\"}]\n"),
    ("[1] - x", "[{\"h\":\"[1] - x\"}]\n"),
    ("[1]'", "[{\"h\":\"[1]'\"}]\n"),
    ("[1] 'q", "[{\"h\":\"[1] 'q\"}]\n"),
    ("[1] | x", "[{\"h\":\"[1] | x\"}]\n"),
    ("[1] > x", "[{\"h\":\"[1] > x\"}]\n"),
    ("[1] @", "[{\"h\":\"[1] @\"}]\n"),
    ("[1] %", "[{\"h\":\"[1] %\"}]\n"),
    ("[1] ?", "[{\"h\":\"[1] ?\"}]\n"),
    ("[1]\n? x", "[{\"h\":[1]}]\n"),
    ("[1]\n\"q", "[{\"h\":\"[1]\\n\\\"q\"}]\n"),
    ("[1]]\n'x", "[{\"h\":\"[1]]\\n'x\"}]\n"),
    ("[1]\n---\n\"x", "[{\"h\":\"[1]\\n---\\n\\\"x\"}]\n"),
    ("[1]\n---\n[2] \"", "[{\"h\":[1]}]\n"),
    ("[1]*a[1]", "[{\"h\":\"[1]*a[1]\"}]\n"),
    ("{a: 1}*a]", "[{\"h\":{\"a\":1}}]\n"),
    ("[1]&a x", "[{\"h\":[1]}]\n"),
    ("[1]x\n'x", "[{\"h\":[1]}]\n"),
    ("{a: 1}\n---\n", "[{\"h\":{\"a\":1}}]\n"),
    ("[1]\n- x\n---\ny", "[{\"h\":[1]}]\n"),
    ("[1, 2]", "[{\"h\":[1,2]}]\n"),
    ("{a: 1}", "[{\"h\":{\"a\":1}}]\n"),
    ("[1]\n...\n[", "[{\"h\":[1]}]\n"),
];

/// The same cells under `-p tsv`.
const TSV: &[(&str, &str)] = &[
    ("[1-4];[5-8];9", "[{\"h\":[\"1-4\"]}]\n"),
    ("[1]x", "[{\"h\":[1]}]\n"),
    ("a: 1\n---\nb: 2", "[{\"h\":{\"a\":1}}]\n"),
];

/// Under `--csv-auto-parse=false` a cell is its own text, whatever it holds.
const NO_AUTO_PARSE: &[(&str, &str)] = &[
    ("[1]x", "[{\"h\":\"[1]x\"}]\n"),
    ("a: 1\n---\nb: 2", "[{\"h\":\"a: 1\\n---\\nb: 2\"}]\n"),
    ("[1-4];[5-8];9", "[{\"h\":\"[1-4];[5-8];9\"}]\n"),
];

fn run(format: &str, extra: &[&str], cell: &str) -> Result<(String, Option<i32>)> {
    let delimiter = if format == "tsv" { '\t' } else { ',' };
    let quoted = format!("\"{}\"", cell.replace('"', "\"\""));
    let table = format!("\"h\"\n{quoted}\n");
    let _ = delimiter;
    let mut child = Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .args(["yq", "-p", format, "-o", "json", "-I=0"])
        .args(extra)
        .arg(".")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    child.stdin.take().unwrap().write_all(table.as_bytes())?;
    let output = child.wait_with_output()?;
    Ok((
        String::from_utf8_lossy(&output.stdout).into_owned(),
        output.status.code(),
    ))
}

#[test]
fn a_cell_is_the_first_node_of_its_first_document_as_in_yq_7136() -> Result<()> {
    for &(cell, want) in ROWS {
        assert_eq!(
            run("csv", &[], cell)?,
            (want.to_string(), Some(0)),
            "{cell:?}"
        );
    }
    Ok(())
}

#[test]
fn tsv_reads_a_cell_the_same_way_7136() -> Result<()> {
    for &(cell, want) in TSV {
        assert_eq!(
            run("tsv", &[], cell)?,
            (want.to_string(), Some(0)),
            "{cell:?}"
        );
    }
    Ok(())
}

#[test]
fn without_auto_parse_a_cell_is_its_text_7136() -> Result<()> {
    for &(cell, want) in NO_AUTO_PARSE {
        assert_eq!(
            run("csv", &["--csv-auto-parse=false"], cell)?,
            (want.to_string(), Some(0)),
            "{cell:?}"
        );
    }
    Ok(())
}
