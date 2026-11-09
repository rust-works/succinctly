//! #6762: a UTF-8 byte-order mark leaves yq no header (#5357), and that holds on the routes that
//! gather a source before indexing it too: `-P`, a write, `-i`, and the `line`/`column` of a node.
//! Each row is a live yq v4.53.3 capture (`printf '\xef\xbb\xbf<text>' | yq <flags>`).

use std::io::Write;

use anyhow::Result;
use tempfile::NamedTempFile;

use crate::{run_yq_files, run_yq_stdin};

const BOM: &str = "\u{feff}";

/// `(text after the mark, flags, yq's output)`.
const ROWS: &[(&str, &[&str], &str)] = &[
    // The leading `---` is read, not echoed, on the DOM route as on the streaming one.
    ("---\na: 1\n---\nb: 2\n", &["-P", "."], "a: 1\n---\nb: 2\n"),
    ("---\na: 1\n", &[".a = 5"], "a: 5\n"),
    ("---\na: 1\n", &["del(.a)"], "{}\n"),
    // A leading run of markers is a null document and then the next one.
    ("---\n---\na: 1\n", &[".a = 5"], "a: 5\n---\na: 5\n"),
    // Comments: the head comment stays and the blank line or marker after it is not its.
    ("# c\n---\na: 1\n", &[".a = 5"], "# c\na: 5\n"),
    ("# c\n\na: 1\n", &[".a = 5"], "# c\na: 5\n"),
    (
        "---\n# k\na: 1\n# foot\n",
        &["-P", "."],
        "# k\na: 1\n# foot\n",
    ),
    ("---\na: &x 1\nb: *x\n", &[".a = 5"], "a: &x 5\nb: *x\n"),
    // `line` and `column` count every line of the text: yq slurped none of them.
    ("---\na: 1\n", &[".a | line"], "2\n"),
    ("# c\na: 1\n", &[".a | line"], "2\n"),
    ("---\n# c\n\na: 1\n", &[".a | line"], "4\n"),
    ("---\na: 1\n", &[".a | column"], "4\n"),
    // `---` then `...` is an empty first document, a blank line.
    ("---\n...\n", &["."], "\n"),
    ("---\n...\n---\na: 1\n", &["."], "\n---\na: 1\n"),
];

/// A row's flags as `(filter, flags before it)`: a row is a filter alone, or one flag and a filter.
fn split<'a>(flags: &'a [&'a str]) -> (&'a str, &'a [&'a str]) {
    match flags {
        [filter] => (filter, &[]),
        [flag, filter] => (filter, std::slice::from_ref(flag)),
        _ => unreachable!("a row is a filter, or a flag and a filter"),
    }
}

#[test]
fn test_a_bom_leaves_no_header_on_the_dom_routes_on_stdin_6762() -> Result<()> {
    for (text, flags, want) in ROWS {
        let (filter, extra) = split(flags);
        let input = format!("{BOM}{text}");
        let (stdout, code) = run_yq_stdin(filter, &input, extra)?;
        assert_eq!((stdout.as_str(), code), (*want, 0), "{flags:?} on {text:?}");
    }
    Ok(())
}

#[test]
fn test_a_bom_leaves_no_header_on_the_dom_routes_in_a_file_6762() -> Result<()> {
    for (text, flags, want) in ROWS {
        let (filter, extra) = split(flags);
        let mut file = NamedTempFile::new()?;
        write!(file, "{BOM}{text}")?;
        let (stdout, stderr, code) = run_yq_files(filter, &[file.path()], extra)?;
        assert_eq!(
            (stdout.as_str(), code),
            (*want, 0),
            "{flags:?} on {text:?}: {stderr}"
        );
    }
    Ok(())
}

#[test]
fn test_inplace_rewrites_a_bom_file_without_its_leading_marker_6762() -> Result<()> {
    for (text, filter, want) in [
        ("---\na: 1\n", ".a = 5", "a: 5\n"),
        ("# c\n\na: 1\n", ".b = 2", "# c\na: 1\nb: 2\n"),
        ("---\na: 1\n---\nb: 2\n", ".", "a: 1\n---\nb: 2\n"),
    ] {
        let mut file = NamedTempFile::new()?;
        write!(file, "{BOM}{text}")?;
        let (_, stderr, code) = run_yq_files(filter, &[file.path()], &["-i"])?;
        assert_eq!(code, 0, "{stderr}");
        assert_eq!(
            std::fs::read_to_string(file.path())?,
            want,
            "{filter} on {text:?}"
        );
    }
    Ok(())
}

#[test]
fn test_the_dom_routes_keep_the_header_without_a_bom_6762() -> Result<()> {
    // The control: with no mark the marker is header, and a write echoes it.
    for (text, flags, want) in [
        ("---\na: 1\n", &["-P", "."][..], "---\na: 1\n"),
        ("---\na: 1\n", &[".a = 5"][..], "---\na: 5\n"),
        ("# c\n\na: 1\n", &[".a = 5"][..], "# c\n\na: 5\n"),
    ] {
        let (filter, extra) = split(flags);
        let (stdout, code) = run_yq_stdin(filter, text, extra)?;
        assert_eq!((stdout.as_str(), code), (want, 0), "{flags:?} on {text:?}");
    }
    Ok(())
}

#[test]
fn test_eval_all_reads_a_bom_file_with_no_header_6762() -> Result<()> {
    // Captured from `yq ea -o=json -I=0`: `--eval-all` is yq's `ea` command, and its `line` is the
    // position resolver the gathered sources feed.
    let extra = ["--eval-all", "-o=json", "-I=0"];
    for (text, filter, want) in [
        ("---\na: 1\n", ".a | line", "2\n"),
        ("---\na: 1\n---\na: 2\n", "[.a, (.a | line)]", "[1,2,2,4]\n"),
        ("# c\n---\na: 1\n", ".a | line", "3\n"),
        ("---\na: 1\n---\nb: 2\n", ".", "{\"a\":1}\n{\"b\":2}\n"),
        // A run of leading markers is a null document and then the next: the documents are counted
        // so, and `line` finds the second one's node.
        ("---\n---\na: 1\n", ".", "null\n{\"a\":1}\n"),
        ("---\n---\na: 1\n", "document_index", "0\n1\n"),
        ("---\n---\na: 1\n", "[.a, (.a | line)]", "[1,3]\n"),
        ("---\n...\n---\na: 1\n", "[.a, (.a | line)]", "[1,4]\n"),
    ] {
        let (stdout, code) = run_yq_stdin(filter, &format!("{BOM}{text}"), &extra)?;
        assert_eq!((stdout.as_str(), code), (want, 0), "{filter} on {text:?}");
    }
    Ok(())
}

#[test]
fn test_the_sort_keys_alias_fallback_reads_a_bom_source_with_no_header_6762() -> Result<()> {
    // `--sort-keys` is not a yq flag (no oracle row): over an alias the sort takes the DOM
    // fallback, which indexes the same source again and must read it as the first pass did.
    let (stdout, code) = run_yq_stdin(
        ".",
        &format!("{BOM}---\nb: &x 1\na: *x\n"),
        &["--sort-keys"],
    )?;
    assert_eq!((stdout.as_str(), code), ("a: 1\nb: &x 1\n", 0));
    Ok(())
}

#[test]
fn test_slurp_reads_a_bom_file_with_no_header_6762() -> Result<()> {
    // Captured from `yq ea '[.]'`: a run of leading markers is a null document and then the next.
    let (stdout, code) = run_yq_stdin(
        ".",
        &format!("{BOM}---\n---\na: 1\n"),
        &["--slurp", "-o=json", "-I=0"],
    )?;
    assert_eq!((stdout.as_str(), code), ("[null,{\"a\":1}]\n", 0));
    Ok(())
}
