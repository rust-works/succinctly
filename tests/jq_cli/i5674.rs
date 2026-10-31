//! #5674: `--seq` input that is clean (one source, valid UTF-8, no byte-order mark, no diagnostic
//! from jq's reader, nothing but whitespace and RS between the values) is read as the plain JSON
//! stream it becomes with each RS blanked, by the route NDJSON takes, instead of a per-record
//! index and owned value. Every row here is a stream, a program and what jq 1.7.1 (`/usr/bin/jq`)
//! answers: `(args, stdout, stderr, exit status)`. The records, the RS before each output, and the
//! `(at <stdin>:N)` a runtime error names have to survive the change of route, and the streams
//! the reader warns about (a truncated record, junk after a value) have to keep the route that
//! prints those warnings.

use anyhow::Result;

use crate::run_jq_full;

type Row = (&'static [&'static str], &'static str, &'static str, i32);

/// `(name, stream, rows)`.
const STREAMS: &[(&str, &str, &[Row])] = &[
    (
        "clean",
        "\x1e{\"a\":1,\"b\":[1,2]}\n\x1e{\"a\":2}\n\x1e[3]\n",
        &[
        (&["-c", "."], "\x1e{\"a\":1,\"b\":[1,2]}\n\x1e{\"a\":2}\n\x1e[3]\n", "", 0),
        (&["-r", ".a?"], "\x1e1\n\x1e2\n", "", 0),
        (&["-c", ".a|error"], "", "jq: error (at <stdin>:1) (not a string): 1\njq: error (at <stdin>:2) (not a string): 2\njq: error (at <stdin>:3): Cannot index array with string \"a\"\n", 5),
        (&["-s", "-c", "."], "\x1e[{\"a\":1,\"b\":[1,2]},{\"a\":2},[3]]\n", "", 0),
        (&["-s", "error(\"x\")"], "", "jq: error (at <stdin>:3): x\n", 5),
        (&["--tab", "."], "\x1e{\n\t\"a\": 1,\n\t\"b\": [\n\t\t1,\n\t\t2\n\t]\n}\n\x1e{\n\t\"a\": 2\n}\n\x1e[\n\t3\n]\n", "", 0),
        (&["-j", ".a?"], "\x1e1\x1e2", "", 0),
        (&["-c", "[.[]?]"], "\x1e[1,[1,2]]\n\x1e[2]\n\x1e[3]\n", "", 0),
        (&["-C", "-c", "."], "\x1e\x1b[1;39m{\x1b[0m\x1b[1;34m\"a\"\x1b[0m\x1b[1;39m:\x1b[0m\x1b[0;39m1\x1b[0m\x1b[1;39m,\x1b[0m\x1b[1;34m\"b\"\x1b[0m\x1b[1;39m:\x1b[0m\x1b[1;39m[\x1b[0;39m1\x1b[0m\x1b[1;39m,\x1b[0;39m2\x1b[0m\x1b[1;39m\x1b[1;39m]\x1b[0m\x1b[1;39m\x1b[1;39m}\x1b[0m\n\x1e\x1b[1;39m{\x1b[0m\x1b[1;34m\"a\"\x1b[0m\x1b[1;39m:\x1b[0m\x1b[0;39m2\x1b[0m\x1b[1;39m\x1b[1;39m}\x1b[0m\n\x1e\x1b[1;39m[\x1b[0;39m3\x1b[0m\x1b[1;39m\x1b[1;39m]\x1b[0m\n", "", 0),
        ],
    ),
    (
        "pretty",
        "\x1e{\n  \"a\": 1,\n  \"b\": {\"c\": [1,\n 2]}\n}\n\x1e{\"a\":\n 7}\n",
        &[
        (&["-c", "."], "\x1e{\"a\":1,\"b\":{\"c\":[1,2]}}\n\x1e{\"a\":7}\n", "", 0),
        (&["-r", ".a?"], "\x1e1\n\x1e7\n", "", 0),
        (&["-c", ".a|error"], "", "jq: error (at <stdin>:5) (not a string): 1\njq: error (at <stdin>:7) (not a string): 7\n", 5),
        (&["-s", "-c", "."], "\x1e[{\"a\":1,\"b\":{\"c\":[1,2]}},{\"a\":7}]\n", "", 0),
        (&["-s", "error(\"x\")"], "", "jq: error (at <stdin>:7): x\n", 5),
        (&["--tab", "."], "\x1e{\n\t\"a\": 1,\n\t\"b\": {\n\t\t\"c\": [\n\t\t\t1,\n\t\t\t2\n\t\t]\n\t}\n}\n\x1e{\n\t\"a\": 7\n}\n", "", 0),
        (&["-j", ".a?"], "\x1e1\x1e7", "", 0),
        (&["-c", "[.[]?]"], "\x1e[1,{\"c\":[1,2]}]\n\x1e[7]\n", "", 0),
        (&["-C", "-c", "."], "\x1e\x1b[1;39m{\x1b[0m\x1b[1;34m\"a\"\x1b[0m\x1b[1;39m:\x1b[0m\x1b[0;39m1\x1b[0m\x1b[1;39m,\x1b[0m\x1b[1;34m\"b\"\x1b[0m\x1b[1;39m:\x1b[0m\x1b[1;39m{\x1b[0m\x1b[1;34m\"c\"\x1b[0m\x1b[1;39m:\x1b[0m\x1b[1;39m[\x1b[0;39m1\x1b[0m\x1b[1;39m,\x1b[0;39m2\x1b[0m\x1b[1;39m\x1b[1;39m]\x1b[0m\x1b[1;39m\x1b[1;39m}\x1b[0m\x1b[1;39m\x1b[1;39m}\x1b[0m\n\x1e\x1b[1;39m{\x1b[0m\x1b[1;34m\"a\"\x1b[0m\x1b[1;39m:\x1b[0m\x1b[0;39m7\x1b[0m\x1b[1;39m\x1b[1;39m}\x1b[0m\n", "", 0),
        ],
    ),
    (
        "scalars",
        "\x1e1\n\x1e\"x\"\n\x1enull\n",
        &[
        (&["-c", "."], "\x1e1\n\x1e\"x\"\n\x1enull\n", "", 0),
        (&["-r", ".a?"], "\x1enull\n", "", 0),
        (&["-c", ".a|error"], "", "jq: error (at <stdin>:1): Cannot index number with string \"a\"\njq: error (at <stdin>:2): Cannot index string with string \"a\"\njq: error (at <stdin>:3) (not a string): null\n", 5),
        (&["-s", "-c", "."], "\x1e[1,\"x\",null]\n", "", 0),
        (&["-s", "error(\"x\")"], "", "jq: error (at <stdin>:3): x\n", 5),
        (&["--tab", "."], "\x1e1\n\x1e\"x\"\n\x1enull\n", "", 0),
        (&["-j", ".a?"], "\x1enull", "", 0),
        (&["-c", "[.[]?]"], "\x1e[]\n\x1e[]\n\x1e[]\n", "", 0),
        (&["-C", "-c", "."], "\x1e\x1b[0;39m1\x1b[0m\n\x1e\x1b[0;32m\"x\"\x1b[0m\n\x1e\x1b[0;90mnull\x1b[0m\n", "", 0),
        ],
    ),
    (
        "bare_last",
        "\x1e{\"a\":1}\n\x1e2",
        &[
        (&["-c", "."], "\x1e{\"a\":1}\n", "", 0),
        (&["-r", ".a?"], "\x1e1\n", "", 0),
        (&["-c", ".a|error"], "", "jq: error (at <stdin>:1) (not a string): 1\n", 5),
        (&["-s", "-c", "."], "\x1e[{\"a\":1}]\n", "jq: ignoring parse error: Potentially truncated top-level numeric value at EOF at line 2, column 2\n", 0),
        (&["-s", "error(\"x\")"], "", "jq: ignoring parse error: Potentially truncated top-level numeric value at EOF at line 2, column 2\njq: error (at <unknown>): x\n", 5),
        (&["--tab", "."], "\x1e{\n\t\"a\": 1\n}\n", "", 0),
        (&["-j", ".a?"], "\x1e1", "", 0),
        (&["-c", "[.[]?]"], "\x1e[1]\n", "", 0),
        (&["-C", "-c", "."], "\x1e\x1b[1;39m{\x1b[0m\x1b[1;34m\"a\"\x1b[0m\x1b[1;39m:\x1b[0m\x1b[0;39m1\x1b[0m\x1b[1;39m\x1b[1;39m}\x1b[0m\n", "", 0),
        ],
    ),
    (
        "no_newline",
        "\x1e{\"a\":1}\n\x1e{\"a\":2}",
        &[
        (&["-c", "."], "\x1e{\"a\":1}\n", "", 0),
        (&["-r", ".a?"], "\x1e1\n", "", 0),
        (&["-c", ".a|error"], "", "jq: error (at <stdin>:1) (not a string): 1\n", 5),
        (&["-s", "-c", "."], "\x1e[{\"a\":1},{\"a\":2}]\n", "", 0),
        (&["-s", "error(\"x\")"], "", "jq: error (at <stdin>:1): x\n", 5),
        (&["--tab", "."], "\x1e{\n\t\"a\": 1\n}\n", "", 0),
        (&["-j", ".a?"], "\x1e1", "", 0),
        (&["-c", "[.[]?]"], "\x1e[1]\n", "", 0),
        (&["-C", "-c", "."], "\x1e\x1b[1;39m{\x1b[0m\x1b[1;34m\"a\"\x1b[0m\x1b[1;39m:\x1b[0m\x1b[0;39m1\x1b[0m\x1b[1;39m\x1b[1;39m}\x1b[0m\n", "", 0),
        ],
    ),
    (
        "truncated",
        "\x1e{\"a\":1}\n\x1e{\"a\":\n\x1e{\"a\":3}\n",
        &[
        (&["-c", "."], "\x1e{\"a\":1}\n\x1e{\"a\":3}\n", "", 0),
        (&["-r", ".a?"], "\x1e1\n\x1e3\n", "", 0),
        (&["-c", ".a|error"], "", "jq: error (at <stdin>:1) (not a string): 1\njq: error (at <stdin>:3) (not a string): 3\n", 5),
        (&["-s", "-c", "."], "\x1e[{\"a\":1},{\"a\":3}]\n", "", 0),
        (&["-s", "error(\"x\")"], "", "jq: error (at <stdin>:3): x\n", 5),
        (&["--tab", "."], "\x1e{\n\t\"a\": 1\n}\n\x1e{\n\t\"a\": 3\n}\n", "", 0),
        (&["-j", ".a?"], "\x1e1\x1e3", "", 0),
        (&["-c", "[.[]?]"], "\x1e[1]\n\x1e[3]\n", "", 0),
        (&["-C", "-c", "."], "\x1e\x1b[1;39m{\x1b[0m\x1b[1;34m\"a\"\x1b[0m\x1b[1;39m:\x1b[0m\x1b[0;39m1\x1b[0m\x1b[1;39m\x1b[1;39m}\x1b[0m\n\x1e\x1b[1;39m{\x1b[0m\x1b[1;34m\"a\"\x1b[0m\x1b[1;39m:\x1b[0m\x1b[0;39m3\x1b[0m\x1b[1;39m\x1b[1;39m}\x1b[0m\n", "", 0),
        ],
    ),
    (
        "junk",
        "\x1e{\"a\":1}@\n\x1e{\"a\":2}\n",
        &[
        (&["-c", "."], "\x1e{\"a\":1}\n\x1e{\"a\":2}\n", "jq: ignoring parse error: Invalid numeric literal at line 2, column 0 (need RS to resync)\n", 0),
        (&["-r", ".a?"], "\x1e1\n\x1e2\n", "jq: ignoring parse error: Invalid numeric literal at line 2, column 0 (need RS to resync)\n", 0),
        (&["-s", "-c", "."], "\x1e[{\"a\":1},{\"a\":2}]\n", "jq: ignoring parse error: Invalid numeric literal at line 2, column 0 (need RS to resync)\n", 0),
        (&["-s", "error(\"x\")"], "", "jq: ignoring parse error: Invalid numeric literal at line 2, column 0 (need RS to resync)\njq: error (at <stdin>:2): x\n", 5),
        (&["--tab", "."], "\x1e{\n\t\"a\": 1\n}\n\x1e{\n\t\"a\": 2\n}\n", "jq: ignoring parse error: Invalid numeric literal at line 2, column 0 (need RS to resync)\n", 0),
        (&["-j", ".a?"], "\x1e1\x1e2", "jq: ignoring parse error: Invalid numeric literal at line 2, column 0 (need RS to resync)\n", 0),
        (&["-c", "[.[]?]"], "\x1e[1]\n\x1e[2]\n", "jq: ignoring parse error: Invalid numeric literal at line 2, column 0 (need RS to resync)\n", 0),
        (&["-C", "-c", "."], "\x1e\x1b[1;39m{\x1b[0m\x1b[1;34m\"a\"\x1b[0m\x1b[1;39m:\x1b[0m\x1b[0;39m1\x1b[0m\x1b[1;39m\x1b[1;39m}\x1b[0m\n\x1e\x1b[1;39m{\x1b[0m\x1b[1;34m\"a\"\x1b[0m\x1b[1;39m:\x1b[0m\x1b[0;39m2\x1b[0m\x1b[1;39m\x1b[1;39m}\x1b[0m\n", "jq: ignoring parse error: Invalid numeric literal at line 2, column 0 (need RS to resync)\n", 0),
        ],
    ),
    (
        "strings",
        "\x1e{\"a\":\"x\\\"y\\\\z\\n\"}\n\x1e{\"a\":\"é😀\"}\n",
        &[
        (&["-c", "."], "\x1e{\"a\":\"x\\\"y\\\\z\\n\"}\n\x1e{\"a\":\"é😀\"}\n", "", 0),
        (&["-r", ".a?"], "x\"y\\z\n\né😀\n", "", 0),
        (&["-c", ".a|error"], "", "jq: error (at <stdin>:1): x\"y\\z\n\njq: error (at <stdin>:2): é😀\n", 5),
        (&["-s", "-c", "."], "\x1e[{\"a\":\"x\\\"y\\\\z\\n\"},{\"a\":\"é😀\"}]\n", "", 0),
        (&["-s", "error(\"x\")"], "", "jq: error (at <stdin>:2): x\n", 5),
        (&["--tab", "."], "\x1e{\n\t\"a\": \"x\\\"y\\\\z\\n\"\n}\n\x1e{\n\t\"a\": \"é😀\"\n}\n", "", 0),
        (&["-j", ".a?"], "x\"y\\z\né😀", "", 0),
        (&["-c", "[.[]?]"], "\x1e[\"x\\\"y\\\\z\\n\"]\n\x1e[\"é😀\"]\n", "", 0),
        (&["-C", "-c", "."], "\x1e\x1b[1;39m{\x1b[0m\x1b[1;34m\"a\"\x1b[0m\x1b[1;39m:\x1b[0m\x1b[0;32m\"x\\\"y\\\\z\\n\"\x1b[0m\x1b[1;39m\x1b[1;39m}\x1b[0m\n\x1e\x1b[1;39m{\x1b[0m\x1b[1;34m\"a\"\x1b[0m\x1b[1;39m:\x1b[0m\x1b[0;32m\"é😀\"\x1b[0m\x1b[1;39m\x1b[1;39m}\x1b[0m\n", "", 0),
        ],
    ),
    (
        "crlf",
        "\x1e{\"a\":1}\r\n\x1e{\"a\":2}\r\n",
        &[
        (&["-c", "."], "\x1e{\"a\":1}\n\x1e{\"a\":2}\n", "", 0),
        (&["-r", ".a?"], "\x1e1\n\x1e2\n", "", 0),
        (&["-c", ".a|error"], "", "jq: error (at <stdin>:1) (not a string): 1\njq: error (at <stdin>:2) (not a string): 2\n", 5),
        (&["-s", "-c", "."], "\x1e[{\"a\":1},{\"a\":2}]\n", "", 0),
        (&["-s", "error(\"x\")"], "", "jq: error (at <stdin>:2): x\n", 5),
        (&["--tab", "."], "\x1e{\n\t\"a\": 1\n}\n\x1e{\n\t\"a\": 2\n}\n", "", 0),
        (&["-j", ".a?"], "\x1e1\x1e2", "", 0),
        (&["-c", "[.[]?]"], "\x1e[1]\n\x1e[2]\n", "", 0),
        (&["-C", "-c", "."], "\x1e\x1b[1;39m{\x1b[0m\x1b[1;34m\"a\"\x1b[0m\x1b[1;39m:\x1b[0m\x1b[0;39m1\x1b[0m\x1b[1;39m\x1b[1;39m}\x1b[0m\n\x1e\x1b[1;39m{\x1b[0m\x1b[1;34m\"a\"\x1b[0m\x1b[1;39m:\x1b[0m\x1b[0;39m2\x1b[0m\x1b[1;39m\x1b[1;39m}\x1b[0m\n", "", 0),
        ],
    ),
];

#[test]
fn seq_streams_answer_as_jq_does_5674() -> Result<()> {
    for (name, stream, rows) in STREAMS {
        for (args, stdout, stderr, status) in *rows {
            let mut all = vec!["--seq"];
            all.extend_from_slice(args);
            let got = run_jq_full(&all, Some(stream))?;
            assert_eq!(
                got,
                ((*stdout).to_string(), (*stderr).to_string(), *status),
                "{name}: {args:?}"
            );
        }
    }
    Ok(())
}

/// The same streams read from a file: the output is the stdin run's, and a runtime error names
/// the file where the stdin run names `<stdin>`.
#[test]
fn seq_streams_from_a_file_answer_as_from_stdin_5674() -> Result<()> {
    let dir = tempfile::tempdir()?;
    for (name, stream, rows) in STREAMS {
        let path = dir.path().join(format!("{name}.seq"));
        std::fs::write(&path, stream)?;
        let path = path.to_string_lossy().to_string();
        for (args, stdout, stderr, status) in *rows {
            let mut all = vec!["--seq"];
            all.extend_from_slice(args);
            all.push(&path);
            let got = run_jq_full(&all, None)?;
            assert_eq!(
                (got.0.as_str(), got.2),
                (*stdout, *status),
                "{name}: {args:?}"
            );
            assert_eq!(got.1.replace(&path, "<stdin>"), *stderr, "{name}: {args:?}");
        }
    }
    Ok(())
}

/// Two sources are one stream to jq, so they keep the route that joins them: a record can span
/// the seam, and the output is the one stream's.
#[test]
fn seq_stream_across_two_files_5674() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let (one, two) = (dir.path().join("one.seq"), dir.path().join("two.seq"));
    std::fs::write(&one, "\x1e{\"a\":1}\n\x1e{\"a\":")?;
    std::fs::write(&two, "2}\n\x1e[3]\n")?;
    let (one, two) = (one.to_string_lossy(), two.to_string_lossy());
    let got = run_jq_full(&["--seq", "-c", ".", &one, &two], None)?;
    assert_eq!(
        got,
        (
            "\x1e{\"a\":1}\n\x1e{\"a\":2}\n\x1e[3]\n".to_string(),
            String::new(),
            0
        )
    );
    Ok(())
}
