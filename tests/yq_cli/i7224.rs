//! #7224: a chain of merges that each name the previous source twice
//! (`mN: &mN {<<: [*m(N-1), *m(N-1)], iN: N}`) is read in time linear in its depth. The kind of a
//! `<<` source was read with `value()`, which resolves a merging mapping's whole field list, and a
//! key lookup walked every source it was named by again, so each level doubled the time (58 s at depth 14
//! in a debug build). The depth-16 rows are yq v4.53.3's answers (yq itself is exponential here: its
//! `.m30.k` does not finish); the depth-40 chain would not finish in the bound below before the fix.

use std::sync::mpsc;
use std::time::Duration;

use anyhow::Result;

use crate::run_yq_stdin_with_stderr;

fn chain(depth: usize) -> String {
    let mut text = String::from("m0: &m0 {k: 0}\n");
    for n in 1..=depth {
        text.push_str(&format!(
            "m{n}: &m{n} {{<<: [*m{}, *m{}], i{n}: {n}}}\n",
            n - 1,
            n - 1
        ));
    }
    text
}

/// What yq v4.53.3 answers at depth 16, for the filters that read a key, count them or list them.
#[test]
fn a_chain_of_double_merges_reads_as_yq_does_7224() -> Result<()> {
    let input = chain(16);
    for (filter, expected) in [
        (".m16.k", "0\n"),
        (".m16.i3", "3\n"),
        (".m16 | keys | length", "2\n"),
        (".m16 | length", "2\n"),
    ] {
        let (out, _, status) = run_yq_stdin_with_stderr(filter, &input, &[])?;
        assert_eq!((out.as_str(), status), (expected, 0), "{filter}");
    }
    Ok(())
}

/// A chain of 40 doubles would take 2^40 steps the old way; it is answered at once, and within a bound
/// so a regression fails the test instead of hanging it.
#[test]
fn a_deep_chain_of_double_merges_is_answered_in_linear_time_7224() -> Result<()> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let input = chain(40);
        let rows: Vec<_> = [".m40.k", ".m40.i3", ".m40", ".m40 | keys | length"]
            .iter()
            .map(|filter| run_yq_stdin_with_stderr(filter, &input, &[]).map(|(out, _, s)| (out, s)))
            .collect();
        let _ = tx.send(rows);
    });
    let rows = rx
        .recv_timeout(Duration::from_secs(60))
        .expect("a chain of 40 double merges is read in linear time");
    let answers: Vec<_> = rows.into_iter().collect::<Result<_>>()?;
    assert_eq!(answers[0], ("0\n".to_string(), 0));
    assert_eq!(answers[1], ("3\n".to_string(), 0));
    assert!(answers[2].0.starts_with("&m40 "), "{:?}", answers[2]);
    assert_eq!(answers[3], ("2\n".to_string(), 0));
    Ok(())
}

/// The shapes a `<<` source takes, read off the index and not by `value()` since #7224: a bare `-` item
/// holding a mapping, a sequence, nothing or a scalar, an alias to a scalar, a sequence or another alias,
/// and a non-mapping element after a mapping. Each is yq v4.53.3's answer (yq refuses some other shapes, which this build reads).
#[rustfmt::skip]
const SOURCES: &[(&str, &str, &str, i32)] = &[
    ("a: &a {x: 1}\nm:\n  <<:\n    -\n      y: 2\n    - *a\n", ".m", "{\"x\":1}\n", 0),
    ("a: &a {x: 1}\nm:\n  <<:\n    -\n      - n\n    - *a\n    - z: 3\n", ".m", "{\"x\":1}\n", 0),
    ("a: &a {x: 1}\nm:\n  <<:\n    -\n    - *a\n    - w: 3\n", ".m", "{\"x\":1}\n", 0),
    ("a: &a {x: 1}\nm:\n  <<:\n    -\n      5\n    - *a\n", ".m", "{\"x\":1}\n", 0),
    ("s: &s 5\na: &a {x: 1}\nm:\n  <<: [*a, 7, ~, [1]]\n", ".m", "{\"x\":1}\n", 0),
    ("m:\n  <<:\n    - - - x\n    - k: 1\n", ".m", "{}\n", 0),
];

#[test]
fn every_shape_of_merge_source_is_read_as_yq_does_7224() -> Result<()> {
    for &(input, filter, expected, status) in SOURCES {
        let (out, _, code) = run_yq_stdin_with_stderr(filter, input, &["-o", "json", "-I0"])?;
        assert_eq!(
            (out.as_str(), code),
            (expected, status),
            "{filter} on {input:?}"
        );
    }
    Ok(())
}
