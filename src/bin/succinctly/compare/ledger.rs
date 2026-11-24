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
//!
//! An id says that a row differs, not how it ends. A robustness entry can therefore also state
//! the outcome class it accepts for each tool (`subject_class`, `reference_class`, #7891): a row
//! that moves between an exit status and death by a signal, or starts to run into the deadline,
//! keeps its id and would otherwise pass unreported.

use super::robust::Class;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
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
    /// Whether the difference shows depends on the host: its speed (a reference that
    /// finishes a probe inside the deadline on a fast machine and not on a slow one), its
    /// address-space footprint (a probe that sits just under a `ulimit -v` cap, #7009), or
    /// the locales it has installed (a probe under `de_DE` falls back to `C` on a host
    /// without it): the entry is expected wherever the difference shows, but is not
    /// called stale where it does not.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub volatile: bool,
    /// The outcome classes this entry accepts for succinctly on a robustness row; `None`
    /// leaves the class unchecked, as before the field existed (#7891).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject_class: Option<Classes>,
    /// The same for the reference jq, where the row's meaning depends on how jq ends.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference_class: Option<Classes>,
}

/// The outcome classes an entry accepts for one tool.
///
/// In the entry file it is one class (`"error"`), a list where the outcome legitimately
/// depends on the build or the host (`["error", "crash"]`), or a table of either per
/// `std::env::consts::OS` where the platforms differ (`{ linux = "timeout", macos = "error" }`).
/// An OS the table does not name is not checked: that is how a platform nobody has observed is
/// left out.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum Classes {
    Everywhere(Vec<Class>),
    PerOs(BTreeMap<String, Vec<Class>>),
}

impl Classes {
    /// The classes accepted on `os`, or `None` when the entry says nothing for it.
    pub fn on(&self, os: &str) -> Option<&[Class]> {
        match self {
            Self::Everywhere(c) => Some(c),
            Self::PerOs(m) => m.get(os).map(Vec::as_slice),
        }
    }
}

/// The values of `std::env::consts::OS` a per-OS class table may name: a misspelt key would
/// otherwise be an expectation that is never checked.
const OSES: &[&str] = &[
    "android",
    "dragonfly",
    "freebsd",
    "illumos",
    "ios",
    "linux",
    "macos",
    "netbsd",
    "openbsd",
    "solaris",
    "windows",
];

/// One class or a non-empty list of distinct classes, as `what` (for the error) wrote it.
fn class_list(what: &str, value: &toml::Value) -> Result<Vec<Class>> {
    let one = |v: &toml::Value| -> Result<Class> {
        let name = v
            .as_str()
            .with_context(|| format!("{what}: a class is a string, not `{v}`"))?;
        Class::from_name(name).with_context(|| {
            format!(
                "{what}: unknown class `{name}` (one of {})",
                Class::ALL.map(|c| format!("`{}`", c.name())).join(", ")
            )
        })
    };
    let list = match value {
        toml::Value::Array(items) => items.iter().map(one).collect::<Result<Vec<_>>>()?,
        v => vec![one(v)?],
    };
    if list.is_empty() {
        anyhow::bail!("{what}: an empty list accepts no outcome; leave the field out instead");
    }
    if let Some(c) = list
        .iter()
        .find(|c| list.iter().filter(|x| x == c).count() > 1)
    {
        anyhow::bail!("{what}: `{}` is listed twice", c.name());
    }
    Ok(list)
}

/// The `field` of entry `id`, whose `platform` is the OS it is restricted to, if any.
fn parse_classes(
    id: &str,
    field: &str,
    platform: Option<&str>,
    value: &toml::Value,
) -> Result<Classes> {
    let what = format!("divergence `{id}`, `{field}`");
    let toml::Value::Table(table) = value else {
        return Ok(Classes::Everywhere(class_list(&what, value)?));
    };
    if table.is_empty() {
        anyhow::bail!("{what}: an empty table checks nothing; leave the field out instead");
    }
    let mut per_os = BTreeMap::new();
    for (os, v) in table {
        if !OSES.contains(&os.as_str()) {
            anyhow::bail!("{what}: `{os}` is not a `std::env::consts::OS` this loader knows");
        }
        if platform.is_some_and(|p| p != os) {
            anyhow::bail!(
                "{what}: names `{os}`, but the entry is for `platform = \"{}\"` only",
                platform.unwrap_or_default()
            );
        }
        per_os.insert(os.clone(), class_list(&format!("{what}, `{os}`"), v)?);
    }
    Ok(Classes::PerOs(per_os))
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
    subject_class: Option<toml::Value>,
    reference_class: Option<toml::Value>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Outcome {
    /// Observed, not listed.
    pub unlisted: Vec<String>,
    /// Listed, no longer observed.
    pub stale: Vec<String>,
    /// Listed and observed, but a tool ended in a class its entry does not accept (#7891).
    /// Left out of `results.json` when empty, so a run without one is written as before.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub class: Vec<ClassMismatch>,
}

impl Outcome {
    pub fn is_clean(&self) -> bool {
        self.unlisted.is_empty() && self.stale.is_empty() && self.class.is_empty()
    }
}

/// A listed row on which a tool ended in a class its entry does not accept.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ClassMismatch {
    pub id: String,
    /// `subject` or `reference`: which of the entry's two fields it is.
    pub tool: &'static str,
    pub expected: Vec<Class>,
    pub observed: Class,
}

impl ClassMismatch {
    /// `subject class `crash`, expected `error``: the part after the id.
    pub fn what(&self) -> String {
        let expected: Vec<String> = self
            .expected
            .iter()
            .map(|c| format!("`{}`", c.name()))
            .collect();
        format!(
            "{} class `{}`, expected {}",
            self.tool,
            self.observed.name(),
            expected.join(" or ")
        )
    }
}

impl std::fmt::Display for ClassMismatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.id, self.what())
    }
}

/// A name in a suite directory or a probe: the characters an id uses, none that a file system
/// reads specially.
fn is_name(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// A suite directory's name: a name, or one with a `.` after its first character, because the
/// differential suites are named after their test file (`upstream-jq.test`, so the entry of
/// its case at line 1942 is `upstream-jq.test/1942.toml`). A leading `.` is a dotfile, which
/// `load` skips. The arity rule reads only the probe's file name, so a dotted suite is no
/// ambiguity.
fn is_suite_name(s: &str) -> bool {
    !s.starts_with('.') && is_name(&s.replace('.', "_"))
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
    if !is_suite_name(suite) || !is_name(name) {
        anyhow::bail!(
            "{suite}/{file_name}: no id maps to this path (a suite and a probe use only \
             letters, digits, `_` and `-`, a suite also a `.` after its first character; \
             an arity is written `.N`)"
        );
    }
    Ok(format!("{suite}:{probe}"))
}

/// The suite whose rows have an outcome class (`robust.rs`), and so whose entries may state one.
const CLASSED_SUITE: &str = "robust";

/// One entry from the text of `<suite>/<file_name>`.
pub fn parse_entry(suite: &str, file_name: &str, text: &str) -> Result<Entry> {
    let id = id_of(suite, file_name)?;
    let body: Body =
        toml::from_str(text).with_context(|| format!("invalid {suite}/{file_name}"))?;
    if !CATEGORIES.contains(&body.category.as_str()) {
        anyhow::bail!("divergence `{id}` has unknown category `{}`", body.category);
    }
    let classes = |field: &str, value: Option<&toml::Value>| -> Result<Option<Classes>> {
        let Some(value) = value else { return Ok(None) };
        // Only the robustness dimension classes an outcome: elsewhere nothing would read the
        // field, and an expectation nothing checks is worse than none.
        if suite != CLASSED_SUITE {
            anyhow::bail!(
                "divergence `{id}` has `{field}`, which only a `{CLASSED_SUITE}` entry can state"
            );
        }
        parse_classes(&id, field, body.platform.as_deref(), value).map(Some)
    };
    Ok(Entry {
        subject_class: classes("subject_class", body.subject_class.as_ref())?,
        reference_class: classes("reference_class", body.reference_class.as_ref())?,
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
        class: vec![],
    }
}

/// The observed rows whose class an entry does not accept (#7891).
///
/// `rows` are the rows the run observed as diverging, each as its ledger id with the
/// reference's and the subject's class. Only those are judged: a listed row that no longer
/// diverges is stale (or volatile and absent), and an unlisted one is already reported. An
/// entry without the field, one for another OS, and a per-OS table that does not name this OS
/// each leave the class unchecked.
pub fn check_classes(entries: &[Entry], rows: &[(String, Class, Class)]) -> Vec<ClassMismatch> {
    check_classes_on(entries, rows, std::env::consts::OS)
}

fn check_classes_on(
    entries: &[Entry],
    rows: &[(String, Class, Class)],
    os: &str,
) -> Vec<ClassMismatch> {
    let mut out = vec![];
    for (id, reference, subject) in rows {
        let Some(e) = entries
            .iter()
            .find(|e| e.id == *id && e.platform.as_deref().map_or(true, |p| p == os))
        else {
            continue;
        };
        for (tool, spec, observed) in [
            ("reference", &e.reference_class, *reference),
            ("subject", &e.subject_class, *subject),
        ] {
            match spec.as_ref().and_then(|s| s.on(os)) {
                Some(accepted) if !accepted.contains(&observed) => out.push(ClassMismatch {
                    id: id.clone(),
                    tool,
                    expected: accepted.to_vec(),
                    observed,
                }),
                _ => {}
            }
        }
    }
    out
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
            subject_class: None,
            reference_class: None,
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
            ("upstream-jq.test", "1942.toml", "upstream-jq.test:1942"),
            ("upstream-man.test", "12.toml", "upstream-man.test:12"),
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
            (".hidden", "a.toml"),
            ("upstream-jq.test", "a.b.toml"),
            ("upstream:jq.test", "a.toml"),
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

    const ROBUST: &str = "category='by-design'\nreason='y'\n";

    fn robust(extra: &str) -> Result<Entry> {
        parse_entry("robust", "p.toml", &format!("{ROBUST}{extra}"))
    }

    /// One class, a list, or a table per OS; absent means unchecked.
    #[test]
    fn an_entry_states_the_classes_it_accepts_in_three_shapes() {
        let none = robust("").unwrap();
        assert_eq!((none.subject_class, none.reference_class), (None, None));
        let e =
            robust("subject_class = 'error'\nreference_class = ['crash', 'timeout']\n").unwrap();
        assert_eq!(
            e.subject_class,
            Some(Classes::Everywhere(vec![Class::Error]))
        );
        assert_eq!(
            e.reference_class,
            Some(Classes::Everywhere(vec![Class::Crash, Class::Timeout]))
        );
        let e = robust("subject_class = { linux = 'timeout', macos = ['error', 'ok'] }\n").unwrap();
        let spec = e.subject_class.unwrap();
        assert_eq!(spec.on("linux"), Some(&[Class::Timeout][..]));
        assert_eq!(spec.on("macos"), Some(&[Class::Error, Class::Ok][..]));
        // An OS the table does not name is not checked.
        assert_eq!(spec.on("freebsd"), None);
        // An entry restricted to one OS may name that OS in the table.
        let e = robust("platform = 'linux'\nsubject_class = { linux = 'crash' }\n").unwrap();
        assert_eq!(
            e.subject_class.unwrap().on("linux"),
            Some(&[Class::Crash][..])
        );
    }

    #[test]
    fn a_class_field_the_loader_cannot_check_is_a_load_error() {
        for (extra, needle) in [
            ("subject_class = 'crashed'\n", "unknown class `crashed`"),
            ("subject_class = 'Crash'\n", "unknown class `Crash`"),
            (
                "reference_class = ['crash', 'abort']\n",
                "unknown class `abort`",
            ),
            (
                "subject_class = { linux = 'signal' }\n",
                "unknown class `signal`",
            ),
            ("subject_class = 1\n", "a class is a string"),
            ("subject_class = ['error', 2]\n", "a class is a string"),
            ("subject_class = []\n", "an empty list"),
            ("subject_class = {}\n", "an empty table"),
            ("subject_class = { linux = [] }\n", "an empty list"),
            ("subject_class = ['error', 'error']\n", "listed twice"),
            ("subject_class = { macosx = 'error' }\n", "`macosx` is not"),
            (
                "platform = 'linux'\nsubject_class = { macos = 'error' }\n",
                "names `macos`",
            ),
            ("subject_classes = 'error'\n", "unknown field"),
        ] {
            let err = format!("{:#}", robust(extra).unwrap_err());
            assert!(err.contains(needle), "{extra}: {err}");
        }
        // The message names the entry and the field.
        let err = format!("{:#}", robust("reference_class = 'x'\n").unwrap_err());
        assert!(
            err.contains("`robust:p`") && err.contains("`reference_class`"),
            "{err}"
        );
        // Only a robustness row has a class.
        let err = parse_entry(
            "cli",
            "p.toml",
            &format!("{ROBUST}subject_class = 'error'\n"),
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("only a `robust` entry"), "{err}");
    }

    fn classed(id: &str, subject: Option<Classes>, reference: Option<Classes>) -> Entry {
        Entry {
            subject_class: subject,
            reference_class: reference,
            ..entry(id)
        }
    }

    /// No mismatch.
    const NONE: [ClassMismatch; 0] = [];

    fn row(id: &str, reference: Class, subject: Class) -> (String, Class, Class) {
        (id.into(), reference, subject)
    }

    /// The case of #7891: the row still differs, so its id is neither unlisted nor stale, and
    /// only the class says that succinctly now dies on a signal.
    #[test]
    fn a_row_that_changes_class_under_an_unchanged_id_is_reported() {
        let e = [classed(
            "robust:m",
            Some(Classes::Everywhere(vec![Class::Error])),
            Some(Classes::Everywhere(vec![Class::Ok])),
        )];
        let ids = set(&["robust:m"]);
        assert!(check(&e, &ids, &["robust"]).is_clean());
        assert_eq!(
            check_classes(&e, &[row("robust:m", Class::Ok, Class::Error)]),
            NONE
        );
        let moved = check_classes(&e, &[row("robust:m", Class::Ok, Class::Crash)]);
        assert_eq!(
            moved,
            [ClassMismatch {
                id: "robust:m".into(),
                tool: "subject",
                expected: vec![Class::Error],
                observed: Class::Crash,
            }]
        );
        assert_eq!(
            moved[0].to_string(),
            "robust:m: subject class `crash`, expected `error`"
        );
        // A mismatch fails the run like an unlisted divergence.
        let mut o = check(&e, &ids, &["robust"]);
        o.class = moved;
        assert!(!o.is_clean());
        // The reference is judged by its own field, and both can be wrong at once.
        let both = check_classes(&e, &[row("robust:m", Class::Timeout, Class::Ok)]);
        let said: Vec<String> = both.iter().map(ToString::to_string).collect();
        assert_eq!(
            said,
            [
                "robust:m: reference class `timeout`, expected `ok`",
                "robust:m: subject class `ok`, expected `error`"
            ]
        );
    }

    #[test]
    fn a_list_accepts_each_of_its_classes_and_names_them_all_on_a_mismatch() {
        let e = [classed(
            "robust:m",
            Some(Classes::Everywhere(vec![Class::Error, Class::Crash])),
            None,
        )];
        for ok in [Class::Error, Class::Crash] {
            assert_eq!(check_classes(&e, &[row("robust:m", Class::Ok, ok)]), NONE);
        }
        let m = check_classes(&e, &[row("robust:m", Class::Ok, Class::Timeout)]);
        assert_eq!(
            m[0].to_string(),
            "robust:m: subject class `timeout`, expected `error` or `crash`"
        );
    }

    /// Without the field an entry behaves as it did before the field existed, whatever the
    /// classes are; so does a row with no entry, which `check` reports as unlisted.
    #[test]
    fn an_entry_without_the_field_accepts_every_class() {
        let e = [entry("robust:m")];
        for r in Class::ALL {
            for s in Class::ALL {
                assert_eq!(check_classes(&e, &[row("robust:m", r, s)]), NONE);
                assert_eq!(check_classes(&e, &[row("robust:other", r, s)]), NONE);
            }
        }
    }

    #[test]
    fn a_class_is_checked_on_the_os_it_is_stated_for() {
        let per_os = |pairs: &[(&str, Class)]| {
            Classes::PerOs(
                pairs
                    .iter()
                    .map(|(os, c)| ((*os).to_string(), vec![*c]))
                    .collect(),
            )
        };
        let e = [classed(
            "robust:m",
            Some(per_os(&[
                ("linux", Class::Timeout),
                ("macos", Class::Error),
            ])),
            None,
        )];
        let rows = [row("robust:m", Class::Ok, Class::Timeout)];
        assert_eq!(check_classes_on(&e, &rows, "linux"), NONE);
        assert_eq!(
            check_classes_on(&e, &rows, "macos")[0].what(),
            "subject class `timeout`, expected `error`"
        );
        // Not named: not checked.
        assert_eq!(check_classes_on(&e, &rows, "freebsd"), NONE);
        // An entry for another OS is not this host's entry at all.
        let mut linux_only = classed(
            "robust:m",
            Some(Classes::Everywhere(vec![Class::Error])),
            None,
        );
        linux_only.platform = Some("linux".into());
        assert_eq!(
            check_classes_on(&[linux_only.clone()], &rows, "macos"),
            NONE
        );
        assert_eq!(check_classes_on(&[linux_only], &rows, "linux").len(), 1);
    }

    /// A recorded run keeps the expectation beside the entry, in the shape the file has it.
    #[test]
    fn an_entry_is_recorded_with_its_classes_only_when_it_states_them() {
        let plain = serde_json::to_value(entry("robust:m")).unwrap();
        assert!(plain.get("subject_class").is_none() && plain.get("reference_class").is_none());
        let e = robust("subject_class = ['error', 'crash']\nreference_class = { linux = 'ok' }\n")
            .unwrap();
        let v = serde_json::to_value(&e).unwrap();
        assert_eq!(v["subject_class"], serde_json::json!(["error", "crash"]));
        assert_eq!(v["reference_class"], serde_json::json!({"linux": ["ok"]}));
        let back: Entry = serde_json::from_value(v).unwrap();
        assert_eq!(
            (back.subject_class, back.reference_class),
            (e.subject_class, e.reference_class)
        );
        // An empty mismatch list leaves `results.json` as it was.
        let o = serde_json::to_value(Outcome::default()).unwrap();
        assert!(o.get("class").is_none(), "{o}");
    }

    /// Every class a checked-in entry states is for a probe that exists.
    #[test]
    fn the_checked_in_class_expectations_name_real_robustness_probes() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("comparison/jq/divergences");
        let probes = super::super::robust::parse_probes(include_str!(
            "../../../../comparison/jq/probes/robust.tsv"
        ))
        .unwrap();
        let classed: Vec<Entry> = load(&dir)
            .unwrap()
            .into_iter()
            .filter(|e| e.subject_class.is_some() || e.reference_class.is_some())
            .collect();
        assert!(!classed.is_empty());
        for e in classed {
            let probe = e.id.strip_prefix("robust:").unwrap();
            assert!(probes.iter().any(|p| p.id == probe), "{}", e.id);
        }
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
