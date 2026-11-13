//! #7154: the right side of a plain or compound assignment that reads `file_index`/`filename` after a
//! `select` that dropped the node (`.k = (select(fi == 1) | fi)`). yq has no node there, so the read
//! emits nothing and `k` is `null`; `succinctly yq` resolved the read to a literal before the empty
//! context could be seen, and a literal re-emits over an empty context (`.k = (select(false) | 7)` is
//! `k: 7`, in yq too), so every file got its own index. The resolved read is now a snapshot of the value
//! rather than a literal.
//!
//! Every row was captured from yq v4.53.3 over three files (`a: 1`, `c: 3`, `e: 5`), one evaluation each.
//! What still differs, listed in `limitations.md`: `fi` and `filename` after a collect (`[.] | .[0] | fi` is
//! `0` for every file in yq, `[.] | filename` is empty), and `fi` of a `select` that keeps the whole
//! document in an assignment's right side.
//!
//! `path` after such a `select` is the same read (`.k = (select(fi == 1) | path)` is `k: null`, not `[]`), and the
//! rows also cover other targets (`.b.x =`, `.l[] =`, `+=`) and `--eval-all`.

use anyhow::Result;
use tempfile::TempDir;

use crate::run_yq_files;

/// (filter, yq's stdout).
#[rustfmt::skip]
const ROWS: &[(&str, &str)] = &[
    (".k = (select(fi == 0) | fi)", "a: 1\nk: 0\n---\nc: 3\nk: null\n---\ne: 5\nk: null\n"),
    (".k = (select(fi == 1) | fi)", "a: 1\nk: null\n---\nc: 3\nk: 1\n---\ne: 5\nk: null\n"),
    (".k = (select(fi == 2) | fi)", "a: 1\nk: null\n---\nc: 3\nk: null\n---\ne: 5\nk: 2\n"),
    (".k = (select(fi != 0) | fi)", "a: 1\nk: null\n---\nc: 3\nk: 1\n---\ne: 5\nk: 2\n"),
    (".k = (select(fi < 2) | fi)", "a: 1\nk: 0\n---\nc: 3\nk: 1\n---\ne: 5\nk: null\n"),
    (".k = (select(fi == 0 or fi == 2) | fi)", "a: 1\nk: 0\n---\nc: 3\nk: null\n---\ne: 5\nk: 2\n"),
    (".k = (select(fi == 1) | fi + 0)", "a: 1\nk: null\n---\nc: 3\nk: 1\n---\ne: 5\nk: null\n"),
    (".k = (select(fi == 1) | fi * 10)", "a: 1\nk: null\n---\nc: 3\nk: 10\n---\ne: 5\nk: null\n"),
    (".k = (select(fi == 1) | [fi])", "a: 1\nk: []\n---\nc: 3\nk:\n  - 1\n---\ne: 5\nk: []\n"),
    (".k = (select(fi == 1) | filename)", "a: 1\nk: null\n---\nc: 3\nk: @DIR@/f2.yaml\n---\ne: 5\nk: null\n"),
    (".k = (select(.c == 3) | fi)", "a: 1\nk: null\n---\nc: 3\nk: 1\n---\ne: 5\nk: null\n"),
    (".k = (select(.a == 1) | fi)", "a: 1\nk: 0\n---\nc: 3\nk: null\n---\ne: 5\nk: null\n"),
    (".k = (select(.c == 3) | filename)", "a: 1\nk: null\n---\nc: 3\nk: @DIR@/f2.yaml\n---\ne: 5\nk: null\n"),
    (".k = (select(di == 1) | fi)", "a: 1\nk: null\n---\nc: 3\nk: null\n---\ne: 5\nk: null\n"),
    (".k = (select(fi == 1) | .c)", "a: 1\nk: null\n---\nc: 3\nk: 3\n---\ne: 5\nk: null\n"),
    (".k = (select(fi == 1) | .c | fi)", "a: 1\nk: null\n---\nc: 3\nk: 1\n---\ne: 5\nk: null\n"),
    (".k = ((select(fi == 0), select(fi == 1)) | fi)", "a: 1\nk: 0\n---\nc: 3\nk: 1\n---\ne: 5\nk: null\n"),
    (".k += (select(fi == 1) | fi)", "a: 1\nk: null\n---\nc: 3\nk: 1\n---\ne: 5\nk: null\n"),
    (".k |= (select(fi == 0) | fi)", "a: 1\nk: 0\n---\nc: 3\nk: null\n---\ne: 5\nk: null\n"),
    ("(.a, .k) = (select(fi == 1) | fi)", "a: 1\nk: null\n---\nc: 3\na: 1\nk: 1\n---\ne: 5\na: null\nk: null\n"),
    (".k = (select(fi == 1) | 7)", "a: 1\nk: 7\n---\nc: 3\nk: 7\n---\ne: 5\nk: 7\n"),
    (".k = (select(fi == 7) | 7)", "a: 1\nk: 7\n---\nc: 3\nk: 7\n---\ne: 5\nk: 7\n"),
    (".k = (select(false) | 7)", "a: 1\nk: 7\n---\nc: 3\nk: 7\n---\ne: 5\nk: 7\n"),
    (".k = (select(false) | fi)", "a: 1\nk: null\n---\nc: 3\nk: null\n---\ne: 5\nk: null\n"),
    (".k = (select(fi == 0) | .a)", "a: 1\nk: 1\n---\nc: 3\nk: null\n---\ne: 5\nk: null\n"),
    (".k = (fi | select(. == 0))", "a: 1\nk: 0\n---\nc: 3\nk: null\n---\ne: 5\nk: null\n"),
    (".k = (select(fi == 1) | \"x\")", "a: 1\nk: x\n---\nc: 3\nk: x\n---\ne: 5\nk: x\n"),
    (".k = (.zz | fi)", "a: 1\nk: null\n---\nc: 3\nk: null\n---\ne: 5\nk: null\n"),
    (".k = (.zz | 7)", "a: 1\nk: 7\n---\nc: 3\nk: 7\n---\ne: 5\nk: 7\n"),
    (".k = (.a | fi)", "a: 1\nk: 0\n---\nc: 3\nk: null\n---\ne: 5\nk: null\n"),
];

#[test]
fn assignment_reads_of_the_file_after_a_select_that_drops_the_node_7154() -> Result<()> {
    let dir = TempDir::new()?;
    let paths: Vec<_> = ["a: 1\n", "c: 3\n", "e: 5\n"]
        .iter()
        .enumerate()
        .map(|(i, text)| {
            let path = dir.path().join(format!("f{}.yaml", i + 1));
            std::fs::write(&path, text).map(|()| path)
        })
        .collect::<std::io::Result<_>>()?;
    let refs: Vec<&std::path::Path> = paths.iter().map(std::path::PathBuf::as_path).collect();
    for &(filter, expected) in ROWS {
        let (stdout, stderr, status) = run_yq_files(filter, &refs, &[])?;
        assert_eq!(
            (
                stdout
                    .replace(&dir.path().display().to_string(), "@DIR@")
                    .as_str(),
                stderr.as_str(),
                status
            ),
            (expected, "", 0),
            "{filter}"
        );
    }
    Ok(())
}

/// (`e` or `ea`, filter, yq's stdout; `ea` under `-o=json -I=0`) over two files, `l: [1, 7, 3]` / `m: {p: 1, q: 9}`
/// and `l: [8]` / `m: {p: 5}`: `path` after a dropping `select`, other targets, and the same under `--eval-all`.
#[rustfmt::skip]
const MORE_ROWS: &[(&str, &str, &str)] = &[
    ("e", ".k = (select(fi == 1) | path)", "l: [1, 7, 3]\nm: {p: 1, q: 9}\nk: null\n---\nl: [8]\nm: {p: 5}\nk: []\n"),
    ("e", ".k = (select(fi == 1) | path | length)", "l: [1, 7, 3]\nm: {p: 1, q: 9}\nk: null\n---\nl: [8]\nm: {p: 5}\nk: 0\n"),
    ("e", ".k = (select(fi == 0) | path)", "l: [1, 7, 3]\nm: {p: 1, q: 9}\nk: []\n---\nl: [8]\nm: {p: 5}\nk: null\n"),
    ("e", ".k = (.l[] | select(. > 5) | key)", "l: [1, 7, 3]\nm: {p: 1, q: 9}\nk: 1\n---\nl: [8]\nm: {p: 5}\nk: 0\n"),
    ("e", ".k = (.l[] | select(. > 100) | key)", "l: [1, 7, 3]\nm: {p: 1, q: 9}\nk: null\n---\nl: [8]\nm: {p: 5}\nk: null\n"),
    ("e", ".k = (.l[] | select(. > 100) | path)", "l: [1, 7, 3]\nm: {p: 1, q: 9}\nk: null\n---\nl: [8]\nm: {p: 5}\nk: null\n"),
    ("e", ".k = (.l[] | select(. > 5) | path)", "l: [1, 7, 3]\nm: {p: 1, q: 9}\nk:\n  - l\n  - 1\n---\nl: [8]\nm: {p: 5}\nk:\n  - l\n  - 0\n"),
    ("e", ".k = (.m | select(.p > 100) | path)", "l: [1, 7, 3]\nm: {p: 1, q: 9}\nk: null\n---\nl: [8]\nm: {p: 5}\nk: null\n"),
    ("e", ".k = (.m | select(.p < 100) | path)", "l: [1, 7, 3]\nm: {p: 1, q: 9}\nk:\n  - m\n---\nl: [8]\nm: {p: 5}\nk:\n  - m\n"),
    ("e", ".k = (.zz | path)", "l: [1, 7, 3]\nm: {p: 1, q: 9}\nk: null\n---\nl: [8]\nm: {p: 5}\nk: null\n"),
    ("e", ".b.x = (select(fi == 1) | fi)", "l: [1, 7, 3]\nm: {p: 1, q: 9}\nb:\n  x: null\n---\nl: [8]\nm: {p: 5}\nb:\n  x: 1\n"),
    ("e", ".l[] = (select(fi == 1) | fi)", "l: [1, 7, 3]\nm: {p: 1, q: 9}\n---\nl: [1]\nm: {p: 5}\n"),
    ("e", ".l[0] = (select(fi == 1) | fi)", "l: [1, 7, 3]\nm: {p: 1, q: 9}\n---\nl: [1]\nm: {p: 5}\n"),
    ("e", ".m.p = (select(fi == 1) | fi)", "l: [1, 7, 3]\nm: {p: 1, q: 9}\n---\nl: [8]\nm: {p: 1}\n"),
    ("e", ".m.p += (select(fi == 1) | fi)", "l: [1, 7, 3]\nm: {p: 1, q: 9}\n---\nl: [8]\nm: {p: 6}\n"),
    ("e", ".m.p = (select(fi == 1) | .l[0])", "l: [1, 7, 3]\nm: {p: 1, q: 9}\n---\nl: [8]\nm: {p: 8}\n"),
    ("ea", ".k = (select(fi == 1) | path)", "{\"l\":[1,7,3],\"m\":{\"p\":1,\"q\":9},\"k\":[]}\n{\"l\":[8],\"m\":{\"p\":5},\"k\":[]}\n"),
    ("ea", ".k = (select(fi == 1) | path | length)", "{\"l\":[1,7,3],\"m\":{\"p\":1,\"q\":9},\"k\":0}\n{\"l\":[8],\"m\":{\"p\":5},\"k\":0}\n"),
    ("ea", ".k = (select(fi == 0) | path)", "{\"l\":[1,7,3],\"m\":{\"p\":1,\"q\":9},\"k\":[]}\n{\"l\":[8],\"m\":{\"p\":5},\"k\":[]}\n"),
    ("ea", ".k = (.l[] | select(. > 5) | key)", "{\"l\":[1,7,3],\"m\":{\"p\":1,\"q\":9},\"k\":0}\n{\"l\":[8],\"m\":{\"p\":5},\"k\":0}\n"),
    ("ea", ".k = (.l[] | select(. > 100) | key)", "{\"l\":[1,7,3],\"m\":{\"p\":1,\"q\":9},\"k\":null}\n{\"l\":[8],\"m\":{\"p\":5},\"k\":null}\n"),
    ("ea", ".k = (.l[] | select(. > 100) | path)", "{\"l\":[1,7,3],\"m\":{\"p\":1,\"q\":9},\"k\":null}\n{\"l\":[8],\"m\":{\"p\":5},\"k\":null}\n"),
    ("ea", ".k = (.l[] | select(. > 5) | path)", "{\"l\":[1,7,3],\"m\":{\"p\":1,\"q\":9},\"k\":[\"l\",0]}\n{\"l\":[8],\"m\":{\"p\":5},\"k\":[\"l\",0]}\n"),
    ("ea", ".k = (.m | select(.p > 100) | path)", "{\"l\":[1,7,3],\"m\":{\"p\":1,\"q\":9},\"k\":null}\n{\"l\":[8],\"m\":{\"p\":5},\"k\":null}\n"),
    ("ea", ".k = (.m | select(.p < 100) | path)", "{\"l\":[1,7,3],\"m\":{\"p\":1,\"q\":9},\"k\":[\"m\"]}\n{\"l\":[8],\"m\":{\"p\":5},\"k\":[\"m\"]}\n"),
    ("ea", ".k = (.zz | path)", "{\"l\":[1,7,3],\"m\":{\"p\":1,\"q\":9},\"k\":null}\n{\"l\":[8],\"m\":{\"p\":5},\"k\":null}\n"),
    ("ea", ".b.x = (select(fi == 1) | fi)", "{\"l\":[1,7,3],\"m\":{\"p\":1,\"q\":9},\"b\":{\"x\":1}}\n{\"l\":[8],\"m\":{\"p\":5},\"b\":{\"x\":1}}\n"),
    ("ea", ".l[] = (select(fi == 1) | fi)", "{\"l\":[1,1,1],\"m\":{\"p\":1,\"q\":9}}\n{\"l\":[1],\"m\":{\"p\":5}}\n"),
    ("ea", ".l[0] = (select(fi == 1) | fi)", "{\"l\":[1,7,3],\"m\":{\"p\":1,\"q\":9}}\n{\"l\":[1],\"m\":{\"p\":5}}\n"),
    ("ea", ".m.p = (select(fi == 1) | fi)", "{\"l\":[1,7,3],\"m\":{\"p\":1,\"q\":9}}\n{\"l\":[8],\"m\":{\"p\":1}}\n"),
    ("ea", ".m.p += (select(fi == 1) | fi)", "{\"l\":[1,7,3],\"m\":{\"p\":2,\"q\":9}}\n{\"l\":[8],\"m\":{\"p\":6}}\n"),
    ("ea", ".m.p = (select(fi == 1) | .l[0])", "{\"l\":[1,7,3],\"m\":{\"p\":8,\"q\":9}}\n{\"l\":[8],\"m\":{\"p\":8}}\n"),
];

#[test]
fn assignment_reads_of_path_and_other_targets_after_a_select_7154() -> Result<()> {
    let dir = TempDir::new()?;
    let paths: Vec<_> = ["l: [1, 7, 3]\nm: {p: 1, q: 9}\n", "l: [8]\nm: {p: 5}\n"]
        .iter()
        .enumerate()
        .map(|(i, text)| {
            let path = dir.path().join(format!("f{}.yaml", i + 1));
            std::fs::write(&path, text).map(|()| path)
        })
        .collect::<std::io::Result<_>>()?;
    let refs: Vec<&std::path::Path> = paths.iter().map(std::path::PathBuf::as_path).collect();
    for &(mode, filter, expected) in MORE_ROWS {
        let extra: &[&str] = if mode == "ea" {
            &["ea", "-o=json", "-I=0"]
        } else {
            &[]
        };
        let (stdout, stderr, status) = run_yq_files(filter, &refs, extra)?;
        assert_eq!(
            (
                stdout
                    .replace(&dir.path().display().to_string(), "@DIR@")
                    .as_str(),
                stderr.as_str(),
                status
            ),
            (expected, "", 0),
            "{mode} {filter}"
        );
    }
    Ok(())
}
