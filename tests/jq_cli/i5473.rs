//! #5473: `-C` writes jq 1.7.1's bytes. jq writes a container's color before its opening bracket
//! with no reset after it, again after every element and before the closing bracket, and resets
//! only after the closing bracket, before each object key and after each `:`; an empty container
//! is its color, the two brackets and a reset. Every row's expected bytes were captured from
//! `/usr/bin/jq` 1.7.1 (`JQ_COLORS` unset, or the eight-field spec in the row).

use anyhow::Result;

use crate::spawn_jq_with_env;

/// jq 1.7.1's default `JQ_COLORS`: null, false, true, numbers, strings, arrays, objects, keys.
const DEFAULT_SPEC: &str = "0;90:0;39:0;39:0;39:0;32:1;39:1;39:1;34";

type Row = (
    &'static str,
    &'static [&'static str],
    Option<&'static str>,
    &'static str,
);

#[test]
fn test_color_bytes_match_jq_5473() -> Result<()> {
    let rows: &[Row] = &[
        ("1", &["-C"], None, "\x1b[0;39m1\x1b[0m\n"),
        ("\"x\"", &["-C"], None, "\x1b[0;32m\"x\"\x1b[0m\n"),
        ("null", &["-C"], None, "\x1b[0;90mnull\x1b[0m\n"),
        ("[]", &["-C", "-c"], None, "\x1b[1;39m[]\x1b[0m\n"),
        ("{}", &["-C", "-c"], None, "\x1b[1;39m{}\x1b[0m\n"),
        ("[]", &["-C"], None, "\x1b[1;39m[]\x1b[0m\n"),
        ("{}", &["-C"], None, "\x1b[1;39m{}\x1b[0m\n"),
        ("[1]", &["-C", "-c"], None, "\x1b[1;39m[\x1b[0;39m1\x1b[0m\x1b[1;39m\x1b[1;39m]\x1b[0m\n"),
        ("{\"a\":1}", &["-C", "-c"], None, "\x1b[1;39m{\x1b[0m\x1b[1;34m\"a\"\x1b[0m\x1b[1;39m:\x1b[0m\x1b[0;39m1\x1b[0m\x1b[1;39m\x1b[1;39m}\x1b[0m\n"),
        ("{\"a\":[1,\"x\",null,true]}", &["-C", "-c"], None, "\x1b[1;39m{\x1b[0m\x1b[1;34m\"a\"\x1b[0m\x1b[1;39m:\x1b[0m\x1b[1;39m[\x1b[0;39m1\x1b[0m\x1b[1;39m,\x1b[0;32m\"x\"\x1b[0m\x1b[1;39m,\x1b[0;90mnull\x1b[0m\x1b[1;39m,\x1b[0;39mtrue\x1b[0m\x1b[1;39m\x1b[1;39m]\x1b[0m\x1b[1;39m\x1b[1;39m}\x1b[0m\n"),
        ("{\"a\":[1,\"x\",null,true]}", &["-C"], None, "\x1b[1;39m{\n  \x1b[0m\x1b[1;34m\"a\"\x1b[0m\x1b[1;39m: \x1b[0m\x1b[1;39m[\n    \x1b[0;39m1\x1b[0m\x1b[1;39m,\n    \x1b[0;32m\"x\"\x1b[0m\x1b[1;39m,\n    \x1b[0;90mnull\x1b[0m\x1b[1;39m,\n    \x1b[0;39mtrue\x1b[0m\x1b[1;39m\n  \x1b[1;39m]\x1b[0m\x1b[1;39m\n\x1b[1;39m}\x1b[0m\n"),
        ("[[1,2],[3]]", &["-C", "-c"], None, "\x1b[1;39m[\x1b[1;39m[\x1b[0;39m1\x1b[0m\x1b[1;39m,\x1b[0;39m2\x1b[0m\x1b[1;39m\x1b[1;39m]\x1b[0m\x1b[1;39m,\x1b[1;39m[\x1b[0;39m3\x1b[0m\x1b[1;39m\x1b[1;39m]\x1b[0m\x1b[1;39m\x1b[1;39m]\x1b[0m\n"),
        ("[[],{}]", &["-C", "-c"], None, "\x1b[1;39m[\x1b[1;39m[]\x1b[0m\x1b[1;39m,\x1b[1;39m{}\x1b[0m\x1b[1;39m\x1b[1;39m]\x1b[0m\n"),
        ("[[],{}]", &["-C"], None, "\x1b[1;39m[\n  \x1b[1;39m[]\x1b[0m\x1b[1;39m,\n  \x1b[1;39m{}\x1b[0m\x1b[1;39m\n\x1b[1;39m]\x1b[0m\n"),
        ("{\"a\":{\"b\":{\"c\":[]}}}", &["-C", "-c"], None, "\x1b[1;39m{\x1b[0m\x1b[1;34m\"a\"\x1b[0m\x1b[1;39m:\x1b[0m\x1b[1;39m{\x1b[0m\x1b[1;34m\"b\"\x1b[0m\x1b[1;39m:\x1b[0m\x1b[1;39m{\x1b[0m\x1b[1;34m\"c\"\x1b[0m\x1b[1;39m:\x1b[0m\x1b[1;39m[]\x1b[0m\x1b[1;39m\x1b[1;39m}\x1b[0m\x1b[1;39m\x1b[1;39m}\x1b[0m\x1b[1;39m\x1b[1;39m}\x1b[0m\n"),
        ("{\"a\":{\"b\":{\"c\":[]}}}", &["-C"], None, "\x1b[1;39m{\n  \x1b[0m\x1b[1;34m\"a\"\x1b[0m\x1b[1;39m: \x1b[0m\x1b[1;39m{\n    \x1b[0m\x1b[1;34m\"b\"\x1b[0m\x1b[1;39m: \x1b[0m\x1b[1;39m{\n      \x1b[0m\x1b[1;34m\"c\"\x1b[0m\x1b[1;39m: \x1b[0m\x1b[1;39m[]\x1b[0m\x1b[1;39m\n    \x1b[1;39m}\x1b[0m\x1b[1;39m\n  \x1b[1;39m}\x1b[0m\x1b[1;39m\n\x1b[1;39m}\x1b[0m\n"),
        ("{\"a\":1,\"b\":\"x\",\"c\":null,\"d\":true,\"e\":[1,2],\"f\":{\"g\":1}}", &["-C", "-c"], None, "\x1b[1;39m{\x1b[0m\x1b[1;34m\"a\"\x1b[0m\x1b[1;39m:\x1b[0m\x1b[0;39m1\x1b[0m\x1b[1;39m,\x1b[0m\x1b[1;34m\"b\"\x1b[0m\x1b[1;39m:\x1b[0m\x1b[0;32m\"x\"\x1b[0m\x1b[1;39m,\x1b[0m\x1b[1;34m\"c\"\x1b[0m\x1b[1;39m:\x1b[0m\x1b[0;90mnull\x1b[0m\x1b[1;39m,\x1b[0m\x1b[1;34m\"d\"\x1b[0m\x1b[1;39m:\x1b[0m\x1b[0;39mtrue\x1b[0m\x1b[1;39m,\x1b[0m\x1b[1;34m\"e\"\x1b[0m\x1b[1;39m:\x1b[0m\x1b[1;39m[\x1b[0;39m1\x1b[0m\x1b[1;39m,\x1b[0;39m2\x1b[0m\x1b[1;39m\x1b[1;39m]\x1b[0m\x1b[1;39m,\x1b[0m\x1b[1;34m\"f\"\x1b[0m\x1b[1;39m:\x1b[0m\x1b[1;39m{\x1b[0m\x1b[1;34m\"g\"\x1b[0m\x1b[1;39m:\x1b[0m\x1b[0;39m1\x1b[0m\x1b[1;39m\x1b[1;39m}\x1b[0m\x1b[1;39m\x1b[1;39m}\x1b[0m\n"),
        ("{\"a\":1,\"b\":\"x\",\"c\":null,\"d\":true,\"e\":[1,2],\"f\":{\"g\":1}}", &["-C"], None, "\x1b[1;39m{\n  \x1b[0m\x1b[1;34m\"a\"\x1b[0m\x1b[1;39m: \x1b[0m\x1b[0;39m1\x1b[0m\x1b[1;39m,\n  \x1b[0m\x1b[1;34m\"b\"\x1b[0m\x1b[1;39m: \x1b[0m\x1b[0;32m\"x\"\x1b[0m\x1b[1;39m,\n  \x1b[0m\x1b[1;34m\"c\"\x1b[0m\x1b[1;39m: \x1b[0m\x1b[0;90mnull\x1b[0m\x1b[1;39m,\n  \x1b[0m\x1b[1;34m\"d\"\x1b[0m\x1b[1;39m: \x1b[0m\x1b[0;39mtrue\x1b[0m\x1b[1;39m,\n  \x1b[0m\x1b[1;34m\"e\"\x1b[0m\x1b[1;39m: \x1b[0m\x1b[1;39m[\n    \x1b[0;39m1\x1b[0m\x1b[1;39m,\n    \x1b[0;39m2\x1b[0m\x1b[1;39m\n  \x1b[1;39m]\x1b[0m\x1b[1;39m,\n  \x1b[0m\x1b[1;34m\"f\"\x1b[0m\x1b[1;39m: \x1b[0m\x1b[1;39m{\n    \x1b[0m\x1b[1;34m\"g\"\x1b[0m\x1b[1;39m: \x1b[0m\x1b[0;39m1\x1b[0m\x1b[1;39m\n  \x1b[1;39m}\x1b[0m\x1b[1;39m\n\x1b[1;39m}\x1b[0m\n"),
        ("{\"a\":1,\"b\":[2,{\"c\":[]}]}", &["-C", "--tab"], None, "\x1b[1;39m{\n\t\x1b[0m\x1b[1;34m\"a\"\x1b[0m\x1b[1;39m: \x1b[0m\x1b[0;39m1\x1b[0m\x1b[1;39m,\n\t\x1b[0m\x1b[1;34m\"b\"\x1b[0m\x1b[1;39m: \x1b[0m\x1b[1;39m[\n\t\t\x1b[0;39m2\x1b[0m\x1b[1;39m,\n\t\t\x1b[1;39m{\n\t\t\t\x1b[0m\x1b[1;34m\"c\"\x1b[0m\x1b[1;39m: \x1b[0m\x1b[1;39m[]\x1b[0m\x1b[1;39m\n\t\t\x1b[1;39m}\x1b[0m\x1b[1;39m\n\t\x1b[1;39m]\x1b[0m\x1b[1;39m\n\x1b[1;39m}\x1b[0m\n"),
        ("{\"a\":1,\"b\":[2,{\"c\":[]}]}", &["-C", "--indent", "1"], None, "\x1b[1;39m{\n \x1b[0m\x1b[1;34m\"a\"\x1b[0m\x1b[1;39m: \x1b[0m\x1b[0;39m1\x1b[0m\x1b[1;39m,\n \x1b[0m\x1b[1;34m\"b\"\x1b[0m\x1b[1;39m: \x1b[0m\x1b[1;39m[\n  \x1b[0;39m2\x1b[0m\x1b[1;39m,\n  \x1b[1;39m{\n   \x1b[0m\x1b[1;34m\"c\"\x1b[0m\x1b[1;39m: \x1b[0m\x1b[1;39m[]\x1b[0m\x1b[1;39m\n  \x1b[1;39m}\x1b[0m\x1b[1;39m\n \x1b[1;39m]\x1b[0m\x1b[1;39m\n\x1b[1;39m}\x1b[0m\n"),
        ("{\"a\":1,\"b\":[2,{\"c\":[]}]}", &["-C", "--indent", "0"], None, "\x1b[1;39m{\x1b[0m\x1b[1;34m\"a\"\x1b[0m\x1b[1;39m:\x1b[0m\x1b[0;39m1\x1b[0m\x1b[1;39m,\x1b[0m\x1b[1;34m\"b\"\x1b[0m\x1b[1;39m:\x1b[0m\x1b[1;39m[\x1b[0;39m2\x1b[0m\x1b[1;39m,\x1b[1;39m{\x1b[0m\x1b[1;34m\"c\"\x1b[0m\x1b[1;39m:\x1b[0m\x1b[1;39m[]\x1b[0m\x1b[1;39m\x1b[1;39m}\x1b[0m\x1b[1;39m\x1b[1;39m]\x1b[0m\x1b[1;39m\x1b[1;39m}\x1b[0m\n"),
        ("{\"b\":1,\"a\":[2,{\"d\":1,\"c\":2}]}", &["-C", "-S"], None, "\x1b[1;39m{\n  \x1b[0m\x1b[1;34m\"a\"\x1b[0m\x1b[1;39m: \x1b[0m\x1b[1;39m[\n    \x1b[0;39m2\x1b[0m\x1b[1;39m,\n    \x1b[1;39m{\n      \x1b[0m\x1b[1;34m\"c\"\x1b[0m\x1b[1;39m: \x1b[0m\x1b[0;39m2\x1b[0m\x1b[1;39m,\n      \x1b[0m\x1b[1;34m\"d\"\x1b[0m\x1b[1;39m: \x1b[0m\x1b[0;39m1\x1b[0m\x1b[1;39m\n    \x1b[1;39m}\x1b[0m\x1b[1;39m\n  \x1b[1;39m]\x1b[0m\x1b[1;39m,\n  \x1b[0m\x1b[1;34m\"b\"\x1b[0m\x1b[1;39m: \x1b[0m\x1b[0;39m1\x1b[0m\x1b[1;39m\n\x1b[1;39m}\x1b[0m\n"),
        ("\"a\\nb\\u00e9\\ud83d\\ude00\"", &["-C", "-a"], None, "\x1b[0;32m\"a\\nb\\u00e9\\ud83d\\ude00\"\x1b[0m\n"),
        ("[\"\\u00e9\",{\"\\u00e9\":\"\\u00e9\"}]", &["-C", "-c", "-a"], None, "\x1b[1;39m[\x1b[0;32m\"\\u00e9\"\x1b[0m\x1b[1;39m,\x1b[1;39m{\x1b[0m\x1b[1;34m\"\\u00e9\"\x1b[0m\x1b[1;39m:\x1b[0m\x1b[0;32m\"\\u00e9\"\x1b[0m\x1b[1;39m\x1b[1;39m}\x1b[0m\x1b[1;39m\x1b[1;39m]\x1b[0m\n"),
        ("{\"a\":[1,\"x\",null,true,false],\"b\":{}}", &["-C", "-c"], None, "\x1b[1;39m{\x1b[0m\x1b[1;34m\"a\"\x1b[0m\x1b[1;39m:\x1b[0m\x1b[1;39m[\x1b[0;39m1\x1b[0m\x1b[1;39m,\x1b[0;32m\"x\"\x1b[0m\x1b[1;39m,\x1b[0;90mnull\x1b[0m\x1b[1;39m,\x1b[0;39mtrue\x1b[0m\x1b[1;39m,\x1b[0;39mfalse\x1b[0m\x1b[1;39m\x1b[1;39m]\x1b[0m\x1b[1;39m,\x1b[0m\x1b[1;34m\"b\"\x1b[0m\x1b[1;39m:\x1b[0m\x1b[1;39m{}\x1b[0m\x1b[1;39m\x1b[1;39m}\x1b[0m\n"),
        ("[]", &["-C", "-c"], Some("0;31:0;32:0;33:0;34:0;35:0;36:0;37:1;31"), "\x1b[0;36m[]\x1b[0m\n"),
        ("{}", &["-C", "-c"], Some("0;31:0;32:0;33:0;34:0;35:0;36:0;37:1;31"), "\x1b[0;37m{}\x1b[0m\n"),
        ("[1]", &["-C", "-c"], Some("0;31:0;32:0;33:0;34:0;35:0;36:0;37:1;31"), "\x1b[0;36m[\x1b[0;34m1\x1b[0m\x1b[0;36m\x1b[0;36m]\x1b[0m\n"),
        ("{\"a\":1}", &["-C", "-c"], Some("0;31:0;32:0;33:0;34:0;35:0;36:0;37:1;31"), "\x1b[0;37m{\x1b[0m\x1b[1;31m\"a\"\x1b[0m\x1b[0;37m:\x1b[0m\x1b[0;34m1\x1b[0m\x1b[0;37m\x1b[0;37m}\x1b[0m\n"),
        ("{\"a\":[1,\"x\",null,true]}", &["-C", "-c"], Some("0;31:0;32:0;33:0;34:0;35:0;36:0;37:1;31"), "\x1b[0;37m{\x1b[0m\x1b[1;31m\"a\"\x1b[0m\x1b[0;37m:\x1b[0m\x1b[0;36m[\x1b[0;34m1\x1b[0m\x1b[0;36m,\x1b[0;35m\"x\"\x1b[0m\x1b[0;36m,\x1b[0;31mnull\x1b[0m\x1b[0;36m,\x1b[0;33mtrue\x1b[0m\x1b[0;36m\x1b[0;36m]\x1b[0m\x1b[0;37m\x1b[0;37m}\x1b[0m\n"),
        ("{\"a\":[1,\"x\",null,true]}", &["-C"], Some("0;31:0;32:0;33:0;34:0;35:0;36:0;37:1;31"), "\x1b[0;37m{\n  \x1b[0m\x1b[1;31m\"a\"\x1b[0m\x1b[0;37m: \x1b[0m\x1b[0;36m[\n    \x1b[0;34m1\x1b[0m\x1b[0;36m,\n    \x1b[0;35m\"x\"\x1b[0m\x1b[0;36m,\n    \x1b[0;31mnull\x1b[0m\x1b[0;36m,\n    \x1b[0;33mtrue\x1b[0m\x1b[0;36m\n  \x1b[0;36m]\x1b[0m\x1b[0;37m\n\x1b[0;37m}\x1b[0m\n"),
        ("[[1,2],[3]]", &["-C", "-c"], Some("0;31:0;32:0;33:0;34:0;35:0;36:0;37:1;31"), "\x1b[0;36m[\x1b[0;36m[\x1b[0;34m1\x1b[0m\x1b[0;36m,\x1b[0;34m2\x1b[0m\x1b[0;36m\x1b[0;36m]\x1b[0m\x1b[0;36m,\x1b[0;36m[\x1b[0;34m3\x1b[0m\x1b[0;36m\x1b[0;36m]\x1b[0m\x1b[0;36m\x1b[0;36m]\x1b[0m\n"),
        ("[[],{}]", &["-C", "-c"], Some("0;31:0;32:0;33:0;34:0;35:0;36:0;37:1;31"), "\x1b[0;36m[\x1b[0;36m[]\x1b[0m\x1b[0;36m,\x1b[0;37m{}\x1b[0m\x1b[0;36m\x1b[0;36m]\x1b[0m\n"),
        ("[[],{}]", &["-C"], Some("0;31:0;32:0;33:0;34:0;35:0;36:0;37:1;31"), "\x1b[0;36m[\n  \x1b[0;36m[]\x1b[0m\x1b[0;36m,\n  \x1b[0;37m{}\x1b[0m\x1b[0;36m\n\x1b[0;36m]\x1b[0m\n"),
        ("{\"a\":{\"b\":{\"c\":[]}}}", &["-C", "-c"], Some("0;31:0;32:0;33:0;34:0;35:0;36:0;37:1;31"), "\x1b[0;37m{\x1b[0m\x1b[1;31m\"a\"\x1b[0m\x1b[0;37m:\x1b[0m\x1b[0;37m{\x1b[0m\x1b[1;31m\"b\"\x1b[0m\x1b[0;37m:\x1b[0m\x1b[0;37m{\x1b[0m\x1b[1;31m\"c\"\x1b[0m\x1b[0;37m:\x1b[0m\x1b[0;36m[]\x1b[0m\x1b[0;37m\x1b[0;37m}\x1b[0m\x1b[0;37m\x1b[0;37m}\x1b[0m\x1b[0;37m\x1b[0;37m}\x1b[0m\n"),
        ("{\"a\":1,\"b\":\"x\",\"c\":null,\"d\":true,\"e\":[1,2],\"f\":{\"g\":1}}", &["-C", "-c"], Some("0;31:0;32:0;33:0;34:0;35:0;36:0;37:1;31"), "\x1b[0;37m{\x1b[0m\x1b[1;31m\"a\"\x1b[0m\x1b[0;37m:\x1b[0m\x1b[0;34m1\x1b[0m\x1b[0;37m,\x1b[0m\x1b[1;31m\"b\"\x1b[0m\x1b[0;37m:\x1b[0m\x1b[0;35m\"x\"\x1b[0m\x1b[0;37m,\x1b[0m\x1b[1;31m\"c\"\x1b[0m\x1b[0;37m:\x1b[0m\x1b[0;31mnull\x1b[0m\x1b[0;37m,\x1b[0m\x1b[1;31m\"d\"\x1b[0m\x1b[0;37m:\x1b[0m\x1b[0;33mtrue\x1b[0m\x1b[0;37m,\x1b[0m\x1b[1;31m\"e\"\x1b[0m\x1b[0;37m:\x1b[0m\x1b[0;36m[\x1b[0;34m1\x1b[0m\x1b[0;36m,\x1b[0;34m2\x1b[0m\x1b[0;36m\x1b[0;36m]\x1b[0m\x1b[0;37m,\x1b[0m\x1b[1;31m\"f\"\x1b[0m\x1b[0;37m:\x1b[0m\x1b[0;37m{\x1b[0m\x1b[1;31m\"g\"\x1b[0m\x1b[0;37m:\x1b[0m\x1b[0;34m1\x1b[0m\x1b[0;37m\x1b[0;37m}\x1b[0m\x1b[0;37m\x1b[0;37m}\x1b[0m\n"),
        ("{\"b\":1,\"a\":[2,{\"d\":1,\"c\":2}]}", &["-C", "-S"], Some("0;31:0;32:0;33:0;34:0;35:0;36:0;37:1;31"), "\x1b[0;37m{\n  \x1b[0m\x1b[1;31m\"a\"\x1b[0m\x1b[0;37m: \x1b[0m\x1b[0;36m[\n    \x1b[0;34m2\x1b[0m\x1b[0;36m,\n    \x1b[0;37m{\n      \x1b[0m\x1b[1;31m\"c\"\x1b[0m\x1b[0;37m: \x1b[0m\x1b[0;34m2\x1b[0m\x1b[0;37m,\n      \x1b[0m\x1b[1;31m\"d\"\x1b[0m\x1b[0;37m: \x1b[0m\x1b[0;34m1\x1b[0m\x1b[0;37m\n    \x1b[0;37m}\x1b[0m\x1b[0;36m\n  \x1b[0;36m]\x1b[0m\x1b[0;37m,\n  \x1b[0m\x1b[1;31m\"b\"\x1b[0m\x1b[0;37m: \x1b[0m\x1b[0;34m1\x1b[0m\x1b[0;37m\n\x1b[0;37m}\x1b[0m\n"),
        ("[\"\\u00e9\",{\"\\u00e9\":\"\\u00e9\"}]", &["-C", "-c", "-a"], Some("0;31:0;32:0;33:0;34:0;35:0;36:0;37:1;31"), "\x1b[0;36m[\x1b[0;35m\"\\u00e9\"\x1b[0m\x1b[0;36m,\x1b[0;37m{\x1b[0m\x1b[1;31m\"\\u00e9\"\x1b[0m\x1b[0;37m:\x1b[0m\x1b[0;35m\"\\u00e9\"\x1b[0m\x1b[0;37m\x1b[0;37m}\x1b[0m\x1b[0;36m\x1b[0;36m]\x1b[0m\n"),
        ("{\"a\":[1,\"x\",null,true,false],\"b\":{}}", &["-C", "-c"], Some("0;31:0;32:0;33:0;34:0;35:0;36:0;37:1;31"), "\x1b[0;37m{\x1b[0m\x1b[1;31m\"a\"\x1b[0m\x1b[0;37m:\x1b[0m\x1b[0;36m[\x1b[0;34m1\x1b[0m\x1b[0;36m,\x1b[0;35m\"x\"\x1b[0m\x1b[0;36m,\x1b[0;31mnull\x1b[0m\x1b[0;36m,\x1b[0;33mtrue\x1b[0m\x1b[0;36m,\x1b[0;32mfalse\x1b[0m\x1b[0;36m\x1b[0;36m]\x1b[0m\x1b[0;37m,\x1b[0m\x1b[1;31m\"b\"\x1b[0m\x1b[0;37m:\x1b[0m\x1b[0;37m{}\x1b[0m\x1b[0;37m\x1b[0;37m}\x1b[0m\n"),
    ];
    for &(input, args, colors, want) in rows {
        let mut argv = args.to_vec();
        argv.push(".");
        // The rows with no spec expect the default scheme, so pass jq 1.7.1's own default spec
        // rather than rely on `JQ_COLORS` being unset in the environment that runs the tests.
        let spec = colors.unwrap_or(DEFAULT_SPEC);
        let (output, code) = spawn_jq_with_env(&argv, "JQ_COLORS", spec, Some(input.as_bytes()))?;
        assert_eq!(code, 0, "{args:?} {colors:?} over {input}");
        let got = String::from_utf8(output.stdout)?;
        assert_eq!(got, want, "{args:?} {colors:?} over {input}");
    }
    Ok(())
}
