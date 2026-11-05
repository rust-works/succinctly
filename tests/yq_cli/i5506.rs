//! #5506: `recurse(.[]?; cond)` (a `--jq-extensions` builtin in yq mode) took the cursor walk only for the
//! `cond`s a hand-written allow-list admitted, and `tag`, `kind`, `style`, `anchor`, `tojson`, `test`,
//! `any`, `all` and `isempty` were not on it. Such a `cond` kept the owned walk, which expands every
//! alias (a ladder of aliases doubles per level) and answers more nodes than `..` does.
//!
//! `recurse(f; cond)` is not a real yq builtin, so there is no yq capture to pin: the rows pin the
//! extension to `..` for a `cond` that is true of every node.

use anyhow::Result;

use crate::run_yq_stdin_with_stderr;

const ARGS: &[&str] = &["--jq-extensions", "-o=json", "-I=0"];

fn run(filter: &str, doc: &str) -> Result<String> {
    let (stdout, stderr, code) = run_yq_stdin_with_stderr(filter, doc, ARGS)?;
    assert_eq!(code, 0, "`{filter}` -- stderr: {stderr:?}");
    Ok(stdout.trim().to_string())
}

/// `cond`s that hold of every node (an alias node has the empty tag, so `tag` is compared to a tag no
/// node has), one for each builtin the allow-list gained.
const CONDS: &[&str] = &[
    r#"tag != "~""#,
    r#"kind != "x""#,
    r#"style != "zzz""#,
    r#"anchor != "~""#,
    "(tojson | length) > 0",
    r#"((strings | test(".")) // true)"#,
    r#"((strings | test("(?i)A")) // true)"#,
    "(isempty(.[]?) or true)",
    "(isempty(.[]?) | not or true)",
    "((arrays | any) // true) or true",
    "((arrays | all) // true) or true",
];

/// Anchors, aliases, a merge key, a tag and a comment: the shapes where the owned walk's alias
/// expansion shows (`..` stops at an alias).
const ALIAS_DOC: &str =
    "a: &b {x: 1, y: [1, 2]}\nc: *b\nd: {<<: *b, z: !!str 5}\ne: [*b, s, null]\n# c\n";

#[test]
fn test_recurse_cond_of_the_gained_builtins_answers_what_recursive_descent_answers_5506(
) -> Result<()> {
    for prefix in ["", ".c | ", ".e | ", ".d | "] {
        for tail in ["| length", "| tojson", "| map(type) | join(\",\")"] {
            let want = run(&format!("{prefix}[..] {tail}"), ALIAS_DOC)?;
            for cond in CONDS {
                let got = run(
                    &format!("{prefix}[recurse(.[]?; {cond})] {tail}"),
                    ALIAS_DOC,
                )?;
                assert_eq!(got, want, "{prefix}`{cond}` {tail}");
            }
        }
    }
    Ok(())
}

/// The issue's own numbers: the `tag` cond answered 11 (aliases expanded) where the `type` cond and
/// `..` answer 7.
#[test]
fn test_recurse_cond_tag_stops_at_an_alias_like_type_5506() -> Result<()> {
    let doc = "a: &b {x: 1, y: [1, 2]}\nc: *b\n";
    for cond in [
        r#"type != "!!null""#,
        r#"tag != "!!null""#,
        r#"kind != "x""#,
    ] {
        assert_eq!(
            run(&format!("[recurse(.[]?; {cond})] | length"), doc)?,
            "7",
            "`{cond}`"
        );
    }
    assert_eq!(run("[..] | length", doc)?, "7");
    Ok(())
}

/// A cond that prunes keeps pruning: `isempty(.[]?) | not` holds of the containers that have children.
#[test]
fn test_recurse_cond_isempty_prunes_leaves_and_empty_containers_5506() -> Result<()> {
    let doc = "a: {x: 1, y: [1, 2], z: []}\nb: {}\n";
    assert_eq!(
        run(
            r#"[recurse(.[]?; isempty(.[]?) | not) | tojson] | join(" ")"#,
            doc
        )?,
        run(
            r#"[.. | select((type == "!!map" or type == "!!seq") and length > 0) | tojson] | join(" ")"#,
            doc
        )?
    );
    Ok(())
}

/// A 22-level alias ladder: the first node of each of these used to take about ten seconds and
/// 800 MB in a debug build (the owned walk doubles per level); the cursor walk delivers it at once.
#[test]
fn test_recurse_cond_of_the_gained_builtins_over_an_alias_ladder_completes_5506() -> Result<()> {
    let mut doc = String::from("a0: &a0 [x, x]\n");
    for i in 1..=22 {
        doc.push_str(&format!("a{i}: &a{i} [*a{}, *a{}]\n", i - 1, i - 1));
    }
    doc.push_str("root: *a22\n");
    for cond in CONDS {
        assert_eq!(
            run(
                &format!(".root | first(recurse(.[]?; {cond})) | length"),
                &doc
            )?,
            "2",
            "`{cond}`"
        );
    }
    Ok(())
}

/// The cursor walk answers what the owned walk answers for a `cond` that fans out, is empty or
/// raises. A `def` parameter keeps the owned walk, so the same `cond` through one is the control.
#[test]
fn test_recurse_cond_of_the_gained_builtins_agrees_with_the_owned_walk_5506() -> Result<()> {
    let doc = "a: {x: 1, y: [1, 2], z: []}\nb: abc\nc: [ab, {k: a}]\nd: ~\n";
    for cond in [
        "isempty(.[]?, error)",
        "isempty(empty)",
        "isempty(.[]?) | not",
        r#"test("a")"#,
        r#"(strings | test("a"))"#,
        r#"(strings | test("(?i)A"))"#,
        "any",
        "all",
        "(arrays | any)",
        "(arrays | all)",
        r#"tag == "!!str""#,
        r#"kind == "scalar""#,
        r#"anchor == """#,
        "(tojson | length) < 5",
        r#"(tojson | test("1"))"#,
    ] {
        let direct = format!("[recurse(.[]?; {cond}) | tojson]");
        let owned = format!("def f(c): [recurse(.[]?; c) | tojson]; f({cond})");
        let (out, _, code) = run_yq_stdin_with_stderr(&direct, doc, ARGS)?;
        let (want, _, want_code) = run_yq_stdin_with_stderr(&owned, doc, ARGS)?;
        assert_eq!((out.trim(), code), (want.trim(), want_code), "`{cond}`");
    }
    Ok(())
}
