//! #5957: the deferred bind (#3856) reads `($x).k`, `$x | (.k)` and `$x | (.k)?` as it reads `$x.k`
//! under a repeated ambient (`.users[] | ...`), where a parenthesis only groups. The answer is the
//! same either way; that the bind is deferred for the parenthesised spelling is pinned by the unit
//! tests in `deferred_bind.rs`, since only a member that does not decode shows it and jq rejects such
//! a document. Every expectation was checked against jq 1.7.1 (`/usr/bin/jq`).

use anyhow::Result;

use crate::run_jq_stdin;

const DOC: &str = r#"{"limit":2,"users":[{"n":1},{"n":3},{"n":5}]}"#;

fn run(filter: &str) -> Result<String> {
    let (out, code) = run_jq_stdin(filter, DOC, &["-c"])?;
    assert_eq!(code, 0, "{filter}");
    Ok(out.trim_end().replace('\n', " "))
}

#[test]
fn a_parenthesized_head_reads_the_member_as_the_bare_one_does() -> Result<()> {
    for (bare, paren, want) in [
        (
            ". as $x | .users[] | $x.limit",
            ". as $x | .users[] | ($x).limit",
            "2 2 2",
        ),
        (
            ". as $x | [.users[] | select(.n > $x.limit)]",
            ". as $x | [.users[] | select(.n > ($x).limit)]",
            "[{\"n\":3},{\"n\":5}]",
        ),
        (
            ". as $x | [.users[] | .n + $x.limit]",
            ". as $x | [.users[] | .n + (($x)).limit]",
            "[3,5,7]",
        ),
        (
            ". as $x | [.users[] | $x.limit?]",
            ". as $x | [.users[] | ($x).limit?]",
            "[2,2,2]",
        ),
    ] {
        assert_eq!(run(bare)?, want, "{bare}");
        assert_eq!(run(paren)?, want, "{paren}");
    }
    Ok(())
}

#[test]
fn a_parenthesized_field_stage_reads_the_member_as_the_bare_one_does() -> Result<()> {
    for (bare, paren, want) in [
        (
            ". as $x | [.users[] | $x | .limit]",
            ". as $x | [.users[] | $x | (.limit)]",
            "[2,2,2]",
        ),
        (
            ". as $x | [.users[] | $x | .limit?]",
            ". as $x | [.users[] | $x | (.limit)?]",
            "[2,2,2]",
        ),
        (
            ". as $x | [.users[] | $x | .limit?]",
            ". as $x | [.users[] | ($x) | (.limit?)]",
            "[2,2,2]",
        ),
    ] {
        assert_eq!(run(bare)?, want, "{bare}");
        assert_eq!(run(paren)?, want, "{paren}");
    }
    Ok(())
}
