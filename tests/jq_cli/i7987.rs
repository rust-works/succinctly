//! #7987 step 6: the default route (a program that reads no `input`, `inputs` or
//! `input_line_number`, over JSON read whole) enters ADR-0033's model of jq 1.7.1's reader on its
//! triggers and hands back to the semi-index at the next sync point. Step 7: so do the routes that
//! read a window at a time, and a long pipe's lazy route reads a mark that breaks off with it.
//!
//! Every expected output was captured from jq 1.7.1 (`/usr/bin/jq`); jq 1.8.2
//! (`/opt/homebrew/bin/jq`) prints the same on every row. The one row that pins `succinctly jq`'s
//! own answer says so, with the rule it follows.

use anyhow::Result;

use crate::{small_window, spawn_with_signal_retry, succinctly_bin};

/// `succinctly jq ARGS` in `dir`, with standard input `stdin` and `env` set: (stdout, stderr,
/// status).
fn run(
    dir: &std::path::Path,
    args: &[&str],
    stdin: &[u8],
    env: &[(&str, &str)],
) -> Result<(String, String, i32)> {
    let (output, code) = spawn_with_signal_retry(
        || {
            let mut command = std::process::Command::new(succinctly_bin());
            command.arg("jq").args(args).current_dir(dir);
            for &(key, value) in env {
                command.env(key, value);
            }
            command
        },
        Some(stdin),
    )?;
    Ok((
        String::from_utf8(output.stdout)?,
        String::from_utf8(output.stderr)?,
        code,
    ))
}

/// One row: the files (`(name, bytes)`, none for standard input), the arguments before them, what
/// standard input holds, and jq's (stdout, stderr, status).
type Row = (
    &'static [(&'static str, &'static [u8])],
    &'static [&'static str],
    &'static [u8],
    (&'static str, &'static str, i32),
);

static ROWS: &[Row] = &[
    // #5804 retired on the default route: a mark that breaks off is jq's `Malformed BOM`, where
    // the route used to word the first read as a numeric literal.
    (
        &[],
        &["."],
        b"\xef\xbf\xbd1",
        ("", "jq: parse error: Malformed BOM\n", 5),
    ),
    (
        &[("f.json", b"\xef\xbb1\n[2]\n")],
        &["-c", "."],
        b"",
        ("", "jq: parse error: Malformed BOM\n", 5),
    ),
    // A mark cut across files is skipped as jq skips it, and the model hands back after the read
    // that completes it.
    (
        &[("m0.json", b"\xef"), ("m1.json", b"\xbb\xbf[1]\n2\n")],
        &["-c", "."],
        b"",
        ("[1]\n2\n", "", 0),
    ),
    // A fault: the model reads the file it is in, so a malformed value there that `.` would have
    // printed unread is jq's fault, and nothing after it is printed.
    (
        &[("v.json", b"[]\n{} \"a\\x\" }\n")],
        &["-c", "."],
        b"",
        (
            "[]\n{}\n",
            "jq: parse error: Invalid escape at line 2, column 8\n",
            5,
        ),
    ),
    // Invalid UTF-8: the model reads the line it is on and validates it as jq's parser does.
    (
        &[("r1.json", b"1\n\"\xff\" \"a\\x\"\n2\n")],
        &["-c", "."],
        b"",
        (
            "1\n\"\u{fffd}\"\n",
            "jq: parse error: Invalid escape at line 2, column 9\n",
            5,
        ),
    ),
    // Several substituted lines, each a region, with the positions of the values between them and
    // the fault after them jq's.
    (
        &[("u.json", b"\"\xff\"\n1\n\"a\xfeb\" 2\n[3]\n\"\xe9\"\n}\n")],
        &["-c", "error(tojson)"],
        b"",
        (
            "",
            concat!(
                "jq: error (at u.json:1): \"\u{fffd}\"\n",
                "jq: error (at u.json:2): 1\n",
                "jq: error (at u.json:3): \"a\u{fffd}b\"\n",
                "jq: error (at u.json:3): 2\n",
                "jq: error (at u.json:4): [3]\n",
                "jq: error (at u.json:5): \"\u{fffd}\"\n",
                "jq: parse error: Unmatched '}' at line 6, column 1\n",
            ),
            5,
        ),
    ),
    // jq names the first fault of its input, which here is in a file before the one the route met
    // its fault in: the model reads the stream from its start for it.
    (
        &[("e0.json", b"\"a\\x\"\n"), ("e1.json", b"}\n")],
        &["-c", "."],
        b"",
        (
            // Not jq's stdout (jq prints nothing): `.` prints a value of an earlier file without
            // reading it (#3035, the #2103 amendment's lazy validation).
            "\"a\\x\"\n",
            "jq: parse error: Invalid escape at line 1, column 5\n",
            5,
        ),
    ),
];

#[test]
fn the_default_route_reads_its_edges_through_the_reader_model_7987() -> Result<()> {
    for &(files, args, stdin, want) in ROWS {
        let dir = tempfile::tempdir()?;
        let mut argv: Vec<&str> = args.to_vec();
        for &(name, bytes) in files {
            std::fs::write(dir.path().join(name), bytes)?;
            argv.push(name);
        }
        let got = run(dir.path(), &argv, stdin, &[])?;
        assert_eq!(
            (got.0.as_str(), got.1.as_str(), got.2),
            want,
            "{args:?} {files:?}"
        );
    }
    Ok(())
}

/// The model hands back at the next sync point: past the substituted line the semi-index reads on,
/// so a malformed value it prints without reading is not diagnosed (#3035), as it is not far from
/// any trigger. jq 1.7.1 says `Invalid escape at line 2, column 5` and prints only the first value.
#[test]
fn the_semi_index_reads_on_after_the_hand_back_7987() -> Result<()> {
    let dir = tempfile::tempdir()?;
    std::fs::write(dir.path().join("r2.json"), b"\"\xff\"\n\"a\\x\"\n")?;
    let got = run(dir.path(), &["-c", ".", "r2.json"], b"", &[])?;
    assert_eq!(got, ("\"\u{fffd}\"\n\"a\\x\"\n".into(), String::new(), 0));
    Ok(())
}

/// `SUCCINCTLY_READER_MODEL=0` selects the deciders the model replaces (ADR-0033: two readers until
/// step 10), so the route words the broken mark as it did before (#5804).
#[test]
fn the_switch_selects_the_old_reader_on_the_default_route_7987() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let got = run(
        dir.path(),
        &["."],
        b"\xef\xbf\xbd1",
        &[("SUCCINCTLY_READER_MODEL", "0")],
    )?;
    assert_eq!(
        got,
        (
            String::new(),
            "jq: parse error: Invalid numeric literal at EOF at line 1, column 4\n".into(),
            5
        )
    );
    Ok(())
}

/// Step 7's fixture: `lines` records `{"id":N,"name":"userN"}` from `start`, one a line.
fn records(start: usize, lines: usize) -> Vec<u8> {
    (start..start + lines)
        .flat_map(|i| format!("{{\"id\":{i},\"name\":\"user{i}\"}}\n").into_bytes())
        .collect()
}

/// The records each side of an edge: about 170 KiB a side, so a file of both is past the four
/// windows the windowed routes take ([`small_window::MIN_BYTES`] in a debug build), a pipe past their
/// two, and a pipe past the 256 KiB a lazy route reads first.
const SIDE: usize = 6000;

/// `succinctly jq ARGS` over `input` as a file (`stdin` false) or a pipe, with the small window of a
/// debug build, `env` set and the windowed routes it took: (stdout, stderr, status, routes).
fn run_windowed(
    input: &[u8],
    args: &[&str],
    stdin: bool,
    env: &[(&str, &str)],
) -> Result<(String, String, i32, String)> {
    let dir = tempfile::tempdir()?;
    let log = dir.path().join("routes.log");
    let mut argv: Vec<&str> = args.to_vec();
    if !stdin {
        std::fs::write(dir.path().join("in.json"), input)?;
        argv.push("in.json");
    }
    let mut env = env.to_vec();
    let window = small_window::WINDOW.to_string();
    if cfg!(debug_assertions) {
        env.push((small_window::ENV, &window));
    }
    let log_path = log.to_string_lossy().into_owned();
    env.push(("SUCCINCTLY_DEBUG_JQ_ROUTE_LOG", &log_path));
    let (out, err, code) = run(dir.path(), &argv, if stdin { input } else { b"" }, &env)?;
    let routes = std::fs::read_to_string(&log).unwrap_or_default();
    Ok((out, err, code, routes))
}

/// #5804 retired on the routes that read a window at a time (step 7): the default route over a file
/// of four windows or more and over a pipe of two, where the stream's fault is read by the model
/// from its start (by position over a file, by the thread that certifies each window over a pipe),
/// and a long pipe's or file's lazy route, which reads the broken mark from the front it read. Every
/// row is jq 1.7.1's (1.8.2 agrees): `printf '\xef\xbf\xbd1\n'` and 12,000 records.
#[test]
fn a_mark_that_breaks_off_is_malformed_on_the_windowed_and_lazy_routes_7987() -> Result<()> {
    let input = [&b"\xef\xbf\xbd1\n"[..], &records(0, 2 * SIDE)].concat();
    assert!(input.len() >= small_window::MIN_BYTES);
    for stdin in [false, true] {
        let name = if stdin { "<stdin>" } else { "in.json" };
        let rows: [(&[&str], String, String, i32, bool); 4] = [
            (
                &["-c", "."],
                String::new(),
                "jq: parse error: Malformed BOM\n".into(),
                5,
                true,
            ),
            (
                &["-c", ".a?"],
                String::new(),
                "jq: parse error: Malformed BOM\n".into(),
                5,
                true,
            ),
            (
                &["-c", "-n", "[inputs]|length"],
                String::new(),
                format!("jq: error (at {name}:1): Malformed BOM\n"),
                5,
                false,
            ),
            (
                &[
                    "-c",
                    "-n",
                    "[limit(3; repeat(try input catch [., input_line_number]))]",
                ],
                "[[\"Malformed BOM\",1],[\"Malformed BOM\",1],[\"Malformed BOM\",1]]\n".into(),
                String::new(),
                0,
                false,
            ),
        ];
        for (args, out, err, code, windowed) in rows {
            let got = run_windowed(&input, args, stdin, &[])?;
            assert_eq!(
                (&got.0, &got.1, got.2),
                (&out, &err, code),
                "{args:?} stdin {stdin}"
            );
            if windowed {
                assert_eq!(
                    got.3, "plain\n",
                    "{args:?} stdin {stdin}: the windowed route"
                );
            }
        }
    }
    Ok(())
}

/// Step 7: a windowed window with invalid UTF-8 in it is read as step 6 reads a file read whole: the
/// model is entered at the line start before the substituted byte, reads the values there and
/// validates them as jq's parser does (ADR-0033 decision 3), so a malformed value on that line is
/// jq's fault where the windows used to print past it unread (`.a?` reads no array). jq 1.7.1's
/// answers (1.8.2 agrees), over a file and over a pipe.
#[test]
fn a_substituted_line_in_a_window_is_read_by_the_model_7987() -> Result<()> {
    let nulls = "null\n".repeat(SIDE);
    let rows: [(&[u8], String, String); 2] = [
        (
            b"[\"\xff\",]\n",
            nulls.clone(),
            format!(
                "jq: parse error: Expected another array element at line {}, column 6\n",
                SIDE + 1
            ),
        ),
        (
            b"{\"a\":\"\xe9\"} [1 2]\n",
            format!("{nulls}\"\u{fffd}\"\n"),
            format!(
                "jq: parse error: Expected separator between values at line {}, column 15\n",
                SIDE + 1
            ),
        ),
    ];
    for (edge, out, err) in rows {
        let input = [records(0, SIDE), edge.to_vec(), records(SIDE, SIDE)].concat();
        for stdin in [false, true] {
            let got = run_windowed(&input, &["-c", ".a?"], stdin, &[])?;
            assert_eq!(
                (&got.0, &got.1, got.2, got.3.as_str()),
                (&out, &err, 5, "plain\n"),
                "{edge:?} stdin {stdin}"
            );
        }
    }
    Ok(())
}

/// A substituted byte on the stream's first line, before a document that runs on to the next line
/// and faults there: the window holding the first line leaves that document to the next, which
/// starts on the same line again, and the line's value is the one window's, read once (the
/// differential's first draft of step 7 printed it twice). jq 1.7.1 (1.8.2 agrees), file and pipe.
#[test]
fn a_value_before_a_document_the_next_window_reads_is_read_once_7987() -> Result<()> {
    let input = [&b"\"a\xe9\" [1,\n2,}\n"[..], &records(0, 2 * SIDE)].concat();
    for stdin in [false, true] {
        let got = run_windowed(&input, &["-c", "."], stdin, &[])?;
        assert_eq!(
            (got.0.as_str(), got.1.as_str(), got.2, got.3.as_str()),
            (
                "\"a\u{fffd}\"\n",
                "jq: parse error: Unmatched '}' at line 2, column 3\n",
                5,
                "plain\n"
            ),
            "stdin {stdin}"
        );
    }
    Ok(())
}

/// `SUCCINCTLY_READER_MODEL=0` keeps the old deciders on the windowed routes too: the broken mark is
/// worded as the numeric literal the splitter met (#5804), and the substituted line is printed past
/// unread, as before step 7.
#[test]
fn the_switch_selects_the_old_reader_on_the_windowed_routes_7987() -> Result<()> {
    let off = [("SUCCINCTLY_READER_MODEL", "0")];
    let mark = [&b"\xef\xbf\xbd1\n"[..], &records(0, 2 * SIDE)].concat();
    for stdin in [false, true] {
        let got = run_windowed(&mark, &["-c", "."], stdin, &off)?;
        assert_eq!(
            (got.1.as_str(), got.2, got.3.as_str()),
            (
                "jq: parse error: Invalid numeric literal at line 2, column 0\n",
                5,
                "plain\n"
            ),
            "stdin {stdin}"
        );
    }
    let line = [
        records(0, SIDE),
        b"[\"\xff\",]\n".to_vec(),
        records(SIDE, SIDE),
    ]
    .concat();
    let got = run_windowed(&line, &["-c", ".a?"], false, &off)?;
    assert_eq!((got.1.as_str(), got.2), ("", 0));
    assert_eq!(got.0.lines().count(), 2 * SIDE);
    Ok(())
}
