//! #7719, #7724: jq's `break $x` raises `{"__jq":N}`, `N` the number jq's one label counter
//! gave the label instance `$x` names, and `label` stops on any error equal to its own value. A
//! `catch` between them sees that object, a hand-made `error({"__jq":N})` is that label's break,
//! the labels jq's `builtin.jq` enters (`first`, `limit`, `isempty`, `_modify`, ...) advance the
//! counter, which keeps counting across inputs, and a `break` stops at the instance it is bound
//! to, not the innermost label of its name. A program that can see any of this runs on the
//! path-mode interpreter (`path_mode::route`). Every row was captured from jq 1.7.1
//! (`/usr/bin/jq`); jq 1.8.2 agrees on each except `limit(-1; 1)`, which it refuses (1.7.1 kept).

use anyhow::Result;

use crate::run_jq_full;

/// `(args, stdin, stdout, stderr, exit)`, as jq 1.7.1 answers.
type Row = (
    &'static [&'static str],
    Option<&'static str>,
    &'static str,
    &'static str,
    i32,
);

fn check(rows: &[Row]) -> Result<()> {
    for &(args, stdin, stdout, stderr, code) in rows {
        let (out, err, status) = run_jq_full(args, stdin)?;
        assert_eq!(
            (out.as_str(), err.as_str(), status),
            (stdout, stderr, code),
            "{args:?} on {stdin:?}"
        );
    }
    Ok(())
}

/// A `catch` between a `break` and its label sees `{"__jq":N}`, and a re-raise of it is stopped by that label.
#[test]
fn test_label_values_caught_7719() -> Result<()> {
    check(&[
        (
            &[r"-nc", r"label $f | try (1, break $f) catch error"],
            None,
            r"1
",
            r"",
            0,
        ),
        (
            &[r"-nc", r"[label $f | try break $f catch type]"],
            None,
            r#"["object"]
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r"[label $f | try break $f catch .]"],
            None,
            r#"[{"__jq":0}]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"first(1) as $x | [label $f | try break $f catch .]",
            ],
            None,
            r#"[{"__jq":1}]
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r"[label $f | label $g | try break $f catch .]"],
            None,
            r#"[{"__jq":0}]
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r"[label $f | label $g | try break $g catch .]"],
            None,
            r#"[{"__jq":1}]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"[label $a | 1, (label $b | try break $a catch .), 2]",
            ],
            None,
            r#"[1,{"__jq":0},2]
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r"[label $f | try break $f catch (.__jq)]"],
            None,
            r"[0]
",
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"[label $f | try break $f catch (.__jq += 1 | error)]",
            ],
            None,
            r"",
            r#"jq: error (at <unknown>) (not a string): {"__jq":1}
"#,
            5,
        ),
        (
            &[r"-nc", r"[label $f | try break $f catch (.x = 1 | error)]"],
            None,
            r"",
            r#"jq: error (at <unknown>) (not a string): {"__jq":0,"x":1}
"#,
            5,
        ),
        (
            &[r"-nc", r"[label $f | 1, error([0]), 2]"],
            None,
            r"",
            r"jq: error (at <unknown>) (not a string): [0]
",
            5,
        ),
        (
            &[r"-nc", r"[label $f | (1, break $f, 2)?, 3]"],
            None,
            r"[1,3]
",
            r"",
            0,
        ),
        (
            &[r"-nc", r"[label $f | (break $f) // 5, 3]"],
            None,
            r"[]
",
            r"",
            0,
        ),
        (
            &[r"-nc", r"[label $f | try (1, break $f) catch error, 3]"],
            None,
            r"[1]
",
            r"",
            0,
        ),
        (
            &[r"-nc", r"[label $f | try break $f catch error]"],
            None,
            r"[]
",
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"label $f | try (1, break $f) catch (. as $s | error($s))",
            ],
            None,
            r"1
",
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"[label $f | try (try break $f catch error) catch .]",
            ],
            None,
            r#"[{"__jq":0}]
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r"[label $f | try break $f]"],
            None,
            r"[]
",
            r"",
            0,
        ),
        (
            &[r"-nc", r"[label $f | (try break $f catch .) as $x | $x]"],
            None,
            r#"[{"__jq":0}]
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r"[label $f | path(try break $f catch .)]"],
            None,
            r"",
            r#"jq: error (at <unknown>): Invalid path expression with result {"__jq":0}
"#,
            5,
        ),
        (
            &[r"-nc", r"[label $f | try break $f catch tojson]"],
            None,
            r#"["{\"__jq\":0}"]
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r"[label $f | try break $f catch keys]"],
            None,
            r#"[["__jq"]]
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r"[label $f | break $f, 1]"],
            None,
            r"[]
",
            r"",
            0,
        ),
        (
            &[r"-nc", r"label $f | error(null)"],
            None,
            r"",
            r"jq: error (at <unknown>) (not a string): null
",
            5,
        ),
        (
            &[r"-nc", r"[label $f | try error(null) catch .]"],
            None,
            r"[null]
",
            r"",
            0,
        ),
        (
            &[r"-nc", r"[label $f | try (label $g | break $f) catch ., 4]"],
            None,
            r#"[{"__jq":0},4]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r#"[label $out | 1, (try (2, break $out) catch "c"), 3]"#,
            ],
            None,
            r#"[1,2,"c",3]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"[label $a | label $b | try break $a catch error, 3]",
            ],
            None,
            r"[]
",
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"[label $a | (label $b | try break $a catch error), 3]",
            ],
            None,
            r"[]
",
            r"",
            0,
        ),
        (
            &[r"-nc", r"[label $f | [1] as [$a] ?// $a | $a, break $f]"],
            None,
            r"[1,[1]]
",
            r"",
            0,
        ),
        (
            &[r"-nc", r#"[label $f | try break $f catch "c"]"#],
            None,
            r#"["c"]
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r"[label $f | try break $f catch empty, 5]"],
            None,
            r"[5]
",
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r#"[label $f | try (1, break $f) catch ("c", error)]"#,
            ],
            None,
            r#"[1,"c"]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"[label $f | try (1, break $f) catch (.__jq = 7 | error)]",
            ],
            None,
            r"",
            r#"jq: error (at <unknown>) (not a string): {"__jq":7}
"#,
            5,
        ),
        (
            &[r"-nc", r#"[label $f | getpath(["__jq"])]"#],
            None,
            r"[null]
",
            r"",
            0,
        ),
        (
            &[r"-nc", r#"[label $f | {"__jq":0} | error]"#],
            None,
            r"[]
",
            r"",
            0,
        ),
        (
            &[r"-nc", r"[label $f | {} | .__jq = 0 | error]"],
            None,
            r"[]
",
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r#"[label $f | {} | setpath(["__jq"]; 0) | error, 1]"#,
            ],
            None,
            r"[]
",
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r#"[label $f | "{\"__jq\":0}" | fromjson | error, 1]"#,
            ],
            None,
            r"[]
",
            r"",
            0,
        ),
        (
            &[r"-nc", r#"[label $f | try error({"__jq":0}) catch ., 1]"#],
            None,
            r#"[{"__jq":0},1]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"[label $f | try (label $g | 1, break $g, 2) catch ., 3]",
            ],
            None,
            r"[1,3]
",
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"[label $f | reduce (1,2) as $x (0; . + $x) | try break $f catch .]",
            ],
            None,
            r#"[{"__jq":0}]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"[label $f | foreach (1,2) as $x (0; . + $x; try break $f catch .)]",
            ],
            None,
            r#"[{"__jq":0},{"__jq":0}]
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r"[label $f | try (.a.b.c = 1 | break $f) catch .]"],
            None,
            r#"[{"__jq":0}]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"[label $f | try ([1,2] | .[] |= (., break $f)) catch .]",
            ],
            None,
            r"[[1,2]]
",
            r"",
            0,
        ),
        (
            &[r"-nc", r"[label $f | try break $f catch (error | 1)]"],
            None,
            r"[]
",
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"[label $f | try break $f catch (try error catch .)]",
            ],
            None,
            r#"[{"__jq":0}]
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r"label $f | try break $f catch halt_error"],
            None,
            r"",
            r#"{"__jq":0}
"#,
            5,
        ),
    ])
}

/// A hand-made `error({"__jq":N})` is the break of the label numbered `N`: `_equal` compares, so `0.0` matches and an extra key does not; `first`, `limit` and `isempty` are labels too.
#[test]
fn test_label_values_handmade_7719() -> Result<()> {
    check(&[
        (
            &[r"-nc", r#"[label $f | 1, error({"__jq":0}), 2]"#],
            None,
            r"[1]
",
            r"",
            0,
        ),
        (
            &[r"-nc", r#"[limit(3; 1, error({"__jq":0}), 2)]"#],
            None,
            r"[1]
",
            r"",
            0,
        ),
        (
            &[r"-nc", r#"[label $f | 1, error({"__jq":0.0}), 2]"#],
            None,
            r"[1]
",
            r"",
            0,
        ),
        (
            &[r"-nc", r#"[label $f | 1, error({"__jq":1}), 2]"#],
            None,
            r"",
            r#"jq: error (at <unknown>) (not a string): {"__jq":1}
"#,
            5,
        ),
        (
            &[r"-nc", r#"[label $f | 1, error({"__jq":0, "x":1}), 2]"#],
            None,
            r"",
            r#"jq: error (at <unknown>) (not a string): {"__jq":0,"x":1}
"#,
            5,
        ),
        (
            &[r"-nc", r#"[label $f | 1, error({"__jq":"0"}), 2]"#],
            None,
            r"",
            r#"jq: error (at <unknown>) (not a string): {"__jq":"0"}
"#,
            5,
        ),
        (
            &[r"-nc", r#"[1, error({"__jq":0}), 2]"#],
            None,
            r"",
            r#"jq: error (at <unknown>) (not a string): {"__jq":0}
"#,
            5,
        ),
        (
            &[r"-nc", r#"error({"__jq":0})"#],
            None,
            r"",
            r#"jq: error (at <unknown>) (not a string): {"__jq":0}
"#,
            5,
        ),
        (
            &[r"-nc", r#"[label $f | try error({"__jq":0}), 3]"#],
            None,
            r"[3]
",
            r"",
            0,
        ),
        (
            &[r"-nc", r#"[first(1, error({"__jq":0}), 2)]"#],
            None,
            r"[1]
",
            r"",
            0,
        ),
        (
            &[r"-nc", r#"[isempty(error({"__jq":0}))]"#],
            None,
            r"[]
",
            r"",
            0,
        ),
        (
            &[r"-nc", r#"[label $f | first(error({"__jq":0})), 7]"#],
            None,
            r"[]
",
            r"",
            0,
        ),
        (
            &[r"-nc", r#"[label $f | first(error({"__jq":1})), 7]"#],
            None,
            r"[7]
",
            r"",
            0,
        ),
        (
            &[r"-nc", r#"[label $f | error({"__jq":0}) ?, 3]"#],
            None,
            r"[3]
",
            r"",
            0,
        ),
        (
            &[r"-nc", r#"[label $f | label $g | error({"__jq":0}), 9]"#],
            None,
            r"[]
",
            r"",
            0,
        ),
        (
            &[r"-nc", r#"[label $f | label $g | error({"__jq":1}), 9]"#],
            None,
            r"[]
",
            r"",
            0,
        ),
        (
            &[r"-nc", r#"[label $f | (label $g | error({"__jq":0})), 9]"#],
            None,
            r"[]
",
            r"",
            0,
        ),
        (
            &[r"-nc", r#"[label $f | (label $g | error({"__jq":1})), 9]"#],
            None,
            r"[9]
",
            r"",
            0,
        ),
        (
            &[r"-nc", r#"[label $f | 1, error({"__jq":-0}), 2]"#],
            None,
            r"[1]
",
            r"",
            0,
        ),
    ])
}

/// The counter: every label entered advances it, including the ones `builtin.jq` enters (`first`, `limit` with a positive count, `isempty`, `any`/`all`, `nth`, `IN`, and `_modify` once per path), and C-coded builtins do not.
#[test]
fn test_label_values_counter_7719() -> Result<()> {
    check(&[
        (
            &[r"-nc", r"[first(1) , (label $f | try break $f catch .)]"],
            None,
            r#"[1,{"__jq":1}]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"[limit(2; 1,2,3), (label $f | try break $f catch .)]",
            ],
            None,
            r#"[1,2,{"__jq":1}]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"[limit(0; 1,2,3), (label $f | try break $f catch .)]",
            ],
            None,
            r#"[{"__jq":0}]
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r"[limit(-1; 1), (label $f | try break $f catch .)]"],
            None,
            r#"[1,{"__jq":0}]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"[nth(1; 1,2,3), (label $f | try break $f catch .)]",
            ],
            None,
            r#"[2,{"__jq":1}]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"[isempty(empty), (label $f | try break $f catch .)]",
            ],
            None,
            r#"[true,{"__jq":1}]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"[all(.[]?; true), (label $f | try break $f catch .)]",
            ],
            None,
            r#"[true,{"__jq":1}]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"[{a:1,b:2} | .[] |= 1 | (label $f | try break $f catch .)]",
            ],
            None,
            r#"[{"__jq":2}]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"[[1,2] | map_values(.+1) | (label $f | try break $f catch .)]",
            ],
            None,
            r#"[{"__jq":2}]
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r"[{} | .a = 1 | (label $f | try break $f catch .)]"],
            None,
            r#"[{"__jq":0}]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"[{} | .a += 1 | (label $f | try break $f catch .)]",
            ],
            None,
            r#"[{"__jq":1}]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"[{} | .a //= 1 | (label $f | try break $f catch .)]",
            ],
            None,
            r#"[{"__jq":1}]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"[until(true; .) | (label $f | try break $f catch .)]",
            ],
            None,
            r#"[{"__jq":0}]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"[range(3) as $i | label $f | try break $f catch .]",
            ],
            None,
            r#"[{"__jq":0},{"__jq":1},{"__jq":2}]
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r"[IN(1; 1, 2), (label $f | try break $f catch .)]"],
            None,
            r#"[true,{"__jq":1}]
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r"[walk(.) , (label $f | try break $f catch .)]"],
            None,
            r#"[null,{"__jq":0}]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"[[1,[2]] | walk(.) , (label $f | try break $f catch .)]",
            ],
            None,
            r#"[[1,[2]],{"__jq":0}]
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r"[label $f | .[]? as $x | try break $f catch .]"],
            None,
            r"[]
",
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"def f: label $f | try break $f catch .; [f, f, (f|f)]",
            ],
            None,
            r#"[{"__jq":0},{"__jq":1},{"__jq":3}]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"def r(n): if n > 0 then label $f | (try break $f catch .), r(n-1) else empty end; [r(3)]",
            ],
            None,
            r#"[{"__jq":0},{"__jq":1},{"__jq":2}]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"[label $f | first(range(10)) as $x | try break $f catch .]",
            ],
            None,
            r#"[{"__jq":0}]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"[label $f | any(1,2; . == 1) as $x | try break $f catch .]",
            ],
            None,
            r#"[{"__jq":0}]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"[label $f | limit(1; 1,2) as $x | try break $f catch .]",
            ],
            None,
            r#"[{"__jq":0}]
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r"[.[]?, (label $f | try break $f catch .)]"],
            None,
            r#"[{"__jq":0}]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r#"[label $f | ("a" | test("a")) as $x | try break $f catch .]"#,
            ],
            None,
            r#"[{"__jq":0}]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"[label $f | input_line_number as $x | try break $f catch .]",
            ],
            None,
            r#"[{"__jq":0}]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r#"[label $f | ltrimstr("a") as $x | try break $f catch .]"#,
            ],
            None,
            r#"[{"__jq":0}]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"[label $f | tostream as $x | try break $f catch .]",
            ],
            None,
            r#"[{"__jq":0}]
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r"[label $f | [paths] as $x | try break $f catch .]"],
            None,
            r#"[{"__jq":0}]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"[label $f | to_entries as $x | try break $f catch .]",
            ],
            None,
            r"",
            r"jq: error (at <unknown>): null (null) has no keys
",
            5,
        ),
        (
            &[
                r"-nc",
                r"[label $f | with_entries(.) as $x | try break $f catch .]",
            ],
            None,
            r"",
            r"jq: error (at <unknown>): null (null) has no keys
",
            5,
        ),
        (
            &[r"-nc", r"[label $f | del(.a) as $x | try break $f catch .]"],
            None,
            r#"[{"__jq":0}]
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r"[label $f | ascii as $x | try break $f catch .]"],
            None,
            r"",
            r"jq: error: ascii/0 is not defined at <top-level>, line 1:
[label $f | ascii as $x | try break $f catch .]            
jq: 1 compile error
",
            3,
        ),
        (
            &[
                r"-nc",
                r"[label $f | [combinations] as $x | try break $f catch .]",
            ],
            None,
            r#"[{"__jq":0}]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"[label $f | ([] | add) as $x | try break $f catch .]",
            ],
            None,
            r#"[{"__jq":0}]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r#"[label $f | ("a" | sub("a";"b")) as $x | try break $f catch .]"#,
            ],
            None,
            r#"[{"__jq":0}]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r#"[label $f | ("aa" | gsub("a";"b")) as $x | try break $f catch .]"#,
            ],
            None,
            r#"[{"__jq":0}]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r#"[label $f | ("a,b" | [scan(",")]) as $x | try break $f catch .]"#,
            ],
            None,
            r#"[{"__jq":0}]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"[label $f | ([3,1] | sort_by(.)) as $x | try break $f catch .]",
            ],
            None,
            r#"[{"__jq":0}]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"[label $f | ([3,1] | group_by(.)) as $x | try break $f catch .]",
            ],
            None,
            r#"[{"__jq":0}]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"[label $f | ([3,1] | unique_by(.)) as $x | try break $f catch .]",
            ],
            None,
            r#"[{"__jq":0}]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"[label $f | ([3,1] | min_by(.)) as $x | try break $f catch .]",
            ],
            None,
            r#"[{"__jq":0}]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"[label $f | ({} | pick(.a)) as $x | try break $f catch .]",
            ],
            None,
            r#"[{"__jq":0}]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"[label $f | ([1] | getpath([0])) as $x | try break $f catch .]",
            ],
            None,
            r#"[{"__jq":0}]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"[label $f | ([[1]] | flatten) as $x | try break $f catch .]",
            ],
            None,
            r#"[{"__jq":0}]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r#"[label $f | ({"a":1} | to_entries | from_entries) as $x | try break $f catch .]"#,
            ],
            None,
            r#"[{"__jq":0}]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"[label $f | ([1,2] | index(1)) as $x | try break $f catch .]",
            ],
            None,
            r#"[{"__jq":0}]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r#"[label $f | ("abc" | indices("b")) as $x | try break $f catch .]"#,
            ],
            None,
            r#"[{"__jq":0}]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"[label $f | ([1,2] | inside([1,2,3])) as $x | try break $f catch .]",
            ],
            None,
            r#"[{"__jq":0}]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"[label $f | ([1,2] | contains([1])) as $x | try break $f catch .]",
            ],
            None,
            r#"[{"__jq":0}]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"[label $f | ([1,2] | any) as $x | try break $f catch .]",
            ],
            None,
            r#"[{"__jq":0}]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"[label $f | ([1,2] | all) as $x | try break $f catch .]",
            ],
            None,
            r#"[{"__jq":0}]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"[label $f | ([1,2] | any(. > 1)) as $x | try break $f catch .]",
            ],
            None,
            r#"[{"__jq":0}]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r#"[label $f | ({"a":[1]} | [paths(type == "number")]) as $x | try break $f catch .]"#,
            ],
            None,
            r#"[{"__jq":0}]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"[label $f | (null | env | length > 0) as $x | try break $f catch .]",
            ],
            None,
            r#"[{"__jq":0}]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"[label $f | ([1,2] | tojson | fromjson) as $x | try break $f catch .]",
            ],
            None,
            r#"[{"__jq":0}]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r#"[label $f | ("2015-03-05T23:51:47Z" | fromdate) as $x | try break $f catch .]"#,
            ],
            None,
            r#"[{"__jq":0}]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"[label $f | (1425599507 | todate) as $x | try break $f catch .]",
            ],
            None,
            r#"[{"__jq":0}]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"[label $f | ([limit(3; repeat(1))]) as $x | try break $f catch .]",
            ],
            None,
            r#"[{"__jq":0}]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"[label $f | ([range(0;10;3)]) as $x | try break $f catch .]",
            ],
            None,
            r#"[{"__jq":0}]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r#"[label $f | ([1,[2]] | getpath(["a"])?) as $x | try break $f catch .]"#,
            ],
            None,
            r"[]
",
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r#"[label $f | ({"a":{"b":1}} | [..]) as $x | try break $f catch .]"#,
            ],
            None,
            r#"[{"__jq":0}]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"[label $f | ([1,2] | first) as $x | try break $f catch .]",
            ],
            None,
            r#"[{"__jq":0}]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"[label $f | ([1,2] | last) as $x | try break $f catch .]",
            ],
            None,
            r#"[{"__jq":0}]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"[label $f | (last(1,2)) as $x | try break $f catch .]",
            ],
            None,
            r#"[{"__jq":0}]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"[label $f | ([1,2] | nth(0)) as $x | try break $f catch .]",
            ],
            None,
            r#"[{"__jq":0}]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r#"[label $f | ({"a":1} | has("a")) as $x | try break $f catch .]"#,
            ],
            None,
            r#"[{"__jq":0}]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r#"[label $f | ("a" | IN("a","b")) as $x | try break $f catch .]"#,
            ],
            None,
            r#"[{"__jq":0}]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"[label $f | ([1,2] | IN([1,2],[3])) as $x | try break $f catch .]",
            ],
            None,
            r#"[{"__jq":0}]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"[label $f | ([[1,2]] | INDEX(.[0])) as $x | try break $f catch .]",
            ],
            None,
            r#"[{"__jq":0}]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"[label $f | (input_filename) as $x | try break $f catch .]",
            ],
            None,
            r#"[{"__jq":0}]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"[label $f | ([1,2] | add(.[])) as $x | try break $f catch .]",
            ],
            None,
            r"",
            r"jq: error: add/1 is not defined at <top-level>, line 1:
[label $f | ([1,2] | add(.[])) as $x | try break $f catch .]                     
jq: 1 compile error
",
            3,
        ),
        (
            &[
                r"-nc",
                r#"[label $f | ("a" | ascii_downcase) as $x | try break $f catch .]"#,
            ],
            None,
            r#"[{"__jq":0}]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r#"[label $f | ("a b" | splits(" ")) as $x | try break $f catch .]"#,
            ],
            None,
            r#"[{"__jq":0},{"__jq":0}]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"[label $f | ([1] | tostream) as $x | try break $f catch .]",
            ],
            None,
            r#"[{"__jq":0},{"__jq":0}]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"[label $f | (fromstream(([1]|tostream))) as $x | try break $f catch .]",
            ],
            None,
            r#"[{"__jq":0}]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"[label $f | ([1,2] | getpath([5])) as $x | try break $f catch .]",
            ],
            None,
            r#"[{"__jq":0}]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r#"[label $f | (getpath(["a","b"])) as $x | try break $f catch .]"#,
            ],
            None,
            r#"[{"__jq":0}]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r#"[label $f | ([{"a":1}] | map(.a)) as $x | try break $f catch .]"#,
            ],
            None,
            r#"[{"__jq":0}]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"[label $f | ([3,1] | sort) as $x | try break $f catch .]",
            ],
            None,
            r#"[{"__jq":0}]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r#"[label $f | ({"a":1} | map_values(.)) as $x | try break $f catch .]"#,
            ],
            None,
            r#"[{"__jq":0}]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r#"[label $f | ({"a":1} | with_entries(.value += 1)) as $x | try break $f catch .]"#,
            ],
            None,
            r#"[{"__jq":0}]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r#"[label $f | ({"a":1} | delpaths([["a"]])) as $x | try break $f catch .]"#,
            ],
            None,
            r#"[{"__jq":0}]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r#"[label $f | ({"a":1} | to_entries) as $x | try break $f catch .]"#,
            ],
            None,
            r#"[{"__jq":0}]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r#"[label $f | ("abc" | test("b")) as $x | try break $f catch .]"#,
            ],
            None,
            r#"[{"__jq":0}]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r#"[label $f | ("abc" | [match("b")]) as $x | try break $f catch .]"#,
            ],
            None,
            r#"[{"__jq":0}]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r#"[label $f | ("abc" | capture("(?<x>b)")) as $x | try break $f catch .]"#,
            ],
            None,
            r#"[{"__jq":0}]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r#"[label $f | ("abc" | ascii_upcase) as $x | try break $f catch .]"#,
            ],
            None,
            r#"[{"__jq":0}]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r#"[label $f | ("a-b" | split("-")) as $x | try break $f catch .]"#,
            ],
            None,
            r#"[{"__jq":0}]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r#"[label $f | ("a-b" | ltrimstr("a")) as $x | try break $f catch .]"#,
            ],
            None,
            r#"[{"__jq":0}]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r#"[label $f | ([1,[2]] | walk(if type == "number" then .+1 else . end)) as $x | try break $f catch .]"#,
            ],
            None,
            r#"[{"__jq":0}]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r#"[label $f | ({"a":{"b":1}} | walk(.)) as $x | try break $f catch .]"#,
            ],
            None,
            r#"[{"__jq":0}]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r#"[label $f | (@base64 "x\(1)") as $x | try break $f catch .]"#,
            ],
            None,
            r#"[{"__jq":0}]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"[label $f | ([1,2] | @csv) as $x | try break $f catch .]",
            ],
            None,
            r#"[{"__jq":0}]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r#"[label $f | (getpath(["a"]) |= 1) as $x | try break $f catch .]"#,
            ],
            None,
            r#"[{"__jq":0}]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"[label $f | ([1,2] | del(.[0])) as $x | try break $f catch .]",
            ],
            None,
            r#"[{"__jq":0}]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"[label $f | ([1,2] | to_entries) as $x | try break $f catch .]",
            ],
            None,
            r#"[{"__jq":0}]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r#"[label $f | (splits("x")?) as $x | try break $f catch .]"#,
            ],
            None,
            r"[]
",
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"[label $f | (ltrimstr(1)) as $x | try break $f catch .]",
            ],
            None,
            r#"[{"__jq":0}]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"[label $f | ({} | .a.b |= 1) as $x | try break $f catch .]",
            ],
            None,
            r#"[{"__jq":0}]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"[label $f | ({} | .a += 1 | .b -= 1) as $x | try break $f catch .]",
            ],
            None,
            r"",
            r"jq: error (at <unknown>): null (null) and number (1) cannot be subtracted
",
            5,
        ),
        (
            &[
                r"-nc",
                r"[label $f | ([1,2,3] | .[1:] |= map(.+1)) as $x | try break $f catch .]",
            ],
            None,
            r#"[{"__jq":0}]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"[label $f | ([1,2,3] | .[] += 1) as $x | try break $f catch .]",
            ],
            None,
            r#"[{"__jq":0}]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"[label $f | ([1,2,3] | (.[] | select(. > 1)) |= 0) as $x | try break $f catch .]",
            ],
            None,
            r#"[{"__jq":0}]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r#"[label $f | ({"a":[1,2]} | .a[] |= empty) as $x | try break $f catch .]"#,
            ],
            None,
            r#"[{"__jq":0}]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"[label $f | ([[1,2]] | .[0][] *= 2) as $x | try break $f catch .]",
            ],
            None,
            r#"[{"__jq":0}]
"#,
            r"",
            0,
        ),
    ])
}

/// A `break` stops at the label instance it is bound to (#7724).
#[test]
fn test_label_values_identity_7719() -> Result<()> {
    check(&[
        (
            &[r"-nc", r"[label $f | (def g: break $f; label $f | g), 3]"],
            None,
            r"[]
",
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"def f(g): label $out | g; [label $out | f(break $out), 2]",
            ],
            None,
            r"[]
",
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"def f(g; n): label $out | if n > 0 then f(break $out; n-1), 5 else g end; [f(1; 2), 9]",
            ],
            None,
            r"[5,9]
",
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"def f(g; n): label $out | if n > 0 then f(break $out; n-1) else g end; [f(1; 2), 9]",
            ],
            None,
            r"[9]
",
            r"",
            0,
        ),
        (
            &[r"-nc", r"[label $f | label $f | try break $f catch .]"],
            None,
            r#"[{"__jq":1}]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"[label $f | (label $f | 1, 2) | try break $f catch .]",
            ],
            None,
            r#"[{"__jq":0},{"__jq":0}]
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r"[label $f | (label $f | 1, 2) | ., break $f]"],
            None,
            r"[1]
",
            r"",
            0,
        ),
    ])
}

/// The counter keeps counting across inputs, on every input route.
#[test]
fn test_label_values_inputs_7719() -> Result<()> {
    check(&[
        (
            &[r"-c", r"[label $f | try break $f catch .]"],
            Some(
                r"1 2 3
",
            ),
            r#"[{"__jq":0}]
[{"__jq":1}]
[{"__jq":2}]
"#,
            r"",
            0,
        ),
        (
            &[r"-c", r"first(.) as $x | [label $f | try break $f catch .]"],
            Some(
                r"1 2 3
",
            ),
            r#"[{"__jq":1}]
[{"__jq":3}]
[{"__jq":5}]
"#,
            r"",
            0,
        ),
        (
            &[r"-c", r#"[label $f | 1, error({"__jq":1}), 2]"#],
            Some(
                r"1 2 3
",
            ),
            r"[1]
",
            r#"jq: error (at <stdin>:1) (not a string): {"__jq":1}
jq: error (at <stdin>:1) (not a string): {"__jq":1}
"#,
            5,
        ),
        (
            &[r"-nc", r"[inputs | label $f | try break $f catch .]"],
            Some(
                r"1 2
",
            ),
            r#"[{"__jq":0},{"__jq":1}]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"[limit(5; inputs), (label $f | try break $f catch .)]",
            ],
            Some(
                r"1 2
",
            ),
            r#"[1,2,{"__jq":1}]
"#,
            r"",
            0,
        ),
        (
            &[r"-sc", r"[label $f | try break $f catch .]"],
            Some(
                r"1 2
",
            ),
            r#"[{"__jq":0}]
"#,
            r"",
            0,
        ),
        (
            &[r"-c", r".[] | [label $f | try break $f catch .]"],
            Some(
                r"[1,2]
[3]
",
            ),
            r#"[{"__jq":0}]
[{"__jq":1}]
[{"__jq":2}]
"#,
            r"",
            0,
        ),
        (
            &[r"-c", r"[label $f | try break $f catch .]", r"--stream"],
            Some(
                r"[1,2]
",
            ),
            r#"[{"__jq":0}]
[{"__jq":1}]
[{"__jq":2}]
"#,
            r"",
            0,
        ),
        (
            &[r"-c", r"[label $f | try break $f catch .]", r"--seq"],
            Some(
                r"1 2
",
            ),
            r"",
            r"jq: ignoring parse error: Unfinished abandoned text at EOF at line 2, column 0
",
            0,
        ),
        (
            &[r"-Rc", r"[label $f | try break $f catch .]"],
            Some(
                r"a
b
",
            ),
            r#"[{"__jq":0}]
[{"__jq":1}]
"#,
            r"",
            0,
        ),
        (
            &[r"-c", r".a as $x | [label $f | try break $f catch .]"],
            Some(
                r#"{"a":1}
{"a":2}
"#,
            ),
            r#"[{"__jq":0}]
[{"__jq":1}]
"#,
            r"",
            0,
        ),
    ])
}

/// Recorded in docs/compliance/jq/limitations.md: a sentinel the program does not spell (built
/// from data or a computed key) keeps the shipped route, which raises it where jq's label stops on
/// it, and a program that also uses a succinctly extension the interpreter declines still hands a
/// `catch` `null` for a `break`.
#[test]
fn test_label_values_recorded_divergences_7719() -> Result<()> {
    check(&[
        (
            &["-c", "[label $f | 1, error(.), 2]"],
            Some("{\"__jq\":0}\n"),
            "",
            "jq: error (at <stdin>:1) (not a string): {\"__jq\":0}\n",
            5,
        ),
        (
            &["-nc", r#"[label $f | {("__" + "jq"): 0} | error, 1]"#],
            None,
            "",
            "jq: error (at <unknown>) (not a string): {\"__jq\":0}\n",
            5,
        ),
        (
            &["-c", ".a | [label $f | try (key, break $f) catch type]"],
            Some("{\"a\":1}\n"),
            "[\"a\",\"null\"]\n",
            "",
            0,
        ),
    ])
}
