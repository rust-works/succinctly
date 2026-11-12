//! #7040: yq's `eval` (`e`) and `eval-all` (`ea`) subcommands. cobra takes the first positional
//! argument as the subcommand when it names one, wherever the flags stand around it, and only the
//! first. `eval` is the root command's own behaviour; `eval-all` evaluates once over every
//! document. Every row was captured from yq v4.53.3 in a directory holding `d.yaml` = `a: 1` /
//! `b: x`, `other.yaml` = `c: 3` and `x.yq` = `.a`.

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
    Ok(dir)
}

/// (args, stdout): each the same as yq's, exit status 0, nothing on stderr.
const OK_ROWS: &[(&[&str], &str)] = &[
    // `e` and `eval` are the root command.
    (&["e", ".a", "d.yaml"], "1\n"),
    (&["eval", ".a", "d.yaml"], "1\n"),
    (&["e", "d.yaml"], D),
    (&["eval", "d.yaml"], D),
    (&["e", "", "d.yaml"], D),
    // `ea` and `eval-all` evaluate once over every document.
    (
        &["ea", ".", "d.yaml", "other.yaml"],
        "a: 1\nb: x\n---\nc: 3\n",
    ),
    (
        &["eval-all", ".", "d.yaml", "other.yaml"],
        "a: 1\nb: x\n---\nc: 3\n",
    ),
    (
        &["-N", "ea", ".", "d.yaml", "other.yaml"],
        "a: 1\nb: x\nc: 3\n",
    ),
    (&["ea", "select(fi == 1)", "d.yaml", "other.yaml"], "c: 3\n"),
    (
        &["ea", "[.]", "d.yaml", "other.yaml"],
        "- a: 1\n  b: x\n- c: 3\n",
    ),
    // `-n`, in front of the subcommand or behind it, is not a file argument.
    (&["-n", "e", "1"], "1\n"),
    (&["e", "-n", "2"], "2\n"),
    (&["eval", "-n", "1 + 1"], "2\n"),
    (&["ea", "-n", "3"], "3\n"),
    (&["eval-all", "-n", "1"], "1\n"),
    // Flags stand before or after the subcommand, a value-taking one included.
    (
        &["-o", "json", "e", ".", "d.yaml"],
        "{\n  \"a\": 1,\n  \"b\": \"x\"\n}\n",
    ),
    (
        &["-I", "4", "-o=json", "e", ".", "d.yaml"],
        "{\n    \"a\": 1,\n    \"b\": \"x\"\n}\n",
    ),
    (
        &["e", "-o=json", "-I=0", ".", "d.yaml"],
        "{\"a\":1,\"b\":\"x\"}\n",
    ),
    // #7027's rules apply to what follows the subcommand.
    (&["e", "x.yq", "d.yaml"], "1\n"),
    (&["e", "--expression", ".a", "d.yaml"], "1\n"),
];

#[test]
fn the_eval_and_eval_all_subcommands_answer_as_yq_does_7040() -> Result<()> {
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

/// With no file, the subcommand reads standard input.
#[test]
fn a_subcommand_without_files_reads_standard_input_7040() -> Result<()> {
    let dir = workdir()?;
    for (args, want) in [
        (&["e"][..], "q: 7\n"),
        (&["ea"], "q: 7\n"),
        (&["e", ".q"], "7\n"),
        (&["e", "-"], "q: 7\n"),
    ] {
        let got = run(dir.path(), args, "q: 7\n")?;
        assert_eq!(
            (got.stdout.as_str(), got.stderr.as_str(), got.code),
            (want, "", 0),
            "{args:?}"
        );
    }
    Ok(())
}

/// Only the first positional is a subcommand, and the name is case-sensitive: anything else is the
/// expression or a file, and fails as it did before.
#[test]
fn only_the_first_positional_and_only_lowercase_is_a_subcommand_7040() -> Result<()> {
    let dir = workdir()?;
    for args in [
        &[".a", "e", "d.yaml"][..],
        &["e", "e", "d.yaml"],
        &["E", ".", "d.yaml"],
        &["EVAL", ".", "d.yaml"],
    ] {
        let got = run(dir.path(), args, "")?;
        assert_eq!(got.code, 1, "{args:?}: {}", got.stderr);
    }
    // `.a e d.yaml` prints nothing before the file `e` fails to open; the file `d.yaml` follows it.
    let got = run(dir.path(), &[".a", "e", "d.yaml"], "")?;
    assert_eq!(got.stdout, "");
    Ok(())
}
