//! The names `succinctly jq` runs that jq 1.7.1 answers `is not defined` (#6672).
//!
//! ADR-0018 rule 5 lets `succinctly jq` run a name its reference refuses only where the name is
//! **marked as an extension in user-facing documentation**, and `comparison/jq/probes/extensions.tsv`
//! is the roster the comparison harness checks against the reference ("the reference must reject,
//! `succinctly jq` must accept"). This file ties the two together without a reference binary:
//!
//! - every name in [`ROSTER`] is listed under its class in the table of
//!   `docs/reference/jq-language.md`'s "Names jq 1.7.1 does not define" section;
//! - every name is pinned by its own row in `extensions.tsv` (whose program uses it), or by jq
//!   1.8.1's vendored tests, and every row belongs to a name;
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

/// Where a name is pinned against the reference.
#[derive(Clone, Copy)]
enum Pin {
    /// Its own row in `comparison/jq/probes/extensions.tsv`: the row's id.
    Row(&'static str),
    /// The vendored jq 1.8.1 test files (`comparison/jq/vendor/jq-1.8.1/jq.test`), which the
    /// harness runs against jq 1.8.1 alone: a jq 1.8 name is not succinctly-only, so it has no
    /// "the reference must reject" row.
    Vendored18,
    /// A row another change adds (#6673 adds `isvalid`'s).
    Elsewhere,
}

use Pin::{Elsewhere, Row, Vendored18};

/// `(class, name, pin)`.
const ROSTER: &[(&str, &str, Pin)] = &[
    (OWN, "at_offset", Row("at-offset")),
    (OWN, "at_position", Row("at-position")),
    (OWN, "leaf_paths", Row("leaf-paths")),
    (OWN, "isvalid", Elsewhere),
    (OWN, "isnull", Row("isnull")),
    (OWN, "isboolean", Row("isboolean")),
    (OWN, "isnumber", Row("isnumber")),
    (OWN, "isstring", Row("isstring")),
    (OWN, "isarray", Row("isarray")),
    (OWN, "isobject", Row("isobject")),
    (OWN, "tojsonstream", Row("tojsonstream")),
    (OWN, "fromjsonstream", Row("fromjsonstream")),
    (OLD_ALIAS, "recurse_down", Row("recurse-down")),
    (JQ_18, "trim", Vendored18),
    (JQ_18, "ltrim", Vendored18),
    (JQ_18, "rtrim", Vendored18),
    (JQ_18, "toboolean", Vendored18),
    (JQ_18, "skip", Vendored18),
    (YQ, "anchor", Row("anchor")),
    (YQ, "tag", Row("tag")),
    (YQ, "kind", Row("kind")),
    (YQ, "style", Row("style")),
    (YQ, "key", Row("key")),
    (YQ, "line", Row("line")),
    (YQ, "column", Row("column")),
    (YQ, "parent", Row("parent")),
    (YQ, "path", Row("path-0")),
    (YQ, "di", Row("di")),
    (YQ, "document_index", Row("document-index")),
    (YQ, "fi", Row("fi")),
    (YQ, "file_index", Row("file-index")),
    (YQ, "fileIndex", Row("file-index-camel")),
    (YQ, "head_comment", Row("head-comment")),
    (YQ, "line_comment", Row("line-comment")),
    (YQ, "foot_comment", Row("foot-comment")),
    (YQ, "from_unix", Row("from-unix")),
    (YQ, "to_unix", Row("to-unix")),
    (YQ, "tz", Row("tz")),
    (YQ, "format_datetime", Row("format-datetime")),
    (YQ, "with_dtf", Row("with-dtf")),
    (YQ, "load", Row("load")),
    (YQ, "omit", Row("omit")),
    (YQ, "pivot", Row("pivot")),
    (YQ, "shuffle", Row("shuffle")),
    (YQ, "split_doc", Row("split-doc")),
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
            .find(|line| line.starts_with(&format!("| {class} ")))
            .with_context(|| format!("no table row for the class {class:?}"))?;
        assert!(
            row.contains(&format!("`{name}`")),
            "`{name}` is not listed on the {class:?} row of the roster table"
        );
    }
    Ok(())
}

#[test]
fn every_name_is_pinned_by_its_own_row_or_the_vendored_tests_6672() -> Result<()> {
    let rows = probe_rows()?;
    let vendored = std::fs::read_to_string(root().join("comparison/jq/vendor/jq-1.8.1/jq.test"))?;
    for (_, name, pin) in ROSTER {
        match pin {
            Row(id) => {
                let (_, args, _) = rows
                    .iter()
                    .find(|(row_id, _, _)| row_id == id)
                    .with_context(|| format!("`{name}`: no row `{id}` in extensions.tsv"))?;
                let program = args.last().cloned().unwrap_or_default();
                assert!(
                    has_word(&program, name),
                    "row `{id}` does not use `{name}`: {program}"
                );
            }
            Vendored18 => assert!(
                has_word(&vendored, name),
                "`{name}` is not exercised by jq 1.8.1's vendored jq.test"
            ),
            Elsewhere => {}
        }
    }
    // A row belongs to a name: one nobody lists (and that is not one of the surface rows from
    // before #6672) is a probe the table above would never mention.
    const EARLIER: &[&str] = &["dsv-format", "input-dsv", "validate"];
    // `parent(n)` is `parent`'s second arity: a row of its own on a name the roster lists once.
    const SECOND_ARITY: &[(&str, &str)] = &[("parent-n", "parent")];
    for (id, name) in SECOND_ARITY {
        let (_, args, _) = rows
            .iter()
            .find(|(row_id, _, _)| row_id == id)
            .with_context(|| format!("no row `{id}` in extensions.tsv"))?;
        assert!(has_word(args.last().map_or("", String::as_str), name));
    }
    for (id, _, _) in &rows {
        assert!(
            EARLIER.contains(&id.as_str())
                || SECOND_ARITY.iter().any(|(row, _)| row == id)
                || ROSTER
                    .iter()
                    .any(|(_, _, pin)| matches!(pin, Row(row) if row == id)),
            "row `{id}` of extensions.tsv names no name in the roster"
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
