//! #6231: a literal-bound slice of an array (`.[:10]`) is read where it stands, by index, instead
//! of through the wildcard bridge that materializes the whole array. It answers as the computed
//! spelling of the same slice (`.[0+a:0+b]`, native since #4287) does, and as jq 1.7.1 does for a
//! well-formed document; like that spelling it decodes only the elements it keeps, so a bad value
//! inside one it drops is no longer diagnosed (the #2103 mechanism, recorded in
//! `docs/compliance/jq/limitations.md`). The array's own structure (a missing separator, a stray
//! comma) is still checked, as the bridge checked it. The rows of
//! `test_literal_slice_rows_captured_from_jq_6231` were captured from jq 1.7.1; the two fault tests
//! pin succinctly's own reading of a document jq rejects for every filter.

use anyhow::Result;

use crate::{names_fault, run_jq_full};

const DOCS: [&str; 9] = [
    "[1,2,3,4,5]",
    "[]",
    "[1]",
    r#"[{"a":1},{"a":2},{"a":3}]"#,
    "[[1,2],[3,4],[5]]",
    r#"[1,"x\u00e9",3,4]"#,
    "[1.5,2.5e3,true,null]",
    r#""hello""#,
    "null",
];

// `.[:]` is not jq syntax.
const BOUNDS: [(Option<i64>, Option<i64>); 13] = [
    (Some(0), None),
    (None, Some(0)),
    (None, Some(2)),
    (Some(1), None),
    (Some(1), Some(3)),
    (Some(2), Some(2)),
    (Some(3), Some(1)),
    (Some(-2), None),
    (None, Some(-1)),
    (Some(-3), Some(-1)),
    (Some(0), Some(99)),
    (Some(99), Some(100)),
    (Some(-99), Some(2)),
];

fn bound(b: Option<i64>) -> String {
    b.map_or_else(String::new, |n| n.to_string())
}

#[test]
fn test_literal_slice_agrees_with_the_computed_spelling_6231() -> Result<()> {
    for doc in DOCS {
        for (from, to) in BOUNDS {
            let literal = format!(".[{}:{}]", bound(from), bound(to));
            let computed = format!(
                ".[{}:{}]",
                from.map_or_else(String::new, |n| format!("0+({n})")),
                to.map_or_else(String::new, |n| format!("0+({n})"))
            );
            let ours = run_jq_full(&["-c", &literal], Some(doc))?;
            let theirs = run_jq_full(&["-c", &computed], Some(doc))?;
            assert_eq!(ours, theirs, "{literal} vs {computed} on {doc}");
            assert_eq!(ours.2, 0, "{literal} on {doc}");
        }
    }
    Ok(())
}

#[test]
fn test_literal_slice_rows_captured_from_jq_6231() -> Result<()> {
    for (filter, doc, expected) in [
        (".[1:3]", "[1,2,3,4,5]", "[2,3]"),
        (".[:2]", "[1,2,3,4,5]", "[1,2]"),
        (".[-2:]", "[1,2,3,4,5]", "[4,5]"),
        (".[3:1]", "[1,2,3,4,5]", "[]"),
        // A bound that is integral folds to a literal one; `.[1.7:2.9]` is the computed spelling.
        (".[1.0:3.0]", "[1,2,3,4,5]", "[2,3]"),
        (".[1.7:2.9]", "[1,2,3,4,5]", "[2,3]"),
        (".[:2][1:]", "[1,2,3,4,5]", "[2]"),
        (".[1:3] | length", "[1,2,3,4,5]", "2"),
        (r".a | .[:2]", r#"{"a":[1,2,3]}"#, "[1,2]"),
        (".[1:3]", r#""hello""#, r#""el""#),
        (".[1:3]", "null", "null"),
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-c", filter], Some(doc))?;
        assert_eq!(
            (stdout.trim_end(), code),
            (expected, 0),
            "{filter} on {doc}: stderr={stderr:?}"
        );
    }
    Ok(())
}

/// The document is `["a","bad\x",3,4]`, whose element 1 has an invalid escape: jq rejects the
/// document, and `succinctly jq` raises where the fault is read. A slice that drops the element no
/// longer reads it (the fault is in a leaf value, which no part of the slice decodes); a slice
/// that keeps it still raises. A structural fault would raise on every route and prove nothing
/// here, so the fault is one the dropped element's text holds.
#[test]
fn test_literal_slice_decodes_only_what_it_keeps_6231() -> Result<()> {
    let doc = r#"["a","bad\x",3,4]"#;
    for (filter, expected_stdout) in [
        (".[:1]", Some(r#"["a"]"#)),
        (".[2:]", Some("[3,4]")),
        (".[-2:]", Some("[3,4]")),
        // The controls: the window holds the bad element.
        (".[1:2]", None),
        (".[:2]", None),
        (".[1:]", None),
        (".[1:2] | tojson", None),
        (".[0:3][1]", None),
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-c", filter], Some(doc))?;
        if let Some(expected) = expected_stdout {
            assert_eq!(
                (stdout.trim_end(), code),
                (expected, 0),
                "{filter}: stderr={stderr:?}"
            );
        } else {
            assert_eq!(code, 5, "{filter}: stdout={stdout:?} stderr={stderr:?}");
            assert!(
                names_fault(&stderr, "invalid escape"),
                "{filter}: {stderr:?}"
            );
        }
    }
    // A bad value that is not a string, outside the window.
    let (stdout, _, code) = run_jq_full(&["-c", ".[1:]"], Some("[tru,1,2]"))?;
    assert_eq!((stdout.trim_end(), code), ("[1,2]", 0));
    let (stdout, _, code) = run_jq_full(&["-c", "(.[0:1])[0]"], Some("[tru,1]"))?;
    assert_eq!((stdout.as_str(), code), ("", 5));
    Ok(())
}

/// The array's own structure is checked wherever the slice falls, as the bridge checked it (and as
/// `.`, `length` and `.[0]` check it): a missing separator, a stray or trailing comma, a comma in
/// an array with no element, a glued token. Every row exited 5 before #6231 and exits 5 now.
#[test]
fn test_literal_slice_still_checks_the_arrays_structure_6231() -> Result<()> {
    let slices = [".[0:2]", ".[1:]", ".[:1]", ".[-1:]", ".[0:1]"];
    for (doc, prefix) in [
        ("[1 2 3]", ""),
        ("[1,,2]", ""),
        ("[1,2,]", ""),
        ("[,]", ""),
        ("[,1]", ""),
        ("[1:2]", ""),
        (r#"[1,"a"b,3]"#, ""),
        (r#"{"a":[1 2 3]}"#, ".a"),
        (r#"{"a":[,]}"#, ".a"),
    ] {
        for slice in slices {
            let filter = if prefix.is_empty() {
                slice.to_string()
            } else {
                format!("{prefix} | {slice}")
            };
            let (stdout, stderr, code) = run_jq_full(&["-c", &filter], Some(doc))?;
            assert_eq!(
                code, 5,
                "{filter} on {doc}: stdout={stdout:?} stderr={stderr:?}"
            );
        }
    }
    Ok(())
}
