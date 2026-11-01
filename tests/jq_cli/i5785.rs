//! #5785: an uncaught error in a last-stage `map(f)` over a document ends the input, as in jq.
//! The document route read the failed `map` as "report and keep going" and went on to the
//! generator's next output, so `.[] | map(f)` printed later elements' arrays after the error. Each
//! row reads a file, which is the route that streams from the document; the answers, exit status
//! and the number of errors reported (jq: one) are captured from jq 1.7.1 (`/usr/bin/jq`).

use anyhow::Result;

use crate::run_jq_full;

/// `(file, filter, stdout, exit status, errors reported)`.
const ROWS: &[(&str, &str, &str, i32, usize)] = &[
    (
        "[[1],[0],[2]]",
        ".[] | map(if .==0 then error(\"boom\") else . end)",
        "[1]\n",
        5,
        1,
    ),
    (
        "[[0],[1]]",
        "(.[] | map(if .==0 then error(\"boom\") else . end)), \"after\"",
        "",
        5,
        1,
    ),
    (
        "[[1],[0],[2]]",
        "limit(3; .[] | map(if .==0 then error(\"boom\") else . end))",
        "[1]\n",
        5,
        1,
    ),
    (
        "[[0],[1]]",
        "label $f | (.[] | map(if .==0 then error(\"boom\") else . end))",
        "",
        5,
        1,
    ),
    ("[[0],0]", ".. | map(1/0)", "", 5, 1),
    ("[[-1],0]", ".. | map(.c = (.[1]?))", "", 5, 1),
    (
        "{\"a\":[1,2],\"b\":[0,3],\"c\":[4]}",
        ".[] | map(if .==0 then error(\"boom\") else . end)",
        "[1,2]\n",
        5,
        1,
    ),
    ("[[1],[2]]", ".[] | map(. + 1)", "[2]\n[3]\n", 0, 0),
    (
        "[[0],[1],[2]]",
        ".[] | try map(if .==0 then error(\"boom\") else . end) catch \"caught\"",
        "\"caught\"\n[1]\n[2]\n",
        0,
        0,
    ),
    (
        "[[0],[1],[2]]",
        ".[] | map(if .==0 then error(\"boom\") else . end)?",
        "[1]\n[2]\n",
        0,
        0,
    ),
    (
        "[[1],[0],[2]]",
        "[.[] | map(if .==0 then error(\"boom\") else . end)] | length",
        "",
        5,
        1,
    ),
    (
        "[[1],[0],[2]]",
        ".. | select(type==\"array\") | map(if .==0 then error(\"boom\") else . end)",
        "[[1],[0],[2]]\n[1]\n",
        5,
        1,
    ),
    (
        "[[1],[0],[2]]",
        ".[] | (., map(if .==0 then error(\"boom\") else . end))",
        "[1]\n[1]\n[0]\n",
        5,
        1,
    ),
];

#[test]
fn an_error_in_a_last_stage_map_ends_the_input_5785() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("in.json");
    for (file, filter, stdout, code, errors) in ROWS {
        std::fs::write(&path, format!("{file}\n"))?;
        let (out, err, status) = run_jq_full(&["-c", filter, path.to_str().unwrap()], None)?;
        assert_eq!(
            (out.as_str(), status, err.matches("jq: error").count()),
            (*stdout, *code, *errors),
            "{filter} over {file}: {err}"
        );
    }
    Ok(())
}
