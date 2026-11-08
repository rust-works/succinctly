//! #6703: yq's `-o, --output-format` defaults to `auto`, the run's own input format: a `.json`
//! file prints JSON, and standard input, `.yaml`, `.yml` and any other name print YAML. The format
//! is the *first* file's (#4394), `-n` and an explicit `-p` keep YAML. Every answer was captured
//! from yq v4.53.3.

use std::io::Write;
use std::process::{Command, Stdio};

use anyhow::Result;
use tempfile::TempDir;

const JSON_DOC: &str = "{\"a\": 1, \"b\": [true, null, \"s\"], \"c\": {\"d\": 1.5}}\n";
const YAML_DOC: &str = "a: 1\nb: [1, 2]\n";
const MULTI_DOC: &str = "a: 1\n---\nb: 2\n";

fn run_in(dir: &TempDir, args: &[&str], files: &[&str], stdin: &str) -> Result<(String, i32)> {
    for (name, text) in [
        ("j.json", JSON_DOC),
        ("j2.json", JSON_DOC),
        ("x.JSON", JSON_DOC),
        ("j.json.bak", JSON_DOC),
        ("y.yaml", YAML_DOC),
        ("y.yml", YAML_DOC),
        ("noext", YAML_DOC),
        ("multi.yaml", MULTI_DOC),
    ] {
        std::fs::write(dir.path().join(name), text)?;
    }
    let mut child = Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .arg("yq")
        .args(args)
        .args(files)
        .current_dir(dir.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    child.stdin.take().unwrap().write_all(stdin.as_bytes())?;
    let output = child.wait_with_output()?;
    Ok((
        String::from_utf8(output.stdout)?,
        output.status.code().unwrap_or(-1),
    ))
}

#[test]
fn test_yq_prints_the_first_files_format_by_default_6703() -> Result<()> {
    let dir = TempDir::new()?;
    for (args, files, want) in [
        (&["."][..], &["j.json"][..], "{\n  \"a\": 1,\n  \"b\": [\n    true,\n    null,\n    \"s\"\n  ],\n  \"c\": {\n    \"d\": 1.5\n  }\n}\n"),
        (&["-r", ".a"][..], &["j.json"][..], "1\n"),
        (&["-I=4", "."][..], &["j.json"][..], "{\n    \"a\": 1,\n    \"b\": [\n        true,\n        null,\n        \"s\"\n    ],\n    \"c\": {\n        \"d\": 1.5\n    }\n}\n"),
        (&["-I=0", "."][..], &["j.json"][..], "{\"a\":1,\"b\":[true,null,\"s\"],\"c\":{\"d\":1.5}}\n"),
        (&["-P", "."][..], &["j.json"][..], "{\n  \"a\": 1,\n  \"b\": [\n    true,\n    null,\n    \"s\"\n  ],\n  \"c\": {\n    \"d\": 1.5\n  }\n}\n"),
        (&["-N", "."][..], &["j.json"][..], "{\n  \"a\": 1,\n  \"b\": [\n    true,\n    null,\n    \"s\"\n  ],\n  \"c\": {\n    \"d\": 1.5\n  }\n}\n"),
        (&[".b"][..], &["j2.json"][..], "[\n  true,\n  null,\n  \"s\"\n]\n"),
        (&[".b[]"][..], &["j.json"][..], "true\nnull\n\"s\"\n"),
        (&[".c"][..], &["j.json"][..], "{\n  \"d\": 1.5\n}\n"),
        (&[".zzz"][..], &["j.json"][..], "null\n"),
        (&[".a = 2"][..], &["j.json"][..], "{\n  \"a\": 2,\n  \"b\": [\n    true,\n    null,\n    \"s\"\n  ],\n  \"c\": {\n    \"d\": 1.5\n  }\n}\n"),
        (&["del(.a)"][..], &["j.json"][..], "{\n  \"b\": [\n    true,\n    null,\n    \"s\"\n  ],\n  \"c\": {\n    \"d\": 1.5\n  }\n}\n"),
        (&["select(.a == 1)"][..], &["j.json"][..], "{\n  \"a\": 1,\n  \"b\": [\n    true,\n    null,\n    \"s\"\n  ],\n  \"c\": {\n    \"d\": 1.5\n  }\n}\n"),
        (&["-e", ".a"][..], &["j.json"][..], "1\n"),
        (&[".a"][..], &["j.json", "j2.json"][..], "1\n1\n"),
        (&[".a"][..], &["multi.yaml", "j.json"][..], "1\n---\nnull\n---\n1\n"),
        (&[".a"][..], &["x.JSON"][..], "1\n"),
        (&[".a"][..], &["j.json.bak"][..], "1\n"),
        (&["-o=yaml", "."][..], &["j.json"][..], "a: 1\nb:\n  - true\n  - null\n  - s\nc:\n  d: 1.5\n"),
        (&["-o=auto", "."][..], &["j.json"][..], "{\n  \"a\": 1,\n  \"b\": [\n    true,\n    null,\n    \"s\"\n  ],\n  \"c\": {\n    \"d\": 1.5\n  }\n}\n"),
        (&["-p=json", "."][..], &["j.json"][..], "a: 1\nb:\n  - true\n  - null\n  - s\nc:\n  d: 1.5\n"),
        (&["-p=yaml", "."][..], &["j.json"][..], "{\"a\": 1, \"b\": [true, null, \"s\"], \"c\": {\"d\": 1.5}}\n"),
        (&[".b"][..], &["y.yaml"][..], "[1, 2]\n"),
        (&[".b"][..], &["y.yml"][..], "[1, 2]\n"),
        (&[".b"][..], &["noext"][..], "[1, 2]\n"),
        (&[".a"][..], &["multi.yaml"][..], "1\n---\nnull\n"),
        (&["-o=json", "-I=0", "."][..], &["y.yaml"][..], "{\"a\":1,\"b\":[1,2]}\n"),
    ] {
        let (stdout, code) = run_in(&dir, args, files, "")?;
        assert_eq!((stdout.as_str(), code), (want, 0), "{args:?} {files:?}");
    }
    Ok(())
}

/// Every file is read with the first one's decoder (#4394), so a YAML file after a JSON one is an
/// error, as in yq (`bad file 'multi.yaml': invalid character 'a'`).
#[test]
fn test_yq_reads_every_file_as_the_first_files_format_6703() -> Result<()> {
    let dir = TempDir::new()?;
    let (_, code) = run_in(&dir, &[".a"], &["j.json", "multi.yaml"], "")?;
    assert_eq!(code, 1);
    Ok(())
}

#[test]
fn test_yq_prints_yaml_for_standard_input_and_null_input_6703() -> Result<()> {
    let dir = TempDir::new()?;
    for (args, stdin, want) in [
        (&[".a"][..], "{\"a\": 1}\n", "1\n"),
        (&[".a"][..], "a: 1\n", "1\n"),
        (&["-p=json", "."][..], "{\"a\": 1}\n", "a: 1\n"),
        (&["-n", "{\"a\": 1}"][..], "", "a: 1\n"),
        (
            &["-p=json", "-o=json", "-I=0", "."][..],
            "{\"a\": 1}\n",
            "{\"a\":1}\n",
        ),
    ] {
        let (stdout, code) = run_in(&dir, args, &[], stdin)?;
        assert_eq!((stdout.as_str(), code), (want, 0), "{args:?}");
    }
    Ok(())
}
