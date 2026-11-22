//! #7451: a `reduce`/`foreach` step that appends or stores a container built by a builtin or a pipe
//! (`. + [$t | tostring | split("")]`, `.[$t] = [range(3)]`, `. + [[$t] | sort]`) declined the
//! in-place fold step, whose closed-operand walk has no stage for `split`, `range` or `sort`. The
//! evaluator's route then ran the whole step on a state the fold still held, so the `+` or the write
//! copied the accumulator every step: quadratic, where jq 1.7.1 is linear. An operand that reads no
//! input and has no effect is now evaluated on `null` and the step writes in place; one that reads
//! `.`, has an effect, raises or yields other than once stays on the evaluator's route.
//!
//! Every expected line was captured from jq 1.7.1 (`/usr/bin/jq`), and jq 1.8.2 answers the same.
//! The in-place proofs are the unit tests `owned_update_step_takes_an_input_free_operand_7451` and
//! `fold_step_storing_an_input_free_operand_copies_nothing_7451` in `src/jq/eval.rs`.

use anyhow::Result;

use crate::run_jq_full;

/// (program, stdout, stderr, exit code) for `succinctly jq -nc <program>`: the shapes the issue names
/// and their neighbours.
const SHAPES: &[(&str, &str, &str, i32)] = &[
    (
        r#"reduce range(0;3) as $t ([]; . + [$t|tostring|split("")]) | length"#,
        "3\n",
        "",
        0,
    ),
    (
        r"reduce range(0;3) as $t ([]; .[$t] = [range(3)]) | length",
        "3\n",
        "",
        0,
    ),
    (
        r"reduce range(0;3) as $t ([]; . + [[range(10)]]) | length",
        "3\n",
        "",
        0,
    ),
    (
        r"reduce range(0;3) as $t ([]; .[$t] = {a:[range(5)]}) | length",
        "3\n",
        "",
        0,
    ),
    (
        r"reduce range(0;3) as $t ([]; . + [[$t]|sort]) | length",
        "3\n",
        "",
        0,
    ),
    (
        r"reduce range(0;3) as $t ([]; . + [[1,2,3]|reverse]) | length",
        "3\n",
        "",
        0,
    ),
    (
        r"reduce range(0;3) as $t ([]; . + [{a:1}|to_entries]) | length",
        "3\n",
        "",
        0,
    ),
    (
        r"reduce range(0;3) as $t ([]; . + [[$t]|map(.+1)]) | length",
        "3\n",
        "",
        0,
    ),
    (
        r"reduce range(0;3) as $t ([]; . + [($t|[.,.])]) | length",
        "3\n",
        "",
        0,
    ),
    (
        r"reduce range(0;3) as $t ({a:[]}; .a += [[$t]|sort]) | .a | length",
        "3\n",
        "",
        0,
    ),
    (
        r"reduce range(0;3) as $t ([]; .[$t] = (null|[range(10)])) | length",
        "3\n",
        "",
        0,
    ),
    (
        r"reduce range(0;3) as $t ([]; .[length] = [range(10)]) | length",
        "3\n",
        "",
        0,
    ),
    (
        r"reduce range(0;3) as $t ([]; . += [[$t]|sort]) | length",
        "3\n",
        "",
        0,
    ),
    (
        r"reduce range(0;3) as $t ([]; . |= . + [[$t]|sort]) | length",
        "3\n",
        "",
        0,
    ),
    (
        r#"reduce range(0;3) as $t ({}; .["k\($t)"] = [range(3)]) | length"#,
        "3\n",
        "",
        0,
    ),
    (
        r#"reduce range(0;3) as $t ({}; . + {("k\($t)"): ([$t]|sort)}) | length"#,
        "3\n",
        "",
        0,
    ),
    (
        r"reduce range(0;3) as $t ({a:[],n:0}; .a += [[$t]|sort] | .n += 1) | .a | length",
        "3\n",
        "",
        0,
    ),
    (
        r"reduce range(0;3) as $t ({a:[],n:0}; .n += 1 | .a += [[$t]|sort]) | .a | length",
        "3\n",
        "",
        0,
    ),
    (
        r"reduce range(0;3) as $t ([]; setpath([$t]; [range(3)])) | length",
        "3\n",
        "",
        0,
    ),
    (
        r"reduce range(0;3) as $t ([]; . + [[$t] | first(.[])]) | length",
        "3\n",
        "",
        0,
    ),
    (
        r"reduce range(0;3) as $t ([]; . + [limit(2; range(5))]) | length",
        "6\n",
        "",
        0,
    ),
    (
        r"reduce range(0;3) as $t ([]; . + [[range(3)] as $x | $x | add]) | length",
        "3\n",
        "",
        0,
    ),
    (
        r"reduce range(0;3) as $t ([]; if $t % 2 == 0 then . + [[$t]|sort] else . end) | length",
        "2\n",
        "",
        0,
    ),
    (
        r"reduce range(0;3) as $t ([]; . + [[$t]|sort] | .[-100:]) | length",
        "3\n",
        "",
        0,
    ),
    (
        r"reduce range(0;3) as $t ([]; .[1:] + [[$t]|sort]) | length",
        "1\n",
        "",
        0,
    ),
    (
        r"reduce range(0;3) as $t ([[],0]; [.[0] + [[$t]|sort], .[1] + 1]) | .[0] | length",
        "3\n",
        "",
        0,
    ),
    (
        r"reduce range(0;3) as $t ([]; . + [reduce range(3) as $u (0; . + $u)]) | length",
        "3\n",
        "",
        0,
    ),
    (
        r"reduce range(0;3) as $t ([]; . + [reduce range(3) as $u ([]; . + [[$u]|sort])]) | length",
        "3\n",
        "",
        0,
    ),
    (
        r"[foreach range(0;3) as $t ([]; . + [[$t]|sort]; length)] | length",
        "3\n",
        "",
        0,
    ),
    (
        r"[foreach range(0;3) as $t ([]; . + [[$t]|sort])] | length",
        "3\n",
        "",
        0,
    ),
    (
        r"def f: reduce range(0;3) as $t ([]; . + [[$t]|sort]); f | length",
        "3\n",
        "",
        0,
    ),
    (
        r"[0,[]] | until(.[0] >= 3; [.[0]+1, .[1] + [[.[0]]|sort]]) | .[1] | length",
        "3\n",
        "",
        0,
    ),
    (
        r"[range(3)] as $s | reduce $s[] as $t ([]; . + [[$t]|sort]) | length",
        "3\n",
        "",
        0,
    ),
    (
        r"[range(3)|[.]] | reduce .[] as $r ([]; . + [$r|sort]) | length",
        "3\n",
        "",
        0,
    ),
    (
        r"reduce range(0;3) as $t ([]; . + [$t|tostring|ascii_downcase]) | length",
        "3\n",
        "",
        0,
    ),
    (
        r"reduce range(0;3) as $t ([]; . + [[$t,$t+1]]) | length",
        "3\n",
        "",
        0,
    ),
    (
        r"reduce range(0;3) as $t ([]; .[$t] = [$t,$t,$t]) | length",
        "3\n",
        "",
        0,
    ),
    (
        r"reduce range(0;3) as $t ([]; . + [[$t]]) | length",
        "3\n",
        "",
        0,
    ),
    (
        r"reduce range(0;3) as $t (0; . + ([range(3)]|length))",
        "9\n",
        "",
        0,
    ),
    (
        r"reduce range(0;3) as $t ([]; . + [1, 2]) | length",
        "6\n",
        "",
        0,
    ),
    (
        r"reduce range(0;3) as $t ([]; . + [($t, $t)]) | length",
        "6\n",
        "",
        0,
    ),
    (
        r"reduce range(0;3) as $t ([]; . + ($t | [., .])) | length",
        "6\n",
        "",
        0,
    ),
    (
        r"reduce range(0;3) as $t ([]; . + [[$t]|sort] + [[$t]|sort]) | length",
        "6\n",
        "",
        0,
    ),
    (
        r"reduce range(0;3) as $t ([]; . - [[$t]|sort]) | length",
        "0\n",
        "",
        0,
    ),
];

/// Operands that raise, break out, have an effect or yield other than once, a `?//` binding whose
/// body a consumer's stop runs again (#2180), and states the operand cannot be added to: each keeps
/// the evaluator's answer, text and stderr.
const EDGES: &[(&str, &str, &str, i32)] = &[
    (r#"reduce range(0;3) as $t ([]; . + [error("x")])"#, "", "jq: error (at <unknown>): x\n", 5),
    (r"reduce range(0;3) as $t ([]; . + [$t | error])", "", "jq: error (at <unknown>) (not a string): 0\n", 5),
    (r"reduce range(0;3) as $t ([]; . + [[$t]|debug|sort])", "[[0],[1],[2]]\n", "[\"DEBUG:\",[0]]\n[\"DEBUG:\",[1]]\n[\"DEBUG:\",[2]]\n", 0),
    (r#"reduce range(0;3) as $t ([]; . + [[$t]|sort|debug("m")])"#, "[[0],[1],[2]]\n", "[\"DEBUG:\",\"m\"]\n[\"DEBUG:\",\"m\"]\n[\"DEBUG:\",\"m\"]\n", 0),
    (r"reduce range(0;3) as $t ([]; . + [[$t]|stderr|sort])", "[[0],[1],[2]]\n", "[0][1][2]", 0),
    (r"label $out | reduce range(0;3) as $t ([]; . + [if $t == 2 then break $out else [$t]|sort end])", "", "", 0),
    (r"reduce range(0;3) as $t ([]; . + ([1],[2]))", "[2,2,2]\n", "", 0),
    (r"reduce range(0;3) as $t ([]; . + ([[$t]|sort],[2]))", "[2,2,2]\n", "", 0),
    (r"reduce range(0;3) as $t ([]; . + empty)", "null\n", "", 0),
    (r"reduce range(0;3) as $t ([]; . + [empty])", "[]\n", "", 0),
    (r"reduce range(0;3) as $t ([]; . + [$__loc__])", "[{\"file\":\"<top-level>\",\"line\":1},{\"file\":\"<top-level>\",\"line\":1},{\"file\":\"<top-level>\",\"line\":1}]\n", "", 0),
    (r"reduce range(0;3) as $t ([]; . + [[$t]|sort|tojson])", "[\"[0]\",\"[1]\",\"[2]\"]\n", "", 0),
    (r"reduce range(0;3) as $t ([]; . + [[$t]|sort][0])", "[0,1,2]\n", "", 0),
    (r"reduce range(0;3) as $t (null; . + [[$t]|sort])", "[[0],[1],[2]]\n", "", 0),
    (r"reduce range(0;3) as $t (1; . + [[$t]|sort])", "", "jq: error (at <unknown>): number (1) and array ([[0]]) cannot be added\n", 5),
    (r"reduce range(0;3) as $t ({}; . + [[$t]|sort])", "", "jq: error (at <unknown>): object ({}) and array ([[0]]) cannot be added\n", 5),
    (r#"reduce range(0;3) as $t ("s"; . + ([$t]|sort|tostring))"#, "\"s[0][1][2]\"\n", "", 0),
    (r"reduce range(0;3) as $t (0; . + ([$t, 2]|max))", "6\n", "", 0),
    (r"reduce range(0;3) as $t (0; . * ([$t, 2]|max))", "0\n", "", 0),
    (r"reduce range(0;3) as $t ([]; .[-1] = [range(2)])", "", "jq: error (at <unknown>): Out of bounds negative array index\n", 5),
    (r"reduce range(0;3) as $t ([]; .[$t+3] = [range(2)])", "[null,null,null,[0,1],[0,1],[0,1]]\n", "", 0),
    (r"reduce range(0;3) as $t ([]; .[$t] = ([1,2]|.[5]))", "[null,null,null]\n", "", 0),
    (r"reduce range(0;3) as $t ([]; . + [[1.10, 3e2]|reverse])", "[[3E+2,1.10],[3E+2,1.10],[3E+2,1.10]]\n", "", 0),
    (r"reduce range(0;3) as $t ({}; .a.b += [[$t]|sort])", "{\"a\":{\"b\":[[0],[1],[2]]}}\n", "", 0),
    (r"reduce range(0;3) as $t ({}; .a //= [range(2)])", "{\"a\":[0,1]}\n", "", 0),
    (r"reduce range(0;3) as $t ({}; .a |= [range(2)])", "{\"a\":[0,1]}\n", "", 0),
    (r"reduce range(0;3) as $t ({}; .a |= . + [[$t]|sort])", "{\"a\":[[0],[1],[2]]}\n", "", 0),
    (r#"reduce range(0;3) as $t ([]; . + [try error("z") catch ([.]|sort)])"#, "[[\"z\"],[\"z\"],[\"z\"]]\n", "", 0),
    (r#"reduce range(0;3) as $t ([]; . + [try error("z") catch (.|debug)])"#, "[\"z\",\"z\",\"z\"]\n", "[\"DEBUG:\",\"z\"]\n[\"DEBUG:\",\"z\"]\n[\"DEBUG:\",\"z\"]\n", 0),
    (r"reduce range(0;3) as $t ([]; . + [$__loc__|type])", "[\"object\",\"object\",\"object\"]\n", "", 0),
    (r#"reduce range(0;3) as $t ([]; . + [[$t]|sort] | if length > 2 then error("big") else . end)"#, "", "jq: error (at <unknown>): big\n", 5),
    (r"[limit(3; foreach range(0;3) as $t ([]; . + [[$t]|sort]))]", "[[[0]],[[0],[1]],[[0],[1],[2]]]\n", "", 0),
    (r"reduce (range(0;3)|[.]) as [$a] ([]; . + [[$a]|sort])", "[[0],[1],[2]]\n", "", 0),
    (r"reduce range(0;3) as $t ([]; . + [[$t] as [$a] | [$a]|sort])", "[[0],[1],[2]]\n", "", 0),
    (r"reduce range(0;3) as $t ([]; . + [[$t]|sort]) | .[0] |= . + [9]", "[[0,9],[1],[2]]\n", "", 0),
    (r". as $d | reduce range(0;3) as $t ([]; . + [[$t]|sort]) | length", "3\n", "", 0),
    (r"reduce range(0;3) as $t ([]; . + [[$t]|sort] ?)", "[[0],[1],[2]]\n", "", 0),
    (r"reduce range(0;3) as $t ([]; (. + [[$t]|sort])?)", "[[0],[1],[2]]\n", "", 0),
    (r"reduce range(0;3) as $t ([]; . + [{} | .a = [range(2)]])", "[{\"a\":[0,1]},{\"a\":[0,1]},{\"a\":[0,1]}]\n", "", 0),
    (r"[first(foreach (1) as $v (0; . + (1 as $x ?// $y | 1)))]", "[1,1]\n", "", 0),
    (r"[isempty(foreach (1) as $v (0; . + (1 as $x ?// $y | 1)))]", "[false,false]\n", "", 0),
    (r"[first(foreach (1) as $v ([]; . + [1 as $x ?// $y | 1]))]", "[[1]]\n", "", 0),
    (r"[first(foreach (1) as $v (0; . + (1 | (. as $x ?// $y | 1))))]", "[1,1]\n", "", 0),
    (r"[first(reduce (1) as $v ({}; .a = ([1] as [$x] ?// $y | $x)))]", "[{\"a\":1}]\n", "", 0),
];

#[test]
fn test_fold_step_storing_an_input_free_operand_matches_jq_7451() -> Result<()> {
    for (program, stdout, stderr, code) in SHAPES.iter().chain(EDGES) {
        let got = run_jq_full(&["-nc", program], None)?;
        assert_eq!(
            (got.0.as_str(), got.1.as_str(), got.2),
            (*stdout, *stderr, *code),
            "{program}"
        );
    }
    Ok(())
}
