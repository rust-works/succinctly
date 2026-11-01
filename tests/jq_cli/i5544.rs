//! #5544: a `reduce`/`foreach` whose single pattern is an object pattern of literal keys converts
//! only those keys of each element, as #5519 does for `$u.field`: the pattern binds names to
//! members, never the element, so nothing else of it is read.
//!
//! The first two tests' expected text, stderr and exit status were captured from jq 1.7.1
//! (`/usr/bin/jq`): members the pattern does not name, an absent key, a repeated key, a non-object
//! element, a nested and an array sub-pattern, a nested computed key, a computed key and a `?//`
//! alternation, which keep the element whole. The malformed-object test pins jq's exit status. The
//! last test pins what is *not* jq's, as #5519's do: a member no key names is not decoded (the
//! document is one jq refuses at parse time).

use anyhow::Result;

use crate::{run_jq_full, run_jq_stdin};

fn jq(filter: &str, input: &str) -> Result<(String, i32)> {
    run_jq_stdin(filter, input, &["-c"])
}

#[test]
fn test_a_fold_destructuring_literal_keys_matches_jq_5544() -> Result<()> {
    for (filter, input, want, status) in [
        (
            "reduce .[] as {a:$x} (0; . + $x)",
            r#"[{"id":1,"a":3,"tags":["x"],"extra":{"k":[1,{"z":null}]}},{"a":4,"id":2},{"id":3}]"#,
            "7\n",
            0,
        ),
        (
            "reduce .[] as {a:$x} (0; . + ($x // 10))",
            r#"[{"a":1},{"b":2},{"a":null}]"#,
            "21\n",
            0,
        ),
        (
            "reduce .[] as {$a} (0; . + $a)",
            r#"[{"a":1,"b":9},{"a":2}]"#,
            "3\n",
            0,
        ),
        (
            r#"reduce .[] as {"a":$x} (0; . + $x)"#,
            r#"[{"a":1,"b":9},{"a":2}]"#,
            "3\n",
            0,
        ),
        (
            "reduce .[] as {a:$x, b:$y} (0; . + $x * $y)",
            r#"[{"a":2,"b":3,"c":7},{"a":4,"b":5}]"#,
            "26\n",
            0,
        ),
        (
            "reduce .[] as {a:$x} (0; . + $x)",
            r#"[{"a":1,"a":2},{"a":5,"b":1,"a":7}]"#,
            "9\n",
            0,
        ),
        (
            "reduce .[] as {a:{b:$x}} (0; . + $x)",
            r#"[{"a":{"b":1,"c":2},"d":3},{"a":{"b":4}}]"#,
            "5\n",
            0,
        ),
        (
            "reduce .[] as {a:[$x,$y]} (0; . + $x + $y)",
            r#"[{"a":[1,2,3],"z":0},{"a":[4,5]}]"#,
            "12\n",
            0,
        ),
        (
            "reduce .[] as {$k: [$x]} (0; . + $x)",
            r#"[{"k":[3,4]},{"k":[5]}]"#,
            "8\n",
            0,
        ),
        (
            "reduce .[] as {a:$x} (0; . + 1)",
            r#"[{"a":1},{"b":[1,2]},{"a":{"z":1}}]"#,
            "3\n",
            0,
        ),
        (
            "[foreach .[] as {a:$x} (0; . + $x; [., $x])]",
            r#"[{"a":1,"p":[1]},{"a":2},{"b":0}]"#,
            "[[1,1],[3,2],[3,null]]\n",
            0,
        ),
        (
            "[foreach .[] as {s:$s} (0; . + $s; .)]",
            r#"[{"s":1,"t":2},{"s":3}]"#,
            "[1,4]\n",
            0,
        ),
        ("reduce .[] as {a:$x} (0; . + $x)", r#"[{"a":1},5]"#, "", 5),
        (
            "reduce .[] as {a:$x} (0; . + $x)",
            r#"[{"a":1},[2]]"#,
            "",
            5,
        ),
        (
            "reduce .[] as {a:$x} (0; . + $x)",
            r#"[{"a":1},"s"]"#,
            "",
            5,
        ),
        (
            "reduce .[] as {a:$x} (0; . + ($x // 100))",
            r#"[{"a":1},null]"#,
            "101\n",
            0,
        ),
        (
            "reduce .[] as {a:$x} (0; (try ($x + 1) catch 7) + .)",
            r#"[1,{"a":3},null]"#,
            "",
            5,
        ),
        (
            r#"reduce .[] as {("a"):$x} (0; . + $x)"#,
            r#"[{"a":1,"b":2},{"a":4}]"#,
            "5\n",
            0,
        ),
        (
            r#"reduce .[] as {a:$x, ("b"):$y} (0; . + $x + $y)"#,
            r#"[{"a":1,"b":2},{"a":4,"b":5}]"#,
            "12\n",
            0,
        ),
        (
            "reduce .[] as {a:$x} ?// [$x] (0; . + $x)",
            r#"[{"a":1},[2]]"#,
            "3\n",
            0,
        ),
        (
            "reduce .[] as [$x] ?// {a:$x} (0; . + $x)",
            r#"[{"a":1},[2]]"#,
            "3\n",
            0,
        ),
        (
            "reduce .[] as {a:$x} (0; . + $x) | [., $__loc__.line]",
            r#"[{"a":1}]"#,
            "[1,1]\n",
            0,
        ),
        (
            "reduce .[] as {a:$x} ([]; . + [$x])",
            r#"[{"a":[1,{"w":2}],"pad":"zzz"},{"a":null},{"a":{"q":1}}]"#,
            "[[1,{\"w\":2}],null,{\"q\":1}]\n",
            0,
        ),
        (
            "reduce .[] as {a:$x, b:$y} ({}; .[$x] = $y)",
            r#"[{"a":"p","b":[1,2],"c":9},{"a":"q","b":null},{"a":"p","b":3}]"#,
            "{\"p\":3,\"q\":null}\n",
            0,
        ),
        (
            "reduce .[] as {a:$x} (false; $x == $x)",
            r#"[{"a":[nan],"c":1}]"#,
            "true\n",
            0,
        ),
        // A computed key under a named key keeps that member whole, so it is still projected by `a`.
        (
            r#"reduce .[] as {a:{("b"):$x}} (0; . + $x) | [.]"#,
            r#"[{"a":{"b":2,"q":7},"p":1},{"a":{"k":1}}]"#,
            "[2]\n",
            0,
        ),
        // A path read of a destructured name, and a field read of it after a pipe.
        (
            "reduce .[] as {a:$x} (0; path($x)?)",
            r#"[{"a":{"b":2},"p":1}]"#,
            "null\n",
            0,
        ),
        (
            r"reduce .[] as {a:$x} ({}; .z = ($x | .k))",
            r#"[{"a":{"k":"m","q":1},"p":1}]"#,
            "{\"z\":\"m\"}\n",
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

/// A non-object element fails the fold with jq's own message, not the projected object's.
#[test]
fn test_a_non_object_element_names_its_own_type_5544() -> Result<()> {
    for (filter, input, want) in [
        (
            "reduce .[] as {a:$x} (0; . + $x)",
            r#"[{"a":1},5]"#,
            "jq: error (at <stdin>:0): Cannot index number with string \"a\"\n",
        ),
        (
            r#"reduce .[] as {a:{("b"):$x}} (0; . + $x)"#,
            r#"[{"a":1}]"#,
            "jq: error (at <stdin>:0): Cannot index number with string \"b\"\n",
        ),
    ] {
        let (out, err, code) = run_jq_full(&["-c", filter], Some(input))?;
        assert_eq!(
            (out.as_str(), err.as_str(), code),
            ("", want, 5),
            "{filter}"
        );
    }
    Ok(())
}

/// A member a named key reads is walked as the whole conversion walks it: an object whose own
/// frame is malformed (a stray or missing `,` or `:`, a key that is not a string, a missing value)
/// fails the fold, as it does in jq, whether or not the members that survive are the ones named.
#[test]
// `[{:1}]` is a malformed document, not a format string.
#[allow(clippy::literal_string_with_formatting_args)]
fn test_a_malformed_object_still_fails_a_destructuring_fold_5544() -> Result<()> {
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
            jq("reduce .[] as {a: $x} (0; . + $x)", input)?,
            (String::new(), 5),
            "{input}"
        );
        assert_eq!(
            jq("[foreach .[] as {a: $x} (0; . + 1; $x)]", input)?,
            (String::new(), 5),
            "{input}"
        );
    }
    // A member nested past the depth limit fails the fold that names it.
    let deep = format!(r#"[{{"a":{}1{},"b":2}}]"#, "[".repeat(255), "]".repeat(255));
    assert_eq!(
        jq("reduce .[] as {a: $x} (0; $x)", &deep)?,
        (String::new(), 5)
    );
    Ok(())
}

/// A member no key names is not decoded, so an undecodable one no longer fails the fold: the
/// answer `reduce .[] as $u (0; . + $u.b)` gives on the same document since #5519 (a filter
/// validates only what it materializes, #2103/#2658; jq itself refuses the document at parse
/// time). A member a key does name still fails it, and so does a computed key or a `?//`
/// alternation, which keep the element whole. A plain `.[] as {b: $b} | $b` still fails, because
/// the deferred bind (#3856) does not destructure. Each fold entry point the projection is wired
/// through has a row.
#[test]
fn test_an_undecodable_member_no_key_names_is_not_decoded_5544() -> Result<()> {
    let input = r#"[{"a":"\ud800","b":5}]"#;
    for (filter, want, status) in [
        ("reduce .[] as {b: $b} (0; . + $b)", "5\n", 0),
        ("reduce .[] as {$b} (0; . + $b)", "5\n", 0),
        ("[reduce .[] as {b: $b} (0; . + $b)]", "[5]\n", 0),
        ("[foreach .[] as {b: $b} (0; . + $b; .)]", "[5]\n", 0),
        ("first(foreach .[] as {b: $b} (0; . + $b; .))", "5\n", 0),
        (
            "[limit(1; foreach .[] as {b: $b} (0; . + $b; $b))]",
            "[5]\n",
            0,
        ),
        ("reduce .[] as {a: $a} (0; . + ($a | length))", "", 5),
        ("reduce .[] as {a: $a, b: $b} (0; . + $b)", "", 5),
        ("reduce .[] as {(\"b\"): $b} (0; . + $b)", "", 5),
        ("reduce .[] as {b: $b} ?// [$b] (0; . + $b)", "", 5),
    ] {
        assert_eq!(
            jq(filter, input)?,
            (want.to_string(), status),
            "{filter} {input}"
        );
    }
    Ok(())
}
