//! #4095: `recurse(.[]?; cond)` and the jq-mode input stream. Making the walk a navigational stage
//! for the position walk must not make jq's `first(recurse(.[]?; input))` read inputs jq never
//! reads: `cond` is asked only of the nodes the consumer reaches. jq 1.7.1 reads one input per
//! `cond` call, so after `first` returns the root nothing is consumed.

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn test_recurse_cond_reads_no_input_past_the_first_node_4095() -> Result<()> {
    let (stdout, _, code) = run_jq_full(
        &[
            "-c",
            "-n",
            "input | [first(recurse(.[]?; input)) | path], [inputs]",
        ],
        Some("{\"a\":[1,2]}\n1\n2\n3\n4\n5\n"),
    )?;
    assert_eq!(code, 0);
    assert_eq!(stdout, "[[]]\n[1,2,3,4,5]\n");
    Ok(())
}

/// The same walk named through `path(...)`, which jq 1.7.1 answers identically (captured live).
#[test]
fn test_path_of_recurse_cond_reads_no_input_past_the_first_node_4095() -> Result<()> {
    let (stdout, _, code) = run_jq_full(
        &[
            "-c",
            "-n",
            "input | [path(first(recurse(.[]?; input)))], [inputs]",
        ],
        Some("{\"a\":[1,2]}\n1\n2\n3\n4\n5\n"),
    )?;
    assert_eq!(code, 0);
    assert_eq!(stdout, "[[]]\n[1,2,3,4,5]\n");
    Ok(())
}
