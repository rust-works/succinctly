//! #6122: a `|=` update that reads no path context (variables, definitions, `reduce`, `label` and
//! `$__loc__` included) is bridged through `eval_each_owned`, as `=`,
//! `+=` and `//=` already were in jq mode, instead of through the wildcard that wrote the owned
//! copy of the document out as text and indexed it again for the full evaluator (a second
//! materialization of the whole document). The route must answer exactly as the wildcard did, so
//! each row is what `/usr/bin/jq` 1.7.1 prints for the program over one document: its stdout and
//! its exit status. The second test pins the jq-mode `path` extension after an earlier update
//! (`path` is not a jq 1.7.1 builtin, so those answers are the evaluator's own, unchanged by this
//! route); `a_path_free_update_is_not_bridged_6122` in `eval_generic.rs` pins the saved
//! materialization itself.

use anyhow::Result;

use crate::{run_jq_full, run_jq_stdin_streams};

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
    (r".a as $x | .arr |= map(select(. != $x))", "{\"a\":1,\"arr\":[2,3,4],\"o\":{\"p\":3,\"q\":\"s\"},\"s\":\"abca\",\"n\":null}\n", 0),
    (r". as $d | .a |= $d.arr[0]", "{\"a\":1,\"arr\":[1,2,3,4],\"o\":{\"p\":3,\"q\":\"s\"},\"s\":\"abca\",\"n\":null}\n", 0),
    (r".arr |= (. as $a | length)", "{\"a\":1,\"arr\":4,\"o\":{\"p\":3,\"q\":\"s\"},\"s\":\"abca\",\"n\":null}\n", 0),
    (r".a |= $__loc__.line", "{\"a\":1,\"arr\":[1,2,3,4],\"o\":{\"p\":3,\"q\":\"s\"},\"s\":\"abca\",\"n\":null}\n", 0),
    (r"first(.arr |= map(.+1))", "{\"a\":1,\"arr\":[2,3,4,5],\"o\":{\"p\":3,\"q\":\"s\"},\"s\":\"abca\",\"n\":null}\n", 0),
    (r"[limit(2; .a |= (.,.))] | length", "1\n", 0),
    (r"def f: .a |= .+1; f | f", "{\"a\":3,\"arr\":[1,2,3,4],\"o\":{\"p\":3,\"q\":\"s\"},\"s\":\"abca\",\"n\":null}\n", 0),
    (r".a as $x | (.arr[] | select(. == $x)) |= 10", "{\"a\":1,\"arr\":[10,2,3,4],\"o\":{\"p\":3,\"q\":\"s\"},\"s\":\"abca\",\"n\":null}\n", 0),
    (r"reduce range(3) as $i (.; .a |= .+$i)", "{\"a\":4,\"arr\":[1,2,3,4],\"o\":{\"p\":3,\"q\":\"s\"},\"s\":\"abca\",\"n\":null}\n", 0),
    (r"[.arr[] as $v | .a |= $v] | length", "4\n", 0),
    (r".arr |= map(. as $e | $e * $e)", "{\"a\":1,\"arr\":[1,4,9,16],\"o\":{\"p\":3,\"q\":\"s\"},\"s\":\"abca\",\"n\":null}\n", 0),
    (r#"try (.a |= error("x")) catch "caught""#, "\"caught\"\n", 0),
];

#[test]
fn a_path_free_update_answers_as_jq_does_6122() -> Result<()> {
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

// `map(.a)`, `map(.a.b)` and `map(.[n])` over an owned array are answered from the tree, not through
// the reindex bridge (`.users |= map(.age)` was 0.75x of jq 1.7.1 on a Ryzen 9 7950X and 0.93x on an Apple
// M4 Pro at 7 MB). The answer must be the one the bridge gave, and a step that raises must still raise the
// bridge's error. Every row was captured from jq 1.7.1 (`/usr/bin/jq -c`).

/// (input, filter, jq 1.7.1's `-c` output).
const ANSWERED: &[(&str, &str, &str)] = &[
    (
        r#"{"users":[{"age":1,"n":"a"},{"age":2,"n":"b"},{"n":"c"},null]}"#,
        r".users |= map(.age)",
        r#"{"users":[1,2,null,null]}
"#,
    ),
    (
        r#"{"users":[{"age":1,"n":"a"},{"age":2,"n":"b"},{"n":"c"},null]}"#,
        r".users |= map(.n)",
        r#"{"users":["a","b","c",null]}
"#,
    ),
    (
        r#"{"a":{"b":[{"c":1},{"c":[2,3]},{"c":{"d":4}}]},"z":9}"#,
        r".a.b |= map(.c)",
        r#"{"a":{"b":[1,[2,3],{"d":4}]},"z":9}
"#,
    ),
    (
        r#"{"a":{"b":[{"c":{"d":1}},{"c":{"d":[5]}},{"c":null},{}]}}"#,
        r".a.b |= map(.c.d)",
        r#"{"a":{"b":[1,[5],null,null]}}
"#,
    ),
    (
        r#"{"a":{"b":[{"c":{"d":1}},{"c":{"d":[5]}},{"c":null},{}]}}"#,
        r".a.b |= map(.c | .d)",
        r#"{"a":{"b":[1,[5],null,null]}}
"#,
    ),
    (
        r#"{"u":[[1,2,3],[4],[],null]}"#,
        r".u |= map(.[0])",
        r#"{"u":[1,4,null,null]}
"#,
    ),
    (
        r#"{"u":[[1,2,3],[4],[],null]}"#,
        r".u |= map(.[-1])",
        r#"{"u":[3,4,null,null]}
"#,
    ),
    (
        r#"{"u":[[1,2,3],[4],[],null]}"#,
        r".u |= map(.[-4])",
        r#"{"u":[null,null,null,null]}
"#,
    ),
    (
        r#"{"u":[[[1,2],[3]],[[4]],[[]]]}"#,
        r".u |= map(.[0][0])",
        r#"{"u":[1,4,null]}
"#,
    ),
    (
        r#"{"u":[{"a":100000000000000000000},{"a":1.50},{"a":-0},{"a":0.10}]}"#,
        r".u |= map(.a)",
        r#"{"u":[100000000000000000000,1.50,-0,0.10]}
"#,
    ),
    (
        r#"{"u":[{"a":1,"a":2},{"b":1,"b":null,"b":3}]}"#,
        r".u |= map(.a)",
        r#"{"u":[2,null]}
"#,
    ),
    (
        r#"{"u":[{"a":{"p":1}},{"a":{"p":2}}]}"#,
        r".u |= map(.a)",
        r#"{"u":[{"p":1},{"p":2}]}
"#,
    ),
    (
        r#"{"u":[{"a":1},{"a":2}],"v":3}"#,
        r".u |= map(.a) | .v += 1",
        r#"{"u":[1,2],"v":4}
"#,
    ),
    (
        r#"{"u":[{"a":1},{"a":2}]}"#,
        r". as $d | .u |= map(.a) | [., $d]",
        r#"[{"u":[1,2]},{"u":[{"a":1},{"a":2}]}]
"#,
    ),
    (
        r#"{"u":[{"a":1},{"a":2}]}"#,
        r".u as $x | .u |= map(.a) | [., $x]",
        r#"[{"u":[1,2]},[{"a":1},{"a":2}]]
"#,
    ),
    (
        r#"[[{"a":1}],[{"a":2}]]"#,
        r".[] |= map(.a)",
        r"[[1],[2]]
",
    ),
    (
        r#"{"u":[{"a":"é\u0000x"},{"a":"\ud83d\ude00"}]}"#,
        r".u |= map(.a)",
        r#"{"u":["é\u0000x","😀"]}
"#,
    ),
    (
        r#"{"u":[{"a":1},2]}"#,
        r".u |= map(.a?)",
        r#"{"u":[1]}
"#,
    ),
    (
        r#"{"u":[{"a":1},2]}"#,
        r#"try (.u |= map(.a)) catch "caught""#,
        r#""caught"
"#,
    ),
    (r#"{"u":[{"a":1},2]}"#, r"(.u |= map(.a))?", r""),
    (
        r#"{"u":{"x":{"a":1},"y":{"a":2}}}"#,
        r".u |= map(.a)",
        r#"{"u":[1,2]}
"#,
    ),
    (
        r#"{"u":[]}"#,
        r".u |= map(.a)",
        r#"{"u":[]}
"#,
    ),
    (
        r#"{"u":[{"a":1},{"a":2}],"w":3}"#,
        r".w = (.u | map(.a))",
        r#"{"u":[{"a":1},{"a":2}],"w":[1,2]}
"#,
    ),
    (
        r#"{"u":[{"a":1},{"a":2}],"w":3}"#,
        r".u |= (map(.a) | length)",
        r#"{"u":2,"w":3}
"#,
    ),
    (
        r#"{"u":[{"a":1},{"a":2}]}"#,
        r".u |= (. | map(.a))",
        r#"{"u":[1,2]}
"#,
    ),
    (
        r#"[{"a":1},{"a":2}]"#,
        r". += map(.a)",
        r#"[{"a":1},{"a":2},1,2]
"#,
    ),
    (
        r#"{"s":[{"x":{"x":{"x":1}}}]}"#,
        r"reduce range(3) as $i (.; .s |= map(.x))",
        r#"{"s":[1]}
"#,
    ),
    (
        r#"{"s":[{"x":1},{"x":2}]}"#,
        r".s |= (map(.x) as $v | $v)",
        r#"{"s":[1,2]}
"#,
    ),
];

/// (input, filter, jq 1.7.1's stderr).
const RAISED: &[(&str, &str, &str)] = &[
    (
        r#"{"u":[1,2]}"#,
        r".u |= map(.a)",
        r#"jq: error (at <stdin>:0): Cannot index number with string "a"
"#,
    ),
    (
        r#"{"u":["x"]}"#,
        r".u |= map(.a)",
        r#"jq: error (at <stdin>:0): Cannot index string with string "a"
"#,
    ),
    (
        r#"{"u":[true]}"#,
        r".u |= map(.a)",
        r#"jq: error (at <stdin>:0): Cannot index boolean with string "a"
"#,
    ),
    (
        r#"{"u":[[1]]}"#,
        r".u |= map(.a)",
        r#"jq: error (at <stdin>:0): Cannot index array with string "a"
"#,
    ),
    (
        r#"{"u":[{"a":1}]}"#,
        r".u |= map(.[0])",
        r"jq: error (at <stdin>:0): Cannot index object with number
",
    ),
    (
        r#"{"u":[{"a":1},2]}"#,
        r".u |= map(.a)",
        r#"jq: error (at <stdin>:0): Cannot index number with string "a"
"#,
    ),
    (
        r#"{"u":7}"#,
        r".u |= map(.a)",
        r"jq: error (at <stdin>:0): Cannot iterate over number (7)
",
    ),
    (
        r#"{"u":[{"a":1},{"a":2}]}"#,
        r"(.u, .u) |= map(.a)",
        r#"jq: error (at <stdin>:0): Cannot index number with string "a"
"#,
    ),
    (
        r#"{"s":[{"x":1},{"x":2}]}"#,
        r"reduce range(3) as $i (.; .s |= map(.x))",
        r#"jq: error (at <stdin>:0): Cannot index number with string "x"
"#,
    ),
];

#[test]
fn test_map_projection_in_an_update_answers_as_jq_does_6122() -> Result<()> {
    for (input, filter, expected) in ANSWERED {
        let (stdout, stderr, code) = run_jq_stdin_streams(filter, input, &["-c"])?;
        assert_eq!(
            (stdout.as_str(), code),
            (*expected, 0),
            "{filter} over {input}: {stderr}"
        );
    }
    Ok(())
}

#[test]
fn test_map_projection_in_an_update_raises_what_jq_raises_6122() -> Result<()> {
    for (input, filter, expected) in RAISED {
        let (stdout, stderr, code) = run_jq_stdin_streams(filter, input, &["-c"])?;
        assert_eq!(
            (stdout.as_str(), stderr.as_str(), code),
            ("", *expected, 5),
            "{filter} over {input}"
        );
    }
    Ok(())
}
