//! #6925: `succinctly yq -p base64` and `-p uri`, each of which decodes the whole input to one string
//! as yq's decoder does and then reads it as `-p json` reads a JSON string. Every answer was captured
//! from yq v4.53.3 (its `-p` deprecation warning on stderr left out). The rows that still differ are in
//! `docs/compliance/yq/limitations.md` (`-p base64|uri`): YAML output of decoded bytes that are not
//! UTF-8, and `--front-matter`. `-p props`, which reads a properties file as the YAML yq prints for it,
//! is at the end.

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

/// The error line of `stderr`: a run with `RUST_BACKTRACE` set (the coverage job) adds anyhow's
/// backtrace after it.
fn error_line(stderr: &str) -> &str {
    stderr.lines().next().unwrap_or("")
}

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
    (
        "base64",
        "YSJiXGMNCWQ=",
        &["-o", "json"],
        ".",
        "\"a\\\"b\\\\c\\r\\td\"\n",
        "",
        0,
    ),
    ("base64", "Pj4+", &["-o", "json"], ".", "\">>>\"\n", "", 0),
    (
        "base64",
        "====",
        &[],
        ".",
        "",
        "Error: bad file '-': illegal base64 data at input byte 0",
        1,
    ),
    (
        "base64",
        "Q===",
        &[],
        ".",
        "",
        "Error: bad file '-': illegal base64 data at input byte 1",
        1,
    ),
    (
        "base64",
        "QQ=Q",
        &[],
        ".",
        "",
        "Error: bad file '-': illegal base64 data at input byte 2",
        1,
    ),
];

#[test]
fn string_input_formats_match_yq_on_stdin_6925() -> Result<()> {
    for &(format, input, extra, filter, stdout, stderr, code) in ROWS {
        let mut args = vec!["-p", format];
        args.extend_from_slice(extra);
        let (out, err, status) = run_yq_stdin_with_stderr(filter, input, &args)?;
        let row = format!("-p {format} {extra:?} {filter:?} on {input:?}");
        assert_eq!(out, stdout, "stdout of {row}");
        assert_eq!(error_line(&err), stderr, "stderr of {row}");
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
    // A leading byte that is not UTF-8 is not white space to Go's `TrimSpace`, so it is data.
    let (_, err, code) =
        run_yq_stdin_bytes_with_stderr(".", b"\xffaGk=", &["-p", "base64", "-o", "json"])?;
    assert_eq!(
        (error_line(&err), code),
        (
            "Error: bad file '-': illegal base64 data at input byte 0",
            1
        )
    );
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
        (error_line(&err), code),
        (
            "Error: bad file '-': illegal base64 data at input byte 404",
            1
        )
    );
    let mut long = "enp6".repeat(2000).into_bytes();
    long[5000] = b'!';
    let (_, err, code) = run_yq_stdin_bytes_with_stderr("length", &long, &["-p", "base64"])?;
    assert_eq!(
        (error_line(&err), code),
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
        error_line(&err),
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
        (error_line(&err), code),
        (
            "Error: --front-matter and --input-format uri are incompatible",
            1
        )
    );
    Ok(())
}

/// Review of #6925: the format comes from the first *input* file, which under `--from-file` is the
/// first positional argument, and Go's `filepath.Ext` gives a name that starts with its only dot
/// that whole name (`.base64`, `.json`). yq v4.53.3 decodes both.
#[test]
fn the_format_comes_from_the_first_input_file_as_go_names_its_extension_6925() -> Result<()> {
    let dir = TempDir::new()?;
    let program = write_file(&dir, "prog.yq", b".")?;
    let a = write_file(&dir, "a.base64", b"aGk=")?;
    let dotted = write_file(&dir, ".base64", b"aGk=")?;
    let dotted_json = write_file(&dir, ".json", b"{\"a\":1}")?;
    let program = program.display().to_string();
    // Under `--from-file` the filter's slot holds the first input file.
    let a_name = a.display().to_string();
    assert_eq!(
        run_yq_files(&a_name, &[], &["-o", "json", "--from-file", &program])?.0,
        "\"hi\"\n"
    );
    assert_eq!(
        run_yq_files(".", &[&dotted], &["-o", "json"])?.0,
        "\"hi\"\n"
    );
    assert_eq!(
        run_yq_files(".", &[&dotted_json], &[])?.0,
        "{\n  \"a\": 1\n}\n"
    );
    // `--front-matter` on a file picked by its extension reads its front matter as YAML, as for a
    // `*.json` file, and is not refused. yq prints `aGk=` too (without the final line break: its
    // pass-through of a file with no front matter, which this change does not touch).
    let (out, err, code) = run_yq_files(".", &[&a], &["--front-matter", "process"])?;
    assert_eq!((out.trim_end(), err.as_str(), code), ("aGk=", "", 0));
    Ok(())
}

/// `-R` (a succinctly extension) reads raw lines whatever `-p` says, on every route.
#[test]
fn raw_input_ignores_a_string_format_6925() -> Result<()> {
    let (out, _, code) = run_yq_stdin_with_stderr(".", "aGk=", &["-R", "-p", "base64"])?;
    assert_eq!((out.as_str(), code), ("aGk=\n", 0));
    // The helper puts the filter last, so it is the `-` here.
    let (out, _, code) = run_yq_stdin_with_stderr("-", "aGk=", &["-R", "-p", "base64", "."])?;
    assert_eq!((out.as_str(), code), ("aGk=\n", 0));
    Ok(())
}

/// yq-mode `@urid` and `-p uri` share one `url.QueryUnescape`, whose bad-escape message Go-quotes
/// its bytes: a control character is `\x01`, not Rust's `\u{1}` (yq v4.53.3).
#[test]
fn urid_quotes_a_bad_escape_as_go_does_6925() -> Result<()> {
    let (_, err, code) = run_yq_stdin_with_stderr("@urid", "\"x%\\u0001\"", &["-p", "json"])?;
    assert_eq!(
        (error_line(&err), code),
        (r#"Error: invalid URL escape "%\x01""#, 1)
    );
    let (_, err, code) = run_yq_stdin_bytes_with_stderr(".", b"x%\x7f", &["-p", "uri"])?;
    assert_eq!(
        (error_line(&err), code),
        (r#"Error: bad file '-': invalid URL escape "%\x7f""#, 1)
    );
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// `-p props` (#6925): magiconair/properties as yq's `decoder_properties.go` builds its nodes. Each
// row was captured from yq v4.53.3 on stdin; the rows that still differ (comments under `-o props`,
// blank lines ending a comment, explicit `? ` keys, the quoting a write keeps) are in
// `docs/compliance/yq/limitations.md` (`-p props`).
// ---------------------------------------------------------------------------------------------

/// (stdin, extra args, filter, yq's stdout, yq's first stderr line, yq's exit status).
type PropsRow = (
    &'static str,
    &'static [&'static str],
    &'static str,
    &'static str,
    &'static str,
    i32,
);

#[rustfmt::skip]
const PROPS_ROWS: &[PropsRow] = &[
    ("# top\na=1\nb.c=true\nb.d = hello world\n# about e\ne.0=x\ne.1=\nf: ~\ng=${a}\n", &[], ".", "# top\na: \"1\"\nb:\n  c: \"true\"\n  d: hello world\ne:\n  # about e\n  - x\n  - \"\"\nf: \"~\"\ng: ${a}\n", "", 0),
    ("# top\na=1\nb.c=true\n# about e\ne.0=x\n", &["-o", "json"], ".", "{\n  \"a\": \"1\",\n  \"b\": {\n    \"c\": \"true\"\n  },\n  \"e\": [\n    \"x\"\n  ]\n}\n", "", 0),
    ("a=1\na.b=2\n", &[], ".", "a: \"1\"\n", "", 0),
    ("a.b=2\na=1\n", &[], ".", "a: \"1\"\n", "", 0),
    ("e.1=y\ne.0=x\n", &[], ".", "e:\n  - x\n  - y\n", "", 0),
    ("e.3=x\n", &[], ".", "e:\n  - null\n  - null\n  - null\n  - x\n", "", 0),
    ("e.+1=x\ne.01=y\n", &[], ".", "e:\n  - null\n  - y\n", "", 0),
    ("0.a=x\n+1=y\n", &[], ".", "\"0\":\n  a: x\n\"1\": y\n", "", 0),
    ("a.b=y\na.0=x\n", &[], ".", "a:\n  b: y\n  \"0\": x\n", "", 0),
    ("a..b=x\n.c=y\nd.=z\n", &[], ".", "a:\n  \"\":\n    b: x\n\"\":\n  c: y\nd:\n  \"\": z\n", "", 0),
    ("abc=1\nabd=2\nab*=9\nx*=3\n", &[], ".", "abc: \"9\"\nabd: \"9\"\nx*: \"3\"\n", "", 0),
    ("abc=1\n# c\nab*=9\n", &[], ".", "# c\nab*: \"9\"\n", "", 0),
    ("e.[]=z\n", &[], ".", "e: null\n", "", 0),
    ("e.0=x\ne.0x1=y\ne.1_0=z\n", &["-o", "json", "-I", "0"], ".", "{\"e\":[\"x\",\"y\",null,null,null,null,null,null,null,null,\"z\"]}\n", "", 0),
    ("e.0=x\ne.-1=y\n", &[], ".", "e:\n  - y\n", "", 0),
    ("a.0=x\n# c\na.b=y\nz=1\n", &[], ".", "a:\n  - x\nz: \"1\"\n", "", 0),
    ("# c1\n#  c2  \n! c3\na=1\n# tail", &[], ".", "# c1\n# c2  \n# c3\na: \"1\"\n", "", 0),
    ("# a\n##x\n#  # y\na=1\n", &[], ".", "# a\n#x\n# y\na: \"1\"\n", "", 0),
    ("# c\nb.c=x\n# d\ne.0.0=q\n", &[], ".", "b:\n  # c\n  c: x\ne:\n  - # d\n    - q\n", "", 0),
    ("e.0.a=x\n# c\ne.0.b=y\ne.1.0=q\n", &[], ".", "e:\n  - a: x\n    # c\n    b: y\n  - - q\n", "", 0),
    ("# c1\na=1\n# c2\na=2\n# c3\nb.c=1\nb=2\n", &[], ".", "# c2\na: \"2\"\nb: \"2\"\n", "", 0),
    ("x\ny = 2\nk v w\nk2:v\n key3 = \\ lead\n", &[], ".", "x: \"\"\ny: \"2\"\nk: v w\nk2: v\nkey3: ' lead'\n", "", 0),
    ("k=v\\\n   cont\nm=multi\\nline\nt=\\u00e9\\u0000\\t\n", &[], ".", "k: vcont\nm: |-\n  multi\n  line\nt: \"\u{e9}\\0\\t\"\n", "", 0),
    ("a=1\r\nb=x\\\r\n  c\r\n", &[], ".", "a: \"1\"\nb: x\nc: \"\"\n", "", 0),
    ("<<=x\n*a=&b\n!t=x\n---=1\n", &[], ".", "\"<<\": x\n'*a': '&b'\n# t=x\n'---': \"1\"\n", "", 0),
    ("true=yes\nnull=~\nk=0x1f\nl=1_000\nm=2001-12-14\nn=a: b\n", &[], ".", "\"true\": yes\n\"null\": \"~\"\nk: \"0x1f\"\nl: \"1_000\"\nm: \"2001-12-14\"\nn: 'a: b'\n", "", 0),
    ("\n\n", &[], ".", "{}\n", "", 0),
    ("#only\n", &["-o", "json"], ".", "{}\n", "", 0),
    ("", &["-o", "json"], ".", "null\n", "", 0),
    ("e[0]=x\ne[1][0]=y\nf[a]=z\n0[1]=r\n", &["--properties-array-brackets"], ".", "e:\n  - x\n  - - y\nf[a]: z\n\"0\":\n  - null\n  - r\n", "", 0),
    ("e[0]=x\n", &[], ".", "e[0]: x\n", "", 0),
    ("a.b=1\nc=2\n", &[], ".a.b", "1\n", "", 0),
    ("a.b=1\nc=2\n", &[], ".a | keys", "- b\n", "", 0),
    ("a.b=1\n", &[], ".a.b | tag", "!!str\n", "", 0),
    ("a=1\n", &["-o", "props"], ". | .b = \"2\"", "a = 1\nb = 2\n", "", 0),
    ("a.0=x\na.b=y\n", &[], ".", "", "Error: bad file '-': cannot index array with 'b' (strconv.ParseInt: parsing \"b\": invalid syntax)", 1),
    ("e.-1=x\n", &[], ".", "", "Error: bad file '-': index [-1] out of range, array size is 0", 1),
    ("e.0=x\ne.a_b=y\n", &[], ".", "", "Error: bad file '-': cannot index array with 'a_b' (strconv.ParseInt: parsing \"ab\": invalid syntax)", 1),
    ("e.0=x\ne.0xzz=y\n", &[], ".", "", "Error: bad file '-': cannot index array with '0xzz' (strconv.ParseInt: parsing \"zz\": invalid syntax)", 1),
    ("e.0=x\ne.99999999999999999999=y\n", &[], ".", "", "Error: bad file '-': cannot index array with '99999999999999999999' (strconv.ParseInt: parsing \"99999999999999999999\": value out of range)", 1),
    ("k=\\u0041\\x\\", &[], ".", "", "Error: bad file '-': properties: Line 1: premature EOF", 1),
    ("k=\\u00zz\n", &[], ".", "", "Error: bad file '-': properties: Line 1: invalid unicode literal", 1),
    ("a=1\nb=2\n\n=3\n", &[], ".", "", "Error: bad file '-': properties: Line 4: \"3\"", 1),
    ("=50%\n", &[], ".", "", "Error: bad file '-': properties: Line 1: \"50%!\"(MISSING)", 1),
    ("=a%sb%5.2fz%%\n", &[], ".", "", "Error: bad file '-': properties: Line 1: \"a%!s(MISSING)b%!f(MISSING)z\"...", 1),
    ("=abcdefghijklmnop\n", &[], ".", "", "Error: bad file '-': properties: Line 1: \"abcdefghij\"...", 1),
    ("a=${b\n", &[], ".", "", "Error: bad file '-': malformed expression", 1),
    ("a=${b}\nb=${a}%d\n", &[], ".", "", "Error: bad file '-': circular reference in:", 1),
    // Review: each segment runs over every match before the next starts, and a glob matches one
    // key per spelling, the last.
    ("x.a=1\ny.0=2\n*..-1=3\n", &[], ".", "", "Error: bad file '-': cannot index array with '' (strconv.ParseInt: parsing \"\": invalid syntax)", 1),
    ("abc=1\nabd=2\n# c\nab*=9\n# d\nab*=8\n", &[], ".", "# d\nab*: \"1\"\n# d\nab*: \"8\"\n", "", 0),
    ("abc=1\nabd=2\n# c\nab*=9\n", &["-o", "json"], ".", "{\n  \"ab*\": \"1\",\n  \"ab*\": \"9\"\n}\n", "", 0),
];

#[test]
fn props_input_matches_yq_on_stdin_6925() -> Result<()> {
    for &(input, extra, filter, stdout, stderr, code) in PROPS_ROWS {
        let mut args = vec!["-p", "props"];
        args.extend_from_slice(extra);
        let (out, err, status) = run_yq_stdin_with_stderr(filter, input, &args)?;
        let row = format!("-p props {extra:?} {filter:?} on {input:?}");
        assert_eq!(out, stdout, "stdout of {row}");
        assert_eq!(error_line(&err), stderr, "stderr of {row}");
        assert_eq!(status, code, "exit status of {row}");
    }
    Ok(())
}

/// `*.properties`, `*.props` and `*.p` (any case) are props both ways under `-p auto`; an empty
/// file holds no document; `-i` writes the output format back.
#[test]
fn props_by_extension_and_inplace_6925() -> Result<()> {
    let dir = TempDir::new()?;
    let a = write_file(&dir, "a.properties", b"a.b=1\ne.0=x\n")?;
    let b = write_file(&dir, "b.P", b"k=v\n")?;
    let empty = write_file(&dir, "c.props", b"")?;
    let txt = write_file(&dir, "d.txt", b"a.b=1\n")?;
    assert_eq!(
        run_yq_files(".", &[&a], &[])?,
        ("a.b = 1\ne.0 = x\n".into(), String::new(), 0)
    );
    assert_eq!(
        run_yq_files(".", &[&b, &empty, &a], &[])?,
        ("k = v\na.b = 1\ne.0 = x\n".into(), String::new(), 0)
    );
    assert_eq!(
        run_yq_files(".", &[&empty], &["-o", "json"])?,
        ("null\n".into(), String::new(), 0)
    );
    run_yq_files(".a.b = \"2\"", &[&a], &["-i"])?;
    assert_eq!(std::fs::read(&a)?, b"a.b = 2\ne.0 = x\n");
    run_yq_files(".a.b = \"2\"", &[&txt], &["-p", "props", "-i"])?;
    assert_eq!(std::fs::read(&txt)?, b"a:\n  b: \"2\"\n");
    Ok(())
}

/// Recorded divergences: `--front-matter` with `-p props` is refused (yq decodes the front matter
/// as props), and an index that would pad a sequence past 2^24 elements is refused where yq
/// allocates a node for every padded `null` until it runs out of memory.
#[test]
fn props_refusals_6925() -> Result<()> {
    let (_, err, code) =
        run_yq_stdin_with_stderr(".", "a=1\n", &["-p", "props", "--front-matter=process"])?;
    assert_eq!(
        (error_line(&err), code),
        (
            "Error: --front-matter and --input-format props are incompatible",
            1
        )
    );
    let (_, err, code) = run_yq_stdin_with_stderr(".", "e.16777216=x\n", &["-p", "props"])?;
    assert_eq!(
        (error_line(&err), code),
        (
            "Error: bad file '-': Cannot grow array to 16777217 elements",
            1
        )
    );
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// -p csv and -p tsv: Go's encoding/csv, each cell a snippet of YAML. Every answer is yq v4.53.3's.
// ---------------------------------------------------------------------------------------------

/// (format, stdin, extra args, filter, yq's stdout, yq's first stderr line, yq's exit status).
#[rustfmt::skip]
const CSV_ROWS: &[Row] = &[
    ("csv", "name,age\nAlice,30\nBob,0x1f\n", &["-o", "yaml"], ".", "- name: Alice\n  age: 30\n- name: Bob\n  age: 0x1f\n", "", 0),
    ("csv", "name,age\nAlice,30\nBob,25\n", &["-o", "json", "-I0"], ".", "[{\"name\":\"Alice\",\"age\":30},{\"name\":\"Bob\",\"age\":25}]\n", "", 0),
    ("csv", "name,age\nAlice,30\nBob,25\n", &["-o", "yaml"], ".[1].name", "Bob\n", "", 0),
    ("csv", "name,age\nAlice,30\nBob,25\n", &["-o", "yaml"], "length", "2\n", "", 0),
    ("csv", "a,b\n", &["-o", "json", "-I0"], ".", "[]\n", "", 0),
    ("csv", "", &["-o", "json", "-I0"], ".", "null\n", "", 0),
    ("csv", "\n\n", &["-o", "json", "-I0"], ".", "null\n", "", 0),
    ("csv", "a\n\n\n1\n\n2", &["-o", "json", "-I0"], ".", "[{\"a\":1},{\"a\":2}]\n", "", 0),
    ("csv", "a,b\r\n\"x\r\ny\",2\r\n", &["-o", "json", "-I0"], ".", "[{\"a\":\"x\\ny\",\"b\":2}]\n", "", 0),
    ("csv", "a,a\n1,2\n", &["-o", "json", "-I0"], ".", "[{\"a\":1,\"a\":2}]\n", "", 0),
    ("csv", "1,2\n3,4\n", &["-o", "yaml"], ".", "- \"1\": 3\n  \"2\": 4\n", "", 0),
    ("csv", ",\n1,2\n", &["-o", "yaml"], ".", "- \"\": 1\n  \"\": 2\n", "", 0),
    ("csv", "a,b\n,\n", &["-o", "json", "-I0"], ".", "[{\"a\":null,\"b\":null}]\n", "", 0),
    ("csv", "﻿a,b\n1,2\n", &["-o", "json", "-I0"], ".", "[{\"a\":1,\"b\":2}]\n", "", 0),
    ("csv", "a,b\n1\n", &["-o", "json", "-I0"], ".", "", "Error: bad file '-': record on line 2: wrong number of fields", 1),
    ("csv", "a,b\n1,2,3\n", &["-o", "json", "-I0"], ".", "", "Error: bad file '-': record on line 2: wrong number of fields", 1),
    ("csv", "a,b\n1,\"x\n", &["-o", "json", "-I0"], ".", "", "Error: bad file '-': parse error on line 2, column 6: extraneous or missing \" in quoted-field", 1),
    ("csv", "a,b\n1,x\"y\n", &["-o", "json", "-I0"], ".", "", "Error: bad file '-': parse error on line 2, column 4: bare \" in non-quoted-field", 1),
    ("csv", "a,b\n\"1\"x,2\n", &["-o", "json", "-I0"], ".", "", "Error: bad file '-': parse error on line 2, column 3: extraneous or missing \" in quoted-field", 1),
    ("csv", "a,b\n\"1\",\"two\"\nthree,\"fo\"\"ur\"\n", &["-o", "json", "-I0"], ".", "[{\"a\":1,\"b\":\"two\"},{\"a\":\"three\",\"b\":\"fo\\\"ur\"}]\n", "", 0),
    ("csv", "k\n[1,2]\n{a: 1}\na: b\n- x\n\"- a\n- b\"\n", &["-o", "yaml"], ".", "", "Error: bad file '-': record on line 2: wrong number of fields", 1),
    ("csv", "k\n5 # c\n#\n!\n<<\n~\nnull\nTrue\n", &["-o", "yaml"], ".", "- k: 5\n- k: #\n- k:\n- k: !!merge <<\n- k: ~\n- k: null\n- k: True\n", "", 0),
    ("csv", "k\n1e3\n.5\n1.\n+5\n007\n0o17\n1_000\n0x1f\n", &["-o", "yaml"], ".", "- k: 1e3\n- k: .5\n- k: 1.\n- k: +5\n- k: 007\n- k: 0o17\n- k: 1_000\n- k: 0x1f\n", "", 0),
    ("csv", "k\n'x'\n\"\"\"x\"\"\"\n!!str 5\n&a x\n*a\na # c\n| x\n%x\nyes\non\ny\n", &["-o", "yaml"], ".", "- k: '''x'''\n- k: '\"x\"'\n- k: '!!str 5'\n- k: '&a x'\n- k: '*a'\n- k: 'a # c'\n- k: '| x'\n- k: '%x'\n- k: yes\n- k: on\n- k: y\n", "", 0),
    ("csv", "k\n 1\n2 \n x \n", &["-o", "json", "-I0"], ".", "[{\"k\":1},{\"k\":2},{\"k\":\" x \"}]\n", "", 0),
    ("csv", "k\n&a 5\n!!int 5\n!foo 5\n!!null x\n&a [1]\n!!seq [1]\n", &["-o", "yaml"], ".", "- k: &a 5\n- k: !!int 5\n- k: !foo 5\n- k: !!null x\n- k: &a [1]\n- k: !!seq [1]\n", "", 0),
    ("csv", "k\n2001-01-01\n2001-01-01T00:00:00Z\n1:30\n12e\n", &["-o", "yaml"], ".", "- k: 2001-01-01\n- k: 2001-01-01T00:00:00Z\n- k: 1:30\n- k: 12e\n", "", 0),
    ("csv", "k\nline1\nline2\n\"line1\nline2\"\n\"x\n\"\n\"a: 1\nb: 2\"\n", &["-o", "yaml"], ".", "- k: line1\n- k: line2\n- k: |-\n    line1\n    line2\n- k: |\n    x\n- k:\n    a: 1\n    b: 2\n", "", 0),
    ("csv", "k\n\"{a: [1, {b: 2}]}\"\n\"[a, 'b', \"\"c\"\"]\"\n\"a: 'b'\nc: \"\"d\"\"\"\n", &["-o", "yaml"], ".", "- k: {a: [1, {b: 2}]}\n- k: [a, 'b', \"c\"]\n- k:\n    a: 'b'\n    c: \"d\"\n", "", 0),
    ("csv", "k\n\"k: # c\n  v\"\n\"- a\n- b # c\"\n\"foo: &x 1\nbar: *x\"\n", &["-o", "yaml"], ".", "- k:\n    k: v # c\n- k:\n    - a\n    - b # c\n- k:\n    foo: &x 1\n    bar: *x\n", "", 0),
    ("csv", "1,x y,\"a\nb\"\n1,2,3\n", &["-o", "yaml"], ".", "- \"1\": 1\n  x y: 2\n  ? |-\n    a\n    b\n  : 3\n", "", 0),
    ("csv", "1,[x],k:\n!!,!,-\n", &["-o", "yaml"], ".", "- \"1\": '!!'\n  '[x]':\n  'k:':\n    -\n", "", 0),
    ("csv", "a,b\n1,[1]\n", &["-o", "json", "-I0"], "[.[] | .b]", "[[1]]\n", "", 0),
    ("csv", "a;b\n1;\"x;y\"\n", &["-o", "json", "-I0", "--csv-separator", ";"], ".", "[{\"a\":1,\"b\":\"x;y\"}]\n", "", 0),
    ("csv", "a,b\n1,[1]\n  2, true\n", &["-o", "json", "-I0", "--csv-auto-parse=false"], ".", "[{\"a\":1,\"b\":\"[1]\"},{\"a\":\"  2\",\"b\":\" true\"}]\n", "", 0),
    ("csv", "k\n1\n01\n0x1f\n[1]\na: b\nyes\n2001-01-01\n true\n", &["-o", "json", "-I0", "--csv-auto-parse=false"], ".", "[{\"k\":1},{\"k\":1},{\"k\":31},{\"k\":\"[1]\"},{\"k\":\"a: b\"},{\"k\":\"yes\"},{\"k\":\"2001-01-01\"},{\"k\":\" true\"}]\n", "", 0),
    ("csv", "k\n1\n01\n0x1f\n[1]\na: b\nyes\n2001-01-01\n true\n", &["-o", "yaml", "--csv-auto-parse=false"], ".", "- k: 1\n- k: 01\n- k: 0x1f\n- k: '[1]'\n- k: 'a: b'\n- k: yes\n- k: 2001-01-01\n- k: ' true'\n", "", 0),
    ("csv", "a,b\n1,2\n", &["-o", "json", "-I0", "--tsv-auto-parse=false"], ".", "[{\"a\":1,\"b\":2}]\n", "", 0),
    ("csv", "k\n\u{a0}5\n5\u{a0}\ntrue\u{a0}\n", &["-o", "json", "-I0"], ".", "[{\"k\":\"\u{a0}5\"},{\"k\":\"5\u{a0}\"},{\"k\":\"true\u{a0}\"}]\n", "", 0),
    ("tsv", "name\tage\nAlice\t30\n\"q\tr\"\t[1]\n", &["-o", "json", "-I0"], ".", "[{\"name\":\"Alice\",\"age\":30},{\"name\":\"q\\tr\",\"age\":[1]}]\n", "", 0),
    ("tsv", "name\tage\nAlice\t30\n\"q\tr\"\t[1]\n", &["-o", "json", "-I0", "--tsv-auto-parse=false"], ".", "[{\"name\":\"Alice\",\"age\":30},{\"name\":\"q\\tr\",\"age\":\"[1]\"}]\n", "", 0),
    ("tsv", "a,b\tc\n1,2\t3\n", &["-o", "yaml"], ".", "- a,b: 1,2\n  c: 3\n", "", 0),
    ("tsv", "a\tb\n1\n", &["-o", "json", "-I0"], ".", "", "Error: bad file '-': record on line 2: wrong number of fields", 1),
    ("tsv", "a\tb\n1\t\"x\n", &["-o", "json", "-I0"], ".", "", "Error: bad file '-': parse error on line 2, column 6: extraneous or missing \" in quoted-field", 1),
    ("tsv", "a\tb\n1,2\t3\n", &["-o", "json", "-I0", "--csv-separator", ";"], ".", "[{\"a\":\"1,2\",\"b\":3}]\n", "", 0),
];

#[test]
fn csv_and_tsv_input_match_yq_on_stdin_6925() -> Result<()> {
    for &(format, input, extra, filter, stdout, stderr, code) in CSV_ROWS {
        let mut args = vec!["-p", format];
        args.extend_from_slice(extra);
        let (out, err, status) = run_yq_stdin_with_stderr(filter, input, &args)?;
        let row = format!("-p {format} {extra:?} {filter:?} on {input:?}");
        assert_eq!(out, stdout, "stdout of {row}");
        assert_eq!(error_line(&err), stderr, "stderr of {row}");
        assert_eq!(status, code, "exit status of {row}");
    }
    Ok(())
}

/// `*.csv` and `*.tsv` (and the aliases `c`, `t`, any case) are tables both ways under `-p auto`;
/// an empty file is `null`; `-i` writes the output format back, which is the table's.
#[test]
fn csv_and_tsv_by_extension_and_inplace_6925() -> Result<()> {
    let dir = TempDir::new()?;
    let a = write_file(&dir, "a.csv", b"name,age\nAlice,30\nBob,25\n")?;
    let b = write_file(&dir, "b.TSV", b"name\tage\nCara\t1\n")?;
    let c = write_file(&dir, "c.c", b"k\n[1]\n")?;
    let empty = write_file(&dir, "d.csv", b"")?;
    assert_eq!(
        run_yq_files(".[0].age", &[&a], &["-o", "json"])?,
        ("30\n".into(), String::new(), 0)
    );
    // The first file's format reads every file, as in yq (#4394): `a.csv` read as a TSV has one
    // column.
    assert_eq!(
        run_yq_files(".[0].age", &[&b, &a], &["-o", "json"])?,
        ("1\nnull\n".into(), String::new(), 0)
    );
    assert_eq!(
        run_yq_files(".[0].k", &[&c], &["-o", "json", "-I0"])?,
        ("[1]\n".into(), String::new(), 0)
    );
    assert_eq!(
        run_yq_files(".", &[&empty], &["-o", "json"])?,
        ("null\n".into(), String::new(), 0)
    );
    run_yq_files(".[0].age = 31", &[&a], &["-i"])?;
    assert_eq!(std::fs::read(&a)?, b"name,age\nAlice,31\nBob,25\n");
    run_yq_files(".[0].age = 2", &[&b], &["-i"])?;
    assert_eq!(std::fs::read(&b)?, b"name\tage\nCara\t2\n");
    Ok(())
}
