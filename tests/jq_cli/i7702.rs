//! #7702: a `reduce`/`foreach` step, an `until`/`while` round or a `recurse` level whose UPDATE is a
//! call to a `def` wrote the whole state out as JSON and indexed it again at every step, so the run's
//! time grew with the size of state members the step never touches, where jq 1.7.1's does not. The
//! step now hands the state to #7483's owned-call door by value, so the call's write lands in place.
//!
//! These rows pin jq 1.7.1's captured answers (stdout, stderr, exit) over fold and loop steps that
//! call a definition, weighted to what a state consumed too early would get wrong: an update with
//! several outputs or none, `break` out of a fold, an error mid-fold caught outside it, `first` and
//! `limit` over a `foreach`, the state bound to a variable and read after the fold, `$__loc__` and
//! `input` inside the update, a call's closure argument with several outputs, and EXTRACT as a call.
//! jq 1.8.2 answers the same except where marked. The in-place proof is
//! `fold_and_loop_steps_that_call_a_def_take_the_door_7702` in `src/jq/eval/owned_call.rs`.

use anyhow::Result;

use crate::run_jq_full;

/// (program, stdout, stderr, exit code) for `succinctly jq -nc <program>`.
#[allow(clippy::literal_string_with_formatting_args)] // jq object literals read like format arguments
const CASES: &[(&str, &str, &str, i32)] = &[
    (r"def f($v): .a = $v; {x:[1,2]} | reduce range(5) as $i (.; f($i))", "{\"x\":[1,2],\"a\":4}\n", "", 0),
    (r"def f($v): .a = $v; {x:[1,2]} | [foreach range(5) as $i (.; f($i))]", "[{\"x\":[1,2],\"a\":0},{\"x\":[1,2],\"a\":1},{\"x\":[1,2],\"a\":2},{\"x\":[1,2],\"a\":3},{\"x\":[1,2],\"a\":4}]\n", "", 0),
    (r"def f($v): .a = $v; {x:[1,2]} | [foreach range(5) as $i (.; f($i); .a)]", "[0,1,2,3,4]\n", "", 0),
    (r"def f($v): .a = $v; def g: .a * 10; {x:[1,2]} | [foreach range(5) as $i (.; f($i); g)]", "[0,10,20,30,40]\n", "", 0),
    (r"def f($v): .a = $v; {x:[1,2], a:0} | until(.a >= 5; f(.a + 1))", "{\"x\":[1,2],\"a\":5}\n", "", 0),
    (r"def f($v): .a = $v; {x:[1,2], a:0} | [while(.a < 5; f(.a + 1))]", "[{\"x\":[1,2],\"a\":0},{\"x\":[1,2],\"a\":1},{\"x\":[1,2],\"a\":2},{\"x\":[1,2],\"a\":3},{\"x\":[1,2],\"a\":4}]\n", "", 0),
    (r"def f($v): .a = $v; {x:[1,2], a:0} | [recurse(if .a < 5 then f(.a + 1) else empty end) | .a]", "[0,1,2,3,4,5]\n", "", 0),
    (r"def f($v): .a = $v; {a:0} | [limit(3; recurse(f(.a + 1))) | .a]", "[0,1,2]\n", "", 0),
    (r"def f($v): .a = $v; {a:0} | last(limit(5; recurse(f(.a + 1)))) | .a", "4\n", "", 0),
    (r"def f($v): .a = $v, .b = $v; {} | reduce range(3) as $i (.; f($i))", "{\"b\":2}\n", "", 0),
    (r"def f($v): .a = $v, .b = $v; {} | [foreach range(3) as $i (.; f($i))]", "[{\"a\":0},{\"b\":0},{\"b\":0,\"a\":1},{\"b\":1},{\"b\":1,\"a\":2},{\"b\":2}]\n", "", 0),
    (r"def f($v): .a = $v, .b = $v; {} | [foreach range(3) as $i (.; f($i); [.a, .b])]", "[[0,null],[null,0],[1,0],[null,1],[2,1],[null,2]]\n", "", 0),
    (r"def f($v): .a = ($v, $v + 10); {} | reduce range(3) as $i (.; f($i))", "{\"a\":12}\n", "", 0),
    (r"def f($v): .a = ($v, $v + 10); {} | [foreach range(3) as $i (.; f($i); .a)]", "[0,10,1,11,2,12]\n", "", 0),
    (r"def f($v): if $v == 2 then empty else .a = $v end; {} | reduce range(5) as $i (.; f($i))", "{\"a\":4}\n", "", 0),
    (r"def f($v): if $v == 2 then empty else .a = $v end; {} | [foreach range(5) as $i (.; f($i); .a)]", "[0,1,3,4]\n", "", 0),
    (r"def f($v): empty; {} | reduce range(3) as $i (.; f($i))", "null\n", "", 0),
    (r"def f($v): empty; {} | [foreach range(3) as $i (.; f($i))]", "[]\n", "", 0),
    (r"def f($v): .a = $v; {a:0} | [until(.a >= 3; f(.a + 1), f(.a + 2))]", "[{\"a\":3},{\"a\":4},{\"a\":3},{\"a\":3},{\"a\":4}]\n", "", 0),
    (r"def f($v): .a = $v; {a:0} | [while(.a < 3; f(.a + 1), f(.a + 2)) | .a]", "[0,1,2,2]\n", "", 0),
    (r"def f($v): .a = $v; label $out | {} | reduce range(10) as $i (.; if $i == 4 then ., break $out else f($i) end)", "", "", 0),
    // jq 1.8.2 differs here (a reworded error, or `-0` printed as `0`); 1.7.1 is the reference.
    (r"def f($v): if $v == 3 then break $out else .a = $v end; label $out | {} | reduce range(10) as $i (.; f($i))", "", "jq: error: $*label-out is not defined at <top-level>, line 1:\ndef f($v): if $v == 3 then break $out else .a = $v end; label $out | {} | reduce range(10) as $i (.; f($i))                           \njq: 1 compile error\n", 3),
    (r"label $out | {} | reduce range(10) as $i (.; if $i == 3 then break $out else .a = $i end)", "", "", 0),
    (r#"def f($v): if $v == 3 then error("boom \($v)") else .a = $v end; try ({} | reduce range(10) as $i (.; f($i))) catch ."#, "\"boom 3\"\n", "", 0),
    (r#"def f($v): if $v == 3 then error("boom") else .a = $v end; [{} | foreach range(10) as $i (.; f($i); .a)] | length"#, "", "jq: error (at <unknown>): boom\n", 5),
    (r#"def f($v): if $v == 3 then error("boom") else .a = $v end; [try ({} | foreach range(10) as $i (.; f($i); .a)) catch "c"]"#, "[0,1,2,\"c\"]\n", "", 0),
    (r#"def f($v): if $v == 3 then error("boom") else .a = $v end; {} | [.[]?, (reduce range(10) as $i (.; f($i)))?]"#, "[]\n", "", 0),
    (r"def f($v): .l = $__loc__ | .a = $v; {} | reduce range(2) as $i (.; f($i))", "{\"l\":{\"file\":\"<top-level>\",\"line\":1},\"a\":1}\n", "", 0),
    (r"def f($v): .a = $v | .b = input; {} | try (reduce range(2) as $i (.; f($i))) catch .", "\"break\"\n", "", 0),
    (r"def f($v): .a = $v; {} | first(foreach range(10) as $i (.; f($i)))", "{\"a\":0}\n", "", 0),
    (r"def f($v): .a = $v; {} | [limit(2; foreach range(10) as $i (.; f($i); .a))]", "[0,1]\n", "", 0),
    (r"def f($v): .a = $v; {} | first(foreach range(10) as $i (.; f($i); select(.a > 3) | .a))", "4\n", "", 0),
    (r"def f($v): .a = $v; {x:[1]} | . as $o | reduce range(3) as $i (.; f($i)) | [., $o]", "[{\"x\":[1],\"a\":2},{\"x\":[1]}]\n", "", 0),
    (r"def f($v): .x[0] = $v; {x:[1,2]} | . as $o | reduce range(3) as $i (.; f($i)) | [., $o]", "[{\"x\":[2,2]},{\"x\":[1,2]}]\n", "", 0),
    (r"def f($v): .x += [$v]; {x:[]} | reduce range(4) as $i (.; f($i))", "{\"x\":[0,1,2,3]}\n", "", 0),
    (r"def f($v): .x[$v] = $v; {x:[]} | reduce range(4) as $i (.; f($i))", "{\"x\":[0,1,2,3]}\n", "", 0),
    (r"def f($v): . + [$v]; [] | reduce range(4) as $i (.; f($i))", "[0,1,2,3]\n", "", 0),
    (r"def f($v): .[$v|tostring] = $v; {} | reduce range(4) as $i (.; f($i))", "{\"0\":0,\"1\":1,\"2\":2,\"3\":3}\n", "", 0),
    (r"def f($v): .a = $v | .n += 1; {n:0} | reduce range(4) as $i (.; f($i))", "{\"n\":4,\"a\":3}\n", "", 0),
    (r"def f($v): .a = $v; {n:0} | reduce range(4) as $i (.; f($i) | .n += 1)", "{\"n\":4,\"a\":3}\n", "", 0),
    (r"def f($v): .a = $v; {n:0} | reduce range(4) as $i (.; .n += 1 | f($i))", "{\"n\":4,\"a\":3}\n", "", 0),
    (r"def f($n): if $n == 0 then . else .d += 1 | f($n - 1) end; {d:0} | reduce range(3) as $i (.; f($i))", "{\"d\":3}\n", "", 0),
    (r"def f($n): if $n == 0 then . else f($n - 1) | .d += 1 end; {d:0} | reduce range(4) as $i (.; f($i))", "{\"d\":6}\n", "", 0),
    (r"def f: .a += 1; {a:0} | reduce range(5) as $i (.; f)", "{\"a\":5}\n", "", 0),
    (r"def f: .a += 1; {a:0} | [foreach range(5) as $i (.; f; .a)]", "[1,2,3,4,5]\n", "", 0),
    (r"def f: .a += 1; {a:0} | until(.a >= 5; f)", "{\"a\":5}\n", "", 0),
    (r"def f: .a += 1; {a:0} | [while(.a < 5; f) | .a]", "[0,1,2,3,4]\n", "", 0),
    (r"def f(g): .a = g; {} | reduce range(3) as $i (.; f($i * 2))", "{\"a\":4}\n", "", 0),
    // jq 1.8.2 differs here (a reworded error, or `-0` printed as `0`); 1.7.1 is the reference.
    (r"def f(g): .a = g; {} | [foreach range(3) as $i (.; f($i, -$i); .a)]", "[0,-0,1,-1,2,-2]\n", "", 0),
    (r"def f($v): .a = $v; [{x:1},{x:2}] | map(reduce range(3) as $i (.; f($i)))", "[{\"x\":1,\"a\":2},{\"x\":2,\"a\":2}]\n", "", 0),
    (r"def f($v): .a = $v; {} | reduce (range(3) | {i:.}) as {i:$i} (.; f($i))", "{\"a\":2}\n", "", 0),
    // jq 1.8.2 differs here (a reworded error, or `-0` printed as `0`); 1.7.1 is the reference.
    (r"def f($v): .a = $v; {} | reduce range(3) as [$i] (.; f($i))", "", "jq: error (at <unknown>): Cannot index number with number\n", 5),
    (r"def f($v): .a = $v; {} | [foreach (1,2) as $i (.; f($i); .a), foreach (3,4) as $j (.; f($j); .a)]", "[1,2,3,4]\n", "", 0),
    (r"def f($v): .a = $v; {} | reduce range(3) as $i (.; f($i)) | reduce range(2) as $j (.; f(.a + $j))", "{\"a\":3}\n", "", 0),
    (r"def f($v): .a = $v; null | reduce range(3) as $i (.; f($i))", "{\"a\":2}\n", "", 0),
    (r"def f($v): .[0] = $v; null | reduce range(3) as $i (.; f($i))", "[2]\n", "", 0),
    // jq 1.8.2 differs here (a reworded error, or `-0` printed as `0`); 1.7.1 is the reference.
    (r"def f($v): .a.b = $v; 5 | try (reduce range(3) as $i (.; f($i))) catch .", "\"Cannot index number with string \\\"a\\\"\"\n", "", 0),
    (r"def f($v): .a = $v; {} | reduce empty as $i (.; f($i))", "{}\n", "", 0),
    (r"def f($v): .a = $v; {} | [foreach empty as $i (.; f($i))]", "[]\n", "", 0),
    (r"def f($v): .a = $v; {a:0} | [recurse(f(.a + 1); .a < 4) | .a]", "[0,1,2,3]\n", "", 0),
    (r"def f($v): .a = $v; {} | [range(3) as $i | reduce range($i) as $j (.; f($j))]", "[{},{\"a\":0},{\"a\":1}]\n", "", 0),
    (r"def f($v): .a = $v; def h: reduce range(3) as $i (.; f($i)); {} | h, h", "{\"a\":2}\n{\"a\":2}\n", "", 0),
    (r#"def f($v): .a = $v; {} | getpath(["a"]) as $p | reduce range(2) as $i (.; f($i)) | [., $p]"#, "[{\"a\":1},null]\n", "", 0),
    (r"def f($v): .a = $v; {} | reduce range(3) as $i (.; f($i) | select(.a != 1))", "{\"a\":2}\n", "", 0),
    (r"def f($v): .a = $v; {} | [foreach range(3) as $i (.; f($i) | select(.a != 1); .a)]", "[0,2]\n", "", 0),
    (r"def f($v): .a = $v; {a:0} | [until(.a >= 3; f(.a + 1) | select(.a != 2))]", "[]\n", "", 0),
    (r"def f($v): .a = $v; {} | reduce range(3) as $i (.; f($i)?)", "{\"a\":2}\n", "", 0),
    (r#"def f($v): error("x"); {} | [foreach range(3) as $i (.; f($i)?; .)]"#, "[]\n", "", 0),
    (r"def f($v): .a = $v; ({} | reduce range(3) as $i (.; f($i))) as $r | $r", "{\"a\":2}\n", "", 0),
    (r"def f($v): .a = $v; {x:[range(5)]} | until(.a >= 3; f((.a // 0) + 1)) | [.a, (.x|length)]", "[3,5]\n", "", 0),
];

#[test]
fn test_fold_and_loop_steps_that_call_a_def_match_jq_7702() -> Result<()> {
    for (program, stdout, stderr, code) in CASES {
        let got = run_jq_full(&["-nc", program], None)?;
        assert_eq!(
            (got.0.as_str(), got.1.as_str(), got.2),
            (*stdout, *stderr, *code),
            "{program}"
        );
    }
    Ok(())
}
