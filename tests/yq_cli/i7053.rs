//! #7053: `--split-exp` writes every file through one printer, as yq does, so the first result of a
//! document after the run's first is written with the `---` separator. Two results of one document
//! (`.[]`, a null input's) are written back to back, as they are on stdout (#2486); `--no-doc` removes
//! the separator; JSON, `props`, `xml` and the other encoders have none (kyaml does). A document whose
//! filter gives no result does not count, so the file after it still carries the separator. Every row
//! was captured from yq v4.53.3 (`-s EXPR FILTER f.yaml`): its file names and their contents.

use std::process::{Command, Stdio};

use anyhow::Result;
use tempfile::TempDir;

/// `(input files, split expression, filter, flags, files written with their contents)`; the first of
/// the input files is `f.yaml`, a second one `g.yaml`.
type Row = (
    &'static [&'static str],
    &'static str,
    &'static str,
    &'static [&'static str],
    &'static [(&'static str, &'static str)],
);

const N: &str = "\"n\"+(.a|tostring)";

const ROWS: &[Row] = &[
    // The separator comes before the second and later documents' files.
    (
        &["a: 1\n---\na: 2\n---\na: 3\n"],
        N,
        ".",
        &[],
        &[
            ("n1.yml", "a: 1\n"),
            ("n2.yml", "---\na: 2\n"),
            ("n3.yml", "---\na: 3\n"),
        ],
    ),
    (
        &["a: 1\n---\na: 2\n---\na: 3\n"],
        N,
        ".",
        &["-o=yaml", "-P"],
        &[
            ("n1.yml", "a: 1\n"),
            ("n2.yml", "---\na: 2\n"),
            ("n3.yml", "---\na: 3\n"),
        ],
    ),
    // Several results from several documents: the separator is before each document's first.
    (
        &["- 1\n- 2\n---\n- 3\n- 4\n---\n- 5\n"],
        "\"n\"+(.|tostring)",
        ".[]",
        &[],
        &[
            ("n1.yml", "1\n"),
            ("n2.yml", "2\n"),
            ("n3.yml", "---\n3\n"),
            ("n4.yml", "4\n"),
            ("n5.yml", "---\n5\n"),
        ],
    ),
    // A raw scalar result carries it too, and `-0`'s terminator does not guard it: each file starts
    // fresh, so no line break is put before the `---`.
    (
        &["x\n---\ny\n---\nz\n"],
        "\"s\"+.",
        ".",
        &["-r"],
        &[
            ("sx.yml", "x\n"),
            ("sy.yml", "---\ny\n"),
            ("sz.yml", "---\nz\n"),
        ],
    ),
    (
        &["x\n---\ny\n---\nz\n"],
        "\"s\"+.",
        ".",
        &["-0"],
        &[
            ("sx.yml", "x\0"),
            ("sy.yml", "---\ny\0"),
            ("sz.yml", "---\nz\0"),
        ],
    ),
    // The other encoders have none either.
    (
        &["[1, 2]\n---\n[3, 4]\n"],
        "\"c\"+(.[0]|tostring)",
        ".",
        &["-o=csv"],
        &[("c1.yml", "1,2\n"), ("c3.yml", "3,4\n")],
    ),
    (
        &["[1, 2]\n---\n[3, 4]\n"],
        "\"c\"+(.[0]|tostring)",
        ".",
        &["-o=tsv"],
        &[("c1.yml", "1\t2\n"), ("c3.yml", "3\t4\n")],
    ),
    (
        &["a: 1\n---\na: 2\n"],
        N,
        ".",
        &["-o=toml"],
        &[("n1.yml", "a = 1\n"), ("n2.yml", "a = 2\n")],
    ),
    // A second input file is a new document too.
    (
        &["a: 1\n", "a: 2\n"],
        N,
        ".",
        &[],
        &[("n1.yml", "a: 1\n"), ("n2.yml", "---\na: 2\n")],
    ),
    // Two results of one document are one document's: no separator between them.
    (
        &["- 1\n- 2\n- 3\n"],
        "\"n\"+(.|tostring)",
        ".[]",
        &[],
        &[("n1.yml", "1\n"), ("n2.yml", "2\n"), ("n3.yml", "3\n")],
    ),
    // The run's first result has none, whichever document it comes from, and a document without a
    // result is not counted.
    (
        &["a: 1\n---\na: 2\n---\na: 3\n"],
        N,
        "select(document_index == 1 or document_index == 2)",
        &[],
        &[("n2.yml", "a: 2\n"), ("n3.yml", "---\na: 3\n")],
    ),
    (
        &["a: 1\n---\na: 2\n---\na: 3\n"],
        N,
        "select(.a != 2)",
        &[],
        &[("n1.yml", "a: 1\n"), ("n3.yml", "---\na: 3\n")],
    ),
    // A comment-only first document is printed before the separator, in the first file.
    (
        &["# only comment\n---\na: 2\n---\na: 3\n"],
        N,
        ".",
        &[],
        &[
            ("n2.yml", "# only comment\n---\na: 2\n"),
            ("n3.yml", "---\na: 3\n"),
        ],
    ),
    // `--no-doc` removes it.
    (
        &["a: 1\n---\na: 2\n---\na: 3\n"],
        N,
        ".",
        &["-N"],
        &[
            ("n1.yml", "a: 1\n"),
            ("n2.yml", "a: 2\n"),
            ("n3.yml", "a: 3\n"),
        ],
    ),
    // Encodings: kyaml is a YAML stream, the rest have no separator.
    (
        &["a: 1\n---\na: 2\n"],
        N,
        ".",
        &["-o=kyaml"],
        &[
            ("n1.yml", "{\n  a: 1,\n}\n"),
            ("n2.yml", "---\n{\n  a: 2,\n}\n"),
        ],
    ),
    (
        &["a: 1\n---\na: 2\n"],
        N,
        ".",
        &["-o=kyaml", "-N"],
        &[("n1.yml", "{\n  a: 1,\n}\n"), ("n2.yml", "{\n  a: 2,\n}\n")],
    ),
    (
        &["a: 1\n---\na: 2\n"],
        N,
        ".",
        &["-o=json"],
        &[
            ("n1.json", "{\n  \"a\": 1\n}\n"),
            ("n2.json", "{\n  \"a\": 2\n}\n"),
        ],
    ),
    (
        &["a: 1\n---\na: 2\n"],
        N,
        ".",
        &["-o=props"],
        &[("n1.properties", "a = 1\n"), ("n2.properties", "a = 2\n")],
    ),
    (
        &["a: 1\n---\na: 2\n"],
        N,
        ".",
        &["-o=xml"],
        &[("n1.yml", "<a>1</a>\n"), ("n2.yml", "<a>2</a>\n")],
    ),
];

/// The files under `dir` other than the inputs, with their contents, sorted by path.
fn written(dir: &std::path::Path) -> Result<Vec<(String, String)>> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if path.is_file() && name != "f.yaml" && name != "g.yaml" {
            out.push((name, std::fs::read_to_string(&path)?));
        }
    }
    out.sort();
    Ok(out)
}

#[test]
fn split_exp_files_after_the_first_document_start_with_the_separator_7053() -> Result<()> {
    for &(inputs, expr, filter, flags, want_files) in ROWS {
        let dir = TempDir::new()?;
        let mut names = Vec::new();
        for (text, name) in inputs.iter().zip(["f.yaml", "g.yaml"]) {
            std::fs::write(dir.path().join(name), text)?;
            names.push(name);
        }
        let output = Command::new(env!("CARGO_BIN_EXE_succinctly"))
            .current_dir(dir.path())
            .arg("yq")
            .args(flags)
            .args(["--split-exp", expr, filter])
            .args(&names)
            .stdin(Stdio::null())
            .output()?;
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(
            output.status.code(),
            Some(0),
            "{flags:?} {filter}: {stderr}"
        );
        let got = written(dir.path())?;
        let got: Vec<(&str, &str)> = got.iter().map(|(n, c)| (n.as_str(), c.as_str())).collect();
        assert_eq!(got, want_files, "{flags:?} {filter} {inputs:?}");
    }
    Ok(())
}

/// A null input's results are one document's, so there is no separator between its files.
#[test]
fn split_exp_over_a_null_input_has_no_separator_7053() -> Result<()> {
    let dir = TempDir::new()?;
    let output = Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .current_dir(dir.path())
        .args(["yq", "-n", "--split-exp", "\"n\"+tostring", "1, 2, 3"])
        .stdin(Stdio::null())
        .output()?;
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        written(dir.path())?,
        [("n1.yml", "1\n"), ("n2.yml", "2\n"), ("n3.yml", "3\n")]
            .map(|(n, c)| (n.to_string(), c.to_string()))
    );
    Ok(())
}
