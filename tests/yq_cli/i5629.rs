//! #5629: yq gives `==`, `!=` and the ordering operators one precedence (40) and its shunting-yard
//! pops only a strictly higher one, so a chain folds from the right: `1 == 1 == true` is
//! `1 == (1 == true)`, which is `false`. Every expected value was captured from yq v4.53.3.
//! `succinctly jq` keeps jq's refusal of a chain (`tests/jq_cli/i5629.rs`).

use std::process::Command;

use anyhow::Result;

/// `(program, expected stdout)`, run as `yq -n <program>`.
const ROWS: &[(&str, &str)] = &[
    ("1 == 1 == true", "false\n"),
    ("1 == 1 == 1", "false\n"),
    ("1 == 2 == false", "false\n"),
    ("true == (1 == 1) == true", "true\n"),
    ("(1 == 1) == 1 == 1", "true\n"),
    ("1 != 2 != 3", "true\n"),
    ("\"a\" == \"a\" != false", "false\n"),
    ("1 + 1 == 2 == true", "false\n"),
    ("[1] | .[0] == 1 == true", "false\n"),
    ("1 == 1 == 1 == 1 == 1", "false\n"),
    ("null == null == true", "false\n"),
    ("1 == 1 and true == true", "true\n"),
    ("1 == 1 == 1 | not", "true\n"),
    ("[1 == 1 == 1]", "- false\n"),
    ("{\"k\": 1 == 1 == 1}", "k: false\n"),
    ("1 == 1 == 1 , 2 == 2 == 2", "false\nfalse\n"),
    ("1 ==\r1 ==\x0c1", "false\n"),
];

fn run(program: &str) -> Result<(String, i32)> {
    let output = Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .args(["yq", "-n", program])
        .output()?;
    Ok((
        String::from_utf8(output.stdout)?,
        output.status.code().unwrap_or(-1),
    ))
}

#[test]
fn test_a_chain_of_equality_comparisons_folds_from_the_right_5629() -> Result<()> {
    for (program, expected) in ROWS {
        assert_eq!(run(program)?, ((*expected).to_string(), 0), "{program:?}");
    }
    Ok(())
}

/// A chain is bounded by the expression-nesting limit, like a long run of `+`: an error, not a
/// stack overflow, however many operands there are.
#[test]
fn test_a_very_long_chain_is_refused_not_a_crash_5629() -> Result<()> {
    let program = vec!["1"; 100_000].join(" == ");
    let (out, code) = run(&program)?;
    assert_eq!((out.as_str(), code), ("", 1));
    Ok(())
}
