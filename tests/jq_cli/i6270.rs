//! #6270: a computed index (`.[0+0]`, `.a[1-1]`) of an array with no element but a comma (`[,]`) is
//! refused, as the literal index and the computed slice (#6257) are: the walk of an array sees no gap
//! when it has no element, so the check is made where the cursor is held. Each row was run through
//! jq 1.7.1 (`/usr/bin/jq -c`); the rows left out are a malformed array the filter never reads
//! (`{"a":[,]} | .[0+0]?`), which stays unread (see `docs/compliance/jq/limitations.md`).

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn a_computed_index_of_an_array_with_a_stray_comma_is_an_error_6270() -> Result<()> {
    for (document, filter, expected) in [
        (r"[,]", r".[0+0]", None),
        (r"[,]", r".[1-1]", None),
        (r"[,]", r".a[0+0]", None),
        (r"[,]", r".[0+0]?", None),
        (r"[,]", r".[(0,0)+0]", None),
        (r"[,]", r".[0][0+0]", None),
        (r"[,]", r"(.a,.a)[0+0]", None),
        (r"[,]", r".[0+0] // 1", None),
        (r"[,]", r"[.[0+0]]|length", None),
        (r#"{"a":[,]}"#, r".[0+0]", None),
        (r#"{"a":[,]}"#, r".[1-1]", None),
        (r#"{"a":[,]}"#, r".a[0+0]", None),
        (r#"{"a":[,]}"#, r".[(0,0)+0]", None),
        (r#"{"a":[,]}"#, r".[0][0+0]", None),
        (r#"{"a":[,]}"#, r"(.a,.a)[0+0]", None),
        (r#"{"a":[,]}"#, r".[0+0] // 1", None),
        (r#"{"a":[,]}"#, r"[.[0+0]]|length", None),
        (r"{,}", r".[0+0]", None),
        (r"{,}", r".[1-1]", None),
        (r"{,}", r".a[0+0]", None),
        (r"{,}", r".[0+0]?", None),
        (r"{,}", r".[(0,0)+0]", None),
        (r"{,}", r".[0][0+0]", None),
        (r"{,}", r"(.a,.a)[0+0]", None),
        (r"{,}", r".[0+0] // 1", None),
        (r"{,}", r"[.[0+0]]|length", None),
        (r"[]", r".[0+0]", Some(r"null")),
        (r"[]", r".[1-1]", Some(r"null")),
        (r"[]", r".a[0+0]", None),
        (r"[]", r".[0+0]?", Some(r"null")),
        (
            r"[]",
            r".[(0,0)+0]",
            Some(
                r"null
null",
            ),
        ),
        (r"[]", r".[0][0+0]", Some(r"null")),
        (r"[]", r"(.a,.a)[0+0]", None),
        (r"[]", r".[0+0] // 1", Some(r"1")),
        (r"[]", r"[.[0+0]]|length", Some(r"1")),
        (r#"{"a":[]}"#, r".[0+0]", None),
        (r#"{"a":[]}"#, r".[1-1]", None),
        (r#"{"a":[]}"#, r".a[0+0]", Some(r"null")),
        (r#"{"a":[]}"#, r".[0+0]?", Some(r"")),
        (r#"{"a":[]}"#, r".[(0,0)+0]", None),
        (r#"{"a":[]}"#, r".[0][0+0]", None),
        (
            r#"{"a":[]}"#,
            r"(.a,.a)[0+0]",
            Some(
                r"null
null",
            ),
        ),
        (r#"{"a":[]}"#, r".[0+0] // 1", None),
        (r#"{"a":[]}"#, r"[.[0+0]]|length", None),
        (r"[1,2]", r".[0+0]", Some(r"1")),
        (r"[1,2]", r".[1-1]", Some(r"1")),
        (r"[1,2]", r".a[0+0]", None),
        (r"[1,2]", r".[0+0]?", Some(r"1")),
        (
            r"[1,2]",
            r".[(0,0)+0]",
            Some(
                r"1
1",
            ),
        ),
        (r"[1,2]", r".[0][0+0]", None),
        (r"[1,2]", r"(.a,.a)[0+0]", None),
        (r"[1,2]", r".[0+0] // 1", Some(r"1")),
        (r"[1,2]", r"[.[0+0]]|length", Some(r"1")),
        (r"[[,]]", r".[0+0]", None),
        (r"[[,]]", r".[1-1]", None),
        (r"[[,]]", r".a[0+0]", None),
        (r"[[,]]", r".[0+0]?", None),
        (r"[[,]]", r".[(0,0)+0]", None),
        (r"[[,]]", r".[0][0+0]", None),
        (r"[[,]]", r"(.a,.a)[0+0]", None),
        (r"[[,]]", r".[0+0] // 1", None),
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
