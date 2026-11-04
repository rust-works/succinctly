//! #6122: a `|=` update with no variable, call or path read in it runs on the owned identity pipe
//! (one owned copy of the document, `eval_positioned_stage_generic`) instead of the wildcard bridge,
//! which wrote that copy out as text and indexed it again for the full evaluator (`.users |= .` and
//! `.users[0].name |= "x"` on a 100 MB document were 0.92x of jq 1.7.1 and are 1.06x). The route
//! must answer exactly as the bridge did, so each row is what `/usr/bin/jq` 1.7.1 prints for the
//! program over one document: its stdout and its exit status.

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

/// The same programs over a document the CLI reads from a file and a second one after it, so the
/// route is taken once per input.
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
