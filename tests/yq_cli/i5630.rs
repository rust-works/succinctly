//! #5630: yq gives `+ - * / % //` one precedence (42) and `==`, `!=`, the ordering operators and the
//! assignments another (40), and its shunting-yard pops only a strictly higher one, so a chain of
//! either folds from the right: `2 * 3 + 4` is `2 * (3 + 4)` (14) and `10 - 3 - 2` is
//! `10 - (3 - 2)` (9). Every expected value was captured from yq (v4.52.2 on the machine that wrote
//! this; the precedence table is the same in v4.53.3). `succinctly jq` keeps jq's grammar.

use std::process::Command;

use anyhow::Result;

/// `(program, expected stdout)`, run as `yq -n <program>`.
const ROWS: &[(&str, &str)] = &[
    ("2 * 3 + 4", "14\n"),
    ("10 - 3 - 2", "9\n"),
    ("1 - 2 - 3 - 4", "-2\n"),
    ("100 / 10 / 5", "50\n"),
    ("2 - 1 + 1", "0\n"),
    ("10 % 4 % 3", "0\n"),
    ("2 == 1 // 2", "false\n"),
    ("1 // 2 == 2", "false\n"),
    ("2 + 3 * 4", "14\n"),
    ("1 + 2 + 3", "6\n"),
    ("2 * 3 * 4", "24\n"),
    ("\"a\" + \"b\" + \"c\"", "abc\n"),
    ("1 // 2 // 3", "1\n"),
    ("null // 2 + 3", "5\n"),
    ("2 * 3 + 4 == 14", "true\n"),
    ("2 * 3 + 4 == 10", "false\n"),
    ("[1 - 2 - 3]", "- 2\n"),
    ("{\"k\": 10 - 3 - 2}", "k: 9\n"),
    ("5 - 2 - 1 , 8 / 4 / 2", "4\n4\n"),
    ("1 + 2 * 3 - 4", "-1\n"),
    (".a = 5 == 5", "a: true\n"),
    (".a = 1 // 2", "a: 1\n"),
    (".a += 1 + 2 * 3", "a: 7\n"),
];

fn run(args: &[&str]) -> Result<(String, i32)> {
    let output = Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .args(args)
        .output()?;
    Ok((
        String::from_utf8(output.stdout)?,
        output.status.code().unwrap_or(-1),
    ))
}

#[test]
fn test_arithmetic_and_alternative_fold_from_the_right_5630() -> Result<()> {
    for (program, expected) in ROWS {
        assert_eq!(run(&["yq", "-n", program])?, ((*expected).to_string(), 0), "{program:?}");
    }
    Ok(())
}

/// jq mode keeps jq's grammar: `*` over `+`, left-associative `-`.
#[test]
fn test_jq_mode_keeps_conventional_arithmetic_5630() -> Result<()> {
    for (program, expected) in [("2 * 3 + 4", "10\n"), ("10 - 3 - 2", "5\n"), ("100 / 10 / 5", "2\n")] {
        assert_eq!(run(&["jq", "-n", program])?, (expected.to_string(), 0), "{program:?}");
    }
    Ok(())
}

/// A long chain is bounded by the expression-nesting limit like any other: an error, not a stack
/// overflow, however many operands there are.
#[test]
fn test_a_long_chain_is_an_error_not_a_stack_overflow_5630() -> Result<()> {
    for op in ["-", "//", "=="] {
        let program = vec!["1"; 20_000].join(&format!(" {op} "));
        let (stdout, code) = run(&["yq", "-n", &program])?;
        assert_eq!((stdout.as_str(), code), ("", 1), "{op}");
    }
    Ok(())
}
