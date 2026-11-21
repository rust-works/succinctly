//! #7519: a `foreach` whose INIT is a generator runs its SOURCE on the fold's real input for every
//! INIT output, as jq 1.8 does. jq 1.7.1 handed every INIT output after the first a `null` input
//! (its `DUPN`), so `[1, 2] | [foreach .[] as $x (0, 1; . + $x)]` raised `Cannot iterate over null`;
//! jq 1.8.0 fixed it as a bug (jqlang/jq#3266, NEWS: "Fix `foreach` not to break init backtracking
//! with `DUPN`") and prints `[1,3,2,4]`. `reduce` keeps the 1.7.1 behaviour in jq 1.8.2 (#3266 "does
//! not address `reduce`"), so its rows are controls, as are a single INIT output and a SOURCE that
//! does not read the input.
//!
//! Every expected stdout and exit status was captured from `/opt/homebrew/bin/jq` 1.8.2 on the given
//! input, and every row runs on both routes: stdin through the generic evaluator, `-n 'input | ...'`
//! through `eval.rs`'s owned one. The `path(...)` and `($x.a) = 9` rows reach the path-mode
//! interpreter. An error row checks jq 1.7.1's spelling of the message where 1.8 reworded it (the
//! value previews, kept on 1.7.1 by the latest-jq rule), so only a stable prefix is pinned there.

use anyhow::Result;

use crate::run_jq_full;

/// (input, program, stdout without its last newline, exit status, message) where jq 1.7.1 answers
/// otherwise: every row here moved with jqlang/jq#3266.
const MOVED: &[(&str, &str, &str, i32, &str)] = &[
    (
        r"[1,2]",
        r"[foreach .[] as $x (0, 1; . + $x)]",
        r"[1,3,2,4]",
        0,
        "",
    ),
    (
        r#"{"a":1,"c":2}"#,
        r"[foreach (.a) as $k ((0,.c); $k)]",
        r"[1,1]",
        0,
        "",
    ),
    (
        r#"{"a":1}"#,
        r"[foreach (.) as $x ((.,.,.); $x)]",
        r#"[{"a":1},{"a":1},{"a":1}]"#,
        0,
        "",
    ),
    (
        r#"{"a":1,"c":2}"#,
        r"[foreach .a as $k ((0,.c); .+$k; [$k,.])]",
        r"[[1,1],[1,3]]",
        0,
        "",
    ),
    (
        r#"{"a":1,"b":2}"#,
        r"[foreach .[] as $x ((0,1); . + $x)]",
        r"[1,3,2,4]",
        0,
        "",
    ),
    (
        r"[1,2]",
        r"[foreach .[] as $x (.[]; . + $x)]",
        r"[2,4,3,5]",
        0,
        "",
    ),
    (
        r"[1,2]",
        r"[foreach .[] as $x (0, 1; . + $x; ., -.)]",
        r"[1,-1,3,-3,2,-2,4,-4]",
        0,
        "",
    ),
    (
        r"[1,2]",
        r"[foreach length as $x (0, 1; . + $x)]",
        r"[2,3]",
        0,
        "",
    ),
    (
        r"[1,2]",
        r"[foreach (.[]?, 5) as $x (0, 1; . + $x)]",
        r"[1,3,8,2,4,9]",
        0,
        "",
    ),
    (
        r"[1,2]",
        r#"[try foreach .[] as $x (0, 1, error("I"); . + $x) catch .]"#,
        r#"[1,3,2,4,"I"]"#,
        0,
        "",
    ),
    (
        r"[1,2]",
        r"[limit(3; foreach .[] as $x (0, 1; . + $x))]",
        r"[1,3,2]",
        0,
        "",
    ),
    (
        r"[1,2]",
        r"[label $out | foreach .[] as $x (0, 1; if . > 2 then ., break $out else . + $x end)]",
        r"[1,3,2,4]",
        0,
        "",
    ),
    (
        r"[1,2]",
        r"[foreach .[] as $x (0, 1; . + $x; $__loc__.line)]",
        r"[1,1,1,1]",
        0,
        "",
    ),
    (
        r"[[1,2],[3,4]]",
        r"[foreach .[] as [$a, $b] (0, 1; . + $a + $b)]",
        r"[3,10,4,11]",
        0,
        "",
    ),
    (
        r"[[1],2]",
        r"[foreach .[] as [$a] ?// $a (0, 10; . + $a)]",
        r"[1,3,11,13]",
        0,
        "",
    ),
    (
        r"[1,2]",
        r"[foreach (foreach .[] as $y (0, 1; $y)) as $x (0, 100; . + $x)]",
        r"[1,3,4,6,101,103,104,106]",
        0,
        "",
    ),
    (
        r"[1,2]",
        r"[foreach .[] as $x (0, 1; [foreach .[]? as $y (0; . + $y)])]",
        r"[[],[],[],[]]",
        0,
        "",
    ),
    (
        r"[1,2]",
        r"[reduce (1, 2) as $r (0; [foreach length as $x (0, 1; [., $x])])]",
        r"[[[0,2],[1,2]]]",
        0,
        "",
    ),
    (
        r"[1,2]",
        r"def f: foreach .[] as $x (0, 1; . + $x); [f]",
        r"[1,3,2,4]",
        0,
        "",
    ),
    (
        r#"{"a":1}"#,
        r"[path(foreach .a as $k ((0,1); $k))]",
        r#"[["a"],["a"]]"#,
        0,
        "",
    ),
    (
        r#"{"a":1,"c":2}"#,
        r"[path(foreach (.a,.c) as $k ((0,1); .; $k))]",
        r#"[["a"],["c"],["a"],["c"]]"#,
        0,
        "",
    ),
    (
        r#"{"c":null}"#,
        r"[path(foreach (.a) as $k ((0,.c); $k))]",
        "",
        5,
        r#"Invalid path expression near attempt to access element "a" of {"c":null}"#,
    ),
    (
        r#"{"a":1}"#,
        r"[foreach (.) as $x ((.,.); ($x.a) = 9)]",
        r#"[{"a":9},{"a":9}]"#,
        0,
        "",
    ),
    (
        r#"{"a":[1,2]}"#,
        r".a |= [foreach .[] as $x (0, 1; . + $x)]",
        r#"{"a":[1,3,2,4]}"#,
        0,
        "",
    ),
    // Same exit status as jq 1.7.1, but a different error: the later fork reads the document, and
    // the failure moves on to `..`'s next value, `[1,2]`.
    (
        r#"{"a":[1,2]}"#,
        r"[paths(foreach .a[] as $x (0, 1; . + $x) | . > 2)]",
        "",
        5,
        r"Cannot index array with string",
    ),
];

/// Rows jq 1.7.1 and jq 1.8.2 answer alike: `reduce`, a single INIT output, a SOURCE that ignores
/// the input, and consumers that stop before a later fork.
const UNCHANGED: &[(&str, &str, &str, i32, &str)] = &[
    (
        r"[1,2]",
        r"[foreach .[] as $x (empty; . + $x)]",
        r"[]",
        0,
        "",
    ),
    (
        r"[1,2]",
        r#"[try foreach (.[], error("S")) as $x (0, 1; . + $x) catch .]"#,
        r#"[1,3,"S"]"#,
        0,
        "",
    ),
    (
        r"[1,2]",
        r"[first(foreach .[] as $x (0, 1; . + $x))]",
        r"[1]",
        0,
        "",
    ),
    (
        r"[1,2]",
        r"[isempty(foreach .[] as $x (0, 1; . + $x))]",
        r"[false]",
        0,
        "",
    ),
    (
        r"[1,2]",
        r"[foreach .[] as $x (0, 1; [reduce (1, 2) as $y (.; . + $y)])]",
        "",
        5,
        r"array ([3]) and number (1) cannot be added",
    ),
    (
        r#"{"a":1}"#,
        r"[reduce (.) as $x ((.,.,.); $x)]",
        r#"[{"a":1},null,null]"#,
        0,
        "",
    ),
    (
        r"[1,2,3]",
        r"reduce .[] as $x (0,1; .+$x)",
        r"6",
        5,
        r"Cannot iterate over null (null)",
    ),
    (
        r#"{"a":1}"#,
        r"[reduce (.) as $x ((.,.); ($x.a) = 9)]",
        "",
        5,
        r#"Invalid path expression near attempt to access element "a" of null"#,
    ),
    (
        r#"{"a":1,"c":2}"#,
        r"[path(reduce (.a) as $k ((0,1); $k))]",
        "",
        5,
        r"Invalid path expression with result 1",
    ),
    (
        r"[1,2]",
        r"[foreach .[] as $x (0; . + $x)]",
        r"[1,3]",
        0,
        "",
    ),
    (
        r"[1,2]",
        r"[foreach range(3) as $x (0, 1; . + $x)]",
        r"[0,1,3,1,2,4]",
        0,
        "",
    ),
];

fn check(rows: &[(&str, &str, &str, i32, &str)]) -> Result<()> {
    for &(input, program, want, want_code, message) in rows {
        for (args, filter) in [
            (&["-c"][..], program.to_string()),
            (&["-n", "-c"][..], format!("input | {program}")),
        ] {
            let mut argv = args.to_vec();
            argv.push(&filter);
            let (stdout, stderr, code) = run_jq_full(&argv, Some(input))?;
            assert_eq!(
                (stdout.trim_end_matches('\n'), code),
                (want, want_code),
                "#7519: `{filter}` ({args:?}) on {input} must answer as jq 1.8.2 does; stderr={stderr:?}"
            );
            assert!(
                stderr.contains(message),
                "#7519: `{filter}` on {input}: wanted {message:?} in stderr, got {stderr:?}"
            );
        }
    }
    Ok(())
}

#[test]
fn foreach_generator_init_forks_read_the_real_input_7519() -> Result<()> {
    check(MOVED)
}

#[test]
fn reduce_and_single_init_folds_are_unchanged_7519() -> Result<()> {
    check(UNCHANGED)
}
