//! #7522: a NaN with a payload (`NaN1`, `nan12`, `-sNaN10`) is an invalid numeric literal, as in jq 1.8
//! (CVE-2024-53427, "Reject NaN with payload while parsing JSON"); jq 1.7.1, the pin, reads it as NaN.
//! The target is jq 1.8.2 (`/opt/homebrew/bin/jq`): every row's stdout, stderr and status below was captured
//! from it. `NaN`, `nan`, `-NaN`, `sNaN`, and a payload of zeros (`NaN0`, `NaN000`) are still numbers in both.
//! `tonumber` rejects the same spellings, with this tool's long-standing 1.7.1 wording for an invalid
//! string (`Invalid numeric literal ... (while parsing '...')`; 1.8.2 words it `string ("NaN1") cannot be
//! parsed as a number`, which is a separate, deliberate difference).

use anyhow::Result;

use crate::run_jq_full;

/// `(document, stdout, stderr, status)` for `jq -c .` on stdin.
#[allow(clippy::type_complexity)]
const DOCUMENTS: &[(&str, &str, &str, i32)] = &[
    ("NaN", "null\n", "", 0),
    ("nan", "null\n", "", 0),
    ("-NaN", "null\n", "", 0),
    ("sNaN", "null\n", "", 0),
    ("-sNaN", "null\n", "", 0),
    ("+NaN", "null\n", "", 0),
    ("NaN0", "null\n", "", 0),
    ("NaN000", "null\n", "", 0),
    ("sNaN0", "null\n", "", 0),
    ("-nan00", "null\n", "", 0),
    ("[NaN0,1]", "[null,1]\n", "", 0),
    ("NaN 1", "null\n1\n", "", 0),
    (
        "NaN1",
        "",
        "jq: parse error: Invalid numeric literal at EOF at line 1, column 4\n",
        5,
    ),
    (
        "NaN123",
        "",
        "jq: parse error: Invalid numeric literal at EOF at line 1, column 6\n",
        5,
    ),
    (
        "-NaN1",
        "",
        "jq: parse error: Invalid numeric literal at EOF at line 1, column 5\n",
        5,
    ),
    (
        "nan12",
        "",
        "jq: parse error: Invalid numeric literal at EOF at line 1, column 5\n",
        5,
    ),
    (
        "NAN1",
        "",
        "jq: parse error: Invalid numeric literal at EOF at line 1, column 4\n",
        5,
    ),
    (
        "sNaN12",
        "",
        "jq: parse error: Invalid numeric literal at EOF at line 1, column 6\n",
        5,
    ),
    (
        "sNaN1",
        "",
        "jq: parse error: Invalid numeric literal at EOF at line 1, column 5\n",
        5,
    ),
    (
        "-sNaN1",
        "",
        "jq: parse error: Invalid numeric literal at EOF at line 1, column 6\n",
        5,
    ),
    (
        "+NaN1",
        "",
        "jq: parse error: Invalid numeric literal at EOF at line 1, column 5\n",
        5,
    ),
    (
        "NaN10",
        "",
        "jq: parse error: Invalid numeric literal at EOF at line 1, column 5\n",
        5,
    ),
    (
        "NaN01",
        "",
        "jq: parse error: Invalid numeric literal at EOF at line 1, column 5\n",
        5,
    ),
    (
        "[NaN1]",
        "",
        "jq: parse error: Invalid numeric literal at line 1, column 6\n",
        5,
    ),
    (
        "[1,NaN1,2]",
        "",
        "jq: parse error: Invalid numeric literal at line 1, column 8\n",
        5,
    ),
    (
        "{\"a\":NaN1}",
        "",
        "jq: parse error: Invalid numeric literal at line 1, column 10\n",
        5,
    ),
    (
        "{\"a\":-NaN99}",
        "",
        "jq: parse error: Invalid numeric literal at line 1, column 12\n",
        5,
    ),
    (
        "NaN1 2",
        "",
        "jq: parse error: Invalid numeric literal at line 1, column 5\n",
        5,
    ),
    (
        "[NaN1",
        "",
        "jq: parse error: Invalid numeric literal at EOF at line 1, column 5\n",
        5,
    ),
    (
        "1\nNaN1\n3\n",
        "1\n",
        "jq: parse error: Invalid numeric literal at line 3, column 0\n",
        5,
    ),
    (
        "{\"a\":1}\n{\"a\":NaN1}\n{\"a\":3}\n",
        "{\"a\":1}\n",
        "jq: parse error: Invalid numeric literal at line 2, column 10\n",
        5,
    ),
    // Already refused before #7522, and still: a word that is not decNumber's.
    (
        "NaN.5",
        "",
        "jq: parse error: Invalid numeric literal at EOF at line 1, column 5\n",
        5,
    ),
    (
        "NaNe1",
        "",
        "jq: parse error: Invalid numeric literal at EOF at line 1, column 5\n",
        5,
    ),
];

#[test]
fn a_nan_payload_is_an_invalid_numeric_literal_in_a_document_7522() -> Result<()> {
    for (document, stdout, stderr, status) in DOCUMENTS {
        let (out, err, code) = run_jq_full(&["-c", "."], Some(document))?;
        assert_eq!(
            (out.as_str(), err.as_str(), code),
            (*stdout, *stderr, *status),
            "{document:?}"
        );
    }
    Ok(())
}

/// `--stream`, `-s` and `-n input`: the same message and status as the plain read.
#[test]
fn a_nan_payload_is_refused_when_streamed_slurped_or_read_by_input_7522() -> Result<()> {
    for (args, document, stdout, stderr, status) in [
        (
            vec!["-c", "--stream", "."],
            "[NaN1]",
            "",
            "jq: parse error: Invalid numeric literal at line 1, column 6\n",
            5,
        ),
        (
            vec!["-c", "--stream", "."],
            "[1,NaN1]",
            "[[0],1]\n",
            "jq: parse error: Invalid numeric literal at line 1, column 8\n",
            5,
        ),
        (vec!["-c", "--stream", "."], "NaN0", "[[],null]\n", "", 0),
        (
            vec!["-c", "-s", "."],
            "1 NaN1",
            "",
            "jq: parse error: Invalid numeric literal at EOF at line 1, column 6\n",
            5,
        ),
        (vec!["-c", "-s", "."], "1 NaN0", "[1,null]\n", "", 0),
    ] {
        let (out, err, code) = run_jq_full(&args, Some(document))?;
        assert_eq!(
            (out.as_str(), err.as_str(), code),
            (stdout, stderr, status),
            "{args:?} {document:?}"
        );
    }
    Ok(())
}

/// `--seq`: the record is ignored, with jq's own warning, and the next one is read.
#[test]
fn a_nan_payload_record_is_ignored_under_seq_7522() -> Result<()> {
    for (document, stderr) in [
        (
            "\u{1e}NaN1\n\u{1e}2\n",
            "jq: ignoring parse error: Invalid numeric literal at line 2, column 0 (need RS to resync)\n",
        ),
        (
            "\u{1e}[NaN1,2]\n\u{1e}2\n",
            "jq: ignoring parse error: Invalid numeric literal at line 1, column 7 (need RS to resync)\njq: ignoring parse error: Unmatched ']' at line 1, column 9 (need RS to resync)\n",
        ),
    ] {
        let (out, err, code) = run_jq_full(&["--seq", "-c", "."], Some(document))?;
        assert_eq!(
            (out.as_str(), err.as_str(), code),
            ("\u{1e}2\n", stderr, 0),
            "{document:?}"
        );
    }
    Ok(())
}

/// `fromjson`, with jq 1.8.2's message, and `tonumber`.
#[test]
fn a_nan_payload_string_is_refused_by_fromjson_and_tonumber_7522() -> Result<()> {
    let (out, _, code) = run_jq_full(
        &[
            "-nc",
            r#"["NaN","NaN0","NaN1","nan12","-NaN10","sNaN","sNaN3","+NaN2","[NaN1]","[1,NaN1]","NaN1 ","[NaN1"][] | try fromjson catch ."#,
        ],
        None,
    )?;
    assert_eq!(code, 0);
    assert_eq!(
        out,
        "null\nnull\n\
         \"Invalid numeric literal at EOF at line 1, column 4 (while parsing 'NaN1')\"\n\
         \"Invalid numeric literal at EOF at line 1, column 5 (while parsing 'nan12')\"\n\
         \"Invalid numeric literal at EOF at line 1, column 6 (while parsing '-NaN10')\"\n\
         null\n\
         \"Invalid numeric literal at EOF at line 1, column 5 (while parsing 'sNaN3')\"\n\
         \"Invalid numeric literal at EOF at line 1, column 5 (while parsing '+NaN2')\"\n\
         \"Invalid numeric literal at line 1, column 6 (while parsing '[NaN1]')\"\n\
         \"Invalid numeric literal at line 1, column 8 (while parsing '[1,NaN1]')\"\n\
         \"Invalid numeric literal at line 1, column 5 (while parsing 'NaN1 ')\"\n\
         \"Invalid numeric literal at EOF at line 1, column 5 (while parsing '[NaN1')\"\n"
    );
    let (out, _, code) = run_jq_full(
        &[
            "-nc",
            r#"["NaN","NaN0","NaN1","nan12","-NaN10","sNaN","sNaN3","+NaN2"][] | try tonumber catch "refused""#,
        ],
        None,
    )?;
    assert_eq!(code, 0);
    assert_eq!(
        out,
        "null\nnull\n\"refused\"\n\"refused\"\n\"refused\"\nnull\n\"refused\"\n\"refused\"\n"
    );
    Ok(())
}

/// `--argjson`, `--jsonargs` and `--slurpfile` exit 2 with jq's first line.
#[test]
fn a_nan_payload_is_refused_as_a_named_json_argument_7522() -> Result<()> {
    for (args, first_line) in [
        (
            vec!["-nc", "--argjson", "x", "NaN1", "$x"],
            "jq: invalid JSON text passed to --argjson\n",
        ),
        (
            vec!["-nc", "--argjson", "x", "-NaN10", "$x"],
            "jq: invalid JSON text passed to --argjson\n",
        ),
        (
            vec!["-nc", "$ARGS.positional", "--jsonargs", "NaN1"],
            "jq: invalid JSON text passed to --jsonargs\n",
        ),
    ] {
        let (out, err, code) = run_jq_full(&args, None)?;
        assert_eq!((out.as_str(), code), ("", 2), "{args:?}");
        assert!(err.starts_with(first_line), "{args:?}: {err:?}");
    }
    for (args, stdout) in [
        (vec!["-nc", "--argjson", "x", "NaN", "$x"], "null\n"),
        (vec!["-nc", "--argjson", "x", "NaN0", "$x"], "null\n"),
        (
            vec!["-nc", "$ARGS.positional", "--jsonargs", "NaN"],
            "[null]\n",
        ),
    ] {
        let (out, _, code) = run_jq_full(&args, None)?;
        assert_eq!((out.as_str(), code), (stdout, 0), "{args:?}");
    }
    let dir = tempfile::tempdir()?;
    let bad = dir.path().join("payload.json");
    std::fs::write(&bad, "NaN1")?;
    let good = dir.path().join("plain.json");
    std::fs::write(&good, "NaN")?;
    let bad = bad.display().to_string();
    let good = good.display().to_string();
    let (out, err, code) = run_jq_full(&["-nc", "--slurpfile", "x", &bad, "$x"], None)?;
    assert_eq!((out.as_str(), code), ("", 2));
    assert!(
        err.starts_with(&format!("jq: Bad JSON in --slurpfile x {bad}: ")),
        "{err:?}"
    );
    let (out, _, code) = run_jq_full(&["-nc", "--slurpfile", "x", &good, "$x"], None)?;
    assert_eq!((out.as_str(), code), ("[null]\n", 0));
    Ok(())
}

/// A document of 4 MiB or more takes the windowed route: a payload deep inside is refused with the same line
/// and column, and the same file with a zero payload reads.
#[test]
fn a_nan_payload_deep_in_a_large_document_is_refused_7522() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let head: String = (0..400_000).map(|n| format!("{n},")).collect();
    let write = |name: &str, word: &str| -> Result<String> {
        let path = dir.path().join(name);
        std::fs::write(&path, format!("[{head}{word},{head}0]\n"))?;
        Ok(path.display().to_string())
    };
    let bad = write("bad.json", "NaN1")?;
    let good = write("good.json", "NaN0")?;
    assert!(std::fs::metadata(&bad)?.len() > 4 << 20);
    let column = 1 + head.len() + "NaN1".len() + 1;
    // `-n` with `inputs` words the same fault as `jq: error (at <file>:0): ...`.
    let want = format!("Invalid numeric literal at line 1, column {column}\n");
    for args in [
        vec!["-c", ".", bad.as_str()],
        vec!["-c", "--stream", ".", bad.as_str()],
        vec!["-c", "-n", "[inputs]|length", bad.as_str()],
    ] {
        let (_, err, code) = run_jq_full(&args, None)?;
        assert_eq!(code, 5, "{args:?}");
        assert!(err.ends_with(&want), "{args:?}: {err:?}");
    }
    let (out, err, code) = run_jq_full(&["-c", "length", &good], None)?;
    assert_eq!((out.as_str(), err.as_str(), code), ("800002\n", "", 0));
    let (out, err, code) = run_jq_full(&["-c", "-n", "[inputs]|length", &good], None)?;
    assert_eq!((out.as_str(), err.as_str(), code), ("1\n", "", 0));
    Ok(())
}
