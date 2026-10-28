//! #5436: a body rebuilt for each value that reads its variable keeps the tails that start after its
//! last reading stage, and copies the ones that include it. The copies are pinned by the unit tests
//! `test_body_that_reads_its_variable_shares_the_tails_after_it_5436` and
//! `test_pipe_stages_rebuilt_from_a_pipe_that_reads_a_variable_keep_the_tails_after_it_5436`; these rows
//! pin that the answers are the ones jq gives for a variable read first, last, in the middle, by
//! two stages and by a nested binder, and that a container keeps its storage identity down the kept
//! tail. Expected outputs were captured from jq 1.7.1 (`/usr/bin/jq`). Ten computing stages, so each
//! tail the pipe hands on is long enough to be kept.

use anyhow::Result;

use crate::run_jq_full;

/// Ten computing stages.
const T: &str = ". + 1 | . + 1 | . + 1 | . + 1 | . + 1 | . + 1 | . + 1 | . + 1 | . + 1 | . + 1";

#[test]
fn test_body_that_reads_its_variable_matches_jq_5436() -> Result<()> {
    let defs = format!("def h: if . > 60 then . else (. as $x | (. + $x | {T} | h)) end; ");
    for (name, filter, want) in [
        ("first", format!("[range(0;6) | . as $x | (. + $x | {T})]"), "[10,12,14,16,18,20]\n"),
        ("first and last", format!("[range(0;6) | . as $x | (. + $x | {T} | . + $x)]"), "[10,13,16,19,22,25]\n"),
        ("middle", format!("[range(0;6) | . as $x | ({T} | . + $x | {T})]"), "[20,22,24,26,28,30]\n"),
        ("two readers", format!("[range(0;6) | . as $x | (. + $x | {T} | . + $x | {T})]"), "[20,23,26,29,32,35]\n"),
        ("destructured", format!("[range(0;6) | [., 2] as [$a, $b] | (. + $a + $b | {T})]"), "[12,14,16,18,20,22]\n"),
        ("two binders", format!("[range(0;6) | . as $x | (. + $x | . as $y | ({T} | . + $y))]"), "[10,14,18,22,26,30]\n"),
        ("a shadowing binder", format!("[range(0;6) | . as $x | (. + $x | {T} | . as $x | . + $x)]"), "[20,24,28,32,36,40]\n"),
        ("nested binders, the inner body long", format!("[range(0;6) | . as $x | (. + $x | {T} | . as $y | ({T} | . + $y | {T}))]"), "[40,44,48,52,56,60]\n"),
        ("destructured, both read in the middle", format!("[range(0;6) | [., 1] as [$a, $b] | ({T} | . + $a | {T} | . + $b | {T})]"), "[31,33,35,37,39,41]\n"),
        ("destructured, the first read first", format!("[range(0;6) | [., 1] as [$a, $b] | (. + $a | {T} | . + $b | {T})]"), "[21,23,25,27,29,31]\n"),
        ("the outer read after the inner", format!("[range(0;6) | . as $x | ({T} | . as $y | (. + $y | {T} | . + $x))]"), "[30,33,36,39,42,45]\n"),
        ("an object pattern", format!("[range(0;6) | {{a: ., b: 2}} as {{a: $a, b: $b}} | (. + $a | {T} | . + $b)]"), "[12,14,16,18,20,22]\n"),
        ("a reduce", format!("reduce range(0;6) as $i (0; . + $i | {T})"), "75\n"),
        ("a foreach", format!("[foreach range(0;6) as $i (0; . + $i; . + $i | {T})]"), "[10,12,15,19,24,30]\n"),
        ("a recursive def", format!("{defs}[range(0;3) | h]"), "[70,78,86]\n"),
        ("an error caught per value", format!("[range(0;6) | try (. as $x | (. + $x | {T} | if . == 14 then error(\"boom\") else . end)) catch \"c\"]"), "[10,12,\"c\",16,18,20]\n"),
        ("a break out of the body", format!("[range(0;6) | label $out | (. as $x | (. + $x | {T} | if . > 24 then break $out else . end))]"), "[10,12,14,16,18,20]\n"),
        ("first", format!("first(range(0;8) | . as $x | (. + $x | {T}) | select(. > 14))"), "16\n"),
        ("limit", format!("[limit(3; range(0;8) | . as $x | (. + $x | {T}))]"), "[10,12,14]\n"),
        ("an update through the body", format!("[1,2,3] | .[] |= (. as $x | (. + $x | {T}))"), "[12,14,16]\n"),
    ] {
        let (stdout, stderr, status) = run_jq_full(&["-n", "-c", &filter], None)?;
        assert_eq!((stdout.as_str(), status), (want, 0), "{name}: `{filter}` -- {stderr:?}");
    }
    Ok(())
}

/// A container read in the first stage and compared by identity after a kept tail: a NaN-bearing
/// array equals itself only by identity in jq. The `unshared-containers` holdout has no node
/// identity (#5254).
#[cfg_attr(
    feature = "unshared-containers",
    ignore = "holdout has no node identity or sharing (#5254)"
)]
#[test]
fn test_container_identity_survives_a_body_that_reads_its_variable_5436() -> Result<()> {
    let long = ". | [.] | .[0] | [.] | .[0] | . | ";
    for (name, filter, want) in [
        (
            "an array",
            format!("[nan] as $a | [range(0;3) | . as $x | ($a | {long}[., $x] | .[0] | . | . | . | . == $a)]"),
            "[true,true,true]\n",
        ),
        (
            "a NaN",
            format!("nan as $n | [range(0;3) | . as $x | ($n | [., $x] | .[0] | {T} | isnan)]"),
            "[true,true,true]\n",
        ),
    ] {
        let (stdout, stderr, status) = run_jq_full(&["-n", "-c", &filter], None)?;
        assert_eq!((stdout.as_str(), status), (want, 0), "{name}: `{filter}` -- {stderr:?}");
    }
    Ok(())
}
