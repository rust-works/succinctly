//! #7027: yq decides what its first positional argument is by looking at the file system
//! (`processArgs`, `cmd/utils.go`). With neither `--expression` nor `--from-file`: a name that ends
//! in `.yq` and is a file holds the expression; `-` and any other file are inputs, and the
//! expression is `.`; anything else, a directory included, is the expression. `-n` refuses files.
//! Every row was captured from yq v4.53.3 in a directory holding `d.yaml` = `a: 1` / `b: x`,
//! `other.yaml` = `c: 3`, `x.yq` = `.a` and `y.txt` = `.b`.

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::Result;

const D: &str = "a: 1\nb: x\n";

struct Outcome {
    stdout: String,
    stderr: String,
    code: i32,
}

/// Runs `succinctly yq ARGS` in `dir`, feeding `stdin` to it.
fn run(dir: &Path, args: &[&str], stdin: &str) -> Result<Outcome> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .current_dir(dir)
        .arg("yq")
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    child
        .stdin
        .take()
        .expect("piped")
        .write_all(stdin.as_bytes())?;
    let output = child.wait_with_output()?;
    Ok(Outcome {
        stdout: String::from_utf8(output.stdout)?,
        stderr: String::from_utf8(output.stderr)?,
        code: output.status.code().unwrap_or(-1),
    })
}

fn workdir() -> Result<tempfile::TempDir> {
    let dir = tempfile::tempdir()?;
    std::fs::write(dir.path().join("d.yaml"), D)?;
    std::fs::write(dir.path().join("other.yaml"), "c: 3\n")?;
    std::fs::write(dir.path().join("x.yq"), ".a")?;
    std::fs::write(dir.path().join("y.txt"), ".b")?;
    std::fs::create_dir(dir.path().join("dir1"))?;
    Ok(dir)
}

/// (args, stdout): each the same as yq's, exit status 0, nothing on stderr.
const OK_ROWS: &[(&[&str], &str)] = &[
    // An existing file first: it is an input and the expression is `.`.
    (&["d.yaml"], D),
    (&["-o=json", "-I=0", "d.yaml"], "{\"a\":1,\"b\":\"x\"}\n"),
    (&["-P", "d.yaml"], D),
    (&["-e", "d.yaml"], D),
    (&["d.yaml", "other.yaml"], "a: 1\nb: x\n---\nc: 3\n"),
    (&["-N", "d.yaml", "other.yaml"], "a: 1\nb: x\nc: 3\n"),
    (&["d.yaml", "d.yaml"], "a: 1\nb: x\n---\na: 1\nb: x\n"),
    // Any file, whatever its name, and a second positional is a file too.
    (&["y.txt", "d.yaml"], ".b\n---\na: 1\nb: x\n"),
    // A name that is not a file is the expression.
    (&[".a", "d.yaml"], "1\n"),
    (&["", "d.yaml"], D),
    // `.yq` holds the expression, when it is the first positional.
    (&["x.yq", "d.yaml"], "1\n"),
    (&["-n", "x.yq"], "null\n"),
    (&["d.yaml", "x.yq"], "a: 1\nb: x\n---\n.a\n"),
    // The flags that name the expression leave the positionals alone.
    (&["--from-file", "x.yq", "d.yaml"], "1\n"),
    (&["--expression", ".a", "d.yaml"], "1\n"),
];

#[test]
fn a_first_positional_that_names_a_file_is_an_input_7027() -> Result<()> {
    let dir = workdir()?;
    for (args, want) in OK_ROWS {
        let got = run(dir.path(), args, "")?;
        assert_eq!(
            (got.stdout.as_str(), got.stderr.as_str(), got.code),
            (*want, "", 0),
            "{args:?}"
        );
    }
    Ok(())
}

/// `-` is standard input, so with it first the expression is `.`.
#[test]
fn a_lone_dash_is_standard_input_and_the_expression_is_identity_7027() -> Result<()> {
    let dir = workdir()?;
    let got = run(dir.path(), &["-"], "q: 7\n")?;
    assert_eq!(
        (got.stdout.as_str(), got.stderr.as_str(), got.code),
        ("q: 7\n", "", 0)
    );
    let got = run(dir.path(), &[".q", "-"], "q: 7\n")?;
    assert_eq!(
        (got.stdout.as_str(), got.stderr.as_str(), got.code),
        ("7\n", "", 0)
    );
    Ok(())
}

/// A file that is named like an expression wins over it, which is what `--expression` is for.
#[test]
fn a_file_named_like_an_expression_is_read_as_a_file_7027() -> Result<()> {
    let dir = workdir()?;
    std::fs::write(dir.path().join(".a"), "z: 1\n")?;
    let got = run(dir.path(), &[".a"], "")?;
    assert_eq!(
        (got.stdout.as_str(), got.stderr.as_str(), got.code),
        ("z: 1\n", "", 0)
    );
    let got = run(dir.path(), &["--expression", ".a", "d.yaml"], "")?;
    assert_eq!(
        (got.stdout.as_str(), got.stderr.as_str(), got.code),
        ("1\n", "", 0)
    );
    Ok(())
}

/// `-n` refuses files, whether they came first or after the expression.
#[test]
fn null_input_refuses_files_7027() -> Result<()> {
    let dir = workdir()?;
    for args in [&["-n", ".", "d.yaml"][..], &["-n", "d.yaml"]] {
        let got = run(dir.path(), args, "")?;
        // A CI run sets `RUST_BACKTRACE`, and anyhow then appends one after the error line.
        assert_eq!((got.stdout.as_str(), got.code), ("", 1), "{args:?}");
        assert!(
            got.stderr
                .starts_with("Error: cannot pass files in when using null-input flag\n"),
            "{args:?}: {}",
            got.stderr
        );
    }
    Ok(())
}

/// A directory is not a file: first it is an expression (which does not parse), after an
/// expression it is an input that cannot be read. Both exit 1, the second after the first file.
#[test]
fn a_directory_is_not_an_input_file_7027() -> Result<()> {
    let dir = workdir()?;
    let got = run(dir.path(), &["dir1", "d.yaml"], "")?;
    assert_eq!((got.stdout.as_str(), got.code), ("", 1), "{}", got.stderr);
    let got = run(dir.path(), &["d.yaml", "dir1"], "")?;
    assert_eq!((got.stdout.as_str(), got.code), (D, 1), "{}", got.stderr);
    Ok(())
}
