//! #7173: under `--eval-all` real yq's `//` is a `crossFunctionWithPrefs` operator like `+` and `==`
//! (`operator_alternative.go`): with every context node carrying `EvaluateTogether` it reads both operands
//! over the **whole** document list. A truthy left output is emitted as it is; any other is replaced by every
//! output of the right side over the list (kept itself when that yields none); a left side with no output runs
//! the right side once. `.a // 5` over three documents of which one has `a` is `1` and then six `5`, where
//! `succinctly yq` ran `//` once per document and printed `1 5 5`.
//!
//! Every row was captured from yq v4.53.3 over `a: 1` / `c: 3` / `e: 5` as `ea -o=json -I=0`. Not covered here,
//! and still differing: a node a literal builds carries no file index in yq (`5 | fi` is `0 0 0`, `.zz // 5 | fi`
//! is nine zeros), and a `as $x` binding or `. as $d | ...` over such a list.

use anyhow::Result;
use tempfile::TempDir;

use crate::run_yq_files;

#[rustfmt::skip]
const ROWS: &[(&str, &str)] = &[
    (".a // 5", "1\n5\n5\n5\n5\n5\n5\n"),
    (".a // .c", "1\nnull\n3\nnull\nnull\n3\nnull\n"),
    (".zz // fi", "0\n1\n2\n0\n1\n2\n0\n1\n2\n"),
    ("(.a, .c) // 7", "1\n7\n7\n7\n7\n7\n7\n7\n7\n7\n3\n7\n7\n7\n"),
    (".a // (5, 6)", "1\n5\n5\n5\n6\n6\n6\n5\n5\n5\n6\n6\n6\n"),
    (". // 5", "{\"a\":1}\n{\"c\":3}\n{\"e\":5}\n"),
    ("[.a // 5]", "[1,5,5]\n"),
    ("(.a // 5) + 1", "2\n2\n2\n"),
    ("(.a // 5) * 2", "2\n2\n2\n"),
    ("(.b // 5) + 1", "6\n6\n6\n6\n6\n6\n6\n6\n6\n"),
    ("(.a // .c) + 1", "2\n2\n2\n"),
    ("(7 // 5) + 1", "8\n8\n8\n8\n8\n8\n8\n8\n8\n"),
    ("(.a // 7) + (.a // 7)", "2\n"),
    ("(.a // 5) == 5", "false\nfalse\nfalse\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\n"),
    ("(.zz // 5) == 5", "true\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\n"),
    (".a // 5 | length", "1\n1\n1\n1\n1\n1\n1\n"),
    (".a // 5 | . + 1", "2\n6\n6\n6\n6\n6\n6\n"),
    ("(.a // 5) | [.]", "[1]\n[5]\n[5]\n[5]\n[5]\n[5]\n[5]\n"),
    (".k = (.a // 5)", "{\"a\":1,\"k\":1}\n{\"c\":3,\"k\":1}\n{\"e\":5,\"k\":1}\n"),
    (".a // .zz[]", "1\nnull\nnull\n"),
    (".zz[] // 4", "4\n4\n4\n"),
    (".a // false", "1\nfalse\nfalse\nfalse\nfalse\nfalse\nfalse\n"),
    ("(false, .a) // 9", "9\n9\n9\n9\n9\n9\n9\n9\n9\n1\n9\n9\n9\n9\n9\n9\n"),
    ("select(.a // false)", "{\"a\":1}\n"),
    (".a // 5 // 6", "1\n5\n5\n5\n5\n5\n5\n"),
    ("(.a // .zz) // 8", "1\n8\n8\n8\n8\n8\n8\n8\n8\n8\n8\n8\n8\n8\n8\n8\n8\n8\n8\n"),
];

fn files(dir: &TempDir) -> Result<Vec<std::path::PathBuf>> {
    ["a: 1\n", "c: 3\n", "e: 5\n"]
        .iter()
        .enumerate()
        .map(|(i, text)| {
            let path = dir.path().join(format!("f{}.yaml", i + 1));
            std::fs::write(&path, text)?;
            Ok(path)
        })
        .collect()
}

#[test]
fn eval_all_alternative_reads_both_operands_over_the_list_7173() -> Result<()> {
    let dir = TempDir::new()?;
    let paths = files(&dir)?;
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

/// A falsy left output makes the right side run, and an error in it ends the run, as in yq; a left that is
/// truthy everywhere never runs it.
#[test]
fn eval_all_alternative_raises_the_right_sides_error_only_when_it_runs_7173() -> Result<()> {
    let dir = TempDir::new()?;
    let paths = files(&dir)?;
    let refs: Vec<&std::path::Path> = paths.iter().map(std::path::PathBuf::as_path).collect();
    for filter in [".a // error(\"boom\")", ".zz // error(\"boom\")"] {
        let (stdout, stderr, status) = run_yq_files(filter, &refs, &["ea", "-o=json", "-I=0"])?;
        assert_eq!((stdout.as_str(), status), ("", 1), "{filter}");
        assert!(stderr.contains("boom"), "{filter}: {stderr}");
    }
    let (stdout, _, status) =
        run_yq_files(". // error(\"boom\")", &refs, &["ea", "-o=json", "-I=0"])?;
    assert_eq!(
        (stdout.as_str(), status),
        ("{\"a\":1}\n{\"c\":3}\n{\"e\":5}\n", 0)
    );
    Ok(())
}

/// Without `--eval-all` each file is its own run, and `//` is per document as before.
#[test]
fn plain_multi_file_alternative_stays_per_file_7173() -> Result<()> {
    let dir = TempDir::new()?;
    let paths = files(&dir)?;
    let refs: Vec<&std::path::Path> = paths.iter().map(std::path::PathBuf::as_path).collect();
    let (stdout, stderr, status) = run_yq_files(".a // 5", &refs, &["-o=json", "-I=0"])?;
    assert_eq!(
        (stdout.as_str(), stderr.as_str(), status),
        ("1\n5\n5\n", "", 0)
    );
    Ok(())
}
