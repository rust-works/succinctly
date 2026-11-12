//! #7025: the argument-level yq flags `succinctly yq` used to reject: `--expression`,
//! `--split-exp-file`, `-f`, `-v`/`--verbose`, `--security-disable-env-ops` and
//! `--security-disable-file-ops`. Every row was captured from yq v4.53.3 (`yq FLAGS FILTER FILE`
//! in a directory holding `d.yaml` = `a: 1` / `b: x`, `other.yaml` = `c: 3`, with `FOO=bar`).
//! `docs/compliance/yq/limitations.md` records where `succinctly yq` answers differently.

use std::path::Path;
use std::process::Command;

use anyhow::Result;

const D: &str = "a: 1\nb: x\n";
const OTHER: &str = "c: 3\n";
/// A variable the tests set for the child only, so `env(..)` has something to read.
const VAR: &str = "SUCCINCTLY_TEST_7025";

struct Outcome {
    stdout: String,
    stderr: String,
    code: i32,
}

/// Runs `succinctly yq ARGS` in `dir` with `VAR=bar`, so file names in `ARGS` are relative.
fn run(dir: &Path, args: &[&str]) -> Result<Outcome> {
    let output = Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .current_dir(dir)
        .env(VAR, "bar")
        .arg("yq")
        .args(args)
        .stdin(std::process::Stdio::null())
        .output()?;
    Ok(Outcome {
        stdout: String::from_utf8(output.stdout)?,
        stderr: String::from_utf8(output.stderr)?,
        code: output.status.code().unwrap_or(-1),
    })
}

fn workdir() -> Result<tempfile::TempDir> {
    let dir = tempfile::tempdir()?;
    std::fs::write(dir.path().join("d.yaml"), D)?;
    std::fs::write(dir.path().join("other.yaml"), OTHER)?;
    std::fs::write(dir.path().join("a.exp"), ".a")?;
    std::fs::write(dir.path().join("empty.exp"), "")?;
    std::fs::write(dir.path().join("name.exp"), "\"out-\" + .a + \".yaml\"")?;
    std::fs::write(dir.path().join("fm.md"), "---\nx: 1\n---\nbody\n")?;
    Ok(dir)
}

/// (args, stdout): each the same as yq's, exit status 0, nothing on stderr.
const OK_ROWS: &[(&[&str], &str)] = &[
    // `--expression` is the filter, so every positional is a file.
    (&["--expression", ".a", "d.yaml"], "1\n"),
    (&["--expression", ".a", "-n"], "null\n"),
    // An empty expression is the identity.
    (&["--expression", "", "d.yaml"], D),
    // Two files: the second has no `.a`.
    (
        &["--expression", ".a", "d.yaml", "other.yaml"],
        "1\n---\nnull\n",
    ),
    // `--from-file` wins over `--expression`, in either order.
    (
        &["--expression", ".b", "--from-file", "a.exp", "d.yaml"],
        "1\n",
    ),
    (
        &["--from-file", "a.exp", "--expression", ".b", "d.yaml"],
        "1\n",
    ),
    // `-f` is `--front-matter`.
    (&["-f", "extract", ".", "fm.md"], "---\nx: 1\n"),
    (
        &["-f", "process", ".x = 5", "fm.md"],
        "---\nx: 5\n---\nbody\n",
    ),
    (&["--front-matter=extract", ".", "fm.md"], "---\nx: 1\n"),
    // An empty `--split-exp-file` means no split, and it wins over `--split-exp`.
    (&["--split-exp-file", "empty.exp", ".", "d.yaml"], D),
    (
        &[
            "--split-exp-file",
            "empty.exp",
            "--split-exp",
            "\"never/\" + .a",
            ".",
            "d.yaml",
        ],
        D,
    ),
    // `-v` is accepted and leaves standard output alone.
    (&["-v", ".a", "d.yaml"], "1\n"),
    (&["-vv", "-n", "1"], "1\n"),
    // The operators the flags do not name still run.
    (
        &["--security-disable-env-ops", ".a = \"x\"", "d.yaml"],
        "a: x\nb: x\n",
    ),
    (
        &["--security-disable-env-ops", "\"\\(.a)\"", "d.yaml"],
        "1\n",
    ),
    (&["--security-disable-file-ops", ".a", "d.yaml"], "1\n"),
    (&["--security-disable-file-ops", "-n", "1"], "1\n"),
    (
        &[
            "--security-disable-file-ops",
            "--from-file",
            "a.exp",
            "d.yaml",
        ],
        "1\n",
    ),
    // An operator no value reaches is not run, so it is not refused.
    (
        &[
            "--security-disable-env-ops",
            ".a // strenv(SUCCINCTLY_TEST_7025)",
            "d.yaml",
        ],
        "1\n",
    ),
    (
        &[
            "--security-disable-file-ops",
            ".a // load(\"other.yaml\")",
            "d.yaml",
        ],
        "1\n",
    ),
    // Without the flags the operators run.
    (&["-n", "env(SUCCINCTLY_TEST_7025)"], "bar\n"),
    (&["-n", "strenv(SUCCINCTLY_TEST_7025)"], "bar\n"),
    (&["-n", "load(\"other.yaml\")"], "c: 3\n"),
    (&["-n", "load_str(\"other.yaml\")"], "c: 3\n\n"),
];

#[test]
fn the_argument_level_flags_answer_as_yq_does_7025() -> Result<()> {
    let dir = workdir()?;
    for (args, want) in OK_ROWS {
        let got = run(dir.path(), args)?;
        assert_eq!(
            (got.stdout.as_str(), got.stderr.as_str(), got.code),
            (*want, "", 0),
            "{args:?}"
        );
    }
    Ok(())
}

/// (args, yq's error line): exit status 1 and nothing on standard output.
const REFUSED_ROWS: &[(&[&str], &str)] = &[
    (
        &[
            "--security-disable-env-ops",
            "env(SUCCINCTLY_TEST_7025)",
            "d.yaml",
        ],
        "Error: env operations have been disabled\n",
    ),
    (
        &[
            "--security-disable-env-ops",
            "strenv(SUCCINCTLY_TEST_7025)",
            "d.yaml",
        ],
        "Error: env operations have been disabled\n",
    ),
    (
        &[
            "--security-disable-env-ops",
            "strenv(SUCCINCTLY_TEST_7025)",
            "-n",
        ],
        "Error: env operations have been disabled\n",
    ),
    (
        &["--security-disable-env-ops", "envsubst", "d.yaml"],
        "Error: env operations have been disabled\n",
    ),
    (
        &[
            "--security-disable-env-ops",
            "\"$HOME\" | envsubst",
            "d.yaml",
        ],
        "Error: env operations have been disabled\n",
    ),
    (
        &[
            "--security-disable-env-ops",
            ".a = env(SUCCINCTLY_TEST_7025)",
            "d.yaml",
        ],
        "Error: env operations have been disabled\n",
    ),
    (
        &[
            "--security-disable-env-ops",
            ".zz // strenv(SUCCINCTLY_TEST_7025)",
            "d.yaml",
        ],
        "Error: env operations have been disabled\n",
    ),
    (
        &[
            "--security-disable-env-ops",
            "[.a, strenv(SUCCINCTLY_TEST_7025)]",
            "d.yaml",
        ],
        "Error: env operations have been disabled\n",
    ),
    (
        &[
            "--security-disable-file-ops",
            "load(\"other.yaml\")",
            "d.yaml",
        ],
        "Error: file operations have been disabled\n",
    ),
    (
        &[
            "--security-disable-file-ops",
            "load_str(\"other.yaml\")",
            "d.yaml",
        ],
        "Error: file operations have been disabled\n",
    ),
    (
        &[
            "--security-disable-file-ops",
            "load_base64(\"YQ==\")",
            "d.yaml",
        ],
        "Error: file operations have been disabled\n",
    ),
    (
        &["--security-disable-file-ops", "-n", "load(\"x\") // 1"],
        "Error: file operations have been disabled\n",
    ),
    (
        &[
            "--security-disable-env-ops",
            "--security-disable-file-ops",
            "-n",
            "load(\"x\") // 1",
        ],
        "Error: file operations have been disabled\n",
    ),
];

#[test]
fn the_security_flags_refuse_the_operators_they_name_7025() -> Result<()> {
    let dir = workdir()?;
    for (args, want) in REFUSED_ROWS {
        let got = run(dir.path(), args)?;
        assert_eq!(
            (got.stdout.as_str(), got.stderr.as_str(), got.code),
            ("", *want, 1),
            "{args:?}"
        );
    }
    Ok(())
}

/// `--split-exp-file` names the output file as `--split-exp` does, and wins over it.
#[test]
fn split_exp_file_names_the_output_file_7025() -> Result<()> {
    let dir = workdir()?;
    let got = run(dir.path(), &["--split-exp-file", "name.exp", ".", "d.yaml"])?;
    assert_eq!((got.stdout.as_str(), got.code), ("", 0), "{}", got.stderr);
    assert_eq!(std::fs::read_to_string(dir.path().join("out-1.yaml"))?, D);

    let got = run(
        dir.path(),
        &[
            "--split-exp-file",
            "name.exp",
            "--split-exp",
            "\"other-\" + .b + \".yaml\"",
            ".",
            "d.yaml",
        ],
    )?;
    assert_eq!((got.stdout.as_str(), got.code), ("", 0), "{}", got.stderr);
    assert!(!dir.path().join("other-x.yaml").exists());
    Ok(())
}

/// A missing `--split-exp-file` is an error, as in yq (the wording differs).
#[test]
fn a_missing_split_exp_file_is_an_error_7025() -> Result<()> {
    let dir = workdir()?;
    let got = run(dir.path(), &["--split-exp-file", "nope.exp", ".", "d.yaml"])?;
    assert_eq!((got.stdout.as_str(), got.code), ("", 1));
    assert!(got.stderr.contains("nope.exp"), "{}", got.stderr);
    Ok(())
}

/// A file operation is not an input file: the flag leaves reading the input, `-i` and
/// `--split-exp` alone, as in yq.
#[test]
fn the_file_flag_leaves_input_in_place_and_split_alone_7025() -> Result<()> {
    let dir = workdir()?;
    let got = run(
        dir.path(),
        &["--security-disable-file-ops", "-i", ".a = 9", "d.yaml"],
    )?;
    assert_eq!((got.stdout.as_str(), got.code), ("", 0), "{}", got.stderr);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("d.yaml"))?,
        "a: 9\nb: x\n"
    );

    let got = run(
        dir.path(),
        &[
            "--security-disable-file-ops",
            "--split-exp",
            "\"out-\" + .c + \".yaml\"",
            ".",
            "other.yaml",
        ],
    )?;
    assert_eq!((got.stdout.as_str(), got.code), ("", 0), "{}", got.stderr);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("out-3.yaml"))?,
        OTHER
    );
    Ok(())
}
