//! #7459, a deliberate divergence from jq: two equally long slices of one array are compared by
//! content here, where jq 1.7.1 and 1.8.2 call them equal whatever they hold.
//!
//! jq's `jv_equal` short-circuits on the storage pointer and the length without checking the
//! slice offset, so in jq `[1,2,3,4,5,6] | .[:3] == .[3:]` is `true`, `unique` keeps one of the
//! two slices and array `-` removes both. Matching that would make `unique`, `group_by` and `-`
//! silently merge arrays that differ, so the repo owner chose to keep the content comparison
//! (ADR-0018 rule 4(b)); `docs/compliance/jq/limitations.md` records it with jq's answers beside
//! these. The rows below pin succinctly's answers so that a "fidelity fix" to slice identity in
//! `==` cannot flip them without this test and the limitations entry changing with it.

use anyhow::Result;

use crate::run_jq_full;

/// The array every row slices.
const SIX: &str = "[1,2,3,4,5,6]";

/// (filter, `succinctly jq -c` output on [`SIX`]). The trailing comment is jq 1.7.1's answer
/// where it differs.
const ROWS: &[(&str, &str)] = &[
    (".[:3] == .[3:]", "false\n"),                   // jq: true
    (".[:3] != .[3:]", "true\n"),                    // jq: false
    (".[1:] == .[:-1]", "false\n"),                  // jq: true
    (".[:3] == .[-3:]", "false\n"),                  // jq: true
    (".[:3] == (.[3:] | .[:3])", "false\n"),         // jq: true
    ("[.[:3]] == [.[3:]]", "false\n"),               // jq: true
    ("{a: .[:2], b: .[2:4]} | .a == .b", "false\n"), // jq: true
    (
        ".[:3] as $a | .[3:] as $b | [$a == $b, $a < $b]",
        "[false,true]\n", // jq: [true,true]
    ),
    ("(.[:3] + []) == .[3:]", "false\n"), // jq: true
    ("[.[:3], .[3:]] | unique", "[[1,2,3],[4,5,6]]\n"), // jq: [[1,2,3]]
    ("[.[:3], .[3:]] | group_by(.) | length", "2\n"), // jq: 1
    ("[.[:3]] - [.[3:]]", "[[1,2,3]]\n"), // jq: []
    (". as $x | [$x[:3]] | indices([$x[3:]])", "[]\n"), // jq: [0]
    (". as $x | $x[:3] | IN($x[3:])", "false\n"), // jq: true
    (". as $x | [[0], $x[:2], $x[4:]] | bsearch($x[2:4])", "-3\n"), // jq 1.7.1: 1, 1.8.2: -3
    ("select(.[:3] == .[-3:]) | length", ""), // jq: 6
    // Agreeing with jq: a different length, a copy, and the by-value consumers.
    (".[:3] == .[1:3]", "false\n"),
    ("map(.) as $c | .[:3] == $c[3:]", "false\n"),
    ("[.[:3], .[3:]] | sort", "[[1,2,3],[4,5,6]]\n"),
    (". as $x | [$x[:3]] | contains([$x[3:]])", "false\n"),
    (".[:2] == .[:2]", "true\n"),
];

#[test]
fn slices_of_one_array_compare_by_content_a_recorded_divergence_7459() -> Result<()> {
    for (filter, want) in ROWS {
        let (stdout, stderr, code) = run_jq_full(&["-c", filter], Some(SIX))?;
        assert_eq!(
            (stdout.as_str(), code),
            (*want, 0),
            "stdin `{filter}`: {stderr}"
        );
        let literal = format!("{SIX} | {filter}");
        let (stdout, stderr, code) = run_jq_full(&["-nc", &literal], None)?;
        assert_eq!(
            (stdout.as_str(), code),
            (*want, 0),
            "-n `{literal}`: {stderr}"
        );
    }
    Ok(())
}
