//! #7479: the `--input-dsv` examples in `README.md` and `CLAUDE.md` ran as written: they did not.
//! `'.[] | .[0]'` indexed a string (a row is one array of strings, and each row is its own input,
//! so `.[]` already walks the fields), and `'\t'` is the two characters backslash and `t`, which
//! the one-character delimiter refuses. jq 1.7.1 and 1.8.2 answer `Unknown option --input-dsv`
//! to all of it, so there is no jq behaviour to follow; the examples are the contract, and they are
//! run here, so a future edit that breaks one fails this test.

#![cfg(unix)]

use std::process::Command;

use anyhow::Result;

use crate::{run_jq_full, succinctly_bin};

const README: &str = include_str!("../../README.md");
const CLAUDE_MD: &str = include_str!("../../CLAUDE.md");

/// Every `sjq --input-dsv ...` example line of a document, minus its trailing `# comment`.
fn examples(doc: &str) -> Vec<&str> {
    doc.lines()
        .filter(|line| line.starts_with("sjq --input-dsv "))
        .map(|line| line.split(" #").next().unwrap_or(line).trim_end())
        .collect()
}

#[test]
fn the_documented_input_dsv_examples_run_7479() -> Result<()> {
    let dir = tempfile::tempdir()?;
    std::fs::write(dir.path().join("users.csv"), "name,age\nAda,36\nAlice,41\n")?;
    std::fs::write(dir.path().join("data.csv"), "name,age\nAda,36\nAlice,41\n")?;
    std::fs::write(dir.path().join("data.tsv"), "name\tage\nAda\t36\n")?;

    let mut found = 0;
    for (doc, text) in [("README.md", README), ("CLAUDE.md", CLAUDE_MD)] {
        for example in examples(text) {
            found += 1;
            // `sjq` is `succinctly jq`; bash is what `$'\t'` needs (the README says so).
            let script = format!("sjq() {{ \"$SUCCINCTLY\" jq \"$@\"; }}\n{example}");
            let output = Command::new("bash")
                .arg("-c")
                .arg(&script)
                .current_dir(dir.path())
                .env("SUCCINCTLY", succinctly_bin())
                .output()?;
            let stderr = String::from_utf8(output.stderr)?;
            assert!(
                output.status.success() && stderr.is_empty(),
                "{doc}: `{example}` exited {:?}: {stderr}",
                output.status.code()
            );
            assert!(
                !output.stdout.is_empty(),
                "{doc}: `{example}` printed nothing"
            );
        }
    }
    // README has two, CLAUDE.md one: a rename of the line shape must not empty the loop.
    assert!(found >= 3, "found only {found} examples");
    Ok(())
}

/// What the examples print, over the files above (the values are the rows' own strings).
#[test]
fn the_documented_input_dsv_examples_print_the_first_column_7479() -> Result<()> {
    let csv = "name,age\nAda,36\nAlice,41\n";
    let (out, err, code) = run_jq_full(&["--input-dsv", ",", "-c", ".[0]"], Some(csv))?;
    assert_eq!(
        (out.as_str(), err.as_str(), code),
        ("\"name\"\n\"Ada\"\n\"Alice\"\n", "", 0)
    );

    let (out, err, code) = run_jq_full(
        &["--input-dsv", ",", "-c", "select(.[0] == \"Alice\")"],
        Some(csv),
    )?;
    assert_eq!(
        (out.as_str(), err.as_str(), code),
        ("[\"Alice\",\"41\"]\n", "", 0)
    );

    let (out, err, code) = run_jq_full(
        &["--input-dsv", "\t", "-c", ".[0]"],
        Some("name\tage\nAda\t36\n"),
    )?;
    assert_eq!(
        (out.as_str(), err.as_str(), code),
        ("\"name\"\n\"Ada\"\n", "", 0)
    );
    Ok(())
}

/// The shapes the old examples had stay what they were: `.[] | .[0]` errors on every field (a
/// string has no index), and the two characters `\t` are no delimiter. The documentation says so
/// (`docs/guides/cli.md`), so these are pinned too.
#[test]
fn the_shapes_the_documentation_warns_against_stay_refused_7479() -> Result<()> {
    let (out, err, code) = run_jq_full(&["--input-dsv", ",", ".[] | .[0]"], Some("a,b\n"))?;
    assert_eq!(out, "");
    assert_eq!(
        err,
        "jq: error (at <stdin>:1): Cannot index string with number\n"
    );
    assert_eq!(code, 5);

    let (out, err, code) = run_jq_full(&["--input-dsv", "\\t", ".[0]"], Some("a\tb\n"))?;
    assert_eq!(out, "");
    assert!(
        err.contains("invalid value '\\t' for '--input-dsv <DELIMITER>': too many characters"),
        "{err}"
    );
    assert_eq!(code, 2);
    Ok(())
}
