//! #7028: `--split-exp` names each file as yq does, `<expression result>.<extension of the output format>`
//! unless the result already ends in an extension. yq v4.53.3 appends `json` for `-o=json`, `properties`
//! for `-o=props` and `yml` for every other format (`xml`, `toml`, `lua`, `csv` and the rest included),
//! and keeps a name that ends in `.` followed by ASCII letters or digits (`x.txt`, `x.1`, `x.tar.gz`,
//! `x.YML`) but not one that ends in `.`, `.a-b` or a non-ASCII letter (`x..yml`, `x.a-b.yml`).
//! Every row was captured from yq v4.53.3 with `-s 'name'`; the name is built from `$index` so that it
//! does not depend on what the expression reads, which the encoder formats do not yet hand it as yq does.

use anyhow::Result;
use tempfile::TempDir;

use crate::run_yq_stdin_with_stderr;

/// `(flags, input, suffix of the name, the files written)`.
type Row = (
    &'static [&'static str],
    &'static str,
    &'static str,
    &'static [&'static str],
);

const ROWS: &[Row] = &[
    (&[], "a: 1\n---\na: 2\n", "", &["x0.yml", "x1.yml"]),
    (&["-o=yaml"], "a: 1\n---\na: 2\n", "", &["x0.yml", "x1.yml"]),
    (
        &["-o=json"],
        "a: 1\n---\na: 2\n",
        "",
        &["x0.json", "x1.json"],
    ),
    (
        &["-o=props"],
        "a: 1\n---\na: 2\n",
        "",
        &["x0.properties", "x1.properties"],
    ),
    (&["-o=xml"], "a: 1\n---\na: 2\n", "", &["x0.yml", "x1.yml"]),
    (&["-o=toml"], "a: 1\n---\na: 2\n", "", &["x0.yml", "x1.yml"]),
    (&["-o=lua"], "a: 1\n---\na: 2\n", "", &["x0.yml", "x1.yml"]),
    (
        &["-o=shell"],
        "a: 1\n---\na: 2\n",
        "",
        &["x0.yml", "x1.yml"],
    ),
    (
        &["-o=kyaml"],
        "a: 1\n---\na: 2\n",
        "",
        &["x0.yml", "x1.yml"],
    ),
    (&["-o=ini"], "a: 1\n---\na: 2\n", "", &["x0.yml", "x1.yml"]),
    (&["-o=auto"], "a: 1\n---\na: 2\n", "", &["x0.yml", "x1.yml"]),
    (&["-o=csv"], "- 1\n---\n- 2\n", "", &["x0.yml", "x1.yml"]),
    (&["-o=tsv"], "- 1\n---\n- 2\n", "", &["x0.yml", "x1.yml"]),
    (
        &["-o=base64"],
        "hello\n---\nworld\n",
        "",
        &["x0.yml", "x1.yml"],
    ),
    (
        &["-o=uri"],
        "hello\n---\nworld\n",
        "",
        &["x0.yml", "x1.yml"],
    ),
    (&["-p=json"], "{\"a\":1}\n", "", &["x0.yml"]),
    (&["-p=json", "-o=json"], "{\"a\":1}\n", "", &["x0.json"]),
    (&["-p=json", "-o=yaml"], "{\"a\":1}\n", "", &["x0.yml"]),
    (&[], "a: 1\n---\na: 2\n", ".txt", &["x0.txt", "x1.txt"]),
    (
        &["-o=json"],
        "a: 1\n---\na: 2\n",
        ".txt",
        &["x0.txt", "x1.txt"],
    ),
    (&[], "a: 1\n---\na: 2\n", ".yml", &["x0.yml", "x1.yml"]),
    (
        &["-o=json"],
        "a: 1\n---\na: 2\n",
        ".yml",
        &["x0.yml", "x1.yml"],
    ),
    (&[], "a: 1\n---\na: 2\n", ".YML", &["x0.YML", "x1.YML"]),
    (
        &["-o=json"],
        "a: 1\n---\na: 2\n",
        ".YML",
        &["x0.YML", "x1.YML"],
    ),
    (&[], "a: 1\n---\na: 2\n", ".yaml", &["x0.yaml", "x1.yaml"]),
    (
        &["-o=json"],
        "a: 1\n---\na: 2\n",
        ".yaml",
        &["x0.yaml", "x1.yaml"],
    ),
    (&[], "a: 1\n---\na: 2\n", ".json", &["x0.json", "x1.json"]),
    (
        &["-o=json"],
        "a: 1\n---\na: 2\n",
        ".json",
        &["x0.json", "x1.json"],
    ),
    (
        &[],
        "a: 1\n---\na: 2\n",
        ".properties",
        &["x0.properties", "x1.properties"],
    ),
    (
        &["-o=json"],
        "a: 1\n---\na: 2\n",
        ".properties",
        &["x0.properties", "x1.properties"],
    ),
    (&[], "a: 1\n---\na: 2\n", ".", &["x0..yml", "x1..yml"]),
    (
        &["-o=json"],
        "a: 1\n---\na: 2\n",
        ".",
        &["x0..json", "x1..json"],
    ),
    (
        &[],
        "a: 1\n---\na: 2\n",
        ".a-b",
        &["x0.a-b.yml", "x1.a-b.yml"],
    ),
    (
        &["-o=json"],
        "a: 1\n---\na: 2\n",
        ".a-b",
        &["x0.a-b.json", "x1.a-b.json"],
    ),
    (
        &[],
        "a: 1\n---\na: 2\n",
        ".a_b",
        &["x0.a_b.yml", "x1.a_b.yml"],
    ),
    (
        &["-o=json"],
        "a: 1\n---\na: 2\n",
        ".a_b",
        &["x0.a_b.json", "x1.a_b.json"],
    ),
    (&[], "a: 1\n---\na: 2\n", ".1", &["x0.1", "x1.1"]),
    (&["-o=json"], "a: 1\n---\na: 2\n", ".1", &["x0.1", "x1.1"]),
    (
        &[],
        "a: 1\n---\na: 2\n",
        ".tar.gz",
        &["x0.tar.gz", "x1.tar.gz"],
    ),
    (
        &["-o=json"],
        "a: 1\n---\na: 2\n",
        ".tar.gz",
        &["x0.tar.gz", "x1.tar.gz"],
    ),
    (&[], "a: 1\n---\na: 2\n", ".é", &["x0.é.yml", "x1.é.yml"]),
    (
        &["-o=json"],
        "a: 1\n---\na: 2\n",
        ".é",
        &["x0.é.json", "x1.é.json"],
    ),
    (
        &[],
        "a: 1\n---\na: 2\n",
        ".txt.",
        &["x0.txt..yml", "x1.txt..yml"],
    ),
    (
        &["-o=json"],
        "a: 1\n---\na: 2\n",
        ".txt.",
        &["x0.txt..json", "x1.txt..json"],
    ),
    (&[], "a: 1\n---\na: 2\n", "..yml", &["x0..yml", "x1..yml"]),
    (
        &["-o=json"],
        "a: 1\n---\na: 2\n",
        "..yml",
        &["x0..yml", "x1..yml"],
    ),
    (&[], "a: 1\n---\na: 2\n", ".A1b2", &["x0.A1b2", "x1.A1b2"]),
    (
        &["-o=json"],
        "a: 1\n---\na: 2\n",
        ".A1b2",
        &["x0.A1b2", "x1.A1b2"],
    ),
    (
        &[],
        "a: 1\n---\na: 2\n",
        ".a b",
        &["x0.a b.yml", "x1.a b.yml"],
    ),
    (
        &["-o=json"],
        "a: 1\n---\na: 2\n",
        ".a b",
        &["x0.a b.json", "x1.a b.json"],
    ),
];

fn files_written(flags: &[&str], input: &str, suffix: &str) -> Result<(Vec<String>, i32, String)> {
    let dir = TempDir::new()?;
    let tail = if suffix.is_empty() {
        String::new()
    } else {
        format!(" + \"{suffix}\"")
    };
    let pattern = format!("\"{}/x\" + ($index|tostring){tail}", dir.path().display());
    let mut args: Vec<&str> = flags.to_vec();
    args.extend(["--split-exp", &pattern]);
    let (stdout, stderr, code) = run_yq_stdin_with_stderr(".", input, &args)?;
    assert_eq!(stdout, "", "{flags:?} {suffix:?}: stdout is suppressed");
    let mut names = Vec::new();
    for entry in std::fs::read_dir(dir.path())? {
        names.push(entry?.file_name().to_string_lossy().into_owned());
    }
    names.sort();
    Ok((names, code, stderr))
}

#[test]
fn split_exp_appends_the_output_formats_extension_to_a_name_without_one_7028() -> Result<()> {
    for &(flags, input, suffix, want) in ROWS {
        let (names, code, stderr) = files_written(flags, input, suffix)?;
        let names: Vec<&str> = names.iter().map(String::as_str).collect();
        assert_eq!(
            (names.as_slice(), code),
            (want, 0),
            "{flags:?} {suffix:?}: {stderr}"
        );
    }
    Ok(())
}
