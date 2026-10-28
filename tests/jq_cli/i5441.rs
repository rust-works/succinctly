//! #5441: a body rebuilt for each value that reads its variable in its last stage, or anywhere
//! before a long run of stages, hands on the kept runs of its pipe around the stages that differ
//! per value, instead of copying the rest of the pipe for each stage. The copies are pinned by the
//! unit tests `test_body_that_reads_its_variable_shares_the_tails_after_it_5436` and
//! `test_pipe_stages_rebuilt_from_a_pipe_that_reads_a_variable_split_around_it_5441`; these rows
//! pin that the answers are the ones jq gives for a variable read last, second last, in the middle,
//! in every stage and in every third, through a generator, an error, a `break`, `first`, `limit`,
//! `isempty` and an update, and that a container keeps its storage identity down a run that is
//! evaluated apart from the stage that reads the variable (`[nan] as $a | ... | . == $a`). Expected
//! outputs were captured from jq 1.7.1 (`/usr/bin/jq`). Ten computing stages, so each tail the pipe
//! hands on is long enough to be split.

use anyhow::Result;

use crate::run_jq_full;

/// Ten computing stages.
const T: &str = ". + 1 | . + 1 | . + 1 | . + 1 | . + 1 | . + 1 | . + 1 | . + 1 | . + 1 | . + 1";

#[test]
fn test_body_that_reads_its_variable_late_matches_jq_5441() -> Result<()> {
    let every = [". + $x"; 10].join(" | ");
    let third = [". + $x | . + 1 | . + 1"; 5].join(" | ");
    for (name, filter, want) in [
        ("last", format!("[range(0;6) | . as $x | ({T} | . + $x)]"), "[10,12,14,16,18,20]\n"),
        ("second last", format!("[range(0;6) | . as $x | ({T} | . + $x | . + 1)]"), "[11,13,15,17,19,21]\n"),
        ("last, then a nested binder", format!("[range(0;6) | . as $x | ({T} | . + $x | . as $y | ({T} | . + $y))]"), "[30,34,38,42,46,50]\n"),
        ("middle, long on both sides", format!("[range(0;6) | . as $x | ({T} | . + $x | {T})]"), "[20,22,24,26,28,30]\n"),
        ("every stage", format!("[range(0;6) | . as $x | ({every})]"), "[0,11,22,33,44,55]\n"),
        ("every third stage", format!("[range(0;6) | . as $x | ({third})]"), "[10,16,22,28,34,40]\n"),
        ("destructured, last", format!("[range(0;6) | [., 2] as [$a, $b] | ({T} | . + $a + $b)]"), "[12,14,16,18,20,22]\n"),
        ("object pattern, last", format!("[range(0;6) | {{a: ., b: 2}} as {{a: $a, b: $b}} | ({T} | . + $b)]"), "[12,13,14,15,16,17]\n"),
        ("alternative pattern, last", format!("[[1], 2, [3]] | [.[] as [$a] ?// $a | ($a | {T} | . + $a)]"), "[12,14,16]\n"),
        ("a def call after the reader", format!("def f: . + 1; [range(0;6) | . as $x | ({T} | . + $x | f)]"), "[11,13,15,17,19,21]\n"),
        ("a generator after the reader", format!("[range(0;3) | . as $x | ({T} | . + $x | (., . + 100) | . + 1)]"), "[11,111,13,113,15,115]\n"),
        ("a generator before the reader", format!("[range(0;3) | . as $x | ((., . + 100) | {T} | . + $x)]"), "[10,110,12,112,14,114]\n"),
        ("a reduce", format!("reduce range(0;6) as $i (0; {T} | . + $i)"), "75\n"),
        ("a foreach", format!("[foreach range(0;6) as $i (0; . + 1; {T} | . + $i)]"), "[11,13,15,17,19,21]\n"),
        ("a recursive def", format!("def h: if . > 60 then . else (. as $x | ({T} | . + $x | h)) end; [range(0;3) | h]"), "[70,78,86]\n"),
        ("an error caught per value", format!("[range(0;6) | try (. as $x | ({T} | if . == 14 then error(\"boom\") else . end | . + $x)) catch \"c\"]"), "[10,12,14,16,\"c\",20]\n"),
        ("an error raised after the reader", format!("[range(0;6) | try (. as $x | ({T} | . + $x | if . == 14 then error(\"boom\") else . end | . + 1)) catch \"c\"]"), "[11,13,\"c\",17,19,21]\n"),
        ("a break out of the body", format!("[range(0;6) | label $out | (. as $x | ({T} | if . > 13 then break $out else . end | . + $x))]"), "[10,12,14,16]\n"),
        ("a break after the reader", format!("[range(0;6) | label $out | (. as $x | ({T} | . + $x | if . > 25 then break $out else . end | . + 1))]"), "[11,13,15,17,19,21]\n"),
        ("first", format!("first(range(0;8) | . as $x | ({T} | . + $x) | select(. > 14))"), "16\n"),
        ("limit", format!("[limit(3; range(0;8) | . as $x | ({T} | . + $x))]"), "[10,12,14]\n"),
        ("limit through a generator before the reader", format!("[limit(5; range(0;3) | . as $x | ((., 10 * .) | {T} | . + $x))]"), "[10,10,12,21,14]\n"),
        ("limit through a generator after the reader", format!("[limit(5; range(0;3) | . as $x | ({T} | . + $x | (., . + 100) | . + 1))]"), "[11,111,13,113,15]\n"),
        ("isempty", format!("[range(0;3) | . as $x | isempty({T} | . + $x | empty)]"), "[true,true,true]\n"),
        ("an update through the body", format!("[1,2,3] | .[] |= (. as $x | ({T} | . + $x))"), "[12,14,16]\n"),
        ("an assignment through the body", format!("{{\"a\": [1,2]}} | .a[] |= (. as $x | ({T} | . + $x))"), "{\"a\":[12,14]}\n"),
    ] {
        let (stdout, stderr, status) = run_jq_full(&["-n", "-c", &filter], None)?;
        assert_eq!((stdout.as_str(), status), (want, 0), "{name}: `{filter}` -- {stderr:?}");
    }
    Ok(())
}

/// A container passed down a run and compared by identity after it: a NaN-bearing array equals itself
/// only by identity in jq. The `unshared-containers` holdout has no node identity (#5254).
#[cfg_attr(
    feature = "unshared-containers",
    ignore = "holdout has no node identity or sharing (#5254)"
)]
#[test]
fn test_container_identity_survives_a_run_split_around_a_reader_5441() -> Result<()> {
    let long = ". | [.] | .[0] | [.] | .[0] | . | ";
    for (name, filter, want) in [
        (
            "the reader last",
            format!("[nan] as $a | [range(0;3) | . as $x | ($a | {long}. == $a | . and ($x >= 0))]"),
            "[true,true,true]\n",
        ),
        (
            "the reader in the middle",
            format!("[nan] as $a | [range(0;3) | . as $x | ($a | {long}[., $x] | .[0] | {long}. == $a)]"),
            "[true,true,true]\n",
        ),
        (
            "the reader second last",
            format!("[nan] as $a | [range(0;3) | . as $x | ($a | {long}. | . | . == $a | [., $x] | .[0])]"),
            "[true,true,true]\n",
        ),
        (
            "a run of identity stages ending the body",
            format!("[nan] as $a | [range(0;3) | . as $x | ($a | . | . | . | . | . | . | . | . | . | .)] | map(. == $a)"),
            "[true,true,true]\n",
        ),
        (
            "a reduce body that reads last",
            format!("[[nan]] as $a | reduce range(0;4) as $i ($a; {{k: .}} | .k | {long}[., $i] | .[0]) | . == $a"),
            "true\n",
        ),
    ] {
        let (stdout, stderr, status) = run_jq_full(&["-n", "-c", &filter], None)?;
        assert_eq!((stdout.as_str(), status), (want, 0), "{name}: `{filter}` -- {stderr:?}");
    }
    Ok(())
}
