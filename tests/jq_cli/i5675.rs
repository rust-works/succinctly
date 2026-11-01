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

/// Texts around the reader's own limits (nesting 64, the validator's 128, repeated keys inside an
/// array, surrogate pairs and halves), what the reader leaves to the general route.
const EDGES: &[(&str, &str, &str, i32)] = &[
    ("[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]", "[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]\n", "", 0),
    ("[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]", "[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]\n", "", 0),
    ("[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]", "[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]\n", "", 0),
    ("[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]", "[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]\n", "", 0),
    ("[{\"a\":1,\"a\":2},{\"b\":1}]", "[{\"a\":2},{\"b\":1}]\n", "", 0),
    ("\"\\ud83d\\ude00\\ud83d\\ude01\"", "\"😀😁\"\n", "", 0),
    ("\"\\ud83d\\ude00\\ud83d\"", "", "jq: error (at <stdin>:0): Invalid \\uXXXX\\uXXXX surrogate pair escape at line 1, column 20 (while parsing '\"\\ud83d\\ude00\\ud83d\"')\n", 5),
    ("\"\\ude00\"", "\"�\"\n", "", 0),
    ("\"\\ud83d\\n\"", "", "jq: error (at <stdin>:0): Invalid \\uXXXX\\uXXXX surrogate pair escape at line 1, column 10 (while parsing '\"\\ud83d\\n\"')\n", 5),
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

#[test]
fn test_fromjson_matches_jq_at_the_readers_limits_5675() -> Result<()> {
    for (text, stdout, stderr, status) in EDGES {
        let quoted = serde_json::to_string(text)?;
        let (out, err, code) = run_jq_full(&["-c", "fromjson"], Some(&quoted))?;
        assert_eq!(
            (out.as_str(), err.as_str(), code),
            (*stdout, *stderr, *status),
            "fromjson over {text:.40?}"
        );
    }
    Ok(())
}

/// The consumers of the same function: `fromjson?` swallows the refusal, `tonumber` reads a number
/// and words a non-number's refusal by whether the text is valid JSON. `(filter, input, stdout)`
/// captured from jq 1.7.1 (`/usr/bin/jq`).
#[test]
fn test_fromjson_optional_and_tonumber_match_jq_5675() -> Result<()> {
    const ROWS: &[(&str, &str, &str)] = &[
        (
            "[.[]|fromjson?]",
            r#"["1","nan","[1,","{\"a\":1}","\"x\"","tru"]"#,
            "[1,null,{\"a\":1},\"x\"]\n",
        ),
        (
            "[.[]|tonumber?]",
            r#"["1","1.50"," 7","nan","[1]","0x10","1e3",""]"#,
            "[1,1.50,7,null,1E+3]\n",
        ),
        (
            "[.[]|try tonumber catch .]",
            r#"["[1]","null","0x10","true","{}"]"#,
            "[\"string (\\\"[1]\\\") cannot be parsed as a number\",\"string (\\\"null\\\") cannot be parsed as a number\",\"Invalid numeric literal at EOF at line 1, column 4 (while parsing '0x10')\",\"string (\\\"true\\\") cannot be parsed as a number\",\"string (\\\"{}\\\") cannot be parsed as a number\"]\n",
        ),
    ];
    for (filter, input, stdout) in ROWS {
        let (out, err, code) = run_jq_full(&["-c", filter], Some(input))?;
        assert_eq!(
            (out.as_str(), err.as_str(), code),
            (*stdout, "", 0),
            "{filter} over {input}"
        );
    }
    Ok(())
}

/// `(arguments, stdin, expected stdout, expected stderr, expected exit status)`.
type RawRow = (
    &'static [&'static str],
    &'static str,
    &'static str,
    &'static str,
    i32,
);

/// Rows captured from jq 1.7.1 (`/usr/bin/jq`): `-R` hands each line to the filter as the string it
/// is, and `length`, `select(length ...)` and the string builtins read it where it is instead of
/// writing it out as JSON text, indexing it and reading it back. A multi-byte line, a CRLF line, an
/// empty line, a last line with no newline, an escape in the text, `input_line_number`, `--seq`,
/// `-s` and `-n` with `inputs` pin what that round trip used to decide.
const RAW_ROWS: &[RawRow] = &[
    (&["-R", "length"], "abc\nhéllo\n世界\n😀x\n\n", "3\n5\n2\n2\n0\n", "", 0),
    (&["-R", "length"], "a\r\nbb\r\n", "2\n3\n", "", 0),
    (&["-R", "length"], "no newline at end", "17\n", "", 0),
    (&["-R", "length"], "", "", "", 0),
    (&["-R", "length"], "\n\n", "0\n0\n", "", 0),
    (&["-R", "length"], "a\"b\\c\td\n{\"id\":1}\n", "7\n8\n", "", 0),
    (&["-R", "utf8bytelength"], "abc\nhéllo\n世界\n😀x\n\n", "3\n6\n6\n5\n0\n", "", 0),
    (&["-R", "utf8bytelength"], "a\r\nbb\r\n", "2\n3\n", "", 0),
    (&["-R", "utf8bytelength"], "no newline at end", "17\n", "", 0),
    (&["-R", "utf8bytelength"], "", "", "", 0),
    (&["-R", "utf8bytelength"], "\n\n", "0\n0\n", "", 0),
    (&["-R", "utf8bytelength"], "a\"b\\c\td\n{\"id\":1}\n", "7\n8\n", "", 0),
    (&["-Rc", "[., length]"], "abc\nhéllo\n世界\n😀x\n\n", "[\"abc\",3]\n[\"héllo\",5]\n[\"世界\",2]\n[\"😀x\",2]\n[\"\",0]\n", "", 0),
    (&["-Rc", "[., length]"], "a\r\nbb\r\n", "[\"a\\r\",2]\n[\"bb\\r\",3]\n", "", 0),
    (&["-Rc", "[., length]"], "no newline at end", "[\"no newline at end\",17]\n", "", 0),
    (&["-Rc", "[., length]"], "", "", "", 0),
    (&["-Rc", "[., length]"], "\n\n", "[\"\",0]\n[\"\",0]\n", "", 0),
    (&["-Rc", "[., length]"], "a\"b\\c\td\n{\"id\":1}\n", "[\"a\\\"b\\\\c\\td\",7]\n[\"{\\\"id\\\":1}\",8]\n", "", 0),
    (&["-R", "select(length>2)"], "abc\nhéllo\n世界\n😀x\n\n", "\"abc\"\n\"héllo\"\n", "", 0),
    (&["-R", "select(length>2)"], "a\r\nbb\r\n", "\"bb\\r\"\n", "", 0),
    (&["-R", "select(length>2)"], "no newline at end", "\"no newline at end\"\n", "", 0),
    (&["-R", "select(length>2)"], "", "", "", 0),
    (&["-R", "select(length>2)"], "\n\n", "", "", 0),
    (&["-R", "select(length>2)"], "a\"b\\c\td\n{\"id\":1}\n", "\"a\\\"b\\\\c\\td\"\n\"{\\\"id\\\":1}\"\n", "", 0),
    (&["-R", "select(length==0)|\"empty\""], "abc\nhéllo\n世界\n😀x\n\n", "\"empty\"\n", "", 0),
    (&["-R", "select(length==0)|\"empty\""], "a\r\nbb\r\n", "", "", 0),
    (&["-R", "select(length==0)|\"empty\""], "no newline at end", "", "", 0),
    (&["-R", "select(length==0)|\"empty\""], "", "", "", 0),
    (&["-R", "select(length==0)|\"empty\""], "\n\n", "\"empty\"\n\"empty\"\n", "", 0),
    (&["-R", "select(length==0)|\"empty\""], "a\"b\\c\td\n{\"id\":1}\n", "", "", 0),
    (&["-R", "ascii_downcase"], "abc\nhéllo\n世界\n😀x\n\n", "\"abc\"\n\"héllo\"\n\"世界\"\n\"😀x\"\n\"\"\n", "", 0),
    (&["-R", "ascii_downcase"], "a\r\nbb\r\n", "\"a\\r\"\n\"bb\\r\"\n", "", 0),
    (&["-R", "ascii_downcase"], "no newline at end", "\"no newline at end\"\n", "", 0),
    (&["-R", "ascii_downcase"], "", "", "", 0),
    (&["-R", "ascii_downcase"], "\n\n", "\"\"\n\"\"\n", "", 0),
    (&["-R", "ascii_downcase"], "a\"b\\c\td\n{\"id\":1}\n", "\"a\\\"b\\\\c\\td\"\n\"{\\\"id\\\":1}\"\n", "", 0),
    (&["-R", "test(\"b\")"], "abc\nhéllo\n世界\n😀x\n\n", "true\nfalse\nfalse\nfalse\nfalse\n", "", 0),
    (&["-R", "test(\"b\")"], "a\r\nbb\r\n", "false\ntrue\n", "", 0),
    (&["-R", "test(\"b\")"], "no newline at end", "false\n", "", 0),
    (&["-R", "test(\"b\")"], "", "", "", 0),
    (&["-R", "test(\"b\")"], "\n\n", "false\nfalse\n", "", 0),
    (&["-R", "test(\"b\")"], "a\"b\\c\td\n{\"id\":1}\n", "true\nfalse\n", "", 0),
    (&["-Rr", "ascii_upcase"], "abc\nhéllo\n世界\n😀x\n\n", "ABC\nHéLLO\n世界\n😀X\n\n", "", 0),
    (&["-Rr", "ascii_upcase"], "a\r\nbb\r\n", "A\r\nBB\r\n", "", 0),
    (&["-Rr", "ascii_upcase"], "no newline at end", "NO NEWLINE AT END\n", "", 0),
    (&["-Rr", "ascii_upcase"], "", "", "", 0),
    (&["-Rr", "ascii_upcase"], "\n\n", "\n\n", "", 0),
    (&["-Rr", "ascii_upcase"], "a\"b\\c\td\n{\"id\":1}\n", "A\"B\\C\tD\n{\"ID\":1}\n", "", 0),
    (&["-Rc", "[input_line_number, length]"], "abc\nhéllo\n世界\n😀x\n\n", "[1,3]\n[2,5]\n[3,2]\n[4,2]\n[5,0]\n", "", 0),
    (&["-Rc", "[input_line_number, length]"], "a\r\nbb\r\n", "[1,2]\n[2,3]\n", "", 0),
    (&["-Rc", "[input_line_number, length]"], "no newline at end", "[0,17]\n", "", 0),
    (&["-Rc", "[input_line_number, length]"], "", "", "", 0),
    (&["-Rc", "[input_line_number, length]"], "\n\n", "[1,0]\n[2,0]\n", "", 0),
    (&["-Rc", "[input_line_number, length]"], "a\"b\\c\td\n{\"id\":1}\n", "[1,7]\n[2,8]\n", "", 0),
    (&["-R", "error"], "abc\nhéllo\n世界\n😀x\n\n", "", "jq: error (at <stdin>:1): abc\njq: error (at <stdin>:2): héllo\njq: error (at <stdin>:3): 世界\njq: error (at <stdin>:4): 😀x\njq: error (at <stdin>:5): \n", 5),
    (&["-R", "error"], "a\r\nbb\r\n", "", "jq: error (at <stdin>:1): a\r\njq: error (at <stdin>:2): bb\r\n", 5),
    (&["-R", "error"], "no newline at end", "", "jq: error (at <stdin>:0): no newline at end\n", 5),
    (&["-R", "error"], "", "", "", 0),
    (&["-R", "error"], "\n\n", "", "jq: error (at <stdin>:1): \njq: error (at <stdin>:2): \n", 5),
    (&["-R", "error"], "a\"b\\c\td\n{\"id\":1}\n", "", "jq: error (at <stdin>:1): a\"b\\c\td\njq: error (at <stdin>:2): {\"id\":1}\n", 5),
    (&["-Rc", "try error catch [., length]"], "abc\nhéllo\n世界\n😀x\n\n", "[\"abc\",3]\n[\"héllo\",5]\n[\"世界\",2]\n[\"😀x\",2]\n[\"\",0]\n", "", 0),
    (&["-Rc", "try error catch [., length]"], "a\r\nbb\r\n", "[\"a\\r\",2]\n[\"bb\\r\",3]\n", "", 0),
    (&["-Rc", "try error catch [., length]"], "no newline at end", "[\"no newline at end\",17]\n", "", 0),
    (&["-Rc", "try error catch [., length]"], "", "", "", 0),
    (&["-Rc", "try error catch [., length]"], "\n\n", "[\"\",0]\n[\"\",0]\n", "", 0),
    (&["-Rc", "try error catch [., length]"], "a\"b\\c\td\n{\"id\":1}\n", "[\"a\\\"b\\\\c\\td\",7]\n[\"{\\\"id\\\":1}\",8]\n", "", 0),
    (&["-Rc", "length as $n | [$n, (. | length)]"], "abc\nhéllo\n世界\n😀x\n\n", "[3,3]\n[5,5]\n[2,2]\n[2,2]\n[0,0]\n", "", 0),
    (&["-Rc", "length as $n | [$n, (. | length)]"], "a\r\nbb\r\n", "[2,2]\n[3,3]\n", "", 0),
    (&["-Rc", "length as $n | [$n, (. | length)]"], "no newline at end", "[17,17]\n", "", 0),
    (&["-Rc", "length as $n | [$n, (. | length)]"], "", "", "", 0),
    (&["-Rc", "length as $n | [$n, (. | length)]"], "\n\n", "[0,0]\n[0,0]\n", "", 0),
    (&["-Rc", "length as $n | [$n, (. | length)]"], "a\"b\\c\td\n{\"id\":1}\n", "[7,7]\n[8,8]\n", "", 0),
    (&["-Rs", "length"], "abc\nhéllo\n世界\n😀x\n\n", "17\n", "", 0),
    (&["-Rs", "length"], "a\r\nbb\r\n", "7\n", "", 0),
    (&["-Rs", "length"], "no newline at end", "17\n", "", 0),
    (&["-Rs", "length"], "", "0\n", "", 0),
    (&["-Rs", "length"], "\n\n", "2\n", "", 0),
    (&["-Rs", "length"], "a\"b\\c\td\n{\"id\":1}\n", "17\n", "", 0),
    (&["-Rc", "(. | length) > 1"], "abc\nhéllo\n世界\n😀x\n\n", "true\ntrue\ntrue\ntrue\nfalse\n", "", 0),
    (&["-Rc", "(. | length) > 1"], "a\r\nbb\r\n", "true\ntrue\n", "", 0),
    (&["-Rc", "(. | length) > 1"], "no newline at end", "true\n", "", 0),
    (&["-Rc", "(. | length) > 1"], "", "", "", 0),
    (&["-Rc", "(. | length) > 1"], "\n\n", "false\nfalse\n", "", 0),
    (&["-Rc", "(. | length) > 1"], "a\"b\\c\td\n{\"id\":1}\n", "true\ntrue\n", "", 0),
    (&["-Rc", "tojson"], "abc\nhéllo\n世界\n😀x\n\n", "\"\\\"abc\\\"\"\n\"\\\"héllo\\\"\"\n\"\\\"世界\\\"\"\n\"\\\"😀x\\\"\"\n\"\\\"\\\"\"\n", "", 0),
    (&["-Rc", "tojson"], "a\r\nbb\r\n", "\"\\\"a\\\\r\\\"\"\n\"\\\"bb\\\\r\\\"\"\n", "", 0),
    (&["-Rc", "tojson"], "no newline at end", "\"\\\"no newline at end\\\"\"\n", "", 0),
    (&["-Rc", "tojson"], "", "", "", 0),
    (&["-Rc", "tojson"], "\n\n", "\"\\\"\\\"\"\n\"\\\"\\\"\"\n", "", 0),
    (&["-Rc", "tojson"], "a\"b\\c\td\n{\"id\":1}\n", "\"\\\"a\\\\\\\"b\\\\\\\\c\\\\td\\\"\"\n\"\\\"{\\\\\\\"id\\\\\\\":1}\\\"\"\n", "", 0),
    (&["-Rr", "@json"], "abc\nhéllo\n世界\n😀x\n\n", "\"abc\"\n\"héllo\"\n\"世界\"\n\"😀x\"\n\"\"\n", "", 0),
    (&["-Rr", "@json"], "a\r\nbb\r\n", "\"a\\r\"\n\"bb\\r\"\n", "", 0),
    (&["-Rr", "@json"], "no newline at end", "\"no newline at end\"\n", "", 0),
    (&["-Rr", "@json"], "", "", "", 0),
    (&["-Rr", "@json"], "\n\n", "\"\"\n\"\"\n", "", 0),
    (&["-Rr", "@json"], "a\"b\\c\td\n{\"id\":1}\n", "\"a\\\"b\\\\c\\td\"\n\"{\\\"id\\\":1}\"\n", "", 0),
    (&["-Rc", "try .a catch ."], "abc\nhéllo\n世界\n😀x\n\n", "\"Cannot index string with string \\\"a\\\"\"\n\"Cannot index string with string \\\"a\\\"\"\n\"Cannot index string with string \\\"a\\\"\"\n\"Cannot index string with string \\\"a\\\"\"\n\"Cannot index string with string \\\"a\\\"\"\n", "", 0),
    (&["-Rc", "try .a catch ."], "a\r\nbb\r\n", "\"Cannot index string with string \\\"a\\\"\"\n\"Cannot index string with string \\\"a\\\"\"\n", "", 0),
    (&["-Rc", "try .a catch ."], "no newline at end", "\"Cannot index string with string \\\"a\\\"\"\n", "", 0),
    (&["-Rc", "try .a catch ."], "", "", "", 0),
    (&["-Rc", "try .a catch ."], "\n\n", "\"Cannot index string with string \\\"a\\\"\"\n\"Cannot index string with string \\\"a\\\"\"\n", "", 0),
    (&["-Rc", "try .a catch ."], "a\"b\\c\td\n{\"id\":1}\n", "\"Cannot index string with string \\\"a\\\"\"\n\"Cannot index string with string \\\"a\\\"\"\n", "", 0),
    (&["-Rc", "try .[0] catch ."], "abc\nhéllo\n世界\n😀x\n\n", "\"Cannot index string with number\"\n\"Cannot index string with number\"\n\"Cannot index string with number\"\n\"Cannot index string with number\"\n\"Cannot index string with number\"\n", "", 0),
    (&["-Rc", "try .[0] catch ."], "a\r\nbb\r\n", "\"Cannot index string with number\"\n\"Cannot index string with number\"\n", "", 0),
    (&["-Rc", "try .[0] catch ."], "no newline at end", "\"Cannot index string with number\"\n", "", 0),
    (&["-Rc", "try .[0] catch ."], "", "", "", 0),
    (&["-Rc", "try .[0] catch ."], "\n\n", "\"Cannot index string with number\"\n\"Cannot index string with number\"\n", "", 0),
    (&["-Rc", "try .[0] catch ."], "a\"b\\c\td\n{\"id\":1}\n", "\"Cannot index string with number\"\n\"Cannot index string with number\"\n", "", 0),
    (&["-Rc", ".[1:3]"], "abc\nhéllo\n世界\n😀x\n\n", "\"bc\"\n\"él\"\n\"界\"\n\"x\"\n\"\"\n", "", 0),
    (&["-Rc", ".[1:3]"], "a\r\nbb\r\n", "\"\\r\"\n\"b\\r\"\n", "", 0),
    (&["-Rc", ".[1:3]"], "no newline at end", "\"o \"\n", "", 0),
    (&["-Rc", ".[1:3]"], "", "", "", 0),
    (&["-Rc", ".[1:3]"], "\n\n", "\"\"\n\"\"\n", "", 0),
    (&["-Rc", ".[1:3]"], "a\"b\\c\td\n{\"id\":1}\n", "\"\\\"b\"\n\"\\\"i\"\n", "", 0),
    (&["-Rc", "[splits(\"b\")]"], "abc\nhéllo\n世界\n😀x\n\n", "[\"a\",\"c\"]\n[\"héllo\"]\n[\"世界\"]\n[\"😀x\"]\n[\"\"]\n", "", 0),
    (&["-Rc", "[splits(\"b\")]"], "a\r\nbb\r\n", "[\"a\\r\"]\n[\"\",\"\",\"\\r\"]\n", "", 0),
    (&["-Rc", "[splits(\"b\")]"], "no newline at end", "[\"no newline at end\"]\n", "", 0),
    (&["-Rc", "[splits(\"b\")]"], "", "", "", 0),
    (&["-Rc", "[splits(\"b\")]"], "\n\n", "[\"\"]\n[\"\"]\n", "", 0),
    (&["-Rc", "[splits(\"b\")]"], "a\"b\\c\td\n{\"id\":1}\n", "[\"a\\\"\",\"\\\\c\\td\"]\n[\"{\\\"id\\\":1}\"]\n", "", 0),
    (&["-Rs", "length"], "abc\nhéllo\n世界\n😀x\n\n", "17\n", "", 0),
    (&["-Rs", "length"], "a\r\nbb\r\n", "7\n", "", 0),
    (&["-Rs", "length"], "no newline at end", "17\n", "", 0),
    (&["-Rs", "length"], "", "0\n", "", 0),
    (&["-Rs", "length"], "\n\n", "2\n", "", 0),
    (&["-Rs", "length"], "a\"b\\c\td\n{\"id\":1}\n", "17\n", "", 0),
    (&["-Rs", "[.,length]"], "abc\nhéllo\n世界\n😀x\n\n", "[\n  \"abc\\nhéllo\\n世界\\n😀x\\n\\n\",\n  17\n]\n", "", 0),
    (&["-Rs", "[.,length]"], "a\r\nbb\r\n", "[\n  \"a\\r\\nbb\\r\\n\",\n  7\n]\n", "", 0),
    (&["-Rs", "[.,length]"], "no newline at end", "[\n  \"no newline at end\",\n  17\n]\n", "", 0),
    (&["-Rs", "[.,length]"], "", "[\n  \"\",\n  0\n]\n", "", 0),
    (&["-Rs", "[.,length]"], "\n\n", "[\n  \"\\n\\n\",\n  2\n]\n", "", 0),
    (&["-Rs", "[.,length]"], "a\"b\\c\td\n{\"id\":1}\n", "[\n  \"a\\\"b\\\\c\\td\\n{\\\"id\\\":1}\\n\",\n  17\n]\n", "", 0),
    (&["-Rsc", "ascii_downcase|length"], "abc\nhéllo\n世界\n😀x\n\n", "17\n", "", 0),
    (&["-Rsc", "ascii_downcase|length"], "a\r\nbb\r\n", "7\n", "", 0),
    (&["-Rsc", "ascii_downcase|length"], "no newline at end", "17\n", "", 0),
    (&["-Rsc", "ascii_downcase|length"], "", "0\n", "", 0),
    (&["-Rsc", "ascii_downcase|length"], "\n\n", "2\n", "", 0),
    (&["-Rsc", "ascii_downcase|length"], "a\"b\\c\td\n{\"id\":1}\n", "17\n", "", 0),
    (&["-nRc", "[inputs|length]"], "abc\nhéllo\n世界\n😀x\n\n", "[3,5,2,2,0]\n", "", 0),
    (&["-nRc", "[inputs|length]"], "a\r\nbb\r\n", "[2,3]\n", "", 0),
    (&["-nRc", "[inputs|length]"], "no newline at end", "[17]\n", "", 0),
    (&["-nRc", "[inputs|length]"], "", "[]\n", "", 0),
    (&["-nRc", "[inputs|length]"], "\n\n", "[0,0]\n", "", 0),
    (&["-nRc", "[inputs|length]"], "a\"b\\c\td\n{\"id\":1}\n", "[7,8]\n", "", 0),
    (&["-nRc", "input|length"], "abc\nhéllo\n世界\n😀x\n\n", "3\n", "", 0),
    (&["-nRc", "input|length"], "a\r\nbb\r\n", "2\n", "", 0),
    (&["-nRc", "input|length"], "no newline at end", "17\n", "", 0),
    (&["-nRc", "input|length"], "", "", "jq: error (at <stdin>:0): break\n", 5),
    (&["-nRc", "input|length"], "\n\n", "0\n", "", 0),
    (&["-nRc", "input|length"], "a\"b\\c\td\n{\"id\":1}\n", "7\n", "", 0),
    (&["-Rc", "--seq", "length"], "abc\nhéllo\n世界\n😀x\n\n", "\u{1e}3\n\u{1e}5\n\u{1e}2\n\u{1e}2\n\u{1e}0\n", "", 0),
    (&["-Rc", "--seq", "length"], "a\r\nbb\r\n", "\u{1e}2\n\u{1e}3\n", "", 0),
    (&["-Rc", "--seq", "length"], "no newline at end", "\u{1e}17\n", "", 0),
    (&["-Rc", "--seq", "length"], "", "", "", 0),
    (&["-Rc", "--seq", "length"], "\n\n", "\u{1e}0\n\u{1e}0\n", "", 0),
    (&["-Rc", "--seq", "length"], "a\"b\\c\td\n{\"id\":1}\n", "\u{1e}7\n\u{1e}8\n", "", 0),
    (&["-Rc", "--arg", "a", "1", "[$a,length]"], "abc\nhéllo\n世界\n😀x\n\n", "[\"1\",3]\n[\"1\",5]\n[\"1\",2]\n[\"1\",2]\n[\"1\",0]\n", "", 0),
    (&["-Rc", "--arg", "a", "1", "[$a,length]"], "a\r\nbb\r\n", "[\"1\",2]\n[\"1\",3]\n", "", 0),
    (&["-Rc", "--arg", "a", "1", "[$a,length]"], "no newline at end", "[\"1\",17]\n", "", 0),
    (&["-Rc", "--arg", "a", "1", "[$a,length]"], "", "", "", 0),
    (&["-Rc", "--arg", "a", "1", "[$a,length]"], "\n\n", "[\"1\",0]\n[\"1\",0]\n", "", 0),
    (&["-Rc", "--arg", "a", "1", "[$a,length]"], "a\"b\\c\td\n{\"id\":1}\n", "[\"1\",7]\n[\"1\",8]\n", "", 0),
    (&["-Rc", "select(length>1)|[length,.]"], "abc\nhéllo\n世界\n😀x\n\n", "[3,\"abc\"]\n[5,\"héllo\"]\n[2,\"世界\"]\n[2,\"😀x\"]\n", "", 0),
    (&["-Rc", "select(length>1)|[length,.]"], "a\r\nbb\r\n", "[2,\"a\\r\"]\n[3,\"bb\\r\"]\n", "", 0),
    (&["-Rc", "select(length>1)|[length,.]"], "no newline at end", "[17,\"no newline at end\"]\n", "", 0),
    (&["-Rc", "select(length>1)|[length,.]"], "", "", "", 0),
    (&["-Rc", "select(length>1)|[length,.]"], "\n\n", "", "", 0),
    (&["-Rc", "select(length>1)|[length,.]"], "a\"b\\c\td\n{\"id\":1}\n", "[7,\"a\\\"b\\\\c\\td\"]\n[8,\"{\\\"id\\\":1}\"]\n", "", 0),
];

#[test]
fn test_raw_input_lines_are_read_where_they_are_5675() -> Result<()> {
    for (args, stdin, stdout, stderr, status) in RAW_ROWS {
        let (out, err, code) = run_jq_full(args, Some(stdin))?;
        assert_eq!(
            (out.as_str(), err.as_str(), code),
            (*stdout, *stderr, *status),
            "{args:?} over {stdin:?}"
        );
    }
    Ok(())
}
