//! #6891: the single `key: value` pair of a flow sequence (`[cron: "40 1 * * *"]`) is a mapping of its
//! own that begins at its key, and yq reports its `style` as `"flow"`, as it does `{k: v}`. A pair's
//! value, a scalar element, a block sequence item and a block mapping keep their own style. Every row
//! is captured from yq v4.53.3: the source document, the filter and the whole output (`-o=json -I0`).

use anyhow::Result;

use crate::run_yq_stdin;

const ROWS: &[(&str, &str, &str)] = &[
    ("a: [k: v]\n", ".a[0] | style", "\"flow\"\n"),
    ("a: [k: v, j: w]\n", "[.a[] | style]", "[\"flow\",\"flow\"]\n"),
    ("a: [x, k: v]\n", "[.a[] | style]", "[\"\",\"flow\"]\n"),
    ("a: [k: v]\n", "[.. | style]", "[\"\",\"flow\",\"flow\",\"\"]\n"),
    ("a: {k: [j: v]}\n", "[.. | style]", "[\"\",\"flow\",\"flow\",\"flow\",\"\"]\n"),
    ("a: [? k : v]\n", ".a[0] | style", "\"flow\"\n"),
    ("a: [{k: v}]\n", ".a[0] | style", "\"flow\"\n"),
    ("a: [[k: v]]\n", "[.. | style]", "[\"\",\"flow\",\"flow\",\"flow\",\"\"]\n"),
    ("a: [k: v]\n", ".a[0].k | style", "\"\"\n"),
    ("a: [x, y]\n", "[.a[] | style]", "[\"\",\"\"]\n"),
    ("a: [&x k: v]\n", "[.. | style]", "[\"\",\"flow\",\"flow\",\"\"]\n"),
    ("a: [!foo k: v]\n", "[.. | style]", "[\"\",\"flow\",\"flow\",\"\"]\n"),
    ("a: [k: [1, 2]]\n", "[.. | style]", "[\"\",\"flow\",\"flow\",\"flow\",\"\",\"\"]\n"),
    ("a: [k: {j: 1}]\n", "[.. | style]", "[\"\",\"flow\",\"flow\",\"flow\",\"\"]\n"),
    ("a:\n  - k: v\n", "[.. | style]", "[\"\",\"\",\"\",\"\"]\n"),
    ("a:\n  - [k: v]\n", "[.. | style]", "[\"\",\"\",\"flow\",\"flow\",\"\"]\n"),
    ("a: &y [k: v]\n", "[.. | style]", "[\"\",\"flow\",\"flow\",\"\"]\n"),
    ("on:\n  schedule: [cron: \"40 1 * * *\"]\n  push: {branches: [main]}\njobs:\n  b:\n    strategy: {matrix: {rust: [nightly, beta]}}\n", "[.. | select(style == \"flow\")] | length", "7\n"),
    ("schedule: [cron: \"40 1 * * *\"]\n", "[.. | select(style == \"flow\")]", "[[{\"cron\":\"40 1 * * *\"}],{\"cron\":\"40 1 * * *\"}]\n"),
    ("a: [k: v]\n", ".a[0]", "{\"k\":\"v\"}\n"),
    ("a: [k: v]\n", ".a", "[{\"k\":\"v\"}]\n"),
];

#[test]
fn the_pair_of_a_flow_sequence_has_flow_style_6891() -> Result<()> {
    for (input, filter, want) in ROWS {
        let (out, code) = run_yq_stdin(filter, input, &["-o=json", "-I0"])?;
        assert_eq!(code, 0, "{filter} over {input:?}");
        assert_eq!(out, *want, "{filter} over {input:?}");
    }
    Ok(())
}
