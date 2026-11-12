//! #7036: `--split-exp` under an encoder output format (`-o=props|shell|csv|tsv|lua|xml|toml|ini|base64|uri`)
//! evaluates its name expression against the result, as yq does, and writes the encoded text. The encoder
//! used to wrap the program, so the expression read the encoded text (`-o=props -s '.a'` saw a string, not
//! `a`) and named every file `x`. Now the program runs unwrapped and each result is encoded as it is
//! written, after its name is known. yq opens the file before it encodes, so a result the encoder refuses
//! (`-o=csv` over a mapping) leaves an empty file and ends the run with exit 1. Every row was captured from
//! yq v4.53.3 (`-s EXPR . f.yaml`): its file names, their contents and its exit code.
//!
//! Not covered, because it differs for a reason of its own: yq starts the second and later files of
//! `-o=yaml` and `-o=kyaml` with `---`.

use std::process::{Command, Stdio};

use anyhow::Result;
use tempfile::TempDir;

/// `(input, split expression, flags, exit code, files written with their contents)`.
type Row = (
    &'static str,
    &'static str,
    &'static [&'static str],
    i32,
    &'static [(&'static str, &'static str)],
);

const ROWS: &[Row] = &[
    (
        "a: 1\n---\na: 2\n",
        "\"n\"+(.a|tostring)",
        &["-o=props"],
        0,
        &[("n1.properties", "a = 1\n"), ("n2.properties", "a = 2\n")],
    ),
    (
        "a: 1\n---\na: 2\n",
        "\"n\"+(.a|tostring)",
        &["-o=shell"],
        0,
        &[("n1.yml", "a=1\n"), ("n2.yml", "a=2\n")],
    ),
    (
        "a: 1\n---\na: 2\n",
        "\"n\"+(.a|tostring)",
        &["-o=xml"],
        0,
        &[("n1.yml", "<a>1</a>\n"), ("n2.yml", "<a>2</a>\n")],
    ),
    (
        "a: 1\n---\na: 2\n",
        "\"n\"+(.a|tostring)",
        &["-o=lua"],
        0,
        &[
            ("n1.yml", "return {\n\t[\"a\"] = 1;\n};\n"),
            ("n2.yml", "return {\n\t[\"a\"] = 2;\n};\n"),
        ],
    ),
    (
        "a: 1\n---\na: 2\n",
        "\"n\"+(.a|tostring)",
        &["-o=toml"],
        0,
        &[("n1.yml", "a = 1\n"), ("n2.yml", "a = 2\n")],
    ),
    (
        "a: 1\n---\na: 2\n",
        "\"n\"+(.a|tostring)",
        &["-o=ini"],
        0,
        &[("n1.yml", "a = 1\n"), ("n2.yml", "a = 2\n")],
    ),
    (
        "a: 1\n---\na: 2\n",
        "\"n\"+($index|tostring)",
        &["-o=props"],
        0,
        &[("n0.properties", "a = 1\n"), ("n1.properties", "a = 2\n")],
    ),
    (
        "- 1\n- 2\n---\n- a\n- \"b c\"\n",
        "\"c\"+(.[0]|tostring)",
        &["-o=csv"],
        0,
        &[("c1.yml", "1,2\n"), ("ca.yml", "a,b c\n")],
    ),
    (
        "a: 1\n---\na: 2\n",
        "\"c\"+($index|tostring)",
        &["-o=csv"],
        1,
        &[("c0.yml", "")],
    ),
    (
        "- 1\n- 2\n---\n- a\n- \"b c\"\n",
        "\"c\"+(.[0]|tostring)",
        &["-o=tsv"],
        0,
        &[("c1.yml", "1\t2\n"), ("ca.yml", "a\tb c\n")],
    ),
    (
        "a: 1\n---\na: 2\n",
        "\"c\"+($index|tostring)",
        &["-o=tsv"],
        1,
        &[("c0.yml", "")],
    ),
    (
        "hello\n---\nworld\n",
        "\"s\"+.",
        &["-o=base64"],
        0,
        &[("shello.yml", "aGVsbG8="), ("sworld.yml", "d29ybGQ=")],
    ),
    (
        "hello\n---\nworld\n",
        "\"s\"+.",
        &["-o=uri"],
        0,
        &[("shello.yml", "hello"), ("sworld.yml", "world")],
    ),
    (
        "{\"a\":1,\"b\":{\"c\":[1,2]}}\n",
        "\"j\"+(.a|tostring)",
        &["-p=json", "-o=props"],
        0,
        &[("j1.properties", "a = 1\nb.c.0 = 1\nb.c.1 = 2\n")],
    ),
    (
        "a: 1\nb: {c: [1, 2]}\n",
        "\"p\"+(.b.c|length|tostring)",
        &["-o=xml"],
        0,
        &[("p2.yml", "<a>1</a>\n<b>\n  <c>1</c>\n  <c>2</c>\n</b>\n")],
    ),
    (
        "a: 1\nb: 2\n",
        "\"x\"+(.|keys|join(\"-\"))",
        &["-o=props"],
        0,
        &[("xa-b.properties", "a = 1\nb = 2\n")],
    ),
    (
        "a: 1\n---\na: 2\n",
        "\"n\"+(.a|tostring)",
        &["-o=props", "--properties-separator=:"],
        0,
        &[("n1.properties", "a:1\n"), ("n2.properties", "a:2\n")],
    ),
    (
        "- a\n- b\n",
        "\"x\"+(.[0])",
        &["-o=csv", "--csv-separator=;"],
        0,
        &[("xa.yml", "a;b\n")],
    ),
];

/// The files under `dir` other than `f.yaml`, with their contents, sorted by path.
fn written(dir: &std::path::Path) -> Result<Vec<(String, String)>> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if path.is_file() && name != "f.yaml" {
            out.push((name, std::fs::read_to_string(&path)?));
        }
    }
    out.sort();
    Ok(out)
}

#[test]
fn split_exp_under_an_encoder_format_names_by_the_result_and_writes_the_encoding_7036() -> Result<()>
{
    for &(input, expr, flags, want_code, want_files) in ROWS {
        let dir = TempDir::new()?;
        std::fs::write(dir.path().join("f.yaml"), input)?;
        let output = Command::new(env!("CARGO_BIN_EXE_succinctly"))
            .current_dir(dir.path())
            .arg("yq")
            .args(flags)
            .args(["--split-exp", expr, ".", "f.yaml"])
            .stdin(Stdio::null())
            .output()?;
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(
            output.status.code(),
            Some(want_code),
            "{flags:?} {expr}: {stderr}"
        );
        let got = written(dir.path())?;
        let got: Vec<(&str, &str)> = got.iter().map(|(n, c)| (n.as_str(), c.as_str())).collect();
        assert_eq!(got, want_files, "{flags:?} {expr}");
    }
    Ok(())
}

/// `-n` has no input to name from, so the name reads each result of the program: `$index` numbers the
/// encoded files, and an encoder error ends the run.
#[test]
fn split_exp_under_an_encoder_format_with_null_input_7036() -> Result<()> {
    let dir = TempDir::new()?;
    let output = Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .current_dir(dir.path())
        .args([
            "yq",
            "-n",
            "-o=props",
            "--split-exp",
            "\"nn\" + ($index|tostring)",
            "{\"a\":1}, {\"a\":2}",
        ])
        .stdin(Stdio::null())
        .output()?;
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        written(dir.path())?,
        [
            ("nn0.properties".to_string(), "a = 1\n".to_string()),
            ("nn1.properties".to_string(), "a = 2\n".to_string())
        ]
    );
    // The encoder refuses the first result: the file is made empty, the second is never written.
    let dir = TempDir::new()?;
    let output = Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .current_dir(dir.path())
        .args([
            "yq",
            "-n",
            "-o=csv",
            "--split-exp",
            "\"cc\" + ($index|tostring)",
            "{\"a\":1}, {\"a\":2}",
        ])
        .stdin(Stdio::null())
        .output()?;
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        written(dir.path())?,
        [("cc0.yml".to_string(), String::new())]
    );
    Ok(())
}
