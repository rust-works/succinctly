//! #6851: yq renders a mapping that merges one source (`<<: *g`) and then writes an own key the source
//! also has as the merged members the own key does not replace, then the own keys: `<<: *g` / `a: 9`
//! over `{a: 1, b: 2, c: 3}` is `{"b":2,"c":3,"a":9}`. Every row is captured from yq v4.53.3: the
//! source document, the extra arguments, the filter and the expected output.
//!
//! Rows that stay as before are here too: an own key written before the `<<`, an own key the source
//! does not have, and the iteration (`[.y[]]`), which yq lists in place.

use anyhow::Result;

use crate::run_yq_stdin;

type Row = (
    &'static str,
    &'static [&'static str],
    &'static str,
    &'static str,
);

const ROWS: &[Row] = &[
    ("x: &G\n  a: 1\n  b: 2\n  c: 3\ny:\n  <<: *G\n  a: 9\n", &["-o=json", "-I0"], ".y", "{\"b\":2,\"c\":3,\"a\":9}\n"),
    ("x: &G\n  a: 1\n  b: 2\n  c: 3\ny:\n  <<: *G\n  a: 9\n", &["-o=json", "-I0"], ".", "{\"x\":{\"a\":1,\"b\":2,\"c\":3},\"y\":{\"b\":2,\"c\":3,\"a\":9}}\n"),
    ("x: &G\n  a: 1\n  b: 2\n  c: 3\ny:\n  <<: *G\n  a: 9\n", &["-o=json", "-I0"], ".y | tojson", "\"{\\n  \\\"b\\\": 2,\\n  \\\"c\\\": 3,\\n  \\\"a\\\": 9\\n}\\n\"\n"),
    ("x: &G\n  a: 1\n  b: 2\n  c: 3\ny:\n  <<: *G\n  a: 9\n", &["-o=json", "-I0"], ".y | with_entries(.)", "{\"b\":2,\"c\":3,\"a\":9}\n"),
    ("x: &G\n  a: 1\n  b: 2\n  c: 3\ny:\n  <<: *G\n  a: 9\n", &["-o=json", "-I0"], ".y | map_values(.)", "{\"b\":2,\"c\":3,\"a\":9}\n"),
    ("x: &G\n  a: 1\n  b: 2\n  c: 3\ny:\n  <<: *G\n  a: 9\n", &["-o=json", "-I0"], "{\"k\": .y}", "{\"k\":{\"b\":2,\"c\":3,\"a\":9}}\n"),
    ("x: &G\n  a: 1\n  b: 2\n  c: 3\ny:\n  <<: *G\n  a: 9\n", &["-o=json", "-I0"], "[.y[]]", "[9,2,3]\n"),
    ("x: &G\n  a: 1\n  b: 2\n  c: 3\ny:\n  <<: *G\n  a: 9\n  b: 8\n", &["-o=json", "-I0"], ".y", "{\"c\":3,\"a\":9,\"b\":8}\n"),
    ("x: &G\n  a: 1\n  b: 2\n  c: 3\ny:\n  <<: *G\n  a: 9\n  b: 8\n", &["-o=json", "-I0"], ".", "{\"x\":{\"a\":1,\"b\":2,\"c\":3},\"y\":{\"c\":3,\"a\":9,\"b\":8}}\n"),
    ("x: &G\n  a: 1\n  b: 2\n  c: 3\ny:\n  <<: *G\n  a: 9\n  b: 8\n", &["-o=json", "-I0"], ".y | tojson", "\"{\\n  \\\"c\\\": 3,\\n  \\\"a\\\": 9,\\n  \\\"b\\\": 8\\n}\\n\"\n"),
    ("x: &G\n  a: 1\n  b: 2\n  c: 3\ny:\n  <<: *G\n  a: 9\n  b: 8\n", &["-o=json", "-I0"], ".y | with_entries(.)", "{\"c\":3,\"a\":9,\"b\":8}\n"),
    ("x: &G\n  a: 1\n  b: 2\n  c: 3\ny:\n  <<: *G\n  a: 9\n  b: 8\n", &["-o=json", "-I0"], ".y | map_values(.)", "{\"c\":3,\"a\":9,\"b\":8}\n"),
    ("x: &G\n  a: 1\n  b: 2\n  c: 3\ny:\n  <<: *G\n  a: 9\n  b: 8\n", &["-o=json", "-I0"], "{\"k\": .y}", "{\"k\":{\"c\":3,\"a\":9,\"b\":8}}\n"),
    ("x: &G\n  a: 1\n  b: 2\n  c: 3\ny:\n  <<: *G\n  a: 9\n  b: 8\n", &["-o=json", "-I0"], "[.y[]]", "[9,8,3]\n"),
    ("x: &G\n  a: 1\n  b: 2\n  c: 3\ny:\n  <<: *G\n  c: 9\n  a: 8\n", &["-o=json", "-I0"], ".y", "{\"b\":2,\"c\":9,\"a\":8}\n"),
    ("x: &G\n  a: 1\n  b: 2\n  c: 3\ny:\n  <<: *G\n  c: 9\n  a: 8\n", &["-o=json", "-I0"], ".", "{\"x\":{\"a\":1,\"b\":2,\"c\":3},\"y\":{\"b\":2,\"c\":9,\"a\":8}}\n"),
    ("x: &G\n  a: 1\n  b: 2\n  c: 3\ny:\n  <<: *G\n  c: 9\n  a: 8\n", &["-o=json", "-I0"], ".y | tojson", "\"{\\n  \\\"b\\\": 2,\\n  \\\"c\\\": 9,\\n  \\\"a\\\": 8\\n}\\n\"\n"),
    ("x: &G\n  a: 1\n  b: 2\n  c: 3\ny:\n  <<: *G\n  c: 9\n  a: 8\n", &["-o=json", "-I0"], ".y | with_entries(.)", "{\"b\":2,\"c\":9,\"a\":8}\n"),
    ("x: &G\n  a: 1\n  b: 2\n  c: 3\ny:\n  <<: *G\n  c: 9\n  a: 8\n", &["-o=json", "-I0"], ".y | map_values(.)", "{\"b\":2,\"c\":9,\"a\":8}\n"),
    ("x: &G\n  a: 1\n  b: 2\n  c: 3\ny:\n  <<: *G\n  c: 9\n  a: 8\n", &["-o=json", "-I0"], "{\"k\": .y}", "{\"k\":{\"b\":2,\"c\":9,\"a\":8}}\n"),
    ("x: &G\n  a: 1\n  b: 2\n  c: 3\ny:\n  <<: *G\n  c: 9\n  a: 8\n", &["-o=json", "-I0"], "[.y[]]", "[8,2,9]\n"),
    ("x: &G\n  a: 1\n  b: 2\n  c: 3\ny:\n  k: 0\n  <<: *G\n  b: 9\n", &["-o=json", "-I0"], ".y", "{\"k\":0,\"a\":1,\"c\":3,\"b\":9}\n"),
    ("x: &G\n  a: 1\n  b: 2\n  c: 3\ny:\n  k: 0\n  <<: *G\n  b: 9\n", &["-o=json", "-I0"], ".", "{\"x\":{\"a\":1,\"b\":2,\"c\":3},\"y\":{\"k\":0,\"a\":1,\"c\":3,\"b\":9}}\n"),
    ("x: &G\n  a: 1\n  b: 2\n  c: 3\ny:\n  k: 0\n  <<: *G\n  b: 9\n", &["-o=json", "-I0"], ".y | tojson", "\"{\\n  \\\"k\\\": 0,\\n  \\\"a\\\": 1,\\n  \\\"c\\\": 3,\\n  \\\"b\\\": 9\\n}\\n\"\n"),
    ("x: &G\n  a: 1\n  b: 2\n  c: 3\ny:\n  k: 0\n  <<: *G\n  b: 9\n", &["-o=json", "-I0"], ".y | with_entries(.)", "{\"k\":0,\"a\":1,\"c\":3,\"b\":9}\n"),
    ("x: &G\n  a: 1\n  b: 2\n  c: 3\ny:\n  k: 0\n  <<: *G\n  b: 9\n", &["-o=json", "-I0"], ".y | map_values(.)", "{\"k\":0,\"a\":1,\"c\":3,\"b\":9}\n"),
    ("x: &G\n  a: 1\n  b: 2\n  c: 3\ny:\n  k: 0\n  <<: *G\n  b: 9\n", &["-o=json", "-I0"], "{\"k\": .y}", "{\"k\":{\"k\":0,\"a\":1,\"c\":3,\"b\":9}}\n"),
    ("x: &G\n  a: 1\n  b: 2\n  c: 3\ny:\n  k: 0\n  <<: *G\n  b: 9\n", &["-o=json", "-I0"], "[.y[]]", "[0,1,9,3]\n"),
    ("x: &G\n  a: 1\n  b: 2\n  c: 3\ny:\n  <<: *G\n  zz: 1\n  a: 2\n  yy: 3\n", &["-o=json", "-I0"], ".y", "{\"b\":2,\"c\":3,\"zz\":1,\"a\":2,\"yy\":3}\n"),
    ("x: &G\n  a: 1\n  b: 2\n  c: 3\ny:\n  <<: *G\n  zz: 1\n  a: 2\n  yy: 3\n", &["-o=json", "-I0"], ".", "{\"x\":{\"a\":1,\"b\":2,\"c\":3},\"y\":{\"b\":2,\"c\":3,\"zz\":1,\"a\":2,\"yy\":3}}\n"),
    ("x: &G\n  a: 1\n  b: 2\n  c: 3\ny:\n  <<: *G\n  zz: 1\n  a: 2\n  yy: 3\n", &["-o=json", "-I0"], ".y | tojson", "\"{\\n  \\\"b\\\": 2,\\n  \\\"c\\\": 3,\\n  \\\"zz\\\": 1,\\n  \\\"a\\\": 2,\\n  \\\"yy\\\": 3\\n}\\n\"\n"),
    ("x: &G\n  a: 1\n  b: 2\n  c: 3\ny:\n  <<: *G\n  zz: 1\n  a: 2\n  yy: 3\n", &["-o=json", "-I0"], ".y | with_entries(.)", "{\"b\":2,\"c\":3,\"zz\":1,\"a\":2,\"yy\":3}\n"),
    ("x: &G\n  a: 1\n  b: 2\n  c: 3\ny:\n  <<: *G\n  zz: 1\n  a: 2\n  yy: 3\n", &["-o=json", "-I0"], ".y | map_values(.)", "{\"b\":2,\"c\":3,\"zz\":1,\"a\":2,\"yy\":3}\n"),
    ("x: &G\n  a: 1\n  b: 2\n  c: 3\ny:\n  <<: *G\n  zz: 1\n  a: 2\n  yy: 3\n", &["-o=json", "-I0"], "{\"k\": .y}", "{\"k\":{\"b\":2,\"c\":3,\"zz\":1,\"a\":2,\"yy\":3}}\n"),
    ("x: &G\n  a: 1\n  b: 2\n  c: 3\ny:\n  <<: *G\n  zz: 1\n  a: 2\n  yy: 3\n", &["-o=json", "-I0"], "[.y[]]", "[2,2,3,1,3]\n"),
    ("x: &G\n  a: 1\n  b: 2\n  c: 3\ny:\n  a: 7\n  <<: *G\n", &["-o=json", "-I0"], ".y", "{\"a\":1,\"b\":2,\"c\":3}\n"),
    ("x: &G\n  a: 1\n  b: 2\n  c: 3\ny:\n  a: 7\n  <<: *G\n", &["-o=json", "-I0"], ".", "{\"x\":{\"a\":1,\"b\":2,\"c\":3},\"y\":{\"a\":1,\"b\":2,\"c\":3}}\n"),
    ("x: &G\n  a: 1\n  b: 2\n  c: 3\ny:\n  a: 7\n  <<: *G\n", &["-o=json", "-I0"], ".y | tojson", "\"{\\n  \\\"a\\\": 1,\\n  \\\"b\\\": 2,\\n  \\\"c\\\": 3\\n}\\n\"\n"),
    ("x: &G\n  a: 1\n  b: 2\n  c: 3\ny:\n  a: 7\n  <<: *G\n", &["-o=json", "-I0"], ".y | with_entries(.)", "{\"a\":1,\"b\":2,\"c\":3}\n"),
    ("x: &G\n  a: 1\n  b: 2\n  c: 3\ny:\n  a: 7\n  <<: *G\n", &["-o=json", "-I0"], ".y | map_values(.)", "{\"a\":1,\"b\":2,\"c\":3}\n"),
    ("x: &G\n  a: 1\n  b: 2\n  c: 3\ny:\n  a: 7\n  <<: *G\n", &["-o=json", "-I0"], "{\"k\": .y}", "{\"k\":{\"a\":1,\"b\":2,\"c\":3}}\n"),
    ("x: &G\n  a: 1\n  b: 2\n  c: 3\ny:\n  a: 7\n  <<: *G\n", &["-o=json", "-I0"], "[.y[]]", "[1,2,3]\n"),
    ("x: &G\n  a: 1\n  b: 2\n  c: 3\ny:\n  <<: *G\n  q: 1\n", &["-o=json", "-I0"], ".y", "{\"a\":1,\"b\":2,\"c\":3,\"q\":1}\n"),
    ("x: &G\n  a: 1\n  b: 2\n  c: 3\ny:\n  <<: *G\n  q: 1\n", &["-o=json", "-I0"], ".", "{\"x\":{\"a\":1,\"b\":2,\"c\":3},\"y\":{\"a\":1,\"b\":2,\"c\":3,\"q\":1}}\n"),
    ("x: &G\n  a: 1\n  b: 2\n  c: 3\ny:\n  <<: *G\n  q: 1\n", &["-o=json", "-I0"], ".y | tojson", "\"{\\n  \\\"a\\\": 1,\\n  \\\"b\\\": 2,\\n  \\\"c\\\": 3,\\n  \\\"q\\\": 1\\n}\\n\"\n"),
    ("x: &G\n  a: 1\n  b: 2\n  c: 3\ny:\n  <<: *G\n  q: 1\n", &["-o=json", "-I0"], ".y | with_entries(.)", "{\"a\":1,\"b\":2,\"c\":3,\"q\":1}\n"),
    ("x: &G\n  a: 1\n  b: 2\n  c: 3\ny:\n  <<: *G\n  q: 1\n", &["-o=json", "-I0"], ".y | map_values(.)", "{\"a\":1,\"b\":2,\"c\":3,\"q\":1}\n"),
    ("x: &G\n  a: 1\n  b: 2\n  c: 3\ny:\n  <<: *G\n  q: 1\n", &["-o=json", "-I0"], "{\"k\": .y}", "{\"k\":{\"a\":1,\"b\":2,\"c\":3,\"q\":1}}\n"),
    ("x: &G\n  a: 1\n  b: 2\n  c: 3\ny:\n  <<: *G\n  q: 1\n", &["-o=json", "-I0"], "[.y[]]", "[1,2,3,1]\n"),
    ("x: &G\n  a: 1\n  b: 2\n  c: 3\ny:\n  <<: [*G]\n  b: 9\n", &["-o=json", "-I0"], ".y", "{\"a\":1,\"c\":3,\"b\":9}\n"),
    ("x: &G\n  a: 1\n  b: 2\n  c: 3\ny:\n  <<: [*G]\n  b: 9\n", &["-o=json", "-I0"], ".", "{\"x\":{\"a\":1,\"b\":2,\"c\":3},\"y\":{\"a\":1,\"c\":3,\"b\":9}}\n"),
    ("x: &G\n  a: 1\n  b: 2\n  c: 3\ny:\n  <<: [*G]\n  b: 9\n", &["-o=json", "-I0"], ".y | tojson", "\"{\\n  \\\"a\\\": 1,\\n  \\\"c\\\": 3,\\n  \\\"b\\\": 9\\n}\\n\"\n"),
    ("x: &G\n  a: 1\n  b: 2\n  c: 3\ny:\n  <<: [*G]\n  b: 9\n", &["-o=json", "-I0"], ".y | with_entries(.)", "{\"a\":1,\"c\":3,\"b\":9}\n"),
    ("x: &G\n  a: 1\n  b: 2\n  c: 3\ny:\n  <<: [*G]\n  b: 9\n", &["-o=json", "-I0"], ".y | map_values(.)", "{\"a\":1,\"c\":3,\"b\":9}\n"),
    ("x: &G\n  a: 1\n  b: 2\n  c: 3\ny:\n  <<: [*G]\n  b: 9\n", &["-o=json", "-I0"], "{\"k\": .y}", "{\"k\":{\"a\":1,\"c\":3,\"b\":9}}\n"),
    ("x: &G\n  a: 1\n  b: 2\n  c: 3\ny:\n  <<: [*G]\n  b: 9\n", &["-o=json", "-I0"], "[.y[]]", "[1,9,3]\n"),
    ("x: &G\n  a: 1\n  b: 2\n  c: 3\ny:\n  <<: *G\n", &["-o=json", "-I0"], ".y", "{\"a\":1,\"b\":2,\"c\":3}\n"),
    ("x: &G\n  a: 1\n  b: 2\n  c: 3\ny:\n  <<: *G\n", &["-o=json", "-I0"], ".", "{\"x\":{\"a\":1,\"b\":2,\"c\":3},\"y\":{\"a\":1,\"b\":2,\"c\":3}}\n"),
    ("x: &G\n  a: 1\n  b: 2\n  c: 3\ny:\n  <<: *G\n", &["-o=json", "-I0"], ".y | tojson", "\"{\\n  \\\"a\\\": 1,\\n  \\\"b\\\": 2,\\n  \\\"c\\\": 3\\n}\\n\"\n"),
    ("x: &G\n  a: 1\n  b: 2\n  c: 3\ny:\n  <<: *G\n", &["-o=json", "-I0"], ".y | with_entries(.)", "{\"a\":1,\"b\":2,\"c\":3}\n"),
    ("x: &G\n  a: 1\n  b: 2\n  c: 3\ny:\n  <<: *G\n", &["-o=json", "-I0"], ".y | map_values(.)", "{\"a\":1,\"b\":2,\"c\":3}\n"),
    ("x: &G\n  a: 1\n  b: 2\n  c: 3\ny:\n  <<: *G\n", &["-o=json", "-I0"], "{\"k\": .y}", "{\"k\":{\"a\":1,\"b\":2,\"c\":3}}\n"),
    ("x: &G\n  a: 1\n  b: 2\n  c: 3\ny:\n  <<: *G\n", &["-o=json", "-I0"], "[.y[]]", "[1,2,3]\n"),
    ("- &GA\n  release_tracks: [GA]\n  help_text: x\n  request: {method: delete}\n- <<: *GA\n  release_tracks: [BETA, ALPHA]\n  request: {method: delete2}\n", &["-o=json", "-I0"], ".", "[{\"release_tracks\":[\"GA\"],\"help_text\":\"x\",\"request\":{\"method\":\"delete\"}},{\"help_text\":\"x\",\"release_tracks\":[\"BETA\",\"ALPHA\"],\"request\":{\"method\":\"delete2\"}}]\n"),
    ("- &GA\n  release_tracks: [GA]\n  help_text: x\n- <<: *GA\n  release_tracks: [BETA]\n", &["-o=json", "-I0"], ".[1]", "{\"help_text\":\"x\",\"release_tracks\":[\"BETA\"]}\n"),
];

#[test]
fn an_own_key_after_a_lone_merge_is_listed_where_it_is_written_6851() -> Result<()> {
    for (input, extra, filter, want) in ROWS {
        let (out, code) = run_yq_stdin(filter, input, extra)?;
        assert_eq!(code, 0, "{input:?} {extra:?} {filter}");
        assert_eq!(out, *want, "{input:?} {extra:?} {filter}");
    }
    Ok(())
}
