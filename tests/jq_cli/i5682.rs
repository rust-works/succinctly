//! #5682: `split(SEP)` of a string value, for a literal `SEP`, reads the pieces off the value's
//! text instead of writing the string out as JSON and indexing it again. Each row is a program,
//! its input, and what jq 1.7.1 (`/usr/bin/jq`) answers: `(args, filter, stdin, stdout, stderr,
//! exit status)`.

use anyhow::Result;

use crate::run_jq_full;

const ROWS: &[(&str, &str, &str, &str, &str, i32)] = &[
    ("-Rc", "split(\",\")", "a,b,c\n", "[\"a\",\"b\",\"c\"]\n", "", 0),
    ("-Rc", "split(\",\")", "\n", "[]\n", "", 0),
    ("-Rc", "split(\",\")", ",a,,b,\n", "[\"\",\"a\",\"\",\"b\",\"\"]\n", "", 0),
    ("-Rc", "split(\"\")", "héllo😀\n", "[\"h\",\"é\",\"l\",\"l\",\"o\",\"😀\"]\n", "", 0),
    ("-Rc", "split(\", \")", "a, b,c, d\n", "[\"a\",\"b,c\",\"d\"]\n", "", 0),
    ("-Rc", "split(\"ab\")", "xabab\n", "[\"x\",\"\",\"\"]\n", "", 0),
    ("-Rc", "(split(\",\"))", "x,y\n", "[\"x\",\"y\"]\n", "", 0),
    ("-Rc", "split(\",\") | .[1]", "x,y\n", "\"y\"\n", "", 0),
    ("-Rc", "[split(\",\")[]|length]", "x,yy,zzz\n", "[1,2,3]\n", "", 0),
    (
        "-c",
        ".[]|split(\",\")",
        "[\"a,b\",\"\",\"é,😀,\\u0000\",\"\\ud83d\\ude00,x\"]",
        "[\"a\",\"b\"]\n[]\n[\"é\",\"😀\",\"\\u0000\"]\n[\"😀\",\"x\"]\n",
        "",
        0,
    ),
    ("-c", "map(split(\"/\"))", "[\"a/b\",\"c\",\"/\"]", "[[\"a\",\"b\"],[\"c\"],[\"\",\"\"]]\n", "", 0),
    // Not a string: the evaluator's own refusal, which `?` and `try` still catch.
    ("-c", "try split(\",\") catch .", "1", "\"split input and separator must be strings\"\n", "", 0),
    ("-c", "try split(\",\") catch .", "null", "\"split input and separator must be strings\"\n", "", 0),
    ("-c", "[.[]|split(\",\")?]", "[\"a,b\",1,null,\"c\"]", "[[\"a\",\"b\"],[\"c\"]]\n", "", 0),
    // A separator that is not a string literal keeps the general route: a non-string raises, a
    // second separator fans out, the two-argument form is the regex one.
    ("-c", "split(1)", "\"a\"", "", "jq: error (at <stdin>:0): split input and separator must be strings\n", 5),
    ("-c", "split(\",\";null)", "\"a,b\"", "[\"a\",\"b\"]\n", "", 0),
    ("-c", "split(\",\",\"b\")", "\"a,b\"", "[\"a\",\"b\"]\n[\"a,\",\"\"]\n", "", 0),
];

#[test]
fn test_split_of_a_string_matches_jq_5682() -> Result<()> {
    for (flag, filter, stdin, stdout, stderr, status) in ROWS {
        let (out, err, code) = run_jq_full(&[flag, filter], Some(stdin))?;
        assert_eq!(
            (out.as_str(), err.as_str(), code),
            (*stdout, *stderr, *status),
            "{flag} {filter} over {stdin:?}"
        );
    }
    Ok(())
}

/// A long line, and enough of them that a per-line cost shows up as a count: the answer is the
/// same as the pieces `str::split` finds.
#[test]
fn test_split_of_many_raw_lines_5682() -> Result<()> {
    let input: String = (0..2000).map(|i| format!("n{i},u{i}@x.org,{}\n", i % 90)).collect();
    let (out, err, code) = run_jq_full(&["-Rr", "split(\",\")|.[2]"], Some(&input))?;
    let expected: String = (0..2000).map(|i| format!("{}\n", i % 90)).collect();
    assert_eq!((out, err.as_str(), code), (expected, "", 0));
    Ok(())
}
