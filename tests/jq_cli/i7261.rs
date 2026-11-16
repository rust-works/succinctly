//! #7261: jq's bare `min`/`max` over a document array take #5683's single walk again, which reads a
//! plain integer element bare. Each row is `(document, filter, stdout)` as jq 1.7.1
//! (`/usr/bin/jq`) answers. The first rows put a plain integer beside a fraction whose `f64` is the
//! same, in both orders and both signs: jq compares two document numbers as their literals, so the
//! walk must not compare the bare integer as an `f64` against the fraction (it answered the integer
//! for `min` until the bare side was decoded too). The rest cover equal values in several spellings
//! (the first minimum and the last maximum win), mixed types and NaN.

use anyhow::Result;

use crate::run_jq_full;

const ROWS: &[(&str, &str, &str)] = &[
    (
        "[123456789012345679,123456789012345678.9]",
        "min",
        "123456789012345678.9\n",
    ),
    (
        "[123456789012345679,123456789012345678.9]",
        "max",
        "123456789012345679\n",
    ),
    (
        "[123456789012345679,123456789012345678.9]",
        "[min,max]",
        "[123456789012345678.9,123456789012345679]\n",
    ),
    (
        "[123456789012345678.9,123456789012345679]",
        "min",
        "123456789012345678.9\n",
    ),
    (
        "[123456789012345678.9,123456789012345679]",
        "max",
        "123456789012345679\n",
    ),
    (
        "[123456789012345678.9,123456789012345679]",
        "[min,max]",
        "[123456789012345678.9,123456789012345679]\n",
    ),
    (
        "[-123456789012345679,-123456789012345678.9]",
        "min",
        "-123456789012345679\n",
    ),
    (
        "[-123456789012345679,-123456789012345678.9]",
        "max",
        "-123456789012345678.9\n",
    ),
    (
        "[-123456789012345679,-123456789012345678.9]",
        "[min,max]",
        "[-123456789012345679,-123456789012345678.9]\n",
    ),
    (
        "[2,123456789012345679,123456789012345678.9,3]",
        "min",
        "2\n",
    ),
    (
        "[2,123456789012345679,123456789012345678.9,3]",
        "max",
        "123456789012345679\n",
    ),
    (
        "[2,123456789012345679,123456789012345678.9,3]",
        "[min,max]",
        "[2,123456789012345679]\n",
    ),
    (
        "[99999999999999999,99999999999999998.9,99999999999999999.0]",
        "min",
        "99999999999999998.9\n",
    ),
    (
        "[99999999999999999,99999999999999998.9,99999999999999999.0]",
        "max",
        "99999999999999999.0\n",
    ),
    (
        "[99999999999999999,99999999999999998.9,99999999999999999.0]",
        "[min,max]",
        "[99999999999999998.9,99999999999999999.0]\n",
    ),
    ("[1,1.0,2,2.0,2e0,1]", "min", "1\n"),
    ("[1,1.0,2,2.0,2e0,1]", "max", "2\n"),
    ("[1,1.0,2,2.0,2e0,1]", "[min,max]", "[1,2]\n"),
    ("[1e17,100000000000000000,1.0e17]", "min", "1E+17\n"),
    ("[1e17,100000000000000000,1.0e17]", "max", "1.0E+17\n"),
    (
        "[1e17,100000000000000000,1.0e17]",
        "[min,max]",
        "[1E+17,1.0E+17]\n",
    ),
    ("[null,1,\"a\",2.5,[],{}]", "min", "null\n"),
    ("[null,1,\"a\",2.5,[],{}]", "max", "{}\n"),
    ("[null,1,\"a\",2.5,[],{}]", "[min,max]", "[null,{}]\n"),
    ("[nan,5,-1]", "min", "null\n"),
    ("[nan,5,-1]", "max", "5\n"),
    ("[nan,5,-1]", "[min,max]", "[null,5]\n"),
    ("[3,nan]", "min", "null\n"),
    ("[3,nan]", "max", "3\n"),
    ("[3,nan]", "[min,max]", "[null,3]\n"),
];

#[test]
fn test_min_max_walk_compares_literals_as_jq_7261() -> Result<()> {
    for (document, filter, stdout) in ROWS {
        let (out, err, code) = run_jq_full(&["-c", filter], Some(document))?;
        assert_eq!(
            (out.as_str(), err.as_str(), code),
            (*stdout, "", 0),
            "{filter} over {document:?}"
        );
    }
    Ok(())
}
