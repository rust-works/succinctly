//! #5231: a NaN slice end bound is the length (jq's `parse_slice`: `if (isnan(dend)) dend = len`), while a NaN start bound
//! is `0`. The end bound was sent to `0` as well, so `.[1:nan]` read `[]` and writes through it inserted or did nothing.
//! Every row is captured from jq 1.7.1; the NaN-start rows are pinned too so that bound cannot regress.

use anyhow::Result;

use crate::run_jq_stdin;

#[test]
fn test_nan_slice_bounds_match_jq_5231() -> Result<()> {
    let rows: &[(&str, &str, &str)] = &[
        ("[1,2,3,4]", ".[1:nan]", "[2,3,4]\n"),
        ("[1,2,3,4]", ".[:nan]", "[1,2,3,4]\n"),
        ("[1,2,3,4]", ".[nan:nan]", "[1,2,3,4]\n"),
        ("[1,2,3,4]", ".[nan:2]", "[1,2]\n"),
        ("[1,2,3,4]", ".[nan:]", "[1,2,3,4]\n"),
        ("\"abcd\"", ".[1:nan]", "\"bcd\"\n"),
        ("\"abcd\"", ".[:nan]", "\"abcd\"\n"),
        ("\"abcd\"", ".[nan:2]", "\"ab\"\n"),
        (
            "[1,2,3,4]",
            "path(.[1:nan])",
            "[{\"start\":1,\"end\":null}]\n",
        ),
        ("[1,2,3,4]", ".[1:nan] = [\"x\"]", "[1,\"x\"]\n"),
        ("[1,2,3,4]", "del(.[1:nan])", "[1]\n"),
        ("[1,2,3,4]", ".[1:nan] |= map(.*10)", "[1,20,30,40]\n"),
        ("[1,2,3,4]", "nan as $n | .[1:$n]", "[2,3,4]\n"),
        ("[1,2,3,4]", "[.[(1,nan):]]", "[[2,3,4],[1,2,3,4]]\n"),
        ("[1,2,3,4]", "nan as $n | del(.[1:$n])", "[1]\n"),
        ("[1,2,3,4]", "nan as $n | .[1:$n] = [\"x\"]", "[1,\"x\"]\n"),
        ("[1,2,3,4]", ".[1:(nan|-.)]", "[2,3,4]\n"),
        ("null", ".[1:nan]", "null\n"),
    ];
    for (input, filter, expected) in rows {
        let (stdout, code) = run_jq_stdin(filter, input, &["-c"])?;
        assert_eq!(code, 0, "{filter} on {input}");
        assert_eq!(&stdout, expected, "{filter} on {input}");
    }
    Ok(())
}
