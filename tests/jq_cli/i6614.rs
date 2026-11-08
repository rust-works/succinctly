//! #6614: a document nested more than 64 levels (a chain of 40 nodes is 80: an object and its
//! `children` array per node) was read 2x slower than one of 60 levels, because the one-pass text
//! reader the conversion tries on every child gave up at 64 levels after a full descent and the walk
//! then tried each child again. The reader now reads to the validator's 128 levels; what is deeper
//! still goes the general way. Every answer is jq 1.7.1's: the chain's ids in order, and its
//! refusal past its own nesting limit.

use anyhow::Result;

use crate::run_jq_full;

/// `n` nodes nested in a chain: `{"id":1,"children":[{"id":2,"children":[...]}]}`.
fn chain(n: usize) -> String {
    let mut text = format!(r#"{{"id":{n},"children":[]}}"#);
    for id in (1..n).rev() {
        text = format!(r#"{{"id":{id},"children":[{text}]}}"#);
    }
    text
}

/// 30 nodes is 60 levels (always read in one pass), 40 to 64 are 80 to 128 (the new range), 65 and
/// 70 are 130 and 140 (past the reader's limit, still read, by the walk).
#[test]
fn a_chain_across_the_text_readers_limit_answers_as_jq_6614() -> Result<()> {
    for n in [30_usize, 40, 50, 63, 64, 65, 70] {
        let input = chain(n);
        let ids = (1..=n)
            .map(|id| id.to_string())
            .collect::<Vec<_>>()
            .join(",");
        let (stdout, stderr, code) =
            run_jq_full(&["-c", "[recurse(.children[]?) | .id]"], Some(&input))?;
        assert_eq!(
            (stdout.as_str(), stderr.as_str(), code),
            (format!("[{ids}]\n").as_str(), "", 0),
            "{n} nodes"
        );
        let (stdout, _, code) = run_jq_full(&["-c", "."], Some(&input))?;
        assert_eq!(
            (stdout, code),
            (format!("{input}\n"), 0),
            "{n} nodes, identity"
        );
        let (stdout, _, code) = run_jq_full(&["-c", "[..|.id?] | length"], Some(&input))?;
        assert_eq!(
            (stdout.as_str(), code),
            (format!("{n}\n").as_str(), 0),
            "{n} nodes, `..`"
        );
    }
    Ok(())
}

/// Past jq's own nesting limit (90 nodes is 180 levels) both refuse, with the same status.
#[test]
fn a_chain_past_the_nesting_limit_is_refused_as_jq_6614() -> Result<()> {
    let input = chain(90);
    let (stdout, _, code) = run_jq_full(
        &["-c", "[recurse(.children[]?) | .id] | length"],
        Some(&input),
    )?;
    assert_eq!((stdout.as_str(), code), ("", 5));
    Ok(())
}
