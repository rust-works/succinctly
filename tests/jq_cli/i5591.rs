//! #5591: `succinctly jq` ran a program with no expression of its own as the identity, and took a
//! reserved word as a `def` name, a bare `def` parameter or an `import ... as` namespace. jq 1.7.1
//! refuses all of them with exit 3. Every row was captured from jq 1.7.1 (`/usr/bin/jq`).

use anyhow::Result;

use crate::run_jq_full;

/// jq's whole answer to a program with nothing to run, byte for byte, and it is the only
/// diagnostic: a module the program names is never looked up.
const NO_PROGRAM: &str =
    "jq: error: Top-level program not given (try \".\")\njq: 1 compile error\n";

#[test]
fn test_a_program_with_no_expression_is_a_compile_error_like_jq_5591() -> Result<()> {
    for program in [
        "",
        "# nothing",
        "  \n\t ",
        "\n\n",
        "def f: 1;",
        "def f: 1; def g: 2;",
        "def f: 1;\n# trailing comment",
        "module {a: 1};",
        "include \"missing\";",
        "import \"missing\" as m;",
        "import \"missing\" as $d;",
        "include \"missing\"; def f: 1;",
    ] {
        for flags in [&["-c"][..], &["-nc"], &["-sc"]] {
            let mut args = flags.to_vec();
            args.push(program);
            let (stdout, stderr, code) = run_jq_full(&args, Some("{\"a\":1}"))?;
            assert_eq!(
                (stdout.as_str(), stderr.as_str(), code),
                ("", NO_PROGRAM, 3),
                "{flags:?} {program:?}"
            );
        }
    }
    Ok(())
}

#[test]
fn test_a_program_with_an_expression_still_runs_5591() -> Result<()> {
    for (program, expected) in [
        (".", "{\"a\":1}"),
        ("def f: 1; f", "1"),
        ("module {a: 1}; 1", "1"),
        ("def not: 1; 2", "2"),
        ("def __loc__: 1; 2", "2"),
        ("def f($if): $if; f(3)", "3"),
        ("1 as $if | $if", "1"),
        ("label $if | 1", "1"),
        ("{if: 1} | .if", "1"),
        ("{\"a\":1} | {if: .a}", "{\"if\":1}"),
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-c", program], Some("{\"a\":1}"))?;
        assert_eq!(
            (stdout.trim_end(), code),
            (expected, 0),
            "{program}: stderr {stderr}"
        );
    }
    Ok(())
}

/// The words jq's lexer turns into keywords, which its grammar will not take for an `IDENT`.
const RESERVED: [&str; 17] = [
    "def", "module", "import", "include", "if", "then", "else", "elif", "reduce", "foreach", "end",
    "and", "or", "as", "label", "try", "catch",
];

#[test]
fn test_a_reserved_word_is_not_a_def_name_a_parameter_or_a_namespace_5591() -> Result<()> {
    for word in RESERVED {
        for (program, expecting) in [
            (format!("def {word}: 1; 2"), "IDENT"),
            (format!("def f({word}): 1; 2"), "IDENT or BINDING"),
            (format!("def f(a; {word}): 1; 2"), "IDENT or BINDING"),
            (format!("import \"m\" as {word}; 1"), "IDENT or BINDING"),
        ] {
            let (stdout, stderr, code) = run_jq_full(&["-nc", &program], None)?;
            assert_eq!((stdout.as_str(), code), ("", 3), "{program}: {stderr}");
            // jq appends `(Unix shell quoting issues?)` and a source echo; the wording up to
            // there is its own, and is what a script greps for.
            let wanted =
                format!("jq: error: syntax error, unexpected {word}, expecting {expecting}");
            assert!(stderr.starts_with(&wanted), "{program}: {stderr}");
            assert!(
                stderr.ends_with("jq: 1 compile error\n"),
                "{program}: {stderr}"
            );
        }
    }
    Ok(())
}

/// A `$name` is a different token and takes any word, so a reserved one binds as a variable.
#[test]
fn test_a_reserved_word_is_a_variable_or_a_data_namespace_5591() -> Result<()> {
    for word in RESERVED {
        let (stdout, stderr, code) =
            run_jq_full(&["-nc", &format!("2 as ${word} | ${word}")], None)?;
        assert_eq!((stdout.trim_end(), code), ("2", 0), "{word}: {stderr}");
        let (stdout, stderr, code) =
            run_jq_full(&["-nc", &format!("def f(${word}): ${word}; f(3)")], None)?;
        assert_eq!((stdout.trim_end(), code), ("3", 0), "{word}: {stderr}");
        // `import ... as $word` gets as far as looking the module up, as jq does.
        let (_, stderr, code) =
            run_jq_full(&["-nc", &format!("import \"missing\" as ${word}; 1")], None)?;
        assert_eq!(code, 3, "{word}: {stderr}");
        assert!(
            stderr.contains("module not found: missing"),
            "{word}: {stderr}"
        );
    }
    Ok(())
}
