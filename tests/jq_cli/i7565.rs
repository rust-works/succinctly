//! #7565: in `map(f) | map(g)`, `succinctly jq` ran the two stages element by element, so `g`'s error,
//! `debug` line or `input` could come before `f` had run on a later element, where jq's `map(f)` is
//! `[.[] | f]` and finishes (or fails) on every element before `map(g)` starts.
//!
//! The lazy `map` chain of `eval_generic.rs` now fuses a `map(g)` onto the stages already on it only
//! when `g`, or every stage already there, is inert (`eval::expr_is_inert`: it can neither raise nor
//! have an effect); otherwise it drains the chain first, as jq builds the array.
//!
//! Every row's standard output and standard error, interleaved on one file, and its exit status were
//! captured from `/usr/bin/jq` 1.7.1; `/opt/homebrew/bin/jq` 1.8.2 answers every row identically. The
//! matrix rows are one per pipeline shape of a generated differential (two- and three-stage pipelines
//! over `map`, `[.[] | f]`, `map_values` and `map(select(..))` producers with `debug`, `error`,
//! `input` and `halt_error` bodies) that differed before the fix, plus controls that already agreed.

use anyhow::Result;

use crate::run_jq_interleaved;

/// The matrix's input: the array the stages map, then three documents for `input` to consume.
const INPUT: &str = "[1,\"x\",3]\n10\n20\n30\n";

/// `(filter, interleaved output, exit status)`, run as `jq -c FILTER` over [`INPUT`].
const MATRIX: &[(&str, &str, i32)] = &[
    (
        r#"map(debug("f")) | map(error("g"))"#,
        r#"["DEBUG:","f"]
["DEBUG:","f"]
["DEBUG:","f"]
jq: error (at <stdin>:1): g
jq: error (at <stdin>:2): Cannot iterate over number (10)
jq: error (at <stdin>:3): Cannot iterate over number (20)
jq: error (at <stdin>:4): Cannot iterate over number (30)
"#,
        5,
    ),
    (
        r#"map(debug("f")) | map(if . == 1 then error("g") else . end)"#,
        r#"["DEBUG:","f"]
["DEBUG:","f"]
["DEBUG:","f"]
jq: error (at <stdin>:1): g
jq: error (at <stdin>:2): Cannot iterate over number (10)
jq: error (at <stdin>:3): Cannot iterate over number (20)
jq: error (at <stdin>:4): Cannot iterate over number (30)
"#,
        5,
    ),
    (
        r#"map(debug("f")) | map(error("g")) | length"#,
        r#"["DEBUG:","f"]
["DEBUG:","f"]
["DEBUG:","f"]
jq: error (at <stdin>:1): g
jq: error (at <stdin>:2): Cannot iterate over number (10)
jq: error (at <stdin>:3): Cannot iterate over number (20)
jq: error (at <stdin>:4): Cannot iterate over number (30)
"#,
        5,
    ),
    (
        r#"map(debug("f")) | map(if . == 1 then error("g") else . end) | length"#,
        r#"["DEBUG:","f"]
["DEBUG:","f"]
["DEBUG:","f"]
jq: error (at <stdin>:1): g
jq: error (at <stdin>:2): Cannot iterate over number (10)
jq: error (at <stdin>:3): Cannot iterate over number (20)
jq: error (at <stdin>:4): Cannot iterate over number (30)
"#,
        5,
    ),
    (
        r#"map(debug("f")) | map(error("g")) | map(.)"#,
        r#"["DEBUG:","f"]
["DEBUG:","f"]
["DEBUG:","f"]
jq: error (at <stdin>:1): g
jq: error (at <stdin>:2): Cannot iterate over number (10)
jq: error (at <stdin>:3): Cannot iterate over number (20)
jq: error (at <stdin>:4): Cannot iterate over number (30)
"#,
        5,
    ),
    (
        r#"map(debug("f")) | map(if . == 1 then error("g") else . end) | map(.)"#,
        r#"["DEBUG:","f"]
["DEBUG:","f"]
["DEBUG:","f"]
jq: error (at <stdin>:1): g
jq: error (at <stdin>:2): Cannot iterate over number (10)
jq: error (at <stdin>:3): Cannot iterate over number (20)
jq: error (at <stdin>:4): Cannot iterate over number (30)
"#,
        5,
    ),
    (
        r#"map(debug("f")) | map(error("g")) | first"#,
        r#"["DEBUG:","f"]
["DEBUG:","f"]
["DEBUG:","f"]
jq: error (at <stdin>:1): g
jq: error (at <stdin>:2): Cannot iterate over number (10)
jq: error (at <stdin>:3): Cannot iterate over number (20)
jq: error (at <stdin>:4): Cannot iterate over number (30)
"#,
        5,
    ),
    (
        r#"map(debug("f")) | map(if . == 1 then error("g") else . end) | first"#,
        r#"["DEBUG:","f"]
["DEBUG:","f"]
["DEBUG:","f"]
jq: error (at <stdin>:1): g
jq: error (at <stdin>:2): Cannot iterate over number (10)
jq: error (at <stdin>:3): Cannot iterate over number (20)
jq: error (at <stdin>:4): Cannot iterate over number (30)
"#,
        5,
    ),
    (
        r#"map(debug("f")) | map(error("g")) | .[0]"#,
        r#"["DEBUG:","f"]
["DEBUG:","f"]
["DEBUG:","f"]
jq: error (at <stdin>:1): g
jq: error (at <stdin>:2): Cannot iterate over number (10)
jq: error (at <stdin>:3): Cannot iterate over number (20)
jq: error (at <stdin>:4): Cannot iterate over number (30)
"#,
        5,
    ),
    (
        r#"map(debug("f")) | map(if . == 1 then error("g") else . end) | .[0]"#,
        r#"["DEBUG:","f"]
["DEBUG:","f"]
["DEBUG:","f"]
jq: error (at <stdin>:1): g
jq: error (at <stdin>:2): Cannot iterate over number (10)
jq: error (at <stdin>:3): Cannot iterate over number (20)
jq: error (at <stdin>:4): Cannot iterate over number (30)
"#,
        5,
    ),
    (
        r#"map(debug("f")) | map(error("g")) | last"#,
        r#"["DEBUG:","f"]
["DEBUG:","f"]
["DEBUG:","f"]
jq: error (at <stdin>:1): g
jq: error (at <stdin>:2): Cannot iterate over number (10)
jq: error (at <stdin>:3): Cannot iterate over number (20)
jq: error (at <stdin>:4): Cannot iterate over number (30)
"#,
        5,
    ),
    (
        r#"map(debug("f")) | map(if . == 1 then error("g") else . end) | last"#,
        r#"["DEBUG:","f"]
["DEBUG:","f"]
["DEBUG:","f"]
jq: error (at <stdin>:1): g
jq: error (at <stdin>:2): Cannot iterate over number (10)
jq: error (at <stdin>:3): Cannot iterate over number (20)
jq: error (at <stdin>:4): Cannot iterate over number (30)
"#,
        5,
    ),
    (
        r#"map(debug("f")) | map(error("g")) | add"#,
        r#"["DEBUG:","f"]
["DEBUG:","f"]
["DEBUG:","f"]
jq: error (at <stdin>:1): g
jq: error (at <stdin>:2): Cannot iterate over number (10)
jq: error (at <stdin>:3): Cannot iterate over number (20)
jq: error (at <stdin>:4): Cannot iterate over number (30)
"#,
        5,
    ),
    (
        r#"map(debug("f")) | map(if . == 1 then error("g") else . end) | add"#,
        r#"["DEBUG:","f"]
["DEBUG:","f"]
["DEBUG:","f"]
jq: error (at <stdin>:1): g
jq: error (at <stdin>:2): Cannot iterate over number (10)
jq: error (at <stdin>:3): Cannot iterate over number (20)
jq: error (at <stdin>:4): Cannot iterate over number (30)
"#,
        5,
    ),
    (
        r#"map(debug("f")) | map(error("g")) | tojson"#,
        r#"["DEBUG:","f"]
["DEBUG:","f"]
["DEBUG:","f"]
jq: error (at <stdin>:1): g
jq: error (at <stdin>:2): Cannot iterate over number (10)
jq: error (at <stdin>:3): Cannot iterate over number (20)
jq: error (at <stdin>:4): Cannot iterate over number (30)
"#,
        5,
    ),
    (
        r#"map(debug("f")) | map(if . == 1 then error("g") else . end) | tojson"#,
        r#"["DEBUG:","f"]
["DEBUG:","f"]
["DEBUG:","f"]
jq: error (at <stdin>:1): g
jq: error (at <stdin>:2): Cannot iterate over number (10)
jq: error (at <stdin>:3): Cannot iterate over number (20)
jq: error (at <stdin>:4): Cannot iterate over number (30)
"#,
        5,
    ),
    (
        r#"map(debug("f")) | map(error("g")) | .[]"#,
        r#"["DEBUG:","f"]
["DEBUG:","f"]
["DEBUG:","f"]
jq: error (at <stdin>:1): g
jq: error (at <stdin>:2): Cannot iterate over number (10)
jq: error (at <stdin>:3): Cannot iterate over number (20)
jq: error (at <stdin>:4): Cannot iterate over number (30)
"#,
        5,
    ),
    (
        r#"map(debug("f")) | map(if . == 1 then error("g") else . end) | .[]"#,
        r#"["DEBUG:","f"]
["DEBUG:","f"]
["DEBUG:","f"]
jq: error (at <stdin>:1): g
jq: error (at <stdin>:2): Cannot iterate over number (10)
jq: error (at <stdin>:3): Cannot iterate over number (20)
jq: error (at <stdin>:4): Cannot iterate over number (30)
"#,
        5,
    ),
    (
        r#"map(debug("f")) | map(select(error("g")|true))"#,
        r#"["DEBUG:","f"]
["DEBUG:","f"]
["DEBUG:","f"]
jq: error (at <stdin>:1): g
jq: error (at <stdin>:2): Cannot iterate over number (10)
jq: error (at <stdin>:3): Cannot iterate over number (20)
jq: error (at <stdin>:4): Cannot iterate over number (30)
"#,
        5,
    ),
    (
        r#"map(debug("f")) | map(select(if . == 1 then error("g") else . end|true))"#,
        r#"["DEBUG:","f"]
["DEBUG:","f"]
["DEBUG:","f"]
jq: error (at <stdin>:1): g
jq: error (at <stdin>:2): Cannot iterate over number (10)
jq: error (at <stdin>:3): Cannot iterate over number (20)
jq: error (at <stdin>:4): Cannot iterate over number (30)
"#,
        5,
    ),
    (
        r#"map(debug("f")) | map(.) | map(error("g"))"#,
        r#"["DEBUG:","f"]
["DEBUG:","f"]
["DEBUG:","f"]
jq: error (at <stdin>:1): g
jq: error (at <stdin>:2): Cannot iterate over number (10)
jq: error (at <stdin>:3): Cannot iterate over number (20)
jq: error (at <stdin>:4): Cannot iterate over number (30)
"#,
        5,
    ),
    (
        r#"map(debug("f")) | map(.) | map(if . == 1 then error("g") else . end)"#,
        r#"["DEBUG:","f"]
["DEBUG:","f"]
["DEBUG:","f"]
jq: error (at <stdin>:1): g
jq: error (at <stdin>:2): Cannot iterate over number (10)
jq: error (at <stdin>:3): Cannot iterate over number (20)
jq: error (at <stdin>:4): Cannot iterate over number (30)
"#,
        5,
    ),
    (
        r#"map(debug("f")) | map(.) | map(error("g")) | length"#,
        r#"["DEBUG:","f"]
["DEBUG:","f"]
["DEBUG:","f"]
jq: error (at <stdin>:1): g
jq: error (at <stdin>:2): Cannot iterate over number (10)
jq: error (at <stdin>:3): Cannot iterate over number (20)
jq: error (at <stdin>:4): Cannot iterate over number (30)
"#,
        5,
    ),
    (
        r#"map(debug("f")) | map(.) | map(if . == 1 then error("g") else . end) | length"#,
        r#"["DEBUG:","f"]
["DEBUG:","f"]
["DEBUG:","f"]
jq: error (at <stdin>:1): g
jq: error (at <stdin>:2): Cannot iterate over number (10)
jq: error (at <stdin>:3): Cannot iterate over number (20)
jq: error (at <stdin>:4): Cannot iterate over number (30)
"#,
        5,
    ),
    (
        r#"map(debug("f")) | map(.) | map(error("g")) | map(.)"#,
        r#"["DEBUG:","f"]
["DEBUG:","f"]
["DEBUG:","f"]
jq: error (at <stdin>:1): g
jq: error (at <stdin>:2): Cannot iterate over number (10)
jq: error (at <stdin>:3): Cannot iterate over number (20)
jq: error (at <stdin>:4): Cannot iterate over number (30)
"#,
        5,
    ),
    (
        r#"map(debug("f")) | map(.) | map(if . == 1 then error("g") else . end) | map(.)"#,
        r#"["DEBUG:","f"]
["DEBUG:","f"]
["DEBUG:","f"]
jq: error (at <stdin>:1): g
jq: error (at <stdin>:2): Cannot iterate over number (10)
jq: error (at <stdin>:3): Cannot iterate over number (20)
jq: error (at <stdin>:4): Cannot iterate over number (30)
"#,
        5,
    ),
    (
        r#"map(debug("f")) | map(.) | map(error("g")) | first"#,
        r#"["DEBUG:","f"]
["DEBUG:","f"]
["DEBUG:","f"]
jq: error (at <stdin>:1): g
jq: error (at <stdin>:2): Cannot iterate over number (10)
jq: error (at <stdin>:3): Cannot iterate over number (20)
jq: error (at <stdin>:4): Cannot iterate over number (30)
"#,
        5,
    ),
    (
        r#"map(debug("f")) | map(.) | map(if . == 1 then error("g") else . end) | first"#,
        r#"["DEBUG:","f"]
["DEBUG:","f"]
["DEBUG:","f"]
jq: error (at <stdin>:1): g
jq: error (at <stdin>:2): Cannot iterate over number (10)
jq: error (at <stdin>:3): Cannot iterate over number (20)
jq: error (at <stdin>:4): Cannot iterate over number (30)
"#,
        5,
    ),
    (
        r#"map(debug("f")) | map(.) | map(error("g")) | .[0]"#,
        r#"["DEBUG:","f"]
["DEBUG:","f"]
["DEBUG:","f"]
jq: error (at <stdin>:1): g
jq: error (at <stdin>:2): Cannot iterate over number (10)
jq: error (at <stdin>:3): Cannot iterate over number (20)
jq: error (at <stdin>:4): Cannot iterate over number (30)
"#,
        5,
    ),
    (
        r#"map(debug("f")) | map(.) | map(if . == 1 then error("g") else . end) | .[0]"#,
        r#"["DEBUG:","f"]
["DEBUG:","f"]
["DEBUG:","f"]
jq: error (at <stdin>:1): g
jq: error (at <stdin>:2): Cannot iterate over number (10)
jq: error (at <stdin>:3): Cannot iterate over number (20)
jq: error (at <stdin>:4): Cannot iterate over number (30)
"#,
        5,
    ),
    (
        r#"map(debug("f")) | map(.) | map(error("g")) | last"#,
        r#"["DEBUG:","f"]
["DEBUG:","f"]
["DEBUG:","f"]
jq: error (at <stdin>:1): g
jq: error (at <stdin>:2): Cannot iterate over number (10)
jq: error (at <stdin>:3): Cannot iterate over number (20)
jq: error (at <stdin>:4): Cannot iterate over number (30)
"#,
        5,
    ),
    (
        r#"map(debug("f")) | map(.) | map(if . == 1 then error("g") else . end) | last"#,
        r#"["DEBUG:","f"]
["DEBUG:","f"]
["DEBUG:","f"]
jq: error (at <stdin>:1): g
jq: error (at <stdin>:2): Cannot iterate over number (10)
jq: error (at <stdin>:3): Cannot iterate over number (20)
jq: error (at <stdin>:4): Cannot iterate over number (30)
"#,
        5,
    ),
    (
        r#"map(debug("f")) | map(.) | map(error("g")) | add"#,
        r#"["DEBUG:","f"]
["DEBUG:","f"]
["DEBUG:","f"]
jq: error (at <stdin>:1): g
jq: error (at <stdin>:2): Cannot iterate over number (10)
jq: error (at <stdin>:3): Cannot iterate over number (20)
jq: error (at <stdin>:4): Cannot iterate over number (30)
"#,
        5,
    ),
    (
        r#"map(debug("f")) | map(.) | map(if . == 1 then error("g") else . end) | add"#,
        r#"["DEBUG:","f"]
["DEBUG:","f"]
["DEBUG:","f"]
jq: error (at <stdin>:1): g
jq: error (at <stdin>:2): Cannot iterate over number (10)
jq: error (at <stdin>:3): Cannot iterate over number (20)
jq: error (at <stdin>:4): Cannot iterate over number (30)
"#,
        5,
    ),
    (
        r#"map(debug("f")) | map(.) | map(error("g")) | tojson"#,
        r#"["DEBUG:","f"]
["DEBUG:","f"]
["DEBUG:","f"]
jq: error (at <stdin>:1): g
jq: error (at <stdin>:2): Cannot iterate over number (10)
jq: error (at <stdin>:3): Cannot iterate over number (20)
jq: error (at <stdin>:4): Cannot iterate over number (30)
"#,
        5,
    ),
    (
        r#"map(debug("f")) | map(.) | map(if . == 1 then error("g") else . end) | tojson"#,
        r#"["DEBUG:","f"]
["DEBUG:","f"]
["DEBUG:","f"]
jq: error (at <stdin>:1): g
jq: error (at <stdin>:2): Cannot iterate over number (10)
jq: error (at <stdin>:3): Cannot iterate over number (20)
jq: error (at <stdin>:4): Cannot iterate over number (30)
"#,
        5,
    ),
    (
        r#"map(debug("f")) | map(.) | map(error("g")) | .[]"#,
        r#"["DEBUG:","f"]
["DEBUG:","f"]
["DEBUG:","f"]
jq: error (at <stdin>:1): g
jq: error (at <stdin>:2): Cannot iterate over number (10)
jq: error (at <stdin>:3): Cannot iterate over number (20)
jq: error (at <stdin>:4): Cannot iterate over number (30)
"#,
        5,
    ),
    (
        r#"map(debug("f")) | map(.) | map(if . == 1 then error("g") else . end) | .[]"#,
        r#"["DEBUG:","f"]
["DEBUG:","f"]
["DEBUG:","f"]
jq: error (at <stdin>:1): g
jq: error (at <stdin>:2): Cannot iterate over number (10)
jq: error (at <stdin>:3): Cannot iterate over number (20)
jq: error (at <stdin>:4): Cannot iterate over number (30)
"#,
        5,
    ),
    (
        r#"map(debug("f")) | map(.) | map(select(error("g")|true))"#,
        r#"["DEBUG:","f"]
["DEBUG:","f"]
["DEBUG:","f"]
jq: error (at <stdin>:1): g
jq: error (at <stdin>:2): Cannot iterate over number (10)
jq: error (at <stdin>:3): Cannot iterate over number (20)
jq: error (at <stdin>:4): Cannot iterate over number (30)
"#,
        5,
    ),
    (
        r#"map(debug("f")) | map(.) | map(select(if . == 1 then error("g") else . end|true))"#,
        r#"["DEBUG:","f"]
["DEBUG:","f"]
["DEBUG:","f"]
jq: error (at <stdin>:1): g
jq: error (at <stdin>:2): Cannot iterate over number (10)
jq: error (at <stdin>:3): Cannot iterate over number (20)
jq: error (at <stdin>:4): Cannot iterate over number (30)
"#,
        5,
    ),
    (
        r#"map(select(debug("f")|true)) | map(error("g"))"#,
        r#"["DEBUG:","f"]
["DEBUG:","f"]
["DEBUG:","f"]
jq: error (at <stdin>:1): g
jq: error (at <stdin>:2): Cannot iterate over number (10)
jq: error (at <stdin>:3): Cannot iterate over number (20)
jq: error (at <stdin>:4): Cannot iterate over number (30)
"#,
        5,
    ),
    (
        r#"map(select(debug("f")|true)) | map(if . == 1 then error("g") else . end)"#,
        r#"["DEBUG:","f"]
["DEBUG:","f"]
["DEBUG:","f"]
jq: error (at <stdin>:1): g
jq: error (at <stdin>:2): Cannot iterate over number (10)
jq: error (at <stdin>:3): Cannot iterate over number (20)
jq: error (at <stdin>:4): Cannot iterate over number (30)
"#,
        5,
    ),
    (
        r#"map(select(debug("f")|true)) | map(error("g")) | length"#,
        r#"["DEBUG:","f"]
["DEBUG:","f"]
["DEBUG:","f"]
jq: error (at <stdin>:1): g
jq: error (at <stdin>:2): Cannot iterate over number (10)
jq: error (at <stdin>:3): Cannot iterate over number (20)
jq: error (at <stdin>:4): Cannot iterate over number (30)
"#,
        5,
    ),
    (
        r#"map(select(debug("f")|true)) | map(if . == 1 then error("g") else . end) | length"#,
        r#"["DEBUG:","f"]
["DEBUG:","f"]
["DEBUG:","f"]
jq: error (at <stdin>:1): g
jq: error (at <stdin>:2): Cannot iterate over number (10)
jq: error (at <stdin>:3): Cannot iterate over number (20)
jq: error (at <stdin>:4): Cannot iterate over number (30)
"#,
        5,
    ),
    (
        r#"map(select(debug("f")|true)) | map(error("g")) | map(.)"#,
        r#"["DEBUG:","f"]
["DEBUG:","f"]
["DEBUG:","f"]
jq: error (at <stdin>:1): g
jq: error (at <stdin>:2): Cannot iterate over number (10)
jq: error (at <stdin>:3): Cannot iterate over number (20)
jq: error (at <stdin>:4): Cannot iterate over number (30)
"#,
        5,
    ),
    (
        r#"map(select(debug("f")|true)) | map(if . == 1 then error("g") else . end) | map(.)"#,
        r#"["DEBUG:","f"]
["DEBUG:","f"]
["DEBUG:","f"]
jq: error (at <stdin>:1): g
jq: error (at <stdin>:2): Cannot iterate over number (10)
jq: error (at <stdin>:3): Cannot iterate over number (20)
jq: error (at <stdin>:4): Cannot iterate over number (30)
"#,
        5,
    ),
    (
        r#"map(select(debug("f")|true)) | map(error("g")) | first"#,
        r#"["DEBUG:","f"]
["DEBUG:","f"]
["DEBUG:","f"]
jq: error (at <stdin>:1): g
jq: error (at <stdin>:2): Cannot iterate over number (10)
jq: error (at <stdin>:3): Cannot iterate over number (20)
jq: error (at <stdin>:4): Cannot iterate over number (30)
"#,
        5,
    ),
    (
        r#"map(select(debug("f")|true)) | map(if . == 1 then error("g") else . end) | first"#,
        r#"["DEBUG:","f"]
["DEBUG:","f"]
["DEBUG:","f"]
jq: error (at <stdin>:1): g
jq: error (at <stdin>:2): Cannot iterate over number (10)
jq: error (at <stdin>:3): Cannot iterate over number (20)
jq: error (at <stdin>:4): Cannot iterate over number (30)
"#,
        5,
    ),
    (
        r#"map(select(debug("f")|true)) | map(error("g")) | .[0]"#,
        r#"["DEBUG:","f"]
["DEBUG:","f"]
["DEBUG:","f"]
jq: error (at <stdin>:1): g
jq: error (at <stdin>:2): Cannot iterate over number (10)
jq: error (at <stdin>:3): Cannot iterate over number (20)
jq: error (at <stdin>:4): Cannot iterate over number (30)
"#,
        5,
    ),
    (
        r#"map(select(debug("f")|true)) | map(if . == 1 then error("g") else . end) | .[0]"#,
        r#"["DEBUG:","f"]
["DEBUG:","f"]
["DEBUG:","f"]
jq: error (at <stdin>:1): g
jq: error (at <stdin>:2): Cannot iterate over number (10)
jq: error (at <stdin>:3): Cannot iterate over number (20)
jq: error (at <stdin>:4): Cannot iterate over number (30)
"#,
        5,
    ),
    (
        r#"map(select(debug("f")|true)) | map(error("g")) | last"#,
        r#"["DEBUG:","f"]
["DEBUG:","f"]
["DEBUG:","f"]
jq: error (at <stdin>:1): g
jq: error (at <stdin>:2): Cannot iterate over number (10)
jq: error (at <stdin>:3): Cannot iterate over number (20)
jq: error (at <stdin>:4): Cannot iterate over number (30)
"#,
        5,
    ),
    (
        r#"map(select(debug("f")|true)) | map(if . == 1 then error("g") else . end) | last"#,
        r#"["DEBUG:","f"]
["DEBUG:","f"]
["DEBUG:","f"]
jq: error (at <stdin>:1): g
jq: error (at <stdin>:2): Cannot iterate over number (10)
jq: error (at <stdin>:3): Cannot iterate over number (20)
jq: error (at <stdin>:4): Cannot iterate over number (30)
"#,
        5,
    ),
    (
        r#"map(select(debug("f")|true)) | map(error("g")) | add"#,
        r#"["DEBUG:","f"]
["DEBUG:","f"]
["DEBUG:","f"]
jq: error (at <stdin>:1): g
jq: error (at <stdin>:2): Cannot iterate over number (10)
jq: error (at <stdin>:3): Cannot iterate over number (20)
jq: error (at <stdin>:4): Cannot iterate over number (30)
"#,
        5,
    ),
    (
        r#"map(select(debug("f")|true)) | map(if . == 1 then error("g") else . end) | add"#,
        r#"["DEBUG:","f"]
["DEBUG:","f"]
["DEBUG:","f"]
jq: error (at <stdin>:1): g
jq: error (at <stdin>:2): Cannot iterate over number (10)
jq: error (at <stdin>:3): Cannot iterate over number (20)
jq: error (at <stdin>:4): Cannot iterate over number (30)
"#,
        5,
    ),
    (
        r#"map(select(debug("f")|true)) | map(error("g")) | tojson"#,
        r#"["DEBUG:","f"]
["DEBUG:","f"]
["DEBUG:","f"]
jq: error (at <stdin>:1): g
jq: error (at <stdin>:2): Cannot iterate over number (10)
jq: error (at <stdin>:3): Cannot iterate over number (20)
jq: error (at <stdin>:4): Cannot iterate over number (30)
"#,
        5,
    ),
    (
        r#"map(select(debug("f")|true)) | map(if . == 1 then error("g") else . end) | tojson"#,
        r#"["DEBUG:","f"]
["DEBUG:","f"]
["DEBUG:","f"]
jq: error (at <stdin>:1): g
jq: error (at <stdin>:2): Cannot iterate over number (10)
jq: error (at <stdin>:3): Cannot iterate over number (20)
jq: error (at <stdin>:4): Cannot iterate over number (30)
"#,
        5,
    ),
    (
        r#"map(select(debug("f")|true)) | map(error("g")) | .[]"#,
        r#"["DEBUG:","f"]
["DEBUG:","f"]
["DEBUG:","f"]
jq: error (at <stdin>:1): g
jq: error (at <stdin>:2): Cannot iterate over number (10)
jq: error (at <stdin>:3): Cannot iterate over number (20)
jq: error (at <stdin>:4): Cannot iterate over number (30)
"#,
        5,
    ),
    (
        r#"map(select(debug("f")|true)) | map(if . == 1 then error("g") else . end) | .[]"#,
        r#"["DEBUG:","f"]
["DEBUG:","f"]
["DEBUG:","f"]
jq: error (at <stdin>:1): g
jq: error (at <stdin>:2): Cannot iterate over number (10)
jq: error (at <stdin>:3): Cannot iterate over number (20)
jq: error (at <stdin>:4): Cannot iterate over number (30)
"#,
        5,
    ),
    (
        r#"map(select(debug("f")|true)) | map(select(error("g")|true))"#,
        r#"["DEBUG:","f"]
["DEBUG:","f"]
["DEBUG:","f"]
jq: error (at <stdin>:1): g
jq: error (at <stdin>:2): Cannot iterate over number (10)
jq: error (at <stdin>:3): Cannot iterate over number (20)
jq: error (at <stdin>:4): Cannot iterate over number (30)
"#,
        5,
    ),
    (
        r#"map(select(debug("f")|true)) | map(select(if . == 1 then error("g") else . end|true))"#,
        r#"["DEBUG:","f"]
["DEBUG:","f"]
["DEBUG:","f"]
jq: error (at <stdin>:1): g
jq: error (at <stdin>:2): Cannot iterate over number (10)
jq: error (at <stdin>:3): Cannot iterate over number (20)
jq: error (at <stdin>:4): Cannot iterate over number (30)
"#,
        5,
    ),
    (
        r#"map(debug("f")) | map(debug("g"))"#,
        r#"["DEBUG:","f"]
["DEBUG:","f"]
["DEBUG:","f"]
["DEBUG:","g"]
["DEBUG:","g"]
["DEBUG:","g"]
jq: error (at <stdin>:2): Cannot iterate over number (10)
jq: error (at <stdin>:3): Cannot iterate over number (20)
jq: error (at <stdin>:4): Cannot iterate over number (30)
[1,"x",3]
"#,
        5,
    ),
    (
        r#"map(debug("f")) | map(debug("g") | error("g"))"#,
        r#"["DEBUG:","f"]
["DEBUG:","f"]
["DEBUG:","f"]
["DEBUG:","g"]
jq: error (at <stdin>:1): g
jq: error (at <stdin>:2): Cannot iterate over number (10)
jq: error (at <stdin>:3): Cannot iterate over number (20)
jq: error (at <stdin>:4): Cannot iterate over number (30)
"#,
        5,
    ),
    (
        r#"map(if . == "x" then error("f") else . end) | map(error("g"))"#,
        r"jq: error (at <stdin>:1): f
jq: error (at <stdin>:2): Cannot iterate over number (10)
jq: error (at <stdin>:3): Cannot iterate over number (20)
jq: error (at <stdin>:4): Cannot iterate over number (30)
",
        5,
    ),
    (
        r#"map(if . == "x" then error("f") else . end) | map(debug("g"))"#,
        r"jq: error (at <stdin>:1): f
jq: error (at <stdin>:2): Cannot iterate over number (10)
jq: error (at <stdin>:3): Cannot iterate over number (20)
jq: error (at <stdin>:4): Cannot iterate over number (30)
",
        5,
    ),
    (
        r#"map(if . == "x" then error("f") else . end) | map(debug("g") | error("g"))"#,
        r"jq: error (at <stdin>:1): f
jq: error (at <stdin>:2): Cannot iterate over number (10)
jq: error (at <stdin>:3): Cannot iterate over number (20)
jq: error (at <stdin>:4): Cannot iterate over number (30)
",
        5,
    ),
    (
        r#"map(debug("f") | if . == "x" then error("f") else . end) | map(error("g"))"#,
        r#"["DEBUG:","f"]
["DEBUG:","f"]
jq: error (at <stdin>:1): f
jq: error (at <stdin>:2): Cannot iterate over number (10)
jq: error (at <stdin>:3): Cannot iterate over number (20)
jq: error (at <stdin>:4): Cannot iterate over number (30)
"#,
        5,
    ),
    (
        r#"map(debug("f") | if . == "x" then error("f") else . end) | map(debug("g"))"#,
        r#"["DEBUG:","f"]
["DEBUG:","f"]
jq: error (at <stdin>:1): f
jq: error (at <stdin>:2): Cannot iterate over number (10)
jq: error (at <stdin>:3): Cannot iterate over number (20)
jq: error (at <stdin>:4): Cannot iterate over number (30)
"#,
        5,
    ),
    (
        r#"map(debug("f") | if . == "x" then error("f") else . end) | map(debug("g") | error("g"))"#,
        r#"["DEBUG:","f"]
["DEBUG:","f"]
jq: error (at <stdin>:1): f
jq: error (at <stdin>:2): Cannot iterate over number (10)
jq: error (at <stdin>:3): Cannot iterate over number (20)
jq: error (at <stdin>:4): Cannot iterate over number (30)
"#,
        5,
    ),
    (
        r#"map(if . == "x" then "H\n" | halt_error(3) else . end) | map(error("g"))"#,
        r"H
",
        3,
    ),
    (
        r#"map(if . == "x" then "H\n" | halt_error(3) else . end) | map(debug("g"))"#,
        r"H
",
        3,
    ),
    (
        r#"map(if . == "x" then "H\n" | halt_error(3) else . end) | map(debug("g") | error("g"))"#,
        r"H
",
        3,
    ),
    (
        r#"[.[] | debug("f") | if . == "x" then error("f") else . end] | map(debug("g") | error("g"))"#,
        r#"["DEBUG:","f"]
["DEBUG:","f"]
jq: error (at <stdin>:1): f
jq: error (at <stdin>:2): Cannot iterate over number (10)
jq: error (at <stdin>:3): Cannot iterate over number (20)
jq: error (at <stdin>:4): Cannot iterate over number (30)
"#,
        5,
    ),
    (
        r#"map(debug("f") | if . == "x" then error("f") else . end) | sort_by(debug("g") | error("g"))"#,
        r#"["DEBUG:","f"]
["DEBUG:","f"]
jq: error (at <stdin>:1): f
jq: error (at <stdin>:2): Cannot iterate over number (10)
jq: error (at <stdin>:3): Cannot iterate over number (20)
jq: error (at <stdin>:4): Cannot iterate over number (30)
"#,
        5,
    ),
    (
        r#"map(debug("f")) | map(.) | map(debug("g"))"#,
        r#"["DEBUG:","f"]
["DEBUG:","f"]
["DEBUG:","f"]
["DEBUG:","g"]
["DEBUG:","g"]
["DEBUG:","g"]
jq: error (at <stdin>:2): Cannot iterate over number (10)
jq: error (at <stdin>:3): Cannot iterate over number (20)
jq: error (at <stdin>:4): Cannot iterate over number (30)
[1,"x",3]
"#,
        5,
    ),
    (
        r#"map(debug("f") | if . == "x" then error("f") else . end) | .[] | debug("g") | error("g")"#,
        r#"["DEBUG:","f"]
["DEBUG:","f"]
jq: error (at <stdin>:1): f
jq: error (at <stdin>:2): Cannot iterate over number (10)
jq: error (at <stdin>:3): Cannot iterate over number (20)
jq: error (at <stdin>:4): Cannot iterate over number (30)
"#,
        5,
    ),
    (
        r#"map(debug("f")) | map(.) | length"#,
        r#"["DEBUG:","f"]
["DEBUG:","f"]
["DEBUG:","f"]
jq: error (at <stdin>:2): Cannot iterate over number (10)
jq: error (at <stdin>:3): Cannot iterate over number (20)
jq: error (at <stdin>:4): Cannot iterate over number (30)
3
"#,
        5,
    ),
];

/// `(arguments, input, interleaved output, exit status)`: the issue's three reproductions, then the
/// same reordering reached through other routes (slurp, an object, a `def`, a computed array, nested
/// `map`, several documents) and an `input` in the first stage read by the second.
const ROUTES: &[(&[&str], &str, &str, i32)] = &[
    (
        &[
            r"-c",
            r#"try (map(if . == "x" then error("first") else . end) | map(error("second"))) catch ."#,
        ],
        r#"[1,"x"]
"#,
        r#""first"
"#,
        0,
    ),
    (
        &[
            r"-c",
            r#"map(debug("a") | if . == "x" then error("first") else . end) | map(debug("b") | error("second"))"#,
        ],
        r#"[1,"x"]
"#,
        r#"["DEBUG:","a"]
["DEBUG:","a"]
jq: error (at <stdin>:1): first
"#,
        5,
    ),
    (
        &[
            r"-c",
            r"try (map(select(has(0))) | map(select(contains({})))) catch .",
        ],
        r#"[[1,2],"x"]
"#,
        r#""Cannot check whether string has a number key"
"#,
        0,
    ),
    (
        &[r"-sc", r#"map(debug("a")) | map(debug("b")) | length"#],
        r"1 2
",
        r#"["DEBUG:","a"]
["DEBUG:","a"]
["DEBUG:","b"]
["DEBUG:","b"]
2
"#,
        0,
    ),
    (
        &[r"-c", r#".a | map(debug("a")) | map(debug("b")) | length"#],
        r#"{"a":[1,2]}
"#,
        r#"["DEBUG:","a"]
["DEBUG:","a"]
["DEBUG:","b"]
["DEBUG:","b"]
2
"#,
        0,
    ),
    (
        &[r"-c", r#"map(debug("a")) | map(debug("b")) | length"#],
        r#"{"a":1,"b":2}
"#,
        r#"["DEBUG:","a"]
["DEBUG:","a"]
["DEBUG:","b"]
["DEBUG:","b"]
2
"#,
        0,
    ),
    (
        &[
            r"-c",
            r#"def m: map(debug("a")); m | map(debug("b")) | length"#,
        ],
        r"[1,2]
",
        r#"["DEBUG:","a"]
["DEBUG:","a"]
["DEBUG:","b"]
["DEBUG:","b"]
2
"#,
        0,
    ),
    (
        &[
            r"-c",
            r#"[.[] | .] | map(debug("a")) | map(debug("b")) | length"#,
        ],
        r"[1,2]
",
        r#"["DEBUG:","a"]
["DEBUG:","a"]
["DEBUG:","b"]
["DEBUG:","b"]
2
"#,
        0,
    ),
    (
        &[
            r"-c",
            r#"keys | map(debug("a")) | map(debug("b")) | length"#,
        ],
        r"[1,2]
",
        r#"["DEBUG:","a"]
["DEBUG:","a"]
["DEBUG:","b"]
["DEBUG:","b"]
2
"#,
        0,
    ),
    (
        &[
            r"-c",
            r#"sort | map(debug("a")) | map(debug("b")) | length"#,
        ],
        r"[1,2]
",
        r#"["DEBUG:","a"]
["DEBUG:","a"]
["DEBUG:","b"]
["DEBUG:","b"]
2
"#,
        0,
    ),
    (
        &[r"-c", r#".[] | map(debug("a")) | map(debug("b")) | length"#],
        r"[[1,2]]
",
        r#"["DEBUG:","a"]
["DEBUG:","a"]
["DEBUG:","b"]
["DEBUG:","b"]
2
"#,
        0,
    ),
    (
        &[r"-c", r#"map(map(debug("a")) | map(debug("b")) | length)"#],
        r"[[1,2]]
",
        r#"["DEBUG:","a"]
["DEBUG:","a"]
["DEBUG:","b"]
["DEBUG:","b"]
[2]
"#,
        0,
    ),
    (
        &[r"-c", r#"[map(debug("a")) | map(debug("b")) | length]"#],
        r"[1,2]
",
        r#"["DEBUG:","a"]
["DEBUG:","a"]
["DEBUG:","b"]
["DEBUG:","b"]
[2]
"#,
        0,
    ),
    (
        &[r"-c", r#"(map(debug("a")) | map(debug("b"))) | length"#],
        r"[1,2]
",
        r#"["DEBUG:","a"]
["DEBUG:","a"]
["DEBUG:","b"]
["DEBUG:","b"]
2
"#,
        0,
    ),
    (
        &[r"-c", r#"map(debug("a")) | map(debug("b")) | length"#],
        r"[1,2]
[3,4]
",
        r#"["DEBUG:","a"]
["DEBUG:","a"]
["DEBUG:","b"]
["DEBUG:","b"]
["DEBUG:","a"]
["DEBUG:","a"]
["DEBUG:","b"]
["DEBUG:","b"]
2
2
"#,
        0,
    ),
    (
        &[
            r"-c",
            r"map(if . == 2 then input else . end) | map(input_line_number)",
        ],
        r"[1,2,3]
7
",
        r"[2,2,2]
",
        0,
    ),
];

#[test]
fn test_map_then_map_finishes_the_first_map_before_the_second_7565() -> Result<()> {
    let mut mismatches = Vec::new();
    for &(filter, expected, code) in MATRIX {
        let got = run_jq_interleaved(&["-c", filter], Some(INPUT))?;
        if got != (expected.to_string(), code) {
            mismatches.push(format!(
                "{filter}\n  want {:?}\n  got  {got:?}",
                (expected, code)
            ));
        }
    }
    for &(args, input, expected, code) in ROUTES {
        let got = run_jq_interleaved(args, Some(input))?;
        if got != (expected.to_string(), code) {
            mismatches.push(format!(
                "{args:?}\n  want {:?}\n  got  {got:?}",
                (expected, code)
            ));
        }
    }
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
    Ok(())
}

/// The truncating consumer after the last `map` still skips that stage's later elements (the
/// recorded `map(f) | first` divergence in `docs/compliance/jq/limitations.md`), but every earlier
/// stage now runs on every element first: jq prints `f` three times and `g` three times, and this
/// pins `f` three times and `g` once. Before #7565 it printed `f` and `g` once each.
#[test]
fn test_map_chain_truncating_consumer_skips_only_the_last_stage_7565() -> Result<()> {
    assert_eq!(
        run_jq_interleaved(
            &["-c", r#"map(debug("f")) | map(debug("g")) | first"#],
            Some("[1,2,3]\n")
        )?,
        (
            "[\"DEBUG:\",\"f\"]\n[\"DEBUG:\",\"f\"]\n[\"DEBUG:\",\"f\"]\n[\"DEBUG:\",\"g\"]\n1\n"
                .to_string(),
            0
        )
    );
    Ok(())
}
