//! #5424: a body rebuilt for each value (`. as $x | (...)`, a destructuring bind, the update of a
//! `reduce`, the extract of a `foreach`) hands on the tails of the pipe it was rebuilt from when
//! the pipe reads no variable, instead of keeping tails of its own per value. The copies are pinned
//! by the unit test `test_body_rebuilt_per_value_shares_the_tails_of_its_pipe_5424`; these rows pin
//! that the answers are the ones jq gives, for a pipe that reads the variable first, in the middle,
//! last or not at all. Expected outputs were captured from jq 1.7.1 (`/usr/bin/jq`). Ten computing
//! stages, so each tail the pipe hands on is long enough to be kept.

use anyhow::Result;

use crate::run_jq_full;

/// Ten computing stages.
const T: &str = ". + 1 | . + 1 | . + 1 | . + 1 | . + 1 | . + 1 | . + 1 | . + 1 | . + 1 | . + 1";

/// The definitions the rows call.
const DEFS: &str = "def f: . + 1; def g: (., . + 100); def h: if . > 40 then . else (. as $x | ({T} | h)) end; def m(x): . as $v | (x | {T}); ";

#[test]
fn test_body_rebuilt_per_value_matches_jq_5424() -> Result<()> {
    let defs = DEFS.replace("{T}", T);
    let defs = defs.as_str();
    for (name, filter, want, code) in [
        ("closed as", format!("{defs}[range(0;6) | . as $x | ({T})]"), "[10,11,12,13,14,15]\n", 0),
        ("mention last", format!("{defs}[range(0;6) | . as $x | ({T} | . + $x)]"), "[10,12,14,16,18,20]\n", 0),
        ("mention first", format!("{defs}[range(0;6) | . as $x | ($x | {T})]"), "[10,11,12,13,14,15]\n", 0),
        ("mention middle", format!("{defs}[range(0;6) | . as $x | ({T} | . + $x | {T})]"), "[20,22,24,26,28,30]\n", 0),
        ("array pattern", format!("{defs}[range(0;6) | [., 1] as [$a, $b] | ({T})]"), "[10,11,12,13,14,15]\n", 0),
        ("object pattern", format!("{defs}[range(0;6) | {{a: ., b: 2}} as {{a: $a, b: $b}} | ({T} | . + $b)]"), "[12,13,14,15,16,17]\n", 0),
        ("alternative pattern", format!("{defs}[[1], 2, [3]] | [.[] as [$a] ?// $a | ($a | {T})]"), "[11,12,13]\n", 0),
        ("shadowed", format!("{defs}[range(0;6) | . as $x | (. + 1 | . as $x | ({T} | . + $x))]"), "[12,14,16,18,20,22]\n", 0),
        ("nested", format!("{defs}[range(0;6) | . as $x | (. + 1 | . as $y | ({T} | . + 1))]"), "[12,13,14,15,16,17]\n", 0),
        ("a reduce body", format!("{defs}reduce range(0;6) as $i (0; {T})"), "60\n", 0),
        ("a reduce body reading its variable", format!("{defs}reduce range(0;6) as $i (0; {T} | . + $i)"), "75\n", 0),
        ("a foreach", format!("{defs}[foreach range(0;6) as $i (0; . + 1; {T})]"), "[11,12,13,14,15,16]\n", 0),
        ("a foreach reading its variable", format!("{defs}[foreach range(0;6) as $i (0; . + 1; {T} | . + $i)]"), "[11,13,15,17,19,21]\n", 0),
        ("a def call", format!("{defs}[range(0;6) | . as $x | ({T} | f)]"), "[11,12,13,14,15,16]\n", 0),
        ("a generating def", format!("{defs}[range(0;3) | . as $x | ({T} | g | . + 1)]"), "[11,111,12,112,13,113]\n", 0),
        ("a recursive def", format!("{defs}[range(0;3) | h]"), "[50,41,42]\n", 0),
        ("a closure argument", format!("{defs}[range(0;4) | m(. + 1)]"), "[11,12,13,14]\n", 0),
        ("select", format!("{defs}[range(0;8) | . as $x | ({T}) | select(. % 2 == 0)]"), "[10,12,14,16]\n", 0),
        ("last", format!("{defs}last(range(0;8) | . as $x | ({T}))"), "17\n", 0),
        ("limit", format!("{defs}[limit(3; range(0;8) | . as $x | ({T}))]"), "[10,11,12]\n", 0),
        ("first", format!("{defs}first(range(0;8) | . as $x | ({T}) | select(. > 14))"), "15\n", 0),
        ("an error caught per value", format!("{defs}[range(0;6) | try (. as $x | ({T} | if . == 14 then error(\"boom\") else . end)) catch \"c\"]"), "[10,11,12,13,\"c\",15]\n", 0),
        ("a break out of the body", format!("{defs}[range(0;6) | label $out | (. as $x | ({T} | if . > 13 then break $out else . end))]"), "[10,11,12,13]\n", 0),
        ("an update through the body", format!("{defs}[1,2,3] | .[] |= (. as $x | ({T}))"), "[11,12,13]\n", 0),
        ("an assignment through the body", format!("{defs}{{\"a\": [1,2]}} | .a[] |= (. as $x | ({T} | . + $x))"), "{\"a\":[12,14]}\n", 0),
    ] {
        let (stdout, stderr, status) = run_jq_full(&["-n", "-c", &filter], None)?;
        assert_eq!(
            (stdout.as_str(), status),
            (want, code),
            "{name}: `{filter}` -- {stderr:?}"
        );
    }
    Ok(())
}

/// A path expression that does more than navigate (`path(.[] | . as $x | .[0])`) is refused by the
/// `unshared-containers` holdout build (#5246).
#[test]
#[cfg_attr(feature = "unshared-containers", ignore = "holdout refusal (#5246)")]
fn test_path_through_a_body_rebuilt_per_value_matches_jq_5424() -> Result<()> {
    let (stdout, stderr, status) = run_jq_full(
        &["-n", "-c", "[[1,2],[3,4]] | [path(.[] | . as $x | .[0])]"],
        None,
    )?;
    assert_eq!(
        (stdout.as_str(), status),
        ("[[0,0],[1,0]]\n", 0),
        "{stderr:?}"
    );
    Ok(())
}
