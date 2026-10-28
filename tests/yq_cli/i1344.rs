//! #1344: an `OwnedValue` object holds a repeated mapping key's earlier entries beside the collapsed
//! view (ADR-0027), so the routes that materialize a document -- `[...]`, a binding, `..`, a write
//! through a path, `with_entries`, `from_entries`, `map_values`, `+`, `omit` -- print what yq v4.53.3
//! prints: every entry, a write reaching only the last one (`+` reaching the first).
//!
//! Every expected value below was captured from yq v4.53.3 with `-o=json -I=0`. The row is run as a
//! plain evaluation and, where yq's `ea` agrees with it, under `--eval-all` too.

use anyhow::Result;

use crate::run_yq_stdin;

const D: &str = "a: 1\na: 2\nb: 3\n";
const INTERLEAVED: &str = "b: 1\na: 2\nb: 3\n";
const NESTED: &str = "x:\n  a: 1\n  a: 2\n  b: 3\nz: [1, 2]\n";
const SEQ: &str = "- a: 1\n  a: 2\n- b: 5\n- a: 7\n  a: 8\n  a: 9\n";

/// (document, filter, expected, also under `--eval-all`)
const ROWS: &[(&str, &str, &str, bool)] = &[
    (D, r"[.]", r#"[{"a":1,"a":2,"b":3}]"#, true),
    (D, r"[..]", r#"[{"a":1,"a":2,"b":3},2,3]"#, true),
    (D, r"with_entries(.)", r#"{"a":1,"a":2,"b":3}"#, true),
    (
        D,
        r"to_entries | from_entries",
        r#"{"a":1,"a":2,"b":3}"#,
        true,
    ),
    (
        D,
        r"to_entries | map(.value += 10) | from_entries",
        r#"{"a":11,"a":12,"b":13}"#,
        true,
    ),
    (D, r"map_values(. + 10)", r#"{"a":1,"a":12,"b":13}"#, true),
    (D, r".[] |= . + 10", r#"{"a":1,"a":12,"b":13}"#, true),
    (D, r".a = 5", r#"{"a":1,"a":5,"b":3}"#, true),
    (D, r".c = 1", r#"{"a":1,"a":2,"b":3,"c":1}"#, true),
    (D, r"del(.b)", r#"{"a":1,"a":2}"#, true),
    (D, r#". + {"a": 9}"#, r#"{"a":9,"a":2,"b":3}"#, true),
    (D, r#". + {"q": 1}"#, r#"{"a":1,"a":2,"b":3,"q":1}"#, true),
    (D, r#"omit(["b"])"#, r#"{"a":1,"a":2}"#, true),
    (D, r". as $d | $d", r#"{"a":1,"a":2,"b":3}"#, true),
    (D, r"[.] | .[0]", r#"{"a":1,"a":2,"b":3}"#, true),
    (D, r".x.a = 7", r#"{"a":1,"a":2,"b":3,"x":{"a":7}}"#, true),
    (D, r"del(.x.b)", r#"{"a":1,"a":2,"b":3}"#, true),
    (D, r".[0].a = 1", r#"{"a":1,"a":2,"b":3,"0":{"a":1}}"#, true),
    (INTERLEAVED, r"[.]", r#"[{"b":1,"a":2,"b":3}]"#, true),
    (INTERLEAVED, r"[..]", r#"[{"b":1,"a":2,"b":3},3,2]"#, true),
    (
        INTERLEAVED,
        r"with_entries(.)",
        r#"{"b":1,"a":2,"b":3}"#,
        true,
    ),
    (
        INTERLEAVED,
        r"to_entries | from_entries",
        r#"{"b":1,"a":2,"b":3}"#,
        true,
    ),
    (
        INTERLEAVED,
        r"to_entries | map(.value += 10) | from_entries",
        r#"{"b":11,"a":12,"b":13}"#,
        true,
    ),
    (
        INTERLEAVED,
        r"map_values(. + 10)",
        r#"{"b":1,"a":12,"b":13}"#,
        true,
    ),
    (
        INTERLEAVED,
        r".[] |= . + 10",
        r#"{"b":1,"a":12,"b":13}"#,
        true,
    ),
    (INTERLEAVED, r".a = 5", r#"{"b":1,"a":5,"b":3}"#, true),
    (INTERLEAVED, r".c = 1", r#"{"b":1,"a":2,"b":3,"c":1}"#, true),
    (INTERLEAVED, r"del(.a)", r#"{"b":1,"b":3}"#, true),
    (
        INTERLEAVED,
        r#". + {"a": 9}"#,
        r#"{"b":1,"a":9,"b":3}"#,
        true,
    ),
    (
        INTERLEAVED,
        r#". + {"q": 1}"#,
        r#"{"b":1,"a":2,"b":3,"q":1}"#,
        true,
    ),
    (INTERLEAVED, r". as $d | $d", r#"{"b":1,"a":2,"b":3}"#, true),
    (INTERLEAVED, r"[.] | .[0]", r#"{"b":1,"a":2,"b":3}"#, true),
    (
        INTERLEAVED,
        r".x.a = 7",
        r#"{"b":1,"a":2,"b":3,"x":{"a":7}}"#,
        true,
    ),
    (INTERLEAVED, r"del(.x.b)", r#"{"b":1,"a":2,"b":3}"#, true),
    (
        INTERLEAVED,
        r".[0].a = 1",
        r#"{"b":1,"a":2,"b":3,"0":{"a":1}}"#,
        true,
    ),
    (
        NESTED,
        r"[.]",
        r#"[{"x":{"a":1,"a":2,"b":3},"z":[1,2]}]"#,
        true,
    ),
    (NESTED, r"[.[]]", r#"[{"a":1,"a":2,"b":3},[1,2]]"#, true),
    (
        NESTED,
        r"[..]",
        r#"[{"x":{"a":1,"a":2,"b":3},"z":[1,2]},{"a":1,"a":2,"b":3},2,3,[1,2],1,2]"#,
        true,
    ),
    (
        NESTED,
        r"with_entries(.)",
        r#"{"x":{"a":1,"a":2,"b":3},"z":[1,2]}"#,
        true,
    ),
    (
        NESTED,
        r"to_entries | from_entries",
        r#"{"x":{"a":1,"a":2,"b":3},"z":[1,2]}"#,
        true,
    ),
    (
        NESTED,
        r".a = 5",
        r#"{"x":{"a":1,"a":2,"b":3},"z":[1,2],"a":5}"#,
        true,
    ),
    (
        NESTED,
        r".c = 1",
        r#"{"x":{"a":1,"a":2,"b":3},"z":[1,2],"c":1}"#,
        true,
    ),
    (
        NESTED,
        r"del(.b)",
        r#"{"x":{"a":1,"a":2,"b":3},"z":[1,2]}"#,
        true,
    ),
    (
        NESTED,
        r"del(.a)",
        r#"{"x":{"a":1,"a":2,"b":3},"z":[1,2]}"#,
        true,
    ),
    (
        NESTED,
        r#". + {"a": 9}"#,
        r#"{"x":{"a":1,"a":2,"b":3},"z":[1,2],"a":9}"#,
        true,
    ),
    (
        NESTED,
        r#". + {"q": 1}"#,
        r#"{"x":{"a":1,"a":2,"b":3},"z":[1,2],"q":1}"#,
        true,
    ),
    (
        NESTED,
        r#"omit(["b"])"#,
        r#"{"x":{"a":1,"a":2,"b":3},"z":[1,2]}"#,
        true,
    ),
    (
        NESTED,
        r". as $d | $d",
        r#"{"x":{"a":1,"a":2,"b":3},"z":[1,2]}"#,
        true,
    ),
    (
        NESTED,
        r"[.] | .[0]",
        r#"{"x":{"a":1,"a":2,"b":3},"z":[1,2]}"#,
        true,
    ),
    (
        NESTED,
        r".x.a = 7",
        r#"{"x":{"a":1,"a":7,"b":3},"z":[1,2]}"#,
        true,
    ),
    (
        NESTED,
        r"del(.x.b)",
        r#"{"x":{"a":1,"a":2},"z":[1,2]}"#,
        true,
    ),
    (NESTED, r"[.x]", r#"[{"a":1,"a":2,"b":3}]"#, true),
    (
        NESTED,
        r".x | with_entries(.)",
        r#"{"a":1,"a":2,"b":3}"#,
        true,
    ),
    (
        NESTED,
        r".[0].a = 1",
        r#"{"x":{"a":1,"a":2,"b":3},"z":[1,2],"0":{"a":1}}"#,
        true,
    ),
    (
        NESTED,
        r#"map(. + {"z": 0})"#,
        r#"[{"a":1,"a":2,"b":3,"z":0},[1,2,{"z":0}]]"#,
        true,
    ),
    (
        SEQ,
        r"[.]",
        r#"[[{"a":1,"a":2},{"b":5},{"a":7,"a":8,"a":9}]]"#,
        true,
    ),
    (
        SEQ,
        r"[.[]]",
        r#"[{"a":1,"a":2},{"b":5},{"a":7,"a":8,"a":9}]"#,
        true,
    ),
    (
        SEQ,
        r"[..]",
        r#"[[{"a":1,"a":2},{"b":5},{"a":7,"a":8,"a":9}],{"a":1,"a":2},2,{"b":5},5,{"a":7,"a":8,"a":9},9]"#,
        true,
    ),
    (
        SEQ,
        r"with_entries(.)",
        r#"{"0":{"a":1,"a":2},"1":{"b":5},"2":{"a":7,"a":8,"a":9}}"#,
        true,
    ),
    (
        SEQ,
        r"to_entries | from_entries",
        r#"{"0":{"a":1,"a":2},"1":{"b":5},"2":{"a":7,"a":8,"a":9}}"#,
        true,
    ),
    (
        SEQ,
        r#". + {"a": 9}"#,
        r#"[{"a":1,"a":2},{"b":5},{"a":7,"a":8,"a":9},{"a":9}]"#,
        true,
    ),
    (
        SEQ,
        r#". + {"q": 1}"#,
        r#"[{"a":1,"a":2},{"b":5},{"a":7,"a":8,"a":9},{"q":1}]"#,
        true,
    ),
    (
        SEQ,
        r#"omit(["b"])"#,
        r#"[{"a":1,"a":2},{"b":5},{"a":7,"a":8,"a":9}]"#,
        true,
    ),
    (
        SEQ,
        r". as $d | $d",
        r#"[{"a":1,"a":2},{"b":5},{"a":7,"a":8,"a":9}]"#,
        true,
    ),
    (
        SEQ,
        r"[.] | .[0]",
        r#"[{"a":1,"a":2},{"b":5},{"a":7,"a":8,"a":9}]"#,
        true,
    ),
    (SEQ, r"[.[0]]", r#"[{"a":1,"a":2}]"#, true),
    (
        SEQ,
        r".[0].a = 1",
        r#"[{"a":1,"a":1},{"b":5},{"a":7,"a":8,"a":9}]"#,
        true,
    ),
    (
        SEQ,
        r#"map(. + {"z": 0})"#,
        r#"[{"a":1,"a":2,"z":0},{"b":5,"z":0},{"a":7,"a":8,"a":9,"z":0}]"#,
        true,
    ),
];

#[test]
fn the_materializing_routes_keep_a_repeated_key_1344() -> Result<()> {
    for &(doc, filter, expected, also_eval_all) in ROWS {
        let (out, code) = run_yq_stdin(filter, doc, &["-o=json", "-I=0"])?;
        assert_eq!(code, 0, "{filter} on {doc:?}");
        assert_eq!(out.trim(), expected, "{filter} on {doc:?}");
        if also_eval_all {
            let (out, code) = run_yq_stdin(filter, doc, &["--eval-all", "-o=json", "-I=0"])?;
            assert_eq!(code, 0, "--eval-all {filter} on {doc:?}");
            assert_eq!(out.trim(), expected, "--eval-all {filter} on {doc:?}");
        }
    }
    Ok(())
}

/// Presentation: YAML output of a value that holds a repeated key prints every entry, the earlier
/// one plain (the comment tree is keyed by key and describes the last entry only).
#[test]
fn yaml_output_of_a_repeated_key_prints_every_entry_1344() -> Result<()> {
    for (filter, args) in [
        ("[.] | .[0]", &[][..]),
        ("with_entries(.)", &[]),
        (".", &["-P"]),
    ] {
        assert_eq!(
            run_yq_stdin(filter, D, args)?,
            (D.to_string(), 0),
            "{filter} {args:?}"
        );
    }
    Ok(())
}

/// The `--jq-extensions` builtins have no yq oracle, so these rows follow the rule the oracle rows
/// show: a listing builtin sees every entry, a rewrite reaches the last one and passes the rest
/// through (jq defines `walk(f)` over `map_values`).
#[test]
fn the_gated_builtins_keep_a_repeated_key_1344() -> Result<()> {
    for (filter, extra, expected) in [
        (
            "[tostream]",
            &[][..],
            r#"[[["a"],1],[["a"],2],[["b"],3],[["b"]]]"#,
        ),
        ("walk(.)", &[], r#"{"a":1,"a":2,"b":3}"#),
        ("[paths]", &["--eval-all"], r#"[["a"],["a"],["b"]]"#),
        ("[leaf_paths]", &[], r#"[["a"],["a"],["b"]]"#),
    ] {
        let mut args = vec!["--jq-extensions", "-o=json", "-I=0"];
        args.extend_from_slice(extra);
        let (out, code) = run_yq_stdin(filter, D, &args)?;
        assert_eq!(code, 0, "{filter}");
        assert_eq!(out.trim(), expected, "{filter}");
    }
    Ok(())
}

/// An earlier entry carries none of its sibling's presentation, and an anchor is never declared
/// twice. yq writes `&x` on the first entry here and `*x` on the second; the earlier entry is
/// written plain, so the alias is resolved instead (a gap recorded in `limitations.md`).
#[test]
fn an_earlier_entry_does_not_repeat_its_siblings_anchor_1344() -> Result<()> {
    let doc = "a: &x 1\na: *x\nb: 3\n";
    let (out, code) = run_yq_stdin("[.] | .[0]", doc, &[])?;
    assert_eq!(code, 0);
    assert!(out.matches("&x").count() <= 1, "{out:?}");
    assert_eq!(out, "a: 1\na: 1\nb: 3\n");
    Ok(())
}

/// `map_values(f)` where `f` yields nothing for a key's live entry drops the key, every entry of
/// it. That is the answer on a document with no repeated key too (yq keeps the entry there, an
/// older gap), so the repeated-key document is consistent with it rather than a new divergence.
#[test]
fn map_values_that_yields_nothing_drops_the_whole_key_1344() -> Result<()> {
    for (doc, expected) in [("a: 2\nb: 3\n", r#"{"b":3}"#), (D, r#"{"b":3}"#)] {
        let (out, code) = run_yq_stdin("map_values(select(. != 2))", doc, &["-o=json", "-I=0"])?;
        assert_eq!(code, 0, "{doc:?}");
        assert_eq!(out.trim(), expected, "{doc:?}");
    }
    Ok(())
}
