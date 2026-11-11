//! #6956: a run of nested explicit-key indicators (`? ? ? ? a`) is a key whose key is a key, and each level
//! was a call of the parser that no depth limit counted. 160,000 indicators took 17 seconds (the parse
//! was quadratic) and about 1.5 million aborted the process with a stack overflow. The recursion is
//! charged like a flow collection now: past 128 levels the run stops with `nesting depth exceeds limit
//! of 128` and exit status 1, where yq reads up to 10,000 levels (a recorded divergence: matching yq
//! would take the host process down).
//!
//! The small documents are captured from yq v4.53.3 and show that ordinary nesting reads as before.

use anyhow::Result;

use crate::{run_yq_stdin, run_yq_stdin_with_stderr};

/// Nested explicit keys yq accepts: `(document, expected JSON output)`.
const ACCEPTED: &[(&str, &str)] = &[
    ("? ? ? a\n", "{\"\":null}\n"),
    ("? ? a\n: b\n", "{\"\":\"b\"}\n"),
    ("? ? a\n  : b\n: c\n", "{\"\":\"c\"}\n"),
    ("? ? ? a\n  ? y\n: c\n", "{\"\":\"c\"}\n"),
    ("- ? ? a\n", "[{\"\":null}]\n"),
    ("- ? a\n  : b\n", "[{\"a\":\"b\"}]\n"),
    ("? - ? a\n    : b\n: c\n", "{\"\":\"c\"}\n"),
    ("? ? ? ? a\n", "{\"\":null}\n"),
    ("? ? a\n? ? b\n", "{\"\":null,\"\":null}\n"),
    ("a: 1\n? ? b\n: c\n", "{\"a\":1,\"\":\"c\"}\n"),
    ("? ? a\n: b\n? ? c\n: d\n", "{\"\":\"b\",\"\":\"d\"}\n"),
    ("? ? ?\n", "{\"\":null}\n"),
    ("? ?\n: x\n", "{\"\":\"x\"}\n"),
    ("? ? a\n  ? b\n  : c\n: d\n", "{\"\":\"d\"}\n"),
];

/// Refused by yq.
const REFUSED: &[&str] = &[
    "? ? ? a\n  : x\n  : y\n: z\n",
    "? ? ? a\n: b\n: c\n: d\n",
    "? !!str ? a\n",
    "? &a ? b\n",
];

#[test]
fn nested_explicit_keys_read_as_yq_reads_them_6956() -> Result<()> {
    for (doc, want) in ACCEPTED {
        let (out, code) = run_yq_stdin(".", doc, &["-o=json", "-I=0"])?;
        assert_eq!(code, 0, "{doc:?}");
        assert_eq!(out, *want, "{doc:?}");
    }
    for doc in REFUSED {
        let (_, code) = run_yq_stdin(".", doc, &["-o=json", "-I=0"])?;
        assert_ne!(code, 0, "{doc:?}");
    }
    // Deep, but within the limit and within yq's: 100 levels read as yq reads them.
    let (out, code) = run_yq_stdin(
        ".",
        &format!("{}a\n", "? ".repeat(100)),
        &["-o=json", "-I=0"],
    )?;
    assert_eq!(code, 0);
    assert_eq!(out, "{\"\":null}\n");
    Ok(())
}

/// A run past the limit is a clean refusal, quickly, however long it is: 3 million indicators
/// aborted the process with a stack overflow, and 160,000 took 17 seconds. yq reads up to 10,000
/// levels, so this is a recorded divergence (`docs/compliance/yq/limitations.md`).
#[test]
fn a_run_of_nested_explicit_keys_past_the_limit_is_refused_cleanly_6956() -> Result<()> {
    let docs = [
        "? ".repeat(200),
        "? ".repeat(10_000),
        "? ".repeat(3_000_000),
        "- ? ".repeat(3_000_000),
        "? ? - ".repeat(1_000_000),
        format!("{}a\n: x\n", "? ".repeat(160_000)),
    ];
    for doc in &docs {
        let started = std::time::Instant::now();
        let (out, err, code) = run_yq_stdin_with_stderr(".", doc, &["-o=json", "-I=0"])?;
        assert_eq!(code, 1, "{:?}: {err}", &doc[..8]);
        assert_eq!(out, "", "{:?}", &doc[..8]);
        assert!(
            err.contains("nesting depth exceeds limit of 128"),
            "{:?}: {err}",
            &doc[..8]
        );
        assert!(
            started.elapsed() < std::time::Duration::from_secs(8),
            "{:?}: {:?}",
            &doc[..8],
            started.elapsed()
        );
    }
    Ok(())
}
