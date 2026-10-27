//! #5397: a comma-grouped `del()` stops at a bare slice of the root (#1325) whether its bounds are
//! literals or computed. A bound is read against the root; the slice is the stop when each one is
//! exactly one integer yq's `ParseInt` takes, and only on the first slice of the run (yq reads a later
//! slice's bounds against the slice before it). A bound that is none, several, `null`, a string or
//! fractional is not the stop: yq raises on those, and so does this tool where it did before.
//!
//! Every output is captured from yq v4.53.3.

use anyhow::Result;

use crate::run_yq_stdin;

const OBJECT: &str = r#"{"a":{"x":1,"y":2},"b":6,"c":9}"#;
const ARRAY: &str = "[[1,2,3],[4,5,6],[7,8,9],[10,11,12]]";

/// (filter, `yq -o=json -I0` output over [`OBJECT`]).
const OBJECT_ROWS: &[(&str, &str)] = &[
    (
        "del(.c, .[(1+1):3])",
        "{\"a\":{\"x\":1,\"y\":2},\"b\":6,\"c\":9}\n",
    ),
    ("del(.[(1+1):3], .c)", "{\"a\":{\"x\":1,\"y\":2},\"b\":6}\n"),
    (
        "del(.c, .[(.b):3])",
        "{\"a\":{\"x\":1,\"y\":2},\"b\":6,\"c\":9}\n",
    ),
    (
        "del(.c, .[:(1+1)])",
        "{\"a\":{\"x\":1,\"y\":2},\"b\":6,\"c\":9}\n",
    ),
    (
        "del(.c, .[(1+1):(2+2)])",
        "{\"a\":{\"x\":1,\"y\":2},\"b\":6,\"c\":9}\n",
    ),
    (
        "del(.c, .[(1+1):3][0:1])",
        "{\"a\":{\"x\":1,\"y\":2},\"b\":6,\"c\":9}\n",
    ),
    (
        "del(.c, .[(1+1):3] | .[0:1])",
        "{\"a\":{\"x\":1,\"y\":2},\"b\":6,\"c\":9}\n",
    ),
    (
        "del(.c, .[(1+1):3][0])",
        "{\"a\":{\"x\":1,\"y\":2},\"b\":6}\n",
    ),
    ("del(.c, .a[(1+1):3])", "{\"b\":6}\n"),
    (
        "del(.c, .[(4/2):3])",
        "{\"a\":{\"x\":1,\"y\":2},\"b\":6,\"c\":9}\n",
    ),
    (
        "del(.c, .[(1.0+1):3])",
        "{\"a\":{\"x\":1,\"y\":2},\"b\":6,\"c\":9}\n",
    ),
    (
        "del(.c, .[(.a|length):3])",
        "{\"a\":{\"x\":1,\"y\":2},\"b\":6,\"c\":9}\n",
    ),
    (
        "del(.c, .[(1+1):3], .b)",
        "{\"a\":{\"x\":1,\"y\":2},\"c\":9}\n",
    ),
    (
        "del(.b, .[(1+1):3], .c)",
        "{\"a\":{\"x\":1,\"y\":2},\"b\":6}\n",
    ),
    (
        "del(.c, .[(1+1):3], .[2:3])",
        "{\"a\":{\"x\":1,\"y\":2},\"b\":6,\"c\":9}\n",
    ),
    (
        "del(.c, .[(1+1):3], .[(1+1):3])",
        "{\"a\":{\"x\":1,\"y\":2},\"b\":6,\"c\":9}\n",
    ),
    (
        "del(.c, . | .[(1+1):3])",
        "{\"a\":{\"x\":1,\"y\":2},\"b\":6,\"c\":9}\n",
    ),
    (
        "del(.c, (.[(1+1):3], .b))",
        "{\"a\":{\"x\":1,\"y\":2},\"c\":9}\n",
    ),
    (
        "del((.c, .[(1+1):3]), .b)",
        "{\"a\":{\"x\":1,\"y\":2},\"c\":9}\n",
    ),
];

/// (filter, `yq -o=json -I0` output over [`ARRAY`]).
const ARRAY_ROWS: &[(&str, &str)] = &[
    (
        "del(.[3], .[(1+1):3])",
        "[[1,2,3],[4,5,6],[7,8,9],[10,11,12]]\n",
    ),
    ("del(.[(1+1):3], .[3])", "[[1,2,3],[4,5,6],[7,8,9]]\n"),
    (
        "del(.[0][0:1], .[(1+1):3])",
        "[[1,2,3],[4,5,6],[7,8,9],[10,11,12]]\n",
    ),
    (
        "del(.[(1+1):3], .[0][0:1])",
        "[[4,5,6],[7,8,9],[10,11,12]]\n",
    ),
    (
        "del(.[1], .[(0+1):(1+1)], .[3])",
        "[[1,2,3],[4,5,6],[7,8,9]]\n",
    ),
];

#[test]
fn test_5397_a_a_computed_root_slice_stops_the_delete_over_an_object() -> Result<()> {
    for (filter, want) in OBJECT_ROWS {
        let (out, code) = run_yq_stdin(filter, OBJECT, &["-o=json", "-I=0"])?;
        assert_eq!(code, 0, "{filter}");
        assert_eq!(&out, want, "{filter}");
    }
    Ok(())
}

#[test]
fn test_5397_b_a_computed_root_slice_stops_the_delete_over_an_array() -> Result<()> {
    for (filter, want) in ARRAY_ROWS {
        let (out, code) = run_yq_stdin(filter, ARRAY, &["-o=json", "-I=0"])?;
        assert_eq!(code, 0, "{filter}");
        assert_eq!(&out, want, "{filter}");
    }
    Ok(())
}

/// A bound that is not one integer is not the stop: yq raises, and so does this tool.
#[test]
fn test_5397_c_a_bound_with_no_single_integer_still_raises() -> Result<()> {
    for filter in ["del(.c, .[(1,2):3])", "del(.c, .[(empty):3])"] {
        let (out, code) = run_yq_stdin(filter, OBJECT, &["-o=json", "-I=0"])?;
        assert_eq!((out.as_str(), code), ("", 1), "{filter}");
    }
    Ok(())
}

/// (document, filter, `yq -o=json -I0` output): a scalar or `null` root takes the same rule.
const SCALAR_ROWS: &[(&str, &str, &str)] = &[
    ("\"abcdef\"", "del(.b, .[(1+1):3])", "\"abcdef\"\n"),
    ("\"abcdef\"", "del(.[(1+1):3], .b)", "\"abcdef\"\n"),
    ("null", "del(.b, .[(1+1):3])", "null\n"),
    ("5", "del(.b, .[(1+1):3])", "5\n"),
];

#[test]
fn test_5397_d_a_scalar_or_null_root_takes_the_same_rule() -> Result<()> {
    for (input, filter, want) in SCALAR_ROWS {
        let (out, code) = run_yq_stdin(filter, input, &["-o=json", "-I=0"])?;
        assert_eq!(code, 0, "{filter} on {input}");
        assert_eq!(&out, want, "{filter} on {input}");
    }
    Ok(())
}

/// A string, a fractional number and a `1e6`-magnitude float are not integers yq's `ParseInt` takes,
/// so the slice is not the stop: yq raises on each, and so does this tool.
#[test]
fn test_5397_e_a_bound_that_is_not_an_integer_still_raises() -> Result<()> {
    for filter in [
        "del(.c, .[(\"a\"):3])",
        "del(.c, .[(1.5):3])",
        "del(.c, .[(1e6):3])",
    ] {
        let (out, code) = run_yq_stdin(filter, OBJECT, &["-o=json", "-I=0"])?;
        assert_eq!((out.as_str(), code), ("", 1), "{filter}");
    }
    Ok(())
}

/// A `null` bound is not the stop either. yq raises on it (`strconv.ParseInt: parsing "null"`); this
/// tool treats it as an open bound and deletes `.c`, as it did before #5397.
#[test]
fn test_5397_f_a_null_bound_is_not_the_stop() -> Result<()> {
    let (out, code) = run_yq_stdin("del(.c, .[(null):3])", OBJECT, &["-o=json", "-I=0"])?;
    assert_eq!(code, 0);
    assert_eq!(out, "{\"a\":{\"x\":1,\"y\":2},\"b\":6}\n");
    Ok(())
}

/// yq reads a later slice's bounds against the slice before it (`.[0:1][(.b):3]` raises), so only the
/// first slice of a run is folded. A later computed slice is not recognised as the stop, where yq
/// stops: the group is deleted as if the slice were absent. Recorded in `limitations.md`.
#[test]
fn test_5397_g_a_computed_bound_on_a_later_slice_is_not_the_stop() -> Result<()> {
    for filter in ["del(.c, .[0:1][(1+1):3])", "del(.c, .[0:1] | .[(1+1):3])"] {
        let (out, code) = run_yq_stdin(filter, OBJECT, &["-o=json", "-I=0"])?;
        assert_eq!(code, 0, "{filter}");
        assert_eq!(out, "{\"a\":{\"x\":1,\"y\":2},\"b\":6}\n", "{filter}");
    }
    Ok(())
}
