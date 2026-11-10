//! #5505: a `recurse(f; cond)` whose `cond` raises late no longer converts every output delivered
//! before the raise (a cursor to its whole subtree) for a partial prefix that a yq document never
//! reads: it answers all or nothing. `recurse(f; cond)` is `--jq-extensions` surface, so these rows
//! have no yq capture; each is what the build before the change printed, so they pin that the
//! change moves the cost and not an answer, whether the raise ends the run or a consumer that stops
//! before it (`first`, `limit`, `isempty`, `any`, `all`) never reaches it.

use anyhow::Result;

use crate::run_yq_stdin_with_stderr;

const WALK: &str = r#"recurse(.[]?; if . == 3 then error("x") else true end)"#;

/// `(program around WALK, stdout, stderr, exit code)`.
const ROWS: &[(&str, &str, &str, i32)] = &[
    ("[W | 1]", "", "Error: x\n", 1),
    ("[W]", "", "Error: x\n", 1),
    ("W | 1", "", "Error: x\n", 1),
    ("[W] // \"alt\"", "", "Error: x\n", 1),
    ("(W | 1) // \"alt\"", "", "Error: x\n", 1),
    ("first(W)", "1\n", "", 0),
    ("[limit(2; W)]", "[[1,[2,3]],1]\n", "", 0),
    ("isempty(W)", "false\n", "", 0),
    ("any(W; . == 1)", "true\n", "", 0),
    ("all(W; . != 2)", "false\n", "", 0),
];

#[test]
fn test_a_recurse_cond_that_raises_late_answers_as_before_5505() -> Result<()> {
    for (program, want_out, want_err, want_code) in ROWS {
        let filter = format!("[1,[2,3]] | {}", program.replace('W', WALK));
        let (out, err, code) = run_yq_stdin_with_stderr(
            &filter,
            "null\n",
            &["--jq-extensions", "-n", "-o=json", "-I=0"],
        )?;
        assert_eq!(
            (out.as_str(), err.as_str(), code),
            (*want_out, *want_err, *want_code),
            "{program}"
        );
    }
    Ok(())
}
