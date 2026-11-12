//! #7023: `succinctly jq` reads one big regular file a window at a time on the routes that hold a
//! stream's text for the whole run: `-n` with `input`/`inputs` (the lazy route) and `-R`. A window
//! ends at a line break, the text before the documents read is dropped with its line breaks counted,
//! and a fault, or a byte that is not UTF-8, makes the rest of the file be read and the stream be
//! worded as the whole file's would be. Every row here was checked against jq 1.7.1 on the same bytes.
//!
//! The files are over 4 MiB (the smallest `-R` reads in windows; the lazy route takes anything past
//! its 256 KiB front). Which runs take the routes, and that they hold a window and not the file, is
//! pinned in `jq_runner.rs` (`a_lone_source_read_in_windows_*_7023`, `raw_lines_read_in_windows_*_7023`).

use anyhow::Result;

use crate::spawn_jq;

/// About 6.6 MiB: well over the 4 MiB `-R` takes, so its minimum can move a little without the tests
/// quietly reading whole files.
const LINES: usize = 60_000;

fn lines(count: usize) -> Vec<String> {
    (0..count)
        .map(|i| {
            format!(
                "{{\"id\":{i},\"name\":\"user {i}\",\"email\":\"user{i}@example.com\",\"tags\":[\"a\",\"b\",\"c\"],\"score\":{}}}",
                i % 1000
            )
        })
        .collect()
}

fn join(lines: &[String]) -> Vec<u8> {
    let mut bytes = lines.join("\n").into_bytes();
    bytes.push(b'\n');
    bytes
}

/// `bytes` as a file, run with `args` before its name: stdout, stderr with the file's name as `<f>`,
/// and the exit status.
fn run(bytes: &[u8], args: &[&str]) -> Result<(String, String, i32)> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("big.json");
    std::fs::write(&path, bytes)?;
    let name = path.to_string_lossy().into_owned();
    let mut argv: Vec<&str> = args.to_vec();
    argv.push(&name);
    let (out, status) = spawn_jq(&argv, None)?;
    Ok((
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).replace(&name, "<f>"),
        status,
    ))
}

#[test]
fn inputs_over_a_big_file_read_every_document_7023() -> Result<()> {
    let bytes = join(&lines(LINES));
    let (out, err, status) = run(&bytes, &["-n", "reduce inputs as $x (0; . + $x.id)"])?;
    assert_eq!(
        (out.as_str(), err.as_str(), status),
        ("1799970000\n", "", 0)
    );
    // jq's line for a document is the count through the read that completes it.
    let (out, _, _) = run(
        &bytes,
        &["-n", "inputs | select(.id == 45000) | input_line_number"],
    )?;
    assert_eq!(out, "45001\n");
    let (out, _, _) = run(
        &bytes,
        &["-nc", "[first(inputs).id, input_filename != null]"],
    )?;
    assert_eq!(out, "[0,true]\n");
    Ok(())
}

#[test]
fn the_end_of_the_stream_counts_the_lines_dropped_7023() -> Result<()> {
    let mut bytes = join(&lines(LINES));
    let at_end = "jq: error (at <unknown>): break\n";
    // A last line with a line break, and one without.
    for ends in [true, false] {
        if !ends {
            bytes.pop();
        }
        let (out, err, status) = run(&bytes, &["-n", "reduce inputs as $x (0; . + 1) | ., input"])?;
        assert_eq!((out.as_str(), err.as_str(), status), ("60000\n", at_end, 5));
    }
    Ok(())
}

#[test]
fn a_fault_past_the_first_window_is_worded_as_jq_words_it_7023() -> Result<()> {
    let mut mid = lines(LINES);
    mid[30_000] = "}".to_string();
    let (out, err, status) = run(
        &join(&mid),
        &["-nc", "inputs | select(.id % 10000 == 0) | .id"],
    )?;
    assert_eq!(out, "0\n10000\n20000\n");
    assert_eq!(
        err,
        "jq: error (at <f>:30001): Unmatched '}' at line 30001, column 1\n"
    );
    assert_eq!(status, 5);

    // A number the fault's own byte completes is never read.
    let mut bare = lines(LINES);
    bare[30_000] = "12}".to_string();
    let (out, err, status) = run(
        &join(&bare),
        &["-nc", "inputs | select(.id % 10000 == 0) | .id"],
    )?;
    assert_eq!(out, "0\n10000\n20000\n");
    assert_eq!(
        err,
        "jq: error (at <f>:30001): Unmatched '}' at line 30001, column 3\n"
    );
    assert_eq!(status, 5);

    // A document the file ends inside.
    let mut eof = lines(LINES);
    eof.push("{\"id\":1,".to_string());
    let bytes = join(&eof);
    let (out, err, status) = run(
        &bytes[..bytes.len() - 1],
        &["-nc", "inputs | select(.id == 59999) | .id"],
    )?;
    assert_eq!(out, "59999\n");
    assert_eq!(
        err,
        "jq: error (at <f>:60000): Unfinished JSON term at EOF at line 60001, column 8\n"
    );
    assert_eq!(status, 5);
    Ok(())
}

#[test]
fn documents_across_lines_and_bytes_that_are_not_utf8_read_as_they_do_whole_7023() -> Result<()> {
    // Documents that run across several lines, so a window may end inside one.
    let mut pretty = Vec::new();
    for i in 0..20_000 {
        pretty.extend_from_slice(
            format!(
                "{{\n  \"id\": {i},\n  \"tags\": [\n    \"a\",\n    \"b\"\n  ],\n  \"o\": {{\n    \"x\": {i}\n  }}\n}}\n"
            )
            .as_bytes(),
        );
    }
    let (out, _, status) = run(&pretty, &["-nc", "last(inputs) | [.id, input_line_number]"])?;
    assert_eq!((out.as_str(), status), ("[19999,200000]\n", 0));

    // A byte that is not UTF-8 in three windows: each run of them becomes one U+FFFD.
    let mut bytes = join(&lines(LINES));
    for line in [10_000usize, 25_000, 40_000] {
        let needle = format!("\"user {line}\"").into_bytes();
        let at = bytes
            .windows(needle.len())
            .position(|w| w == needle.as_slice())
            .unwrap();
        bytes.insert(at + 1, 0xff);
    }
    let (out, err, status) = run(
        &bytes,
        &[
            "-nc",
            "inputs | select(.id == 10000 or .id == 25000 or .id == 40000) | .name",
        ],
    )?;
    assert_eq!(
        (out.as_str(), err.as_str(), status),
        (
            "\"\u{fffd}user 10000\"\n\"\u{fffd}user 25000\"\n\"\u{fffd}user 40000\"\n",
            "",
            0
        )
    );
    // The end of the stream is the file's, with the bytes as jq read them.
    let (out, _, _) = run(&bytes, &["-n", "last(inputs) | input_line_number"])?;
    assert_eq!(out, "60000\n");
    Ok(())
}

#[test]
fn raw_lines_of_a_big_file_are_numbered_and_ended_as_jq_does_7023() -> Result<()> {
    let mut bytes = join(&lines(LINES));
    let (out, _, _) = run(
        &bytes,
        &["-R", "select(input_line_number == 45000) | .[0:14]"],
    )?;
    assert_eq!(out, "\"{\\\"id\\\":44999,\\\"n\"\n");
    let (out, err, status) = run(
        &bytes,
        &["-nR", "reduce inputs as $l (0; . + 1) | ., input"],
    )?;
    assert_eq!(
        (out.as_str(), err.as_str(), status),
        ("60000\n", "jq: error (at <unknown>): break\n", 5)
    );
    let (out, _, _) = run(&bytes, &["-nR", "last(inputs) | input_line_number"])?;
    assert_eq!(out, "60000\n");
    // A last line with no line break is on the line before it.
    bytes.pop();
    let (out, _, _) = run(&bytes, &["-nR", "last(inputs) | input_line_number"])?;
    assert_eq!(out, "0\n");
    let (out, err, status) = run(
        &bytes,
        &["-nR", "reduce inputs as $l (0; . + 1) | ., input"],
    )?;
    assert_eq!(
        (out.as_str(), err.as_str(), status),
        ("60000\n", "jq: error (at <unknown>): break\n", 5)
    );
    // Lines count the same as `-s`'s array of them does.
    let (out, _, _) = run(&bytes, &["-R", "-n", "[inputs] | length"])?;
    assert_eq!(out, "60000\n");
    Ok(())
}

#[test]
fn raw_lines_with_bytes_that_are_not_utf8_are_substituted_a_line_at_a_time_7023() -> Result<()> {
    let mut bytes = join(&lines(LINES));
    for line in [10_000usize, 25_000, 40_000] {
        let needle = format!("\"user {line}\"").into_bytes();
        let at = bytes
            .windows(needle.len())
            .position(|w| w == needle.as_slice())
            .unwrap();
        bytes.splice(at + 1..at + 1, [0xffu8, 0xfe]);
    }
    let (out, err, status) = run(&bytes, &["-R", "select(contains(\"\u{fffd}\")) | .[0:30]"])?;
    assert_eq!(status, 0, "{err:?}");
    // Each byte that is not UTF-8 is a U+FFFD of its own in its line (`-R` substitutes per line).
    assert_eq!(
        out,
        "\"{\\\"id\\\":10000,\\\"name\\\":\\\"\u{fffd}\u{fffd}user 100\"\n\
         \"{\\\"id\\\":25000,\\\"name\\\":\\\"\u{fffd}\u{fffd}user 250\"\n\
         \"{\\\"id\\\":40000,\\\"name\\\":\\\"\u{fffd}\u{fffd}user 400\"\n"
    );
    Ok(())
}
