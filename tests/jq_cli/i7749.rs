//! #7749, re-pinned by #7966: jq 1.7.1 and 1.8.2 read a NUL glued to a number token into the
//! token, up to the next byte that ends one, and their number parser stops at the NUL: an error jq
//! raises when the token ends was at that byte's column (`[[1]1`, NUL, `]` was `Expected separator
//! between values` at column 7), `1`, NUL, `}` was `Unmatched '}'` at column 3, and `[1`, NUL, `,2]`
//! was `[1,2]`. Under ADR-0018's #7963 amendment E2 the NUL ends the token and is itself the fault,
//! as jq master (unreleased; commit fbee8f4, jqlang/jq#3606) reads it: the token before it is
//! checked there (`[[1]1` is `Expected separator between values` at column 6, the NUL's), and
//! otherwise the NUL is `Invalid character` at its own column. Every first fault below was checked
//! against a driver over master's `jv_parse.c`, with and without `JV_PARSE_STREAMING`; no row here
//! matches either release.

use anyhow::Result;

use crate::{run_jq_full, run_jq_over_byte_files};

/// `(input, [-c ., -c -s ., -c --stream .] as (stdout, stderr), exit)`: every row exits 5.
type Row = (&'static str, [(&'static str, &'static str); 3]);

const ROWS: &[Row] = &[
    (
        "[[1]1\0]\n",
        [
            (
                "",
                "jq: parse error: Expected separator between values at line 1, column 6\n",
            ),
            (
                "",
                "jq: parse error: Expected separator between values at line 1, column 6\n",
            ),
            (
                "[[0,0],1]\n[[0,0]]\n",
                "jq: parse error: Expected separator between values at line 1, column 6\n",
            ),
        ],
    ),
    (
        "{\"a\":[true,null]12\0}\n",
        [
            (
                "",
                "jq: parse error: Expected separator between values at line 1, column 19\n",
            ),
            (
                "",
                "jq: parse error: Expected separator between values at line 1, column 19\n",
            ),
            (
                "[[\"a\",0],true]\n[[\"a\",1],null]\n[[\"a\",1]]\n",
                "jq: parse error: Expected separator between values at line 1, column 19\n",
            ),
        ],
    ),
    (
        "1\0}\n",
        [
            (
                "",
                "jq: parse error: Invalid character at line 1, column 2\n",
            ),
            (
                "",
                "jq: parse error: Invalid character at line 1, column 2\n",
            ),
            (
                "",
                "jq: parse error: Invalid character at line 1, column 2\n",
            ),
        ],
    ),
    (
        "1\0]\n",
        [
            (
                "",
                "jq: parse error: Invalid character at line 1, column 2\n",
            ),
            (
                "",
                "jq: parse error: Invalid character at line 1, column 2\n",
            ),
            (
                "",
                "jq: parse error: Invalid character at line 1, column 2\n",
            ),
        ],
    ),
    (
        "[1\0}\n",
        [
            (
                "",
                "jq: parse error: Invalid character at line 1, column 3\n",
            ),
            (
                "",
                "jq: parse error: Invalid character at line 1, column 3\n",
            ),
            (
                "",
                "jq: parse error: Invalid character at line 1, column 3\n",
            ),
        ],
    ),
    (
        "{\"a\":1\0]\n",
        [
            (
                "",
                "jq: parse error: Invalid character at line 1, column 7\n",
            ),
            (
                "",
                "jq: parse error: Invalid character at line 1, column 7\n",
            ),
            (
                "",
                "jq: parse error: Invalid character at line 1, column 7\n",
            ),
        ],
    ),
    (
        "12\0x:\n",
        [
            (
                "",
                "jq: parse error: Invalid character at line 1, column 3\n",
            ),
            (
                "",
                "jq: parse error: Invalid character at line 1, column 3\n",
            ),
            (
                "",
                "jq: parse error: Invalid character at line 1, column 3\n",
            ),
        ],
    ),
    (
        "\"s\"1\0x}\n",
        [
            (
                "\"s\"\n",
                "jq: parse error: Invalid character at line 1, column 5\n",
            ),
            (
                "",
                "jq: parse error: Invalid character at line 1, column 5\n",
            ),
            (
                "[[],\"s\"]\n",
                "jq: parse error: Invalid character at line 1, column 5\n",
            ),
        ],
    ),
    (
        "[1,2\0x\0y 3]\n",
        [
            (
                "",
                "jq: parse error: Invalid character at line 1, column 5\n",
            ),
            (
                "",
                "jq: parse error: Invalid character at line 1, column 5\n",
            ),
            (
                "[[0],1]\n",
                "jq: parse error: Invalid character at line 1, column 5\n",
            ),
        ],
    ),
    (
        "[1\0 2]\n",
        [
            (
                "",
                "jq: parse error: Invalid character at line 1, column 3\n",
            ),
            (
                "",
                "jq: parse error: Invalid character at line 1, column 3\n",
            ),
            (
                "",
                "jq: parse error: Invalid character at line 1, column 3\n",
            ),
        ],
    ),
    (
        "1\0x]\n2\n",
        [
            (
                "",
                "jq: parse error: Invalid character at line 1, column 2\n",
            ),
            (
                "",
                "jq: parse error: Invalid character at line 1, column 2\n",
            ),
            (
                "",
                "jq: parse error: Invalid character at line 1, column 2\n",
            ),
        ],
    ),
    (
        "[1\0,2]\n1.5\0e}\n",
        [
            (
                "",
                "jq: parse error: Invalid character at line 1, column 3\n",
            ),
            (
                "",
                "jq: parse error: Invalid character at line 1, column 3\n",
            ),
            (
                "",
                "jq: parse error: Invalid character at line 1, column 3\n",
            ),
        ],
    ),
    (
        "{\"a\":-1\0x\"b\"}\n",
        [
            (
                "",
                "jq: parse error: Invalid character at line 1, column 8\n",
            ),
            (
                "",
                "jq: parse error: Invalid character at line 1, column 8\n",
            ),
            (
                "",
                "jq: parse error: Invalid character at line 1, column 8\n",
            ),
        ],
    ),
    (
        "nan\0}\n",
        [
            (
                "",
                "jq: parse error: Invalid character at line 1, column 4\n",
            ),
            (
                "",
                "jq: parse error: Invalid character at line 1, column 4\n",
            ),
            (
                "",
                "jq: parse error: Invalid character at line 1, column 4\n",
            ),
        ],
    ),
    (
        "[[1]1\0x\n",
        [
            (
                "",
                "jq: parse error: Expected separator between values at line 1, column 6\n",
            ),
            (
                "",
                "jq: parse error: Expected separator between values at line 1, column 6\n",
            ),
            (
                "[[0,0],1]\n[[0,0]]\n",
                "jq: parse error: Expected separator between values at line 1, column 6\n",
            ),
        ],
    ),
];

#[test]
fn a_nul_ends_its_token_and_is_the_fault_7966() -> Result<()> {
    let routes: [&[&str]; 3] = [&["-c", "."], &["-c", "-s", "."], &["-c", "--stream", "."]];
    for (input, answers) in ROWS {
        for (args, (stdout, stderr)) in routes.iter().zip(answers) {
            // `-s` over standard input words a fault inside a container in the splitter's words
            // (`Invalid JSON text: expected ',' or ']', found ...`), a NUL or any other byte (#7762
            // covered the missing separator; `[1 }]` still reads so); the file route below reads
            // those rows.
            let slurp_gap = args.contains(&"-s") && input.starts_with(['[', '{']);
            let (out, err, code) = run_jq_full(args, Some(input))?;
            if !slurp_gap {
                assert_eq!(
                    (out.as_str(), err.as_str(), code),
                    (*stdout, *stderr, 5),
                    "stdin {args:?} {input:?}"
                );
            }
            let (out, err, code, _) = run_jq_over_byte_files(args, &[input.as_bytes()])?;
            assert_eq!(
                (out.as_str(), err.as_str(), code),
                (*stdout, *stderr, 5),
                "file {args:?} {input:?}"
            );
        }
    }
    Ok(())
}

/// A number token that starts in one file and has its NUL in the next (jq reads the files as one
/// stream): `(first file, second file, -c . stdout, stderr, exit)`. Both releases read `[12]`,
/// `[123]` and `[1,2,3]` for the second, fourth and sixth rows; the NUL is the fault there now, at
/// its column in the stream, the first file's bytes counted.
const ACROSS_FILES: &[(&str, &str, &str, &str, i32)] = &[
    (
        "[[1]1",
        "\0]\n",
        "",
        "jq: parse error: Expected separator between values at line 1, column 6\n",
        5,
    ),
    (
        "[1",
        "2\0x]\n",
        "",
        "jq: parse error: Invalid character at line 1, column 4\n",
        5,
    ),
    (
        "1",
        "\0}\n",
        "",
        "jq: parse error: Invalid character at line 1, column 2\n",
        5,
    ),
    (
        "[123",
        "\0]\n",
        "",
        "jq: parse error: Invalid character at line 1, column 5\n",
        5,
    ),
    (
        "{\"a\":[true,null]1",
        "2\0}\n",
        "",
        "jq: parse error: Expected separator between values at line 1, column 19\n",
        5,
    ),
    (
        "[1,2",
        "\0x,3]\n",
        "",
        "jq: parse error: Invalid character at line 1, column 5\n",
        5,
    ),
];

#[test]
fn a_nul_after_a_number_from_an_earlier_file_is_the_fault_7966() -> Result<()> {
    for (first, second, stdout, stderr, exit) in ACROSS_FILES {
        for args in [&["-c", "."][..], &["-n", "-c", "inputs"]] {
            let (out, err, code, paths) =
                run_jq_over_byte_files(args, &[first.as_bytes(), second.as_bytes()])?;
            // `-n inputs` names the file in its error (`jq: error (at FILE:1): ...`).
            let err = err.replace(
                &format!("jq: error (at {}:1): ", paths[1]),
                "jq: parse error: ",
            );
            assert_eq!(
                (out.as_str(), err.as_str(), code),
                (*stdout, *stderr, *exit),
                "{args:?} {first:?} {second:?}"
            );
        }
    }
    Ok(())
}
