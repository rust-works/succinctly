//! #4749: a `del()` of several plain array indices, one of them past the end. yq handles the
//! highest first and pads the array to an index it reaches past the end of, so
//! `del(.[5], .[0])` on three elements is `[20,30,null,null]` whatever order the paths come in;
//! `succinctly yq` dropped the padding for a batch of two or more (a single index already
//! padded, #2305). Every row's answer was captured from yq v4.53.3 (`-p=json -o=json -I0`).
//!
//! Part two: a negative index and a repeated index. yq evaluates the paths left to right into
//! nodes (a past-the-end index grows the array as it is evaluated, a negative one counts from the
//! length at that moment), then deletes them from the back, each by the key it holds then; a node
//! already deleted keeps its old key, so naming it twice deletes what moved into its place. A
//! batch with a slice is not pinned.

use anyhow::Result;

use crate::run_yq_stdin;

fn del(doc: &str, filter: &str) -> Result<String> {
    let (stdout, code) = run_yq_stdin(filter, doc, &["-p=json", "-o=json", "-I=0"])?;
    assert_eq!(code, 0, "{filter}");
    Ok(stdout)
}

#[test]
fn test_multi_key_del_past_the_end_pads_to_the_highest_index_4749() -> Result<()> {
    for (filter, want) in [
        ("del(.[5], .[0])", "[20,30,null,null]\n"),
        ("del(.[0], .[5])", "[20,30,null,null]\n"),
        ("del(.[3], .[0])", "[20,30]\n"),
        ("del(.[4], .[1])", "[10,30,null]\n"),
        ("del(.[5], .[7])", "[10,20,30,null,null,null]\n"),
        ("del(.[7], .[5])", "[10,20,30,null,null,null]\n"),
        ("del(.[1], .[5], .[0])", "[30,null,null]\n"),
        ("del(.[5], .[1], .[0])", "[30,null,null]\n"),
        ("del(.[6], .[5], .[0])", "[20,30,null,null]\n"),
        ("del(.[5], .[4])", "[10,20,30,null]\n"),
    ] {
        assert_eq!(del("[10,20,30]", filter)?, want, "{filter}");
    }
    Ok(())
}

#[test]
fn test_multi_key_del_pads_each_array_of_a_nested_batch_4749() -> Result<()> {
    assert_eq!(
        del(
            r#"{"a":[1,2,3],"b":[4]}"#,
            "del(.a[5], .a[0], .b[3], .b[0])"
        )?,
        "{\"a\":[2,3,null,null],\"b\":[null,null]}\n"
    );
    Ok(())
}

#[test]
fn test_batches_that_already_agreed_still_do_4749() -> Result<()> {
    for (filter, want) in [
        // In range, or a single past-the-end index (#2305).
        ("del(.[0], .[2])", "[20]\n"),
        ("del(.[5])", "[10,20,30,null,null]\n"),
        ("del(.[3])", "[10,20,30]\n"),
    ] {
        assert_eq!(del("[10,20,30]", filter)?, want, "{filter}");
    }
    Ok(())
}

#[test]
fn test_multi_key_del_with_a_negative_index_resolves_in_path_order_4749() -> Result<()> {
    for (filter, want) in [
        // `.[-1]` is the last of three before `.[5]` grows the array, the sixth after.
        ("del(.[-1], .[5])", "[10,20,null,null]\n"),
        ("del(.[5], .[-1])", "[10,20,30,null,null]\n"),
        ("del(.[-2], .[7])", "[10,30,null,null,null,null]\n"),
        ("del(.[7], .[-2])", "[10,20,30,null,null,null]\n"),
        ("del(.[2], .[-1])", "[10,20]\n"),
        ("del(.[-1], .[-1])", "[10,20]\n"),
    ] {
        assert_eq!(del("[10,20,30]", filter)?, want, "{filter}");
    }
    Ok(())
}

#[test]
fn test_multi_key_del_of_a_repeated_index_deletes_again_4749() -> Result<()> {
    for (filter, want) in [
        ("del(.[0], .[0])", "[30]\n"),
        ("del(.[1], .[1], .[0])", "[]\n"),
        ("del(.[0], .[1], .[0])", "[]\n"),
        ("del(.[1], .[0], .[1])", "[30]\n"),
        // A repeat past the end finds nothing the second time.
        ("del(.[5], .[5])", "[10,20,30,null,null]\n"),
        // A string spelling an index names the same node.
        ("del(.[0], .[\"0\"])", "[30]\n"),
        ("del(.[\"5\"], .[0])", "[20,30,null,null]\n"),
    ] {
        assert_eq!(del("[10,20,30]", filter)?, want, "{filter}");
    }
    assert_eq!(
        del(
            r#"{"a":[1,2,3],"b":[4]}"#,
            "del(.a[0], .a[0], .b[-1], .b[2])"
        )?,
        "{\"a\":[3],\"b\":[null]}\n"
    );
    Ok(())
}

#[test]
fn test_a_repeated_index_of_a_yaml_sequence_deletes_again_4749() -> Result<()> {
    let (stdout, code) = run_yq_stdin("del(.a[0], .a[0])", "a:\n  - x\n  - y\n  - z\n", &[])?;
    assert_eq!(code, 0);
    assert_eq!(stdout, "a:\n  - z\n");
    Ok(())
}
