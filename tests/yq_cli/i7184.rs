//! #7184: under `--eval-all` real yq's `and` and `or` are `crossFunctionWithPrefs` operators like `+` and `==`
//! (`operator_booleans.go`) over a read-only context: both operands are read over the **whole** document list (an
//! absent key is no node), and every left result is paired with every right result, with a short circuit on the
//! left (`returnLHSWhen`). `(.k and .k)` over four documents of which three have `k` is `true` nine times, where
//! `succinctly yq` ran the operators per document and printed one boolean each.
//!
//! The count is `|left| x |right|` for a left that does not decide (a truthy one under `and`, a falsy one under
//! `or`) and `|left|` otherwise; a side with no output stands as one `null` on the left and runs `false` once on
//! the right. Every row was captured from yq v4.53.3 as `ea -o=json -I=0` over `b1` (`a: 1`, `k: 10`, `l: [1, 2]`),
//! `b2` (`c: 3`, `k: 20`, `l: [3]`, `m: {p: 1}`) and `b3` (`e: 5`, `k: 30`, `l: []`, `m: {q: 2}`, then a second
//! document `x: 9`).

use anyhow::Result;
use tempfile::TempDir;

use crate::run_yq_files;

#[rustfmt::skip]
const ROWS: &[(&str, &str)] = &[
    ("(.k and .k)", "true\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\n"),
    ("(.k or .k)", "true\ntrue\ntrue\n"),
    ("(.k and 1)", "true\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\n"),
    ("(.k or 1)", "true\ntrue\ntrue\n"),
    ("false and .k", "false\nfalse\nfalse\nfalse\n"),
    ("false or .k", "true\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\n"),
    (".zz and 1", "false\n"),
    (".zz or 1", "true\ntrue\ntrue\ntrue\n"),
    ("1 and .zz", "false\nfalse\nfalse\nfalse\n"),
    (".k and .zz", "false\nfalse\nfalse\n"),
    ("1 or .zz", "true\ntrue\ntrue\ntrue\n"),
    (".k or .zz", "true\ntrue\ntrue\n"),
    (".zz and .zz", "false\n"),
    (".zz or .zz", "false\n"),
    ("true and true", "true\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\n"),
    ("false or false", "false\nfalse\nfalse\nfalse\nfalse\nfalse\nfalse\nfalse\nfalse\nfalse\nfalse\nfalse\nfalse\nfalse\nfalse\nfalse\n"),
    ("(.k > 15) and (.k < 25)", "false\nfalse\nfalse\nfalse\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\nfalse\nfalse\nfalse\nfalse\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\nfalse\nfalse\nfalse\nfalse\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\nfalse\nfalse\nfalse\nfalse\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\nfalse\nfalse\nfalse\nfalse\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\nfalse\nfalse\nfalse\nfalse\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\nfalse\nfalse\nfalse\nfalse\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\nfalse\nfalse\nfalse\nfalse\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\nfalse\nfalse\nfalse\nfalse\n"),
    ("select(.k and .l)", "{\"a\":1,\"k\":10,\"l\":[1,2]}\n{\"c\":3,\"k\":20,\"l\":[3],\"m\":{\"p\":1}}\n{\"e\":5,\"k\":30,\"l\":[],\"m\":{\"q\":2}}\n"),
    ("(.a and .k) | [.]", "[true]\n[true]\n[true]\n"),
    (".k and .l", "true\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\n"),
    (".m or .k", "true\ntrue\n"),
    ("(.k and 1) or 0", "true\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\n"),
    ("(.k and 1) and (.zz or 1)", "true\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\n"),
    (". and true", "true\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\n"),
    ("(.a, .c) and 1", "true\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\n"),
    (".l and .k | length", "true\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\n"),
    ("(.a, .zz, false) and (.k, 1)", "true\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\nfalse\nfalse\nfalse\nfalse\n"),
    ("(0, \"\", null, [], {}) and true", "true\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\nfalse\nfalse\nfalse\nfalse\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\n"),
    ("(0, \"\", null, [], {}) or false", "true\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\nfalse\nfalse\nfalse\nfalse\nfalse\nfalse\nfalse\nfalse\nfalse\nfalse\nfalse\nfalse\nfalse\nfalse\nfalse\nfalse\ntrue\ntrue\n"),
    ("(.k, false) or (.l, 2)", "true\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\n"),
    ("(.a, 1) and (.zz, 2)", "true\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\n"),
    ("(true, false, null) or (true, false)", "true\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\nfalse\nfalse\nfalse\nfalse\ntrue\ntrue\ntrue\ntrue\nfalse\nfalse\nfalse\nfalse\ntrue\ntrue\ntrue\ntrue\nfalse\nfalse\nfalse\nfalse\ntrue\ntrue\ntrue\ntrue\nfalse\nfalse\nfalse\nfalse\ntrue\ntrue\ntrue\ntrue\nfalse\nfalse\nfalse\nfalse\ntrue\ntrue\ntrue\ntrue\nfalse\nfalse\nfalse\nfalse\ntrue\ntrue\ntrue\ntrue\nfalse\nfalse\nfalse\nfalse\ntrue\ntrue\ntrue\ntrue\nfalse\nfalse\nfalse\nfalse\n"),
    ("(\"false\", \"no\", 0) and 1", "true\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\n"),
];

fn files(dir: &TempDir) -> Result<Vec<std::path::PathBuf>> {
    [
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
    .collect()
}

#[test]
fn eval_all_and_or_read_both_operands_over_the_list_7184() -> Result<()> {
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

/// The right side is read only for a left output that does not decide, so its error is raised only then.
#[test]
fn eval_all_and_or_raise_the_right_sides_error_only_when_it_is_read_7184() -> Result<()> {
    let dir = TempDir::new()?;
    let paths = files(&dir)?;
    let refs: Vec<&std::path::Path> = paths.iter().map(std::path::PathBuf::as_path).collect();
    let args = ["ea", "-o=json", "-I=0"];
    let filter = ".k and error(\"boom\")";
    let (stdout, stderr, status) = run_yq_files(filter, &refs, &args)?;
    assert_eq!((stdout.as_str(), status), ("", 1), "{filter}");
    assert!(stderr.contains("boom"), "{filter}: {stderr}");
    for (filter, expected) in [
        (".zz and error(\"boom\")", "false\n"),
        (".k or error(\"boom\")", "true\ntrue\ntrue\n"),
        ("false and error(\"boom\")", "false\nfalse\nfalse\nfalse\n"),
        ("true or error(\"boom\")", "true\ntrue\ntrue\ntrue\n"),
    ] {
        let (stdout, stderr, status) = run_yq_files(filter, &refs, &args)?;
        assert_eq!(
            (stdout.as_str(), stderr.as_str(), status),
            (expected, "", 0),
            "{filter}"
        );
    }
    Ok(())
}

/// Without `--eval-all` each document is its own run and nothing differs.
#[test]
fn plain_multi_file_and_or_stay_per_document_7184() -> Result<()> {
    let dir = TempDir::new()?;
    let paths = files(&dir)?;
    let refs: Vec<&std::path::Path> = paths.iter().map(std::path::PathBuf::as_path).collect();
    let (stdout, stderr, status) = run_yq_files(".zz and 1", &refs, &["-o=json", "-I=0"])?;
    assert_eq!(
        (stdout.as_str(), stderr.as_str(), status),
        ("false\nfalse\nfalse\nfalse\n", "", 0)
    );
    Ok(())
}
