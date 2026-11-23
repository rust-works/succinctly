//! #7843: `inside(xs)` is jq's `def inside(xs): . as $x | xs | contains($x);`, so in a path
//! expression `xs` runs as a path expression too. #7474 binds a generator argument first,
//! `xs as $v | inside($v)`, and the path-mode interpreter ran that binding's source as a value, so
//! `path(inside(.b |= 1))` reached `contains`'s result and reported `Invalid path expression with
//! result true` where jq reports the update's own path error. Every expected string was captured
//! from `/usr/bin/jq` 1.7.1; jq 1.8.2 gives the same, apart from the value preview it adds to
//! `Cannot index` errors (an improvement, so 1.7.1 stays the reference).

use anyhow::Result;

use crate::run_jq_full;

/// (arguments, standard input, stdout, stderr, exit status).
type Row = (
    &'static [&'static str],
    Option<&'static str>,
    &'static str,
    &'static str,
    i32,
);

fn check(rows: &[Row]) -> Result<()> {
    for &(args, input, stdout, stderr, code) in rows {
        let (out, err, status) = run_jq_full(args, input)?;
        assert_eq!(
            (out.as_str(), err.as_str(), status),
            (stdout, stderr, code),
            "{args:?} over {input:?}"
        );
    }
    Ok(())
}

#[test]
fn inside_runs_its_argument_as_a_path_expression_in_path_mode_7843() -> Result<()> {
    check(&[
        (&[r"-c", r"path(inside(.b |= 1))"], Some("{\"a\":1}\n"), "", "jq: error (at <stdin>:1): Invalid path expression near attempt to access element 0 of [{\"a\":1,\"b\":1},[]]\n", 5),
        (&[r"-c", r"del(inside(.b += 1))"], Some("{\"a\":1}\n"), "", "jq: error (at <stdin>:1): Invalid path expression near attempt to access element 0 of [{\"a\":1,\"b\":1},[]]\n", 5),
        (&[r"-c", r"path(inside(.b += 1) | not)"], Some("{\"a\":1}\n"), "", "jq: error (at <stdin>:1): Invalid path expression near attempt to access element 0 of [{\"a\":1,\"b\":1},[]]\n", 5),
        (&[r"-c", r"def f: inside(.b |= 1); path(f)"], Some("{\"a\":1}\n"), "", "jq: error (at <stdin>:1): Invalid path expression near attempt to access element 0 of [{\"a\":1,\"b\":1},[]]\n", 5),
        (&[r"-c", r"try path(inside(.b |= 1)) catch ."], Some("{\"a\":1}\n"), "\"Invalid path expression near attempt to access element 0 of [{\\\"a\\\":1,\\\"b\\\":1},[]]\"\n", "", 0),
        (&[r"-c", r"path(inside((.b |= 1), 2))"], Some("{\"a\":1}\n"), "", "jq: error (at <stdin>:1): Invalid path expression near attempt to access element 0 of [{\"a\":1,\"b\":1},[]]\n", 5),
        (&[r"-c", r"inside(.b |= 1) |= 1"], Some("{\"a\":1}\n"), "", "jq: error (at <stdin>:1): Invalid path expression near attempt to access element 0 of [{\"a\":1,\"b\":1},[]]\n", 5),
        (&[r"-c", r"[paths(inside(.b |= 1))]"], Some("{\"a\":1}\n"), "", "jq: error (at <stdin>:1): Cannot index number with string \"b\"\n", 5),
        (&[r"-c", r"path(inside(.b, .a))"], Some("{\"a\":1}\n"), "", "jq: error (at <stdin>:1): null (null) and object ({\"a\":1}) cannot have their containment checked\n", 5),
        (&[r"-c", r"[inside(.b |= 1)]"], Some("{\"a\":1}\n"), "[true]\n", "", 0),
        (&[r"-c", r"path(inside(1, 2))"], Some("{\"a\":1}\n"), "", "jq: error (at <stdin>:1): number (1) and object ({\"a\":1}) cannot have their containment checked\n", 5),
        (&[r"-c", r"path(inside(.b |= 1) // 1)"], Some("{\"a\":1}\n"), "", "jq: error (at <stdin>:1): Invalid path expression near attempt to access element 0 of [{\"a\":1,\"b\":1},[]]\n", 5),
        (&[r"-c", r#"[path(inside({"a":1,"b":2}, .) | select(.))]"#], Some("{\"a\":1}\n"), "", "jq: error (at <stdin>:1): Invalid path expression with result true\n", 5),
    ])
}
