//! #7523: `jq -s --stream` over input whose last byte closes a container keeps the closing event in
//! the slurped array. jq 1.7.1's slurp loop ends at the call that reads the last byte, so the event
//! its streaming parser still held came out after the array, as a value of its own (`printf '[1][2]'
//! | jq -c -s --stream .` printed the array and then `[[0]]`, and #5548 modelled that). jq 1.8.2 reads
//! once more before it closes the array (jqlang/jq#3279, listed in the 1.8 NEWS under CLI changes:
//! "Fix `--slurp --stream` when input has no trailing newline character"), so the array has every
//! event. Every row was captured from jq 1.8.2 (`/opt/homebrew/bin/jq`); #5548's own rows, moved
//! by this change, are in `i5548.rs`.

use anyhow::Result;

use crate::run_jq_full;

/// (input, expected stdout of `jq -c -s --stream .`).
const SLURPED: &[(&str, &str)] = &[
    ("[1][2]", "[[[0],1],[[0]],[[0],2],[[0]]]\n"),
    ("[1][2]\n", "[[[0],1],[[0]],[[0],2],[[0]]]\n"),
    ("{\"a\":1}", "[[[\"a\"],1],[[\"a\"]]]\n"),
    ("[1,2]", "[[[0],1],[[1],2],[[1]]]\n"),
    ("[1]", "[[[0],1],[[0]]]\n"),
    ("[1,2]\n", "[[[0],1],[[1],2],[[1]]]\n"),
    ("[1,[2]]", "[[[0],1],[[1,0],2],[[1,0]],[[1]]]\n"),
    ("1 2", "[[[],1],[[],2]]\n"),
    ("[]", "[[[],[]]]\n"),
    ("{}", "[[[],{}]]\n"),
    ("[1,2] ", "[[[0],1],[[1],2],[[1]]]\n"),
    (
        "[1,2]\n[3,4]",
        "[[[0],1],[[1],2],[[1]],[[0],3],[[1],4],[[1]]]\n",
    ),
    ("[1,2][3]", "[[[0],1],[[1],2],[[1]],[[0],3],[[0]]]\n"),
    ("[[1]]", "[[[0,0],1],[[0,0]],[[0]]]\n"),
    ("[true]", "[[[0],true],[[0]]]\n"),
    (
        "[1,{\"a\":2}]",
        "[[[0],1],[[1,\"a\"],2],[[1,\"a\"]],[[1]]]\n",
    ),
    ("\"x\"", "[[[],\"x\"]]\n"),
    (
        "[1,2]\n[3,4]\n",
        "[[[0],1],[[1],2],[[1]],[[0],3],[[1],4],[[1]]]\n",
    ),
    ("{\"a\":\"x\"}", "[[[\"a\"],\"x\"],[[\"a\"]]]\n"),
];

#[test]
fn the_last_closing_event_is_in_the_slurped_array_7523() -> Result<()> {
    for (input, expected) in SLURPED {
        let (stdout, stderr, code) = run_jq_full(&["-c", "-s", "--stream", "."], Some(input))?;
        assert_eq!(
            (stdout.as_str(), code),
            (*expected, 0),
            "{input:?}: {stderr}"
        );
    }
    Ok(())
}

/// (flags, program, input, expected stdout): the program sees one input, the array, and under `-n`
/// the array is the only thing `input` and `inputs` have.
const PROGRAMS: &[(&[&str], &str, &str, &str)] = &[
    (&["-c", "-s", "--stream"], "length", "[1,2]", "3\n"),
    (
        &["-c", "-s", "--stream"],
        "[., input_line_number]",
        "[1,\n2]",
        "[[[[0],1],[[1],2],[[1]]],1]\n",
    ),
    (
        &["-c", "-n", "-s", "--stream"],
        "[inputs]",
        "[1,2]",
        "[[[[0],1],[[1],2],[[1]]]]\n",
    ),
    (
        &["-c", "-n", "-s", "--stream"],
        "[inputs]",
        "[1][2]",
        "[[[[0],1],[[0]],[[0],2],[[0]]]]\n",
    ),
    (
        &["-c", "-n", "-s", "--stream"],
        "input",
        "[1,2]",
        "[[[0],1],[[1],2],[[1]]]\n",
    ),
];

#[test]
fn the_program_reads_one_array_with_the_last_closing_event_7523() -> Result<()> {
    for (flags, program, input, expected) in PROGRAMS {
        let mut args = flags.to_vec();
        args.push(program);
        let (stdout, stderr, code) = run_jq_full(&args, Some(input))?;
        assert_eq!(
            (stdout.as_str(), code),
            (*expected, 0),
            "{flags:?} {program} on {input:?}: {stderr}"
        );
    }
    Ok(())
}
