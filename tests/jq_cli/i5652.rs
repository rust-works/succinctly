//! #5652: the stage after an array or object the filter built (or `fromjson` parsed) reads it from the
//! owned tree instead of writing it out as JSON, indexing it and reading one key back. The answers
//! are the ones the round trip gave, which are jq's, including the identity jq keeps for a
//! container holding a NaN. Expected values were captured from jq 1.7.1 (`/usr/bin/jq`).

use anyhow::Result;

use crate::run_jq_full;

const NUMBERS: &str = r"[0,-0,-0.0,1,-1,1.5,-1.5,2.5,0.5,1E5,1e-5,1e19,100000000000000000000,1.7976931348623157e308,1e1000,-1e1000,9007199254740993,3.14159265358979323846,0.30000000000000004,16,1.0,1.10,100]";

#[test]
fn test_the_stage_after_a_built_container_matches_jq_5652() -> Result<()> {
    for (program, expected) in [
        (
            r"map({a: .} | .a)",
            r"[0,-0,-0.0,1,-1,1.5,-1.5,2.5,0.5,1E+5,0.00001,1E+19,100000000000000000000,1.7976931348623157E+308,1E+1000,-1E+1000,9007199254740993,3.14159265358979323846,0.30000000000000004,16,1.0,1.10,100]",
        ),
        (
            r"map({a: ., b: [.]} | .b)",
            r"[[0],[-0],[-0.0],[1],[-1],[1.5],[-1.5],[2.5],[0.5],[1E+5],[0.00001],[1E+19],[100000000000000000000],[1.7976931348623157E+308],[1E+1000],[-1E+1000],[9007199254740993],[3.14159265358979323846],[0.30000000000000004],[16],[1.0],[1.10],[100]]",
        ),
        (
            r"map({a: ., b: .} | .b)",
            r"[0,-0,-0.0,1,-1,1.5,-1.5,2.5,0.5,1E+5,0.00001,1E+19,100000000000000000000,1.7976931348623157E+308,1E+1000,-1E+1000,9007199254740993,3.14159265358979323846,0.30000000000000004,16,1.0,1.10,100]",
        ),
        (
            r"map({a: ., a: 2} | .a)",
            r"[2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,2]",
        ),
        (
            r"map([., .] | .[1])",
            r"[0,-0,-0.0,1,-1,1.5,-1.5,2.5,0.5,1E+5,0.00001,1E+19,100000000000000000000,1.7976931348623157E+308,1E+1000,-1E+1000,9007199254740993,3.14159265358979323846,0.30000000000000004,16,1.0,1.10,100]",
        ),
        (
            r"map([., .] | .[0])",
            r"[0,-0,-0.0,1,-1,1.5,-1.5,2.5,0.5,1E+5,0.00001,1E+19,100000000000000000000,1.7976931348623157E+308,1E+1000,-1E+1000,9007199254740993,3.14159265358979323846,0.30000000000000004,16,1.0,1.10,100]",
        ),
        (
            r"map([.] | .[0])",
            r"[0,-0,-0.0,1,-1,1.5,-1.5,2.5,0.5,1E+5,0.00001,1E+19,100000000000000000000,1.7976931348623157E+308,1E+1000,-1E+1000,9007199254740993,3.14159265358979323846,0.30000000000000004,16,1.0,1.10,100]",
        ),
        (
            r"map({a: .} | select(.a > 1))",
            r#"[{"a":1.5},{"a":2.5},{"a":1E+5},{"a":1E+19},{"a":100000000000000000000},{"a":1.7976931348623157E+308},{"a":1E+1000},{"a":9007199254740993},{"a":3.14159265358979323846},{"a":16},{"a":1.10},{"a":100}]"#,
        ),
        (
            r"map({a: .} | select(.a > 1) | .a)",
            r"[1.5,2.5,1E+5,1E+19,100000000000000000000,1.7976931348623157E+308,1E+1000,9007199254740993,3.14159265358979323846,16,1.10,100]",
        ),
        (
            r"map({a: .} | if .a > 1 then .a else empty end)",
            r"[1.5,2.5,1E+5,1E+19,100000000000000000000,1.7976931348623157E+308,1E+1000,9007199254740993,3.14159265358979323846,16,1.10,100]",
        ),
        (
            r#"map({a: .} | if .a > 1 then "big" else "small" end)"#,
            r#"["small","small","small","small","small","big","small","big","small","big","small","big","big","big","big","small","big","big","small","big","small","big","big"]"#,
        ),
        (
            r#"map({a: .} | has("a"))"#,
            r"[true,true,true,true,true,true,true,true,true,true,true,true,true,true,true,true,true,true,true,true,true,true,true]",
        ),
        (
            r#"map({a: .} | has("b"))"#,
            r"[false,false,false,false,false,false,false,false,false,false,false,false,false,false,false,false,false,false,false,false,false,false,false]",
        ),
        (
            r"map({a: .} | keys)",
            r#"[["a"],["a"],["a"],["a"],["a"],["a"],["a"],["a"],["a"],["a"],["a"],["a"],["a"],["a"],["a"],["a"],["a"],["a"],["a"],["a"],["a"],["a"],["a"]]"#,
        ),
        (
            r"map([., .] | has(1))",
            r"[true,true,true,true,true,true,true,true,true,true,true,true,true,true,true,true,true,true,true,true,true,true,true]",
        ),
        (
            r"map([., .] | has(2))",
            r"[false,false,false,false,false,false,false,false,false,false,false,false,false,false,false,false,false,false,false,false,false,false,false]",
        ),
        (
            r"map([., .] | keys)",
            r"[[0,1],[0,1],[0,1],[0,1],[0,1],[0,1],[0,1],[0,1],[0,1],[0,1],[0,1],[0,1],[0,1],[0,1],[0,1],[0,1],[0,1],[0,1],[0,1],[0,1],[0,1],[0,1],[0,1]]",
        ),
        (
            r"map({a: .} | tojson)",
            r#"["{\"a\":0}","{\"a\":-0}","{\"a\":-0.0}","{\"a\":1}","{\"a\":-1}","{\"a\":1.5}","{\"a\":-1.5}","{\"a\":2.5}","{\"a\":0.5}","{\"a\":1E+5}","{\"a\":0.00001}","{\"a\":1E+19}","{\"a\":100000000000000000000}","{\"a\":1.7976931348623157E+308}","{\"a\":1E+1000}","{\"a\":-1E+1000}","{\"a\":9007199254740993}","{\"a\":3.14159265358979323846}","{\"a\":0.30000000000000004}","{\"a\":16}","{\"a\":1.0}","{\"a\":1.10}","{\"a\":100}"]"#,
        ),
        (
            r"map([.] | tojson)",
            r#"["[0]","[-0]","[-0.0]","[1]","[-1]","[1.5]","[-1.5]","[2.5]","[0.5]","[1E+5]","[0.00001]","[1E+19]","[100000000000000000000]","[1.7976931348623157E+308]","[1E+1000]","[-1E+1000]","[9007199254740993]","[3.14159265358979323846]","[0.30000000000000004]","[16]","[1.0]","[1.10]","[100]"]"#,
        ),
        (
            r"map({a: .} | tostring)",
            r#"["{\"a\":0}","{\"a\":-0}","{\"a\":-0.0}","{\"a\":1}","{\"a\":-1}","{\"a\":1.5}","{\"a\":-1.5}","{\"a\":2.5}","{\"a\":0.5}","{\"a\":1E+5}","{\"a\":0.00001}","{\"a\":1E+19}","{\"a\":100000000000000000000}","{\"a\":1.7976931348623157E+308}","{\"a\":1E+1000}","{\"a\":-1E+1000}","{\"a\":9007199254740993}","{\"a\":3.14159265358979323846}","{\"a\":0.30000000000000004}","{\"a\":16}","{\"a\":1.0}","{\"a\":1.10}","{\"a\":100}"]"#,
        ),
        (
            r"map({a: .} | .a == .a)",
            r"[true,true,true,true,true,true,true,true,true,true,true,true,true,true,true,true,true,true,true,true,true,true,true]",
        ),
        (
            r"map({a: .} | .a | tojson)",
            r#"["0","-0","-0.0","1","-1","1.5","-1.5","2.5","0.5","1E+5","0.00001","1E+19","100000000000000000000","1.7976931348623157E+308","1E+1000","-1E+1000","9007199254740993","3.14159265358979323846","0.30000000000000004","16","1.0","1.10","100"]"#,
        ),
        (
            r"map({a: .} | .a | tostring)",
            r#"["0","-0","-0.0","1","-1","1.5","-1.5","2.5","0.5","1E+5","0.00001","1E+19","100000000000000000000","1.7976931348623157E+308","1E+1000","-1E+1000","9007199254740993","3.14159265358979323846","0.30000000000000004","16","1.0","1.10","100"]"#,
        ),
        (
            r"map({a: .} | .a | type)",
            r#"["number","number","number","number","number","number","number","number","number","number","number","number","number","number","number","number","number","number","number","number","number","number","number"]"#,
        ),
        (
            r"map({a: .} | .b)",
            r"[null,null,null,null,null,null,null,null,null,null,null,null,null,null,null,null,null,null,null,null,null,null,null]",
        ),
        (
            r#"map({a: .} | .["a"])"#,
            r"[0,-0,-0.0,1,-1,1.5,-1.5,2.5,0.5,1E+5,0.00001,1E+19,100000000000000000000,1.7976931348623157E+308,1E+1000,-1E+1000,9007199254740993,3.14159265358979323846,0.30000000000000004,16,1.0,1.10,100]",
        ),
        (
            r"map({a: .} | .a // 1)",
            r"[0,-0,-0.0,1,-1,1.5,-1.5,2.5,0.5,1E+5,0.00001,1E+19,100000000000000000000,1.7976931348623157E+308,1E+1000,-1E+1000,9007199254740993,3.14159265358979323846,0.30000000000000004,16,1.0,1.10,100]",
        ),
        (
            r"map({a: .} | .a and true)",
            r"[true,true,true,true,true,true,true,true,true,true,true,true,true,true,true,true,true,true,true,true,true,true,true]",
        ),
        (
            r"map([., .] | .[-1])",
            r"[0,-0,-0.0,1,-1,1.5,-1.5,2.5,0.5,1E+5,0.00001,1E+19,100000000000000000000,1.7976931348623157E+308,1E+1000,-1E+1000,9007199254740993,3.14159265358979323846,0.30000000000000004,16,1.0,1.10,100]",
        ),
        (
            r"map([., .] | .[5])",
            r"[null,null,null,null,null,null,null,null,null,null,null,null,null,null,null,null,null,null,null,null,null,null,null]",
        ),
        (
            r"map({a: .} | tojson | fromjson | .a)",
            r"[0,-0,-0.0,1,-1,1.5,-1.5,2.5,0.5,1E+5,0.00001,1E+19,100000000000000000000,1.7976931348623157E+308,1E+1000,-1E+1000,9007199254740993,3.14159265358979323846,0.30000000000000004,16,1.0,1.10,100]",
        ),
        (
            r"map({a: .} | tojson | fromjson | select(.a > 1) | .a)",
            r"[1.5,2.5,1E+5,1E+19,100000000000000000000,1.7976931348623157E+308,1E+1000,9007199254740993,3.14159265358979323846,16,1.10,100]",
        ),
        (
            r"map(tojson | fromjson)",
            r"[0,-0,-0.0,1,-1,1.5,-1.5,2.5,0.5,1E+5,0.00001,1E+19,100000000000000000000,1.7976931348623157E+308,1E+1000,-1E+1000,9007199254740993,3.14159265358979323846,0.30000000000000004,16,1.0,1.10,100]",
        ),
        (
            r"map({a: .} | tojson | fromjson | keys)",
            r#"[["a"],["a"],["a"],["a"],["a"],["a"],["a"],["a"],["a"],["a"],["a"],["a"],["a"],["a"],["a"],["a"],["a"],["a"],["a"],["a"],["a"],["a"],["a"]]"#,
        ),
        (
            r"map({a: .} | try .[0] catch .)",
            r#"["Cannot index object with number","Cannot index object with number","Cannot index object with number","Cannot index object with number","Cannot index object with number","Cannot index object with number","Cannot index object with number","Cannot index object with number","Cannot index object with number","Cannot index object with number","Cannot index object with number","Cannot index object with number","Cannot index object with number","Cannot index object with number","Cannot index object with number","Cannot index object with number","Cannot index object with number","Cannot index object with number","Cannot index object with number","Cannot index object with number","Cannot index object with number","Cannot index object with number","Cannot index object with number"]"#,
        ),
        (
            r"map([., .] | try .a catch .)",
            r#"["Cannot index array with string \"a\"","Cannot index array with string \"a\"","Cannot index array with string \"a\"","Cannot index array with string \"a\"","Cannot index array with string \"a\"","Cannot index array with string \"a\"","Cannot index array with string \"a\"","Cannot index array with string \"a\"","Cannot index array with string \"a\"","Cannot index array with string \"a\"","Cannot index array with string \"a\"","Cannot index array with string \"a\"","Cannot index array with string \"a\"","Cannot index array with string \"a\"","Cannot index array with string \"a\"","Cannot index array with string \"a\"","Cannot index array with string \"a\"","Cannot index array with string \"a\"","Cannot index array with string \"a\"","Cannot index array with string \"a\"","Cannot index array with string \"a\"","Cannot index array with string \"a\"","Cannot index array with string \"a\""]"#,
        ),
        (
            r"map({a: .} | .a?)",
            r"[0,-0,-0.0,1,-1,1.5,-1.5,2.5,0.5,1E+5,0.00001,1E+19,100000000000000000000,1.7976931348623157E+308,1E+1000,-1E+1000,9007199254740993,3.14159265358979323846,0.30000000000000004,16,1.0,1.10,100]",
        ),
        (
            r"map({a: .} | select(.a))",
            r#"[{"a":0},{"a":-0},{"a":-0.0},{"a":1},{"a":-1},{"a":1.5},{"a":-1.5},{"a":2.5},{"a":0.5},{"a":1E+5},{"a":0.00001},{"a":1E+19},{"a":100000000000000000000},{"a":1.7976931348623157E+308},{"a":1E+1000},{"a":-1E+1000},{"a":9007199254740993},{"a":3.14159265358979323846},{"a":0.30000000000000004},{"a":16},{"a":1.0},{"a":1.10},{"a":100}]"#,
        ),
        (r"map({a: .} | select(.a == 1.0))", r#"[{"a":1},{"a":1.0}]"#),
        (r"map({a: .} | empty)", r"[]"),
        (
            r"map({a: .} | . == .)",
            r"[true,true,true,true,true,true,true,true,true,true,true,true,true,true,true,true,true,true,true,true,true,true,true]",
        ),
        (
            r"map({a: .} | . + {b: 1})",
            r#"[{"a":0,"b":1},{"a":-0,"b":1},{"a":-0.0,"b":1},{"a":1,"b":1},{"a":-1,"b":1},{"a":1.5,"b":1},{"a":-1.5,"b":1},{"a":2.5,"b":1},{"a":0.5,"b":1},{"a":1E+5,"b":1},{"a":0.00001,"b":1},{"a":1E+19,"b":1},{"a":100000000000000000000,"b":1},{"a":1.7976931348623157E+308,"b":1},{"a":1E+1000,"b":1},{"a":-1E+1000,"b":1},{"a":9007199254740993,"b":1},{"a":3.14159265358979323846,"b":1},{"a":0.30000000000000004,"b":1},{"a":16,"b":1},{"a":1.0,"b":1},{"a":1.10,"b":1},{"a":100,"b":1}]"#,
        ),
        (
            r"map({a: .} | . + null)",
            r#"[{"a":0},{"a":-0},{"a":-0.0},{"a":1},{"a":-1},{"a":1.5},{"a":-1.5},{"a":2.5},{"a":0.5},{"a":1E+5},{"a":0.00001},{"a":1E+19},{"a":100000000000000000000},{"a":1.7976931348623157E+308},{"a":1E+1000},{"a":-1E+1000},{"a":9007199254740993},{"a":3.14159265358979323846},{"a":0.30000000000000004},{"a":16},{"a":1.0},{"a":1.10},{"a":100}]"#,
        ),
        (
            r"map({a: .} | length)",
            r"[1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1]",
        ),
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-c", program], Some(NUMBERS))?;
        assert_eq!(code, 0, "{program}: {stderr:?}");
        assert_eq!(stdout.trim_end(), expected, "{program}");
    }
    Ok(())
}

/// A container holding a NaN is the same storage wherever it is read: `[nan] | . == .` is `true`.
#[test]
fn test_a_built_container_keeps_its_identity_5652() -> Result<()> {
    for (program, expected) in [
        (r"[nan] as $n | {a: $n} | .a == .a", r"true"),
        (r"[nan] as $n | {a: $n} | [.a] == [.a]", r"true"),
        (r"[nan] as $n | [{a: $n} | .a] | .[0] == .[0]", r"true"),
        (r"{a: [nan]} | .a == .a", r"true"),
        (r"{a: [nan]} | [.a, .a] | .[0] == .[1]", r"true"),
        (r"[[nan]] | .[0] == .[0]", r"true"),
        (r"[[nan]] | [.[0], .[0]] | .[0] == .[1]", r"true"),
        (r"[nan] | {a: .} | .a == .a", r"true"),
        (r"[nan] | [., .] | .[0] == .[1]", r"true"),
        (r"[nan] | {a: .} | select(.a == .a) | .a | length", r"1"),
        (r#"[nan] | {a: .} | has("a")"#, r"true"),
        (r"[nan, 1] | {a: .} | keys", r#"["a"]"#),
        (r"[nan] | [.] | tojson", r#""[[null]]""#),
        (r"[nan] | {a: .} | tojson | fromjson | .a", r"[null]"),
        (r"[nan] | {a: .} | tojson | fromjson | .a == .a", r"true"),
        (r"{a: nan} | .a == .a", r"false"),
        (r"{a: nan} | .a | isnan", r"true"),
        (
            r"[infinite, -infinite] | {a: .} | .a",
            r"[1.7976931348623157e+308,-1.7976931348623157e+308]",
        ),
        (r"[infinite] | [., .] | .[0]", r"[1.7976931348623157e+308]"),
        (r"[1,2] | {a: .} | .a | .[0]", r"1"),
        (r"{a: {b: [1, 2]}} | .a | .b", r"[1,2]"),
        (r"{a: {b: [1, 2]}} | .a.b", r"[1,2]"),
        (r"{a: {b: 1.0}} | .a.b", r"1.0"),
        (r"{a: 1.0} | .a", r"1.0"),
        (r"{a: 100000000000000000000} | .a", r"100000000000000000000"),
        (
            r"[1.10, 1.0, 100000000000000000000] | {a: .} | .a",
            r"[1.10,1.0,100000000000000000000]",
        ),
        (
            r"[1.10, 1.0, 100000000000000000000] | [., .] | .[1]",
            r"[1.10,1.0,100000000000000000000]",
        ),
        (r"[1.10] | tojson | fromjson | .[0]", r"1.10"),
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-nc", program], None)?;
        assert_eq!(code, 0, "{program}: {stderr:?}");
        assert_eq!(stdout.trim_end(), expected, "{program}");
    }
    Ok(())
}
