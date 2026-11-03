//! #5957: the deferred bind (#3856) reads `($x).k`, `$x | (.k)` and `$x | (.k)?` as it reads `$x.k`
//! under a repeated ambient (`.users[] | ...`), where a parenthesis only groups. The answer is the
//! same either way, so the first two tests compare against jq 1.7.1 (`/usr/bin/jq`); that the bind is
//! deferred shows only on a member that does not decode, which jq rejects at parse time, so the last
//! test is succinctly's own (the semi-index reads a member nothing asks for no further than its frame).

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

/// A document whose `bad` member holds a lone surrogate, which the eager bind decodes and raises on.
/// jq refuses the document outright, so there is no oracle row: this pins that the parenthesised
/// spellings leave the member undecoded as the bare one does, and that a whole-node read still does
/// not (#5957).
#[test]
fn a_parenthesized_read_leaves_an_undecodable_member_alone() -> Result<()> {
    let doc = format!(
        r#"{{"limit":2,"bad":"{}ud800","users":[{{"n":1}},{{"n":3}}]}}"#,
        '\\'
    );
    for filter in [
        ". as $x | [.users[] | $x.limit]",
        ". as $x | [.users[] | ($x).limit]",
        ". as $x | [.users[] | $x | (.limit)]",
        ". as $x | [.users[] | $x | (.limit)?]",
        ". as $x | [.users[] | ($x) | (.limit?)?]",
    ] {
        let (out, code) = run_jq_stdin(filter, &doc, &["-c"])?;
        assert_eq!((out.trim_end(), code), ("[2,2]", 0), "{filter}");
    }
    let (_, code) = run_jq_stdin(". as $x | [.users[] | ($x)] | tojson", &doc, &["-c"])?;
    assert_ne!(code, 0, "a whole-node read decodes the member");
    Ok(())
}
