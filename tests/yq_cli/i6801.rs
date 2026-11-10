//! #6801: yq builds an empty `{}` **once** however many nodes reach it, where `[]` and every other
//! literal are built once per node: `.x[] | {}` is one `{}`, `(.x[], .x[]) | {}` too, and
//! `.x[] | select(. > 5) | {}` is one from an empty context. Every row is `yq -o=json -I0 FILTER`
//! on [`DOC`], captured from yq v4.53.3. `docs/compliance/yq/limitations.md` records `map({})`,
//! the one shape that still answers per element.

use anyhow::Result;

use crate::run_yq_stdin;

const DOC: &str = "x: [1, 2, 3]\n";

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
    ("del(.x[] | {})", "{\"x\":[1,2,3]}\n"),
    (".x[] |= {}", "{\"x\":[{},{},{}]}\n"),
    ("[.x[] | {}] | length", "1\n"),
    (".x[] | {} | keys", "[]\n"),
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
