//! #7749: a NUL glued to a number token ends the number for jq's number parser, but not the token:
//! jq's tokenizer reads the NUL and what follows it into the token up to the next byte that ends a
//! token, and acts on the token there. So an error jq raises when the token ends (`Expected
//! separator between values`, an unmatched closer) is at that byte's column, and `1`, NUL, `}` is
//! that error alone, with no `1` printed first. #7437 blanked the tail to spaces, which ended the
//! token at the NUL. Every row was captured from jq 1.7.1 (`/usr/bin/jq`), and jq 1.8.2
//! (`/opt/homebrew/bin/jq`) gives the same answer for each.

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
                "jq: parse error: Expected separator between values at line 1, column 7\n",
            ),
            (
                "",
                "jq: parse error: Expected separator between values at line 1, column 7\n",
            ),
            (
                "[[0,0],1]\n[[0,0]]\n",
                "jq: parse error: Expected separator between values at line 1, column 7\n",
            ),
        ],
    ),
    (
        "{\"a\":[true,null]12\0}\n",
        [
            (
                "",
                "jq: parse error: Expected separator between values at line 1, column 20\n",
            ),
            (
                "",
                "jq: parse error: Expected separator between values at line 1, column 20\n",
            ),
            (
                "[[\"a\",0],true]\n[[\"a\",1],null]\n[[\"a\",1]]\n",
                "jq: parse error: Expected separator between values at line 1, column 20\n",
            ),
        ],
    ),
    (
        "1\0}\n",
        [
            ("", "jq: parse error: Unmatched '}' at line 1, column 3\n"),
            ("", "jq: parse error: Unmatched '}' at line 1, column 3\n"),
            (
                "",
                "jq: parse error: Unmatched '}' at the top-level at line 1, column 3\n",
            ),
        ],
    ),
    (
        "1\0]\n",
        [
            ("", "jq: parse error: Unmatched ']' at line 1, column 3\n"),
            ("", "jq: parse error: Unmatched ']' at line 1, column 3\n"),
            (
                "",
                "jq: parse error: Unmatched ']' at the top-level at line 1, column 3\n",
            ),
        ],
    ),
    (
        "[1\0}\n",
        [
            (
                "",
                "jq: parse error: Objects must consist of key:value pairs at line 1, column 4\n",
            ),
            (
                "",
                "jq: parse error: Objects must consist of key:value pairs at line 1, column 4\n",
            ),
            (
                "",
                "jq: parse error: Unmatched '}' in the middle of an array at line 1, column 4\n",
            ),
        ],
    ),
    (
        "{\"a\":1\0]\n",
        [
            ("", "jq: parse error: Unmatched ']' at line 1, column 8\n"),
            ("", "jq: parse error: Unmatched ']' at line 1, column 8\n"),
            (
                "",
                "jq: parse error: Unmatched ']' in the middle of an object at line 1, column 8\n",
            ),
        ],
    ),
    (
        "12\0x:\n",
        [
            (
                "",
                "jq: parse error: Expected string key before ':' at line 1, column 5\n",
            ),
            (
                "",
                "jq: parse error: Expected string key before ':' at line 1, column 5\n",
            ),
            (
                "",
                "jq: parse error: ':' not as part of an object at line 1, column 5\n",
            ),
        ],
    ),
    (
        "\"s\"1\0x}\n",
        [
            (
                "\"s\"\n",
                "jq: parse error: Unmatched '}' at line 1, column 7\n",
            ),
            ("", "jq: parse error: Unmatched '}' at line 1, column 7\n"),
            (
                "[[],\"s\"]\n",
                "jq: parse error: Unmatched '}' at the top-level at line 1, column 7\n",
            ),
        ],
    ),
    (
        "[1,2\0x\0y 3]\n",
        [
            (
                "",
                "jq: parse error: Expected separator between values at line 1, column 11\n",
            ),
            (
                "",
                "jq: parse error: Expected separator between values at line 1, column 11\n",
            ),
            (
                "[[0],1]\n",
                "jq: parse error: Expected separator between values at line 1, column 11\n",
            ),
        ],
    ),
    (
        "[1\0 2]\n",
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
                "",
                "jq: parse error: Expected separator between values at line 1, column 6\n",
            ),
        ],
    ),
    (
        "1\0x]\n2\n",
        [
            ("", "jq: parse error: Unmatched ']' at line 1, column 4\n"),
            ("", "jq: parse error: Unmatched ']' at line 1, column 4\n"),
            (
                "",
                "jq: parse error: Unmatched ']' at the top-level at line 1, column 4\n",
            ),
        ],
    ),
    (
        "[1\0,2]\n1.5\0e}\n",
        [
            (
                "[1,2]\n",
                "jq: parse error: Unmatched '}' at line 2, column 6\n",
            ),
            ("", "jq: parse error: Unmatched '}' at line 2, column 6\n"),
            (
                "[[0],1]\n[[1],2]\n[[1]]\n",
                "jq: parse error: Unmatched '}' at the top-level at line 2, column 6\n",
            ),
        ],
    ),
    (
        "{\"a\":-1\0x\"b\"}\n",
        [
            (
                "",
                "jq: parse error: Expected separator between values at line 1, column 12\n",
            ),
            (
                "",
                "jq: parse error: Expected separator between values at line 1, column 12\n",
            ),
            (
                "",
                "jq: parse error: Expected separator between values at line 1, column 12\n",
            ),
        ],
    ),
    (
        "nan\0}\n",
        [
            ("", "jq: parse error: Unmatched '}' at line 1, column 5\n"),
            ("", "jq: parse error: Unmatched '}' at line 1, column 5\n"),
            (
                "",
                "jq: parse error: Unmatched '}' at the top-level at line 1, column 5\n",
            ),
        ],
    ),
    (
        "[[1]1\0x\n",
        [
            (
                "",
                "jq: parse error: Expected separator between values at line 2, column 0\n",
            ),
            (
                "",
                "jq: parse error: Expected separator between values at line 2, column 0\n",
            ),
            (
                "[[0,0],1]\n[[0,0]]\n",
                "jq: parse error: Expected separator between values at line 2, column 0\n",
            ),
        ],
    ),
];

#[test]
fn a_nul_tail_ends_its_token_where_jq_ends_it_7749() -> Result<()> {
    let routes: [&[&str]; 3] = [&["-c", "."], &["-c", "-s", "."], &["-c", "--stream", "."]];
    for (input, answers) in ROWS {
        for (args, (stdout, stderr)) in routes.iter().zip(answers) {
            // #7762: `-s` over standard input words a missing separator in a container as
            // `Invalid JSON text`, NUL or not; the file route below reads those rows.
            let slurp_gap = args.contains(&"-s") && stderr.contains("Expected separator");
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
/// stream): `(first file, second file, -c . stdout, stderr, exit)`.
const ACROSS_FILES: &[(&str, &str, &str, &str, i32)] = &[
    (
        "[[1]1",
        "\0]\n",
        "",
        "jq: parse error: Expected separator between values at line 1, column 7\n",
        5,
    ),
    ("[1", "2\0x]\n", "[12]\n", "", 0),
    (
        "1",
        "\0}\n",
        "",
        "jq: parse error: Unmatched '}' at line 1, column 3\n",
        5,
    ),
    ("[123", "\0]\n", "[123]\n", "", 0),
    (
        "{\"a\":[true,null]1",
        "2\0}\n",
        "",
        "jq: parse error: Expected separator between values at line 1, column 20\n",
        5,
    ),
    ("[1,2", "\0x,3]\n", "[1,2,3]\n", "", 0),
];

#[test]
fn a_number_from_an_earlier_file_is_rewritten_over_both_files_7749() -> Result<()> {
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
