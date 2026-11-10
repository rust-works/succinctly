//! #5505: yq drops the outputs a collected walk delivered before an error (a yq document answers
//! all or nothing), and jq mode must not: jq prints what came before an error. Every row is
//! `jq -nc PROGRAM` against what jq 1.7.1 (`/usr/bin/jq`) prints on stdout and as its exit status
//! (5, an uncaught error); the stderr text differs in its prefix (`jq: error (at <unknown>): x`).

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn test_a_walk_that_raises_late_prints_what_came_before_it_5505() -> Result<()> {
    let program = r#"[1,[2,3]] | recurse(.[]?; if . == 3 then error("x") else true end)"#;
    let (out, err, code) = run_jq_full(&["-n", "-c", program], None)?;
    assert_eq!(out, "[1,[2,3]]\n1\n[2,3]\n2\n");
    assert!(err.contains('x'), "{err}");
    assert_eq!(code, 5);
    Ok(())
}
