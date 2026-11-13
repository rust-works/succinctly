//! #7108: a yq-mode fold (`ireduce`, and the fold `with(PATH; SRC as $v | WRITES)` is) whose UPDATE
//! is a plain `=` of a scalar onto a field of the object it updates, alone, in a pipe, or behind a
//! `SOURCE as $v` binding, is answered on the owned tree and not by re-indexing the whole state every
//! step. The rows are yq v4.53.3's answers (the ones this build already gave: nothing here changes an
//! output); a glob key and a non-scalar value stay on the evaluator and are pinned too.

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
    ("k:\n  x: [0, 1, 2, 3]\n", &[], ".k |= (.x[] as $v ireduce (.; .y = $v))", "k:\n  x: [0, 1, 2, 3]\n  y: 3\n", "", 0),
    ("k:\n  x: [0, 1, 2, 3]\n", &["-o", "json", "-I0"], ".k |= (.x[] as $v ireduce (.; .y = $v))", "{\"k\":{\"x\":[0,1,2,3],\"y\":3}}\n", "", 0),
    ("k:\n  x: [0, 1, 2, 3]\n", &[], ".k |= (.x[] as $v ireduce (.; .y = $v | .w = 1))", "k:\n  x: [0, 1, 2, 3]\n  y: 3\n  w: 1\n", "", 0),
    ("k:\n  x: [0, 1, 2, 3]\n", &["-o", "json", "-I0"], ".k |= (.x[] as $v ireduce (.; .y = $v | .w = 1))", "{\"k\":{\"x\":[0,1,2,3],\"y\":3,\"w\":1}}\n", "", 0),
    ("k:\n  x: [0, 1, 2, 3]\n", &[], ".k |= (.x[] as $v ireduce (.; $v as $w | .y = $w))", "k:\n  x: [0, 1, 2, 3]\n  y: 3\n", "", 0),
    ("k:\n  x: [0, 1, 2, 3]\n", &["-o", "json", "-I0"], ".k |= (.x[] as $v ireduce (.; $v as $w | .y = $w))", "{\"k\":{\"x\":[0,1,2,3],\"y\":3}}\n", "", 0),
    ("k:\n  x: [1, \"a\", null, true, 2.5, \"\"]\n", &[], ".k |= (.x[] as $v ireduce (.; .y = $v))", "k:\n  x: [1, \"a\", null, true, 2.5, \"\"]\n  y: \"\"\n", "", 0),
    ("k:\n  x: [1, \"a\", null, true, 2.5, \"\"]\n", &["-o", "json", "-I0"], ".k |= (.x[] as $v ireduce (.; .y = $v))", "{\"k\":{\"x\":[1,\"a\",null,true,2.5,\"\"],\"y\":\"\"}}\n", "", 0),
    ("k:\n  x: [1, 2]\n  y: old\n", &[], ".k |= (.x[] as $v ireduce (.; .y = $v))", "k:\n  x: [1, 2]\n  y: 2\n", "", 0),
    ("k:\n  x: [1, 2]\n  y: old\n", &["-o", "json", "-I0"], ".k |= (.x[] as $v ireduce (.; .y = $v))", "{\"k\":{\"x\":[1,2],\"y\":2}}\n", "", 0),
    ("k: {x: [1, 2], y: 5}\n", &[], ".k |= (.x[] as $v ireduce (.; .y = $v))", "k: {x: [1, 2], y: 2}\n", "", 0),
    ("k: {x: [1, 2], y: 5}\n", &["-o", "json", "-I0"], ".k |= (.x[] as $v ireduce (.; .y = $v))", "{\"k\":{\"x\":[1,2],\"y\":2}}\n", "", 0),
    ("# head\nk: # kc\n  x: [1, 2] # xc\n  y: old # yc\n  z: 3\n", &[], ".k |= (.x[] as $v ireduce (.; .y = $v))", "# head\nk: # kc\n  x: [1, 2] # xc\n  y: 2 # yc\n  z: 3\n", "", 0),
    ("# head\nk: # kc\n  x: [1, 2] # xc\n  y: old # yc\n  z: 3\n", &["-o", "json", "-I0"], ".k |= (.x[] as $v ireduce (.; .y = $v))", "{\"k\":{\"x\":[1,2],\"y\":2,\"z\":3}}\n", "", 0),
    ("k:\n  x: [1, 2]\n", &[], ".k |= ((1,2,3) as $v ireduce (.; .y = $v))", "k:\n  x: [1, 2]\n  y: 3\n", "", 0),
    ("k:\n  x: [1, 2]\n", &["-o", "json", "-I0"], ".k |= ((1,2,3) as $v ireduce (.; .y = $v))", "{\"k\":{\"x\":[1,2],\"y\":3}}\n", "", 0),
    ("k:\n  x: [1, 2]\n", &[], ".k |= ((1,2,3) as $v ireduce (.; .w = $v | .y = $v))", "k:\n  x: [1, 2]\n  w: 3\n  y: 3\n", "", 0),
    ("k:\n  x: [1, 2]\n", &["-o", "json", "-I0"], ".k |= ((1,2,3) as $v ireduce (.; .w = $v | .y = $v))", "{\"k\":{\"x\":[1,2],\"w\":3,\"y\":3}}\n", "", 0),
    ("k:\n  x: [1, 2]\n", &[], ".k |= ((1,2,3) as $v ireduce (.; .[\"y z\"] = $v))", "k:\n  x: [1, 2]\n  y z: 3\n", "", 0),
    ("k:\n  x: [1, 2]\n", &["-o", "json", "-I0"], ".k |= ((1,2,3) as $v ireduce (.; .[\"y z\"] = $v))", "{\"k\":{\"x\":[1,2],\"y z\":3}}\n", "", 0),
    ("k:\n  x: [1, 2]\n", &[], ".k |= ((1,2,3) as $v ireduce (.; .a.b = $v))", "k:\n  x: [1, 2]\n  a:\n    b: 3\n", "", 0),
    ("k:\n  x: [1, 2]\n", &["-o", "json", "-I0"], ".k |= ((1,2,3) as $v ireduce (.; .a.b = $v))", "{\"k\":{\"x\":[1,2],\"a\":{\"b\":3}}}\n", "", 0),
    ("k:\n  x: [1, 2]\n", &[], ".k |= ((1,2,3) as $v ireduce (.; .[\"y*\"] = $v))", "k:\n  x: [1, 2]\n  y*: 3\n", "", 0),
    ("k:\n  x: [1, 2]\n", &["-o", "json", "-I0"], ".k |= ((1,2,3) as $v ireduce (.; .[\"y*\"] = $v))", "{\"k\":{\"x\":[1,2],\"y*\":3}}\n", "", 0),
    ("k:\n  x: [1, 2]\n  yy: 1\n  yz: 2\n", &[], ".k |= ((7,8) as $v ireduce (.; .[\"y*\"] = $v))", "k:\n  x: [1, 2]\n  yy: 8\n  yz: 8\n", "", 0),
    ("k:\n  x: [1, 2]\n  yy: 1\n  yz: 2\n", &["-o", "json", "-I0"], ".k |= ((7,8) as $v ireduce (.; .[\"y*\"] = $v))", "{\"k\":{\"x\":[1,2],\"yy\":8,\"yz\":8}}\n", "", 0),
    ("k:\n  x: [1, 2]\n  yy: 1\n  yz: 2\n", &[], ".k |= ((7,8) as $v ireduce (.; .[\"y?\"] = $v))", "k:\n  x: [1, 2]\n  yy: 8\n  yz: 8\n", "", 0),
    ("k:\n  x: [1, 2]\n  yy: 1\n  yz: 2\n", &["-o", "json", "-I0"], ".k |= ((7,8) as $v ireduce (.; .[\"y?\"] = $v))", "{\"k\":{\"x\":[1,2],\"yy\":8,\"yz\":8}}\n", "", 0),
    ("k:\n  x: [1, 2]\n", &[], ".k |= ((7,8) as $v ireduce (.; .y = [$v]))", "k:\n  x: [1, 2]\n  y:\n    - 8\n", "", 0),
    ("k:\n  x: [1, 2]\n", &["-o", "json", "-I0"], ".k |= ((7,8) as $v ireduce (.; .y = [$v]))", "{\"k\":{\"x\":[1,2],\"y\":[8]}}\n", "", 0),
    ("k:\n  x: [1, 2]\n", &[], ".k |= ((7,8) as $v ireduce (.; .y = {\"a\": $v}))", "k:\n  x: [1, 2]\n  y:\n    a: 8\n", "", 0),
    ("k:\n  x: [1, 2]\n", &["-o", "json", "-I0"], ".k |= ((7,8) as $v ireduce (.; .y = {\"a\": $v}))", "{\"k\":{\"x\":[1,2],\"y\":{\"a\":8}}}\n", "", 0),
    ("k:\n  x: [1, 2]\n", &[], ".k |= ((7,8) as $v ireduce (.; .y = null))", "k:\n  x: [1, 2]\n  y: null\n", "", 0),
    ("k:\n  x: [1, 2]\n", &["-o", "json", "-I0"], ".k |= ((7,8) as $v ireduce (.; .y = null))", "{\"k\":{\"x\":[1,2],\"y\":null}}\n", "", 0),
    ("k:\n", &[], ".k |= ((7,8) as $v ireduce (.; .y = $v))", "k:\n  y: 8\n", "", 0),
    ("k:\n", &["-o", "json", "-I0"], ".k |= ((7,8) as $v ireduce (.; .y = $v))", "{\"k\":{\"y\":8}}\n", "", 0),
    ("k: {}\n", &[], ".k |= ((7,8) as $v ireduce (.; .y = $v))", "k:\n  y: 8\n", "", 0),
    ("k: {}\n", &["-o", "json", "-I0"], ".k |= ((7,8) as $v ireduce (.; .y = $v))", "{\"k\":{\"y\":8}}\n", "", 0),
    ("k: 5\n", &[], ".k |= ((7,8) as $v ireduce (.; .y = $v))", "k: 5\n", "", 0),
    ("k: 5\n", &["-o", "json", "-I0"], ".k |= ((7,8) as $v ireduce (.; .y = $v))", "{\"k\":5}\n", "", 0),
    ("k:\n  x: [1, 2]\n", &[], "with(.k; .x[] as $v | .y = $v)", "k:\n  x: [1, 2]\n  y: 2\n", "", 0),
    ("k:\n  x: [1, 2]\n", &["-o", "json", "-I0"], "with(.k; .x[] as $v | .y = $v)", "{\"k\":{\"x\":[1,2],\"y\":2}}\n", "", 0),
    ("k:\n  x: [1, 2]\n", &[], "with(.k; .x[] as $v | .y += $v)", "k:\n  x: [1, 2]\n  y: 3\n", "", 0),
    ("k:\n  x: [1, 2]\n", &["-o", "json", "-I0"], "with(.k; .x[] as $v | .y += $v)", "{\"k\":{\"x\":[1,2],\"y\":3}}\n", "", 0),
    ("k:\n  x: [1, 2]\n", &[], "with(.k; .x[] as $v | .y = $v | .z = $v)", "k:\n  x: [1, 2]\n  y: 2\n  z: 2\n", "", 0),
    ("k:\n  x: [1, 2]\n", &["-o", "json", "-I0"], "with(.k; .x[] as $v | .y = $v | .z = $v)", "{\"k\":{\"x\":[1,2],\"y\":2,\"z\":2}}\n", "", 0),
    ("k:\n  x: []\n", &[], "with(.k; .x[] as $v | .y = $v)", "k:\n  x: []\n  y: null\n", "", 0),
    ("k:\n  x: []\n", &["-o", "json", "-I0"], "with(.k; .x[] as $v | .y = $v)", "{\"k\":{\"x\":[],\"y\":null}}\n", "", 0),
    ("k:\n  x: [1, \"s\", null]\n", &[], "with(.k; .x[] as $v | .y = $v)", "k:\n  x: [1, \"s\", null]\n  y: null\n", "", 0),
    ("k:\n  x: [1, \"s\", null]\n", &["-o", "json", "-I0"], "with(.k; .x[] as $v | .y = $v)", "{\"k\":{\"x\":[1,\"s\",null],\"y\":null}}\n", "", 0),
    ("k:\n  x: [1, 2]\n", &[], "with(.k; .x[] as $v | .[\"a*\"] = $v)", "k:\n  x: [1, 2]\n  a*: 2\n", "", 0),
    ("k:\n  x: [1, 2]\n", &["-o", "json", "-I0"], "with(.k; .x[] as $v | .[\"a*\"] = $v)", "{\"k\":{\"x\":[1,2],\"a*\":2}}\n", "", 0),
];

#[test]
fn yq_fold_plain_assign_matches_yq_7108() -> Result<()> {
    for &(input, extra, filter, stdout, stderr, code) in ROWS {
        let (out, err, status) = run_yq_stdin_with_stderr(filter, input, extra)?;
        let row = format!("{extra:?} {filter:?} on {input:?}");
        assert_eq!(out, stdout, "stdout of {row}");
        assert_eq!(err.lines().next().unwrap_or(""), stderr, "stderr of {row}");
        assert_eq!(status, code, "exit status of {row}");
    }
    Ok(())
}

/// 40,000 steps over a 40,000-element state: through the bridge each step re-indexed the whole
/// state (minutes; 14 s at 10,000), answered on the owned tree it is a few milliseconds.
#[test]
fn yq_fold_plain_assign_is_linear_7108() -> Result<()> {
    let items: Vec<String> = (0..40_000).map(|n| n.to_string()).collect();
    let input = format!("k:\n  x: [{}]\n", items.join(", "));
    let start = std::time::Instant::now();
    let (out, _, status) = run_yq_stdin_with_stderr(
        ".k |= (.x[] as $v ireduce (.; .y = $v)) | .k.y",
        &input,
        &[],
    )?;
    assert_eq!(out, "39999\n");
    assert_eq!(status, 0);
    let (out, _, _) =
        run_yq_stdin_with_stderr("with(.k; .x[] as $v | .y += $v) | .k.y", &input, &[])?;
    assert_eq!(out, "799980000\n");
    assert!(
        start.elapsed() < std::time::Duration::from_secs(60),
        "the fold took {:?}",
        start.elapsed()
    );
    Ok(())
}
