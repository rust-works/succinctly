//! #5519: a `reduce`/`foreach` whose UPDATE (and EXTRACT) read its loop variable only as
//! `$u.field` converts just those members of each element, not the element's whole subtree.
//!
//! The first test's expected text and exit status were captured from jq 1.7.1 (`/usr/bin/jq`):
//! members the fold does not read, an absent field, a repeated key, a non-object element, a
//! read through a builtin or a comparison, a whole-variable read, a destructuring pattern. So
//! are the malformed-object rows of the second. The last two tests pin what is *not* jq's: a
//! member nothing reads is not decoded (the document is one jq refuses at parse time).

use anyhow::Result;

use crate::run_jq_stdin;

fn jq(filter: &str, input: &str) -> Result<(String, i32)> {
    run_jq_stdin(filter, input, &["-c"])
}

#[test]
fn test_a_fold_reading_fields_of_its_variable_matches_jq_5519() -> Result<()> {
    for (filter, input, want, status) in [
        (
            "reduce .[] as $u (0; . + $u.score)",
            r#"[{"id":1,"name":"a","tags":["x","y"],"score":3,"extra":{"k":[1,2,{"z":null}]}},{"score":4,"id":2},{"id":3}]"#,
            "7\n",
            0,
        ),
        (
            "reduce .[] as $u (0; . + 1)",
            r#"[{"a":1},{"b":[1,2]},7,"s",null,[3]]"#,
            "6\n",
            0,
        ),
        (
            "reduce .[] as $u (0; . + ($u.a.b // 10))",
            r#"[{"a":{"b":1,"c":2},"d":3},{"a":{"c":2}},{"a":null}]"#,
            "21\n",
            0,
        ),
        (
            "reduce .[] as $u ({}; .[$u.k] = $u.v)",
            r#"[{"k":"a","v":[1,{"w":2}],"pad":"zzz"},{"k":"b","v":null,"pad":{"p":1}},{"k":"a","v":3}]"#,
            "{\"a\":3,\"b\":null}\n",
            0,
        ),
        (
            "reduce .[] as $u (0; . + $u.n)",
            r#"[{"n":1,"n":2},{"n":5,"m":1,"n":7}]"#,
            "9\n",
            0,
        ),
        (
            "[foreach .[] as $u (0; . + 1; [., $u.a, $u.b?])]",
            r#"[{"a":1,"b":2,"c":3},{"a":4},{"c":0}]"#,
            "[[1,1,2],[2,4,null],[3,null,null]]\n",
            0,
        ),
        (
            "[foreach .[] as $u (0; . + 1; $u.name)]",
            r#"[{"name":"x","big":[1,2,3]},{"name":"y"}]"#,
            "[\"x\",\"y\"]\n",
            0,
        ),
        (
            "[foreach .[] as $u (0; . + $u.s; [., $u.t])]",
            r#"[{"s":1,"t":"a","u":1},{"s":2,"t":"b","u":2}]"#,
            "[[1,\"a\"],[3,\"b\"]]\n",
            0,
        ),
        ("reduce .[] as $u (0; . + $u.a)", r#"[{"a":1},5]"#, "", 5),
        (
            "reduce .[] as $u (0; (try $u.a catch 7) + .)",
            r#"[1,{"a":3,"b":4},null]"#,
            "10\n",
            0,
        ),
        (
            "reduce .[] as $u (null; . + ($u | .a))",
            r#"[{"a":1,"z":[1]},{"a":2}]"#,
            "3\n",
            0,
        ),
        (
            "reduce .[] as $u (0; . + ($u | length))",
            r#"[{"a":1,"b":2},{"c":3}]"#,
            "3\n",
            0,
        ),
        (
            "reduce .[] as $u ([]; . + [$u])",
            r#"[{"a":1,"b":2},{"c":3}]"#,
            "[{\"a\":1,\"b\":2},{\"c\":3}]\n",
            0,
        ),
        (
            r#"reduce .[] as $u (0; . + $u["a"])"#,
            r#"[{"a":1,"b":2}]"#,
            "1\n",
            0,
        ),
        (
            "reduce .[] as $u (0; . + $u.a + ($u | .b))",
            r#"[{"a":1,"b":2,"c":9}]"#,
            "3\n",
            0,
        ),
        (
            "reduce .[] as {a: $x} (0; . + $x)",
            r#"[{"a":1,"b":2}]"#,
            "1\n",
            0,
        ),
        (
            "reduce .[] as $u (0; . + $u.a) | [., $__loc__.line]",
            r#"[{"a":1}]"#,
            "[1,1]\n",
            0,
        ),
        (
            "reduce .[] as $u (0; . + ($u.a | floor))",
            r#"[{"a":1.5,"b":"x"},{"a":2.5}]"#,
            "3\n",
            0,
        ),
        (
            "reduce .[] as $u (0; if $u.a == $u.b then . + 1 else . end)",
            r#"[{"a":1,"b":1,"c":9},{"a":1,"b":2}]"#,
            "1\n",
            0,
        ),
        (
            "reduce .[] as $u (0; . - $u.a)",
            r#"[{"a":1,"b":1,"c":9},{"a":3}]"#,
            "-4\n",
            0,
        ),
        (
            "reduce .[] as $u (false; $u.b == $u.b)",
            r#"[{"b":[nan],"c":1}]"#,
            "true\n",
            0,
        ),
        (
            "reduce .[] as $u (0; . + ($u.a | length))",
            r#"[{"a":"xy","b":1},{"a":[1,2,3]}]"#,
            "5\n",
            0,
        ),
        (
            r#"reduce .[] as $u (""; . + $u.a)"#,
            r#"[{"a":"x","b":1},{"a":"y"}]"#,
            "\"xy\"\n",
            0,
        ),
        (
            "[reduce .[] as $u (0; . + $u.b)]",
            r#"[{"b":1,"c":[1]},{"b":2}]"#,
            "[3]\n",
            0,
        ),
        (
            "first(foreach .[] as $u (0; . + $u.b; .))",
            r#"[{"b":1,"c":[1]},{"b":2}]"#,
            "1\n",
            0,
        ),
    ] {
        assert_eq!(
            jq(filter, input)?,
            (want.to_string(), status),
            "{filter} {input}"
        );
    }
    Ok(())
}

/// A member the fold reads is walked as the whole conversion walks it: an object whose own frame
/// is malformed (a stray or missing `,` or `:`, a key that is not a string, a missing value) fails
/// the fold, as it does in jq, whether or not the members that survive are the ones read.
#[test]
// `[{:1}]` is a malformed document, not a format string.
#[allow(clippy::literal_string_with_formatting_args)]
fn test_a_malformed_object_still_fails_a_fold_that_reads_it_5519() -> Result<()> {
    for input in [
        "[{,}]",
        r#"[{"a"}]"#,
        r#"[{"a":}]"#,
        "[{:1}]",
        r#"[{"a":1},{"a"}]"#,
        r#"[{"a":1,}]"#,
        r#"[{"a":1 "b":2}]"#,
        r#"[{"a":1,,"b":2}]"#,
        r#"[{123:1,"a":2}]"#,
    ] {
        assert_eq!(
            jq("reduce .[] as $u (0; . + $u.a)", input)?,
            (String::new(), 5),
            "{input}"
        );
        assert_eq!(
            jq("[foreach .[] as $u (0; . + 1; $u.a)]", input)?,
            (String::new(), 5),
            "{input}"
        );
    }
    // A member nested past the depth limit fails the fold that reads it.
    let deep = format!(r#"[{{"a":{}1{},"b":2}}]"#, "[".repeat(255), "]".repeat(255));
    assert_eq!(jq("reduce .[] as $u (0; $u.a)", &deep)?, (String::new(), 5));
    Ok(())
}

/// A member the fold never reads is not decoded, so an undecodable one no longer fails the fold:
/// the answer `.[] as $x | $x.b` and `.[] | .b` already give on the same document (a filter
/// validates only what it materializes, #2103/#2658; jq itself refuses the document at parse
/// time). A member the fold does read, and a fold that reads its variable whole, still fail. Each
/// fold entry point the projection is wired through has a row, and so does each spelling of the
/// read (a builtin, a comparison, an arithmetic operator), which all answer alike.
#[test]
fn test_an_undecodable_member_nothing_reads_is_not_decoded_5519() -> Result<()> {
    let input = r#"[{"a":"\ud800","b":5}]"#;
    for (filter, want, status) in [
        ("reduce .[] as $u (0; . + $u.b)", "5\n", 0),
        ("[reduce .[] as $u (0; . + $u.b)]", "[5]\n", 0),
        ("reduce .[] as $u (0; . + 1)", "1\n", 0),
        ("[foreach .[] as $u (0; . + $u.b; .)]", "[5]\n", 0),
        ("first(foreach .[] as $u (0; . + $u.b; .))", "5\n", 0),
        (
            "[limit(1; foreach .[] as $u (0; . + $u.b; $u.b))]",
            "[5]\n",
            0,
        ),
        ("reduce .[] as $u (0; . + ($u.b | floor))", "5\n", 0),
        ("reduce .[] as $u (0; . - $u.b)", "-5\n", 0),
        (
            "reduce .[] as $u (0; if $u.b == 5 then . + 1 else . end)",
            "1\n",
            0,
        ),
        (".[] as $u | $u.b", "5\n", 0),
        ("reduce .[] as $u (0; . + ($u.a | length))", "", 5),
        ("reduce .[] as $u (0; . + ($u | length))", "", 5),
        ("reduce .[] as $u ([]; . + [$u])", "", 5),
        (
            "reduce .[] as $u (0; . + $u.b + ($u[\"a\"] | length))",
            "",
            5,
        ),
    ] {
        assert_eq!(
            jq(filter, input)?,
            (want.to_string(), status),
            "{filter} {input}"
        );
    }
    Ok(())
}

/// A fold that reads its variable in path position keeps the element whole, so the error names
/// every member of it. A dynamic path target is refused outright by the holdout build.
#[test]
#[cfg_attr(feature = "unshared-containers", ignore = "holdout refusal (#5246)")]
fn test_a_path_position_read_still_names_the_whole_element_5519() -> Result<()> {
    let want = "\"Invalid path expression near attempt to access element \\\"a\\\" of {\\\"a\\\":1,\\\"b\\\":2}\"\n";
    for filter in [
        "reduce .[] as $u (0; try ($u.a = 3) catch .)",
        "reduce .[] as $u (0; try path($u.a) catch .)",
    ] {
        assert_eq!(
            jq(filter, r#"[{"a":1,"b":2}]"#)?,
            (want.to_string(), 0),
            "{filter}"
        );
    }
    Ok(())
}
