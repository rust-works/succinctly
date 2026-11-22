//! #7750: after a fault a program's `input` catches, a file that does not open later in the list stopped
//! the run for good. jq's parser stops at the fault, drops the rest of the line it read and parses on
//! from the next one; the stray `"` after the fault (`[x"]`) looked like a string the file ends inside,
//! so the stitch of #4305 carried the fault and every value after it onto the next file, the reads
//! found the failure before them, and the main loop stopped one value later. Every expected string
//! was captured from `/usr/bin/jq` 1.7.1, and jq 1.8.2 gives the same.

use anyhow::Result;

use crate::run_jq_in_dir;

fn files() -> Result<tempfile::TempDir> {
    let dir = tempfile::tempdir()?;
    for (name, text) in [
        ("v.json", "0 [x\"]\n1\n2\n3\n4\n"),
        ("w.json", "100\n101\n"),
        ("o.json", "0 {\"a\":x\"}\n1\n2\n"),
        ("p.json", "[1,\n}2"),
        ("q.json", "]\n"),
        ("pq.json", "[1,\n}2]\n"),
    ] {
        std::fs::write(dir.path().join(name), text)?;
    }
    std::fs::create_dir(dir.path().join("dir"))?;
    Ok(dir)
}

/// (arguments, standard input, stdout, stderr, exit status).
type Row = (
    &'static [&'static str],
    Option<&'static str>,
    &'static str,
    &'static str,
    i32,
);

fn check(rows: &[Row]) -> Result<()> {
    let dir = files()?;
    for &(args, input, stdout, stderr, code) in rows {
        let (out, err, status) = run_jq_in_dir(dir.path(), args, input)?;
        assert_eq!(out, stdout, "{args:?} stdout");
        assert_eq!(err, stderr, "{args:?} stderr");
        assert_eq!(status, code, "{args:?} status");
    }
    Ok(())
}

#[test]
fn a_caught_fault_then_a_file_that_does_not_open_reads_on_7750() -> Result<()> {
    check(&[
        (&["-c", "try input catch \"E\"", "v.json", "missing.json", "w.json"], None, "\"E\"\n2\n4\n101\n", "jq: error: Could not open file missing.json: No such file or directory\n", 2),
        (&["-c", "input", "v.json", "missing.json", "w.json"], None, "2\n4\n101\n", "jq: error (at v.json:1): Invalid numeric literal at line 1, column 5\njq: error: Could not open file missing.json: No such file or directory\n", 2),
        (&["-c", "[., input]", "v.json", "missing.json", "w.json"], None, "[1,2]\n[3,4]\n[100,101]\n", "jq: error (at v.json:1): Invalid numeric literal at line 1, column 5\njq: error: Could not open file missing.json: No such file or directory\n", 2),
        (&["-c", "[., (try input catch \"E\")]", "v.json", "missing.json", "w.json"], None, "[0,\"E\"]\n[1,2]\n[3,4]\n[100,101]\n", "jq: error: Could not open file missing.json: No such file or directory\n", 2),
        (&["-c", "try input catch \"E\"", "o.json", "missing.json", "w.json"], None, "\"E\"\n2\n101\n", "jq: error: Could not open file missing.json: No such file or directory\n", 2),
        (&["-c", "-n", "[inputs?]", "v.json", "missing.json", "w.json"], None, "[0]\n", "", 0),
        (&["-c", "-n", "[limit(9; repeat(try input catch \"E\"))]", "v.json", "missing.json", "w.json"], None, "[0,\"E\",1,2,3,4,100,101,\"E\"]\n", "jq: error: Could not open file missing.json: No such file or directory\n", 2),
        (&["-c", "try input catch \"E\"", "v.json", "dir", "w.json"], None, "\"E\"\n2\n4\n101\n", "jq: error: Is a directory\n", 2),
        (&["-c", "input", "v.json", "dir", "w.json"], None, "2\n4\n101\n", "jq: error (at v.json:1): Invalid numeric literal at line 1, column 5\njq: error: Is a directory\n", 2),
        (&["-c", "[., input]", "v.json", "dir", "w.json"], None, "[1,2]\n[3,4]\n[100,101]\n", "jq: error (at v.json:1): Invalid numeric literal at line 1, column 5\njq: error: Is a directory\n", 2),
        (&["-c", "[., (try input catch \"E\")]", "v.json", "dir", "w.json"], None, "[0,\"E\"]\n[1,2]\n[3,4]\n[100,101]\n", "jq: error: Is a directory\n", 2),
        (&["-c", "try input catch \"E\"", "o.json", "dir", "w.json"], None, "\"E\"\n2\n101\n", "jq: error: Is a directory\n", 2),
        (&["-c", "-n", "[inputs?]", "v.json", "dir", "w.json"], None, "[0]\n", "", 0),
        (&["-c", "-n", "[limit(9; repeat(try input catch \"E\"))]", "v.json", "dir", "w.json"], None, "[0,\"E\",1,2,3,4,100,101,\"E\"]\n", "jq: error: Is a directory\n", 2),
        (&["-c", "try input catch \"E\"", "v.json", "w.json"], None, "\"E\"\n2\n4\n101\n", "", 0),
        (&["-c", "input", "v.json", "w.json"], None, "2\n4\n101\n", "jq: error (at v.json:1): Invalid numeric literal at line 1, column 5\n", 0),
        (&["-c", "[., input]", "v.json", "w.json"], None, "[1,2]\n[3,4]\n[100,101]\n", "jq: error (at v.json:1): Invalid numeric literal at line 1, column 5\n", 0),
        (&["-c", "[., (try input catch \"E\")]", "v.json", "w.json"], None, "[0,\"E\"]\n[1,2]\n[3,4]\n[100,101]\n", "", 0),
        (&["-c", "try input catch \"E\"", "o.json", "w.json"], None, "\"E\"\n2\n101\n", "", 0),
        (&["-c", "-n", "[inputs?]", "v.json", "w.json"], None, "[0]\n", "", 0),
        (&["-c", "-n", "[limit(9; repeat(try input catch \"E\"))]", "v.json", "w.json"], None, "[0,\"E\",1,2,3,4,100,101,\"E\"]\n", "", 0),
        (&["-c", "try input catch \"E\"", "v.json", "missing.json", "dir", "w.json"], None, "\"E\"\n2\n4\n101\n", "jq: error: Could not open file missing.json: No such file or directory\njq: error: Is a directory\n", 2),
        (&["-c", ".", "v.json", "missing.json", "w.json"], None, "0\n", "jq: parse error: Invalid numeric literal at line 1, column 5\n", 5),
    ])
}

/// A cut that falls after a fault the lookup steps over (h2-07, recorded in
/// `docs/compliance/jq/limitations.md`): the file read whole answers as a lookup reads it, the two
/// files raise jq's parse error, because the cut leaves the fault in the bytes the first file ends
/// with and the stitch carries only what jq's parser holds open there.
#[test]
fn a_cut_after_a_fault_a_lookup_steps_over_is_recorded_7750() -> Result<()> {
    const UNMATCHED: &str = "jq: parse error: Unmatched '}' at line 2, column 1\n";
    check(&[
        // jq 1.7.1 and 1.8.2: the parse error, exit 5.
        (&["-c", "first", "pq.json"], None, "", UNMATCHED, 5),
        (&["-c", "first", "p.json", "q.json"], None, "", UNMATCHED, 5),
        (&["-c", "type", "p.json", "q.json"], None, "", UNMATCHED, 5),
        (&["-c", ".a", "p.json", "q.json"], None, "", UNMATCHED, 5),
        // The recorded divergence: one file, read as the lookup reads it.
        (&["-c", "type", "pq.json"], None, "\"array\"\n", "", 0),
        (
            &["-c", ".a", "pq.json"],
            None,
            "",
            "jq: error (at pq.json:2): Cannot index array with string \"a\"\n",
            5,
        ),
    ])
}
