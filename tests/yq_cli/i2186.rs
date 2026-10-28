//! #2186: the routes that printed a repeated mapping key once, where yq v4.53.3 prints it twice.
//!
//! The DOM route's result is an `OwnedValue` (`ResultWithComments`). It used to be `IndexMap`-backed,
//! so anything that reached it had merged `a: 1 / a: 2` into `a: 2` before output. Since the
//! duplicate-capable value (#1344, ADR-0027) those routes print yq's answer, every key kept. The
//! first test is the control: it shows the document really has a repeated key and that the cursor
//! route, which a plain `.` takes, keeps it.

use std::io::Write;
use std::process::{Command, Stdio};

use anyhow::Result;
use tempfile::NamedTempFile;

use crate::run_yq_stdin;

/// What the file holds after `succinctly yq -i filter` ran over `contents`.
fn inplace(contents: &str, filter: &str) -> Result<String> {
    let mut file = NamedTempFile::new()?;
    write!(file, "{contents}")?;
    let output = Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .arg("yq")
        .arg("-i")
        .arg(filter)
        .arg(file.path())
        .stdin(Stdio::null())
        .output()?;
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(std::fs::read_to_string(file.path())?)
}

const DUP: &str = "a: 1\na: 2\nb: 3\n";

#[test]
fn the_cursor_route_keeps_the_repeated_key_2186() -> Result<()> {
    // `-o=json` rewrites the text, so a verbatim echo of the input cannot satisfy this.
    assert_eq!(
        run_yq_stdin(".", DUP, &["-o=json", "-I=0"])?,
        ("{\"a\":1,\"a\":2,\"b\":3}\n".to_string(), 0)
    );
    assert_eq!(run_yq_stdin(".", DUP, &[])?, (DUP.to_string(), 0));
    // A top-level comma of streamable branches keeps it too (#5283); yq prints a node twice once.
    assert_eq!(run_yq_stdin(". , .", DUP, &[])?, (DUP.to_string(), 0));
    assert_eq!(
        run_yq_stdin(".", DUP, &["--slurp"])?.0,
        "- a: 1\n  a: 2\n  b: 3\n"
    );
    Ok(())
}

#[test]
fn the_dom_forcing_routes_keep_the_repeated_key_2186() -> Result<()> {
    let rows: &[(&str, &[&str], &str)] = &[
        (".", &["-P"], DUP),
        (".", &["--arg", "z", "y"], DUP),
        (".", &["--eval-all"], DUP),
    ];
    for (filter, args, expected) in rows {
        let (out, code) = run_yq_stdin(filter, DUP, args)?;
        assert_eq!(code, 0, "{filter} {args:?}");
        assert_eq!(out, *expected, "{filter} {args:?}");
    }
    Ok(())
}

/// Since #1344 (ADR-0027) the owned value holds a repeated key's earlier entry beside the
/// collapsed view, so the routes above and these -- `--slurp`'s document and a write through a
/// path -- all print yq's answer: every entry, the written one replacing only the last.
#[test]
fn the_routes_without_a_comment_tree_keep_the_repeated_key_2186() -> Result<()> {
    let rows: &[(&str, &[&str], &str)] = &[
        (".[0]", &["--slurp"], DUP),
        (".b = 5", &[], "a: 1\na: 2\nb: 5\n"),
        (".c = 1", &[], "a: 1\na: 2\nb: 3\nc: 1\n"),
        ("del(.b)", &[], "a: 1\na: 2\n"),
    ];
    for (filter, args, expected) in rows {
        let (out, code) = run_yq_stdin(filter, DUP, args)?;
        assert_eq!(code, 0, "{filter} {args:?}");
        assert_eq!(out, *expected, "{filter} {args:?}");
    }
    Ok(())
}

/// The part of #2186 that was a wiring fault: the DOM route's `LazySeq` arm printed a large
/// integer as `1e+20`; it now keeps yq's spelling (the JSON form is `100000000000000000000.0`).
#[test]
fn the_dom_route_keeps_a_large_integers_spelling_through_a_lazy_seq_2186() -> Result<()> {
    let doc = "- 99999999999999999999\n- 1\n";
    let rows: &[(&str, &str)] = &[
        ("map(.)", "- 99999999999999999999\n- 1\n"),
        ("sort", "- 1\n- 99999999999999999999\n"),
        ("reverse", "- 1\n- 99999999999999999999\n"),
    ];
    for (filter, expected) in rows {
        assert_eq!(
            run_yq_stdin(filter, doc, &["-P"])?,
            ((*expected).to_string(), 0),
            "{filter}"
        );
    }
    Ok(())
}

/// `-i` follows stdout: a read-only filter is evaluated on cursors and keeps the repeated key,
/// and since #1344 a write keeps it too (yq: `.b = 5` leaves both `a` entries).
#[test]
fn inplace_keeps_the_repeated_key_for_a_read_only_filter_and_for_a_write_2186() -> Result<()> {
    assert_eq!(inplace(DUP, "keys")?, "- a\n- a\n- b\n");
    assert_eq!(inplace(DUP, "to_entries | length")?, "3\n");
    assert_eq!(inplace(DUP, ".b = 5")?, "a: 1\na: 2\nb: 5\n");
    assert_eq!(inplace(DUP, "del(.b)")?, "a: 1\na: 2\n");
    Ok(())
}
