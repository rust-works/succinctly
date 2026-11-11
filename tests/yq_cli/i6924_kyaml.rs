//! #6924: `succinctly yq -o kyaml`, a port of yq's KYAML encoder: YAML's flow subset, one entry per line with
//! a trailing comma, bare or double-quoted keys, `-I` spaces of indent, `---` between documents (`-N`
//! removes them). Every answer was captured from yq v4.53.3; a row is kept only when both agree on stdout and
//! the exit status. Left out because they still differ (see `docs/compliance/yq/limitations.md`): a comment,
//! an input that opens with `---` (yq echoes it), the YAML 1.1 integer `1_000`, `-0` between documents and
//! `-e`.

use anyhow::Result;

use crate::run_yq_stdin;

/// (flags, input, yq's stdout, yq's exit status).
const ROWS: &[(&[&str], &str, &str, i32)] = &[
    (&[], "a: 1\nb: [x, y]\nc: {d: e}\n", "{\n  a: 1,\n  b: [\n    \"x\",\n    \"y\",\n  ],\n  c: {\n    d: \"e\",\n  },\n}\n", 0),
    (&["-I4"], "a: 1\nb: [1, [2, {c: d}]]\n", "{\n    a: 1,\n    b: [\n        1,\n        [\n            2,\n            {\n                c: \"d\",\n            },\n        ],\n    ],\n}\n", 0),
    (&["-I0"], "a: 1\n", "{\na: 1,\n}\n", 0),
    (&["-I1"], "a: [1]\n", "{\n a: [\n  1,\n ],\n}\n", 0),
    (&["-I3"], "a: 1\nb: [1, [2, {c: d}]]\n", "{\n   a: 1,\n   b: [\n      1,\n      [\n         2,\n         {\n            c: \"d\",\n         },\n      ],\n   ],\n}\n", 0),
    (&[], "a: 1\n---\nb: 2\n", "{\n  a: 1,\n}\n---\n{\n  b: 2,\n}\n", 0),
    (&["-N"], "a: 1\n---\nb: 2\n", "{\n  a: 1,\n}\n{\n  b: 2,\n}\n", 0),
    (&[], "a: 1\n---\n---\nb: 2\n", "{\n  a: 1,\n}\n---\nnull\n---\n{\n  b: 2,\n}\n", 0),
    (&[], "hello\n---\nworld\n", "\"hello\"\n---\n\"world\"\n", 0),
    (&[], "[1]\n---\n[2]\n", "[\n  1,\n]\n---\n[\n  2,\n]\n", 0),
    (&[], "hello\n", "\"hello\"\n", 0),
    (&[], "3\n", "3\n", 0),
    (&[], "null\n", "null\n", 0),
    (&[], "true\n", "true\n", 0),
    (&[], "[]\n", "[]\n", 0),
    (&[], "{}\n", "{}\n", 0),
    (&[], "{a: [], b: {}}\n", "{\n  a: [],\n  b: {},\n}\n", 0),
    (&[], "a: \"x\\u0001\\u007f\\n\\r\\t\\\\ \\\"q\\\"\"\n", "{\n  a: \"x\\u0001\\n\\r\\t\\\\ \\\"q\\\"\",\n}\n", 0),
    (&[], "\"a b\": 1\nc.d: 2\n1x: 3\n_ok: 4\na-b: 5\n\"\": 6\né: 7\n", "{\n  \"a b\": 1,\n  \"c.d\": 2,\n  \"1x\": 3,\n  _ok: 4,\n  a-b: 5,\n  \"\": 6,\n  \"é\": 7,\n}\n", 0),
    (&[], "x: 2001-01-01\ny: !!binary aGk=\nz: yes\nw: null\nv: 0x1F\nt: .inf\ns: 1e3\nr: True\n", "{\n  x: \"2001-01-01\",\n  y: \"aGk=\",\n  z: \"yes\",\n  w: null,\n  v: 0x1F,\n  t: .inf,\n  s: 1e3,\n  r: true,\n}\n", 0),
    (&[], "a: &x [1]\nb: *x\n", "{\n  a: [\n    1,\n  ],\n  b: [\n    1,\n  ],\n}\n", 0),
];

#[test]
fn kyaml_rows_match_yq_6924() -> Result<()> {
    for (flags, input, expected, code) in ROWS {
        let mut args = vec!["-o=kyaml"];
        args.extend(flags.iter().copied());
        let (stdout, status) = run_yq_stdin(".", input, &args)?;
        assert_eq!(
            (stdout.as_str(), status),
            (*expected, *code),
            "-o=kyaml {flags:?} over {input:?}"
        );
    }
    Ok(())
}

#[test]
fn kyaml_short_name_6924() -> Result<()> {
    let one = run_yq_stdin(".", "a: 1\n", &["-o=ky"])?;
    let other = run_yq_stdin(".", "a: 1\n", &["-o=kyaml"])?;
    assert_eq!(one, other);
    Ok(())
}

/// (filter, input, yq's stdout, yq's exit status): several results, keys that read as other types, escapes.
const FILTER_ROWS: &[(&str, &str, &str, i32)] = &[
    (".a, .b", "a: 1\nb: [2]\n", "1\n[\n  2,\n]\n", 0),
    (
        ".[]",
        "[{a: 1}, {b: 2}, 3]\n",
        "{\n  a: 1,\n}\n{\n  b: 2,\n}\n3\n",
        0,
    ),
    (
        ".a, .b",
        "a: 1\nb: [2]\n---\na: 3\nb: 4\n",
        "1\n[\n  2,\n]\n---\n3\n4\n",
        0,
    ),
    (
        "{\"null\": 1, \"true\": 2, \"yes\": 3, \"123\": 4}",
        "a: 1\n",
        "{\n  null: 1,\n  true: 2,\n  yes: 3,\n  \"123\": 4,\n}\n",
        0,
    ),
    (
        "\"x\\u001b\\u001f\"",
        "a: 1\n",
        "\"x\\\\u001b\\\\u001f\"\n",
        0,
    ),
];

#[test]
fn kyaml_filter_rows_match_yq_6924() -> Result<()> {
    for (filter, input, expected, code) in FILTER_ROWS {
        let (stdout, status) = run_yq_stdin(filter, input, &["-o=kyaml"])?;
        assert_eq!(
            (stdout.as_str(), status),
            (*expected, *code),
            "-o=kyaml {filter:?} over {input:?}"
        );
    }
    Ok(())
}
