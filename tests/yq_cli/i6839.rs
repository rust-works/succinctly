//! #6839: `.list[] |= (.x = 7)` over N mappings that each merge one source, or N aliases of it, took
//! time quadratic in N: every write the body made walked the whole table of merged members and
//! aliases, whose paths are the document's, not the body's, and the aliases were found and written
//! by scans and rebuilds of the whole document. The table is now off inside an update body, whose
//! writes are relative to its target, and the alias lookups are indexed. The output is what it was.
//!
//! Each time cap is two orders of magnitude over the run (about 1 s in a debug build, where the
//! quadratic walk took minutes), so a loaded machine does not trip it.

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

fn aliases(n: usize) -> String {
    let mut doc = String::from("base: &base {x: 1, y: 2}\nlist:\n");
    for _ in 0..n {
        doc.push_str("  - *base\n");
    }
    doc
}

#[test]
fn test_an_update_body_over_many_merging_mappings_is_not_quadratic_6839() -> Result<()> {
    let n = 6_000;
    let started = Instant::now();
    let (stdout, code) = run_yq_stdin(".list[] |= (.x = 7)", &mappings(n), JSON)?;
    let took = started.elapsed();
    assert_eq!(code, 0);
    assert!(stdout.starts_with("{\"base\":{\"x\":7,\"y\":2},\"list\":[{\"x\":7,\"y\":2,\"i\":0},"));
    assert!(stdout.ends_with(&format!("{{\"x\":7,\"y\":2,\"i\":{}}}]}}\n", n - 1)));
    assert!(took < Duration::from_secs(60), "took {took:?}");
    Ok(())
}

/// The same shape over aliases (no merge key): each alias position was looked up by a scan of every
/// alias and written by rebuilding the document around it, so 60,000 of them took minutes.
#[test]
fn test_an_update_body_over_many_aliases_is_not_quadratic_6839() -> Result<()> {
    let started = Instant::now();
    let (stdout, code) = run_yq_stdin(".list[] |= (.x = 7)", &aliases(60_000), JSON)?;
    let took = started.elapsed();
    assert_eq!(code, 0);
    assert!(stdout.starts_with("{\"base\":{\"x\":7,\"y\":2},\"list\":[{\"x\":7,\"y\":2},"));
    assert!(stdout.ends_with("{\"x\":7,\"y\":2}]}\n"));
    assert!(took < Duration::from_secs(60), "took {took:?}");
    Ok(())
}

/// A body that deletes writes the same way.
#[test]
fn test_a_deleting_body_over_many_merging_mappings_is_not_quadratic_6839() -> Result<()> {
    let started = Instant::now();
    let (stdout, code) = run_yq_stdin(".list[] |= del(.x)", &mappings(6_000), JSON)?;
    assert_eq!(code, 0);
    assert!(stdout.starts_with("{\"base\":{\"y\":2},\"list\":[{\"y\":2,\"i\":0},"));
    assert!(
        started.elapsed() < Duration::from_secs(60),
        "took {:?}",
        started.elapsed()
    );
    Ok(())
}

/// A root target keeps the table on for its body, whose paths are the document's: `. |= (.b.p = 9)`
/// over `b: *a` still changes the shared node, as it did.
#[test]
fn test_a_root_update_body_still_writes_through_an_alias_6839() -> Result<()> {
    let (stdout, code) = run_yq_stdin(
        ". |= (.b.p = 9)",
        "a: &x {p: 1}\nb: *x\nitems:\n  - {b: {q: 1}}\n",
        JSON,
    )?;
    assert_eq!(code, 0);
    assert_eq!(
        stdout,
        "{\"a\":{\"p\":9},\"b\":{\"p\":9},\"items\":[{\"b\":{\"q\":1}}]}\n"
    );
    Ok(())
}

/// A body's path is relative to its target, so `.b.p` inside an element is the element's own `b`, not
/// the document's alias position `["b"]`: with an element holding a copy of the anchor, the old
/// matching sent the write to the element's `a` (`"a":{"p":9}`). A root path after another path keeps
/// the table for its body, whose paths are the document's. Every row is a yq v4.53.3 capture.
#[test]
fn test_a_body_path_that_spells_an_alias_position_is_the_bodys_own_6839() -> Result<()> {
    let document = "a: &x {p: 1}\nb: *x\nitems:\n  - {a: {p: 1}, b: {p: 1}}\n";
    for (filter, want) in [
    (".items[] |= (.b.p = 9)", "{\"a\":{\"p\":1},\"b\":{\"p\":1},\"items\":[{\"a\":{\"p\":1},\"b\":{\"p\":9}}]}\n"),
    (".items[0] |= (.b.p = 9)", "{\"a\":{\"p\":1},\"b\":{\"p\":1},\"items\":[{\"a\":{\"p\":1},\"b\":{\"p\":9}}]}\n"),
    (".items[] |= (.sub = 1 | .b.p = 9)", "{\"a\":{\"p\":1},\"b\":{\"p\":1},\"items\":[{\"a\":{\"p\":1},\"b\":{\"p\":9},\"sub\":1}]}\n"),
    ("(.items[], .) |= (.b.p = 9)", "{\"a\":{\"p\":9},\"b\":{\"p\":9},\"items\":[{\"a\":{\"p\":1},\"b\":{\"p\":9}}]}\n"),
    (".items[] |= (.sub |= (.b.p = 9))", "{\"a\":{\"p\":1},\"b\":{\"p\":1},\"items\":[{\"a\":{\"p\":1},\"b\":{\"p\":1},\"sub\":{\"b\":{\"p\":9}}}]}\n"),
    ] {
        let (stdout, code) = run_yq_stdin(filter, document, JSON)?;
        assert_eq!((stdout.as_str(), code), (want, 0), "{filter}");
    }
    Ok(())
}
