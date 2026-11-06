//! #5721: a NUL ends a JSON input (a lone NUL is the empty document), and an explicit `-p=json`
//! makes `-o auto` YAML. Expected output is yq v4.53.3's.
//!
//! Also here: the empty document has no file, so `filename` is empty over it; and a result that
//! is a document with no content of its own (a comment-only or empty one) is not followed by a
//! second line break.

use anyhow::Result;
use tempfile::TempDir;

use crate::{run_yq_files, run_yq_stdin, run_yq_stdin_bytes_with_stderr};

fn json_nul(input: &[u8], extra: &[&str]) -> Result<(String, i32)> {
    let mut args = vec!["-p=json", "-o=json", "-I0"];
    args.extend_from_slice(extra);
    let (out, _, code) = run_yq_stdin_bytes_with_stderr(".", input, &args)?;
    Ok((out, code))
}

#[test]
fn a_nul_ends_a_json_input() -> Result<()> {
    for (input, expected) in [
        (&b"\0"[..], "null\n"),
        (b"\0\n", "null\n"),
        (b" \0 ", "null\n"),
        (b"\0\0", "null\n"),
        (b"\0 1", "null\n"),
        (b"\0{\"a\":1}", "null\n"),
        (b"1\0", "1\n"),
        (b"1\0\n2", "1\n"),
        (b"{\"a\":1}\0", "{\"a\":1}\n"),
    ] {
        assert_eq!(
            json_nul(input, &[])?,
            (expected.to_string(), 0),
            "{input:?}"
        );
    }
    Ok(())
}

#[test]
fn a_lone_nul_is_the_empty_document_for_yaml_output() -> Result<()> {
    let (out, _, code) = run_yq_stdin_bytes_with_stderr(".", b"\0", &["-p=json"])?;
    assert_eq!((out.as_str(), code), ("\n", 0));
    Ok(())
}

#[test]
fn explicit_json_input_makes_output_auto_yaml() -> Result<()> {
    let (out, code) = run_yq_stdin(".", "{\"a\":1}", &["-p=json", "-o", "auto"])?;
    assert_eq!((out.as_str(), code), ("a: 1\n", 0));
    let (out, code) = run_yq_stdin(".", "{\"a\":1}", &["-p=json", "-o", "auto", "-I0"])?;
    assert_eq!((out.as_str(), code), ("a: 1\n", 0));
    Ok(())
}

/// A result that is a document with no content prints only the header it was written with, whose
/// own line break ends it. `select` and the other filters that keep the document as a result
/// wrote the terminator on top, a blank line yq never writes; `.` already did not.
#[test]
fn a_kept_document_with_no_content_gets_no_extra_blank_line() -> Result<()> {
    for filter in [
        ".",
        "select(true)",
        "select(. == null)",
        "(.)",
        "..",
        "select(true) | select(true)",
    ] {
        for (input, expected) in [
            ("", "\n"),
            ("\n", "\n"),
            ("# c\n", "# c\n"),
            ("# c\n\n", "# c\n\n"),
            ("---\n", "---\n"),
            ("# c\n---\na: 1\n", "# c\n---\na: 1\n"),
        ] {
            if filter == ".." && input.contains("a: 1") {
                continue; // `..` also yields the child, `1`
            }
            if filter == "select(. == null)" && input.contains("a: 1") {
                continue; // the document is a mapping, so nothing is kept
            }
            let (out, code) = run_yq_stdin(filter, input, &[])?;
            assert_eq!(
                (out.as_str(), code),
                (expected, 0),
                "{filter} over {input:?}"
            );
        }
    }
    Ok(())
}

/// The terminator still follows a result that is not such a document: another result of the same
/// document, and a later document's own empty `---`.
#[test]
fn a_header_only_document_does_not_drop_the_terminator_of_other_results() -> Result<()> {
    let (out, code) = run_yq_stdin("select(. == null), 1", "# c\n", &[])?;
    assert_eq!((out.as_str(), code), ("# c\n1\n", 0));
    let (out, code) = run_yq_stdin("select(. == null), 1", "", &[])?;
    assert_eq!((out.as_str(), code), ("\n1\n", 0));
    Ok(())
}

/// The empty document is not a file: `filename` is empty over it (yq prints an empty line), for
/// stdin, an empty file, several empty files and an empty or whitespace-only JSON file, while
/// `file_index` and `document_index` stay `0`. A file that holds a line break is a document of
/// its own, and is named.
#[test]
fn the_empty_document_has_no_filename_5721() -> Result<()> {
    let dir = TempDir::new()?;
    let empty = dir.path().join("empty.yaml");
    std::fs::write(&empty, "")?;
    let empty2 = dir.path().join("empty2.yaml");
    std::fs::write(&empty2, "")?;
    let newline = dir.path().join("newline.yaml");
    std::fs::write(&newline, "\n")?;
    let one = dir.path().join("one.yaml");
    std::fs::write(&one, "a: 1\n")?;
    let json = dir.path().join("empty.json");
    std::fs::write(&json, " \n")?;

    for args in [&[][..], &["--eval-all"][..]] {
        let (out, code) = run_yq_stdin("filename", "", args)?;
        assert_eq!((out.as_str(), code), ("\n", 0), "stdin {args:?}");
        let (out, code) = run_yq_stdin("[filename, file_index, document_index]", "", args)?;
        assert_eq!(
            (out.as_str(), code),
            ("- \"\"\n- 0\n- 0\n", 0),
            "stdin {args:?}"
        );

        let (out, _, code) = run_yq_files("filename", &[&empty], args)?;
        assert_eq!((out.as_str(), code), ("\n", 0), "empty file {args:?}");
        let (out, _, code) = run_yq_files("filename", &[&empty, &empty2], args)?;
        assert_eq!((out.as_str(), code), ("\n", 0), "two empty files {args:?}");
        let (out, _, code) = run_yq_files("[filename, file_index]", &[&empty], args)?;
        assert_eq!(
            (out.as_str(), code),
            ("- \"\"\n- 0\n", 0),
            "empty file {args:?}"
        );

        let (out, _, code) = run_yq_files("filename", &[&newline], args)?;
        assert_eq!(
            (out.as_str(), code),
            (format!("{}\n", newline.display()).as_str(), 0),
            "a file holding a line break is named {args:?}"
        );
        // An empty file among others reads no document; the others are named as before.
        let (out, _, code) = run_yq_files("filename", &[&empty, &one], args)?;
        assert_eq!(
            (out.as_str(), code),
            (format!("{}\n", one.display()).as_str(), 0),
            "empty beside a document {args:?}"
        );

        let mut json_args = vec!["-p=json"];
        json_args.extend_from_slice(args);
        let (out, _, code) = run_yq_files("filename", &[&json], &json_args)?;
        assert_eq!(
            (out.as_str(), code),
            ("\n", 0),
            "whitespace-only JSON file {args:?}"
        );
    }
    Ok(())
}
