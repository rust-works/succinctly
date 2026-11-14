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

/// `is_key` is `true` for the node `key` returns, which a plain string stands for here, so the
/// answer is read off the stage straight after `key` (`(.a | key) | is_key` too); anywhere else a
/// node is not a key node. Each row is `yq -I0 -o json FILTER` on `a: {b: 1}`, captured from yq
/// (v4.52.2 for these rows; the operator's rule is the same in v4.53.3).
const IS_KEY_ROWS: &[(&str, &str)] = &[
    (".a | key | is_key", "true\n"),
    ("(.a | key) | is_key", "true\n"),
    (".a | key | iskey", "true\n"),
    (".a | key | (is_key)", "true\n"),
    (".a | key | . | is_key", "true\n"),
    (".a | key | is_key and true", "true\n"),
    (".a.b | key | is_key", "true\n"),
    (".a | keys | .[0] | is_key", "false\n"),
    (". | is_key", "false\n"),
    (".a | is_key", "false\n"),
    ("[.a | key | is_key, (.a | is_key)]", "[true,false]\n"),
];

#[test]
fn test_is_key_is_true_straight_after_key_6081() -> Result<()> {
    for (filter, expected) in IS_KEY_ROWS {
        let (stdout, code) = run_yq_stdin(filter, "a: {b: 1}\n", &["-I0", "-o", "json"])?;
        assert_eq!((stdout.as_str(), code), (*expected, 0), "{filter}");
    }
    Ok(())
}

/// A function the program defines under the name is the one called, and jq has no `is_key`.
#[test]
fn test_is_key_defers_to_a_defined_function_and_is_undefined_in_jq_mode_6081() -> Result<()> {
    let (stdout, code) = run_yq_stdin(
        "def is_key: 5; .a | key | is_key",
        "a: {b: 1}\n",
        &["--jq-extensions"],
    )?;
    assert_eq!((stdout.as_str(), code), ("5\n", 0));
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .args(["jq", "-n", "is_key"])
        .output()?;
    assert!(!output.status.success());
    assert!(String::from_utf8(output.stderr)?.contains("is_key/0 is not defined"));
    Ok(())
}

/// Where `is_key` is not the whole stage straight after `key` it is `false` here and `true` in yq
/// (docs/compliance/yq/limitations.md): the divergence is pinned so that a change to it is seen.
#[test]
fn test_is_key_beyond_the_stage_after_key_stays_false_6081() -> Result<()> {
    for filter in [
        ".a | key | [is_key]",
        ".a | key | is_key as $x | $x",
        ".a | key as $k | $k | is_key",
    ] {
        let (stdout, code) = run_yq_stdin(filter, "a: {b: 1}\n", &["-I0", "-o", "json"])?;
        assert_eq!(code, 0, "{filter}");
        assert!(
            stdout.contains("false") && !stdout.contains("true"),
            "{filter}: {stdout}"
        );
    }
    Ok(())
}
