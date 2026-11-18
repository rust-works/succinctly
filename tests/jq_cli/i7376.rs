//! #7376: a debug build of `succinctly jq` reads the size of its windows from
//! `SUCCINCTLY_DEBUG_JQ_WINDOW_BYTES`, so the windowed-route tests can run over fixtures of tens of
//! KiB (`small_window`). The variable is a test knob and not part of the tool: a release build does
//! not have it, which these rows pin from both sides, and a debug build refuses a value it cannot
//! use instead of running the test on the 1 MiB window, where a small fixture is read whole.

use std::process::Command;

use anyhow::Result;

use crate::small_window;

fn run(value: Option<&str>, file: &std::path::Path) -> Result<(String, String, i32)> {
    let mut command = Command::new(env!("CARGO_BIN_EXE_succinctly"));
    command
        .args(["jq", "-c", ".id"])
        .arg(file)
        .env("LC_ALL", "C")
        .env_remove(small_window::ENV);
    if let Some(value) = value {
        command.env(small_window::ENV, value);
    }
    let output = command.output()?;
    Ok((
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code().unwrap_or(-1),
    ))
}

/// The variable's name is in a debug binary and not in a release one.
#[test]
fn the_window_knob_is_compiled_out_of_a_release_build_7376() -> Result<()> {
    let binary = std::fs::read(env!("CARGO_BIN_EXE_succinctly"))?;
    let name = small_window::ENV.as_bytes();
    let found = binary.windows(name.len()).any(|bytes| bytes == name);
    assert_eq!(
        found,
        cfg!(debug_assertions),
        "{} in the binary",
        small_window::ENV
    );
    Ok(())
}

/// A value the debug build cannot use is refused at exit 2, whatever the file's size; a release
/// build never reads the variable, so the same run answers.
#[test]
fn a_window_that_is_not_usable_is_refused_by_a_debug_build_only_7376() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let file = dir.path().join("small.json");
    std::fs::write(&file, "{\"id\":1}\n{\"id\":2}\n")?;
    let answer = ("1\n2\n".to_string(), String::new(), 0);
    assert_eq!(run(None, &file)?, answer);
    for value in ["", "0", "4096", "1048577", "64k", "-1", " 65536"] {
        let got = run(Some(value), &file)?;
        if cfg!(debug_assertions) {
            assert_eq!(
                got,
                (
                    String::new(),
                    format!(
                        "succinctly: {}={value} is not a count of bytes from 8192 to 1048576 \
                         (a debug-build test knob, #7376)\n",
                        small_window::ENV
                    ),
                    2
                ),
                "{value:?}"
            );
        } else {
            assert_eq!(got, answer, "{value:?}");
        }
    }
    // Every window the knob takes gives the answer the 1 MiB window gives.
    for value in ["8192", "65536", "1048576"] {
        assert_eq!(run(Some(value), &file)?, answer, "{value:?}");
    }
    Ok(())
}

/// A file of four small windows is read a window at a time by a debug build told so, and gives
/// what the same file gives read whole (no variable: it is far under 4 MiB), a runtime error's
/// line and a fault's wording included.
#[test]
fn a_file_of_four_small_windows_reads_as_it_does_whole_7376() -> Result<()> {
    let lines = small_window::scaled(64_000).min(8_000);
    let mut text: String = (0..lines)
        .map(|i| {
            format!(
                "{{\"id\":{i},\"name\":\"user {i}\",\"pad\":\"{}\"}}\n",
                "x".repeat(40)
            )
        })
        .collect();
    text.push_str("{\"id\": [1 2]}\n");
    assert!(text.len() >= 4 * (64 << 10) && text.len() < 4 << 20);
    let dir = tempfile::tempdir()?;
    let file = dir.path().join("f.json");
    std::fs::write(&file, &text)?;
    let whole = run(None, &file)?;
    assert_eq!(whole.0.lines().count(), lines);
    assert!(
        whole.1.contains(&format!("at line {}", lines + 1)),
        "{}",
        whole.1
    );
    assert_eq!(run(Some("65536"), &file)?, whole);
    assert_eq!(run(Some("8192"), &file)?, whole);
    Ok(())
}
