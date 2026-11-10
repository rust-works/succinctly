//! #4569: yq slices only a string as text. An unquoted timestamp, a `!!binary`, an explicit core
//! tag other than `!!str` and an alias are not strings to it, so a slice of one is an empty
//! sequence, where `succinctly yq` cut the scalar's text. The text alone cannot say which (a
//! quoted `"2024-01-01"` is a string), so the answer comes from the cursor's style and tag.
//! Each row is a live yq v4.53.3 capture over the document below, `-o=json -I0`.

use anyhow::Result;

use crate::run_yq_stdin;

const DOC: &str = "ts: 2024-01-01\n\
dt: 2024-01-01T10:00:00Z\n\
bin: !!binary aGVsbG8=\n\
vbin: !<tag:yaml.org,2002:binary> aGVsbG8=\n\
qts: \"2024-01-01\"\n\
str: !!str 2024-01-01\n\
n: !!int \"5\"\n\
cu: !mytag hello\n\
al: &x hello\n\
al2: *x\n";

/// (filter, expected output): a literal bound and a computed one (`1+0:`, which is not folded) for
/// every key, so both slice routes answer alike.
const ROWS: &[(&str, &str)] = &[
    (".ts[0:2]", "[]"),
    (".ts[1+0:]", "[]"),
    (".dt[0:2]", "[]"),
    (".dt[1+0:]", "[]"),
    (".bin[0:2]", "[]"),
    (".bin[1+0:]", "[]"),
    (".vbin[0:2]", "[]"),
    (".vbin[1+0:]", "[]"),
    (".qts[0:2]", "\"20\""),
    (".qts[1+0:]", "\"024-01-01\""),
    (".str[0:2]", "\"20\""),
    (".str[1+0:]", "\"024-01-01\""),
    (".n[0:2]", "[]"),
    (".n[1+0:]", "[]"),
    (".cu[0:2]", "\"he\""),
    (".cu[1+0:]", "\"ello\""),
    (".al[0:2]", "\"he\""),
    (".al[1+0:]", "\"ello\""),
    (".al2[0:2]", "[]"),
    (".al2[1+0:]", "[]"),
    ("[.ts[0:2], .qts[0:2]]", "[[],\"20\"]"),
    (".ts[0:2] | length", "0"),
    (".ts[1+0:] | length", "0"),
    (".bin[:-1] | length", "0"),
    (".ts[0:2][0]", "null"),
    (".ts[0:2] + .qts[0:2]", "[\"20\"]"),
];

#[test]
fn test_slice_of_a_scalar_yq_does_not_read_as_a_string_is_empty_4569() -> Result<()> {
    for (filter, expected) in ROWS {
        let (stdout, code) = run_yq_stdin(filter, DOC, &["-o=json", "-I0"])?;
        assert_eq!(
            (stdout.trim_end(), code),
            (*expected, 0),
            "{filter} over {DOC:?}"
        );
    }
    Ok(())
}

/// A field after the slice is then an index into an empty sequence, which yq refuses; the same
/// path over a quoted date is a string and answers nothing.
#[test]
fn test_field_after_the_slice_of_a_timestamp_fails_like_yq_4569() -> Result<()> {
    let (stdout, code) = run_yq_stdin(".ts[0:2].x", DOC, &["-o=json", "-I0"])?;
    assert_ne!(code, 0, "a field after an empty sequence is an error");
    assert_eq!(stdout, "");
    let (stdout, code) = run_yq_stdin(".qts[0:2].x", DOC, &["-o=json", "-I0"])?;
    assert_eq!((stdout.as_str(), code), ("", 0));
    Ok(())
}

/// JSON has no timestamp, binary or alias: every string is one, so a slice cuts its text.
#[test]
fn test_json_string_still_slices_as_text_4569() -> Result<()> {
    let (stdout, code) = run_yq_stdin(
        "[.a[0:2], .a[1+0:]]",
        "{\"a\": \"2024-01-01\"}",
        &["-p=json", "-o=json", "-I0"],
    )?;
    assert_eq!((stdout.trim_end(), code), ("[\"20\",\"024-01-01\"]", 0));
    Ok(())
}
