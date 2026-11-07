//! #6501: a `--seq` stream that is RFC 7464's own framing, every record `RS value LF` with the value
//! one RFC 8259 document, is proved clean by a record check and read as the plain JSON stream it
//! becomes with each RS blanked, without the walk of jq's reader. Every stream the check does not
//! vouch for keeps the walk. These rows sit on each guard of the check (bytes before the first RS,
//! no final newline, an RS straight after a number, a blank line, an empty record) and on records
//! that are not RFC 8259 (a leading zero, `nan`), and give what jq 1.7.1 (`/usr/bin/jq`) answers:
//! `(args, stdout, stderr, exit status)`. The stdout, the warnings and the exit status must be what
//! they were before the check existed, whichever route reads the stream.

use anyhow::Result;

use crate::run_jq_full;

type Row = (&'static [&'static str], &'static str, &'static str, i32);

/// `(name, stream, rows)`.
const STREAMS: &[(&str, &str, &[Row])] = &[
    (
        "before_the_first_rs",
        "false\n",
        &[
            (&["-c", "."], "", "jq: ignoring parse error: Unfinished abandoned text at EOF at line 2, column 0\n", 0),
            (&["-s", "-c", "."], "\x1e[]\n", "jq: ignoring parse error: Unfinished abandoned text at EOF at line 2, column 0\n", 0),
        ],
    ),
    (
        "no_final_newline",
        "\x1e12.5",
        &[
            (&["-c", "."], "", "jq: ignoring parse error: Potentially truncated top-level numeric value at EOF at line 1, column 5\n", 0),
            (&["-s", "-c", "."], "\x1e[]\n", "jq: ignoring parse error: Potentially truncated top-level numeric value at EOF at line 1, column 5\n", 0),
        ],
    ),
    (
        "rs_after_a_number",
        "\x1e1\x1e",
        &[
            (&["-c", "."], "", "jq: ignoring parse error: Potentially truncated top-level numeric value at line 1, column 3\n", 0),
            (&["-s", "-c", "."], "\x1e[]\n", "jq: ignoring parse error: Potentially truncated top-level numeric value at line 1, column 3\n", 0),
        ],
    ),
    (
        "blank_line_between",
        "\x1e{\"a\":1}\n\n\x1e2\n",
        &[
            (&["-c", "."], "\x1e{\"a\":1}\n\x1e2\n", "", 0),
            (&["-s", "-c", "."], "\x1e[{\"a\":1},2]\n", "", 0),
        ],
    ),
    (
        "space_before_newline",
        "\x1e1 \n\x1e2\n",
        &[
            (&["-c", "."], "\x1e1\n\x1e2\n", "", 0),
            (&["-s", "-c", "."], "\x1e[1,2]\n", "", 0),
        ],
    ),
    (
        "empty_record",
        "\x1e{\"a\":1}\n\x1e\n",
        &[
            (&["-c", "."], "\x1e{\"a\":1}\n", "", 0),
            (&["-s", "-c", "."], "\x1e[{\"a\":1}]\n", "", 0),
        ],
    ),
    (
        "leading_zero",
        "\x1e007\n\x1e8\n",
        &[
            (&["-c", "."], "\x1e7\n\x1e8\n", "", 0),
            (&["-s", "-c", "."], "\x1e[7,8]\n", "", 0),
        ],
    ),
    (
        "nan_record",
        "\x1enan\n\x1e1\n",
        &[
            (&["-c", "."], "\x1enull\n\x1e1\n", "", 0),
            (&["-s", "-c", "."], "\x1e[null,1]\n", "", 0),
        ],
    ),
    (
        "two_records_clean",
        "\x1e{\"a\":[1,{\"b\":null}]}\n\x1e\"é\\u00e9\"\n",
        &[
            (&["-c", "."], "\x1e{\"a\":[1,{\"b\":null}]}\n\x1e\"éé\"\n", "", 0),
            (&["-s", "-c", "."], "\x1e[{\"a\":[1,{\"b\":null}]},\"éé\"]\n", "", 0),
        ],
    ),
];

#[test]
fn seq_streams_at_the_record_checks_guards_answer_as_jq_does_6501() -> Result<()> {
    for (name, stream, rows) in STREAMS {
        for (args, stdout, stderr, status) in *rows {
            let mut argv = vec!["--seq"];
            argv.extend_from_slice(args);
            let (got_out, got_err, got_status) = run_jq_full(&argv, Some(stream))?;
            assert_eq!(got_out, *stdout, "{name}: stdout of {args:?}");
            assert_eq!(got_err, *stderr, "{name}: stderr of {args:?}");
            assert_eq!(got_status, *status, "{name}: exit status of {args:?}");
        }
    }
    Ok(())
}
