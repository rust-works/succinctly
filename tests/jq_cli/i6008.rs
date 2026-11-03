//! #6008: jq orders a NaN before every number and before another NaN (`nan < nan` is true both
//! ways), and its sort and `min_by`/`max_by` loops settle those ties by input position: the sort
//! keeps NaN-keyed items in input order, `min`/`min_by` take the last of a run of NaNs and
//! `max`/`max_by` the first. The rows are `tests/data/jq-nan-ties-oracle-6008.tsv`: a program, its
//! stdout, stderr and exit status, captured from `/usr/bin/jq` 1.7.1 over 13 arrays (NaN first,
//! middle, last, alone, in nested arrays and in object fields) and 16 builtins. The stderr is
//! compared on its first line, as jq words it `(at <unknown>)` and so do we.

use anyhow::Result;

use crate::run_jq_full;

fn unescape(field: &str) -> String {
    let mut out = String::new();
    let mut chars = field.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('\\') => out.push('\\'),
            other => panic!("bad escape {other:?} in {field:?}"),
        }
    }
    out
}

#[test]
fn nan_ties_sort_and_pick_as_jq_does_6008() -> Result<()> {
    let table = include_str!("../data/jq-nan-ties-oracle-6008.tsv");
    let mut count = 0;
    for line in table.lines() {
        let [program, stdout, stderr, code]: [&str; 4] = line
            .split('\t')
            .collect::<Vec<_>>()
            .try_into()
            .unwrap_or_else(|_| panic!("malformed row {line:?}"));
        let program = unescape(program);
        let (got_out, got_err, got_code) = run_jq_full(&["-nc", &program], None)?;
        assert_eq!(got_out, unescape(stdout), "stdout of {program}");
        assert_eq!(
            got_err.lines().next(),
            unescape(stderr).lines().next(),
            "stderr of {program}"
        );
        assert_eq!(got_code, code.parse::<i32>()?, "status of {program}");
        count += 1;
    }
    assert_eq!(count, 208, "every row of the table was replayed");
    Ok(())
}

/// The sorts and the picks are one rule on a document too, where the elements come from an index
/// and not from a literal, and on arrays long enough that a merge sort takes its general path.
#[test]
fn nan_ties_keep_input_order_in_a_long_document_6008() -> Result<()> {
    // 2,000 records, every third with a NaN key: the NaN-keyed ones keep their order.
    let documents: String = (0..2000)
        .map(|n| {
            if n % 3 == 0 {
                format!("{{\"k\":nan,\"n\":{n}}}")
            } else {
                format!("{{\"k\":{n},\"n\":{n}}}")
            }
        })
        .collect::<Vec<_>>()
        .join(",");
    let input = format!("[{documents}]");
    let (stdout, stderr, code) = run_jq_full(
        &[
            "-c",
            "sort_by(.k) | map(select(.k == null) | .n) | . == sort",
        ],
        Some(&input),
    )?;
    assert_eq!((stdout.as_str(), stderr.as_str(), code), ("true\n", "", 0));
    let (stdout, _, _) = run_jq_full(&["-c", "min_by(.k).n, max_by(.k).n"], Some(&input))?;
    // The last NaN-keyed record is the minimum, the first the maximum of the NaNs; 1999 is the
    // largest number, and the maximum of everything.
    assert_eq!(stdout, "1998\n1999\n");
    Ok(())
}
