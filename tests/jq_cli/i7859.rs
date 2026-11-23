//! #7859: a builtin argument, a loop operand or a construction slot that reads input (`inputs`,
//! `input`) ran to its end before the value it feeds was used, where jq runs each value through
//! before it asks for the next: the flags of `split`/`splits`, the count of `combinations(n)`, the
//! `xs` of `in(xs)`, the body of `map_values`, `walk`, `repeat`, `while` and `until`, the operand of
//! a unary minus, the condition of `if` and the keys and values of an object construction. An error
//! raised on one value then named `<unknown>` and left the main loop nothing to read, and a
//! consumer that stops (`first`, `limit`) or a later read of `input_line_number` found inputs gone.
//! Every expected string was captured from `/usr/bin/jq` 1.7.1; jq 1.8.2 gives the same, apart from
//! the operand order of `split`'s `"g" + flags` (its 1.8 definition is `$flags + "g"`, a refactor)
//! and the location it gives a `break` raised at the end of the input.

use anyhow::Result;

use crate::run_jq_in_dir;

/// (arguments, standard input, stdout, stderr, exit status).
type Row = (
    &'static [&'static str],
    Option<&'static str>,
    &'static str,
    &'static str,
    i32,
);

fn check(rows: &[Row]) -> Result<()> {
    let dir = tempfile::tempdir()?;
    std::fs::write(dir.path().join("in.json"), "1 {\"z\":[]} 2 1")?;
    std::fs::write(dir.path().join("in.ndjson"), "1\n{\"z\":[]}\n2\n1\n")?;
    for &(args, input, stdout, stderr, code) in rows {
        let (out, err, status) = run_jq_in_dir(dir.path(), args, input)?;
        assert_eq!(
            (out.as_str(), err.as_str(), status),
            (stdout, stderr, code),
            "{args:?} over {input:?}"
        );
    }
    Ok(())
}

/// The issue's six rows: each reads inputs only up to the value it raises on or answers with.
#[test]
fn an_inputs_argument_of_a_builtin_is_read_one_value_at_a_time_7859() -> Result<()> {
    check(&[
        (
            &[r"-c", r#""aAbB" | [split("a"; inputs)]"#],
            Some("\"\" {\"z\":[]} \"i\" \"\""),
            "[[\"\",\"AbB\"]]\n",
            "jq: error (at <stdin>:0): string (\"g\") and object ({\"z\":[]}) cannot be added\n",
            0,
        ),
        (
            &[r"-c", r#""aAbB" | [splits("a"; inputs)]"#],
            Some("\"\" {\"z\":[]} \"i\" \"\""),
            "[\"\",\"AbB\"]\n",
            "jq: error (at <stdin>:0): string (\"g\") and object ({\"z\":[]}) cannot be added\n",
            0,
        ),
        (
            &[r"-c", r"[[1,2],[3]] | [combinations(inputs)]"],
            Some("1 {\"z\":[]} 2 1"),
            "[[[1,2]],[[3]]]\n",
            "jq: error (at <stdin>:0): Range bounds must be numeric\n",
            0,
        ),
        (
            &[r"-c", r#"first("a" | in(inputs)), input_line_number"#],
            Some("{\"a\":1}\n{\"z\":[]}\n{\"b\":2}\n{\"a\":1}\n"),
            "false\n2\ntrue\n4\n",
            "",
            0,
        ),
        (
            &[r"-c", r#"{"a":1} | map_values(inputs)"#],
            Some("1 {\"z\":[]} 2 1"),
            "{\"a\":{\"z\":[]}}\n{\"a\":1}\n",
            "",
            0,
        ),
        (
            &[
                r"-nc",
                r"first([1,[2]] | walk((1, input))), input_line_number",
            ],
            Some("1\n{\"z\":[]}\n2\n1\n"),
            "1\n3\n",
            "",
            0,
        ),
    ])
}

/// The same over newline-separated input, a file of either layout and `-n`, and with a consumer
/// that stops (`first`, `limit`).
#[test]
fn an_inputs_argument_is_read_one_value_at_a_time_on_every_route_7859() -> Result<()> {
    check(&[
        (&[r"-c", r#""aAbB" | [split("a"; inputs)]"#], Some("\"\"\n{\"z\":[]}\n\"i\"\n\"\"\n"), "[[\"\",\"AbB\"]]\n", "jq: error (at <stdin>:2): string (\"g\") and object ({\"z\":[]}) cannot be added\n", 0),
        (&[r"-c", r"[[1,2],[3]] | [combinations(inputs)]", r"in.ndjson"], None, "[[[1,2]],[[3]]]\n", "jq: error (at in.ndjson:2): Range bounds must be numeric\n", 0),
        (&[r"-c", r"[[1,2],[3]] | [combinations(inputs)]", r"in.json"], None, "[[[1,2]],[[3]]]\n", "jq: error (at in.json:0): Range bounds must be numeric\n", 0),
        (&[r"-nc", r"input | [[1,2],[3]] | [combinations(inputs)]"], Some("1\n{\"z\":[]}\n2\n1\n"), "", "jq: error (at <stdin>:2): Range bounds must be numeric\n", 5),
        (&[r"-c", r"[1,2] | map_values(inputs)"], Some("1 {\"z\":[]} 2 1"), "[{\"z\":[]},2]\n[]\n", "", 0),
        (&[r"-c", r#"{"a":1} | map_values(inputs)"#, r"in.ndjson"], None, "{\"a\":{\"z\":[]}}\n{\"a\":1}\n", "", 0),
        (&[r"-nc", r#"{"a":1,"b":{"c":2}} | walk((., input)), input_line_number"#], Some("1\n2\n3\n4\n5\n"), "{\"a\":1,\"b\":{\"c\":2}}\n1\n1\n", "", 0),
        (&[r"-c", r"first(1 | in(inputs)), input_line_number", r"in.ndjson"], None, "", "jq: error (at in.ndjson:2): Cannot check whether object has a number key\njq: error (at in.ndjson:4): Cannot check whether number has a number key\n", 5),
        (&[r"-c", r#"[limit(2; "a" | in(inputs))], input_line_number"#], Some("{\"a\":1}\n{\"b\":2}\n{\"a\":3}\n{\"a\":4}\n"), "[false,true]\n3\n[]\n4\n", "", 0),
    ])
}

/// Effects other than reading input: a `debug` after the first output of `map_values`' or a
/// member's `walk` body never runs, and one in the flags or count runs after the value before it
/// has been checked.
#[test]
fn a_debug_in_a_builtin_argument_runs_where_jq_runs_it_7859() -> Result<()> {
    check(&[
        (
            &[r"-nc", r#"{"a":1} | map_values((., ("x" | debug)))"#],
            None,
            "{\"a\":1}\n",
            "",
            0,
        ),
        (
            &[r"-nc", r#"{"a":1} | walk((., ("x" | debug)))"#],
            None,
            "{\"a\":1}\n\"x\"\n",
            "[\"DEBUG:\",\"x\"]\n",
            0,
        ),
        (
            &[r"-nc", r#"[first([1] | walk((., ("y" | debug))))]"#],
            None,
            "[[1,\"y\"]]\n",
            "[\"DEBUG:\",\"y\"]\n",
            0,
        ),
        (
            &[r"-nc", r#"[first("a" | in({"a":1}, ("y" | debug)))]"#],
            None,
            "[true]\n",
            "",
            0,
        ),
        (
            &[
                r"-nc",
                r#"[[1,2],[3]] | [combinations(("x", ("y" | debug)))]"#,
            ],
            None,
            "",
            "jq: error (at <unknown>): Range bounds must be numeric\n",
            5,
        ),
        (
            &[r"-nc", r#""aAbB" | [split("a"; (1, ("y" | debug)))]"#],
            None,
            "",
            "jq: error (at <unknown>): string (\"g\") and number (1) cannot be added\n",
            5,
        ),
        (
            &[r"-nc", r#""aAbB" | [splits("a"; ("", ("y" | debug)))]"#],
            None,
            "",
            "[\"DEBUG:\",\"y\"]\njq: error (at <unknown>): gy is not a valid modifier string\n",
            5,
        ),
    ])
}

/// `repeat`'s body, a unary minus's operand and `while`'s and `until`'s condition and update.
#[test]
fn a_loop_body_or_operand_that_reads_input_is_read_one_value_at_a_time_7859() -> Result<()> {
    check(&[
        (
            &[r"-c", r"[limit(3; repeat((1, input)))]"],
            Some("1 2 3 4 5 6 7"),
            "[1,2,1]\n[1,4,1]\n[1,6,1]\n",
            "jq: error (at <unknown>): break\n",
            5,
        ),
        (
            &[r"-c", r"first(repeat((1, input))), input_line_number"],
            Some("1\n2\n3\n"),
            "1\n1\n1\n2\n1\n3\n",
            "",
            0,
        ),
        (
            &[r"-c", r"[null | [-(inputs)]]"],
            Some("1 {\"z\":[]} 2 1"),
            "[[-1]]\n",
            "jq: error (at <stdin>:0): object ({\"z\":[]}) cannot be negated\n",
            0,
        ),
        (
            &[r"-c", r"[-(inputs)], input_line_number"],
            Some("1\n{\"z\":[]}\n2\n"),
            "[]\n3\n",
            "jq: error (at <stdin>:2): object ({\"z\":[]}) cannot be negated\n",
            0,
        ),
        (
            &[r"-c", r"--stream", r"[0 | [while(inputs; . + 1)]]"],
            Some("false\n{\"z\":[]}\nfalse\nfalse\n"),
            "[[0,1,2,3]]\n",
            "",
            0,
        ),
        (
            &[r"-c", r"--stream", r"[0 | [until(inputs | not; . + 1)]]"],
            Some("false\n{\"z\":[]}\nfalse\nfalse\n"),
            "[[]]\n",
            "",
            0,
        ),
        (
            &[
                r"-nc",
                r"[limit(3; 0 | while(true; (. + 1, input)))], input_line_number",
            ],
            Some("10\n20\n30\n40\n"),
            "[0,1,2]\n0\n",
            "",
            0,
        ),
    ])
}

/// The condition of an `if` and the keys and values of an object construction.
#[test]
fn an_if_condition_or_object_slot_that_reads_input_is_read_one_value_at_a_time_7859() -> Result<()>
{
    check(&[
        (
            &[r"-c", r"[if inputs then input_line_number else -1 end]"],
            Some("1\n2\n[3]\n4\n5\n"),
            "[2,3,4,5]\n",
            "",
            0,
        ),
        (
            &[r"-c", r"[if inputs then input else -1 end]"],
            Some("1\n2\n[3]\n4\n5\n"),
            "[[3],5]\n",
            "",
            0,
        ),
        (
            &[r"-c", r"[{a: inputs, b: input_line_number}]"],
            Some("1\n2\n[3]\n4\n5\n"),
            "[{\"a\":2,\"b\":2},{\"a\":[3],\"b\":3},{\"a\":4,\"b\":4},{\"a\":5,\"b\":5}]\n",
            "",
            0,
        ),
        (
            &[r"-c", r"[{(inputs | tostring): input_line_number}]"],
            Some("1\n2\n[3]\n4\n5\n"),
            "[{\"2\":2},{\"[3]\":3},{\"4\":4},{\"5\":5}]\n",
            "",
            0,
        ),
        (
            &[r"-c", r"first({a: inputs}), input_line_number"],
            Some("1\n2\n3\n"),
            "{\"a\":2}\n2\n3\n",
            "",
            0,
        ),
        (
            &[
                r"-c",
                r"first(if inputs then 1 else 0 end), input_line_number",
            ],
            Some("1\n2\n3\n"),
            "1\n2\n3\n",
            "",
            0,
        ),
    ])
}

/// A `debug` over a document (the cursor evaluator's route): an object slot's, a `walk` or `in`
/// argument's after a consumer stops, and `map_values`' after the first output.
#[test]
fn a_debug_argument_over_a_document_runs_where_jq_runs_it_7859() -> Result<()> {
    check(&[
        (
            &[r"-c", r#"[{a: (1, ("x" | debug)), b: ("y" | debug)}]"#],
            Some("[1]"),
            "[{\"a\":1,\"b\":\"y\"},{\"a\":\"x\",\"b\":\"y\"}]\n",
            "[\"DEBUG:\",\"y\"]\n[\"DEBUG:\",\"x\"]\n[\"DEBUG:\",\"y\"]\n",
            0,
        ),
        (
            &[r"-c", r#"[first(walk((., ("y" | debug))))]"#],
            Some("[1]"),
            "[[1,\"y\"]]\n",
            "[\"DEBUG:\",\"y\"]\n",
            0,
        ),
        (
            &[r"-c", r#"[first(walk((., ("y" | debug))))]"#],
            Some("{\"a\":1}"),
            "[{\"a\":1}]\n",
            "",
            0,
        ),
        (
            &[r"-c", r#"[first("a" | in(({"a":1}, ("y" | debug))))]"#],
            Some("{\"a\":1}"),
            "[true]\n",
            "",
            0,
        ),
        (
            &[r"-c", r#"map_values((., ("y" | debug)))"#],
            Some("{\"a\":1}"),
            "{\"a\":1}\n",
            "",
            0,
        ),
    ])
}
