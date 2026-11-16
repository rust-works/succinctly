//! #7293: an object of 786,432 keys or more (`KeyHashes::SATURATING_KEYS`, the width past which
//! the key-hash table stops growing) is certified by the canonical scan when no key repeats,
//! settled by a sort of its key hashes, so `.` re-indents it and `-c .` echoes it; one whose last
//! key repeats its first is still declined and printed by the walk, which collapses the repeat to
//! the first position holding the last value, as jq 1.7.1 does. Every expected text below was
//! checked against `/usr/bin/jq` 1.7.1 over the same bytes.

use anyhow::Result;

use crate::run_jq_full;

/// One key past the width at which the table saturates.
const KEYS: usize = 786_433;

/// A flat object of [`KEYS`] keys `"k<i>": <i % 1000>`; with `repeat`, the last key is `k0`.
fn wide(repeat: bool) -> String {
    let mut doc = String::with_capacity(KEYS * 14);
    doc.push('{');
    for i in 0..KEYS {
        if i > 0 {
            doc.push(',');
        }
        let key = if repeat && i == KEYS - 1 { 0 } else { i };
        doc.push_str(&format!("\"k{key}\":{}", i % 1000));
    }
    doc.push_str("}\n");
    doc
}

/// What jq prints for [`wide`] with `sep` between fields and `colon` after each key, at
/// `indent` before each field; a repeated `k0` takes its first position and its last value.
fn printed(repeat: bool, open: &str, sep: &str, colon: &str, close: &str) -> String {
    let fields = if repeat { KEYS - 1 } else { KEYS };
    let mut out = String::with_capacity(KEYS * 18);
    out.push_str(open);
    for i in 0..fields {
        if i > 0 {
            out.push_str(sep);
        }
        let value = if repeat && i == 0 {
            (KEYS - 1) % 1000
        } else {
            i % 1000
        };
        out.push_str(&format!("\"k{i}\"{colon}{value}"));
    }
    out.push_str(close);
    out
}

fn run(flags: &[&str], text: &str) -> Result<(String, String, i32)> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("in.json");
    std::fs::write(&path, text)?;
    let mut args = flags.to_vec();
    args.extend([".", path.to_str().expect("utf-8 temp path")]);
    run_jq_full(&args, None)
}

#[test]
fn test_an_object_past_the_key_table_ceiling_prints_what_jq_prints_7293() -> Result<()> {
    for repeat in [false, true] {
        let doc = wide(repeat);
        let (out, err, code) = run(&[], &doc)?;
        assert_eq!((err.as_str(), code), ("", 0), "repeat {repeat}");
        assert!(
            out == printed(repeat, "{\n  ", ",\n  ", ": ", "\n}\n"),
            "pretty, repeat {repeat}: {}",
            &out[..200.min(out.len())]
        );
        let (out, err, code) = run(&["-c"], &doc)?;
        assert_eq!((err.as_str(), code), ("", 0), "repeat {repeat}");
        assert!(
            out == printed(repeat, "{", ",", ":", "}\n"),
            "compact, repeat {repeat}: {}",
            &out[..200.min(out.len())]
        );
    }
    Ok(())
}

/// Indentation of up to eight characters is written a character at a time and deeper indentation
/// as slices of a run (#7293); both widths, and the step between them, print what jq prints.
#[test]
fn test_shallow_and_deep_indentation_print_what_jq_prints_7293() -> Result<()> {
    // Nine levels of `{"a":[...]}`, so `--indent 1` reaches 18 columns and `--indent 3` 54.
    let mut doc = String::from("1");
    for _ in 0..9 {
        doc = format!("{{\"a\":[{doc},true]}}");
    }
    for (flags, width) in [(vec!["--indent", "1"], 1), (vec!["--indent", "3"], 3)] {
        // Built inside out, each level adds one object and one array level of indentation.
        let mut want = String::from("1");
        let pad = |n: usize| " ".repeat(n * width);
        for _ in 0..9 {
            let inner = want.replace('\n', &format!("\n{}", pad(2)));
            want = format!(
                "{{\n{o}\"a\": [\n{a}{inner},\n{a}true\n{o}]\n}}",
                o = pad(1),
                a = pad(2),
            );
        }
        want.push('\n');
        let (out, err, code) = run(&flags, &doc)?;
        assert_eq!((err.as_str(), code), ("", 0), "{flags:?}");
        assert_eq!(out, want, "{flags:?}");
    }
    Ok(())
}
