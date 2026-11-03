//! #5504: a gated `recurse(.[]?; cond)` (a `--jq-extensions` builtin) is `..` with a prune, so a position
//! reader after it, a union of position readers and `path(...)` of it answer as `..` does: a union
//! branch-major, a navigation stage over the real nodes, `path()` stopping at an alias.
//! The `..` rows are live yq v4.53.3 captures; the gated spellings must equal them.

use anyhow::Result;

use crate::run_yq_stdin;

const ALIAS: &str = "a: &x {p: 1}\nb: *x\n";
const NESTED: &str = "a: {b: 1}\n";
const SHARED: &str = "a: &b {x: 1}\nc: *b\n";

fn json(filter: &str, doc: &str) -> Result<String> {
    let (out, code) = run_yq_stdin(filter, doc, &["--jq-extensions", "-o=json", "-I0"])?;
    assert_eq!(code, 0, "{filter}");
    Ok(out.trim_end().to_string())
}

#[test]
fn a_union_after_the_walk_is_branch_major_as_in_yq() -> Result<()> {
    // yq: `[.. | (path, key)]`
    let want = r#"[[],["a"],["a","b"],"a","b"]"#;
    for walk in ["..", "recurse(.[]?; true)", "recurse(.[]?)", "recurse"] {
        assert_eq!(
            json(&format!("[{walk} | (path, key)]"), NESTED)?,
            want,
            "{walk}"
        );
    }
    Ok(())
}

#[test]
fn a_navigation_stage_before_a_position_reader_sees_the_real_nodes() -> Result<()> {
    // yq: `[.. | select(type == "!!map") | .x | select(. != null) | path]` reaches the anchor only.
    let tail = r#"select(type == "!!map") | .x | select(. != null) | path"#;
    for walk in ["..", "recurse(.[]?; true)", "recurse(.[]?)"] {
        assert_eq!(
            json(&format!("[{walk} | {tail}]"), SHARED)?,
            r#"[["a","x"]]"#,
            "{walk}"
        );
    }
    Ok(())
}

#[test]
fn path_of_a_gated_walk_stops_at_an_alias_as_the_walk_does() -> Result<()> {
    for filter in [
        "[path(recurse(.[]?; true))] | length",
        "[recurse(.[]?; true) | path] | length",
        "[path(..)] | length",
    ] {
        assert_eq!(json(filter, ALIAS)?, "4", "{filter}");
    }
    Ok(())
}

#[test]
fn path_of_a_gated_walk_prunes_with_its_cond_as_the_value_walk_does() -> Result<()> {
    let doc = "a: {b: 1, c: [2, 3]}\nd: 4\n";
    for cond in [
        "true",
        "false",
        ". != 1",
        r#"type != "!!int""#,
        r#"type == "!!map" or type == "!!seq""#,
    ] {
        let by_path = json(&format!("[path(recurse(.[]?; {cond}))]"), doc)?;
        let by_value = json(&format!("[recurse(.[]?; {cond}) | path]"), doc)?;
        assert_eq!(by_path, by_value, "{cond}");
    }
    // `cond` is asked of a child and never of the root, so `false` keeps just the root.
    assert_eq!(json("[path(recurse(.[]?; false))]", doc)?, "[[]]");
    // A `cond` that reads the tag is asked at the node. (The collecting value walk stamps every node
    // with the entry position for such a `cond`: #5506.)
    assert_eq!(
        json("[path(recurse(.[]?; tag != \"!!int\"))]", doc)?,
        r#"[[],["a"],["a","c"]]"#
    );
    Ok(())
}

#[test]
fn path_of_a_gated_walk_reads_the_nodes_tag_as_the_value_walk_does() -> Result<()> {
    let doc = "ak: !!binary aGVsbG8=\nb: 1\n";
    let want = r#"["","ak"]"#;
    assert_eq!(
        json(
            r#"[path(recurse(.[]?; type == "!!binary")) | join(".")]"#,
            doc
        )?,
        want
    );
    assert_eq!(
        json(
            r#"[recurse(.[]?; type == "!!binary") | path | join(".")]"#,
            doc
        )?,
        want
    );
    Ok(())
}

#[test]
fn a_consumers_stop_ends_the_gated_walk_before_a_later_siblings_cond() -> Result<()> {
    // `limit(1; ...)` is satisfied by the root, so no child's `cond` is asked: a `cond` that raises
    // never runs.
    assert_eq!(
        json(
            "[limit(1; path(recurse(.[]?; error(\"asked\"))))]",
            "a: 1\nb: 2\n"
        )?,
        "[[]]"
    );
    // After the first child, a later sibling's `cond` is still not asked: it raises on `2`, which
    // `limit(2; ...)` never reaches, as in the value walk.
    let raises_on_two = r#"if . == 2 then error("asked") else true end"#;
    assert_eq!(
        json(
            &format!("[limit(2; path(recurse(.[]?; {raises_on_two})))]"),
            "[1, 2, 3]\n"
        )?,
        "[[],[0]]"
    );
    assert_eq!(
        json(
            &format!("[limit(2; recurse(.[]?; {raises_on_two}) | path)]"),
            "[1, 2, 3]\n"
        )?,
        "[[],[0]]"
    );
    Ok(())
}
