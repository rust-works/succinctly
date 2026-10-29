//! `comparison/jq/divergences.toml`: every known difference from the reference (#5409).
//!
//! An id is `<suite>:<probe>` (`features:cbrt/0`, `cli:version`). The run fails
//! on a difference that is not listed and on a listed one that no longer
//! differs, so the ledger cannot drift from the behaviour in either direction
//! (`jq_error_message_tests.rs` does the same for error wording).

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::Path;

pub const CATEGORIES: &[&str] = &[
    "by-design",
    "host-dependent",
    "newer-than-primary",
    "reference-crash",
    "to-triage",
];

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Entry {
    pub id: String,
    /// One of [`CATEGORIES`].
    pub category: String,
    pub reason: String,
    /// Tracking issue, e.g. `#5409`, once one exists.
    pub issue: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct File {
    #[serde(default)]
    divergence: Vec<Entry>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Outcome {
    /// Observed, not listed.
    pub unlisted: Vec<String>,
    /// Listed, no longer observed.
    pub stale: Vec<String>,
}

impl Outcome {
    pub fn is_clean(&self) -> bool {
        self.unlisted.is_empty() && self.stale.is_empty()
    }
}

pub fn parse(text: &str) -> Result<Vec<Entry>> {
    let f: File = toml::from_str(text).context("invalid divergences.toml")?;
    let mut seen = BTreeSet::new();
    for e in &f.divergence {
        if !seen.insert(&e.id) {
            anyhow::bail!("divergence `{}` is listed twice", e.id);
        }
        if !CATEGORIES.contains(&e.category.as_str()) {
            anyhow::bail!(
                "divergence `{}` has unknown category `{}`",
                e.id,
                e.category
            );
        }
    }
    Ok(f.divergence)
}

pub fn load(path: &Path) -> Result<Vec<Entry>> {
    parse(&std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?)
}

/// Compare the listed ids with the observed ones, restricted to `suites`
/// (a partial run must not call another suite's entries stale).
pub fn check<S: AsRef<str>>(
    entries: &[Entry],
    observed: &BTreeSet<String>,
    suites: &[S],
) -> Outcome {
    let in_run = |id: &str| {
        suites
            .iter()
            .any(|s| id.starts_with(&format!("{}:", s.as_ref())))
    };
    let listed: BTreeSet<&str> = entries
        .iter()
        .map(|e| e.id.as_str())
        .filter(|id| in_run(id))
        .collect();
    Outcome {
        unlisted: observed
            .iter()
            .filter(|id| !listed.contains(id.as_str()))
            .cloned()
            .collect(),
        stale: listed
            .iter()
            .filter(|id| !observed.contains(**id))
            .map(|s| (*s).to_string())
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str) -> Entry {
        Entry {
            id: id.into(),
            category: "to-triage".into(),
            reason: "r".into(),
            issue: None,
        }
    }

    fn set(ids: &[&str]) -> BTreeSet<String> {
        ids.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn unlisted_and_stale_are_both_reported() {
        let e = [entry("cli:a"), entry("cli:b")];
        let o = check(&e, &set(&["cli:b", "cli:c"]), &["cli"]);
        assert_eq!(
            (o.unlisted, o.stale),
            (vec!["cli:c".into()], vec!["cli:a".into()])
        );
        assert!(check(&e, &set(&["cli:a", "cli:b"]), &["cli"]).is_clean());
    }

    #[test]
    fn a_partial_run_does_not_call_other_suites_stale() {
        let e = [entry("cli:a"), entry("features:x/0")];
        assert!(check(&e, &set(&["cli:a"]), &["cli"]).is_clean());
    }

    #[test]
    fn duplicates_and_unknown_categories_are_rejected() {
        let one = "[[divergence]]\nid='a:b'\ncategory='to-triage'\nreason='y'\n";
        assert!(parse(one).is_ok());
        assert!(parse(&format!("{one}{one}")).is_err());
        assert!(parse(&one.replace("to-triage", "whatever")).is_err());
    }

    #[test]
    fn the_checked_in_ledger_parses() {
        let text = include_str!("../../../../comparison/jq/divergences.toml");
        assert!(parse(text).unwrap().len() >= 10);
    }
}
