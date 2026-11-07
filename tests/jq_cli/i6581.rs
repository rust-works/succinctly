//! #6581: a `foreach` whose EXTRACT is a constructor holding the state (`[$i, .]`, `{i: $i, s: .}`) is
//! built on the owned tree, where it used to run on the evaluator's general route every step and
//! re-index the state for each `.` it read. Every expected string was captured from jq 1.7.1
//! (`/usr/bin/jq`).

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn a_constructor_extract_around_the_state_answers_as_jq_6581() -> Result<()> {
    for (filter, want, want_code) in [
        (
            r"[foreach range(5) as $i (0; . + $i; [$i, .])]",
            "[[0,0],[1,1],[2,3],[3,6],[4,10]]\n",
            0,
        ),
        (
            r"[foreach range(5) as $i (0; . + $i; {i: $i, s: .})]",
            "[{\"i\":0,\"s\":0},{\"i\":1,\"s\":1},{\"i\":2,\"s\":3},{\"i\":3,\"s\":6},{\"i\":4,\"s\":10}]\n",
            0,
        ),
        (
            r"[foreach range(4) as $i ([]; . + [$i]; [$i, .])]",
            "[[0,[0]],[1,[0,1]],[2,[0,1,2]],[3,[0,1,2,3]]]\n",
            0,
        ),
        (
            r#"[foreach range(4) as $i ({}; . + {("k\($i)"): $i}; {i: $i, s: .})]"#,
            "[{\"i\":0,\"s\":{\"k0\":0}},{\"i\":1,\"s\":{\"k0\":0,\"k1\":1}},{\"i\":2,\"s\":{\"k0\":0,\"k1\":1,\"k2\":2}},{\"i\":3,\"s\":{\"k0\":0,\"k1\":1,\"k2\":2,\"k3\":3}}]\n",
            0,
        ),
        (
            r"[foreach range(4) as $i ([]; . + [$i]; [., .])]",
            "[[[0],[0]],[[0,1],[0,1]],[[0,1,2],[0,1,2]],[[0,1,2,3],[0,1,2,3]]]\n",
            0,
        ),
        (
            r"[foreach range(4) as $i ([]; . + [$i]; {a: ., b: (. | length)})]",
            "[{\"a\":[0],\"b\":1},{\"a\":[0,1],\"b\":2},{\"a\":[0,1,2],\"b\":3},{\"a\":[0,1,2,3],\"b\":4}]\n",
            0,
        ),
        (
            r"[foreach range(4) as $i ([]; [$i, .]; [., $i])]",
            "[[[0,[]],0],[[1,[0,[]]],1],[[2,[1,[0,[]]]],2],[[3,[2,[1,[0,[]]]]],3]]\n",
            0,
        ),
        (
            r"[foreach range(3) as $i (null; .; [$i, .])]",
            "[[0,null],[1,null],[2,null]]\n",
            0,
        ),
        (
            r"[foreach range(3) as $i (1; . * 2; {a: $i, a: .})]",
            "[{\"a\":2},{\"a\":4},{\"a\":8}]\n",
            0,
        ),
        (
            r"[foreach range(3) as $i (1; . * 2; {a: ., a: $i})]",
            "[{\"a\":0},{\"a\":1},{\"a\":2}]\n",
            0,
        ),
        (
            r"[foreach range(3) as $i (1; . * 2; [., (., 10)])]",
            "[[2,2,10],[4,4,10],[8,8,10]]\n",
            0,
        ),
        (
            r"[foreach range(3) as $i (1; . * 2; [., empty])]",
            "[[2],[4],[8]]\n",
            0,
        ),
        (
            r"[foreach range(3) as $i (1; . * 2; {(tostring): .})]",
            "[{\"2\":2},{\"4\":4},{\"8\":8}]\n",
            0,
        ),
        (
            r"[foreach range(3) as $i ([5]; . + [$i]; [.[0], .])]",
            "[[5,[5,0]],[5,[5,0,1]],[5,[5,0,1,2]]]\n",
            0,
        ),
        (
            r"[foreach range(3) as $i ({a: 1}; .a += $i; {k: keys, v: .})]",
            "[{\"k\":[\"a\"],\"v\":{\"a\":1}},{\"k\":[\"a\"],\"v\":{\"a\":2}},{\"k\":[\"a\"],\"v\":{\"a\":4}}]\n",
            0,
        ),
        (
            r#"[foreach range(3) as $i (0; . + 1; [., error("x")])] "#,
            "",
            5,
        ),
        (
            r#"try [foreach range(3) as $i (0; . + 1; [., error("x")])] catch ."#,
            "\"x\"\n",
            0,
        ),
        (
            r"[foreach range(3) as $i (0; . + 1; [., .a?])]",
            "[[1],[2],[3]]\n",
            0,
        ),
        (
            r"try [foreach range(3) as $i (0; . + 1; [., .a])] catch .",
            "\"Cannot index number with string \\\"a\\\"\"\n",
            0,
        ),
        (
            r"[limit(2; foreach range(10) as $i (0; . + $i; [$i, .]))]",
            "[[0,0],[1,1]]\n",
            0,
        ),
        (
            r"[first(foreach range(10) as $i (0; . + $i; [$i, .]))]",
            "[[0,0]]\n",
            0,
        ),
        (
            r"last(foreach range(10) as $i (0; . + $i; {i: $i, s: .}))",
            "{\"i\":9,\"s\":45}\n",
            0,
        ),
        (
            r"isempty(foreach range(10) as $i (0; . + $i; [$i, .]))",
            "false\n",
            0,
        ),
        (
            r"[foreach (1,2) as $i ?// [$i] (0; . + 1; [$i, .])]",
            "[[1,1],[2,2]]\n",
            0,
        ),
        (
            r"[foreach ([1],[2]) as [$a] ?// $a (0; . + 1; [$a, .])]",
            "[[1,1],[2,2]]\n",
            0,
        ),
        (
            r"[foreach range(3) as $i (0; . + $i; [$i, .] | length)]",
            "[2,2,2]\n",
            0,
        ),
        (
            r"[foreach range(3) as $i (0; . + $i; ([$i, .]))]",
            "[[0,0],[1,1],[2,3]]\n",
            0,
        ),
        (
            r"[foreach range(3) as $i (0; . + $i; [$i, .], [$i])]",
            "[[0,0],[0],[1,1],[1],[2,3],[2]]\n",
            0,
        ),
        (
            r"[foreach range(3) as $i (0; . + $i; {s: .}, .)]",
            "[{\"s\":0},0,{\"s\":1},1,{\"s\":3},3]\n",
            0,
        ),
        (
            r"[foreach range(2) as $i ([0]; . + [$i]; [$i, .]) | .[1] | length]",
            "[2,3]\n",
            0,
        ),
        (
            r"reduce (foreach range(5) as $i ([]; . + [$i]; [., $i])) as $x (0; . + ($x[0] | length))",
            "15\n",
            0,
        ),
        (
            r"[foreach range(3) as $i (0; . + $i; [$i, ., input_line_number])]",
            "[[0,0,0],[1,1,0],[2,3,0]]\n",
            0,
        ),
        (
            r"[foreach range(3) as $i (0; . + $i; [$i, $__loc__.line])]",
            "[[0,1],[1,1],[2,1]]\n",
            0,
        ),
        (
            r"[foreach ([1,2],[3]) as $a ([]; . + $a; [$a, .])]",
            "[[[1,2],[1,2]],[[3],[1,2,3]]]\n",
            0,
        ),
        (
            r#"[foreach range(3) as $i ("a"; . + "b"; [$i, .])]"#,
            "[[0,\"ab\"],[1,\"abb\"],[2,\"abbb\"]]\n",
            0,
        ),
        (
            r"[foreach range(3) as $i (0; . + $i; {($i|tostring): .})]",
            "[{\"0\":0},{\"1\":1},{\"2\":3}]\n",
            0,
        ),
        (
            r"[foreach range(3) as $i (0; . + $i; {a: [., .]})]",
            "[{\"a\":[0,0]},{\"a\":[1,1]},{\"a\":[3,3]}]\n",
            0,
        ),
        (
            r"[foreach range(3) as $i ([[0]]; . + [[$i]]; [.[0], .])]",
            "[[[0],[[0],[0]]],[[0],[[0],[0],[1]]],[[0],[[0],[0],[1],[2]]]]\n",
            0,
        ),
    ] {
        let (stdout, _, code) = run_jq_full(&["-nc", filter], None)?;
        assert_eq!((stdout.as_str(), code), (want, want_code), "{filter}");
    }
    Ok(())
}

/// The route pin: a state nested `N` levels deep, a constructor EXTRACT around it every step. On the
/// owned tree this is `O(N)`; through the bridge each step wrote the whole nest out and indexed it
/// again, `O(N^2)` (about 1 s for 10,000 steps in a release build, against 12 ms here).
#[test]
fn a_constructor_extract_around_a_deep_state_is_linear_6581() -> Result<()> {
    let filter = r"last(foreach range(10000) as $i (null; [.]; [$i, .])) | [.[0], (.[1] | length)]";
    let started = std::time::Instant::now();
    let (stdout, _, code) = run_jq_full(&["-nc", filter], None)?;
    assert_eq!((stdout.as_str(), code), ("[9999,1]\n", 0));
    assert!(
        started.elapsed() < std::time::Duration::from_secs(20),
        "a constructor EXTRACT around the state took {:?}: it left the owned route",
        started.elapsed()
    );
    Ok(())
}

/// The declines the arm relies on: a loop inside a binding that holds a document node (the embed table
/// is live), a `?//` alternative, and a constructor the step declines (a second `.`, a member that reads a
/// container). Each answers as jq 1.7.1 does, on a document read from stdin.
#[test]
fn a_constructor_extract_that_declines_answers_as_jq_6581() -> Result<()> {
    let doc = "{\"a\":[1,2],\"b\":{\"c\":3}}\n";
    for (filter, want) in [
        (
            r". as $d | [foreach range(3) as $i ($d; .; [$i, .])]",
            "[[0,{\"a\":[1,2],\"b\":{\"c\":3}}],[1,{\"a\":[1,2],\"b\":{\"c\":3}}],[2,{\"a\":[1,2],\"b\":{\"c\":3}}]]\n",
        ),
        (
            r". as $d | [foreach range(3) as $i ($d.a; . + [$i]; [., $d])]",
            "[[[1,2,0],{\"a\":[1,2],\"b\":{\"c\":3}}],[[1,2,0,1],{\"a\":[1,2],\"b\":{\"c\":3}}],[[1,2,0,1,2],{\"a\":[1,2],\"b\":{\"c\":3}}]]\n",
        ),
        (
            r". as $d | [foreach range(2) as $i ($d; .; {k: $i, v: .})] | map(.v.b.c)",
            "[3,3]\n",
        ),
        (
            r"[foreach (1,2) as $i ?// [$i] (0; . + 1; [$i, .])]",
            "[[1,1],[2,2]]\n",
        ),
        (
            r"[foreach ([1],[2]) as [$a] ?// $a (.; .; [$a, .])] | length",
            "2\n",
        ),
        (
            r"[foreach range(3) as $i (.; .; [., .])] | length",
            "3\n",
        ),
        (
            r"[foreach range(3) as $i (.; .; [.a, .b])]",
            "[[[1,2],{\"c\":3}],[[1,2],{\"c\":3}],[[1,2],{\"c\":3}]]\n",
        ),
        (
            r"[foreach range(2) as $i (.; .; {x: .a, y: (.b | keys)})]",
            "[{\"x\":[1,2],\"y\":[\"c\"]},{\"x\":[1,2],\"y\":[\"c\"]}]\n",
        ),
        (
            r"[foreach range(2) as $i (.; .; [$i, (.. | numbers)])]",
            "[[0,1,2,3],[1,1,2,3]]\n",
        ),
    ] {
        let (stdout, _, code) = run_jq_full(&["-c", filter], Some(doc))?;
        assert_eq!((stdout.as_str(), code), (want, 0), "{filter}");
    }
    Ok(())
}
