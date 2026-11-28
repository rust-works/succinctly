//! #8080: `error(msg)` evaluates its message at the input's position, so a `key`, `path`,
//! `parent` or `line` inside it reads the node the call stands at. It used to be evaluated over a
//! copy with no position: `.b | error(key)` raised `aborted` (the message produced nothing) and
//! `.b | error(path | join("."))` raised an empty message. Every row is captured from yq v4.53.3
//! on `a: 1` / `b: 5`, which writes `Error: <message>` to stderr and exits 1.

use anyhow::Result;

use crate::{run_jq_stdin_with_stderr, run_yq_stdin_with_stderr};

const INPUT: &str = "a: 1\nb: 5\n";

/// `(filter, message)`: yq v4.53.3 raises `Error: <message>`.
const ROWS: &[(&str, &str)] = &[
    // The issue's table.
    (".b | error(key)", "b"),
    (".b | error(key | tostring)", "b"),
    (".b | error(\"k=\" + key)", "k=b"),
    (".b | error(path | join(\".\"))", "b"),
    (".b |= error(key)", "b"),
    ("with(.b; . |= error(key))", "b"),
    // The controls, already right.
    (".b | (key as $k | error($k))", "b"),
    (".b | error(.)", "5"),
    // Beyond the table: an iteration, `select`, `map`, an `as` body, `parent`, `line`.
    (".[] | error(key)", "a"),
    (".[] | select(. == 5) | error(key)", "b"),
    (".b | select(error(key))", "b"),
    ("map(error(key))", "a"),
    (".b | . as $v | error(key)", "b"),
    (".b | error(parent | keys | join(\",\"))", "a,b"),
    (".b | error(line)", "2"),
    (".b | error(document_index)", "0"),
    // Interpolation, `//`, and a generator message, whose first output is raised.
    (".b | error(\"\\(key)\")", "b"),
    (".b | error(key // \"d\")", "b"),
    (".b | error(key, \"x\")", "b"),
    // An absent key still has a position.
    (".c | error(key)", "c"),
    (".c | error(path | join(\".\"))", "c"),
    // ... and so does one followed by stages that never run.
    (".c | error(key) | tostring", "c"),
    ("[.c | error(key) | 1]", "c"),
    (".c.d | error(path | join(\".\")) | 1", "c.d"),
    // The raise stops the pipe and the comma.
    (".b | error(key) | 1", "b"),
    (".b | (error(key), 1)", "b"),
    // A message that produces nothing raises `aborted`: `key` at the root has no output.
    (".b as $v | error(key)", "aborted"),
    (".b | error(null)", "null"),
];

#[test]
fn an_error_message_reads_the_inputs_position_8080() -> Result<()> {
    for (filter, message) in ROWS {
        let (stdout, stderr, code) = run_yq_stdin_with_stderr(filter, INPUT, &[])?;
        assert_eq!(
            (stdout.as_str(), stderr.trim_end(), code),
            ("", format!("Error: {message}").as_str(), 1),
            "{filter}"
        );
    }
    Ok(())
}

/// The same rule under `--eval-all` and `-o json`, which take other routes to the message.
#[test]
fn an_error_message_reads_the_position_on_every_route_8080() -> Result<()> {
    for args in [&["--eval-all"][..], &["-o", "json"][..]] {
        for (filter, message) in [(".b | error(key)", "b"), (".b |= error(key)", "b")] {
            let (stdout, stderr, code) = run_yq_stdin_with_stderr(filter, INPUT, args)?;
            assert_eq!(
                (stdout.as_str(), stderr.trim_end(), code),
                ("", format!("Error: {message}").as_str(), 1),
                "{filter} {args:?}"
            );
        }
    }
    Ok(())
}

/// The recorded divergences (`docs/compliance/yq/limitations.md`, "An `error` message that reads
/// its position"): a positioned message answers what `msg as $m | error($m)` answers, where the
/// construct around it differs from yq. yq v4.53.3's answer is in each row's comment.
#[test]
fn an_error_message_answers_as_its_binding_spelling_does_8080() -> Result<()> {
    let two_docs = "a: 1\n---\nb: 2\n";
    for (filter, input, args, message) in [
        // yq: `aborted`, raised for the first document, whose `.a.x` matched nothing.
        (".a.x | error(key)", two_docs, &[][..], "x"),
        (".a.x | (key as $k | error($k))", two_docs, &[][..], "x"),
        // yq: `aborted`.
        ("[.b | error(key)]", two_docs, &["--eval-all"][..], "b"),
        // yq: `aborted`, at the root, which has no key.
        ("with(..; error(key))", INPUT, &[][..], "a"),
        (
            "with(..; . |= (key as $k | error($k)))",
            INPUT,
            &[][..],
            "a",
        ),
        // yq: `b`, the last target.
        (".[] |= error(key)", INPUT, &[][..], "a"),
        (".[] |= (key as $k | error($k))", INPUT, &[][..], "a"),
        // yq: an empty message, as for any array or mapping.
        (".b | error(path)", INPUT, &[][..], "[\"b\"]"),
    ] {
        let (stdout, stderr, code) = run_yq_stdin_with_stderr(filter, input, args)?;
        assert_eq!(
            (stdout.as_str(), stderr.trim_end(), code),
            ("", format!("Error: {message}").as_str(), 1),
            "{filter} {args:?}"
        );
    }
    Ok(())
}

/// jq mode keeps jq 1.7.1's answers (captured from `/usr/bin/jq`): its messages never read a
/// position, so none of them takes the new route.
#[test]
fn jq_error_messages_are_unchanged_8080() -> Result<()> {
    let input = "{\"a\":{\"x\":1},\"b\":\"hi\"}\n";
    for (filter, expected) in [
        (".a | try error(.) catch .", "{\"x\":1}"),
        (".a | try error(.x) catch .", "1"),
        ("try error(path(..)) catch .", "[]"),
        (".b | try error(\"k=\" + .) catch .", "\"k=hi\""),
        ("[.a | try error((1, 2)) catch .]", "[1]"),
        ("[.a | try error(empty) catch .]", "[]"),
        // jq 1.7.1 has no `key`, `path` or `line` (it rejects these at compile time); as
        // succinctly extensions they read the position as `key as $k | error($k)` already did.
        (".a | try error(key) catch .", "\"a\""),
        (".a.x | try error(path) catch .", "[\"a\",\"x\"]"),
        (".a.x | try error(line) catch .", "1"),
    ] {
        let (stdout, stderr, code) = run_jq_stdin_with_stderr(filter, input, &["-c"])?;
        assert_eq!(
            (stdout.trim_end(), stderr.as_str(), code),
            (expected, "", 0),
            "{filter}"
        );
    }
    Ok(())
}
