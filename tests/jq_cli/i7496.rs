//! #7496: a number literal directly followed by `else`, `elif` or `end` is the number and then the
//! keyword. jq's lexer takes the longest match of `[0-9]+(\.[0-9]*)?([eE][+-]?[0-9]+)?` (or the
//! `.5` spelling), so an `e` that no digit follows ends the number before it; `succinctly jq` read
//! `2else` as a malformed exponent and refused with `invalid number`. Every expected byte string was
//! captured from jq 1.7.1 (`/usr/bin/jq`); jq 1.8.2 prints the same output for each valid program and
//! refuses each invalid one. Syntax-error wording is a recorded divergence (#2703), so the invalid
//! programs pin the exit status and the fact of refusal.

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn test_number_directly_before_else_elif_end_is_lexed_as_number_then_keyword_7496() -> Result<()> {
    for (program, expected) in [
        ("if 1 then 2else 3 end", "2\n"),
        ("if 0 then 2 else 3end", "2\n"),
        ("if 0 then 2 elif 1 then 3end", "2\n"),
        ("if 1 then 1.5else 2 end", "1.5\n"),
        ("if 1 then 1-1else 2 end", "0\n"),
        ("if 1 then 31end", "31\n"),
        ("if 1 then .5end", "0.5\n"),
        ("if 1 then .5else 2 end", "0.5\n"),
        ("if 1 then 3.else 2 end", "3\n"),
        ("if 1 then 3.end", "3\n"),
        ("if null then 1 elif null then 2 else 3end", "3\n"),
        (
            "if null then 1 elif null then 2 elif 1 then 4else 5end",
            "4\n",
        ),
        ("[if 1 then 2end]", "[2]\n"),
        ("if 0 then 1 else 2end|.+1", "2\n"),
        // A real exponent still binds first.
        ("if 1 then 2e1else 3 end", "2E+1\n"),
        ("if 1 then 2e+1else 3 end", "2E+1\n"),
        ("if 1 then 2E-1end", "0.2\n"),
        ("if 1 then 1.5e2end", "1.5E+2\n"),
        // Controls that already matched: a number before `and`, `or` and `as`.
        ("2and 3", "true\n"),
        ("1or 2", "true\n"),
        ("1as $x|$x", "1\n"),
    ] {
        let (out, err, code) = run_jq_full(&["-nc", program], None)?;
        assert_eq!((out.as_str(), code), (expected, 0), "{program}: {err}");
    }
    Ok(())
}

#[test]
fn test_number_then_e_that_no_digit_follows_is_still_a_syntax_error_7496() -> Result<()> {
    // jq: the number ends before the `e`, and the rest is no valid continuation (exit 3).
    for program in [
        "1else", "1e1else", "[1end]", "2e", "2ex", "2e+", "2e+x", "1.e", "1.e+", "1e-end", "1ex1",
        "0x10", "1if", "2then", "3 end", "1E5end", "1_000",
    ] {
        let (out, _err, code) = run_jq_full(&["-nc", program], None)?;
        assert_eq!((out.as_str(), code), ("", 3), "{program}");
    }
    Ok(())
}
