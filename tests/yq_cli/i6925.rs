//! #6925: `succinctly yq -p base64` and `-p uri`, each of which decodes the whole input to one string
//! as yq's decoder does and then reads it as `-p json` reads a JSON string. Every answer was captured
//! from yq v4.53.3 (its `-p` deprecation warning on stderr left out). The rows that still differ are in
//! `docs/compliance/yq/limitations.md` (`-p base64|uri`): YAML output of decoded bytes that are not
//! UTF-8, and `--front-matter`.

use std::io::Write;

use anyhow::Result;
use tempfile::TempDir;

use crate::{run_yq_files, run_yq_stdin_bytes_with_stderr, run_yq_stdin_with_stderr};

/// (format, stdin, extra args, filter, yq's stdout, yq's stderr, yq's exit status).
type Row = (
    &'static str,
    &'static str,
    &'static [&'static str],
    &'static str,
    &'static str,
    &'static str,
    i32,
);

const ROWS: &[Row] = &[
    ("base64", "aGVsbG8=", &[], ".", "hello\n", "", 0),
    (
        "base64",
        "aGVsbG8=\n",
        &["-o", "json"],
        ".",
        "\"hello\"\n",
        "",
        0,
    ),
    (
        "base64",
        "  aGk  \n",
        &["-o", "json"],
        ".",
        "\"hi\"\n",
        "",
        0,
    ),
    ("base64", "", &["-o", "json"], ".", "\"\"\n", "", 0),
    ("base64", "MTIz", &[], "{\"a\": .}", "a: \"123\"\n", "", 0),
    ("base64", "MTIz", &[], "tag", "!!str\n", "", 0),
    ("base64", "eDogMQ==", &[], ".", "x: 1\n", "", 0),
    ("base64", "eDogMQ==", &[], "[.]", "- 'x: 1'\n", "", 0),
    (
        "base64",
        "bGluZTEKbGluZTI=",
        &[],
        "{\"a\": .}",
        "a: |-\n  line1\n  line2\n",
        "",
        0,
    ),
    (
        "base64",
        "dHJ1ZQ==",
        &["-o", "json"],
        ". + \"!\"",
        "\"true!\"\n",
        "",
        0,
    ),
    (
        "base64",
        "aGk=",
        &["-o", "props"],
        "{\"a\": .}",
        "a = hi\n",
        "",
        0,
    ),
    (
        "base64",
        "aG\nk=",
        &[],
        ".",
        "",
        "Error: bad file '-': unexpected EOF",
        1,
    ),
    (
        "base64",
        "aGk=\naGk=",
        &[],
        ".",
        "",
        "Error: bad file '-': illegal base64 data at input byte 4",
        1,
    ),
    (
        "base64",
        "aGk=aGk=",
        &[],
        ".",
        "",
        "Error: bad file '-': illegal base64 data at input byte 4",
        1,
    ),
    (
        "base64",
        "!!!",
        &[],
        ".",
        "",
        "Error: bad file '-': illegal base64 data at input byte 0",
        1,
    ),
    (
        "uri",
        "a+b%41\n",
        &["-o", "json"],
        ".",
        "\"a bA\\n\"\n",
        "",
        0,
    ),
    ("uri", "a%20b", &[], "{\"k\": .}", "k: a b\n", "", 0),
    ("uri", "", &["-o", "json"], ".", "\"\"\n", "", 0),
    (
        "uri",
        "%zz",
        &[],
        ".",
        "",
        "Error: bad file '-': invalid URL escape \"%zz\"",
        1,
    ),
    (
        "uri",
        "ab%",
        &[],
        ".",
        "",
        "Error: bad file '-': invalid URL escape \"%\"",
        1,
    ),
    (
        "uri",
        "ab%\n",
        &[],
        ".",
        "",
        "Error: bad file '-': invalid URL escape \"%\\n\"",
        1,
    ),
    ("uri", "x%C3%A9", &["-o", "json"], ".", "\"xé\"\n", "", 0),
];

#[test]
fn string_input_formats_match_yq_on_stdin_6925() -> Result<()> {
    for &(format, input, extra, filter, stdout, stderr, code) in ROWS {
        let mut args = vec!["-p", format];
        args.extend_from_slice(extra);
        let (out, err, status) = run_yq_stdin_with_stderr(filter, input, &args)?;
        let row = format!("-p {format} {extra:?} {filter:?} on {input:?}");
        assert_eq!(out, stdout, "stdout of {row}");
        assert_eq!(err.trim_end(), stderr, "stderr of {row}");
        assert_eq!(status, code, "exit status of {row}");
    }
    Ok(())
}

/// A decoded byte that is not UTF-8 is U+FFFD under `-o json`, as in yq (which writes it as the
/// escape `\ufffd`, the `-p json` divergence of #1616), and YAML output refuses it.
#[test]
fn decoded_bytes_that_are_not_utf8_take_the_json_route_6925() -> Result<()> {
    let (out, _, code) =
        run_yq_stdin_bytes_with_stderr(".", b"/w==", &["-p", "base64", "-o", "json"])?;
    assert_eq!((out.as_str(), code), ("\"\u{fffd}\"\n", 0));
    let (out, err, code) = run_yq_stdin_bytes_with_stderr(".", b"a%FF", &["-p", "uri"])?;
    assert_eq!((out.as_str(), code), ("", 1));
    assert!(err.contains("invalid UTF-8"), "{err}");
    Ok(())
}

/// Go's stream decoder hands the input to `Encoding.Decode` a chunk at a time, the first 680 bytes
/// and then sizes that follow `bytes.Buffer`'s growth. Padding that ends a chunk is accepted with more
/// data after it, and an error's offset counts from its chunk's start. yq v4.53.3: `510`, `illegal
/// base64 data at input byte 404` and `... 564`.
#[test]
fn base64_reads_the_input_in_go_stream_chunks_6925() -> Result<()> {
    // 508 bytes encode to 680 characters ending in `==`, the whole first chunk.
    let first_chunk = "eHh4".repeat(169) + "eA==";
    assert_eq!(first_chunk.len(), 680);
    let (out, err, code) = run_yq_stdin_with_stderr(
        "length",
        &(first_chunk.clone() + "aGk="),
        &["-p", "base64", "-o", "json"],
    )?;
    assert_eq!((out.as_str(), err.as_str(), code), ("510\n", "", 0));
    // Inside the first chunk the same padding is followed by data, which is an error.
    let (_, err, code) = run_yq_stdin_with_stderr(
        "length",
        &(first_chunk[..400].to_owned() + "aGk=" + &first_chunk[400..]),
        &["-p", "base64"],
    )?;
    assert_eq!(
        (err.trim_end(), code),
        (
            "Error: bad file '-': illegal base64 data at input byte 404",
            1
        )
    );
    let mut long = "enp6".repeat(2000).into_bytes();
    long[5000] = b'!';
    let (_, err, code) = run_yq_stdin_bytes_with_stderr("length", &long, &["-p", "base64"])?;
    assert_eq!(
        (err.trim_end(), code),
        (
            "Error: bad file '-': illegal base64 data at input byte 564",
            1
        )
    );
    Ok(())
}

fn write_file(dir: &TempDir, name: &str, content: &[u8]) -> Result<std::path::PathBuf> {
    let path = dir.path().join(name);
    std::fs::File::create(&path)?.write_all(content)?;
    Ok(path)
}

/// yq takes the format from the first file's lowercased extension, reads every file with it, and
/// prints in it under `-o auto`; an explicit `-p` prints YAML. An empty file is the empty string.
#[test]
fn string_input_format_by_extension_and_per_file_6925() -> Result<()> {
    let dir = TempDir::new()?;
    let a = write_file(&dir, "a.base64", b"aGk=")?;
    let up = write_file(&dir, "b.URI", b"a+b%41")?;
    let txt = write_file(&dir, "c.txt", b"eW8=")?;
    let empty = write_file(&dir, "d.txt", b"")?;
    let bad = write_file(&dir, "e.txt", b"bad!")?;
    let json = write_file(&dir, "f.JSON", b"{\"x\":1}")?;

    assert_eq!(
        run_yq_files(".", &[&a], &[])?,
        ("aGk=".into(), String::new(), 0)
    );
    assert_eq!(
        run_yq_files(". + \"!\"", &[&a, &txt], &[])?,
        ("aGkheW8h".into(), String::new(), 0)
    );
    assert_eq!(
        run_yq_files(".", &[&a], &["-o", "yaml"])?,
        ("hi\n".into(), String::new(), 0)
    );
    assert_eq!(
        run_yq_files(".", &[&up], &["-o", "json"])?,
        ("\"a bA\"\n".into(), String::new(), 0)
    );
    assert_eq!(
        run_yq_files(".", &[&txt, &empty, &txt], &["-p", "base64"])?,
        ("yo\n---\n\n---\nyo\n".into(), String::new(), 0)
    );
    assert_eq!(
        run_yq_files(
            "[filename, file_index]",
            &[&txt, &a],
            &["-p", "base64", "-o", "json", "-I0"]
        )?
        .0,
        format!(
            "[{:?},0]\n[{:?},1]\n",
            txt.display().to_string(),
            a.display().to_string()
        )
    );
    let (out, err, code) = run_yq_files(".", &[&txt, &bad], &["-p", "base64"])?;
    assert_eq!((out.as_str(), code), ("yo\n", 1));
    assert_eq!(
        err.trim_end(),
        format!(
            "Error: bad file '{}': illegal base64 data at input byte 3",
            bad.display()
        )
    );
    assert_eq!(
        run_yq_files(".x", &[&json], &[])?,
        ("1\n".into(), String::new(), 0)
    );
    Ok(())
}

/// `-i` writes the result back in the run's output format: YAML for an explicit `-p`, the input's
/// own format for one picked by its extension.
#[test]
fn string_input_inplace_writes_the_output_format_6925() -> Result<()> {
    let dir = TempDir::new()?;
    let txt = write_file(&dir, "a.txt", b"aGk=")?;
    let b64 = write_file(&dir, "b.base64", b"aGk=")?;
    run_yq_files(".", &[&txt], &["-p", "base64", "-i"])?;
    assert_eq!(std::fs::read(&txt)?, b"hi\n");
    run_yq_files(". + \"!\"", &[&b64], &["-i"])?;
    assert_eq!(std::fs::read(&b64)?, b"aGkh");
    Ok(())
}

/// A second `-` finds stdin drained, which the string decoders read as the empty string.
#[test]
fn a_drained_stdin_argument_is_the_empty_string_6925() -> Result<()> {
    // The helper puts the filter last, so it is the second `-` here.
    let (out, _, code) = run_yq_stdin_with_stderr("-", "aGk=", &["-p", "base64", ".", "-"])?;
    assert_eq!((out.as_str(), code), ("hi\n---\n\n", 0));
    Ok(())
}

/// Recorded divergence: yq decodes the extracted front matter; here the combination is refused,
/// as `-p json` is, naming the format given.
#[test]
fn front_matter_with_a_string_format_is_refused_6925() -> Result<()> {
    let (_, err, code) =
        run_yq_stdin_with_stderr(".", "aGk=", &["-p", "uri", "--front-matter=extract"])?;
    assert_eq!(
        (err.trim_end(), code),
        (
            "Error: --front-matter and --input-format uri are incompatible",
            1
        )
    );
    Ok(())
}
