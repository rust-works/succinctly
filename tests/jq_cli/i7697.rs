//! #7697: a program the path-mode interpreter runs keeps the last value's document to the end of
//! the run, which the CLI then forgets instead of taking its tree apart, unless the program can
//! walk it with `..`. Nothing a program sees
//! moves: these pin the answers on the routes the held document passes through (one document,
//! several on one stream or over several files, a write, a stream of paths, an error on every
//! value and a halt on the last). Every expectation was captured from `/usr/bin/jq` 1.7.1, which
//! jq 1.8.2 answers the same.

use anyhow::Result;

use crate::{run_jq_full, run_jq_over_files};

/// A document whose first and last members are not the one the program reads, so `.data` lets
/// go of the rest as it reads it.
const DOC: &str = r#"{"meta":{"n":[1,2],"s":["x","y"]},"data":[{"name":"a"},{"x":1},{"name":"b"}],"tail":{"k":"v"}}
"#;

/// The same document after two others on one stream.
const THREE: &str = r#"{"data":[{"name":"p"}],"z":[3]}
{"data":[]}
{"meta":{"n":[1,2],"s":["x","y"]},"data":[{"name":"a"},{"x":1},{"name":"b"}],"tail":{"k":"v"}}
"#;

#[test]
fn test_7697_the_kept_document_moves_no_answer() -> Result<()> {
    let rows: &[(&str, &str, &str, &str, i32)] = &[
        (
            "[path(.data[] | .name // empty)] | length",
            DOC,
            "2\n",
            "",
            0,
        ),
        (
            "[path(.data[] | .name // empty)]",
            THREE,
            "[[\"data\",0,\"name\"]]\n[]\n[[\"data\",0,\"name\"],[\"data\",2,\"name\"]]\n",
            "",
            0,
        ),
        (
            "(.data[] | .name // empty) |= 1 | .data",
            DOC,
            "[{\"name\":1},{\"x\":1},{\"name\":1}]\n",
            "",
            0,
        ),
        (
            r#"path(.. | if type == "string" then . else empty end)"#,
            DOC,
            "[\"meta\",\"s\",0]\n[\"meta\",\"s\",1]\n[\"data\",0,\"name\"]\n[\"data\",2,\"name\"]\n[\"tail\",\"k\"]\n",
            "",
            0,
        ),
        // A walk with `..` keeps nothing; its answer is unchanged too.
        (
            r#"[path(.. | if type == "string" then . else empty end)] | length"#,
            DOC,
            "5\n",
            "",
            0,
        ),
        (
            "[path(.data[] | .name // empty)] | error",
            THREE,
            "",
            "jq: error (at <stdin>:1) (not a string): [[\"data\",0,\"name\"]]\n\
             jq: error (at <stdin>:2) (not a string): []\n\
             jq: error (at <stdin>:3) (not a string): [[\"data\",0,\"name\"],[\"data\",2,\"name\"]]\n",
            5,
        ),
        (
            "if .data then (.data[] | .name // empty) |= halt_error else . end",
            DOC,
            "",
            "a",
            5,
        ),
    ];
    for &(filter, input, stdout, stderr, code) in rows {
        let (out, err, exit) = run_jq_full(&["-c", filter], Some(input))?;
        assert_eq!(
            (out.as_str(), err.as_str(), exit),
            (stdout, stderr, code),
            "{filter}"
        );
    }
    Ok(())
}

/// Over two files, only the last value of the run is kept: each file's answers are its own.
#[test]
fn test_7697_the_kept_document_is_the_last_files_last_value() -> Result<()> {
    let (out, err, exit, _) =
        run_jq_over_files(&["-c", "[path(.data[] | .name // empty)]"], &[THREE, DOC])?;
    assert_eq!(
        (out.as_str(), err.as_str(), exit),
        (
            "[[\"data\",0,\"name\"]]\n[]\n[[\"data\",0,\"name\"],[\"data\",2,\"name\"]]\n\
             [[\"data\",0,\"name\"],[\"data\",2,\"name\"]]\n",
            "",
            0
        )
    );
    Ok(())
}
