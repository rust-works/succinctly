//! #5421: a pipe tail holding a `def` call keeps the copies of its stateless runs and copies only the
//! call per value, instead of copying every stage after the call's predecessors for every value at
//! every stage. The copies are pinned by the unit test `test_pipe_tail_holding_a_def_call_copies_
//! only_the_call_per_value_5421`; these rows pin that the answers are the ones jq gives. Expected
//! outputs were captured from jq 1.7.1 (`/usr/bin/jq`). Ten computing stages, so each tail the pipe
//! hands on is long enough to be split.

use anyhow::Result;

use crate::run_jq_full;

/// Ten computing stages.
const T: &str = ". + 1 | . + 1 | . + 1 | . + 1 | . + 1 | . + 1 | . + 1 | . + 1 | . + 1 | . + 1";

/// The definitions the rows call.
const DEFS: &str =
    "def f: . + 1; def g: (., . + 100); def h: if . > 40 then . else (. + 1 | h) end; def m(x): x + 1; ";

#[test]
fn test_long_pipe_with_a_def_call_matches_jq_5421() -> Result<()> {
    for (name, filter, want, code) in [
        ("def at the end", format!("[range(0;6) | ({T} | f)]"), "[11,12,13,14,15,16]\n", 0),
        ("def in the middle", format!("[range(0;6) | ({T} | f | {T})]"), "[21,22,23,24,25,26]\n", 0),
        ("three defs", format!("[range(0;4) | (f | {T} | f | {T} | f)] | add"), "98\n", 0),
        ("bare pipe", format!("[range(0;6) | {T} | f]"), "[11,12,13,14,15,16]\n", 0),
        ("select", format!("[range(0;8) | ({T} | f) | select(. % 2 == 0)]"), "[12,14,16,18]\n", 0),
        ("last", format!("last(range(0;8) | ({T} | f))"), "18\n", 0),
        ("limit", format!("[limit(3; range(0;8) | ({T} | f))]"), "[11,12,13]\n", 0),
        ("first", format!("first(range(0;8) | ({T} | f) | select(. > 14))"), "15\n", 0),
        ("any", format!("any(range(0;8) | ({T} | f); . == 15)"), "true\n", 0),
        ("isempty", format!("isempty(range(0;8) | ({T} | f) | select(. > 99))"), "true\n", 0),
        ("a def that generates", format!("[range(0;3) | ({T} | g | . + 1)]"), "[11,111,12,112,13,113]\n", 0),
        ("a recursive def", format!("[range(0;4) | ({T} | h)]"), "[41,41,41,41]\n", 0),
        ("a def with a closure argument", format!("[range(0;4) | ({T} | m(. * 2))]"), "[21,23,25,27]\n", 0),
        ("fan-out before the def", format!("[range(0;3) | (. + 1 | (., . + 100) | {T} | f)]"), "[12,112,13,113,14,114]\n", 0),
        ("empty before the def", format!("[range(0;4) | (. + 1 | (empty, .) | {T} | f)]"), "[12,13,14,15]\n", 0),
        ("an effect before the def", format!("[range(0;4) | (. + input_line_number | {T} | f)]"), "[11,12,13,14]\n", 0),
        ("an error caught per value", format!("[range(0;6) | try ({T} | f | if . == 14 then error(\"boom\") else . end) catch \"c\"]"), "[11,12,13,\"c\",15,16]\n", 0),
        ("an error after outputs", format!("range(0;6) | ({T} | f | if . == 14 then error(\"boom\") else . end)"), "11\n12\n13\n", 5),
        ("a break out of the pipe", format!("[range(0;6) | label $out | ({T} | f | if . > 13 then break $out else . end)]"), "[11,12,13]\n", 0),
        ("a reduce body", format!("reduce range(0;4) as $i (0; . + ($i | ({T} | f)))"), "50\n", 0),
        ("a foreach", format!("[foreach range(0;4) as $i (0; . + $i; ({T} | f))]"), "[11,12,14,17]\n", 0),
        ("an as binding", format!("[range(0;4) | . as $x | ({T} | f | . + $x)]"), "[11,13,15,17]\n", 0),
    ] {
        let filter = format!("{DEFS}{filter}");
        let (stdout, stderr, status) = run_jq_full(&["-n", "-c", &filter], None)?;
        assert_eq!(
            (stdout.as_str(), status),
            (want, code),
            "{name}: `{filter}` -- {stderr:?}"
        );
    }
    Ok(())
}

/// The same pipes over a document: the values of the first stage are cursors into the input, the
/// computed ones owned.
#[test]
fn test_long_pipe_with_a_def_call_over_a_document_matches_jq_5421() -> Result<()> {
    for (filter, want) in [
        (format!("[.[] | ({T} | f)]"), "[12,13,14,15]\n"),
        (format!("[.[] | ({T} | f) | select(. > 13)]"), "[14,15]\n"),
        (format!("last(.[] | ({T} | f))"), "15\n"),
        (format!("[limit(2; .[] | ({T} | f))]"), "[12,13]\n"),
        (format!("[.[] | {T} | f | {T}]"), "[22,23,24,25]\n"),
    ] {
        let filter = format!("{DEFS}{filter}");
        let (stdout, stderr, status) = run_jq_full(&["-c", &filter], Some("[1,2,3,4]"))?;
        assert_eq!(
            (stdout.as_str(), status),
            (want, 0),
            "`{filter}` -- {stderr:?}"
        );
    }
    Ok(())
}
