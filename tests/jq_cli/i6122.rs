//! #6122: a `|=` update that reads no path context is bridged through `eval_each_owned`, as `=`,
//! `+=` and `//=` already were in jq mode, instead of through the wildcard that wrote the owned
//! copy of the document out as text and indexed it again for the full evaluator (a second
//! materialization of the whole document). The route must answer exactly as the wildcard did, so
//! each row is what `/usr/bin/jq` 1.7.1 prints for the program over one document: its stdout and
//! its exit status. The second test pins the jq-mode `path` extension after an earlier update
//! (`path` is not a jq 1.7.1 builtin, so those answers are the evaluator's own, unchanged by this
//! route); `a_path_free_update_is_not_bridged_6122` in `eval_generic.rs` pins the saved
//! materialization itself.

use anyhow::Result;

use crate::run_jq_full;

const DOC: &str = r#"{"a":1,"arr":[1,2,3,4],"o":{"p":3,"q":"s"},"s":"abca","n":null}"#;

/// `(program, stdout, exit status)`.
const ROWS: &[(&str, &str, i32)] = &[
    (".a |= 5", "{\"a\":5,\"arr\":[1,2,3,4],\"o\":{\"p\":3,\"q\":\"s\"},\"s\":\"abca\",\"n\":null}\n", 0),
    (".a |= empty", "{\"arr\":[1,2,3,4],\"o\":{\"p\":3,\"q\":\"s\"},\"s\":\"abca\",\"n\":null}\n", 0),
    (".a |= (1,2)", "{\"a\":1,\"arr\":[1,2,3,4],\"o\":{\"p\":3,\"q\":\"s\"},\"s\":\"abca\",\"n\":null}\n", 0),
    (r#".a |= (try error("x") catch 9)"#, "{\"a\":9,\"arr\":[1,2,3,4],\"o\":{\"p\":3,\"q\":\"s\"},\"s\":\"abca\",\"n\":null}\n", 0),
    (".a |= .+1", "{\"a\":2,\"arr\":[1,2,3,4],\"o\":{\"p\":3,\"q\":\"s\"},\"s\":\"abca\",\"n\":null}\n", 0),
    (".arr |= map(.*2)", "{\"a\":1,\"arr\":[2,4,6,8],\"o\":{\"p\":3,\"q\":\"s\"},\"s\":\"abca\",\"n\":null}\n", 0),
    (".arr |= map(select(. > 1))", "{\"a\":1,\"arr\":[2,3,4],\"o\":{\"p\":3,\"q\":\"s\"},\"s\":\"abca\",\"n\":null}\n", 0),
    (".arr |= sort_by(-.)", "{\"a\":1,\"arr\":[4,3,2,1],\"o\":{\"p\":3,\"q\":\"s\"},\"s\":\"abca\",\"n\":null}\n", 0),
    (".arr[1:3] |= map(.*10)", "{\"a\":1,\"arr\":[1,20,30,4],\"o\":{\"p\":3,\"q\":\"s\"},\"s\":\"abca\",\"n\":null}\n", 0),
    (".arr |= first", "{\"a\":1,\"arr\":1,\"o\":{\"p\":3,\"q\":\"s\"},\"s\":\"abca\",\"n\":null}\n", 0),
    (".arr |= (.[0], .[1])", "{\"a\":1,\"arr\":1,\"o\":{\"p\":3,\"q\":\"s\"},\"s\":\"abca\",\"n\":null}\n", 0),
    (".arr |= [limit(2; .[])]", "{\"a\":1,\"arr\":[1,2],\"o\":{\"p\":3,\"q\":\"s\"},\"s\":\"abca\",\"n\":null}\n", 0),
    (".arr |= add", "{\"a\":1,\"arr\":10,\"o\":{\"p\":3,\"q\":\"s\"},\"s\":\"abca\",\"n\":null}\n", 0),
    (".arr |= length", "{\"a\":1,\"arr\":4,\"o\":{\"p\":3,\"q\":\"s\"},\"s\":\"abca\",\"n\":null}\n", 0),
    (".o.p |= 7", "{\"a\":1,\"arr\":[1,2,3,4],\"o\":{\"p\":7,\"q\":\"s\"},\"s\":\"abca\",\"n\":null}\n", 0),
    (".o |= with_entries(.value |= tostring)", "{\"a\":1,\"arr\":[1,2,3,4],\"o\":{\"p\":\"3\",\"q\":\"s\"},\"s\":\"abca\",\"n\":null}\n", 0),
    (".o |= keys", "{\"a\":1,\"arr\":[1,2,3,4],\"o\":[\"p\",\"q\"],\"s\":\"abca\",\"n\":null}\n", 0),
    (".o |= del(.p)", "{\"a\":1,\"arr\":[1,2,3,4],\"o\":{\"q\":\"s\"},\"s\":\"abca\",\"n\":null}\n", 0),
    (".x.y.z |= 1", "{\"a\":1,\"arr\":[1,2,3,4],\"o\":{\"p\":3,\"q\":\"s\"},\"s\":\"abca\",\"n\":null,\"x\":{\"y\":{\"z\":1}}}\n", 0),
    (".n |= (. // 5)", "{\"a\":1,\"arr\":[1,2,3,4],\"o\":{\"p\":3,\"q\":\"s\"},\"s\":\"abca\",\"n\":5}\n", 0),
    ("(.a, .arr[0]) |= 100", "{\"a\":100,\"arr\":[100,2,3,4],\"o\":{\"p\":3,\"q\":\"s\"},\"s\":\"abca\",\"n\":null}\n", 0),
    (".arr |= (reduce .[] as $x (0; . + $x))", "{\"a\":1,\"arr\":10,\"o\":{\"p\":3,\"q\":\"s\"},\"s\":\"abca\",\"n\":null}\n", 0),
    (".arr |= (label $out | .[] | if . > 1 then ., break $out else empty end)", "{\"a\":1,\"arr\":2,\"o\":{\"p\":3,\"q\":\"s\"},\"s\":\"abca\",\"n\":null}\n", 0),
    (r#".s |= split("")"#, "{\"a\":1,\"arr\":[1,2,3,4],\"o\":{\"p\":3,\"q\":\"s\"},\"s\":[\"a\",\"b\",\"c\",\"a\"],\"n\":null}\n", 0),
    (".o |= map_values(.+1)", "", 5),
    (".arr[10] |= 1", "{\"a\":1,\"arr\":[1,2,3,4,null,null,null,null,null,null,1],\"o\":{\"p\":3,\"q\":\"s\"},\"s\":\"abca\",\"n\":null}\n", 0),
    (". |= .", "{\"a\":1,\"arr\":[1,2,3,4],\"o\":{\"p\":3,\"q\":\"s\"},\"s\":\"abca\",\"n\":null}\n", 0),
    (". |= 3", "3\n", 0),
    (". |= empty", "null\n", 0),
    (r#".a |= error("x")"#, "", 5),
    (".arr |= (.[] | tostring)", "{\"a\":1,\"arr\":\"1\",\"o\":{\"p\":3,\"q\":\"s\"},\"s\":\"abca\",\"n\":null}\n", 0),
];

#[test]
fn a_variable_free_update_answers_as_jq_does_6122() -> Result<()> {
    for (program, want, want_code) in ROWS {
        let (stdout, stderr, code) = run_jq_full(&["-c", program], Some(DOC))?;
        assert_eq!(
            (stdout.as_str(), code),
            (*want, *want_code),
            "{program}: {stderr}"
        );
    }
    Ok(())
}

/// An update before a position-reading `|=` leaves no position behind: the path the later one
/// reads is the document's, not the earlier update's input path prepended to it.
#[test]
fn an_earlier_update_leaves_no_path_prefix_6122() -> Result<()> {
    for (doc, program, want) in [
        (
            r#"{"a":{"b":1,"c":{"d":2}}}"#,
            ".a | (.b |= 5) | .c | (.d |= path)",
            "{\"d\":[\"a\",\"c\",\"d\"]}\n",
        ),
        (
            r#"{"arr":[{"x":0,"b":0}]}"#,
            ".arr[] | (.x |= 1) | (.b |= path)",
            "{\"x\":1,\"b\":[\"arr\",0,\"b\"]}\n",
        ),
        (
            r#"{"x":{"a":0},"b":1}"#,
            "(.x | (.a |= 1)) as $u | (.b |= path)",
            "{\"x\":{\"a\":0},\"b\":[\"b\"]}\n",
        ),
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-c", program], Some(doc))?;
        assert_eq!((stdout.as_str(), code), (want, 0), "{program}: {stderr}");
    }
    Ok(())
}

/// The same program over two documents in one input, so the route is taken once per input.
#[test]
fn the_update_is_taken_once_per_input_6122() -> Result<()> {
    let input = format!("{DOC}\n{DOC}\n");
    let (stdout, stderr, code) = run_jq_full(&["-c", ".arr |= map(.*2)"], Some(&input))?;
    assert_eq!(code, 0, "{stderr}");
    let line =
        "{\"a\":1,\"arr\":[2,4,6,8],\"o\":{\"p\":3,\"q\":\"s\"},\"s\":\"abca\",\"n\":null}\n";
    assert_eq!(stdout, format!("{line}{line}"));
    Ok(())
}
