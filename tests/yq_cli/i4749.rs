//! #4749: a `del()` of several plain array indices, one of them past the end. yq handles the
//! highest first and pads the array to an index it reaches past the end of, so
//! `del(.[5], .[0])` on three elements is `[20,30,null,null]` whatever order the paths come in;
//! `succinctly yq` dropped the padding for a batch of two or more (a single index already
//! padded, #2305). Every row's answer was captured from yq v4.53.3 (`-p=json -o=json -I0`).
//! A batch with a negative index or a slice resolves order-dependently in yq and is not pinned.

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
