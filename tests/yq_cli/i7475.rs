//! #7475 in yq mode: a long chain of `E as $x |` bindings aborted the process with a native stack
//! overflow. `succinctly yq` now refuses a chain past the native-stack floor (ADR-0025) at parse
//! time, with jq's words, `memory exhausted`, where yq v4.53.3 answers (its Go stack grows on
//! demand: 100,000 bindings print `1`). That is ADR-0018 rule 4(c), recorded in
//! `docs/compliance/yq/limitations.md`: matching would take the host process down.

use std::process::Command;

use anyhow::Result;

use crate::{run_yq_stdin_with_stderr, spawn_with_signal_retry};

#[test]
fn test_a_long_binding_chain_is_refused_not_an_abort_in_yq_mode_7475() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("program.yq");
    // 400,000 (4 MB): a yq-mode binding spends less stack than a jq-mode one, and a debug build's
    // floor admits more than 100,000 of them.
    std::fs::write(&path, format!("{}$x", "1 as $x | ".repeat(400_000)))?;
    let (output, code) = spawn_with_signal_retry(
        || {
            let mut command = Command::new(env!("CARGO_BIN_EXE_succinctly"));
            command.args(["yq", "-n", "--from-file"]).arg(&path);
            command
        },
        None,
    )?;
    let stderr = String::from_utf8(output.stderr)?;
    assert_eq!(
        (output.stdout.as_slice(), code),
        (&b""[..], 1),
        "{stderr:.300}"
    );
    assert!(stderr.contains("memory exhausted"), "{stderr:.300}");
    // A chain well inside the floor still runs, as in yq.
    let (stdout, stderr, code) =
        run_yq_stdin_with_stderr(&format!("{}$x", "1 as $x | ".repeat(500)), "", &["-n"])?;
    assert_eq!((stdout.as_str(), code), ("1\n", 0), "{stderr}");
    Ok(())
}
