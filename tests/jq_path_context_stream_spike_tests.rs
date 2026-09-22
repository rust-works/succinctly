//! Focused control-flow probes for #3022's path-context streaming spike.
//!
//! Each expectation was checked before the streaming change against the
//! installed jq 1.8.2 or yq v4.53.3 oracle, as indicated per test.  The yq
//! rows use only operators accepted by yq itself; this deliberately avoids
//! treating succinctly's jq-extension surface as an oracle.

#![cfg(feature = "cli")]

use std::io::Write;
use std::process::{Command, Stdio};

use anyhow::Result;

fn run(mode: &str, filter: &str, input: &str) -> Result<(String, String, i32)> {
    let mut command = Command::new(env!("CARGO_BIN_EXE_succinctly"));
    command.arg(mode);
    if mode == "jq" {
        command.arg("-c");
    } else {
        command.args(["-o=json", "-I=0"]);
    }
    let mut child = command
        .arg(filter)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    child
        .stdin
        .take()
        .expect("stdin piped")
        .write_all(input.as_bytes())?;
    let output = child.wait_with_output()?;
    let code = output.status.code().expect("child was not signal-killed");
    Ok((
        String::from_utf8(output.stdout)?,
        String::from_utf8(output.stderr)?,
        code,
    ))
}

/// A `try` handles an error raised in its body, but a later pipeline stage is
/// outside that body.  A push-based implementation must therefore park the
/// downstream error rather than feed it into the earlier `try` callback.
///
/// Captured from jq 1.8.2. `path(.)` makes both rows enter the jq
/// path-context route without relying on yq-only builtins.
#[test]
fn upstream_try_does_not_catch_a_later_path_context_error() -> Result<()> {
    let (stdout, stderr, code) = run(
        "jq",
        "(try error(\"upstream\") catch \"caught\") | path(.)",
        "null\n",
    )?;
    assert_eq!(code, 0, "stderr={stderr:?}");
    assert_eq!(stdout, "[]\n");

    let (stdout, stderr, code) = run(
        "jq",
        "(try .a catch \"caught\") | .[] | path(.)",
        "{\"a\":1}\n",
    )?;
    assert_eq!(code, 5, "stdout={stdout:?} stderr={stderr:?}");
    assert_eq!(stdout, "");
    assert!(
        stderr.contains("Cannot iterate over number"),
        "stderr={stderr:?}"
    );
    Ok(())
}

/// `first` supplies Stop after the first result, so the generator's later
/// `error` must never be evaluated. Captured from jq 1.8.2.
#[test]
fn first_stops_a_path_context_generator_before_its_later_error() -> Result<()> {
    let (stdout, stderr, code) = run(
        "jq",
        "first(.a[] | path(.), error(\"late\"))",
        "{\"a\":[1,2]}\n",
    )?;
    assert_eq!(code, 0, "stderr={stderr:?}");
    assert_eq!(stdout, "[]\n");
    assert_eq!(stderr, "");
    Ok(())
}

/// A normal jq consumer exposes the prefix already yielded before an error.
/// This is the companion to the Stop row above: streaming must preserve this
/// prefix, while `first` is allowed to prevent it from continuing. Captured
/// from jq 1.8.2.
#[test]
fn jq_path_context_generator_keeps_its_prefix_before_error() -> Result<()> {
    let (stdout, stderr, code) = run("jq", "(.a[] | path(.)), error(\"late\")", "{\"a\":[1,2]}\n")?;
    assert_eq!(code, 5, "stdout={stdout:?} stderr={stderr:?}");
    assert_eq!(stdout, "[]\n[]\n");
    assert!(stderr.contains("late"), "stderr={stderr:?}");
    Ok(())
}

/// Nested navigational fields retain the position needed by all three yq
/// path-context builtins. Captured from yq v4.53.3.
#[test]
fn yq_nested_fields_keep_path_context() -> Result<()> {
    for (filter, expected) in [
        (".a | .b | key", "\"b\"\n"),
        (".a | .b | path", "[\"a\",\"b\"]\n"),
        (".a | .b | parent", "{\"b\":1}\n"),
    ] {
        let (stdout, stderr, code) = run("yq", filter, "a:\n  b: 1\n")?;
        assert_eq!(code, 0, "{filter}: stderr={stderr:?}");
        assert_eq!(stdout, expected, "{filter}");
    }
    Ok(())
}

/// The issue's fan-out shape walks a nested field pipe and then hops once
/// per element. Its answer must stay unchanged as each stage loses its
/// temporary position buffer.
#[test]
fn yq_field_chain_then_parent_after_fanout() -> Result<()> {
    let (stdout, stderr, code) = run(
        "yq",
        "[.[] | .k.x | parent] | length",
        "- k: {x: 1}\n- k: {x: 2}\n",
    )?;
    assert_eq!(code, 0, "stderr={stderr:?}");
    assert_eq!(stdout, "2\n");
    Ok(())
}

/// A comma stage keeps both branch order and each branch's path context.
/// Captured from yq v4.53.3.
#[test]
fn yq_comma_stage_streams_positions_in_order() -> Result<()> {
    let (stdout, stderr, code) = run("yq", "(.a, .b) | key", "a: 1\nb: 2\n")?;
    assert_eq!(code, 0, "stderr={stderr:?}");
    assert_eq!(stdout, "\"a\"\n\"b\"\n");
    Ok(())
}

/// yq discards a computed-index prefix when a later component errors. The
/// trailing `path` ensures the row traverses path-context machinery as well
/// as the computed-component rollback boundary. Captured from yq v4.53.3.
#[test]
fn yq_computed_component_error_rolls_back_path_context_prefix() -> Result<()> {
    let (stdout, stderr, code) = run(
        "yq",
        ".arr | .[(0, error(\"late\"))] | path",
        "arr:\n  - 1\n  - 2\n",
    )?;
    assert_eq!(code, 1, "stdout={stdout:?} stderr={stderr:?}");
    assert_eq!(stdout, "");
    assert!(stderr.contains("late"), "stderr={stderr:?}");
    Ok(())
}
