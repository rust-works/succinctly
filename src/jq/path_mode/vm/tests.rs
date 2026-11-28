//! The call machine's unit tests (#7977). Every expected output and jq error was captured from
//! `/usr/bin/jq` 1.7.1 (macOS arm64) with `jq -nc`; the budget's refusal wording is succinctly's
//! own (ADR-0025), which jq, having no such bound, never prints.

// STYLE-0004: jq object literals, not format strings.
#![allow(clippy::literal_string_with_formatting_args)]

use super::*;
use alloc::collections::BTreeSet;
use proptest::prelude::*;

/// Every instruction, by [`compile::insn_name`].
const INSTRUCTIONS: [&str; 40] = [
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
    "BinK",
    "CmpK",
    "ToBool",
    "Not",
    "Negate",
    "Index",
    "IndexK",
    "Raise",
    "SetPath",
    "Call",
    "TailJump",
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
    (
        "def f: if . < 3 then . + 1 | f else . end; [0 | f] | [.[0] + .[0], .[0] < .[0]]",
        &["[6,false]"],
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
    machine_within(SHIPPED, filter)
}

/// [`machine`] within `limits`.
fn machine_within(limits: Limits, filter: &str) -> Answer {
    machine_compiled(limits, filter, true).unwrap_or_else(|e| panic!("{filter}: {e}"))
}

/// [`machine_within`], with last-use moves (`moves`) or without them; `Err` when the machine
/// declines.
fn machine_compiled(limits: Limits, filter: &str, moves: bool) -> Result<Answer, String> {
    machine_built(
        limits,
        filter,
        Build {
            moves,
            ..Build::SHIPPED
        },
    )
}

/// What a test turns off of the compiler and the machine, to check that it changes no answer.
#[derive(Clone, Copy, Debug)]
struct Build {
    /// Last-use moves ([`compile::compile_built`]).
    moves: bool,
    /// Tail jumps ([`compile::compile_built`]).
    jumps: bool,
    /// Dropping dead fork points ([`Vm::drop_dead_forks`]).
    drops: bool,
    /// Leaves that collect the generators they hold ([`compile::compile_built`]); without them
    /// step 1's rule, under which the machine runs every `.[]` and `range` itself.
    leaves: bool,
}

impl Build {
    const SHIPPED: Self = Self {
        moves: true,
        jumps: true,
        drops: true,
        leaves: true,
    };

    /// Step 1's leaves: the machine's own code for every `[...]`, `reduce`, `.[]` and `range`,
    /// which the tests of moves and of instructions read.
    const NARROW: Self = Self {
        leaves: false,
        ..Self::SHIPPED
    };
}

/// [`machine_compiled`] of the machine `build` says.
fn machine_built(limits: Limits, filter: &str, build: Build) -> Result<Answer, String> {
    let program = program(filter);
    with_prelude(|prelude| {
        let it = Interp::new(prelude);
        let code = compile::compile_built(&it, &program, build.moves, build.jumps, build.leaves)?;
        let mut input = OwnedValue::Null;
        it.prepare(&program, &mut input);
        let mut outputs = Vec::new();
        let mut vm = Vm::new(&it, &code, limits);
        vm.drops = build.drops;
        let result = vm.exec(input, &mut |v| {
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
    own_instructions_built(filter, Build::SHIPPED)
}

/// [`own_instructions`] of the build `build` says.
fn own_instructions_built(filter: &str, build: Build) -> BTreeSet<&'static str> {
    let program = program(filter);
    with_prelude(|prelude| {
        let it = Interp::new(prelude);
        let code = compile::compile_built(&it, &program, build.moves, build.jumps, build.leaves)
            .unwrap_or_else(|e| panic!("{filter}: {e}"));
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
        assert_eq!(
            machine_built(SHIPPED, filter, Build::NARROW),
            Ok(ok(want)),
            "{filter}"
        );
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
    let program = decide(&program("def nat: ., (. + 1 | nat); 0 | nat")).expect("takes");
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
        assert_eq!(
            machine_built(SHIPPED, filter, Build::NARROW),
            Ok(ok(want)),
            "{filter}"
        );
        let insns = own_instructions_built(filter, Build::NARROW);
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
        assert!(decide(&program(filter)).is_some(), "takes {filter}");
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
        assert!(decide(&program(filter)).is_none(), "declines {filter}");
    }
}

/// #7977 step 5: a pinned program is decided and compiled once, as its route is pinned, and every
/// document of the run runs that code; the pin frees it. A program that is not pinned is decided
/// per call, and a pinned program the machine declined is not decided again. `no_std` pins
/// nothing.
#[cfg(feature = "std")]
#[test]
fn a_pinned_program_keeps_its_code_for_the_run_7977() {
    let taken = program("def f($n): if $n == 0 then 0 else f($n - 1) + 1 end; f(10)");
    let pin = super::super::pin(&taken, &Route::Interpreter, decide(&taken));
    assert!(matches!(route(&taken), Route::Interpreter));
    let first = machine_for(&taken).expect("pinned with its code");
    for _ in 0..3 {
        let again = machine_for(&taken).expect("pinned with its code");
        assert!(Rc::ptr_eq(&first, &again), "one compilation for the run");
    }
    // Another program is decided afresh, per call, and leaves the pin as it is.
    let other = program("def g: if . > 0 then . - 1 | g else . end; 3 | g");
    let (a, b) = (machine_for(&other), machine_for(&other));
    assert!(a.is_some() && b.is_some() && !Rc::ptr_eq(&a.unwrap(), &b.unwrap()));
    assert!(Rc::ptr_eq(
        &first,
        &machine_for(&taken).expect("still pinned")
    ));
    drop(first);
    drop(pin);
    assert!(PINNED_MACHINE.with(|machine| machine.borrow().is_none()));

    // A pinned program the machine was not given finds no code, though it would take it.
    let _pin = super::super::pin(&taken, &Route::Interpreter, None);
    assert!(machine_for(&taken).is_none());
}

/// Run `machine`, compiled code a run keeps, on `input` (an integer) within `limits`, as `run_program` runs
/// a document: its stacks are those the last document left. `stop` stops the consumer after that
/// many outputs.
fn kept_run(limits: Limits, machine: &Compiled, input: &str, stop: Option<usize>) -> Answer {
    with_prelude(|prelude| {
        let it = Interp::new(prelude);
        let mut input = OwnedValue::Int(input.parse().expect("an integer"));
        it.prepare_keys(machine.hands_out_keys(), &mut input);
        let mut outputs = Vec::new();
        let result = run_within(limits, &it, machine, input, &mut |v| {
            outputs.push(v.to_json_jq_preserve());
            stop.is_none_or(|n| outputs.len() < n)
        });
        let error = match result {
            Ok(()) | Err(Raise::Stop) => None,
            Err(raise) => match Interp::outcome(raise) {
                Outcome::Error(error) => Some(match error.payload() {
                    OwnedValue::String(s) => s.as_str().to_string(),
                    other => other.to_json_jq_preserve(),
                }),
                other => panic!("{other:?}"),
            },
        };
        Answer { outputs, error }
    })
}

/// The bytes of the vectors `machine` keeps for its next document, per vector.
fn kept_bytes(machine: &Compiled) -> [usize; 5] {
    let spare = machine.spare.take();
    let bytes = [
        spare.data.capacity() * core::mem::size_of::<Block<OwnedValue>>(),
        spare.frames.capacity() * core::mem::size_of::<Block<FrameRec>>(),
        spare.slots.capacity() * core::mem::size_of::<Slot>(),
        spare.forks.capacity() * core::mem::size_of::<Fork>(),
        spare.scratch.capacity() * core::mem::size_of::<Slot>(),
    ];
    assert!(
        spare.data.is_empty()
            && spare.frames.is_empty()
            && spare.slots.is_empty()
            && spare.forks.is_empty()
            && spare.scratch.is_empty(),
        "a kept vector holds nothing of the run before"
    );
    machine.spare.set(spare);
    bytes
}

/// #7977 step 5: the stacks a document leaves are reused by the next one, emptied, and nothing
/// else carries over. Whatever the document before left (a refusal at the byte budget or the
/// loop cap, an error with fork points live, a consumer that stopped), the next answers exactly
/// as on a fresh machine, and refuses at the same bounds: the budget charges a kept vector only
/// as the run grows into it.
#[test]
fn a_document_after_a_dirty_one_answers_as_on_a_fresh_machine_7977() {
    // One program, so one machine, whose input picks the shape: `f` grows the data and frame
    // stacks, `h` the fork points (a `,` live at every level, then an error), `t` a tail loop.
    let filter = r#"def f($n): if $n == 0 then 0 else f($n - 1) + 1 end;
        def h($n): if $n == 0 then error("boom") else $n, h($n - 1) end;
        def t: if . > 0 then . - 1 | t else . end;
        if . >= 0 then f(.) elif . > -1000000 then h(-.) else -. - 1000000 | t end"#;
    let limits = small(1 << 16, 50, 1 << 20);
    let fresh = |input: &str| {
        let machine = decide(&program(filter)).expect("takes");
        kept_run(limits, &machine, input, None)
    };
    // The deepest `f` and `h` the budget answers on a fresh machine.
    let deepest = |sign: i64| {
        (1..4000)
            .take_while(|n| {
                let a = fresh(&(sign * n).to_string());
                a.error.is_none() || a.error.as_deref() == Some("boom")
            })
            .last()
            .expect("some depth fits")
    };
    let (f_depth, h_depth) = (deepest(1), deepest(-1));
    let probes = [
        f_depth.to_string(),
        (f_depth + 1).to_string(),
        String::from("3"),
        (-h_depth).to_string(),
        (-h_depth - 1).to_string(),
        String::from("-2"),
        String::from("-1000049"),
        String::from("-1000051"),
    ];
    assert_eq!(
        fresh(&probes[1]).error.as_deref(),
        Some("f/1 exceeded maximum recursion depth")
    );
    assert_eq!(
        fresh(&probes[4]).error.as_deref(),
        Some("h/1 exceeded maximum recursion depth")
    );
    for (dirty, stop, ended) in [
        // Refused at the byte budget with its data and frame stacks at their largest.
        (
            probes[1].clone(),
            None,
            Some("f/1 exceeded maximum recursion depth"),
        ),
        // Refused at the byte budget with a fork point live at every level.
        (
            probes[4].clone(),
            None,
            Some("h/1 exceeded maximum recursion depth"),
        ),
        // An error raised with fork points live.
        (String::from("-40"), None, Some("boom")),
        // A consumer that stopped with every fork point live.
        (String::from("-40"), Some(3), None),
        // Refused at the loop cap.
        (
            String::from("-1001000"),
            None,
            Some("t/0 exceeded maximum recursion depth"),
        ),
    ] {
        let machine = decide(&program(filter)).expect("takes");
        let first = kept_run(limits, &machine, &dirty, stop);
        assert_eq!(first.error.as_deref(), ended, "{dirty}");
        assert_eq!(first.outputs.len(), stop.unwrap_or(first.outputs.len()));
        // The vectors are kept, emptied.
        assert!(kept_bytes(&machine).iter().any(|&b| b > 0), "{dirty}");
        // Every document after it, on the same machine, answers as on a fresh machine, at and
        // past each bound, and the dirty document again answers as it did.
        for input in probes.iter().chain([&dirty]) {
            assert_eq!(
                kept_run(limits, &machine, input, None),
                fresh(input),
                "{input} after {dirty}"
            );
        }
    }
}

/// A document that grew its stacks near the budget leaves at most `KEEP_BYTES` of each for the
/// next one: what a run holds on to between documents stays small, and uncharged memory with it.
#[test]
fn a_kept_vector_is_cut_back_after_a_large_document_7977() {
    let machine = decide(&program(
        "def f($n): if $n == 0 then 0 else f($n - 1) + 1 end; f(.)",
    ))
    .expect("takes");
    let answer = kept_run(SHIPPED, &machine, "100000", None);
    assert_eq!(answer.outputs, ["100000"]);
    for bytes in kept_bytes(&machine) {
        assert!(bytes <= KEEP_BYTES, "{bytes}");
    }
    // A small document after it keeps what it grew, and answers.
    assert_eq!(kept_run(SHIPPED, &machine, "3", None).outputs, ["3"]);
    assert!(kept_bytes(&machine).iter().any(|&b| b > 0));
}

/// The code a run keeps answers each document as a fresh compilation does: nothing in it changes
/// as it runs (#7977 step 5), and labels keep counting across documents in the interpreter.
#[test]
fn one_compilation_answers_every_document_7977() {
    let filter = "def f($n): if $n == 0 then [label $out | 1, break $out] else f($n - 1) end; \
                  [f(. % 3)], ([.] as $a | $a | setpath([0]; 9)), [label $l | try break $l catch .]";
    let program = program(filter);
    let machine = Rc::new(decide(&program).expect("takes"));
    let run_on = |code: &vm::Compiled, input: i64| {
        with_prelude(|prelude| {
            let it = Interp::new(prelude);
            it.next_label.set(LabelNumbers::resume(&program));
            let mut input = OwnedValue::Int(input);
            it.prepare(&program, &mut input);
            let mut outputs = Vec::new();
            vm::run(&it, code, input, &mut |v| {
                outputs.push(v.to_json_jq_preserve());
                true
            })
            .unwrap_or_else(|_| panic!("{filter}"));
            LabelNumbers::keep(&program, it.next_label.get());
            outputs
        })
    };
    let mut kept = Vec::new();
    for input in 0..6 {
        kept.push(run_on(&machine, input));
    }
    LabelNumbers::keep(&program, 0);
    let mut fresh = Vec::new();
    for input in 0..6 {
        fresh.push(run_on(&decide(&program).expect("takes"), input));
    }
    assert_eq!(kept, fresh);
    // jq 1.7.1 over `0 1 2 3 4 5`: the label value counts on from one document to the next.
    // `no_std` keeps no count across documents (`LabelNumbers`), so there each starts at 0.
    for (input, outputs) in kept.iter().enumerate() {
        let n = if cfg!(feature = "std") {
            2 * input + 1
        } else {
            1
        };
        let label = format!(r#"[{{"__jq":{n}}}]"#);
        assert_eq!(outputs, &["[[1]]", "[9]", label.as_str()], "{input}");
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
                at = block.next as isize;
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
        // Over step 1's leaves, where the machine's own code holds the moves.
        let moved = machine_built(SHIPPED, &filter, Build::NARROW);
        let copied = machine_built(SHIPPED, &filter, Build { moves: false, ..Build::NARROW });
        prop_assert_eq!(moved.is_ok(), copied.is_ok(), "{}", filter);
        if let (Ok(moved), Ok(copied)) = (&moved, copied) {
            prop_assert_eq!(moved, &copied, "{}", filter);
        }
        // And the shipped build, whose leaves take some of them, answers the same.
        if let (Ok(moved), Ok(leafed)) = (moved, machine_built(SHIPPED, &filter, Build::SHIPPED)) {
            prop_assert_eq!(moved, leafed, "{}", filter);
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
        let own = own_instructions_built(&filter, Build::NARROW);
        last += usize::from(own.contains("LoadLast"));
        kill += usize::from(own.contains("Kill"));
    }
    assert!(
        last > 20 && kill > 20,
        "LoadLast in {last}, Kill in {kill} of 200"
    );
}

/// Limits small enough for a unit test: `rounds` rounds, `outputs` outputs and `bytes` bytes.
fn small(bytes: usize, rounds: u32, outputs: u32) -> Limits {
    Limits {
        bytes,
        rounds,
        outputs,
        range: MAX_RANGE_TOTAL,
        memory: None,
    }
}

/// The answer of a refusal: the outputs before it, then the error.
fn refused(outputs: usize, error: &str) -> Answer {
    Answer {
        outputs: (0..outputs).map(|n| n.to_string()).collect(),
        error: Some(error.to_string()),
    }
}

#[test]
fn the_machine_keeps_the_shipped_bounds_7977() {
    assert_eq!(SHIPPED.bytes, 1 << 30);
    assert_eq!(SHIPPED.rounds as usize, MAX_LOOP_ROUNDS);
    assert_eq!(SHIPPED.outputs as usize, MAX_LOOP_OUTPUTS);
    assert_eq!(SHIPPED.range, MAX_RANGE_TOTAL);
}

/// A loop that never deepens is refused after its rounds with the shipped wording (decision 3 of
/// #7977), whatever carries it: no argument, a `$` argument, a forwarded closure, an operator a
/// `def` replaces, and the prelude's `until`, `while` and `repeat`. jq spins on each; the shipped
/// evaluators refuse each with the same message (captured from `main`'s release binary).
#[test]
fn a_loop_that_never_deepens_is_refused_after_its_rounds_7977() {
    let limits = small(BUDGET, 1000, 1 << 20);
    for (filter, error) in [
        ("def f: f; f", "f/0 exceeded maximum recursion depth"),
        (
            "def f($n): f($n); f(0)",
            "f/1 exceeded maximum recursion depth",
        ),
        (
            "def f(g): f(g); f(.)",
            "f/1 exceeded maximum recursion depth",
        ),
        (
            "def f: select(true) | (. + 1 | f); 0 | f",
            "f/0 exceeded maximum recursion depth",
        ),
        (
            "def _negate: -.; 1 | -.",
            "_negate/0 exceeded maximum recursion depth",
        ),
        (
            "def _plus(a; b): a + b; 1 | . + 1",
            "_plus/2 exceeded maximum recursion depth",
        ),
        (
            "[0 | until(false; . + 1)]",
            "until: maximum iterations exceeded",
        ),
        (
            "[0 | while(true; . + 1)] | length",
            "while: maximum iterations exceeded",
        ),
        (
            "[0 | repeat(. + 1)] | length",
            "repeat: maximum iterations exceeded",
        ),
        // Uncatchable, as the shipped refusal is.
        (
            r#"[try (def f: f; f) catch "caught"]"#,
            "f/0 exceeded maximum recursion depth",
        ),
    ] {
        assert_eq!(
            machine_within(limits, filter),
            refused(0, error),
            "{filter}"
        );
    }
}

/// The rounds are a loop's, counted from the ordinary call that started it: a loop within them
/// answers, however many loops a run starts, and one past them is refused even if it would end.
#[test]
fn the_rounds_are_counted_per_loop_7977() {
    let limits = small(BUDGET, 1000, 1 << 20);
    let to = |n: usize| format!("def f($n): if $n < {n} then f($n + 1) else $n end; [f(0)]");
    // `[f(0)]`'s call is an ordinary one, so the loop's rounds are its 999 tail calls.
    assert_eq!(machine_within(limits, &to(1000)), ok(&["[1000]"]));
    assert_eq!(
        machine_within(limits, &to(1001)),
        refused(0, "f/1 exceeded maximum recursion depth")
    );
    assert_eq!(
        machine_within(
            limits,
            "def f($n): if $n < 900 then f($n + 1) else $n end; [range(5) | f(0)] | add"
        ),
        ok(&["4500"])
    );
    assert_eq!(
        machine_within(limits, "[0 | until(. >= 900; . + 1)]"),
        ok(&["[900]"])
    );
    // The tail call that enters a loop is not one of its rounds: exactly `limits.rounds` rounds
    // answer, as `def f: if . >= 16777216 then . else . + 1 | f end; 0 | f` answers on the
    // shipped routes.
    assert_eq!(
        machine_within(
            limits,
            "def f: if . >= 1000 then . else . + 1 | f end; 0 | f"
        ),
        ok(&["1000"])
    );
    assert_eq!(
        machine_within(
            limits,
            "def f: if . >= 1001 then . else . + 1 | f end; 0 | f"
        ),
        refused(0, "f/0 exceeded maximum recursion depth")
    );
}

/// A zero-argument `def`'s tail loop is refused after its outputs once it has run
/// `OUTPUT_BUDGET_AFTER_ROUNDS` rounds, as the shipped tail loop is (#5379), after the same number
/// of outputs: the shipped binary prints 2^20 + 1 lines of `def f: ., (. + 1 | f); 0 | f` before
/// `f/0: maximum outputs exceeded`. A consumer that stops is not charged for what it never asks
/// for, and a loop with arguments is bounded by its rounds alone.
#[test]
fn a_tail_generator_is_refused_after_its_outputs_7977() {
    let limits = small(BUDGET, 1 << 20, 100);
    assert_eq!(
        machine_within(limits, "def f: ., (. + 1 | f); 0 | f"),
        refused(101, "f/0: maximum outputs exceeded")
    );
    assert_eq!(
        machine_within(limits, "[def f: ., (. + 1 | f); 0 | f] | length"),
        refused(0, "f/0: maximum outputs exceeded")
    );
    assert_eq!(
        machine_within(
            limits,
            "first(def f: ., (. + 1 | f); 0 | f | select(. > 50))"
        ),
        ok(&["51"])
    );
    // A helper's outputs in front of the call are the loop's, as the shipped loop counts them,
    // and a loop of the helper's own is not one of the caller's rounds.
    assert_eq!(
        machine_within(limits, "def g: .; def f: g, (. + 1 | f); [0 | f] | length"),
        refused(0, "f/0: maximum outputs exceeded")
    );
    assert_eq!(
        machine_within(
            small(BUDGET, 1000, 1 << 20),
            "def g: if . < 900 then . + 1 | g else . end; def f($n): if $n < 900 then (0 | g) as $x | f($n + 1) else $n end; f(0)"
        ),
        ok(&["900"])
    );
    let limits = small(BUDGET, 1000, 100);
    assert_eq!(
        machine_within(limits, "def f($n): $n, f($n + 1); [f(0)] | length"),
        refused(0, "f/1 exceeded maximum recursion depth")
    );
    // The prelude's loops keep their own wording.
    assert_eq!(
        machine_within(limits, "[0 | repeat(. + 1)] | length"),
        refused(0, "repeat: maximum iterations exceeded")
    );
}

/// Past the byte budget a recursion that deepens is refused naming the innermost definition.
/// The budget counts the control state only: what `[...]` collects and what a leaf buffers are
/// values the program builds, which the shipped evaluators and jq hold as well.
#[test]
fn past_the_byte_budget_a_run_is_refused_7977() {
    let limits = small(1 << 20, 1 << 24, 1 << 20);
    assert_eq!(
        machine_within(limits, "def f: [f]; f"),
        refused(0, "f/0 exceeded maximum recursion depth")
    );
    assert_eq!(
        machine_within(limits, "def f: 1 + f; f"),
        refused(0, "f/0 exceeded maximum recursion depth")
    );
    // An accumulator of 100,000 values is past a MiB at any `OwnedValue` size the machine runs
    // on, and so are 160,000 buffered outputs of one leaf; neither is control state.
    assert!(core::mem::size_of::<OwnedValue>() * 100_000 > 1 << 20);
    assert_eq!(
        machine_within(limits, "def f: .; [range(100000)] | length"),
        ok(&["100000"])
    );
    let ints = (1..=400)
        .map(|n| n.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let leaf = format!("first({{a: ({ints}), b: ({ints})}}) | .a");
    assert_eq!(machine_within(limits, &leaf), ok(&["1"]));
}

/// The budget check of a call and of a fork point, each driven on its own: a call whose frame
/// does not fit is refused naming the definition called (the innermost named one when the callee
/// is a closure), and a fork point that does not fit names the innermost definition.
#[test]
fn a_call_or_a_fork_point_past_the_budget_is_refused_7977() {
    let program = program("def f: f; def g(h): h; f, g(. as $x | $x)");
    with_prelude(|prelude| {
        let it = Interp::new(prelude);
        let code = compile::compile(&it, &program).expect("compiles");
        let func = |name: &str| {
            code.funcs
                .iter()
                .position(|f| &*f.name == name)
                .expect("compiled") as u32
        };
        let (f, g) = (func("f"), func("g"));
        let lambda = code
            .insns
            .iter()
            .find_map(|insn| match insn {
                Insn::Call { args, .. } => args.iter().find_map(|a| match a {
                    ArgSpec::Lambda(l) => Some(*l),
                    _ => None,
                }),
                _ => None,
            })
            .expect("g's argument");
        assert_eq!(code.funcs[lambda as usize].nslots, 1, "`$x`");
        let message = |raise: Option<Raise>| match raise {
            Some(Raise::Fatal(error)) => error.message,
            other => panic!("not a refusal: {other:?}"),
        };
        // A budget that holds the main frame and its input, with no room for a slot.
        let mut vm = Vm::new(&it, &code, small(usize::MAX, 1 << 24, 1 << 20));
        vm.push_frame(code.main, -1, (NO_RET, -1), Counts::fresh(code.main))
            .expect("fits");
        vm.push(OwnedValue::Null).expect("fits");
        vm.limits.bytes = vm.bytes();
        let call = |vm: &mut Vm, callee, args: &[ArgSpec]| {
            let callee = Callee::Def {
                func: callee,
                level: NO_LEVEL,
            };
            // The call's input, which a refused call leaves popped.
            if vm.data.index < 0 {
                vm.push(OwnedValue::Null).expect("fits");
            }
            vm.call(0, callee, args, Tail::No).err()
        };
        // `f` has no slots, so its frame fits.
        assert!(call(&mut vm, f, &[]).is_none());
        vm.pop_frame();
        assert_eq!(
            message(call(&mut vm, g, &[ArgSpec::Lambda(lambda)])),
            "g/1 exceeded maximum recursion depth"
        );
        // A closure is named by the innermost definition, or by none.
        assert_eq!(
            message(call(&mut vm, lambda, &[])),
            "exceeded maximum recursion depth"
        );
        assert!(call(&mut vm, f, &[]).is_none());
        assert_eq!(
            message(call(&mut vm, lambda, &[])),
            "f/0 exceeded maximum recursion depth"
        );
        // A fork point: the fork stack is empty, so its first growth is past the budget.
        assert!(vm.forks.is_empty());
        assert_eq!(
            message(vm.fork(Kind::Branch(0)).err()),
            "f/0 exceeded maximum recursion depth"
        );
        assert!(vm.forks.is_empty());
    });
}

/// A growth past the budget is refused before it is made, and a failed allocation is refused
/// too, for the stacks and for an accumulator alike, which is what stops a run under a cap on the
/// address space (#5634, #7733) where an infallible one would abort.
#[test]
fn a_growth_past_the_budget_or_the_allocator_is_refused_7977() {
    let mut v: Vec<u64> = Vec::new();
    let mut cap = 0;
    assert_eq!(grow(&mut v, &mut cap, 0, 31), Err(()));
    assert_eq!(grow(&mut v, &mut cap, 0, 32), Ok(32));
    assert_eq!((v.capacity(), cap), (4, 4));
    // From 4 to 8: the old block (32 bytes, in `used`) and the new one (64) are held at once.
    assert_eq!(grow(&mut v, &mut cap, 32, 95), Err(()));
    assert_eq!(grow(&mut v, &mut cap, 32, 96), Ok(32));
    assert_eq!(
        grow(&mut v, &mut cap, 64 + 8, 64 + 128),
        Err(()),
        "the bytes held already count"
    );
    // A vector kept with room is charged as a fresh one grows, and allocates nothing until the
    // charge passes its capacity.
    let mut kept: Vec<u64> = Vec::with_capacity(64);
    let mut cap = 0;
    assert_eq!(grow(&mut kept, &mut cap, 0, 31), Err(()), "charged from 0");
    assert_eq!(grow(&mut kept, &mut cap, 0, 32), Ok(32));
    assert_eq!((kept.capacity(), cap), (64, 4));
    // One element of 2^47 bytes, and four: past any address space, so the allocator refuses.
    #[cfg(target_pointer_width = "64")]
    {
        let mut huge: Vec<[u64; 1 << 44]> = Vec::new();
        assert_eq!(grow(&mut huge, &mut 0, 0, usize::MAX), Err(()));
        assert_eq!(reserve(&mut huge), Err(()));
    }
    let mut items: Vec<OwnedValue> = Vec::new();
    assert_eq!(reserve(&mut items), Ok(()));
    assert!(items.capacity() >= 1);
}

/// `range` keeps the shipped cap (#5292), `range/2` on the machine's own `RANGE` and `range/3` on
/// the prelude's `while`, each with the shipped wording; a consumer that stops is not charged.
/// Under step 1's leaves, so that a test's cap reaches every `range`.
#[test]
fn a_range_past_its_cap_is_refused_as_the_shipped_range_is_7977() {
    let limits = Limits {
        range: 50,
        ..small(BUDGET, 1 << 24, 1 << 20)
    };
    let error = "range: maximum iterations exceeded";
    // The machine's own `RANGE` (a `[range(n)]` is a leaf, whose `range` is `Interp::eval`'s,
    // which keeps the same cap of its own).
    let narrow = |limits, filter| machine_built(limits, filter, Build::NARROW).expect(filter);
    assert_eq!(narrow(limits, "[range(50)] | length"), ok(&["50"]));
    assert_eq!(narrow(limits, "[range(51)]"), refused(0, error));
    assert_eq!(narrow(limits, "[range(0; 40; 1)] | length"), ok(&["40"]));
    assert_eq!(narrow(limits, "[range(0; 100; 1)]"), refused(0, error));
    assert_eq!(narrow(limits, "[limit(3; range(1e9))]"), ok(&["[0,1,2]"]));
    // `range/3`'s `while` is its own: a program's own `while` keeps `while`'s cap and wording.
    assert_eq!(
        narrow(limits, "[0 | while(. < 100; . + 1)] | length"),
        ok(&["100"])
    );
}

/// `repeat` keeps the shipped evaluators' two regimes (#2014): an eager one, whose consumer takes
/// every output (`[...]`, `reduce`), is refused after `MAX_REPEAT_ROUNDS` rounds; one a consumer
/// can stop (`limit`, `first`) runs unbounded, and its stream ends after
/// `MAX_EMPTY_REPEAT_ROUNDS` rounds in a row that output nothing (`[limit(3; repeat(empty))]` is
/// `[]` on the shipped routes; jq 1.7.1 never ends there).
#[test]
fn repeat_keeps_the_shipped_eager_and_demand_driven_bounds_7977() {
    let limits = small(BUDGET, 1000, 1 << 20);
    let error = "repeat: maximum iterations exceeded";
    for filter in [
        "[repeat(1)] | length",
        "reduce repeat(1) as $x (0; . + 1)",
        "limit(5; [repeat(1)])",
        "def f: f; [repeat(1, 2)] | length",
    ] {
        assert_eq!(
            machine_within(limits, filter),
            refused(0, error),
            "{filter}"
        );
    }
    // Past `limits.rounds` too: a demand-driven `repeat` is not charged rounds.
    assert_eq!(
        machine_within(limits, "[limit(5000; repeat(1))] | length"),
        ok(&["5000"])
    );
    assert_eq!(
        machine_within(limits, "[limit(3; repeat(empty))]"),
        ok(&["[]"])
    );
    assert_eq!(
        machine_within(limits, "first(repeat(empty), 1)"),
        ok(&["1"])
    );
    // A round whose closure calls a definition that returns, but outputs nothing, is empty.
    assert_eq!(
        machine_within(limits, "[limit(3; repeat(def g: .; g | empty))]"),
        ok(&["[]"])
    );
    // A loop inside a round is a loop of its own.
    assert_eq!(
        machine_within(
            limits,
            "[limit(3; repeat(def g: if . < 900 then . + 1 | g else . end; 0 | g))]"
        ),
        ok(&["[900,900,900]"])
    );
}

/// A closure chain is not bounded but by the byte budget: the machine follows jq, which answers
/// a finite chain however deep (#7870 is one of the issues the machine is for). The shipped
/// evaluators refuse these at their native-stack floor. Answers captured from jq 1.7.1.
#[test]
fn a_closure_chain_answers_as_jq_does_7977() {
    for (filter, want) in [
        (
            "def f(g; $n): if $n <= 0 then g else f(g + (1,3); $n - 1) end; \
             first(f((1,3); 128000)) | type",
            r#""number""#,
        ),
        // Two closures a level, through a nested helper.
        (
            "def f(g; $n): def h(q): f(q + 1; $n - 1); if $n == 0 then g else h(g + 1) end; \
             0 | f(.; 9000)",
            "18000",
        ),
        (
            "def f: if . > 0 then [. - 1] | map(f) | .[0] else . end; 100000 | f",
            "0",
        ),
    ] {
        assert_eq!(machine(filter), ok(&[want]), "{filter}");
    }
}

/// The prelude loops keep the shipped evaluators' caps exactly, counting the round that starts
/// the loop as they do: `0 | until(. >= 16777215; . + 1)` answers on the shipped routes and
/// `16777216` is refused (#5292), and `range/3` makes `MAX_RANGE_TOTAL` numbers and no more.
#[test]
fn the_prelude_loops_keep_the_shipped_caps_exactly_7977() {
    let limits = small(BUDGET, 1000, 1 << 20);
    assert_eq!(
        machine_within(limits, "0 | until(. >= 999; . + 1)"),
        ok(&["999"])
    );
    assert_eq!(
        machine_within(limits, "0 | until(. >= 1000; . + 1)"),
        refused(0, "until: maximum iterations exceeded")
    );
    assert_eq!(
        machine_within(limits, "[0 | while(. < 999; . + 1)] | length"),
        ok(&["999"])
    );
    assert_eq!(
        machine_within(limits, "[0 | while(. < 1000; . + 1)] | length"),
        refused(0, "while: maximum iterations exceeded")
    );
    let limits = Limits {
        range: 50,
        ..small(BUDGET, 1 << 24, 1 << 20)
    };
    assert_eq!(
        machine_within(limits, "[limit(50; range(0; 1e9; 1))] | length"),
        ok(&["50"])
    );
    // Exactly `MAX_RANGE_TOTAL` numbers answer: the failing comparison after the last is not
    // counted (`[range(0; 16777216; 1)] | length` answers on the shipped routes).
    assert_eq!(
        machine_within(limits, "[range(0; 50; 1)] | length"),
        ok(&["50"])
    );
    assert_eq!(
        machine_within(limits, "[range(50; 0; -1)] | last"),
        ok(&["1"])
    );
    assert_eq!(
        machine_within(limits, "[limit(51; range(0; 1e9; 1))] | length"),
        refused(0, "range: maximum iterations exceeded")
    );
    assert_eq!(
        machine_within(limits, "first(range(0; 1e9; 1) | select(. >= 49))"),
        ok(&["49"])
    );
    assert_eq!(
        machine_within(limits, "first(range(0; 1e9; 1) | select(. >= 50))"),
        refused(0, "range: maximum iterations exceeded")
    );
}

/// An eager `repeat` is told by its nearest consumer while the `repeat` runs inside it, as the
/// shipped evaluators tell it (answers captured from `main` with the switch unset; where `main`
/// never ends, the machine's refusal is noted). A label whose body has already output is no
/// longer the consumer, nor is a user's own label, which stops nothing by itself. An eager one
/// is bounded by `REPEAT_WIDTH_BUDGET` outputs as well as `MAX_REPEAT_ROUNDS` rounds.
#[test]
fn an_eager_repeat_is_told_by_the_consumer_it_runs_in_7977() {
    let error = "repeat: maximum iterations exceeded";
    for (filter, want) in [
        ("[first(repeat(1))]", ok(&["[1]"])),
        ("reduce limit(5; repeat(1)) as $x (0; . + $x)", ok(&["5"])),
        ("first([repeat(1)])", refused(0, error)),
        (
            "label $l | [repeat(1) | if . then break $l else . end]",
            ok(&[]),
        ),
        ("[repeat(1)] | first", refused(0, error)),
        ("[first(1) | repeat(1)] | length", refused(0, error)),
        (
            "[1 | . as $x | label $l | repeat(1)] | length",
            refused(0, error),
        ),
        // `main` never ends on these two; the machine refuses them as eager.
        ("[repeat(1) as $x | $x]", refused(0, error)),
        ("[def h: repeat(1); h]", refused(0, error)),
        // 10,000 outputs in the first round: `main` refuses at once.
        ("[repeat(range(1e6))] | length", refused(0, error)),
    ] {
        assert_eq!(machine(filter), want, "{filter}");
    }
}

/// A refusal names the user's definition, past the prelude's own frames, as the shipped
/// evaluators name it (captured from `main`): `first/1`, `limit/2` and `to_entries/0` are the
/// prelude's.
#[test]
fn a_refusal_names_the_users_definition_past_the_preludes_7977() {
    let limits = small(1 << 20, 1 << 24, 1 << 20);
    for (filter, error) in [
        ("def f: first(f); f", "f/0 exceeded maximum recursion depth"),
        (
            "def f: limit(1; f); f",
            "f/0 exceeded maximum recursion depth",
        ),
        (
            "def f: isempty(f); f",
            "f/0 exceeded maximum recursion depth",
        ),
        (
            "def f: any(f; .); f",
            "f/0 exceeded maximum recursion depth",
        ),
        (
            "def f: to_entries | map(f); {a: 1} | f",
            "f/0 exceeded maximum recursion depth",
        ),
        (
            "def f($n): if $n == 0 then 0 else first(f($n - 1) + 1) end; f(1000000)",
            "f/1 exceeded maximum recursion depth",
        ),
    ] {
        assert_eq!(
            machine_within(limits, filter),
            refused(0, error),
            "{filter}"
        );
    }
}

/// The instructions a run takes.
fn steps(filter: &str) -> u64 {
    let program = program(filter);
    with_prelude(|prelude| {
        let it = Interp::new(prelude);
        let code = compile::compile(&it, &program).unwrap_or_else(|e| panic!("{filter}: {e}"));
        let mut vm = Vm::new(&it, &code, SHIPPED);
        let result = vm.exec(OwnedValue::Null, &mut |_| true);
        assert!(result.is_ok(), "{filter}: {result:?}");
        vm.steps
    })
}

/// A loop through a nested `def` with a fork point pending returns each output at once
/// (`Tail::Keep`), not through every frame the loop has kept, so its work is linear in its
/// length: it was quadratic (24.9 s at 100,000), where the shipped evaluators answer at once.
#[test]
fn a_loop_through_a_nested_def_runs_in_linear_work_7977() {
    let at = |n: u32| {
        steps(&format!(
            "[def f: def g: if . < {n} then ., (. + 1 | f) else empty end; g; 0 | f] | length"
        ))
    };
    let (small, large) = (at(4000), at(16000));
    assert!(
        large < 5 * small,
        "4x the length took {large} steps against {small}"
    );
    // The loop's caps follow the shipped loop's: a nested definition is a round, and one that
    // declares a definition costs `BOUND_ROUND_WEIGHT` rounds.
    assert_eq!(
        machine(
            "[def f: def g: if . < 300000 then ., (. + 1 | f) else empty end; g; 0 | f] | length"
        ),
        ok(&["300000"])
    );
}

/// Where the machine and the shipped evaluators bound a program differently, a different bound
/// is met first: `h`'s loop is the machine's, whose outputs count against `f`'s, where the
/// shipped evaluators nest `h`'s calls and meet their native-stack floor (`h/1 exceeded`).
#[test]
fn a_different_bound_can_be_met_first_7977() {
    assert_eq!(
        machine(
            "def h($m): if $m < 16132 then $m, h($m + 1) else empty end; \
             def f: if . < 65 then h(0), (. + 1 | f) else empty end; [0 | f] | length"
        ),
        refused(0, "f/0: maximum outputs exceeded")
    );
}

/// The output bound starts after `OUTPUT_BUDGET_AFTER_ROUNDS` tail rounds counted one each,
/// whatever a round costs against the round cap (`BOUND_ROUND_WEIGHT` for a definition that
/// declares one): the shipped tail loop answers 64 rounds of `range(17000)` with or without a
/// nested `def`, and refuses 65.
#[test]
fn the_output_bound_starts_after_unweighted_rounds_7977() {
    let limits = small(BUDGET, 1 << 24, 100);
    for nested in ["", "def g: .; "] {
        let at = |n: u32| {
            format!(
                "def f: {nested}if . < {n} then range(2), (. + 1 | f) else empty end; [0 | f] | length"
            )
        };
        assert_eq!(machine_within(limits, &at(64)), ok(&["128"]), "{nested}");
        assert_eq!(
            machine_within(limits, &at(65)),
            refused(0, "f/0: maximum outputs exceeded"),
            "{nested}"
        );
    }
}

/// A prelude loop whose update leaves a fork point that can only answer nothing (`. + 1, empty`,
/// or `//` once its left side gave a truthy output) runs in constant control state: the dead fork
/// point is dropped at the next round's tail call. Within a 1 MiB budget, where a frame a round
/// would be refused long before; the shipped evaluators and jq answer these too.
#[test]
fn a_loop_whose_update_leaves_a_dead_fork_runs_in_constant_state_7977() {
    let limits = small(1 << 20, 1 << 24, 1 << 20);
    for (filter, want) in [
        ("0 | until(. >= 100000; (. + 1) // 0)", "100000"),
        ("0 | until(. >= 100000; . + 1, empty)", "100000"),
        ("[0 | while(. < 100000; . + 1, empty)] | length", "100000"),
        ("[0 | while(. < 100000; (. + 1) // 0)] | length", "100000"),
    ] {
        assert_eq!(machine_within(limits, filter), ok(&[want]), "{filter}");
    }
    // A fork point that can still answer is kept: `//`'s default runs when the left side gave
    // nothing truthy.
    assert_eq!(
        machine("[0 | until(. >= 3; (null // . + 1))]"),
        ok(&["[3]"])
    );
}

/// A tail call to a helper that takes a closure argument (a call that keeps the caller's frame)
/// is not one of the caller's rounds: the helper's loop counts its own, and its outputs are
/// charged to the caller's loop, as a helper's are.
#[test]
fn a_helper_given_a_closure_keeps_its_own_count_7977() {
    let limits = small(BUDGET, 1000, 100);
    assert_eq!(
        machine_within(
            limits,
            "def h(g): if . >= 100 then g else . + 1 | h(g) end; \
             def f: if . < 990 then . + 1 | f else 0 | h(.) end; 0 | f"
        ),
        ok(&["100"])
    );
    assert_eq!(
        machine_within(
            limits,
            "def g: range(15); \
             def f: if . < 100 then (g | select(true)), (. + 1 | f) else empty end; [0 | f] | length"
        ),
        refused(0, "f/0: maximum outputs exceeded")
    );
}

/// Whether `filter`'s own code holds a [`Insn::TailJump`].
fn jumps(filter: &str) -> bool {
    own_instructions(filter).contains("TailJump")
}

/// A self tail call of a zero-argument `def` compiles to a jump when nothing makes a closure over
/// the frame it reuses: not when the body passes a closure argument (which closes over it) or
/// declares a `def`, and not for a call that is not in tail position (under `try` or `label`,
/// or an operand). `$__loc__` is a constant, and a finished `reduce` or `foreach` leaves nothing
/// over the frame, so those jump; a fork point the round leaves protects the frame, and the jump
/// is then the ordinary tail call. Every answer is jq 1.7.1's.
#[test]
fn a_zero_argument_self_tail_call_compiles_to_a_jump_7977() {
    for (filter, jump, want) in [
        ("def f: if . < 3 then . + 1 | f else . end; 0 | f", true, "3"),
        (
            "def f: if . < 3 then . + 1 | f else $__loc__ end; 0 | f",
            true,
            r#"{"file":"<top-level>","line":1}"#,
        ),
        (
            "def f: if . < 10 then reduce (1, 2) as $x (.; . + $x) | f else . end; 0 | f",
            true,
            "12",
        ),
        (
            "[def f: foreach (1, 2) as $x (.; . + $x; if . < 10 then f else . end); 0 | f] | length",
            true,
            "60",
        ),
        (
            "[def f: if . < 3 then (. + 1, . + 10) | f else . end; 0 | f]",
            true,
            "[3,12,11,10]",
        ),
        (
            "[def f: if . < 3 then ., (. + 1 | f) else . end; 0 | f]",
            true,
            "[0,1,2,3]",
        ),
        (
            "def f: if . < 5 then (label $x | (. + 1, break $x)) | f else . end; 0 | f",
            true,
            "5",
        ),
        // A closure argument closes over the frame, and so does a `def` the body declares.
        (
            "def f: if . < 3 then (. + 1 | first(.)) | f else . end; 0 | f",
            false,
            "3",
        ),
        (
            "def f: def g: . + 1; if . < 3 then g | f else . end; 0 | f",
            false,
            "3",
        ),
        // Not a tail call.
        (
            "def f: try (if . < 3 then . + 1 | f else . end); 0 | f",
            false,
            "3",
        ),
        (
            "def f: label $x | if . < 3 then . + 1 | f else . end; 0 | f",
            false,
            "3",
        ),
        (
            "def f: if . < 3 then (. + 1 | f) + 0 else . end; 0 | f",
            false,
            "3",
        ),
        // Arguments: not a zero-argument definition.
        (
            "def f($n): if $n < 3 then f($n + 1) else $n end; f(0)",
            false,
            "3",
        ),
    ] {
        assert_eq!(jumps(filter), jump, "{filter}");
        assert_eq!(machine(filter), ok(&[want]), "{filter}");
    }
    // The prelude's `until` and `while` loop through a zero-argument `def` of their own.
    let until = "def f: f; 0 | until(. >= 3; . + 1)";
    let program = program(until);
    with_prelude(|prelude| {
        let it = Interp::new(prelude);
        let code = compile::compile(&it, &program).expect("compiles");
        assert!(code
            .insns
            .iter()
            .any(|i| matches!(i, Insn::TailJump { .. })));
    });
}

/// The tail jump keeps every bound of the call it replaces: the loop cap refuses at the same
/// round with the same wording, the output bound starts after the same unweighted rounds, and
/// the dead fork points a round leaves are dropped as the call drops them. Each row answers the
/// same compiled with jumps and without, around each bound.
#[test]
fn a_tail_jump_meets_each_bound_where_the_call_does_7977() {
    let limits = small(1 << 20, 100, 70);
    let rows = [
        "def f: if . < {n} then . + 1 | f else . end; 0 | f",
        "def f: if . < {n} then (. + 1, empty) | f else . end; 0 | f",
        "def f: if . < {n} then ((. + 1) // 0) | f else . end; 0 | f",
        "def f: if . < {n} then (try (. + 1) catch 0) | f else . end; 0 | f",
        "def f: if . < {n} then range(2), (. + 1 | f) else empty end; [0 | f] | length",
        "def f: if . < {n} then . as $x | $x + 1 | f else . end; [0 | f] | length",
        "def f: f; 0 | until(. >= {n}; . + 1)",
        "def f: f; [0 | while(. < {n}; . + 1)] | length",
        "def g: def f: if . < {n} then . + 1 | f else . end; f; 0 | g",
    ];
    let mut refusals = BTreeSet::new();
    for row in rows {
        for n in [34, 35, 36, 64, 65, 66, 98, 99, 100, 101, 102] {
            let filter = row.replace("{n}", &n.to_string());
            let jumped = machine_built(limits, &filter, Build::SHIPPED);
            let called = machine_built(
                limits,
                &filter,
                Build {
                    jumps: false,
                    ..Build::SHIPPED
                },
            );
            assert_eq!(jumped, called, "{filter}");
            if let Ok(Answer {
                error: Some(error), ..
            }) = jumped
            {
                refusals.insert(error);
            }
        }
    }
    assert_eq!(
        refusals,
        [
            "f/0 exceeded maximum recursion depth",
            "f/0: maximum outputs exceeded",
            // The nested loop is named for the definition it started at, as the shipped loop
            // names it.
            "g/0 exceeded maximum recursion depth",
            "until: maximum iterations exceeded",
            "while: maximum iterations exceeded",
        ]
        .into_iter()
        .map(String::from)
        .collect(),
        "the rows reach every bound"
    );
}

/// A loop whose update leaves a finished `try` or label behind (its `TRY_END` above it, nothing
/// between), or `first`'s label with the `break` that ends it, runs in constant control state:
/// the dead fork points are dropped at the next round. Within a 1 MiB budget, where a frame a
/// round is refused long before (as it was until these were dropped); the shipped evaluators
/// and jq answer these too.
#[test]
fn a_finished_try_or_label_is_dropped_at_the_next_round_7977() {
    let limits = small(1 << 20, 1 << 24, 1 << 20);
    for update in [
        "try (. + 1) catch 0",
        "try (. + 1)",
        "label $x | . + 1",
        "first(. + 1, 0)",
        "label $x | (. + 1, break $x)",
        "label $a | label $b | (. + 1, break $a)",
        "try (label $x | (. + 1, break $x)) catch 99",
        "try (try (. + 1) catch 0) catch 1",
    ] {
        for filter in [
            format!("0 | until(. >= 100000; {update})"),
            format!("[0 | while(. < 100000; {update})] | length"),
            format!("def f: if . < 100000 then ({update}) | f else . end; 0 | f"),
        ] {
            assert_eq!(machine_within(limits, &filter), ok(&["100000"]), "{filter}");
            let kept = machine_built(
                limits,
                &filter,
                Build {
                    drops: false,
                    ..Build::SHIPPED
                },
            );
            let refusal = kept.ok().and_then(|answer| answer.error);
            assert!(
                refusal
                    .as_deref()
                    .is_some_and(|e| e.ends_with("exceeded maximum recursion depth")),
                "{filter}: kept, the fork points meet the budget: {refusal:?}"
            );
        }
    }
}

/// A `try` or label that can still answer is kept: one whose body can still output, whose
/// `catch` can still fire, or whose label a `break` still pending can reach, and a `try` an
/// error raised later passes through. Every answer is jq 1.7.1's.
#[test]
fn a_try_or_label_that_can_still_answer_is_kept_7977() {
    for (filter, want) in [
        (
            "[0 | until(. >= 3; try (. + 1, . + 10) catch 0)]",
            "[3,12,11,10]",
        ),
        (
            r#"[0 | until(. >= 3; try (. + 1, error("x")) catch 100)]"#,
            "[3,100,100,100]",
        ),
        (r#"[0 | until(. >= 3; try (. + 1, error("x")))]"#, "[3]"),
        (
            "[label $out | 0 | until(. >= 3; (. + 1, break $out))]",
            "[3]",
        ),
        (
            "[label $out | 0 | until(. >= 3; label $x | . + 1) | ., break $out]",
            "[3]",
        ),
        (
            "[0 | until(. >= 3; label $x | try (. + 1, break $x) catch 99)]",
            "[3,99,99,99]",
        ),
        (
            r#"try (0 | until(. >= 3; try (. + 1) catch 0) | error("later")) catch "outer""#,
            r#""outer""#,
        ),
        (
            r#"try ([0 | until(. >= 3; label $x | (. + 1, error("e")))]) catch ."#,
            r#""e""#,
        ),
        ("[0 | until(. >= 3; first(. + 1, 0))]", "[3]"),
        (
            "[0 | until(. >= 3; label $a | label $b | (. + 1, break $a))]",
            "[3]",
        ),
    ] {
        assert_eq!(machine(filter), ok(&[want]), "{filter}");
    }
}

/// Zero-argument tail loops and `$` argument loops, plain and nested, whose rounds pass through
/// an update that leaves fork points of every kind behind: `,`, `//`, `try`, label, `first`,
/// and a pending `break` or error.
fn tail_loop_program() -> impl Strategy<Value = String> {
    let once = prop_oneof![
        Just(". + 1"),
        Just("(. + 1, empty)"),
        Just("((. + 1) // 0)"),
        Just("(null // . + 1)"),
        Just("(try (. + 1) catch 0)"),
        Just("(. + 1)?"),
        Just("(label $x | . + 1)"),
        Just("(label $x | (. + 1, break $x))"),
        Just("first(. + 1, 0)"),
        Just("(label $a | label $b | (. + 1, break $a))"),
        Just("(try (label $x | (. + 1, break $x)) catch 99)"),
        Just("(if . == 2 then error(\"two\") else . + 1 end)"),
    ];
    let many = prop_oneof![
        Just("(. + 1, . + 2)"),
        Just("(try (. + 1, error(\"x\")) catch 100)"),
        Just("(try (. + 1, . + 10) catch 0)"),
        Just("(label $x | try (. + 1, break $x) catch 99)"),
        Just("(label $x | (. + 1, error(\"e\")))"),
        Just("((. + 1) as $v | $v, $v + 5)"),
    ];
    let update = prop_oneof![
        (
            once,
            prop_oneof![Just(0u32), Just(1), Just(3), Just(40), Just(150)]
        ),
        (many, prop_oneof![Just(0u32), Just(1), Just(3)]),
    ];
    let shape = prop_oneof![
        Just("def f: if . < N then U | f else . end; [0 | f]"),
        Just("def f: if . >= N then . else U | f end; [0 | f]"),
        Just("def f: f; [0 | until(. >= N; U)]"),
        Just("def f: f; [0 | while(. < N; U)]"),
        Just("def g: def f: if . < N then U | f else . end; f; [0 | g]"),
        Just("def f($n): if $n < N then U | f($n + 1) else . end; [0 | f(0)]"),
        Just("def f: if . < N then U, (. + 3 | f) else . end; [0 | f]"),
        Just("def f: try (if . < N then U | f else . end) catch -1; [0 | f]"),
        Just("[label $out | 0 | def f: if . < N then U | f else ., break $out end; f]"),
        Just("try (0 | def f: if . < N then U | f else . end; f | error(\"later\")) catch ."),
    ];
    (shape, update).prop_map(|(shape, (u, n))| shape.replace('U', u).replace('N', &n.to_string()))
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// The tail jumps and the dead fork points' drops change no answer and no bound (#7977 step
    /// 4): every tail loop answers the same with neither, with either and with both, its loop
    /// cap a hundred rounds, so the longest loops meet it.
    #[test]
    fn tail_jumps_and_dropped_forks_change_no_answer_7977(filter in tail_loop_program()) {
        let limits = small(BUDGET, 100, 1 << 20);
        let plain = machine_built(limits, &filter, Build { jumps: false, drops: false, ..Build::SHIPPED });
        for build in [
            Build::SHIPPED,
            Build { jumps: false, ..Build::SHIPPED },
            Build { drops: false, ..Build::SHIPPED },
        ] {
            prop_assert_eq!(&machine_built(limits, &filter, build), &plain, "{} {:?}", filter, build);
        }
    }
}

/// #7977 step 3: a call-free `[...]` or `reduce` is one leaf with the `.[]` and `range` it holds,
/// and so is a call-free node that holds them only under one (an object constructor, a string
/// interpolation), which the machine declined before: their every output is taken, so the leaf
/// does the work jq does. A generator a consumer can stop stays the machine's: under `first`,
/// `limit` or a label, or with no collector around it. So does one that runs a call or an
/// effect, a `range/2` (`Interp::eval` would start it at its bound as written, which negates to
/// `0`, not jq's `-0`), and a pattern `Interp::eval` binds otherwise than jq. Every answer is jq
/// 1.7.1's, with and without the wide leaves.
#[test]
fn a_collected_generator_is_one_leaf_and_a_stoppable_one_is_not_7977() {
    const R: &str = "def r: if false then r else . end; ";
    let leafed: &[(&str, &[&str])] = &[
        ("[range(5)]", &["[0,1,2,3,4]"]),
        ("[1,2,3] | [.[] | . * 2]", &["[2,4,6]"]),
        ("{a: [1, 2]} | {b: [.a[] | . + 1]}", &[r#"{"b":[2,3]}"#]),
        ("[1,2,3] | reduce .[] as $x (0; . + $x)", &["6"]),
        ("reduce range(5) as $i ([]; . + [$i])", &["[0,1,2,3,4]"]),
        ("[[1,2],[3]] | [.[] | [.[] | . * 10]]", &["[[10,20],[30]]"]),
        (r#"{"a":1,"b":2} | [.[]?] | length"#, &["2"]),
        (r#""\([range(3)])""#, &[r#""[0,1,2]""#]),
        (
            "[[1], 2] | try [.[] | .[]] catch .",
            &[r#""Cannot iterate over number (2)""#],
        ),
        ("[range(3) | -.]", &["[-0,-1,-2]"]),
    ];
    for (filter, want) in leafed {
        let filter = format!("{R}{filter}");
        assert_eq!(machine(&filter), ok(want), "{filter}");
        let own = own_instructions(&filter);
        assert!(
            own.contains("Leaf") && !own.contains("Each") && !own.contains("Range"),
            "{filter}: {own:?}"
        );
    }
    let native: &[(&str, &[&str])] = &[
        ("[1,2,3] | first(.[])", &["1"]),
        ("[1,2,3] | [limit(2; .[])]", &["[1,2]"]),
        ("[1,2,3] | [label $x | .[] | ., break $x]", &["[1]"]),
        ("[1,2,3] | [.[] | r]", &["[1,2,3]"]),
        // `select` is the prelude's: a call.
        ("[3,1,2] | [.[] | select(. > 1)]", &["[3,2]"]),
        ("[range(0; 3) | -.]", &["[-0,-1,-2]"]),
        ("[1,2] | [.[] | debug]", &["[1,2]"]),
        ("[[1,2],[3,4]] | reduce .[] as [$a, $a] (0; . + $a)", &["6"]),
        ("[1,2,3] | .[] | . + 1", &["2", "3", "4"]),
    ];
    for (filter, want) in native {
        let filter = format!("{R}{filter}");
        assert_eq!(machine(&filter), ok(want), "{filter}");
        let own = own_instructions(&filter);
        assert!(
            own.contains("Each") || own.contains("Range"),
            "{filter}: {own:?}"
        );
    }
    // An error inside a leaf ends it where jq's run ends, after nothing is output.
    assert_eq!(
        machine(&format!(
            r#"{R}[range(3) | if . == 1 then error("e") else . end]"#
        )),
        Answer {
            outputs: Vec::new(),
            error: Some(String::from("e")),
        }
    );
    // Step 1's leaves declined the constructor and the interpolation.
    for (filter, _) in [&leafed[2], &leafed[7]] {
        assert!(machine_built(SHIPPED, &format!("{R}{filter}"), Build::NARROW).is_err());
    }
}

/// A leaf reads its free variables from the slots where they are (#7977 step 3), at any level
/// out, under a name the leaf binds again itself, and from a definition's `$` parameter. Every
/// answer is jq 1.7.1's.
#[test]
fn a_leaf_reads_its_variables_from_the_slots_7977() {
    const R: &str = "def r: if false then r else . end; ";
    for (filter, want) in [
        ("2 as $n | [range($n) | . + $n]", "[2,3]"),
        ("1 as $x | [range(2) | (10 as $x | $x), $x]", "[10,1,10,1]"),
        (
            "5 as $k | def g($m): [range($m) | . * $k]; g(3)",
            "[0,5,10]",
        ),
        ("[1,2] as $a | [$a[] | . + ($a | length)]", "[3,4]"),
        (
            "def f($n): if $n == 0 then [range(3) | . + $n] else f($n - 1) end; f(2)",
            "[0,1,2]",
        ),
        (
            "3 as $a | 4 as $b | {s: [$a, $b], t: [range($a) | $b]}",
            r#"{"s":[3,4],"t":[4,4,4]}"#,
        ),
    ] {
        let filter = format!("{R}{filter}");
        assert_eq!(machine(&filter), ok(&[want]), "{filter}");
        // Step 1's leaves answer the same, where they take the program (not the constructor).
        if let Ok(narrow) = machine_built(SHIPPED, &filter, Build::NARROW) {
            assert_eq!(narrow, ok(&[want]), "{filter}");
        }
    }
}

/// Recursive programs whose bodies hold `[...]`, `reduce`, `length`, `.[]`, `range`, `first`,
/// `limit`, a label, `//`, `try` and an effect, in and around what can be a leaf.
fn leaf_program() -> impl Strategy<Value = String> {
    let gen = prop_oneof![
        Just(".[]"),
        Just(".[]?"),
        Just("range(3)"),
        Just("range(.[0]? // 2)"),
        Just("(.[] | . * 2)"),
        Just("(1, 2)"),
        Just("empty"),
    ];
    let wrap = prop_oneof![
        Just("[G]"),
        Just("([G] | length)"),
        Just("reduce G as $x (0; . + $x)"),
        Just("first(G)"),
        Just("[limit(2; G)]"),
        Just("(label $l | G | ., break $l)"),
        Just("(G // 7)"),
        Just("try (G | if . == 2 then error(\"e\") else . end) catch 9"),
        Just("[G | select(. > 0)]"),
        Just("{a: [G]}"),
        Just("[G | debug]"),
        Just("(G)"),
        Just("[G | f($n - 1)]"),
    ];
    (
        gen,
        wrap,
        prop_oneof![Just("[1,2,3]"), Just("[4]"), Just("[]"), Just("5")],
    )
        .prop_map(|(g, w, input)| {
            let body = w.replace('G', g);
            format!("def f($n): if $n <= 0 then . else [{input} | {body}] end; [{input} | f(2)]")
        })
}

std::thread_local! {
    /// What [`fake_memory`] answers on this thread, and how many times it was asked.
    static FAKE_MEMORY: core::cell::Cell<(usize, usize, u32)> = const { core::cell::Cell::new((0, 0, 0)) };
}

/// A memory probe for the tests (#8064): answers the bytes set for this thread, growing by the
/// step set with them at each sample, and counts how often it is asked.
fn fake_memory() -> Option<usize> {
    FAKE_MEMORY.with(|cell| {
        let (bytes, step, asked) = cell.get();
        cell.set((bytes + step, step, asked + 1));
        Some(bytes)
    })
}

/// `limits` with [`fake_memory`] as the probe against `limit`, starting at `bytes` and growing
/// by `step` a sample.
fn probed(limits: Limits, limit: usize, bytes: usize, step: usize) -> Limits {
    FAKE_MEMORY.with(|cell| cell.set((bytes, step, 0)));
    Limits {
        memory: Some(MemoryProbe {
            sample: fake_memory,
            limit,
        }),
        ..limits
    }
}

/// How many times [`fake_memory`] was asked since [`probed`].
fn samples() -> u32 {
    FAKE_MEMORY.with(|cell| cell.get().2)
}

/// A run whose process holds more than the limit is refused with the memory bound's own wording
/// (#8064), naming the innermost definition, uncatchably, whatever the program does: a collector,
/// a fold's state, a recursion that collects nothing. A run too short to reach a sample (fewer
/// check points than the first interval) holds too little to matter and is not sampled.
#[test]
fn past_the_memory_limit_a_run_is_refused_8064() {
    for filter in [
        "[def f: ., (. + 1 | f); 0 | f] | length",
        "reduce (def f: ., (. + 1 | f); 0 | f) as $x (null; [$x, .]) | length",
        "def f: [range(10)] as $a | f, ($a | length); f | empty",
        r#"[try (def f: ., (. + 1 | f); 0 | f) catch "caught"]"#,
    ] {
        let answer = machine_within(probed(SHIPPED, 1 << 20, 2 << 20, 0), filter);
        assert!(answer.outputs.is_empty(), "{filter}: {answer:?}");
        let error = answer.error.unwrap_or_default();
        assert!(
            error == "f/0: memory limit exceeded" || error == "memory limit exceeded",
            "{filter}: {error}"
        );
    }
}

/// With a probe no loop is refused for its outputs: past the shipped output bound a loop answers
/// as jq 1.7.1 answers it, and one that never ends meets its rounds; without one (a library
/// caller that installs none), the shipped bound stands, so no run is unbounded (#8064).
#[test]
fn a_probe_lifts_the_output_bound_8064() {
    let limits = small(BUDGET, 1000, 100);
    let filter = "[def f: if . < 900 then ., (. + 1 | f) else empty end; 0 | f] | length";
    assert_eq!(
        machine_within(probed(limits, usize::MAX, 1 << 20, 0), filter),
        ok(&["900"])
    );
    assert_eq!(
        machine_within(limits, filter),
        refused(0, "f/0: maximum outputs exceeded")
    );
    assert_eq!(
        machine_within(
            probed(limits, usize::MAX, 1 << 20, 0),
            "def f: ., (. + 1 | f); 0 | f"
        ),
        refused(1001, "f/0 exceeded maximum recursion depth")
    );
    assert_eq!(
        machine_within(
            probed(small(BUDGET, 1 << 24, 1 << 20), usize::MAX, 1 << 20, 0),
            "def h($m): if $m < 16132 then $m, h($m + 1) else empty end; \
             def f: if . < 65 then h(0), (. + 1 | f) else empty end; [0 | f] | length"
        ),
        ok(&["1048580"])
    );
}

/// The probe is asked rarely while memory stands still, so a run far from the limit pays nearly
/// nothing (200,000 rounds in constant memory sample a handful of times), and a run is refused at
/// the first sample that reads past the limit.
#[test]
fn the_memory_is_sampled_rarely_while_it_stands_still_8064() {
    let filter = "def f: if . < 200000 then . + 1 | f else . end; 0 | f";
    assert_eq!(
        machine_within(probed(SHIPPED, 1 << 30, 1 << 20, 0), filter),
        ok(&["200000"])
    );
    assert!(samples() < 20, "{} samples", samples());
    // From 1 MiB, 16 MiB more at each sample, towards 64 MiB: the fifth reads 65 MiB.
    let answer = machine_within(
        probed(SHIPPED, 64 << 20, 1 << 20, 16 << 20),
        "[def f: ., (. + 1 | f); 0 | f] | length",
    );
    assert_eq!(answer.error.as_deref(), Some("f/0: memory limit exceeded"));
    assert_eq!(samples(), 5);
}

/// The interval to the next sample: the least at the first reading, the most while nothing grows,
/// and otherwise the check points that, at the growth per check point since the last sample,
/// spend half the room left, within the least and the most.
#[test]
fn the_next_sample_comes_when_half_the_room_could_be_spent_8064() {
    assert_eq!(Sampler::next_interval(0, 100, 16, 1000), MIN_INTERVAL);
    assert_eq!(Sampler::next_interval(100, 100, 16, 1000), MAX_INTERVAL);
    assert_eq!(Sampler::next_interval(100, 50, 16, 1000), MAX_INTERVAL);
    // 1,600 bytes over 16 check points is 100 a check point; 4,000 room, half is 20 check points.
    assert_eq!(Sampler::next_interval(1000, 2600, 16, 6600), 20);
    assert_eq!(Sampler::next_interval(1000, 2600, 16, 2700), MIN_INTERVAL);
    assert_eq!(
        Sampler::next_interval(1000, 1016, 16, usize::MAX),
        MAX_INTERVAL
    );
    // Past the limit there is no room.
    assert_eq!(Sampler::next_interval(1000, 3000, 16, 2000), MIN_INTERVAL);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// The wide leaves change no answer (#7977 step 3): every program answers the same as with
    /// step 1's leaves, which run every `.[]` and `range` on the machine.
    #[test]
    fn the_wide_leaves_change_no_answer_7977(filter in leaf_program()) {
        let narrow = machine_built(SHIPPED, &filter, Build::NARROW);
        let wide = machine_built(SHIPPED, &filter, Build::SHIPPED);
        if let (Ok(narrow), Ok(wide)) = (&narrow, &wide) {
            prop_assert_eq!(narrow, wide, "{}", filter);
        }
        // The wide rule declines nothing step 1's took.
        prop_assert!(narrow.is_err() || wide.is_ok(), "{}", filter);
    }
}
