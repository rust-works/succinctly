//! #6923: an `-o=<format>` encoder runs while yq prints, so the results before the one it refuses are
//! still printed (`-o=csv '.[]'` over `x`, `[1]`, `{d: e}` prints `x` and `1`, then the error, exit 1),
//! and `-e` reads the value the filter produced, not the encoded text (`-o=props '.zz'` encodes `null`
//! as the truthy line `null`, and exits 1). An error the program itself raises still prints nothing.
//!
//! Every row was captured from yq v4.53.3. A top-level comma (`.a, .b`) still prints nothing before
//! the error here; `docs/compliance/yq/limitations.md` records it.

use anyhow::Result;

use crate::{run_yq_split, run_yq_stdin};

const DOC: &str = "a: x\nb: [1]\nc: {d: e}\n";

/// (format, yq's stdout, a phrase of its stderr).
const ROWS: &[(&str, &str, &str)] = &[
    (
        "csv",
        "x\n1\n",
        "csv encoding only works for arrays, got: !!map",
    ),
    (
        "tsv",
        "x\n1\n",
        "csv encoding only works for arrays, got: !!map",
    ),
    (
        "base64",
        "eA==",
        "cannot encode !!seq as base64, can only operate on strings",
    ),
    (
        "uri",
        "x",
        "cannot encode !!seq as URI, can only operate on strings",
    ),
    (
        "ini",
        "x\n",
        "INI encoder supports only MappingNode at the root level",
    ),
];

#[test]
fn the_results_before_a_refused_one_are_printed_6923() -> Result<()> {
    for (format, expected, error) in ROWS {
        let (stdout, stderr, code) = run_yq_split(".[]", DOC, &["-o", format])?;
        assert_eq!(code, 1, "{format}");
        assert_eq!(&stdout, expected, "{format}");
        assert!(stderr.contains(error), "{format}: {stderr}");
    }
    Ok(())
}

#[test]
fn the_results_before_a_refused_one_are_printed_under_eval_all_and_null_input_6923() -> Result<()> {
    let (stdout, stderr, code) =
        run_yq_split(".[]", "a: x\nc: {d: e}\n", &["--eval-all", "-o=csv"])?;
    assert_eq!((stdout.as_str(), code), ("x\n", 1), "{stderr}");
    let (stdout, stderr, code) = run_yq_split(
        "[[\"a\"], [\"b\"], {\"c\": 1}] | .[]",
        "",
        &["-n", "-o=csv"],
    )?;
    assert_eq!((stdout.as_str(), code), ("a\nb\n", 1), "{stderr}");
    Ok(())
}

#[test]
fn an_error_the_program_raises_still_prints_nothing_6923() -> Result<()> {
    let (stdout, stderr, code) = run_yq_split(
        ".[] | select(tag != \"!!map\") , (select(tag == \"!!map\") | error(\"boom\"))",
        DOC,
        &["-o=csv"],
    )?;
    assert_eq!((stdout.as_str(), code), ("", 1), "{stderr}");
    Ok(())
}

#[test]
fn exit_status_reads_the_value_not_the_encoded_text_6923() -> Result<()> {
    // (filter, yq's exit code with `-e -o=<format>`) over `a: 1` / `b: false`.
    for (filter, expected) in [(".zz", 1), (".a", 0), (".b", 1), ("false", 1), ("[]", 0)] {
        for format in ["csv", "props", "lua"] {
            let (_, _, code) = run_yq_split(filter, "a: 1\nb: false\n", &["-e", "-o", format])?;
            assert_eq!(code, expected, "-e -o={format} {filter}");
        }
    }
    // The plain run is unchanged: a falsy result is only an error under `-e`.
    let (_, code) = run_yq_stdin(".zz", "a: 1\n", &["-o=props"])?;
    assert_eq!(code, 0);
    Ok(())
}
