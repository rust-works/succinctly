//! #7769: a `--slurpfile` file or a data import (`import "d" as $d;`) that jq's parser refuses is
//! refused in jq's words, the parser's message and the line and column it stops at, inside jq's
//! own wrappers (`Bad JSON in --slurpfile NAME FILE: ...`, exit 2; `error loading data file PATH:
//! ...`, exit 3). They used to carry `serde_json`'s or the splitter's wording (`Invalid JSON in
//! stream: EOF while parsing a value at line 1 column 3`).
//!
//! A NUL outside a string is a fault at its own column (#7966, ADR-0018's #7963 amendment E2),
//! worded `Invalid character` as jq master (unreleased; commit fbee8f4) words it: jq 1.7.1 and
//! 1.8.2 read a number glued to a NUL up to the NUL (`1.5e3`, a NUL is `[1.5E+3]`), and refuse a
//! token that starts with one one byte later than the NUL. Those rows are [`REFUSED_AS_E2`].
//!
//! Every row was captured from jq 1.7.1 (`/usr/bin/jq`, macOS); jq 1.8.2 answers the same except
//! where a row says otherwise (a 1.8 rewording, and its 10,000-level parse depth).

use anyhow::Result;

/// (file bytes, what jq's parser says of it). jq 1.7.1 and 1.8.2 agree on each row.
const REFUSED: &[(&[u8], &str)] = &[
    // The issue's two files.
    (b"[1,\n", "Unfinished JSON term at EOF at line 2, column 0"),
    (
        b"{\"a\" 1}\n",
        "Expected separator between values at line 1, column 7",
    ),
    // A stray or mismatched closer.
    (b"[1]]\n", "Unmatched ']' at line 1, column 4"),
    (b"]\n", "Unmatched ']' at line 1, column 1"),
    (
        b"{\"a\":[1,2}\n",
        "Objects must consist of key:value pairs at line 1, column 10",
    ),
    // Bad literals.
    (b"nul\n", "Invalid literal at line 2, column 0"),
    (b"tru\n", "Invalid literal at line 2, column 0"),
    (
        b"true false x\n",
        "Invalid numeric literal at line 2, column 0",
    ),
    (b"1e\n", "Invalid numeric literal at line 2, column 0"),
    (b"-\n", "Invalid numeric literal at line 2, column 0"),
    (b"1 # c\n", "Invalid numeric literal at line 1, column 4"),
    (b"[1,2]x\n", "Invalid numeric literal at line 2, column 0"),
    // Missing separators, trailing commas, keys and colons.
    (
        b"[1 2]\n",
        "Expected separator between values at line 1, column 5",
    ),
    (
        b"{\"a\":1 \"b\":2}\n",
        "Expected separator between values at line 1, column 10",
    ),
    (
        b"[1,]\n",
        "Expected another array element at line 1, column 4",
    ),
    (
        b"{\"a\":1,}\n",
        "Expected another key-value pair at line 1, column 8",
    ),
    (
        b"{1:2}\n",
        "Object keys must be strings at line 1, column 3",
    ),
    (
        b"[1:2]\n",
        "':' not as part of an object at line 1, column 3",
    ),
    // Strings and escapes.
    (b"\"abc\n", "Unfinished string at EOF at line 2, column 0"),
    (b"\"\\x\"\n", "Invalid escape at line 1, column 4"),
    (
        b"\"\\ud800\"\n",
        "Invalid \\uXXXX\\uXXXX surrogate pair escape at line 1, column 8",
    ),
    // Bytes that are not UTF-8 outside a string: one column each, as jq counts the bytes.
    (b"\xff\n", "Invalid numeric literal at line 2, column 0"),
    (
        b"[\xff, 1 2]\n",
        "Invalid numeric literal at line 1, column 3",
    ),
    (
        b"\"\xff\xff\" x\n",
        "Invalid numeric literal at line 2, column 0",
    ),
    // Several documents, the fault in the second or the third.
    (
        b"1\n2 [\n",
        "Unfinished JSON term at EOF at line 3, column 0",
    ),
    (b"1\n2\n{\"a\":}\n", "Unmatched '}' at line 3, column 6"),
    (
        b"1\n[\n\n",
        "Unfinished JSON term at EOF at line 4, column 0",
    ),
    // jq skips one byte-order mark at the front and counts no column for it; a second is a byte.
    (
        b"\xef\xbb\xbf[1,\n",
        "Unfinished JSON term at EOF at line 2, column 0",
    ),
    (
        b"\xef\xbb\xbf[1 2]\n",
        "Expected separator between values at line 1, column 5",
    ),
    (
        b" \xef\xbb\xbf[1]\n",
        "Invalid numeric literal at line 1, column 5",
    ),
    (
        b"\xef\xbb\xbf\xef\xbb\xbf[1]\n",
        "Invalid numeric literal at line 1, column 4",
    ),
];

/// Rows where jq 1.8.2 differs from 1.7.1, which is the reference for both: 1.8.2 words a quote
/// it refuses (`Invalid string literal; expected ", but got ' ...`), a 1.8 rewording, not a fix.
const REFUSED_1_7_1_ONLY: &[(&[u8], &str)] =
    &[(b"'a'\n", "Invalid numeric literal at line 2, column 0")];

/// Rows that follow jq 1.8.2: a NaN with a payload is not a number (#7522, recorded in
/// `docs/compliance/jq/limitations.md`), where jq 1.7.1 reads `nan1` as `null`.
const REFUSED_AS_1_8_2: &[(&[u8], &str)] = &[(
    b"nan1",
    "Invalid numeric literal at EOF at line 1, column 4",
)];

/// (file bytes, `$x`). jq 1.7.1 and 1.8.2 agree on each row.
const ACCEPTED: &[(&[u8], &str)] = &[
    (b"", "[]"),
    (b" \n\t\n", "[]"),
    (b"007\n", "[7]"),
    (b"\"a\xffb\"\n", "[\"a\u{fffd}b\"]"),
    (b"{\"a\":1}{\"b\":2}\n", "[{\"a\":1},{\"b\":2}]"),
    (b"nan\n", "[null]"),
];

/// (file bytes, the fault) for a NUL outside a string (#7966): `Invalid character` at the NUL's own
/// column, captured from a driver over jq master's `jv_parse.c`. Neither release refuses these as
/// this does: jq 1.7.1 and 1.8.2 say `Invalid numeric literal at line 1, column 6`, `... at EOF at
/// line 1, column 4` and `... at line 2, column 0` for the first three (the NUL is a token byte to
/// them), and read the last four as `[1]`, `[1.5E+3]`, `[-1.5E+3]` and `[1,"a"]` (their number
/// parser reads the token as a C string).
const REFUSED_AS_E2: &[(&[u8], &str)] = &[
    (b"[1,\x002]\n", "Invalid character at line 1, column 4"),
    (b"[1,\x00", "Invalid character at line 1, column 4"),
    (b"\x00\n", "Invalid character at line 1, column 1"),
    (b"1\x00\n", "Invalid character at line 1, column 2"),
    (b"1.5e3\x00", "Invalid character at line 1, column 6"),
    (b"-1.5e3\x00", "Invalid character at line 1, column 7"),
    (b"01\x00null-\"a\"", "Invalid character at line 1, column 3"),
];

fn run(dir: &std::path::Path, args: &[&str]) -> Result<(String, String, Option<i32>)> {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .arg("jq")
        .args(args)
        .current_dir(dir)
        .env("LC_ALL", "C")
        .env_remove("JQ_LIBRARY_PATH")
        .output()?;
    Ok((
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    ))
}

fn slurpfile(dir: &std::path::Path) -> Result<(String, String, Option<i32>)> {
    run(dir, &["-nc", "--slurpfile", "x", "d.json", "$x"])
}

fn import(dir: &std::path::Path) -> Result<(String, String, Option<i32>)> {
    run(dir, &["-nc", "-L", ".", "import \"d\" as $x; $x"])
}

fn check_refused(dir: &std::path::Path, bytes: &[u8], detail: &str) -> Result<()> {
    std::fs::write(dir.join("d.json"), bytes)?;
    let label = String::from_utf8_lossy(bytes);
    assert_eq!(
        slurpfile(dir)?,
        (
            String::new(),
            format!("jq: Bad JSON in --slurpfile x d.json: {detail}\n"),
            Some(2)
        ),
        "--slurpfile {label:?}"
    );
    // jq names the data file by its full path, which is the temporary directory's here.
    let (stdout, stderr, status) = import(dir)?;
    assert_eq!((stdout.as_str(), status), ("", Some(3)), "import {label:?}");
    assert!(
        stderr.starts_with("jq: error loading data file ")
            && stderr.ends_with(&format!("/d.json: {detail}\n\njq: 1 compile error\n")),
        "import {label:?}: {stderr}"
    );
    Ok(())
}

#[test]
fn a_refused_file_is_worded_as_jqs_parser_words_it_7769() -> Result<()> {
    let dir = tempfile::tempdir()?;
    for &(bytes, detail) in REFUSED
        .iter()
        .chain(REFUSED_1_7_1_ONLY)
        .chain(REFUSED_AS_1_8_2)
        .chain(REFUSED_AS_E2)
    {
        check_refused(dir.path(), bytes, detail)?;
    }
    Ok(())
}

/// The parser keeps its place across jq's 4,096-byte reads of the file.
#[test]
fn a_fault_past_the_first_read_is_placed_as_jq_places_it_7769() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let mut array = b"[".to_vec();
    array.extend(b"1,".repeat(3000));
    array.extend(b"]\n");
    check_refused(
        dir.path(),
        &array,
        "Expected another array element at line 1, column 6002",
    )?;
    let mut lines = b"{\"a\":[1,2,3]}\n".repeat(600);
    lines.extend(b"{\"a\" 1}\n");
    check_refused(
        dir.path(),
        &lines,
        "Expected separator between values at line 601, column 7",
    )?;
    // Each byte that is not UTF-8 is one column, however many strings hold one.
    let mut strings = b"[".to_vec();
    strings.extend(b"\"\xff\",".repeat(2000));
    strings.extend(b"1 2]");
    check_refused(
        dir.path(),
        &strings,
        "Expected separator between values at line 1, column 8005",
    )
}

/// The nesting limit is jq 1.7.1's, 256 levels (jq 1.8.2 parses to 10,000).
#[test]
fn a_file_past_the_depth_limit_is_refused_at_jq_1_7_1s_limit_7769() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let mut deep = b"[".repeat(300);
    deep.extend(b"]".repeat(300));
    deep.push(b'\n');
    check_refused(
        dir.path(),
        &deep,
        "Exceeds depth limit for parsing at line 1, column 257",
    )
}

#[test]
fn an_accepted_file_binds_jqs_values_7769() -> Result<()> {
    let dir = tempfile::tempdir()?;
    for &(bytes, value) in ACCEPTED {
        std::fs::write(dir.path().join("d.json"), bytes)?;
        let want = (format!("{value}\n"), String::new(), Some(0));
        let label = String::from_utf8_lossy(bytes);
        assert_eq!(slurpfile(dir.path())?, want, "--slurpfile {label:?}");
        assert_eq!(import(dir.path())?, want, "import {label:?}");
    }
    Ok(())
}

/// The wrappers around a file that does not open, and the flags that parse text rather than a
/// file, which name no fault (jq 1.7.1; 1.8.2 words its help trailer's link differently).
#[test]
fn a_file_that_does_not_open_and_a_bad_argument_keep_jqs_wording_7769() -> Result<()> {
    let dir = tempfile::tempdir()?;
    std::fs::create_dir(dir.path().join("adir"))?;
    for flag in ["--slurpfile", "--rawfile"] {
        for (file, detail) in [
            ("adir", "Could not open adir: It's a directory"),
            (
                "nope.json",
                "Could not open nope.json: No such file or directory",
            ),
        ] {
            assert_eq!(
                run(dir.path(), &["-n", flag, "x", file, "$x"])?,
                (
                    String::new(),
                    format!("jq: Bad JSON in {flag} x {file}: {detail}\n"),
                    Some(2)
                ),
                "{flag} {file}"
            );
        }
    }
    // `--rawfile` reads the text, faults and all, with U+FFFD for a byte that is not UTF-8.
    std::fs::write(dir.path().join("d.json"), b"[\xff, 1 2]\n")?;
    assert_eq!(
        run(dir.path(), &["-n", "--rawfile", "x", "d.json", "$x"])?,
        (
            "\"[\u{fffd}, 1 2]\\n\"\n".to_string(),
            String::new(),
            Some(0)
        )
    );
    for (flag, args) in [
        ("argjson", vec!["-n", "--argjson", "x", "[1,", "$x"]),
        ("argjson", vec!["-n", "--argjson", "x", "1 2", "$x"]),
        ("argjson", vec!["-n", "--argjson", "x", "", "$x"]),
        ("jsonargs", vec!["-n", "$ARGS", "--jsonargs", "[1,"]),
        ("jsonargs", vec!["-n", "$ARGS", "--jsonargs", "1 2"]),
    ] {
        let (stdout, stderr, status) = run(dir.path(), &args)?;
        assert_eq!((stdout.as_str(), status), ("", Some(2)), "{args:?}");
        assert!(
            stderr.starts_with(&format!(
                "jq: invalid JSON text passed to --{flag}\nUse jq --help for help"
            )),
            "{args:?}: {stderr}"
        );
    }
    Ok(())
}
