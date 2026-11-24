//! #7870: a closure argument that adds a `,` at each level of a recursion (`f(g + (1, 3); $n - 1)`)
//! was read eagerly, every output at once, because each link is pure and each `,` is over constants.
//! Its output count doubles at each level, so `first(f((1, 3); 500))` built `2^500` sums before `first`
//! saw one and the process aborted on a failed allocation, where jq 1.7.1 and 1.8.2 print `501` at
//! once. An argument is now eager only while its output count, the product of every `,` and of every
//! argument it reads, cannot grow from level to level (`eval::is_eager_arg`); past that a link of a
//! chain is read one output at a time, as jq reads it, and any other pure argument is read ahead by a
//! bounded number of outputs.
//!
//! These rows pin jq 1.7.1's captured answers (stdout, stderr, exit); jq 1.8.2 answers every one of
//! them the same. The first rows are the ones that aborted or ran out of time. The rest are what a
//! change to how an argument is read could get wrong: every output in jq's order, an error raised
//! after the outputs before it and not earlier, a `debug` or an `input` in the chain run as often as
//! jq runs it and in its order, a chain that keeps its width (still eager), and paths and updates
//! through a chain. The last group is arguments read more than once (`REREAD_CASES`).

use anyhow::Result;

use crate::run_jq_full;

/// (program, stdout, stderr, exit code) for `succinctly jq -nc <program>` with no input.
#[allow(clippy::literal_string_with_formatting_args)] // jq object literals read like format arguments
const NULL_INPUT_CASES: &[(&str, &str, &str, i32)] = &[
    (r"def f(g; $n): if $n <= 0 then g else f(g + (1,3); $n - 1) end; first(f((1,3); 500))", "501\n", "", 0),
    (r"def f(g; n): if n == 0 then g else f(g + (1,3); n - 1) end; first(f((1,3); 500))", "501\n", "", 0),
    (r"def f(g; $n): if $n <= 0 then g else f(g + (1,3,5); $n - 1) end; first(f((1,3,5); 500))", "501\n", "", 0),
    (r"def f(g; $n): if $n <= 0 then g else f(g | . + (1,3); $n - 1) end; first(f((1,3); 500))", "501\n", "", 0),
    (r"def f(g; $n): if $n <= 0 then g else f((1,3) + g; $n - 1) end; first(f((1,3); 500))", "501\n", "", 0),
    (r"def f(g; $n): if $n <= 0 then g else f(g + (1,3); $n - 1) end; [limit(5; f((1,3); 500))]", "[501,503,503,505,503]\n", "", 0),
    (r"def f(g; $n): if $n <= 0 then g else f(g + (1,3); $n - 1) end; isempty(f((1,3); 300))", "false\n", "", 0),
    (r"def f(g; $n): if $n <= 0 then g else f(g + (1,3); $n - 1) end; label $out | f((1,3); 300) | ., break $out", "301\n", "", 0),
    (r"def f(g; $n): if $n <= 0 then g else f(g + (1,3); $n - 1) end; first(f((1,3); 40) | select(. > 45))", "47\n", "", 0),
    (r#"def f(g; $n): if $n <= 0 then g else f(g + (1,3); $n - 1) end; first(f((1,"a"); 300))"#, "301\n", "", 0),
    (r"def h(x): x + x; def f(g; $n): if $n <= 0 then g else f(h(g); $n - 1) end; [limit(3; f((1,3); 5))]", "[32,34,34]\n", "", 0),
    (r"def h($x): $x + (1,3); def f(g; $n): if $n <= 0 then g else f(h(g); $n - 1) end; first(f((1,3); 300))", "301\n", "", 0),
    (r"def r(x; $k): if $k <= 0 then x else r(x + (1,3); $k - 1) end; def f(g): first(g); f(r((1,3); 300))", "301\n", "", 0),
    (r"def f(g; $n): if $n <= 0 then g else f(g as $x | ($x, 7); $n - 1) end; first(f((1,3); 300))", "1\n", "", 0),
    (r"def f(g; $n): if $n <= 0 then g else f(if g > 2 then (g, 0) else (1, g) end; $n - 1) end; first(f((1,3); 12))", "1\n", "", 0),
    (r"def f(g; $n): if $n <= 0 then g else f(g + (1,3); $n - 1) end; [f((1,3); 4)]", "[5,7,7,9,7,9,9,11,7,9,9,11,9,11,11,13,7,9,9,11,9,11,11,13,9,11,11,13,11,13,13,15]\n", "", 0),
    (r"def f(g; $n): if $n <= 0 then g else f(g + (1,3); $n - 1) end; [f((1,3); 10)] | length, add", "2048\n45056\n", "", 0),
    (r"def f(g; $n): if $n <= 0 then g else f(g + (1,3,5); $n - 1) end; [f((1,3,5); 2)]", "[3,5,7,5,7,9,7,9,11,5,7,9,7,9,11,9,11,13,7,9,11,9,11,13,11,13,15]\n", "", 0),
    (r"def f(g; $n): if $n <= 0 then g else f(g | . + (1,3); $n - 1) end; [f((1,3); 3)]", "[4,6,6,8,6,8,8,10,6,8,8,10,8,10,10,12]\n", "", 0),
    (r"def f(g; $n): if $n <= 0 then g else f((1,3) + g; $n - 1) end; [f((1,3); 3)]", "[4,6,6,8,6,8,8,10,6,8,8,10,8,10,10,12]\n", "", 0),
    (r"def f(g; $n): if $n <= 0 then g else f(g * (1,2) - (0,1); $n - 1) end; [f((1,3); 2)]", "[1,3,2,6,0,2,1,5,2,6,4,12,0,4,2,10,0,2,1,5,-1,1,0,4,1,5,3,11,-1,3,1,9]\n", "", 0),
    (r"def f(g; $n): if $n <= 0 then g else f(if g > 2 then (g, 0) else (1, g) end; $n - 1) end; [f((1,3); 3)]", "[1,1,1,1,3,1,3,0,1,1,1,3,1,3,0,1,1,3,1,3,0,0,1,1,1,3,1,3,0,1,1,3,1,3,0,0,1,1,1,3,1,3,0,1,1,1,1,3,1,3,0,1,1,1,3,1,3,0,1,1,3,1,3,0,0,1,1,1,3,1,3,0,1,1,3,1,3,0,0,1,1,1,3,1,3,0,1,1,1,1,3,1,3,0,1,1,1,3,1,3,0,1,1,3,1,3,0,0,1,1,1,3,1,3,0,1,1,3,1,3,0,0,1,1,1,3,1,3,0,1,1,1,3,1,3,0,1,1,1,3,1,3,0,1,1,3,1,3,0,0,1,1,1,3,1,3,0,1,1,3,1,3,0,0,1,1,1,3,1,3,0,0,1,1,1,1,3,1,3,0,1,1,1,3,1,3,0,1,1,3,1,3,0,0,1,1,1,3,1,3,0,1,1,3,1,3,0,0,1,1,1,3,1,3,0,1,1,1,3,1,3,0,1,1,1,3,1,3,0,1,1,3,1,3,0,0,1,1,1,3,1,3,0,1,1,3,1,3,0,0,1,1,1,3,1,3,0,0,1,1,1,1,3,1,3,0,1,1,1,3,1,3,0,1,1,3,1,3,0,0,1,1,1,3,1,3,0,1,1,3,1,3,0,0,1,1,1,3,1,3,0,1,1,1,1,3,1,3,0,1,1,1,3,1,3,0,1,1,3,1,3,0,0,1,1,1,3,1,3,0,1,1,3,1,3,0,0,1,1,1,3,1,3,0,1,1,1,1,3,1,3,0,1,1,1,3,1,3,0,1,1,3,1,3,0,0,1,1,1,3,1,3,0,1,1,3,1,3,0,0,1,1,1,3,1,3,0,1,1,1,1,3,1,3,0,1,1,1,3,1,3,0,1,1,3,1,3,0,0,1,1,1,3,1,3,0,1,1,3,1,3,0,0,1,1,1,3,1,3,0,1,1,1,3,1,3,0,1,1,1,3,1,3,0,1,1,3,1,3,0,0,1,1,1,3,1,3,0,1,1,3,1,3,0,0,1,1,1,3,1,3,0,0,1,1,1,1,3,1,3,0,1,1,1,3,1,3,0,1,1,3,1,3,0,0,1,1,1,3,1,3,0,1,1,3,1,3,0,0,1,1,1,3,1,3,0,1,1,1,3,1,3,0,1,1,1,3,1,3,0,1,1,3,1,3,0,0,1,1,1,3,1,3,0,1,1,3,1,3,0,0,1,1,1,3,1,3,0,0,1,1,1,1,3,1,3,0,1,1,1,3,1,3,0,1,1,3,1,3,0,0,1,1,1,3,1,3,0,1,1,3,1,3,0,0,1,1,1,3,1,3,0,1,1,1,1,3,1,3,0,1,1,1,3,1,3,0,1,1,3,1,3,0,0,1,1,1,3,1,3,0,1,1,3,1,3,0,0,1,1,1,3,1,3,0,1,1,1,1,3,1,3,0,1,1,1,3,1,3,0,1,1,3,1,3,0,0,1,1,1,3,1,3,0,1,1,3,1,3,0,0,1,1,1,3,1,3,0,1,1,1,3,1,3,0,1,1,1,3,1,3,0,1,1,3,1,3,0,0,1,1,1,3,1,3,0,1,1,3,1,3,0,0,1,1,1,3,1,3,0,0,1,1,1,1,3,1,3,0,1,1,1,3,1,3,0,1,1,3,1,3,0,0,1,1,1,3,1,3,0,1,1,3,1,3,0,0,1,1,1,3,1,3,0,1,1,1,3,1,3,0,1,1,1,3,1,3,0,1,1,3,1,3,0,0,1,1,1,3,1,3,0,1,1,3,1,3,0,0,1,1,1,3,1,3,0,0,1,1,1,1,3,1,3,0,1,1,1,3,1,3,0,1,1,3,1,3,0,0,1,1,1,3,1,3,0,1,1,3,1,3,0,0,1,1,1,3,1,3,0,1,1,1,1,3,1,3,0,1,1,1,3,1,3,0,1,1,3,1,3,0,0,1,1,1,3,1,3,0,1,1,3,1,3,0,0,1,1,1,3,1,3,0,1,1,1,1,3,1,3,0,1,1,1,3,1,3,0,1,1,3,1,3,0,0,1,1,1,3,1,3,0,1,1,3,1,3,0,0,1,1,1,3,1,3,0,1,1,1,1,3,1,3,0,1,1,1,3,1,3,0,1,1,3,1,3,0,0,1,1,1,3,1,3,0,1,1,3,1,3,0,0,1,1,1,3,1,3,0,1,1,1,1,3,1,3,0,1,1,1,3,1,3,0,1,1,3,1,3,0,0,1,1,1,3,1,3,0,1,1,3,1,3,0,0,1,1,1,3,1,3,0,1,1,1,3,1,3,0,1,1,1,3,1,3,0,1,1,3,1,3,0,0,1,1,1,3,1,3,0,1,1,3,1,3,0,0,1,1,1,3,1,3,0,0,1,1,1,1,3,1,3,0,1,1,1,3,1,3,0,1,1,3,1,3,0,0,1,1,1,3,1,3,0,1,1,3,1,3,0,0,1,1,1,3,1,3,0,1,1,1,3,1,3,0,1,1,1,3,1,3,0,1,1,3,1,3,0,0,1,1,1,3,1,3,0,1,1,3,1,3,0,0,1,1,1,3,1,3,0,0,1,1,1,1,3,1,3,0,1,1,1,3,1,3,0,1,1,3,1,3,0,0,1,1,1,3,1,3,0,1,1,3,1,3,0,0,1,1,1,3,1,3,0,1,1,1,1,3,1,3,0,1,1,1,3,1,3,0,1,1,3,1,3,0,0,1,1,1,3,1,3,0,1,1,3,1,3,0,0,1,1,1,3,1,3,0,1,1,1,1,3,1,3,0,1,1,1,3,1,3,0,1,1,3,1,3,0,0,1,1,1,3,1,3,0,1,1,3,1,3,0,0,1,1,1,3,1,3,0,1,1,1,3,1,3,0,1,1,1,3,1,3,0,1,1,3,1,3,0,0,1,1,1,3,1,3,0,1,1,3,1,3,0,0,1,1,1,3,1,3,0,0,1,1,1,1,3,1,3,0,1,1,1,3,1,3,0,1,1,3,1,3,0,0,1,1,1,3,1,3,0,1,1,3,1,3,0,0,1,1,1,3,1,3,0,1,1,1,3,1,3,0,1,1,1,3,1,3,0,1,1,3,1,3,0,0,1,1,1,3,1,3,0,1,1,3,1,3,0,0,1,1,1,3,1,3,0,0,1,1,1,1,3,1,3,0,1,1,1,3,1,3,0,1,1,3,1,3,0,0,1,1,1,3,1,3,0,1,1,3,1,3,0,0,1,1,1,3,1,3,0,1,1,1,1,3,1,3,0,1,1,1,3,1,3,0,1,1,3,1,3,0,0,1,1,1,3,1,3,0,1,1,3,1,3,0,0,1,1,1,3,1,3,0,1,1,1,1,3,1,3,0,1,1,1,3,1,3,0,1,1,3,1,3,0,0,1,1,1,3,1,3,0,1,1,3,1,3,0,0,1,1,1,3,1,3,0,1,1,1,1,3,1,3,0,1,1,1,3,1,3,0,1,1,3,1,3,0,0,1,1,1,3,1,3,0,1,1,3,1,3,0,0,1,1,1,3,1,3,0,1,1,1,1,3,1,3,0,1,1,1,3,1,3,0,1,1,3,1,3,0,0,1,1,1,3,1,3,0,1,1,3,1,3,0,0,1,1,1,3,1,3,0,1,1,1,3,1,3,0,1,1,1,3,1,3,0,1,1,3,1,3,0,0,1,1,1,3,1,3,0,1,1,3,1,3,0,0,1,1,1,3,1,3,0,0,1,1,1,1,3,1,3,0,1,1,1,3,1,3,0,1,1,3,1,3,0,0,1,1,1,3,1,3,0,1,1,3,1,3,0,0,1,1,1,3,1,3,0,1,1,1,3,1,3,0,1,1,1,3,1,3,0,1,1,3,1,3,0,0,1,1,1,3,1,3,0,1,1,3,1,3,0,0,1,1,1,3,1,3,0,0,1,1,1,1,3,1,3,0,1,1,1,3,1,3,0,1,1,3,1,3,0,0,1,1,1,3,1,3,0,1,1,3,1,3,0,0,1,1,1,3,1,3,0]\n", "", 0),
    (r"def f(g; $n): if $n <= 0 then g else f(g as $x | ($x, 7); $n - 1) end; [f((1,3); 3)]", "[1,7,7,7,7,7,7,7,3,7,7,7,7,7,7,7]\n", "", 0),
    (r"def h(x): x + x; def f(g; $n): if $n <= 0 then g else f(h(g); $n - 1) end; [f((1,3); 2)]", "[4,6,6,8,6,8,8,10,6,8,8,10,8,10,10,12]\n", "", 0),
    (r"def h($x): $x + (1,3); def f(g; $n): if $n <= 0 then g else f(h(g); $n - 1) end; [f((1,3); 3)]", "[4,6,6,8,6,8,8,10,6,8,8,10,8,10,10,12]\n", "", 0),
    (r"def h($x): 5; def f(g): [h(g)]; f((1,3) + (1,3))", "[5,5,5,5]\n", "", 0),
    (r"def f(g; $n): if $n <= 0 then g else f(g + (1,3); $n - 1) end; reduce f((1,3); 12) as $x (0; . + $x)", "212992\n", "", 0),
    (r"def f(g; $n): if $n <= 0 then g else f(g + (1,3); $n - 1) end; [f(1,3; 2,1)]", "[3,5,5,7,5,7,7,9,2,4,4,6]\n", "", 0),
    (r"def f(g): [g, g]; f((1,3) + (10,20))", "[11,13,21,23,11,13,21,23]\n", "", 0),
    (r"def f(g; h): [g + h]; f((1,3); (10,20))", "[11,13,21,23]\n", "", 0),
    (r"def f(g): def k: g + (1,3); [k, k]; f((1,3))", "[2,4,4,6,2,4,4,6]\n", "", 0),
    (r"def f(g): [first(g), first(g | select(. > 1))]; f((1,2) + (0,1))", "[1,2]\n", "", 0),
    (r"def f(g; $n): if $n <= 0 then g else f(g + (1,3); $n - 1) end; [range(3) as $i | first(f(($i, 9); $i))]", "[0,2,4]\n", "", 0),
    (r"def f(g; $n): if $n <= 0 then g else f(g + (1,3); $n - 1) end; [f((1,3); 2) | $__loc__.line]", "[1,1,1,1,1,1,1,1]\n", "", 0),
    (r"def f(g; $n): if $n <= 0 then g else f(g + 1; $n - 1) end; [f((1,3); 2000)]", "[2001,2003]\n", "", 0),
    (r"def f(n): if n == 0 then n else f(n - 1 | .) end; [limit(1; f(1, 2))]", "[0]\n", "", 0),
    (r#"def f(g; $n): if $n <= 0 then g else f(g + (1,3); $n - 1) end; [limit(3; f((1,"a"); 2))]"#, "", "jq: error (at <unknown>): string (\"a\") and number (1) cannot be added\n", 5),
    (r#"def f(g; $n): if $n <= 0 then g else f(g + (1,3); $n - 1) end; [limit(4; f((1,"a"); 2))]"#, "", "jq: error (at <unknown>): string (\"a\") and number (1) cannot be added\n", 5),
    (r#"def f(g; $n): if $n <= 0 then g else f(g + (1,3); $n - 1) end; [.[]?, (f((1,"a"); 2))?]"#, "[3]\n", "", 0),
    (r#"def f(g; $n): if $n <= 0 then g else f(g + (1,"a"); $n - 1) end; [limit(2; f((1,3); 2))]"#, "[3,5]\n", "", 0),
    (r#"def f(g; $n): if $n <= 0 then g else f(g + (1,"a"); $n - 1) end; [limit(3; f((1,3); 2))]"#, "", "jq: error (at <unknown>): number (1) and string (\"a\") cannot be added\n", 5),
    (r#"def f(g; $n): if $n <= 0 then g else f(g + (1,"a"); $n - 1) end; [f((1,3); 2)]"#, "", "jq: error (at <unknown>): number (1) and string (\"a\") cannot be added\n", 5),
    (r#"def f(g; $n): if $n <= 0 then g else f(g + (1,3); $n - 1) end; [f((1, error("x")); 2)]"#, "", "jq: error (at <unknown>): x\n", 5),
    (r#"def f(g; $n): if $n <= 0 then g else f(g + (1,3); $n - 1) end; [limit(4; f((1, error("x")); 2))]"#, "", "jq: error (at <unknown>): x\n", 5),
    (r#"def f(g; $n): if $n <= 0 then g else f(g + (1,3); $n - 1) end; [limit(5; f((1, error("x")); 2))]"#, "", "jq: error (at <unknown>): x\n", 5),
    (r#"def f(g; $n): if $n <= 0 then g else f(g + (1,3); $n - 1) end; try [f((1, error("x")); 2)] catch ."#, "\"x\"\n", "", 0),
    (r#"def f(g; $n): if $n <= 0 then g else f(g + (1,3); $n - 1) end; [f((1, error("x")); 2)?]"#, "[3]\n", "", 0),
    (r"def f(g; $n): if $n <= 0 then g else f(g + (1,3); $n - 1) end; first(f((1, (3 | debug)); 3))", "4\n", "", 0),
    (r"def f(g; $n): if $n <= 0 then g else f(g + (1,3); $n - 1) end; [limit(3; f((1, (3 | debug)); 2))]", "[3,5,5]\n", "[\"DEBUG:\",3]\n", 0),
    (r"def f(g; $n): if $n <= 0 then g else f(g + (1, (3 | debug)); $n - 1) end; [limit(3; f((1,3); 2))]", "[3,5,5]\n", "[\"DEBUG:\",3]\n", 0),
    (r"def f(g; $n): if $n <= 0 then g else f(g + (1, (3 | debug)); $n - 1) end; [f((1,3); 2)]", "[3,5,5,7,5,7,7,9]\n", "[\"DEBUG:\",3]\n[\"DEBUG:\",3]\n[\"DEBUG:\",3]\n", 0),
    (r"def f(g; $n): if $n <= 0 then g else f((g | debug) + (1,3); $n - 1) end; [limit(3; f((1,3); 2))]", "[3,5,5]\n", "[\"DEBUG:\",1]\n[\"DEBUG:\",2]\n[\"DEBUG:\",3]\n[\"DEBUG:\",4]\n[\"DEBUG:\",1]\n[\"DEBUG:\",4]\n", 0),
    (r"def f(g; $n): if $n <= 0 then g else f(g + ((1,3) | debug); $n - 1) end; first(f((1,3); 3))", "4\n", "[\"DEBUG:\",1]\n[\"DEBUG:\",1]\n[\"DEBUG:\",1]\n", 0),
];

/// The input the `input` rows read: nine numbers, so a row shows how many were taken and which is next.
const NUMBERS: &str = "10 20 30 40 50 60 70 80 90";

/// (program, stdout, stderr, exit code) for `succinctly jq -nc <program>` over [`NUMBERS`].
const INPUT_CASES: &[(&str, &str, &str, i32)] = &[
    (
        r"def f(g; $n): if $n <= 0 then g else f(g + (1,3); $n - 1) end; [limit(3; f((input, input); 2))], input",
        "[12,22,34]\n40\n",
        "",
        0,
    ),
    (
        r"def f(g; $n): if $n <= 0 then g else f(g + (1,3); $n - 1) end; [f((input, input); 2)], input",
        "[12,22,34,44,54,64,76,86]\n90\n",
        "",
        0,
    ),
    (
        r"def f(g; $n): if $n <= 0 then g else f(g + (input, 100); $n - 1) end; [limit(5; f((1,3); 2))], input",
        "[31,33,111,113,131]\n40\n",
        "",
        0,
    ),
    (
        r"def f(g; $n): if $n <= 0 then g else f((g, g) + (1,3); $n - 1) end; [limit(4; f((input, 100); 2))], input",
        "[12,102,22,102]\n30\n",
        "",
        0,
    ),
    (
        r"def f(g; $n): if $n <= 0 then g else f(g + (1,3); $n - 1) end; first(f((1,3); 3) + input), input",
        "14\n20\n",
        "",
        0,
    ),
    (
        r"def f(g; $n): if $n <= 0 then g else f(g + (1,3); $n - 1) end; [limit(3; f((1,3); 2) + input)], input",
        "[13,15,15]\n20\n",
        "",
        0,
    ),
    (
        r"def f(g; $n): if $n <= 0 then g else f(g + (1,3); $n - 1) end; first(f((1,3) + input; 5)), input",
        "16\n20\n",
        "",
        0,
    ),
];

/// The document the path rows navigate.
const DOCUMENT: &str = r#"{"a":{"a":1,"b":2},"b":{"a":3,"b":4}}"#;

/// (program, stdout, stderr, exit code) for `succinctly jq -c <program>` over [`DOCUMENT`].
const DOCUMENT_CASES: &[(&str, &str, &str, i32)] = &[
    (
        r"def f(g; $n): if $n <= 0 then g else f(g + (1,3); $n - 1) end; [path(f((.a,.b); 0))]",
        "[[\"a\"],[\"b\"]]\n",
        "",
        0,
    ),
    (
        r"def f(g; $n): if $n <= 0 then g else f(g | (.a,.b); $n - 1) end; [path(f(.; 2))]",
        "[[\"a\",\"a\"],[\"a\",\"b\"],[\"b\",\"a\"],[\"b\",\"b\"]]\n",
        "",
        0,
    ),
    (
        r"def f(g; $n): if $n <= 0 then g else f(g | (.a,.b); $n - 1) end; f(.; 1) |= 5",
        "{\"a\":5,\"b\":5}\n",
        "",
        0,
    ),
    (
        r"def f(g; $n): if $n <= 0 then g else f(g | (.a,.b); $n - 1) end; first(f(.; 2))",
        "1\n",
        "",
        0,
    ),
    (
        r"def f(g; $n): if $n <= 0 then g else f(g | (.a,.b); $n - 1) end; [paths(f(.; 0) | numbers)]",
        "[[\"a\",\"a\"],[\"a\",\"b\"],[\"b\",\"a\"],[\"b\",\"b\"]]\n",
        "",
        0,
    ),
    (
        r"def f(g; $n): if $n <= 0 then g else f(g + (1,3); $n - 1) end; [f((.a.a,.b.b); 2)]",
        "[3,6,5,8,5,8,7,10]\n",
        "",
        0,
    ),
    (
        r"def f(g; $n): if $n <= 0 then g else f(g + (1,3); $n - 1) end; [f((.a,.b); 2)]",
        "",
        "jq: error (at <stdin>:0): object ({\"a\":1,\"b\":2}) and number (1) cannot be added\n",
        5,
    ),
    (
        r"def f(g; $n): if $n <= 0 then g else f(g + (1,3); $n - 1) end; first(f((.a.a,.b.b); 300))",
        "301\n",
        "",
        0,
    ),
];

/// (program, stdout, stderr, exit code) for `succinctly jq -nc <program>`: arguments that are read
/// more than once. An eager read is over before its consumer runs, and a chain that reads its
/// closure twice a level relies on that: read lazily, each read stays live under the next, and the
/// native stack holds one for every pending read. The first four are `main`'s answers that a version
/// of this fix without the read-ahead refused (`exceeded maximum recursion depth`): an argument whose
/// count the walk cannot bound (a helper that calls itself, a stream an error cuts short) but which
/// yields few outputs. The next three are wide ones, which neither aborted nor answered before; the
/// last two are a helper's and a literal's own `,`s, eager as they were.
const REREAD_CASES: &[(&str, &str, &str, i32)] = &[
    (
        r"def h10(x; $k): if $k <= 0 then x else h10(x + (1,3); $k - 1) end; def f(g; $n): if $n <= 0 then g else f(if g > 2 then (g, (-1,1)) else (1, g) end; $n - 1) end; [limit(3; f(h10(1; 2); 12))]",
        "[3,5,5]\n",
        "",
        0,
    ),
    (
        r"def h11(x): if x > 3 then x else x + 1 end; def h10(x; $k): if $k <= 0 then x else h10(x + (1,3); $k - 1) end; def f(g; $n): if $n <= 0 then g else f(h11(g % 1); $n - 1) end; isempty(f(h10(1; 0); 16))",
        "false\n",
        "",
        0,
    ),
    (
        r"def f(g; $n): if $n <= 0 then g | . * 2 else f((g | (1,null)) - g; $n - 1) end; label $out | f((1,3); 16) | ., break $out",
        "2\n",
        "",
        0,
    ),
    (
        r"def h10(x; $k): if $k <= 0 then x else h10(x + (1,3); $k - 1) end; def f(g; n): if n == 0 then g else f((g | 0) and g; n - 1) end; label $out | f(h10(1; 3) + (1,3); 12) | ., break $out",
        "true\n",
        "",
        0,
    ),
    (
        r"def f(g; $n): if $n <= 0 then g else f(g + g; $n - 1) end; first(f((1,3); 12))",
        "4096\n",
        "",
        0,
    ),
    (
        r"def f(g; $n): if $n <= 0 then g else f((g | (1,3)) + g; $n - 1) end; [limit(3; f((1,3); 10))]",
        "[11,13,11]\n",
        "",
        0,
    ),
    (
        r"def h10(x; $k): if $k <= 0 then x else h10(x + (1,3); $k - 1) end; def f(g): [limit(3; g)]; f(h10(1; 300))",
        "[301,303,303]\n",
        "",
        0,
    ),
    (
        r"def h10(x; $k): if $k <= 0 then x else h10(x + (1,3); $k - 1) end; def f(g): [g]; f(h10(1; 3))",
        "[4,6,6,8,6,8,8,10]\n",
        "",
        0,
    ),
    (
        r"def f(g): [g, g] | length; f((1,2,3) + (1,2,3) + (1,2,3) + (1,2,3))",
        "162\n",
        "",
        0,
    ),
];

fn check(args: &[&str], input: Option<&str>, cases: &[(&str, &str, &str, i32)]) -> Result<()> {
    for (program, stdout, stderr, code) in cases {
        let mut argv = args.to_vec();
        argv.push(program);
        let got = run_jq_full(&argv, input)?;
        assert_eq!(
            (got.0.as_str(), got.1.as_str(), got.2),
            (*stdout, *stderr, *code),
            "{program}"
        );
    }
    Ok(())
}

#[test]
fn test_closure_argument_chain_over_a_generator_matches_jq_7870() -> Result<()> {
    check(&["-nc"], None, NULL_INPUT_CASES)
}

#[test]
fn test_closure_argument_chain_reads_input_as_jq_does_7870() -> Result<()> {
    check(&["-nc"], Some(NUMBERS), INPUT_CASES)
}

#[test]
fn test_closure_argument_chain_of_paths_matches_jq_7870() -> Result<()> {
    check(&["-c"], Some(DOCUMENT), DOCUMENT_CASES)
}

#[test]
fn test_closure_argument_read_twice_a_level_matches_jq_7870() -> Result<()> {
    check(&["-nc"], None, REREAD_CASES)
}
