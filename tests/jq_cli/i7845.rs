//! #7845: an `as` whose source has an effect (`inputs`, `input`) ran the source to its end before the
//! body ran once, when the binding was collected (`[inputs as $v | B]`). An error the body raised on
//! the first value then came after every input had been read: it named `<unknown>` and left the main
//! loop nothing, so the run stopped there with exit 5. jq runs the body on each value as it is read.
//! #7474 binds a native builtin's generator argument the same way (`join(inputs)` is `inputs as $v |
//! join($v)`), which is how `[.[] | join(inputs)]` and its siblings reached it. Every expected string
//! was captured from `/usr/bin/jq` 1.7.1; jq 1.8.2 gives the same, apart from the value preview it
//! adds to `Cannot index` errors and its untruncated previews (an improvement, so 1.7.1 stays the
//! reference).

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
    std::fs::write(dir.path().join("in.json"), "[\"a\"] [\"b\"] [\"c\"]")?;
    std::fs::write(dir.path().join("in.ndjson"), "[\"a\"]\n[\"b\"]\n[\"c\"]\n")?;
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

/// Each builtin #7474 binds, with `inputs` as its argument and an argument value it raises on.
#[test]
fn a_builtin_raising_on_an_inputs_argument_reads_one_input_7845() -> Result<()> {
    check(&[
        (&[r"-c", r"[.[] | join(inputs)]"], Some("[\"a\"] [\"b\"] [\"c\"]"), "[]\n", "jq: error (at <stdin>:0): Cannot iterate over string (\"a\")\n", 0),
        (&[r"-c", r"[.[] | has(inputs)]"], Some("[\"a\"] [\"b\"] [\"c\"]"), "[]\n", "jq: error (at <stdin>:0): Cannot check whether string has a array key\n", 0),
        (&[r"-c", r"[.[] | contains(inputs)]"], Some("[\"a\"] [\"b\"] [\"c\"]"), "[]\n", "jq: error (at <stdin>:0): string (\"a\") and array ([\"b\"]) cannot have their containment checked\n", 0),
        (&[r"-c", r"[.[] | inside(inputs)]"], Some("[\"a\"] [\"b\"] [\"c\"]"), "[]\n", "jq: error (at <stdin>:0): array ([\"b\"]) and string (\"a\") cannot have their containment checked\n", 0),
        (&[r"-c", r"[.[] | split(inputs)]"], Some("[\"a\"] [\"b\"] [\"c\"]"), "[]\n", "jq: error (at <stdin>:0): split input and separator must be strings\n", 0),
        (&[r"-c", r"[.[] | splits(inputs)]"], Some("[\"a\"] [\"b\"] [\"c\"]"), "[]\n", "jq: error (at <stdin>:0): array ([\"b\"]) is not a string\n", 0),
        (&[r"-c", r"[.[] | startswith(inputs)]"], Some("[\"a\"] [\"b\"] [\"c\"]"), "[]\n", "jq: error (at <stdin>:0): startswith() requires string inputs\n", 0),
        (&[r"-c", r"[.[] | endswith(inputs)]"], Some("[\"a\"] [\"b\"] [\"c\"]"), "[]\n", "jq: error (at <stdin>:0): endswith() requires string inputs\n", 0),
        (&[r"-c", r"[.[] | index(inputs)]"], Some("[\"a\"] [\"b\"] [\"c\"]"), "[]\n", "jq: error (at <stdin>:0): Cannot index string with array\n", 0),
        (&[r"-c", r"[.[] | indices(inputs)]"], Some("[\"a\"] [\"b\"] [\"c\"]"), "[]\n", "jq: error (at <stdin>:0): Cannot index string with array\n", 0),
        (&[r"-c", r"[.[] | rindex(inputs)]"], Some("[\"a\"] [\"b\"] [\"c\"]"), "[]\n", "jq: error (at <stdin>:0): Cannot index string with array\n", 0),
        (&[r"-c", r"[.[] | getpath(inputs)]"], Some("[\"a\"] [\"b\"] [\"c\"]"), "[]\n", "jq: error (at <stdin>:0): Cannot index string with string \"b\"\n", 0),
        (&[r"-c", r"[.[] | flatten(inputs)]"], Some("[\"a\"] [\"b\"] [\"c\"]"), "[]\n", "jq: error (at <stdin>:0): Cannot iterate over string (\"a\")\n", 0),
        (&[r"-c", r"[.[] | scan(inputs)]"], Some("[\"a\"] [\"b\"] [\"c\"]"), "[]\n", "jq: error (at <stdin>:0): array ([\"b\"]) is not a string\n", 0),
        (&[r"-c", r#"[.[] | sub(inputs; "x")]"#], Some("[\"a\"] [\"b\"] [\"c\"]"), "[]\n", "jq: error (at <stdin>:0): array ([\"b\"]) is not a string\n", 0),
    ])
}

/// The same run over newline-separated input, a file, `-n` and `--stream`.
#[test]
fn an_inputs_argument_reads_one_input_on_every_route_7845() -> Result<()> {
    check(&[
        (&[r"-c", r"[.[] | join(inputs)]"], Some("[\"a\"]\n[\"b\"]\n[\"c\"]\n"), "[]\n", "jq: error (at <stdin>:2): Cannot iterate over string (\"a\")\n", 0),
        (&[r"-c", r"[.[] | join(inputs)]", r"in.json"], None, "[]\n", "jq: error (at in.json:0): Cannot iterate over string (\"a\")\n", 0),
        (&[r"-c", r"[.[] | join(inputs)]", r"in.ndjson"], None, "[]\n", "jq: error (at in.ndjson:2): Cannot iterate over string (\"a\")\n", 0),
        (&[r"-c", r"[.[] | join(inputs)], input_line_number"], Some("[\"a\"]\n[\"b\"]\n[\"c\"]\n[\"d\"]\n"), "", "jq: error (at <stdin>:2): Cannot iterate over string (\"a\")\njq: error (at <stdin>:4): Cannot iterate over string (\"c\")\n", 5),
        (&[r"-nc", r"input | [.[] | join(inputs)]"], Some("[\"a\"]\n[\"b\"]\n[\"c\"]\n"), "", "jq: error (at <stdin>:2): Cannot iterate over string (\"a\")\n", 5),
        (&[r"-c", r"--stream", r"[.[] | join(inputs)]"], Some("{\"a\":{\"b\":[{\"c\":1},{\"c\":2}]}}"), "[]\n", "jq: error (at <stdin>:0): string (\"a\") and array ([[\"a\",\"b\",0...) cannot be added\njq: error (at <stdin>:0): string (\"a\") and array ([[\"a\",\"b\",1...) cannot be added\njq: error (at <stdin>:0): string (\"a\") and array ([[\"a\",\"b\"]]) cannot be added\n", 0),
    ])
}

/// The cause, written out: a collected `as` with an effect in its source, bare and destructuring.
#[test]
fn a_collected_bind_runs_its_body_on_each_input_as_it_is_read_7845() -> Result<()> {
    check(&[
        (&[r"-c", r#"[inputs as $v | error("x")]"#], Some("[\"a\"] [\"b\"] [\"c\"]"), "[]\n", "jq: error (at <stdin>:0): x\n", 0),
        (&[r"-c", r#"[inputs as [$a] | error("x")]"#], Some("[\"a\"] [\"b\"] [\"c\"]"), "[]\n", "jq: error (at <stdin>:0): x\n", 0),
        (&[r"-c", r#"[(1, input) as $v | error("x")]"#], Some("[\"a\"] [\"b\"] [\"c\"]"), "", "jq: error (at <stdin>:0): x\njq: error (at <stdin>:0): x\njq: error (at <stdin>:0): x\n", 5),
        (&[r"-c", r#"{a: [inputs as $v | error("x")]}"#], Some("[\"a\"] [\"b\"] [\"c\"]"), "{\"a\":[]}\n", "jq: error (at <stdin>:0): x\n", 0),
        (&[r"-c", r"[inputs as $v | $v[0]], input_line_number"], Some("[\"a\"]\n[\"b\"]\n[\"c\"]\n"), "[\"b\",\"c\"]\n3\n", "", 0),
        (&[r"-c", r#"[inputs as $v | error("x")]"#, r"in.ndjson"], None, "[]\n", "jq: error (at in.ndjson:2): x\n", 0),
        (&[r"-c", r"[inputs as $v | {}[$v]]"], Some("[\"a\"] [\"b\"] [\"c\"]"), "[]\n", "jq: error (at <stdin>:0): Cannot index object with array\n", 0),
    ])
}
