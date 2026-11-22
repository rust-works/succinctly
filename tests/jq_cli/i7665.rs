//! #7665: a `~/.jq` file the user cannot read is left out, as an absent one is, where it was a
//! compile error for every program (`error loading data file <path>: Permission denied`, exit 3).
//! jq imports `~/.jq` as an `optional` library, and its `load_library` reports a file it cannot
//! read only for an import that is not optional. A `~/.jq` that reads but does not parse is still a
//! compile error (#5351, `tests/jq_cli/i5351.rs`).
//!
//! Every row was captured from jq 1.7.1 (`/usr/bin/jq`, and the x86_64 Linux release binary for
//! the first two) with `-nc -L .` run beside `g.jq` = `def g: 2;` and `HOME` holding an unreadable
//! `.jq` = `def hb: 1;`. jq 1.8.2 agrees on every row but `hb`, where it echoes the source with its
//! caret (the reworded echo 1.7.1 stays the reference for).

use anyhow::Result;

/// (program, stdout, stderr, exit status), from jq 1.7.1.
const ROWS: &[(&str, &str, &str, i32)] = &[
    ("1", "1\n", "", 0),
    (
        "hb",
        "",
        "jq: error: hb/0 is not defined at <top-level>, line 1:\nhb\njq: 1 compile error\n",
        3,
    ),
    (
        "include \"zz\"; 1",
        "",
        "jq: error: module not found: zz\n\njq: 1 compile error\n",
        3,
    ),
    ("include \"g\"; g", "2\n", "", 0),
    ("import \"g\" as m; [m::g, 3]", "[2,3]\n", "", 0),
    ("\"g\" | modulemeta | .defs", "[\"g/0\"]\n", "", 0),
    ("def hb: 5; hb", "5\n", "", 0),
];

/// Every row, and the same answer as with no `~/.jq` at all. Skipped where `chmod 000` still
/// reads (root).
#[cfg(unix)]
#[test]
fn an_unreadable_dot_jq_is_left_out_as_jq_leaves_it_out_7665() -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir()?;
    std::fs::write(dir.path().join("g.jq"), "def g: 2;\n")?;
    let home = tempfile::tempdir()?;
    let empty_home = tempfile::tempdir()?;
    let dot_jq = home.path().join(".jq");
    std::fs::write(&dot_jq, "def hb: 1;\n")?;
    std::fs::set_permissions(&dot_jq, std::fs::Permissions::from_mode(0o000))?;
    if std::fs::read(&dot_jq).is_ok() {
        eprintln!("skipped: chmod 000 does not bind this user");
        return Ok(());
    }
    let run = |home: &std::path::Path, program: &str| -> Result<(String, String, Option<i32>)> {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_succinctly"))
            .args(["jq", "-nc", "-L", ".", program])
            .current_dir(dir.path())
            .env("HOME", home)
            .env("LC_ALL", "C")
            .env_remove("JQ_LIBRARY_PATH")
            .output()?;
        Ok((
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
            output.status.code(),
        ))
    };
    for &(program, stdout, stderr, status) in ROWS {
        let got = run(home.path(), program)?;
        assert_eq!(
            (got.0.as_str(), got.1.as_str(), got.2),
            (stdout, stderr, Some(status)),
            "{program}"
        );
        assert_eq!(got, run(empty_home.path(), program)?, "{program}");
    }
    Ok(())
}
