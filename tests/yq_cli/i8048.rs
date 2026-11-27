//! #8048: yq has no unary minus. Its `-` is `Subtract` alone (`operation.go`, v4.53.3), so a `-`
//! that is not the sign of a number token (#7998) is a subtraction with no left operand. yq's
//! tree builder (`createExpressionTree`, `expression_parser.go`) works from a postfix stack, so
//! that `-` pops the operand just before it, every enclosing operator pops one further left, and
//! the outermost one runs out: `'<op>' expects 2 args but there is 1`, exit 1, where `<op>` is
//! the outermost operator whose right side holds the `-`, or the `-` itself when nothing stands
//! before it. Every row is a live yq v4.53.3 capture over `a: 1` / `b: 2`. `--jq-extensions` does
//! not change it: it is yq's tree builder that refuses, not its lexer. `succinctly jq` keeps jq's
//! unary minus.

use anyhow::Result;

use crate::{run_jq_stdin_with_stderr, run_yq_stdin, run_yq_stdin_with_stderr};

const DOC: &str = "a: 1\nb: 2\n";

/// The filters yq refuses, with the operator its message names.
const REFUSED: &[(&str, &str)] = &[
    ("-(1)", "-"),
    ("-.a", "-"),
    ("-.", "-"),
    ("- 1", "-"),
    ("- 1.5", "-"),
    ("--1", "-"),
    ("- -1", "-"),
    ("-.5", "-"),
    ("-$x", "-"),
    ("-\"x\"", "-"),
    ("-[1]", "-"),
    ("-{}", "-"),
    ("-null", "-"),
    ("-true", "-"),
    ("-(.a)", "-"),
    ("(-.a)", "-"),
    ("[-.a]", "-"),
    ("{-.a: 1}", "-"),
    ("-.a | .b", "-"),
    ("-.a, .b", "-"),
    ("-.a as $x | $x", "-"),
    ("select(-.a > 0)", "-"),
    ("test(-.a)", "-"),
    ("del(-.a)", "-"),
    ("with(-.a; .b)", "-"),
    ("\"x\\(-.a)\"", "-"),
    (".a | \"x\\(-.b)\"", "-"),
    ("1 - -.a", "-"),
    ("1 - (-.a)", "-"),
    (".a - - 1", "-"),
    (".a.b - -.c", "-"),
    (".[\"x\"] - -.a", "-"),
    ("1 | -.", "|"),
    (".a | (-.b)", "|"),
    (".a | [-.b]", "|"),
    (".a | {\"x\": -.b}", "|"),
    (".a | {\"x\": 1, \"y\": -.b}", "|"),
    (".a | select(-.b)", "|"),
    (".a | select(.b == -.c)", "|"),
    (".a | with(.b; -.c)", "|"),
    (".a | .b * -.c", "|"),
    (".a | . * -.b", "|"),
    (".a | .b - -.c", "|"),
    (".a | .[-.b]", "|"),
    (".a.b | .[-.c]", "|"),
    (".a as $x | -$x", "|"),
    (".a as $x | $x - -.b", "|"),
    (".a as $x | (1, -.a)", "|"),
    (".a as $x | $x[-.a]", "|"),
    (".a + .b | -.c", "|"),
    (".a |= .b | -.c", "|"),
    ("[.a | -.b]", "|"),
    ("(1, -.a)", ","),
    (".a, -.b", ","),
    (".a, -.b, .c", ","),
    (".a, .b | -.c", ","),
    (".a | .b, -.c", ","),
    ("[.b, -.a]", ","),
    ("[.a, .b | -.c]", ","),
    ("{\"a\": 1, \"b\": -.c}", ","),
    ("{\"a\": .b, \"c\": .d | -.e}", ","),
    ("{\"a\": -.b}", ":"),
    ("{\"a\": (.b | -.c)}", ":"),
    ("{.a: -.b}", ":"),
    ("{\"x\": [-.a]}", ":"),
    (".a and -.b", "and"),
    (".a and (.b or -.c)", "and"),
    (".a and .b or -.c", "and"),
    (".a == .b and -.c", "and"),
    (".a or -.b", "or"),
    (".a // -.b", "//"),
    (".a // (-.b)", "//"),
    (".a // .b // -.c", "//"),
    ("1 + -.a", "+"),
    ("(1 + -.a) | .b", "+"),
    (".a * -.b", "*"),
    (".a * .b + -.c", "*"),
    (".a *+ -.b", "*+"),
    (".a == -.b", "=="),
    (".a != -.b", "!="),
    (".a < -.b", "<"),
    (".a >= -.b", ">="),
    ("select(.a == -.b)", "=="),
    (".a = -.b", "="),
    (".a = .b // -.c", "="),
    (".a |= -.", "|="),
    (".a |= (. + -.b)", "|="),
    (".a += -.b", "+="),
    (".a -= -.b", "-="),
    (".a *= -.b", "*="),
    (".a *=+ -.b", "*=+"),
    ("with(.a; -.b)", ";"),
    ("with(.a; .b | -.c)", ";"),
    ("sub(\"a\"; -.b)", ";"),
    ("sub(\"a\"; \"b\"; -.c)", ";"),
    ("sub(\"a\"; \"b\"; -.c; 1)", ";"),
    ("test(\"a\"; -.b)", ";"),
    ("capture(\"a\"; -.b)", ";"),
    ("match(\"a\"; -.b)", ";"),
    ("with(.a; .b; -.c)", ";"),
    ("with_dtf(\"x\"; -.a)", ";"),
    (".a | sub(\"a\"; -.b)", "|"),
    ("[.a, sub(\"a\"; -.b)]", ","),
    (".[-.a]", "TRAVERSE_ARRAY"),
    (".[-.a][0]", "TRAVERSE_ARRAY"),
    (".[1:-.a]", "TRAVERSE_ARRAY"),
    (".a[-.b]", ""),
    (".a[-.b].c", ""),
    (".a[1:-.b]", ""),
    (".a[-.b:1]", ""),
    (".a[-.b:]", ""),
    (".a[:-.b]", ""),
    (".a[\"x\"][-.b]", ""),
    (".a[0][-.b]", ""),
    (".[0][-.a]", ""),
    (".a[.b[-.c]]", ""),
    ("$x[-.a]", ""),
    ("[1][-.a]", ""),
    ("(.a)[-.b]", ""),
    (".a.b[-.c]", "."),
    (".a.b.c[-.d]", "."),
];

#[test]
fn test_minus_with_no_left_operand_is_refused_8048() -> Result<()> {
    for &(filter, op) in REFUSED {
        let want = format!("'{op}' expects 2 args but there is 1");
        for extensions in [false, true] {
            let mut args = vec!["-o=json", "-I0"];
            if extensions {
                args.push("--jq-extensions");
            }
            args.push("--");
            let (out, err, code) = run_yq_stdin_with_stderr(filter, DOC, &args)?;
            assert_eq!((out.as_str(), code), ("", 1), "{filter:?} {args:?}: {err}");
            assert!(
                err.contains(&want),
                "{filter:?} {args:?}: want {want:?}, got {err}"
            );
        }
    }
    Ok(())
}

/// `eval` parses its text on its own, so yq names an operator inside the text (`'-'`, or `'|'` for
/// `eval("1 | -.")`). `eval` here reports every parse error of its text as `bad expression, please
/// check expression syntax`, so only the refusal is pinned.
#[test]
fn test_minus_with_no_left_operand_in_eval_text_is_refused_8048() -> Result<()> {
    for filter in ["eval(\"-.a\")", ".a | eval(\"-.b\")", "eval(\"1 | -.\")"] {
        let (out, err, code) = run_yq_stdin_with_stderr(filter, DOC, &["-o=json", "-I0", "--"])?;
        assert_eq!((out.as_str(), code), ("", 1), "{filter:?}: {err}");
    }
    Ok(())
}

/// A `-` directly followed by a digit is the sign of a number token (#7998), wherever a value may
/// start, and a `-` after a value is `Subtract`: neither changes.
#[test]
fn test_negative_numbers_and_subtraction_unchanged_8048() -> Result<()> {
    for (filter, want) in [
        ("-1", "-1"),
        ("-1.5", "-1.5"),
        ("-1e3 | tostring", "\"-1e3\""),
        ("-0 | tostring", "\"-0\""),
        (".a | -1", "-1"),
        (". | -1", "-1"),
        ("1 - -1", "2"),
        (".a - -1", "2"),
        ("-1 - .a", "-2"),
        (".a * -1", "-1"),
        (".a, -1", "1\n-1"),
        ("[.a, -1]", "[1,-1]"),
        ("{\"x\": -1}", "{\"x\":-1}"),
        (".a == -1", "false"),
        ("[1, 2] | .[-1]", "2"),
        (".a -.b", "-1"),
        ("1 -.a", "0"),
        (".a - 1", "0"),
        (".a -(1)", "0"),
    ] {
        let (out, code) = run_yq_stdin(filter, DOC, &["-o=json", "-I0", "--"])?;
        assert_eq!((out.trim_end(), code), (want, 0), "{filter:?}");
    }
    Ok(())
}

/// jq has a unary minus (`'-' Exp`), and `succinctly jq` keeps it (jq 1.7.1).
#[test]
fn test_jq_mode_keeps_unary_minus_8048() -> Result<()> {
    for (filter, want) in [
        ("-(1)", "-1"),
        ("-.a", "-1"),
        ("1 | -.", "-1"),
        ("[-.a]", "[-1]"),
        ("{\"x\": -.b}", "{\"x\":-2}"),
        ("1 - -.a", "2"),
        (".a, -.b", "1\n-2"),
    ] {
        let (out, err, code) = run_jq_stdin_with_stderr(filter, "{\"a\":1,\"b\":2}", &["-c"])?;
        assert_eq!((out.trim_end(), code), (want, 0), "{filter:?}: {err}");
    }
    Ok(())
}
