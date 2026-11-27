//! #8028: yq's `with_dtf(LAYOUT; E)` evaluates `E` over the **whole** context it is handed
//! (`withDateTimeFormat`, `operator_datetime.go`, v4.53.3: it sets the layout and returns
//! `d.GetMatchingNodes(context, E)`), so a call whose `E` holds a stage yq answers once for its
//! whole context (`now`, `env(NAME)`, `strenv(NAME)`, `{}`, a `$name`) is answered once too:
//! `[.[] | with_dtf("2006"; now)]` over three members holds one timestamp, as `[.[] | now]` does
//! since #8007. A body with no such stage (`.`, `[now]`, `{"t": now}`, an `as` body) still answers
//! once per node.
//!
//! The clock moves, so no row prints a timestamp: each counts the values or prints the length of
//! one, which is 20 under `TZ=UTC`. Every row was captured from yq v4.53.3 under `TZ=UTC X=hi`.

use std::process::{Command, Stdio};

use anyhow::Result;

const TWO: &str = "a: 1\n---\nb: 2\n";
const SEQ: &str = "- 1\n- 2\n- 3\n";
const SEQ2: &str = "- 1\n- 2\n---\n- 3\n- 4\n";
const XS: &str = "x: [1, 2, 3]\n";

/// (flags, filter, stdin, yq's stdout), each with exit status 0 and nothing on stderr.
const ROWS: &[(&[&str], &str, &str, &str)] = &[
    (&[], "[.[] | with_dtf(\"2006\"; now)] | length", SEQ, "1\n"),
    (
        &["ea"],
        "[.[] | with_dtf(\"2006\"; now)] | length",
        SEQ,
        "1\n",
    ),
    (
        &[],
        "[.[] | with_dtf(\"2006\"; strenv(X))] | length",
        SEQ,
        "1\n",
    ),
    (
        &[],
        "[.[] | with_dtf(\"2006\"; env(X))] | length",
        SEQ,
        "1\n",
    ),
    (&[], "[.[] | with_dtf(\"2006\"; {})] | length", SEQ, "1\n"),
    (
        &[],
        "[.[] | with_dtf(\"2006\"; (now))] | length",
        SEQ,
        "1\n",
    ),
    // A stage after the once-stage runs over its one node.
    (
        &[],
        "[.[] | with_dtf(\"2006\"; now | tostring)] | length",
        SEQ,
        "1\n",
    ),
    (
        &[],
        "[.[] | with_dtf(\"2006\"; now) | tostring] | length",
        SEQ,
        "1\n",
    ),
    (
        &[],
        "[.[] | with_dtf(\"2006\"; now) | select(false)] | length",
        SEQ,
        "0\n",
    ),
    (
        &[],
        "[.[] | with_dtf(\"2006\"; now | select(false))] | length",
        SEQ,
        "0\n",
    ),
    (
        &[],
        "[.[] | with_dtf(\"2006\"; with_dtf(\"2006\"; now))] | length",
        SEQ,
        "1\n",
    ),
    // Stages before the once-stage inside the body run over the whole context and leave one node.
    (
        &[],
        "[.[] | with_dtf(\"2006\"; select(true) | now)] | length",
        SEQ,
        "1\n",
    ),
    (
        &[],
        "[.[] | with_dtf(\"2006\"; .[] | now)] | length",
        SEQ,
        "1\n",
    ),
    (
        &[],
        "[.[] | with_dtf(\"2006\"; . | now)] | length",
        SEQ,
        "1\n",
    ),
    (
        &[],
        "[.[] | with_dtf(\"2006\"; with_dtf(\"2006\"; .) | now)] | length",
        SEQ,
        "1\n",
    ),
    // Over an empty context too, as `select(false) | now` is one value.
    (
        &[],
        "[.[] | select(false) | with_dtf(\"2006\"; now)] | length",
        SEQ,
        "1\n",
    ),
    (
        &[],
        "7 as $v | [.[] | with_dtf(\"2006\"; $v)] | length",
        SEQ,
        "1\n",
    ),
    // `|` binds tighter than `,`, so the `1` is not after the fan-out.
    (
        &[],
        "[.[] | with_dtf(\"2006\"; now), 1] | length",
        SEQ,
        "2\n",
    ),
    (&[], "map(with_dtf(\"2006\"; now)) | length", SEQ, "1\n"),
    (&[], "map(with_dtf(\"2006\"; now), 1) | length", SEQ, "4\n"),
    (&[], ".[] | with_dtf(\"2006\"; now) | length", SEQ, "20\n"),
    // No once-stage in the body: one answer per node, as before.
    (&[], "[.[] | with_dtf(\"2006\"; .)] | length", SEQ, "3\n"),
    (
        &[],
        "[.[] | with_dtf(\"2006\"; [now])] | length",
        SEQ,
        "3\n",
    ),
    (
        &[],
        "[.[] | with_dtf(\"2006\"; {\"t\": now})] | length",
        SEQ,
        "3\n",
    ),
    (
        &[],
        "[.[] | with_dtf(\"2006\"; now) as $t | $t] | length",
        SEQ,
        "3\n",
    ),
    (
        &[],
        "[.[] | with_dtf(\"2006\"; . as $w | now)] | length",
        SEQ,
        "3\n",
    ),
    // Several documents: one value per document with no `---`, and one in all under `ea`.
    (&[], "with_dtf(\"2006\"; now) | length", TWO, "20\n20\n"),
    (&["ea"], "with_dtf(\"2006\"; now) | length", TWO, "20\n"),
    (
        &[],
        ".zz[] | with_dtf(\"2006\"; now) | length",
        TWO,
        "20\n20\n",
    ),
    (
        &["ea"],
        ".zz[] | with_dtf(\"2006\"; now) | length",
        TWO,
        "20\n",
    ),
    (&[], ".[] | with_dtf(\"2006\"; strenv(X))", SEQ2, "hi\nhi\n"),
    (&["ea"], ".[] | with_dtf(\"2006\"; strenv(X))", SEQ2, "hi\n"),
    (
        &[],
        ".[] | with_dtf(\"2006\"; now) | length",
        SEQ2,
        "20\n20\n",
    ),
    // The separator follows the body's node: a literal has no document, `.` keeps its own.
    (&[], ".[] | with_dtf(\"2006\"; \"a\")", SEQ2, "a\na\na\na\n"),
    (
        &[],
        ".[] | with_dtf(\"2006\"; .)",
        SEQ2,
        "1\n2\n---\n3\n4\n",
    ),
    (
        &[],
        ".[] | with_dtf(\"2006\"; .) | \"a\"",
        SEQ2,
        "a\na\na\na\n",
    ),
];

/// The right side of a write, which the owned-value evaluator answers. (filter, yq's `-o json -I0`
/// stdout) over `x: [1, 2, 3]`.
const WRITE_ROWS: &[(&str, &str)] = &[
    (
        ".y = ([.x[] | with_dtf(\"2006\"; now)] | length)",
        "{\"x\":[1,2,3],\"y\":1}\n",
    ),
    (
        ".y = [.x[] | with_dtf(\"2006\"; strenv(X))]",
        "{\"x\":[1,2,3],\"y\":[\"hi\"]}\n",
    ),
    (
        ".y = [.x[] | with_dtf(\"2006\"; select(. > 1) | strenv(X))]",
        "{\"x\":[1,2,3],\"y\":[\"hi\"]}\n",
    ),
    // `|=` reads its right side from `.y`, which is `null`: `.x[]` yields nothing there.
    (
        ".y |= [.x[] | with_dtf(\"2006\"; strenv(X))]",
        "{\"x\":[1,2,3],\"y\":[\"hi\"]}\n",
    ),
    (
        ".y += [.x[] | with_dtf(\"2006\"; strenv(X))]",
        "{\"x\":[1,2,3],\"y\":[\"hi\"]}\n",
    ),
    (
        ".x |= (map(with_dtf(\"2006\"; now)) | length)",
        "{\"x\":1}\n",
    ),
    (
        ".x = ([.x[] | with_dtf(\"2006\"; now | tostring)] | length)",
        "{\"x\":1}\n",
    ),
    (
        ".y = [.x[] | with_dtf(\"2006\"; .)]",
        "{\"x\":[1,2,3],\"y\":[1,2,3]}\n",
    ),
    (
        ".y = [.x[] | with_dtf(\"2006\"; [strenv(X)])]",
        "{\"x\":[1,2,3],\"y\":[[\"hi\"],[\"hi\"],[\"hi\"]]}\n",
    ),
    (
        ".x[] |= with_dtf(\"2006\"; strenv(X))",
        "{\"x\":[\"hi\",\"hi\",\"hi\"]}\n",
    ),
];

fn run(args: &[&str], filter: &str, stdin: &str) -> Result<(String, String, i32)> {
    run_tool("yq", args, filter, stdin)
}

fn run_tool(tool: &str, args: &[&str], filter: &str, stdin: &str) -> Result<(String, String, i32)> {
    use std::io::Write;
    let mut child = Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .arg(tool)
        .args(args)
        .arg(filter)
        .env("TZ", "UTC")
        .env("X", "hi")
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
fn with_dtf_over_a_once_stage_is_answered_once_8028() -> Result<()> {
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
fn a_write_reads_with_dtf_over_a_once_stage_once_8028() -> Result<()> {
    for (filter, want) in WRITE_ROWS {
        let got = run(&["-o", "json", "-I0"], filter, XS)?;
        assert_eq!(
            (got.0.as_str(), got.1.as_str(), got.2),
            (*want, "", 0),
            "{filter}"
        );
    }
    Ok(())
}

/// `succinctly jq` keeps jq's per-value pipe for its `with_dtf` extension (jq 1.7.1 has no
/// `with_dtf`): the body runs once per input.
const PREFIX_INPUT: &str = "s: [A, B]\nm: [{a: 1}, {b: 2}]\nx: [1, 2, 3]\n";

/// The stages of a body before its once-stage run over every node of the context, not over a
/// stand-in: (filter, yq's `-o json -I0` stdout) over [`PREFIX_INPUT`].
const PREFIX_ROWS: &[(&str, &str)] = &[
    (
        "[.s[] | with_dtf(\"2006\"; test(\"A\") | now)] | length",
        "1\n",
    ),
    (
        "[.s[] | with_dtf(\"2006\"; downcase | now)] | length",
        "1\n",
    ),
    ("[.m[] | with_dtf(\"2006\"; keys | now)] | length", "1\n"),
    (
        "[.s[] | select(. == \"Z\") | with_dtf(\"2006\"; downcase | now)] | length",
        "1\n",
    ),
    (
        "[.x[] | with_dtf(\"2006\"; tostring | now | tostring)] | length",
        "1\n",
    ),
    (
        "[.s[] | with_dtf(\"2006\"; downcase | strenv(X))]",
        "[\"hi\"]\n",
    ),
    (
        ".s | map(with_dtf(\"2006\"; downcase | now)) | length",
        "1\n",
    ),
    (
        ".y = [.s[] | with_dtf(\"2006\"; downcase | strenv(X))] | .y",
        "[\"hi\"]\n",
    ),
    (
        "[.s[] | with_dtf(\"2006\"; test(\"A\") | with_dtf(\"2006\"; now))] | length",
        "1\n",
    ),
    (
        "[.s[] | with_dtf(\"2006\"; with_dtf(\"2006\"; test(\"A\") | now))] | length",
        "1\n",
    ),
    (
        "[.s[] | with_dtf(\"2006\"; downcase) | with_dtf(\"2006\"; now)] | length",
        "1\n",
    ),
];

/// So an error of theirs over a node surfaces, as yq's `cannot split !!int, can only split strings`.
const PREFIX_ERROR_ROWS: &[&str] = &[
    "[.x[] | with_dtf(\"2006\"; split(\"x\") | now)] | length",
    ".x | map(with_dtf(\"2006\"; split(\"x\") | now)) | length",
    ".y = [.x[] | with_dtf(\"2006\"; split(\"x\") | strenv(X))] | .y",
];

#[test]
fn stages_before_the_once_stage_read_every_node_8028() -> Result<()> {
    for (filter, want) in PREFIX_ROWS {
        let got = run(&["-o", "json", "-I0"], filter, PREFIX_INPUT)?;
        assert_eq!(
            (got.0.as_str(), got.1.as_str(), got.2),
            (*want, "", 0),
            "{filter}"
        );
    }
    for filter in PREFIX_ERROR_ROWS {
        let got = run(&["-o", "json", "-I0"], filter, PREFIX_INPUT)?;
        assert_eq!(
            (got.0.as_str(), got.1.as_str(), got.2),
            ("", "Error: cannot split !!int, can only split strings\n", 1),
            "{filter}"
        );
    }
    // yq words this one `!!int has no keys`; the error itself is the point here.
    let got = run(
        &["-o", "json", "-I0"],
        "[.x[] | with_dtf(\"2006\"; to_entries | now)] | length",
        PREFIX_INPUT,
    )?;
    assert_eq!((got.0.as_str(), got.2), ("", 1), "{}", got.1);
    assert!(got.1.starts_with("Error: "), "{}", got.1);
    Ok(())
}

#[test]
fn jq_mode_reads_with_dtf_per_value_8028() -> Result<()> {
    for (filter, want) in [
        ("[.[] | with_dtf(\"2006\"; now)] | length", "3\n"),
        ("[.[] | with_dtf(\"2006\"; strenv(X))] | length", "3\n"),
        ("map(with_dtf(\"2006\"; now)) | length", "3\n"),
        (
            "[.[] | select(false) | with_dtf(\"2006\"; now)] | length",
            "0\n",
        ),
    ] {
        let got = run_tool("jq", &["-c"], filter, "[1,2,3]")?;
        assert_eq!(
            (got.0.as_str(), got.1.as_str(), got.2),
            (want, "", 0),
            "{filter}"
        );
    }
    Ok(())
}
