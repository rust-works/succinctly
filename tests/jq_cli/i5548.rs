//! #5548: `jq -s --stream` takes the closing event of a top-level array or object whose last
//! element is a scalar out of the slurped array when nothing follows that closer. jq's parser
//! emits it once its input is exhausted, after the array is finished, so it is a value of its own.
//! Every expected output was captured from jq 1.7.1 (`/usr/bin/jq -c --stream -s`).

use anyhow::Result;

use crate::{run_jq_full, run_jq_over_files};

fn slurp_stream(input: &str) -> Result<String> {
    let (stdout, stderr, code) = run_jq_full(&["-c", "--stream", "-s", "."], Some(input))?;
    assert_eq!(code, 0, "{input:?}: {stderr:?}");
    Ok(stdout)
}

#[test]
fn test_a_closer_after_a_scalar_at_the_end_of_the_input_is_its_own_value_5548() -> Result<()> {
    for (input, expected) in [
        ("[1,2]", "[[[0],1],[[1],2]]\n[[1]]\n"),
        ("[1]", "[[[0],1]]\n[[0]]\n"),
        (r#"{"a":1}"#, "[[[\"a\"],1]]\n[[\"a\"]]\n"),
        (r#"{"a":"x"}"#, "[[[\"a\"],\"x\"]]\n[[\"a\"]]\n"),
        ("[true]", "[[[0],true]]\n[[0]]\n"),
        ("[null]", "[[[0],null]]\n[[0]]\n"),
        ("[[],1]", "[[[0],[]],[[1],1]]\n[[1]]\n"),
        ("[\n1,\n2]", "[[[0],1],[[1],2]]\n[[1]]\n"),
        (
            "[1,2]\n[3,4]",
            "[[[0],1],[[1],2],[[1]],[[0],3],[[1],4]]\n[[1]]\n",
        ),
        ("[1,2][3]", "[[[0],1],[[1],2],[[1]],[[0],3]]\n[[0]]\n"),
    ] {
        assert_eq!(slurp_stream(input)?, expected, "{input:?}");
    }
    Ok(())
}

#[test]
fn test_any_byte_after_the_closer_keeps_it_in_the_array_5548() -> Result<()> {
    for (input, expected) in [
        ("[1,2]\n", "[[[0],1],[[1],2],[[1]]]\n"),
        ("[1,2] ", "[[[0],1],[[1],2],[[1]]]\n"),
        ("[1,2]\t", "[[[0],1],[[1],2],[[1]]]\n"),
        ("[1,2]\r\n", "[[[0],1],[[1],2],[[1]]]\n"),
        (
            "[1,2]\n[3,4]\n",
            "[[[0],1],[[1],2],[[1]],[[0],3],[[1],4],[[1]]]\n",
        ),
    ] {
        assert_eq!(slurp_stream(input)?, expected, "{input:?}");
    }
    Ok(())
}

#[test]
fn test_a_closer_after_a_container_stays_in_the_array_5548() -> Result<()> {
    for (input, expected) in [
        ("[1,[2]]", "[[[0],1],[[1,0],2],[[1,0]],[[1]]]\n"),
        ("[1,[]]", "[[[0],1],[[1],[]],[[1]]]\n"),
        (
            r#"[1,{"a":2}]"#,
            "[[[0],1],[[1,\"a\"],2],[[1,\"a\"]],[[1]]]\n",
        ),
        (
            r#"{"a":[1,2]}"#,
            "[[[\"a\",0],1],[[\"a\",1],2],[[\"a\",1]],[[\"a\"]]]\n",
        ),
        ("[[1]]", "[[[0,0],1],[[0,0]],[[0]]]\n"),
        ("[]", "[[[],[]]]\n"),
        ("{}", "[[[],{}]]\n"),
        ("1 2", "[[[],1],[[],2]]\n"),
    ] {
        assert_eq!(slurp_stream(input)?, expected, "{input:?}");
    }
    Ok(())
}

#[test]
fn test_only_the_last_file_decides_5548() -> Result<()> {
    for (files, expected) in [
        (
            &["[1,2]", "[3]"][..],
            "[[[0],1],[[1],2],[[1]],[[0],3]]\n[[0]]\n",
        ),
        (
            &["[1,2]", "[3]\n"],
            "[[[0],1],[[1],2],[[1]],[[0],3],[[0]]]\n",
        ),
        (&["[1,2]", ""], "[[[0],1],[[1],2],[[1]]]\n"),
        (&["", "[1,2]"], "[[[0],1],[[1],2]]\n[[1]]\n"),
        // One document across the files' seam.
        (&["[1,", "2]"], "[[[0],1],[[1],2]]\n[[1]]\n"),
    ] {
        let (stdout, stderr, code, _) = run_jq_over_files(&["-c", "--stream", "-s", "."], files)?;
        assert_eq!(code, 0, "{files:?}: {stderr:?}");
        assert_eq!(stdout, expected, "{files:?}");
    }
    Ok(())
}

#[test]
fn test_the_held_back_closer_is_an_input_to_the_program_5548() -> Result<()> {
    // The program runs once for the slurped array and once for the closer, which reads as line 0.
    let (stdout, _, code) = run_jq_full(&["-c", "--stream", "-s", "length"], Some("[1,2]"))?;
    assert_eq!((stdout.as_str(), code), ("2\n1\n", 0));
    let (stdout, _, code) = run_jq_full(
        &["-c", "--stream", "-s", "[., input_line_number]"],
        Some("[1,\n2]"),
    )?;
    assert_eq!(
        (stdout.as_str(), code),
        ("[[[[0],1],[[1],2]],1]\n[[[1]],0]\n", 0)
    );
    // Without `-s` the events are the same stream either way.
    let (stdout, _, code) = run_jq_full(&["-c", "--stream", "."], Some("[1,2]"))?;
    assert_eq!((stdout.as_str(), code), ("[[0],1]\n[[1],2]\n[[1]]\n", 0));
    Ok(())
}
