//! #8041: yq's flag parser (cobra/pflag, v4.53.3) gives a string-valued flag the next argument
//! whatever it starts with, so `--expression -1` is the expression `-1` and `--from-file -f.yq`
//! names a file that starts with a hyphen. clap read the hyphen word as an unknown flag and
//! refused the command line (exit 2). Every row is a live yq v4.53.3 capture; a bare `-1` that is
//! no flag's value is still refused by both (`unknown shorthand flag: '1' in -1`).

use std::process::{Command, Stdio};

use anyhow::Result;
use tempfile::TempDir;

/// Run `succinctly yq ARGS` with the current directory `dir`, no stdin.
fn run(dir: &TempDir, args: &[&str]) -> Result<(String, String, i32)> {
    let output = Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .arg("yq")
        .args(args)
        .current_dir(dir.path())
        .stdin(Stdio::null())
        .output()?;
    Ok((
        String::from_utf8(output.stdout)?,
        String::from_utf8(output.stderr)?,
        output.status.code().unwrap_or(-1),
    ))
}

fn dir_with(files: &[(&str, &str)]) -> Result<TempDir> {
    let dir = TempDir::new()?;
    for (name, text) in files {
        std::fs::write(dir.path().join(name), text)?;
    }
    Ok(dir)
}

#[test]
fn test_expression_takes_a_hyphen_value_8041() -> Result<()> {
    let dir = dir_with(&[("in.yaml", "a: 1\n")])?;
    // (args, stdout): yq prints `-1` for each.
    for args in [
        &["-n", "--expression", "-1"][..],
        &["-n", "--expression=-1"],
        &["-n", "--expression", "-1", "-e"],
        &["-n", "--expression", "-1", "--"],
        &["--expression", "-1", "in.yaml"],
        &["-n", "--", "-1"],
    ] {
        let (out, err, code) = run(&dir, args)?;
        assert_eq!((out.as_str(), code), ("-1\n", 0), "{args:?}: {err}");
    }
    // The last --expression wins.
    let (out, _, code) = run(&dir, &["-n", "--expression", "-1", "--expression", "-2"])?;
    assert_eq!((out.as_str(), code), ("-2\n", 0));
    let (out, _, code) = run(&dir, &["-n", "--expression", "-1", "--expression", ".a"])?;
    assert_eq!((out.as_str(), code), ("null\n", 0));
    Ok(())
}

#[test]
fn test_expression_hyphen_value_is_parsed_as_an_expression_8041() -> Result<()> {
    let dir = dir_with(&[("in.yaml", "a: 1\n")])?;
    // A hyphen word after `--expression` is the expression, even when it looks like a flag.
    for (args, message) in [
        (
            &["-n", "--expression", "-.a"][..],
            "Error: '-' expects 2 args but there is 1",
        ),
        (
            &["--expression=-.a", "in.yaml"],
            "Error: '-' expects 2 args but there is 1",
        ),
        (
            &["-n", "--expression", "-"],
            "Error: '-' expects 2 args but there is 0",
        ),
        (
            &["-n", "--expression", "--help"],
            "Error: 1:3: lexer: invalid input text \"help\"",
        ),
        (
            &["-n", "--expression", "-e"],
            "Error: 1:2: lexer: invalid input text \"e\"",
        ),
        (
            &["-n", "--expression", "--indent"],
            "Error: 1:3: lexer: invalid input text \"indent\"",
        ),
        (
            &["-n", "--expression", "-1 | abs"],
            "Error: 1:6: lexer: invalid input text \"abs\"",
        ),
    ] {
        let (out, err, code) = run(&dir, args)?;
        assert_eq!((out.as_str(), code), ("", 1), "{args:?}: {err}");
        assert!(err.contains(message), "{args:?}: {err}");
    }
    Ok(())
}

#[test]
fn test_bare_hyphen_digit_is_still_refused_8041() -> Result<()> {
    let dir = dir_with(&[("in.yaml", "a: 1\n")])?;
    // No flag takes the word, so it is a flag: both tools refuse (yq exit 1, clap exit 2).
    for args in [
        &["-n", "-1"][..],
        &["-n", "-e", "-1"],
        &[".", "in.yaml", "-1"],
    ] {
        let (out, err, code) = run(&dir, args)?;
        assert_eq!(out, "", "{args:?}");
        assert_ne!(code, 0, "{args:?}: {err}");
        assert!(err.contains("-1"), "{args:?}: {err}");
    }
    Ok(())
}

#[test]
fn test_from_file_and_split_exp_file_take_hyphen_names_8041() -> Result<()> {
    let dir = dir_with(&[("in.yaml", "a: 1\n"), ("-f2.yq", ".a\n")])?;
    let (out, err, code) = run(&dir, &["--from-file", "-f2.yq", "in.yaml"])?;
    assert_eq!((out.as_str(), code), ("1\n", 0), "{err}");
    let (out, err, code) = run(&dir, &["--from-file=-f2.yq", "in.yaml"])?;
    assert_eq!((out.as_str(), code), ("1\n", 0), "{err}");

    // A missing file is the file error of the name, not an argument error.
    for (args, message) in [
        (
            &["-n", "--from-file", "-1"][..],
            "Error: open -1: no such file or directory",
        ),
        (
            &["--split-exp-file", "-x", ".", "in.yaml"],
            "Error: open -x: no such file or directory",
        ),
    ] {
        let (out, err, code) = run(&dir, args)?;
        assert_eq!((out.as_str(), code), ("", 1), "{args:?}: {err}");
        assert!(err.contains(message), "{args:?}: {err}");
    }

    let dir = dir_with(&[
        ("in.yaml", "a: 1\n"),
        ("-sp.yq", "\"out_\" + ($index|tostring) + \".yml\"\n"),
    ])?;
    let (out, err, code) = run(&dir, &["--split-exp-file", "-sp.yq", ".", "in.yaml"])?;
    assert_eq!((out.as_str(), code), ("", 0), "{err}");
    assert_eq!(
        std::fs::read_to_string(dir.path().join("out_0.yml"))?,
        "a: 1\n"
    );
    Ok(())
}

#[test]
fn test_split_exp_takes_a_hyphen_value_8041() -> Result<()> {
    let dir = dir_with(&[("in.yaml", "a: 1\n")])?;
    for args in [
        &["--split-exp", "-.a", ".", "in.yaml"][..],
        &["--split-exp=-.a", ".", "in.yaml"],
    ] {
        let (out, err, code) = run(&dir, args)?;
        assert_eq!((out.as_str(), code), ("", 1), "{args:?}: {err}");
        assert!(
            err.contains("bad split document expression: '-' expects 2 args but there is 1"),
            "{args:?}: {err}"
        );
    }
    // `-1` evaluates to the number -1, which names the file `-1.yml`.
    let (out, err, code) = run(&dir, &["--split-exp", "-1", ".", "in.yaml"])?;
    assert_eq!((out.as_str(), code), ("", 0), "{err}");
    assert_eq!(
        std::fs::read_to_string(dir.path().join("-1.yml"))?,
        "a: 1\n"
    );
    Ok(())
}

#[test]
fn test_string_valued_output_flags_take_hyphen_values_8041() -> Result<()> {
    let dir = dir_with(&[
        ("in.yaml", "a: 1\n"),
        ("att.yaml", "a:\n  -xb: 1\n  c: 2\n"),
        ("nest.yaml", "a:\n  b:\n    c: 1\n"),
    ])?;
    for (args, expected) in [
        (
            &[
                "-o",
                "props",
                "--properties-separator",
                "-x",
                ".",
                "in.yaml",
            ][..],
            "a-x1\n",
        ),
        (
            &["-o", "props", "--properties-separator=-x", ".", "in.yaml"],
            "a-x1\n",
        ),
        (
            &[
                "-o",
                "shell",
                "--shell-key-separator",
                "-x",
                ".",
                "nest.yaml",
            ],
            "a-xb-xc=1\n",
        ),
        (
            &["-o", "lua", "--lua-prefix", "-x", ".", "in.yaml"],
            "-x{\n\t[\"a\"] = 1;\n};\n",
        ),
        (
            &["-o", "lua", "--lua-suffix", "-x", ".", "in.yaml"],
            "return {\n\t[\"a\"] = 1;\n}-x",
        ),
        (
            &["-o", "xml", "--xml-attribute-prefix", "-x", ".", "att.yaml"],
            "<a b=\"1\">\n  <c>2</c>\n</a>\n",
        ),
        (
            &["-o", "xml", "--xml-content-name", "-x", ".", "in.yaml"],
            "<a>1</a>\n",
        ),
        (
            &["-o", "xml", "--xml-proc-inst-prefix", "-x", ".", "in.yaml"],
            "<a>1</a>\n",
        ),
        (
            &["-o", "xml", "--xml-directive-name", "-x", ".", "in.yaml"],
            "<a>1</a>\n",
        ),
    ] {
        let (out, err, code) = run(&dir, args)?;
        assert_eq!((out.as_str(), code), (expected, 0), "{args:?}: {err}");
    }
    Ok(())
}
