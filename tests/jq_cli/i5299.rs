//! #5299: in jq a `.` followed by a digit is a number literal (`.5`, `.00005`), because its lexer
//! takes the longest token, so `select(. > .00005)` compiles. #1171 fixed the same spelling in input
//! documents, not in filter text. The spelling is the one jq's decNumber prints (`.50` is `0.50`,
//! `.5e2` is `5E+1`). A `.` and a digit after a term (`.a.5`, `.5.5`) is still a syntax error.
//! Every answer below was captured from jq 1.7.1.

use anyhow::Result;

use crate::run_jq_stdin;

#[test]
fn test_leading_dot_number_literal_in_a_filter_5299() -> Result<()> {
    for (filter, input, want, want_code) in [
        (".5 + 1", "null", "1.5\n", 0),
        (
            "[.5, .00005, .50, .0, .5e2, .5E-1]",
            "null",
            "[0.5,0.00005,0.50,0.0,5E+1,0.05]\n",
            0,
        ),
        ("map(select(. > .00005))", "[0, 1]", "[1]\n", 0),
        (". > .5", "1", "true\n", 0),
        ("{a: .5}", "null", "{\"a\":0.5}\n", 0),
        (".5 as $x | $x", "null", "0.5\n", 0),
        ("-.5", "null", "-0.5\n", 0),
        (".5 * 2", "null", "1\n", 0),
        (".5 | tostring", "null", "\"0.5\"\n", 0),
        ("[.5] | tojson", "null", "\"[0.5]\"\n", 0),
        ("[.5?]", "null", "[0.5]\n", 0),
        (".5 // 1", "null", "0.5\n", 0),
        (".[.5]", "[1,2]", "1\n", 0),
        (".5.b", "null", "", 5),
        (".5[0]", "null", "", 5),
        (".e5", "{\"e5\":3}", "3\n", 0),
        ("\"\\(.5)\"", "null", "\"0.5\"\n", 0),
        (".[.5:]", "[1,2,3]", "[1,2,3]\n", 0),
        (".[:1.5]", "[1,2,3]", "[1,2]\n", 0),
        ("[limit(.5; 1,2)]", "null", "[1]\n", 0),
        ("reduce range(3) as $i (.5; . + .5)", "null", "2\n", 0),
        (".[.5]", "[1,2,3]", "1\n", 0),
        ("@base64 \"x\\(.5)\"", "null", "\"xMC41\"\n", 0),
        (".5 | -.", "null", "-0.5\n", 0),
        ("if .5 then 1 else 2 end", "null", "1\n", 0),
    ] {
        let (out, code) = run_jq_stdin(filter, input, &["-c"])?;
        assert_eq!(
            (out.as_str(), code),
            (want, want_code),
            "{filter} on {input}"
        );
    }
    Ok(())
}

#[test]
fn test_dot_digit_after_a_term_stays_a_syntax_error_5299() -> Result<()> {
    for filter in [".a.5", ".5.5", "..5", "1.5.5"] {
        let (out, code) = run_jq_stdin(filter, "null", &["-c"])?;
        assert_eq!((out.as_str(), code), ("", 3), "{filter}");
    }
    Ok(())
}

/// A literal is not a path: `path(.5)` raises `Invalid path expression with result 0.5` (exit 5) in
/// jq.
#[test]
fn test_leading_dot_literal_is_not_a_path_5299() -> Result<()> {
    let (out, code) = run_jq_stdin("path(.5)", "null", &["-c"])?;
    assert_eq!((out.as_str(), code), ("", 5));
    Ok(())
}
