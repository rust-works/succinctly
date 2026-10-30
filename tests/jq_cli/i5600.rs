//! #5600: jq's lexer reads `a::b` as one `IDENT` wherever an identifier is allowed. #5590 taught
//! the `def` name and a call; this covers an unquoted object key (and the key-only shorthand), the
//! `{a::b: $x}` destructuring key and a closure or `$` parameter. Every expected byte string was
//! captured from the pinned jq 1.7.1 source build (`scripts/compare-jq-build.sh 1.7.1`).

use anyhow::Result;

use crate::run_jq_full;

fn run(filter: &str) -> Result<(String, String, i32)> {
    run_jq_full(&["-c", filter], Some(r#"{"a":1}"#))
}

#[test]
fn test_qualified_name_is_an_unquoted_object_key_5600() -> Result<()> {
    for (program, expected) in [
        ("{a::b: 1}", "{\"a::b\":1}\n"),
        ("{a::b}", "{\"a::b\":null}\n"),
        ("{a::b::c: 2}", "{\"a::b::c\":2}\n"),
        ("{a::b::c}", "{\"a::b::c\":null}\n"),
        ("{a::b: 1, c::d: 2}", "{\"a::b\":1,\"c::d\":2}\n"),
        // A reserved word before `::` is part of the identifier, not a keyword.
        ("{if::x: 1}", "{\"if::x\":1}\n"),
        ("def a::b: 5; {a::b:1}", "{\"a::b\":1}\n"),
    ] {
        let (out, err, code) = run(program)?;
        assert_eq!((out.as_str(), code), (expected, 0), "{program}: {err}");
    }
    Ok(())
}

#[test]
fn test_qualified_name_is_a_parameter_name_5600() -> Result<()> {
    for (program, expected) in [
        ("def f(a::b): a::b; f(1)", "1\n"),
        ("def f(a::b::c): a::b::c; f(3)", "3\n"),
        ("def f($a::b): $a::b; f(1)", "1\n"),
        ("def f(if::x): 1; f(2)", "1\n"),
        ("def f(a::b; c): a::b + c; f(1;2)", "3\n"),
    ] {
        let (out, err, code) = run(program)?;
        assert_eq!((out.as_str(), code), (expected, 0), "{program}: {err}");
    }
    Ok(())
}

#[test]
fn test_qualified_name_is_a_destructuring_key_5600() -> Result<()> {
    let (out, err, code) = run(". as {a::b: $x} | $x")?;
    assert_eq!((out.as_str(), code), ("null\n", 0), "{err}");
    // The key-only shorthand has no `$name` to bind, in jq as before.
    let (_, _, code) = run(". as {a::b} | 1")?;
    assert_eq!(code, 3);
    Ok(())
}

/// A `label`/`break` name is a `BINDING`, so `$a::b` is one name there too.
#[test]
fn test_qualified_name_is_a_label_name_5600() -> Result<()> {
    for (program, expected) in [
        ("label $a::b | 1, break $a::b", "1\n"),
        ("label $a::b | break $a::b", ""),
        ("label $a::b | (1, break $a::b, 2)", "1\n"),
    ] {
        let (out, err, code) = run(program)?;
        assert_eq!((out.as_str(), code), (expected, 0), "{program}: {err}");
    }
    Ok(())
}

/// A qualified key binds in every pattern context, and a parameter shadows a `def` of the same name.
#[test]
fn test_qualified_name_in_pattern_contexts_and_shadowing_5600() -> Result<()> {
    for (program, expected) in [
        ("[{\"a::b\":3}] | .[] as {a::b: $v} | $v", "3\n"),
        (
            "reduce ({\"a::b\":1},{\"a::b\":2}) as {a::b: $x} (0; .+$x)",
            "3\n",
        ),
        (". as [$a] ?// {a::b: $x} | [$a,$x]", "[null,null]\n"),
        ("def a::b: 7; def f(a::b): a::b; f(1)", "1\n"),
        ("def f(a::b; a::b): a::b; f(1;2)", "2\n"),
        ("def f(x::y): x::y; f(1,2)", "1\n2\n"),
    ] {
        let (out, err, code) = run(program)?;
        assert_eq!((out.as_str(), code), (expected, 0), "{program}: {err}");
    }
    Ok(())
}

/// The `::` takes no whitespace and must be followed by an identifier; each of these is a compile
/// error in jq 1.7.1 (exit 3) and stays one here.
#[test]
fn test_malformed_qualified_names_still_fail_5600() -> Result<()> {
    for program in [
        "{a::: 1}",
        "{a::1: 1}",
        "{a:: 1}",
        "{a ::b: 1}",
        "def f(a:: b): 1; f(1)",
        "def f(a::b): 1; f",
    ] {
        let (out, _, code) = run(program)?;
        assert_eq!((out.as_str(), code), ("", 3), "{program}");
    }
    Ok(())
}
