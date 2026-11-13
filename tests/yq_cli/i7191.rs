//! #7191: under `--eval-all` the boolean of `and` and `or` is made from the left node in yq
//! (`createBooleanCandidate(owner)`, `operator_booleans.go`; the owner is the right node when the left side is
//! empty), so it keeps where that node was read: `(.k and 1) | file_index` over the three files below is
//! `0 0 0 0 1 1 1 1 2 2 2 2`, and `filename` follows. A literal, a built node or one of the list's own nodes as
//! the owner gives it none (`(1 and .k) | fi` is all `0`). #7184 gave every such boolean none.
//!
//! Every row was captured from yq v4.53.3 as `ea -o=json -I=0` over `b1` (`a: 1`, `k: 10`, `l: [1, 2]`), `b2`
//! (`c: 3`, `k: 20`, `l: [3]`, `m: {p: 1}`) and `b3` (`e: 5`, `k: 30`, `l: []`, `m: {q: 2}`, then a second
//! document `x: 9`). An `and`/`or` as the left of a comparison keeps its origin there too, which waits for #7189.

use anyhow::Result;
use tempfile::TempDir;

use crate::run_yq_files;

#[rustfmt::skip]
const ROWS: &[(&str, &str)] = &[
    ("(.k and 1) | fi", "0\n0\n0\n0\n1\n1\n1\n1\n2\n2\n2\n2\n"),
    ("(.k or 1) | fi", "0\n1\n2\n"),
    ("(.k and .k) | fi", "0\n0\n0\n1\n1\n1\n2\n2\n2\n"),
    ("(1 and .k) | fi", "0\n0\n0\n0\n0\n0\n0\n0\n0\n0\n0\n0\n"),
    ("(false or .k) | fi", "0\n0\n0\n0\n0\n0\n0\n0\n0\n0\n0\n0\n"),
    ("(.zz or .k) | fi", "0\n1\n2\n"),
    ("(.a and .k) | di", "0\n0\n0\n"),
    ("(. and .k) | fi", "0\n0\n0\n0\n0\n0\n0\n0\n0\n0\n0\n0\n"),
    ("(.k and 1) | filename", "\"b1.yaml\"\n\"b1.yaml\"\n\"b1.yaml\"\n\"b1.yaml\"\n\"b2.yaml\"\n\"b2.yaml\"\n\"b2.yaml\"\n\"b2.yaml\"\n\"b3.yaml\"\n\"b3.yaml\"\n\"b3.yaml\"\n\"b3.yaml\"\n"),
    ("(.k or 1) | di", "0\n0\n0\n"),
    ("(.zz and .k) | fi", "0\n"),
    ("(.zz or 1) | fi", "0\n0\n0\n0\n"),
    ("(.k and .zz) | fi", "0\n1\n2\n"),
    ("(.k or .zz) | filename", "\"b1.yaml\"\n\"b2.yaml\"\n\"b3.yaml\"\n"),
    ("(.a, .k) and 1 | fi", "true\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\n"),
    ("(.k and 1) | [.]", "[true]\n[true]\n[true]\n[true]\n[true]\n[true]\n[true]\n[true]\n[true]\n[true]\n[true]\n[true]\n"),
    ("(1, .k) or .l | fi", "true\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\n"),
    ("((.k and 1) and .k) | fi", "0\n0\n0\n0\n0\n0\n0\n0\n0\n0\n0\n0\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n2\n2\n2\n2\n2\n2\n2\n2\n2\n2\n2\n2\n"),
];

#[test]
fn eval_all_and_or_boolean_keeps_its_owners_origin_7191() -> Result<()> {
    let dir = TempDir::new()?;
    let paths: Vec<std::path::PathBuf> = [
        ("b1.yaml", "a: 1\nk: 10\nl: [1, 2]\n"),
        ("b2.yaml", "c: 3\nk: 20\nl: [3]\nm: {p: 1}\n"),
        ("b3.yaml", "e: 5\nk: 30\nl: []\nm: {q: 2}\n---\nx: 9\n"),
    ]
    .iter()
    .map(|(name, text)| {
        let path = dir.path().join(name);
        std::fs::write(&path, text)?;
        Ok(path)
    })
    .collect::<Result<_>>()?;
    let refs: Vec<&std::path::Path> = paths.iter().map(std::path::PathBuf::as_path).collect();
    for &(filter, expected) in ROWS {
        let (stdout, stderr, status) = run_yq_files(filter, &refs, &["ea", "-o=json", "-I=0"])?;
        // `filename` is the path as it was typed: the rows were captured with bare names.
        let mut expected = expected.to_string();
        for name in ["b1.yaml", "b2.yaml", "b3.yaml"] {
            expected = expected.replace(
                &format!("\"{name}\""),
                &format!("\"{}\"", dir.path().join(name).display()),
            );
        }
        assert_eq!(
            (stdout.as_str(), stderr.as_str(), status),
            (expected.as_str(), "", 0),
            "{filter}"
        );
    }
    Ok(())
}
