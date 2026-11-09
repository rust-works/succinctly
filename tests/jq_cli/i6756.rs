//! #6756: `select`'s comparison against a literal decides a container by being one (#6350), so the
//! container the condition keeps or drops is no longer decoded on the way. A lone-surrogate
//! escape inside it passes through at exit 0 where jq 1.7.1 rejects the document at parse time
//! (exit 5, whatever the filter). That is #2103's rule, "a filter validates only what it reads",
//! the one #2692 applied to a condition that reads only truthiness, and it is recorded in
//! `docs/compliance/jq/limitations.md` ("Every M2 filter streams"). A scalar the condition
//! compares is still copied, so a scalar that cannot be decoded still raises, and a consumer that
//! goes on to read the container still raises with the parser's own words.
//!
//! Every jq column was captured from `/usr/bin/jq` 1.7.1 (exit 5 on each document).

use anyhow::Result;

use crate::{names_fault, run_jq_full};

/// A container a comparison keeps or drops is answered without reading what is inside it.
#[test]
fn select_compares_a_container_without_decoding_it_6756() -> Result<()> {
    for (doc, filter, want) in [
        (
            r#"[{"x":"\ud800"}]"#,
            r#".[] | select(. != null)"#,
            "{\"x\":\"\\ud800\"}\n",
        ),
        (r#"[{"x":"\ud800"}]"#, r#".[] | select(. == null)"#, ""),
        (
            r#"[{"x":"\ud800"}]"#,
            r#".[] | select(type == "object")"#,
            "{\"x\":\"\\ud800\"}\n",
        ),
        (
            r#"[{"x":"\ud800"}]"#,
            r#".[] | select(. != null and . != false)"#,
            "{\"x\":\"\\ud800\"}\n",
        ),
        (
            r#"{"a":{"x":"\ud800"}}"#,
            r#".[] | select(. != null)"#,
            "{\"x\":\"\\ud800\"}\n",
        ),
        (
            r#"[{"x":"\ud800"}]"#,
            r#"select(. != null)"#,
            "[{\"x\":\"\\ud800\"}]\n",
        ),
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-c", filter], Some(doc))?;
        assert_eq!(
            code, 0,
            "`{filter}` over {doc} only compares the container\nstdout: {stdout:?}\nstderr: {stderr:?}"
        );
        assert_eq!(stdout, want, "`{filter}` over {doc}");
        assert!(
            stderr.is_empty(),
            "`{filter}` over {doc}: stderr {stderr:?}"
        );
    }
    Ok(())
}

/// What the comparison does not read is read by whoever goes on to: the raise moved to the
/// consumer, with the parser's words, as it did for `select(.)` (#2692).
#[test]
fn a_consumer_of_the_kept_container_still_raises_6756() -> Result<()> {
    for filter in [
        r#"[.[] | select(. != null)] | tojson"#,
        r#".[] | select(.x != null)"#,
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-c", filter], Some(r#"[{"x":"\ud800"}]"#))?;
        assert_eq!(
            code, 5,
            "`{filter}` reads the lone surrogate\nstdout: {stdout:?}"
        );
        assert!(
            names_fault(&stderr, "surrogate pair escape"),
            "`{filter}`: stderr {stderr:?}"
        );
    }
    Ok(())
}

/// A scalar is copied to be compared, so the comparison still reads it: an escape that does not
/// decode raises, as before #6350.
#[test]
fn select_still_decodes_a_scalar_it_compares_6756() -> Result<()> {
    for filter in [
        r#".[] | select(. != null)"#,
        r#".[] | select(. == null)"#,
        r#".[] | select(. != null and . != false)"#,
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-c", filter], Some(r#"["\x"]"#))?;
        assert_eq!(code, 5, "`{filter}` decodes the scalar\nstdout: {stdout:?}");
        assert!(
            names_fault(&stderr, "invalid escape sequence"),
            "`{filter}`: stderr {stderr:?}"
        );
    }
    Ok(())
}
