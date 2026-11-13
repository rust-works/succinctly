//! #7108: a yq-mode fold whose UPDATE is `|= . + N` (or another operator) of a number field by a
//! number is answered on the owned tree, as `+=` is, and no longer re-indexes the state every step.
//! The rows are yq v4.53.3's answers; the ones with a non-number, a missing key, a glob key or a
//! number past `i64` stay on the evaluator, whose output is unchanged.

use anyhow::Result;

use crate::run_yq_stdin_with_stderr;

/// (stdin, extra args, filter, yq's stdout, yq's stderr line, yq's exit status).
type Row = (
    &'static str,
    &'static [&'static str],
    &'static str,
    &'static str,
    &'static str,
    i32,
);

const ROWS: &[Row] = &[
    ("k:\n  x: [1, 2, 3, 4]\n  y: 10\n", &[], ".k |= (.x[] as $v ireduce (.; .y |= . + $v))", "k:\n  x: [1, 2, 3, 4]\n  y: 20\n", "", 0),
    ("k:\n  x: [1, 2, 3, 4]\n  y: 10\n", &["-o", "json", "-I0"], ".k |= (.x[] as $v ireduce (.; .y |= . + $v))", "{\"k\":{\"x\":[1,2,3,4],\"y\":20}}\n", "", 0),
    ("k:\n  x: [1, 2, 3, 4]\n  y: 10\n", &[], ".k |= (.x[] as $v ireduce (.; .y |= . - $v))", "k:\n  x: [1, 2, 3, 4]\n  y: 0\n", "", 0),
    ("k:\n  x: [1, 2, 3, 4]\n  y: 10\n", &["-o", "json", "-I0"], ".k |= (.x[] as $v ireduce (.; .y |= . - $v))", "{\"k\":{\"x\":[1,2,3,4],\"y\":0}}\n", "", 0),
    ("k:\n  x: [1, 2, 3, 4]\n  y: 10\n", &[], ".k |= (.x[] as $v ireduce (.; .y |= . * $v))", "k:\n  x: [1, 2, 3, 4]\n  y: 240\n", "", 0),
    ("k:\n  x: [1, 2, 3, 4]\n  y: 10\n", &["-o", "json", "-I0"], ".k |= (.x[] as $v ireduce (.; .y |= . * $v))", "{\"k\":{\"x\":[1,2,3,4],\"y\":240}}\n", "", 0),
    ("k:\n  x: [1, 2, 3, 4]\n  y: 10\n", &[], ".k |= (.x[] as $v ireduce (.; .y |= . / $v))", "k:\n  x: [1, 2, 3, 4]\n  y: 0.4166666666666667\n", "", 0),
    ("k:\n  x: [1, 2, 3, 4]\n  y: 10\n", &["-o", "json", "-I0"], ".k |= (.x[] as $v ireduce (.; .y |= . / $v))", "{\"k\":{\"x\":[1,2,3,4],\"y\":0.4166666666666667}}\n", "", 0),
    ("k:\n  x: [1, 2, 3, 4]\n  y: 10\n", &[], ".k |= (.x[] as $v ireduce (.; .y |= . % $v))", "k:\n  x: [1, 2, 3, 4]\n  y: 0\n", "", 0),
    ("k:\n  x: [1, 2, 3, 4]\n  y: 10\n", &["-o", "json", "-I0"], ".k |= (.x[] as $v ireduce (.; .y |= . % $v))", "{\"k\":{\"x\":[1,2,3,4],\"y\":0}}\n", "", 0),
    ("k:\n  x: [1, 2, 3, 4]\n  y: 10\n", &[], ".k |= (.x[] as $v ireduce (.; .y |= . + $v | .n |= . + 1))", "k:\n  x: [1, 2, 3, 4]\n  y: 20\n  n: 4\n", "", 0),
    ("k:\n  x: [1, 2, 3, 4]\n  y: 10\n", &["-o", "json", "-I0"], ".k |= (.x[] as $v ireduce (.; .y |= . + $v | .n |= . + 1))", "{\"k\":{\"x\":[1,2,3,4],\"y\":20,\"n\":4}}\n", "", 0),
    ("k:\n  x: [1, 2, 3]\n", &[], ".k |= (.x[] as $v ireduce (.; .y |= . + $v))", "k:\n  x: [1, 2, 3]\n  y: 6\n", "", 0),
    ("k:\n  x: [1, 2, 3]\n", &["-o", "json", "-I0"], ".k |= (.x[] as $v ireduce (.; .y |= . + $v))", "{\"k\":{\"x\":[1,2,3],\"y\":6}}\n", "", 0),
    ("k:\n  x: [1.5, 2, 0.25]\n  y: 1.5\n", &[], ".k |= (.x[] as $v ireduce (.; .y |= . + $v))", "k:\n  x: [1.5, 2, 0.25]\n  y: 5.25\n", "", 0),
    ("k:\n  x: [1.5, 2, 0.25]\n  y: 1.5\n", &["-o", "json", "-I0"], ".k |= (.x[] as $v ireduce (.; .y |= . + $v))", "{\"k\":{\"x\":[1.5,2,0.25],\"y\":5.25}}\n", "", 0),
    ("k:\n  x: [1, 2]\n  y: abc\n", &[], ".k |= (.x[] as $v ireduce (.; .y |= . + $v))", "k:\n  x: [1, 2]\n  y: abc12\n", "", 0),
    ("k:\n  x: [1, 2]\n  y: abc\n", &["-o", "json", "-I0"], ".k |= (.x[] as $v ireduce (.; .y |= . + $v))", "{\"k\":{\"x\":[1,2],\"y\":\"abc12\"}}\n", "", 0),
    ("k:\n  x: [1, 2]\n  y: ~\n", &[], ".k |= (.x[] as $v ireduce (.; .y |= . + $v))", "k:\n  x: [1, 2]\n  y: 3\n", "", 0),
    ("k:\n  x: [1, 2]\n  y: ~\n", &["-o", "json", "-I0"], ".k |= (.x[] as $v ireduce (.; .y |= . + $v))", "{\"k\":{\"x\":[1,2],\"y\":3}}\n", "", 0),
    ("k:\n  x: [9223372036854775807, 1]\n  y: 9223372036854775807\n", &[], ".k |= (.x[] as $v ireduce (.; .y |= . + $v))", "k:\n  x: [9223372036854775807, 1]\n  y: -1\n", "", 0),
    ("k:\n  x: [9223372036854775807, 1]\n  y: 9223372036854775807\n", &["-o", "json", "-I0"], ".k |= (.x[] as $v ireduce (.; .y |= . + $v))", "{\"k\":{\"x\":[9223372036854775807,1],\"y\":-1}}\n", "", 0),
    ("k:\n  x: [1, 2]\n  y: 0x1F\n", &[], ".k |= (.x[] as $v ireduce (.; .y |= . + $v))", "k:\n  x: [1, 2]\n  y: 0x22\n", "", 0),
    ("k:\n  x: [1, 2]\n  y: 0x1F\n", &["-o", "json", "-I0"], ".k |= (.x[] as $v ireduce (.; .y |= . + $v))", "{\"k\":{\"x\":[1,2],\"y\":34}}\n", "", 0),
    ("k:\n  x: [1, 2] # c\n  y: 5 # yc\n", &[], ".k |= (.x[] as $v ireduce (.; .y |= . + $v))", "k:\n  x: [1, 2] # c\n  y: 8 # yc\n", "", 0),
    ("k:\n  x: [1, 2] # c\n  y: 5 # yc\n", &["-o", "json", "-I0"], ".k |= (.x[] as $v ireduce (.; .y |= . + $v))", "{\"k\":{\"x\":[1,2],\"y\":8}}\n", "", 0),
    ("k:\n  x: [0, 0, 2]\n  y: 0\n", &[], ".k |= (.x[] as $v ireduce (.; .y |= . / $v))", "k:\n  x: [0, 0, 2]\n  y: !!float NaN\n", "", 0),
    ("k:\n  x: [0, 0, 2]\n  y: 0\n", &["-o", "json", "-I0"], ".k |= (.x[] as $v ireduce (.; .y |= . / $v))", "", "Error: json: error calling MarshalJSON for type *yqlib.CandidateNode: json: error calling MarshalJSON for type *yqlib.CandidateNode: json: error calling MarshalJSON for type *yqlib.CandidateNode: json: unsupported value: NaN", 1),
    ("k:\n  x: [1, 2, 3]\n  y: 1\n", &[], ".k |= (.x[] as $v ireduce (.; .[\"y*\"] |= . + $v))", "k:\n  x: [1, 2, 3]\n  y: 7\n", "", 0),
    ("k:\n  x: [1, 2, 3]\n  y: 1\n", &["-o", "json", "-I0"], ".k |= (.x[] as $v ireduce (.; .[\"y*\"] |= . + $v))", "{\"k\":{\"x\":[1,2,3],\"y\":7}}\n", "", 0),
    ("k:\n  x: [1, 2, 3]\n  y: 1\n", &[], ".k |= (.x[] as $v ireduce (.; .a.b |= . + $v))", "k:\n  x: [1, 2, 3]\n  y: 1\n  a:\n    b: 6\n", "", 0),
    ("k:\n  x: [1, 2, 3]\n  y: 1\n", &["-o", "json", "-I0"], ".k |= (.x[] as $v ireduce (.; .a.b |= . + $v))", "{\"k\":{\"x\":[1,2,3],\"y\":1,\"a\":{\"b\":6}}}\n", "", 0),
    ("k:\n  x: [1, 2, 3]\n  y: 1\n", &[], "with(.k; .x[] as $v | .y |= . + $v)", "k:\n  x: [1, 2, 3]\n  y: 7\n", "", 0),
    ("k:\n  x: [1, 2, 3]\n  y: 1\n", &["-o", "json", "-I0"], "with(.k; .x[] as $v | .y |= . + $v)", "{\"k\":{\"x\":[1,2,3],\"y\":7}}\n", "", 0),
    ("k:\n  x: [1, 2, 3]\n  y: 1\n", &[], ".k.y |= . + 1", "k:\n  x: [1, 2, 3]\n  y: 2\n", "", 0),
    ("k:\n  x: [1, 2, 3]\n  y: 1\n", &["-o", "json", "-I0"], ".k.y |= . + 1", "{\"k\":{\"x\":[1,2,3],\"y\":2}}\n", "", 0),
    ("k:\n  x: [1, 2]\n  y: &a 1\n  z: *a\n", &[], ".k |= (.x[] as $v ireduce (.; .y |= . + $v))", "k:\n  x: [1, 2]\n  y: &a 4\n  z: *a\n", "", 0),
    ("k:\n  x: [1, 2]\n  y: &a 1\n  z: *a\n", &["-o", "json", "-I0"], ".k |= (.x[] as $v ireduce (.; .y |= . + $v))", "{\"k\":{\"x\":[1,2],\"y\":4,\"z\":4}}\n", "", 0),
    ("k:\n  x: [1, 2]\n  y: &a 1\n  z: *a\n", &[], "with(.k; .x[] as $v | .y |= . + $v)", "k:\n  x: [1, 2]\n  y: &a 4\n  z: *a\n", "", 0),
    ("k:\n  x: [1, 2]\n  y: &a 1\n  z: *a\n", &["-o", "json", "-I0"], "with(.k; .x[] as $v | .y |= . + $v)", "{\"k\":{\"x\":[1,2],\"y\":4,\"z\":4}}\n", "", 0),
    ("k:\n  x: [0, 2]\n  y: 7\n", &[], ".k |= (.x[] as $v ireduce (.; .y |= . / $v))", "k:\n  x: [0, 2]\n  y: !!float +Inf\n", "", 0),
    ("k:\n  x: [0, 2]\n  y: 7\n", &["-o", "json", "-I0"], ".k |= (.x[] as $v ireduce (.; .y |= . / $v))", "", "Error: json: error calling MarshalJSON for type *yqlib.CandidateNode: json: error calling MarshalJSON for type *yqlib.CandidateNode: json: error calling MarshalJSON for type *yqlib.CandidateNode: json: unsupported value: +Inf", 1),
    ("k:\n  x: [0, 2]\n  y: 7\n", &[], ".k |= (.x[] as $v ireduce (.; .y |= . % $v))", "", "Error: cannot modulo by 0", 1),
    ("k:\n  x: [0, 2]\n  y: 7\n", &["-o", "json", "-I0"], ".k |= (.x[] as $v ireduce (.; .y |= . % $v))", "", "Error: cannot modulo by 0", 1),
    ("k:\n  x: [-1]\n  y: -9223372036854775808\n", &[], ".k |= (.x[] as $v ireduce (.; .y |= . / $v))", "k:\n  x: [-1]\n  y: 9.223372036854776e+18\n", "", 0),
    ("k:\n  x: [-1]\n  y: -9223372036854775808\n", &["-o", "json", "-I0"], ".k |= (.x[] as $v ireduce (.; .y |= . / $v))", "{\"k\":{\"x\":[-1],\"y\":9.223372036854776e+18}}\n", "", 0),
    ("k:\n  x: [-1]\n  y: -9223372036854775808\n", &[], ".k |= (.x[] as $v ireduce (.; .y |= . * $v))", "k:\n  x: [-1]\n  y: -9223372036854775808\n", "", 0),
    ("k:\n  x: [-1]\n  y: -9223372036854775808\n", &["-o", "json", "-I0"], ".k |= (.x[] as $v ireduce (.; .y |= . * $v))", "{\"k\":{\"x\":[-1],\"y\":-9223372036854775808}}\n", "", 0),
    ("k:\n  x: [1]\n  y: 9223372036854775807\n", &[], ".k |= (.x[] as $v ireduce (.; .y |= . + $v))", "k:\n  x: [1]\n  y: -9223372036854775808\n", "", 0),
    ("k:\n  x: [1]\n  y: 9223372036854775807\n", &["-o", "json", "-I0"], ".k |= (.x[] as $v ireduce (.; .y |= . + $v))", "{\"k\":{\"x\":[1],\"y\":-9223372036854775808}}\n", "", 0),
    ("k:\n  x: [1, 2]\n  y: [5, 6]\n", &[], ".k |= (.x[] as $v ireduce (.; .y[0] |= . + $v))", "k:\n  x: [1, 2]\n  y: [8, 6]\n", "", 0),
    ("k:\n  x: [1, 2]\n  y: [5, 6]\n", &["-o", "json", "-I0"], ".k |= (.x[] as $v ireduce (.; .y[0] |= . + $v))", "{\"k\":{\"x\":[1,2],\"y\":[8,6]}}\n", "", 0),
];

#[test]
fn yq_fold_numeric_update_matches_yq_7108() -> Result<()> {
    for &(input, extra, filter, stdout, stderr, code) in ROWS {
        let (out, err, status) = run_yq_stdin_with_stderr(filter, input, extra)?;
        let row = format!("{extra:?} {filter:?} on {input:?}");
        assert_eq!(out, stdout, "stdout of {row}");
        assert_eq!(err.lines().next().unwrap_or(""), stderr, "stderr of {row}");
        assert_eq!(status, code, "exit status of {row}");
    }
    Ok(())
}

/// 40,000 steps over a 40,000-element state: a few milliseconds owned, minutes through the bridge.
#[test]
fn yq_fold_numeric_update_is_linear_7108() -> Result<()> {
    let items: Vec<String> = (0..40_000).map(|n| n.to_string()).collect();
    let input = format!("k:\n  y: 0\n  x: [{}]\n", items.join(", "));
    let start = std::time::Instant::now();
    let (out, _, status) = run_yq_stdin_with_stderr(
        ".k |= (.x[] as $v ireduce (.; .y |= . + $v)) | .k.y",
        &input,
        &[],
    )?;
    assert_eq!(out, "799980000\n");
    assert_eq!(status, 0);
    assert!(
        start.elapsed() < std::time::Duration::from_secs(30),
        "the fold took {:?}",
        start.elapsed()
    );
    Ok(())
}
