//! #5363: a capture group that took no part in the match is `null`, not absent, in `capture` and
//! `scan` (and in the object `sub`/`gsub` hand their replacement). Every row was captured from
//! jq 1.7.1 (`/usr/bin/jq`).

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn test_capture_and_scan_keep_a_group_that_did_not_match_as_null_5363() -> Result<()> {
    for (program, want) in [
        // `capture`: leading, trailing and several unmatched named groups keep their keys.
        (
            r#""b" | capture("(?<x>a)?(?<y>b)")"#,
            "{\"x\":null,\"y\":\"b\"}\n",
        ),
        (
            r#""a" | capture("(?<x>a)(?<y>b)?")"#,
            "{\"x\":\"a\",\"y\":null}\n",
        ),
        (
            r#""a" | capture("(?<x>a)(?<y>b)?(?<z>c)?")"#,
            "{\"x\":\"a\",\"y\":null,\"z\":null}\n",
        ),
        // Unnamed groups are not part of the object.
        (r#""x" | capture("(x)(?<n>q)?")"#, "{\"n\":null}\n"),
        // The `"g"` flag: one object per match, each with the group that missed.
        (
            r#""ab" | [capture("(?<x>a)|(?<y>b)";"g")]"#,
            "[{\"x\":\"a\",\"y\":null},{\"x\":null,\"y\":\"b\"}]\n",
        ),
        // `scan`: the group stays in its place.
        (r#""ab" | [scan("(a)(b)?(c)?")]"#, "[[\"a\",\"b\",null]]\n"),
        (r#""a" | [scan("(a)(b)?(c)?")]"#, "[[\"a\",null,null]]\n"),
        (r#""b" | [scan("(a)|(b)")]"#, "[[null,\"b\"]]\n"),
        (
            r#""aXb" | [scan("(?<l>[a-z])|(?<u>[A-Z])")]"#,
            "[[\"a\",null],[null,\"X\"],[\"b\",null]]\n",
        ),
        (r#""x" | [scan("(a)?x")]"#, "[[null]]\n"),
        (
            r#""abab" | [scan("(a)(z)?(b)")]"#,
            "[[\"a\",null,\"b\"],[\"a\",null,\"b\"]]\n",
        ),
        // The object `sub`/`gsub` evaluate their replacement against has the key too.
        (
            r#""a" | sub("(?<x>a)(?<y>z)?"; "\(keys)")"#,
            "\"[\\\"x\\\",\\\"y\\\"]\"\n",
        ),
        (
            r#""a" | sub("(?<x>a)(?<y>z)?"; "\(has("y"))")"#,
            "\"true\"\n",
        ),
        // Unchanged: a group that matched, and `match`, which already listed every group.
        (
            r#""ab" | capture("(?<x>a)(?<y>b)")"#,
            "{\"x\":\"a\",\"y\":\"b\"}\n",
        ),
        (
            r#""a" | [match("(a)(z)?").captures[] | .string]"#,
            "[\"a\",null]\n",
        ),
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-nc", program], None)?;
        assert_eq!((stdout.as_str(), code), (want, 0), "{program}: {stderr}");
    }
    Ok(())
}
