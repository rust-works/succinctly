//! #6684: jq's `_nwise` (an undocumented builtin its builtin.jq defines, and a widely quoted
//! chunking idiom) is defined here too, as jq defines it:
//! `def _nwise($n): def n: if length <= $n then . else .[0:$n] , (.[$n:] | n) end; n;` and
//! `def _nwise(a; $n): a | _nwise($n);`. Every answer below was captured from jq 1.7.1.

use anyhow::Result;

use crate::{run_jq_full, run_jq_null};

#[test]
fn test_jq_nwise_chunks_as_jq_does_6684() -> Result<()> {
    for (filter, want) in [
        ("[range(10)] | [_nwise(3)]", "[[0,1,2],[3,4,5],[6,7,8],[9]]\n"),
        ("[range(7)] | [_nwise(1)] | length", "7\n"),
        ("[range(7)] | [_nwise(10)]", "[[0,1,2,3,4,5,6]]\n"),
        ("[] | [_nwise(2)]", "[[]]\n"),
        ("\"abcdefg\" | [_nwise(3)]", "[\"abc\",\"def\",\"g\"]\n"),
        ("{} | [_nwise(2)]", "[{}]\n"),
        ("null | [_nwise(2)]", "[null]\n"),
        ("[range(5)] | [_nwise(2,3)]", "[[0,1],[2,3],[4],[0,1,2],[3,4]]\n"),
        ("[range(5)] | [_nwise(1.5)]", "[[0,1],[1,2],[2,3],[3,4],[4]]\n"),
        ("[range(10)] | [_nwise(.;3)]", "[[0,1,2],[3,4,5],[6,7,8],[9]]\n"),
        ("[range(10)] | [_nwise(.[:6];3)]", "[[0,1,2],[3,4,5]]\n"),
        ("[range(10)] | [_nwise(.,.[:4];3)]", "[[0,1,2],[3,4,5],[6,7,8],[9],[0,1,2],[3]]\n"),
        ("[range(10)] | [_nwise(.;2,3)]", "[[0,1],[2,3],[4,5],[6,7],[8,9],[0,1,2],[3,4,5],[6,7,8],[9]]\n"),
        ("[range(10)] | [_nwise(.[2:];3)]", "[[2,3,4],[5,6,7],[8,9]]\n"),
        ("[range(7)] | [limit(2;_nwise(2))]", "[[0,1],[2,3]]\n"),
        ("[range(6)] | [_nwise(2)] | map(add)", "[1,5,9]\n"),
        ("[[1,2],[3,4],[5]] | [_nwise(2)]", "[[[1,2],[3,4]],[[5]]]\n"),
        ("def f: _nwise(2); [range(5)] | [f]", "[[0,1],[2,3],[4]]\n"),
        ("[range(10)] | [path(_nwise(3))]", "[[{\"start\":0,\"end\":3}],[{\"start\":3,\"end\":null},{\"start\":0,\"end\":3}],[{\"start\":3,\"end\":null},{\"start\":3,\"end\":null},{\"start\":0,\"end\":3}],[{\"start\":3,\"end\":null},{\"start\":3,\"end\":null},{\"start\":3,\"end\":null}]]\n"),
        ("[range(10)] | [limit(4; _nwise(0))]", "[[],[],[],[]]\n"),
        ("[range(10)] | [limit(4; _nwise(-1))]", "[[0,1,2,3,4,5,6,7,8],[],[],[]]\n"),
        ("[range(5)] | [_nwise(empty)]", "[]\n"),
        ("[range(6)] | [_nwise(.[0:2] | length)]", "[[0,1],[2,3],[4,5]]\n"),
        ("[range(4)] | 5 as $n | [_nwise($n)]", "[[0,1,2,3]]\n"),
        ("[range(6)] | 2 as $n | [_nwise(.[0:$n]; $n)]", "[[0,1]]\n"),
        ("[range(10)] | 3 as $n | [_nwise(.[:$n+1]; 2)]", "[[0,1],[2,3]]\n"),
        ("def _nwise($n): [$n]; [1] | _nwise(7)", "[7]\n"),
        ("def _nwise(x): \"mine\"; [1] | _nwise(1)", "\"mine\"\n"),
        ("[range(5)] | [_nwise(2)?]", "[[0,1],[2,3],[4]]\n"),
        ("def length: 0; [range(5)] | [_nwise(2)]", "[[0,1],[2,3],[4]]\n"),
        ("def n: 7; [range(5)] | [_nwise(2)]", "[[0,1],[2,3],[4]]\n"),
        ("def _nwise(a;b;c): 1; [range(5)] | [_nwise(2)]", "[[0,1],[2,3],[4]]\n"),
        ("[range(5)] | [_nwise(2)] # def _nwise", "[[0,1],[2,3],[4]]\n"),
        ("\"def _nwise\" as $x | [range(5)] | [_nwise(2)]", "[[0,1],[2,3],[4]]\n"),
        ("def f: def _nwise($n): \"inner\"; _nwise(1); [range(5)] | [_nwise(2)], f", "[[0,1],[2,3],[4]]\n\"inner\"\n"),
        ("def _nwise($n): \"mine\"; [range(5)] | [_nwise(2)]", "[\"mine\"]\n"),
        ("def _nwise(a; $n): \"mine2\"; [range(5)] | [_nwise(.; 2)], [_nwise(2)]", "[\"mine2\"]\n[[0,1],[2,3],[4]]\n"),
        ("def _nwise($n): \"mine\"; def g: _nwise(1); [range(5)] | [_nwise(2)], [g]", "[\"mine\"]\n[\"mine\"]\n"),
        ("def f(_nwise): _nwise; 1 | f(2)", "2\n"),
        ("def _nwise: \"zero\"; [range(5)] | [_nwise], [_nwise(2)]", "[\"zero\"]\n[[0,1],[2,3],[4]]\n"),
        ("[range(10)] | [_nwise(3)][0]", "[0,1,2]\n"),
        ("[range(10)] | _nwise(3) | length", "3\n3\n3\n1\n"),
        ("[builtins[] | select(startswith(\"_\"))]", "[]\n"),
        ("\"_nwise/1\" as $n | [builtins] | index($n)", "null\n"),
    ] {
        let (stdout, code) = run_jq_null(filter, &["-c"])?;
        assert_eq!((stdout.as_str(), code), (want, 0), "{filter}");
    }
    Ok(())
}

#[test]
fn test_jq_nwise_of_a_non_container_raises_as_jq_does_6684() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(&["-nc", "5 | [_nwise(2)]"], None)?;
    assert_eq!(stdout, "");
    assert_eq!(
        stderr,
        "jq: error (at <unknown>): Cannot index number with object\n"
    );
    assert_eq!(code, 5);
    Ok(())
}

#[test]
fn test_jq_nwise_has_no_other_arity_6684() -> Result<()> {
    for (filter, call) in [("_nwise", "_nwise/0"), ("_nwise(1;2;3)", "_nwise/3")] {
        let (stdout, stderr, code) = run_jq_full(&["-nc", filter], None)?;
        assert_eq!(stdout, "", "{filter}");
        assert!(
            stderr.starts_with(&format!(
                "jq: error: {call} is not defined at <top-level>, line 1:\n"
            )),
            "{filter}: {stderr}"
        );
        assert_eq!(code, 3, "{filter}");
    }
    Ok(())
}
