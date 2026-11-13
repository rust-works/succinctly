//! #7179: under `--eval-all`, yq's `!=` evaluates both operands through a read-only context, where a key that
//! finds nothing yields no node, and `==`, `<`, `>`, `<=`, `>=` through an ordinary one, where it yields `null`.
//! Both cross the whole document list, so an operand over an absent key is empty for `!=` (`.zz != 1` is four
//! `true`, one per right-hand result, where `==` pairs sixteen) and `.a != .c` pairs the one document that has
//! `a` with the one that has `c`. `succinctly yq` read the operands of `!=` as `==` does.
//!
//! Every row was captured from yq v4.53.3 over the three files below (the third holds two documents) under
//! `--eval-all -o=json -I=0`. What still differs, listed in `limitations.md`: the origin of a comparison's boolean
//! (`.k != 20 | fi`, #6973) and `and`/`or` under `--eval-all`.

use anyhow::Result;
use tempfile::TempDir;

use crate::run_yq_files;

/// (filter, yq's stdout).
#[rustfmt::skip]
const ROWS: &[(&str, &str)] = &[
    (".zz != 1", "true\ntrue\ntrue\ntrue\n"),
    ("1 != .zz", "true\ntrue\ntrue\ntrue\n"),
    (".zz != .zz", "false\n"),
    (".a != .c", "true\n"),
    (".k != 20", "true\ntrue\ntrue\ntrue\nfalse\nfalse\nfalse\nfalse\ntrue\ntrue\ntrue\ntrue\n"),
    (".k != 20 | [.]", "[true]\n[true]\n[true]\n[true]\n[false]\n[false]\n[false]\n[false]\n[true]\n[true]\n[true]\n[true]\n"),
    (".zz != null", "false\nfalse\nfalse\nfalse\n"),
    (".k != .k", "false\ntrue\ntrue\ntrue\nfalse\ntrue\ntrue\ntrue\nfalse\n"),
    (".k != .zz", "true\ntrue\ntrue\n"),
    (".l != .l", "true\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\n"),
    (".m != .m", "true\ntrue\ntrue\ntrue\n"),
    (".k != 20 | length", "4\n4\n4\n4\n5\n5\n5\n5\n4\n4\n4\n4\n"),
    (".a != 1", "false\nfalse\nfalse\nfalse\n"),
    ("(.zz != 1) | [.]", "[true]\n[true]\n[true]\n[true]\n"),
    (".zz != 1 | fi", "0\n0\n0\n0\n"),
    ("(.k, .zz) != 20", "true\ntrue\ntrue\ntrue\nfalse\nfalse\nfalse\nfalse\ntrue\ntrue\ntrue\ntrue\n"),
    (".k != (20, 30)", "true\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\nfalse\nfalse\nfalse\nfalse\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\nfalse\nfalse\nfalse\nfalse\n"),
    (".k != (.a, .c)", "true\ntrue\ntrue\ntrue\ntrue\ntrue\n"),
    (". != .", "true\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\n"),
    ("select(.k != 20)", "{\"a\":1,\"k\":10,\"l\":[1,2]}\n{\"e\":5,\"k\":30,\"l\":[],\"m\":{\"q\":2}}\n{\"x\":9}\n"),
    (".zz == 1", "false\nfalse\nfalse\nfalse\nfalse\nfalse\nfalse\nfalse\nfalse\nfalse\nfalse\nfalse\nfalse\nfalse\nfalse\nfalse\n"),
    (".zz == .zz", "true\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\n"),
    (".zz < 1", "false\nfalse\nfalse\nfalse\nfalse\nfalse\nfalse\nfalse\nfalse\nfalse\nfalse\nfalse\nfalse\nfalse\nfalse\nfalse\n"),
    ("[.zz != 1]", "[true,true,true,true]\n"),
];

#[test]
fn eval_all_not_equal_reads_its_operands_read_only_7179() -> Result<()> {
    let dir = TempDir::new()?;
    let texts = [
        "a: 1\nk: 10\nl: [1, 2]\n",
        "c: 3\nk: 20\nl: [3]\nm: {p: 1}\n",
        "e: 5\nk: 30\nl: []\nm: {q: 2}\n---\nx: 9\n",
    ];
    let paths: Vec<_> = texts
        .iter()
        .enumerate()
        .map(|(i, text)| {
            let path = dir.path().join(format!("f{}.yaml", i + 1));
            std::fs::write(&path, text).map(|()| path)
        })
        .collect::<std::io::Result<_>>()?;
    let refs: Vec<&std::path::Path> = paths.iter().map(std::path::PathBuf::as_path).collect();
    for &(filter, expected) in ROWS {
        let (stdout, stderr, status) = run_yq_files(filter, &refs, &["ea", "-o=json", "-I=0"])?;
        assert_eq!(
            (stdout.as_str(), stderr.as_str(), status),
            (expected, "", 0),
            "{filter}"
        );
    }
    Ok(())
}
