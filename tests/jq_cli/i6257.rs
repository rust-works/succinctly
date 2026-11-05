//! #6257: the computed-bound slice `E[S:T]` raises the structural error its own walk finds in the
//! array it slices (a missing separator, a stray or trailing comma, a comma in an array with no
//! element), as the literal spelling does since #6231. Before, it swallowed the walk's error and
//! answered over the malformed array. Each row was run through jq 1.7.1 (`/usr/bin/jq -c`).

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn a_computed_slice_of_a_malformed_array_is_an_error_6257() -> Result<()> {
    for (document, filter, expected) in [
        (r"[1 2 3]", r".[0+0:0+2]", None),
        (r"[1 2 3]", r".[1+0:]", None),
        (r"[1 2 3]", r".[:0+2]", None),
        (r"[1 2 3]", r".[0+0:0+2]?", None),
        (r"[1 2 3]", r"[.[0+0:1]]|length", None),
        (r"[1,,2]", r".[0+0:0+2]", None),
        (r"[1,,2]", r".[1+0:]", None),
        (r"[1,,2]", r".[:0+2]", None),
        (r"[1,,2]", r".[0+0:0+2]?", None),
        (r"[1,,2]", r"[.[0+0:1]]|length", None),
        (r"[1,2,]", r".[0+0:0+2]", None),
        (r"[1,2,]", r".[1+0:]", None),
        (r"[1,2,]", r".[:0+2]", None),
        (r"[1,2,]", r".[0+0:0+2]?", None),
        (r"[1,2,]", r"[.[0+0:1]]|length", None),
        (r"[,]", r".[0+0:0+2]", None),
        (r"[,]", r".[1+0:]", None),
        (r"[,]", r".[:0+2]", None),
        (r"[,]", r".[0+0:0+2]?", None),
        (r"[,]", r"[.[0+0:1]]|length", None),
        (r"[1,2,3]", r".[0+0:0+2]", Some(r"[1,2]")),
        (r"[1,2,3]", r".[1+0:]", Some(r"[2,3]")),
        (r"[1,2,3]", r".[:0+2]", Some(r"[1,2]")),
        (r"[1,2,3]", r".[0+0:0+2]?", Some(r"[1,2]")),
        (r"[1,2,3]", r"[.[0+0:1]]|length", Some(r"1")),
        (r"[]", r".[0+0:0+2]", Some(r"[]")),
        (r"[]", r".[1+0:]", Some(r"[]")),
        (r"[]", r".[:0+2]", Some(r"[]")),
        (r"[]", r".[0+0:0+2]?", Some(r"[]")),
        (r"[]", r"[.[0+0:1]]|length", Some(r"1")),
        (r"[1,2]", r".[0+0:0+2]", Some(r"[1,2]")),
        (r"[1,2]", r".[1+0:]", Some(r"[2]")),
        (r"[1,2]", r".[:0+2]", Some(r"[1,2]")),
        (r"[1,2]", r".[0+0:0+2]?", Some(r"[1,2]")),
        (r"[1,2]", r"[.[0+0:1]]|length", Some(r"1")),
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-c", filter], Some(document))?;
        match expected {
            Some(expected) => {
                assert_eq!(
                    stdout.trim_end(),
                    expected,
                    "{document} | {filter}: {stderr:?}"
                );
                assert_eq!(code, 0, "{document} | {filter}");
            }
            None => {
                assert_eq!(stdout.trim_end(), "", "{document} | {filter}");
                assert_ne!(code, 0, "{document} | {filter}");
            }
        }
    }
    Ok(())
}
