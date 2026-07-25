//! Shape-statistics generator for the real-workload benchmark corpus (#301).
//!
//! Walks a corpus directory (`<data-dir>/<format>/<workload>/<file>`), parses
//! every file with the crate's own semi-index (`JsonIndex` / `YamlIndex` /
//! `DsvIndex`), and derives the structural distributions a perf reviewer needs
//! to answer "does this input shape exist in real workloads?" — the check that
//! killed P5 (flow-collection fast path) at the analysis stage.
//!
//! The report is **deterministic**: no timings, no timestamps, no host info —
//! only shapes derived from the bytes. That lets a golden snapshot over the
//! committed seed corpus be verified in CI (`--check`), so the tooling and the
//! report cannot silently rot as the parsers change.
//!
//! Metrics are read from the real index (not a bespoke parser) so they measure
//! exactly what the code sees: nesting depth from balanced-parens excess, flow
//! collection extents from cursor byte ranges, anchor/alias counts from the BP
//! structure, string lengths and escape density from raw string spans.

use anyhow::{Context, Result};
use serde::Serialize;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use succinctly::dsv::{build_index as build_dsv_index, DsvConfig, DsvRows};
use succinctly::json::light::{JsonCursor, StandardJson};
use succinctly::json::JsonIndex;
use succinctly::yaml::{YamlCursor, YamlIndex, YamlValue};

/// A distribution of integer samples, summarised as min/median/p90/p99/max.
#[derive(Default)]
struct Dist {
    values: Vec<u64>,
}

impl Dist {
    fn push(&mut self, v: u64) {
        self.values.push(v);
    }

    /// Nearest-rank percentile over the sorted samples (`p` in `0.0..=100.0`).
    fn percentile(sorted: &[u64], p: f64) -> u64 {
        let n = sorted.len();
        if n == 0 {
            return 0;
        }
        let rank = ((p / 100.0) * n as f64).ceil() as usize;
        sorted[rank.clamp(1, n) - 1]
    }

    fn summary(&self) -> Option<DistSummary> {
        if self.values.is_empty() {
            return None;
        }
        let mut sorted = self.values.clone();
        sorted.sort_unstable();
        Some(DistSummary {
            count: sorted.len() as u64,
            min: sorted[0],
            p50: Self::percentile(&sorted, 50.0),
            p90: Self::percentile(&sorted, 90.0),
            p99: Self::percentile(&sorted, 99.0),
            max: sorted[sorted.len() - 1],
        })
    }
}

struct DistSummary {
    count: u64,
    min: u64,
    p50: u64,
    p90: u64,
    p99: u64,
    max: u64,
}

/// One row in a per-format distribution table.
fn dist_row(metric: &str, unit: &str, dist: &Dist) -> Vec<String> {
    match dist.summary() {
        Some(s) => vec![
            metric.to_string(),
            unit.to_string(),
            s.count.to_string(),
            s.min.to_string(),
            s.p50.to_string(),
            s.p90.to_string(),
            s.p99.to_string(),
            s.max.to_string(),
        ],
        None => vec![
            metric.to_string(),
            unit.to_string(),
            "0".to_string(),
            "-".to_string(),
            "-".to_string(),
            "-".to_string(),
            "-".to_string(),
            "-".to_string(),
        ],
    }
}

// ---------------------------------------------------------------------------
// Per-format accumulators
// ---------------------------------------------------------------------------

#[derive(Default)]
struct JsonStats {
    files: u64,
    total_bytes: u64,
    string_len: Dist,
    keys_per_object: Dist,
    elems_per_array: Dist,
    collection_bytes: Dist,
    depth: Dist,
    escape_density: Dist, // escape chars per KiB, one sample per file
}

#[derive(Default)]
struct YamlStats {
    files: u64,
    total_bytes: u64,
    scalar_len: Dist,
    keys_per_mapping: Dist,
    items_per_sequence: Dist,
    flow_collection_bytes: Dist,
    depth: Dist,
    escape_density: Dist, // per file
    anchors_per_file: Dist,
    aliases_per_file: Dist,
    bare_dash_items_per_file: Dist,
}

/// Counters that are accumulated over a whole file rather than per node, so the
/// distribution gets one sample per file (like `anchors_per_file`) instead of one
/// per occurrence. Threaded through `visit_yaml` as a single `&mut` to keep the
/// recursion's parameter count down as counters are added.
#[derive(Default)]
struct YamlFileCounters {
    /// `\` bytes across every scalar and key, for the escape-density metric.
    escapes: u64,
    /// Block-sequence items whose `-` indicator sits alone on its own line.
    bare_dash_items: u64,
}

#[derive(Default)]
struct DsvStats {
    files: u64,
    total_bytes: u64,
    field_len: Dist,
    fields_per_row: Dist,
    rows_per_file: Dist,
    embedded_delims_per_file: Dist,
    embedded_newlines_per_file: Dist,
    embedded_quotes_per_file: Dist,
    quoted_fields: u64,
    total_fields: u64,
}

#[derive(Default)]
struct Corpus {
    json: JsonStats,
    yaml: YamlStats,
    dsv: DsvStats,
    inventory: Vec<FileRecord>,
}

/// Per-file record, also emitted to the optional `--output` JSONL.
#[derive(Serialize)]
pub struct FileRecord {
    pub format: String,
    pub workload: String,
    pub file: String,
    pub bytes: u64,
}

// ---------------------------------------------------------------------------
// JSON traversal
// ---------------------------------------------------------------------------

fn visit_json(index: &JsonIndex<Vec<u64>>, cur: JsonCursor<'_, Vec<u64>>, st: &mut JsonStats) {
    st.depth
        .push(index.bp().depth(cur.bp_position()).unwrap_or(0) as u64);

    match cur.value() {
        StandardJson::String(js) => {
            let raw = js.raw_bytes();
            // content length excludes the surrounding quotes
            st.string_len.push(raw.len().saturating_sub(2) as u64);
        }
        StandardJson::Object(_) => {
            if let Some((s, e)) = cur.text_range() {
                st.collection_bytes.push((e - s) as u64);
            }
            let mut keys = 0u64;
            let mut child = cur.first_child();
            while let Some(k) = child {
                keys += 1;
                visit_json(index, k, st); // key (a string node)
                match k.next_sibling() {
                    Some(v) => {
                        visit_json(index, v, st); // value
                        child = v.next_sibling();
                    }
                    None => child = None,
                }
            }
            st.keys_per_object.push(keys);
        }
        StandardJson::Array(_) => {
            if let Some((s, e)) = cur.text_range() {
                st.collection_bytes.push((e - s) as u64);
            }
            let mut n = 0u64;
            for elem in cur.children() {
                n += 1;
                visit_json(index, elem, st);
            }
            st.elems_per_array.push(n);
        }
        StandardJson::Number(_) | StandardJson::Bool(_) | StandardJson::Null => {}
        StandardJson::Error(_) => {}
    }
}

/// Count JSON `\`-escape characters inside every string span (for escape density).
fn count_json_escapes(cur: JsonCursor<'_, Vec<u64>>) -> u64 {
    let mut total = 0u64;
    match cur.value() {
        StandardJson::String(js) => {
            total += js.raw_bytes().iter().filter(|&&b| b == b'\\').count() as u64;
        }
        StandardJson::Object(_) => {
            let mut child = cur.first_child();
            while let Some(k) = child {
                total += count_json_escapes(k);
                match k.next_sibling() {
                    Some(v) => {
                        total += count_json_escapes(v);
                        child = v.next_sibling();
                    }
                    None => child = None,
                }
            }
        }
        StandardJson::Array(_) => {
            for elem in cur.children() {
                total += count_json_escapes(elem);
            }
        }
        _ => {}
    }
    total
}

fn ingest_json(bytes: &[u8], st: &mut JsonStats) {
    let index = JsonIndex::build(bytes);
    let root = index.root(bytes);
    visit_json(&index, root, st);
    let escapes = count_json_escapes(index.root(bytes));
    push_escape_density(&mut st.escape_density, escapes, bytes.len());
    st.files += 1;
    st.total_bytes += bytes.len() as u64;
}

// ---------------------------------------------------------------------------
// YAML traversal
// ---------------------------------------------------------------------------

fn visit_yaml(
    index: &YamlIndex<Vec<u64>>,
    bytes: &[u8],
    cur: YamlCursor<'_, Vec<u64>>,
    st: &mut YamlStats,
    counters: &mut YamlFileCounters,
) {
    st.depth
        .push(index.bp().depth(cur.bp_position()).unwrap_or(0) as u64);

    match cur.value() {
        YamlValue::String(ys) => {
            let raw = ys.raw_bytes();
            st.scalar_len.push(raw.len() as u64);
            counters.escapes += raw.iter().filter(|&&b| b == b'\\').count() as u64;
        }
        YamlValue::Mapping(fields) => {
            record_flow(bytes, &cur, st);
            let mut keys = 0u64;
            for field in fields {
                keys += 1;
                if let YamlValue::String(ks) = field.key() {
                    let raw = ks.raw_bytes();
                    st.scalar_len.push(raw.len() as u64);
                    counters.escapes += raw.iter().filter(|&&b| b == b'\\').count() as u64;
                }
                visit_yaml(index, bytes, field.value_cursor(), st, counters);
            }
            st.keys_per_mapping.push(keys);
        }
        YamlValue::Sequence(elements) => {
            record_flow(bytes, &cur, st);
            // Only block sequences carry a `-` indicator to classify; flow
            // sequences (`[a, b]`) have none.
            let block = cur.style() != "flow";
            let mut es = elements;
            let mut n = 0u64;
            while let Some((child, rest)) = es.uncons_cursor() {
                n += 1;
                if block && has_bare_dash_indicator(bytes, &child) {
                    counters.bare_dash_items += 1;
                }
                visit_yaml(index, bytes, child, st, counters);
                es = rest;
            }
            st.items_per_sequence.push(n);
        }
        // Aliases are counted via the BP scan below; do not recurse (cycle-safe).
        YamlValue::Alias { .. } | YamlValue::Null | YamlValue::Error(_) => {}
    }
}

/// Whether a block-sequence item's `-` indicator sits alone on its own line
/// (`-\n  key: value`) rather than inline with the content (`- key: value`).
///
/// Derived from the item's own text position rather than a bespoke re-scan of the
/// document, on the same principle as O4's seq-item wrappers: the text already
/// encodes it. The index distinguishes the two forms by where it anchors the item
/// — an inline item's position is its *content* (the `b` of `- b: 2`), while a
/// bare-dash item has no content on that line so the position is the `-` itself.
/// So the test is: the position is a `-`, and the rest of that line is blank.
///
/// Both halves are load-bearing. Requiring a `-` rejects inline items; requiring
/// the rest of the line to be blank rejects the two shapes whose *content* also
/// starts with a dash — a negative number (`- -5`) and a nested inline sequence
/// (`- - deep`) — which the first check alone would miscount.
///
/// One deliberate undercount: a comment between the indicator and the content
/// (`-  # note\n  key: v`) anchors the item at the content, so it reads as inline.
/// The metric is therefore a lower bound on bare-dash usage.
fn has_bare_dash_indicator(bytes: &[u8], child: &YamlCursor<'_, Vec<u64>>) -> bool {
    child.text_position().is_some_and(|start| {
        bytes.get(start) == Some(&b'-')
            // The indicator is bare only if nothing but horizontal whitespace
            // follows it on that line. Running out of input counts: a trailing
            // `-` with no newline after it is still alone on its line.
            && match bytes[start + 1..]
                .iter()
                .find(|&&b| !matches!(b, b' ' | b'\t' | b'\r'))
            {
                Some(&b) => b == b'\n',
                None => true,
            }
    })
}

/// Record a flow collection's byte extent (the P5 metric) when `cur` is a flow
/// node. `YamlCursor::raw_bytes()` returns `None` for flow containers, so the
/// extent is measured by bracket-matching from the node's text position — the
/// one position the index exposes reliably for flow nodes.
fn record_flow(bytes: &[u8], cur: &YamlCursor<'_, Vec<u64>>, st: &mut YamlStats) {
    if cur.style() != "flow" {
        return;
    }
    if let Some(len) = cur.text_position().and_then(|s| flow_extent(bytes, s)) {
        st.flow_collection_bytes.push(len as u64);
    }
}

/// Byte length of a flow collection whose opening `[`/`{` is at `start`,
/// including both brackets. Balances `[]`/`{}` while skipping quoted scalars so
/// brackets inside strings do not confuse the match. Returns `None` if `start`
/// is not a flow opener or the collection is unterminated.
fn flow_extent(bytes: &[u8], start: usize) -> Option<usize> {
    let open = *bytes.get(start)?;
    if open != b'[' && open != b'{' {
        return None;
    }
    let mut depth = 0i32;
    let mut i = start;
    let (mut in_single, mut in_double) = (false, false);
    while i < bytes.len() {
        let ch = bytes[i];
        if in_double {
            if ch == b'\\' {
                i += 2;
                continue;
            }
            if ch == b'"' {
                in_double = false;
            }
        } else if in_single {
            if ch == b'\'' {
                if bytes.get(i + 1) == Some(&b'\'') {
                    i += 2; // '' is the single-quote escape
                    continue;
                }
                in_single = false;
            }
        } else {
            match ch {
                b'"' => in_double = true,
                b'\'' => in_single = true,
                b'[' | b'{' => depth += 1,
                b']' | b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(i - start + 1);
                    }
                }
                _ => {}
            }
        }
        i += 1;
    }
    None
}

fn ingest_yaml(bytes: &[u8], st: &mut YamlStats) -> Result<()> {
    let index = YamlIndex::build(bytes).map_err(|e| anyhow::anyhow!("YAML parse failed: {e:?}"))?;

    // Anchor/alias totals from the BP structure (no bulk accessor exists).
    let bp = index.bp();
    let (mut anchors, mut aliases) = (0u64, 0u64);
    for p in 0..bp.len() {
        if bp.is_open(p) {
            if index.get_anchor_name(p).is_some() {
                anchors += 1;
            }
            if index.is_alias(p) {
                aliases += 1;
            }
        }
    }
    st.anchors_per_file.push(anchors);
    st.aliases_per_file.push(aliases);

    // The YAML root is a virtual sequence of documents; visit each document so
    // the doc-list does not pollute the sequence-item distribution.
    let mut counters = YamlFileCounters::default();
    let root = index.root(bytes);
    match root.value() {
        YamlValue::Sequence(mut docs) => {
            while let Some((doc, rest)) = docs.uncons_cursor() {
                visit_yaml(&index, bytes, doc, st, &mut counters);
                docs = rest;
            }
        }
        // Defensive, and not reachable for any input observed: `index.root()`
        // returns the virtual document sequence for empty, comment-only, scalar,
        // flow, block-scalar and multi-document input alike.
        _ => visit_yaml(&index, bytes, root, st, &mut counters),
    }
    st.bare_dash_items_per_file.push(counters.bare_dash_items);
    push_escape_density(&mut st.escape_density, counters.escapes, bytes.len());
    st.files += 1;
    st.total_bytes += bytes.len() as u64;
    Ok(())
}

// ---------------------------------------------------------------------------
// DSV traversal
// ---------------------------------------------------------------------------

fn ingest_dsv(bytes: &[u8], delimiter: u8, st: &mut DsvStats) {
    let config = DsvConfig::default().with_delimiter(delimiter);
    let index = build_dsv_index(bytes, &config);
    let rows = DsvRows::new(bytes, &index);

    let mut row_count = 0u64;
    let (mut emb_delims, mut emb_newlines, mut emb_quotes) = (0u64, 0u64, 0u64);

    for row in rows {
        row_count += 1;
        let mut fields_in_row = 0u64;
        for field in row.fields() {
            fields_in_row += 1;
            st.total_fields += 1;
            st.field_len.push(field.len() as u64);

            let quoted =
                field.first() == Some(&b'"') && field.len() >= 2 && field.last() == Some(&b'"');
            if quoted {
                st.quoted_fields += 1;
            }

            // `fields()` returns the raw span incl. quotes, so any delimiter or
            // newline that survived inside the field was quoted (embedded).
            let quotes = field.iter().filter(|&&b| b == b'"').count() as u64;
            emb_delims += field.iter().filter(|&&b| b == delimiter).count() as u64;
            emb_newlines += field.iter().filter(|&&b| b == b'\n').count() as u64;
            emb_quotes += if quoted {
                quotes.saturating_sub(2)
            } else {
                quotes
            };
        }
        st.fields_per_row.push(fields_in_row);
    }

    st.rows_per_file.push(row_count);
    st.embedded_delims_per_file.push(emb_delims);
    st.embedded_newlines_per_file.push(emb_newlines);
    st.embedded_quotes_per_file.push(emb_quotes);
    st.files += 1;
    st.total_bytes += bytes.len() as u64;
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Escape density = escape characters per KiB of input (one sample per file).
fn push_escape_density(dist: &mut Dist, escapes: u64, bytes: usize) {
    if bytes == 0 {
        return;
    }
    let per_kib = (escapes as f64) * 1024.0 / (bytes as f64);
    // store rounded to 2 dp as hundredths so the Dist stays integer & stable
    dist.push((per_kib * 100.0).round() as u64);
}

/// Format a hundredths-encoded density sample summary as `x.xx` values.
fn density_row(metric: &str, dist: &Dist) -> Vec<String> {
    let fmt = |v: u64| format!("{}.{:02}", v / 100, v % 100);
    match dist.summary() {
        Some(s) => vec![
            metric.to_string(),
            "per KiB/file".to_string(),
            s.count.to_string(),
            fmt(s.min),
            fmt(s.p50),
            fmt(s.p90),
            fmt(s.p99),
            fmt(s.max),
        ],
        None => dist_row(metric, "per KiB/file", dist),
    }
}

/// Extension → (format tag, optional DSV delimiter).
fn classify(path: &Path) -> Option<(&'static str, Option<u8>)> {
    match path.extension().and_then(|e| e.to_str()) {
        Some("json" | "geojson" | "ndjson") => Some(("json", None)),
        Some("yaml" | "yml") => Some(("yaml", None)),
        Some("csv") => Some(("dsv", Some(b','))),
        Some("tsv") => Some(("dsv", Some(b'\t'))),
        Some("psv") => Some(("dsv", Some(b'|'))),
        _ => None,
    }
}

/// Recursively collect corpus files under `dir`, sorted for deterministic output.
fn collect_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .with_context(|| format!("reading directory {}", dir.display()))?
        .map(|e| e.map(|e| e.path()))
        .collect::<std::io::Result<_>>()?;
    entries.sort();
    for path in entries {
        if path.is_dir() {
            collect_files(&path, out)?;
        } else if classify(&path).is_some() {
            out.push(path);
        }
    }
    Ok(())
}

/// Derive `(workload, file)` labels from the path relative to the corpus root.
fn labels(root: &Path, path: &Path) -> (String, String) {
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
    (workload, file)
}

// ---------------------------------------------------------------------------
// Report rendering
// ---------------------------------------------------------------------------

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
    let line = |cells: &[String]| {
        let mut s = String::from("|");
        for (i, w) in widths.iter().enumerate() {
            let cell = cells.get(i).map_or("", String::as_str);
            let _ = write!(s, " {cell:<w$} |", w = *w);
        }
        s.push('\n');
        s
    };
    out.push_str(&line(
        &headers.iter().map(|h| (*h).to_string()).collect::<Vec<_>>(),
    ));
    let sep: Vec<String> = widths.iter().map(|w| "-".repeat(*w)).collect();
    out.push_str(&line(&sep));
    for row in rows {
        out.push_str(&line(row));
    }
    out
}

const DIST_HEADERS: &[&str] = &["metric", "unit", "n", "min", "p50", "p90", "p99", "max"];

/// Build the full deterministic markdown report for a corpus root directory.
pub fn generate_report(data_dir: &Path) -> Result<(String, Vec<FileRecord>)> {
    let mut files = Vec::new();
    collect_files(data_dir, &mut files)
        .with_context(|| format!("scanning corpus at {}", data_dir.display()))?;

    let mut corpus = Corpus::default();
    for path in &files {
        let (fmt, delim) = classify(path).expect("filtered by classify");
        let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        let (workload, file) = labels(data_dir, path);
        corpus.inventory.push(FileRecord {
            format: fmt.to_string(),
            workload: workload.clone(),
            file: file.clone(),
            bytes: bytes.len() as u64,
        });
        match fmt {
            "json" => ingest_json(&bytes, &mut corpus.json),
            "yaml" => ingest_yaml(&bytes, &mut corpus.yaml)
                .with_context(|| format!("in {}", path.display()))?,
            "dsv" => ingest_dsv(&bytes, delim.unwrap(), &mut corpus.dsv),
            _ => unreachable!(),
        }
    }

    let mut md = String::new();
    md.push_str("# Real-Workload Corpus — Shape Statistics\n\n");
    md.push_str("> Generated by `succinctly dev bench corpus-stats`. Do not edit by hand.\n\n");
    md.push_str(
        "Distributions are derived from the crate's own semi-index (`JsonIndex` / \
`YamlIndex` / `DsvIndex`), so they measure exactly what the parsers see. Use \
this as the representativeness lookup that #235 Phase 6 requires before any \
performance doc claim: **before optimising for an input shape, confirm that \
shape's size actually appears here** (the check that rejected P5 — real flow \
collections are 10–30 B, not the 64–128 B the micro-benchmark favoured).\n\n",
    );
    md.push_str(
        "`n` is the number of samples (nodes, rows, or files per the `unit`); \
percentiles use nearest-rank.\n\n",
    );

    // Inventory
    md.push_str("## Corpus inventory\n\n");
    let mut inv_rows: Vec<Vec<String>> = corpus
        .inventory
        .iter()
        .map(|r| {
            vec![
                r.format.clone(),
                r.workload.clone(),
                r.file.clone(),
                r.bytes.to_string(),
            ]
        })
        .collect();
    inv_rows.sort();
    if inv_rows.is_empty() {
        md.push_str("_No corpus files found. Run `./scripts/sync-bench-corpus.sh` first._\n\n");
    } else {
        md.push_str(&render_table(
            &["format", "workload", "file", "bytes"],
            &inv_rows,
        ));
        md.push('\n');
    }

    // YAML
    md.push_str("## YAML\n\n");
    md.push_str(&format!(
        "files: {}, total bytes: {}, anchors: {}, aliases: {}, bare-dash items: {}\n\n",
        corpus.yaml.files,
        corpus.yaml.total_bytes,
        corpus.yaml.anchors_per_file.values.iter().sum::<u64>(),
        corpus.yaml.aliases_per_file.values.iter().sum::<u64>(),
        corpus
            .yaml
            .bare_dash_items_per_file
            .values
            .iter()
            .sum::<u64>(),
    ));
    md.push_str(&render_table(
        DIST_HEADERS,
        &[
            dist_row("scalar length", "bytes/scalar", &corpus.yaml.scalar_len),
            dist_row(
                "mapping keys",
                "keys/mapping",
                &corpus.yaml.keys_per_mapping,
            ),
            dist_row(
                "sequence items",
                "items/seq",
                &corpus.yaml.items_per_sequence,
            ),
            dist_row(
                "bare-dash seq items",
                "count/file",
                &corpus.yaml.bare_dash_items_per_file,
            ),
            dist_row(
                "flow-collection size",
                "bytes/flow",
                &corpus.yaml.flow_collection_bytes,
            ),
            dist_row("nesting depth", "levels/node", &corpus.yaml.depth),
            dist_row("anchors", "count/file", &corpus.yaml.anchors_per_file),
            dist_row("aliases", "count/file", &corpus.yaml.aliases_per_file),
            density_row("escape density", &corpus.yaml.escape_density),
        ],
    ));
    md.push('\n');

    // JSON
    md.push_str("## JSON\n\n");
    md.push_str(&format!(
        "files: {}, total bytes: {}\n\n",
        corpus.json.files, corpus.json.total_bytes,
    ));
    md.push_str(&render_table(
        DIST_HEADERS,
        &[
            dist_row("string length", "bytes/string", &corpus.json.string_len),
            dist_row("object keys", "keys/object", &corpus.json.keys_per_object),
            dist_row(
                "array elements",
                "elems/array",
                &corpus.json.elems_per_array,
            ),
            dist_row(
                "collection size",
                "bytes/coll",
                &corpus.json.collection_bytes,
            ),
            dist_row("nesting depth", "levels/node", &corpus.json.depth),
            density_row("escape density", &corpus.json.escape_density),
        ],
    ));
    md.push('\n');

    // DSV
    md.push_str("## DSV\n\n");
    let quoted_pct = if corpus.dsv.total_fields > 0 {
        100.0 * corpus.dsv.quoted_fields as f64 / corpus.dsv.total_fields as f64
    } else {
        0.0
    };
    md.push_str(&format!(
        "files: {}, total bytes: {}, quoted fields: {:.2}% ({}/{})\n\n",
        corpus.dsv.files,
        corpus.dsv.total_bytes,
        quoted_pct,
        corpus.dsv.quoted_fields,
        corpus.dsv.total_fields,
    ));
    md.push_str(&render_table(
        DIST_HEADERS,
        &[
            dist_row("field length", "bytes/field", &corpus.dsv.field_len),
            dist_row("fields per row", "fields/row", &corpus.dsv.fields_per_row),
            dist_row("rows", "rows/file", &corpus.dsv.rows_per_file),
            dist_row(
                "embedded delimiters",
                "count/file",
                &corpus.dsv.embedded_delims_per_file,
            ),
            dist_row(
                "embedded newlines",
                "count/file",
                &corpus.dsv.embedded_newlines_per_file,
            ),
            dist_row(
                "embedded quotes",
                "count/file",
                &corpus.dsv.embedded_quotes_per_file,
            ),
        ],
    ));
    md.push('\n');

    Ok((md, corpus.inventory))
}

// ---------------------------------------------------------------------------
// CLI entry point
// ---------------------------------------------------------------------------

/// Run the corpus-stats subcommand. Returns the process exit code.
pub fn run(
    data_dir: &Path,
    markdown: Option<&Path>,
    output_jsonl: Option<&Path>,
    check: bool,
) -> Result<i32> {
    let (report, records) = generate_report(data_dir)?;

    if let Some(path) = output_jsonl {
        let mut buf = String::new();
        for r in &records {
            buf.push_str(&serde_json::to_string(r)?);
            buf.push('\n');
        }
        std::fs::write(path, buf).with_context(|| format!("writing {}", path.display()))?;
    }

    if check {
        let expected_path = markdown.context(
            "--check requires --markdown <path> to compare the generated report against",
        )?;
        let expected = std::fs::read_to_string(expected_path)
            .with_context(|| format!("reading golden {}", expected_path.display()))?;
        if expected != report {
            eprintln!(
                "error: shape report for {} does not match {} — regenerate with \
`succinctly dev bench corpus-stats --data-dir {} --markdown {}`",
                data_dir.display(),
                expected_path.display(),
                data_dir.display(),
                expected_path.display(),
            );
            return Ok(1);
        }
        eprintln!("shape report up to date with {}", expected_path.display());
        return Ok(0);
    }

    match markdown {
        Some(path) => {
            std::fs::write(path, &report).with_context(|| format!("writing {}", path.display()))?;
            eprintln!("wrote {}", path.display());
        }
        None => print!("{report}"),
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_shapes_are_derived_from_the_index() {
        // {"name":"ab","nested":{"x":1,"y":2},"list":[10,20,30]}
        let doc = br#"{"name":"ab","nested":{"x":1,"y":2},"list":[10,20,30]}"#;
        let mut st = JsonStats::default();
        ingest_json(doc, &mut st);

        // three objects/arrays? root object + nested object + list array = 3 collections
        assert_eq!(st.collection_bytes.values.len(), 3);
        // root object has 3 keys, nested has 2
        let mut keys = st.keys_per_object.values.clone();
        keys.sort_unstable();
        assert_eq!(keys, vec![2, 3]);
        // one array with 3 elements
        assert_eq!(st.elems_per_array.values, vec![3]);
        // value string "ab" has content length 2; keys are strings too
        assert!(st.string_len.values.contains(&2));
        assert_eq!(st.files, 1);
    }

    #[test]
    fn yaml_flow_and_anchor_counts() {
        // flow sequence [1, 2, 3] plus an anchor/alias pair
        let doc = b"base: &a\n  k: v\nuse: *a\nflow: [1, 2, 3]\n";
        let mut st = YamlStats::default();
        ingest_yaml(doc, &mut st).unwrap();

        assert_eq!(st.anchors_per_file.values, vec![1]);
        assert_eq!(st.aliases_per_file.values, vec![1]);
        // exactly one flow collection recorded, of length == "[1, 2, 3]"
        assert_eq!(st.flow_collection_bytes.values.len(), 1);
        assert_eq!(st.flow_collection_bytes.values[0], "[1, 2, 3]".len() as u64);
    }

    #[test]
    fn yaml_bare_dash_items_counted() {
        // Two bare-dash items (the `-` alone on its line) and one inline `- `.
        let doc = b"rules:\n  -\n    a: 1\n  - b: 2\n  -\n    c: 3\n";
        let mut st = YamlStats::default();
        ingest_yaml(doc, &mut st).unwrap();
        assert_eq!(st.bare_dash_items_per_file.values, vec![2]);
        // The sequence itself still reports all three items.
        assert_eq!(st.items_per_sequence.values, vec![3]);
    }

    #[test]
    fn yaml_inline_and_flow_items_are_not_bare_dash() {
        // Inline block items and flow sequences carry no bare-dash indicator.
        let doc = b"block:\n  - one\n  - two\nflow: [1, 2, 3]\nnested:\n  - - deep\n";
        let mut st = YamlStats::default();
        ingest_yaml(doc, &mut st).unwrap();
        assert_eq!(st.bare_dash_items_per_file.values, vec![0]);
    }

    #[test]
    fn yaml_bare_dash_nests_and_counts_empty_items() {
        // A bare dash whose content is itself a bare-dash sequence, plus a trailing
        // empty item (`-` with no content) — which is also a dash alone on its own
        // line, so it counts too.
        let doc = b"outer:\n  -\n    inner:\n      -\n        x: 1\n  -\n";
        let mut st = YamlStats::default();
        ingest_yaml(doc, &mut st).unwrap();
        assert_eq!(st.bare_dash_items_per_file.values, vec![3]);
    }

    #[test]
    fn bare_dash_counts_a_trailing_indicator_at_end_of_input() {
        // A `-` that is the last byte of the file, with no newline after it, is
        // still alone on its line — the "ran out of input" arm of the scan.
        let doc = b"k:\n  -";
        let mut st = YamlStats::default();
        ingest_yaml(doc, &mut st).unwrap();
        assert_eq!(st.bare_dash_items_per_file.values, vec![1]);
    }

    #[test]
    fn bare_dash_ignores_content_that_starts_with_a_dash() {
        // `- -5` anchors the item at the `-` of the negative number, so the
        // "rest of the line is blank" half of the check is what keeps it out.
        let doc = b"neg:\n  - -5\n  -\n    z: 1\n";
        let mut st = YamlStats::default();
        ingest_yaml(doc, &mut st).unwrap();
        assert_eq!(st.bare_dash_items_per_file.values, vec![1]);
    }

    #[test]
    fn bare_dash_undercounts_when_a_comment_follows_the_indicator() {
        // A comment sits between the indicator and the content, so the index
        // anchors the item at the content and it reads as inline. This is the
        // documented undercount, pinned so it stays a deliberate choice.
        let doc = b"k:\n  -  # note\n    a: 1\n";
        let mut st = YamlStats::default();
        ingest_yaml(doc, &mut st).unwrap();
        assert_eq!(st.bare_dash_items_per_file.values, vec![0]);
    }

    #[test]
    fn dsv_quoting_and_embedded_detection() {
        // second field is quoted and embeds a comma
        let doc = b"a,b,c\n1,\"x,y\",3\n";
        let mut st = DsvStats::default();
        ingest_dsv(doc, b',', &mut st);

        assert_eq!(st.rows_per_file.values, vec![2]);
        assert_eq!(st.fields_per_row.values, vec![3, 3]);
        assert_eq!(st.quoted_fields, 1);
        assert_eq!(st.embedded_delims_per_file.values, vec![1]); // the comma inside "x,y"
    }

    #[test]
    fn flow_extent_balances_and_skips_quotes() {
        // simple
        assert_eq!(flow_extent(b"[1, 2, 3]", 0), Some(9));
        // nested flow
        assert_eq!(flow_extent(b"[[1], [2]]", 0), Some(10));
        // a bracket inside a double-quoted scalar must not close the collection
        assert_eq!(flow_extent(br#"["a]b", "c"]"#, 0), Some(12));
        // flow mapping
        assert_eq!(flow_extent(b"{a: 1, b: 2}", 0), Some(12));
        // not a flow opener
        assert_eq!(flow_extent(b"x: 1", 0), None);
        // unterminated
        assert_eq!(flow_extent(b"[1, 2", 0), None);
    }

    #[test]
    fn percentile_is_nearest_rank() {
        let sorted = [1u64, 2, 3, 4, 5, 6, 7, 8, 9, 10];
        assert_eq!(Dist::percentile(&sorted, 50.0), 5);
        assert_eq!(Dist::percentile(&sorted, 90.0), 9);
        assert_eq!(Dist::percentile(&sorted, 100.0), 10);
        assert_eq!(Dist::percentile(&sorted, 0.0), 1);
    }

    // --- report-generation path -------------------------------------------

    /// Write `content` to `root/rel`, creating parent directories.
    fn write_file(root: &Path, rel: &str, content: &[u8]) -> PathBuf {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, content).unwrap();
        p
    }

    #[test]
    fn generate_report_covers_all_formats() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        write_file(root, "yaml/svc/app.yaml", b"name: web\nports: [80, 443]\n");
        write_file(
            root,
            "json/api/resp.json",
            br#"{"a":"x","b":[1,2,3],"c":{"d":true}}"#,
        );
        write_file(root, "dsv/tab/data.csv", b"h1,h2\n1,\"x,y\"\n3,4\n");
        // a non-corpus extension is ignored by the walk
        write_file(root, "notes/readme.txt", b"ignore me");

        let (md, records) = generate_report(root).unwrap();

        assert!(md.contains("## Corpus inventory"));
        assert!(md.contains("## YAML") && md.contains("## JSON") && md.contains("## DSV"));
        assert!(md.contains("flow-collection size")); // YAML flow row rendered
        assert!(md.contains("quoted fields:")); // DSV summary line rendered
        assert!(md.contains("| json   |")); // inventory row for the json file

        // .txt ignored; three corpus files, workload = parent-dir basename
        assert_eq!(records.len(), 3);
        assert!(records
            .iter()
            .any(|r| r.format == "yaml" && r.workload == "svc" && r.file == "app.yaml"));
        assert!(records
            .iter()
            .any(|r| r.format == "dsv" && r.workload == "tab"));
    }

    #[test]
    fn generate_report_on_empty_dir_reports_no_files() {
        let tmp = tempfile::tempdir().unwrap();
        let (md, records) = generate_report(tmp.path()).unwrap();
        assert!(records.is_empty());
        assert!(md.contains("No corpus files found"));
        // empty distributions still render their section tables (dist_row None path)
        assert!(md.contains("scalar length") && md.contains("field length"));
    }

    #[test]
    fn run_write_then_check_roundtrips() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("corpus");
        write_file(&root, "json/x/a.json", br#"{"k":"v","n":[1,2]}"#);
        let md_path = tmp.path().join("report.md");
        let jsonl_path = tmp.path().join("out.jsonl");

        // write mode + JSONL output
        assert_eq!(
            run(&root, Some(&md_path), Some(&jsonl_path), false).unwrap(),
            0
        );
        assert!(md_path.exists());
        assert!(std::fs::read_to_string(&jsonl_path)
            .unwrap()
            .contains("\"format\":\"json\""));

        // check mode against the just-written golden -> match
        assert_eq!(run(&root, Some(&md_path), None, true).unwrap(), 0);

        // mutate the golden -> mismatch returns exit code 1 (not an error)
        std::fs::write(&md_path, "different\n").unwrap();
        assert_eq!(run(&root, Some(&md_path), None, true).unwrap(), 1);

        // check without --markdown is a usage error
        assert!(run(&root, None, None, true).is_err());
    }

    #[test]
    fn run_stdout_mode_returns_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("c");
        write_file(&root, "yaml/w/a.yaml", b"a: 1\n");
        assert_eq!(run(&root, None, None, false).unwrap(), 0);
    }

    #[test]
    fn classify_maps_extensions() {
        assert_eq!(classify(Path::new("a/b.json")), Some(("json", None)));
        assert_eq!(classify(Path::new("a/b.geojson")), Some(("json", None)));
        assert_eq!(classify(Path::new("a/b.ndjson")), Some(("json", None)));
        assert_eq!(classify(Path::new("a/b.yaml")), Some(("yaml", None)));
        assert_eq!(classify(Path::new("a/b.yml")), Some(("yaml", None)));
        assert_eq!(classify(Path::new("a/b.csv")), Some(("dsv", Some(b','))));
        assert_eq!(classify(Path::new("a/b.tsv")), Some(("dsv", Some(b'\t'))));
        assert_eq!(classify(Path::new("a/b.psv")), Some(("dsv", Some(b'|'))));
        assert_eq!(classify(Path::new("a/b.txt")), None);
        assert_eq!(classify(Path::new("noext")), None);
    }

    #[test]
    fn labels_derive_workload_and_file() {
        let (w, f) = labels(
            Path::new("/corpus"),
            Path::new("/corpus/yaml/k8s/deploy.yaml"),
        );
        assert_eq!(w, "k8s");
        assert_eq!(f, "deploy.yaml");
    }

    #[test]
    fn render_table_pads_and_tolerates_short_rows() {
        let t = render_table(
            &["a", "bb"],
            &[vec!["x".into(), "yy".into()], vec!["z".into()]],
        );
        let lines: Vec<&str> = t.lines().collect();
        assert_eq!(lines.len(), 4); // header, separator, two rows
        assert_eq!(lines[0], "| a | bb |"); // cols padded to max width (1, 2)
        assert_eq!(lines[1], "| - | -- |"); // separator sized per column
        assert_eq!(lines[2], "| x | yy |");
        assert_eq!(lines[3], "| z |    |"); // missing 2nd cell -> blank-padded
    }

    #[test]
    fn density_row_formats_two_decimals() {
        let mut d = Dist::default();
        push_escape_density(&mut d, 3, 1024); // 3 escapes / 1 KiB -> 3.00
        let row = density_row("escape density", &d);
        assert_eq!(row[0], "escape density");
        assert_eq!(row[3], "3.00"); // min
        assert_eq!(row[7], "3.00"); // max

        // zero-byte input contributes no sample
        let mut e = Dist::default();
        push_escape_density(&mut e, 5, 0);
        assert!(e.values.is_empty());
        // empty density falls back to the placeholder row
        assert_eq!(density_row("escape density", &e)[3], "-");
    }

    #[test]
    fn dist_row_empty_uses_placeholder() {
        let row = dist_row("m", "u", &Dist::default());
        assert_eq!(row[2], "0"); // count
        assert_eq!(row[3], "-"); // min placeholder
    }
}
