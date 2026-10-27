//! #3563: a computed or multi-key index (`.[$k]`, `.["b","c"]`, `.[0,1]`) reads its target's
//! explicit YAML tag before its text at the null test, as the literal field and index arms do
//! (#3533): `!!null foo` is null, `!!str null` is not.
//!
//! Every expected output is captured from yq v4.53.3 (`-o=json -I=0`, stdout only).

use anyhow::Result;

use crate::run_yq_stdin;

fn check(input: &str, rows: &[(&str, &str)]) -> Result<()> {
    for (filter, want) in rows {
        let (out, code) = run_yq_stdin(filter, input, &["-o=json", "-I=0"])?;
        assert_eq!(code, 0, "{filter}");
        assert_eq!(out, *want, "{filter}");
    }
    Ok(())
}

/// A scalar tagged `!!null` is null for every key kind, though its text is `foo`.
#[test]
fn test_3563_tagged_null_scalar_indexes_as_null() -> Result<()> {
    check(
        "a: !!null foo\n",
        &[
            (r#""b" as $k | .a | .[$k]"#, "null\n"),
            (r#".a | .["b","c"]"#, "null\nnull\n"),
            (".a | .[0,1]", "null\nnull\n"),
            ("0 as $k | .a | .[$k]", "null\n"),
            (".a | .[true,null]", "null\nnull\n"),
        ],
    )
}

/// A scalar tagged `!!str` is not null though its text is `null`: indexing it is empty.
#[test]
fn test_3563_tagged_str_null_scalar_is_not_null() -> Result<()> {
    check(
        "a: !!str null\n",
        &[
            (r#""b" as $k | .a | .[$k]"#, ""),
            (r#".a | .["b","c"]"#, ""),
            (".a | .[0,1]", ""),
            ("(0 - 1) as $k | .a | .[$k]", ""),
            (".a | .[true,null]", ""),
        ],
    )
}

/// An untagged null, a plain string and the containers keep their answers.
#[test]
fn test_3563_untagged_targets_unchanged() -> Result<()> {
    check(
        "n: null\ntilde: ~\ns: foo\nl: [1, 2]\nm: {b: 1}\n",
        &[
            (".n | .[0,1]", "null\nnull\n"),
            (r#".tilde | .["b","c"]"#, "null\nnull\n"),
            (r#".s | .["b","c"]"#, ""),
            (".l | .[0,1]", "1\n2\n"),
            (r#".m | .["b","c"]"#, "1\nnull\n"),
        ],
    )
}
