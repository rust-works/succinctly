//! #7726: `combinations` and `combinations(n)` hand each row on as it is built, as jq's recursive
//! definitions do, so a consumer that stops early (`first`, `limit`, `nth`, `label`/`break`,
//! `isempty`, `any`) stops the walk.
//!
//! They built the whole product first: `first(combinations(20))` on `[1,2]` took 1.2 GB,
//! `first(combinations(24))` 22 GB, and `limit(3; combinations(1000))` was refused with
//! `Cannot allocate 2^1000 elements for combinations`. Collecting every row (`[combinations(63)]`)
//! is still refused when the product cannot be held, and so is a row wider than 2^29
//! (`combinations(4294967297)`), which neither jq can answer: 1.8.2 aborts collecting
//! `[range(n)]` past `INT_MAX >> 2` and 1.7.1 grows without bound. Every other row is captured
//! from jq 1.7.1 (`/usr/bin/jq`); jq 1.8.2 answers each the same.

use anyhow::Result;

use crate::run_jq_stdin_streams;

/// `(input, filter, stdout, stderr, exit)`, each run with `-c` on the input as written.
const ROWS: &[(&str, &str, &str, &str, i32)] = &[
    // The issue's rows: a product of 2^20, 2^1000 and 2^30 rows, of which one or three are read.
    ("[1,2]", "first(combinations(20)) | length", "20\n", "", 0),
    (
        "[1,2]",
        "[limit(3; combinations(1000))] | map(length)",
        "[1000,1000,1000]\n",
        "",
        0,
    ),
    (
        "null",
        "[range(30) | [1,2]] | first(combinations) | length",
        "30\n",
        "",
        0,
    ),
    // jq's order: the last position turns fastest.
    (
        "[[1,2],[3,4]]",
        "[limit(3; combinations)]",
        "[[1,3],[1,4],[2,3]]\n",
        "",
        0,
    ),
    (
        "[1,2]",
        "[combinations(2)]",
        "[[1,1],[1,2],[2,1],[2,2]]\n",
        "",
        0,
    ),
    (
        "[1,2,3]",
        "nth(5; combinations(30)) | .[-3:]",
        "[1,2,3]\n",
        "",
        0,
    ),
    // The other consumers that stop.
    (
        "[1,2]",
        "[label $o | combinations(2) | if .[0] == 2 then break $o else . end]",
        "[[1,1],[1,2]]\n",
        "",
        0,
    ),
    ("[1,2]", "isempty(combinations(1000))", "false\n", "", 0),
    (
        "[1,2]",
        "any(combinations(1000); length == 1000)",
        "true\n",
        "",
        0,
    ),
    (
        "[1,2]",
        "reduce limit(1000; combinations(64)) as $c (0; . + 1)",
        "1000\n",
        "",
        0,
    ),
    (
        "[1,2]",
        "[foreach limit(3; combinations(40)) as $c (0; . + 1)]",
        "[1,2,3]\n",
        "",
        0,
    ),
    // A position's error or emptiness is reached on the way to the first row, so it comes before
    // any row; an object's values are iterated.
    (
        "[[1,2],3]",
        "first(combinations)",
        "",
        "jq: error (at <stdin>:0): Cannot iterate over number (3)\n",
        5,
    ),
    ("[[1,2],[],3]", "[limit(1; combinations)]", "[]\n", "", 0),
    (
        "\"ab\"",
        "[limit(1; combinations(3))]",
        "",
        "jq: error (at <stdin>:0): Cannot iterate over string (\"ab\")\n",
        5,
    ),
    (
        "5",
        "first(combinations(3))",
        "",
        "jq: error (at <stdin>:0): Cannot iterate over number (5)\n",
        5,
    ),
    (
        "{\"a\":[1,2]}",
        "first(combinations(3))",
        "[[1,2],[1,2],[1,2]]\n",
        "",
        0,
    ),
    (
        "[{\"b\":1,\"a\":2},[3]]",
        "[combinations]",
        "[[1,3],[2,3]]\n",
        "",
        0,
    ),
    // `n` as `[range(n)] | length`.
    ("[1,2]", "first(combinations(-1))", "[]\n", "", 0),
    (
        "[1,2]",
        "[limit(2; combinations(1.5))]",
        "[[1,1],[1,2]]\n",
        "",
        0,
    ),
    (
        "[[1,2],[3,4]]",
        "try path(combinations) catch .",
        "\"Invalid path expression with result [1,3]\"\n",
        "",
        0,
    ),
];

#[test]
fn test_7726_combinations_streams_its_rows_as_jq_does() -> Result<()> {
    for (input, filter, want_out, want_err, want_code) in ROWS {
        let (out, err, code) = run_jq_stdin_streams(filter, input, &["-c"])?;
        assert_eq!(
            (out.as_str(), err.as_str(), code),
            (*want_out, *want_err, *want_code),
            "{filter} on {input}"
        );
        // The same program on a value built in the program, not read (the owned route).
        if *want_code == 0 {
            let program = format!("{input} | {filter}");
            let (out, err, code) = run_jq_stdin_streams(&program, "", &["-nc"])?;
            assert_eq!(
                (out.as_str(), err.as_str(), code),
                (*want_out, "", 0),
                "-n {program}"
            );
        }
    }
    Ok(())
}

/// Where jq cannot answer, succinctly refuses at once rather than growing until it is killed:
/// collecting a product too large to hold, and a row wider than 2^29 even for a consumer that
/// takes one row (`{"a":[1]} | first(combinations(4294967297))` grew to 3 GB in 0.8 s).
#[test]
fn test_7726_combinations_refuses_only_what_cannot_be_held() -> Result<()> {
    for (input, filter, want_err) in [
        (
            "[1,2]",
            "[combinations(63)] | length",
            "jq: error (at <stdin>:0): Cannot allocate 2^63 elements for combinations\n",
        ),
        (
            "{\"a\":[1]}",
            "first(combinations(4294967297)) | length",
            "jq: error (at <stdin>:0): Cannot allocate 4294967297 elements for combinations\n",
        ),
        (
            "[1]",
            "[limit(1; combinations(536870913))] | length",
            "jq: error (at <stdin>:0): Cannot allocate 536870913 elements for combinations\n",
        ),
    ] {
        let (out, err, code) = run_jq_stdin_streams(filter, input, &["-c"])?;
        assert_eq!(
            (out.as_str(), err.as_str(), code),
            ("", want_err, 5),
            "{filter} on {input}"
        );
    }
    // A row of a million, which jq 1.7.1's quadratic recursion does not reach in 30 s.
    let (out, err, code) =
        run_jq_stdin_streams("first(combinations(1000000)) | length", "[1]", &["-c"])?;
    assert_eq!((out.as_str(), err.as_str(), code), ("1000000\n", "", 0));
    Ok(())
}
