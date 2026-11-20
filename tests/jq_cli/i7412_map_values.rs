//! #7412: `map_values(f)` over a document object takes its verdict on the object from a walk that
//! keeps nothing (`validate_cursor`), where it materialized the object and dropped the copy. The
//! verdict must stay the copy's: a fault nested anywhere below a field raises jq's parse error before
//! `f` runs on any field, whether `f` reads the value or not. Captured from `/usr/bin/jq` 1.7.1.

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn nested_fault_raises_before_f_runs_7412() -> Result<()> {
    let deep = format!(
        "{{\"a\":1,\"b\":{}{}}}",
        "[".repeat(10001),
        "]".repeat(10001)
    );
    let cases: [(&str, &str); 5] = [
        (
            r#"{"a":1,"b":{"c":["\q"]}}"#,
            "jq: parse error: Invalid escape at line 1, column 21\n",
        ),
        (
            r#"{"a":1,"b":[1,2,]}"#,
            "jq: parse error: Expected another array element at line 1, column 17\n",
        ),
        (
            r#"{"a":1,"b":[,]}"#,
            "jq: parse error: Expected value before ',' at line 1, column 13\n",
        ),
        (
            r#"{"a":1,"b":-}"#,
            "jq: parse error: Invalid numeric literal at line 1, column 13\n",
        ),
        (
            &deep,
            "jq: parse error: Exceeds depth limit for parsing at line 1, column 266\n",
        ),
    ];
    for (input, want) in cases {
        for filter in [
            "map_values(1)",
            "map_values(error(\"f ran\"))",
            "map_values(empty)",
        ] {
            let (stdout, stderr, code) = run_jq_full(&["-c", filter], Some(input))?;
            assert_eq!(
                (stdout.as_str(), stderr.as_str(), code),
                ("", want, 5),
                "{filter} on {}",
                &input[..input.len().min(40)]
            );
        }
    }
    Ok(())
}

#[test]
fn well_formed_object_maps_every_field_7412() -> Result<()> {
    // Wider than the small-object cut-off, so the result's map is indexed; `f` drops the odd values.
    let fields: Vec<String> = (0..40).map(|i| format!("\"k{i}\":{i}")).collect();
    let input = format!("{{{}}}", fields.join(","));
    let want: Vec<String> = (0..40)
        .filter(|i| i % 2 == 0)
        .map(|i| format!("\"k{i}\":{}", i + 1))
        .collect();
    let (stdout, stderr, code) = run_jq_full(
        &["-c", "map_values(select(. % 2 == 0) | . + 1)"],
        Some(&input),
    )?;
    assert_eq!(
        (stdout.as_str(), stderr.as_str(), code),
        (format!("{{{}}}\n", want.join(",")).as_str(), "", 0)
    );
    Ok(())
}
