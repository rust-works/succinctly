//! #6985: yq's operator spellings of three encoders (`to_csv`/`tocsv`, `to_tsv`/`totsv`,
//! `to_props`/`toprops`) and its text loaders (`load_str`, `strload`, `loadstr`, `load_base64`), and
//! yq's rules for `load`'s argument (its first output only, a scalar by its text, no output is
//! `filename expression returned nil`) and for a file it cannot read.
//!
//! Every row is captured from yq v4.53.3 with `-o=json -I=0`, run in a directory holding the files
//! below.

use std::process::Command;

use anyhow::Result;
use tempfile::TempDir;

use crate::run_jq_stdin;

const DOC: &str = "a: 1\nl: [1, \"x,y\", null]\nm: {p: 1, q: [2, 3]}\n";

/// Run `succinctly yq -o=json -I=0 FILTER d.yaml` in `dir`: (stdout, stderr, exit code).
fn yq_in(dir: &TempDir, filter: &str, extra: &[&str]) -> Result<(String, String, i32)> {
    let output = Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .current_dir(dir.path())
        .arg("yq")
        .args(["-o=json", "-I=0"])
        .args(extra)
        .arg(filter)
        .arg("d.yaml")
        .output()?;
    Ok((
        String::from_utf8(output.stdout)?,
        String::from_utf8(output.stderr)?,
        output.status.code().unwrap_or(-1),
    ))
}

fn fixture() -> Result<TempDir> {
    let dir = TempDir::new()?;
    let p = dir.path();
    std::fs::write(p.join("d.yaml"), DOC)?;
    std::fs::write(p.join("s.txt"), "hi\nthere\n")?;
    std::fs::write(p.join("b64.txt"), "aGk=")?;
    std::fs::write(p.join("b64nl.txt"), "aGVsbG8gd29ybGQ=\n")?;
    std::fs::write(p.join("bom.txt"), b"\xEF\xBB\xBFhi")?;
    std::fs::write(p.join("x.txt"), "aG k=")?;
    std::fs::write(p.join("1"), "x")?;
    std::fs::create_dir(p.join("adir"))?;
    Ok(dir)
}

/// `(filter, -o=json -I=0 output)`.
const ANSWERED: &[(&str, &str)] = &[
    (".l | to_csv", "\"1,\\\"x,y\\\",null\"\n"),
    (".l | tocsv", "\"1,\\\"x,y\\\",null\"\n"),
    (
        "[.l, .l] | to_csv",
        "\"1,\\\"x,y\\\",null\\n1,\\\"x,y\\\",null\"\n",
    ),
    (".l | to_tsv", "\"1\\tx,y\\tnull\"\n"),
    (".l | totsv", "\"1\\tx,y\\tnull\"\n"),
    (".m | to_props", "\"p = 1\\nq.0 = 2\\nq.1 = 3\\n\"\n"),
    (".m | toprops", "\"p = 1\\nq.0 = 2\\nq.1 = 3\\n\"\n"),
    (".a | to_props", "\"1\\n\"\n"),
    ("load_str(\"s.txt\")", "\"hi\\nthere\\n\"\n"),
    ("strload(\"s.txt\")", "\"hi\\nthere\\n\"\n"),
    ("loadstr(\"s.txt\")", "\"hi\\nthere\\n\"\n"),
    ("load_str(\"s.txt\") | length", "9\n"),
    ("load_str(\"s.txt\") | tag", "\"!!str\"\n"),
    ("\"s.txt\" as $f | load_str($f)", "\"hi\\nthere\\n\"\n"),
    (
        "[load_str(\"s.txt\", \"b64.txt\")]",
        "[\"hi\\nthere\\n\"]\n",
    ),
    ("load_str(\"bom.txt\")", "\"\u{feff}hi\"\n"),
    ("load_str(1)", "\"x\"\n"),
    ("load_base64(\"b64.txt\")", "\"hi\"\n"),
    ("loadbase64(\"b64.txt\")", "\"hi\"\n"),
    ("str_load(\"s.txt\")", "\"hi\\nthere\\n\"\n"),
    ("load_base64(\"b64nl.txt\")", "\"hello world\"\n"),
    ("[load(\"s.txt\", \"b64.txt\")]", "[\"hi there\"]\n"),
    ("load(1)", "\"x\"\n"),
];

/// `(filter, error)`: refused, nothing printed.
const REFUSED: &[(&str, &str)] = &[
    (
        ".m | to_csv",
        "csv encoding only works for arrays, got: !!map",
    ),
    (
        "load_str(\"nope\")",
        "failed to load nope: open nope: no such file or directory",
    ),
    (
        "load_str(\"\")",
        "failed to load : open : no such file or directory",
    ),
    (
        "load_str(true)",
        "failed to load true: open true: no such file or directory",
    ),
    (
        "load_str([1])",
        "failed to load : open : no such file or directory",
    ),
    ("load_str(.zz)", "filename expression returned nil"),
    ("load_str(error(\"e\"))", "e"),
    ("load_str(\"s.txt\", error(\"e\"))", "e"),
    (
        "load_str(\"adir\")",
        "failed to load adir: read adir: is a directory",
    ),
    (
        "load_base64(\"adir\")",
        "failed to load adir: read adir: is a directory",
    ),
    (
        "load_base64(\"bom.txt\")",
        "failed to load bom.txt: bad file 'bom.txt': illegal base64 data at input byte 0",
    ),
    (
        "load_base64(\"x.txt\")",
        "failed to load x.txt: bad file 'x.txt': illegal base64 data at input byte 2",
    ),
    (
        "load(\"nope\")",
        "failed to load nope: open nope: no such file or directory",
    ),
    (
        "load(2)",
        "failed to load 2: open 2: no such file or directory",
    ),
    ("load(.zz)", "filename expression returned nil"),
];

#[test]
fn yq_encoder_spellings_and_text_loaders_6985() -> Result<()> {
    let dir = fixture()?;
    for &(filter, expected) in ANSWERED {
        let (stdout, stderr, code) = yq_in(&dir, filter, &[])?;
        assert_eq!((stdout.as_str(), code), (expected, 0), "{filter}: {stderr}");
    }
    for &(filter, expected) in REFUSED {
        let (stdout, stderr, code) = yq_in(&dir, filter, &[])?;
        assert_eq!((stdout.as_str(), code), ("", 1), "{filter}: {stderr}");
        assert_eq!(stderr.trim_end(), format!("Error: {expected}"), "{filter}");
    }
    Ok(())
}

/// A program that defines one of the encoder names calls its own definition (`def` needs
/// `--jq-extensions` in yq mode).
#[test]
fn yq_encoder_spelling_defined_by_the_program_wins_6985() -> Result<()> {
    let dir = fixture()?;
    let (stdout, stderr, code) = yq_in(&dir, "def to_csv: 5; to_csv", &["--jq-extensions"])?;
    assert_eq!((stdout.as_str(), code), ("5\n", 0), "{stderr}");
    Ok(())
}

/// jq has none of these names, so a jq program may define them, and `load` keeps its jq-mode rules.
#[test]
fn jq_mode_has_no_yq_loaders_or_encoder_spellings_6985() -> Result<()> {
    for (filter, expected) in [
        ("def to_csv: 5; to_csv", "5\n"),
        ("def load_str(f): 7; load_str(1)", "7\n"),
    ] {
        let (stdout, code) = run_jq_stdin(filter, "null", &["-c"])?;
        assert_eq!((stdout.as_str(), code), (expected, 0), "{filter}");
    }
    for filter in ["load_str(\"s.txt\")", "[1] | to_csv", "1 | toprops"] {
        let (_, code) = run_jq_stdin(filter, "null", &["-c"])?;
        assert_eq!(code, 3, "{filter}: not defined in jq mode");
    }
    Ok(())
}

/// A file `load` cannot parse is a bad file, named as yq names it; the reason after the prefix is
/// this parser's own (recorded in `docs/compliance/yq/limitations.md`).
#[test]
fn yq_load_names_a_file_that_does_not_parse_6985() -> Result<()> {
    let dir = fixture()?;
    std::fs::write(dir.path().join("bad.yaml"), "a: [1\n")?;
    std::fs::write(dir.path().join("bad.json"), "{\"a\": 1")?;
    for name in ["bad.yaml", "bad.json"] {
        let (stdout, stderr, code) = yq_in(&dir, &format!("load(\"{name}\")"), &[])?;
        assert_eq!((stdout.as_str(), code), ("", 1), "{name}: {stderr}");
        let prefix = format!("Error: failed to load {name}: bad file '{name}': ");
        assert!(stderr.starts_with(&prefix), "{name}: {stderr}");
    }
    Ok(())
}
