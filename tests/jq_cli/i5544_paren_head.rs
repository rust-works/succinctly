//! #5544: a `reduce`/`foreach` loop variable read as the head of a field chain through a
//! parenthesis (`($u).k`, `(($u)).k`, `($u)["k"]`) converts only the members it reads of each
//! element, as `$u.k` does (#5519): a parenthesis only groups.
//!
//! The first test's expected values were captured from jq 1.7.1 (`/usr/bin/jq`). The second pins
//! what is *not* jq's, as #5519's and #5544's other tests do: a member nothing names is not
//! decoded, in a document jq refuses at parse time, so only the spellings can be compared with
//! each other and not with jq. Reading the undecodable member, or the whole element, still raises.

use anyhow::Result;

use crate::run_jq_stdin;

fn jq(filter: &str, input: &str) -> Result<(String, i32)> {
    run_jq_stdin(filter, input, &["-c"])
}

#[test]
fn a_fold_reading_a_parenthesized_field_head_matches_jq_5544() -> Result<()> {
    let users = r#"{"users":[{"id":1,"score":10,"tags":["a"]},{"id":2,"score":20,"tags":[]},{"id":3,"tags":["b","c"]}]}"#;
    for (filter, want) in [
        ("reduce .users[] as $u (0; .+($u).id)", "6\n"),
        ("reduce .users[] as $u (0; .+(($u)).score? // 0)", "30\n"),
        ("[foreach .users[] as $u (0; .+($u).id; .)]", "[1,3,6]\n"),
        (r#"reduce .users[] as $u (0; .+($u)["id"])"#, "6\n"),
        ("reduce .users[] as $u (0; .+($u).id+($u | .id))", "12\n"),
        (
            "[foreach .users[] as $u (0; .+($u).id; [., ($u|length)])]",
            "[[1,3],[3,3],[6,2]]\n",
        ),
        ("reduce .users[] as $u (0; .+($u).id+([$u]|length))", "9\n"),
        (
            "reduce .users[] as $u ([]; . + [($u).tags|length])",
            "[1,0,2]\n",
        ),
        ("reduce .users[] as $u (0; .+(($u).score // 7))", "37\n"),
    ] {
        assert_eq!(jq(filter, users)?, (want.to_string(), 0), "{filter}");
    }
    Ok(())
}

#[test]
fn a_parenthesized_head_leaves_the_members_nothing_names_undecoded_5544() -> Result<()> {
    // `"a"` holds a lone surrogate: jq refuses the document, and here only a read of it raises.
    let doc = r#"[{"a":"\ud800","b":5}]"#;
    for (spelling, want) in [
        ("$u.b", "5\n"),
        ("($u).b", "5\n"),
        ("(($u)).b", "5\n"),
        (r#"($u)["b"]"#, "5\n"),
        ("($u | .b)", "5\n"),
        ("($u).b + ($u).b", "10\n"),
        ("$u.b + ($u).b", "10\n"),
    ] {
        let filter = format!("reduce .[] as $u (0; . + ({spelling}))");
        assert_eq!(jq(&filter, doc)?, (want.to_string(), 0), "{filter}");
    }
    // A read of the member, or of the whole element, still decodes it.
    for spelling in [
        "($u).a | length",
        "($u).b + ([$u] | length)",
        "($u | length)",
        "($u)",
    ] {
        let filter = format!("reduce .[] as $u (0; . + ({spelling}))");
        let (_, status) = jq(&filter, doc)?;
        assert_eq!(status, 5, "{filter}");
    }
    Ok(())
}
