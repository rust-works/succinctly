//! #7475: a program with a long enough chain of `E as $x |` bindings, or of definitions, aborted
//! the process with a native stack overflow (exit 134). jq 1.7.1 refuses both with a compile error,
//! `jq: error: memory exhausted at <top-level>, line 1:`, the program, and `jq: 1 compile error`,
//! exit 3, and so does succinctly now, past the point its native-stack floor (ADR-0025) allows: a
//! binding's body checks the floor, and a module-level chain of definitions, which the parser reads
//! in a loop, is charged a fixed share of the room per definition. (A chain of definitions nested in
//! an expression already stops at the expression nesting limit, 256.) jq refuses much sooner (about 2,500 bindings, and between 5,000 and 10,000
//! definitions: its parser's own stack), a limit succinctly does not adopt; jq 1.8.2 refuses the
//! same programs, adding a column and a caret to the message. Captured from jq 1.7.1.

use std::process::Command;

use anyhow::Result;

use crate::{run_jq_full, spawn_with_signal_retry, succinctly_bin};

/// `succinctly jq -n -f <file>` over `program`, written to a file (the programs are megabytes).
fn run_program_file(mode: &str, program: &str) -> Result<(String, String, i32)> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("program.jq");
    std::fs::write(&path, program)?;
    let (output, code) = spawn_with_signal_retry(
        || {
            let mut command = Command::new(succinctly_bin());
            command.arg(mode).arg("-n").arg("--from-file").arg(&path);
            command
        },
        None,
    )?;
    Ok((
        String::from_utf8(output.stdout)?,
        String::from_utf8(output.stderr)?,
        code,
    ))
}

/// jq's refusal: the message on line 1, the program echoed (succinctly pads it differently, a
/// recorded limit of the report), and the trailer; exit 3.
fn assert_memory_exhausted(program: &str, (stdout, stderr, code): (String, String, i32)) {
    let head = &program[..program.len().min(60)];
    assert_eq!((stdout.as_str(), code), ("", 3), "{head}: {stderr:.300}");
    let lines: Vec<&str> = stderr.lines().collect();
    assert_eq!(lines.len(), 3, "{head}: {stderr:.300}");
    assert_eq!(
        lines[0],
        "jq: error: memory exhausted at <top-level>, line 1:"
    );
    assert_eq!(lines[1].trim_end(), program.trim_end(), "{head}");
    assert_eq!(lines[2], "jq: 1 compile error");
}

#[test]
fn test_a_long_binding_chain_is_a_compile_error_not_an_abort_7475() -> Result<()> {
    // The issue's 100,000 bindings (1 MB), a distinct variable per link, and a chain under `def`.
    for program in [
        format!("{}$x", "1 as $x | ".repeat(100_000)),
        format!(
            "{}$x0",
            (0..100_000)
                .map(|i| format!("1 as $x{i}|"))
                .collect::<String>()
        ),
        format!("def f: {}$x; f", ". as $x | ".repeat(100_000)),
    ] {
        let got = run_program_file("jq", &program)?;
        assert_memory_exhausted(&program, got);
    }
    // A chain well inside the floor still runs, as in jq.
    let program = format!("{}$x", "1 as $x | ".repeat(500));
    let (stdout, stderr, code) = run_jq_full(&["-n", &program], None)?;
    assert_eq!((stdout.as_str(), code), ("1\n", 0), "{stderr}");
    Ok(())
}

#[test]
fn test_a_long_definition_chain_is_a_compile_error_not_an_abort_7475() -> Result<()> {
    let chain = |n: usize| -> String {
        (0..n)
            .map(|i| match i {
                0 => "def f0: 1;".to_string(),
                _ => format!("def f{i}: f{}+1;", i - 1),
            })
            .collect()
    };
    // The issue's 500,000 chained definitions (11 MB), read by the module-level loop, and a chain
    // of definitions that are never called.
    for program in [
        format!("{}f499999", chain(500_000)),
        format!(
            "{}7",
            (0..200_000)
                .map(|i| format!("def f{i}: 1;"))
                .collect::<String>()
        ),
    ] {
        let got = run_program_file("jq", &program)?;
        assert_memory_exhausted(&program, got);
    }
    // 5,000 definitions run, as in jq (which refuses from somewhere below 10,000).
    let program = format!(
        "{}7",
        (0..5_000)
            .map(|i| format!("def f{i}: 1;"))
            .collect::<String>()
    );
    let (stdout, stderr, code) = run_program_file("jq", &program)?;
    assert_eq!((stdout.as_str(), code), ("7\n", 0), "{stderr}");
    Ok(())
}
