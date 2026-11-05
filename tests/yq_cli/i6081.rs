//! #6081: yq's `eval(expr)` reads each string `expr` yields as a yq expression and evaluates it
//! against the current node, in the context it was written in: `eval(".a")` is `.a`, `eval("")`
//! is the node, `eval(.s)` runs the expression stored in `s`, and a write through it lands
//! (`eval(".a") = 5`). Every row is `yq -I0 -o json FILTER` on [`DOC`], captured from yq v4.53.3.
//! `docs/compliance/yq/limitations.md` records where succinctly answers differently.

use anyhow::Result;

use crate::run_yq_stdin;

const DOC: &str =
    "a: {b: 1}\nc: [1, 2]\ns: \".a.b\"\nt: \"1 + 2\"\nbad: \"((\"\nlist: [\".a\", \".c\"]\n";

/// (filter, stdout), each the same as yq v4.53.3's with exit code 0.
const ROWS: &[(&str, &str)] = &[
    ("eval(\".a\")", "{\"b\":1}\n"),
    (".a | eval(\".b + 1\")", "2\n"),
    ("\"1 + 2\" | eval(.)", "3\n"),
    ("eval(.s)", "1\n"),
    ("eval(.t)", "3\n"),
    ("eval(.list[])", "{\"b\":1}\n[1,2]\n"),
    ("eval(1)", "1\n"),
    ("eval(.zz)", ""),
    ("eval(\"\")", "{\"a\":{\"b\":1},\"c\":[1,2],\"s\":\".a.b\",\"t\":\"1 + 2\",\"bad\":\"((\",\"list\":[\".a\",\".c\"]}\n"),
    ("eval(\".c[]\")", "1\n2\n"),
    ("eval(\".a.b = 5\")", "{\"a\":{\"b\":5},\"c\":[1,2],\"s\":\".a.b\",\"t\":\"1 + 2\",\"bad\":\"((\",\"list\":[\".a\",\".c\"]}\n"),
    (".a |= eval(\".b\")", "{\"a\":1,\"c\":[1,2],\"s\":\".a.b\",\"t\":\"1 + 2\",\"bad\":\"((\",\"list\":[\".a\",\".c\"]}\n"),
    ("eval(\".a\", \".c\")", "{\"b\":1}\n[1,2]\n"),
    ("[eval(\".c[]\")]", "[1,2]\n"),
    ("eval(\"eval(\\\".a\\\")\")", "{\"b\":1}\n"),
    ("eval(\".c | length\")", "2\n"),
    ("eval(\"..\") | length", "6\n1\n1\n2\n1\n1\n4\n5\n2\n2\n2\n2\n"),
    ("eval(\".a\" + \".b\")", "1\n"),
    ("eval(.s + \" + 1\")", "2\n"),
    ("to_entries | .[0].value | eval(\".b\")", "1\n"),
    ("eval(\"1,2\")", "1\n2\n"),
    ("eval(\"# c\")", "{\"a\":{\"b\":1},\"c\":[1,2],\"s\":\".a.b\",\"t\":\"1 + 2\",\"bad\":\"((\",\"list\":[\".a\",\".c\"]}\n"),
    ("eval(\" \")", "{\"a\":{\"b\":1},\"c\":[1,2],\"s\":\".a.b\",\"t\":\"1 + 2\",\"bad\":\"((\",\"list\":[\".a\",\".c\"]}\n"),
    ("eval(\".a | keys\")", "[\"b\"]\n"),
    ("eval(\".a\") = 5", "{\"a\":5,\"c\":[1,2],\"s\":\".a.b\",\"t\":\"1 + 2\",\"bad\":\"((\",\"list\":[\".a\",\".c\"]}\n"),
    ("(eval(\".a\")) |= 7", "{\"a\":7,\"c\":[1,2],\"s\":\".a.b\",\"t\":\"1 + 2\",\"bad\":\"((\",\"list\":[\".a\",\".c\"]}\n"),
    ("del(eval(\".a\"))", "{\"c\":[1,2],\"s\":\".a.b\",\"t\":\"1 + 2\",\"bad\":\"((\",\"list\":[\".a\",\".c\"]}\n"),
    ("(eval(\".c[]\")) += 1", "{\"a\":{\"b\":1},\"c\":[2,3],\"s\":\".a.b\",\"t\":\"1 + 2\",\"bad\":\"((\",\"list\":[\".a\",\".c\"]}\n"),
    ("eval(true)", "true\n"),
    ("eval(1.5)", "1.5\n"),
    ("eval(\".a\") as $x | $x.b", "1\n"),
    ("eval(\"1\") + eval(\"2\")", "3\n"),
    ("eval(.list[]) | length", "1\n2\n"),
    ("eval(select(false))", ""),
    ("eval(.c[0])", "1\n"),
    ("eval(\".a\") | tag", "\"!!map\"\n"),
    ("with(eval(\".a\"); .b = 9)", "{\"a\":{\"b\":9},\"c\":[1,2],\"s\":\".a.b\",\"t\":\"1 + 2\",\"bad\":\"((\",\"list\":[\".a\",\".c\"]}\n"),
    ("(eval(\".a.b\"), eval(\".c[0]\")) = 0", "{\"a\":{\"b\":0},\"c\":[0,2],\"s\":\".a.b\",\"t\":\"1 + 2\",\"bad\":\"((\",\"list\":[\".a\",\".c\"]}\n"),
    ("eval(.list[0]) | keys", "[\"b\"]\n"),
    ("eval(\".nope\") // \"d\"", "\"d\"\n"),
    ("eval(\".c\") | length", "2\n"),    ("del(eval(.zz))", "{\"a\":{\"b\":1},\"c\":[1,2],\"s\":\".a.b\",\"t\":\"1 + 2\",\"bad\":\"((\",\"list\":[\".a\",\".c\"]}\n"),
    ("eval(.zz) = 5", "{\"a\":{\"b\":1},\"c\":[1,2],\"s\":\".a.b\",\"t\":\"1 + 2\",\"bad\":\"((\",\"list\":[\".a\",\".c\"]}\n"),
    ("(eval(.zz)) |= 1", "{\"a\":{\"b\":1},\"c\":[1,2],\"s\":\".a.b\",\"t\":\"1 + 2\",\"bad\":\"((\",\"list\":[\".a\",\".c\"]}\n"),
    ("(eval(.zz), eval(\".a.b\")) = 9", "{\"a\":{\"b\":9},\"c\":[1,2],\"s\":\".a.b\",\"t\":\"1 + 2\",\"bad\":\"((\",\"list\":[\".a\",\".c\"]}\n"),
];

#[test]
fn test_eval_reads_each_string_as_an_expression_6081() -> Result<()> {
    for (filter, expected) in ROWS {
        let (stdout, code) = run_yq_stdin(filter, DOC, &["-I0", "-o", "json"])?;
        assert_eq!((stdout.as_str(), code), (*expected, 0), "{filter}");
    }
    Ok(())
}

/// An expression that does not parse is an error in yq (`bad expression - probably missing
/// close bracket on (`, or the lexer's own message); only the failure is the same here.
#[test]
fn test_eval_of_an_expression_that_does_not_parse_fails_6081() -> Result<()> {
    for filter in ["eval(.bad)", "eval(\"!!str 1\")"] {
        let (stdout, code) = run_yq_stdin(filter, DOC, &["-I0", "-o", "json"])?;
        assert_eq!((stdout.as_str(), code), ("", 1), "{filter}");
    }
    Ok(())
}

/// yq reads an expression stored in the document without limit and runs out of stack on one that
/// evaluates itself (`s: "eval(.s)"`); a bound nobody crosses on purpose stops it here with an
/// error instead of taking the process down.
#[test]
fn test_eval_that_evaluates_itself_stops_at_a_nesting_limit_6081() -> Result<()> {
    let (stdout, code) = run_yq_stdin("eval(.s)", "s: \"eval(.s)\"\n", &["-I0", "-o", "json"])?;
    assert_eq!((stdout.as_str(), code), ("", 1));
    Ok(())
}
