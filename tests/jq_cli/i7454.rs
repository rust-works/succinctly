//! #7454: input that ends inside the UTF-8 byte-order mark (`EF`, or `EF BB`) is empty input, as in
//! jq 1.7.1 and 1.8.2, which agree on every row: its parser keeps its place in the mark across a refill
//! and accepts the end of the stream there. It was a numeric-literal fault (exit 5). A mark that goes on
//! to something else is a fault in both (`Malformed BOM` in jq, wording that is #5804's), and so are a
//! stray `EF` after a whole mark and one after a value, which the last rows pin.
//!
//! Every expectation was captured from jq 1.7.1 and 1.8.2.

use anyhow::Result;

use crate::spawn_jq;

/// A program's flags and what jq prints for input that is empty: stdout, and the exit status. `-n input`
/// is the one that says so on stderr (`break`), with the source's name.
const EMPTY_INPUT: [(&[&str], &str, i32); 8] = [
    (&["-c", "."], "", 0),
    (&["-c", "-s", "."], "[]\n", 0),
    (&["-c", "-n", "[inputs]"], "[]\n", 0),
    (&["-c", "--stream", "."], "", 0),
    (&["-c", "-s", "--stream", "."], "[]\n", 0),
    (&["-c", "-n", "--stream", "[inputs]"], "[]\n", 0),
    (&["-e", "."], "", 4),
    (&["-c", "-n", "[inputs]", "--stream"], "[]\n", 0),
];

const MARKS: [&[u8]; 2] = [b"\xef", b"\xef\xbb"];

fn run(args: &[&str], stdin: &[u8]) -> Result<(String, String, i32)> {
    let (output, status) = spawn_jq(args, Some(stdin))?;
    Ok((
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        status,
    ))
}

#[test]
fn input_that_ends_inside_the_bom_is_empty_on_stdin_7454() -> Result<()> {
    for mark in MARKS {
        for (args, stdout, status) in EMPTY_INPUT {
            let got = run(args, mark)?;
            assert_eq!(
                (got.0.as_str(), got.1.as_str(), got.2),
                (stdout, "", status),
                "{args:?} over {mark:?}"
            );
        }
        let got = run(&["-c", "-n", "input"], mark)?;
        assert_eq!(
            (got.0.as_str(), got.1.as_str(), got.2),
            ("", "jq: error (at <stdin>:0): break\n", 5),
            "input over {mark:?}"
        );
    }
    Ok(())
}

#[test]
fn input_that_ends_inside_the_bom_is_empty_in_a_file_7454() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let empty = dir.path().join("empty.json");
    std::fs::write(&empty, b"")?;
    let empty = empty.to_string_lossy().into_owned();
    for (at, mark) in MARKS.iter().enumerate() {
        let file = dir.path().join(format!("mark{at}.json"));
        std::fs::write(&file, mark)?;
        let file = file.to_string_lossy().into_owned();
        // The mark alone, and with a file that has no bytes on either side of it: the stream still
        // ends inside the mark.
        for files in [
            vec![file.as_str()],
            vec![empty.as_str(), file.as_str()],
            vec![file.as_str(), empty.as_str()],
        ] {
            for (args, stdout, status) in EMPTY_INPUT {
                let mut all = args.to_vec();
                all.extend(&files);
                let got = run(&all, b"")?;
                assert_eq!(
                    (got.0.as_str(), got.1.as_str(), got.2),
                    (stdout, "", status),
                    "{all:?} over {mark:?}"
                );
            }
        }
        let got = run(&["-c", "-n", "input", &file], b"")?;
        assert_eq!(
            (got.0.as_str(), got.1.as_str(), got.2),
            ("", format!("jq: error (at {file}:0): break\n").as_str(), 5),
            "input over {file}"
        );
    }
    Ok(())
}

#[test]
fn a_mark_that_goes_on_is_still_a_fault_7454() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let mark = dir.path().join("mark.json");
    std::fs::write(&mark, b"\xef")?;
    let mark = mark.to_string_lossy().into_owned();
    let one = dir.path().join("one.json");
    std::fs::write(&one, b"1")?;
    let one = one.to_string_lossy().into_owned();
    // `EF` and then `1` in the next file is `Malformed BOM` in jq: still a fault, with nothing printed.
    let got = run(&["-c", ".", &mark, &one], b"")?;
    assert_eq!((got.0.as_str(), got.2), ("", 5), "{got:?}");
    // A stray `EF` after a whole mark or after a value is a fault in jq as well, with the wording
    // pinned from jq.
    for (bytes, stdout, stderr) in [
        (
            &b"\xef\xbb\xbf\xef"[..],
            "",
            "jq: parse error: Invalid numeric literal at EOF at line 1, column 1\n",
        ),
        (
            &b"\xef\xbb\xbf1\xef"[..],
            "",
            "jq: parse error: Invalid numeric literal at EOF at line 1, column 2\n",
        ),
        (
            &b"1\n\xef"[..],
            "1\n",
            "jq: parse error: Invalid numeric literal at EOF at line 2, column 1\n",
        ),
    ] {
        for args in [
            &["-c", "."][..],
            &["-c", "-s", "."],
            &["-c", "--stream", "."],
        ] {
            let got = run(args, bytes)?;
            let want_stdout = if args.contains(&"--stream") && stdout == "1\n" {
                "[[],1]\n"
            } else if args.contains(&"-s") {
                ""
            } else {
                stdout
            };
            assert_eq!(
                (got.0.as_str(), got.1.as_str(), got.2),
                (want_stdout, stderr, 5),
                "{args:?} over {bytes:?}"
            );
        }
    }
    // The mark followed by a line break departs from it: a fault, as `Malformed BOM` is (#5804).
    let got = run(&["-c", "."], b"\xef\n")?;
    assert_eq!((got.0.as_str(), got.2), ("", 5), "{got:?}");
    Ok(())
}
