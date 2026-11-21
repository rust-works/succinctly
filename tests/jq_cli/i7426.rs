//! #7426: a command-line argument that is not valid UTF-8 panicked (exit 101): argv was read with
//! `std::env::args`, which unwraps every word. jq 1.7.1 reads a text word through its string
//! constructor, which replaces each bad sequence with U+FFFD by jq's own rule, and opens a file by
//! the bytes of its name. Every row was captured from jq 1.7.1 (`jq -nc --arg x $'a\xff' '$x |
//! explode'` and the like); the yq rows from yq v4.53.3.
//!
//! macOS refuses to create a file whose name is not UTF-8 (APFS), so the test that opens one is
//! Linux only (the `Test (x86_64)` and `Test (ARM64)` CI legs); the rest run on every Unix.

#![cfg(unix)]

use std::ffi::{OsStr, OsString};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::process::CommandExt;
use std::process::{Command, Output};

use anyhow::Result;

fn word(bytes: &[u8]) -> OsString {
    OsString::from_vec(bytes.to_vec())
}

/// `word` with `prefix` and `suffix` around it.
fn wrapped(prefix: &[u8], bytes: &[u8], suffix: &[u8]) -> OsString {
    word(&[prefix, bytes, suffix].concat())
}

/// The binary run as `argv0` (a multi-call name, or `succinctly` with a subcommand in `words`).
fn run_as(argv0: &str, words: &[OsString]) -> Result<Output> {
    Ok(Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .arg0(argv0)
        .args(words)
        .output()?)
}

fn jq(words: &[OsString]) -> Result<Output> {
    let mut argv = vec![OsString::from("jq")];
    argv.extend_from_slice(words);
    run_as("succinctly", &argv)
}

/// What `output` printed, with its exit status; never a panic.
fn printed(output: &Output) -> (String, Vec<u8>, Option<i32>) {
    assert_ne!(
        output.status.code(),
        Some(101),
        "panicked: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    (
        String::from_utf8_lossy(&output.stdout)
            .trim_end()
            .to_owned(),
        output.stderr.clone(),
        output.status.code(),
    )
}

/// The invalid sequences and what jq makes of each (`explode`).
const ROWS: &[(&[u8], &str)] = &[
    (b"\xff\xfe", "[65533,65533]"),
    (b"\x80", "[65533]"),
    (b"a\xe2\x82", "[97,65533]"),
    (b"\xc0\xaf", "[65533,65533]"),
    (b"\xed\xa0\x80", "[65533]"),
    (b"\xf4\x90\x80\x80", "[65533]"),
    (b"\xe2\x82x", "[65533,120]"),
    (b"\xc3", "[65533]"),
];

#[test]
fn every_text_word_reads_as_jq_reads_it_7426() -> Result<()> {
    for &(bytes, want) in ROWS {
        let cases: Vec<(&str, Vec<OsString>)> = vec![
            (
                "--arg value",
                vec![
                    "-nc".into(),
                    "--arg".into(),
                    "x".into(),
                    word(bytes),
                    "$x | explode".into(),
                ],
            ),
            (
                "--arg name",
                vec![
                    "-nc".into(),
                    "--arg".into(),
                    word(bytes),
                    "1".into(),
                    "$ARGS.named | keys[0] | explode".into(),
                ],
            ),
            (
                "--argjson text",
                vec![
                    "-nc".into(),
                    "--argjson".into(),
                    "x".into(),
                    wrapped(b"\"", bytes, b"\""),
                    "$x | explode".into(),
                ],
            ),
            (
                "--argjson name",
                vec![
                    "-nc".into(),
                    "--argjson".into(),
                    word(bytes),
                    "1".into(),
                    "$ARGS.named | keys[0] | explode".into(),
                ],
            ),
            (
                "--args",
                vec![
                    "-nc".into(),
                    "$ARGS.positional[0] | explode".into(),
                    "--args".into(),
                    word(bytes),
                ],
            ),
            (
                "--jsonargs",
                vec![
                    "-nc".into(),
                    "$ARGS.positional[0] | explode".into(),
                    "--jsonargs".into(),
                    wrapped(b"\"", bytes, b"\""),
                ],
            ),
            (
                "program string",
                vec!["-nc".into(), wrapped(b"\"", bytes, b"\" | explode")],
            ),
            (
                "program object key",
                vec![
                    "-nc".into(),
                    wrapped(b"{\"", bytes, b"\": 1} | keys[0] | explode"),
                ],
            ),
            (
                "program after --args",
                vec![
                    "-nc".into(),
                    "--args".into(),
                    wrapped(b"\"", bytes, b"\" | explode"),
                ],
            ),
        ];
        for (what, words) in cases {
            let (stdout, stderr, code) = printed(&jq(&words)?);
            assert_eq!(
                (stdout.as_str(), code),
                (want, Some(0)),
                "{what} {bytes:?}: {}",
                String::from_utf8_lossy(&stderr)
            );
        }
        // A comment, and a regex matched against the U+FFFD text it reads as.
        let (stdout, _, code) = printed(&jq(&["-nc".into(), wrapped(b"1 # ", bytes, b"")])?);
        assert_eq!((stdout.as_str(), code), ("1", Some(0)), "comment {bytes:?}");
    }
    let (stdout, _, _) = printed(&jq(&[
        "-nc".into(),
        word(b"\"a\xef\xbf\xbd\" | test(\"a\xff\")"),
    ])?);
    assert_eq!(stdout, "true");
    Ok(())
}

/// jq reads `--indent` with `atoi`, so a value that starts with a bad byte is 0.
#[test]
fn an_indent_that_is_not_utf8_is_zero_7426() -> Result<()> {
    let (stdout, _, code) = printed(&jq(&[
        "-n".into(),
        "--indent".into(),
        word(b"\xff"),
        "[1]".into(),
    ])?);
    assert_eq!((stdout.as_str(), code), ("[1]", Some(0)));
    Ok(())
}

/// A file is opened by the bytes of its name, which jq prints as they are where it `fprintf`s
/// the word and as U+FFFD text where it builds a string of it.
#[test]
fn a_file_name_keeps_its_bytes_7426() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let missing = |name: &[u8]| {
        let mut path = dir.path().as_os_str().as_bytes().to_vec();
        path.push(b'/');
        path.extend_from_slice(name);
        path
    };
    // A file named by the U+FFFD text of `nof\xff` exists, so opening by the lossy text would
    // find it: jq does not, and neither may this.
    std::fs::write(OsStr::from_bytes(&missing(b"nof\xef\xbf\xbd")), "\"lossy\"")?;
    let raw = missing(b"nof\xff");
    let lossy = missing(b"nof\xef\xbf\xbd");
    let raw_word = || word(&raw);

    // An input file: `fprintf`ed.
    let (stdout, stderr, code) = printed(&jq(&["-c".into(), ".".into(), raw_word()])?);
    assert_eq!(stdout, "");
    assert_eq!(code, Some(2));
    assert_eq!(
        stderr,
        [
            b"jq: error: Could not open file ".as_slice(),
            &raw,
            b": No such file or directory\n"
        ]
        .concat()
    );

    // `-f`: a string.
    let (_, stderr, code) = printed(&jq(&["-n".into(), "-f".into(), raw_word()])?);
    assert_eq!(code, Some(2));
    assert_eq!(
        stderr,
        [
            b"jq: Could not open ".as_slice(),
            &lossy,
            b": No such file or directory\n"
        ]
        .concat()
    );

    // `--rawfile`/`--slurpfile`: the prefix `fprintf`ed, the detail a string.
    for flag in ["--rawfile", "--slurpfile"] {
        let (_, stderr, code) = printed(&jq(&[
            "-n".into(),
            flag.into(),
            word(b"n\xff"),
            raw_word(),
            "1".into(),
        ])?);
        assert_eq!(code, Some(2), "{flag}");
        assert_eq!(
            stderr,
            [
                b"jq: Bad JSON in ".as_slice(),
                flag.as_bytes(),
                b" n\xff ",
                &raw,
                b": Could not open ",
                &lossy,
                b": No such file or directory\n"
            ]
            .concat(),
            "{flag}"
        );
    }
    // A name that is not UTF-8 binds as its U+FFFD text.
    let (stdout, _, code) = printed(&jq(&[
        "-nc".into(),
        "--rawfile".into(),
        word(b"n\xff"),
        word(&lossy),
        "$ARGS.named | to_entries[0] | [(.key | explode), .value]".into(),
    ])?);
    assert_eq!(
        (stdout.as_str(), code),
        ("[[110,65533],\"\\\"lossy\\\"\"]", Some(0))
    );
    Ok(())
}

/// On Linux a file can be named by bytes that are not UTF-8: it opens, and `input_filename` is
/// its U+FFFD text.
#[cfg(target_os = "linux")]
#[test]
fn a_file_named_by_bytes_that_are_not_utf8_opens_7426() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let mut path = dir.path().as_os_str().as_bytes().to_vec();
    path.extend_from_slice(b"/f\xed\xa0\x80.json");
    std::fs::write(OsStr::from_bytes(&path), "{\"a\":1}")?;
    for program in [".a", "input_filename | explode | .[-8:]"] {
        for flag in ["-c", "--seq"] {
            let (stdout, stderr, code) = printed(&jq(&[flag.into(), program.into(), word(&path)])?);
            let want = if program == ".a" {
                "1"
            } else {
                // `/f`, one U+FFFD, `.json`
                "[47,102,65533,46,106,115,111,110]"
            };
            assert_eq!(
                (stdout.trim_start_matches('\u{1e}'), code),
                (want, Some(0)),
                "{flag} {program}: {}",
                String::from_utf8_lossy(&stderr)
            );
        }
    }
    let (stdout, _, code) = printed(&jq(&[
        "-nc".into(),
        "--rawfile".into(),
        "x".into(),
        word(&path),
        "$x".into(),
    ])?);
    assert_eq!((stdout.as_str(), code), ("\"{\\\"a\\\":1}\"", Some(0)));
    let mut program = path.clone();
    program.truncate(program.len() - ".json".len());
    program.extend_from_slice(b".jq");
    std::fs::write(OsStr::from_bytes(&program), ".a + 1")?;
    let (stdout, _, code) = printed(&jq(&["-f".into(), word(&program), word(&path)])?);
    assert_eq!((stdout.as_str(), code), ("2", Some(0)));
    Ok(())
}

/// Every alias and subcommand reads argv without panicking: `sjq`/`jq` as `succinctly jq` does,
/// `syq`/`yq` as yq does (each bad byte one U+FFFD), and the locate commands and the rest of the
/// tree (clap reads argv as the operating system's words).
#[test]
fn no_alias_or_subcommand_panics_on_a_word_that_is_not_utf8_7426() -> Result<()> {
    for argv0 in ["sjq", "jq"] {
        let (stdout, _, code) = printed(&run_as(
            argv0,
            &["-nc".into(), word(b"\"\xed\xa0\x80\" | explode")],
        )?);
        assert_eq!((stdout.as_str(), code), ("[65533]", Some(0)), "{argv0}");
    }
    // yq v4.53.3: `"a\xed\xa0\x80" | length` is 10 (bytes: `a` and three U+FFFD).
    let yq_program = word(b"\"a\xed\xa0\x80\" | length");
    for (argv0, mut words) in [
        ("syq", vec![]),
        ("yq", vec![]),
        ("succinctly", vec![OsString::from("yq")]),
    ] {
        words.extend(["-n".into(), yq_program.clone()]);
        let (stdout, stderr, code) = printed(&run_as(argv0, &words)?);
        assert_eq!(
            (stdout.as_str(), code),
            ("10", Some(0)),
            "{argv0}: {}",
            String::from_utf8_lossy(&stderr)
        );
    }
    let (stdout, _, _) = printed(&run_as("syq", &["-n".into(), word(b"\"a\xff\"")])?);
    assert_eq!(stdout, "a\u{FFFD}");

    // Each of these fails to find its file (or refuses the word), cleanly.
    for (argv0, words) in [
        (
            "sjq-locate",
            vec![word(b"f\xff"), "--offset".into(), "1".into()],
        ),
        (
            "syq-locate",
            vec![word(b"f\xff"), "--offset".into(), "1".into()],
        ),
        (
            "succinctly",
            vec![
                "jq-locate".into(),
                word(b"f\xff"),
                "--offset".into(),
                "1".into(),
            ],
        ),
        (
            "succinctly",
            vec!["json".into(), "validate".into(), word(b"f\xff")],
        ),
        ("sjq", vec![word(b"--wat\xff")]),
        ("succinctly", vec![word(b"\xff")]),
    ] {
        let (_, stderr, code) = printed(&run_as(argv0, &words)?);
        assert_ne!(code, Some(0), "{argv0} {words:?}");
        assert!(!stderr.is_empty(), "{argv0} {words:?}");
    }
    // A program name that is not UTF-8 is no alias, and still runs the tree.
    let (stdout, _, code) = printed(
        &Command::new(env!("CARGO_BIN_EXE_succinctly"))
            .arg0(OsStr::from_bytes(b"s\xffjq"))
            .args(["jq", "-n", "1"])
            .output()?,
    );
    assert_eq!((stdout.as_str(), code), ("1", Some(0)));
    Ok(())
}
