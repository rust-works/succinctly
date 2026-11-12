//! #7065: an error a `--split-exp` name expression raises ends the run, as one the program raises does
//! (#2810): yq reports it once, has written the files before the document it failed on and writes none
//! after it. `succinctly yq` reported it once per document and went on writing the later files.
//! `--on-error=continue` keeps writing them (#5507). The rows were captured from yq v4.53.3
//! (`-s '"n"+(.a|tonumber|tostring)' . f.yaml` over three documents with `a: x` in one of them): the
//! files left, and that the error is reported once; the wording of a `tonumber` error is another
//! difference.

use std::process::{Command, Stdio};

use anyhow::Result;
use tempfile::TempDir;

/// The three documents `a: 1`, `a: 2`, `a: 3` with the `bad`th (1-based) replaced by `a: x`.
fn input(bad: usize) -> String {
    (1..=3)
        .map(|i| {
            if i == bad {
                "a: x\n".to_string()
            } else {
                format!("a: {i}\n")
            }
        })
        .collect::<Vec<_>>()
        .join("---\n")
}

/// The run's exit code, the number of error lines it printed, and the `n*.yml` files left.
fn run(bad: usize, flags: &[&str]) -> Result<(i32, usize, Vec<String>)> {
    let dir = TempDir::new()?;
    std::fs::write(dir.path().join("f.yaml"), input(bad))?;
    let output = Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .current_dir(dir.path())
        .arg("yq")
        .args(flags)
        .args(["--split-exp", "\"n\"+(.a|tonumber|tostring)", ".", "f.yaml"])
        .stdin(Stdio::null())
        .output()?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    let errors = stderr.lines().filter(|l| l.starts_with("Error:")).count();
    let mut files: Vec<String> = std::fs::read_dir(dir.path())?
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with('n'))
        .collect();
    files.sort();
    Ok((output.status.code().unwrap_or(-1), errors, files))
}

#[test]
fn a_name_expression_error_ends_the_run_7065() -> Result<()> {
    assert_eq!(run(1, &[])?, (1, 1, vec![]));
    assert_eq!(run(2, &[])?, (1, 1, vec!["n1.yml".to_string()]));
    assert_eq!(
        run(3, &[])?,
        (1, 1, vec!["n1.yml".to_string(), "n2.yml".to_string()])
    );
    Ok(())
}

#[test]
fn continue_keeps_writing_the_files_after_a_name_error_7065() -> Result<()> {
    assert_eq!(
        run(2, &["--on-error=continue"])?,
        (1, 1, vec!["n1.yml".to_string(), "n3.yml".to_string()])
    );
    Ok(())
}
