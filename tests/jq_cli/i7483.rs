//! #7483: a call to a `def` over a value the program built bridged that value -- wrote it out as
//! JSON and indexed it again -- at every call, so a recursion that threads a state through its calls
//! (`def f($n): if $n == 0 then .a = 1 else f($n - 1) | f($n - 1) | .a += 1 end; {x: [range(k)]} |
//! f(12)`) cost time in proportion to members it never touched, where jq 1.7.1 does not. And a
//! recursive def that loops in place (#7473's `def f: if .i >= n then . else (.p["k\(.i)"] = 1 | .i
//! += 1 | f) end`) copied the growing `p` every round, because the loop kept its state while the stage
//! wrote it. The owned evaluator now runs a call, a definition, a pipe a call leads, an `if` and a
//! `$`-parameter's binding on the tree with the value handed down, so the writes land in place.
//!
//! These rows pin jq 1.7.1's captured answers (stdout, stderr, exit) over programs that lean on the
//! input being used again after a call, a pipe or a branch was handed it: several outputs of a call,
//! a branch or a closure argument, `label`, `limit`, `first`, `try`, `reduce`/`foreach`, `recurse`,
//! `walk`, `path` and `getpath`/`setpath`/`del` inside a definition, and a state bound to a variable
//! and read after a write. jq 1.8.2 answers the same except where it rewords an error (marked). The
//! in-place proof is `def_call_recursion_writes_the_state_in_place_7483` in
//! `src/jq/eval/owned_call.rs`.

use anyhow::Result;

use crate::run_jq_full;

/// (program, stdout, stderr, exit code) for `succinctly jq -nc <program>`.
#[allow(clippy::literal_string_with_formatting_args)] // jq object literals read like format arguments
const CASES: &[(&str, &str, &str, i32)] = &[
    (r"def f($n): if $n==0 then .a = 1 else f($n-1) | f($n-1) | .a += 1 end; {x:[range(10)]} | f(4)", "{\"x\":[0,1,2,3,4,5,6,7,8,9],\"a\":5}\n", "", 0),
    (r#"def f: if .i>=50 then . else (.p["k\(.i)"]=1 | .i += 1 | f) end; {i:0,p:{}} | f | [.i, (.p|length)]"#, "[50,50]\n", "", 0),
    (r"def f($n): if $n==0 then .a = 1 else f($n-1) | .a as $p | f($n-1) | .a = $p + .a end; {x:[1]} | f(3)", "{\"x\":[1],\"a\":8}\n", "", 0),
    (r"def f: .a = 1, .a = 2; {a:0} | [f]", "[{\"a\":1},{\"a\":2}]\n", "", 0),
    (r"def f: (.a = 1), .; {a:0} | [f, .]", "[{\"a\":1},{\"a\":0},{\"a\":0}]\n", "", 0),
    (r"def f: if .a then .b = 1 else .c = 2 end; ({a:true}|f), ({a:false}|f)", "{\"a\":true,\"b\":1}\n{\"a\":false,\"c\":2}\n", "", 0),
    (r"def f(g): g | .z = 1; {a:1} | [f(.a = 2, .a = 3)]", "[{\"a\":2,\"z\":1},{\"a\":3,\"z\":1}]\n", "", 0),
    (r"def f: .a += 1; {a:0} | [limit(3; repeat(f))]", "[{\"a\":1},{\"a\":1},{\"a\":1}]\n", "", 0),
    (r"[{a:0} | recurse(if .a < 3 then .a += 1 else empty end)]", "[{\"a\":0},{\"a\":1},{\"a\":2},{\"a\":3}]\n", "", 0),
    (r#"{a:[1,{b:2}]} | walk(if type == "number" then . + 1 else . end)"#, "{\"a\":[2,{\"b\":3}]}\n", "", 0),
    (r"def f: reduce range(3) as $i (.; .a += $i); {a:0} | f, f", "{\"a\":3}\n{\"a\":3}\n", "", 0),
    (r"def f: [foreach range(3) as $i (.; .a += $i)]; {a:0} | f", "[{\"a\":0},{\"a\":1},{\"a\":3}]\n", "", 0),
    (r"def f: label $out | (.a = 1 | ., break $out), .a = 2; {a:0} | [f]", "[{\"a\":1}]\n", "", 0),
    (r#"def f: try (.a = error("x")) catch .; {a:0} | f"#, "\"x\"\n", "", 0),
    (r"def f: . as $s | .a = 1 | [$s, .]; {a:0} | f", "[{\"a\":0},{\"a\":1}]\n", "", 0),
    (r"def f($v): .a = $v; {a:0} | . as $o | f(1) | [$o, .]", "[{\"a\":0},{\"a\":1}]\n", "", 0),
    (r"def f: path(.a, .b); {a:1} | [f]", "[[\"a\"],[\"b\"]]\n", "", 0),
    (r#"def f: getpath(["a"]), setpath(["a"]; 9), del(.a); {a:1,b:2} | [f]"#, "[1,{\"a\":9,\"b\":2},{\"b\":2}]\n", "", 0),
    (r"def f: if .n == 0 then . else (.n -= 1 | .l += [.n] | f) end; {n:5, l:[]} | f", "{\"n\":0,\"l\":[4,3,2,1,0]}\n", "", 0),
    (r"def f: if .n <= 0 then . else (.n -= 1 | f), (.n -= 2 | f) end; {n:3} | [f | .n]", "[0,-1,0,0,-1]\n", "", 0),
    (r"def f: .a = 1 | .b = 2; {x:1} | f, .", "{\"x\":1,\"a\":1,\"b\":2}\n{\"x\":1}\n", "", 0),
    (r"def g: .a = 1; {x:1} | [g, g, .]", "[{\"x\":1,\"a\":1},{\"x\":1,\"a\":1},{\"x\":1}]\n", "", 0),
    (r"def f($n): if $n == 0 then . else (f($n-1) | .c += 1), (f($n-1) | .d += 1) end; {c:0,d:0} | [f(2)]", "[{\"c\":2,\"d\":0},{\"c\":1,\"d\":1},{\"c\":1,\"d\":1},{\"c\":0,\"d\":2}]\n", "", 0),
    (r"def f: (.a,.b) |= . + 1; {a:1,b:2} | f", "{\"a\":2,\"b\":3}\n", "", 0),
    (r"def f: .a[0] = 9; {a:[1,2]} as $o | $o | f | [., $o]", "[{\"a\":[9,2]},{\"a\":[1,2]}]\n", "", 0),
    (r"def f: .[0] = 9; [1,2] as $o | $o | f | [., $o]", "[[9,2],[1,2]]\n", "", 0),
    (r"def f: .x = 1; [{}, {}] | map(f)", "[{\"x\":1},{\"x\":1}]\n", "", 0),
    (r"def f: first(.a = (1,2)); {} | f", "{\"a\":1}\n", "", 0),
    (r"def f: .a = (1,2); {} | [f]", "[{\"a\":1},{\"a\":2}]\n", "", 0),
    (r"def f: select(.a > 0) | .b = 1; [{a:1},{a:0}] | map(f)", "[{\"a\":1,\"b\":1}]\n", "", 0),
    (r"def f: .a |= empty; {a:1,b:2} | f", "{\"b\":2}\n", "", 0),
    (r"def f: if . then .a = 1 elif .b then .c = 1 else empty end; {b:1} | f", "{\"b\":1,\"a\":1}\n", "", 0),
    (r"def f($n): if $n > 0 then f($n - 1) | .k += [$n] else . end; {k:[]} | f(5)", "{\"k\":[1,2,3,4,5]}\n", "", 0),
    // jq 1.8.2 rewords this error; 1.7.1 is the reference.
    (r"def f: .a.b = 1; {a:5} | try f catch .", "\"Cannot index number with string \\\"b\\\"\"\n", "", 0),
    (r"def f: .a = 1; null | f", "{\"a\":1}\n", "", 0),
    (r"def f: .l = $__loc__; {} | f", "{\"l\":{\"file\":\"<top-level>\",\"line\":1}}\n", "", 0),
    (r"def f: .a = 1 | debug | .b = 2; {} | f", "{\"a\":1,\"b\":2}\n", "[\"DEBUG:\",{\"a\":1}]\n", 0),
    (r"def f: limit(2; .a = (1,2,3)); {} | [f]", "[{\"a\":1},{\"a\":2}]\n", "", 0),
    (r"def f: ., (.a = 1 | f); {} | [limit(3; f)]", "[{},{\"a\":1},{\"a\":1}]\n", "", 0),
    (r#"def f: getpath(["a","b"]) as $x | setpath(["a","b"]; $x + 1); {a:{b:1}} | f | f"#, "{\"a\":{\"b\":3}}\n", "", 0),
    (r"def f(x): x as $v | .a = $v; {} | [f(1,2)]", "[{\"a\":1},{\"a\":2}]\n", "", 0),
    (r"def f: .; {a:1} as $o | $o | f | .a = 2 | [., $o]", "[{\"a\":2},{\"a\":1}]\n", "", 0),
    (r"def f: .a = 1; {a:0} | [f, f] | .[0] == .[1]", "true\n", "", 0),
    (r"def f: . as [$a] ?// $a | .; [1] | f", "[1]\n", "", 0),
    (r"def f: to_entries; {a:1} | f", "[{\"key\":\"a\",\"value\":1}]\n", "", 0),
    (r"def f: .[] |= . + 1; [1,2] | f", "[2,3]\n", "", 0),
    (r"def f($n): if $n == 0 then .a = 1 else (f($n-1) | .a += 1) as $r | $r end; {} | f(3)", "{\"a\":4}\n", "", 0),
    (r"def f: if .a then .a = false | f else .done = true end; {a:true} | f", "{\"a\":false,\"done\":true}\n", "", 0),
    (r#"def f: error("boom"); {} | try f catch ."#, "\"boom\"\n", "", 0),
    (r"def f: if .a < 3 then .a += 1 | f else . end; {a:0} | f, .", "{\"a\":3}\n{\"a\":0}\n", "", 0),
    (r"def f: if .a < 3 then .a += 1 | f else . end; {a:0} as $o | $o | f | [., $o]", "[{\"a\":3},{\"a\":0}]\n", "", 0),
    (r"def f($n): if $n == 0 then .a = 1 else f($n-1) | f($n-1) | .a += 1 end; [{x:1}] | map(f(2))", "[{\"x\":1,\"a\":3}]\n", "", 0),
    (r#"def f: if .a then error("x") else . end; {a:1} | try f catch "caught""#, "\"caught\"\n", "", 0),
    (r"def f: if (.a, .b) then .c = 1 else .d = 1 end; {a:true,b:false} | [f]", "[{\"a\":true,\"b\":false,\"c\":1},{\"a\":true,\"b\":false,\"d\":1}]\n", "", 0),
    (r"def f: if .a > 1 then .z = 1 else .z = 2 end; [{a:1},{a:2}] | map(f)", "[{\"a\":1,\"z\":2},{\"a\":2,\"z\":1}]\n", "", 0),
    (r"def f: .a = 1; {b:{c:[1,2,3]}} | f | .b.c[1] = 9 | .", "{\"b\":{\"c\":[1,9,3]},\"a\":1}\n", "", 0),
    (r#"def f($k): .[$k] = 1; {} | f("x") | f("y") | f("x")"#, "{\"x\":1,\"y\":1}\n", "", 0),
    (r"def f: .a = 1; {} | f as $x | {} | f | [$x, .]", "[{\"a\":1},{\"a\":1}]\n", "", 0),
    (r"def f: if .i < 3 then .i += 1 | .l += [.i] | f else . end; {i:0,l:[]} | [f, .]", "[{\"i\":3,\"l\":[1,2,3]},{\"i\":0,\"l\":[]}]\n", "", 0),
    (r"def f: if .i < 3 then (.i += 1 | f), . else . end; {i:0} | [f | .i]", "[3,2,1,0]\n", "", 0),
    (r"def f: .a = 1; {x:[1,2]} | [f, (.x[0] = 5)]", "[{\"x\":[1,2],\"a\":1},{\"x\":[5,2]}]\n", "", 0),
    (r"def f: .x[0] = 5; {x:[1,2]} | [., f, .]", "[{\"x\":[1,2]},{\"x\":[5,2]},{\"x\":[1,2]}]\n", "", 0),
    (r"def f($n): if $n == 0 then .v = 1 else f($n-1) | f($n-1) | .v += 1 end; {v:0} | [f(2), .]", "[{\"v\":3},{\"v\":0}]\n", "", 0),
    (r"def f: if .a then .a = null | f else . end; {a:1} | [f, f]", "[{\"a\":null},{\"a\":null}]\n", "", 0),
    (r"def f: .a = 1 | .b = 2 | .c = 3; {} | [f, f]", "[{\"a\":1,\"b\":2,\"c\":3},{\"a\":1,\"b\":2,\"c\":3}]\n", "", 0),
    (r"def f(g): if g then .t = 1 else .t = 0 end; {} | [f(true, false)]", "[{\"t\":1},{\"t\":0}]\n", "", 0),
    (r"def f: if .a == null then .a = 0 end; {} | f", "{\"a\":0}\n", "", 0),
    (r"def f: def g: .a += 1; g | g; {a:0} | f", "{\"a\":2}\n", "", 0),
    (r"def f($n): if $n == 0 then . else .s += [$n] | f($n - 1) end; {s:[]} | f(3), .", "{\"s\":[3,2,1]}\n{\"s\":[]}\n", "", 0),
    // jq 1.8.2 rewords this error; 1.7.1 is the reference.
    (r"def f: reduce .[] as $x (.; .[$x|tostring] = $x); [1,2] | f", "", "jq: error (at <unknown>): Cannot index array with string \"1\"\n", 5),
    (r"def f: (.a = 1) as $x | $x | .b = 2; {} | f", "{\"a\":1,\"b\":2}\n", "", 0),
    (r"def f: .a = 1; {} | [f] | .[0].b = 2", "[{\"a\":1,\"b\":2}]\n", "", 0),
    (r"def f: if .n > 0 then .n -= 1 | f | .r += 1 else .r = 0 end; {n:4} | f", "{\"n\":0,\"r\":4}\n", "", 0),
    (r"def f: if .n > 0 then .n -= 1 | f | .r += 1 else .r = 0 end; {n:4} | [f, .]", "[{\"n\":0,\"r\":4},{\"n\":4}]\n", "", 0),
    (r"def f: .a = (1,2) | .b = (3,4); {} | [f]", "[{\"a\":1,\"b\":3},{\"a\":1,\"b\":4},{\"a\":2,\"b\":3},{\"a\":2,\"b\":4}]\n", "", 0),
    (r"def f($n): if $n == 0 then .a = (1,2) else f($n-1) | .b += 1 end; {b:0} | [f(2)]", "[{\"b\":2,\"a\":1},{\"b\":2,\"a\":2}]\n", "", 0),
    (r"def f($n): if $n == 0 then (.a = 1), (.a = 2) else f($n-1) | f($n-1) | .c += 1 end; {c:0} | [f(2) | [.a,.c]]", "[[1,3],[2,3],[1,3],[2,3],[1,3],[2,3],[1,3],[2,3],[1,3],[2,3],[1,3],[2,3],[1,3],[2,3],[1,3],[2,3]]\n", "", 0),
    (r"def f: (.a = 1 | ., (.b = 2)), (.c = 3); {} | [f]", "[{\"a\":1},{\"a\":1,\"b\":2},{\"c\":3}]\n", "", 0),
    (r"def f: if .x then (.y = 1), (.y = 2) else . end; {x:true} | [f | .y]", "[1,2]\n", "", 0),
    (r"def f: first((.a = 1), (.a = 2)); {} | f", "{\"a\":1}\n", "", 0),
    (r#"def f: limit(1; (.a = 1), error("no")); {} | [f]"#, "[{\"a\":1}]\n", "", 0),
    (r#"def f: isempty((.a = 1), error("no")); {} | f"#, "false\n", "", 0),
    (r#"def f: try ((.a = 1), error("e")) catch "c"; {} | [f]"#, "[{\"a\":1},\"c\"]\n", "", 0),
    (r#"def f: (.a = 1), error("e"); {} | [f?]"#, "[{\"a\":1}]\n", "", 0),
    (r#"def f: (.a = 1), error("e"); {} | [try f catch .]"#, "[{\"a\":1},\"e\"]\n", "", 0),
    (r"def f: .a = 1 | error(.); {} | try f catch .", "{\"a\":1}\n", "", 0),
    (r"def f: label $l | (.a = 1), break $l, (.a = 2); {} | [f]", "[{\"a\":1}]\n", "", 0),
    (r"def f($n): label $l | if $n == 0 then ., break $l else f($n - 1) | .d += 1 end; {d:0} | [f(3)]", "[{\"d\":3}]\n", "", 0),
    (r"def f: [paths]; {a:{b:1}} | .a.c = 2 | f", "[[\"a\"],[\"a\",\"b\"],[\"a\",\"c\"]]\n", "", 0),
    (r"def f: path(.a | .b); {} | [f]", "[[\"a\",\"b\"]]\n", "", 0),
    (r"def g: .a; def f: path(g); {a:1} | f", "[\"a\"]\n", "", 0),
    (r"def f: .a; {a:1} | [path(f)]", "[[\"a\"]]\n", "", 0),
    (r"def f: .a; {a:{b:1}} | f |= 5", "{\"a\":5}\n", "", 0),
    (r"def f: .a; {a:{b:1}} | del(f)", "{}\n", "", 0),
    (r"def f: .a; {a:1} | (f, .b) = 9", "{\"a\":9,\"b\":9}\n", "", 0),
    (r"def f($v): .a = $v; {} | [f(1,2)]", "[{\"a\":1},{\"a\":2}]\n", "", 0),
    (r"def f(g): [g]; {a:1} | f(.a, (.a = 2 | .a))", "[1,2]\n", "", 0),
    (r"def f(g): g | g; {a:1} | f(.a += 1)", "{\"a\":3}\n", "", 0),
    (r"def f(g): .a = 1 | g; {} | f(.a, .a = 2)", "1\n{\"a\":2}\n", "", 0),
    (r"def f(g): reduce g as $x (.; .s += $x); {s:0} | f(1,2,3)", "{\"s\":6}\n", "", 0),
    (r"def f(g): [foreach g as $x (.; .s += $x; .s)]; {s:0} | f(1,2,3)", "[1,3,6]\n", "", 0),
    (r"def f: recurse(if .n < 3 then .n += 1 else empty end); {n:0} | [f | .n]", "[0,1,2,3]\n", "", 0),
    (r#"def f: walk(if type == "object" then .k = 1 else . end); {a:{b:{}}} | f"#, "{\"a\":{\"b\":{\"k\":1},\"k\":1},\"k\":1}\n", "", 0),
    (r"def f: to_entries | map(.value += 1) | from_entries; {a:1,b:2} | f", "{\"a\":2,\"b\":3}\n", "", 0),
    (r"def f: with_entries(.value += 1); {a:1} | f", "{\"a\":2}\n", "", 0),
    (r"def f: .[] += 1; {a:1,b:2} | f, .", "{\"a\":2,\"b\":3}\n{\"a\":1,\"b\":2}\n", "", 0),
    (r"def f: map(.a = 1); [{},{}] | f, .", "[{\"a\":1},{\"a\":1}]\n[{},{}]\n", "", 0),
    (r"def f: map_values(. + 1); {a:1} | f", "{\"a\":2}\n", "", 0),
    (r"def f: .a as $a | .a = $a + 1 | .b = $a; {a:1} | f", "{\"a\":2,\"b\":1}\n", "", 0),
    (r"def f: . as {a:$a} | .a = $a + 1; {a:1} | f", "{\"a\":2}\n", "", 0),
    (r"def f: . as [$x, $y] | [$y, $x]; [1,2] | f", "[2,1]\n", "", 0),
    (r"def f: . as $o | ($o | .a = 2) as $p | [$o, $p]; {a:1} | f", "[{\"a\":1},{\"a\":2}]\n", "", 0),
    (r#"def f: getpath(["a"]) as $v | setpath(["b"]; $v) | delpaths([["a"]]); {a:1} | f"#, "{\"b\":1}\n", "", 0),
    (r"def f: .a // (.a = 5); {} | f, ({a:1}|f)", "{\"a\":5}\n1\n", "", 0),
    (r"def f: (.a // 0) as $v | .a = $v + 1; {} | f | f", "{\"a\":2}\n", "", 0),
    (r"def f: .a //= 3; {} | f, ({a:false}|f)", "{\"a\":3}\n{\"a\":3}\n", "", 0),
    (r"def f: .a |= (. // 0) + 1; {} | f | f", "{\"a\":2}\n", "", 0),
    (r"def f: if .a then .a else empty end; {a:1}, {a:null} | f", "1\n", "", 0),
    (r"def f: select(.a) | .b = 1; ({a:1},{a:false}) | [f]", "[{\"a\":1,\"b\":1}]\n[]\n", "", 0),
    (r"def f: .a = input; {} | f", "", "jq: error (at <stdin>:0): break\n", 5),
    (r"def f: .l = input_line_number; {} | f", "{\"l\":0}\n", "", 0),
    (r"def f: .s = ([.[]?] | length); {a:1,b:2} | f", "{\"a\":1,\"b\":2,\"s\":2}\n", "", 0),
    (r"def f: .[0] = 1; null | f", "[1]\n", "", 0),
    (r"def f: .[2] = 1; [] | f", "[null,null,1]\n", "", 0),
    (r"def f: .[-1] = 9; [1,2] | f", "[1,9]\n", "", 0),
    (r#"def f: .[1:] = ["x"]; [1,2,3] | f"#, "[1,\"x\"]\n", "", 0),
    (r#"def f: .a[1:2] |= ["y"]; {a:[1,2,3]} | f"#, "{\"a\":[1,\"y\",3]}\n", "", 0),
    (r"def f($n): if $n == 0 then . else (.v[$n] = $n | f($n-1)) end; {v:[]} | f(3)", "{\"v\":[null,1,2,3]}\n", "", 0),
    (r"def f($n): if $n == 0 then . else f($n-1) | .v += [$n] end; {v:[]} | f(4) | .v", "[1,2,3,4]\n", "", 0),
    (r"def f($n): if $n == 0 then . else f($n-1) as $r | $r | .v += [$n] end; {v:[]} | f(4) | .v", "[1,2,3,4]\n", "", 0),
    (r#"def f($n): if $n == 0 then {} else f($n-1) | .["k\($n)"] = $n end; f(3)"#, "{\"k1\":1,\"k2\":2,\"k3\":3}\n", "", 0),
    (r"def f(n): if n == 0 then . else .v += 1 | f(n - 1) end; {v:0} | f(5)", "{\"v\":5}\n", "", 0),
    (r"def f(n): if n == 0 then . else f(n - 1) | .v += 1 end; {v:0} | f(5)", "{\"v\":5}\n", "", 0),
    (r"def f: def g: .a += 1; if .a < 3 then g | f else . end; {a:0} | f", "{\"a\":3}\n", "", 0),
    (r"def fib: if . < 2 then . else (. - 1 | fib) + (. - 2 | fib) end; [range(10) | fib]", "[0,1,1,2,3,5,8,13,21,34]\n", "", 0),
    (r"def fac: if . <= 1 then 1 else . * (. - 1 | fac) end; 10 | fac", "3628800\n", "", 0),
    (r"def f: if .a > 0 then .a -= 1 | f | .c += 1 else .c = 0 end; {a:3} | f, ., f", "{\"a\":0,\"c\":3}\n{\"a\":3}\n{\"a\":0,\"c\":3}\n", "", 0),
    (r"def f: .a = 1; def g: f | .b = 2; {} | g, g | .", "{\"a\":1,\"b\":2}\n{\"a\":1,\"b\":2}\n", "", 0),
    (r"def f: .a = 1; {} | [f, f, f] | length", "3\n", "", 0),
    (r"def f($x): $x; {a:1} | f(.a)", "1\n", "", 0),
    (r"def f($x): .b = $x; {a:1} | f(.a)", "{\"a\":1,\"b\":1}\n", "", 0),
    (r"def f($x): .b = $x; {a:{c:1}} | f(.a) | .b.c = 2 | .", "{\"a\":{\"c\":1},\"b\":{\"c\":2}}\n", "", 0),
    (r"def f($x): $x | .c = 2; {a:{c:1}} | [f(.a), .]", "[{\"c\":2},{\"a\":{\"c\":1}}]\n", "", 0),
    (r"def f($x; $y): .a = $x + $y; {} | f(1; 2)", "{\"a\":3}\n", "", 0),
    (r"def f($x; $y): .a = $x + $y; {} | [f(1, 2; 3, 4)]", "[{\"a\":4},{\"a\":5},{\"a\":5},{\"a\":6}]\n", "", 0),
    (r"def f: if . == {} then .a = 1 else . end; {} | f", "{\"a\":1}\n", "", 0),
    (r"def f: if length > 0 then .z = 1 else . end; {a:1} | f, ({} | f)", "{\"a\":1,\"z\":1}\n{}\n", "", 0),
    (r#"def f: if has("a") then del(.a) else .a = 1 end; {a:1} | f | f"#, "{\"a\":1}\n", "", 0),
    (r#"def f: if (.a | type) == "number" then .a += 1 else . end; {a:1} | f"#, "{\"a\":2}\n", "", 0),
    (r"def f: if .a then .b = 1 end; {a:true} | f", "{\"a\":true,\"b\":1}\n", "", 0),
    (r"def f: if .a then .b = 1 elif .c then .d = 1 end; {c:true} | f", "{\"c\":true,\"d\":1}\n", "", 0),
    (r#"def f: if .a? then .b = 1 else .c = 1 end; [] | try f catch "e""#, "", "", 0),
    // jq 1.8.2 rewords this error; 1.7.1 is the reference.
    (r"def f: if .a then .b = 1 else .c = 1 end; [] | try f catch .", "\"Cannot index array with string \\\"a\\\"\"\n", "", 0),
    (r"def f: if .a then .b = 1 else .c = 1 end; [] | f?", "", "", 0),
    (r"def f: (if .a then .b = 1 else .c = 1 end)?; [] | [f]", "[]\n", "", 0),
];

#[test]
fn test_def_calls_over_a_built_value_match_jq_7483() -> Result<()> {
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

/// The issue's two programs at a size where the old route's cost shows even in a debug build: a
/// 20,000-element member the calls never touch, and 2,000 rounds of a growing object. Pinned by
/// output only; the timing is the unit test's reindex and copy counts.
#[test]
fn test_issue_programs_answer_with_a_large_untouched_member_7483() -> Result<()> {
    let got = run_jq_full(
        &[
            "-nc",
            "def f($n): if $n == 0 then .a = 1 else f($n - 1) | f($n - 1) | .a += 1 end; {x: [range(20000)]} | f(8) | [.a, (.x | length)]",
        ],
        None,
    )?;
    assert_eq!(got, ("[9,20000]\n".to_string(), String::new(), 0));
    let got = run_jq_full(
        &[
            "-nc",
            r#"def f: if .i >= 2000 then . else (.p["k\(.i)"] = 1 | .i += 1 | f) end; {i: 0, p: {}} | f | [.i, (.p | length)]"#,
        ],
        None,
    )?;
    assert_eq!(got, ("[2000,2000]\n".to_string(), String::new(), 0));
    Ok(())
}
