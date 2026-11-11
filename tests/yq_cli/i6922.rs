//! The flags of `-o props` and `-o csv`: `--properties-separator`, `--properties-array-brackets`
//! and `--csv-separator` (#6922, a follow-up to the `-o` encoders of #6850). Every answer was captured
//! from yq v4.53.3; a row is kept only when both agree on stdout and the exit status. The `@props`
//! / `to_props` and `@csv` / `to_csv` operators read the same flags, as in yq; the text an `eval`
//! reads does not (see the limitations page).

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
    (
        "props",
        &["--properties-array-brackets"],
        "[[a, b], [c]]\n",
        "0[0] = a\n0[1] = b\n1[0] = c\n",
        0,
    ),
    ("csv", &["--csv-separator=\r"], "[{a: 1}]\n", "", 0),
    ("csv", &["--csv-separator=\r"], "[a]\n", "", 1),
    (
        "csv",
        &["--csv-separator=;"],
        "[\";\", \"a\\nb\", \"x\\\"y\"]\n",
        "\";\";\"a\nb\";\"x\"\"y\"\n",
        0,
    ),
    (
        "props",
        &["--properties-separator=\t"],
        "a: 1\n",
        "a\t1\n",
        0,
    ),
];

#[test]
fn encoder_flag_rows_match_yq_6922() -> Result<()> {
    for (format, flags, input, expected, code) in ROWS {
        let mut args = vec![format!("-o={format}")];
        args.extend(flags.iter().map(|flag| (*flag).to_string()));
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

/// (filter, flags, yq's stdout, yq's exit status), over [`OPERATOR_INPUT`].
const OPERATOR_ROWS: &[(&str, &[&str], &str, i32)] = &[
    (".x | @csv", &["--csv-separator=;"], "1;\"a;b\"\n", 0),
    (".x | to_csv", &["--csv-separator=;"], "1;\"a;b\"\n", 0),
    (
        "[{\"a\": 1, \"b\": \"x;y\"}] | @csv",
        &["--csv-separator=;"],
        "a;b\n1;\"x;y\"\n",
        0,
    ),
    (".x | @tsv", &["--csv-separator=;"], "1\ta;b\n", 0),
    (".x | @csv", &["--csv-separator=\""], "", 1),
    (
        ".y = (.x | @csv) | .y",
        &["--csv-separator=|"],
        "1|a;b\n",
        0,
    ),
    (
        ".m | @props",
        &["--properties-separator=:"],
        "k.0:1\nk.1.j:2\ns\\ p:v w\n\n",
        0,
    ),
    (
        ".m | to_props",
        &["--properties-array-brackets"],
        "k[0] = 1\nk[1].j = 2\ns\\ p = v w\n\n",
        0,
    ),
    (
        ".m | @props",
        &["--properties-separator=:", "--properties-array-brackets"],
        "k[0]:1\nk[1].j:2\ns\\ p:v w\n\n",
        0,
    ),
    (
        ".m |= @props | .m",
        &["--properties-array-brackets"],
        "k[0] = 1\nk[1].j = 2\ns\\ p = v w\n\n",
        0,
    ),
    (
        "\"a b\" | @props",
        &["--properties-separator=:"],
        "a b\n\n",
        0,
    ),
    (".x | @csv", &["--properties-separator=:"], "1,a;b\n", 0),
    (
        ".m | @props",
        &["--csv-separator=;"],
        "k.0 = 1\nk.1.j = 2\ns\\ p = v w\n\n",
        0,
    ),
    (
        ".m | @props",
        &["-o=props", "--properties-separator=:"],
        "k.0:1\nk.1.j:2\ns\\ p:v w\n\n",
        0,
    ),
];

const OPERATOR_INPUT: &str = "x: [1, \"a;b\"]\nm: {k: [1, {j: 2}], \"s p\": v w}\n";

#[test]
fn the_operators_read_the_encoder_flags_6922() -> Result<()> {
    for (filter, flags, expected, code) in OPERATOR_ROWS {
        let (stdout, status) = run_yq_stdin(filter, OPERATOR_INPUT, flags)?;
        assert_eq!(
            (stdout.as_str(), status),
            (*expected, *code),
            "{flags:?} {filter}"
        );
    }
    Ok(())
}

/// A refused `--csv-separator` fails the operator with yq's message from Go's `csv.Writer`.
#[test]
fn the_csv_operator_refuses_a_quote_separator_6922() -> Result<()> {
    let (stdout, stderr, status) =
        run_yq_stdin_with_stderr(".x | @csv", OPERATOR_INPUT, &["--csv-separator=\""])?;
    assert_eq!(
        (stdout.as_str(), stderr.as_str(), status),
        ("", "Error: csv: invalid field or comment delimiter\n", 1)
    );
    Ok(())
}

/// The text `eval` parses keeps the default separators, where yq applies the flags to it too.
#[test]
fn eval_text_keeps_the_default_separators_6922() -> Result<()> {
    let (stdout, status) = run_yq_stdin(
        "eval(\".x | @csv\")",
        OPERATOR_INPUT,
        &["--csv-separator=;"],
    )?;
    assert_eq!((stdout.as_str(), status), ("1,a;b\n", 0), "yq: 1;\"a;b\"");
    Ok(())
}
