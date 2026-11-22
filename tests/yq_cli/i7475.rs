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
    // 2,000,000 (20 MB): a yq-mode binding spends less stack than a jq-mode one, and the floor
    // falls with the build's frame sizes. An unoptimised build refuses 400,000, but an
    // `opt-level=1` build admits that many (it refuses 1,600,000, measured on aarch64), and an
    // admitted chain costs time quadratic in its length (yq mode rewrites the rest of the chain at
    // every link): over a minute at 100,000. A refusal costs the same at any length (the parser
    // stops at the floor: about 1.1 GB of stack, under a second).
    std::fs::write(&path, format!("{}$x", "1 as $x | ".repeat(2_000_000)))?;
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
