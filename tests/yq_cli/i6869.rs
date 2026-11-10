//! #6869: a write through N aliases whose anchors are all different took time quadratic in N: every
//! path component of every fanned-out path asked the alias table for its redirect bound, which summed
//! the alias count of every group. The bound is now kept with the table. The output is what it was.
//!
//! The time cap is well over an order of magnitude over the run (about 3 s for 30,000 anchors in a
//! debug build, where the sum took minutes), so a loaded machine does not trip it.

use std::time::{Duration, Instant};

use anyhow::Result;

use crate::run_yq_stdin;

const JSON: &[&str] = &["-o=json", "-I=0"];

/// N anchored mappings, and N aliases of them, one each.
fn anchors(n: usize) -> String {
    let mut doc = String::from("l:\n");
    for i in 0..n {
        doc.push_str(&format!("  - &a{i} {{p: {i}}}\n"));
    }
    doc.push_str("m:\n");
    for i in 0..n {
        doc.push_str(&format!("  - *a{i}\n"));
    }
    doc
}

#[test]
fn test_a_write_through_many_aliases_of_different_anchors_is_not_quadratic_6869() -> Result<()> {
    let n = 30_000;
    let started = Instant::now();
    let (stdout, code) = run_yq_stdin(".m[].p += 1", &anchors(n), JSON)?;
    let took = started.elapsed();
    assert_eq!(code, 0);
    // Every position follows its own anchor: the list and the aliases both read `p + 1`.
    assert!(
        stdout.starts_with("{\"l\":[{\"p\":1},{\"p\":2},"),
        "{}",
        stdout.chars().take(80).collect::<String>()
    );
    assert!(stdout.contains(&format!("{{\"p\":{n}}}],\"m\":[{{\"p\":1}},")));
    assert!(stdout.ends_with(&format!("{{\"p\":{n}}}]}}\n")));
    assert!(took < Duration::from_secs(60), "took {took:?}");
    Ok(())
}

/// The other write shapes over the same document.
#[test]
fn test_the_update_and_assignment_shapes_over_distinct_anchors_match_6869() -> Result<()> {
    let doc = anchors(3);
    for (filter, want) in [
        (
            ".m[] |= (.p += 10)",
            "{\"l\":[{\"p\":10},{\"p\":11},{\"p\":12}],\"m\":[{\"p\":10},{\"p\":11},{\"p\":12}]}\n",
        ),
        (
            ".m[].q = 1",
            "{\"l\":[{\"p\":0,\"q\":1},{\"p\":1,\"q\":1},{\"p\":2,\"q\":1}],\"m\":[{\"p\":0,\"q\":1},{\"p\":1,\"q\":1},{\"p\":2,\"q\":1}]}\n",
        ),
        (
            ".m[1].p = 9",
            "{\"l\":[{\"p\":0},{\"p\":9},{\"p\":2}],\"m\":[{\"p\":0},{\"p\":9},{\"p\":2}]}\n",
        ),
    ] {
        let (stdout, code) = run_yq_stdin(filter, &doc, JSON)?;
        assert_eq!((stdout.as_str(), code), (want, 0), "{filter}");
    }
    Ok(())
}
