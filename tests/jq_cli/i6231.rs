//! #6231: a literal-bound slice of an array (`.[:10]`) is read where it stands, by index, instead
//! of through the wildcard bridge that materializes the whole array. It answers as the computed
//! spelling of the same slice (`.[0+a:0+b]`, native since #4287) does, and as jq 1.7.1 does for a
//! well-formed document; like that spelling it decodes only the elements it keeps, so a fault in
//! one it drops is no longer diagnosed (the #2103 mechanism, recorded in
//! `docs/compliance/jq/limitations.md`). Every row below was captured from `/usr/bin/jq`.

use anyhow::Result;

use crate::run_jq_full;

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
fn test_literal_slice_agrees_with_the_computed_spelling_and_with_jq_6231() -> Result<()> {
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
            let (stdout, _, code) = ours;
            assert_eq!(code, 0, "{literal} on {doc}");
            let oracle = std::process::Command::new("/usr/bin/jq")
                .args(["-c", &literal])
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .spawn()
                .and_then(|mut child| {
                    use std::io::Write as _;
                    child.stdin.take().unwrap().write_all(doc.as_bytes())?;
                    child.wait_with_output()
                });
            // The oracle is the pinned jq on a developer machine; CI without it checks the
            // agreement above and the fixed rows below.
            if let Ok(out) = oracle {
                assert_eq!(
                    stdout,
                    String::from_utf8(out.stdout)?,
                    "{literal} on {doc} against /usr/bin/jq"
                );
            }
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
        (".[1.7:2.9]", "[1,2,3,4,5]", "[2,3]"),
        (".[:2][1:]", "[1,2,3,4,5]", "[2]"),
        (".[1:3] | length", "[1,2,3,4,5]", "2"),
        (r#".a | .[:2]"#, r#"{"a":[1,2,3]}"#, "[1,2]"),
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
/// that keeps it still raises. A structural fault at the root would raise on every route and
/// prove nothing here.
#[test]
fn test_literal_slice_decodes_only_what_it_keeps_6231() -> Result<()> {
    let doc = r#"["a","bad\x",3,4]"#;
    for (filter, expected_stdout, expected_code) in [
        (".[:1]", Some(r#"["a"]"#), 0),
        (".[2:]", Some("[3,4]"), 0),
        (".[-2:]", Some("[3,4]"), 0),
        // The controls: the window holds the bad element.
        (".[1:2]", None, 5),
        (".[:2]", None, 5),
        (".[1:]", None, 5),
        (".[1:2] | tojson", None, 5),
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-c", filter], Some(doc))?;
        assert_eq!(
            code, expected_code,
            "{filter}: stdout={stdout:?} stderr={stderr:?}"
        );
        if let Some(expected) = expected_stdout {
            assert_eq!(stdout.trim_end(), expected, "{filter}");
        } else {
            assert!(stderr.contains("Invalid escape"), "{filter}: {stderr:?}");
        }
    }
    Ok(())
}
