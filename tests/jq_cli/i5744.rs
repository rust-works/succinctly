//! #5744: `--stream -s` hands the array it slurps the events into to the filter as that tree, where
//! it used to write it out as JSON text, index it and read it back (31% of the instructions of
//! `--stream -s 'length'`). The answers below were captured from jq 1.7.1 (`/usr/bin/jq`), and pin
//! that the owned route reads the events the way the text round trip did: number spellings, empty
//! and scalar documents, multi-document streams, and the filters whose stages the owned evaluator
//! declines and re-indexes.
//! Pin moved (#7523): a `--stream -s` row whose closing event jq 1.7.1 gave out after the array is jq
//! 1.8.2's answer, with the event in the array.

use anyhow::Result;

use crate::run_jq_full;

const CASES: &[(&str, &str, &str, i32)] = &[
    (r#"{"a":[1,{"b":2.50}],"c":"x"}"#, r"length", "6\n", 0),
    (r#"{"a":[1,{"b":2.50}],"c":"x"}"#, r"map(length)|add", "9\n", 0),
    (r#"{"a":[1,{"b":2.50}],"c":"x"}"#, r"[.[]|select(length==2)]|length", "3\n", 0),
    (r#"{"a":[1,{"b":2.50}],"c":"x"}"#, r"fromstream(.[])", "{\"a\":[1,{\"b\":2.50}],\"c\":\"x\"}\n", 0),
    (r#"{"a":[1,{"b":2.50}],"c":"x"}"#, r".[0:3]", "[[[\"a\",0],1],[[\"a\",1,\"b\"],2.50],[[\"a\",1,\"b\"]]]\n", 0),
    (r#"{"a":[1,{"b":2.50}],"c":"x"}"#, r"first", "[[\"a\",0],1]\n", 0),
    (r#"{"a":[1,{"b":2.50}],"c":"x"}"#, r"last", "[[\"c\"]]\n", 0),
    (r#"{"a":[1,{"b":2.50}],"c":"x"}"#, r"[limit(2;.[])]", "[[[\"a\",0],1],[[\"a\",1,\"b\"],2.50]]\n", 0),
    (r#"{"a":[1,{"b":2.50}],"c":"x"}"#, r"tojson", "\"[[[\\\"a\\\",0],1],[[\\\"a\\\",1,\\\"b\\\"],2.50],[[\\\"a\\\",1,\\\"b\\\"]],[[\\\"a\\\",1]],[[\\\"c\\\"],\\\"x\\\"],[[\\\"c\\\"]]]\"\n", 0),
    (r#"{"a":[1,{"b":2.50}],"c":"x"}"#, r".[]|select(length==2)", "[[\"a\",0],1]\n[[\"a\",1,\"b\"],2.50]\n[[\"c\"],\"x\"]\n", 0),
    (r#"[1,[2,3]] {"k":null} 7 "s" []"#, r"length", "10\n", 0),
    (r#"[1,[2,3]] {"k":null} 7 "s" []"#, r"fromstream(.[])", "[1,[2,3]]\n{\"k\":null}\n7\n\"s\"\n[]\n", 0),
    (r#"[1,[2,3]] {"k":null} 7 "s" []"#, r"map(.[0]|tojson)", "[\"[0]\",\"[1,0]\",\"[1,1]\",\"[1,1]\",\"[1]\",\"[\\\"k\\\"]\",\"[\\\"k\\\"]\",\"[]\",\"[]\",\"[]\"]\n", 0),
    (r"[]", r".", "[[[],[]]]\n", 0),
    (r"1e1000 100000000000000000000 0.10", r"map(.[1])", "[1E+1000,100000000000000000000,0.10]\n", 0),
    (r"", r".", "[]\n", 0),
    (r"", r"length", "0\n", 0),
];

#[test]
fn stream_slurp_answers_as_jq_does_5744() -> Result<()> {
    for (input, program, expected, code) in CASES {
        let (stdout, stderr, status) =
            run_jq_full(&["--stream", "-c", "-s", program], Some(input))?;
        assert_eq!(
            (stdout.as_str(), status),
            (*expected, *code),
            "{program} over {input}: {stderr}"
        );
    }
    Ok(())
}
