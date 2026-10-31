//! #5635: `--stream` over a document with a syntax error part-way through prints the events jq's
//! streaming parser read before the fault, then fails. Every expected output was captured from
//! jq 1.7.1 (`/usr/bin/jq`); stderr wording differs between the tools and is not compared.

use anyhow::Result;

use crate::run_jq_full;

/// Each row is `(input, expected stdout, expected exit status)` of `jq <args>` over `input`.
fn check(args: &[&str], rows: &[(&str, &str, i32)]) -> Result<()> {
    for &(input, expected, status) in rows {
        let (stdout, _stderr, code) = run_jq_full(args, Some(input))?;
        assert_eq!(
            (stdout.as_str(), code),
            (expected, status),
            "{args:?} {input:?}"
        );
    }
    Ok(())
}

#[test]
fn test_events_before_a_mid_document_fault_are_printed_5635() -> Result<()> {
    check(
        &["--stream", "-c", "."],
        &[
            ("[1,2,3 x]", "[[0],1]\n[[1],2]\n", 5),
            ("{\"a\":1,\"b\":[2,@]}", "[[\"a\"],1]\n[[\"b\",0],2]\n", 5),
            ("{\"a\":1,\"b\",", "[[\"a\"],1]\n", 5),
            ("{\"x\":\"y\",{\"a\":\"b\"}}", "[[\"x\"],\"y\"]\n", 5),
            ("{\"x\":\"y\",[\"a\",\"b\"]}", "[[\"x\"],\"y\"]\n", 5),
            ("{{\"a\":\"b\"}}", "", 5),
            ("[1,2]\n[3,x", "[[0],1]\n[[1],2]\n[[1]]\n[[0],3]\n", 5),
            (
                "{\"a\":{\"b\":[1,{\"c\":2,\"d\" x}]}}",
                "[[\"a\",\"b\",0],1]\n[[\"a\",\"b\",1,\"c\"],2]\n",
                5,
            ),
            (
                "[[1,2],[3,\"a\\u00e9\",tru]]",
                "[[0,0],1]\n[[0,1],2]\n[[0,1]]\n[[1,0],3]\n[[1,1],\"aé\"]\n",
                5,
            ),
            ("[1,2,,3]", "[[0],1]\n[[1],2]\n", 5),
        ],
    )
}

#[test]
fn test_the_events_name_the_line_jq_had_read_5635() -> Result<()> {
    check(
        &["--stream", "-c", "[., input_line_number]"],
        &[
            (
                "1\n[1,\n2,\n3 x]\n",
                "[[[],1],1]\n[[[0],1],2]\n[[[1],2],3]\n",
                5,
            ),
            (
                "[1,2,\n3,4,\n5 x]\n",
                "[[[0],1],1]\n[[[1],2],1]\n[[[2],3],2]\n[[[3],4],2]\n",
                5,
            ),
            (
                "[1]\n{\"a\":1,\"b\":[2,@]}",
                "[[[0],1],1]\n[[[0]],1]\n[[[\"a\"],1],1]\n[[[\"b\",0],2],1]\n",
                5,
            ),
        ],
    )
}

#[test]
fn test_inputs_reads_the_events_before_the_fault_5635() -> Result<()> {
    check(
        &["--stream", "-nc", "[inputs]"],
        &[("[1,2,3 x]", "", 5), ("[1 x]\n[4 y]\n[5]", "", 5)],
    )
}

#[test]
fn test_input_stops_before_the_fault_when_the_program_does_5635() -> Result<()> {
    check(
        &["--stream", "-nc", "input"],
        &[("[1,2,3 x]", "[[0],1]\n", 0)],
    )
}

#[test]
fn test_a_fault_a_program_resumes_after_comes_after_its_events_5635() -> Result<()> {
    check(
        &[
            "--stream",
            "-nc",
            "[limit(6; repeat(try input catch \"E\"))]",
        ],
        &[
            (
                "[1 x]\n[4 y]\n[5]",
                "[\"E\",\"E\",[[0],5],[[0]],\"E\",\"E\"]\n",
                0,
            ),
            (
                "[1,2 x]\n{\"a\":[3,@]}\n[5]",
                "[[[0],1],\"E\",[[\"a\",0],3],\"E\",[[0],5],[[0]]]\n",
                0,
            ),
            (
                "[7]\n[8,9 x]\n[10]",
                "[[[0],7],[[0]],[[0],8],\"E\",[[0],10],[[0]]]\n",
                0,
            ),
        ],
    )
}
