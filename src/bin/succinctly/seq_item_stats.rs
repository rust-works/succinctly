//! Seq-item density and position-storage statistics (#106).
//!
//! Optimization O4 removed the stored `seq_items` bitvector from `YamlIndex`, so
//! seq-item wrapper nodes are now derived from text. Issue #106 asked whether the
//! O(1) lookup should be restored either by stealing a high bit from the `u32`
//! text position (Option 2) or by a sparse sorted `Vec<u32>` with binary search
//! (Option 3). Both proposals depend on numbers nobody had measured:
//!
//! - **Option 3** trades `4 * S` bytes for an `O(log S)` probe, so it needs the
//!   seq-item share of nodes to sit below a break-even density.
//! - **Option 2** can only tag a position where a position is physically stored
//!   as a `u32`, i.e. the `OpenPositions::Dense` fallback. The `Compact`
//!   (`AdvancePositions`) representation stores positions as interest bits keyed
//!   by text offset, shared between duplicate opens, with no per-open word to
//!   tag. So Option 2's reach is exactly the share of files on `Dense`.
//!
//! This reports both, plus two correctness invariants on the text predicate: it
//! must never call a non-item a wrapper (false positive, which would unwrap the
//! wrong node), and never miss a wrapper that has content beneath it.
//!
//! Both must be zero. Note that "block-sequence child" is *not* the same as
//! "wrapper": the parser emits a wrapper node only for items with structured
//! content, so a plain `- foo` makes the scalar a direct child of the sequence
//! with its text starting past the `- `. A sequence can therefore mix wrappers
//! and wrapperless scalars, which is why wrapper-ness cannot be cached per
//! sequence and must stay a per-element test.
//!
//! Deliberately **not** wired into a golden or CI. `corpus-stats` owns the
//! durable representativeness contract and is byte-exact-checked; these numbers
//! are decision inputs for one issue, and `heap_size()` is implementation-coupled
//! (it moves if `SELECT_SAMPLE_RATE` changes), which would turn a required CI job
//! into an internal-layout tripwire.

use anyhow::{Context, Result};
use serde::Serialize;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use succinctly::yaml::YamlIndex;

use crate::corpus_stats::{collect_files, labels, render_table};

/// Per-file seq-item and storage measurements.
#[derive(Debug, Serialize)]
pub struct FileStats {
    pub workload: String,
    pub file: String,
    /// Text length in bytes (`L`).
    pub bytes: usize,
    /// Total BP opens (`N`) — one per node.
    pub opens: usize,
    /// Container nodes (`C`), which are never seq-item wrappers.
    pub containers: usize,
    /// Nodes the text predicate calls seq-item wrappers (`S`).
    pub seq_items: usize,
    /// Direct children of a block sequence (wrappers *and* wrapperless scalars).
    pub block_seq_children: usize,
    /// Predicate says wrapper, but the node is not a block-sequence child.
    /// This is the dangerous direction: it would unwrap something that is not an
    /// item. Must be 0.
    pub false_positives: usize,
    /// A block-sequence child with its own children that the predicate did *not*
    /// call a wrapper. Must be 0.
    pub undetected_wrappers: usize,
    /// Opens whose stored position is the `text.len()` null sentinel.
    pub sentinel_opens: usize,
    /// True if `OpenPositions` chose the compact Advance Index encoding.
    pub open_compact: bool,
    /// Actual heap bytes held by `OpenPositions`.
    pub open_heap_bytes: usize,
}

impl FileStats {
    /// Seq-item share of nodes — the quantity Option 3's break-even is stated in.
    fn density(&self) -> f64 {
        if self.opens == 0 {
            0.0
        } else {
            self.seq_items as f64 / self.opens as f64
        }
    }

    /// Bytes a sparse `Vec<u32>` of seq-item positions would retain.
    fn sparse_bytes(&self) -> usize {
        4 * self.seq_items
    }

    /// Sparse cost normalised per MiB of input, for comparison against the
    /// 19-41 KB/MB that eliminating the bitvector saved.
    fn sparse_kb_per_mib(&self) -> f64 {
        if self.bytes == 0 {
            0.0
        } else {
            (self.sparse_bytes() as f64) * (1_048_576.0 / self.bytes as f64) / 1024.0
        }
    }
}

/// Corpus-wide rollup.
#[derive(Default)]
struct Totals {
    files: usize,
    bytes: usize,
    opens: usize,
    containers: usize,
    seq_items: usize,
    block_seq_children: usize,
    false_positives: usize,
    undetected_wrappers: usize,
    compact: usize,
    dense: usize,
    open_heap: usize,
    max_density: f64,
    max_density_file: String,
}

/// Measure one YAML document.
///
/// Single pass over BP positions accumulating both the predicate count and the
/// structural count, so the two can be compared without a second traversal.
pub fn measure(text: &[u8], workload: String, file: String) -> Result<FileStats> {
    let index = YamlIndex::build(text).context("building YAML index")?;
    let bp = index.bp();

    let mut st = FileStats {
        workload,
        file,
        bytes: text.len(),
        opens: bp.total_ones(),
        containers: 0,
        seq_items: 0,
        block_seq_children: 0,
        false_positives: 0,
        undetected_wrappers: 0,
        sentinel_opens: 0,
        open_compact: index.open_positions().is_compact(),
        open_heap_bytes: index.open_positions().heap_size(),
    };

    for p in 0..bp.len() {
        if !bp.is_open(p) {
            continue;
        }

        let is_container = index.is_container(p);
        if is_container {
            st.containers += 1;
        }

        if index.bp_to_text_pos(p) == Some(text.len()) {
            st.sentinel_opens += 1;
        }

        let by_text = index.is_seq_item(text, p);

        // Is this a direct child of a *block* sequence? The virtual root
        // (bp_pos 0) is excluded — documents are its children, not items. Flow
        // sequences are excluded via the container's first byte, which for a
        // block sequence is its first `-`.
        //
        // Note this is NOT the same as "is a wrapper". The parser emits a wrapper
        // node only when the item has structured content; a plain scalar item like
        // `- foo` becomes a direct child of the sequence whose text position points
        // at `foo`, past the `- `. So block-sequence children legitimately split
        // into wrappers and wrapperless scalars, and any caching of "these elements
        // are wrappers" across a sequence is unsound — `- scalar` followed by a
        // mapping item mixes both in one sequence.
        let is_block_seq_child = match bp.parent(p) {
            Some(q) if q != 0 && index.is_sequence_at_bp(q) => index
                .bp_to_text_pos(q)
                .is_some_and(|qp| text.get(qp).copied() != Some(b'[')),
            _ => false,
        };
        // A wrapper has content beneath it; `first_child` exists iff bp[p+1] opens.
        let has_child = p + 1 < bp.len() && bp.is_open(p + 1);

        if by_text {
            st.seq_items += 1;
        }
        if is_block_seq_child {
            st.block_seq_children += 1;
        }
        if by_text && !is_block_seq_child {
            st.false_positives += 1;
        }
        if is_block_seq_child && !is_container && has_child && !by_text {
            st.undetected_wrappers += 1;
        }
    }

    Ok(st)
}

/// Break-even densities, derived rather than asserted.
///
/// A sparse list costs `4 * S` bytes. A dense bitvector indexed by BP open costs
/// `N / 8` bytes (1 bit per node); indexed by BP position it costs `2N / 8 = N / 4`
/// (2 bits per node), which is the form O4 actually removed. Setting the two equal:
///
/// - `4S = N/8`  ->  `S/N = 1/32` = 3.125%
/// - `4S = N/4`  ->  `S/N = 1/16` = 6.25%
///
/// Note the issue itself states 12.5%, which drops the bits-to-bytes conversion.
const BREAK_EVEN_VS_BIT_PER_NODE: f64 = 1.0 / 32.0;
const BREAK_EVEN_VS_TWO_BITS_PER_NODE: f64 = 1.0 / 16.0;

fn pct(x: f64) -> String {
    format!("{:.2}%", x * 100.0)
}

pub fn generate_report(data_dir: &Path) -> Result<(String, Vec<FileStats>)> {
    let mut paths = Vec::new();
    collect_files(data_dir, &mut paths)?;
    paths.sort();

    let mut stats = Vec::new();
    for path in &paths {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if ext != "yaml" && ext != "yml" {
            continue;
        }
        let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        let (workload, file) = labels(data_dir, path);
        match measure(&bytes, workload, file) {
            Ok(s) => stats.push(s),
            Err(e) => eprintln!("  skipping {}: {e}", path.display()),
        }
    }

    let mut t = Totals::default();
    for s in &stats {
        t.files += 1;
        t.bytes += s.bytes;
        t.opens += s.opens;
        t.containers += s.containers;
        t.seq_items += s.seq_items;
        t.block_seq_children += s.block_seq_children;
        t.false_positives += s.false_positives;
        t.undetected_wrappers += s.undetected_wrappers;
        t.open_heap += s.open_heap_bytes;
        if s.open_compact {
            t.compact += 1;
        } else {
            t.dense += 1;
        }
        if s.density() > t.max_density {
            t.max_density = s.density();
            t.max_density_file = format!("{}/{}", s.workload, s.file);
        }
    }
    let agg_density = if t.opens == 0 {
        0.0
    } else {
        t.seq_items as f64 / t.opens as f64
    };

    let mut out = String::new();
    out.push_str("# Seq-Item Density and Position Storage (#106)\n\n");
    out.push_str(
        "Decision inputs for issue #106 (restore an O(1) seq-item lookup?). Generated by\n\
         `succinctly dev bench seq-item-stats`; no golden, no CI gate — see the module\n\
         docs for why.\n\n",
    );
    writeln!(
        out,
        "Scanned {} YAML file(s), {} bytes, {} nodes.\n",
        t.files, t.bytes, t.opens
    )?;

    // Per-file density
    out.push_str("## Seq-item density\n\n");
    out.push_str(
        "`S` = nodes the text predicate calls seq-item wrappers, `N` = total nodes\n\
         (BP opens). Option 3 stores one `u32` per seq-item node, so `S/N` is the\n\
         ratio its break-even is stated in.\n\n",
    );
    let mut rows = Vec::new();
    for s in &stats {
        rows.push(vec![
            format!("{}/{}", s.workload, s.file),
            s.bytes.to_string(),
            s.opens.to_string(),
            s.containers.to_string(),
            s.seq_items.to_string(),
            pct(s.density()),
            format!("{:.1}", s.bytes as f64 / s.opens.max(1) as f64),
        ]);
    }
    out.push_str(&render_table(
        &[
            "file",
            "bytes",
            "N nodes",
            "containers",
            "S seq-items",
            "density S/N",
            "bytes/node",
        ],
        &rows,
    ));
    out.push('\n');
    writeln!(
        out,
        "- **Aggregate density: {} ({} / {})**",
        pct(agg_density),
        t.seq_items,
        t.opens
    )?;
    writeln!(
        out,
        "- **Per-file maximum: {} ({})**",
        pct(t.max_density),
        if t.max_density_file.is_empty() {
            "n/a"
        } else {
            &t.max_density_file
        }
    )?;
    out.push('\n');

    // Correctness invariant
    out.push_str("## Predicate vs structure\n\n");
    out.push_str(
        "Block-sequence children split into *wrappers* (structured content beneath a\n\
         `-`) and *wrapperless scalars* (`- foo`, where the parser makes the scalar a\n\
         direct child whose text starts past the `- `). So `children > S` is expected\n\
         and correct, and no per-sequence caching of wrapper-ness is sound.\n\n\
         The two numbers that must be zero: a false positive would unwrap a non-item,\n\
         and an undetected wrapper would leak a wrapper node to a caller.\n\n",
    );
    let mut rows = Vec::new();
    for s in &stats {
        rows.push(vec![
            format!("{}/{}", s.workload, s.file),
            s.seq_items.to_string(),
            s.block_seq_children.to_string(),
            (s.block_seq_children as i64 - s.seq_items as i64).to_string(),
            s.false_positives.to_string(),
            s.undetected_wrappers.to_string(),
            s.sentinel_opens.to_string(),
        ]);
    }
    out.push_str(&render_table(
        &[
            "file",
            "S wrappers",
            "block-seq children",
            "wrapperless",
            "false pos",
            "undetected",
            "null sentinels",
        ],
        &rows,
    ));
    writeln!(
        out,
        "\n- **False positives: {}; undetected wrappers: {}**{}\n\
         - Wrapperless scalar items: {} (expected, not a defect)\n",
        t.false_positives,
        t.undetected_wrappers,
        if t.false_positives == 0 && t.undetected_wrappers == 0 {
            " — predicate is exact"
        } else {
            " — INVESTIGATE"
        },
        t.block_seq_children as i64 - t.seq_items as i64
    )?;

    // Storage representation
    out.push_str("## Position storage: Compact vs Dense\n\n");
    out.push_str(
        "Option 2 can only tag a high bit where a position is physically stored as a\n\
         `u32`, i.e. `OpenPositions::Dense`. `Compact` holds positions as interest bits\n\
         keyed by text offset (shared between duplicate opens) plus a per-open advance\n\
         bitmap, so there is no per-open word to tag.\n\n",
    );
    let mut rows = Vec::new();
    for s in &stats {
        rows.push(vec![
            format!("{}/{}", s.workload, s.file),
            if s.open_compact { "Compact" } else { "Dense" }.to_string(),
            s.open_heap_bytes.to_string(),
            (4 * s.opens).to_string(),
            format!(
                "{:.2}x",
                (4 * s.opens) as f64 / s.open_heap_bytes.max(1) as f64
            ),
        ]);
    }
    out.push_str(&render_table(
        &[
            "file",
            "representation",
            "heap bytes",
            "dense equiv (4N)",
            "compression",
        ],
        &rows,
    ));
    writeln!(
        out,
        "\n- **Compact: {} file(s); Dense: {} file(s)** — Option 2 reaches only the Dense share.\n",
        t.compact, t.dense
    )?;

    // Option 3 sizing
    out.push_str("## Option 3 sizing (sparse `Vec<u32>`)\n\n");
    writeln!(
        out,
        "Break-even: **{}** against a 1-bit-per-node bitvector, **{}** against the\n\
         2-bits-per-BP-position form O4 removed. The issue states 12.5%, which drops\n\
         the bits-to-bytes conversion (`4S = N/8` gives 1/32, not 1/8).\n\n\
         The baseline today is **0 bytes** — O4 deleted the structure — so this is a\n\
         pure retained-memory addition, not a cheaper encoding.\n",
        pct(BREAK_EVEN_VS_BIT_PER_NODE),
        pct(BREAK_EVEN_VS_TWO_BITS_PER_NODE)
    )?;
    out.push('\n');
    let mut rows = Vec::new();
    for s in &stats {
        rows.push(vec![
            format!("{}/{}", s.workload, s.file),
            pct(s.density()),
            s.sparse_bytes().to_string(),
            format!("{:.1}", s.sparse_kb_per_mib()),
            if s.density() < BREAK_EVEN_VS_BIT_PER_NODE {
                "under"
            } else {
                "OVER"
            }
            .to_string(),
        ]);
    }
    out.push_str(&render_table(
        &[
            "file",
            "density",
            "sparse bytes (4S)",
            "KB per MiB",
            "vs 3.125%",
        ],
        &rows,
    ));
    out.push('\n');

    let verdict_o3 =
        agg_density < BREAK_EVEN_VS_BIT_PER_NODE && t.max_density < BREAK_EVEN_VS_TWO_BITS_PER_NODE;
    writeln!(
        out,
        "## Verdict\n\n\
         - Option 2 (high-bit tag): reaches {} of {} file(s) (the Dense share). {}\n\
         - Option 3 (sparse index): aggregate {} vs {} break-even, max {} vs {}. {}\n",
        t.dense,
        t.files,
        if t.dense == 0 {
            "**Infeasible** — no file stores a taggable per-open `u32`."
        } else {
            "Partially applicable; needs a second mechanism for Compact files."
        },
        pct(agg_density),
        pct(BREAK_EVEN_VS_BIT_PER_NODE),
        pct(t.max_density),
        pct(BREAK_EVEN_VS_TWO_BITS_PER_NODE),
        if verdict_o3 {
            "**Passes** the density gate; proceed to a benchmark."
        } else {
            "**Fails** the density gate."
        }
    )?;

    Ok((out, stats))
}

pub fn run(data_dir: &Path, markdown: Option<&Path>, output_jsonl: Option<&Path>) -> Result<i32> {
    let (report, stats) = generate_report(data_dir)?;

    if let Some(path) = output_jsonl {
        let mut buf = String::new();
        for s in &stats {
            buf.push_str(&serde_json::to_string(s)?);
            buf.push('\n');
        }
        std::fs::write(path, buf).with_context(|| format!("writing {}", path.display()))?;
    }

    match markdown {
        Some(path) => {
            std::fs::write(path, &report).with_context(|| format!("writing {}", path.display()))?;
            println!("wrote {}", path.display());
        }
        None => print!("{report}"),
    }

    Ok(0)
}

/// Measure the vendored YAML Test Suite: the only corpus in the repo where the
/// `Dense` fallback is reachable, since it needs non-monotonic opens from
/// explicit `?` keys.
pub fn run_test_suite(path: &Path) -> Result<i32> {
    let raw =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let cases: Vec<serde_json::Value> = serde_json::from_str(&raw)?;

    let mut parsed = 0usize;
    let mut dense = Vec::new();
    let mut mismatched = Vec::new();
    let mut opens = 0usize;
    let mut seq_items = 0usize;

    for case in &cases {
        if case.get("fail").and_then(serde_json::Value::as_bool) == Some(true) {
            continue;
        }
        let Some(yaml) = case.get("yaml").and_then(|v| v.as_str()) else {
            continue;
        };
        let id = case
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("?")
            .to_string();
        let Ok(s) = measure(yaml.as_bytes(), "suite".into(), id.clone()) else {
            continue;
        };
        parsed += 1;
        opens += s.opens;
        seq_items += s.seq_items;
        if !s.open_compact {
            dense.push(id.clone());
        }
        if s.false_positives > 0 || s.undetected_wrappers > 0 {
            mismatched.push((id, s.false_positives + s.undetected_wrappers));
        }
    }

    println!("YAML Test Suite: {parsed} valid case(s) indexed");
    println!("  nodes: {opens}, seq-items: {seq_items}");
    if opens > 0 {
        println!("  density: {}", pct(seq_items as f64 / opens as f64));
    }
    println!(
        "  OpenPositions::Dense: {} case(s){}",
        dense.len(),
        if dense.is_empty() {
            String::new()
        } else {
            format!(" -> {:?}", &dense[..dense.len().min(15)])
        }
    );
    println!(
        "  predicate/structure mismatches: {} case(s)",
        mismatched.len()
    );
    for (id, n) in mismatched.iter().take(15) {
        println!("    {id}: {n}");
    }

    Ok(0)
}

/// Print every node where the text predicate and the structural definition
/// disagree, with text context, so a mismatch can be diagnosed rather than
/// guessed at.
pub fn explain_mismatch(text: &[u8], label: &str) -> Result<()> {
    let index = YamlIndex::build(text).context("building YAML index")?;
    let bp = index.bp();

    let snippet = |pos: Option<usize>| -> String {
        match pos {
            Some(p) if p < text.len() => {
                let end = (p + 24).min(text.len());
                format!("{:?}", String::from_utf8_lossy(&text[p..end]))
            }
            Some(p) => format!("<sentinel @{p} == len {}>", text.len()),
            None => "<none>".to_string(),
        }
    };

    println!("=== {label} ===");
    for p in 0..bp.len() {
        if !bp.is_open(p) {
            continue;
        }
        let is_container = index.is_container(p);
        let by_text = index.is_seq_item(text, p);
        let parent = bp.parent(p);
        let is_block_seq_child = match parent {
            Some(q) if q != 0 && index.is_sequence_at_bp(q) => index
                .bp_to_text_pos(q)
                .is_some_and(|qp| text.get(qp).copied() != Some(b'[')),
            _ => false,
        };
        let has_child = p + 1 < bp.len() && bp.is_open(p + 1);
        let fp = by_text && !is_block_seq_child;
        let undetected = is_block_seq_child && !is_container && has_child && !by_text;
        if !fp && !undetected {
            continue;
        }
        let by_struct = is_block_seq_child;
        println!(
            "  bp={p:<5} text={:<8} container={is_container:<5} text_pred={by_text:<5} struct={by_struct:<5}",
            format!("{:?}", index.bp_to_text_pos(p))
        );
        println!("      self  : {}", snippet(index.bp_to_text_pos(p)));
        match parent {
            Some(q) => println!(
                "      parent: bp={q} seq={} {}",
                index.is_sequence_at_bp(q),
                snippet(index.bp_to_text_pos(q))
            ),
            None => println!("      parent: <none>"),
        }
    }
    Ok(())
}

/// Convenience wrapper so callers can pass an optional suite path.
pub fn run_all(
    data_dir: &Path,
    markdown: Option<&Path>,
    output_jsonl: Option<&Path>,
    test_suite: Option<&PathBuf>,
    explain: bool,
) -> Result<i32> {
    if explain {
        let mut paths = Vec::new();
        collect_files(data_dir, &mut paths)?;
        paths.sort();
        for path in &paths {
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or_default()
                .to_ascii_lowercase();
            if ext != "yaml" && ext != "yml" {
                continue;
            }
            let bytes = std::fs::read(path)?;
            explain_mismatch(&bytes, &path.display().to_string())?;
        }
        return Ok(0);
    }
    let code = run(data_dir, markdown, output_jsonl)?;
    if let Some(suite) = test_suite {
        println!();
        run_test_suite(suite)?;
    }
    Ok(code)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every shape below was measured, not assumed. The point of these tests is to
    /// pin the two invariants, and to record that wrapper emission is
    /// context-dependent -- which is why wrapper-ness cannot be hoisted to the
    /// sequence level.
    fn m(yaml: &[u8]) -> FileStats {
        measure(yaml, "t".into(), "t".into()).unwrap()
    }

    #[test]
    fn top_level_items_have_wrappers() {
        let s = m(b"- a\n- b\n");
        assert_eq!((s.seq_items, s.block_seq_children), (2, 2));
        assert_eq!((s.false_positives, s.undetected_wrappers), (0, 0));
    }

    #[test]
    fn bare_dash_items_have_wrappers() {
        let s = m(b"-\n  a: 1\n-\n  a: 2\n");
        assert_eq!((s.seq_items, s.block_seq_children), (2, 2));
        assert_eq!((s.false_positives, s.undetected_wrappers), (0, 0));
    }

    #[test]
    fn scalar_and_mapping_items_in_one_sequence() {
        let s = m(b"- scalar\n-\n  a: 1\n");
        assert_eq!((s.seq_items, s.block_seq_children), (2, 2));
        assert_eq!((s.false_positives, s.undetected_wrappers), (0, 0));
    }

    #[test]
    fn flow_sequences_have_no_wrappers() {
        // `-1` and `-2` start with `-` but the next byte is a digit, so the
        // predicate must not treat them as indicators.
        let s = m(b"[-1, -2]\n");
        assert_eq!((s.seq_items, s.block_seq_children), (0, 0));
        assert_eq!(s.false_positives, 0);
    }

    #[test]
    fn wrapper_emission_is_context_dependent() {
        // Two sibling sequences under one mapping, the first followed by a dedent.
        // This real docker-compose shape yields three block-sequence children but
        // only two wrappers, so "all children of a block sequence are wrappers" is
        // false and per-sequence caching of wrapper-ness would be unsound (#106).
        let s = m(b"services:\n  db:\n    secrets:\n      - db-password\n    volumes:\n      - db-data:/var\n");
        assert_eq!(s.block_seq_children, 3);
        assert_eq!(s.seq_items, 2);
        // Still exact: the extra child is a leaf, not a missed wrapper.
        assert_eq!((s.false_positives, s.undetected_wrappers), (0, 0));
    }

    #[test]
    fn invariants_hold_across_assorted_shapes() {
        for yaml in [
            &b"a:\n  - x\n  - y\n"[..],
            b"- a: 1\n- b: 2\n",
            b"---\n- a\n---\n- b\n",
            b"? -\n: v\n",
            b"- - a\n  - b\n",
            b"k: {a: 1, b: [1, 2]}\n",
            b"- a\n-",
            b"- \n- b\n",
            b"-\ta\n",
        ] {
            let s = m(yaml);
            assert_eq!(
                (s.false_positives, s.undetected_wrappers),
                (0, 0),
                "invariant violated for {:?}",
                String::from_utf8_lossy(yaml)
            );
        }
    }

    #[test]
    fn break_even_arithmetic_is_derived_not_asserted() {
        // 4S = N/8  =>  S/N = 1/32. The issue states 12.5%, which drops the /8.
        assert!((BREAK_EVEN_VS_BIT_PER_NODE - 0.03125).abs() < 1e-12);
        assert!((BREAK_EVEN_VS_TWO_BITS_PER_NODE - 0.0625).abs() < 1e-12);
    }

    #[test]
    fn real_corpus_files_use_the_compact_representation() {
        // Option 2 could only tag a per-open u32, which only Dense holds.
        let s = m(b"services:\n  db:\n    image: x\n    ports:\n      - 80:80\n");
        assert!(s.open_compact, "typical YAML should reach the compact encoding");
    }
}
