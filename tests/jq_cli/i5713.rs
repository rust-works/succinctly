//! #5713: the `$ARGS` object holds a copy of every named container argument, so it is built only
//! for a program that names `$ARGS`; a program that does not read it answers as before, and one
//! that does, in any position, still sees the same object. A named string is shared once, so a
//! `$s` read, a `def` parameter and a `$ARGS` entry take a refcount, and every read answers as it
//! did. Outputs were captured from jq 1.7.1.

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

// The string sharing: these rows are the shapes a first design (a marker node in place of the
// literal, #5742) made slower or answered differently, each with the answer the pinned jq 1.7.1
// gives. A string is read at a short and a long length; nothing depends on the length.

/// `"hé"` repeated `n` times: `2n` codepoints, `3n` bytes.
fn he(n: usize) -> String {
    "h\u{e9}".repeat(n)
}

const SHARED_ROWS: &[(&str, &str)] = &[
    ("$s|length", "LEN\n"),
    // A fold whose update is a pipe of assignments reading `$s` after the first stage.
    (
        "reduce range(0;50) as $i ({arr:[]}; .arr += [$i] | .s = $s) | [(.arr|length), (.s|length)]|tojson",
        "\"[50,LEN]\"\n",
    ),
    ("reduce range(0;20) as $i ({}; .s //= $s)|.s|length", "LEN\n"),
    ("def f($x): ($x|length)+1; [range(3)|f($s)]|add", "ADD\n"),
    ("def f($x): [$x,$x]|map(length)|add; f($s)", "DOUBLE\n"),
    ("$s as $x|$x|length", "LEN\n"),
    ("[$s,$s]|map(startswith(\"h\"))|all", "true\n"),
    ("$s|startswith($s)", "true\n"),
    ("$s|endswith($s)", "true\n"),
    ("{($s):1}|keys|length", "1\n"),
    ("{a:1}|has(\"a\"), ({a:1}|has($s))", "true\nfalse\n"),
    ("$s|split($s)|length", "2\n"),
    ("$ARGS.named.s|length", "LEN\n"),
    ("[$ARGS.named.s,$s]|unique|length", "1\n"),
];

#[test]
fn a_named_string_reads_alike_at_every_length_5713() -> Result<()> {
    for n in [10usize, 2100] {
        let value = he(n);
        let len = (2 * n).to_string();
        let add = (2 * n * 3 + 3).to_string();
        let double = (4 * n).to_string();
        for (filter, expected) in SHARED_ROWS {
            let expected = expected
                .replace("LEN", &len)
                .replace("ADD", &add)
                .replace("DOUBLE", &double);
            let (stdout, stderr, code) = run_jq_full(&["-n", "--arg", "s", &value, filter], None)?;
            assert_eq!(
                (stdout.as_str(), stderr.as_str(), code),
                (expected.as_str(), "", 0),
                "{filter} at {n}"
            );
        }
    }
    Ok(())
}

/// A `--argjson` string reads the same as an `--arg`.
#[test]
fn an_argjson_string_reads_alike_5713() -> Result<()> {
    let value = he(2100);
    let json = format!("\"{value}\"");
    let (stdout, stderr, code) = run_jq_full(
        &[
            "-n",
            "--argjson",
            "s",
            &json,
            "[$s|length, ($s|startswith(\"h\"))]|tojson",
        ],
        None,
    )?;
    assert_eq!(
        (stdout.as_str(), stderr.as_str(), code),
        ("\"[4200,true]\"\n", "", 0)
    );
    Ok(())
}

/// A `--slurpfile` always binds an array, so a string element is read through it.
#[test]
fn a_slurpfile_string_element_reads_alike_5713() -> Result<()> {
    let (_file, path) = crate::temp_json(&format!("\"{}\"", he(2100)))?;
    let (stdout, stderr, code) = run_jq_full(
        &[
            "-n",
            "--slurpfile",
            "s",
            &path,
            "[$s[0]|length, ([range(3)|$s[0]|startswith(\"h\")]|all)]|tojson",
        ],
        None,
    )?;
    assert_eq!(
        (stdout.as_str(), stderr.as_str(), code),
        ("\"[4200,true]\"\n", "", 0)
    );
    Ok(())
}

/// A long `--rawfile` (the only way to reach a string past the argument-size limit) reads alike.
#[test]
fn a_long_rawfile_reads_alike_5713() -> Result<()> {
    let (_file, path) = crate::temp_json(&he(40_000))?;
    let (stdout, stderr, code) = run_jq_full(
        &[
            "-n",
            "--rawfile",
            "s",
            &path,
            "[$s|length, ($s|startswith(\"h\")), ([range(5)|$s|length]|add), ($s|test(\"zzzz\"))]|tojson",
        ],
        None,
    )?;
    assert_eq!(
        (stdout.as_str(), stderr.as_str(), code),
        ("\"[80000,true,400000,false]\"\n", "", 0)
    );
    Ok(())
}

/// A write to a string derived from `$s` never reaches `$s` or another read of it: the literal
/// and every read share one storage, so a write has to copy first.
#[test]
fn a_write_to_a_string_from_a_named_string_leaves_it_alone_5713() -> Result<()> {
    let value = he(2100);
    for (filter, expected) in [
        (
            "($s + \"x\") as $t | [($s|length), ($t|length)]|tojson",
            "\"[4200,4201]\"\n",
        ),
        (
            "[$s, ($s|ascii_upcase), $s]|map(length)|tojson",
            "\"[4200,4200,4200]\"\n",
        ),
        (
            "reduce range(0;3) as $i (\"\"; . + $s) | $s | length",
            "4200\n",
        ),
        (
            "{a:$s}|.a|=1|[., ($s|length)]|tojson",
            "\"[{\\\"a\\\":1},4200]\"\n",
        ),
        ("$s|(.|=ascii_upcase)|length", "4200\n"),
        (
            "[$s|ltrimstr(\"zzz\"), $s]|map(length)|tojson",
            "\"[4200,4200]\"\n",
        ),
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-n", "--arg", "s", &value, filter], None)?;
        assert_eq!(
            (stdout.as_str(), stderr.as_str(), code),
            (expected, "", 0),
            "{filter}"
        );
    }
    Ok(())
}

/// A string used as a path target is refused with jq's own error.
#[test]
#[cfg_attr(feature = "unshared-containers", ignore = "holdout refusal (#5246)")]
fn a_named_string_is_not_a_path_5713() -> Result<()> {
    let value = he(2100);
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
