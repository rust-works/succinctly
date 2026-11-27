//! #5654, re-pinned by #7966: a NUL byte outside a string is a parse fault at its own line and
//! column (ADR-0018's #7963 amendment E2), worded `Invalid character` as jq master words it
//! (unreleased; commit fbee8f4, jqlang/jq#3606). The wording and the columns were captured from a
//! driver built over jq master's `jv_parse.c`, which has no release yet.
//!
//! jq 1.7.1 truncates its input at a NUL in a read chunk that holds no newline and exits 0 having
//! dropped the rest; jq 1.8.2 reads through the NUL (CVE-2026-33948), so `[1`, a NUL, `,2]` is
//! `[1,2]` there. `succinctly jq` follows neither, so a row with no newline differs from both
//! releases. With a newline in the chunk both releases refuse the NUL as a token byte
//! (`Invalid numeric literal`) one column later than the NUL's own, where `succinctly jq` names the
//! NUL itself. The jq columns are captured with `printf '<input>' | jq -c .` from jq 1.7.1
//! (`/usr/bin/jq`) and jq 1.8.2 (`/opt/homebrew/bin/jq`); the doc is
//! `docs/compliance/jq/limitations.md` § "A NUL byte outside a string is a parse error (#7966)".
//!
//! | input        | jq 1.7.1                                        | jq 1.8.2                                              |
//! |--------------|-------------------------------------------------|-------------------------------------------------------|
//! | `\0[1]`      | nothing, exit 0                                 | `Invalid numeric literal at line 1, column 2`, exit 5 |
//! | `[1]\0`      | `[1]`, exit 0                                   | `[1]`, `Invalid numeric literal at EOF`, exit 5       |
//! | `[1]\0[2]`   | `[1]`, exit 0                                   | `[1]`, `Invalid numeric literal` at column 5, exit 5  |
//! | `[1\0,2]`    | `Unfinished JSON term at EOF` at column 2       | `[1,2]`, exit 0                                       |
//! | `[1,2\0]`    | `Unfinished JSON term at EOF` at column 4       | `[1,2]`, exit 0                                       |
//! | `[1]\0[2]\n` | `[1]`, `Invalid numeric literal` at column 5    | the same                                              |

use anyhow::Result;

use crate::run_jq_full;

/// `(stdin, our stdout, our exit, our stderr)`.
const ROWS: &[(&str, &str, i32, &str)] = &[
    (
        "\0[1]",
        "",
        5,
        "jq: parse error: Invalid character at line 1, column 1",
    ),
    (
        "[1]\0",
        "[1]\n",
        5,
        "jq: parse error: Invalid character at line 1, column 4",
    ),
    (
        "[1]\0[2]",
        "[1]\n",
        5,
        "jq: parse error: Invalid character at line 1, column 4",
    ),
    (
        "[1\0,2]",
        "",
        5,
        "jq: parse error: Invalid character at line 1, column 3",
    ),
    (
        "[1,2\0]",
        "",
        5,
        "jq: parse error: Invalid character at line 1, column 5",
    ),
    // A newline in the chunk: the same fault at the same column, where both releases name the
    // byte after the NUL (`Invalid numeric literal at line 1, column 5`).
    (
        "[1]\0[2]\n",
        "[1]\n",
        5,
        "jq: parse error: Invalid character at line 1, column 4",
    ),
];

#[test]
fn a_nul_byte_outside_a_string_is_a_parse_error_at_its_own_column_7966() -> Result<()> {
    for (input, stdout, exit, stderr) in ROWS {
        let (out, err, code) = run_jq_full(&["-c", "."], Some(input))?;
        assert_eq!(
            (out.as_str(), code, err.trim_end()),
            (*stdout, *exit, *stderr),
            "{input:?}"
        );
    }
    Ok(())
}

/// Whether the read holds a newline or not, the answer is the same: the NUL's own fault.
#[test]
fn a_newline_after_the_nul_changes_nothing_7966() -> Result<()> {
    for (input, ..) in ROWS {
        if input.ends_with('\n') {
            continue;
        }
        let with_newline = format!("{input}\n");
        assert_eq!(
            run_jq_full(&["-c", "."], Some(&with_newline))?,
            run_jq_full(&["-c", "."], Some(input))?,
            "{input:?}"
        );
    }
    Ok(())
}
