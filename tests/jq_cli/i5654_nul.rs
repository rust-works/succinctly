//! #5654: jq 1.7.1 truncates its input at a NUL byte in a read chunk that holds no newline, and
//! exits 0 having dropped the rest; `succinctly jq` reports the NUL as a parse error instead,
//! which stands as a divergence under ADR-0018 (the input is discarded). With a newline in the
//! chunk jq passes the NUL to its parser and refuses it as we do. The `jq` column in each row is
//! captured from jq 1.7.1 (`/usr/bin/jq`) with `printf '<input>' | jq -c .`, and the doc is
//! `docs/compliance/jq/limitations.md` § "A NUL byte truncates jq's input ...".

use anyhow::Result;

use crate::run_jq_full;

/// `(stdin, jq's stdout, jq's exit, our stdout, our exit, our stderr)`.
const ROWS: &[(&str, &str, i32, &str, i32, &str)] = &[
    (
        "\0[1]",
        "",
        0,
        "",
        5,
        "jq: parse error: Invalid numeric literal at line 1, column 2",
    ),
    (
        "[1]\0",
        "[1]\n",
        0,
        "[1]\n",
        5,
        "jq: parse error: Invalid numeric literal at EOF at line 1, column 4",
    ),
    (
        "[1]\0[2]",
        "[1]\n",
        0,
        "[1]\n",
        5,
        "jq: parse error: Invalid numeric literal at line 1, column 5",
    ),
    (
        "[1\0,2]",
        "",
        5,
        "",
        5,
        "jq: error (at <stdin>:0): Invalid JSON text: expected ',' or ']', found '\\0'",
    ),
    (
        "[1,2\0]",
        "",
        5,
        "",
        5,
        "jq: error (at <stdin>:0): Invalid JSON text: expected ',' or ']', found '\\0'",
    ),
    // A newline in the chunk: jq hands the NUL to its parser and refuses it, as we do.
    (
        "[1]\0[2]\n",
        "[1]\n",
        5,
        "[1]\n",
        5,
        "jq: parse error: Invalid numeric literal at line 1, column 5",
    ),
];

#[test]
fn a_nul_byte_is_a_parse_error_where_jq_truncates_its_input_5654() -> Result<()> {
    for (input, _jq_stdout, _jq_exit, stdout, exit, stderr) in ROWS {
        let (out, err, code) = run_jq_full(&["-c", "."], Some(input))?;
        assert_eq!(
            (out.as_str(), code, err.trim_end()),
            (*stdout, *exit, *stderr),
            "{input:?}"
        );
    }
    Ok(())
}

/// With a newline in the chunk jq does not truncate, so its answer is ours; without one it either
/// drops the input after the NUL with exit 0 or reports the fault in its own words.
#[test]
fn a_newline_in_the_chunk_makes_jq_agree_5654() {
    for (input, jq_stdout, jq_exit, stdout, exit, _) in ROWS {
        if input.contains('\n') {
            assert_eq!((jq_stdout, jq_exit), (stdout, exit), "{input:?}");
        } else {
            assert!(
                (jq_stdout, jq_exit) != (stdout, exit) || *exit == 5,
                "{input:?}: a silent success on both sides would not be a divergence"
            );
        }
    }
}
