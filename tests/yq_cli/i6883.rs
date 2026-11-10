//! #6883: `(.. | select(tag == "!!int")) |= . + 1` over N aliases of N different anchors took time
//! quadratic in N: the descent's paths below an alias position were removed from the resolved list
//! with a scan and a shift per path, each a deep comparison. They are removed in one pass now. The
//! output is what it was.
//!
//! The time cap is over an order of magnitude over the run (about 2 s in a debug build for 40,000
//! anchors, where the removal took minutes), so a loaded machine does not trip it.

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
fn test_a_descent_write_below_many_aliases_is_not_quadratic_6883() -> Result<()> {
    let n = 40_000;
    let started = Instant::now();
    let (stdout, code) = run_yq_stdin(
        "(.. | select(tag == \"!!int\")) |= . + 1",
        &anchors(n),
        JSON,
    )?;
    let took = started.elapsed();
    assert_eq!(code, 0);
    // The descent does not go into an alias, so each anchored mapping is incremented once and
    // every alias follows its anchor.
    assert!(stdout.starts_with("{\"l\":[{\"p\":1},{\"p\":2},"));
    assert!(stdout.contains(&format!("{{\"p\":{n}}}],\"m\":[{{\"p\":1}},")));
    assert!(stdout.ends_with(&format!("{{\"p\":{n}}}]}}\n")));
    assert!(took < Duration::from_secs(60), "took {took:?}");
    Ok(())
}

/// The shapes that reach the same removal, over a document small enough to check by eye (yq v4.53.3):
/// the descent does not go into an alias, so each anchored mapping changes once and every alias
/// follows.
#[test]
fn test_the_descent_writes_over_aliases_match_yq_6883() -> Result<()> {
    let doc = "base: &b {p: 1, q: 2}\ncopy: *b\nl:\n  - *b\n  - {p: 7, n: *b}\n";
    for (filter, want) in [
        ("(.. | select(tag == \"!!int\")) |= . + 1", "{\"base\":{\"p\":2,\"q\":3},\"copy\":{\"p\":2,\"q\":3},\"l\":[{\"p\":2,\"q\":3},{\"p\":8,\"n\":{\"p\":2,\"q\":3}}]}\n"),
        ("(.l[1] | ..) |= (select(tag == \"!!int\") | . + 5)", "{\"base\":{\"p\":1,\"q\":2},\"copy\":{\"p\":1,\"q\":2},\"l\":[{\"p\":1,\"q\":2},{\"p\":12,\"n\":{\"p\":1,\"q\":2}}]}\n"),
        ("(.. | select(tag == \"!!int\")) = 0", "{\"base\":{\"p\":0,\"q\":0},\"copy\":{\"p\":0,\"q\":0},\"l\":[{\"p\":0,\"q\":0},{\"p\":0,\"n\":{\"p\":0,\"q\":0}}]}\n"),
        ("(.base | ..) |= (select(tag == \"!!int\") | . + 5)", "{\"base\":{\"p\":6,\"q\":7},\"copy\":{\"p\":6,\"q\":7},\"l\":[{\"p\":6,\"q\":7},{\"p\":7,\"n\":{\"p\":6,\"q\":7}}]}\n"),
        ("(.. | select(tag == \"!!int\")) |= . * 2", "{\"base\":{\"p\":2,\"q\":4},\"copy\":{\"p\":2,\"q\":4},\"l\":[{\"p\":2,\"q\":4},{\"p\":14,\"n\":{\"p\":2,\"q\":4}}]}\n"),
    ] {
        let (stdout, code) = run_yq_stdin(filter, doc, JSON)?;
        assert_eq!((stdout.as_str(), code), (want, 0), "{filter}");
    }
    Ok(())
}
