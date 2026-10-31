//! #5675: `fromjson` reads plain RFC 8259 text in one pass and leaves everything else (jq's
//! leniencies, a repeated key, a surrogate escape, every error and its message) to the general
//! route. Each row is a text and what jq 1.7.1 (`/usr/bin/jq`) answers to `fromjson` on it:
//! `(text, stdout, stderr, exit status)`.

use anyhow::Result;

use crate::run_jq_full;

const ROWS: &[(&str, &str, &str, i32)] = &[
    ("null", "null\n", "", 0),
    ("true", "true\n", "", 0),
    (" false ", "false\n", "", 0),
    ("0", "0\n", "", 0),
    ("-0", "-0\n", "", 0),
    ("1.0", "1.0\n", "", 0),
    ("1.50", "1.50\n", "", 0),
    ("1e2", "1E+2\n", "", 0),
    ("1E-2", "0.01\n", "", 0),
    ("100000000000000000000", "100000000000000000000\n", "", 0),
    ("1e1000", "1E+1000\n", "", 0),
    ("9007199254740993", "9007199254740993\n", "", 0),
    ("0.1", "0.1\n", "", 0),
    ("\"a\"", "\"a\"\n", "", 0),
    ("\"a\\nb\\t\\\"\\\\\\/\"", "\"a\\nb\\t\\\"\\\\/\"\n", "", 0),
    ("\"\\u00e9\\u0041\"", "\"éA\"\n", "", 0),
    ("\"\\ud83d\\ude00\"", "\"😀\"\n", "", 0),
    ("\"\\u0000\"", "\"\\u0000\"\n", "", 0),
    ("[]", "[]\n", "", 0),
    ("[ 1 , 2 ]", "[1,2]\n", "", 0),
    ("{}", "{}\n", "", 0),
    ("{\"a\":1,\"b\":[true,null,{\"c\":\"d\"}]}", "{\"a\":1,\"b\":[true,null,{\"c\":\"d\"}]}\n", "", 0),
    ("{\"a\":1,\"a\":2}", "{\"a\":2}\n", "", 0),
    ("{\"b\":2,\"a\":1,\"b\":3}", "{\"b\":3,\"a\":1}\n", "", 0),
    ("[1,2,3.0,4.50]", "[1,2,3.0,4.50]\n", "", 0),
    ("{\"id\":7,\"name\":\"User7\",\"score\":70.5}", "{\"id\":7,\"name\":\"User7\",\"score\":70.5}\n", "", 0),
    ("nan", "null\n", "", 0),
    ("[nan]", "[null]\n", "", 0),
    ("-Infinity", "-1.7976931348623157e+308\n", "", 0),
    ("007", "7\n", "", 0),
    ("1 2", "", "jq: error (at <stdin>:0): Unexpected extra JSON values (while parsing '1 2')\n", 5),
    ("[1,]", "", "jq: error (at <stdin>:0): Expected another array element at line 1, column 4 (while parsing '[1,]')\n", 5),
    ("{\"a\":1,}", "", "jq: error (at <stdin>:0): Expected another key-value pair at line 1, column 8 (while parsing '{\"a\":1,}')\n", 5),
    ("tru", "", "jq: error (at <stdin>:0): Invalid literal at EOF at line 1, column 3 (while parsing 'tru')\n", 5),
    ("\"\\x\"", "", "jq: error (at <stdin>:0): Invalid escape at line 1, column 4 (while parsing '\"\\x\"')\n", 5),
    ("\"\\ud800\"", "", "jq: error (at <stdin>:0): Invalid \\uXXXX\\uXXXX surrogate pair escape at line 1, column 8 (while parsing '\"\\ud800\"')\n", 5),
    ("[[[[[[[[[[[[1]]]]]]]]]]]]", "[[[[[[[[[[[[1]]]]]]]]]]]]\n", "", 0),
    ("{\"é\":\"ü\",\"\\u00e9\":1}", "{\"é\":1}\n", "", 0),
];

#[test]
fn test_fromjson_matches_jq_on_plain_and_lenient_text_5675() -> Result<()> {
    for (text, stdout, stderr, status) in ROWS {
        let quoted = serde_json::to_string(text)?;
        let (out, err, code) = run_jq_full(&["-c", "fromjson"], Some(&quoted))?;
        assert_eq!(
            (out.as_str(), err.as_str(), code),
            (*stdout, *stderr, *status),
            "fromjson over {text:?}"
        );
    }
    Ok(())
}
