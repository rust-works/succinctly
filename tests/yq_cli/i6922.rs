//! The flags of `-o props` and `-o csv`: `--properties-separator`, `--properties-array-brackets`
//! and `--csv-separator` (#6922, a follow-up to the `-o` encoders of #6850). Every answer was captured
//! from yq v4.53.3; a row is kept only when both agree on stdout and the exit status. The `@props`
//! and `@csv` operators ignore the flags here, where yq applies them (see the limitations page).

use anyhow::Result;

use crate::{run_yq_stdin, run_yq_stdin_with_stderr};

/// (format, flags, input, yq's stdout, yq's exit status).
const ROWS: &[(&str, &[&str], &str, &str, i32)] = &[
    (
        "props",
        &["--properties-separator=:"],
        "a: 1\nb: [x, y]\nc: {d: [1, {e: 2}]}\n",
        "a:1\nb.0:x\nb.1:y\nc.d.0:1\nc.d.1.e:2\n",
        0,
    ),
    (
        "props",
        &["--properties-separator= : "],
        "a: 1\nb: [x]\n",
        "a : 1\nb.0 : x\n",
        0,
    ),
    (
        "props",
        &["--properties-separator="],
        "a: 1\n",
        "a = 1\n",
        0,
    ),
    (
        "props",
        &["--properties-array-brackets"],
        "a: 1\nb: [x, y]\nc: {d: [1, {e: 2}]}\n",
        "a = 1\nb[0] = x\nb[1] = y\nc.d[0] = 1\nc.d[1].e = 2\n",
        0,
    ),
    (
        "props",
        &["--properties-array-brackets", "--properties-separator=|"],
        "- {a: 1}\n- [2]\n",
        "0.a|1\n1[0]|2\n",
        0,
    ),
    (
        "props",
        &["--properties-array-brackets"],
        "[a, b]\n",
        "0 = a\n1 = b\n",
        0,
    ),
    (
        "props",
        &["--properties-separator=:"],
        "hello\n",
        "hello\n",
        0,
    ),
    (
        "props",
        &["--properties-separator=="],
        "a: \"a:b\"\nb: \"c=d\"\n",
        "a=a:b\nb=c=d\n",
        0,
    ),
    (
        "csv",
        &["--csv-separator=;"],
        "[a, \"b;c\", \"d,e\", \"f g\", \" lead\", \"q\\\"r\"]\n",
        "a;\"b;c\";d,e;f g;\" lead\";\"q\"\"r\"\n",
        0,
    ),
    (
        "csv",
        &["--csv-separator=;"],
        "[[a, \"b;c\"], [1, \"d\\\"e\"]]\n",
        "a;\"b;c\"\n1;\"d\"\"e\"\n",
        0,
    ),
    (
        "csv",
        &["--csv-separator=;"],
        "[{a: 1, b: \"x;y\"}, {a: 2}]\n",
        "a;b\n1;\"x;y\"\n2;\n",
        0,
    ),
    ("csv", &["--csv-separator=\t"], "[a, b]\n", "a\tb\n", 0),
    (
        "csv",
        &["--csv-separator= "],
        "[a, \"b c\"]\n",
        "a \"b c\"\n",
        0,
    ),
    ("csv", &["--csv-separator=\""], "[a, b]\n", "", 1),
    ("csv", &["--csv-separator=\""], "[{a: 1}]\n", "", 0),
    ("csv", &["--csv-separator=\""], "[]\n", "", 0),
    ("csv", &["--csv-separator=\n"], "[[a]]\n", "", 1),
    (
        "tsv",
        &["--csv-separator=;"],
        "[a, \"b;c\"]\n",
        "a\tb;c\n",
        0,
    ),
];

#[test]
fn encoder_flag_rows_match_yq_6922() -> Result<()> {
    for (format, flags, input, expected, code) in ROWS {
        let mut args = vec![format!("-o={format}")];
        args.extend(flags.iter().map(|flag| flag.to_string()));
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let (stdout, status) = run_yq_stdin(".", input, &args)?;
        assert_eq!(
            (stdout.as_str(), status),
            (*expected, *code),
            "-o={format} {flags:?} over {input:?}"
        );
    }
    Ok(())
}

#[test]
fn csv_separator_must_be_one_byte_6922() -> Result<()> {
    for separator in ["ab", "", "é"] {
        let (stdout, stderr, status) = run_yq_stdin_with_stderr(
            ".",
            "[a, b]\n",
            &["-o=csv", format!("--csv-separator={separator}").as_str()],
        )?;
        assert_eq!((stdout.as_str(), status), ("", 1), "{separator:?}");
        let length = separator.len();
        assert!(
            stderr.starts_with(&format!(
                "Error: invalid argument \"{separator}\" for \"--csv-separator\" flag: [{separator}] is not a valid character. Must be length 1 was {length}"
            )),
            "{separator:?}: {stderr}"
        );
    }
    Ok(())
}
