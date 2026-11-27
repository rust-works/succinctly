//! #8008: yq's `env(NAME)`, `strenv(NAME)` and `now` build a node of their own whatever their input,
//! as a literal does, so in `del()` they are parentless targets: `deleteChildOperator` walks its
//! targets back to front and returns at the first one with no parent (#1865). `del(env(X))` and
//! `del(.[] | env(X))` delete nothing, `del(env(X), .a)` deletes `.a` and `del(.a, env(X))` does
//! not, where `succinctly yq` raised `Invalid path expression with result "hi"`. A write through an
//! env reader (`env(X) = 1`) already found nothing to write, and `succinctly jq` still raises jq's
//! error for `del(now)`.
//!
//! Every yq row is captured from yq v4.53.3 with `I8008_X=hi` in the environment.

use std::io::Write;
use std::process::{Command, Stdio};

use anyhow::Result;

const SEQ: &str = "- 1\n- 2\n- 3\n";
const MAP: &str = "a: 1\nb: hi\nc: [1, 2]\n";
const TWO: &str = "a: 1\nb: hi\n---\na: 2\nb: hi\n";

/// Runs `succinctly <args>` with `I8008_X=hi` set and `input` on stdin: (stdout, stderr, code).
fn run(args: &[&str], input: &str) -> Result<(String, String, i32)> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .args(args)
        .env("I8008_X", "hi")
        .env_remove("I8008_NOPE")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    // A small input; a write error (the child exited early) shows in the status below.
    let _ = child
        .stdin
        .take()
        .expect("piped stdin")
        .write_all(input.as_bytes());
    let output = child.wait_with_output()?;
    Ok((
        String::from_utf8(output.stdout)?,
        String::from_utf8(output.stderr)?,
        output.status.code().unwrap_or(-1),
    ))
}

/// (filter, yq's stdout) over [`SEQ`].
#[rustfmt::skip]
const SEQ_ROWS: &[(&str, &str)] = &[
    ("del(env(I8008_X))", "- 1\n- 2\n- 3\n"),
    ("del(.[] | env(I8008_X))", "- 1\n- 2\n- 3\n"),
    ("del(strenv(I8008_X))", "- 1\n- 2\n- 3\n"),
    ("del(.[] | strenv(I8008_X))", "- 1\n- 2\n- 3\n"),
    ("del(now)", "- 1\n- 2\n- 3\n"),
    ("del(.[] | now)", "- 1\n- 2\n- 3\n"),
    ("del(.[0] | env(I8008_X))", "- 1\n- 2\n- 3\n"),
    ("del((env(I8008_X)))", "- 1\n- 2\n- 3\n"),
    ("map(del(env(I8008_X)))", "- 1\n- 2\n- 3\n"),
    ("del(.[] | select(. == 2) | env(I8008_X))", "- 1\n- 2\n- 3\n"),
];

/// (filter, yq's stdout) over [`MAP`]: the back-to-front order with a tracked target, a condition
/// that reads an env reader (a real path, still deleted), and writes through one.
#[rustfmt::skip]
const MAP_ROWS: &[(&str, &str)] = &[
    ("del(.a, env(I8008_X))", "a: 1\nb: hi\nc: [1, 2]\n"),
    ("del(env(I8008_X), .a)", "b: hi\nc: [1, 2]\n"),
    ("del(.a, strenv(I8008_X))", "a: 1\nb: hi\nc: [1, 2]\n"),
    ("del(strenv(I8008_X), .a)", "b: hi\nc: [1, 2]\n"),
    ("del(.a, now)", "a: 1\nb: hi\nc: [1, 2]\n"),
    ("del(now, .a)", "b: hi\nc: [1, 2]\n"),
    ("del(.a, env(I8008_X), .b)", "a: 1\nc: [1, 2]\n"),
    ("del(.a, .b, env(I8008_X))", "a: 1\nb: hi\nc: [1, 2]\n"),
    ("del(.[] | (., env(I8008_X)))", "a: 1\nb: hi\nc: [1, 2]\n"),
    ("del(.[] | select(. == env(I8008_X)))", "a: 1\nc: [1, 2]\n"),
    ("del(.[] | select(. == strenv(I8008_X)))", "a: 1\nc: [1, 2]\n"),
    ("del(.a | env(I8008_X))", "a: 1\nb: hi\nc: [1, 2]\n"),
    ("del(.a, (.b | strenv(I8008_X)))", "a: 1\nb: hi\nc: [1, 2]\n"),
    ("del(.a, (.c[] | env(I8008_X)))", "a: 1\nb: hi\nc: [1, 2]\n"),
    ("del(.c[], env(I8008_X))", "a: 1\nb: hi\nc: [1, 2]\n"),
    ("del(env(I8008_X), .c[])", "a: 1\nb: hi\nc: []\n"),
    ("del(env(I8008_X), .)", ""),
    ("del(., env(I8008_X))", "a: 1\nb: hi\nc: [1, 2]\n"),
    ("del(.c[] | now)", "a: 1\nb: hi\nc: [1, 2]\n"),
    ("del(env(I8008_X)) | .a", "1\n"),
    (".c |= del(env(I8008_X), .[0])", "a: 1\nb: hi\nc: [2]\n"),
    (".c |= del(.[] | env(I8008_X))", "a: 1\nb: hi\nc: [1, 2]\n"),
    ("env(I8008_X) = 1", "a: 1\nb: hi\nc: [1, 2]\n"),
    ("env(I8008_X) |= 1", "a: 1\nb: hi\nc: [1, 2]\n"),
    ("strenv(I8008_X) = 1", "a: 1\nb: hi\nc: [1, 2]\n"),
    ("now = 1", "a: 1\nb: hi\nc: [1, 2]\n"),
    ("env(I8008_X) += 1", "a: 1\nb: hi\nc: [1, 2]\n"),
    ("(.a, env(I8008_X)) = 5", "a: 5\nb: hi\nc: [1, 2]\n"),
    ("(env(I8008_X), .a) = 5", "a: 5\nb: hi\nc: [1, 2]\n"),
    (".a | env(I8008_X) = 1", "1\n"),
];

#[test]
fn test_8008_a_an_env_reader_is_a_parentless_del_target() -> Result<()> {
    for (doc, rows) in [(SEQ, SEQ_ROWS), (MAP, MAP_ROWS)] {
        for (filter, want) in rows {
            let (out, err, code) = run(&["yq", filter], doc)?;
            assert_eq!(
                (out.as_str(), err.as_str(), code),
                (*want, "", 0),
                "{filter}"
            );
        }
    }
    Ok(())
}

/// (arguments after `yq`, stdin, yq's stdout): JSON output, `-n`, `ea` and a multi-document stream.
#[rustfmt::skip]
const ROUTE_ROWS: &[(&[&str], &str, &str)] = &[
    (&["-o=json", "-I=0", "del(.a, env(I8008_X))"], MAP, "{\"a\":1,\"b\":\"hi\",\"c\":[1,2]}\n"),
    (&["-o=json", "-I=0", "del(env(I8008_X), .a)"], MAP, "{\"b\":\"hi\",\"c\":[1,2]}\n"),
    (&["-o=json", "-I=0", "del(.[] | env(I8008_X))"], SEQ, "[1,2,3]\n"),
    (&["-n", "del(env(I8008_X))"], "", "\n"),
    (&["-n", "del(now)"], "", "\n"),
    (&["-n", "{\"a\": 1} | del(env(I8008_X), .a)"], "", "{}\n"),
    (&["-n", "{\"a\": 1} | del(.a, env(I8008_X))"], "", "a: 1\n"),
    (&["-n", "[1, 2] | del(.[] | strenv(I8008_X))"], "", "- 1\n- 2\n"),
    (&["ea", "del(env(I8008_X))"], TWO, "a: 1\nb: hi\n---\na: 2\nb: hi\n"),
    (&["ea", "del(.a, env(I8008_X))"], TWO, "a: 1\nb: hi\n---\na: 2\nb: hi\n"),
    (&["ea", "del(env(I8008_X), .a)"], TWO, "b: hi\n---\nb: hi\n"),
    (&["ea", "del(.[] | strenv(I8008_X))"], TWO, "a: 1\nb: hi\n---\na: 2\nb: hi\n"),
    (&["ea", "select(di == 1) | del(env(I8008_X), .a)"], TWO, "b: hi\n"),
    (&["del(env(I8008_X), .a)"], TWO, "b: hi\n---\nb: hi\n"),
    (&["del(.a, env(I8008_X))"], TWO, "a: 1\nb: hi\n---\na: 2\nb: hi\n"),
];

#[test]
fn test_8008_b_every_route_orders_an_env_reader_like_a_literal() -> Result<()> {
    for (args, doc, want) in ROUTE_ROWS {
        let mut full = vec!["yq"];
        full.extend_from_slice(args);
        let (out, err, code) = run(&full, doc)?;
        assert_eq!(
            (out.as_str(), err.as_str(), code),
            (*want, "", 0),
            "{args:?}"
        );
    }
    Ok(())
}

/// (arguments after `yq` and before `-i`, the file, the file as yq leaves it): `-i` prints nothing.
#[rustfmt::skip]
const INPLACE_ROWS: &[(&[&str], &str, &str)] = &[
    (&["del(env(I8008_X))"], MAP, "a: 1\nb: hi\nc: [1, 2]\n"),
    (&["del(.a, env(I8008_X))"], MAP, "a: 1\nb: hi\nc: [1, 2]\n"),
    (&["del(env(I8008_X), .a)"], MAP, "b: hi\nc: [1, 2]\n"),
    (&["del(.[] | strenv(I8008_X))"], SEQ, "- 1\n- 2\n- 3\n"),
    (&["ea", "del(env(I8008_X), .a)"], TWO, "b: hi\n---\nb: hi\n"),
];

#[test]
fn test_8008_c_in_place_keeps_what_yq_keeps() -> Result<()> {
    let dir = tempfile::TempDir::new()?;
    for (args, doc, want) in INPLACE_ROWS {
        let path = dir.path().join("doc.yaml");
        std::fs::write(&path, doc)?;
        let path_arg = path.to_str().expect("utf-8 temp path");
        let mut full = vec!["yq"];
        match args {
            ["ea", rest @ ..] => {
                full.extend(["ea", "-i"]);
                full.extend_from_slice(rest);
            }
            _ => {
                full.push("-i");
                full.extend_from_slice(args);
            }
        }
        full.push(path_arg);
        let (out, err, code) = run(&full, "")?;
        assert_eq!((out.as_str(), err.as_str(), code), ("", "", 0), "{args:?}");
        assert_eq!(std::fs::read_to_string(&path)?, *want, "{args:?}");
    }
    Ok(())
}

/// An unset variable is still yq's error, in front of the order rule.
#[test]
fn test_8008_d_an_unset_variable_still_errors() -> Result<()> {
    for filter in ["del(env(I8008_NOPE))", "del(.a, env(I8008_NOPE))"] {
        let (out, err, code) = run(&["yq", filter], MAP)?;
        assert_eq!(
            (out.as_str(), err.as_str(), code),
            (
                "",
                "Error: value for env variable 'I8008_NOPE' not provided in env()\n",
                1
            ),
            "{filter}"
        );
    }
    Ok(())
}

/// `succinctly jq` keeps jq 1.7.1's refusal (exit 5): `now` has no path there.
#[test]
fn test_8008_e_jq_mode_still_refuses_del_now() -> Result<()> {
    for filter in [
        "del(now)",
        "[1,2] | del(.[] | now)",
        "{\"a\":1} | del(now, .a)",
        "{\"a\":1} | del(.a, now)",
    ] {
        let (out, err, code) = run(&["jq", "-nc", filter], "")?;
        assert_eq!((out.as_str(), code), ("", 5), "{filter}");
        assert!(
            err.starts_with("jq: error (at <unknown>): Invalid path expression with result "),
            "{filter}: {err}"
        );
    }
    Ok(())
}
