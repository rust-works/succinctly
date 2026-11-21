//! #7433: an identifier is ASCII. jq's lexer reads `[a-zA-Z_][a-zA-Z_0-9]*` (and `::` between the
//! parts of a name), so a letter such as `é` or `中` in a `.field`, an object key, a `def` name or
//! parameter, a `$variable`, a label, an import alias or an `@format` is `INVALID_CHARACTER`: a
//! compile error, exit 3, and nothing on stdout. A quoted name takes any text. jq 1.7.1 and 1.8.2
//! agree on every row (1.8.2 only adds the column and caret to the message, which is not pinned:
//! the wording of a syntax error is a recorded divergence, #2703).

use anyhow::Result;

use crate::run_jq_full;

/// Programs jq refuses, each captured as exit 3 with empty stdout on jq 1.7.1 and 1.8.2.
const REFUSED: &[&str] = &[
    ".é",
    ".a.é",
    ".a | .é",
    ".é?",
    ".é = 1",
    ".中",
    ".a.中",
    ".aé",
    ".é_",
    "{ é:1}",
    "{a:1,é:2}",
    "{ é }",
    "{$é}",
    "def é: 1; é",
    "def aé: 1; aé",
    "def _é: 1; 1",
    "def a(é): 1; a(2)",
    "def a($é): $é; a(2)",
    ". as $é | $é",
    ". as [$é] | 1",
    ". as {a:$é} | 1",
    ". as { é:$a} | 1",
    "reduce .[] as $é (0; .)",
    "foreach .[] as $é (0; .)",
    "$é",
    "label $é | 1",
    "import \"x\" as é; 1",
    "é",
    "ünicode",
    "a é",
    "@é",
    ".. | é?",
];

/// Programs jq accepts: a quoted name takes any text, and so does a string.
const ACCEPTED: &[(&str, &str)] = &[
    (r#"."é""#, "1\n"),
    (r#".["é"]"#, "1\n"),
    (r#".a."é""#, "2\n"),
    (r#"{"é":1}"#, "{\"é\":1}\n"),
    (r"{(.a|keys[0]):1}", "{\"é\":1}\n"),
    (r#""é""#, "\"é\"\n"),
    (r#""\("é")""#, "\"é\"\n"),
    (r#".["中"]"#, "3\n"),
    ("1 as $x | $x", "1\n"),
    (".a as $x | [$x]", "[{\"é\":2}]\n"),
];

const DOC: &str = r#"{"é":1,"a":{"é":2},"中":3}"#;

#[test]
fn test_a_non_ascii_identifier_is_a_compile_error_7433() -> Result<()> {
    for program in REFUSED {
        let (stdout, stderr, code) = run_jq_full(&["-c", program], Some(DOC))?;
        assert_eq!(
            (stdout.as_str(), code),
            ("", 3),
            "{program}: stdout {stdout:?}, stderr {stderr:?}"
        );
        assert!(
            stderr.starts_with("jq: error: ") && stderr.contains("compile error"),
            "{program}: {stderr:?}"
        );
    }
    Ok(())
}

#[test]
fn test_a_quoted_non_ascii_name_still_works_7433() -> Result<()> {
    for (program, want) in ACCEPTED {
        let (stdout, stderr, code) = run_jq_full(&["-c", program], Some(DOC))?;
        assert_eq!(
            (stdout.as_str(), code),
            (*want, 0),
            "{program}: stderr {stderr:?}"
        );
    }
    Ok(())
}

#[test]
fn test_the_refusal_holds_for_a_program_file_and_for_input_files_7433() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let program = dir.path().join("p.jq");
    std::fs::write(&program, ".é")?;
    let input = dir.path().join("in.json");
    std::fs::write(&input, DOC)?;
    let (stdout, stderr, code) = run_jq_full(
        &[
            "-c",
            "-f",
            program.to_str().unwrap_or_default(),
            input.to_str().unwrap_or_default(),
        ],
        None,
    )?;
    assert_eq!((stdout.as_str(), code), ("", 3), "{stderr}");
    let (stdout, stderr, code) =
        run_jq_full(&["-c", ".é", input.to_str().unwrap_or_default()], None)?;
    assert_eq!((stdout.as_str(), code), ("", 3), "{stderr}");
    Ok(())
}
