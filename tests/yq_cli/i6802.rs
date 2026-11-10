//! #6802: yq's `ireduce` folds a *list* of accumulators (`reduceOperator`): every output of UPDATE is
//! an accumulator of the next step, a union in UPDATE is branch-major over the list, an UPDATE with
//! no output drops its accumulator, and several INIT outputs all fold the one source (jq's later
//! forks read a `null` source). Every row is `yq -o=json -I0 FILTER` on [`DOC`], captured from yq
//! v4.53.3. `docs/compliance/yq/limitations.md` records the absent-key read that still differs.

use anyhow::Result;

use crate::{run_yq_stdin, run_yq_stdin_with_stderr};

const DOC: &str = "l: [1, 2]\nm: {a: 1, b: 2}\ne: []\n";

/// (filter, stdout), each the same as yq v4.53.3's with exit code 0.
const ROWS: &[(&str, &str)] = &[
    (".l[] as $i ireduce((0, 100); . + $i)", "3\n103\n"),
    (".l[] as $i ireduce(0; select(false))", ""),
    (".l[] as $i ireduce(0; select(. < 1))", "0\n"),
    (".l[] as $i ireduce(0; (. + $i, . * 10))", "3\n2\n10\n0\n"),
    (
        ".l[] as $i ireduce((1, 2); (. + $i, . * 10))",
        "4\n5\n12\n22\n20\n30\n100\n200\n",
    ),
    ("[.l[] as $i ireduce(0; (. + $i, . * 10))]", "[3,2,10,0]\n"),
    (
        ".l[] as $i ireduce(0; ((. + $i), (. + $i)))",
        "3\n3\n3\n3\n",
    ),
    (".l[] as $i ireduce(0; . + $i)", "3\n"),
    (".e[] as $i ireduce(5; . + $i)", "5\n"),
    (".m[] as $i ireduce(0; . + $i)", "3\n"),
    (".l[] as $i ireduce([]; . + [$i])", "[1,2]\n"),
    (
        ".l[] as $i ireduce((0, 10); (. + $i, . - $i))",
        "3\n13\n1\n11\n-1\n9\n-3\n7\n",
    ),
    (".l[] as $i ireduce(0; (., .))", "0\n"),
    (".l[] as $i ireduce({}; .a += $i)", "{\"a\":3}\n"),
    (
        ".l[] as $i ireduce((1, 2, 3); select(. != 2) | . + $i)",
        "6\n",
    ),
    (
        ".l[] as $i ireduce(0; (. + 1) , (. + 2) | . * 2)",
        "2\n5\n6\n12\n",
    ),
    (".l[] as $i ireduce(0; [.] | .[0] + $i)", "3\n"),
    (".l[] as $i ireduce(0; . + $i) | . + 1", "4\n"),
];

#[test]
fn every_row_matches_yq_6802() -> Result<()> {
    for (filter, expected) in ROWS {
        let (stdout, code) = run_yq_stdin(filter, DOC, &["-o", "json", "-I0"])?;
        assert_eq!(code, 0, "{filter}");
        assert_eq!(&stdout, expected, "{filter}");
    }
    Ok(())
}

/// Under `--eval-all` the accumulators fold the document list: `0` is one seed per document and each
/// seed's UPDATE fans out over the list of accumulators.
#[test]
fn the_accumulator_list_folds_the_document_list_under_eval_all_6802() -> Result<()> {
    let (stdout, stderr, code) = run_yq_stdin_with_stderr(
        ". as $i ireduce (0; (. + $i.a, . * 10))",
        "a: 1\n---\na: 2\n",
        &["--eval-all", "-o", "json", "-I0"],
    )?;
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(stdout, "3\n3\n2\n2\n10\n10\n0\n0\n");
    Ok(())
}

/// An error in UPDATE ends the run with no output.
#[test]
fn an_error_in_the_update_prints_nothing_6802() -> Result<()> {
    let (stdout, code) = run_yq_stdin(
        ".l[] as $i ireduce (0; error(\"x\"))",
        DOC,
        &["-o", "json", "-I0"],
    )?;
    assert_ne!(code, 0);
    assert_eq!(stdout, "");
    Ok(())
}
