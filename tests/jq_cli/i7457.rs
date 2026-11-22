//! #7457: a `def`'s parameters are separated by `;` only. jq's grammar has no `,` there
//! (`def f(a,b): a; f(5;6)` is `syntax error, unexpected ','`, exit 3), and `succinctly jq` took
//! the comma for a `;` and ran the program. Outputs below were captured from jq 1.7.1
//! (`/usr/bin/jq`); jq 1.8.2 gives the same exit status for each program (only its syntax-error
//! layout differs, and syntax-error wording is a recorded divergence, #2703, so the refused
//! programs pin the exit status and the fact of refusal).

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn test_comma_between_def_parameters_is_a_syntax_error_7457() -> Result<()> {
    for program in [
        "def f(a,b): 1; 1",
        "def f(a,b): a; f(5;6)",
        "def f($a,$b): $a; f(1;2)",
        "def f(a,$b;c): c; f(1;2;3)",
        "def f(a,b,c;d): a; f(1;2;3;4)",
        "def f(a, b): a; f(5;6)",
        "def f(a ,b): a; f(5;6)",
        "def f(a;b,c): a; 1",
        "def f(a, length): length; f(1;2)",
        "def f(a,b): 1; def g(x;y): x; g(1;2)",
        "def f(a;b): a; def g(x,y): x; 1",
        "1 as $x | def f(a,b): $x; f(1;2)",
        "[1,2]|map(def f(a,b): a; f(.;2))",
        // Other malformed lists were refused already.
        "def f(a,): 1; 1",
        "def f(,): 1; 1",
        "def f(a;): 1; 1",
        "def f(;a): 1; 1",
        "def f(a b): a; 1",
    ] {
        let (out, _err, code) = run_jq_full(&["-nc", program], None)?;
        assert_eq!((out.as_str(), code), ("", 3), "{program}");
    }
    Ok(())
}

#[test]
fn test_semicolon_between_def_parameters_and_comma_in_a_call_still_work_7457() -> Result<()> {
    for (program, expected) in [
        ("def f($a;$b): $a+$b; f(1;2)", "3\n"),
        ("def f(a;b): a+b; f(1;2)", "3\n"),
        ("def f(a;b;c): [a,b,c]; f(1;2;3)", "[1,2,3]\n"),
        ("def f(a; length): length; f(1;2)", "2\n"),
        // The call side: a comma inside an argument is the comma operator.
        ("def f(a): a; f(1,2)", "1\n2\n"),
        ("def f(a): a; [f(1,2)]", "[1,2]\n"),
        ("def f(a;b): a; f(5,6;7)", "5\n6\n"),
    ] {
        let (out, err, code) = run_jq_full(&["-nc", program], None)?;
        assert_eq!((out.as_str(), code), (expected, 0), "{program}: {err}");
    }
    Ok(())
}
