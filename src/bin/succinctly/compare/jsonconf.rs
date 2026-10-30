//! Dimension C: JSON input conformance on JSONTestSuite (#5409).
//!
//! Each case is fed to the reference, to `succinctly jq` as shipped (which
//! semi-indexes and validates little) and to `succinctly jq --validate` (strict
//! RFC 8259). Both succinctly modes are reported, never only the flattering one.
//! `y_` must be accepted, `n_` rejected, `i_` is implementation-defined.

use super::exec::exec;
use super::harness::Tool;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::Path;

#[derive(Debug, Deserialize)]
struct RawCase {
    bytes_b64: String,
    expect: String,
    id: String,
}

#[derive(Debug, Clone)]
pub struct Case {
    pub id: String,
    pub expect: char,
    pub bytes: Vec<u8>,
}

fn base64_decode(s: &str) -> Result<Vec<u8>> {
    let (mut out, mut acc, mut bits) = (vec![], 0u32, 0u32);
    for b in s.bytes().filter(|b| *b != b'=') {
        let v = match b {
            b'A'..=b'Z' => b - b'A',
            b'a'..=b'z' => b - b'a' + 26,
            b'0'..=b'9' => b - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => anyhow::bail!("not base64: {}", b as char),
        };
        acc = (acc << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Ok(out)
}

pub fn parse(text: &str) -> Result<Vec<Case>> {
    let raw: Vec<RawCase> = serde_json::from_str(text).context("invalid JSONTestSuite file")?;
    raw.into_iter()
        .map(|c| {
            Ok(Case {
                expect: c.expect.chars().next().unwrap_or('?'),
                bytes: base64_decode(&c.bytes_b64)?,
                id: c.id,
            })
        })
        .collect()
}

#[derive(Debug, Clone, Serialize)]
pub struct CaseRow {
    pub id: String,
    pub expect: char,
    pub reference_accepts: bool,
    pub default_accepts: bool,
    pub validate_accepts: bool,
    /// Both default-mode and reference accept but print different values.
    pub output_differs: bool,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Tally {
    pub total: usize,
    pub reference: usize,
    pub default: usize,
    pub validate: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct InputSummary {
    pub y: Tally,
    pub n: Tally,
    pub i: Tally,
    pub rows: Vec<CaseRow>,
}

/// `only`: run just these case ids.
pub fn run(
    text: &str,
    reference: &Tool,
    subject: &Tool,
    only: Option<&BTreeSet<String>>,
    cwd: &Path,
) -> Result<InputSummary> {
    let mut rows = vec![];
    let validate = Tool {
        name: "succinctly jq --validate".into(),
        program: subject.program.clone(),
        prefix: subject
            .prefix
            .iter()
            .cloned()
            .chain(["--validate".to_string()])
            .collect(),
    };
    let args = vec!["-c".to_string(), ".".to_string()];
    for c in parse(text)? {
        if only.is_some_and(|s| !s.contains(&c.id)) {
            continue;
        }
        let r = exec(reference, &args, &c.bytes, cwd)?;
        let d = exec(subject, &args, &c.bytes, cwd)?;
        let v = exec(&validate, &args, &c.bytes, cwd)?;
        let (ra, da) = (r.exit == Some(0), d.exit == Some(0));
        rows.push(CaseRow {
            id: c.id,
            expect: c.expect,
            reference_accepts: ra,
            default_accepts: da,
            validate_accepts: v.exit == Some(0),
            output_differs: ra && da && r.stdout != d.stdout,
        });
    }
    let tally = |e: char| {
        let mut t = Tally::default();
        for r in rows.iter().filter(|r| r.expect == e) {
            t.total += 1;
            t.reference += usize::from(r.reference_accepts);
            t.default += usize::from(r.default_accepts);
            t.validate += usize::from(r.validate_accepts);
        }
        t
    };
    Ok(InputSummary {
        y: tally('y'),
        n: tally('n'),
        i: tally('i'),
        rows,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_checked_in_suite_parses_with_its_known_shape() {
        let cs = parse(include_str!(
            "../../../../tests/data/json-test-suite-1ef36fa.json"
        ))
        .unwrap();
        assert_eq!(cs.len(), 318);
        assert_eq!(cs.iter().filter(|c| c.expect == 'y').count(), 95);
        assert!(cs.iter().all(|c| "yni".contains(c.expect)));
    }

    fn sh(script: &str) -> Tool {
        Tool {
            name: "sh".into(),
            program: "sh".into(),
            prefix: vec!["-c".into(), script.into(), "sh".into()],
        }
    }

    /// Accepting is exit 0; a tool that rejects everything is counted as rejecting.
    #[test]
    fn base64_round_trips_known_vectors() {
        assert_eq!(base64_decode("MQ==").unwrap(), b"1");
        assert_eq!(
            base64_decode("WzEyMy40NTZlLTc4OV0=").unwrap(),
            b"[123.456e-789]"
        );
        assert!(base64_decode("!").is_err());
    }

    #[test]
    fn accept_and_reject_are_told_apart() {
        let text = r#"[{"bytes_b64":"MQ==","expect":"y","id":"y_one"},{"bytes_b64":"ew==","expect":"n","id":"n_brace"}]"#;
        let s = run(
            text,
            &sh("cat; exit 0"),
            &sh("cat; exit 1"),
            None,
            Path::new("."),
        )
        .unwrap();
        assert_eq!(
            (s.y.reference, s.y.default, s.n.reference, s.n.default),
            (1, 0, 1, 0)
        );
        let only: BTreeSet<String> = ["n_brace".to_string()].into();
        let s = run(
            text,
            &sh("cat; exit 0"),
            &sh("cat; exit 1"),
            Some(&only),
            Path::new("."),
        )
        .unwrap();
        assert_eq!(
            s.rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            ["n_brace"]
        );
    }
}
