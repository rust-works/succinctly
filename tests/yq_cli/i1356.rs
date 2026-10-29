//! #1356 (integer spellings): `tonumber` of a string yq reads with `parseInt64` keeps the text as
//! the node's value, so `"0x10" | tonumber` prints `0x10`, `"007"` prints `007` and `"1_000"`
//! prints `1_000` through `tostring`, interpolation and YAML output, while `-o=json` prints the
//! number and arithmetic keeps the left operand's base (#3027). A float spelling (`"+2.0"`,
//! `"2."`, `".5"`) is not here: it needs the float half of the literal-versus-JSON-text split.
//! Every row is a live capture from yq v4.53.3 (YAML output and `-o=json -I0`), over one document.

use anyhow::Result;

use crate::{run_yq_stdin, run_yq_stdin_with_stderr};

const DOC: &str = "a: \"+2.0\"\nb: \"007\"\nc: \"2.\"\nd: \".5\"\ne: \"0x10\"\nf: \"0o17\"\ng: \"1_000\"\nh: \"+5\"\ni: \"0X10\"\nj: \"-0x10\"\nk: \"1e3\"\nl: \"12\"\nm: \"-007\"\nn: \"00\"\no: \"-0\"\np: \"+0\"\nq: \"1__0\"\nr: \"_1\"\ns: \"1_\"\nt: \"0x\"\nu: \"0x_1\"\nv: \" 0x10\"\nw: \"0x-10\"\nx: \"0o9\"\ny: \"9223372036854775807\"\nz: \"0x7fffffffffffffff\"\n";

/// (filter, YAML output, JSON output) captured from yq v4.53.3.
const ROWS: &[(&str, &str, &str)] = &[
    (".b | tonumber", "007\n", "7\n"),
    (".b | tonumber | tostring", "007\n", "\"007\"\n"),
    (".b | tonumber | . + 1", "8\n", "8\n"),
    (".b | tonumber | . * 2", "14\n", "14\n"),
    ("[.b | tonumber] | .[0] | tostring", "007\n", "\"007\"\n"),
    ("{\"z2\": (.b | tonumber)}", "z2: 007\n", "{\"z2\":7}\n"),
    ("\"\\(.b | tonumber)\"", "007\n", "\"007\"\n"),
    ("[.b | tonumber, 1] | sort | .[0]", "1\n", "1\n"),
    (".e | tonumber", "0x10\n", "16\n"),
    (".e | tonumber | tostring", "0x10\n", "\"0x10\"\n"),
    (".e | tonumber | . + 1", "0x11\n", "17\n"),
    (".e | tonumber | . * 2", "0x20\n", "32\n"),
    ("[.e | tonumber] | .[0] | tostring", "0x10\n", "\"0x10\"\n"),
    ("{\"z2\": (.e | tonumber)}", "z2: 0x10\n", "{\"z2\":16}\n"),
    ("\"\\(.e | tonumber)\"", "0x10\n", "\"0x10\"\n"),
    ("[.e | tonumber, 1] | sort | .[0]", "1\n", "1\n"),
    (".h | tonumber", "+5\n", "5\n"),
    (".h | tonumber | tostring", "+5\n", "\"+5\"\n"),
    (".h | tonumber | . + 1", "6\n", "6\n"),
    (".h | tonumber | . * 2", "10\n", "10\n"),
    ("[.h | tonumber] | .[0] | tostring", "+5\n", "\"+5\"\n"),
    ("{\"z2\": (.h | tonumber)}", "z2: +5\n", "{\"z2\":5}\n"),
    ("\"\\(.h | tonumber)\"", "+5\n", "\"+5\"\n"),
    ("[.h | tonumber, 1] | sort | .[0]", "1\n", "1\n"),
    (".f | tonumber", "0o17\n", "15\n"),
    (".f | tonumber | tostring", "0o17\n", "\"0o17\"\n"),
    (".f | tonumber | . + 1", "0o20\n", "16\n"),
    (".f | tonumber | . * 2", "0o36\n", "30\n"),
    ("[.f | tonumber] | .[0] | tostring", "0o17\n", "\"0o17\"\n"),
    ("{\"z2\": (.f | tonumber)}", "z2: 0o17\n", "{\"z2\":15}\n"),
    ("\"\\(.f | tonumber)\"", "0o17\n", "\"0o17\"\n"),
    ("[.f | tonumber, 1] | sort | .[0]", "1\n", "1\n"),
    (".g | tonumber", "1_000\n", "1000\n"),
    (".g | tonumber | tostring", "1_000\n", "\"1_000\"\n"),
    (".g | tonumber | . + 1", "1001\n", "1001\n"),
    (".g | tonumber | . * 2", "2000\n", "2000\n"),
    (
        "[.g | tonumber] | .[0] | tostring",
        "1_000\n",
        "\"1_000\"\n",
    ),
    (
        "{\"z2\": (.g | tonumber)}",
        "z2: 1_000\n",
        "{\"z2\":1000}\n",
    ),
    ("\"\\(.g | tonumber)\"", "1_000\n", "\"1_000\"\n"),
    ("[.g | tonumber, 1] | sort | .[0]", "1\n", "1\n"),
    (".i | tonumber", "0X10\n", "16\n"),
    (".i | tonumber | tostring", "0X10\n", "\"0X10\"\n"),
    (".i | tonumber | . + 1", "0x11\n", "17\n"),
    (".i | tonumber | . * 2", "0x20\n", "32\n"),
    ("[.i | tonumber] | .[0] | tostring", "0X10\n", "\"0X10\"\n"),
    ("{\"z2\": (.i | tonumber)}", "z2: 0X10\n", "{\"z2\":16}\n"),
    ("\"\\(.i | tonumber)\"", "0X10\n", "\"0X10\"\n"),
    ("[.i | tonumber, 1] | sort | .[0]", "1\n", "1\n"),
    (".l | tonumber", "12\n", "12\n"),
    (".l | tonumber | tostring", "12\n", "\"12\"\n"),
    (".l | tonumber | . + 1", "13\n", "13\n"),
    (".l | tonumber | . * 2", "24\n", "24\n"),
    ("[.l | tonumber] | .[0] | tostring", "12\n", "\"12\"\n"),
    ("{\"z2\": (.l | tonumber)}", "z2: 12\n", "{\"z2\":12}\n"),
    ("\"\\(.l | tonumber)\"", "12\n", "\"12\"\n"),
    ("[.l | tonumber, 1] | sort | .[0]", "1\n", "1\n"),
    (".m | tonumber", "-007\n", "-7\n"),
    (".m | tonumber | tostring", "-007\n", "\"-007\"\n"),
    (".m | tonumber | . + 1", "-6\n", "-6\n"),
    (".m | tonumber | . * 2", "-14\n", "-14\n"),
    ("[.m | tonumber] | .[0] | tostring", "-007\n", "\"-007\"\n"),
    ("{\"z2\": (.m | tonumber)}", "z2: -007\n", "{\"z2\":-7}\n"),
    ("\"\\(.m | tonumber)\"", "-007\n", "\"-007\"\n"),
    ("[.m | tonumber, 1] | sort | .[0]", "-007\n", "-7\n"),
    (".n | tonumber", "00\n", "0\n"),
    (".n | tonumber | tostring", "00\n", "\"00\"\n"),
    (".n | tonumber | . + 1", "1\n", "1\n"),
    (".n | tonumber | . * 2", "0\n", "0\n"),
    ("[.n | tonumber] | .[0] | tostring", "00\n", "\"00\"\n"),
    ("{\"z2\": (.n | tonumber)}", "z2: 00\n", "{\"z2\":0}\n"),
    ("\"\\(.n | tonumber)\"", "00\n", "\"00\"\n"),
    ("[.n | tonumber, 1] | sort | .[0]", "00\n", "0\n"),
    (".p | tonumber", "+0\n", "0\n"),
    (".p | tonumber | tostring", "+0\n", "\"+0\"\n"),
    (".p | tonumber | . + 1", "1\n", "1\n"),
    (".p | tonumber | . * 2", "0\n", "0\n"),
    ("[.p | tonumber] | .[0] | tostring", "+0\n", "\"+0\"\n"),
    ("{\"z2\": (.p | tonumber)}", "z2: +0\n", "{\"z2\":0}\n"),
    ("\"\\(.p | tonumber)\"", "+0\n", "\"+0\"\n"),
    ("[.p | tonumber, 1] | sort | .[0]", "+0\n", "0\n"),
    (".q | tonumber", "1__0\n", "10\n"),
    (".q | tonumber | tostring", "1__0\n", "\"1__0\"\n"),
    (".q | tonumber | . + 1", "11\n", "11\n"),
    (".q | tonumber | . * 2", "20\n", "20\n"),
    ("[.q | tonumber] | .[0] | tostring", "1__0\n", "\"1__0\"\n"),
    ("{\"z2\": (.q | tonumber)}", "z2: 1__0\n", "{\"z2\":10}\n"),
    ("\"\\(.q | tonumber)\"", "1__0\n", "\"1__0\"\n"),
    ("[.q | tonumber, 1] | sort | .[0]", "1\n", "1\n"),
    (".r | tonumber", "_1\n", "1\n"),
    (".r | tonumber | tostring", "_1\n", "\"_1\"\n"),
    (".r | tonumber | . + 1", "2\n", "2\n"),
    (".r | tonumber | . * 2", "2\n", "2\n"),
    ("[.r | tonumber] | .[0] | tostring", "_1\n", "\"_1\"\n"),
    (
        "{\"z2\": (.r | tonumber)}",
        "z2: !!int _1\n",
        "{\"z2\":1}\n",
    ),
    ("\"\\(.r | tonumber)\"", "_1\n", "\"_1\"\n"),
    ("[.r | tonumber, 1] | sort | .[0]", "_1\n", "1\n"),
    (".s | tonumber", "1_\n", "1\n"),
    (".s | tonumber | tostring", "1_\n", "\"1_\"\n"),
    (".s | tonumber | . + 1", "2\n", "2\n"),
    (".s | tonumber | . * 2", "2\n", "2\n"),
    ("[.s | tonumber] | .[0] | tostring", "1_\n", "\"1_\"\n"),
    ("{\"z2\": (.s | tonumber)}", "z2: 1_\n", "{\"z2\":1}\n"),
    ("\"\\(.s | tonumber)\"", "1_\n", "\"1_\"\n"),
    ("[.s | tonumber, 1] | sort | .[0]", "1_\n", "1\n"),
    (".u | tonumber", "0x_1\n", "1\n"),
    (".u | tonumber | tostring", "0x_1\n", "\"0x_1\"\n"),
    (".u | tonumber | . + 1", "0x2\n", "2\n"),
    (".u | tonumber | . * 2", "0x2\n", "2\n"),
    ("[.u | tonumber] | .[0] | tostring", "0x_1\n", "\"0x_1\"\n"),
    ("{\"z2\": (.u | tonumber)}", "z2: 0x_1\n", "{\"z2\":1}\n"),
    ("\"\\(.u | tonumber)\"", "0x_1\n", "\"0x_1\"\n"),
    ("[.u | tonumber, 1] | sort | .[0]", "0x_1\n", "1\n"),
    (".w | tonumber", "0x-10\n", "-16\n"),
    (".w | tonumber | tostring", "0x-10\n", "\"0x-10\"\n"),
    (".w | tonumber | . + 1", "0x-F\n", "-15\n"),
    (".w | tonumber | . * 2", "0x-20\n", "-32\n"),
    (
        "[.w | tonumber] | .[0] | tostring",
        "0x-10\n",
        "\"0x-10\"\n",
    ),
    (
        "{\"z2\": (.w | tonumber)}",
        "z2: !!int 0x-10\n",
        "{\"z2\":-16}\n",
    ),
    ("\"\\(.w | tonumber)\"", "0x-10\n", "\"0x-10\"\n"),
    ("[.w | tonumber, 1] | sort | .[0]", "0x-10\n", "-16\n"),
    (
        ".y | tonumber",
        "9223372036854775807\n",
        "9223372036854775807\n",
    ),
    (
        ".y | tonumber | tostring",
        "9223372036854775807\n",
        "\"9223372036854775807\"\n",
    ),
    (
        ".y | tonumber | . + 1",
        "-9223372036854775808\n",
        "-9223372036854775808\n",
    ),
    (".y | tonumber | . * 2", "-2\n", "-2\n"),
    (
        "[.y | tonumber] | .[0] | tostring",
        "9223372036854775807\n",
        "\"9223372036854775807\"\n",
    ),
    (
        "{\"z2\": (.y | tonumber)}",
        "z2: 9223372036854775807\n",
        "{\"z2\":9223372036854775807}\n",
    ),
    (
        "\"\\(.y | tonumber)\"",
        "9223372036854775807\n",
        "\"9223372036854775807\"\n",
    ),
    ("[.y | tonumber, 1] | sort | .[0]", "1\n", "1\n"),
    (
        ".z | tonumber",
        "0x7fffffffffffffff\n",
        "9223372036854775807\n",
    ),
    (
        ".z | tonumber | tostring",
        "0x7fffffffffffffff\n",
        "\"0x7fffffffffffffff\"\n",
    ),
    (
        ".z | tonumber | . + 1",
        "0x-8000000000000000\n",
        "-9223372036854775808\n",
    ),
    (".z | tonumber | . * 2", "0x-2\n", "-2\n"),
    (
        "[.z | tonumber] | .[0] | tostring",
        "0x7fffffffffffffff\n",
        "\"0x7fffffffffffffff\"\n",
    ),
    (
        "{\"z2\": (.z | tonumber)}",
        "z2: 0x7fffffffffffffff\n",
        "{\"z2\":9223372036854775807}\n",
    ),
    (
        "\"\\(.z | tonumber)\"",
        "0x7fffffffffffffff\n",
        "\"0x7fffffffffffffff\"\n",
    ),
    ("[.z | tonumber, 1] | sort | .[0]", "1\n", "1\n"),
];

/// (filter, the error yq prints) for a string `parseInt64` and `ParseFloat` both refuse.
const ERRORS: &[(&str, &str)] = &[
    (
        ".j | tonumber",
        "cannot convert node value [-0x10] at path j of tag !!str to number",
    ),
    (
        ".t | tonumber",
        "cannot convert node value [0x] at path t of tag !!str to number",
    ),
    (
        ".v | tonumber",
        "cannot convert node value [ 0x10] at path v of tag !!str to number",
    ),
    (
        ".x | tonumber",
        "cannot convert node value [0o9] at path x of tag !!str to number",
    ),
];

#[test]
fn test_tonumber_keeps_the_spelling_of_an_integer_string_1356() -> Result<()> {
    for (filter, yaml, json) in ROWS {
        let (out, code) = run_yq_stdin(filter, DOC, &[])?;
        assert_eq!((out.as_str(), code), (*yaml, 0), "{filter}");
        let (out, code) = run_yq_stdin(filter, DOC, &["-o=json", "-I0"])?;
        assert_eq!((out.as_str(), code), (*json, 0), "{filter} -o=json");
    }
    Ok(())
}

#[test]
fn test_tonumber_refuses_what_parseint64_and_parsefloat_refuse_1356() -> Result<()> {
    for (filter, _message) in ERRORS {
        let (stdout, _stderr, code) = run_yq_stdin_with_stderr(filter, DOC, &[])?;
        assert_eq!((stdout.as_str(), code), ("", 1), "{filter}");
    }
    Ok(())
}

/// (text, `[.a | tonumber]` as yq prints it) for `a: "<text>"`: a nested integer text go-yaml would
/// not resolve as an integer (`09`, `0x-1`, an unsigned `_1`) carries `!!int`, any other is bare.
const NESTED_ROWS: &[(&str, &str)] = &[
    ("_1", "- !!int _1\n"),
    ("__1", "- !!int __1\n"),
    ("_1_0", "- !!int _1_0\n"),
    ("1_000", "- 1_000\n"),
    ("1__0", "- 1__0\n"),
    ("1_", "- 1_\n"),
    ("0x1_0", "- 0x1_0\n"),
    ("0o_7", "- 0o_7\n"),
    ("0x_1", "- 0x_1\n"),
    ("0X10", "- 0X10\n"),
    ("0x1F", "- 0x1F\n"),
    ("0o17", "- 0o17\n"),
    ("09", "- !!int 09\n"),
    ("008", "- !!int 008\n"),
    ("-09", "- !!int -09\n"),
    ("+09", "- !!int +09\n"),
    ("0_9", "- !!int 0_9\n"),
    ("07", "- 07\n"),
    ("0_7", "- 0_7\n"),
    ("007", "- 007\n"),
    ("-007", "- -007\n"),
    ("+007", "- +007\n"),
    ("00", "- 00\n"),
    ("0x+1", "- !!int 0x+1\n"),
    ("0x-1", "- !!int 0x-1\n"),
    ("0o-7", "- !!int 0o-7\n"),
    ("0o+7", "- !!int 0o+7\n"),
    ("-_1", "- -_1\n"),
    ("+_1", "- +_1\n"),
    ("-__1", "- -__1\n"),
    ("1_0", "- 1_0\n"),
    ("+1_0", "- +1_0\n"),
    ("-1_0", "- -1_0\n"),
    ("12", "- 12\n"),
    ("+12", "- +12\n"),
    ("-12", "- -12\n"),
    ("0", "- 0\n"),
    ("+0", "- +0\n"),
    ("0_0", "- 0_0\n"),
    ("00_7", "- 00_7\n"),
    ("0_07", "- 0_07\n"),
    ("0xff", "- 0xff\n"),
    ("0XFF", "- 0XFF\n"),
    ("0x0", "- 0x0\n"),
    ("0x00", "- 0x00\n"),
    ("0o0", "- 0o0\n"),
];

#[test]
fn test_a_nested_tonumber_integer_is_tagged_when_yq_would_read_a_string_1356() -> Result<()> {
    for (text, yaml) in NESTED_ROWS {
        let doc = format!("a: \"{text}\"\n");
        let (out, code) = run_yq_stdin("[.a | tonumber]", &doc, &[])?;
        assert_eq!((out.as_str(), code), (*yaml, 0), "{text}");
    }
    Ok(())
}
