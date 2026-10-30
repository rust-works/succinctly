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
    "reference-slow",
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
    /// `std::env::consts::OS` this entry was observed on (`macos`, `linux`); an
    /// entry for another OS is neither expected nor called stale here.
    pub platform: Option<String>,
    /// Whether the difference shows depends on the host's speed (a reference that
    /// finishes a probe inside the deadline on a fast machine and not on a slow one):
    /// the entry is expected wherever the difference shows, but is not called stale
    /// where it does not.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub volatile: bool,
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

/// An entry observed on another OS is neither expected nor called stale here.
fn applies_here(e: &Entry) -> bool {
    e.platform
        .as_deref()
        .map_or(true, |p| p == std::env::consts::OS)
}

fn suite_of(id: &str) -> &str {
    id.split_once(':').map_or(id, |(suite, _)| suite)
}

/// `--ledger-only` (#5585): the probe ids of `suite` that the ledger lists for this host,
/// without the `suite:` prefix. The harness runs only these probes.
pub fn ids_in(entries: &[Entry], suite: &str) -> BTreeSet<String> {
    entries
        .iter()
        .filter(|e| applies_here(e))
        .filter_map(|e| e.id.strip_prefix(suite)?.strip_prefix(':'))
        .map(String::from)
        .collect()
}

/// The entries a run of `run_suites` cannot check on this host, each with the reason: a
/// `--ledger-only` run says so rather than reading them as clean.
pub fn unchecked<'a, S: AsRef<str>>(
    entries: &'a [Entry],
    run_suites: &[S],
) -> Vec<(&'a str, &'static str)> {
    entries
        .iter()
        .filter_map(|e| {
            if !applies_here(e) {
                Some((e.id.as_str(), "listed for another OS"))
            } else if !run_suites.iter().any(|s| s.as_ref() == suite_of(&e.id)) {
                Some((e.id.as_str(), "its suite was not run"))
            } else {
                None
            }
        })
        .collect()
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
        .filter(|e| applies_here(e))
        .map(|e| e.id.as_str())
        .filter(|id| in_run(id))
        .collect();
    let volatile: BTreeSet<&str> = entries
        .iter()
        .filter(|e| e.volatile)
        .map(|e| e.id.as_str())
        .collect();
    Outcome {
        unlisted: observed
            .iter()
            .filter(|id| !listed.contains(id.as_str()))
            .cloned()
            .collect(),
        stale: listed
            .iter()
            .filter(|id| !observed.contains(**id) && !volatile.contains(**id))
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
            platform: None,
            volatile: false,
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
    fn an_entry_for_another_os_is_not_stale_here() {
        let mut other = entry("cli:a");
        other.platform = Some("not-this-os".into());
        let mut here = entry("cli:b");
        here.platform = Some(std::env::consts::OS.into());
        assert!(check(&[other.clone(), here.clone()], &set(&["cli:b"]), &["cli"]).is_clean());
        let o = check(&[here], &set(&[]), &["cli"]);
        assert_eq!(o.stale, ["cli:b"]);
    }

    #[test]
    fn a_volatile_entry_is_expected_but_not_stale_when_absent() {
        let mut v = entry("robust:slow");
        v.volatile = true;
        assert!(check(&[v.clone()], &set(&[]), &["robust"]).is_clean());
        assert!(check(&[v.clone()], &set(&["robust:slow"]), &["robust"]).is_clean());
        let o = check(&[v], &set(&["robust:other"]), &["robust"]);
        assert_eq!((o.unlisted, o.stale), (vec!["robust:other".into()], vec![]));
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
    fn ids_in_lists_this_hosts_probes_of_one_suite_without_the_prefix() {
        let mut other = entry("cli:elsewhere");
        other.platform = Some("not-this-os".into());
        let e = [
            entry("cli:a"),
            entry("cli:b:with-colon"),
            entry("robust:c"),
            other,
        ];
        assert_eq!(ids_in(&e, "cli"), set(&["a", "b:with-colon"]));
        assert_eq!(ids_in(&e, "robust"), set(&["c"]));
        assert!(ids_in(&e, "features").is_empty());
        // `cli` must not match a suite that merely starts with it.
        assert!(ids_in(&[entry("cli-extra:x")], "cli").is_empty());
    }

    #[test]
    fn unchecked_names_other_os_entries_and_suites_that_did_not_run() {
        let mut other = entry("cli:elsewhere");
        other.platform = Some("not-this-os".into());
        let e = [
            entry("cli:a"),
            other,
            entry("since-secondary:add/1"),
            entry("perf:q@1mb"),
        ];
        assert_eq!(
            unchecked(&e, &["cli"]),
            [
                ("cli:elsewhere", "listed for another OS"),
                ("since-secondary:add/1", "its suite was not run"),
                ("perf:q@1mb", "its suite was not run"),
            ]
        );
    }

    #[test]
    fn the_checked_in_ledger_parses() {
        let text = include_str!("../../../../comparison/jq/divergences.toml");
        assert!(parse(text).unwrap().len() >= 10);
    }
}
