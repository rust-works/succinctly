//! #7751: after a fault a program's `input` catches in the first of several files, `input_filename`
//! named the last file. jq's parser stops at the fault and parses on from the next line of the same
//! file; a container left open by the fault (`{y`, `[x"]`) looked like one the file ends inside, so
//! the stitch of #4305 moved it, and every value after it, onto the next file, which then answered
//! for them. Every expected string was captured from `/usr/bin/jq` 1.7.1, and jq 1.8.2 gives the
//! same.

use anyhow::Result;

use crate::run_jq_in_dir;

fn files() -> Result<tempfile::TempDir> {
    let dir = tempfile::tempdir()?;
    for (name, text) in [
        ("a.json", "1\n{y\n2\n"),
        ("b.json", "3\n"),
        ("c.json", "1 {y\n2 3\n"),
        ("d.json", "4\n5\n"),
        ("e.json", "1\n[x\"]\n2\n"),
        ("f.json", "{\"a\":\n1}\n{\"b\":[x\"\n7\n"),
    ] {
        std::fs::write(dir.path().join(name), text)?;
    }
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
fn input_filename_after_a_caught_fault_names_the_file_it_reads_7751() -> Result<()> {
    check(&[
        (&["-n", "-c", "(try input catch \"E\"), (try input catch \"E\"), (input|[., input_filename, input_line_number]), (input|[., input_filename, input_line_number])", "a.json", "b.json"], None, "1\n\"E\"\n[2,\"a.json\",3]\n[3,\"b.json\",1]\n", "", 0),
        (&["-n", "-c", "(try input catch \"E\"), (try input catch \"E\"), (input|[., input_filename, input_line_number]), (input|[., input_filename, input_line_number])", "e.json", "b.json"], None, "1\n\"E\"\n[2,\"e.json\",3]\n[3,\"b.json\",1]\n", "", 0),
        (&["-c", "input|[., input_filename, input_line_number]", "c.json", "d.json"], None, "[3,\"c.json\",2]\n[5,\"d.json\",2]\n", "jq: error (at c.json:1): Invalid numeric literal at line 2, column 0\n", 0),
        (&["-c", "[input_filename, input_line_number, (try input catch \"E\")]", "c.json", "d.json"], None, "[\"c.json\",1,\"E\"]\n[\"c.json\",2,3]\n[\"d.json\",1,5]\n", "", 0),
        (&["-n", "-c", "[limit(6; repeat(try (input|[., input_filename, input_line_number]) catch \"E\"))]", "a.json", "b.json"], None, "[[1,\"a.json\",1],\"E\",[2,\"a.json\",3],[3,\"b.json\",1],\"E\",\"E\"]\n", "", 0),
        (&["-n", "-c", "[limit(6; repeat(try (input|[., input_filename, input_line_number]) catch \"E\"))]", "e.json", "b.json"], None, "[[1,\"e.json\",1],\"E\",[2,\"e.json\",3],[3,\"b.json\",1],\"E\",\"E\"]\n", "", 0),
        (&["-n", "-c", "[limit(6; repeat(try (input|[., input_filename, input_line_number]) catch \"E\"))]", "f.json", "b.json"], None, "[[{\"a\":1},\"f.json\",2],\"E\",[7,\"f.json\",4],[3,\"b.json\",1],\"E\",\"E\"]\n", "", 0),
        (&["-n", "-c", "[limit(6; repeat(try (input|[., input_filename, input_line_number]) catch \"E\"))]", "-", "b.json"], Some("1\n{y\n2\n"), "[[1,\"<stdin>\",1],\"E\",[2,\"<stdin>\",3],[3,\"b.json\",1],\"E\",\"E\"]\n", "", 0),
        (&["-n", "-c", "[limit(6; repeat(try (input|[., input_filename, input_line_number]) catch \"E\"))]", "a.json"], None, "[[1,\"a.json\",1],\"E\",[2,\"a.json\",3],\"E\",\"E\",\"E\"]\n", "", 0),
    ])
}
