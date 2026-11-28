//! #8075: yq answers `now`, `env(NAME)` and `strenv(NAME)` once over an empty context, the way it
//! answers a literal there (`nowOp` and `envOperator`, `operator_datetime.go` and
//! `operator_env.go`, v4.53.3, return one new node whatever the context holds). So inside an
//! operator, an object, a binding or a fold after a stage that matched nothing they answer once:
//! `[.x[] | select(false) | (now + "1h")] | length` is `1`, as `("x" + "y")` is `["xy"]` there.
//! `with_dtf(L; E)` (`withDateTimeFormat`) reads `L` over the context and evaluates `E` over the
//! same one, so over an empty context it answers what `E` does: `with_dtf("2006"; "x")` is `"x"`.
//! All of these used to print nothing here.
//!
//! The clock moves, so a row over `now` prints a length. Every row was captured from yq v4.53.3 as
//! `yq -o=json -I0 FILTER` under `TZ=UTC X=hi`, on `x: [1, 2, 3]` unless it says otherwise.

use std::process::{Command, Stdio};

use anyhow::Result;

const XS: &str = "x: [1, 2, 3]\n";
const JSON: &[&str] = &["-o=json", "-I0"];

fn run_tool(tool: &str, args: &[&str], filter: &str, stdin: &str) -> Result<(String, String, i32)> {
    use std::io::Write;
    let mut child = Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .arg(tool)
        .args(args)
        .arg(filter)
        .env("TZ", "UTC")
        .env("X", "hi")
        .env_remove("NOPE")
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

fn assert_rows(args: &[&str], input: &str, rows: &[(&str, &str)]) -> Result<()> {
    for (filter, want) in rows {
        let got = run_tool("yq", args, filter, input)?;
        assert_eq!(
            (got.0.as_str(), got.1.as_str(), got.2),
            (*want, "", 0),
            "{args:?} {filter}"
        );
    }
    Ok(())
}

/// The issue's three rows.
const ISSUE_ROWS: &[(&str, &str)] = &[
    ("[.x[] | select(false) | (now + \"1h\")] | length", "1\n"),
    ("[.x[] | select(false) | {\"t\": now}] | length", "1\n"),
    (
        "[.x[] | select(false) | with_dtf(\"2006\"; \"x\")]",
        "[\"x\"]\n",
    ),
];

#[test]
fn the_issue_rows_answer_once_8075() -> Result<()> {
    assert_rows(JSON, XS, ISSUE_ROWS)
}

/// A once-built stage inside an operator, an object, a binding, a fold or an `error`.
const OPERAND_ROWS: &[(&str, &str)] = &[
    ("[.x[] | select(false) | (env(X) + \"a\")]", "[\"hia\"]\n"),
    (
        "[.x[] | select(false) | (\"a\" + strenv(X))]",
        "[\"ahi\"]\n",
    ),
    (
        "[.x[] | select(false) | (strenv(X) + strenv(X))]",
        "[\"hihi\"]\n",
    ),
    ("[.x[] | select(false) | (strenv(X) * 2)]", "[\"hihi\"]\n"),
    ("[.x[] | select(false) | (strenv(X) == \"hi\")]", "[true]\n"),
    ("[.x[] | select(false) | (strenv(X) != \"z\")]", "[true]\n"),
    ("[.x[] | select(false) | (strenv(X) < \"z\")]", "[true]\n"),
    ("[.x[] | select(false) | (now == now)]", "[true]\n"),
    ("[.x[] | select(false) | (now and true)]", "[true]\n"),
    (
        "[.x[] | select(false) | (strenv(X) and false)]",
        "[false]\n",
    ),
    ("[.x[] | select(false) | (strenv(X) or false)]", "[true]\n"),
    (
        "[.x[] | select(false) | (strenv(X) // \"z\")]",
        "[\"hi\"]\n",
    ),
    ("[.x[] | select(false) | (now // 1)] | length", "1\n"),
    (
        "[.x[] | select(false) | (strenv(NOPE) + \"a\")]",
        "[\"a\"]\n",
    ),
    (
        "[.x[] | select(false) | {\"t\": strenv(X)}]",
        "[{\"t\":\"hi\"}]\n",
    ),
    (
        "[.x[] | select(false) | {\"t\": {\"u\": strenv(X)}}]",
        "[{\"t\":{\"u\":\"hi\"}}]\n",
    ),
    (
        "[.x[] | select(false) | {\"t\": {\"u\": now}}] | length",
        "1\n",
    ),
    (
        "[.x[] | select(false) | {\"t\": (now | tostring)}] | length",
        "1\n",
    ),
    (
        "[.x[] | select(false) | {(strenv(X)): 1}]",
        "[{\"hi\":1}]\n",
    ),
    // A union in an object value answers every branch, the once-built one included.
    (
        "[.x[] | select(false) | {\"a\": (strenv(X), 1)}]",
        "[{\"a\":\"hi\"},{\"a\":1}]\n",
    ),
    (
        "[.x[] | select(false) | (strenv(X) as $v | $v)]",
        "[\"hi\"]\n",
    ),
    (
        "[.x[] | select(false) | (1 as $v | strenv(X) + \"a\")]",
        "[\"hia\"]\n",
    ),
    ("[.x[] | select(false) | (1 as $v | now)] | length", "1\n"),
    ("[.x[] | select(false) | (now as $t | $t)] | length", "1\n"),
    ("[.x[] | select(false) | (. as $t | now)] | length", "1\n"),
    (
        "[.x[] | select(false) | (.[] as $t ireduce (now; .))] | length",
        "1\n",
    ),
    // Other empty contexts: an absent key, a fan-out of nothing, a nested pipe.
    ("[select(false) | (strenv(X) + \"a\")]", "[\"hia\"]\n"),
    ("[.[] | select(false) | (strenv(X) + \"a\")]", "[\"hia\"]\n"),
    ("[.y[] | (strenv(X) + \"a\")]", "[\"hia\"]\n"),
    ("[.x[] | (select(false) | (now + \"1h\"))] | length", "1\n"),
    ("[.x[] | (select(false) | {\"t\": now})] | length", "1\n"),
];

#[test]
fn a_once_built_stage_answers_inside_an_operand_8075() -> Result<()> {
    assert_rows(JSON, XS, OPERAND_ROWS)
}

/// `with_dtf(L; E)` over an empty context answers what `E` does there, under `L`'s first answer.
const WITH_DTF_ROWS: &[(&str, &str)] = &[
    (
        "[.x[] | select(false) | with_dtf(\"2006\"; [now])]",
        "[[]]\n",
    ),
    ("[.x[] | select(false) | with_dtf(\"2006\"; [1])]", "[[]]\n"),
    (
        "[.x[] | select(false) | with_dtf((\"a\", \"b\"); \"x\")]",
        "[\"x\"]\n",
    ),
    ("[.x[] | select(false) | with_dtf(1; \"x\")]", "[\"x\"]\n"),
    (
        "[.x[] | select(false) | with_dtf(strenv(X); \"x\")]",
        "[\"x\"]\n",
    ),
    (
        "[.x[] | select(false) | with_dtf(strenv(X) + \"a\"; \"x\")]",
        "[\"x\"]\n",
    ),
    (
        "[.x[] | select(false) | with_dtf(\"2006\"; {\"a\": now})] | length",
        "1\n",
    ),
    (
        "[.x[] | select(false) | with_dtf(\"2006\"; {\"a\": strenv(X)})]",
        "[{\"a\":\"hi\"}]\n",
    ),
    (
        "[.x[] | select(false) | with_dtf(\"2006\"; strenv(X) + \"a\")]",
        "[\"hia\"]\n",
    ),
    (
        "[.x[] | select(false) | with_dtf(\"2006\"; \"2021\" | format_datetime(\"2006\"))]",
        "[2021]\n",
    ),
    (
        "[.x[] | select(false) | with_dtf(\"2006\"; with_dtf(\"2007\"; \"y\"))]",
        "[\"y\"]\n",
    ),
    (
        "[.x[] | select(false) | with_dtf(\"2006\"; \"a\", \"b\")]",
        "[\"a\",\"b\"]\n",
    ),
    ("[.x[] | select(false) | with_dtf(\"2006\"; 1, .)]", "[1]\n"),
    (
        "[.x[] | select(false) | with_dtf(\"2006\"; \"x\" | length)]",
        "[1]\n",
    ),
    (
        "[.x[] | select(false) | with_dtf(\"2006\"; \"x\") | length]",
        "[1]\n",
    ),
    (
        "[.x[] | select(false) | {\"t\": with_dtf(\"2006\"; \"y\")}]",
        "[{\"t\":\"y\"}]\n",
    ),
    (
        "[.x[] | select(false) | (\"a\" + with_dtf(\"2006\"; \"y\"))]",
        "[\"ay\"]\n",
    ),
    (
        "[.x[] | select(false) | {\"t\": with_dtf(\"2006\"; strenv(X))}]",
        "[{\"t\":\"hi\"}]\n",
    ),
    // A body that answers nothing over the empty context answers nothing, as in yq.
    ("[.x[] | select(false) | with_dtf(\"2006\"; .)]", "[]\n"),
    ("[.x[] | select(false) | with_dtf(\"2006\"; .a)]", "[]\n"),
    (
        "[.x[] | select(false) | with_dtf(\"2006\"; (1 | select(false)))]",
        "[]\n",
    ),
    // `now` as the stage itself was answered once before #8075, and still is.
    (
        "[.x[] | select(false) | with_dtf(\"2006\"; now)] | length",
        "1\n",
    ),
    ("[.x[] | select(false) | now] | length", "1\n"),
    ("[.x[] | select(false) | strenv(X)]", "[\"hi\"]\n"),
    ("[.x[] | select(false) | (now, 1)] | length", "2\n"),
];

#[test]
fn with_dtf_answers_what_its_body_does_over_an_empty_context_8075() -> Result<()> {
    assert_rows(JSON, XS, WITH_DTF_ROWS)
}

#[test]
fn with_dtf_raises_what_its_body_raises_over_an_empty_context_8075() -> Result<()> {
    let got = run_tool(
        "yq",
        JSON,
        "[.x[] | select(false) | with_dtf(\"2006\"; error(\"boom\"))]",
        XS,
    )?;
    assert_eq!(
        (got.0.as_str(), got.1.as_str(), got.2),
        ("", "Error: boom\n", 1)
    );
    Ok(())
}

/// The body of a write, a `map` or `with_entries` is a pipe of its own (#8002).
const BODY_ROWS: &[(&str, &str)] = &[
    (
        ".x[] |= (select(false) | (strenv(X) + \"a\"))",
        "x: [hia, hia, hia]\n",
    ),
    (
        ".x[] |= (select(false) | {\"t\": strenv(X)})",
        "x: [{t: hi}, {t: hi}, {t: hi}]\n",
    ),
    (
        ".x[] |= (select(false) | with_dtf(\"2006\"; \"x\"))",
        "x: [x, x, x]\n",
    ),
    (".x | map(select(false) | (strenv(X) + \"a\"))", "[hia]\n"),
    (
        ".x | map(select(false) | {\"t\": strenv(X)})",
        "[{t: hi}]\n",
    ),
    (
        ".x | map(select(false) | with_dtf(\"2006\"; \"x\"))",
        "[x]\n",
    ),
];

#[test]
fn a_body_a_write_or_map_runs_answers_once_8075() -> Result<()> {
    assert_rows(&[], XS, BODY_ROWS)
}

/// Over a context with nodes each node answers, as before.
const NONEMPTY_ROWS: &[(&str, &str)] = &[
    ("[.x[] | (now + \"1h\")] | length", "3\n"),
    ("[.x[] | {\"t\": now}] | length", "3\n"),
    (
        "[.x[] | with_dtf(\"2006\"; \"x\")]",
        "[\"x\",\"x\",\"x\"]\n",
    ),
    (
        "[.x[] | (strenv(X) + \"a\")]",
        "[\"hia\",\"hia\",\"hia\"]\n",
    ),
    (
        "[.x[] | {\"t\": strenv(X)}]",
        "[{\"t\":\"hi\"},{\"t\":\"hi\"},{\"t\":\"hi\"}]\n",
    ),
];

#[test]
fn a_context_with_nodes_answers_per_node_8075() -> Result<()> {
    assert_rows(JSON, XS, NONEMPTY_ROWS)
}

#[test]
fn several_documents_answer_once_each_8075() -> Result<()> {
    assert_rows(
        &[],
        "x: [1]\n---\nx: [2]\n",
        &[
            (".x[] | select(false) | (strenv(X) + \"a\")", "hia\nhia\n"),
            (
                ".x[] | select(false) | {\"t\": strenv(X)}",
                "t: hi\nt: hi\n",
            ),
            (".x[] | select(false) | with_dtf(\"2006\"; \"x\")", "x\nx\n"),
        ],
    )
}

/// jq's pipe loops zero times over an empty stage, so `succinctly jq` answers nothing.
#[test]
fn jq_mode_is_unchanged_8075() -> Result<()> {
    for filter in [
        "[.x[] | select(false) | (now + 1)] | length",
        "[.x[] | select(false) | {\"t\": now}] | length",
        "[.x[] | select(false) | ($ENV.X + \"a\")] | length",
    ] {
        let got = run_tool("jq", &["-c"], filter, "{\"x\": [1, 2, 3]}")?;
        assert_eq!(
            (got.0.as_str(), got.1.as_str(), got.2),
            ("0\n", "", 0),
            "{filter}"
        );
    }
    Ok(())
}
