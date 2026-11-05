//! #6214: a plain scalar in a flow collection that continues, after a line break, on a line whose
//! leading blanks hold a tab at a column below the enclosing block's indent plus one is refused by
//! yq (`found a tab character that violates indentation`, #6059). The break may be U+2028, U+2029
//! or U+0085 as well as `\n`: the scalar decoder folds them alike, and the scanners read them as
//! text, so the tab went unseen. The document root has no block indent to violate. A tab after
//! enough spaces, and a quoted scalar, are legal. Every row was captured from yq v4.53.3.

use anyhow::Result;

use crate::{run_yq_stdin, run_yq_stdin_with_stderr};

const BREAKS: [&str; 3] = ["\u{2028}", "\u{2029}", "\u{85}"];

#[test]
fn test_tab_after_a_unicode_break_in_a_flow_plain_scalar_is_refused_6214() -> Result<()> {
    for brk in BREAKS {
        for template in [
            "a: [x{B}\ty]\n",
            "a: [x{B}\t y]\n",
            "- [x{B}\ty]\n",
            "a: {k: x{B}\ty}\n",
            "a: [x{B}\ty, z]\n",
            "a: [? x{B}\ty : 1]\n",
            "a: {? x{B}\ty : 1}\n",
            // A tab before a closing indicator, a comma or a comment is refused as well.
            "a: [x{B}\t]\n",
            "a: [x{B}\t, z]\n",
            "a: [x{B}\t# c\n]\n",
        ] {
            let doc = template.replace("{B}", brk);
            let (stdout, stderr, code) = run_yq_stdin_with_stderr(".", &doc, &["-o=json", "-I=0"])?;
            assert_eq!((stdout.as_str(), code), ("", 1), "{doc:?}: {stderr}");
            assert!(stderr.contains("tab"), "{doc:?}: {stderr}");
        }
    }
    Ok(())
}

#[test]
fn test_unicode_break_with_a_legal_tab_still_reads_6214() -> Result<()> {
    for (brk, folded) in [
        // U+2028 and U+2029 are kept and JSON-escaped; U+0085 folds to a space (#4913).
        ("\u{2028}", "x\\u2028y"),
        ("\u{2029}", "x\\u2029y"),
        ("\u{85}", "x y"),
    ] {
        // A tab after spaces, a quoted scalar and the document root.
        for (template, expected) in [
            (
                "a: [x{B}\u{20}\ty]\n",
                format!("{{\"a\":[\"{folded}\"]}}\n"),
            ),
            (
                "a: [x{B}\u{20}\u{20}\ty]\n",
                format!("{{\"a\":[\"{folded}\"]}}\n"),
            ),
            ("[x{B}\ty]\n", format!("[\"{folded}\"]\n")),
            ("a: \"x{B}\ty\"\n", format!("{{\"a\":\"{folded}\"}}\n")),
        ] {
            let doc = template.replace("{B}", brk);
            let (stdout, code) = run_yq_stdin(".", &doc, &["-o=json", "-I=0"])?;
            assert_eq!((stdout.as_str(), code), (expected.as_str(), 0), "{doc:?}");
        }
    }
    Ok(())
}
