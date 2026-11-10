//! #6839: `.list[] |= (.x = 7)` over N mappings that each merge one source took time quadratic in
//! N: every write the body made walked the whole table of merged members and aliases, whose paths
//! are the document's, not the body's. The table is now off inside an update body, whose writes are
//! relative to its target. The output is what it was.
//!
//! The time cap is two orders of magnitude over the run (about 1 s for 6,000 mappings in a debug
//! build, where the quadratic walk took minutes), so a loaded machine does not trip it.

use std::time::{Duration, Instant};

use anyhow::Result;

use crate::run_yq_stdin;

const JSON: &[&str] = &["-o=json", "-I=0"];

fn mappings(n: usize) -> String {
    let mut doc = String::from("base: &base {x: 1, y: 2}\nlist:\n");
    for i in 0..n {
        doc.push_str(&format!("  - {{<<: *base, i: {i}}}\n"));
    }
    doc
}

#[test]
fn test_an_update_body_over_many_merging_mappings_is_not_quadratic_6839() -> Result<()> {
    let n = 6_000;
    let doc = mappings(n);
    let started = Instant::now();
    let (stdout, code) = run_yq_stdin(".list[] |= (.x = 7)", &doc, JSON)?;
    let took = started.elapsed();
    assert_eq!(code, 0);
    assert!(stdout.starts_with("{\"base\":{\"x\":7,\"y\":2},\"list\":[{\"x\":7,\"y\":2,\"i\":0},"));
    assert!(stdout.contains(&format!("{{\"x\":7,\"y\":2,\"i\":{}}}]}}", n - 1)));
    assert!(took < Duration::from_secs(60), "took {took:?}");
    Ok(())
}

/// The same shape over aliases (no merge key), and a body that deletes.
#[test]
fn test_the_alias_and_delete_bodies_over_many_positions_are_not_quadratic_6839() -> Result<()> {
    let n = 6_000;
    let mut doc = String::from("base: &base {x: 1, y: 2}\nlist:\n");
    for _ in 0..n {
        doc.push_str("  - *base\n");
    }
    let started = Instant::now();
    let (stdout, code) = run_yq_stdin(".list[] |= (.x = 7)", &doc, JSON)?;
    assert_eq!(code, 0);
    assert!(stdout.starts_with("{\"base\":{\"x\":7,\"y\":2},\"list\":[{\"x\":7,\"y\":2},"));
    let (stdout, code) = run_yq_stdin(".list[] |= del(.x)", &mappings(n), JSON)?;
    assert_eq!(code, 0);
    assert!(stdout.starts_with("{\"base\":{\"y\":2},\"list\":[{\"y\":2,\"i\":0},"));
    assert!(
        started.elapsed() < Duration::from_secs(120),
        "took {:?}",
        started.elapsed()
    );
    Ok(())
}
