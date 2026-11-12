//! The flags of `-o props` and `-o csv`: `--properties-separator`, `--properties-array-brackets`
//! and `--csv-separator` (#6922, a follow-up to the `-o` encoders of #6850). Every answer was captured
//! from yq v4.53.3; a row is kept only when both agree on stdout and the exit status. The `@props`
//! / `to_props` and `@csv` / `to_csv` operators read the same flags, as in yq, wherever they are
//! parsed: in the program, in the text an `eval` reads and in a `--split-exp` expression.

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

/// The text `eval` parses reads the flags too, as in yq: the setting belongs to the run, not to
/// the program text it was parsed with.
#[test]
fn eval_text_reads_the_encoder_flags_6922() -> Result<()> {
    for (filter, flag, expected) in [
        ("eval(\".x | @csv\")", "--csv-separator=;", "1;\"a;b\"\n"),
        (
            "eval(\".m | @props\")",
            "--properties-separator=:",
            "k.0:1\nk.1.j:2\ns\\ p:v w\n\n",
        ),
    ] {
        let (stdout, status) = run_yq_stdin(filter, OPERATOR_INPUT, &[flag])?;
        assert_eq!((stdout.as_str(), status), (expected, 0), "{flag} {filter}");
    }
    Ok(())
}

/// `format("csv")` (a jq builtin, behind `--jq-extensions`) is `@csv`, so it reads the flag too.
#[test]
fn format_csv_reads_the_csv_separator_6922() -> Result<()> {
    let (stdout, status) = run_yq_stdin(
        ".x | format(\"csv\")",
        OPERATOR_INPUT,
        &["--jq-extensions", "--csv-separator=;"],
    )?;
    assert_eq!((stdout.as_str(), status), ("1;\"a;b\"\n", 0));
    Ok(())
}

/// A `--split-exp` expression is parsed apart from the program and reads the flag as well: yq names
/// the files `1;2.yml` and `3;4.yml`. The names are compared without the extension, which yq adds and
/// `succinctly yq` does not (a separate difference). `--split-exp` is long-only here, since `-s` is
/// `--slurp` (#715).
#[test]
fn split_exp_reads_the_csv_separator_6922() -> Result<()> {
    let dir = tempfile::TempDir::new()?;
    std::fs::write(dir.path().join("rows.yaml"), "- [1, 2]\n- [3, 4]\n")?;
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .args([
            "yq",
            "--csv-separator=;",
            "--split-exp",
            ". | @csv",
            ".[]",
            "rows.yaml",
        ])
        .current_dir(dir.path())
        .stdin(std::process::Stdio::null())
        .output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let mut names: Vec<String> = std::fs::read_dir(dir.path())?
        .map(|entry| {
            entry.map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                match name.strip_suffix(".yml") {
                    Some(stem) => stem.to_string(),
                    None => name,
                }
            })
        })
        .collect::<Result<_, _>>()?;
    names.sort();
    assert_eq!(names, ["1;2", "3;4", "rows.yaml"]);
    Ok(())
}

/// (input, arguments, yq's stdout): `-o=props` and `@props` write each property's comments above
/// it, and the first document's leading comments above the first property. Captured from yq
/// v4.53.3.
const COMMENT_ROWS: &[(&str, &[&str], &str)] = &[
    ("# doc head\n\n# a head\na: 1 # a line\nb: # b key line\n  # x head\n  - x # x line\n  - y\nc:\n  d: \"p q\" # d line\n  # e head\n  e: 2\n", &["-o=props", "."], "# doc head\n\n# a head\n# a line\na = 1\n\n# x head x line\nb.0 = x\nb.1 = y\n\n# d line\nc.d = p q\n\n# e head\nc.e = 2\n"),
    ("# doc head\n\n# a head\na: 1 # a line\nc:\n  d: \"p q\" # d line\n  # e head\n  e: 2\n", &["-o=props", ".c"], "# d line\nd = p q\n\n# e head\ne = 2\n"),
    ("k: v\n# l1\n#l2\n#   l3\na: 1 #x\n", &["-o=props", "."], "k = v\n\n# l1\n# #l2\n# #   l3x\na = 1\n"),
    ("s:\n  - p: 1 # pl\n    # q head\n    q: 2\n  - # item head\n    r: 3\n", &["-o=props", "."], "# pl\ns.0.p = 1\n\n# q head\ns.0.q = 2\n\n# item head\ns.1.r = 3\n"),
    ("s:\n  - p: 1 # pl\n  - # item head\n    r: 3\n", &["--properties-array-brackets", "-o=props", ".s"], "# pl\n0.p = 1\n\n# item head\n1.r = 3\n"),
    ("- # d\n  - v\n- # e\n  - - 1\n", &["-o=props", "."], "# d\n0.0 = v\n1.0.0 = 1\n"),
    ("- #2\n  #1\n  c: 3.5\n", &["-o=props", "."], "#2\n# #1\n0.c = 3.5\n"),
    ("al: &an 5 # anchor line\nref: *an # alias line\ne: {} # empty line\n", &["-o=props", "."], "# anchor line\nal = 5\n\n# alias line\nref = 5\n"),
    ("a: {b: 1} # dropped\n", &["-o=props", "."], "a.b = 1\n"),
    ("---\n# after the marker\nn: 1\n", &["-o=props", "."], "# after the marker\nn = 1\n"),
    ("\n---\nn: 1\n", &["-o=props", "."], "\nn = 1\n"),
    ("# lead\na: 1 # one\n", &["@props"], "# lead\n# one\na = 1\n\n"),
    ("# lead\na: 1 # one\n", &["--properties-separator=:", "@props"], "# lead\n# one\na:1\n\n"),
    ("# lead\na: 1\nb: 2 # two\n", &["-o=props", ".b"], "2\n"),
    ("a: 1 # one\n", &["{\"a\": 1} | @props"], "a = 1\n\n"),
];

#[test]
fn props_writes_each_propertys_comments_6922() -> Result<()> {
    for (input, args, expected) in COMMENT_ROWS {
        let (filter, flags) = args.split_last().expect("a filter");
        let (stdout, status) = run_yq_stdin(filter, input, flags)?;
        assert_eq!(
            (stdout.as_str(), status),
            (*expected, 0),
            "{args:?} over {input:?}"
        );
    }
    Ok(())
}
