//! #5548, moved by #7523: `jq -s --stream` took the closing event of a top-level array or object
//! whose last element is a scalar out of the slurped array when nothing followed that closer,
//! because jq 1.7.1's parser emitted it once its input was exhausted, after the array was finished.
//! jq 1.8.2 reads once more before it closes the array (jqlang/jq#3279), so the event is in the
//! array whatever the last byte is. Every expected output below was captured from jq 1.8.2
//! (`/opt/homebrew/bin/jq -c --stream -s`); the rows with a byte after the closer, and the closer
//! after a container, were never held back and did not move.

use anyhow::Result;

use crate::{run_jq_full, run_jq_over_files};

fn slurp_stream(input: &str) -> Result<String> {
    let (stdout, stderr, code) = run_jq_full(&["-c", "--stream", "-s", "."], Some(input))?;
    assert_eq!(code, 0, "{input:?}: {stderr:?}");
    Ok(stdout)
}

#[test]
fn test_a_closer_after_a_scalar_at_the_end_of_the_input_stays_in_the_array_5548_7523() -> Result<()>
{
    for (input, expected) in [
        ("[1,2]", "[[[0],1],[[1],2],[[1]]]\n"),
        ("[1]", "[[[0],1],[[0]]]\n"),
        (r#"{"a":1}"#, "[[[\"a\"],1],[[\"a\"]]]\n"),
        (r#"{"a":"x"}"#, "[[[\"a\"],\"x\"],[[\"a\"]]]\n"),
        ("[true]", "[[[0],true],[[0]]]\n"),
        ("[null]", "[[[0],null],[[0]]]\n"),
        ("[[],1]", "[[[0],[]],[[1],1],[[1]]]\n"),
        ("[\n1,\n2]", "[[[0],1],[[1],2],[[1]]]\n"),
        (
            "[1,2]\n[3,4]",
            "[[[0],1],[[1],2],[[1]],[[0],3],[[1],4],[[1]]]\n",
        ),
        ("[1,2][3]", "[[[0],1],[[1],2],[[1]],[[0],3],[[0]]]\n"),
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
            "[[[0],1],[[1],2],[[1]],[[0],3],[[0]]]\n",
        ),
        (
            &["[1,2]", "[3]\n"],
            "[[[0],1],[[1],2],[[1]],[[0],3],[[0]]]\n",
        ),
        (&["[1,2]", ""], "[[[0],1],[[1],2],[[1]]]\n"),
        (&["", "[1,2]"], "[[[0],1],[[1],2],[[1]]]\n"),
        // One document across the files' seam.
        (&["[1,", "2]"], "[[[0],1],[[1],2],[[1]]]\n"),
    ] {
        let (stdout, stderr, code, _) = run_jq_over_files(&["-c", "--stream", "-s", "."], files)?;
        assert_eq!(code, 0, "{files:?}: {stderr:?}");
        assert_eq!(stdout, expected, "{files:?}");
    }
    Ok(())
}

#[test]
fn test_the_closer_is_in_the_one_array_the_program_reads_5548() -> Result<()> {
    // The program runs once, for the one array that has the closer in it.
    let (stdout, _, code) = run_jq_full(&["-c", "--stream", "-s", "length"], Some("[1,2]"))?;
    assert_eq!((stdout.as_str(), code), ("3\n", 0));
    let (stdout, _, code) = run_jq_full(
        &["-c", "--stream", "-s", "[., input_line_number]"],
        Some("[1,\n2]"),
    )?;
    assert_eq!(
        (stdout.as_str(), code),
        ("[[[[0],1],[[1],2],[[1]]],1]\n", 0)
    );
    // ... and `input_filename` is the last file's name, for the one array that holds all of it.
    let (stdout, _, code, paths) = run_jq_over_files(
        &["-c", "--stream", "-s", "[input_filename, length]"],
        &["[1,2]", "[3]"],
    )?;
    assert_eq!(stdout, format!("[{:?},5]\n", paths[1]), "{code}");
    // ... and an error on it names the file.
    let (_, stderr, _, paths) =
        run_jq_over_files(&["-c", "--stream", "-s", r#"error("boom")"#], &["[1,2]"])?;
    assert_eq!(stderr, format!("jq: error (at {}:0): boom\n", paths[0]));
    // Without `-s` the events are the same stream either way.
    let (stdout, _, code) = run_jq_full(&["-c", "--stream", "."], Some("[1,2]"))?;
    assert_eq!((stdout.as_str(), code), ("[[0],1]\n[[1],2]\n[[1]]\n", 0));
    Ok(())
}
