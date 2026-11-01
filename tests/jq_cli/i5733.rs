//! #5733: a fold step whose write path holds an interpolated key (`.["k\($i)"] = $s`) answers the key
//! from its closed slots instead of through the re-index bridge, which serialised the whole accumulator
//! to evaluate a key that never reads it: quadratic in the loop count when the values were long. The
//! answers were captured from jq 1.7.1 (`/usr/bin/jq`).

use anyhow::Result;

use crate::run_jq_full;

fn check(cases: &[(&str, &str)]) -> Result<()> {
    for (program, expected) in cases {
        let (stdout, stderr, code) = run_jq_full(&["-nc", program], None)?;
        assert_eq!(
            (stdout.as_str(), code),
            (*expected, 0),
            "{program}: {stderr}"
        );
    }
    Ok(())
}

#[test]
fn an_interpolated_key_in_a_fold_write_answers_as_jq_does_5733() -> Result<()> {
    check(&[
        (
            r#"reduce range(0;4) as $i ({}; .["k\($i)"] = "v\($i)")"#,
            "{\"k0\":\"v0\",\"k1\":\"v1\",\"k2\":\"v2\",\"k3\":\"v3\"}\n",
        ),
        (
            r#"reduce range(0;3) as $i ({}; .["k\($i)-\($i*2)x\(null)\([1,{"a":2.50}])"] |= $i)"#,
            "{\"k0-0xnull[1,{\\\"a\\\":2.50}]\":0,\"k1-2xnull[1,{\\\"a\\\":2.50}]\":1,\"k2-4xnull[1,{\\\"a\\\":2.50}]\":2}\n",
        ),
        (
            r#"reduce range(0;4) as $i ({}; .["k\($i % 2)"] += 1)"#,
            "{\"k0\":2,\"k1\":2}\n",
        ),
        (
            r#"reduce range(0;3) as $i ({}; .["a\($i)"].b = $i)"#,
            "{\"a0\":{\"b\":0},\"a1\":{\"b\":1},\"a2\":{\"b\":2}}\n",
        ),
        // A slot that yields two values is a generator: not one key, so not this route's.
        (
            r#"reduce range(0;2) as $i ({}; .["k\("x","y")"] = $i)"#,
            "{\"kx\":1,\"ky\":1}\n",
        ),
    ])
}

#[test]
fn an_interpolated_key_with_a_long_value_is_linear_in_the_loop_count_5733() -> Result<()> {
    // 5000-byte value, 4000 keys: 44 s at 2000 keys before the fix, milliseconds after.
    let started = std::time::Instant::now();
    let (stdout, stderr, code) = run_jq_full(
        &[
            "-nc",
            r#"("x" * 5000) as $s | reduce range(0;4000) as $i ({}; .["k\($i)"] = $s) | length"#,
        ],
        None,
    )?;
    assert_eq!((stdout.as_str(), code), ("4000\n", 0), "{stderr}");
    assert!(
        started.elapsed() < std::time::Duration::from_secs(20),
        "quadratic again: {:?}",
        started.elapsed()
    );
    Ok(())
}
