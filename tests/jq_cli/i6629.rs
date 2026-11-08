//! #6629: a `foreach` whose EXTRACT is an array with no `.` member (`[.a, .b]`) is
//! built on the owned tree, and whether an EXTRACT is such a constructor is decided once per fold
//! rather than walked for every step. Every expected string was captured from jq 1.7.1
//! (`/usr/bin/jq`).

use anyhow::Result;

use crate::run_jq_full;

/// The grid: `.`-free constructors the step settles (scalar reads, container slots, a nested
/// constructor, a repeated key), the ones it declines (overlapping slots, a read that raises, a
/// source-spelled float, a multi-output or non-read member), and EXTRACTs the fold never tries.
#[test]
fn a_dot_free_constructor_extract_answers_as_jq_6629() -> Result<()> {
    for (filter, want, want_code) in [
        (
            r"[foreach range(5) as $i ({a: 1, b: 2}; .a += 1; [.a, .b])]",
            "[[2,2],[3,2],[4,2],[5,2],[6,2]]\n",
            0,
        ),
        (
            r"[foreach range(4) as $i ({a: 1, b: 2}; .a += 1; {x: .a, y: .b})]",
            "[{\"x\":2,\"y\":2},{\"x\":3,\"y\":2},{\"x\":4,\"y\":2},{\"x\":5,\"y\":2}]\n",
            0,
        ),
        (
            r"[foreach range(4) as $i ({a: 1, b: 2}; .a += 1; [.a, .b, .a + .b])]",
            "[[2,2,4],[3,2,5],[4,2,6],[5,2,7]]\n",
            0,
        ),
        (
            r"[foreach range(4) as $i ({a: 1, b: 2}; .a += 1; [$i, .a])]",
            "[[0,2],[1,3],[2,4],[3,5]]\n",
            0,
        ),
        (
            r"[foreach range(4) as $i ({a: 1, b: 2}; .a += 1; {k: $i, v: .b})]",
            "[{\"k\":0,\"v\":2},{\"k\":1,\"v\":2},{\"k\":2,\"v\":2},{\"k\":3,\"v\":2}]\n",
            0,
        ),
        (
            r"[foreach range(3) as $i ({a: [], n: 0}; .a += [$i] | .n += 1; [.a, .n])]",
            "[[[0],1],[[0,1],2],[[0,1,2],3]]\n",
            0,
        ),
        (
            r"[foreach range(3) as $i ({a: [], n: 0}; .a += [$i] | .n += 1; {a: .a, l: (.a | length)})]",
            "[{\"a\":[0],\"l\":1},{\"a\":[0,1],\"l\":2},{\"a\":[0,1,2],\"l\":3}]\n",
            0,
        ),
        (
            r"[foreach range(3) as $i ({a: [1], b: [2]}; .a += [$i]; [.a, .b])]",
            "[[[1,0],[2]],[[1,0,1],[2]],[[1,0,1,2],[2]]]\n",
            0,
        ),
        (
            r"[foreach range(3) as $i ({a: {b: 1}}; .a.b += 1; [.a, .a.b])]",
            "[[{\"b\":2},2],[{\"b\":3},3],[{\"b\":4},4]]\n",
            0,
        ),
        (
            r"[foreach range(3) as $i ({a: {b: 1}}; .a.b += 1; [.a.b, .a])]",
            "[[2,{\"b\":2}],[3,{\"b\":3}],[4,{\"b\":4}]]\n",
            0,
        ),
        (
            r"[foreach range(3) as $i ({a: 1}; .a += 1; [.a, .a])]",
            "[[2,2],[3,3],[4,4]]\n",
            0,
        ),
        (
            r"[foreach range(3) as $i ({a: 1}; .a += 1; {x: .a, y: .a})]",
            "[{\"x\":2,\"y\":2},{\"x\":3,\"y\":3},{\"x\":4,\"y\":4}]\n",
            0,
        ),
        (
            r"[foreach range(3) as $i ({a: 1, b: 2}; .a += 1; {a: .a, a: .b})]",
            "[{\"a\":2},{\"a\":2},{\"a\":2}]\n",
            0,
        ),
        (
            r"[foreach range(3) as $i ({a: 1, b: 2}; .a += 1; {a: .b, a: .a})]",
            "[{\"a\":2},{\"a\":3},{\"a\":4}]\n",
            0,
        ),
        (
            r#"[foreach range(3) as $i ({a: 1, k: "z"}; .a += 1; {(.k): .a})]"#,
            "[{\"z\":2},{\"z\":3},{\"z\":4}]\n",
            0,
        ),
        (
            r"[foreach range(3) as $i ({a: 1}; .a += 1; [.b, .a])]",
            "[[null,2],[null,3],[null,4]]\n",
            0,
        ),
        (
            r"[foreach range(3) as $i (null; .; [.a, .b])]",
            "[[null,null],[null,null],[null,null]]\n",
            0,
        ),
        (
            r"[foreach range(3) as $i ([5, 6]; . + [$i]; [.[0], .[1]])]",
            "[[5,6],[5,6],[5,6]]\n",
            0,
        ),
        (
            r"[foreach range(3) as $i ([5, 6]; . + [$i]; [.[0], .[-1]])]",
            "[[5,0],[5,1],[5,2]]\n",
            0,
        ),
        (
            r"[foreach range(3) as $i ([5, 6]; . + [$i]; [.[5], .[0]])]",
            "[[null,5],[null,5],[null,5]]\n",
            0,
        ),
        (
            r"[foreach range(3) as $i ({a: 1.5, b: 1e1000}; .; [.a, .b])]",
            "[[1.5,1E+1000],[1.5,1E+1000],[1.5,1E+1000]]\n",
            0,
        ),
        (
            r"[foreach range(3) as $i ({a: 1.0, b: 100000000000000000000}; .; [.a, .b, $i])]",
            "[[1.0,100000000000000000000,0],[1.0,100000000000000000000,1],[1.0,100000000000000000000,2]]\n",
            0,
        ),
        (
            r"[foreach range(3) as $i ({a: 1.0}; .; {x: .a, i: $i})]",
            "[{\"x\":1.0,\"i\":0},{\"x\":1.0,\"i\":1},{\"x\":1.0,\"i\":2}]\n",
            0,
        ),
        (
            r#"[foreach range(3) as $i ({a: "s", b: true}; .; [.a, .b, null, 1, "x"])]"#,
            "[[\"s\",true,null,1,\"x\"],[\"s\",true,null,1,\"x\"],[\"s\",true,null,1,\"x\"]]\n",
            0,
        ),
        (
            r"[foreach range(3) as $i ({a: 1}; .; [1, 2])]",
            "[[1,2],[1,2],[1,2]]\n",
            0,
        ),
        (
            r"[foreach range(3) as $i ({a: 1}; .; {x: 1})]",
            "[{\"x\":1},{\"x\":1},{\"x\":1}]\n",
            0,
        ),
        (
            r"[foreach range(3) as $i ({a: 1}; .; [])]",
            "[[],[],[]]\n",
            0,
        ),
        (
            r"[foreach range(3) as $i ({a: 1}; .; {})]",
            "[{},{},{}]\n",
            0,
        ),
        (
            r"[foreach range(3) as $i ({a: 1}; .; [.a, (1, 2)])]",
            "[[1,1,2],[1,1,2],[1,1,2]]\n",
            0,
        ),
        (
            r"[foreach range(3) as $i ({a: 1}; .; [.a, empty])]",
            "[[1],[1],[1]]\n",
            0,
        ),
        (
            r"[foreach range(3) as $i ({a: 1}; .; [.a, (.a | tostring)])]",
            "[[1,\"1\"],[1,\"1\"],[1,\"1\"]]\n",
            0,
        ),
        (
            r"[foreach range(3) as $i ({a: 1}; .; [.a, (.a | length)])]",
            "[[1,1],[1,1],[1,1]]\n",
            0,
        ),
        (
            r#"[foreach range(3) as $i ({a: "abc"}; .; [.a | length, .a])]"#,
            "",
            5,
        ),
        (
            r"[foreach range(3) as $i ({a: 1}; .; [.a, .])]",
            "[[1,{\"a\":1}],[1,{\"a\":1}],[1,{\"a\":1}]]\n",
            0,
        ),
        (
            r"[foreach range(3) as $i ({a: 1}; .; [., .])]",
            "[[{\"a\":1},{\"a\":1}],[{\"a\":1},{\"a\":1}],[{\"a\":1},{\"a\":1}]]\n",
            0,
        ),
        (
            r"[foreach range(3) as $i ({a: 1}; .; [.a // 7, .b // 8])]",
            "[[1,8],[1,8],[1,8]]\n",
            0,
        ),
        (
            r"[foreach range(3) as $i ({a: 1}; .; [.a > 0, .a == 1])]",
            "[[true,true],[true,true],[true,true]]\n",
            0,
        ),
        (
            r"[foreach range(3) as $i ({a: 1}; .; [.a + 1, .a - 1, .a * 2, .a / 2])]",
            "[[2,0,2,0.5],[2,0,2,0.5],[2,0,2,0.5]]\n",
            0,
        ),
        (
            r"[foreach range(3) as $i ({a: 1}; .; [.a / 0, 1])]",
            "",
            5,
        ),
        (
            r"try [foreach range(3) as $i (1; .; [.a, .b])] catch .",
            "\"Cannot index number with string \\\"a\\\"\"\n",
            0,
        ),
        (
            r"try [foreach range(3) as $i ({a: 1}; .; [.a.b, 1])] catch .",
            "\"Cannot index number with string \\\"b\\\"\"\n",
            0,
        ),
        (
            r"[foreach range(3) as $i ({a: 1}; .; [.a?, .b?])]",
            "[[1,null],[1,null],[1,null]]\n",
            0,
        ),
        (
            r"[foreach range(3) as $i ({a: 1}; if $i % 2 == 0 then {a: $i} else {a: [$i]} end; [.a, 1])]",
            "[[0,1],[[1],1],[2,1]]\n",
            0,
        ),
        (
            r"[foreach range(4) as $i ({a: 1}; if $i == 2 then [9] else {a: $i} end; [.a?, 1])]",
            "[[0,1],[1,1],[1],[3,1]]\n",
            0,
        ),
        (
            r"[foreach range(6) as $i ({a: 1, b: [0]}; if $i % 2 == 0 then {a: $i, b: [$i]} else {a: 1.0, b: [$i]} end; [.a, .b])]",
            "[[0,[0]],[1.0,[1]],[2,[2]],[1.0,[3]],[4,[4]],[1.0,[5]]]\n",
            0,
        ),
        (
            r"[limit(2; foreach range(10) as $i ({a: 0}; .a += 1; [.a, $i]))]",
            "[[1,0],[2,1]]\n",
            0,
        ),
        (
            r"[first(foreach range(10) as $i ({a: 0}; .a += 1; [.a, .a]))]",
            "[[1,1]]\n",
            0,
        ),
        (
            r"last(foreach range(10) as $i ({a: 0}; .a += 1; {n: .a}))",
            "{\"n\":10}\n",
            0,
        ),
        (
            r"isempty(foreach range(10) as $i ({a: 0}; .a += 1; [.a]))",
            "false\n",
            0,
        ),
        (
            r"[foreach (1,2) as $i ?// [$i] ({a: 0}; .a += 1; [$i, .a])]",
            "[[1,1],[2,2]]\n",
            0,
        ),
        (
            r"[foreach ([1],[2]) as [$a] ?// $a ({a: 0}; .a += 1; [$a, .a])]",
            "[[1,1],[2,2]]\n",
            0,
        ),
        (
            r"[foreach range(3) as $i (({a: 0}, {a: 10}); .a += 1; [.a, $i])]",
            "[[1,0],[2,1],[3,2],[11,0],[12,1],[13,2]]\n",
            0,
        ),
        (
            r"[foreach range(3) as $i ({a: 0}; .a += 1; [.a, .a] | length)]",
            "[2,2,2]\n",
            0,
        ),
        (
            r"[foreach range(3) as $i ({a: 0}; .a += 1; [.a, $__loc__.line])]",
            "[[1,1],[2,1],[3,1]]\n",
            0,
        ),
        (
            r"[foreach range(3) as $i ({a: 0}; .a += 1; [.a, input_line_number])]",
            "[[1,0],[2,0],[3,0]]\n",
            0,
        ),
        (
            r"[foreach range(3) as $i ({a: 0}; .a += 1; [.a, (1 as $x | $x)])]",
            "[[1,1],[2,1],[3,1]]\n",
            0,
        ),
        (
            r"[foreach range(3) as $i ({a: 0}; .a += 1; [.a, ({a: $i} | .a)])]",
            "[[1,0],[2,1],[3,2]]\n",
            0,
        ),
        (
            r"def f: .a; [foreach range(3) as $i ({a: 0}; .a += 1; [f, $i])]",
            "[[1,0],[2,1],[3,2]]\n",
            0,
        ),
        (
            r"[foreach range(3) as $i ({a: 0}; .a += 1; [.a, $ENV.NOPE_6629])]",
            "[[1,null],[2,null],[3,null]]\n",
            0,
        ),
        (
            r#"[foreach range(3) as $i ({a: 0}; .a += 1; [.a, "k\($i)"])]"#,
            "[[1,\"k0\"],[2,\"k1\"],[3,\"k2\"]]\n",
            0,
        ),
        (
            r#"[foreach range(3) as $i ({a: 0}; .a += 1; {("k\($i)"): .a})]"#,
            "[{\"k0\":1},{\"k1\":2},{\"k2\":3}]\n",
            0,
        ),
        (
            r#"[foreach range(3) as $i ({a: 0}; .a += 1; {"k\($i)": .a})]"#,
            "[{\"k0\":1},{\"k1\":2},{\"k2\":3}]\n",
            0,
        ),
        (
            r"[foreach range(3) as $i ({a: 0}; .a += 1; {$i, v: .a})]",
            "[{\"i\":0,\"v\":1},{\"i\":1,\"v\":2},{\"i\":2,\"v\":3}]\n",
            0,
        ),
        (
            r"[foreach range(3) as $i ({a: 0}; .a += 1; {a})]",
            "[{\"a\":1},{\"a\":2},{\"a\":3}]\n",
            0,
        ),
        (
            r#"[foreach range(3) as $i ({a: 0}; .a += 1; {"a"})]"#,
            "[{\"a\":1},{\"a\":2},{\"a\":3}]\n",
            0,
        ),
        (
            r"[foreach range(3) as $i ({a: 0}; .a += 1; [.a, $i], {z: .a})]",
            "[[1,0],{\"z\":1},[2,1],{\"z\":2},[3,2],{\"z\":3}]\n",
            0,
        ),
        (
            r"[foreach range(3) as $i ({a: 0}; .a += 1; [.a, $i] | .[0])]",
            "[1,2,3]\n",
            0,
        ),
        (
            r#"[foreach range(2) as $i ({a: 0}; .a += 1; [.a, (try error("x") catch .)])]"#,
            "[[1,\"x\"],[2,\"x\"]]\n",
            0,
        ),
        (
            r"[foreach range(3) as $i ({a: 0}; .a += 1; [.a, $i, $i + 1, $i * 2])]",
            "[[1,0,1,0],[2,1,2,2],[3,2,3,4]]\n",
            0,
        ),
        (
            r"[foreach range(3) as $i ({a: 0}; .a += 1; [.a + $i, .a - $i])]",
            "[[1,1],[3,1],[5,1]]\n",
            0,
        ),
        (
            r"reduce (foreach range(3) as $i ({a: 0}; .a += 1; [.a, $i])) as $r (0; . + $r[0])",
            "6\n",
            0,
        ),
        (
            r"[foreach range(1000) as $i ({a: 0, b: 0}; .a += 1; {x: .a, y: .b})] | last",
            "{\"x\":1000,\"y\":0}\n",
            0,
        ),
    ] {
        let (stdout, _, code) = run_jq_full(&["-nc", filter], None)?;
        assert_eq!((stdout.as_str(), code), (want, want_code), "{filter}");
    }
    Ok(())
}
