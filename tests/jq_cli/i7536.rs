//! #7536: an index write past 2^29 elements is `Array index too large` in `succinctly jq`, the
//! sentence jq 1.8.2 answers since CVE-2024-23337 bounded `jv_array_set`. jq 1.7.1 has no bound
//! (it maps ~4 GB for `.[536870912] = 1` and dies with SIGSEGV on `.[1e12] = 1`), so 1.8.2 is
//! the only reference. Every expectation below was captured from
//! `/opt/homebrew/bin/jq -nc PROGRAM` (jq 1.8.2): stderr `jq: error (at <unknown>): Array index
//! too large`, exit 5. The bound itself (index `536870911` is the last one written) is #5500's
//! and is unchanged; no row writes that index, since it would allocate 8.6 GB.

use anyhow::Result;

use crate::{run_jq_null, run_jq_stdin_streams};

#[test]
fn test_index_write_past_the_bound_says_array_index_too_large_7536() -> Result<()> {
    for filter in [
        "[] | .[536870912] = 1",
        "null | .[536870912] = 1",
        "null | .[1e12] = 1",
        "[] | setpath([536870912]; 1)",
        "[] | setpath([1e12]; 1)",
        "[] | setpath([1e30]; 1)",
        "[] | .[1e12] |= 1",
        "[] | .[536870912] += 1",
        r#"{"a":[]} | .a[2147483648] = 1"#,
        r#"{} | setpath(["a", 536870912]; 1)"#,
    ] {
        let (stdout, stderr, code) = run_jq_stdin_streams(filter, "null", &["-c"])?;
        assert_eq!(stdout, "", "{filter}");
        assert_eq!(
            stderr, "jq: error (at <stdin>:0): Array index too large\n",
            "{filter}"
        );
        assert_eq!(code, 5, "{filter}");
    }
    Ok(())
}

#[test]
fn test_array_index_too_large_is_caught_by_try_7536() -> Result<()> {
    let (stdout, code) = run_jq_null(
        r#"try ([] | .[536870912] = 1 | length) catch ., try setpath([536870912]; 1) catch ., (try ([] | .[1e12] = 1) catch .)"#,
        &["-c"],
    )?;
    assert_eq!(
        (stdout.as_str(), code),
        (
            "\"Array index too large\"\n\"Array index too large\"\n\"Array index too large\"\n",
            0
        )
    );
    Ok(())
}
