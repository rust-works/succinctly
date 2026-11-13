//! #6973: under `--eval-all` an arithmetic result is a copy of its left operand's node in yq (`lhs.Copy()`), so it
//! keeps that node's `EvaluateTogether` flag: `. * {"z":1} | [.]` is one array of every result, and a second
//! binary stage over the results crosses the whole list (`(. * {"z":1}) | . + {"w":2}` is 4 x 4 results). A left
//! operand that navigated or built its node has no flag (`.x + 1 | [.]` is one array per result), and a
//! comparison's boolean is a node of its own. `succinctly yq` gave every pairing a fresh tag.
//!
//! Every row was captured from yq v4.53.3 as `ea -o=json -I=0` over one file of two or three documents: `md` is
//! `x: 1`, `y: 2` / `x: 3`, `y: 4`; `ar` is `[1, 2]` / `[3]`; `sc` is `5` / `6`; `st` is `a: x` / `a: y`. Still
//! differing, listed in `limitations.md`: `line` of a computed result, an `as` binding over the list, and the
//! `document_index` of a comparison's boolean.

use anyhow::Result;
use tempfile::TempDir;

use crate::run_yq_files;

#[rustfmt::skip]
const ROWS: &[(&str, &str, &str)] = &[
    ("md", ". * {\"z\":1} | [.]", "[{\"x\":1,\"y\":2,\"z\":1},{\"x\":1,\"y\":2,\"z\":1},{\"x\":3,\"y\":4,\"z\":1},{\"x\":3,\"y\":4,\"z\":1}]\n"),
    ("md", ". + {\"z\":1} | [.]", "[{\"x\":1,\"y\":2,\"z\":1},{\"x\":1,\"y\":2,\"z\":1},{\"x\":3,\"y\":4,\"z\":1},{\"x\":3,\"y\":4,\"z\":1}]\n"),
    ("md", ". * . | [.]", "[{\"x\":1,\"y\":2},{\"x\":3,\"y\":4},{\"x\":1,\"y\":2},{\"x\":3,\"y\":4}]\n"),
    ("md", ". + . | [.]", "[{\"x\":1,\"y\":2},{\"x\":3,\"y\":4},{\"x\":1,\"y\":2},{\"x\":3,\"y\":4}]\n"),
    ("ar", ". + [9] | [.]", "[[1,2,9,9],[3,9,9]]\n"),
    ("ar", ". * [9] | [.]", "[[9,9],[9,9]]\n"),
    ("ar", ". - [1] | [.]", "[[2],[3]]\n"),
    ("sc", ". + 1 | [.]", "[6,6,7,7]\n"),
    ("sc", ". * 2 | [.]", "[10,10,12,12]\n"),
    ("st", ".a + \"s\" | [.]", "[\"xs\"]\n[\"xs\"]\n[\"ys\"]\n[\"ys\"]\n"),
    ("md", ".x + 1 | [.]", "[2]\n[2]\n[4]\n[4]\n"),
    ("md", ". *+ {\"z\":1} | [.]", "[{\"x\":1,\"y\":2,\"z\":1},{\"x\":1,\"y\":2,\"z\":1},{\"x\":3,\"y\":4,\"z\":1},{\"x\":3,\"y\":4,\"z\":1}]\n"),
    ("md", ". == . | [.]", "[false]\n[false]\n[false]\n[false]\n"),
    ("md", "{\"q\":1} + . | [.]", "[{\"q\":1,\"x\":1,\"y\":2}]\n[{\"q\":1,\"x\":3,\"y\":4}]\n[{\"q\":1,\"x\":1,\"y\":2}]\n[{\"q\":1,\"x\":3,\"y\":4}]\n"),
    ("md", "(. * {\"z\":1}) | . + {\"w\":2}", "{\"x\":1,\"y\":2,\"z\":1,\"w\":2}\n{\"x\":1,\"y\":2,\"z\":1,\"w\":2}\n{\"x\":1,\"y\":2,\"z\":1,\"w\":2}\n{\"x\":1,\"y\":2,\"z\":1,\"w\":2}\n{\"x\":1,\"y\":2,\"z\":1,\"w\":2}\n{\"x\":1,\"y\":2,\"z\":1,\"w\":2}\n{\"x\":1,\"y\":2,\"z\":1,\"w\":2}\n{\"x\":1,\"y\":2,\"z\":1,\"w\":2}\n{\"x\":3,\"y\":4,\"z\":1,\"w\":2}\n{\"x\":3,\"y\":4,\"z\":1,\"w\":2}\n{\"x\":3,\"y\":4,\"z\":1,\"w\":2}\n{\"x\":3,\"y\":4,\"z\":1,\"w\":2}\n{\"x\":3,\"y\":4,\"z\":1,\"w\":2}\n{\"x\":3,\"y\":4,\"z\":1,\"w\":2}\n{\"x\":3,\"y\":4,\"z\":1,\"w\":2}\n{\"x\":3,\"y\":4,\"z\":1,\"w\":2}\n"),
    ("md", ". * {\"z\":1} | . + {\"w\":2} | [.]", "[{\"x\":1,\"y\":2,\"z\":1,\"w\":2},{\"x\":1,\"y\":2,\"z\":1,\"w\":2},{\"x\":1,\"y\":2,\"z\":1,\"w\":2},{\"x\":1,\"y\":2,\"z\":1,\"w\":2},{\"x\":1,\"y\":2,\"z\":1,\"w\":2},{\"x\":1,\"y\":2,\"z\":1,\"w\":2},{\"x\":1,\"y\":2,\"z\":1,\"w\":2},{\"x\":1,\"y\":2,\"z\":1,\"w\":2},{\"x\":3,\"y\":4,\"z\":1,\"w\":2},{\"x\":3,\"y\":4,\"z\":1,\"w\":2},{\"x\":3,\"y\":4,\"z\":1,\"w\":2},{\"x\":3,\"y\":4,\"z\":1,\"w\":2},{\"x\":3,\"y\":4,\"z\":1,\"w\":2},{\"x\":3,\"y\":4,\"z\":1,\"w\":2},{\"x\":3,\"y\":4,\"z\":1,\"w\":2},{\"x\":3,\"y\":4,\"z\":1,\"w\":2}]\n"),
    ("md", "(. + {\"z\":1}) | del(.x) | [.]", "[{\"y\":2,\"z\":1},{\"y\":2,\"z\":1},{\"y\":4,\"z\":1},{\"y\":4,\"z\":1}]\n"),
    ("md", ".x + 1 | . + 1", "3\n3\n5\n5\n"),
    ("sc", ". + 1 | . + 1", "7\n7\n7\n7\n7\n7\n7\n7\n8\n8\n8\n8\n8\n8\n8\n8\n"),
    ("md", "(. | . + {\"z\":1}) | [.]", "[{\"x\":1,\"y\":2,\"z\":1},{\"x\":1,\"y\":2,\"z\":1},{\"x\":3,\"y\":4,\"z\":1},{\"x\":3,\"y\":4,\"z\":1}]\n"),
    ("md", "(.x, 7) + (10,20) | [.]", "[11]\n[11]\n[21]\n[21]\n[13]\n[13]\n[23]\n[23]\n[17]\n[17]\n[27]\n[27]\n[17]\n[17]\n[27]\n[27]\n"),
];

fn text(which: &str) -> &'static str {
    match which {
        "md" => "x: 1\ny: 2\n---\nx: 3\ny: 4\n",
        "ar" => "- 1\n- 2\n---\n- 3\n",
        "sc" => "5\n---\n6\n",
        "st" => "a: x\n---\na: y\n",
        other => unreachable!("no file {other}"),
    }
}

#[test]
fn eval_all_arithmetic_keeps_the_left_operands_flag_6973() -> Result<()> {
    let dir = TempDir::new()?;
    for &(which, filter, expected) in ROWS {
        let path = dir.path().join(format!("{which}.yaml"));
        std::fs::write(&path, text(which))?;
        let (stdout, stderr, status) =
            run_yq_files(filter, &[path.as_path()], &["ea", "-o=json", "-I=0"])?;
        assert_eq!(
            (stdout.as_str(), stderr.as_str(), status),
            (expected, "", 0),
            "{filter} over {which}"
        );
    }
    Ok(())
}
