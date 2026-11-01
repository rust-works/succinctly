//! #5478: an empty or whitespace-only JSON input is the same empty document as an empty YAML one
//! (#4099), not a JSON `null`. yq prints it as a blank line (`[.]` is `-`) and spells it `null`
//! only when the output is JSON; `succinctly yq` printed `null` for a zero-byte file, nothing at
//! all for a whitespace-only one (`--eval-all` failed with `invalid cursor position`), and counted
//! a whitespace-only file among others as a document of its own. Every row is a live yq v4.53.3
//! capture; space, tab, line feed and carriage return are whitespace to its JSON reader and a
//! form feed, vertical tab or no-break space is not.
//!
//! Not matched, recorded in `docs/compliance/yq/limitations.md`: `--eval-all` over such an input
//! prints `null` where yq prints a blank line, the owned-value route it takes having no spelling
//! for an empty node; a lone NUL byte, which yq also reads as whitespace, is still refused.

use anyhow::Result;
use tempfile::TempDir;

use crate::{run_yq_files, run_yq_stdin};

/// What `-p=json` makes of an input that holds no document: (filter, extra args, stdout).
const ROWS: &[(&str, &[&str], &str)] = &[
    (".", &["-p=json"], "\n"),
    ("[.]", &["-p=json"], "-\n"),
    (".", &["-p=json", "-P"], "\n"),
    (".", &["-p=json", "-I=4"], "\n"),
    (".", &["-p=json", "-N"], "\n"),
    (".a = 1", &["-p=json"], "a: 1\n"),
    ("length", &["-p=json"], "0\n"),
    (".a", &["-p=json"], "null\n"),
    ("tag", &["-p=json"], "!!null\n"),
    ("type", &["-p=json"], "!!null\n"),
    ("document_index", &["-p=json"], "0\n"),
    (". == null", &["-p=json"], "true\n"),
    // A JSON output is where the `null` is spelled.
    (".", &["-p=json", "-o=json", "-I=0"], "null\n"),
    ("[.]", &["-p=json", "-o=json", "-I=0"], "[null]\n"),
    (".a = 1", &["-p=json", "-o=json", "-I=0"], "{\"a\":1}\n"),
    // `-o auto` follows the input's own format: JSON.
    (".", &["-o", "auto"], "null\n"),
    ("[.]", &["-o", "auto", "-I=0"], "[null]\n"),
    (".a = 1", &["-o", "auto", "-I=0"], "{\"a\":1}\n"),
];

/// Every spelling of "no document" in a JSON file: empty, then each whitespace byte yq skips.
const NO_DOCUMENT: &[&str] = &["", " ", "\n", "\n\n", "\t", "\r\n", " \n ", "  \n"];

#[test]
fn test_empty_and_whitespace_only_json_files_are_the_empty_document_5478() -> Result<()> {
    let dir = TempDir::new()?;
    for (at, content) in NO_DOCUMENT.iter().enumerate() {
        let path = dir.path().join(format!("w{at}.json"));
        std::fs::write(&path, content)?;
        for (filter, args, expected) in ROWS {
            let (stdout, stderr, code) = run_yq_files(filter, &[path.as_path()], args)?;
            assert_eq!(
                (stdout.as_str(), code),
                (*expected, 0),
                "{filter} {args:?} over {content:?}: {stderr}"
            );
        }
    }
    Ok(())
}

#[test]
fn test_empty_and_whitespace_only_json_stdin_is_the_empty_document_5478() -> Result<()> {
    for content in NO_DOCUMENT {
        for (filter, args, expected) in ROWS {
            // `-o auto` follows a file's extension; stdin has none, so it reads as YAML.
            if args.contains(&"auto") {
                continue;
            }
            let (stdout, code) = run_yq_stdin(filter, content, args)?;
            assert_eq!(
                (stdout.as_str(), code),
                (*expected, 0),
                "{filter} {args:?} over {content:?}"
            );
        }
    }
    Ok(())
}

/// A byte that is not JSON whitespace is not "no document": yq refuses it, and so does this.
#[test]
fn test_a_non_json_whitespace_byte_is_still_refused_5478() -> Result<()> {
    let dir = TempDir::new()?;
    for (at, content) in ["\u{c}", "\u{b}", "\u{a0}", "// c\n", "\u{feff}", " \u{c} "]
        .iter()
        .enumerate()
    {
        let path = dir.path().join(format!("r{at}.json"));
        std::fs::write(&path, content)?;
        let (stdout, _, code) = run_yq_files(".", &[path.as_path()], &["-p=json"])?;
        assert_eq!((stdout.as_str(), code), ("", 1), "{content:?}");
    }
    Ok(())
}

/// A whitespace-only file among others reads no document, as an empty one does (#4099); every
/// file without a document is one empty document.
#[test]
fn test_whitespace_only_json_file_among_several_inputs_5478() -> Result<()> {
    let dir = TempDir::new()?;
    let write = |name: &str, content: &str| -> Result<std::path::PathBuf> {
        let path = dir.path().join(name);
        std::fs::write(&path, content)?;
        Ok(path)
    };
    let empty = write("e.json", "")?;
    let blank = write("w.json", "  \n")?;
    let one = write("x.json", "{\"a\":1}")?;
    // More than one 8 KiB read of whitespace, and a document after it: a later file scanned in
    // chunks must read as the one-shot check of the first does.
    let long_blank = write("long.json", &" \n".repeat(10_000))?;
    let long_then_one = write(
        "long_one.json",
        &format!("{}{{\"a\":1}}", " ".repeat(20_000)),
    )?;
    let json = ["-p=json", "-o=json", "-I=0"];
    for (filter, files, expected) in [
        (".", vec![&empty, &blank], "null\n"),
        (".", vec![&blank, &empty], "null\n"),
        (".", vec![&blank, &blank], "null\n"),
        (".", vec![&blank, &one], "{\"a\":1}\n"),
        (".", vec![&one, &blank], "{\"a\":1}\n"),
        (".", vec![&empty, &one, &blank], "{\"a\":1}\n"),
        ("file_index", vec![&empty, &blank], "0\n"),
        ("file_index", vec![&blank, &one], "1\n"),
        ("file_index", vec![&one, &blank], "0\n"),
        ("filename", vec![&blank, &one], "\"x.json\"\n"),
        (".", vec![&long_blank, &blank], "null\n"),
        (".", vec![&blank, &long_blank, &one], "{\"a\":1}\n"),
        ("file_index", vec![&blank, &long_blank, &one], "2\n"),
        (".", vec![&long_blank, &long_then_one], "{\"a\":1}\n"),
        ("file_index", vec![&long_blank, &long_then_one], "1\n"),
    ] {
        let paths: Vec<&std::path::Path> = files.iter().map(|p| p.as_path()).collect();
        let (stdout, stderr, code) = run_yq_files(filter, &paths, &json)?;
        // `filename` is the name as given on the command line.
        let expected = expected.replace("x.json", &one.display().to_string());
        assert_eq!(
            (stdout.as_str(), code),
            (expected.as_str(), 0),
            "{filter}: {stderr}"
        );
    }
    // The blank line is the YAML spelling, over several files too.
    let (stdout, _, code) = run_yq_files(".", &[blank.as_path(), empty.as_path()], &["-p=json"])?;
    assert_eq!((stdout.as_str(), code), ("\n", 0));
    Ok(())
}

/// `--eval-all` over a whitespace-only file used to fail with `invalid cursor position`.
#[test]
fn test_eval_all_over_whitespace_only_json_5478() -> Result<()> {
    let dir = TempDir::new()?;
    let blank = dir.path().join("w.json");
    std::fs::write(&blank, "  \n")?;
    let (stdout, stderr, code) = run_yq_files(
        ".",
        &[blank.as_path()],
        &["--eval-all", "-p=json", "-o=json"],
    )?;
    assert_eq!((stdout.as_str(), code), ("null\n", 0), "{stderr}");
    let (stdout, stderr, code) = run_yq_files(".", &[blank.as_path()], &["--eval-all", "-p=json"])?;
    assert_eq!((stdout.as_str(), code), ("null\n", 0), "{stderr}");
    Ok(())
}

/// `-i` rewrites the file as the empty document would be printed.
#[test]
fn test_inplace_over_empty_and_whitespace_only_json_5478() -> Result<()> {
    let dir = TempDir::new()?;
    for (at, content) in ["", "  \n"].iter().enumerate() {
        for (filter, args, expected) in [
            (
                ".a = 1",
                vec!["-i", "-p=json", "-o=json"],
                "{\n  \"a\": 1\n}\n",
            ),
            (
                ".a = 1",
                vec!["-i", "-p=json", "-o=json", "-I=0"],
                "{\"a\":1}\n",
            ),
            (".a = 1", vec!["-i", "-p=json"], "a: 1\n"),
            (".", vec!["-i", "-p=json"], "\n"),
            (".", vec!["-i", "-p=json", "-o=json"], "null\n"),
        ] {
            let path = dir.path().join(format!("i{at}.json"));
            std::fs::write(&path, content)?;
            let (stdout, stderr, code) = run_yq_files(filter, &[path.as_path()], &args)?;
            assert_eq!(
                (stdout.as_str(), code),
                ("", 0),
                "{filter} {args:?}: {stderr}"
            );
            assert_eq!(
                std::fs::read_to_string(&path)?,
                expected,
                "{filter} {args:?} over {content:?}"
            );
        }
    }
    Ok(())
}

/// A document that is there is read as JSON as before.
#[test]
fn test_a_json_document_is_read_as_json_5478() -> Result<()> {
    for (filter, input, args, expected) in [
        (".", "{\"a\":1}\n", vec!["-p=json"], "a: 1\n"),
        (".", " [1] ", vec!["-p=json", "-o=json", "-I=0"], "[1]\n"),
        (".", "\n\n1\n", vec!["-p=json", "-o=json"], "1\n"),
        (".", "null", vec!["-p=json"], "null\n"),
        (".", " null\n", vec!["-p=json", "-o=json"], "null\n"),
    ] {
        let (stdout, code) = run_yq_stdin(filter, input, &args)?;
        assert_eq!(
            (stdout.as_str(), code),
            (expected, 0),
            "{filter} over {input:?}"
        );
    }
    Ok(())
}
