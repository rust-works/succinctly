//! #2802: a YAML integer written other than the way it prints (`0x1f`, `0o17`, `01`, `+5`, `007`)
//! keeps its spelling wherever yq echoes a scalar's text -- `tostring`, interpolation, `join`,
//! YAML output, the node a `sort`/`max`/`unique` returns, `==` -- while yq's JSON encoder writes
//! the number. Every row is a live capture from yq v4.53.3 (the YAML output and `-o=json -I0`
//! of the same filter), over one document.
//!
//! Not here, still different from yq: arithmetic on a hex or octal integer keeps its base
//! (`.a + 1` is `0x20` there, #3027), and `tonumber` of a spelled string (#1356).

use anyhow::Result;

use crate::run_yq_stdin;

const DOC: &str = "a: 0x1f\nb: 0o17\nc: 01\nd: +5\ne: 007\nf: 31\n";

/// (filter, YAML output, JSON output) captured from yq v4.53.3.
const ROWS: &[(&str, &str, &str)] = &[
    (".a | tostring", "0x1f\n", "\"0x1f\"\n"),
    (
        "[.a,.b,.c,.d,.e] | map(tostring)",
        "- \"0x1f\"\n- \"0o17\"\n- \"01\"\n- \"+5\"\n- \"007\"\n",
        "[\"0x1f\",\"0o17\",\"01\",\"+5\",\"007\"]\n",
    ),
    (
        "\"\\(.a) \\(.c) \\(.d)\"",
        "0x1f 01 +5\n",
        "\"0x1f 01 +5\"\n",
    ),
    ("[.a] | .[0] | tostring", "0x1f\n", "\"0x1f\"\n"),
    ("[.a,.c] | join(\",\")", "0x1f,01\n", "\"0x1f,01\"\n"),
    ("[.a,.f] | sort", "- 0x1f\n- 31\n", "[31,31]\n"),
    ("[.a,.f] | max", "0x1f\n", "31\n"),
    (".a == 31", "false\n", "false\n"),
    (".a == .f", "false\n", "false\n"),
    (".a | @yaml", "0x1f\n\n", "\"0x1f\\n\"\n"),
    ("{\"x\": .a}", "x: 0x1f\n", "{\"x\":31}\n"),
    (
        ".z = .a",
        "a: 0x1f\nb: 0o17\nc: 01\nd: +5\ne: 007\nf: 31\nz: 0x1f\n",
        "{\"a\":31,\"b\":15,\"c\":1,\"d\":5,\"e\":7,\"f\":31,\"z\":31}\n",
    ),
    (
        ".z = (.a | tostring)",
        "a: 0x1f\nb: 0o17\nc: 01\nd: +5\ne: 007\nf: 31\nz: \"0x1f\"\n",
        "{\"a\":31,\"b\":15,\"c\":1,\"d\":5,\"e\":7,\"f\":31,\"z\":\"0x1f\"}\n",
    ),
    (
        "with_entries(.)",
        "a: 0x1f\nb: 0o17\nc: 01\nd: +5\ne: 007\nf: 31\n",
        "{\"a\":31,\"b\":15,\"c\":1,\"d\":5,\"e\":7,\"f\":31}\n",
    ),
    (
        "to_entries | map(.value | tostring)",
        "- \"0x1f\"\n- \"0o17\"\n- \"01\"\n- \"+5\"\n- \"007\"\n- \"31\"\n",
        "[\"0x1f\",\"0o17\",\"01\",\"+5\",\"007\",\"31\"]\n",
    ),
    ("[.a, .c] | unique", "- 0x1f\n- 01\n", "[31,1]\n"),
    ("[.a] | .[0] | tag", "!!int\n", "\"!!int\"\n"),
    ("[.a] | .[0] | length", "4\n", "4\n"),
    (
        "[.a, .b] | map(. == 31)",
        "- false\n- false\n",
        "[false,false]\n",
    ),
    (
        ".[] |= tostring",
        "a: \"0x1f\"\nb: \"0o17\"\nc: \"01\"\nd: \"+5\"\ne: \"007\"\nf: \"31\"\n",
        "{\"a\":\"0x1f\",\"b\":\"0o17\",\"c\":\"01\",\"d\":\"+5\",\"e\":\"007\",\"f\":\"31\"}\n",
    ),
    (
        ".a tag = \"!!str\"",
        "a: \"0x1f\"\nb: 0o17\nc: 01\nd: +5\ne: 007\nf: 31\n",
        "{\"a\":\"0x1f\",\"b\":15,\"c\":1,\"d\":5,\"e\":7,\"f\":31}\n",
    ),
    (
        ".a style = \"double\"",
        "a: \"0x1f\"\nb: 0o17\nc: 01\nd: +5\ne: 007\nf: 31\n",
        "{\"a\":31,\"b\":15,\"c\":1,\"d\":5,\"e\":7,\"f\":31}\n",
    ),
    (
        ".a style = \"tagged\"",
        "a: !!int 0x1f\nb: 0o17\nc: 01\nd: +5\ne: 007\nf: 31\n",
        "{\"a\":31,\"b\":15,\"c\":1,\"d\":5,\"e\":7,\"f\":31}\n",
    ),
    (
        "to_entries | from_entries",
        "a: 0x1f\nb: 0o17\nc: 01\nd: +5\ne: 007\nf: 31\n",
        "{\"a\":31,\"b\":15,\"c\":1,\"d\":5,\"e\":7,\"f\":31}\n",
    ),
    (
        "[.a, .c] | reverse | map(tostring)",
        "- \"01\"\n- \"0x1f\"\n",
        "[\"01\",\"0x1f\"]\n",
    ),
    (
        ".e |= tostring",
        "a: 0x1f\nb: 0o17\nc: 01\nd: +5\ne: \"007\"\nf: 31\n",
        "{\"a\":31,\"b\":15,\"c\":1,\"d\":5,\"e\":\"007\",\"f\":31}\n",
    ),
];

#[test]
fn test_spelled_integers_keep_their_text_where_yq_echoes_it_2802() -> Result<()> {
    for (filter, yaml, json) in ROWS {
        let (out, code) = run_yq_stdin(filter, DOC, &[])?;
        assert_eq!((out.as_str(), code), (*yaml, 0), "{filter}");
        let (out, code) = run_yq_stdin(filter, DOC, &["-o=json", "-I0"])?;
        assert_eq!((out.as_str(), code), (*json, 0), "{filter} -o=json");
    }
    Ok(())
}
