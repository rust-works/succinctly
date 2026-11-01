//! `comparison/jq/divergences/<suite>/<probe>.toml`: every known difference from the reference
//! (#5409), one entry per file (#5769).
//!
//! An id is `<suite>:<probe>` (`features:cbrt/0`, `cli:version`), and it is the path: the suite is
//! the directory, the probe is the file name, with the `/` before an arity written as `.`
//! (`features:strftime/1` is `features/strftime.1.toml`). The file holds the entry's other
//! fields. Taking the id from the path means it cannot drift from a second copy, and one file
//! per entry means two PRs that add or delete different entries never touch the same lines,
//! as `changelog.d/` did for `CHANGELOG.md` (#3920). The run fails on a difference that is not
//! listed and on a listed one that no longer differs, so the ledger cannot drift from the
//! behaviour in either direction (`jq_error_message_tests.rs` does the same for error wording).

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

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

/// The fields of an entry file: every field of [`Entry`] but the id, which is the path.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Body {
    category: String,
    reason: String,
    issue: Option<String>,
    platform: Option<String>,
    #[serde(default)]
    volatile: bool,
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

/// A name in a suite directory or a probe: the characters an id uses, none that a file system
/// reads specially.
fn is_name(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// The id of the entry in `<suite>/<file_name>`: `features` and `strftime.1.toml` are
/// `features:strftime/1`. A name that no id maps to is an error, so a stray or misspelt file
/// cannot be silently ignored.
pub fn id_of(suite: &str, file_name: &str) -> Result<String> {
    let stem = file_name
        .strip_suffix(".toml")
        .with_context(|| format!("{suite}/{file_name}: not a `.toml` file"))?;
    // `name.N`: the arity's `/` is written as `.`.
    let probe = match stem.rsplit_once('.') {
        Some((name, arity)) if !arity.is_empty() && arity.bytes().all(|b| b.is_ascii_digit()) => {
            format!("{name}/{arity}")
        }
        _ => stem.to_string(),
    };
    let name = probe.split_once('/').map_or(probe.as_str(), |(n, _)| n);
    if !is_name(suite) || !is_name(name) {
        anyhow::bail!(
            "{suite}/{file_name}: no id maps to this path (a suite and a probe use only \
             letters, digits, `_` and `-`; an arity is written `.N`)"
        );
    }
    Ok(format!("{suite}:{probe}"))
}

/// One entry from the text of `<suite>/<file_name>`.
pub fn parse_entry(suite: &str, file_name: &str, text: &str) -> Result<Entry> {
    let id = id_of(suite, file_name)?;
    let body: Body =
        toml::from_str(text).with_context(|| format!("invalid {suite}/{file_name}"))?;
    if !CATEGORIES.contains(&body.category.as_str()) {
        anyhow::bail!("divergence `{id}` has unknown category `{}`", body.category);
    }
    Ok(Entry {
        id,
        category: body.category,
        reason: body.reason,
        issue: body.issue,
        platform: body.platform,
        volatile: body.volatile,
    })
}

/// The entries under `dir` (`<dir>/<suite>/<probe>.toml`), sorted by id. `dir/README.md` is the
/// only other file allowed, and a name starting with `.` (`.DS_Store`) is skipped. Fails on a stray file or directory, on a name no id maps to, and
/// when the single-file ledger (`<dir>.toml`) exists again, so a branch rebased across the split
/// cannot bring it back.
pub fn load(dir: &Path) -> Result<Vec<Entry>> {
    let old = dir.with_extension("toml");
    if old.exists() {
        anyhow::bail!(
            "{} exists: the ledger is one file per entry under {} (#5769); move its entries there",
            old.display(),
            dir.display()
        );
    }
    let mut entries = Vec::new();
    for suite_dir in sorted_children(dir)? {
        let suite = file_name(&suite_dir)?;
        if suite.starts_with('.') {
            continue;
        }
        if suite_dir.is_file() {
            if suite == "README.md" {
                continue;
            }
            anyhow::bail!("{}: not a suite directory", suite_dir.display());
        }
        for file in sorted_children(&suite_dir)? {
            let name = file_name(&file)?;
            if name.starts_with('.') {
                continue;
            }
            if !file.is_file() {
                anyhow::bail!("{}: a directory inside a suite", file.display());
            }
            let text = std::fs::read_to_string(&file)
                .with_context(|| format!("reading {}", file.display()))?;
            // The id is the path and a path is unique, so no id can be listed twice.
            entries.push(parse_entry(&suite, &name, &text)?);
        }
    }
    entries.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(entries)
}

fn sorted_children(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut v = std::fs::read_dir(dir)
        .with_context(|| format!("reading {}", dir.display()))?
        .map(|e| e.map(|e| e.path()))
        .collect::<std::io::Result<Vec<_>>>()
        .with_context(|| format!("reading {}", dir.display()))?;
    v.sort();
    Ok(v)
}

fn file_name(path: &Path) -> Result<String> {
    path.file_name()
        .and_then(|n| n.to_str())
        .map(String::from)
        .with_context(|| format!("{}: not a UTF-8 file name", path.display()))
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
    fn an_id_is_the_path_with_the_arity_dotted() {
        for (suite, file, id) in [
            ("cli", "version.toml", "cli:version"),
            ("cli", "help-exit.toml", "cli:help-exit"),
            ("features", "strftime.1.toml", "features:strftime/1"),
            (
                "features",
                "get_jq_origin.0.toml",
                "features:get_jq_origin/0",
            ),
            ("since-secondary", "add.1.toml", "since-secondary:add/1"),
        ] {
            assert_eq!(id_of(suite, file).unwrap(), id);
        }
        for (suite, file) in [
            ("cli", "version.json"),
            ("cli", "version"),
            ("cli", ".toml"),
            ("cli", "a b.toml"),
            ("cli", "a:b.toml"),
            ("cli", "a.b.toml"),
            ("cli", "a..1.toml"),
            ("cli", "a.1.2.toml"),
            ("", "a.toml"),
            ("c li", "a.toml"),
        ] {
            assert!(id_of(suite, file).is_err(), "{suite}/{file}");
        }
    }

    #[test]
    fn an_entry_file_is_the_other_fields_and_nothing_else() {
        let ok = "category='to-triage'\nreason='y'\nissue='#1'\nplatform='linux'\nvolatile=true\n";
        let e = parse_entry("robust", "slow.toml", ok).unwrap();
        assert_eq!(
            (
                e.id.as_str(),
                e.issue.as_deref(),
                e.platform.as_deref(),
                e.volatile
            ),
            ("robust:slow", Some("#1"), Some("linux"), true)
        );
        // The id is the path, so a second copy of it is refused.
        assert!(parse_entry("cli", "a.toml", &format!("id='cli:a'\n{ok}")).is_err());
        assert!(parse_entry("cli", "a.toml", "category='to-triage'\n").is_err());
        assert!(parse_entry("cli", "a.toml", &ok.replace("to-triage", "whatever")).is_err());
        assert!(parse_entry("cli", "a.toml", &format!("{ok}extra='x'\n")).is_err());
    }

    fn write(root: &Path, rel: &str, text: &str) {
        let path = root.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    const BODY: &str = "category='to-triage'\nreason='y'\n";

    #[test]
    fn load_reads_the_directory_sorted_by_id() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("divergences");
        write(&dir, "README.md", "fields\n");
        write(&dir, ".DS_Store", "x");
        write(&dir, "cli/.DS_Store", "x");
        write(&dir, "features/strftime.1.toml", BODY);
        write(&dir, "cli/b.toml", BODY);
        write(&dir, "cli/a.toml", BODY);
        let ids: Vec<String> = load(&dir).unwrap().into_iter().map(|e| e.id).collect();
        assert_eq!(ids, ["cli:a", "cli:b", "features:strftime/1"]);
    }

    #[test]
    fn load_rejects_stray_files_and_the_old_single_file_ledger() {
        for (rel, what) in [
            ("stray.toml", "a file beside the suites"),
            ("cli/notes.txt", "a non-toml file in a suite"),
            ("cli/sub/a.toml", "a directory in a suite"),
            ("cli/bad name.toml", "a name no id maps to"),
        ] {
            let tmp = tempfile::tempdir().unwrap();
            let dir = tmp.path().join("divergences");
            write(&dir, "cli/ok.toml", BODY);
            write(&dir, rel, BODY);
            assert!(load(&dir).is_err(), "{what}");
        }
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("divergences");
        write(&dir, "cli/ok.toml", BODY);
        assert!(load(&dir).is_ok());
        // A rebased branch that brings the old file back is refused.
        std::fs::write(tmp.path().join("divergences.toml"), "").unwrap();
        let err = load(&dir).unwrap_err().to_string();
        assert!(err.contains("one file per entry"), "{err}");
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
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("comparison/jq/divergences");
        assert!(load(&dir).unwrap().len() >= 10);
    }
}
