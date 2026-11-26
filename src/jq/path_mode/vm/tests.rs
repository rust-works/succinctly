//! The call machine's unit tests (#7977). Every expected answer was captured from `/usr/bin/jq`
//! 1.7.1 (macOS arm64) with `jq -nc`.

// STYLE-0004: jq object literals, not format strings.
#![allow(clippy::literal_string_with_formatting_args)]

use super::*;
use alloc::collections::BTreeSet;
use proptest::prelude::*;

/// Every instruction, by its `Debug` name.
const INSTRUCTIONS: [&str; 37] = [
    "Dup",
    "DupN",
    "Pop",
    "Swap",
    "Const",
    "Leaf",
    "Each",
    "Range",
    "Fork",
    "Jump",
    "JumpF",
    "JumpIfSet",
    "SetSlot",
    "Backtrack",
    "TryBegin",
    "TryEnd",
    "Label",
    "Break",
    "Store",
    "Load",
    "LoadMove",
    "LoadLast",
    "Kill",
    "NewAcc",
    "Append",
    "LoadAcc",
    "Bin",
    "Cmp",
    "ToBool",
    "Not",
    "Negate",
    "Index",
    "IndexK",
    "Raise",
    "SetPath",
    "Call",
    "Ret",
];

/// Programs that between them compile to every instruction, with jq's outputs.
const INSTRUCTION_ROWS: &[(&str, &[&str])] = &[
    (
        r#"def r($n): if $n <= 0 then . else r($n - 1) end; [1, [2, 3]] as [$a, [$b, $c]] | {k: 4, j: 5} as {k: $k, ("j"): $j} | r(2) | [$a, $b, $c, $k, $j]"#,
        &["[1,2,3,4,5]"],
    ),
    (
        "def f: if . > 0 then (. - 1 | f) + 1 else 0 end; [(3 | f) * 2, (3 | f) == 3, -(3 | f), ((3 | f) | not), ((3|f) > 1 and (2|f) < 1), (false or (1|f) == 1)]",
        &["[6,true,-3,false,false,true]"],
    ),
    (
        "def r($n): if $n <= 0 then . else r($n - 1) end; [r(1) | (1, 2), (if . then 3 else 4 end), empty, ([5, 6] | .[]), ({a: 7} | .[]), range(1; 3), ([1, null] | .[] | .[]?)]",
        &["[1,2,4,5,6,7,1,2]"],
    ),
    (
        "def r($n): if $n <= 0 then . else r($n - 1) end; [(null, false) // 1, (2, null) // 3, (empty // 4), (r(1) | (null // 5))]",
        &["[1,2,4,5]"],
    ),
    (
        r#"def r($n): if $n <= 0 then . else r($n - 1) end; [try error("x") catch ., try (1, error({a: 1})) catch .a, (try error("y")), (error(null)?), ([1] | try (.[] | error) catch .)]"#,
        &[r#"["x",1,1,1]"#],
    ),
    (
        "def r($n): if $n <= 0 then . else r($n - 1) end; [label $out | range(10) | ., (select(. == 2) | break $out)]",
        &["[0,1,2]"],
    ),
    (
        "def r($n): if $n <= 0 then . else r($n - 1) end; [reduce range(5) as $i (0; . + $i), foreach (1, 2, 3) as $i (0; . + $i; [$i, .])]",
        &["[10,[1,1],[2,3],[3,6]]"],
    ),
    (
        "def r($n): if $n <= 0 then . else r($n - 1) end; [[], [range(3)], [r(1) | 1, 2]]",
        &["[[],[0,1,2],[1,2]]"],
    ),
    (
        "def g($n): if $n == 0 then [0, 0, 0] else g($n - 1) | setpath([$n]; $n) end; [1, 2] as $a | ([$a] | length) as $l | [g(2), $l]",
        &["[[0,1,2],1]"],
    ),
    (
        r#"def f(g; $n): if $n <= 0 then g else f(g; $n - 1) end; def h(g): f(g | tostring; 2); [h(1), h(. + 1), "c"]"#,
        &[r#"["1","1","c"]"#],
    ),
    (
        "def r($n): if $n <= 0 then . else r($n - 1) end; [1, 2] as $a | r(1) | $a | setpath([0]; 9)",
        &["[9,2]"],
    ),
    (
        "def r($n): if $n <= 0 then . else r($n - 1) end; try setpath(1; 2) catch .",
        &[r#""Path must be specified as an array""#],
    ),
    (
        "def r($n): if $n <= 0 then . else r($n - 1) end; try (1 | .[]) catch .",
        &[r#""Cannot iterate over number (1)""#],
    ),
];

/// Programs written to catch an unsound move by `LoadLast` or `Kill` (#7977): forks after the
/// binding, `try`, `label`/`break`, `reduce`, `foreach`, `limit`, `first`, `//`, a closure called
/// twice, nested defs, a fork inside a callee and `$` parameters, each writing a bound array with
/// `setpath` and then reading the binding again. With jq's outputs.
const LAST_USE_ROWS: &[(&str, &[&str])] = &[
    (
        "[[1,2] as $a | ($a | setpath([0]; 9)), $a]",
        &["[[9,2],[1,2]]"],
    ),
    (
        "[[1,2] as $a | (1,2) as $i | $a | setpath([0]; $i)]",
        &["[[1,2],[2,2]]"],
    ),
    (
        "[[1,2] as $a | ($a | setpath([0]; 9)), ($a | setpath([1]; 8))]",
        &["[[9,2],[1,8]]"],
    ),
    (
        "[[1,2] as $a | try ($a | setpath([0]; 9) | error) catch $a]",
        &["[[1,2]]"],
    ),
    (
        r#"[[1,2] as $a | try (($a | setpath([0]; 9)), error("x")) catch ($a | length)]"#,
        &["[[9,2],2]"],
    ),
    (
        "[[1,2] as $a | label $out | ($a | setpath([0]; 9)), break $out, $a]",
        &["[[9,2]]"],
    ),
    (
        "[label $out | [1,2] as $a | ($a | setpath([0]; 9)), ($a | if .[0] == 1 then break $out else . end)]",
        &["[[9,2]]"],
    ),
    (
        "[1,2] as $a | reduce (0, 1) as $i ($a; setpath([$i]; $a[$i] + 10))",
        &["[11,12]"],
    ),
    (
        "reduce ([1,2] as $a | ($a | setpath([0]; 9)), $a) as $x ([]; . + [$x])",
        &["[[9,2],[1,2]]"],
    ),
    (
        "[[1,2] as $a | foreach (0, 1) as $i (0; . + 1; $a | setpath([$i]; 9))]",
        &["[[9,2],[1,9]]"],
    ),
    (
        "[[1,2] as $a | limit(2; ($a | setpath([0]; 9)), $a, $a)]",
        &["[[9,2],[1,2]]"],
    ),
    (
        "[[1,2] as $a | first(($a | setpath([0]; 9)), $a), $a]",
        &["[[9,2],[1,2]]"],
    ),
    (
        "[[1,2] as $a | (($a | setpath([0]; 9)) | select(.[0] == 1)) // $a]",
        &["[[1,2]]"],
    ),
    (
        "[[1,2] as $a | ((null, ($a | setpath([0]; 9))) // $a), $a]",
        &["[[9,2],[1,2]]"],
    ),
    (
        "def twice(f): f, f; [[1,2] as $a | twice($a | setpath([0]; 9)), $a]",
        &["[[9,2],[9,2],[1,2]]"],
    ),
    (
        "def twice(f): [f, f]; [1,2] as $a | twice($a | setpath([0]; length))",
        &["[[2,2],[2,2]]"],
    ),
    (
        "[1,2] as $a | def g: $a | setpath([0]; 9); [g, g, $a]",
        &["[[9,2],[9,2],[1,2]]"],
    ),
    (
        "def f($x): def g: $x | setpath([0]; 9); [g, $x]; f([1,2])",
        &["[[9,2],[1,2]]"],
    ),
    (
        "def gen: 1, 2; [[1,2] as $a | gen as $i | $a | setpath([0]; $i)]",
        &["[[1,2],[2,2]]"],
    ),
    (
        "def each_set($a): (0, 1) as $i | $a | setpath([$i]; 9); [each_set([1,2])]",
        &["[[9,2],[1,9]]"],
    ),
    (
        "def f($a): ($a | setpath([0]; 9)), $a; [f([1,2])]",
        &["[[9,2],[1,2]]"],
    ),
    (
        "def r($a; $n): if $n == 0 then $a else r($a | setpath([$n]; $n); $n - 1) end; r([0,0,0,0]; 3)",
        &["[0,1,2,3]"],
    ),
    (
        "def r($a; $n): if $n == 0 then $a, $a else (r($a | setpath([$n]; $n); $n - 1), $a) end; [r([0,0,0]; 2)]",
        &["[[0,1,2],[0,1,2],[0,0,2],[0,0,0]]"],
    ),
    (
        "[[[1,2],3] as [$a, $b] | ($a | setpath([0]; $b)), $a]",
        &["[[3,2],[1,2]]"],
    ),
    (
        "[[1,2] as $a | if ($a | setpath([0]; 9) | .[0]) == 9 then $a else 0 end]",
        &["[[1,2]]"],
    ),
    (
        r#"[{a: [1]} as $o | ($o | setpath(["a", 0]; 9)), $o.a]"#,
        &[r#"[{"a":[9]},[1]]"#],
    ),
    (
        "[1,2] as $a | ([$a] | length), $a",
        &["1", "[1,2]"],
    ),
    (
        r#"[[1,2] as $a | try (($a | setpath([0]; 9)), error("x")) catch ., $a]"#,
        &[r#"[[9,2],"x",[1,2]]"#],
    ),
];

/// What a run printed: each output compactly, then the uncaught error's message, if any.
#[derive(Debug, PartialEq)]
struct Answer {
    outputs: Vec<String>,
    error: Option<String>,
}

/// Parse and resolve `filter` as the CLI hands it over.
fn program(filter: &str) -> Expr {
    super::super::compile(filter).unwrap_or_else(|e| panic!("{filter}: {e}"))
}

/// Run `filter` on `null` on the machine, as `run_program` does with the switch on.
fn machine(filter: &str) -> Answer {
    let program = program(filter);
    with_prelude(|prelude| {
        let it = Interp::new(prelude);
        let mut input = OwnedValue::Null;
        it.prepare(&program, &mut input);
        let mut outputs = Vec::new();
        let result = run(&it, &program, input, &mut |v| {
            outputs.push(v.to_json_jq_preserve());
            true
        });
        let error = match result {
            Ok(()) => None,
            Err(raise) => match Interp::outcome(raise) {
                Outcome::Done => None,
                Outcome::Error(error) => Some(match error.payload() {
                    OwnedValue::String(s) => s.as_str().to_string(),
                    other => other.to_json_jq_preserve(),
                }),
                other => panic!("{filter}: {other:?}"),
            },
        };
        Answer { outputs, error }
    })
}

/// The instructions `filter` compiles to, by name.
fn instructions(filter: &str) -> BTreeSet<String> {
    let program = program(filter);
    with_prelude(|prelude| {
        let it = Interp::new(prelude);
        let code = compile::compile(&it, &program).unwrap_or_else(|e| panic!("{filter}: {e}"));
        code.insns
            .iter()
            .map(|insn| {
                let name = format!("{insn:?}");
                let end = name
                    .find(|c: char| !c.is_alphanumeric())
                    .unwrap_or(name.len());
                name[..end].to_string()
            })
            .collect()
    })
}

fn ok(outputs: &[&str]) -> Answer {
    Answer {
        outputs: outputs.iter().map(|s| String::from(*s)).collect(),
        error: None,
    }
}

#[test]
fn every_instruction_runs_as_jq_7977() {
    let mut seen = BTreeSet::new();
    for (filter, want) in INSTRUCTION_ROWS {
        assert_eq!(machine(filter), ok(want), "{filter}");
        seen.extend(instructions(filter));
    }
    let all: BTreeSet<String> = INSTRUCTIONS.iter().map(|s| String::from(*s)).collect();
    assert_eq!(
        seen, all,
        "the rows compile to every instruction, and to nothing else"
    );
}

#[test]
fn an_error_after_an_output_ends_the_run_after_the_output_7977() {
    let want = Answer {
        outputs: vec!["1".to_string()],
        error: Some("boom".to_string()),
    };
    assert_eq!(
        machine(r#"def r($n): if $n <= 0 then . else r($n - 1) end; r(2) | 1, error("boom"), 3"#),
        want
    );
}

#[test]
fn a_consumer_that_stops_stops_the_producer_7977() {
    let program = program("def nat: ., (. + 1 | nat); 0 | nat");
    let outputs = with_prelude(|prelude| {
        let it = Interp::new(prelude);
        let mut seen = Vec::new();
        let result = run(&it, &program, OwnedValue::Null, &mut |v| {
            seen.push(v.to_json_jq_preserve());
            seen.len() < 3
        });
        assert!(matches!(result, Err(Raise::Stop)), "{result:?}");
        seen
    });
    assert_eq!(outputs, ["0", "1", "2"]);
}

#[test]
fn last_reads_move_only_where_no_fork_point_can_read_again_7977() {
    let mut moves = 0;
    for (filter, want) in LAST_USE_ROWS {
        assert_eq!(machine(filter), ok(want), "{filter}");
        let insns = instructions(filter);
        if insns.contains("LoadLast") || insns.contains("Kill") {
            moves += 1;
        }
    }
    // The rows that read the binding inside a nested `def` are excluded from the analysis; the
    // others must reach a move, or they test nothing.
    assert_eq!(
        moves,
        LAST_USE_ROWS.len() - 2,
        "rows that compile to a LoadLast or a Kill"
    );
}

/// #7803's memo: `$q4 | setpath([$n]; $ans)` writes the array `$q4` holds, which is its last read,
/// so the write is in place. jq itself copies there (it loads `$q4` with `LOADV`) and is quadratic
/// in N; the shipped evaluators copy at every level.
#[test]
fn the_7803_memo_copies_its_array_at_most_twice_7977() {
    let filter = "def Q: def Q(n): n as $n | (if . == null then [1, 1, 1] else . end) as $q | if $q[$n] != null then $q else $q | Q($n - 1) as $q1 | $q1 | Q($n - 2) as $q2 | $q2 | Q($n - $q2[$n - 1]) as $q3 | $q3 | Q($n - $q3[$n - 2]) as $q4 | ($q4[$n - $q4[$n - 1]] + $q4[$n - $q4[$n - 2]]) as $ans | $q4 | setpath([$n]; $ans) end; . as $n | null | Q($n) | .[$n]; 3200 | Q";
    let (answer, recorded) = crate::jq::share_stats::measure(|| machine(filter));
    assert_eq!(answer, ok(&["1500"]));
    let copies: u64 = recorded.iter().map(|(_, n)| n).sum();
    assert!(copies <= 2, "forced copies: {recorded:?}");
    // The count is not vacuous: a write while a fork point holds the binding does copy.
    let ((), recorded) = crate::jq::share_stats::measure(|| {
        machine("[[1,2] as $a | ($a | setpath([0]; 9)), $a]");
    });
    assert_eq!(
        recorded.iter().map(|(_, n)| n).sum::<u64>(),
        1,
        "{recorded:?}"
    );
}

#[test]
fn the_router_takes_recursive_programs_and_declines_the_rest_7977() {
    for filter in [
        "def f($n): if $n == 0 then 0 else f($n - 1) + 1 end; f(10)",
        "def f: if . > 0 then (. - 1 | f), . else . end; [5 | f]",
        "def nat: ., (. + 1 | nat); first(0 | nat | select(. > 5))",
    ] {
        assert!(decide(&program(filter)), "takes {filter}");
    }
    for filter in [
        // No recursive def.
        "def k(f): f * 2; [1, 2] | map(k(.))",
        "[range(10)] | add",
        // `?//`, an input builtin, a path form, `last`.
        "def f($n): if $n == 0 then [1] as [$x] ?// $x | $x else f($n - 1) end; f(2)",
        "def f($n): if $n == 0 then input else f($n - 1) end; f(2)",
        "def d($n): if $n == 0 then .a else d($n - 1) end; {a: 1} | path(d(3))",
        "def f($n): if $n == 0 then 1, 2 else f($n - 1) end; last(f(3))",
        "def f($n): if $n == 0 then . else f($n - 1) end; {a: 1} | .a |= f(2)",
        // A call inside an object constructor or a string interpolation.
        "def f($n): if $n == 0 then 1 else f($n - 1) end; {a: f(2)}",
        r#"def f($n): if $n == 0 then 1 else f($n - 1) end; "\(f(2))""#,
    ] {
        assert!(!decide(&program(filter)), "declines {filter}");
    }
}

#[cfg(feature = "std")]
#[test]
fn the_switch_is_on_only_for_a_value_but_0_7977() {
    use std::ffi::OsStr;
    assert!(!switch_value_on(None));
    assert!(!switch_value_on(Some(OsStr::new(""))));
    assert!(!switch_value_on(Some(OsStr::new("0"))));
    assert!(switch_value_on(Some(OsStr::new("1"))));
    assert!(switch_value_on(Some(OsStr::new("yes"))));
}

#[test]
fn past_the_budget_a_call_refuses_naming_its_definition_7977() {
    let Raise::Fatal(error) = refusal("_sp_limit", 2) else {
        panic!("a refusal is fatal");
    };
    assert_eq!(error.message, "limit/2 exceeded maximum recursion depth");
}

/// One step of [`Stack`]'s property test.
#[derive(Clone, Debug)]
enum Op {
    Push,
    Pop,
    Save,
    Restore,
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        3 => Just(Op::Push),
        3 => Just(Op::Pop),
        1 => Just(Op::Save),
        1 => Just(Op::Restore),
    ]
}

proptest! {
    /// gojq's protection rule: whatever is pushed and popped after a `save`, a `restore` finds
    /// exactly the stack that was saved, so no read through a restored fork point sees a value
    /// that was moved out (a moved value leaves `null`, which this never pushes). Fork points
    /// nest, so saves are restored last first.
    #[test]
    fn a_restored_stack_holds_every_value_it_saved_7977(ops in proptest::collection::vec(op(), 0..200)) {
        let mut stack: Stack<OwnedValue> = Stack::new();
        let mut model: Vec<i64> = Vec::new();
        let mut saved: Vec<((isize, isize), Vec<i64>)> = Vec::new();
        let mut next = 1;
        for op in ops {
            match op {
                Op::Push => {
                    // An array, so a moved value and a copied one differ in storage too.
                    stack.push(OwnedValue::Array(vec![OwnedValue::Int(next)].into()));
                    model.push(next);
                    next += 1;
                }
                Op::Pop if !model.is_empty() => {
                    let want = model.pop();
                    let got = match stack.pop() {
                        OwnedValue::Array(items) => match items.first() {
                            Some(OwnedValue::Int(n)) => Some(*n),
                            _ => None,
                        },
                        _ => None,
                    };
                    prop_assert_eq!(got, want);
                }
                Op::Save => saved.push((stack.save(), model.clone())),
                Op::Restore => {
                    if let Some((heights, values)) = saved.pop() {
                        stack.restore(heights);
                        model = values;
                    }
                }
                Op::Pop => {}
            }
            // The live stack, read down its `next` links without popping, is the model.
            let mut live = Vec::new();
            let mut at = stack.index;
            while at >= 0 {
                let block = &stack.data[at as usize];
                match &block.value {
                    OwnedValue::Array(items) => match items.first() {
                        Some(OwnedValue::Int(n)) => live.push(*n),
                        other => prop_assert!(false, "a moved value at {}: {:?}", at, other),
                    },
                    other => prop_assert!(false, "a moved value at {}: {:?}", at, other),
                }
                at = block.next;
            }
            live.reverse();
            prop_assert_eq!(&live, &model);
        }
    }
}

/// `stack_popn` (`DUPN`): it moves the value out even from a protected block, leaving `null`
/// there, which is sound only because the fork point that protects it resumes by popping it
/// unread (`reduce` and `foreach` load their accumulator instead).
#[test]
fn pop_n_moves_even_a_protected_value_and_leaves_null_7977() {
    let mut stack: Stack<OwnedValue> = Stack::new();
    stack.push(OwnedValue::Int(1));
    let heights = stack.save();
    assert!(!stack.top_free());
    assert_eq!(stack.pop_n(), OwnedValue::Int(1));
    stack.restore(heights);
    assert_eq!(stack.pop(), OwnedValue::Null);
    // Above the watermark it frees the block as `pop` does.
    stack.push(OwnedValue::Int(2));
    assert!(stack.top_free());
    assert_eq!(stack.pop_n(), OwnedValue::Int(2));
    assert!(stack.data.is_empty());
}
