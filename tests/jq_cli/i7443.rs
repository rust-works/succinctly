//! #7443: a def reached only through an argument of an unresolved call is still compiled. jq
//! never compiles an unresolved call's arguments, but a call in one still references the def it
//! names, so jq keeps that def (`block_bind_referenced`) and compiles it as a closure of the block
//! the def is written in: `def rs: foo; def run: count(rs); run` reports `foo/0` and `count/1`,
//! because `rs` is a sibling of `run`, not a unit beneath the erroring call. Where the block that
//! owns the def has an error of its own, the def stays hidden (#3391): `def h: nosuchfn2;
//! nosuchfn(h)` is one error. Every stdout, stderr and exit status below was captured from jq
//! 1.7.1 (`/usr/bin/jq -n`); jq 1.8.2 reports the same errors in the same order, with its own
//! column-and-caret location format.

use anyhow::Result;

use crate::run_jq_full;

/// (program, stdout, stderr, exit status) from `jq -n <program>`.
const CASES: &[(&str, &str, &str, i32)] = &[
    (
        "def rs: foo; def run: count(rs); run",
        "",
        "jq: error: foo/0 is not defined at <top-level>, line 1:\ndef rs: foo; def run: count(rs); run        \njq: error: count/1 is not defined at <top-level>, line 1:\ndef rs: foo; def run: count(rs); run                      \njq: 2 compile errors\n",
        3,
    ),
    (
        "def rs: foo; def run: count(1;rs); run",
        "",
        "jq: error: foo/0 is not defined at <top-level>, line 1:\ndef rs: foo; def run: count(1;rs); run        \njq: error: count/2 is not defined at <top-level>, line 1:\ndef rs: foo; def run: count(1;rs); run                      \njq: 2 compile errors\n",
        3,
    ),
    (
        "def rs: foo; def run: count(.[rs]); run",
        "",
        "jq: error: foo/0 is not defined at <top-level>, line 1:\ndef rs: foo; def run: count(.[rs]); run        \njq: error: count/1 is not defined at <top-level>, line 1:\ndef rs: foo; def run: count(.[rs]); run                      \njq: 2 compile errors\n",
        3,
    ),
    (
        "def rs: $v; def run: count(rs); run",
        "",
        "jq: error: $v is not defined at <top-level>, line 1:\ndef rs: $v; def run: count(rs); run        \njq: error: count/1 is not defined at <top-level>, line 1:\ndef rs: $v; def run: count(rs); run                     \njq: 2 compile errors\n",
        3,
    ),
    (
        "def rs: foo; def mid: rs; def run: count(mid); run",
        "",
        "jq: error: foo/0 is not defined at <top-level>, line 1:\ndef rs: foo; def mid: rs; def run: count(mid); run        \njq: error: count/1 is not defined at <top-level>, line 1:\ndef rs: foo; def mid: rs; def run: count(mid); run                                   \njq: 2 compile errors\n",
        3,
    ),
    (
        "def rs: foo; def run: count(def q: rs; q); run",
        "",
        "jq: error: foo/0 is not defined at <top-level>, line 1:\ndef rs: foo; def run: count(def q: rs; q); run        \njq: error: count/1 is not defined at <top-level>, line 1:\ndef rs: foo; def run: count(def q: rs; q); run                      \njq: 2 compile errors\n",
        3,
    ),
    (
        "def rs: foo; def run: map(count(rs)); run",
        "",
        "jq: error: foo/0 is not defined at <top-level>, line 1:\ndef rs: foo; def run: map(count(rs)); run        \njq: error: count/1 is not defined at <top-level>, line 1:\ndef rs: foo; def run: map(count(rs)); run                          \njq: 2 compile errors\n",
        3,
    ),
    (
        "def f(g): g; def rs: foo; f(count(rs))",
        "",
        "jq: error: foo/0 is not defined at <top-level>, line 1:\ndef f(g): g; def rs: foo; f(count(rs))                     \njq: error: count/1 is not defined at <top-level>, line 1:\ndef f(g): g; def rs: foo; f(count(rs))                            \njq: 2 compile errors\n",
        3,
    ),
    (
        "def rs: foo; def run: count(rs) | bar; run",
        "",
        "jq: error: foo/0 is not defined at <top-level>, line 1:\ndef rs: foo; def run: count(rs) | bar; run        \njq: error: count/1 is not defined at <top-level>, line 1:\ndef rs: foo; def run: count(rs) | bar; run                      \njq: error: bar/0 is not defined at <top-level>, line 1:\ndef rs: foo; def run: count(rs) | bar; run                                  \njq: 3 compile errors\n",
        3,
    ),
    (
        "def rs: foo; def run: bar | count(rs); run",
        "",
        "jq: error: foo/0 is not defined at <top-level>, line 1:\ndef rs: foo; def run: bar | count(rs); run        \njq: error: bar/0 is not defined at <top-level>, line 1:\ndef rs: foo; def run: bar | count(rs); run                      \njq: error: count/1 is not defined at <top-level>, line 1:\ndef rs: foo; def run: bar | count(rs); run                            \njq: 3 compile errors\n",
        3,
    ),
    (
        "def rs: zz; def run: count(rs) + count(rs); run",
        "",
        "jq: error: zz/0 is not defined at <top-level>, line 1:\ndef rs: zz; def run: count(rs) + count(rs); run        \njq: error: count/1 is not defined at <top-level>, line 1:\ndef rs: zz; def run: count(rs) + count(rs); run                     \njq: error: count/1 is not defined at <top-level>, line 1:\ndef rs: zz; def run: count(rs) + count(rs); run                                 \njq: 3 compile errors\n",
        3,
    ),
    (
        "def a: x; def b: y; def run: u(a; b); run",
        "",
        "jq: error: x/0 is not defined at <top-level>, line 1:\ndef a: x; def b: y; def run: u(a; b); run       \njq: error: y/0 is not defined at <top-level>, line 1:\ndef a: x; def b: y; def run: u(a; b); run                 \njq: error: u/2 is not defined at <top-level>, line 1:\ndef a: x; def b: y; def run: u(a; b); run                             \njq: 3 compile errors\n",
        3,
    ),
    (
        "def a: x; def b: u(a); def c: v(a); b, c",
        "",
        "jq: error: x/0 is not defined at <top-level>, line 1:\ndef a: x; def b: u(a); def c: v(a); b, c       \njq: error: u/1 is not defined at <top-level>, line 1:\ndef a: x; def b: u(a); def c: v(a); b, c                 \njq: error: v/1 is not defined at <top-level>, line 1:\ndef a: x; def b: u(a); def c: v(a); b, c                              \njq: 3 compile errors\n",
        3,
    ),
    (
        "def a: x; def b: count(a); def c: bar(b); c",
        "",
        "jq: error: x/0 is not defined at <top-level>, line 1:\ndef a: x; def b: count(a); def c: bar(b); c       \njq: error: count/1 is not defined at <top-level>, line 1:\ndef a: x; def b: count(a); def c: bar(b); c                 \njq: error: bar/1 is not defined at <top-level>, line 1:\ndef a: x; def b: count(a); def c: bar(b); c                                  \njq: 3 compile errors\n",
        3,
    ),
    (
        "def h: nosuchfn2; nosuchfn(h)",
        "",
        "jq: error: nosuchfn/1 is not defined at <top-level>, line 1:\ndef h: nosuchfn2; nosuchfn(h)                  \njq: 1 compile error\n",
        3,
    ),
    (
        "def rs: foo; count(rs)",
        "",
        "jq: error: count/1 is not defined at <top-level>, line 1:\ndef rs: foo; count(rs)             \njq: 1 compile error\n",
        3,
    ),
    (
        "def rs: foo; def run: count(rs); 1",
        "1\n",
        "",
        0,
    ),
    (
        "def rs: foo; def run: rs | count(1); run",
        "",
        "jq: error: foo/0 is not defined at <top-level>, line 1:\ndef rs: foo; def run: rs | count(1); run        \njq: error: count/1 is not defined at <top-level>, line 1:\ndef rs: foo; def run: rs | count(1); run                           \njq: 2 compile errors\n",
        3,
    ),
    (
        "def rs: foo; def run: map(rs) | bar; run",
        "",
        "jq: error: foo/0 is not defined at <top-level>, line 1:\ndef rs: foo; def run: map(rs) | bar; run        \njq: error: bar/0 is not defined at <top-level>, line 1:\ndef rs: foo; def run: map(rs) | bar; run                                \njq: 2 compile errors\n",
        3,
    ),
];

#[test]
fn test_a_def_reached_only_through_an_unresolved_calls_argument_is_compiled_7443() -> Result<()> {
    for &(program, stdout, stderr, code) in CASES {
        let got = run_jq_full(&["-n", program], None)?;
        assert_eq!(
            got,
            (stdout.to_string(), stderr.to_string(), code),
            "{program}"
        );
    }
    Ok(())
}
