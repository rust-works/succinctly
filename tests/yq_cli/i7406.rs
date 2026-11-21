//! #7406: yq mode's `map_values` keeps its field list once (it was copied into a second list), and
//! its `to_entries` walks the fields without the key-hash census jq mode's route takes. These pin
//! what both answer over mappings that repeat a key, type one, end a flow mapping with a
//! comma, or are wider than the census's small-object and prefilter cut-offs. Each row is a live
//! yq v4.53.3 capture.

use anyhow::Result;

use crate::run_yq_stdin;

/// 300 fields, keys `k0` to `k249` and then `k0` to `k49` again.
fn wide() -> String {
    (0..300).map(|i| format!("k{}: {i}\n", i % 250)).collect()
}

#[test]
fn map_values_and_to_entries_keep_every_field_7406() -> Result<()> {
    let dup = "a: 1\nb: 2\na: 3\nc: 4\n";
    let flow = "{\"a\": 1, \"b\": 2, \"a\": 3,}\n";
    let rows: &[(&str, &str, &[&str], &str)] = &[
        (dup, "map_values(. + 1)", &[], "a: 1\nb: 3\na: 4\nc: 5\n"),
        (
            dup,
            "to_entries",
            &["-o", "json", "-I0"],
            "[{\"key\":\"a\",\"value\":1},{\"key\":\"b\",\"value\":2},\
             {\"key\":\"a\",\"value\":3},{\"key\":\"c\",\"value\":4}]\n",
        ),
        (
            "1: x\ntrue: y\nnull: z\n",
            "to_entries",
            &["-o", "json", "-I0"],
            "[{\"key\":1,\"value\":\"x\"},{\"key\":true,\"value\":\"y\"},\
             {\"key\":null,\"value\":\"z\"}]\n",
        ),
        (
            flow,
            "to_entries",
            &["-o", "json", "-I0"],
            "[{\"key\":\"a\",\"value\":1},{\"key\":\"b\",\"value\":2},\
             {\"key\":\"a\",\"value\":3}]\n",
        ),
        (
            flow,
            "map_values(. * 10)",
            &["-o", "json", "-I0"],
            "{\"a\":1,\"b\":20,\"a\":30}\n",
        ),
    ];
    for &(input, filter, args, expected) in rows {
        let (stdout, code) = run_yq_stdin(filter, input, args)?;
        assert_eq!(
            (stdout.as_str(), code),
            (expected, 0),
            "{filter} on {input:?}"
        );
    }

    let wide = wide();
    let wide_rows: &[(&str, &str)] = &[
        ("to_entries | length", "300\n"),
        (
            "[to_entries[] | .key] | .[248:252]",
            "[\"k248\",\"k249\",\"k0\",\"k1\"]\n",
        ),
        (
            "map_values(. + 1) | [.k0, .k249, length]",
            "[251,250,300]\n",
        ),
        (
            "[map_values(. + 1) | to_entries[] | select(.key == \"k3\")]",
            "[{\"key\":\"k3\",\"value\":3},{\"key\":\"k3\",\"value\":254}]\n",
        ),
    ];
    for &(filter, expected) in wide_rows {
        let (stdout, code) = run_yq_stdin(filter, &wide, &["-o", "json", "-I0"])?;
        assert_eq!((stdout.as_str(), code), (expected, 0), "{filter} on wide");
    }
    Ok(())
}
