//! #7616: yq has no `$ENV` or `$ARGS`. Each is a variable name like any other, unbound unless the
//! program binds it, so a filter that reads it yields nothing (`[$ENV] | length` is `0`, as for
//! `$foo`). `succinctly yq` follows that without `--jq-extensions`; under the flag they keep jq's
//! environment and arguments objects, and `succinctly jq` is unchanged. Every plain row was captured
//! from yq v4.53.3 with `I7616_FOO=bar` in the environment.

use anyhow::Result;
use std::io::Write;
use std::process::{Command, Stdio};

/// Runs `succinctly <args>` with `I7616_FOO=bar` set and `input` on stdin: (stdout, stderr, code).
fn run(args: &[&str], input: &str) -> Result<(String, String, i32)> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .args(args)
        .env("I7616_FOO", "bar")
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

/// (filter, yq's stdout) over the document `a: 1`, printed with `-o=json -I=0`.
#[rustfmt::skip]
const PLAIN_ROWS: &[(&str, &str)] = &[
    ("$ENV.I7616_FOO", ""),
    ("$ENV | keys | length", ""),
    ("[$ENV] | length", "0\n"),
    ("$ENV | type", ""),
    ("[$ENV, 1]", "[1]\n"),
    ("$ENV // \"d\"", "\"d\"\n"),
    ("$ARGS", ""),
    ("$ARGS.named", ""),
    ("[$ARGS] | length", "0\n"),
    ("[$ENV.I7616_FOO, $ARGS.named]", "[]\n"),
    ("1 as $ENV | $ENV", "1\n"),
    ("1 as $ENV | (1, 2, 3) | $ENV", "1\n"),
    ("1 as $ARGS | (1, 2, 3) | $ARGS", "1\n"),
    ("env(I7616_FOO)", "\"bar\"\n"),
    ("eval(\"$ENV.I7616_FOO\")", ""),
    ("[eval(\"$ENV\")] | length", "0\n"),
    (".a = $ENV.I7616_FOO", "{\"a\":1}\n"),
    (".b = [$ENV]", "{\"a\":1,\"b\":[]}\n"),
    (".a |= $ENV", "{\"a\":1}\n"),
    ("[.[] | $ENV] | length", "0\n"),
    ("with_entries(.value = $ENV)", "{\"a\":1}\n"),
];

#[test]
fn test_env_and_args_are_unbound_names_without_jq_extensions_7616() -> Result<()> {
    for &(filter, expected) in PLAIN_ROWS {
        assert_eq!(
            run(&["yq", "-o=json", "-I=0", filter], "a: 1\n")?,
            (expected.to_string(), String::new(), 0),
            "`{filter}`"
        );
    }
    // `-n` and `eval-all` read them the same way.
    for args in [
        &["yq", "-n", "-o=json", "-I=0"][..],
        &["yq", "ea", "-o=json", "-I=0"][..],
    ] {
        let mut args = args.to_vec();
        args.push("[$ENV, $ARGS] | length");
        assert_eq!(
            run(&args, "a: 1\n---\nb: 2\n")?,
            ("0\n".to_string(), String::new(), 0),
            "{args:?}"
        );
    }
    Ok(())
}

/// `--security-disable-env-ops` has nothing to refuse for an unbound `$ENV` (yq prints nothing and
/// exits 0); it still refuses `env(NAME)`, and `$ENV` under `--jq-extensions`.
#[test]
fn test_env_flag_and_unbound_env_7616() -> Result<()> {
    let sec = ["yq", "--security-disable-env-ops", "-o=json", "-I=0"];
    for (filter, expected) in [("$ENV.I7616_FOO", ""), ("[$ENV] | length", "0\n")] {
        let mut args = sec.to_vec();
        args.push(filter);
        assert_eq!(
            run(&args, "a: 1\n")?,
            (expected.to_string(), String::new(), 0),
            "`{filter}`"
        );
    }
    for args in [
        &["yq", "--security-disable-env-ops", "env(I7616_FOO)"][..],
        &[
            "yq",
            "--security-disable-env-ops",
            "--jq-extensions",
            "$ENV.I7616_FOO",
        ][..],
    ] {
        let (stdout, stderr, code) = run(args, "a: 1\n")?;
        assert_eq!((stdout.as_str(), code), ("", 1), "{args:?}");
        assert!(
            stderr.contains("env operations have been disabled"),
            "{args:?}: {stderr:?}"
        );
    }
    Ok(())
}

/// Under `--jq-extensions` they are jq's objects, as before; `succinctly jq` always has them.
#[test]
fn test_env_and_args_under_jq_extensions_and_in_jq_7616() -> Result<()> {
    for (args, expected) in [
        (
            &["yq", "--jq-extensions", "-o=json", "-I=0", "$ENV.I7616_FOO"][..],
            "\"bar\"\n",
        ),
        (
            &["yq", "--jq-extensions", "-o=json", "-I=0", "$ENV | type"][..],
            "\"!!map\"\n",
        ),
        (
            &["yq", "--jq-extensions", "-o=json", "-I=0", "$ARGS"][..],
            "{\"named\":{},\"positional\":[]}\n",
        ),
        (
            &[
                "yq",
                "--jq-extensions",
                "-o=json",
                "-I=0",
                "1 as $ENV | $ENV",
            ][..],
            "1\n",
        ),
        // The flag reaches an `eval(..)` text too (#7012); without it the row is in `PLAIN_ROWS`.
        (
            &[
                "yq",
                "--jq-extensions",
                "-o=json",
                "-I=0",
                "eval(\"$ENV.I7616_FOO\")",
            ][..],
            "\"bar\"\n",
        ),
        (&["jq", "-c", "$ENV.I7616_FOO"][..], "\"bar\"\n"),
        (&["jq", "-c", "[$ENV] | length"][..], "1\n"),
        (
            &["jq", "-c", "$ARGS"][..],
            "{\"positional\":[],\"named\":{}}\n",
        ),
    ] {
        assert_eq!(
            run(args, "{\"a\":1}\n")?,
            (expected.to_string(), String::new(), 0),
            "{args:?}"
        );
    }
    Ok(())
}
