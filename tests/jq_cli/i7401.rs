//! #7401: an object's key read twice is one `jv` in jq 1.7.1, so `path($k)` accepts it.
//!
//! jq's `keys`, `keys_unsorted`, `.[]` (and so `to_entries`, `paths` and `tostream`) all hand out
//! the object's own key `jv`, and `path()` accepts a result `jv_identical` to the register's value:
//! `keys[0] as $k | keys[0] | path($k)` is `[]`. A string with the same text from anywhere else is
//! a different `jv` and is refused, and so is the same key of another object, sibling records
//! included. Writes keep the keys they had and store the key `jv` they are given: `. + {b: 1}`,
//! `.a = 5` and `{} * .` keep or carry the key, `{($k): 1}` stores `$k` itself, and an object
//! constructor's literal key is one `jv` per site. Every row captured from `/usr/bin/jq` 1.7.1.

use anyhow::Result;

use crate::assert_path_rows_both_routes_3749;
use crate::run_jq_stdin_streams;

const REFUSED: &str = "Invalid path expression with result \"a\"";

/// The issue's rows and their controls, on stdin and as a `-n` literal.
#[test]
fn test_7401_key_read_twice_is_identical() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":1}"#,
            r"keys[0] as $k | keys[0] | path($k)",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":1}"#,
            r"keys_unsorted[0] as $k | keys_unsorted[0] | path($k)",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":1}"#,
            r"to_entries[0].key as $k | keys[0] | path($k)",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":1}"#,
            r". as $d | [paths][0][0] as $k | $d | keys[0] | path($k)",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":1}"#,
            r"keys[0] as $k | [paths][0][0] | path($k)",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":1}"#,
            r"keys[0] as $k | tostream | .[0][0] | path($k)",
            "[]\n[]\n",
            "",
            0,
        ),
        // A fresh string of the same text, the key itself with the register elsewhere, and the
        // path component of `.a` (the program's own constant) are other `jv`s.
        (
            r#"{"a":1}"#,
            r#""a" as $k | keys[0] | path($k)"#,
            "",
            REFUSED,
            5,
        ),
        (r#"{"a":1}"#, r"keys[0] as $k | path($k)", "", REFUSED, 5),
        (
            r#"{"a":1}"#,
            r"keys[0] as $k | path(.a)[0] | path($k)",
            "",
            REFUSED,
            5,
        ),
    ])
}

/// Two objects' keys are two `jv`s: another object's, a sibling record's (the reader shares one
/// key among records, #7260), and a constructor site's are not the input's.
#[test]
fn test_7401_keys_of_two_objects_are_distinct() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":1,"b":{"a":2}}"#,
            r"keys[0] as $k | .b | keys[0] | path($k)",
            "",
            REFUSED,
            5,
        ),
        (
            r#"[{"a":1},{"a":2}]"#,
            r"(.[0] | keys[0]) as $k | .[1] | keys[0] | path($k)",
            "",
            REFUSED,
            5,
        ),
        (
            r#"{"a":1}"#,
            r"keys[0] as $k | {a: 3} + . | keys[0] | path($k)",
            "",
            REFUSED,
            5,
        ),
        (
            "null",
            r"[{a: 1}, {a: 2}] | (.[0] | keys[0]) as $k | .[1] | keys[0] | path($k)",
            "",
            REFUSED,
            5,
        ),
        (
            "null",
            r#""[{\"a\":1},{\"a\":2}]" | fromjson | (.[0] | keys[0]) as $k | .[1] | keys[0] | path($k)"#,
            "",
            REFUSED,
            5,
        ),
    ])
}

/// Writes keep a key the object has and store the key `jv` they are given; a literal key is one
/// `jv` per constructor site, so the records one site builds share it.
#[test]
fn test_7401_writes_and_constructors_keep_the_key() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":1}"#,
            r"keys[0] as $k | {($k): 2} | keys[0] | path($k)",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":1}"#,
            r"keys[0] as $k | . + {b: 2} | keys[0] | path($k)",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":1}"#,
            r"keys[0] as $k | .a = 5 | keys[0] | path($k)",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":1}"#,
            r"keys[0] as $k | null | setpath([$k]; 1) | keys[0] | path($k)",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":1}"#,
            r"keys[0] as $k | with_entries(.) | keys[0] | path($k)",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"b":{"a":1},"a":"b"}"#,
            r"keys[0] as $k | {z: 1} * . | keys_unsorted[-1] | path($k)",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":1,"b":2}"#,
            r"to_entries | (.[0] | keys[0]) as $k | .[1] | keys[0] | path($k)",
            "[]\n",
            "",
            0,
        ),
        (
            "null",
            r"[range(2) | {a: .}] | (.[0] | keys[0]) as $k | .[1] | keys[0] | path($k)",
            "[]\n",
            "",
            0,
        ),
    ])
}

/// `input` parses a document as the run's input is parsed: each object's keys are its own.
#[test]
fn test_7401_input_document_keys() -> Result<()> {
    let rows = [
        (
            r"input | keys[0] as $k | keys[0] | path($k)",
            r#"{"a":1}"#,
            "[]\n",
            0,
        ),
        (
            r"input | (.[0] | keys[0]) as $k | .[1] | keys[0] | path($k)",
            r#"[{"a":1},{"a":2}]"#,
            "",
            5,
        ),
    ];
    for (filter, input, want, exit) in rows {
        let (stdout, stderr, code) = run_jq_stdin_streams(filter, input, &["-nc"])?;
        assert_eq!((stdout.as_str(), code), (want, exit), "{filter}: {stderr}");
        assert_eq!(exit != 0, stderr.contains(REFUSED), "{filter}: {stderr}");
    }
    Ok(())
}
