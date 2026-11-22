//! #5351: a library -- a module, or the `~/.jq` file -- that does not parse, or that holds an
//! expression of its own after its definitions, is a compile error (exit 3) for every program,
//! whether or not the program uses it. Before, `~/.jq` was dropped without a word and a module's
//! main expression was ignored. The main-expression text and the counts are jq 1.7.1's, captured
//! live; a syntax error's *wording* is succinctly's own (the fidelity limit the module reports
//! already document), so those assertions stop at the location and the trailer.

use anyhow::Result;

use std::process::Command;

use crate::{spawn_with_signal_retry, succinctly_bin};

const MAIN_EXPRESSION: &str =
    "jq: error: jq: error: library should only have function definitions, not a main expression\n<unknown location>\n";

/// `succinctly jq -nc <filter>` in a scratch directory holding `m.jq` (`def a: "cwd";`), with
/// `home/.jq` holding `dot_jq` (no `.jq` at all when `None`) and the files in `modules`.
fn run(
    dot_jq: Option<&str>,
    modules: &[(&str, &str)],
    filter: &str,
) -> Result<(String, String, i32)> {
    let dir = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    std::fs::write(dir.path().join("m.jq"), "def a: \"cwd\";")?;
    for (name, text) in modules {
        std::fs::write(dir.path().join(name), text)?;
    }
    if let Some(text) = dot_jq {
        std::fs::write(home.path().join(".jq"), text)?;
    }
    let (output, code) = spawn_with_signal_retry(
        || {
            let mut command = Command::new(succinctly_bin());
            command
                .current_dir(dir.path())
                .env("HOME", home.path())
                .args(["jq", "-nc", filter]);
            command
        },
        None,
    )?;
    let home_path = std::fs::canonicalize(home.path())?;
    let stderr = String::from_utf8(output.stderr)?
        .replace(&home_path.display().to_string(), "HOME")
        .replace(&dir.path().display().to_string(), "DIR");
    Ok((String::from_utf8(output.stdout)?, stderr, code))
}

#[test]
fn test_dot_jq_with_a_main_expression_is_a_compile_error_5351() -> Result<()> {
    for dot_jq in [
        "def hb: 1; hb\n",
        "5\n",
        ".\n",
        "def f: 1;\n.\n",
        // jq checks right after parsing, so its directives are not read.
        "import \"nonexist\" as x; def hb: 1; hb\n",
        "import \"m\" as hm; def hb: hm::a; hb\n",
    ] {
        // Raised whether or not the filter uses the file.
        let (stdout, stderr, code) = run(Some(dot_jq), &[], "2")?;
        assert_eq!(
            (stdout.as_str(), stderr.as_str(), code),
            (
                "",
                format!("{MAIN_EXPRESSION}jq: 1 compile error\n").as_str(),
                3
            ),
            "{dot_jq:?}"
        );
    }
    Ok(())
}

#[test]
fn test_dot_jq_that_does_not_parse_is_a_compile_error_5351() -> Result<()> {
    for dot_jq in [
        "def (",
        "def hb: 1 +;\n",
        "def f: 1;; def g: 2;\n",
        "def hb: 1; import \"m\" as hm;\n",
        // Its directives are not read once it does not parse.
        "import \"nonexist\" as x;\ndef (",
    ] {
        let (stdout, stderr, code) = run(Some(dot_jq), &[], "1")?;
        assert_eq!((stdout.as_str(), code), ("", 3), "{dot_jq:?}: {stderr}");
        assert!(
            stderr.starts_with("jq: error: ") && stderr.contains(" at HOME/.jq, line "),
            "{dot_jq:?}: {stderr}"
        );
        assert!(
            stderr.ends_with("jq: 1 compile error\n") && !stderr.contains("module not found"),
            "{dot_jq:?}: {stderr}"
        );
    }
    Ok(())
}

#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object literals read like format arguments
fn test_dot_jq_without_a_fault_is_untouched_5351() -> Result<()> {
    // Nothing, blanks, a comment, a module header and plain defs are all libraries.
    for dot_jq in [
        "",
        "   \n",
        "# only a comment\n",
        "module {a:1};\n",
        "module {a:1}; def hb: 1;\n",
        "def hb: 1;\n",
        // ... and a def that calls nothing defined is only an error where it is used.
        "def hb: nosuch;\n",
    ] {
        let (stdout, stderr, code) = run(Some(dot_jq), &[], "1")?;
        assert_eq!((stdout.as_str(), code), ("1\n", 0), "{dot_jq:?}: {stderr}");
    }
    let (stdout, stderr, code) = run(None, &[], "1")?;
    assert_eq!((stdout.as_str(), code), ("1\n", 0), "{stderr}");
    Ok(())
}

/// jq parses a library's raw bytes: a byte that is not UTF-8 is harmless in a comment, reads as
/// U+FFFD in a string, and is a syntax error anywhere else.
#[test]
fn test_dot_jq_is_read_as_bytes_5351() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    for (bytes, want) in [
        (&b"# caf\xe9 comment\ndef hb: 1;\n"[..], Some("1\n")),
        (&b"def hb: \"caf\xe9\";\n"[..], Some("\"caf\u{fffd}\"\n")),
        (&b"\xff\xfe def hb: 1;\n"[..], None),
    ] {
        std::fs::write(home.path().join(".jq"), bytes)?;
        let (output, code) = spawn_with_signal_retry(
            || {
                let mut command = Command::new(succinctly_bin());
                command
                    .current_dir(dir.path())
                    .env("HOME", home.path())
                    .args(["jq", "-nc", "hb"]);
                command
            },
            None,
        )?;
        match want {
            Some(want) => {
                assert_eq!(
                    (output.stdout.as_slice(), code),
                    (want.as_bytes(), 0),
                    "{bytes:?}"
                );
            }
            None => {
                let stderr = String::from_utf8(output.stderr)?;
                assert_eq!(code, 3, "{bytes:?}: {stderr}");
                assert!(stderr.ends_with("jq: 1 compile error\n"), "{stderr}");
                assert!(stderr.contains("/.jq, line 1:"), "{stderr}");
            }
        }
    }
    Ok(())
}

#[test]
fn test_dot_jq_fault_is_reported_after_the_programs_own_directives_5351() -> Result<()> {
    // The file is declared ahead of the program, so jq reaches it last, and a missing module
    // there stops the level before the file.
    for dot_jq in ["def (", "def hb: 1; hb\n"] {
        let (_, stderr, code) = run(Some(dot_jq), &[], "include \"nonexist2\"; 1")?;
        assert_eq!(
            (stderr.as_str(), code),
            (
                "jq: error: module not found: nonexist2\n\njq: 1 compile error\n",
                3
            ),
            "{dot_jq:?}"
        );
    }
    // With the program's directives all found, the file's fault is the one reported, and a
    // main expression in a module beside it is counted too.
    let (_, stderr, code) = run(
        Some("def hb: 1; hb\n"),
        &[("mainexp.jq", "def f: 1; f\n")],
        "include \"mainexp\"; 1",
    )?;
    assert_eq!(
        (stderr.as_str(), code),
        (
            format!("{MAIN_EXPRESSION}{MAIN_EXPRESSION}jq: 2 compile errors\n").as_str(),
            3
        )
    );
    Ok(())
}

#[test]
fn test_module_with_a_main_expression_is_a_compile_error_5351() -> Result<()> {
    for (module, filter) in [
        ("def f: 1; f\n", "include \"mod\"; 1"),
        ("5\n", "import \"mod\" as m; 1"),
        (".\n", "include \"mod\"; 1"),
        ("def g: 1;\n.\n", "include \"mod\"; 1"),
        // Checked before the module's own directives are read.
        (
            "import \"nonexist\" as x; def hb: 1; hb\n",
            "include \"mod\"; 1",
        ),
    ] {
        let (stdout, stderr, code) = run(None, &[("mod.jq", module)], filter)?;
        assert_eq!(
            (stdout.as_str(), stderr.as_str(), code),
            (
                "",
                format!("{MAIN_EXPRESSION}jq: 1 compile error\n").as_str(),
                3
            ),
            "{module:?}"
        );
    }
    // A module of definitions alone, with or without a trailing comment, still loads.
    let (stdout, stderr, code) = run(None, &[("mod.jq", "def g: 1; # c\n")], "include \"mod\"; g")?;
    assert_eq!((stdout.as_str(), code), ("1\n", 0), "{stderr}");
    Ok(())
}

#[test]
fn test_module_faults_are_counted_once_each_in_jqs_order_5351() -> Result<()> {
    let modules = [
        ("mainexp.jq", "def f: 1; f\n"),
        ("synerr.jq", "def (\n"),
        ("ok.jq", "def g: 1;\n"),
    ];
    // Last-declared first, one entry per faulty module, `2 compile errors`.
    let (_, stderr, code) = run(None, &modules, "include \"mainexp\"; include \"synerr\"; 1")?;
    assert_eq!(code, 3, "{stderr}");
    assert!(
        stderr.starts_with("jq: error: ") && stderr.contains("synerr.jq, line 1:"),
        "{stderr}"
    );
    assert!(
        stderr.ends_with(&format!("{MAIN_EXPRESSION}jq: 2 compile errors\n")),
        "{stderr}"
    );
    let (_, stderr, code) = run(None, &modules, "include \"synerr\"; include \"mainexp\"; 1")?;
    assert_eq!(code, 3, "{stderr}");
    assert!(
        stderr.starts_with(MAIN_EXPRESSION)
            && stderr.contains("synerr.jq, line 1:")
            && stderr.ends_with("jq: 2 compile errors\n"),
        "{stderr}"
    );
    // A missing module stops its level before the ones declared ahead of it.
    let (_, stderr, code) = run(
        None,
        &modules,
        "include \"mainexp\"; include \"nonexist\"; 1",
    )?;
    assert_eq!(
        (stderr.as_str(), code),
        (
            "jq: error: module not found: nonexist\n\njq: 1 compile error\n",
            3
        )
    );
    Ok(())
}
