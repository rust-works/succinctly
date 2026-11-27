//! The call machine's unit tests (#7977). Every expected output and jq error was captured from
//! `/usr/bin/jq` 1.7.1 (macOS arm64) with `jq -nc`; the budget's refusal wording is succinctly's
//! own (ADR-0025), which jq, having no such bound, never prints.

// STYLE-0004: jq object literals, not format strings.
#![allow(clippy::literal_string_with_formatting_args)]

use super::*;
use alloc::collections::BTreeSet;
use proptest::prelude::*;

/// Every instruction, by [`compile::insn_name`].
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
    machine_compiled(filter, true).unwrap_or_else(|e| panic!("{filter}: {e}"))
}

/// [`machine`], with last-use moves (`moves`) or without them; `Err` when the machine declines.
fn machine_compiled(filter: &str, moves: bool) -> Result<Answer, String> {
    let program = program(filter);
    with_prelude(|prelude| {
        let it = Interp::new(prelude);
        let code = if moves {
            compile::compile(&it, &program)?
        } else {
            compile::compile_without_moves(&it, &program)?
        };
        let mut input = OwnedValue::Null;
        it.prepare(&program, &mut input);
        let mut outputs = Vec::new();
        let result = run_code(&it, &code, input, &mut |v| {
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
        Ok(Answer { outputs, error })
    })
}

/// The instructions `filter`'s own functions compile to, by name: the main program, its `def`s
/// and closure arguments, not the prelude bodies it calls.
fn own_instructions(filter: &str) -> BTreeSet<&'static str> {
    let program = program(filter);
    with_prelude(|prelude| {
        let it = Interp::new(prelude);
        let code = compile::compile(&it, &program).unwrap_or_else(|e| panic!("{filter}: {e}"));
        let mut entries: Vec<usize> = code.funcs.iter().map(|f| f.entry).collect();
        entries.sort_unstable();
        let mut own = BTreeSet::new();
        for (i, func) in code.funcs.iter().enumerate() {
            if code.prelude_funcs.contains(&(i as u32)) {
                continue;
            }
            let end = entries
                .iter()
                .copied()
                .find(|&e| e > func.entry)
                .unwrap_or(code.insns.len());
            own.extend(code.insns[func.entry..end].iter().map(compile::insn_name));
        }
        own
    })
}

/// Whether the machine declines `filter`.
fn declines(filter: &str) -> bool {
    let program = program(filter);
    with_prelude(|prelude| compile::compile(&Interp::new(prelude), &program).is_err())
}

fn ok(outputs: &[&str]) -> Answer {
    Answer {
        outputs: outputs.iter().map(|s| String::from(*s)).collect(),
        error: None,
    }
}

#[test]
fn every_instruction_runs_as_jq_7977() {
    EXECUTED.with(|seen| seen.borrow_mut().clear());
    for (filter, want) in INSTRUCTION_ROWS {
        assert_eq!(machine(filter), ok(want), "{filter}");
    }
    // `INSTRUCTIONS` is every name `compile::insn_name` can return, read from its own source:
    // a variant named there and missing here fails, as one missing from both cannot compile.
    let source = include_str!("compile.rs");
    let body = &source[source.find("fn insn_name(").expect("insn_name")..];
    let body = &body[..body.find("\n}\n").expect("its end")];
    let named: BTreeSet<&str> = body
        .lines()
        .filter_map(|line| line.split("=> \"").nth(1))
        .filter_map(|rest| rest.split('"').next())
        .collect();
    assert_eq!(
        named,
        INSTRUCTIONS.iter().copied().collect::<BTreeSet<&str>>(),
        "INSTRUCTIONS and insn_name name the same instructions"
    );
    let ran = EXECUTED.with(|seen| seen.borrow().clone());
    let all: BTreeSet<&str> = INSTRUCTIONS.iter().copied().collect();
    assert_eq!(all.len(), INSTRUCTIONS.len(), "a name listed twice");
    assert_eq!(
        ran, all,
        "the rows run every instruction (`Vm::step` counts what it runs)"
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
        let insns = own_instructions(filter);
        if insns.contains("LoadLast") || insns.contains("Kill") {
            moves += 1;
        }
    }
    // The rows that read the binding inside a nested `def` are excluded from the analysis; the
    // others must reach a move, or they test nothing.
    assert_eq!(
        moves,
        LAST_USE_ROWS.len() - 2,
        "rows whose own code compiles to a LoadLast or a Kill"
    );
}

/// A name bound twice in one pattern is the first store that runs (#7977 review): the last
/// element of an array pattern, the first entry of an object pattern, as in jq and on the shipped
/// route.
#[test]
fn a_name_bound_twice_in_a_pattern_is_its_first_store_7977() {
    const R: &str = "def r: if false then r else . end; ";
    for (filter, want) in [
        ("[1,2] as [$a, $a] | $a", "2"),
        ("[1,2,3] as [$a, $a, $a] | $a", "3"),
        (r#"{"a":1,"b":2} as {a: $x, b: $x} | $x"#, "1"),
        ("[[1,2],3] as [[$a, $a], $a] | $a", "3"),
        ("[1,[2,3]] as [$a, [$a, $a]] | $a", "3"),
        (r#"{"a":1,"b":[2]} as {a: $x, b: [$x]} | $x"#, "1"),
        (r#"[{"a":1},2] as [{a: $x}, $x] | $x"#, "2"),
        (r#"{"a":1,"b":2} as {$a, b: $a} | $a"#, "1"),
        ("reduce ([1,2],[3,4]) as [$a, $a] (0; . + $a)", "6"),
        ("[foreach ([1,2],[3,4]) as [$a, $a] (0; . + $a)]", "[2,6]"),
        (
            r#"[foreach ({"a":1,"b":2}) as {a: $x, b: $x} (0; . + $x)]"#,
            "[1]",
        ),
    ] {
        assert_eq!(machine(&format!("{R}{filter}")), ok(&[want]), "{filter}");
    }
}

/// A key expression in an object pattern sees the scope outside the pattern, not the pattern's
/// own variables, as in jq (#7977 re-review); with no outer binding the machine declines the
/// program, and the shipped route reports jq's compile error.
#[test]
fn a_pattern_key_sees_only_the_scope_outside_the_pattern_7977() {
    const R: &str = "def r: if false then r else . end; ";
    for (filter, want) in [
        (
            r#""z" as $x | {"a":"b","b":"c","z":9} as {a: $x, ($x): $y} | [$x, $y]"#,
            r#"["b",9]"#,
        ),
        (
            r#""z" as $x | {"x":1,"z":9} as {$x, ($x): $y} | [$x,$y]"#,
            "[1,9]",
        ),
        (
            r#""z" as $x | reduce ({"a":"b","z":1},{"a":"c","z":2}) as {a: $x, ($x): $y} (0; . + $y)"#,
            "3",
        ),
        (
            r#""z" as $x | [foreach ({"a":"b","z":1}) as {a: $x, ($x): $y} (0; . + $y)]"#,
            "[1]",
        ),
        (
            r#""z" as $x | {"a":{"k":"b"},"z":5} as {a: {k: $x}, ($x): $y} | [$x,$y]"#,
            r#"["b",5]"#,
        ),
    ] {
        assert_eq!(machine(&format!("{R}{filter}")), ok(&[want]), "{filter}");
    }
    let unbound = r#"def r: if false then r else . end; {"a":"b"} as {a: $x, ($x): $y} | $y"#;
    let program = parse(unbound).expect("parses");
    assert!(
        with_prelude(|prelude| compile::compile(&Interp::new(prelude), &program).is_err()),
        "declines {unbound}"
    );
}

/// `range`'s first output has the shipped route's representation, so `range(0; 1) | -.` is `-0`
/// as in jq (#7977 review).
#[test]
fn ranges_first_output_negates_as_jqs_7977() {
    const R: &str = "def r: if false then r else . end; ";
    for (filter, want) in [
        ("range(0;1) | -.", &["-0"][..]),
        ("[range(0;3;1) | -.]", &["[-0,-1,-2]"]),
        ("[range(0;2) | [., -.]]", &["[[0,-0],[1,-1]]"]),
        ("range(0.0;2)", &["0.0", "1"]),
        ("[range(1.0;3)]", &["[1.0,2]"]),
        // `range/3`'s `$from` when it is not a constant (#7977 re-review).
        ("0 as $x | [range($x; 2; 1) | -.]", &["[-0,-1]"]),
        ("[0 | range(.;3;1) | -.]", &["[-0,-1,-2]"]),
        ("[range((0,1);3;1) | -.]", &["[-0,-1,-2,-1,-2]"]),
        ("def g($n): [range($n; 2; 1) | -.]; g(0)", &["[-0,-1]"]),
    ] {
        assert_eq!(machine(&format!("{R}{filter}")), ok(want), "{filter}");
    }
}

/// A leaf runs to its end before its first output is used, so one that can answer more than once
/// may not hold `debug` or `stderr` (#7977 review): the effect of a later output would fire past
/// a consumer that stops, and before the consumer saw the earlier one. The machine declines
/// such a program; `debug` alone, which answers once, is still a leaf.
#[test]
fn a_side_effect_never_runs_inside_a_leaf_that_answers_twice_7977() {
    for filter in [
        "def f($n): if $n == 0 then {a: ((1,2) | debug)} else f($n - 1) end; first(f(2))",
        "def r: if false then r else . end; first({a: (1, (2|debug))})",
        r#"def r: if false then r else . end; "\((1,2)|stderr)""#,
        "def r: if false then r else . end; [5,6] | .[((0,1)|debug)]",
        // `halt_error/1` writes its message when it runs (#7977 re-review).
        r#"def r: if false then r else . end; first({a: (1, ("bye\n" | halt_error(1)))})"#,
        r#"def r: if false then r else . end; first("\(1, ("bye\n" | halt_error(1)))")"#,
    ] {
        assert!(declines(filter), "declines {filter}");
    }
    // `?` and unary `-` over such a subtree compile natively instead, outputs one at a time.
    for filter in [
        "def r: if false then r else . end; [((1,2)|debug)?]",
        "def r: if false then r else . end; -((1,2)|debug)",
        "def r: if false then r else . end; 1 | debug | . + 1",
        "def r: if false then r else . end; (1 | debug), (2 | stderr)",
        "def r: if false then r else . end; {a: (1, 2)}",
    ] {
        assert!(!declines(filter), "takes {filter}");
    }
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

#[test]
fn the_machine_is_off_until_a_caller_turns_it_on_7977() {
    // The process-wide flag, read without changing it: no test turns it on, so another test
    // running at the same time cannot see it move.
    assert!(!switched_on());
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

/// Programs that bind an array to `$a`, write it with `setpath` and read it again, under the
/// control a move must survive: `,`, `//`, `try`, `label`/`break`, `first`, `reduce` and
/// `foreach` (which reach `pop_n` and the slots watermark), `if`, rebinding and destructuring.
fn moving_program() -> impl Strategy<Value = String> {
    let leaf = prop_oneof![
        Just("$a".to_string()),
        Just("($a | setpath([0]; 9))".to_string()),
        Just("($a | setpath([1]; length))".to_string()),
        Just("1".to_string()),
        Just(".".to_string()),
        Just("empty".to_string()),
        Just(r#"error("e")"#.to_string()),
    ];
    let body = leaf.prop_recursive(4, 32, 3, |inner| {
        prop_oneof![
            (inner.clone(), inner.clone()).prop_map(|(x, y)| format!("({x}, {y})")),
            (inner.clone(), inner.clone()).prop_map(|(x, y)| format!("({x} // {y})")),
            (inner.clone(), inner.clone()).prop_map(|(x, y)| format!("(try {x} catch {y})")),
            inner.clone().prop_map(|x| format!("first({x})")),
            (inner.clone(), inner.clone())
                .prop_map(|(x, y)| format!("(label $l | {x}, break $l, {y})")),
            (inner.clone(), inner.clone()).prop_map(|(x, y)| format!("({x} | {y})")),
            inner.clone().prop_map(|x| format!("[{x}]")),
            (inner.clone(), inner.clone())
                .prop_map(|(x, y)| format!("(reduce (0, 1) as $i ({x}; {y}))")),
            (inner.clone(), inner.clone())
                .prop_map(|(x, y)| format!("(foreach (0, 1) as $i ({x}; {y}))")),
            (inner.clone(), inner.clone()).prop_map(|(x, y)| format!("({x} as $a | {y})")),
            (inner.clone(), inner.clone())
                .prop_map(|(x, y)| format!("([{x}, 0] as [$a, $b] | {y})")),
            (inner.clone(), inner.clone(), inner)
                .prop_map(|(c, x, y)| format!("(if {c} then {x} else {y} end)")),
        ]
    });
    (body, any::<bool>()).prop_map(|(e, in_def)| {
        if in_def {
            format!(
                "def f($a; $n): if $n == 0 then [{e}] else f($a | setpath([0]; $n); $n - 1), [$a] end; \
                 [f([1,2]; 2)]"
            )
        } else {
            format!("def r: if false then r else . end; [1,2] as $a | [{e}]")
        }
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// The last-use moves change no answer (#7977 review): every program answers the same with
    /// `LoadLast` and `Kill` as with plain `Load`s, which never move.
    #[test]
    fn the_last_use_moves_change_no_answer_7977(filter in moving_program()) {
        let moved = machine_compiled(&filter, true);
        let copied = machine_compiled(&filter, false);
        prop_assert_eq!(moved.is_ok(), copied.is_ok(), "{}", filter);
        if let (Ok(moved), Ok(copied)) = (moved, copied) {
            prop_assert_eq!(moved, copied, "{}", filter);
        }
    }
}

/// The generator above reaches moves at all: a sample of its programs compiles to them.
#[test]
fn the_move_programs_reach_loadlast_and_kill_7977() {
    use proptest::strategy::ValueTree;
    use proptest::test_runner::TestRunner;
    let mut runner = TestRunner::deterministic();
    let (mut last, mut kill) = (0, 0);
    for _ in 0..200 {
        let filter = moving_program()
            .new_tree(&mut runner)
            .expect("a program")
            .current();
        if declines(&filter) {
            continue;
        }
        let own = own_instructions(&filter);
        last += usize::from(own.contains("LoadLast"));
        kill += usize::from(own.contains("Kill"));
    }
    assert!(
        last > 20 && kill > 20,
        "LoadLast in {last}, Kill in {kill} of 200"
    );
}
