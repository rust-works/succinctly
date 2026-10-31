//! #4095: `recurse(.[]?; cond)` (a `--jq-extensions` builtin in yq mode) takes the cursor walk
//! `..` takes, for a `cond` that yields at most one value. It stops at an alias as `..` does
//! (#4214), a `key`/`path`/`parent`/`line` stage after it reads each node's position, and
//! `first(recurse(.[]?; true))` over an alias ladder no longer expands the ladder.
//!
//! `recurse(f; cond)` is not a real yq builtin, so there is no yq capture to pin: the rows
//! pin the extension to `..` (`cond` = `true`), to `path(recurse(.[]?; cond))`, and to the owned
//! walk it replaces wherever that walk read the right thing.

use anyhow::Result;

use crate::run_yq_stdin_with_stderr;

const ARGS: &[&str] = &["--jq-extensions", "-o=json", "-I=0"];

fn run(filter: &str, doc: &str) -> Result<String> {
    let (stdout, stderr, code) = run_yq_stdin_with_stderr(filter, doc, ARGS)?;
    assert_eq!(code, 0, "`{filter}` -- stderr: {stderr:?}");
    Ok(stdout.trim().to_string())
}

/// Anchors, aliases, a merge key, a tag, a duplicate key and a comment: every shape `..`'s
/// alias rule and position reads have to agree on.
const ALIAS_DOC: &str = "base: &b {x: 1, y: [1, 2]}\nd: {<<: *b, z: !!str 5}\ne: [*b, \"s\", null, 3.5, {k: v}, *b]\ndup: {a: 1, a: 2}\nf: &f [*b, &g {q: 1}, *g]\ng: *f\n# c\nh: !!str 1 # tail\n";

#[test]
fn test_recurse_cond_true_answers_what_recursive_descent_answers_4095() -> Result<()> {
    for prefix in ["", ".e | ", ".f | ", ".g | ", ".dup | "] {
        for tail in [
            "",
            "| key",
            "| path",
            "| parent",
            "| tag",
            "| [., key, path, parent] | tojson",
            "| select(type == \"!!map\") | keys",
            "| line",
            "| column",
            "| anchor",
            "| (path | join(\".\"))",
        ] {
            let want = run(&format!("{prefix}[.. {tail}]"), ALIAS_DOC)?;
            assert!(!want.is_empty(), "`{prefix}[.. {tail}]` produced nothing");
            for cond in ["true", "type != \"!!null\" or true"] {
                let got = run(
                    &format!("{prefix}[recurse(.[]?; {cond}) {tail}]"),
                    ALIAS_DOC,
                )?;
                assert_eq!(got, want, "`{prefix}[recurse(.[]?; {cond}) {tail}]`");
            }
        }
    }
    Ok(())
}

/// The owned walk this replaces expanded an alias, so `recurse(.[]?; true)` visited the anchor's
/// children again at every alias; `recurse(.[]?)` and `..` already stopped there (#3719, #4214).
#[test]
fn test_recurse_cond_stops_at_an_alias_4095() -> Result<()> {
    for (filter, want) in [
        ("[recurse(.[]?; true)] | length", "29"),
        ("[..] | length", "29"),
        ("[recurse(.[]?)] | length", "29"),
        (".e | [recurse(.[]?; true)] | length", "8"),
        (".e | [..] | length", "8"),
    ] {
        assert_eq!(run(filter, ALIAS_DOC)?, want, "`{filter}`");
    }
    Ok(())
}

const TREE: &str = "a: {b: 1, c: [1, 2, {d: x}]}\nl: [10, 20, \"s\", null, 3.5, true]\ndup: {k: 1, k: 2}\nm: !!map {t: !!str 5}\nz: ~\n";

/// A `key`/`path`/`parent`/`line` stage after the walk used to read nothing (`[]`, `null`, `0`
/// for every node); each row was checked against `path(recurse(.[]?; cond))`, the position
/// resolver's own answer.
#[test]
fn test_recurse_cond_keeps_each_nodes_position_4095() -> Result<()> {
    for (filter, want) in [
        (
            r#"[recurse(.[]?; type != "!!seq") | path | join(".")]"#,
            r#"["","a","a.b","dup","dup.k","m","m.t","z"]"#,
        ),
        (
            r#"[recurse(.[]?; type == "!!map" or type == "!!seq") | key]"#,
            r#"["a","c",2,"l","dup","m"]"#,
        ),
        (
            r".a | [recurse(.[]?; length > 0) | [key, (parent | key)] | tojson]",
            r#"["[\"a\"]","[\"b\",\"a\"]","[\"c\",\"a\"]","[0,\"c\"]","[1,\"c\"]","[2,\"c\"]","[\"d\",2]"]"#,
        ),
        (r"[recurse(.[]?; false) | path]", "[[]]"),
        (
            r"[recurse(.[]?; . == 1 | not) | line]",
            "[1,1,1,1,1,1,2,2,2,2,2,2,2,3,3,4,4,5]",
        ),
        (
            r#".a.c | [recurse(.[]?; true) | path | join(".")]"#,
            r#"["a.c","a.c.0","a.c.1","a.c.2","a.c.2.d"]"#,
        ),
    ] {
        assert_eq!(run(filter, TREE)?, want, "`{filter}`");
    }
    Ok(())
}

#[test]
fn test_recurse_cond_positions_match_path_of_the_same_walk_4095() -> Result<()> {
    for cond in [
        "true",
        "false",
        ". != null",
        "type != \"!!null\"",
        "type == \"!!map\" or type == \"!!seq\"",
        "type != \"!!seq\"",
        "length > 0",
        "(. | type) == \"!!map\"",
        ". == 1 | not",
    ] {
        for prefix in ["", ".a | ", ".l | ", ".dup | ", ".a.c | ", ".a.c[2] | "] {
            let want = run(&format!("[path({prefix}recurse(.[]?; {cond}))]"), TREE)?;
            let got = run(&format!("{prefix}[recurse(.[]?; {cond}) | path]"), TREE)?;
            assert_eq!(got, want, "`{prefix}` / `{cond}`");
        }
    }
    Ok(())
}

/// A computed node has no live cursor, so the walk runs over an owned value that carries its own
/// position. Constructed values stand at the root of their own path, which makes
/// `path(recurse(.[]?; cond))` over the same value the position reference; `cond` = `true` is also
/// pinned to `..`, with `key` and `parent`.
#[test]
fn test_recurse_cond_keeps_the_positions_of_computed_nodes_4095() -> Result<()> {
    for prefix in ["{a: [1, {b: 2}]} | ", "[[1, 2], {c: 3}] | "] {
        for cond in ["true", "type != \"!!seq\"", ". != 1", "false"] {
            let want = run(&format!("{prefix}[path(recurse(.[]?; {cond}))]"), "a: 5\n")?;
            let got = run(&format!("{prefix}[recurse(.[]?; {cond}) | path]"), "a: 5\n")?;
            assert_eq!(got, want, "`{prefix}` / `{cond}`");
        }
        for tail in ["| [key, path, parent] | tojson", "| key", "| path"] {
            let want = run(&format!("{prefix}[.. {tail}]"), "a: 5\n")?;
            let got = run(&format!("{prefix}[recurse(.[]?; true) {tail}]"), "a: 5\n")?;
            assert_eq!(got, want, "`{prefix}` / `{tail}`");
        }
    }
    Ok(())
}

/// A `cond` that raises while the walk asks it at a position raises out of the position stage
/// too, for a live document and for a computed value.
#[test]
fn test_recurse_cond_that_raises_reaches_a_position_stage_4095() -> Result<()> {
    for (doc, filter) in [
        (TREE, r#"[recurse(.[]?; error("boom")) | path]"#),
        (
            "a: 5\n",
            r#"{a: [1]} | [recurse(.[]?; error("boom")) | path]"#,
        ),
        (
            "a: 5\n",
            r#"{a: [1]} | [recurse(.[]?; error("boom")) | key]"#,
        ),
        // A node with no cursor (a slice of the document) is asked through the owned evaluator.
        (TREE, r#".l[1:5] | [recurse(.[]?; error("boom")) | path]"#),
    ] {
        let (stdout, stderr, code) = run_yq_stdin_with_stderr(filter, doc, ARGS)?;
        assert_ne!(code, 0, "`{filter}` -- stdout: {stdout:?}");
        assert!(stderr.contains("boom"), "`{filter}` -- stderr: {stderr:?}");
    }
    Ok(())
}

/// A `cond` that compares `.` with a literal is computed without the general evaluator
/// (`direct_cond_verdict`): a container from its kind, a scalar with the general route's own
/// comparison. Each row is pinned to the same `cond` spelled so the general route takes it
/// (`. | cond`), on documents holding every kind of node, an alias and a duplicate key.
#[test]
fn test_recurse_cond_comparison_agrees_with_the_general_route_4095() -> Result<()> {
    for doc in [TREE, ALIAS_DOC] {
        for cond in [
            ". != null",
            ". == null",
            ". != 1",
            ". == \"s\"",
            "null != .",
            ". == 1 | not",
            ". != null and . != 2",
            ". == null or . == 3.5",
            "(. != null)",
            "true",
            "false",
            ".",
        ] {
            for tail in ["| tojson", "| path", "| key"] {
                let want = run(&format!("[recurse(.[]?; . | ({cond})) {tail}]"), doc)?;
                let got = run(&format!("[recurse(.[]?; {cond}) {tail}]"), doc)?;
                assert_eq!(got, want, "`{cond}` {tail}");
            }
        }
    }
    Ok(())
}

/// The direct verdict for a comparison with `null`, a boolean or `type` has to agree with the
/// general route on every spelling YAML gives those: the nulls (`~`, `null`, `Null`, an empty
/// value), their quoted and tagged look-alikes (yq compares a scalar with a literal by its text,
/// so `e: "null"` is `== null`), the booleans (`true`, `True`, a quoted `"true"`) and `yes`, which is a string, a
/// container carrying a `!!null`/`!!bool` tag, and a node with a custom tag (`type` answers the
/// tag).
#[test]
fn test_recurse_cond_null_and_bool_verdicts_agree_with_the_general_route_4095() -> Result<()> {
    let doc = "a: ~\nb: null\nc: Null\nd:\ne: \"null\"\nf: 'null'\ng: !!str null\nh: true\ni: True\nj: \"true\"\nk: yes\nl: false\nm: !!bool true\nn: 0\no: \"\"\np: []\nq: {}\nr: !!null [1]\ns: !!bool [1]\nt: [~, true, false, [null]]\nu: !custom {x: 5}\nv: !mytag 5\nw: !!map {x: 0x1F}\nx: 2.50\ny: 0x1F\nz: 0o17\naa: 007\nab: +5\nac: 1_000\nad: 1e3\n";
    for cond in [
        "type == \"!!map\"",
        "type != \"!!str\"",
        "\"!!int\" == type",
        "type == \"!!map\" or type == \"!!seq\"",
        "type == \"!!str\" and . != \"s\"",
        "type == \"!custom\" or type == \"!mytag\"",
        "type == \"!!float\" | not",
        ". == null",
        ". != null",
        "null == .",
        "null != .",
        ". == true",
        ". != true",
        ". == false",
        ". != false",
        "true == .",
        ". == null or . == false",
        ". != null and . != false",
        ". == true | not",
        // A spelled integer (#2802): yq compares a scalar with a literal by its text, so `0x1F ==
        // 31` is false in yq v4.53.3 and only ordering compares the value. Whatever the general route
        // answers, the direct verdict has to answer too. (Ordering is pinned below, over a document
        // of numbers: yq raises on an ordering comparison of a string or a container, #5631.)
        ". == 31",
        ". != 15",
        ". == 7",
        ". == 5",
        ". == 1000",
        ". == 1e3",
    ] {
        for tail in ["| tojson", "| path"] {
            let want = run(&format!("[recurse(.[]?; . | ({cond})) {tail}]"), doc)?;
            let got = run(&format!("[recurse(.[]?; {cond}) {tail}]"), doc)?;
            assert_eq!(got, want, "`{cond}` {tail}");
        }
    }
    Ok(())
}

/// A comparison of a container with a literal is decided by the node being a container, so it does
/// not decode the container: an undecodable string inside a subtree the walk stops short of is not
/// raised (the general route copies the container to compare it, and raises). jq 1.7.1 rejects such
/// a document at parse time, so there is no reference answer (ADR-0018: performance decides).
#[test]
fn test_recurse_cond_comparing_a_container_does_not_decode_it_4095() -> Result<()> {
    let doc = "a:\n  b: \"bad\\q\"\nz: 5\n";
    assert_eq!(
        run("[limit(2; recurse(.[]?; . != null)) | path]", doc)?,
        r#"[[],["a"]]"#
    );
    let (_, stderr, code) = run_yq_stdin_with_stderr(
        "[limit(2; recurse(.[]?; . | (. != null))) | path]",
        doc,
        ARGS,
    )?;
    assert_ne!(code, 0);
    assert!(stderr.contains("invalid escape"), "{stderr:?}");
    Ok(())
}

/// Nodes that are not live (a slice of an array, a constructed value) have no cursor to ask, so
/// the walk asks `cond` of their owned values. Each row is pinned to the `cond` spelled so the
/// general route takes it (`. | cond`), for the values and for the positions.
#[test]
fn test_recurse_cond_over_sliced_and_computed_nodes_agrees_with_the_general_route_4095(
) -> Result<()> {
    for prefix in [
        ".l[1:5] | ",
        ".a.c[0:3] | ",
        "{a: [1, {b: 2}], s: \"x\", n: null} | ",
        "[[1, 2], {c: 3}, \"s\", null, 3.5, true] | ",
    ] {
        for cond in [
            ". != null",
            "null",
            "(. != 1)",
            "type == \"!!int\" | not",
            "type != \"!!seq\"",
            "length > 0",
            ". == 1 | not",
        ] {
            for tail in ["| tojson", "| path"] {
                let want = run(
                    &format!("{prefix}[recurse(.[]?; . | ({cond})) {tail}]"),
                    TREE,
                )?;
                let got = run(&format!("{prefix}[recurse(.[]?; {cond}) {tail}]"), TREE)?;
                assert_eq!(got, want, "`{prefix}` / `{cond}` {tail}");
            }
        }
    }
    Ok(())
}

/// A computed bracket on the walk steps it as a target (the collecting form of the position
/// walk), and answers what the same bracket on `..` answers.
#[test]
fn test_recurse_cond_as_a_bracket_target_answers_what_recursive_descent_answers_4095() -> Result<()>
{
    for key in ["(0, 1)", "(1, 0)", "-1", "(2, -1)"] {
        let want = run(&format!("[(..)[{key}] | path]"), TREE)?;
        let got = run(&format!("[(recurse(.[]?; true))[{key}] | path]"), TREE)?;
        assert_eq!(got, want, "{key}");
    }
    Ok(())
}

/// `limit` stops the position walk where jq stops: a `cond` is not asked of a node the consumer
/// never reached, so an effect in it (`debug`) does not run for those nodes. (yq's own `first(exp)`
/// is not jq's `first`: it returns the first child of the context for which `exp` is truthy, so it
/// says nothing about where a walk stops.)
#[test]
fn test_recurse_cond_stops_asking_cond_when_the_consumer_stops_4095() -> Result<()> {
    // (filter, how many times `cond` is asked): the walk delivers the root without asking, then
    // asks once per child it reaches, in order, and stops where the consumer does.
    for (filter, asked) in [
        ("[limit(1; recurse(.[]?; debug | true) | path)]", 0),
        ("[limit(1; recurse(.[]?; debug | true))] | length", 0),
        ("[limit(3; recurse(.[]?; debug | true) | path)]", 2),
        // The root has no key, so the fourth key comes from the fourth kept node.
        ("[limit(4; recurse(.[]?; debug | true) | key)]", 4),
    ] {
        let (_, stderr, code) = run_yq_stdin_with_stderr(filter, TREE, ARGS)?;
        assert_eq!(code, 0, "`{filter}` -- stderr: {stderr:?}");
        assert_eq!(
            stderr.matches("DEBUG").count(),
            asked,
            "`{filter}` -- {stderr:?}"
        );
    }
    Ok(())
}

/// A `?` around the walk still suppresses an error `cond` raises, over a computed value as over a
/// document: the nodes before the error are kept and the rest of the walk is dropped. Over a
/// document the position stage still answers for the nodes it reached (`[[],["a"]]`; the owned
/// walk gave `[[],[]]`); a computed value takes the eager route and gives a position stage none.
#[test]
fn test_recurse_cond_optional_suppresses_a_raising_cond_4095() -> Result<()> {
    let cond = r#"if type == "!!int" then error("boom") else true end"#;
    for (filter, want) in [
        (format!("[recurse(.[]?; {cond})? | path]"), r#"[[],["a"]]"#),
        (
            format!("{{a: [1, {{b: 2}}]}} | [recurse(.[]?; {cond})? | path]"),
            "[[],[]]",
        ),
        (
            format!("{{a: [1, {{b: 2}}]}} | [recurse(.[]?; {cond})? | key]"),
            "[]",
        ),
    ] {
        assert_eq!(run(&filter, TREE)?, want, "`{filter}`");
    }
    Ok(())
}

/// The default (YAML) output of a bare walk is rendered from each kept node's own cursor, as
/// `..`'s is: an alias, a flow collection and a comment keep their spelling, and the walk does not
/// copy every node's subtree again at output time (twice `..`'s memory before the M2 gate took it).
#[test]
fn test_recurse_cond_yaml_output_is_rendered_like_recursive_descent_4095() -> Result<()> {
    let doc = "base: &b {x: 1}\ne: [*b, 2] # tail\nf: {k: [1, 2]}\n";
    let args = ["--jq-extensions"];
    let (want, e1, c1) = run_yq_stdin_with_stderr("..", doc, &args)?;
    assert_eq!(c1, 0, "stderr: {e1:?}");
    for filter in ["recurse(.[]?; true)", "recurse(.[]?; . != null)"] {
        let (got, stderr, code) = run_yq_stdin_with_stderr(filter, doc, &args)?;
        assert_eq!(code, 0, "`{filter}` -- stderr: {stderr:?}");
        assert_eq!(got, want, "`{filter}`");
    }
    Ok(())
}

/// A value-only consumer of the walk is unchanged: the cursor walk and the owned walk it
/// replaced list the same nodes in the same order, and a `cond` that raises does so after the
/// earlier siblings' subtrees were delivered.
#[test]
fn test_recurse_cond_values_and_errors_are_unchanged_4095() -> Result<()> {
    for (filter, want) in [
        (
            r#"[recurse(.[]?; type != "!!null") | select(type == "!!map") | keys | join(",")]"#,
            r#"["a,l,dup,m,z","b,c","d","k,k","t"]"#,
        ),
        (
            r#"try ([recurse(.[]?; if type == "!!int" then error("boom") else true end) | key]) catch ."#,
            r#""boom""#,
        ),
        (r"[recurse(.[]?; false)] | length", "1"),
    ] {
        assert_eq!(run(filter, TREE)?, want, "`{filter}`");
    }
    Ok(())
}

/// The issue's reproducer: a 26-level alias ladder (2^26 leaves when expanded). The owned walk
/// took 33 s and 2 GB to deliver its first node; the cursor walk delivers it at once, as `..`
/// does.
#[test]
fn test_recurse_cond_over_alias_fanout_completes_4095() -> Result<()> {
    let mut doc = String::from("a0: &a0 [x, x]\n");
    for i in 1..=26 {
        doc.push_str(&format!("a{i}: &a{i} [*a{}, *a{}]\n", i - 1, i - 1));
    }
    doc.push_str("root: *a26\n");

    for (filter, want) in [
        (".root | first(recurse(.[]?; true)) | length", "2"),
        (".root | first(..) | length", "2"),
        (".a26 | [recurse(.[]?; true)] | length", "3"),
        (".a26 | [..] | length", "3"),
        (
            r#".a26 | [limit(5; recurse(.[]?; true) | path | join("."))]"#,
            r#"["a26","a26.0","a26.1"]"#,
        ),
        (".root | [limit(3; recurse(.[]?; true))] | length", "1"),
    ] {
        assert_eq!(run(filter, &doc)?, want, "`{filter}`");
    }
    Ok(())
}

/// Ordering compares a spelled integer by value (`0x1F > 6`). yq refuses an ordering comparison of a
/// container or a string (#5631), so the root mapping is settled by `type` before `>` sees it.
#[test]
fn test_recurse_cond_ordering_agrees_with_the_general_route_4095() -> Result<()> {
    let doc = "a: 0x1F\nb: 007\nc: 2.50\nd: 5\ne: 1e3\nf: 0o17\n";
    for cond in ["type == \"!!map\" or . > 6", "type == \"!!map\" or . <= 7"] {
        for tail in ["| tojson", "| path"] {
            let want = run(&format!("[recurse(.[]?; . | ({cond})) {tail}]"), doc)?;
            let got = run(&format!("[recurse(.[]?; {cond}) {tail}]"), doc)?;
            assert_eq!(got, want, "`{cond}` {tail}");
        }
    }
    Ok(())
}
