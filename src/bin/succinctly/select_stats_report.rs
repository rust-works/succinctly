//! Scan-length report for the `select` word-scan loops (#40).
//!
//! Issue #40 proposes vectorising the word scan inside `select`: SIMD-popcount
//! several words at once, prefix-sum, and skip straight to the word holding the
//! k-th set bit. That can only pay off if the scans are long enough to amortise
//! the vector setup. This command measures how long they actually are, by
//! traversing real corpus files through the same cursor APIs `jq`/`yq` use and
//! reading the counters that [`succinctly::select_stats`] records.
//!
//! Requires the `select-stats` feature; without it the counters are never
//! written and the command says so rather than printing a misleading table of
//! zeros.
//!
//! ```text
//! cargo run --release --features cli,select-stats -- \
//!     dev select-stats --data-dir data/bench/corpus
//! ```

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

use succinctly::json::light::{JsonCursor, StandardJson};
use succinctly::json::JsonIndex;
use succinctly::select_stats::{self, Histogram, Site};
use succinctly::yaml::{YamlCursor, YamlIndex, YamlValue};

use crate::corpus_stats::{classify, collect_files};

/// Traverse every JSON node, materialising each one's text position.
///
/// This is the access pattern `jq` produces: `text_position()` is called for
/// every value that gets looked at, and it is the sole entry point to
/// `ib_select1_from`.
fn walk_json(cur: JsonCursor<'_, Vec<u64>>) {
    // The call whose cost we are measuring.
    let _ = cur.text_position();

    match cur.value() {
        StandardJson::Object(_) => {
            let mut child = cur.first_child();
            while let Some(k) = child {
                walk_json(k);
                match k.next_sibling() {
                    Some(v) => {
                        walk_json(v);
                        child = v.next_sibling();
                    }
                    None => child = None,
                }
            }
        }
        StandardJson::Array(_) => {
            for elem in cur.children() {
                walk_json(elem);
            }
        }
        _ => {}
    }
}

/// Traverse every YAML node, materialising each one's text position.
///
/// `YamlCursor::text_position()` reaches `AdvancePositions::get_sequential` —
/// the word scan #40 is really about — so a document-order walk reproduces the
/// scan-length distribution of `yq` streaming.
fn walk_yaml(cur: YamlCursor<'_, Vec<u64>>) {
    let _ = cur.text_position();

    match cur.value() {
        YamlValue::Mapping(fields) => {
            for field in fields {
                walk_yaml(field.value_cursor());
            }
        }
        YamlValue::Sequence(mut elements) => {
            while let Some((child, rest)) = elements.uncons_cursor() {
                walk_yaml(child);
                elements = rest;
            }
        }
        // Aliases are not followed: the target is visited at its definition,
        // and following them risks a cycle.
        YamlValue::Alias { .. } | YamlValue::String(_) | YamlValue::Null | YamlValue::Error(_) => {}
    }
}

/// Per-file result, kept so the report can attribute scans to inputs.
struct FileRun {
    workload: String,
    file: String,
    bytes: usize,
    /// Scans recorded while traversing this file, per site.
    calls: [u64; Site::COUNT],
}

/// Traverse one corpus file, returning the per-site call counts it produced.
fn run_file(root: &Path, path: &Path) -> Result<Option<FileRun>> {
    let Some((format, _delim)) = classify(path) else {
        return Ok(None);
    };
    // DSV has no select word scan on its read path; skip rather than report zeros.
    if format == "dsv" {
        return Ok(None);
    }

    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;

    let before: [u64; Site::COUNT] =
        core::array::from_fn(|i| select_stats::snapshot(Site::all()[i]).calls());

    match format {
        "json" => {
            let index = JsonIndex::build(&bytes);
            walk_json(index.root(&bytes));
        }
        "yaml" => {
            let index = YamlIndex::build(&bytes)
                .map_err(|e| anyhow::anyhow!("YAML parse failed for {}: {e:?}", path.display()))?;
            let root = index.root(&bytes);
            // The YAML root is a virtual sequence of documents.
            match root.value() {
                YamlValue::Sequence(mut docs) => {
                    while let Some((doc, rest)) = docs.uncons_cursor() {
                        walk_yaml(doc);
                        docs = rest;
                    }
                }
                _ => walk_yaml(root),
            }
        }
        _ => return Ok(None),
    }

    let after: [u64; Site::COUNT] =
        core::array::from_fn(|i| select_stats::snapshot(Site::all()[i]).calls());

    let file = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("?")
        .to_string();
    let workload = path
        .parent()
        .and_then(|p| p.strip_prefix(root).ok().and_then(|_| p.file_name()))
        .and_then(|n| n.to_str())
        .unwrap_or("-")
        .to_string();

    Ok(Some(FileRun {
        workload,
        file,
        bytes: bytes.len(),
        calls: core::array::from_fn(|i| after[i] - before[i]),
    }))
}

/// Render the fixed-width markdown table used across this repo's reports.
fn render_table(headers: &[&str], rows: &[Vec<String>]) -> String {
    let mut widths: Vec<usize> = headers.iter().map(|h| h.len()).collect();
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            if i < widths.len() {
                widths[i] = widths[i].max(cell.len());
            }
        }
    }

    let mut out = String::new();
    let header: Vec<String> = headers
        .iter()
        .enumerate()
        .map(|(i, h)| format!("{:width$}", h, width = widths[i]))
        .collect();
    out.push_str(&format!("| {} |\n", header.join(" | ")));
    let sep: Vec<String> = widths.iter().map(|w| "-".repeat(*w)).collect();
    out.push_str(&format!("| {} |\n", sep.join(" | ")));
    for row in rows {
        let cells: Vec<String> = widths
            .iter()
            .enumerate()
            .map(|(i, w)| {
                format!(
                    "{:width$}",
                    row.get(i).map_or("", |c| c.as_str()),
                    width = w
                )
            })
            .collect();
        out.push_str(&format!("| {} |\n", cells.join(" | ")));
    }
    out
}

/// One row of the distribution table for `site`.
fn site_row(site: Site, h: &Histogram) -> Vec<String> {
    let fmt = |v: Option<u64>| v.map_or_else(|| "-".to_string(), |v| v.to_string());
    let pct = |v: Option<f64>| v.map_or_else(|| "-".to_string(), |v| format!("{:.1}%", v * 100.0));

    vec![
        site.name().to_string(),
        site.unit().to_string(),
        h.calls().to_string(),
        h.mean()
            .map_or_else(|| "-".to_string(), |m| format!("{m:.2}")),
        fmt(h.percentile(50.0)),
        fmt(h.percentile(90.0)),
        fmt(h.percentile(99.0)),
        h.max().to_string(),
        pct(h.fraction_below(4)),
        pct(h.work_fraction_at_or_above(4)),
    ]
}

/// Build the markdown report for `data_dir`.
pub fn generate_report(data_dir: &Path) -> Result<String> {
    let mut files = Vec::new();
    collect_files(data_dir, &mut files)?;

    select_stats::reset();

    let mut runs = Vec::new();
    for path in &files {
        if let Some(run) = run_file(data_dir, path)? {
            runs.push(run);
        }
    }

    let mut out = String::new();
    out.push_str("# Select scan-length distribution (#40)\n\n");
    out.push_str(
        "How many words each `select` word scan traverses, measured by walking the\n\
         real-workload corpus through the same cursor APIs `jq`/`yq` use. A SIMD\n\
         block kernel processes 4 words (AVX2) or 2 (NEON) per iteration, so the\n\
         `<4 words` column is the decisive one: scans shorter than one block\n\
         cannot amortise the vector setup.\n\n",
    );

    if !cfg!(feature = "select-stats") {
        out.push_str(
            "> **No data: built without the `select-stats` feature.** The counters are\n\
             > compiled out, so every row below would read zero. Rebuild with\n\
             > `--features cli,select-stats` and re-run.\n\n",
        );
    }

    out.push_str("## Scan lengths by site\n\n");
    out.push_str(
        "`ib_select1_from` rows count `ib_rank` probes rather than words: that path\n\
         searches a precomputed rank array and never popcounts, so #40's trick does\n\
         not apply to it. It is listed because it is the *hot* select on the query\n\
         path, which is what makes the comparison interesting.\n\n",
    );

    let rows: Vec<Vec<String>> = Site::all()
        .iter()
        .map(|&s| site_row(s, &select_stats::snapshot(s)))
        .collect();
    out.push_str(&render_table(
        &[
            "site", "unit", "calls", "mean", "p50", "p90", "p99", "max", "calls <4", "work >=4",
        ],
        &rows,
    ));
    out.push_str(
        "\n`calls <4` is the share of *calls* too short for a 4-word block to help.\n\
         `work >=4` is the share of *total words scanned* that happens in scans of 4+\n\
         words. When these disagree the distribution is bimodal, and `work >=4` is the\n\
         one that predicts a SIMD kernel's effect on runtime: a scalar prologue keeps\n\
         the short calls free while the vector path absorbs the long tail.\n",
    );

    out.push_str("\n## Files traversed\n\n");
    let file_rows: Vec<Vec<String>> = runs
        .iter()
        .map(|r| {
            let total: u64 = r.calls.iter().sum();
            vec![
                r.workload.clone(),
                r.file.clone(),
                r.bytes.to_string(),
                total.to_string(),
            ]
        })
        .collect();
    out.push_str(&render_table(
        &["workload", "file", "bytes", "select calls"],
        &file_rows,
    ));

    Ok(out)
}

/// Entry point for `succinctly dev select-stats`.
pub fn run(data_dir: &Path, markdown: Option<&PathBuf>) -> Result<i32> {
    let report = generate_report(data_dir)?;

    match markdown {
        Some(path) => {
            std::fs::write(path, &report)
                .with_context(|| format!("writing report to {}", path.display()))?;
            eprintln!("wrote {}", path.display());
        }
        None => print!("{report}"),
    }

    // A run that recorded nothing is a tooling failure, not a finding — most
    // likely the binary was built without `select-stats`. Surface it as a
    // non-zero exit so a scripted invocation cannot mistake it for "no scans".
    let recorded: u64 = Site::all()
        .iter()
        .map(|&s| select_stats::snapshot(s).calls())
        .sum();
    Ok(i32::from(recorded == 0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_file(root: &Path, rel: &str, content: &[u8]) -> PathBuf {
        let path = root.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, content).unwrap();
        path
    }

    #[test]
    fn render_table_pads_columns() {
        let table = render_table(&["a", "bb"], &[vec!["x".into(), "yy".into()]]);
        assert!(table.contains("| a | bb |"));
        assert!(table.contains("| x | yy |"));
    }

    #[test]
    fn site_row_renders_placeholders_for_empty_histogram() {
        let row = site_row(Site::YamlAdvance, &Histogram::default());
        assert_eq!(row[0], Site::YamlAdvance.name());
        assert_eq!(row[1], "words");
        assert_eq!(row[2], "0");
        // mean / percentiles / fraction have no value to report
        assert_eq!(row[3], "-");
        assert_eq!(row[4], "-");
        assert_eq!(row[8], "-");
    }

    #[test]
    fn ib_select_from_sites_are_labelled_in_probes() {
        let row = site_row(Site::JsonIbSelectFrom, &Histogram::default());
        assert_eq!(row[1], "probes");
    }

    #[test]
    fn report_covers_json_and_yaml_and_skips_dsv() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write_file(root, "json/a/one.json", br#"{"a":[1,2,{"b":"c"}]}"#);
        write_file(root, "yaml/b/two.yaml", b"a:\n  - 1\n  - k: v\n");
        write_file(root, "dsv/c/three.csv", b"x,y\n1,2\n");
        write_file(root, "other/skip.txt", b"ignored");

        let report = generate_report(root).unwrap();

        assert!(report.contains("one.json"), "{report}");
        assert!(report.contains("two.yaml"), "{report}");
        // DSV has no select word scan on its read path, and non-corpus
        // extensions are not walked at all.
        assert!(!report.contains("three.csv"), "{report}");
        assert!(!report.contains("skip.txt"), "{report}");

        // Every site gets a row, whether or not it recorded anything.
        for site in Site::all() {
            assert!(report.contains(site.name()), "missing {}", site.name());
        }
    }

    #[test]
    fn run_writes_markdown_when_asked() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write_file(root, "json/a/one.json", br#"{"a":1}"#);
        let out = root.join("report.md");

        let code = run(root, Some(&out)).unwrap();
        let written = std::fs::read_to_string(&out).unwrap();
        assert!(written.contains("Select scan-length distribution"));

        // Exit code reflects whether counters were populated, which depends on
        // the feature this build was compiled with.
        assert_eq!(code, i32::from(!cfg!(feature = "select-stats")));
    }

    #[test]
    fn run_to_stdout_succeeds() {
        let dir = tempfile::tempdir().unwrap();
        write_file(dir.path(), "json/a/one.json", br"[1,2,3]");
        // Exercises the stdout branch; the code again tracks the feature.
        let code = run(dir.path(), None).unwrap();
        assert_eq!(code, i32::from(!cfg!(feature = "select-stats")));
    }

    #[test]
    fn empty_corpus_reports_no_files() {
        let dir = tempfile::tempdir().unwrap();
        let report = generate_report(dir.path()).unwrap();
        assert!(report.contains("Files traversed"));
    }
}
