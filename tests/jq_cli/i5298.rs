//! #5298: an object key written as an interpolated string or a format applied to one
//! (`{"a$\\(1+1)"}`, `{"a\\(.k)": 1}`, `{@base64 "x\\(.k)"}`) is jq's `String` production, and
//! the key alone is shorthand for `{(K): .[K]}`. Every expected output is what jq 1.7.1 prints.
//! A shorthand whose string may yield several values pairs each key with its own value, as jq
//! does (#5323, `i5323.rs`).

use anyhow::Result;

use crate::run_jq_full;

const INPUT: &str =
    r#"{"a":1,"b":2,"c":3,"a$2":4,"k":"Z","z":9,"p13":"hit","arr":[1,2],"x1":5,"x2":6}"#;

/// (filter, expected `-c` output, exit code), run on [`INPUT`].
const ROWS: &[(&str, &str, i32)] = &[
    (
        "{\"a\",b,\"a$\\(1+1)\"}",
        "{\"a\":1,\"b\":2,\"a$2\":4}\n",
        0,
    ),
    ("{\"\\(.k|ascii_downcase)\"}", "{\"z\":9}\n", 0),
    ("{\"\\(.k)\"}", "{\"Z\":null}\n", 0),
    ("{\"p\\(.a|tostring)\\(.b+1)\"}", "{\"p13\":\"hit\"}\n", 0),
    ("{\"x\\(1)\"}", "{\"x1\":5}\n", 0),
    ("{\"x\\(2)\", \"x\\(1)\"}", "{\"x2\":6,\"x1\":5}\n", 0),
    ("{@base64 \"x\\(.k)\"}", "{\"xWg==\":null}\n", 0),
    ("{@uri \"a b\\(.k)\"}", "{\"a bZ\":null}\n", 0),
    ("{@json \"j\\(.a)\"}", "{\"j1\":null}\n", 0),
    ("{@base64 \"a\"}", "{\"a\":1}\n", 0),
    (
        "{@text \"p\\(.a|tostring)\\(.b+1)\"}",
        "{\"p13\":\"hit\"}\n",
        0,
    ),
    ("{\"a\\(1)\": 2}", "{\"a1\":2}\n", 0),
    ("{\"a\\(1,2)\": 3}", "{\"a1\":3}\n{\"a2\":3}\n", 0),
    (
        "{\"a\\(1,2)\": (4,5)}",
        "{\"a1\":4}\n{\"a1\":5}\n{\"a2\":4}\n{\"a2\":5}\n",
        0,
    ),
    ("{\"a\\(.k)\", \"b\": 1}", "{\"aZ\":null,\"b\":1}\n", 0),
    ("{\"a\\(1)\": .a, b}", "{\"a1\":1,\"b\":2}\n", 0),
    ("{\"\\(.k)\": 1, \"\\(.z)\": 2}", "{\"Z\":1,\"9\":2}\n", 0),
    ("[{\"x\\(1,2)\": 0}]", "[{\"x1\":0},{\"x2\":0}]\n", 0),
    (
        "{@base64 \"a\\(1,2)\": 0}",
        "{\"aMQ==\":0}\n{\"aMg==\":0}\n",
        0,
    ),
    ("{\"a$\\(1+1)\"} | keys", "[\"a$2\"]\n", 0),
    (
        "{(\"a\" + \"b\"): 1, \"q\\(1)\": 2}",
        "{\"ab\":1,\"q1\":2}\n",
        0,
    ),
    ("{\"\\(\"\")\"}", "{\"\":null}\n", 0),
    ("{\"\\(null)\"}", "{\"null\":null}\n", 0),
    ("{\"\\([.a])\"}", "{\"[1]\":null}\n", 0),
    ("{\"\\(.arr|length)\"}", "{\"2\":null}\n", 0),
    ("{\"a\\(.a // 0)\"}", "{\"a1\":null}\n", 0),
    ("{\"a\\(.zz // 0)\"}", "{\"a0\":null}\n", 0),
    ("{\"a\\(if .a then 1 else 2 end)\"}", "{\"a1\":null}\n", 0),
    ("{\"a\\(.k[1:])\"}", "{\"a\":1}\n", 0),
    ("{\"a\\(.a and .b)\"}", "{\"atrue\":null}\n", 0),
    ("{@base64 \"a\": 1}", "{\"a\":1}\n", 0),
];

/// (input, filter, expected `-c` output, exit code).
const OTHER_INPUTS: &[(&str, &str, &str, i32)] = &[
    ("1", "{\"a\\(1)\"}", "", 5),
    ("[1]", "{\"a\\(1)\"}", "", 5),
    ("null", "{\"a\\(1)\"}", "{\"a1\":null}\n", 0),
    ("\"s\"", "{\"\\(.)\"}", "", 5),
];

#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq string interpolation reads like a format argument
fn test_interpolated_string_object_keys_answer_as_jq_5298() -> Result<()> {
    for (filter, want, want_code) in ROWS {
        let (stdout, stderr, code) =
            run_jq_full(&["-n", "-c", &format!("{INPUT} | {filter}")], None)?;
        assert_eq!(
            (stdout.as_str(), code),
            (*want, *want_code),
            "`{filter}`: {stderr}"
        );
    }
    for (input, filter, want, want_code) in OTHER_INPUTS {
        let (stdout, stderr, code) =
            run_jq_full(&["-n", "-c", &format!("{input} | {filter}")], None)?;
        assert_eq!(
            (stdout.as_str(), code),
            (*want, *want_code),
            "{input} | `{filter}`: {stderr}"
        );
    }
    Ok(())
}
