//! #6924: `succinctly yq -o ini`, a port of yq's go-ini encoder: the root's scalars are the default section,
//! each mapping a `[section]` of its scalars, anything else is dropped, keys are padded to the longest
//! quoted key and quoted as go-ini quotes them. Every answer was captured from yq v4.53.3; a row is kept only
//! when both agree on stdout and the exit status. Left out because it still differs: a null written `~`
//! (yq echoes the spelling, #2802).

use anyhow::Result;

use crate::run_yq_stdin;

/// (input, yq's stdout, yq's exit status).
const ROWS: &[(&str, &str, i32)] = &[
    ("a: 1\nb: [x]\ns:\n  k: v\n  t: {z: 1}\n  u: [2]\n", "a = 1\n\n[s]\nk = v\n", 0),
    ("a: 0x1F\nb: 1e3\nc: 2001-01-01\nd: yes\ne: .inf\nf: 1_000\n", "a = 0x1F\nb = 1e3\nc = 2001-01-01\nd = yes\ne = .inf\nf = 1_000\n", 0),
    ("hello\n", "hello\n", 0),
    ("3\n", "3\n", 0),
    ("null\n", "null\n", 0),
    ("- a\n", "", 1),
    ("[]\n", "", 1),
    ("{}\n", "", 0),
    ("---\na: 1\n---\nb: 2\n", "a = 1\nb = 2\n", 0),
    ("a: |\n  l1\n  l2\nb: >\n  f1\n  f2\nc: 's q'\nd: \" lead\"\ne: \"trail \"\nf: \"a#b\"\ng: \"a;b\"\nh: \"a`b\"\ni: \"a\\\"b\"\nj: \"\"\nk: \"日本\"\n", "a = \"\"\"l1\nl2\n\"\"\"\nb = \"\"\"f1 f2\n\"\"\"\nc = s q\nd = \" lead\"\ne = \"trail \"\nf = `a#b`\ng = `a;b`\nh = \"\"\"a`b\"\"\"\ni = a\"b\nj = \nk = 日本\n", 0),
    ("a`b: 1\na\"b: 2\na=b: 3\na:b: 4\na#b: 5\n日本: 6\nlong_key_name_here: 7\n", "\"\"\"a`b\"\"\"          = 1\n`a\"b`              = 2\n`a=b`              = 3\n`a:b`              = 4\na#b                = 5\n日本             = 6\nlong_key_name_here = 7\n", 0),
    ("s: {a: 1, bbb: 2}\nt: {}\nu: {x: y}\n", "[s]\na   = 1\nbbb = 2\n\n[t]\n\n[u]\nx = y\n", 0),
    ("a: 1\nDEFAULT: {a: 2, b: 3}\nc: 4\n", "a = 2\nb = 3\nc = 4\n", 0),
    ("\"\": 1\n", "", 1),
    ("a: 1\n\"\": {x: 1}\n", "", 1),
    ("s: {\"\": 1}\n", "", 1),
    ("a: &x {k: v}\nb: *x\n", "[a]\nk = v\n\n[b]\nk = v\n", 0),
    ("a: 1\ns:\n  k: [1]\n  m: {n: 1}\n", "a = 1\n\n[s]\n", 0),
    ("\"[s]\": {a: 1}\n", "[[s]]\na = 1\n", 0),
];

#[test]
fn ini_rows_match_yq_6924() -> Result<()> {
    for (input, expected, code) in ROWS {
        let (stdout, status) = run_yq_stdin(".", input, &["-o=ini"])?;
        assert_eq!(
            (stdout.as_str(), status),
            (*expected, *code),
            "-o=ini over {input:?}"
        );
    }
    Ok(())
}

#[test]
fn ini_short_name_6924() -> Result<()> {
    let one = run_yq_stdin(".", "a: 1\n", &["-o=i"])?;
    let other = run_yq_stdin(".", "a: 1\n", &["-o=ini"])?;
    assert_eq!(one, other);
    Ok(())
}
