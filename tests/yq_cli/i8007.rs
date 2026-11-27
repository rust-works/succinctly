//! #8007: yq's `now` reads the clock **once** for its whole context and returns one new node with
//! no document (`nowOp`, `operator_datetime.go`, v4.53.3: `context.SingleChildContext(node)`, which
//! never looks at the nodes it was given), as `env(NAME)` does (#7996). So `[.[] | now]` over three
//! members holds one timestamp, `select(false) | now` is one value, a multi-document input prints one
//! value per document with no `---` between them, and `--eval-all` prints one in all.
//!
//! The clock moves, so no row prints a timestamp: each counts the values (`[...] | length`) or
//! prints the length of one, which is 20 under `TZ=UTC` (`2006-01-02T15:04:05Z`, RFC 3339 with no
//! fraction). Every row was captured from yq v4.53.3 under `TZ=UTC`.
//!
//! The `!!timestamp` tag yq gives the node is not reproduced: an owned value has no timestamp kind
//! (#2802), so `now | type` is still `!!str` here (`docs/compliance/yq/limitations.md`).

use std::process::{Command, Stdio};

use anyhow::Result;

const ONE: &str = "a: 1\n";
const TWO: &str = "a: 1\n---\nb: 2\n";
const THREE: &str = "a: 1\n---\nb: 2\n---\nc: 3\n";
const SEQ: &str = "- 1\n- 2\n- 3\n";
const XS: &str = "x: [1, 2, 3]\n";

/// (flags, filter, stdin, yq's stdout), each with exit status 0 and nothing on stderr.
const ROWS: &[(&[&str], &str, &str, &str)] = &[
    (&[], "[.[] | now] | length", SEQ, "1\n"),
    (&["ea"], "[.[] | now] | length", SEQ, "1\n"),
    (&[], "[.[] | now] | length", THREE, "1\n1\n1\n"),
    // yq's collect runs its body once per document of the context, so each holds one value.
    (&["ea"], "[.[] | now] | length", THREE, "3\n"),
    (&["ea"], "[now] | length", THREE, "3\n"),
    (&[], ".[] | now | length", SEQ, "20\n"),
    (&[], "[.[] | select(. > 1) | now] | length", SEQ, "1\n"),
    (&[], "[.[] | select(false) | now] | length", SEQ, "1\n"),
    (&[], "select(false) | now | length", TWO, "20\n20\n"),
    (&["ea"], "select(false) | now | length", TWO, "20\n"),
    (&[], ".zz[] | now | length", TWO, "20\n20\n"),
    (&["ea"], ".zz[] | now | length", TWO, "20\n"),
    (&[], "now | length", TWO, "20\n20\n"),
    (&["ea"], "now | length", TWO, "20\n"),
    (&[], "now | length", THREE, "20\n20\n20\n"),
    (&["ea"], "now | length", THREE, "20\n"),
    (&[], "now | length, \"lit\"", TWO, "20\nlit\n20\nlit\n"),
    (&["ea"], "(now | length), \"lit\"", TWO, "20\nlit\nlit\n"),
    (&[], "\"lit\" | now | length", TWO, "20\n20\n"),
    (&["ea"], "\"lit\" | now | length", TWO, "20\n"),
    (&[], "map(now) | length", SEQ, "1\n"),
    (&[], "[.[] | (now, now)] | length", SEQ, "2\n"),
    (&[], "[.[] | ({}, now)] | length", SEQ, "2\n"),
    (&[], "[(.[] | now), (.[] | now)] | length", SEQ, "2\n"),
    (&[], "[.[] | now | tostring] | length", SEQ, "1\n"),
    (&[], "[.[] | now] | unique | length", SEQ, "1\n"),
    (
        &[],
        "[.[] | now | to_unix | tag] | unique",
        SEQ,
        "- '!!int'\n",
    ),
    (
        &[],
        "[.[] | now | format_datetime(\"2006\") | tag] | unique",
        SEQ,
        "- '!!int'\n",
    ),
    // An `as` body runs once per node in yq, so neither of these is owned by the walk.
    (&[], "[.[] | . as $v | now] | length", SEQ, "3\n"),
    (&[], "[.[] | now as $t | 1] | length", SEQ, "3\n"),
    (&[], "now | key", TWO, ""),
];

/// The right side of a write, which the owned-value evaluator answers: yq reads the clock once
/// there too, and a write to several targets reads it per target. (filter, yq's `-o json -I0`
/// stdout) over `x: [1, 2, 3]`.
const WRITE_ROWS: &[(&str, &str)] = &[
    (".y = ([.x[] | now] | length)", "{\"x\":[1,2,3],\"y\":1}\n"),
    (".x |= (map(now) | length)", "{\"x\":1}\n"),
    (".x = ([.x[] | now] | length)", "{\"x\":1}\n"),
    ("with(.x; . = ([.[] | now] | length))", "{\"x\":1}\n"),
    (".x = (.x[] | now | length)", "{\"x\":20}\n"),
    (".x |= ([.[] | now | length])", "{\"x\":[20]}\n"),
    (".x = [.x[] | now | length]", "{\"x\":[20]}\n"),
    (".x[] |= (now | length)", "{\"x\":[20,20,20]}\n"),
];

fn command(tool: &str, args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_succinctly"));
    command.arg(tool).args(args).env("TZ", "UTC");
    command
}

fn run(tool: &str, args: &[&str], filter: &str, stdin: &str) -> Result<(String, String, i32)> {
    use std::io::Write;
    let mut child = command(tool, args)
        .arg(filter)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    child
        .stdin
        .take()
        .expect("piped stdin")
        .write_all(stdin.as_bytes())?;
    let output = child.wait_with_output()?;
    Ok((
        String::from_utf8(output.stdout)?,
        String::from_utf8(output.stderr)?,
        output.status.code().unwrap_or(-1),
    ))
}

#[test]
fn now_is_built_once_per_context_with_no_document_8007() -> Result<()> {
    for (args, filter, stdin, want) in ROWS {
        let got = run("yq", args, filter, stdin)?;
        assert_eq!(
            (got.0.as_str(), got.1.as_str(), got.2),
            (*want, "", 0),
            "{args:?} {filter} over {stdin:?}"
        );
    }
    Ok(())
}

#[test]
fn a_write_reads_now_once_over_its_context_8007() -> Result<()> {
    for (filter, want) in WRITE_ROWS {
        let got = run("yq", &["-o", "json", "-I0"], filter, XS)?;
        assert_eq!(
            (got.0.as_str(), got.1.as_str(), got.2),
            (*want, "", 0),
            "{filter}"
        );
    }
    Ok(())
}

/// Several files: each document is its own context in a plain run, and `--eval-all` reads every
/// document of every file as one context, so it prints one value.
#[test]
fn several_files_print_one_now_per_context_8007() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let two = dir.path().join("two.yaml");
    let one = dir.path().join("one.yaml");
    std::fs::write(&two, TWO)?;
    std::fs::write(&one, ONE)?;
    let files = [two.to_str().expect("utf-8"), one.to_str().expect("utf-8")];
    for (args, want) in [
        (&["now | length"][..], "20\n20\n20\n"),
        (&["ea", "now | length"][..], "20\n"),
        // yq's collect runs its body once per document of the context, so each holds one value.
        (&["ea", "[.[] | now] | length"][..], "3\n"),
    ] {
        let output = command("yq", args)
            .args(files)
            .stdin(Stdio::null())
            .output()?;
        assert_eq!(
            (
                String::from_utf8(output.stdout)?.as_str(),
                String::from_utf8(output.stderr)?.as_str(),
                output.status.code()
            ),
            (want, "", Some(0)),
            "{args:?}"
        );
    }
    Ok(())
}

/// `succinctly jq` keeps jq's per-value pipe: `now` is a number, read once per input.
#[test]
fn jq_mode_reads_now_per_value_8007() -> Result<()> {
    for (filter, want) in [
        ("[.[] | now] | length", "3\n"),
        ("map(now) | length", "3\n"),
        ("now | type", "\"number\"\n"),
        ("[.[] | now | type] | unique", "[\"number\"]\n"),
        ("[.[] | select(false) | now] | length", "0\n"),
    ] {
        let got = run("jq", &["-c"], filter, "[1,2,3]")?;
        assert_eq!(
            (got.0.as_str(), got.1.as_str(), got.2),
            (want, "", 0),
            "{filter}"
        );
    }
    Ok(())
}

/// `--jq-extensions` only, so no yq row: the stages before `now` run to the end before it is read,
/// as they do before `env(X)` (#7996) and a `$name` stage (#6821), so an early exit cannot stop an
/// endless prefix. Recorded in `docs/compliance/yq/limitations.md`.
#[test]
fn an_endless_prefix_runs_to_its_limit_before_now_8007() -> Result<()> {
    for (filter, want) in [
        (
            "limit(1; repeat(1) | now)",
            ("", "Error: repeat: maximum iterations exceeded\n", 1),
        ),
        ("limit(1; range(3) | now) | length", ("20\n", "", 0)),
    ] {
        let got = run("yq", &["--jq-extensions", "-o", "json", "-I0"], filter, ONE)?;
        assert_eq!((got.0.as_str(), got.1.as_str(), got.2), want, "{filter}");
    }
    Ok(())
}
