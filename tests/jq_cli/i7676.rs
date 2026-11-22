//! #7676: since #7475 the parser refuses a program at the native-stack floor (ADR-0025), and two
//! parses of the crate's own text can happen there. A program's `_nwise(n)` is expanded from jq's
//! template, which binds `$n` with `as`, inside the program's own parse: a binding chain long
//! enough to bring the parse to the floor, ending in `_nwise`, passed its own last check and then
//! had the template refused, a panic (exit 101, `jq's _nwise template parses`) where the program
//! should have run or been refused as too long. The template is now parsed with the floor lifted,
//! so a chain either runs or is refused with jq's `memory exhausted` (a compile error, exit 3), and
//! nothing in between. (The path-mode prelude's first use, the other such parse, is pinned by unit
//! tests at the floor: from the CLI it needs the first use to land within 2 KiB of the floor in a
//! release build, which no chain shape here reaches deterministically.)
//!
//! jq 1.7.1 answers `def g: 1 as $x | [[1,2,3] | _nwise(2)]; 7` with `7` and refuses a chain of
//! 3,000 bindings with `memory exhausted`, exit 3 (its own parser's stack; succinctly admits far
//! longer chains, #7475's recorded limit). jq 1.8.2 has no `_nwise/1` (`not defined`), a builtin
//! it dropped, so 1.7.1 stays the reference.

use std::process::Command;

use anyhow::Result;

use crate::{spawn_with_signal_retry, succinctly_bin};

/// What one chain length did: ran (`7`), or was refused as too long for the floor.
#[derive(Debug, PartialEq)]
enum Outcome {
    Ran,
    Refused,
}

/// `succinctly jq -n -f <file>` over `links` bindings ending in `_nwise`, under a `def` that is
/// never called, so an admitted chain costs its parse and nothing more.
fn run_chain(links: usize) -> Result<Outcome> {
    let program = format!(
        "def g: {}[[1,2,3] | _nwise(2)]; 7",
        "1 as $x | ".repeat(links)
    );
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("program.jq");
    std::fs::write(&path, &program)?;
    let (output, code) = spawn_with_signal_retry(
        || {
            let mut command = Command::new(succinctly_bin());
            command.arg("jq").arg("-n").arg("--from-file").arg(&path);
            command
        },
        None,
    )?;
    let stdout = String::from_utf8(output.stdout)?;
    let stderr = String::from_utf8(output.stderr)?;
    match (code, stdout.as_str(), stderr.lines().next()) {
        (0, "7\n", None) => Ok(Outcome::Ran),
        (3, "", Some("jq: error: memory exhausted at <top-level>, line 1:")) => {
            Ok(Outcome::Refused)
        }
        _ => anyhow::bail!("{links} links: exit {code}, stdout {stdout:?}, stderr {stderr:.300}"),
    }
}

#[test]
fn test_a_binding_chain_ending_in_nwise_runs_or_is_refused_at_the_floor_7676() -> Result<()> {
    // Far past every build's floor (#7475's `LINKS`), and well inside it.
    let (mut ran, mut refused) = (1, 1_000_000);
    assert_eq!(run_chain(ran)?, Outcome::Ran);
    assert_eq!(run_chain(refused)?, Outcome::Refused);
    // The longest chain that runs. Every probe either runs or is refused.
    while refused - ran > 1 {
        let mid = ran + (refused - ran) / 2;
        match run_chain(mid)? {
            Outcome::Ran => ran = mid,
            Outcome::Refused => refused = mid,
        }
    }
    // Just past it the program's own last binding is at the floor, or the template is: before the
    // fix the template's refusal panicked over several lengths there (23,359 to 23,361 at least in
    // a release build on an Apple M5 Max). Each is now the program's refusal.
    for links in refused..refused + 4 {
        assert_eq!(run_chain(links)?, Outcome::Refused, "{links} links");
    }
    Ok(())
}
