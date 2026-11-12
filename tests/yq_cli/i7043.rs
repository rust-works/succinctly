//! #7043: a mapping key that merely begins like a null, bool or number (`n1`, `no`, `t1`, `1a`,
//! `.a`) is a plain string, and `key` after a `select` has to hand on the key's own node, as it does
//! for `a1`. It handed on an owned string, so `head_comment`, `line` and the rest read nothing.
//! Every expected value was captured from yq v4.53.3 (`yq -o=json -I0`).

use anyhow::Result;

use crate::run_yq_stdin;

const DOC: &str = "x:\n  # c\n  n1: 1 # l\n  # d\n  no: 2\n  # e\n  t1: 3\n  # f\n  a1: 4\n  # g\n  1a: 5\n  # h\n  nul: 6\n";

fn run(filter: &str, doc: &str) -> Result<String> {
    let (out, code) = run_yq_stdin(filter, doc, &["-o=json", "-I0"])?;
    assert_eq!(code, 0, "{filter}");
    Ok(out.trim_end().to_string())
}

#[test]
fn key_after_select_keeps_the_head_comment_of_a_key_spelled_like_a_scalar_7043() -> Result<()> {
    for (filter, expected) in [
        (
            "[.x[] | select(true) | key | head_comment]",
            r#"["c","d","e","f","g","h"]"#,
        ),
        ("[.x[] | select(true) | key | line]", "[3,5,7,9,11,13]"),
        (
            "[.x[] | select(true) | key | [line_comment]]",
            r#"[[""],[""],[""],[""],[""],[""]]"#,
        ),
        (
            r#"[.. | select(parent | tag == "!!map") | key | head_comment]"#,
            r#"["","c","d","e","f","g","h"]"#,
        ),
    ] {
        assert_eq!(run(filter, DOC)?, expected, "{filter}");
    }
    Ok(())
}

/// The issue's table: the spelling of the key decides nothing.
#[test]
fn the_first_byte_of_a_key_does_not_change_its_comment_7043() -> Result<()> {
    for key in [
        "n1", "n", "nx", "no", "N1", "null", "a1", "z", "t1", "f1", "y1", "o1", "1a", ".a", "-a",
    ] {
        let doc = format!("x:\n  # c\n  {key}: 1 # l\n");
        assert_eq!(
            run(
                r#"[.. | select(parent | tag == "!!map") | key | [head_comment, line_comment]]"#,
                &doc
            )?,
            r#"[["",""],["c",""]]"#,
            "key {key}"
        );
    }
    Ok(())
}

/// The guard `key` keeps: `01` is the integer 1 to `select(key == 1)`, as in yq, and a key whose
/// text the ladder retypes does not become a node a later consumer retypes again (#2785).
#[test]
fn a_retyped_key_still_matches_by_value_7043() -> Result<()> {
    let doc = "1: a\n# c\n01: b\n";
    assert_eq!(run("[select(key == 1) | key | head_comment]", doc)?, "[]");
    assert_eq!(run("[.[] | key | head_comment]", doc)?, r#"["","c"]"#);
    Ok(())
}

/// The owned-identity pipe (`| tostring | key`, `. as $v | key`) decides the same thing as
/// `select(true) | key` in a second place, and a quoted key is a `!!str` node whatever it spells.
#[test]
fn every_route_to_key_keeps_the_node_7043() -> Result<()> {
    let doc = "x:\n  # c\n  n1: 1\n  # d\n  \"no\": 2\n  # e\n  \"1\": 3\n  # f\n  a1: 4\n";
    for (filter, expected) in [
        (
            "[.x[] | tostring | key | head_comment]",
            r#"["c","d","e","f"]"#,
        ),
        (
            "[.x[] | select(true) | key | head_comment]",
            r#"["c","d","e","f"]"#,
        ),
        ("[.x[] | tostring | key | line]", "[3,5,7,9]"),
        (
            r#"[.x[] | select(key == "n1") | key | head_comment]"#,
            r#"["c"]"#,
        ),
        (
            "[.x[] | select(true) | key | style]",
            r#"["","double","double",""]"#,
        ),
        (".x.n1 | . as $v | key | head_comment", r#""c""#),
        (".x.n1 | tostring | key | head_comment", r#""c""#),
    ] {
        assert_eq!(run(filter, doc)?, expected, "{filter}");
    }
    Ok(())
}
