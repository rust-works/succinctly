//! #7996: yq's `env(NAME)` and `strenv(NAME)` read the variable **once** for their whole context
//! and return one new node with no document (`envOperator`, `operator_env.go`, v4.53.3:
//! `context.SingleChildContext(node)`, which never looks at the nodes it was given). So a
//! multi-document input prints one copy per document with no `---` between them, `--eval-all`
//! prints one in all, `.[] | env(X)` over three members is one value, and `select(false) | env(X)`
//! is one value too. A write still runs per document, and keeps its `---`, because its result is
//! the document. Every row was captured from yq v4.53.3 with the variables below set.

use std::process::{Command, Stdio};

use anyhow::Result;

const TWO: &str = "a: 1\n---\nb: 2\n";
const THREE: &str = "a: 1\n---\nb: 2\n---\nc: 3\n";
const SEQ: &str = "- 1\n- 2\n- 3\n";
const SEQS: &str = "x: [1, 2, 3]\n---\nx: [4]\n";

/// The variables every row runs with; `SUCCINCTLY_NOPE7996` is never set.
const VARIABLES: &[(&str, &str)] = &[
    ("SUCCINCTLY_X7996", "hi"),
    ("SUCCINCTLY_M7996", "{b: 1}"),
    ("SUCCINCTLY_Q7996", "[1, 2]"),
];

/// (flags, filter, stdin, yq's stdout), each with exit status 0 and nothing on stderr.
const ROWS: &[(&[&str], &str, &str, &str)] = &[
    (&[], "env(SUCCINCTLY_X7996)", TWO, "hi\nhi\n"),
    (&["ea"], "env(SUCCINCTLY_X7996)", TWO, "hi\n"),
    (&[], "strenv(SUCCINCTLY_X7996)", TWO, "hi\nhi\n"),
    (&["ea"], "strenv(SUCCINCTLY_X7996)", TWO, "hi\n"),
    (&[], "env(SUCCINCTLY_X7996) | .", TWO, "hi\nhi\n"),
    (&["ea"], "env(SUCCINCTLY_X7996) | .", TWO, "hi\n"),
    (
        &[],
        "env(SUCCINCTLY_X7996), strenv(SUCCINCTLY_X7996)",
        TWO,
        "hi\nhi\nhi\nhi\n",
    ),
    (
        &["ea"],
        "env(SUCCINCTLY_X7996), strenv(SUCCINCTLY_X7996)",
        TWO,
        "hi\nhi\n",
    ),
    (&[], "[env(SUCCINCTLY_X7996)]", TWO, "- hi\n- hi\n"),
    (&["ea"], "[env(SUCCINCTLY_X7996)]", TWO, "- hi\n- hi\n"),
    (&[], "env(SUCCINCTLY_X7996) | length", TWO, "2\n2\n"),
    (&["ea"], "env(SUCCINCTLY_X7996) | length", TWO, "2\n"),
    (&[], "env(SUCCINCTLY_M7996) | .b", TWO, "1\n1\n"),
    (&["ea"], "env(SUCCINCTLY_M7996) | .b", TWO, "1\n"),
    (
        &[],
        "select(false) | env(SUCCINCTLY_X7996)",
        TWO,
        "hi\nhi\n",
    ),
    (
        &["ea"],
        "select(false) | env(SUCCINCTLY_X7996)",
        TWO,
        "hi\n",
    ),
    (
        &[],
        "select(true) | strenv(SUCCINCTLY_X7996)",
        TWO,
        "hi\nhi\n",
    ),
    (
        &["ea"],
        "select(true) | strenv(SUCCINCTLY_X7996)",
        TWO,
        "hi\n",
    ),
    (&[], "\"lit\" | env(SUCCINCTLY_X7996)", TWO, "hi\nhi\n"),
    (&["ea"], "\"lit\" | env(SUCCINCTLY_X7996)", TWO, "hi\n"),
    (&[], "env(SUCCINCTLY_X7996) | \"lit\"", TWO, "lit\nlit\n"),
    (&["ea"], "env(SUCCINCTLY_X7996) | \"lit\"", TWO, "lit\n"),
    (
        &[],
        ".a = env(SUCCINCTLY_X7996)",
        TWO,
        "a: hi\n---\nb: 2\na: hi\n",
    ),
    (
        &["ea"],
        ".a = env(SUCCINCTLY_X7996)",
        TWO,
        "a: hi\n---\nb: 2\na: hi\n",
    ),
    (
        &[],
        ".a = strenv(SUCCINCTLY_X7996)",
        TWO,
        "a: hi\n---\nb: 2\na: hi\n",
    ),
    (
        &["ea"],
        ".a = strenv(SUCCINCTLY_X7996)",
        TWO,
        "a: hi\n---\nb: 2\na: hi\n",
    ),
    (
        &[],
        ".a |= env(SUCCINCTLY_X7996)",
        TWO,
        "a: hi\n---\nb: 2\na: hi\n",
    ),
    (
        &["ea"],
        ".a |= env(SUCCINCTLY_X7996)",
        TWO,
        "a: hi\n---\nb: 2\na: hi\n",
    ),
    (&[], "env(SUCCINCTLY_X7996) | file_index", TWO, "0\n0\n"),
    (&["ea"], "env(SUCCINCTLY_X7996) | file_index", TWO, "0\n"),
    (
        &[],
        "env(SUCCINCTLY_X7996), \"lit\"",
        TWO,
        "hi\nlit\nhi\nlit\n",
    ),
    (
        &["ea"],
        "env(SUCCINCTLY_X7996), \"lit\"",
        TWO,
        "hi\nlit\nlit\n",
    ),
    (&[], ".[] | env(SUCCINCTLY_X7996)", TWO, "hi\nhi\n"),
    (&["ea"], ".[] | env(SUCCINCTLY_X7996)", TWO, "hi\n"),
    (&[], "env(SUCCINCTLY_X7996)", THREE, "hi\nhi\nhi\n"),
    (&["ea"], "env(SUCCINCTLY_X7996)", THREE, "hi\n"),
    (&[], "strenv(SUCCINCTLY_X7996)", THREE, "hi\nhi\nhi\n"),
    (&["ea"], "strenv(SUCCINCTLY_X7996)", THREE, "hi\n"),
    (&[], "env(SUCCINCTLY_X7996) | .", THREE, "hi\nhi\nhi\n"),
    (&["ea"], "env(SUCCINCTLY_X7996) | .", THREE, "hi\n"),
    (
        &[],
        "env(SUCCINCTLY_X7996), strenv(SUCCINCTLY_X7996)",
        THREE,
        "hi\nhi\nhi\nhi\nhi\nhi\n",
    ),
    (
        &["ea"],
        "env(SUCCINCTLY_X7996), strenv(SUCCINCTLY_X7996)",
        THREE,
        "hi\nhi\n",
    ),
    (&[], "[env(SUCCINCTLY_X7996)]", THREE, "- hi\n- hi\n- hi\n"),
    (
        &["ea"],
        "[env(SUCCINCTLY_X7996)]",
        THREE,
        "- hi\n- hi\n- hi\n",
    ),
    (&[], "env(SUCCINCTLY_X7996) | length", THREE, "2\n2\n2\n"),
    (&["ea"], "env(SUCCINCTLY_X7996) | length", THREE, "2\n"),
    (&[], "env(SUCCINCTLY_M7996) | .b", THREE, "1\n1\n1\n"),
    (&["ea"], "env(SUCCINCTLY_M7996) | .b", THREE, "1\n"),
    (
        &[],
        "select(false) | env(SUCCINCTLY_X7996)",
        THREE,
        "hi\nhi\nhi\n",
    ),
    (
        &["ea"],
        "select(false) | env(SUCCINCTLY_X7996)",
        THREE,
        "hi\n",
    ),
    (
        &[],
        "select(true) | strenv(SUCCINCTLY_X7996)",
        THREE,
        "hi\nhi\nhi\n",
    ),
    (
        &["ea"],
        "select(true) | strenv(SUCCINCTLY_X7996)",
        THREE,
        "hi\n",
    ),
    (
        &[],
        "\"lit\" | env(SUCCINCTLY_X7996)",
        THREE,
        "hi\nhi\nhi\n",
    ),
    (&["ea"], "\"lit\" | env(SUCCINCTLY_X7996)", THREE, "hi\n"),
    (
        &[],
        "env(SUCCINCTLY_X7996) | \"lit\"",
        THREE,
        "lit\nlit\nlit\n",
    ),
    (&["ea"], "env(SUCCINCTLY_X7996) | \"lit\"", THREE, "lit\n"),
    (
        &[],
        ".a = env(SUCCINCTLY_X7996)",
        THREE,
        "a: hi\n---\nb: 2\na: hi\n---\nc: 3\na: hi\n",
    ),
    (
        &["ea"],
        ".a = env(SUCCINCTLY_X7996)",
        THREE,
        "a: hi\n---\nb: 2\na: hi\n---\nc: 3\na: hi\n",
    ),
    (
        &[],
        ".a = strenv(SUCCINCTLY_X7996)",
        THREE,
        "a: hi\n---\nb: 2\na: hi\n---\nc: 3\na: hi\n",
    ),
    (
        &["ea"],
        ".a = strenv(SUCCINCTLY_X7996)",
        THREE,
        "a: hi\n---\nb: 2\na: hi\n---\nc: 3\na: hi\n",
    ),
    (
        &[],
        ".a |= env(SUCCINCTLY_X7996)",
        THREE,
        "a: hi\n---\nb: 2\na: hi\n---\nc: 3\na: hi\n",
    ),
    (
        &["ea"],
        ".a |= env(SUCCINCTLY_X7996)",
        THREE,
        "a: hi\n---\nb: 2\na: hi\n---\nc: 3\na: hi\n",
    ),
    (
        &[],
        "env(SUCCINCTLY_X7996) | file_index",
        THREE,
        "0\n0\n0\n",
    ),
    (&["ea"], "env(SUCCINCTLY_X7996) | file_index", THREE, "0\n"),
    (
        &[],
        "env(SUCCINCTLY_X7996), \"lit\"",
        THREE,
        "hi\nlit\nhi\nlit\nhi\nlit\n",
    ),
    (
        &["ea"],
        "env(SUCCINCTLY_X7996), \"lit\"",
        THREE,
        "hi\nlit\nlit\nlit\n",
    ),
    (&[], ".[] | env(SUCCINCTLY_X7996)", THREE, "hi\nhi\nhi\n"),
    (&["ea"], ".[] | env(SUCCINCTLY_X7996)", THREE, "hi\n"),
    (&[], ".[] | env(SUCCINCTLY_X7996)", SEQ, "hi\n"),
    (&["ea"], ".[] | env(SUCCINCTLY_X7996)", SEQ, "hi\n"),
    (&[], ".[] | strenv(SUCCINCTLY_X7996)", SEQ, "hi\n"),
    (&["ea"], ".[] | strenv(SUCCINCTLY_X7996)", SEQ, "hi\n"),
    (&[], "[.[] | env(SUCCINCTLY_X7996)]", SEQ, "- hi\n"),
    (&["ea"], "[.[] | env(SUCCINCTLY_X7996)]", SEQ, "- hi\n"),
    (&[], "[.[] | env(SUCCINCTLY_X7996)] | length", SEQ, "1\n"),
    (
        &["ea"],
        "[.[] | env(SUCCINCTLY_X7996)] | length",
        SEQ,
        "1\n",
    ),
    (&[], "map(env(SUCCINCTLY_X7996))", SEQ, "- hi\n"),
    (&["ea"], "map(env(SUCCINCTLY_X7996))", SEQ, "- hi\n"),
    (
        &[],
        "map(strenv(SUCCINCTLY_X7996) | . + \"!\")",
        SEQ,
        "- hi!\n",
    ),
    (
        &["ea"],
        "map(strenv(SUCCINCTLY_X7996) | . + \"!\")",
        SEQ,
        "- hi!\n",
    ),
    (
        &[],
        "map(. as $v | env(SUCCINCTLY_X7996))",
        SEQ,
        "- hi\n- hi\n- hi\n",
    ),
    (
        &["ea"],
        "map(. as $v | env(SUCCINCTLY_X7996))",
        SEQ,
        "- hi\n- hi\n- hi\n",
    ),
    (
        &[],
        ".[] | select(. > 5) | env(SUCCINCTLY_X7996)",
        SEQ,
        "hi\n",
    ),
    (
        &["ea"],
        ".[] | select(. > 5) | env(SUCCINCTLY_X7996)",
        SEQ,
        "hi\n",
    ),
    (
        &[],
        ".[] | (env(SUCCINCTLY_X7996), .)",
        SEQ,
        "hi\n1\n2\n3\n",
    ),
    (
        &[],
        ".[] as $v | env(SUCCINCTLY_X7996)",
        SEQ,
        "hi\nhi\nhi\n",
    ),
    (
        &["ea"],
        ".[] as $v | env(SUCCINCTLY_X7996)",
        SEQ,
        "hi\nhi\nhi\n",
    ),
    (
        &[],
        ".[] | env(SUCCINCTLY_X7996) as $e | $e",
        SEQ,
        "hi\nhi\nhi\n",
    ),
    (
        &["ea"],
        ".[] | env(SUCCINCTLY_X7996) as $e | $e",
        SEQ,
        "hi\nhi\nhi\n",
    ),
    (
        &[],
        ".[] | [env(SUCCINCTLY_X7996)]",
        SEQ,
        "- hi\n- hi\n- hi\n",
    ),
    (&[], ".[] | env(SUCCINCTLY_X7996), 9", SEQ, "hi\n9\n"),
    (&["ea"], ".[] | env(SUCCINCTLY_X7996), 9", SEQ, "hi\n9\n"),
    (
        &[],
        ".[] |= env(SUCCINCTLY_X7996)",
        SEQ,
        "- hi\n- hi\n- hi\n",
    ),
    (
        &["ea"],
        ".[] |= env(SUCCINCTLY_X7996)",
        SEQ,
        "- hi\n- hi\n- hi\n",
    ),
    (
        &[],
        ".[] = strenv(SUCCINCTLY_X7996)",
        SEQ,
        "- hi\n- hi\n- hi\n",
    ),
    (
        &["ea"],
        ".[] = strenv(SUCCINCTLY_X7996)",
        SEQ,
        "- hi\n- hi\n- hi\n",
    ),
    (&[], ".[] | env(SUCCINCTLY_Q7996) | .[]", SEQ, "1\n2\n"),
    (&["ea"], ".[] | env(SUCCINCTLY_Q7996) | .[]", SEQ, "1\n2\n"),
    (&[], ".[] | env(SUCCINCTLY_X7996) | path", SEQ, "[]\n"),
    (&["ea"], ".[] | env(SUCCINCTLY_X7996) | path", SEQ, "[]\n"),
    (
        &[],
        ".[] | (strenv(SUCCINCTLY_X7996), strenv(SUCCINCTLY_X7996))",
        SEQ,
        "hi\nhi\n",
    ),
    (
        &["ea"],
        ".[] | (strenv(SUCCINCTLY_X7996), strenv(SUCCINCTLY_X7996))",
        SEQ,
        "hi\nhi\n",
    ),
    (&[], ".[1:] | .[] | env(SUCCINCTLY_X7996)", SEQ, "hi\n"),
    (&["ea"], ".[1:] | .[] | env(SUCCINCTLY_X7996)", SEQ, "hi\n"),
    (
        &[],
        "with_entries(.value = env(SUCCINCTLY_X7996))",
        SEQ,
        "0: hi\n1: hi\n2: hi\n",
    ),
    (
        &["ea"],
        "with_entries(.value = env(SUCCINCTLY_X7996))",
        SEQ,
        "0: hi\n1: hi\n2: hi\n",
    ),
    (&[], ".x[] | env(SUCCINCTLY_X7996)", SEQS, "hi\nhi\n"),
    (&["ea"], ".x[] | env(SUCCINCTLY_X7996)", SEQS, "hi\n"),
    (&[], ".x[] | strenv(SUCCINCTLY_X7996)", SEQS, "hi\nhi\n"),
    (&["ea"], ".x[] | strenv(SUCCINCTLY_X7996)", SEQS, "hi\n"),
    (
        &[],
        ".x[] | select(. > 9) | env(SUCCINCTLY_X7996)",
        SEQS,
        "hi\nhi\n",
    ),
    (
        &["ea"],
        ".x[] | select(. > 9) | env(SUCCINCTLY_X7996)",
        SEQS,
        "hi\n",
    ),
    (
        &[],
        ".x[] | env(SUCCINCTLY_X7996) | . + \"!\"",
        SEQS,
        "hi!\nhi!\n",
    ),
    (
        &["ea"],
        ".x[] | env(SUCCINCTLY_X7996) | . + \"!\"",
        SEQS,
        "hi!\n",
    ),
    (
        &["-o", "json"],
        "env(SUCCINCTLY_X7996)",
        TWO,
        "\"hi\"\n\"hi\"\n",
    ),
    (
        &["ea", "-o", "json"],
        "env(SUCCINCTLY_M7996)",
        TWO,
        "{\n  \"b\": 1\n}\n",
    ),
    (
        &["-o", "json"],
        ".[] | env(SUCCINCTLY_M7996)",
        SEQ,
        "{\n  \"b\": 1\n}\n",
    ),
    (&["-n"], "env(SUCCINCTLY_X7996)", "", "hi\n"),
];

/// (flags, filter, stdin, yq's stderr), each with exit status 1 and nothing on stdout: the operator
/// runs once however many nodes reach it, none included, so its error is raised.
const ERROR_ROWS: &[(&[&str], &str, &str, &str)] = &[
    (
        &[],
        ".[] | select(false) | env(SUCCINCTLY_NOPE7996)",
        SEQ,
        "Error: value for env variable 'SUCCINCTLY_NOPE7996' not provided in env()\n",
    ),
    (
        &[],
        ".[] | env(SUCCINCTLY_NOPE7996)",
        SEQ,
        "Error: value for env variable 'SUCCINCTLY_NOPE7996' not provided in env()\n",
    ),
    (
        &["--security-disable-env-ops"],
        "select(false) | strenv(SUCCINCTLY_X7996)",
        SEQ,
        "Error: env operations have been disabled\n",
    ),
    (
        &["--security-disable-env-ops"],
        ".[] | select(false) | strenv(SUCCINCTLY_X7996)",
        SEQ,
        "Error: env operations have been disabled\n",
    ),
    (
        &["--security-disable-env-ops"],
        ".zz[] | strenv(SUCCINCTLY_X7996)",
        "a: 1\n",
        "Error: env operations have been disabled\n",
    ),
    (
        &["--security-disable-env-ops"],
        ".a | select(. == 2) | strenv(SUCCINCTLY_X7996)",
        "a: 1\n",
        "Error: env operations have been disabled\n",
    ),
    // The right side of a write takes the owned-value evaluator, which the rule reaches too.
    (
        &["--security-disable-env-ops"],
        ".a = (select(false) | strenv(SUCCINCTLY_X7996))",
        "a: 1\n",
        "Error: env operations have been disabled\n",
    ),
    (
        &["--security-disable-env-ops"],
        ".a |= (select(false) | strenv(SUCCINCTLY_X7996))",
        "a: 1\n",
        "Error: env operations have been disabled\n",
    ),
    (
        &["--security-disable-env-ops"],
        ".a = [.zz[] | strenv(SUCCINCTLY_X7996)]",
        "a: 1\n",
        "Error: env operations have been disabled\n",
    ),
];

/// The right side of a write, which the owned-value evaluator answers: yq reads the variable once
/// there too. (filter, yq's `-o json -I0` stdout) over `x: [1, 2, 3]`.
const WRITE_ROWS: &[(&str, &str)] = &[
    (".x |= map(env(SUCCINCTLY_X7996))", "{\"x\":[\"hi\"]}\n"),
    (".x = [.x[] | env(SUCCINCTLY_X7996)]", "{\"x\":[\"hi\"]}\n"),
    (".x = (.x[] | env(SUCCINCTLY_X7996))", "{\"x\":\"hi\"}\n"),
    (
        "with(.x; . = [.[] | strenv(SUCCINCTLY_X7996)])",
        "{\"x\":[\"hi\"]}\n",
    ),
    ("[1, 2, 3] | .[] | env(SUCCINCTLY_X7996)", "\"hi\"\n"),
    ("map(. + 1) | map(env(SUCCINCTLY_X7996))", "[\"hi\"]\n"),
    (".x | map(env(SUCCINCTLY_X7996))", "[\"hi\"]\n"),
    (".x[] | env(SUCCINCTLY_M7996) | .b", "1\n"),
    ("[.x[] | env(SUCCINCTLY_Q7996)[]]", "[1,2]\n"),
];

fn command(args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_succinctly"));
    command.arg("yq").args(args);
    for (name, value) in VARIABLES {
        command.env(name, value);
    }
    command.env_remove("SUCCINCTLY_NOPE7996");
    command
}

fn run(args: &[&str], filter: &str, stdin: &str) -> Result<(String, String, i32)> {
    use std::io::Write;
    let mut child = command(args)
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
fn env_and_strenv_read_once_per_context_with_no_document_7996() -> Result<()> {
    for (args, filter, stdin, want) in ROWS {
        let got = run(args, filter, stdin)?;
        assert_eq!(
            (got.0.as_str(), got.1.as_str(), got.2),
            (*want, "", 0),
            "{args:?} {filter} over {stdin:?}"
        );
    }
    Ok(())
}

#[test]
fn an_error_of_env_is_raised_however_many_nodes_reach_it_7996() -> Result<()> {
    for (args, filter, stdin, want) in ERROR_ROWS {
        let got = run(args, filter, stdin)?;
        assert_eq!(
            (got.0.as_str(), got.1.as_str(), got.2),
            ("", *want, 1),
            "{args:?} {filter} over {stdin:?}"
        );
    }
    Ok(())
}

#[test]
fn a_write_reads_env_once_over_its_context_7996() -> Result<()> {
    for (filter, want) in WRITE_ROWS {
        let got = run(&["-o", "json", "-I0"], filter, "x: [1, 2, 3]\n")?;
        assert_eq!(
            (got.0.as_str(), got.1.as_str(), got.2),
            (*want, "", 0),
            "{filter}"
        );
    }
    Ok(())
}

/// Several files: each document is its own context in a plain run, and `--eval-all` reads every
/// document of every file as one context, so it prints one value (`yq ea 'env(X)' two.yaml one.yaml`).
#[test]
fn several_files_print_one_value_per_context_7996() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let two = dir.path().join("two.yaml");
    let one = dir.path().join("one.yaml");
    std::fs::write(&two, TWO)?;
    std::fs::write(&one, "a: 1\n")?;
    let files = [two.to_str().expect("utf-8"), one.to_str().expect("utf-8")];
    for (args, want) in [
        (&["env(SUCCINCTLY_X7996)"][..], "hi\nhi\nhi\n"),
        (&["ea", "env(SUCCINCTLY_X7996)"][..], "hi\n"),
        (
            &["ea", "strenv(SUCCINCTLY_X7996), \"lit\""][..],
            "hi\nlit\nlit\nlit\n",
        ),
        (&["\"lit\""][..], "lit\nlit\nlit\n"),
    ] {
        let output = command(args).args(files).stdin(Stdio::null()).output()?;
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

/// `succinctly jq` keeps jq's per-value pipe: its `strenv(VAR)` and `env(VAR)` extensions run
/// once per input, as every jq filter does.
#[test]
fn jq_mode_reads_env_per_value_7996() -> Result<()> {
    let filter_out = |filter: &str| -> Result<(String, i32)> {
        let output = Command::new(env!("CARGO_BIN_EXE_succinctly"))
            .args(["jq", "-c", filter])
            .env("SUCCINCTLY_X7996", "hi")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .and_then(|mut child| {
                use std::io::Write;
                child
                    .stdin
                    .take()
                    .expect("piped stdin")
                    .write_all(b"[1,2,3]")?;
                child.wait_with_output()
            })?;
        Ok((
            String::from_utf8(output.stdout)?,
            output.status.code().unwrap_or(-1),
        ))
    };
    for (filter, want) in [
        (".[] | strenv(SUCCINCTLY_X7996)", "\"hi\"\n\"hi\"\n\"hi\"\n"),
        (".[] | env(SUCCINCTLY_X7996)", "\"hi\"\n\"hi\"\n\"hi\"\n"),
        ("map(strenv(SUCCINCTLY_X7996))", "[\"hi\",\"hi\",\"hi\"]\n"),
        (".[] | select(false) | strenv(SUCCINCTLY_X7996)", ""),
    ] {
        assert_eq!(filter_out(filter)?, (want.to_string(), 0), "{filter}");
    }
    Ok(())
}

/// `--jq-extensions` only, so no yq row: the stages before `env(X)` run to the end before it is
/// read, as they do before a `$name` stage (#6821), so an early exit cannot stop an endless prefix.
/// Recorded in `docs/compliance/yq/limitations.md`.
#[test]
fn an_endless_prefix_runs_to_its_limit_before_env_7996() -> Result<()> {
    for (filter, want) in [
        (
            "limit(1; repeat(1) | env(SUCCINCTLY_X7996))",
            ("", "Error: repeat: maximum iterations exceeded\n", 1),
        ),
        (
            ".n as $v | limit(1; repeat(1) | $v)",
            ("", "Error: repeat: maximum iterations exceeded\n", 1),
        ),
        (
            "limit(1; range(3) | env(SUCCINCTLY_X7996))",
            ("\"hi\"\n", "", 0),
        ),
    ] {
        let got = run(&["--jq-extensions", "-o", "json", "-I0"], filter, "n: 7\n")?;
        assert_eq!((got.0.as_str(), got.1.as_str(), got.2), want, "{filter}");
    }
    Ok(())
}
