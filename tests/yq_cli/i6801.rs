//! #6801: yq builds an empty `{}` **once** however many nodes reach it, where `[]` and every other
//! literal are built once per node: `.x[] | {}` is one `{}`, `(.x[], .x[]) | {}` too, and
//! `.x[] | select(. > 5) | {}` is one from an empty context. Every row is `yq -o=json -I0 FILTER`
//! on [`DOC`], captured from yq v4.53.3. `map(f)` is `[.[] | f]`, so `.x | map({})` is `[{}]` too.

use anyhow::Result;

use crate::{run_jq_stdin, run_yq_stdin};

const DOC: &str = "x: [1, 2, 3]\ny: {z: 1}\n";

/// (filter, stdout), each the same as yq v4.53.3's with exit code 0.
const ROWS: &[(&str, &str)] = &[
    (".x[] | {}", "{}\n"),
    ("(.x[] | select(. > 0)) | {}", "{}\n"),
    (".x[] | {} | length", "0\n"),
    ("(.x[], .x[]) | {}", "{}\n"),
    (".x[] | select(. > 5) | {}", "{}\n"),
    (".zz | {}", "{}\n"),
    (".zz[] | {}", "{}\n"),
    (".x[] | ({}, {})", "{}\n{}\n"),
    (".x[] | {\"a\": 1}", "{\"a\":1}\n{\"a\":1}\n{\"a\":1}\n"),
    (".x[] | . as $v | {}", "{}\n{}\n{}\n"),
    ("[.x[] | {}]", "[{}]\n"),
    (".x[] | {} | .a = 1", "{\"a\":1}\n"),
    (".x[] | ({} | .a = 1)", "{\"a\":1}\n"),
    (
        ".x[] | {} + {\"a\": 1}",
        "{\"a\":1}\n{\"a\":1}\n{\"a\":1}\n",
    ),
    (".x[] | {} | to_entries", "[]\n"),
    (".x[] | []", "[]\n[]\n[]\n"),
    (".x | {}", "{}\n"),
    (".x[] | (.[]? | {})", "{}\n"),
    (".x[] | {} | {}", "{}\n"),
    ("({}, {}) | length", "0\n0\n"),
    ("{}, .x[]", "{}\n1\n2\n3\n"),
    ("(.x[] | {}) as $e | $e", "{}\n"),
    (". as $r | .x[] | {}", "{}\n"),
    ("del(.x[] | {})", "{\"x\":[1,2,3],\"y\":{\"z\":1}}\n"),
    (".x[] |= {}", "{\"x\":[{},{},{}],\"y\":{\"z\":1}}\n"),
    ("[.x[] | {}] | length", "1\n"),
    (".x[] | {} | keys", "[]\n"),
    (".x[] | (.y | {})", "{}\n"),
    (".x[] | (.zz // {})", "{}\n{}\n{}\n"),
    (".x[] | (. as $v | {})", "{}\n{}\n{}\n"),
    (".x[] | {} | .a", "null\n"),
    (".x[] | select(true) | {}", "{}\n"),
    (".y | {}", "{}\n"),
];

#[test]
fn every_row_matches_yq_6801() -> Result<()> {
    for (filter, expected) in ROWS {
        let (stdout, code) = run_yq_stdin(filter, DOC, &["-o", "json", "-I0"])?;
        assert_eq!(code, 0, "{filter}");
        assert_eq!(&stdout, expected, "{filter}");
    }
    Ok(())
}

/// An error upstream of the `{}` still ends the run: the stage before it is evaluated, not skipped.
#[test]
fn an_error_before_the_empty_mapping_still_raises_6801() -> Result<()> {
    let (stdout, code) = run_yq_stdin(".x[] | error(\"e\") | {}", DOC, &["-o", "json", "-I0"])?;
    assert_ne!(code, 0);
    assert_eq!(stdout, "");
    Ok(())
}

/// The default (YAML) output prints the one mapping as yq does, and a document with a comment keeps it.
#[test]
fn the_one_mapping_prints_as_yaml_6801() -> Result<()> {
    for (filter, expected) in [(".x[] | {}", "{}\n"), (".x[] | ({} | .a = 1)", "a: 1\n")] {
        let (stdout, code) = run_yq_stdin(filter, "x: [1, 2] # c\n", &[])?;
        assert_eq!(code, 0, "{filter}");
        assert_eq!(stdout, expected, "{filter}");
    }
    Ok(())
}

/// `map(f)` splats its node and runs `f` over the whole member list, so an empty `{}` in `f` is
/// built once: `.x | map({})` is `[{}]`, where a `[]`, a constant or an `as` body is still one per
/// member. Each row is `yq -o=json -I0 FILTER` on [`DOC`], from yq v4.53.3.
const MAP_ROWS: &[(&str, &str)] = &[
    (".x | map({})", "[{}]\n"),
    (".y | map({})", "[{}]\n"),
    ("map({})", "[{}]\n"),
    (".x | map(select(. > 5) | {})", "[{}]\n"),
    (".x | map(select(. > 1) | {})", "[{}]\n"),
    (".x | map({} | length)", "[0]\n"),
    (".x | map({}, {})", "[{},{}]\n"),
    (".x | map({\"a\": 1})", "[{\"a\":1},{\"a\":1},{\"a\":1}]\n"),
    (".x | map(. | {})", "[{}]\n"),
    (".x | map(. as $v | {})", "[{},{},{}]\n"),
    (".x | map([{}])", "[[{}],[{}],[{}]]\n"),
    (".x | map({} | .a = 1)", "[{\"a\":1}]\n"),
    (".x | (map({}))", "[{}]\n"),
    (".x | map({}) | length", "1\n"),
    (".zz | map({})", "[]\n"),
    (".x | map(({}))", "[{}]\n"),
    (".x | map(. | ({} | .a = 1))", "[{\"a\":1}]\n"),
    (".x | map(.. | {})", "[{}]\n"),
    ("[.x | map({})]", "[[{}]]\n"),
    (".x | map({} | to_entries)", "[[]]\n"),
];

#[test]
fn map_builds_the_empty_mapping_once_6801() -> Result<()> {
    for (filter, expected) in MAP_ROWS {
        let (stdout, code) = run_yq_stdin(filter, DOC, &["-o", "json", "-I0"])?;
        assert_eq!(code, 0, "{filter}");
        assert_eq!(&stdout, expected, "{filter}");
    }
    Ok(())
}

/// The rewrite keeps `map`'s answer for an input with no members: an empty sequence and `null` are
/// `[]`, an empty mapping and a scalar are themselves, and `f` never runs.
#[test]
fn map_of_an_empty_mapping_over_a_memberless_node_6801() -> Result<()> {
    let doc = "e: []\nem: {}\ns: hi\nn: null\n";
    for (filter, expected) in [
        (".e | map({})", "[]\n"),
        (".em | map({})", "{}\n"),
        (".s | map({})", "\"hi\"\n"),
        (".n | map({})", "[]\n"),
    ] {
        let (stdout, code) = run_yq_stdin(filter, doc, &["-o", "json", "-I0"])?;
        assert_eq!(code, 0, "{filter}");
        assert_eq!(stdout, expected, "{filter}");
    }
    Ok(())
}

/// `map({})` is the same as the spelling it stands for, in a program that holds both.
#[test]
fn map_and_its_splat_spelling_agree_6801() -> Result<()> {
    let (map, _) = run_yq_stdin(".x | map({})", DOC, &["-o", "json", "-I0"])?;
    let (spelled, _) = run_yq_stdin(".x | [.[] | {}]", DOC, &["-o", "json", "-I0"])?;
    assert_eq!(map, spelled);
    Ok(())
}

/// The rule is yq's: `succinctly jq` builds one `{}` per member, as jq does, for the same filter
/// (the mode decides, never the input format).
#[test]
fn jq_mode_map_of_the_empty_object_stays_per_member_6801() -> Result<()> {
    let (stdout, code) = run_jq_stdin(".x | map({})", "{\"x\":[1,2,3]}", &["-c"])?;
    assert_eq!(code, 0);
    assert_eq!(stdout, "[{},{},{}]\n");
    let (stdout, code) = run_jq_stdin(".x | [.[] | {}]", "{\"x\":[1,2,3]}", &["-c"])?;
    assert_eq!(code, 0);
    assert_eq!(stdout, "[{},{},{}]\n");
    Ok(())
}

/// A body that builds the mapping some other way stays per member, as in yq: a constant that
/// holds it, an arithmetic stage, an `as` binding and the `//` alternative.
#[test]
fn map_bodies_that_are_not_a_bare_empty_mapping_stay_per_member_6801() -> Result<()> {
    for (filter, expected) in [
        (
            ".x | map({} + {\"a\": 1})",
            "[{\"a\":1},{\"a\":1},{\"a\":1}]\n",
        ),
        (".x | map({} as $z | $z)", "[{},{},{}]\n"),
        (
            ".x | map({\"k\": {}})",
            "[{\"k\":{}},{\"k\":{}},{\"k\":{}}]\n",
        ),
        (".x | map(select(. > 1) // {})", "[{},2,3]\n"),
        (".x | map({} | select(true))", "[{}]\n"),
        (".x | map({} | . as $z | $z)", "[{}]\n"),
    ] {
        let (stdout, code) = run_yq_stdin(filter, DOC, &["-o", "json", "-I0"])?;
        assert_eq!(code, 0, "{filter}");
        assert_eq!(stdout, expected, "{filter}");
    }
    Ok(())
}
