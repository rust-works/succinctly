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
    ] {
        let (stdout, stderr, code) = run_yq_stdin_with_stderr(filter, doc, ARGS)?;
        assert_ne!(code, 0, "`{filter}` -- stdout: {stdout:?}");
        assert!(stderr.contains("boom"), "`{filter}` -- stderr: {stderr:?}");
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
