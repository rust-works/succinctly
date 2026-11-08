//! The names `succinctly jq` runs that jq 1.7.1 answers `is not defined` (#6672).
//!
//! ADR-0018 rule 5 lets `succinctly jq` run a name its reference refuses only where the name is
//! **marked as an extension in user-facing documentation**, and `comparison/jq/probes/extensions.tsv`
//! is the roster the comparison harness checks against the reference ("the reference must reject,
//! `succinctly jq` must accept"). This file ties the two together without a reference binary:
//!
//! - every name in [`ROSTER`] is listed under its class in the table of
//!   `docs/reference/jq-language.md`'s "Names jq 1.7.1 does not define" section;
//! - every name that has no row elsewhere has a row in `extensions.tsv` whose program uses it;
//! - every row of `extensions.tsv` runs under `succinctly jq` and exits 0, the half of the verdict
//!   that needs no reference (the harness judges the other half against jq 1.7.1).
//!
//! The roster was captured on 2026-11-09 by running every name the parser matches, at arities 0-3,
//! through `/usr/bin/jq` 1.7.1 (`-n`), jq 1.8.2 and yq v4.53.3. A name added to the parser that is
//! neither jq's nor on this list is not caught here: the list of jq's own builtins needs the
//! reference, so the harness's builtin sweep is what notices it.
//!
//! Run with: cargo test --features cli --test jq_extension_roster_tests

#![cfg(feature = "cli")]

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use anyhow::{Context, Result};

/// Succinctly's own: in no jq and not in yq.
const OWN: &str = "succinctly's own";
/// Defined by jq 1.8.2, not by 1.7.1; `succinctly jq` follows 1.8.
const JQ_18: &str = "a jq 1.8 name";
/// Defined by yq v4.53.3 and also run in jq mode.
const YQ: &str = "a yq name";
/// No jq 1.7.1 or 1.8.2 and no yq has it; it behaves as `recurse`.
const OLD_ALIAS: &str = "an old alias";

/// `(class, name, covered by a row elsewhere)`: a name with `true` is pinned by a row this file
/// does not look for (`at_offset`, `at_position` and `leaf_paths` have their own; `isvalid` has
/// one in #6673; the jq 1.8 names are probed against jq 1.8.1 by its vendored tests).
const ROSTER: &[(&str, &str, bool)] = &[
    (OWN, "at_offset", true),
    (OWN, "at_position", true),
    (OWN, "leaf_paths", true),
    (OWN, "isvalid", true),
    (OWN, "isnull", false),
    (OWN, "isboolean", false),
    (OWN, "isnumber", false),
    (OWN, "isstring", false),
    (OWN, "isarray", false),
    (OWN, "isobject", false),
    (OWN, "tojsonstream", false),
    (OWN, "fromjsonstream", false),
    (OLD_ALIAS, "recurse_down", false),
    (JQ_18, "trim", true),
    (JQ_18, "ltrim", true),
    (JQ_18, "rtrim", true),
    (JQ_18, "toboolean", true),
    (JQ_18, "skip", true),
    (YQ, "anchor", false),
    (YQ, "tag", false),
    (YQ, "kind", false),
    (YQ, "style", false),
    (YQ, "key", false),
    (YQ, "line", false),
    (YQ, "column", false),
    (YQ, "parent", false),
    (YQ, "path", false),
    (YQ, "di", false),
    (YQ, "document_index", false),
    (YQ, "fi", false),
    (YQ, "file_index", false),
    (YQ, "fileIndex", false),
    (YQ, "head_comment", false),
    (YQ, "line_comment", false),
    (YQ, "foot_comment", false),
    (YQ, "from_unix", false),
    (YQ, "to_unix", false),
    (YQ, "tz", false),
    (YQ, "format_datetime", false),
    (YQ, "with_dtf", false),
    (YQ, "load", false),
    (YQ, "omit", false),
    (YQ, "pivot", false),
    (YQ, "shuffle", false),
    (YQ, "split_doc", false),
];

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// The rows of `extensions.tsv`: `(id, args, stdin)`.
fn probe_rows() -> Result<Vec<(String, Vec<String>, String)>> {
    let text = std::fs::read_to_string(root().join("comparison/jq/probes/extensions.tsv"))?;
    let mut rows = Vec::new();
    for line in text
        .lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
    {
        let cols: Vec<&str> = line.split('\t').collect();
        anyhow::ensure!(cols.len() == 4, "malformed row: {line}");
        let args: Vec<String> = serde_json::from_str(cols[2]).context(cols[0].to_string())?;
        let stdin: String = serde_json::from_str(cols[3]).context(cols[0].to_string())?;
        rows.push((cols[0].to_string(), args, stdin));
    }
    Ok(rows)
}

/// Whether `word` occurs in `text` as a whole identifier.
fn has_word(text: &str, word: &str) -> bool {
    let is_ident = |c: char| c.is_ascii_alphanumeric() || c == '_';
    text.match_indices(word).any(|(at, _)| {
        !text[..at].chars().next_back().is_some_and(is_ident)
            && !text[at + word.len()..].chars().next().is_some_and(is_ident)
    })
}

/// The text of the roster section of the language reference: its heading to the next heading.
fn doc_section() -> Result<String> {
    let doc = std::fs::read_to_string(root().join("docs/reference/jq-language.md"))?;
    let start = doc
        .find("#### Names jq 1.7.1 does not define")
        .context("the roster heading is missing from docs/reference/jq-language.md")?;
    let rest = &doc[start..];
    let end = rest[1..].find("\n#").map_or(rest.len(), |at| at + 1);
    Ok(rest[..end].to_string())
}

#[test]
fn every_name_is_listed_under_its_class_in_the_reference_6672() -> Result<()> {
    let section = doc_section()?;
    for (class, name, _) in ROSTER {
        let row = section
            .lines()
            .find(|line| line.starts_with('|') && line.contains(class))
            .with_context(|| format!("no table row for the class {class:?}"))?;
        assert!(
            row.contains(&format!("`{name}`")),
            "`{name}` is not listed on the {class:?} row of the roster table"
        );
    }
    Ok(())
}

#[test]
fn every_uncovered_name_has_a_probe_row_6672() -> Result<()> {
    let rows = probe_rows()?;
    for (_, name, covered_elsewhere) in ROSTER {
        if *covered_elsewhere {
            continue;
        }
        let program = |args: &[String]| args.last().cloned().unwrap_or_default();
        assert!(
            rows.iter()
                .any(|(_, args, _)| has_word(&program(args), name)),
            "`{name}` has no row in comparison/jq/probes/extensions.tsv"
        );
    }
    Ok(())
}

#[test]
fn every_probe_row_runs_under_succinctly_jq_6672() -> Result<()> {
    let fixtures = root().join("comparison/jq/fixtures");
    for (id, args, stdin) in probe_rows()? {
        let mut child = Command::new(env!("CARGO_BIN_EXE_succinctly"))
            .arg("jq")
            .args(&args)
            .current_dir(&fixtures)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        child
            .stdin
            .take()
            .context("stdin")?
            .write_all(stdin.as_bytes())?;
        let out = child.wait_with_output()?;
        assert!(
            out.status.success(),
            "row {id}: exit {:?}: {}",
            out.status.code(),
            String::from_utf8_lossy(&out.stderr)
        );
    }
    Ok(())
}
