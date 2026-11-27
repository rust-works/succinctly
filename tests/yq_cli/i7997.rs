//! #7997: yq has no `$__loc__`. It is a variable name like any other, unbound unless the program
//! binds it, so `$__loc__` alone yields nothing and `1 as $__loc__ | $__loc__` is `1`.
//! `succinctly yq` follows that without `--jq-extensions` (it used to read every `$__loc__` as jq's
//! location, so a binding of the name could never be read); under the flag it is jq's
//! `{file, line}` object, and `succinctly jq` is unchanged. Every plain row was captured from yq
//! v4.53.3, and every jq row from jq 1.7.1.

use anyhow::Result;
use std::io::Write;
use std::process::{Command, Stdio};

/// Runs `succinctly <args>` with `input` on stdin: (stdout, stderr, code).
fn run(args: &[&str], input: &str) -> Result<(String, String, i32)> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .args(args)
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

/// (filter, yq's stdout) over the document `a: 1`, printed with `-o=json -I=0`. `.a as $__loc__ |
/// eval("$__loc__")` (`1` in yq) is left out: no variable of the enclosing program reaches `eval`'s
/// text here, whatever its name (`docs/compliance/yq/limitations.md`, `eval(expr)`).
#[rustfmt::skip]
const PLAIN_ROWS: &[(&str, &str)] = &[
    ("$__loc__", ""),
    ("[$__loc__] | length", "0\n"),
    ("$__loc__ | .file", ""),
    ("$__loc__.line", ""),
    ("[$__loc__, 1]", "[1]\n"),
    ("$__loc__ // \"d\"", "\"d\"\n"),
    ("1 as $__loc__ | $__loc__", "1\n"),
    ("{\"a\": 5} as $__loc__ | $__loc__.a", "5\n"),
    ("1 as $__loc__ | $__loc__ + 1", "2\n"),
    ("1 as $__loc__ | [$__loc__, 2]", "[1,2]\n"),
    ("1 as $__loc__ | {\"k\": $__loc__}", "{\"k\":1}\n"),
    ("1 as $__loc__ | \"\\($__loc__)\"", "\"1\"\n"),
    ("1 as $__loc__ | (1, 2, 3) | $__loc__", "1\n"),
    (".a as $__loc__ | .b = $__loc__", "{\"a\":1,\"b\":1}\n"),
    (".a as $__loc__ | $__loc__ | type", "\"!!int\"\n"),
    ("eval(\"1 as $__loc__ | $__loc__\")", "1\n"),
    ("$__loc__ as $__loc__ | 3", "3\n"),
    ("[.[] | $__loc__] | length", "0\n"),
    (".a = $__loc__", "{\"a\":1}\n"),
    ("with_entries(.value = $__loc__)", "{\"a\":1}\n"),
];

#[test]
fn test_loc_is_an_ordinary_name_without_jq_extensions_7997() -> Result<()> {
    for &(filter, expected) in PLAIN_ROWS {
        let answer = run(&["yq", "-o=json", "-I=0", filter], "a: 1\n")?;
        assert_eq!(
            answer,
            (expected.to_string(), String::new(), 0),
            "`{filter}`"
        );
        // The name is nothing special: the same filter over another name answers the same.
        let renamed = filter.replace("$__loc__", "$i7997");
        assert_eq!(
            run(&["yq", "-o=json", "-I=0", &renamed], "a: 1\n")?,
            answer,
            "`{renamed}`"
        );
    }
    // `-n` and `eval-all` read it the same way (yq: `1`, and `1` per document under `ea`).
    for (args, expected) in [
        (&["yq", "-n", "-o=json", "-I=0"][..], "1\n"),
        (&["yq", "ea", "-o=json", "-I=0"][..], "1\n1\n"),
    ] {
        let mut args = args.to_vec();
        args.push("1 as $__loc__ | $__loc__");
        assert_eq!(
            run(&args, "a: 1\n---\nb: 2\n")?,
            (expected.to_string(), String::new(), 0),
            "{args:?}"
        );
    }
    Ok(())
}

/// `--arg` is succinctly's own flag in yq mode (yq rejects it as an unknown flag); it binds
/// `$__loc__` like any other name now, where the value used to be unreadable.
#[test]
fn test_arg_binds_loc_without_jq_extensions_7997() -> Result<()> {
    assert_eq!(
        run(
            &["yq", "-n", "-o=json", "-I=0", "--arg", "__loc__", "X", "$__loc__"],
            ""
        )?,
        ("\"X\"\n".to_string(), String::new(), 0)
    );
    Ok(())
}

/// Under `--jq-extensions` `$__loc__` is jq's location object, as in `succinctly jq`, and so, as in
/// jq, it is not a name a program can bind or `--arg` can set.
#[test]
fn test_loc_is_jqs_location_under_jq_extensions_7997() -> Result<()> {
    for (filter, expected) in [
        ("$__loc__", "{\"file\":\"<top-level>\",\"line\":1}\n"),
        (".\n| $__loc__.line", "2\n"),
        ("[$__loc__] | length", "1\n"),
    ] {
        assert_eq!(
            run(
                &["yq", "--jq-extensions", "-o=json", "-I=0", filter],
                "a: 1\n"
            )?,
            (expected.to_string(), String::new(), 0),
            "`{filter}`"
        );
    }
    assert_eq!(
        run(
            &[
                "yq",
                "--jq-extensions",
                "-n",
                "-o=json",
                "-I=0",
                "--arg",
                "__loc__",
                "X",
                "$__loc__"
            ],
            ""
        )?,
        (
            "{\"file\":\"<top-level>\",\"line\":1}\n".to_string(),
            String::new(),
            0
        )
    );
    let (out, err, code) = run(
        &["yq", "--jq-extensions", "-n", "1 as $__loc__ | $__loc__"],
        "",
    )?;
    assert_eq!(out, "", "stderr: {err}");
    assert_ne!(code, 0, "stderr: {err}");
    assert!(err.contains("unexpected $__loc__"), "stderr: {err}");
    Ok(())
}

/// jq mode is unchanged (jq 1.7.1): `$__loc__` is the location, `--arg __loc__` does not replace
/// it, and binding the name is a syntax error.
#[test]
fn test_jq_mode_keeps_loc_7997() -> Result<()> {
    let loc = "{\"file\":\"<top-level>\",\"line\":1}\n".to_string();
    assert_eq!(
        run(&["jq", "-nc", "$__loc__"], "")?,
        (loc.clone(), String::new(), 0)
    );
    assert_eq!(
        run(&["jq", "-nc", "--arg", "__loc__", "X", "$__loc__"], "")?,
        (loc, String::new(), 0)
    );
    let (out, err, code) = run(&["jq", "-nc", "1 as $__loc__ | $__loc__"], "")?;
    assert_eq!((out.as_str(), code), ("", 3), "stderr: {err}");
    assert!(
        err.contains("syntax error, unexpected $__loc__, expecting BINDING or '[' or '{'"),
        "stderr: {err}"
    );
    Ok(())
}
