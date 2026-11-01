//! #5713: the `$ARGS` object holds a copy of every named argument, so it is built only for a
//! program that names `$ARGS`. A program that does not read it answers as before, and one that
//! does, in any position, still sees the same object. Outputs were captured from jq 1.7.1
//! (`/usr/bin/jq`).

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn programs_that_read_args_and_programs_that_do_not_answer_as_jq_5713() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let raw = dir.path().join("r.txt");
    std::fs::write(&raw, "hello\n")?;
    let raw = raw.to_str().unwrap();
    for (filter, expected) in [
        (
            "$ARGS",
            "{\"positional\":[\"p\"],\"named\":{\"a\":\"1\",\"b\":[1,2],\"r\":\"hello\\n\"}}\n",
        ),
        ("$ARGS.named | keys", "[\"a\",\"b\",\"r\"]\n"),
        ("$ARGS.positional", "[\"p\"]\n"),
        ("def f: $ARGS.named.a; f", "\"1\"\n"),
        ("[limit(1; $ARGS.named.b[])]", "[1]\n"),
        ("\"v=\\($ARGS.positional)\"", "\"v=[\\\"p\\\"]\"\n"),
        (". as $ARGS | $ARGS", "null\n"),
        ("$a", "\"1\"\n"),
        ("$b | length", "2\n"),
        ("$r | length", "6\n"),
        ("[$a, $r]", "[\"1\",\"hello\\n\"]\n"),
        ("1", "1\n"),
        // A call a `def` shadows at another arity keeps its `$ARGS` argument out of the plain walk.
        (
            "def limit(f): 1; [limit(1; $ARGS.positional)]",
            "[[\"p\"]]\n",
        ),
        (
            "def range(f): 1; [range(0; $ARGS.positional | length)]",
            "[0]\n",
        ),
    ] {
        let (stdout, stderr, code) = run_jq_full(
            &[
                "-nc",
                "--arg",
                "a",
                "1",
                "--argjson",
                "b",
                "[1,2]",
                "--rawfile",
                "r",
                raw,
                filter,
                "--args",
                "p",
            ],
            None,
        )?;
        assert_eq!(code, 0, "{filter}: {stderr:?}");
        assert_eq!(stdout, expected, "{filter}");
    }
    Ok(())
}

// The string splice: a named string of 4096 bytes or more is one shared marker, not a literal
// copied at each read. Every row is a program over such a string with the answer the pinned
// jq 1.7.1 gives: the marker reads as the plain string in value position, in a fold, under `def`,
// and refuses a write through it exactly as a literal does.

/// 4200 codepoints, 6300 bytes: over the threshold, and not ASCII, so a byte count and a
/// codepoint count differ.
fn long_string() -> String {
    "h\u{e9}".repeat(2100)
}

const ROWS: &[(&str, &str)] = &[
    ("$s|length", "4200\n"),
    ("$s|(.|=ascii_upcase)|length", "4200\n"),
    ("$s|(select(length>3)|=1)", "1\n"),
    ("$s|(..|=1)", "1\n"),
    ("$s|first(.)|=1", "1\n"),
    ("$s|getpath([])|length", "4200\n"),
    (
        "try ($s|.[0]=1) catch .",
        "\"Cannot index string with number\"\n",
    ),
    (
        "reduce range(0;300) as $i (\"\"; . + $s)|length",
        "1260000\n",
    ),
    ("reduce range(0;50) as $i ([]; . + [$s])|length", "50\n"),
    ("def f($x): $x|length; f($s)", "4200\n"),
    ("[range(20)|$s|startswith(\"h\")]|all", "true\n"),
    ("{a:$s}|.a|=1|tojson", "\"{\\\"a\\\":1}\"\n"),
    ("[$s,$s]|unique|length", "1\n"),
    ("$s|. == $s", "true\n"),
    (
        "[$s|.[0:3], .[-2:]]|tojson",
        "\"[\\\"h\u{e9}h\\\",\\\"h\u{e9}\\\"]\"\n",
    ),
];

#[test]
fn a_long_named_string_reads_as_the_plain_string_5713() -> Result<()> {
    let value = long_string();
    for (filter, expected) in ROWS {
        let (stdout, stderr, code) = run_jq_full(&["-n", "--arg", "s", &value, filter], None)?;
        assert_eq!(
            (stdout.as_str(), stderr.as_str(), code),
            (*expected, "", 0),
            "{filter}"
        );
    }
    Ok(())
}

/// A string spliced as a path target is refused with jq's own error, under the marker as under a
/// literal.
#[test]
fn a_long_named_string_is_not_a_path_5713() -> Result<()> {
    let value = long_string();
    for filter in ["path($s)", "($s|getpath([]))|=1"] {
        let (stdout, stderr, code) = run_jq_full(&["-n", "--arg", "s", &value, filter], None)?;
        assert_eq!(stdout, "", "{filter}");
        assert!(
            stderr.contains("Invalid path expression with result"),
            "{filter}: {stderr}"
        );
        assert_eq!(code, 5, "{filter}");
    }
    Ok(())
}

/// A `--rawfile` and a `--slurpfile` string element reach the program the same way.
#[test]
fn a_long_rawfile_is_read_alike_5713() -> Result<()> {
    let dir = std::env::temp_dir().join(format!("succinctly-5713-{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("v.txt");
    std::fs::write(&path, long_string())?;
    let path = path.to_string_lossy().into_owned();
    let (stdout, stderr, code) = run_jq_full(
        &[
            "-n",
            "--rawfile",
            "s",
            &path,
            "[$s|length, ($s|startswith(\"h\")), ([range(5)|$s|length]|add)]|tojson",
        ],
        None,
    )?;
    std::fs::remove_dir_all(&dir)?;
    assert_eq!(
        (stdout.as_str(), stderr.as_str(), code),
        ("\"[4200,true,21000]\"\n", "", 0)
    );
    Ok(())
}
