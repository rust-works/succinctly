//! #5433: yq's `as` takes a variable name and nothing else, so a destructuring pattern is jq
//! surface `succinctly yq` accepts only under `--jq-extensions`. Pinned yq v4.53.3 rejects every
//! one of these: `{...}` patterns at its lexer (`invalid input text`), `[...]` and `{"k": ...}`
//! patterns with `RHS of 'as' operator must be a variable name`.

use anyhow::Result;

use crate::run_yq_stdin_with_stderr;

#[test]
fn test_destructuring_patterns_need_jq_extensions_5433() -> Result<()> {
    let plain = &["-o=json", "-I=0"];
    let extended = &["--jq-extensions", "-o=json", "-I=0"];
    for (filter, expected) in [
        (". as {a: $v} | $v", "[1]\n"),
        (". as {$a} | $a", "[1]\n"),
        (r#". as {"a": $v} | $v"#, "[1]\n"),
        (". as {(\"a\"): $v} | $v", "[1]\n"),
        (". as {a: [$v]} | $v", "1\n"),
        // A pattern key spelled like a name #4808 gated is still a pattern, not a builtin.
        (". as {values: $v} | $v", "null\n"),
        (".a as [$v] | $v", "1\n"),
        ("[.] as [{a: $v}] | $v", "[1]\n"),
    ] {
        let (stdout, stderr, code) = run_yq_stdin_with_stderr(filter, "a: [1]\n", plain)?;
        assert_eq!(stdout, "", "`{filter}` must print nothing without the flag");
        assert_ne!(code, 0, "`{filter}` must be rejected without the flag");
        assert!(
            stderr.contains("--jq-extensions"),
            "`{filter}` error must name the flag: {stderr:?}"
        );
        assert_eq!(
            run_yq_stdin_with_stderr(filter, "a: [1]\n", extended)?,
            (expected.into(), String::new(), 0),
            "--jq-extensions `{filter}`"
        );
    }
    Ok(())
}

/// The plain `as $name` binding is yq grammar, with or without the flag.
#[test]
fn test_plain_variable_binding_needs_no_flag_5433() -> Result<()> {
    for args in [
        &["-o=json", "-I=0"][..],
        &["--jq-extensions", "-o=json", "-I=0"],
    ] {
        assert_eq!(
            run_yq_stdin_with_stderr(". as $x | $x.a", "a: [1]\n", args)?,
            ("[1]\n".into(), String::new(), 0),
            "{args:?}"
        );
    }
    Ok(())
}
