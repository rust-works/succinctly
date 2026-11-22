//! #7755: `-s` with a directory in the file list reported a later fault than jq's first, or the
//! first in the evaluator's words (`jq: error (at f:N): Invalid JSON text: ...`): the directory is a
//! source with no bytes, and the slurp read its members past it as a document of its own. jq's slurp
//! ends at the first fault of the text the files make, and its main loop reports that fault as a
//! parse error. Every expected string was captured from `/usr/bin/jq` 1.7.1, and jq 1.8.2 gives the
//! same.

use anyhow::Result;

use crate::run_jq_in_dir;

fn files() -> Result<tempfile::TempDir> {
    let dir = tempfile::tempdir()?;
    for (name, text) in [
        ("f.json", "[{0,\"s\"]\n"),
        ("g.json", "nul"),
        ("gn.json", "nul\n"),
        ("p.json", "[0,"),
        ("q3.json", "]"),
        ("q4.json", "\"s\"] "),
        ("a.json", "\"str0\"\n"),
        ("b.json", "{\"k\":[1,{\"z\":2}]2]}\n"),
        ("empty.json", ""),
    ] {
        std::fs::write(dir.path().join(name), text)?;
    }
    std::fs::create_dir(dir.path().join("d"))?;
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
fn a_slurp_over_a_directory_reports_the_first_fault_7755() -> Result<()> {
    check(&[
        (&["-s", "-c", ".", "f.json", "g.json", "d"], None, "", "jq: parse error: Objects must consist of key:value pairs at line 1, column 4\n", 5),
        (&["-s", "-c", ".", "d", "f.json", "g.json"], None, "", "jq: error: Is a directory\njq: parse error: Objects must consist of key:value pairs at line 1, column 4\n", 2),
        (&["-s", "-c", ".", "f.json", "d", "g.json"], None, "", "jq: parse error: Objects must consist of key:value pairs at line 1, column 4\n", 5),
        (&["-s", "-c", ".", "f.json", "gn.json", "d"], None, "", "jq: parse error: Objects must consist of key:value pairs at line 1, column 4\n", 5),
        (&["-s", "-c", ".", "p.json", "d", "q3.json", "q4.json"], None, "", "jq: error: Is a directory\njq: parse error: Expected another array element at line 1, column 4\n", 2),
        (&["-s", "-c", ".", "d", "a.json", "b.json"], None, "", "jq: error: Is a directory\njq: parse error: Expected separator between values at line 2, column 18\n", 2),
        (&["-s", "-c", ".", "a.json", "d", "b.json"], None, "", "jq: error: Is a directory\njq: parse error: Expected separator between values at line 2, column 18\n", 2),
        (&["-s", "-c", ".", "f.json", "g.json", "missing.json"], None, "", "jq: parse error: Objects must consist of key:value pairs at line 1, column 4\n", 5),
        (&["-s", "-c", ".", "empty.json", "a.json", "b.json"], None, "", "jq: parse error: Expected separator between values at line 2, column 18\n", 5),
        (&["-s", "-c", "length", "f.json", "g.json", "d"], None, "", "jq: parse error: Objects must consist of key:value pairs at line 1, column 4\n", 5),
        (&["-s", "-c", "length", "d", "f.json", "g.json"], None, "", "jq: error: Is a directory\njq: parse error: Objects must consist of key:value pairs at line 1, column 4\n", 2),
        (&["-s", "-c", "length", "f.json", "d", "g.json"], None, "", "jq: parse error: Objects must consist of key:value pairs at line 1, column 4\n", 5),
        (&["-s", "-c", "length", "f.json", "gn.json", "d"], None, "", "jq: parse error: Objects must consist of key:value pairs at line 1, column 4\n", 5),
        (&["-s", "-c", "length", "p.json", "d", "q3.json", "q4.json"], None, "", "jq: error: Is a directory\njq: parse error: Expected another array element at line 1, column 4\n", 2),
        (&["-s", "-c", "length", "d", "a.json", "b.json"], None, "", "jq: error: Is a directory\njq: parse error: Expected separator between values at line 2, column 18\n", 2),
        (&["-s", "-c", "length", "a.json", "d", "b.json"], None, "", "jq: error: Is a directory\njq: parse error: Expected separator between values at line 2, column 18\n", 2),
        (&["-s", "-c", "length", "f.json", "g.json", "missing.json"], None, "", "jq: parse error: Objects must consist of key:value pairs at line 1, column 4\n", 5),
        (&["-s", "-c", "length", "empty.json", "a.json", "b.json"], None, "", "jq: parse error: Expected separator between values at line 2, column 18\n", 5),
        (&["-s", "-c", "input_filename", "f.json", "g.json", "d"], None, "", "jq: parse error: Objects must consist of key:value pairs at line 1, column 4\n", 5),
        (&["-s", "-c", "input_filename", "d", "f.json", "g.json"], None, "", "jq: error: Is a directory\njq: parse error: Objects must consist of key:value pairs at line 1, column 4\n", 2),
        (&["-s", "-c", "input_filename", "f.json", "d", "g.json"], None, "", "jq: parse error: Objects must consist of key:value pairs at line 1, column 4\n", 5),
        (&["-s", "-c", "input_filename", "f.json", "gn.json", "d"], None, "", "jq: parse error: Objects must consist of key:value pairs at line 1, column 4\n", 5),
        (&["-s", "-c", "input_filename", "p.json", "d", "q3.json", "q4.json"], None, "", "jq: error: Is a directory\njq: parse error: Expected another array element at line 1, column 4\n", 2),
        (&["-s", "-c", "input_filename", "d", "a.json", "b.json"], None, "", "jq: error: Is a directory\njq: parse error: Expected separator between values at line 2, column 18\n", 2),
        (&["-s", "-c", "input_filename", "a.json", "d", "b.json"], None, "", "jq: error: Is a directory\njq: parse error: Expected separator between values at line 2, column 18\n", 2),
        (&["-s", "-c", "input_filename", "f.json", "g.json", "missing.json"], None, "", "jq: parse error: Objects must consist of key:value pairs at line 1, column 4\n", 5),
        (&["-s", "-c", "input_filename", "empty.json", "a.json", "b.json"], None, "", "jq: parse error: Expected separator between values at line 2, column 18\n", 5),
    ])
}
