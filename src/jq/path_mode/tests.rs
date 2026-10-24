//! Every expected answer here was captured from `/usr/bin/jq` 1.7.1 (macOS
//! arm64); the comment on a row says what in jq's bytecode it pins.

// STYLE-0004: jq object literals, not format strings.
#![allow(clippy::literal_string_with_formatting_args)]

use super::*;
use crate::jq::eval_generic::to_owned;
use crate::json::JsonIndex;

/// What jq printed: each output compactly, then the uncaught error's message,
/// if any.
#[derive(Debug, PartialEq)]
struct Answer {
    outputs: Vec<String>,
    error: Option<String>,
}

fn answer(doc: &str, filter: &str) -> Answer {
    let index = JsonIndex::build(doc.as_bytes());
    let input = to_owned::<JqSemantics, _>(&index.root(doc.as_bytes()).value()).expect("doc");
    let run = run(filter, input).expect("parses");
    let error = match run.outcome {
        Outcome::Done => None,
        Outcome::Error(error) => Some(match error.payload() {
            OwnedValue::String(s) => s.as_str().to_string(),
            other => other.to_json_jq_preserve(),
        }),
        other => panic!("{filter}: {other:?}"),
    };
    Answer {
        outputs: run
            .outputs
            .iter()
            .map(OwnedValue::to_json_jq_preserve)
            .collect(),
        error,
    }
}

/// The stack a test body runs on, registered as the native-stack floor's
/// budget, as the CLI registers its own evaluation thread's.
const TEST_STACK: usize = 256 << 20;

/// Run a test body on a thread with room for the continuation-threaded
/// evaluator in a debug build (the binary runs it on a large stack too).
fn on_big_stack(body: impl FnOnce() + Send + 'static) {
    std::thread::Builder::new()
        .stack_size(TEST_STACK)
        .spawn(|| crate::jq::with_stack_budget(TEST_STACK, body))
        .expect("spawn")
        .join()
        .unwrap_or_else(|e| std::panic::resume_unwind(e));
}

fn ok(outputs: &[&str]) -> Answer {
    Answer {
        outputs: outputs.iter().map(|s| String::from(*s)).collect(),
        error: None,
    }
}

fn err(outputs: &[&str], message: &str) -> Answer {
    Answer {
        outputs: outputs.iter().map(|s| String::from(*s)).collect(),
        error: Some(message.to_string()),
    }
}

fn check(rows: &[(&str, &str, Answer)]) {
    for (doc, filter, want) in rows {
        assert_eq!(&answer(doc, filter), want, "{doc} | {filter}");
    }
}

/// The issue's live-probe table, row for row.
#[test]
fn the_issues_live_probes_match_jq_5223() {
    on_big_stack(|| {
        let ab = r#"{"a":{"b":1}}"#;
        check(&[
            // `UPDATE` is checked against the register `SOURCE` left, not the state's.
            (
                ab,
                "path(reduce (.a,.a) as $x (.; .a))",
                err(
                    &[],
                    r#"Invalid path expression near attempt to access element "a" of {"a":{"b":1}}"#,
                ),
            ),
            // The result carries the register after `INIT`.
            (ab, "path(reduce .a as $v (.; .))", ok(&["[]"])),
            // `EXTRACT` sees `UPDATE`'s register; step 2 restarts from `SOURCE`'s.
            (
                ab,
                "path(foreach (1,2) as $x (.; .a; .b))",
                err(
                    &[r#"["a","b"]"#],
                    r#"Invalid path expression near attempt to access element "a" of {"b":1}"#,
                ),
            ),
            (
                ab,
                "path(foreach .a as $v (.; .; .))",
                err(&[], r#"Invalid path expression with result {"a":{"b":1}}"#),
            ),
            // A pattern is tracked: its `INDEX` checks the bound value.
            (
                ab,
                "path(.a as {b:$v} | .)",
                err(
                    &[],
                    r#"Invalid path expression near attempt to access element "b" of {"b":1}"#,
                ),
            ),
            // `[E]`'s `E` is tracked, and the array carries the entry register.
            (
                ab,
                "path([{} | .a])",
                err(
                    &[],
                    r#"Invalid path expression near attempt to access element "a" of {}"#,
                ),
            ),
            (ab, "path(. as $x | [.a] | $x)", ok(&["[]"])),
            // A handler runs at the `try`-entry register.
            (ab, "path(. as $x | try (.a | error) catch $x)", ok(&["[]"])),
            (ab, "path(. as $x | (.a | empty) // $x)", ok(&["[]"])),
            // Arithmetic operands are subexps: the register stays at `.a`.
            (
                ab,
                "path(. as $x | .a | (1 + (.b)) | $x)",
                err(&[], r#"Invalid path expression with result {"a":{"b":1}}"#),
            ),
            // `getpath` on a non-register input is transparent; `PATH_END` checks.
            (
                ab,
                r#"path(.a | {c:2} | getpath(["c"]))"#,
                err(&[], "Invalid path expression with result 2"),
            ),
            (
                r#"{"a":"x"}"#,
                r#"path(.a | ltrimstr("x"))"#,
                err(&[], r#"Invalid path expression with result """#),
            ),
            // A jq-defined builtin's own body is tracked (`explode | map(..)`).
            (
                r#"{"a":"x"}"#,
                "path(.a | ascii_downcase)",
                err(
                    &[],
                    "Invalid path expression near attempt to iterate through [120]",
                ),
            ),
            // Equal literals parsed from different places are not identical.
            (
                r#"{"a":1,"b":1}"#,
                "path(.a as $v | .b | select(. == $v) | $v)",
                err(&[], "Invalid path expression with result 1"),
            ),
        ]);
    });
}

/// `ltrimstr` with no match returns its input in jq (`f_ltrimstr`), so the
/// path survives; the C-builtin table (`c_builtin_identity`) says so.
#[test]
fn a_c_builtin_that_returns_its_input_keeps_the_path_5223() {
    on_big_stack(|| {
        check(&[(
            r#"{"a":"x"}"#,
            r#"path(.a | ltrimstr("z"))"#,
            ok(&[r#"["a"]"#]),
        )]);
    });
}

/// `LOADK` hands out one `jv` per constant site: the same site evaluated
/// twice is identical, two equal constants at different sites are not.
#[test]
fn a_constant_is_one_value_per_site_5223() {
    on_big_stack(|| {
        check(&[
            (
                "null",
                r#"def f: {"a":1}; f | path(f | .a)"#,
                ok(&[r#"["a"]"#]),
            ),
            (
                "null",
                r#"{"a":1} | path({"a":1} | .a)"#,
                err(
                    &[],
                    r#"Invalid path expression near attempt to access element "a" of {"a":1}"#,
                ),
            ),
            ("null", "def f: [1]; f | path(f | .[0])", ok(&["[0]"])),
            (
                "null",
                "[1] | path([1] | .[0])",
                err(
                    &[],
                    "Invalid path expression near attempt to access element 0 of [1]",
                ),
            ),
            (
                "null",
                r#"def f: "s"; f | path(f | .[0:1])"#,
                ok(&[r#"[{"start":0,"end":1}]"#]),
            ),
        ]);
    });
}

/// `jvp_array_slice` returns a view into the same allocation: the whole
/// range is the array itself, and equal ranges of one array (a slice of a
/// slice included) are one value.
#[test]
fn an_array_slice_is_a_view_with_jqs_identity_5223() {
    on_big_stack(|| {
        check(&[
            ("[1,2]", "path(reduce (.[0:2]) as $k (.; $k))", ok(&["[]"])),
            (
                "[1,2]",
                "path(foreach (.[0:2]) as $k (.; .[0]))",
                ok(&[r#"[{"start":0,"end":2},0]"#]),
            ),
            (
                "[1,2,3]",
                "path(.[0:2] as $a | .[0:2] | $a)",
                ok(&[r#"[{"start":0,"end":2}]"#]),
            ),
            (
                "[1,2,3]",
                "path(.[1:3] | .[0:1] as $a | . as $b | $b[0:1] | $a)",
                ok(&[r#"[{"start":1,"end":3},{"start":0,"end":1}]"#]),
            ),
            (
                "[1,2,3]",
                "[.[1:], .[-2:], .[:-1], .[1.5:2.5], .[5:], .[null:2]]",
                ok(&["[[2,3],[2,3],[1,2],[2,3],[],[1,2]]"]),
            ),
        ]);
    });
}

/// `jv_identical` by kind and payload: `null`/`true`/`false` of the same kind
/// always are; a computed number never equals a literal.
#[test]
fn scalar_identity_follows_jv_identical_5223() {
    on_big_stack(|| {
        check(&[
            (r#"{"a":null}"#, "path(.a | null)", ok(&[r#"["a"]"#])),
            (r#"{"a":true}"#, "path(.a | true)", ok(&[r#"["a"]"#])),
            (
                r#"{"a":true}"#,
                "path(.a | false)",
                err(&[], "Invalid path expression with result false"),
            ),
            (
                r#"{"a":1}"#,
                "path(1 + 1 as $x | 2 | $x)",
                err(&[], "Invalid path expression with result 2"),
            ),
            (
                r#"{"a":1}"#,
                "path(1 as $x | $x)",
                err(&[], "Invalid path expression with result 1"),
            ),
            // `$v` bound by navigation is the node itself.
            (
                r#"{"a":1}"#,
                "[.[] as $v | path(.a | select(. == $v))]",
                ok(&[r#"[["a"]]"#]),
            ),
        ]);
    });
}

/// `.k?` is `INDEX_OPT`: it suppresses the get's own error but still raises a
/// `path_intact` failure; `(E)?` is `try E` and swallows it.
#[test]
fn an_opt_index_still_raises_a_path_error_and_try_does_not_5223() {
    on_big_stack(|| {
        check(&[
            (
                "null",
                "path(1|.a?)",
                err(
                    &[],
                    r#"Invalid path expression near attempt to access element "a" of 1"#,
                ),
            ),
            ("null", "[path(1|(.a)?)]", ok(&["[]"])),
            (
                r#"{"a":1}"#,
                "try path(1 | .b) catch .",
                ok(&[r#""Invalid path expression near attempt to access element \"b\" of 1""#]),
            ),
        ]);
    });
}

/// `TRY_END` wraps an error raised downstream of a body output, so the `try`
/// re-raises it; a `break` is an error a `try` catches (`GENLABEL`'s
/// `{"__jq": n}`); `?//` retries on any error after it, downstream included.
#[test]
fn try_label_and_alternatives_handle_errors_as_jq_5223() {
    on_big_stack(|| {
        check(&[
            ("null", r#"(try 1) | error("d")"#, err(&[], "d")),
            (
                "null",
                "label $f | try (break $f) catch .",
                ok(&[r#"{"__jq":0}"#]),
            ),
            (
                "null",
                r#"[[1] | . as [$a] ?// $a | if ($a|type) == "number" then error("retry") else $a end]"#,
                ok(&["[[1]]"]),
            ),
            (
                r#"{"a":{"b":1},"b":2}"#,
                "[. as [$q] ?// $q | $q]",
                ok(&[r#"[{"a":{"b":1},"b":2}]"#]),
            ),
        ]);
    });
}

/// `gen_array_matcher` emits the last element's `INDEX` first, so the second
/// `INDEX` of a two-element pattern is off the register.
#[test]
fn an_array_pattern_indexes_its_last_element_first_5223() {
    on_big_stack(|| {
        check(&[
            (
                "[1,2]",
                "path(. as [$a,$b] | .)",
                err(
                    &[],
                    "Invalid path expression near attempt to access element 0 of [1,2]",
                ),
            ),
            (
                r#"{"a":1,"b":2}"#,
                "path(. as {a:$a,b:$b} | .)",
                err(
                    &[],
                    r#"Invalid path expression near attempt to access element "b" of {"a":1,"b":2}"#,
                ),
            ),
        ]);
    });
}

/// `DUPN` moves the fold's input out of a slot `INIT`'s fork point shares, so
/// every `INIT` output after the first runs `SOURCE` on `null`.
#[test]
fn a_folds_later_init_outputs_see_a_null_source_input_5223() {
    on_big_stack(|| {
        check(&[
            (
                r#"{"a":1}"#,
                "[reduce .a as $i ((1,2); [$i,.])]",
                ok(&["[[1,1],[null,2]]"]),
            ),
            (
                r#"{"a":1}"#,
                "[foreach .a as $i ((1,2,3); [$i,.])]",
                ok(&["[[1,1],[null,2],[null,3]]"]),
            ),
            (
                r#"{"a":{"a":null}}"#,
                "path(foreach .a as $i ((.,.); .; $i))",
                err(
                    &[r#"["a"]"#],
                    r#"Invalid path expression near attempt to access element "a" of null"#,
                ),
            ),
        ]);
    });
}

/// `gen_index` evaluates the key on the node's own input, before the target;
/// 1.7.1 keeps a negative index as written; `getpath` moves the register only
/// from an identical input; `range/2` emits its start value itself first.
#[test]
fn navigation_records_jqs_own_components_5223() {
    on_big_stack(|| {
        check(&[
            (r#"{"a":{"x":1},"b":"x"}"#, "[.a[.b]]", ok(&["[1]"])),
            ("[1,2,3]", "path(.[-1])", ok(&["[-1]"])),
            (
                r#"{"a":[1,2]}"#,
                "path(.a | first, last)",
                ok(&[r#"["a",0]"#, r#"["a",-1]"#]),
            ),
            (
                r#"{"a":[1,2]}"#,
                "path(.a[1:] | .[0])",
                ok(&[r#"["a",{"start":1,"end":null},0]"#]),
            ),
            (
                r#"{"a":{"b":1}}"#,
                r#"path(.a | getpath(["b"]) | getpath([]))"#,
                ok(&[r#"["a","b"]"#]),
            ),
            (
                r#"{"a":{"b":1}}"#,
                r#"path(getpath(["x","y"]))"#,
                ok(&[r#"["x","y"]"#]),
            ),
            (
                r#"{"a":{"b":1}}"#,
                r#"path(.a | {} | getpath(["b"]))"#,
                err(&[], "Invalid path expression with result null"),
            ),
            (
                r#"{"a":{"b":1},"b":2}"#,
                "path(range(.a.b; 3))",
                err(&[r#"["a","b"]"#], "Invalid path expression with result 2"),
            ),
        ]);
    });
}

/// The write doors run `builtin.jq`'s `_modify`/`_assign`/`del` over the
/// interpreter's own `path()`.
#[test]
fn writes_and_jq_defined_builtins_run_from_the_prelude_5223() {
    on_big_stack(|| {
        let doc = r#"{"a":{"b":1},"b":2}"#;
        check(&[
            (doc, "del(.a)", ok(&[r#"{"b":2}"#])),
            (doc, ".a.b |= .+1", ok(&[r#"{"a":{"b":2},"b":2}"#])),
            (doc, ".a.b += 5", ok(&[r#"{"a":{"b":6},"b":2}"#])),
            (doc, ".c //= 3", ok(&[r#"{"a":{"b":1},"b":2,"c":3}"#])),
            (doc, ".a = 9", ok(&[r#"{"a":9,"b":2}"#])),
            // 1.7.1's `_modify` collects every emptied path and deletes them
            // all at the end.
            (doc, ".[] |= empty", ok(&["{}"])),
            (doc, "[paths]", ok(&[r#"[["a"],["a","b"],["b"]]"#])),
            (doc, "del(..|select(. == 1))", ok(&[r#"{"a":{},"b":2}"#])),
            (doc, "pick(.a.b)", ok(&[r#"{"a":{"b":1}}"#])),
            (doc, "[limit(2; .[])]", ok(&[r#"[{"b":1},2]"#])),
            (doc, "to_entries | from_entries", ok(&[doc])),
            ("[1,[2,[3]]]", "flatten", ok(&["[1,2,3]"])),
            ("[1,2]", r#"join(",")"#, ok(&[r#""1,2""#])),
        ]);
    });
}

/// A construct the interpreter does not implement ends the run as unsupported,
/// never as a jq error a `try` could swallow.
#[test]
fn an_unimplemented_construct_is_unsupported_not_an_error_5223() {
    on_big_stack(|| {
        let run = run("try at_offset(0) catch 1", OwnedValue::Null).expect("parses");
        assert!(
            matches!(run.outcome, Outcome::Unsupported(_)),
            "{:?}",
            run.outcome
        );
    });
}

/// jq rejects an unbound variable at compile time, before the input is read;
/// the interpreter reports it the same way rather than when evaluation reaches it.
#[test]
fn an_unbound_variable_is_a_compile_error_5223() {
    on_big_stack(|| {
        assert!(run(
            "reduce .a? as $i (.; try ($x as $q | $a))",
            OwnedValue::Null
        )
        .is_err());
    });
}

/// The prelude parsed one definition at a time is the chain [`PRELUDE`] spells,
/// each definition's own `FuncDef` with the rest of the chain cut off; and it
/// parses on a 2 MiB thread, which the whole chain does not in a debug build.
#[test]
fn the_prelude_parses_one_definition_at_a_time_5223() {
    on_big_stack(|| {
        let prelude = Prelude::new();
        let mut chain = parse(PRELUDE).expect("the prelude parses");
        let mut defs = Vec::new();
        while let Expr::FuncDef { then, .. } = &mut chain {
            let rest = core::mem::replace(&mut **then, Expr::Identity);
            defs.push(mark_loadvn(&chain));
            chain = rest;
        }
        assert_eq!(chain, Expr::Identity);
        assert_eq!(prelude.defs, defs);
    });
    std::thread::Builder::new()
        .stack_size(2 << 20)
        .spawn(|| drop(Prelude::new()))
        .expect("spawn")
        .join()
        .expect("the prelude parses on a 2 MiB thread");
}

/// Every jq-defined builtin [`prelude_call`] maps resolves to a definition of
/// the right arity in [`PRELUDE`].
#[test]
fn every_prelude_call_resolves_5223() {
    on_big_stack(|| {
        let prelude = Prelude::new();
        let interp = Interp::new(&prelude);
        let calls = [
            "select(.)",
            "map(.)",
            "map_values(.)",
            "recurse",
            "recurse(.[]?)",
            "recurse(.[]?; .)",
            "add",
            "del(.a)",
            "to_entries",
            "from_entries",
            "with_entries(.)",
            "reverse",
            "paths",
            "paths(.)",
            "arrays",
            "objects",
            "iterables",
            "booleans",
            "numbers",
            "normals",
            "finites",
            "strings",
            "nulls",
            "values",
            "scalars",
            r#"join(",")"#,
            "flatten",
            "flatten(1)",
            "first",
            "last",
            "nth(0)",
            "isempty(.)",
            "any",
            "all",
            "any(.)",
            "all(.)",
            "any(.[]; .)",
            "all(.[]; .)",
            "in({})",
            "inside({})",
            "IN(1)",
            "IN(1; 1)",
            "ascii_downcase",
            "ascii_upcase",
            "walk(.)",
            "pick(.a)",
            "abs",
            "sort_by(.)",
            "group_by(.)",
            "unique",
            "unique_by(.)",
            "min_by(.)",
            "max_by(.)",
            "indices(1)",
            "index(1)",
            "rindex(1)",
            "debug(.)",
            "isfinite",
            "halt_error",
            "todate",
            "fromdate",
            "todateiso8601",
            "fromdateiso8601",
            r#"match("a")"#,
            r#"match("a"; "g")"#,
            r#"test("a")"#,
            r#"test("a"; "g")"#,
            r#"capture("a")"#,
            r#"capture("a"; "g")"#,
            r#"scan("a")"#,
            r#"scan("a"; "g")"#,
            r#"splits("a")"#,
            r#"splits("a"; "g")"#,
            r#"split("a"; "g")"#,
            r#"sub("a"; "b")"#,
            r#"sub("a"; "b"; "g")"#,
            r#"gsub("a"; "b")"#,
            r#"gsub("a"; "b"; "g")"#,
            "combinations",
            "combinations(2)",
            "transpose",
            "truncate_stream(.)",
            "fromstream(.)",
            "tostream",
            "bsearch(1)",
            "INDEX(.)",
            "INDEX(.; .)",
            "leaf_paths",
            "skip(1; .)",
            "isvalid(.)",
            "recurse_down",
        ];
        for call in calls {
            let expr = parse(call).expect(call);
            let Expr::Builtin(builtin) = &expr else {
                panic!("{call} parsed as {expr:?}");
            };
            let (name, args) =
                prelude_call(builtin).unwrap_or_else(|| panic!("{call} is not mapped"));
            assert!(
                interp.prelude_env.lookup_func(name, args.len()).is_some(),
                "{call} -> {name}/{} is not in the prelude",
                args.len()
            );
        }
    });
}

/// `sort_by`/`group_by`/`unique`/`min_by`/`max_by` run their `builtin.jq` bodies over
/// native `_sort_by_impl`-style helpers, errors included.
#[test]
fn the_sort_group_and_min_max_helpers_keep_jqs_answers_5223() {
    on_big_stack(|| {
        check(&[
            (
                r"[3,1,2]",
                r"sort_by(-.), group_by(. > 1), unique, unique_by(. % 2), min_by(.), max_by(.)",
                ok(&[r"[3,2,1]", r"[[1],[3,2]]", r"[1,2,3]", r"[2,3]", r"1", r"3"]),
            ),
            (
                r"[[1],[1]]",
                r"[min_by(.), max_by(.)] | map(. == [1])",
                ok(&[r"[true,true]"]),
            ),
            (
                r"[]",
                r"min_by(.), max_by(.), sort_by(.), group_by(.)",
                ok(&[r"null", r"null", r"[]", r"[]"]),
            ),
            (
                r#"{"a":1}"#,
                r"try sort_by(.) catch .",
                ok(&[
                    r#""object ({\"a\":1}) and array ([[1]]) cannot be sorted, as they are not both arrays""#,
                ]),
            ),
            (
                r#"{"a":1}"#,
                r"try group_by(.) catch .",
                ok(&[
                    r#""object ({\"a\":1}) and array ([[1]]) cannot be sorted, as they are not both arrays""#,
                ]),
            ),
            (
                r#"{"a":1}"#,
                r"try min_by(.) catch .",
                ok(&[r#""object ({\"a\":1}) and array ([[1]]) cannot be iterated over""#]),
            ),
            (
                r"[3,1,2]",
                r"path(min_by(.))",
                err(&[], r"Invalid path expression with result 1"),
            ),
        ]);
    });
}

/// `indices`/`index`/`rindex` and `debug(msg)` run their `builtin.jq` bodies, so
/// `index` navigates its fresh `indices` result (jq raises on it in path mode).
#[test]
fn the_index_family_runs_its_builtin_jq_body_5223() {
    on_big_stack(|| {
        check(&[
            (
                r#""xyxy""#,
                r#"indices("x"), index("y"), rindex("x"), indices("")"#,
                ok(&[r"[0,2]", r"1", r"2", r"[]"]),
            ),
            (
                r"[1,2,1,2]",
                r"indices(1), indices([1,2]), index(2), rindex(2)",
                ok(&[r"[0,2]", r"[0,2]", r"1", r"3"]),
            ),
            (r#"{"a":1}"#, r#"try indices("a") catch ."#, ok(&[r"1"])),
            (
                r#""xyx""#,
                r#"path(.[0:1] | index("x"))"#,
                err(
                    &[],
                    r"Invalid path expression near attempt to access element 0 of [0]",
                ),
            ),
            (
                r"[1,2]",
                r#"debug("m"), [debug(.)]"#,
                ok(&[r"[1,2]", r"[[1,2]]"]),
            ),
        ]);
    });
}

/// The update-assignment operators, negation, interpolation, the generator
/// builtins, constants and object construction.
#[test]
fn operators_folds_and_generators_match_jq_5223() {
    on_big_stack(|| {
        check(&[
            (
                r#"{"a":5}"#,
                r".a -= 1, .a *= 2, .a /= 2, .a %= 3, .b //= 7",
                ok(&[
                    r#"{"a":4}"#,
                    r#"{"a":10}"#,
                    r#"{"a":2.5}"#,
                    r#"{"a":2}"#,
                    r#"{"a":5,"b":7}"#,
                ]),
            ),
            (
                r#"{"a":5}"#,
                r"-(.a), (.a | not), ([.a, 1] | not)",
                ok(&[r"-5", r"false", r"false"]),
            ),
            (
                r#"{"a":5}"#,
                r"-1.5, [-1.5, 1+2, 1 < 2, {b:1, b:2}]",
                ok(&[r"-1.5", r#"[-1.5,3,true,{"b":2}]"#]),
            ),
            (
                r#"{"a":5}"#,
                r#""x\(.a)y\(.a + 1)", @base64 "\(.a)", @text"#,
                ok(&[r#""x5y6""#, r#""NQ==""#, r#""{\"a\":5}""#]),
            ),
            (
                r#"{"a":5}"#,
                r"first(range(3; 10)), last(range(3)), nth(1; range(5)), [limit(2; repeat(.a))]",
                ok(&[r"3", r"2", r"1", r"[5,5]"]),
            ),
            (
                r#"{"a":5}"#,
                r"[.a | until(. > 20; . * 2)], [.a | while(. < 20; . * 2)], [range(0; 10; 3)], [range(0.5; 2)]",
                ok(&[r"[40]", r"[5,10]", r"[0,3,6,9]", r"[0.5,1.5]"]),
            ),
            (
                r#"{"a":5}"#,
                r#"try ([range("a"; 2)]) catch ., [limit(3; range(0; nan))]"#,
                ok(&[r#""Range bounds must be numeric""#, r"[0,1,2]"]),
            ),
            (
                r#"{"a":5}"#,
                r#"$__loc__, {a: 1, a: 2}, [1 + 1, 2 < 1], {(.a | tostring): 1, ("5"): 2}"#,
                ok(&[
                    r#"{"file":"<top-level>","line":1}"#,
                    r#"{"a":2}"#,
                    r"[2,false]",
                    r#"{"5":2}"#,
                ]),
            ),
            (
                r#"{"a":5}"#,
                r"try {(.a): 1} catch .",
                ok(&[r#""Cannot use number (5) as object key""#]),
            ),
        ]);
    });
}

/// Every index, slice and iterate form, optional or not, including the shapes
/// `jv_get` delegates and the errors they raise.
#[test]
fn index_slice_and_iterate_forms_match_jq_5223() {
    on_big_stack(|| {
        check(&[
            (
                r"[1,2,3]",
                r".[0]?, .[1:]?, (2 as $k | .[$k]?), (1 as $a | .[$a:]?), (1 as $b | .[:$b]), (1 as $b | .[:$b]?)",
                ok(&[r"1", r"[2,3]", r"3", r"[2,3]", r"[1]", r"[1]"]),
            ),
            (
                r"[1,2,3]",
                r#".[nan], .[-1], .[10], (try .["a"] catch .), (try .[[1]] catch .), .[[2,3]]"#,
                ok(&[
                    r"null",
                    r"3",
                    r"null",
                    r#""Cannot index array with string \"a\"""#,
                    r"[0]",
                    r"[1]",
                ]),
            ),
            (
                r#"{"a":1}"#,
                r"(1 | .a?), (try (1 | .a) catch .), [.[]?], (try (1 | .[]) catch .), [1 | .[]?]",
                ok(&[
                    r#""Cannot index number with string \"a\"""#,
                    r"[1]",
                    r#""Cannot iterate over number (1)""#,
                    r"[]",
                ]),
            ),
            (
                r#""abcd""#,
                r#".[1:3], .[nan:2], .[1:nan], (try .["x":2] catch .), .[-2:]"#,
                ok(&[
                    r#""bc""#,
                    r#""ab""#,
                    r#""bcd""#,
                    r#""Array/string slice indices must be integers""#,
                    r#""cd""#,
                ]),
            ),
            (
                r"null",
                r".a, .[0], .[1:2], path(.a.b[0])",
                ok(&[r"null", r"null", r"null", r#"["a","b",0]"#]),
            ),
            (
                r#"{"a":[1,2]}"#,
                r#"getpath(["a",1]), (try getpath("a") catch .), [paths], path(getpath(["a"]) | .[0])"#,
                ok(&[
                    r"2",
                    r#""Path must be specified as an array""#,
                    r#"[["a"],["a",0],["a",1]]"#,
                    r#"["a",0]"#,
                ]),
            ),
        ]);
    });
}

/// Object and array patterns, `$name: pattern` binds, computed keys and `?//`
/// chains, in `as` and in a fold.
#[test]
fn destructuring_patterns_and_alternatives_match_jq_5223() {
    on_big_stack(|| {
        check(&[
            (
                r#"{"a":1,"b":[2]}"#,
                r". as {a:$x, $b} | [$x, $b]",
                ok(&[r"[1,[2]]"]),
            ),
            (
                r#"{"a":1,"b":[2]}"#,
                r". as {$a: $v} | [$a, $v]",
                ok(&[r"[1,1]"]),
            ),
            (
                r#"{"a":1,"b":[2]}"#,
                r#". as {("a","b"): $v} | $v"#,
                ok(&[r"1", r"[2]"]),
            ),
            (
                r#"{"a":1,"b":[2]}"#,
                r"[. as {a: [$x]} ?// {b: [$x]} | $x]",
                ok(&[r"[2]"]),
            ),
            (
                r#"{"a":1,"b":[2]}"#,
                r"[.[] as [$x] ?// $x | $x]",
                ok(&[r"[1,2]"]),
            ),
            (
                r#"{"a":1,"b":[2]}"#,
                r"reduce .b[] as [$q] ?// $q (0; . + $q)",
                ok(&[r"2"]),
            ),
        ]);
    });
}

/// `try`, nested labels, a label's downstream error, and delegated C builtins
/// (several outputs, an error, a variable in an argument).
#[test]
fn try_labels_and_delegated_builtins_match_jq_5223() {
    on_big_stack(|| {
        check(&[
            (
                r#"{"a":1}"#,
                r#"try .a catch ., (try error({}) catch .), [.[] | try error("x") catch .]"#,
                ok(&[r"1", r"{}", r#"["x"]"#]),
            ),
            (
                r#"{"a":1}"#,
                r#"(label $f | 1, 2) | if . == 2 then error("e") else . end"#,
                err(&[r"1"], r"e"),
            ),
            (
                r#"{"a":1}"#,
                r#"try ((label $f | 1) | error("x")) catch ."#,
                ok(&[r#""x""#]),
            ),
            (
                r#"{"a":1}"#,
                r"label $f | label $g | 1, break $f",
                ok(&[r"1"]),
            ),
            (
                r#"{"a":1}"#,
                r"[label $out | foreach (1,2,3) as $i (0; . + $i; ., if . > 2 then break $out else empty end)]",
                ok(&[r"[1,3]"]),
            ),
            (
                r#"{"a":1}"#,
                r#".a as $x | "\($x)" as $s | [$s, ($x | has(0)?)]"#,
                ok(&[r#"["1"]"#]),
            ),
            (
                r#"{"a":1}"#,
                r"(.a | keys)",
                err(&[], r"number (1) has no keys"),
            ),
            (
                r#"{"a":1}"#,
                // `combinations` is delegated and answers several values (no
                // regex builtin here: `cargo test --lib` builds without it).
                r#""a,b" | ([[1,2],[3]] | [combinations]), ascii_downcase, ascii_upcase, ltrimstr("a"), tostring"#,
                ok(&[
                    r"[[1,3],[2,3]]",
                    r#""a,b""#,
                    r#""A,B""#,
                    r#"",b""#,
                    r#""a,b""#,
                ]),
            ),
            (
                r#"{"a":1}"#,
                r#"path(1 | getpath(["a"])?), [path(..)]"#,
                ok(&[r#"[[],["a"]]"#]),
            ),
        ]);
    });
}

/// `jv_identical` on values built directly: computed numbers by their bits,
/// a document's numbers as literals, containers and strings by allocation.
#[test]
fn identity_is_jv_identical_on_every_kind_5223() {
    on_big_stack(|| {
        // The SVal reference and `jv_identical` on promoted `OwnedValue`s give
        // every kind the same answer.
        use super::reference::{identical, SVal};
        let two = SVal::computed(OwnedValue::Int(2));
        assert!(identical(&two, &SVal::computed(OwnedValue::Float(2.0))));
        assert!(jv_identical(&OwnedValue::Int(2), &OwnedValue::Float(2.0)));
        assert!(!identical(&two, &SVal::computed(OwnedValue::Float(-0.0))));
        assert!(!jv_identical(&OwnedValue::Int(2), &OwnedValue::Float(-0.0)));
        let doc = literal(NumberRepr::Int(2), "2");
        assert!(jv_identical(&doc, &doc.clone()));
        assert!(!jv_identical(&doc, &OwnedValue::Int(2)));
        assert!(!jv_identical(&doc, &literal(NumberRepr::Int(2), "2")));
        let text = string("s");
        assert!(jv_identical(&text, &text.clone()));
        assert!(!jv_identical(&text, &string("s")));
        assert!(identical(&SVal::Null, &SVal::Null));
        assert!(jv_identical(&OwnedValue::Null, &OwnedValue::Null));
        assert!(!jv_identical(
            &OwnedValue::Bool(true),
            &OwnedValue::Bool(false)
        ));
        assert!(!jv_identical(&OwnedValue::Null, &OwnedValue::Bool(false)));
        let _ = Prelude::default();
    });
}

/// `halt_error` halts with jq's exit status (5 by default, or its argument),
/// after the shipped builtin has written its message; `input` without an
/// input queue raises `break`, as jq's does, and a `try` catches it. Every expected
/// status is jq 1.7.1's.
#[test]
fn halt_error_and_input_run_as_c_builtins_5223() {
    on_big_stack(|| {
        for (filter, code) in [
            (r#""bye\n" | halt_error"#, 5),
            (r#"{"a":1} | halt_error(3)"#, 3),
        ] {
            let run = crate::jq::with_stderr_muted(|| run(filter, OwnedValue::Null)).expect(filter);
            assert!(
                matches!(run.outcome, Outcome::Halt(c) if c == code),
                "{filter}: {:?}",
                run.outcome
            );
        }
        // `no_std` has no input queue to read.
        #[cfg(feature = "std")]
        check(&[("null", "try input catch .", ok(&[r#""break""#]))]);
    });
}

/// The jq-defined builtins Phase 0 stubbed run their `builtin.jq` bodies
/// now: their values, and in path mode the register effect of the body
/// (`transpose` and `tostream` build fresh arrays; `combinations` indexes its
/// input). Every expected answer is jq 1.7.1's.
#[test]
fn the_rest_of_builtin_jq_runs_from_its_definitions_5223() {
    on_big_stack(|| {
        check(&[
            ("[[1,2],[3]]", "transpose", ok(&["[[1,3],[2,null]]"])),
            (
                "[1,[2]]",
                "[tostream]",
                ok(&["[[[0],1],[[1,0],2],[[1,0]],[[1]]]"]),
            ),
            (
                "[[1,2],[3,4]]",
                "[combinations]",
                ok(&["[[1,3],[1,4],[2,3],[2,4]]"]),
            ),
            (
                "[1,2]",
                "[combinations(2)]",
                ok(&["[[1,1],[1,2],[2,1],[2,2]]"]),
            ),
            (
                "[1,2,3]",
                "bsearch(2), bsearch(0), bsearch(4)",
                ok(&["1", "-1", "-4"]),
            ),
            (
                r#"[{"id":1},{"id":2}]"#,
                "INDEX(.id)",
                ok(&[r#"{"1":{"id":1},"2":{"id":2}}"#]),
            ),
            (
                "null",
                r#"INDEX(({"id":"a"},{"id":"b"}); .id)"#,
                ok(&[r#"{"a":{"id":"a"},"b":{"id":"b"}}"#]),
            ),
            (
                "null",
                "[1|truncate_stream([[0],1],[[1,0],2],[[1,0]],[[1]])]",
                ok(&["[[[0],2],[[0]]]"]),
            ),
            (
                "null",
                "fromstream(1|truncate_stream([[0],1],[[1,0],2],[[1,0]],[[1]]))",
                ok(&["[2]"]),
            ),
            ("null", "1e1000 | isfinite", ok(&["false"])),
            (
                "[[1]]",
                "path(transpose)",
                err(&[], "Invalid path expression with result [[1]]"),
            ),
            (
                "[1,2]",
                "path(tostream)",
                err(&[], "Invalid path expression with result [[0],1]"),
            ),
            (
                r#""abc""#,
                "path(combinations)",
                err(&[], "Cannot index string with number"),
            ),
        ]);
    });
}

/// A spelled index or slice bound is recorded as written (`.[1.0]` is
/// `[1.0]`), and a string slice counts codepoints.
#[test]
fn spelled_keys_and_string_slices_match_jq_5223() {
    on_big_stack(|| {
        check(&[
            (
                "[1,2,3]",
                "path(.[1.0]), .[1.0], path(.[1.0:2.50]), path(.[1.0]?)",
                ok(&["[1.0]", "2", r#"[{"start":1.0,"end":2.50}]"#, "[1.0]"]),
            ),
            (
                "[1,2,3]",
                "path(.[1.5:3]), .[1.5:3]",
                ok(&[r#"[{"start":1.5,"end":3}]"#, "[2,3]"]),
            ),
            (
                "null",
                r#"path(.[1.5:"a"])"#,
                ok(&[r#"[{"start":1.5,"end":"a"}]"#]),
            ),
            (r#""äöü""#, ".[1:]", ok(&[r#""öü""#])),
            // jq 1.7.1 slices to the end on a NaN end bound; succinctly jq's own
            // value route answers "" here (a separate bug).
            (r#""abcd""#, ".[1:nan]", ok(&[r#""bcd""#])),
        ]);
    });
}

/// One run, as the differential test compares it: each output compactly, then
/// how the run ended.
type Summary = (Vec<String>, String);

fn summary(outputs: &[OwnedValue], ending: String) -> Summary {
    (
        outputs
            .iter()
            .map(OwnedValue::to_json_jq_preserve)
            .collect(),
        ending,
    )
}

fn owned_summary(prelude: &Prelude, program: &Expr, doc: &OwnedValue) -> Summary {
    let run = Interp::new(prelude).run(program, doc.clone());
    let ending = match run.outcome {
        Outcome::Done => String::from("done"),
        Outcome::Error(e) => format!("error {}", e.payload().to_json_jq_preserve()),
        Outcome::Halt(code) => format!("halt {code}"),
        Outcome::Unsupported(what) => format!("unsupported {what}"),
    };
    summary(&run.outputs, ending)
}

/// The reference's answer, or `None` when it answered a builtin through its
/// fresh stub, which the C-builtin table now answers as jq does (its rows are
/// pinned against jq in `c_builtin_identity`).
fn reference_summary(
    prelude: &super::reference::Prelude,
    program: &Expr,
    doc: &OwnedValue,
) -> Option<Summary> {
    use super::reference::{Interp as RefInterp, Outcome as RefOutcome};
    let run = RefInterp::new(prelude).run(program, doc);
    if !run.stubbed.is_empty() {
        return None;
    }
    let ending = match run.outcome {
        RefOutcome::Done => String::from("done"),
        RefOutcome::Error(payload) => format!("error {}", payload.to_json_jq_preserve()),
        RefOutcome::Halt(code) => format!("halt {code}"),
        RefOutcome::Unsupported(what) => format!("unsupported {what}"),
    };
    Some(summary(&run.outputs, ending))
}

fn parse_doc(doc: &str) -> OwnedValue {
    let index = JsonIndex::build(doc.as_bytes());
    to_owned::<JqSemantics, _>(&index.root(doc.as_bytes()).value()).expect("doc")
}

/// The differential corpus: every issue repro, and a grid of two-stage path
/// programs over the construct forms these tests pin, on a handful of
/// documents. The builtins whose identity the Phase 0 stub got wrong
/// (`. + null` and the other operands `binop_plus` hands back) are pinned
/// against jq by the C-builtin table's own tests instead.
fn differential_corpus() -> Vec<(String, String)> {
    let mut rows = Vec::new();
    for line in include_str!("../../../scripts/jq-path-mode-rows/repros.jsonl").lines() {
        let row: serde_json::Value = serde_json::from_str(line).expect("a JSON row");
        let args = row["args"].as_array().expect("args");
        let filter = args.last().and_then(|a| a.as_str()).expect("a filter");
        let stdin = row["stdin"].as_str().expect("stdin").trim().to_string();
        rows.push((stdin, filter.to_string()));
    }
    let stages = [
        ".",
        ".a",
        ".b",
        ".[]",
        ".[]?",
        ".a?",
        ".[0]",
        ".[1:]",
        "..",
        "(.a, .b)",
        ". as $x | $x",
        ". as $x | .a | $x",
        ".a as $x | .b | $x",
        ". as [$p] | $p",
        ". as {a: $p} ?// [$p] | $p",
        "select(.)",
        "select(type == \"object\")",
        "if . then .a else .b end",
        "try .a catch .",
        "try error(.) catch .",
        ".a // .b",
        "(.a | empty) // .",
        "first(.[])",
        "last(.[])",
        "limit(1; .[])",
        "label $l | .[] | ., break $l",
        "reduce .[] as $x (.; .)",
        "reduce .a as $x (.; .b)",
        "foreach .[] as $x (.; .; .)",
        "foreach (1,2) as $x (.; .a; .)",
        "getpath([\"a\"])",
        "getpath([\"a\",\"b\"])",
        "[.a][0]",
        "{k: .}.k",
        "1",
        "\"x\"",
        "null",
        ". - 1",
        "tostring",
        "ltrimstr(\"x\")",
        "recurse",
        "to_entries",
        "map(.)",
        "paths",
        "def f: .a; f",
        "def f(g): g | g; f(.[]?)",
        ".a |= 1",
        "del(.a)",
    ];
    let docs = [
        r#"{"a":{"b":1},"b":[1,"x",null]}"#,
        r#"[1,[2],{"a":"x"}]"#,
        r#"{"a":"x","b":"x"}"#,
        r#"{"a":1,"b":1}"#,
        r#""s""#,
    ];
    for doc in docs {
        for s1 in stages {
            for s2 in stages {
                rows.push((doc.to_string(), format!("path({s1} | {s2})")));
            }
            rows.push((doc.to_string(), format!("[{s1}]")));
            rows.push((doc.to_string(), format!("del({s1})")));
            rows.push((doc.to_string(), format!("({s1}) |= 7")));
        }
    }
    rows
}

/// The `OwnedValue` interpreter answers exactly what the Phase 0 `SVal`
/// reference answers, on every row of [`differential_corpus`] the reference
/// answered without its fresh stub.
#[test]
fn owned_interpreter_matches_the_sval_reference_5223() {
    on_big_stack(|| {
        let prelude = Prelude::new();
        let reference = super::reference::Prelude::new();
        let mut differ = Vec::new();
        let mut compared = 0;
        for (doc, filter) in differential_corpus() {
            let Ok(program) = parse(&filter) else {
                continue;
            };
            if !crate::jq::resolve::resolve_all_jq(&mut program.clone()).is_empty() {
                continue;
            }
            let input = parse_doc(&doc);
            let Some(expected) = reference_summary(&reference, &program, &input) else {
                continue;
            };
            let owned = owned_summary(&prelude, &program, &input);
            compared += 1;
            if owned != expected {
                differ.push(format!(
                    "{doc} | {filter}\n  owned: {owned:?}\n  ref:   {expected:?}"
                ));
            }
        }
        assert!(compared > 10_000, "{compared}");
        assert!(
            differ.is_empty(),
            "{} rows differ:\n{}",
            differ.len(),
            differ.join("\n")
        );
    });
}

/// A negative number is `_negate` of a literal, so it is computed and compares
/// by value, including the spellings the parser folds itself (`-01`, `-00`,
/// `-01.5`): each row is `[0]` in jq 1.7.1.
#[test]
fn a_negative_number_is_computed_however_spelled_5223() {
    on_big_stack(|| {
        check(&[
            ("null", "[-01] | path(.[0] | -01)", ok(&["[0]"])),
            ("null", "[-1.5e0] | path(.[0] | -01.5)", ok(&["[0]"])),
            ("null", "[-1] | path(.[0] | -1)", ok(&["[0]"])),
            ("null", "[-00] | path(.[0] | -00)", ok(&["[0]"])),
        ]);
    });
}

/// A C builtin's arguments and an interpolation's slots are subexps the
/// interpreter evaluates itself, the last the outermost loop, so a closure
/// call in one resolves (every expected answer is jq 1.7.1's).
#[test]
fn c_arguments_and_interpolation_slots_run_last_outermost_5223() {
    on_big_stack(|| {
        check(&[
            ("null", r#"def f: 1; "\(f)""#, ok(&[r#""1""#])),
            ("null", "def f: 2; pow(f; 3)", ok(&["8"])),
            ("null", "[pow((1,2);(3,4))]", ok(&["[1,8,1,16]"])),
            (
                "null",
                r#"["\(1,2)-\(3,4)"]"#,
                ok(&[r#"["1-3","2-3","1-4","2-4"]"#]),
            ),
            (
                "null",
                r#"[@base64 "x\(1,2)y\(3,4)"]"#,
                ok(&[r#"["xMQ==yMw==","xMg==yMw==","xMQ==yNA==","xMg==yNA=="]"#]),
            ),
        ]);
    });
}

/// The regex family runs `builtin.jq`'s bodies over the `_match_impl` hook:
/// `match/2` iterates the array of matches (so `path(match(..))` raises at
/// `EACH`), and `sub`'s replacement is a closure over each match's captures,
/// a generator included. Every expected answer is jq 1.7.1's.
#[test]
#[cfg(feature = "regex")]
fn the_regex_family_runs_from_builtin_jq_5223() {
    on_big_stack(|| {
        check(&[
            (
                r#""a1b2""#,
                r#"[match("[0-9]";"g") | .offset]"#,
                ok(&["[1,3]"]),
            ),
            (
                r#""abc""#,
                r#"test("B"; "i"), test("x")"#,
                ok(&["true", "false"]),
            ),
            (
                r#""xyz-123""#,
                r#"capture("(?<a>[a-z]+)-(?<n>[0-9]+)")"#,
                ok(&[r#"{"a":"xyz","n":"123"}"#]),
            ),
            (r#""a1b22""#, r#"[scan("[0-9]+")]"#, ok(&[r#"["1","22"]"#])),
            (
                r#""a1b2c""#,
                r#"[splits("[0-9]")]"#,
                ok(&[r#"["a","b","c"]"#]),
            ),
            (
                r#""a,b, c""#,
                r#"split(", *"; null)"#,
                ok(&[r#"["a","b","c"]"#]),
            ),
            (
                r#""aaa""#,
                r#"sub("a";"X"), gsub("a";"X")"#,
                ok(&[r#""Xaa""#, r#""XXX""#]),
            ),
            (
                r#""abc""#,
                r#"sub("(?<x>b)"; "[\(.x)]")"#,
                ok(&[r#""a[b]c""#]),
            ),
            (
                r#""abab""#,
                r#"gsub("(?<l>a)"; "\(.l|ascii_upcase)", "-")"#,
                ok(&[r#""AbAb""#, r#""-b-b""#]),
            ),
            (
                r#""ab""#,
                r#"[match(["a","g"]) | .string]"#,
                ok(&[r#"["a"]"#]),
            ),
            (
                r#""a""#,
                "match(1)",
                err(&[], "number not a string or array"),
            ),
            (
                r#"{"a":"x"}"#,
                r#"path(.a | match("x"))"#,
                err(
                    &[],
                    r#"Invalid path expression near attempt to iterate through [{"offset":0,"length":1,"s..."#,
                ),
            ),
            (
                r#""a""#,
                r#"path(test("a"))"#,
                err(&[], "Invalid path expression with result true"),
            ),
        ]);
    });
}

/// `leaf_paths` (a succinctly extension) answers what the shipped evaluator
/// answers: every childless node's path, the root's `[]` included.
#[test]
fn leaf_paths_matches_the_shipped_extension_5223() {
    on_big_stack(|| {
        check(&[
            (
                r#"{"a":{"b":1,"c":null},"d":[],"e":[2,3]}"#,
                "[leaf_paths]",
                ok(&[r#"[["a","b"],["a","c"],["d"],["e",0],["e",1]]"#]),
            ),
            (r#""x""#, "[leaf_paths]", ok(&["[[]]"])),
            ("[]", "[leaf_paths]", ok(&["[[]]"])),
            ("[[[]]]", "[leaf_paths]", ok(&["[[0,0]]"])),
        ]);
    });
}

/// The CLI environment reads as the shipped builtins answer it: `$__loc__`
/// (a constant), `$ENV` and `env`, and `input_filename` with no file (jq
/// 1.7.1's answers).
#[test]
fn the_cli_environment_reads_as_jq_5223() {
    on_big_stack(|| {
        check(&[(
            "null",
            "$__loc__, ($ENV|type), (env|type), input_filename",
            ok(&[
                r#"{"file":"<top-level>","line":1}"#,
                r#""object""#,
                r#""object""#,
                "null",
            ]),
        )]);
    });
}

/// A value substituted into the program (`Expr::TrackedVar`, as a library
/// caller or a CLI binding hands it over) is one `jv` however often it is
/// read, as `$t` is in jq 1.7.1: `--argjson t '[1]' '$t as $a | $t | path($a)'`
/// is `[]`.
#[test]
fn a_substituted_value_is_one_value_5223() {
    on_big_stack(|| {
        let marker = Expr::tracked_value(parse_doc("[1]"));
        fn substitute(expr: &Expr, marker: &Expr) -> Expr {
            match expr {
                Expr::Var(name) if name == "t" => marker.clone(),
                other => crate::jq::walk::map_subexprs(other, &mut |e| substitute(e, marker)),
            }
        }
        let program = substitute(&parse("$t as $a | $t | path($a)").expect("parses"), &marker);
        let prelude = Prelude::new();
        let run = Interp::new(&prelude).run(&program, OwnedValue::Null);
        assert!(matches!(run.outcome, Outcome::Done), "{:?}", run.outcome);
        let outputs: Vec<String> = run
            .outputs
            .iter()
            .map(OwnedValue::to_json_jq_preserve)
            .collect();
        assert_eq!(outputs, ["[]"]);
    });
}

/// The interpreter runs the resolved program, as the CLI hands it over, so a
/// `def` that shadows a builtin's name wins only at its own arity (jq 1.7.1).
#[test]
fn shadowed_builtins_resolve_as_jq_5223() {
    on_big_stack(|| {
        check(&[
            ("null", "def length: 5; length", ok(&["5"])),
            ("null", "def length(x): x; [1,2,3] | length", ok(&["3"])),
        ]);
    });
}

/// A recursion deeper than the native stack can hold is refused with the
/// shipped evaluator's uncatchable resource limit, never an overflow, and a
/// `try` does not catch it; one the stack holds still answers (a debug
/// build's frames are large, so the answering depth here is modest).
#[test]
// The floor lives in a thread-local, which `no_std` does not have.
#[cfg(feature = "std")]
fn a_recursion_too_deep_is_refused_not_overflowed_5223() {
    on_big_stack(|| {
        let def = "def f(n): if n == 0 then 0 else f(n - 1) end;";
        check(&[("null", &format!("{def} f(300)"), ok(&["0"]))]);
        // Which check trips first -- the call (`f/1 exceeded ...`) or an
        // argument read (`exceeded ...`) -- depends on where the floor
        // falls, as in the shipped evaluator.
        let dollar = "def f($n): if $n == 0 then 0 else f($n - 1) end;";
        for filter in [
            format!("{dollar} f(10000000)"),
            format!("{dollar} try f(10000000) catch 1"),
            format!("{def} f(10000000)"),
        ] {
            let run = run(&filter, OwnedValue::Null).expect("parses");
            match run.outcome {
                Outcome::Error(e) => {
                    assert!(e.is_uncatchable(), "{filter}: {e:?}");
                    assert!(
                        e.message.ends_with("exceeded maximum recursion depth"),
                        "{filter}: {}",
                        e.message
                    );
                }
                other => panic!("{filter}: {other:?}"),
            }
        }
    });
}

/// `until`, `while`, `range/3` and `recurse` run jq's tail-recursive bodies
/// on a heap work list when their arguments are pure, in jq's order, so a
/// long loop costs no native stack per iteration; an impure argument keeps
/// the `builtin.jq` body. The shipped evaluator's caps stay, each a
/// documented divergence from jq's unbounded loop: 100,000 `until`/`while`
/// states and `range` values, 10,000 nodes for a `recurse(f)` that is not a
/// tree descent, and `repeat`'s round limits. Every expected answer under a
/// cap is jq 1.7.1's; the capped rows are the shipped evaluator's.
#[test]
fn loops_run_in_constant_stack_and_in_jqs_order_5223() {
    on_big_stack(|| {
        check(&[
            ("0", "until(. >= 99999; . + 1)", ok(&["99999"])),
            (
                "0",
                "until(. >= 100000; . + 1)",
                err(&[], "until: maximum iterations exceeded"),
            ),
            ("0", "[range(0; 100000; 1)] | length", ok(&["100000"])),
            (
                "0",
                "[range(100001)] | length",
                err(&[], "range: maximum iterations exceeded"),
            ),
            (
                "0",
                "[recurse(if . < 9999 then . + 1 else empty end)] | length",
                ok(&["10000"]),
            ),
            (
                "0",
                "[recurse(if . < 50000 then . + 1 else empty end)] | length",
                err(&[], "recurse: maximum nodes exceeded"),
            ),
            (
                "1",
                "[while(. < 20; . * 2, . * 3)]",
                ok(&["[1,2,4,8,16,12,6,12,18,3,6,12,18,9,18]"]),
            ),
            (
                "1",
                "[until(. > 10; . * 2, . * 3)]",
                ok(&["[16,24,12,12,18,12,18,18,27]"]),
            ),
            (
                "[[1,[2]],3]",
                "[recurse(.[]?; type == \"array\")]",
                ok(&["[[[1,[2]],3],[1,[2]],[2]]"]),
            ),
            ("0", "[limit(3; repeat(1))]", ok(&["[1,1,1]"])),
            ("1", "[limit(5; repeat(. * 2))]", ok(&["[2,2,2,2,2]"])),
            ("0", "[limit(3; repeat(empty))]", ok(&["[]"])),
            ("[1,2]", "[path(repeat(.))] | length", ok(&["1000"])),
            (
                "[1,2]",
                "[path(limit(20000; repeat(.[])))] | length",
                ok(&["2000"]),
            ),
            ("0", "[until(. > 2; . + 1, error(\"x\"))]", err(&[], "x")),
            ("0", "[until(. > 2; (. + 1 | debug))] | length", ok(&["1"])),
        ]);
    });
}

/// Every routed library entry answers a dynamic path program as jq 1.7.1
/// does, through the interpreter: the collecting cursor entry, `eval_using` on
/// a document value and eval.rs's owned entry. A document that cannot be
/// materialized whole (a lone surrogate) stays on the shipped route, which
/// reads only what the program does (#4412).
#[test]
fn the_routed_library_entries_run_the_interpreter_5223() {
    fn json(values: &[OwnedValue]) -> Vec<String> {
        values.iter().map(OwnedValue::to_json_jq_preserve).collect()
    }
    on_big_stack(|| {
        // The shipped resolver refuses this row (`Invalid path expression with
        // result {"b":1,...}`), so an answer shows the interpreter ran.
        let doc = br#"{"a":{"b":1,"c":{"b":3}},"b":2}"#;
        let expr = parse("[path(. as $x | .a | $x | .a as $q | $q)]").expect("parses");
        let want = [r#"[["a"]]"#];
        let index = JsonIndex::build(doc);
        let collected = crate::jq::eval_generic::eval_with_cursor(&expr, index.root(doc))
            .collect_owned::<JqSemantics>()
            .expect("materializes");
        assert_eq!(json(&collected), want, "eval_with_cursor");
        let using =
            crate::jq::eval_generic::eval_using::<JqSemantics, _>(&expr, index.root(doc).value())
                .collect_owned::<JqSemantics>()
                .expect("materializes");
        assert_eq!(json(&using), want, "eval_using");
        let owned = to_owned::<JqSemantics, _>(&index.root(doc).value()).expect("doc");
        let result = crate::jq::eval::eval_owned_with_file_index::<Vec<u64>, JqSemantics>(
            &expr,
            &owned,
            &[],
        );
        assert_eq!(
            json(&result.collect_owned::<JqSemantics>()),
            want,
            "eval_owned_with_file_index"
        );

        let lone = br#"{"a":"\ud800","d":5}"#;
        let index = JsonIndex::build(lone);
        let expr = parse("path(try .d)").expect("parses");
        let collected = crate::jq::eval_generic::eval_with_cursor(&expr, index.root(lone))
            .collect_owned::<JqSemantics>()
            .expect("materializes");
        assert_eq!(json(&collected), [r#"["d"]"#]);
    });
}

/// `error({"__jq": n})` raised by hand is the break of the label numbered `n`
/// (jq numbers them from 0 per run, as `GENLABEL` does): the label stops, as
/// for `break $f`. A payload naming no live label is an ordinary error.
#[test]
fn a_hand_raised_label_payload_breaks_its_label_5223() {
    on_big_stack(|| {
        check(&[
            (
                "{}",
                r#"[path(label $f | .a, error({"__jq": 0}), .b)]"#,
                ok(&[r#"[["a"]]"#]),
            ),
            (
                "{}",
                r#"[path(label $f | .a, error({"__jq": 1}), .b)]"#,
                err(&[], r#"{"__jq":1}"#),
            ),
        ]);
    });
}

/// A program that skipped `resolve_all_jq` (a library caller parsing it
/// itself) raises the shipped evaluator's errors for an unbound variable or a
/// `break` with no enclosing label, rather than an internal error.
#[test]
fn an_unresolved_program_raises_the_shipped_errors_5223() {
    on_big_stack(|| {
        for (filter, message) in [
            ("path(if $x then .a else .b end)", "undefined variable: $x"),
            ("path(.a | (break $out))", "$*label-out is not defined"),
        ] {
            let program = parse(filter).expect(filter);
            let control = run_program(&program, OwnedValue::Null, &mut |_| true);
            match control {
                Some(Control::Error(e)) => assert_eq!(e.message, message, "{filter}"),
                other => panic!("{filter}: {other:?}"),
            }
        }
    });
}

/// `takes_program` routes a program to the interpreter exactly when it holds
/// a path target that is neither navigation nor a value filter over it, and
/// nothing the interpreter declines.
#[test]
fn takes_program_routes_dynamic_path_targets_only_5223() {
    let takes = |filter: &str| takes_program(&parse(filter).expect(filter));
    for filter in [
        "del(.users[] | .score as $y | select($y < 100))",
        "path(.a as $x | $x)",
        "(.. | numbers) |= . + 1",
        "[paths(type == \"number\") ] as $p | .a | path(first(.[] | if . then .a else .b end))",
        "def f: .a; f |= 1",
        "pick(first(.[] | . as $x | $x))",
        ".a //= (path(if . then .b else .c end) | length)",
        // A condition that can retry, call a function or nest a dynamic path.
        "path(.a | select(. as [$q] ?// $z | $z))",
        "def ok: true; del(.[] | select(ok))",
        "del(.[] | select(path(if . then .a else .b end) == [\"a\"]))",
        // A handler that does more than drop the error.
        "path(try .a catch .b)",
    ] {
        assert!(takes(filter), "{filter}");
    }
    for filter in [
        ".users[] | .name",
        "del(.a, .b[0])",
        "[path(.users[] | .age)] | length",
        "(.users[] | .score) = 1",
        ".a |= (.b | select(. > 1))",
        "path(..)",
        "[paths]",
        "to_entries",
        "path(.a | select(.)) | line",
        // Value filters over navigation.
        "del(.users[] | select(.score < 100))",
        "pick(first(.[]))",
        "path(try .a catch empty)",
        "path(.a // .b)",
        "[path(.[$k])]",
        "path(recurse(.[]?))",
    ] {
        assert!(!takes(filter), "{filter}");
    }
}

/// `skip`, `isvalid` and `recurse_down` (succinctly extensions) answer what
/// the shipped evaluator answers, in path mode too.
#[test]
fn the_extension_builtins_match_the_shipped_evaluator_5223() {
    on_big_stack(|| {
        check(&[
            (
                "null",
                "[skip(2; 1,2,3,4)], [skip(0; 1,2)], [skip(5; 1,2)]",
                ok(&["[3,4]", "[1,2]", "[]"]),
            ),
            (
                "null",
                "try [skip(-1; 1,2)] catch .",
                ok(&[r#""skip doesn't support negative count""#]),
            ),
            (
                "null",
                "isvalid(1), isvalid(empty), isvalid(error), isvalid(1, error)",
                ok(&["true", "false", "false", "false"]),
            ),
            ("[1,[2]]", "[recurse_down]", ok(&["[[1,[2]],1,[2],2]"])),
            (
                r#"{"a":[1,2,3]}"#,
                "[path(skip(1; .a[]))]",
                ok(&[r#"[["a",1],["a",2]]"#]),
            ),
        ]);
    });
}

/// `|=`, `=`, the update-assignments and `//=` write in place outside path
/// mode, with `_modify`'s meaning: the update's first output is written, an
/// update with none deletes the path (all deletions at the end), and a `?//`
/// in the update (which can catch the fold's `break`) runs jq's own body,
/// quirk included. Every expected answer is jq 1.7.1's.
#[test]
fn updates_and_assignments_follow_modify_and_assign_5223() {
    on_big_stack(|| {
        check(&[
            (
                r#"{"a":[1,2]}"#,
                ".a[] |= . * 10",
                ok(&[r#"{"a":[10,20]}"#]),
            ),
            (
                r#"{"a":[1,{"b":2}],"c":"x"}"#,
                "(.. | numbers) |= . + 1",
                ok(&[r#"{"a":[2,{"b":3}],"c":"x"}"#]),
            ),
            (r#"{"a":1,"b":2}"#, ".a |= empty", ok(&[r#"{"b":2}"#])),
            (
                r#"{"a":1,"b":2,"c":3}"#,
                "(.a, .b) |= empty",
                ok(&[r#"{"c":3}"#]),
            ),
            ("[1,2,3]", ".[] |= select(. > 1)", ok(&["[2,3]"])),
            (r#"{"a":1}"#, ".a |= (1,2)", ok(&[r#"{"a":1}"#])),
            (
                r#"{"a":[1]}"#,
                ".a |= (. as [$x] ?// $x | $x)",
                err(&[], "Paths must be specified as an array"),
            ),
            (r#"{"a":0}"#, "[.a = (1,2)]", ok(&[r#"[{"a":1},{"a":2}]"#])),
            (
                r#"{"a":1}"#,
                ".a += 1, .a //= 3, (.b //= 3)",
                ok(&[r#"{"a":2}"#, r#"{"a":1}"#, r#"{"a":1,"b":3}"#]),
            ),
            (
                r#"{"a":1}"#,
                ".a.b |= 1",
                err(&[], r#"Cannot index number with string "b""#),
            ),
            (
                r#"{"a":1}"#,
                "try (.a.b = 1) catch .",
                ok(&[r#""Cannot index number with string \"b\"""#]),
            ),
            (
                r#"{"a":{"b":1}}"#,
                "(.a | select(.b) | .b) |= . + 1",
                ok(&[r#"{"a":{"b":2}}"#]),
            ),
            ("null", "(.a[2] |= 5)", ok(&[r#"{"a":[null,null,5]}"#])),
            ("[1,2,3]", "(.[] | select(. == 2)) |= empty", ok(&["[1,3]"])),
            (
                r#"{"a":[1,2,3]}"#,
                "del(.a[] | select(. >= 2))",
                ok(&[r#"{"a":[1]}"#]),
            ),
            // A `?//` in the paths catches the write's error (the fold runs in
            // the path generator's fork) and retries into `LOADVN`-emptied
            // variables.
            (
                "[1]",
                "(. as $x ?// $y | .[if $x == null then 0 else -5 end]) |= 1",
                err(&[], "Paths must be specified as an array"),
            ),
            (
                r#"[1,"a"]"#,
                r#"(. as $x ?// $y | .[if $x == null then 1 else 0 end]) += "x""#,
                err(&[], "Paths must be specified as an array"),
            ),
        ]);
    });
}

/// Rows the shadow run found the interpreter wrong on, fixed from jq's
/// source: a slice descriptor missing a bound is not a slice (`jv_object_get`
/// answers *invalid*), `range/1`'s argument is a `$x` binding (a subexp), and
/// a slice 65,536 or more elements into its root is a copy, not a view (`jv`'s
/// 16-bit `offset`). Every expected answer is jq 1.7.1's.
#[test]
fn shadow_findings_match_jq_5223() {
    on_big_stack(|| {
        check(&[
            (
                "[1,2,3]",
                r#"path(.[{"start":1}])"#,
                err(&[], "Array/string slice indices must be integers"),
            ),
            (
                r#"{"a":{"b":1}}"#,
                "path(reduce range([1] as [$a]|$a) as $k (.; .))",
                ok(&["[]"]),
            ),
            (
                "null",
                r#"{"a":[range(70003)]} | try path(.a[70000:] as $x | .a[70000:][0:] | $x) catch "c""#,
                ok(&[r#""c""#]),
            ),
            (
                "null",
                "[range(70003)] | .[60000:] | [path(.[5535:] as $x | .[5535:] | $x)]",
                ok(&[r#"[[{"start":5535,"end":null}]]"#]),
            ),
            (
                "null",
                r#"[range(70003)] | .[60000:] | try path(.[5536:] as $x | .[5536:] | $x) catch "c""#,
                ok(&[r#""c""#]),
            ),
        ]);
    });
}

/// A program's own `$__lvn_x` is an ordinary variable (the prelude's `LOADVN`
/// spelling is renamed to one no jq name can hold), and a closure argument
/// that is a generator stays lazy, so `limit`/`first` stop it after the
/// outputs they take. Every expected answer is jq 1.7.1's.
#[test]
fn loadvn_and_generator_arguments_keep_jqs_meaning_5223() {
    on_big_stack(|| {
        check(&[
            (
                "null",
                "1 as $x | 2 as $__lvn_x | [$__lvn_x, $x]",
                ok(&["[2,1]"]),
            ),
            ("null", "3 as $__lvn_y | $__lvn_y", ok(&["3"])),
            (
                "null",
                "def g(x): x; [limit(3; g(range(100000000)))]",
                ok(&["[0,1,2]"]),
            ),
            (
                "null",
                "def g(x): first(x); g(range(100000000))",
                ok(&["0"]),
            ),
        ]);
    });
}
