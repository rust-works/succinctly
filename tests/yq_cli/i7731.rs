//! #7731: jq mode steps an all-integer `range` past 2^53 in doubles, as jq does; yq mode keeps
//! the exact `int64` walk. `range` is a jq builtin `succinctly yq` runs only under
//! `--jq-extensions` (yq v4.53.3's lexer rejects it), and yq's integers are exact `int64`, so
//! these rows are succinctly's own answers, unchanged by #7731.

use anyhow::Result;

use crate::run_yq_stdin;

const ROWS: &[(&str, &str)] = &[
    (
        "[range(9007199254740990; 9007199254740994)]",
        "[9007199254740990,9007199254740991,9007199254740992,9007199254740993]\n",
    ),
    (
        "[range(-9007199254740994; -9007199254740990)]",
        "[-9007199254740994,-9007199254740993,-9007199254740992,-9007199254740991]\n",
    ),
    (
        "[range(9223372036854775806; 9223372036854775807)]",
        "[9223372036854775806]\n",
    ),
    (
        "[range(1700000000000000000; 1700000000000000003; 1)]",
        "[1700000000000000000,1700000000000000001,1700000000000000002]\n",
    ),
];

#[test]
fn test_range_past_2_53_keeps_the_int64_walk_in_yq_mode_7731() -> Result<()> {
    for &(filter, want) in ROWS {
        let (out, code) =
            run_yq_stdin(filter, "", &["-n", "--jq-extensions", "-o", "json", "-I0"])?;
        assert_eq!((out.as_str(), code), (want, 0), "{filter}");
    }
    Ok(())
}
