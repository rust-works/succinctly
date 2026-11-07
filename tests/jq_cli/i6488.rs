//! #6488: a loop whose groups the rewrites copy, in a pattern with `\z` or a `$` before its end, is
//! written for the exact `fancy-regex` route that reads such a pattern, where nothing in the pattern
//! can match a newline: `(?:$\z|\B(\w*)){2,}` on `"ba b"` was `["a",""]`, where jq answers
//! `["",""]`. A pattern that may match the final newline its `$` stands before keeps the plain
//! rewrite, which reads it as jq does. Every row was captured from jq 1.7.1 (`jq -nc`).

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn a_loop_with_groups_beside_dollar_and_z_answers_as_jq_does_6488() -> Result<()> {
    for (pattern, flags, input, expected) in [
        (
            r"(?:$\z|\B(\w*)){2,}",
            "g",
            "ba b",
            r#"[[1,"",[""]],[4,"",[""]]]"#,
        ),
        (
            r"(?:$\z|\B(\w*)){2,}",
            "g",
            "ab",
            r#"[[1,"b",["b"]],[2,"",[""]]]"#,
        ),
        (
            r"(?:$\z|\B(\w*)){2,}",
            "g",
            "ba b\n",
            r#"[[1,"",[""]],[5,"",[""]]]"#,
        ),
        // The `i` and `x` flags are read by the rewrites as they are by the engine.
        (
            r"(?:$\z|\B(\w*)){2,}",
            "gi",
            "BA B",
            r#"[[1,"",[""]],[4,"",[""]]]"#,
        ),
        (
            r"(?: $\z | \B(\w*) ){2,}",
            "gx",
            "ba b",
            r#"[[1,"",[""]],[4,"",[""]]]"#,
        ),
        // A way that may take the final newline after `$`: the plain rewrite's reading, as jq's.
        (
            r"($()|(?:a|\n*))+",
            "g",
            "a\n",
            r#"[[0,"a",["",""]],[1,"",["",""]],[2,"",["",""]]]"#,
        ),
        (
            r"($(a)?|(b)*(a)|(a|\n))+",
            "g",
            "a\n",
            r#"[[0,"a",["",null,null,"a",null]],[1,"",["","","","",""]],[2,"",["","","","",""]]]"#,
        ),
        // A group that holds only an anchor keeps the plain rewrite too (#6492).
        (
            r"(?:(a)|b*|c)+(?:(?i)|(\w*)(\z)){2,5}?$",
            "g",
            "cba",
            r#"[[0,"cba",[null,"cba",""]],[3,"",["","",""]]]"#,
        ),
        // The copies write `(.+?)*?`, which `fancy-regex` answers wrongly beside a lookaround: the
        // rewrite is refused there.
        (r"(?:(.*?)|$\b)+?c", "g", "abc", r#"[[0,"abc",["b"]]]"#),
        // The copies of this loop exhaust the backtracking budget, and the pattern as written
        // answers instead.
        (
            r"(?:(b*|(?:(a)|b?){0,2}(b|)*){0,12}?$|c){0,3}",
            "g",
            "ba",
            r#"[[0,"ba",["a","a",""]],[2,"",["","",""]]]"#,
        ),
    ] {
        let (stdout, stderr, status) = run_jq_full(
            &[
                "-nc",
                "--arg",
                "p",
                pattern,
                "--arg",
                "f",
                flags,
                "--arg",
                "s",
                input,
                r"$s | [match($p; $f) | [.offset, .string, (.captures | map(.string))]]",
            ],
            None,
        )?;
        assert_eq!(
            (stdout.trim_end(), stderr.as_str(), status),
            (expected, "", 0),
            "{pattern} ({flags}) on {input:?}"
        );
    }
    Ok(())
}
