//! #7807: `-s` over a text whose last byte completes a value while the parser still holds an
//! unfinished one (`2"`, `1[`, and under `--stream` also `[1,`). jq 1.7.1's slurp loop closed its
//! array at the call that read that byte, so it printed the array and then raised the fault the
//! next call met at the end of the input. jq 1.8.2 reads the empty closing refill before it closes
//! the array (jqlang/jq#3279, commit 4e3088f, the same change #7523 adopted for the closing event of
//! `--stream -s`), so the fault is met first and only the error is printed. `succinctly jq` follows
//! 1.8.2 on every route that slurps: before this, plain `-s` followed 1.8.2 for `2"` and 1.7.1 for
//! `2[`, and `--stream -s` followed 1.7.1 for both.
//!
//! Every row was captured from jq 1.8.2 (`/opt/homebrew/bin/jq`); where jq 1.7.1 (`/usr/bin/jq`)
//! answers differently, its answer is in the comment above the row.

use anyhow::Result;

use crate::run_jq_full;

/// (flags, input, expected stdout, expected stderr, expected exit), the program `.`.
const SLURPED: &[(&[&str], &str, &str, &str, i32)] = &[
    // 1.7.1: `[2]`, then the error.
    (
        &["-c", "-s"],
        "2\"",
        "",
        "jq: parse error: Unfinished string at EOF at line 1, column 2\n",
        5,
    ),
    // 1.7.1: `[[[],2]]`, then the error.
    (
        &["-c", "-s", "--stream"],
        "2\"",
        "",
        "jq: parse error: Unfinished string at EOF at line 1, column 2\n",
        5,
    ),
    // 1.7.1: `[[[],2]]`, then the fault event as an input of its own.
    (
        &["-c", "-s", "--stream-errors"],
        "2\"",
        "[[[],2],[\"Unfinished string at EOF at line 1, column 2\",[]]]\n",
        "",
        0,
    ),
    // 1.7.1: `[1,2]`, then the error.
    (
        &["-c", "-s"],
        "1 2\"",
        "",
        "jq: parse error: Unfinished string at EOF at line 1, column 4\n",
        5,
    ),
    // 1.7.1: `[[[],1],[[],2]]`, then the error.
    (
        &["-c", "-s", "--stream"],
        "1 2\"",
        "",
        "jq: parse error: Unfinished string at EOF at line 1, column 4\n",
        5,
    ),
    // 1.7.1: `[true]`, then the error.
    (
        &["-c", "-s"],
        "true\"",
        "",
        "jq: parse error: Unfinished string at EOF at line 1, column 5\n",
        5,
    ),
    // 1.7.1: `[null]`, then the error.
    (
        &["-c", "-s"],
        "null\"",
        "",
        "jq: parse error: Unfinished string at EOF at line 1, column 5\n",
        5,
    ),
    // 1.7.1: `[[[],0.5]]`, then the error.
    (
        &["-c", "-s", "--stream"],
        "0.5\"",
        "",
        "jq: parse error: Unfinished string at EOF at line 1, column 4\n",
        5,
    ),
    // 1.7.1: `["a",2]`, then the error.
    (
        &["-c", "-s"],
        "\"a\"2\"",
        "",
        "jq: parse error: Unfinished string at EOF at line 1, column 5\n",
        5,
    ),
    // 1.7.1: `[2]`, then the error (#6004 matched that).
    (
        &["-c", "-s"],
        "2[",
        "",
        "jq: parse error: Unfinished JSON term at EOF at line 1, column 2\n",
        5,
    ),
    // 1.7.1: `[[[],2]]`, then the error.
    (
        &["-c", "-s", "--stream"],
        "2[",
        "",
        "jq: parse error: Unfinished JSON term at EOF at line 1, column 2\n",
        5,
    ),
    // 1.7.1: `[[[],2]]`, then the fault event.
    (
        &["-c", "-s", "--stream-errors"],
        "2[",
        "[[[],2],[\"Unfinished JSON term at EOF at line 1, column 2\",[0]]]\n",
        "",
        0,
    ),
    // 1.7.1: `[1,2]`, then the error.
    (
        &["-c", "-s"],
        "1 2{",
        "",
        "jq: parse error: Unfinished JSON term at EOF at line 1, column 4\n",
        5,
    ),
    // 1.7.1: `[true]`, then the error.
    (
        &["-c", "-s"],
        "true[",
        "",
        "jq: parse error: Unfinished JSON term at EOF at line 1, column 5\n",
        5,
    ),
    // 1.7.1: `[[[0],1]]`, then the error.
    (
        &["-c", "-s", "--stream"],
        "[1,",
        "",
        "jq: parse error: Unfinished JSON term at EOF at line 1, column 3\n",
        5,
    ),
    // 1.7.1: `[[[0],1]]`, then the fault event.
    (
        &["-c", "-s", "--stream-errors"],
        "[1,",
        "[[[0],1],[\"Unfinished JSON term at EOF at line 1, column 3\",[1]]]\n",
        "",
        0,
    ),
    // 1.7.1: `[[["a"],1]]`, then the error.
    (
        &["-c", "-s", "--stream"],
        "{\"a\":1,",
        "",
        "jq: parse error: Unfinished JSON term at EOF at line 1, column 7\n",
        5,
    ),
    // 1.7.1: `[[[0],1],[[1],2]]`, then the error.
    (
        &["-c", "-s", "--stream"],
        "[1,2,",
        "",
        "jq: parse error: Unfinished JSON term at EOF at line 1, column 5\n",
        5,
    ),
    // The closing call yields the inner close, so the array holds it and the fault comes after.
    // 1.7.1: `[[[0,0],1]]`, then `[[0,0]]`, then the error.
    (
        &["-c", "-s", "--stream"],
        "[[1]",
        "[[[0,0],1],[[0,0]]]\n",
        "jq: parse error: Unfinished JSON term at EOF at line 1, column 4\n",
        5,
    ),
    // 1.7.1: `[[[0,0],1]]`, then `[[0,0]]` and the fault event.
    (
        &["-c", "-s", "--stream-errors"],
        "[[1]",
        "[[[0,0],1],[[0,0]]]\n[\"Unfinished JSON term at EOF at line 1, column 4\",[0]]\n",
        "",
        0,
    ),
    // Unchanged: the two releases agree where the fault is met before the last byte's value.
    (
        &["-c", "-s"],
        "[1,",
        "",
        "jq: parse error: Unfinished JSON term at EOF at line 1, column 3\n",
        5,
    ),
    (
        &["-c", "-s"],
        "2\"abc",
        "",
        "jq: parse error: Unfinished string at EOF at line 1, column 5\n",
        5,
    ),
    (
        &["-c", "-s", "--stream"],
        "2\"abc",
        "",
        "jq: parse error: Unfinished string at EOF at line 1, column 5\n",
        5,
    ),
    (
        &["-c", "-s", "--stream"],
        "tru",
        "",
        "jq: parse error: Invalid literal at EOF at line 1, column 3\n",
        5,
    ),
    (
        &["-c", "-s"],
        "[1]]",
        "",
        "jq: parse error: Unmatched ']' at line 1, column 4\n",
        5,
    ),
    (
        &["-c", "-s", "--stream-errors"],
        "[1]]",
        "[[[0],1],[[0]],[\"Unmatched ']' at the top-level at line 1, column 4\",[]]]\n",
        "",
        0,
    ),
    (&["-c", "-s", "--stream"], "  ", "[]\n", "", 0),
    (&["-c", "-s"], "[1]", "[[1]]\n", "", 0),
];

#[test]
fn a_fault_after_the_last_value_is_met_before_the_array_closes_7807() -> Result<()> {
    for (flags, input, stdout, stderr, code) in SLURPED {
        let mut args = flags.to_vec();
        args.push(".");
        let got = run_jq_full(&args, Some(input))?;
        assert_eq!(
            (got.0.as_str(), got.1.as_str(), got.2),
            (*stdout, *stderr, *code),
            "{flags:?} over {input:?}"
        );
    }
    Ok(())
}

const PROGRAM: &str = r#"[try input catch "error: \(.)", try input catch "error: \(.)"]"#;

/// (flags, input, expected stdout of `PROGRAM`): under `-n` the fault is raised by the first
/// `input` and the array is the second, where jq 1.7.1 gave the array first.
const UNDER_NULL_INPUT: &[(&[&str], &str, &str)] = &[
    // 1.7.1: `[[[[],2]],"error: ..."]`.
    (
        &["-c", "-n", "-s", "--stream"],
        "2\"",
        "[\"error: Unfinished string at EOF at line 1, column 2\",[[[],2]]]\n",
    ),
    // 1.7.1: `[[2],"error: ..."]`.
    (
        &["-c", "-n", "-s"],
        "2\"",
        "[\"error: Unfinished string at EOF at line 1, column 2\",[2]]\n",
    ),
    // 1.7.1: `[[[[0],1]],"error: ..."]`.
    (
        &["-c", "-n", "-s", "--stream"],
        "[1,",
        "[\"error: Unfinished JSON term at EOF at line 1, column 3\",[[[0],1]]]\n",
    ),
    // 1.7.1: `[[[[],2]],"error: ..."]`.
    (
        &["-c", "-n", "-s", "--stream"],
        "2[",
        "[\"error: Unfinished JSON term at EOF at line 1, column 2\",[[[],2]]]\n",
    ),
    // 1.7.1: `[[2],"error: ..."]`.
    (
        &["-c", "-n", "-s"],
        "2[",
        "[\"error: Unfinished JSON term at EOF at line 1, column 2\",[2]]\n",
    ),
    // 1.7.1: `[[[[0,0],1]],[[0,0]]]`.
    (
        &["-c", "-n", "-s", "--stream"],
        "[[1]",
        "[[[[0,0],1],[[0,0]]],\"error: Unfinished JSON term at EOF at line 1, column 4\"]\n",
    ),
];

#[test]
fn under_null_input_the_fault_comes_before_the_array_7807() -> Result<()> {
    for (flags, input, expected) in UNDER_NULL_INPUT {
        let mut args = flags.to_vec();
        args.push(PROGRAM);
        let (stdout, stderr, code) = run_jq_full(&args, Some(input))?;
        assert_eq!(
            (stdout.as_str(), code),
            (*expected, 0),
            "{flags:?} over {input:?}: {stderr}"
        );
    }
    Ok(())
}

/// The texts whose last byte completes a value the parser then finds unfinished.
const SHAPES: &[&str] = &["2\"", "1 2\"", "false\"", "2[", "2{", "[1,", "{\"a\":1,"];

/// Every route `-s` reads a text by answers it alike: standard input, a file, the text at the end
/// of a second file, and `--stream`, which reads with jq's streaming parser. All print the error
/// alone, as jq 1.8.2 does.
#[test]
fn every_slurp_route_answers_a_fault_after_the_last_value_alike_7807() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let first = dir.path().join("first.json");
    std::fs::write(&first, "1\n")?;
    let file = dir.path().join("f.json");
    let (first, file) = (first.to_string_lossy(), file.to_string_lossy());
    for shape in SHAPES {
        std::fs::write(file.as_ref(), shape)?;
        // The fault's text names its column on line 1, or on line 2 after the first file.
        let mut answers = Vec::new();
        for stream in [false, true] {
            let mut flags = vec!["-c", "-s"];
            if stream {
                flags.push("--stream");
            }
            flags.push(".");
            let mut with = |extra: &[&str], input: Option<&str>| -> Result<_> {
                let mut args = flags.clone();
                args.extend_from_slice(extra);
                let got = run_jq_full(&args, input)?;
                answers.push((format!("{args:?}"), got.clone()));
                Ok(got)
            };
            let piped = with(&[], Some(shape))?;
            let named = with(&[file.as_ref()], None)?;
            let second = with(&[first.as_ref(), file.as_ref()], None)?;
            assert_eq!(piped, named, "{shape:?}, stream {stream}");
            assert_eq!(
                (piped.0.as_str(), piped.2),
                ("", 5),
                "{shape:?}, stream {stream}: {}",
                piped.1
            );
            assert_eq!(
                (second.0.as_str(), second.2),
                ("", 5),
                "{shape:?} after a file, stream {stream}: {}",
                second.1
            );
            assert!(
                second.1.contains("at line 2, column"),
                "{shape:?} after a file, stream {stream}: {}",
                second.1
            );
        }
        // `--stream` reads the same text to the same answer as the documents' route.
        let (plain, stream) = answers.split_at(answers.len() / 2);
        for ((plain_args, plain), (stream_args, stream)) in plain.iter().zip(stream) {
            assert_eq!(
                plain, stream,
                "{plain_args} and {stream_args} over {shape:?}"
            );
        }
    }
    Ok(())
}

/// jq 1.7.1's main loop reads nothing after the array once a file has failed to open, so with a
/// failed file before the text it printed the array and never met the fault (`[[[],2]]` here, and
/// `[2]` for `-s` without `--stream`); jq 1.8.2 meets it before the array closes, prints the error
/// and exits 2 for the file.
#[test]
fn a_file_that_did_not_open_before_the_text_does_not_hide_the_fault_7807() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let missing = dir.path().join("missing.json");
    let file = dir.path().join("f.json");
    std::fs::write(&file, "2\"")?;
    let (missing, file) = (missing.to_string_lossy(), file.to_string_lossy());
    for stream in [false, true] {
        let mut args = vec!["-c", "-s"];
        if stream {
            args.push("--stream");
        }
        args.extend([".", missing.as_ref(), file.as_ref()]);
        let (stdout, stderr, code) = run_jq_full(&args, None)?;
        assert_eq!((stdout.as_str(), code), ("", 2), "{args:?}: {stderr}");
        assert_eq!(
            stderr,
            format!(
                "jq: error: Could not open file {missing}: No such file or directory\n\
                 jq: parse error: Unfinished string at EOF at line 1, column 2\n"
            ),
            "{args:?}"
        );
    }
    Ok(())
}
