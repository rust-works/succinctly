//! #8002: yq's empty-stage rule (#6943) where a write or a builtin evaluates the body itself. In a
//! pipe a literal, a constructor or `error(...)` after a stage that matched nothing still runs once
//! (`select(false) | 3` is `3`); the right side of `|=` is such a pipe over its target, and so are
//! the bodies of `map_values` (`.[] |= f`), `with_entries` (each entry its own context), `map` and
//! a collect `[...]` over a computed value. Those used to answer nothing here, so the update was
//! skipped and an `error` never fired.
//!
//! Every row was captured from yq v4.53.3. `succinctly jq` is unchanged: jq's pipe loops zero times
//! over an empty stage, and its `|=` deletes a target its filter has no output for.

use std::io::Write;
use std::process::{Command, Stdio};

use anyhow::Result;
use tempfile::NamedTempFile;

use crate::{run_jq_stdin, run_yq_stdin, run_yq_stdin_with_stderr};

const DOC: &str = "a: 1\nb: [1, 2]\nc: {x: 1, y: 2}\n";

/// (filter, yq's stdout with `-o=json -I0` over [`DOC`]).
const ROWS: &[(&str, &str)] = &[
    // The right side of `|=`, over one target and over a fan-out.
    (
        ".a |= (select(false) | 3)",
        "{\"a\":3,\"b\":[1,2],\"c\":{\"x\":1,\"y\":2}}\n",
    ),
    (
        ".b[] |= (select(false) | 3)",
        "{\"a\":1,\"b\":[3,3],\"c\":{\"x\":1,\"y\":2}}\n",
    ),
    (
        ".b[0] |= (select(false) | \"s\")",
        "{\"a\":1,\"b\":[\"s\",2],\"c\":{\"x\":1,\"y\":2}}\n",
    ),
    (
        ".b[] |= (select(. == 1) | 3)",
        "{\"a\":1,\"b\":[3,3],\"c\":{\"x\":1,\"y\":2}}\n",
    ),
    (
        "(.a, .b[0]) |= (select(false) | 3)",
        "{\"a\":3,\"b\":[3,2],\"c\":{\"x\":1,\"y\":2}}\n",
    ),
    (
        ".zz |= (select(false) | 3)",
        "{\"a\":1,\"b\":[1,2],\"c\":{\"x\":1,\"y\":2},\"zz\":3}\n",
    ),
    (
        ".c[] |= (select(false) | {\"k\": [1]})",
        "{\"a\":1,\"b\":[1,2],\"c\":{\"x\":{\"k\":[]},\"y\":{\"k\":[]}}}\n",
    ),
    // A later stage after the re-emitted one runs on it, and may empty it again.
    (
        ".b[] |= (select(false) | 3 | select(false) | 4)",
        "{\"a\":1,\"b\":[4,4],\"c\":{\"x\":1,\"y\":2}}\n",
    ),
    (
        ".b[] |= (select(false) | 3 | select(false))",
        "{\"a\":1,\"b\":[1,2],\"c\":{\"x\":1,\"y\":2}}\n",
    ),
    (
        ".b[] |= ((select(false) | 3) | . + 1)",
        "{\"a\":1,\"b\":[4,4],\"c\":{\"x\":1,\"y\":2}}\n",
    ),
    // A stage that reads its input does not re-emit.
    (
        ".b[] |= (select(false) | . + 1)",
        "{\"a\":1,\"b\":[1,2],\"c\":{\"x\":1,\"y\":2}}\n",
    ),
    (
        ".b[] |= (select(false) | length)",
        "{\"a\":1,\"b\":[1,2],\"c\":{\"x\":1,\"y\":2}}\n",
    ),
    // The other update operators and `with`.
    (
        ".a += (select(false) | 3)",
        "{\"a\":4,\"b\":[1,2],\"c\":{\"x\":1,\"y\":2}}\n",
    ),
    (
        "with(.a; . |= (select(false) | 3))",
        "{\"a\":3,\"b\":[1,2],\"c\":{\"x\":1,\"y\":2}}\n",
    ),
    // `map_values(f)` is `.[] |= f`: every member is a target of its own.
    (".c | map_values(select(false) | 3)", "{\"x\":3,\"y\":3}\n"),
    (".b | map_values(select(false) | 3)", "[3,3]\n"),
    (
        ".c | map_values(select(key == \"x\") | 3)",
        "{\"x\":3,\"y\":3}\n",
    ),
    (
        ".c |= map_values(select(false) | 3)",
        "{\"a\":1,\"b\":[1,2],\"c\":{\"x\":3,\"y\":3}}\n",
    ),
    // `with_entries(f)` runs `f` on each entry, so every entry re-emits.
    (
        ".c | with_entries(select(false) | {\"key\":\"z\",\"value\":9})",
        "{\"z\":9,\"z\":9}\n",
    ),
    (
        ".c | with_entries(select(.key == \"x\") | {\"key\":\"z\",\"value\":9})",
        "{\"z\":9,\"z\":9}\n",
    ),
    (
        ".c | with_entries(select((path | length) > 9) | {\"key\":\"z\",\"value\":9})",
        "{\"z\":9,\"z\":9}\n",
    ),
    (".c | with_entries(select(.key == \"x\"))", "{\"x\":1}\n"),
    // `map(f)` and a collect run `f` over their members as one list: once.
    (".c | to_entries | map(select(false) | 3)", "[3]\n"),
    (".c | to_entries | [.[] | select(false) | 3]", "[3]\n"),
    (
        ".b |= map(select(false) | 3)",
        "{\"a\":1,\"b\":[3],\"c\":{\"x\":1,\"y\":2}}\n",
    ),
    (
        ".b |= [.[] | select(false) | 3]",
        "{\"a\":1,\"b\":[3],\"c\":{\"x\":1,\"y\":2}}\n",
    ),
    (
        ".b |= (.[] | select(false) | 3)",
        "{\"a\":1,\"b\":3,\"c\":{\"x\":1,\"y\":2}}\n",
    ),
    (
        ".c |= (to_entries | map(select(false) | 3))",
        "{\"a\":1,\"b\":[1,2],\"c\":[3]}\n",
    ),
];

#[test]
fn an_update_right_side_keeps_the_pipes_empty_stage_rule_8002() -> Result<()> {
    for (filter, expected) in ROWS {
        let (stdout, code) = run_yq_stdin(filter, DOC, &["-o=json", "-I0"])?;
        assert_eq!(code, 0, "{filter}");
        assert_eq!(&stdout, expected, "{filter}");
    }
    Ok(())
}

/// The issue's rows, over JSON input and under `-n`: the route that evaluates a computed value
/// answers the same.
#[test]
fn the_rule_holds_over_json_input_and_a_computed_value_8002() -> Result<()> {
    for (input, filter, expected) in [
        ("[1,2]", ".[0] |= (select(false) | 3)", "[3,2]\n"),
        ("[1,2]", ".[0] |= (select(false) | \"s\")", "[\"s\",2]\n"),
        ("[1,2]", ".[] |= (select(false) | 3 + 4)", "[7,7]\n"),
        ("[1,2]", ".[] |= (select(false) | [3])", "[[],[]]\n"),
        ("[1,2]", "map_values(select(false) | 3)", "[3,3]\n"),
        (
            "{\"a\":1}",
            "with_entries(select(false) | {\"key\":\"z\",\"value\":9})",
            "{\"z\":9}\n",
        ),
        ("{\"a\":1}", "to_entries | map(select(false) | 3)", "[3]\n"),
        // Unchanged: the plain pipe already agreed, and a stage that reads its input is empty.
        ("[1,2]", "[select(false) | 3]", "[3]\n"),
        ("[1,2]", ".[0] = (select(false) | 3)", "[3,2]\n"),
        ("[1,2]", ".[0] |= (select(false) | .)", "[1,2]\n"),
    ] {
        let (stdout, code) = run_yq_stdin(filter, input, &["-p=json", "-o=json", "-I0"])?;
        assert_eq!(code, 0, "-p=json {filter}");
        assert_eq!(stdout, expected, "-p=json {filter}");
    }
    for (filter, expected) in [
        (".a |= (select(false) | 3)", "{\"a\":3}\n"),
        (".a |= (.b[] | 3)", "{\"a\":3}\n"),
        ("[1,2] | map_values(select(false) | 3)", "[3,3]\n"),
        ("[1,2] | .[] |= (select(false) | 3)", "[3,3]\n"),
        (
            "{\"p\":1,\"q\":2} | to_entries | map(select(false) | 3)",
            "[3]\n",
        ),
        (
            "{\"x\":{\"p\":1}} | .x | with_entries(select(false) | {\"key\":\"z\",\"value\":9})",
            "{\"z\":9}\n",
        ),
    ] {
        let (stdout, code) = run_yq_stdin(filter, "", &["-n", "-o=json", "-I0"])?;
        assert_eq!(code, 0, "-n {filter}");
        assert_eq!(stdout, expected, "-n {filter}");
    }
    Ok(())
}

/// An `error(...)` after the empty stage raises, where the update used to be skipped.
#[test]
fn an_error_after_an_empty_stage_raises_in_an_update_8002() -> Result<()> {
    for (input, filter, message) in [
        ("[1,2]", ".[] |= (select(. == 5) | error(\"x\"))", "x"),
        ("{\"a\":[]}", ".a |= (.[] | error(\"x\"))", "x"),
        (
            "{\"a\":1}",
            "with_entries(select(false) | error(\"x\"))",
            "x",
        ),
        (
            "{\"a\":1}",
            ".a |= (select(false) | error(\"x\" + \"y\"))",
            "xy",
        ),
        ("{\"a\":1}", "map_values(select(false) | error(\"x\"))", "x"),
        ("{\"a\":[1]}", ".a | map(select(false) | error(\"x\"))", "x"),
    ] {
        let (stdout, stderr, code) =
            run_yq_stdin_with_stderr(filter, input, &["-p=json", "-o=json", "-I0"])?;
        assert_ne!(code, 0, "{filter}");
        assert_eq!(stdout, "", "{filter}");
        assert_eq!(stderr, format!("Error: {message}\n"), "{filter}");
    }
    Ok(())
}

/// `--eval-all` and `-i` take routes of their own and answer the same.
#[test]
fn the_rule_holds_under_eval_all_and_inplace_8002() -> Result<()> {
    for (filter, expected) in [
        (
            ".a |= (select(false) | 3)",
            "{\"a\":3,\"b\":[1,2],\"c\":{\"x\":1,\"y\":2}}\n",
        ),
        (
            ".b[] |= (select(false) | 3)",
            "{\"a\":1,\"b\":[3,3],\"c\":{\"x\":1,\"y\":2}}\n",
        ),
        (".c | map_values(select(false) | 3)", "{\"x\":3,\"y\":3}\n"),
        (".c | to_entries | map(select(false) | 3)", "[3]\n"),
    ] {
        let (stdout, code) = run_yq_stdin(filter, DOC, &["--eval-all", "-o=json", "-I0"])?;
        assert_eq!(code, 0, "--eval-all {filter}");
        assert_eq!(stdout, expected, "--eval-all {filter}");
    }
    for (filter, expected) in [
        (
            ".b[] |= (select(false) | 3)",
            "a: 1\nb: [3, 3]\nc: {x: 1, y: 2}\n",
        ),
        (
            ".c |= with_entries(select(false) | {\"key\":\"z\",\"value\":9})",
            "a: 1\nb: [1, 2]\nc: {z: 9, z: 9}\n",
        ),
        (
            ".c |= map_values(select(false) | 3)",
            "a: 1\nb: [1, 2]\nc: {x: 3, y: 3}\n",
        ),
    ] {
        let mut file = NamedTempFile::new()?;
        write!(file, "{DOC}")?;
        let output = Command::new(env!("CARGO_BIN_EXE_succinctly"))
            .args(["yq", "-i", filter])
            .arg(file.path())
            .stdin(Stdio::null())
            .output()?;
        assert!(output.status.success(), "-i {filter}");
        assert_eq!(
            std::fs::read_to_string(file.path())?,
            expected,
            "-i {filter}"
        );
    }
    Ok(())
}

/// `succinctly jq` is unchanged (jq 1.7.1): `|=` deletes a target its filter has no output for, and
/// a collect after an empty stage is empty.
#[test]
fn jq_mode_keeps_its_own_rule_8002() -> Result<()> {
    for (input, filter, expected) in [
        ("[1,2]", ".[0] |= (select(false) | 3)", "[2]\n"),
        ("[1,2]", ".[] |= (select(. == 5) | error(\"x\"))", "[]\n"),
        ("[1,2]", "map_values(select(false) | 3)", "[]\n"),
        ("[1,2]", "[.[] | select(false) | 3]", "[]\n"),
        ("[1,2]", "map(select(false) | 3)", "[]\n"),
        ("{\"a\":1}", "to_entries | map(select(false) | 3)", "[]\n"),
        (
            "{\"a\":1}",
            "with_entries(select(false) | error(\"x\"))",
            "{}\n",
        ),
        (
            "{\"a\":1}",
            "with_entries(select(false) | {\"key\":\"z\",\"value\":9})",
            "{}\n",
        ),
    ] {
        let (stdout, code) = run_jq_stdin(filter, input, &["-c"])?;
        assert_eq!(code, 0, "jq {filter}");
        assert_eq!(stdout, expected, "jq {filter}");
    }
    Ok(())
}
