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
        // Each arithmetic update-assignment, which the prelude spells as its
        // own `_sp_upd_*` call.
        for op in ["+", "-", "*", "/", "%"] {
            rows.push((doc.to_string(), format!("(.a, .b) {op}= 2")));
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
/// documented divergence from jq's unbounded loop: 100,000 forks and 16.7M
/// rounds for `until`/`while` (#5292), 2^20 nodes for a `recurse(f)` that is
/// not a tree descent (#5378), and `repeat`'s round limits. Every expected
/// answer under a cap is jq 1.7.1's; the capped rows are the shipped
/// evaluator's, and `until_while_caps_hold_in_the_path_interpreter_5292` pins
/// the loop caps.
#[test]
fn loops_run_in_constant_stack_and_in_jqs_order_5223() {
    on_big_stack(|| {
        check(&[
            ("0", "until(. >= 99999; . + 1)", ok(&["99999"])),
            // #5292: past 100,000 states `until` goes on, as jq's does.
            ("0", "until(. >= 300000; . + 1)", ok(&["300000"])),
            ("0", "[range(0; 100000; 1)] | length", ok(&["100000"])),
            // #5292: past 100,000 values `range` goes on, as jq's does.
            ("0", "[range(100001)] | length", ok(&["100001"])),
            (
                "0",
                "[recurse(if . < 9999 then . + 1 else empty end)] | length",
                ok(&["10000"]),
            ),
            // #5378: the cap is 2^20 nodes, so 50,001 answers as jq's does.
            (
                "0",
                "[recurse(if . < 50000 then . + 1 else empty end)] | length",
                ok(&["50001"]),
            ),
            (
                "0",
                "[recurse(if . < 1100000 then . + 1 else empty end)] | length",
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

/// #5379: a call with no arguments, last in the body of a `def`, runs as a loop, as jq's
/// `TAIL_CALL_JQ` does. The nested form was refused at `MAX_EVAL_FRAMES` live calls (about
/// 13,000 levels here, on the native-stack floor). The register follows the input from stage to
/// stage, so a path through the loop is the path jq reports.
#[test]
fn a_tail_recursive_def_runs_as_a_loop_5379() {
    on_big_stack(|| {
        check(&[
            (
                "0",
                "def f: if . < 300000 then . + 1 | f else . end; f",
                ok(&["300000"]),
            ),
            (
                r#"{"a":0}"#,
                "def f: if . < 200000 then . + 1 | f else . end; [(.a | f), path(.a)]",
                ok(&[r#"[200000,["a"]]"#]),
            ),
            // The register is carried through the stages in front of the call.
            (
                r#"{"a":{"a":{"a":0}}}"#,
                r#"def f: if type == "object" then .a | f else . end; [path(f)]"#,
                ok(&[r#"[["a","a","a"]]"#]),
            ),
            (
                r#"{"a":{"a":{"a":0}}}"#,
                r#"def f: if type == "object" then .a | f else . end; path(f)"#,
                ok(&[r#"["a","a","a"]"#]),
            ),
            (
                "[0]",
                "def f: if (.[0] // 0) < 200000 then .[0] += 1 | f else .[0] end; [f, path(.[0])]",
                ok(&["[200000,[0]]"]),
            ),
            // An outer def called from a nested one.
            (
                "0",
                "def f: def h: f; if . < 200000 then . + 1 | h else . end; f",
                ok(&["200000"]),
            ),
            // An `as` binding in front of the call: the body runs from the register at
            // entry, the source from `reg.sub()`.
            (
                "0",
                "def f: if . < 200000 then . as $x | ($x + 1) as $y | $y | f else . end; f",
                ok(&["200000"]),
            ),
            (
                r#"{"a":0}"#,
                "def f: .a as $n | if $n < 200000 then .a = $n + 1 | f else $n end; [f, path(.a)]",
                ok(&[r#"[200000,["a"]]"#]),
            ),
            // A comma emits its front branches before the call, and a consumer that
            // stops ends the loop.
            (
                "0",
                "def f: if . < 5 then ., (. + 1 | f) else . end; [f]",
                ok(&["[0,1,2,3,4,5]"]),
            ),
            (
                "0",
                "[limit(3; def f: ., (. + 1 | f); f)]",
                ok(&["[0,1,2]"]),
            ),
            (
                "0",
                r#"def f: if . < 200000 then . + 1 | f else error("boom") end; try f catch ."#,
                ok(&[r#""boom""#]),
            ),
            // The stages the loop passes through do not make a path of what they build.
            (
                "0",
                "def f: if . < 3 then . + 1 | f else . end; [path(f)]",
                err(&[], "Invalid path expression with result 3"),
            ),
            // jq loops on this forever; the round budget stops it with the refusal the
            // nested form raised.
            (
                "0",
                "def f: f; f",
                err(&[], "f/0 exceeded maximum recursion depth"),
            ),
        ]);
        // Not a call jq jumps from: it still nests, and still reaches the guard. The
        // guard is the native-stack floor, a thread-local `no_std` does not have.
        #[cfg(feature = "std")]
        check(&[(
            "0",
            "def f: [f]; f",
            err(&[], "f/0 exceeded maximum recursion depth"),
        )]);
    });
}

/// #5379: what the tail loop asks of the interpreter's `single` is bounded, and what it
/// learns while the bound runs out is not kept. A closure parameter read once the fuel of a
/// loop's question was spent used to be cached as not single-valued for good, and every deeper
/// read of it then nested: the program answers `[0,[]]` in jq 1.7.1, and did on the shipped
/// route. A stage a long chain of `+ 0` makes too big for the interpreter's own question is
/// still one the shared question settles, so the loop runs on both routes.
#[test]
fn the_tail_loops_settle_question_is_bounded_and_not_remembered_5379() {
    on_big_stack(|| {
        let plus_zeros = " + 0".repeat(40);
        let deep_arg = format!(
            "def r(n): def g: r(n - 1{plus_zeros}); def f: if n > 0 then g else . end; f; \
             [(0 | r(100)), path(if true then . else . end)]"
        );
        let long_stage = format!(
            "def f: if . < 20000 then . + 1{plus_zeros} | f else . end; \
             [(0 | f), path(if true then .a else .b end)]"
        );
        check(&[
            ("0", &deep_arg, ok(&["[0,[]]"])),
            (r#"{"a":1}"#, &long_stage, ok(&[r#"[20000,["a"]]"#])),
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

        // A document the interpreter cannot materialize whole (a lone surrogate)
        // raises the decode error on every entry, where jq rejects the document
        // when it parses it (#5233). The shipped route no longer answers a
        // dynamic target in its place.
        let lone = br#"{"a":"\ud800","d":5}"#;
        let index = JsonIndex::build(lone);
        let expr = parse("[.d | path(if . then . else empty end)]").expect("parses");
        assert!(matches!(route(&expr), Route::Interpreter));
        use crate::jq::eval_generic::GenericResult;
        let collected = crate::jq::eval_generic::eval_with_cursor(&expr, index.root(lone));
        assert!(
            matches!(&collected, GenericResult::Error(e) if e.is_decode_failure()),
            "eval_with_cursor: {collected:?}"
        );
        let using =
            crate::jq::eval_generic::eval_using::<JqSemantics, _>(&expr, index.root(lone).value());
        assert!(
            matches!(&using, GenericResult::Error(e) if e.is_decode_failure()),
            "eval_using: {using:?}"
        );
        let mut streamed = 0;
        let escape =
            crate::jq::eval_generic::eval_each_with_cursor(&expr, index.root(lone), &mut |_| {
                streamed += 1;
                true
            });
        assert_eq!(streamed, 0);
        assert!(
            matches!(&escape, Some(Control::Error(e)) if e.is_decode_failure()),
            "eval_each_with_cursor: {escape:?}"
        );

        // A taken program that raises ends the owned entry's run with the error,
        // after what it emitted.
        let doc = br#"{"a":1}"#;
        let index = JsonIndex::build(doc);
        let owned = to_owned::<JqSemantics, _>(&index.root(doc).value()).expect("doc");
        let expr = parse("path(.a | if . then ., error(\"boom\") else empty end)").expect("parses");
        match crate::jq::eval::eval_owned_with_file_index::<Vec<u64>, JqSemantics>(
            &expr,
            &owned,
            &[],
        ) {
            crate::jq::eval::QueryResult::Partial(outputs, Control::Error(e)) => {
                assert_eq!(json(&outputs), [r#"["a"]"#]);
                assert_eq!(e.message, "boom");
            }
            other => panic!("expected a partial result, got {other:?}"),
        }
    });
}

/// The interpreter's internals a CLI run does not reach: `unshare_top` gives
/// a result identical to a builtin's input storage of its own (a container at
/// the top only, a string or a literal whole); `raise_eval` keeps a resource
/// limit uncatchable; and a consumer that stops ends `run_program` cleanly
/// after the outputs it took.
#[test]
fn the_interpreter_internals_a_cli_run_does_not_reach_5223() {
    on_big_stack(|| {
        let inner = OwnedValue::from(alloc::vec![OwnedValue::Int(1)]);
        let array = OwnedValue::from(alloc::vec![inner.clone()]);
        let unshared = unshare_top(array.clone());
        assert!(!jv_identical(&unshared, &array));
        assert_eq!(unshared, array);
        let OwnedValue::Array(items) = &unshared else {
            panic!("an array")
        };
        assert!(jv_identical(&items[0], &inner), "the contents stay shared");
        let mut map = IndexMap::new();
        map.insert(String::from("k"), inner.clone());
        let object = OwnedValue::Object(map.into());
        let unshared = unshare_top(object.clone());
        assert!(!jv_identical(&unshared, &object));
        assert_eq!(unshared, object);
        for scalar in [string("s"), literal(NumberRepr::Int(5), "5")] {
            let unshared = unshare_top(scalar.clone());
            assert!(!jv_identical(&unshared, &scalar));
            assert_eq!(unshared, scalar);
        }
        assert!(matches!(
            raise_eval(EvalError::resource_limit("cap")),
            Raise::Fatal(_)
        ));

        let program = parse(".[]").expect("parses");
        let input = OwnedValue::from(alloc::vec![OwnedValue::Int(1), OwnedValue::Int(2)]);
        let mut taken = Vec::new();
        let control = run_program(&program, input, &mut |v| {
            taken.push(v);
            false
        });
        assert!(control.is_none());
        assert_eq!(taken, [OwnedValue::Int(1)]);
    });
}

/// #5292: the interpreter's `until`/`while` are bounded without a counter of their
/// own length: a fork spends the fork budget (an `update`, a `cond`, or a trailing
/// error that makes a round answer other than once), a loop that never ends the round
/// cap, a `while` that keeps emitting the emission cap -- each the uncatchable
/// resource limit, whatever the program wraps it in.
#[test]
fn until_while_caps_hold_in_the_path_interpreter_5292() {
    on_big_stack(|| {
        check(&[
            (
                "0",
                "until(false; ., .)",
                err(&[], "until: maximum iterations exceeded"),
            ),
            (
                "0",
                "[while(true; ., .)] | length",
                err(&[], "while: maximum iterations exceeded"),
            ),
            // A round that answers once and then raises is a fork: it used to park
            // one error per round and run all 2^24 of them.
            (
                "0",
                "[until((false, error(\"x\")); .)] | length",
                err(&[], "until: maximum iterations exceeded"),
            ),
            (
                "0",
                "[until(false; (., error(\"x\")))] | length",
                err(&[], "until: maximum iterations exceeded"),
            ),
            // ...uncatchable.
            (
                "0",
                "try (until(false; ., .)) catch \"caught\"",
                err(&[], "until: maximum iterations exceeded"),
            ),
            (
                "0",
                "[while(true; .+1)] | length",
                err(&[], "while: maximum iterations exceeded"),
            ),
            // A branch is charged when it is entered, not when its siblings are
            // queued: a consumer that stops never enters the rest (this spent the
            // fork budget at the first element of a 200,000-element array).
            (
                "null",
                "[range(200000)] | first(path(until(type == \"number\"; .[])))",
                ok(&["[0]"]),
            ),
            // A round whose `cond` and `step` both fork spends one fork per branch
            // entered, as the value evaluator does (131,070 here before: over 100,000).
            (
                "{\"a\":{\"a\":{\"a\":{\"a\":{\"a\":{\"a\":{\"a\":{\"a\":1}}}}}}}}",
                "[path(until(type != \"object\", type != \"object\"; .a, .a))] | length",
                ok(&["131072"]),
            ),
            // A trailing error makes the round a fork, so it is not an unbounded
            // lineage: the pre-change interpreter answered 150,000 states here.
            (
                "0",
                "[limit(150000; path(while((true, error(\"x\")); .)))] | length",
                err(&[], "while: maximum iterations exceeded"),
            ),
            // A loop that forks once and then steps in place is as long as it is.
            (
                "0",
                "[until(. >= 150000; if . == 0 then (1, 2) else .+1 end)]",
                ok(&["[150000,150000]"]),
            ),
        ]);
    });
}

/// Rows that reach the interpreter's less common arms, each jq 1.7.1's answer.
#[test]
fn less_common_interpreter_arms_answer_as_jq_does_5223() {
    on_big_stack(|| {
        check(&[
            ("null", "[range(0; 200000; 1)] | length", ok(&["200000"])),
            // A zero-arity def as a C builtin's argument.
            ("null", r#"def f: "a"; "ab" | ltrimstr(f)"#, ok(&[r#""b""#])),
            // An `until` condition that raises.
            (
                "null",
                r#"try (0 | until(error("c"); . + 1)) catch ."#,
                ok(&[r#""c""#]),
            ),
            // A `recurse` condition that forks.
            (
                "[[1],[2]]",
                "[recurse(.[]?; (true, true))] | length",
                ok(&["13"]),
            ),
            // A constant sum that hands back its left operand is that constant.
            (
                "null",
                "[path(([1] + []) as $c | $c | .[0])]",
                err(
                    &[],
                    "Invalid path expression near attempt to access element 0 of [1]",
                ),
            ),
            // An object slice key, and a `null` bound.
            (
                "[1,2,3]",
                r#"[path(.[{"start":1,"end":null}])]"#,
                ok(&[r#"[[{"start":1,"end":null}]]"#]),
            ),
            (
                "[1,2,3]",
                "[path(.[null:2])]",
                ok(&[r#"[[{"start":null,"end":2}]]"#]),
            ),
            // A def, and a closure parameter, named twice in a write's paths.
            ("{\"a\":1}", "def f: .a; (f, f) |= 1", ok(&[r#"{"a":1}"#])),
            (
                "{\"a\":1}",
                "def h(g): (g, g) |= 1; h(.a)",
                ok(&[r#"{"a":1}"#]),
            ),
        ]);
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

/// Every routed library entry raises the refusal for a program `route` refuses
/// (the CLI reports it at compile time, before any entry runs): a library
/// caller reaches it through the entry itself (#5233).
#[test]
fn every_routed_library_entry_raises_the_refusal_5233() {
    on_big_stack(|| {
        let doc = br#"{"a":{"b":1}}"#;
        let index = JsonIndex::build(doc);
        let expr = parse("path(.a as $x | .b), key").expect("parses");
        let Route::Refused(want) = route(&expr) else {
            panic!("not refused");
        };
        use crate::jq::eval::QueryResult;
        use crate::jq::eval_generic::GenericResult;

        let result = crate::jq::eval::eval_full::<Vec<u64>, JqSemantics>(&expr, index.root(doc));
        assert!(
            matches!(&result, QueryResult::Error(e) if e.message == want.message),
            "eval_full"
        );
        let owned = to_owned::<JqSemantics, _>(&index.root(doc).value()).expect("doc");
        let result = crate::jq::eval::eval_owned_with_file_index::<Vec<u64>, JqSemantics>(
            &expr,
            &owned,
            &[],
        );
        assert!(
            matches!(&result, QueryResult::Error(e) if e.message == want.message),
            "eval_owned_with_file_index"
        );
        let result = crate::jq::eval_generic::eval_with_cursor(&expr, index.root(doc));
        assert!(
            matches!(&result, GenericResult::Error(e) if e.message == want.message),
            "eval_with_cursor"
        );
        let result =
            crate::jq::eval_generic::eval_using::<JqSemantics, _>(&expr, index.root(doc).value());
        assert!(
            matches!(&result, GenericResult::Error(e) if e.message == want.message),
            "eval_using"
        );
        let escape =
            crate::jq::eval_generic::eval_each_with_cursor(&expr, index.root(doc), &mut |_| true);
        assert!(
            matches!(&escape, Some(Control::Error(e)) if e.message == want.message),
            "eval_each_with_cursor"
        );
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

/// `route` sends a program to the interpreter exactly when it holds a path
/// target that is neither navigation nor a value filter over it, and nothing
/// the interpreter declines; with something it declines, it refuses it (#5233).
#[test]
fn route_sends_dynamic_path_targets_to_the_interpreter_only_5223() {
    let takes = |filter: &str| matches!(route(&parse(filter).expect(filter)), Route::Interpreter);
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
        // An index key that can retry, calls a function, or holds a dynamic
        // target of its own (#5243).
        ".[. as [$a] ?// $a | $a] = 1",
        "def k: \"a\"; .[k] = 1",
        ".[path(if . then .a else .b end) | length] = 1",
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
        // A value-only index key, which jq runs as a subexpression (#5243).
        "reduce .[] as $r ({}; .[$r.name] = $r.score)",
        "reduce .[] as $r ({}; .x[$r.name | tostring] += 1)",
        ".[.k] = 1",
        "del(.[(0, 1)])",
        "path(.a[$i + 1][length - 1])",
        "(.[$x // \"z\"] | select(. > 1)) |= . + 1",
        // A NaN key, which the walker carries as jq does (#5245).
        "path(.[nan])",
        "path(.a[$x | . / nan])",
    ] {
        assert!(!takes(filter), "{filter}");
        assert!(
            matches!(route(&parse(filter).expect(filter)), Route::Shipped),
            "{filter}"
        );
    }
}

/// A program with a dynamic path target that also uses an extension builtin
/// the interpreter does not run is refused, naming it, wherever the builtin is
/// (#5233). Every one is a succinctly extension jq 1.7.1 does not define
/// (`key/0 is not defined`, ...), so jq rejects each program whole; the same
/// construct beside static targets only stays on the shipped route.
#[test]
fn route_refuses_a_dynamic_path_target_beside_a_declined_construct_5233() {
    for (filter, name) in [
        ("path(.a as $x | .b), key", "key"),
        ("[path(.a | key as $k | .b)]", "key"),
        ("file_index, del(.[] | . as $x | $x)", "file_index"),
        ("path(.a | if . then path else . end)", "path"),
        ("path(.a as $x | .b) | parent", "parent"),
        ("path(.a as $x | .b) | parent(1)", "parent"),
        ("del(.a | . as $x | .b), line", "line"),
        ("column, (.a as $x | .b) |= 1", "column"),
        (
            "document_index, path(first(.[] | . as $x | $x))",
            "document_index",
        ),
        ("anchor, path(.a as $x | .b)", "anchor"),
        ("style, path(.a as $x | .b)", "style"),
        ("line_comment, path(.a as $x | .b)", "line_comment"),
        ("head_comment, path(.a as $x | .b)", "head_comment"),
        ("foot_comment, path(.a as $x | .b)", "foot_comment"),
        ("at_offset(0), path(.a as $x | .b)", "at_offset"),
        ("at_position(1; 1), path(.a as $x | .b)", "at_position"),
    ] {
        let program = parse(filter).expect(filter);
        let Route::Refused(error) = route(&program) else {
            panic!("{filter}: not refused");
        };
        assert_eq!(
            error.message,
            format!(
                "{name} is not supported in a program whose path expression does more than navigate"
            ),
            "{filter}"
        );
    }
    let namespaced = Expr::Pipe(
        vec![
            Expr::NamespacedCall {
                namespace: "m".into(),
                name: "f".into(),
                args: Vec::new(),
            },
            parse("path(.a as $x | .b)").expect("parses"),
        ]
        .into(),
    );
    // A module call (rewritten away by the CLI) and the shipped evaluator's own
    // rewrites keep the shipped route, as before #5233: a `Shared` argument
    // under a target that only its substitution made dynamic is one.
    assert!(matches!(route(&namespaced), Route::Shipped));
    let rewritten = Expr::Pipe(
        vec![
            Expr::Builtin(Builtin::Key),
            Expr::Builtin(Builtin::Path(Box::new(Expr::shared(Expr::Identity)))),
        ]
        .into(),
    );
    assert!(matches!(route(&rewritten), Route::Shipped));
    for filter in ["path(.a) | key", "del(.a), line", "[paths] | at_offset(0)"] {
        assert!(
            matches!(route(&parse(filter).expect(filter)), Route::Shipped),
            "{filter}"
        );
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

/// `filename` is a yq builtin, so the jq parser never hands it to the router,
/// but the name table is shared: each path-context builtin is named, and a
/// builtin that reads no path context is not (#5233).
#[test]
fn path_context_builtins_are_named_5233() {
    assert_eq!(path_context_name(&Builtin::Key), Some("key"));
    assert_eq!(path_context_name(&Builtin::FileIndex), Some("file_index"));
    assert_eq!(path_context_name(&Builtin::Filename), Some("filename"));
    assert_eq!(path_context_name(&Builtin::PathNoArg), Some("path"));
    assert_eq!(path_context_name(&Builtin::Parent), Some("parent"));
    assert_eq!(
        path_context_name(&Builtin::ParentN(Box::new(Expr::Identity))),
        Some("parent")
    );
    assert_eq!(path_context_name(&Builtin::Length), None);
}

/// An index key that reads a succinctly extension (`line`, `key`, ...) keeps the
/// target dynamic, so the program is refused as before #5243: the walker would
/// evaluate the key off the cursor and read the wrong node.
#[test]
fn an_extension_in_an_index_key_keeps_the_target_dynamic_5243() {
    for filter in [
        ".a[line | tostring] = 9",
        "[path(.a[line | tostring])]",
        "del(.[key])",
        ".x[$v + column] |= 1",
    ] {
        assert!(
            matches!(route(&parse(filter).expect(filter)), Route::Refused(_)),
            "{filter}"
        );
    }
}

/// Routing a key that nests path targets inside index keys inside path targets
/// is linear in the nesting: `is_value_only_condition` does not re-walk a
/// nested target `is_static_target` already judged (it was exponential, about
/// 4x per two levels, before the #5243 review).
#[test]
fn nested_index_keys_route_in_linear_time_5243() {
    on_big_stack(|| {
        let mut target = ".a".to_string();
        for _ in 0..64 {
            target = format!(".[path({target}) | length]");
        }
        let program = parse(&format!("{target} = 1")).expect("parses");
        assert!(matches!(route(&program), Route::Shipped));
    });
}

/// A key or condition holding a path construct of its own is judged by that
/// construct's parts: static when its target is static and an assignment's
/// right side only computes a value, dynamic otherwise (#5243).
#[test]
fn an_index_key_holding_an_assignment_is_judged_by_its_parts_5243() {
    for filter in [
        ".[(.a = 7) | .a | tostring] = 2",
        ".[(.a |= . + 1) | .a | tostring] = 2",
        ".[(.a += 3) | .a | tostring] = 2",
        ".[(.z //= \"q\") | .z] = 2",
        "[path(.[(.b = \"a\") | .b])]",
    ] {
        assert!(
            matches!(route(&parse(filter).expect(filter)), Route::Shipped),
            "{filter}"
        );
    }
    for filter in [
        "def f: 9; .[(.a = f) | .a | tostring] = 2",
        ".[(path(if . then .a else .b end) | length)] = 2",
        ".[(del(.a as $x | $x)) | length] = 2",
    ] {
        assert!(
            matches!(route(&parse(filter).expect(filter)), Route::Interpreter),
            "{filter}"
        );
    }
}
