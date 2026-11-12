//! #7025: yq's `--string-interpolation`. With `=false` a `\(` in a string literal is the two
//! characters, in the program and in an `eval(..)` text alike; the backslash pair in front of a `(`
//! is left alone, so `\\(1)` stays two backslashes (a run of n backslashes before `(` reads as
//! `n / 2 + 1` of them, while the other escapes still apply). The flag is a boolean that takes
//! its value after `=` only, in Go's `strconv.ParseBool` spellings. Every row was captured from
//! yq v4.53.3 (`yq FLAG -n --from-file FILE`, the program in the file so no shell touches it).

use anyhow::Result;

use crate::run_yq_stdin;

/// (flag, program, yq's stdout).
const ROWS: &[(&str, &str, &str)] = &[
    (
        r"--string-interpolation=false",
        r#""x\(1 + 2)y""#,
        r"x\(1 + 2)y
",
    ),
    (
        r"--string-interpolation=false",
        r#""\(1)""#,
        r"\(1)
",
    ),
    (
        r"--string-interpolation=false",
        r#""\\(1)""#,
        r"\\(1)
",
    ),
    (
        r"--string-interpolation=false",
        r#""\\\(1)""#,
        r"\\(1)
",
    ),
    (
        r"--string-interpolation=false",
        r#""\\\\(1)""#,
        r"\\\(1)
",
    ),
    (
        r"--string-interpolation=false",
        r#""a\\\\\(1)b""#,
        r"a\\\(1)b
",
    ),
    (
        r"--string-interpolation=false",
        r#""\\\\\\(1)""#,
        r"\\\\(1)
",
    ),
    (
        r"--string-interpolation=false",
        r#""\(1)\(2)""#,
        r"\(1)\(2)
",
    ),
    (
        r"--string-interpolation=false",
        r#""a\nb\(1)\tc\"d""#,
        r#"a
b\(1)	c"d
"#,
    ),
    (
        r"--string-interpolation=false",
        r#""\(1)" | length"#,
        r"4
",
    ),
    (
        r"--string-interpolation=false",
        r#""\(1 + 2)" == "3""#,
        r"false
",
    ),
    (
        r"--string-interpolation=false",
        r#"{"a\(1)": 2}"#,
        r"a\(1): 2
",
    ),
    (
        r"--string-interpolation=false",
        r#""é\(1)\u00e9""#,
        r"é\(1)\u00e9
",
    ),
    (
        r"--string-interpolation=false",
        r#""a" + "\(1)""#,
        r"a\(1)
",
    ),
    (
        r"--string-interpolation=false",
        r#"[1, "\(1)"]"#,
        r"- 1
- \(1)
",
    ),
    (
        r"--string-interpolation=false",
        r#"eval("\"x\\(1)\"")"#,
        r"x\\(1)
",
    ),
    (
        r"--string-interpolation=false",
        r#""abc\(""#,
        r"abc\(
",
    ),
    (
        r"--string-interpolation=true",
        r#""x\(1 + 2)y""#,
        r"x3y
",
    ),
    (
        r"--string-interpolation=0",
        r#""x\(1 + 2)y""#,
        r"x\(1 + 2)y
",
    ),
    (
        r"--string-interpolation=F",
        r#""x\(1 + 2)y""#,
        r"x\(1 + 2)y
",
    ),
    (
        r"--string-interpolation=T",
        r#""x\(1 + 2)y""#,
        r"x3y
",
    ),
    (
        r"",
        r#""x\(1 + 2)y""#,
        r"x3y
",
    ),
];

#[test]
fn the_flag_decides_whether_a_backslash_paren_interpolates_7025() -> Result<()> {
    for &(flag, program, expected) in ROWS {
        let mut args = vec!["-n"];
        if !flag.is_empty() {
            args.push(flag);
        }
        let (out, code) = run_yq_stdin(program, "", &args)?;
        assert_eq!((out.as_str(), code), (expected, 0), "{flag} {program}");
    }
    Ok(())
}

/// A bare `--string-interpolation` is `=true`, and a later flag wins over an earlier one.
#[test]
fn a_bare_flag_is_true_and_the_last_one_wins_7025() -> Result<()> {
    let program = r#""x\(1 + 2)y""#;
    for (args, expected) in [
        (vec!["-n", "--string-interpolation"], "x3y\n"),
        (
            vec![
                "-n",
                "--string-interpolation=false",
                "--string-interpolation=true",
            ],
            "x3y\n",
        ),
        (
            vec![
                "-n",
                "--string-interpolation=true",
                "--string-interpolation=false",
            ],
            "x\\(1 + 2)y\n",
        ),
    ] {
        let (out, code) = run_yq_stdin(program, "", &args)?;
        assert_eq!((out.as_str(), code), (expected, 0), "{args:?}");
    }
    Ok(())
}

/// `--verbose` takes a value after `=` the same way: `=false` is the default, and standard output
/// is the same with it off. (`=true` makes yq write its debug log to stderr, which `succinctly yq`
/// does not.)
#[test]
fn verbose_takes_a_boolean_value_after_an_equals_sign_7025() -> Result<()> {
    for flag in ["--verbose=false", "-v=false", "--verbose=0", "--verbose=F"] {
        let (out, code) = run_yq_stdin(".a", "a: 1\n", &[flag])?;
        assert_eq!((out.as_str(), code), ("1\n", 0), "{flag}");
    }
    Ok(())
}

/// A value Go's `strconv.ParseBool` rejects is refused at the flag, as in yq (which exits 1 where
/// `succinctly yq` exits 2, like every other flag clap refuses).
#[test]
fn a_value_that_is_not_a_boolean_is_refused_7025() -> Result<()> {
    for flag in [
        "--string-interpolation=maybe",
        "--verbose=zz",
        "--string-interpolation=",
    ] {
        let (_, code) = run_yq_stdin(".a", "a: 1\n", &[flag])?;
        assert_ne!(code, 0, "{flag}");
    }
    Ok(())
}
