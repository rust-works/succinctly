//! Measures real input-string length and escape-density distributions for
//! jq's format functions (`@uri`, `@html`, `@csv`, `@dsv`, `@sh`) against this
//! repo's pinned real-workload corpus (#301) plus two small supplementary
//! real corpora (shell-argument literals from `scripts/*.sh`, URLs from
//! `docs/**/*.md`), to answer issue #663: does any format's realistic input
//! clear the ~32-byte AVX2 SIMD win threshold established by ADR-0007/ADR-0013?
//!
//! Measurement only -- no production code changes. The escape predicates
//! below are hand-copied (not imported) from `src/jq/eval.rs`'s
//! `format_uri`/`format_html`/`quote_csv_field`/`format_sh`, since this is a
//! throwaway analysis tool, not a dependency of the format functions.
//!
//! Run with:
//! ```bash
//! ./scripts/sync-bench-corpus.sh   # populate data/bench/corpus (network, once)
//! cargo run --release --example jq_format_shape
//! ```

use std::path::{Path, PathBuf};
use succinctly::dsv::{build_index as build_dsv_index, DsvConfig, DsvRows};
use succinctly::json::light::{JsonCursor, StandardJson};
use succinctly::json::JsonIndex;
use succinctly::yaml::{YamlCursor, YamlIndex, YamlValue};

// ---------------------------------------------------------------------------
// Distribution helper
// ---------------------------------------------------------------------------

#[derive(Default)]
struct Dist {
    values: Vec<u64>,
}

impl Dist {
    fn push(&mut self, v: u64) {
        self.values.push(v);
    }

    fn percentile(&self, p: f64) -> u64 {
        if self.values.is_empty() {
            return 0;
        }
        let mut sorted = self.values.clone();
        sorted.sort_unstable();
        let n = sorted.len();
        let rank = ((p / 100.0) * n as f64).ceil() as usize;
        sorted[rank.clamp(1, n) - 1]
    }

    fn frac_ge(&self, threshold: u64) -> f64 {
        if self.values.is_empty() {
            return 0.0;
        }
        100.0 * self.values.iter().filter(|&&v| v >= threshold).count() as f64
            / self.values.len() as f64
    }

    fn min(&self) -> u64 {
        self.values.iter().copied().min().unwrap_or(0)
    }

    fn max(&self) -> u64 {
        self.values.iter().copied().max().unwrap_or(0)
    }
}

// ---------------------------------------------------------------------------
// Escape predicates -- mirrored from src/jq/eval.rs, NOT imported (this is a
// throwaway measurement tool; see module doc).
// ---------------------------------------------------------------------------

/// `format_uri` (eval.rs): everything except ASCII alnum and `-_.~` is escaped.
fn is_uri_safe(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~')
}

/// `format_html` (eval.rs): five single-byte entities.
fn is_html_safe(b: u8) -> bool {
    !matches!(b, b'<' | b'>' | b'&' | b'"' | b'\'')
}

/// `quote_csv_field` (eval.rs, shared by `format_csv`/`format_dsv`): every
/// string is unconditionally wrapped in `"..."` (a fixed cost, not scanned
/// for); the only byte the scan itself must find is an embedded `"`.
fn is_csv_safe(b: u8) -> bool {
    b != b'"'
}

/// `format_sh` (eval.rs): every string is unconditionally wrapped in `'...'`;
/// the only byte the scan must find is an embedded `'`.
fn is_sh_safe(b: u8) -> bool {
    b != b'\''
}

/// Escape count and longest contiguous safe-byte run for `s` under `safe`.
/// The run length is the SIMD-relevant quantity: a bulk-copy scan only wins
/// on runs long enough to fill a lane, regardless of overall string length.
fn analyze(s: &str, safe: impl Fn(u8) -> bool) -> (u64, u64) {
    let mut escapes = 0u64;
    let mut longest_run = 0u64;
    let mut run = 0u64;
    for &b in s.as_bytes() {
        if safe(b) {
            run += 1;
        } else {
            escapes += 1;
            longest_run = longest_run.max(run);
            run = 0;
        }
    }
    (escapes, longest_run.max(run))
}

#[derive(Default)]
struct FormatStats {
    len: Dist,
    escapes: Dist,
    safe_run: Dist,
    zero_escape: u64,
    n: u64,
}

impl FormatStats {
    fn record(&mut self, s: &str, safe: impl Fn(u8) -> bool) {
        self.len.push(s.len() as u64);
        let (escapes, run) = analyze(s, safe);
        self.escapes.push(escapes);
        self.safe_run.push(run);
        self.n += 1;
        if escapes == 0 {
            self.zero_escape += 1;
        }
    }

    fn row(&self, name: &str) -> String {
        if self.n == 0 {
            return format!("| {name} | 0 | - | - | - | - | - | - | - | - | - |");
        }
        format!(
            "| {name} | {} | {} | {} | {} | {} | {} | {:.1}% | {:.1}% | {:.1}% | {:.1}% |",
            self.n,
            self.len.min(),
            self.len.percentile(50.0),
            self.len.percentile(90.0),
            self.len.percentile(99.0),
            self.len.max(),
            self.len.frac_ge(16),
            self.len.frac_ge(32),
            100.0 * self.zero_escape as f64 / self.n as f64,
            self.safe_run.frac_ge(32),
        )
    }
}

const STATS_HEADER: &str = "| format | n | min len | p50 len | p90 len | p99 len | max len | len>=16B | len>=32B | 0-escape strings | longest safe run >=32B |\n\
| ------ | - | ------- | ------- | ------- | ------- | ------- | -------- | -------- | ----------------- | ----------------------- |";

// ---------------------------------------------------------------------------
// Corpus walking
// ---------------------------------------------------------------------------

fn walk_files(root: &Path, exts: &[&str]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    fn rec(dir: &Path, exts: &[&str], out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        let mut entries: Vec<_> = entries.flatten().map(|e| e.path()).collect();
        entries.sort();
        for p in entries {
            if p.is_dir() {
                let skip = p
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n == "target" || n == ".git");
                if !skip {
                    rec(&p, exts, out);
                }
            } else if p
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| exts.contains(&e))
            {
                out.push(p);
            }
        }
    }
    rec(root, exts, &mut out);
    out
}

fn visit_json(cur: JsonCursor<'_, Vec<u64>>, out: &mut Vec<String>) {
    match cur.value() {
        StandardJson::String(js) => {
            if let Ok(s) = js.as_str() {
                out.push(s.into_owned());
            }
        }
        StandardJson::Object(_) => {
            let mut child = cur.first_child();
            while let Some(k) = child {
                visit_json(k, out); // key
                match k.next_sibling() {
                    Some(v) => {
                        visit_json(v, out); // value
                        child = v.next_sibling();
                    }
                    None => child = None,
                }
            }
        }
        StandardJson::Array(_) => {
            for elem in cur.children() {
                visit_json(elem, out);
            }
        }
        _ => {}
    }
}

fn visit_yaml(cur: YamlCursor<'_, Vec<u64>>, out: &mut Vec<String>) {
    match cur.value() {
        YamlValue::String(ys) => {
            if let Ok(s) = ys.as_str() {
                out.push(s.into_owned());
            }
        }
        YamlValue::Mapping(fields) => {
            for field in fields {
                if let YamlValue::String(ks) = field.key() {
                    if let Ok(s) = ks.as_str() {
                        out.push(s.into_owned());
                    }
                }
                visit_yaml(field.value_cursor(), out);
            }
        }
        YamlValue::Sequence(elements) => {
            let mut es = elements;
            while let Some((child, rest)) = es.uncons_cursor() {
                visit_yaml(child, out);
                es = rest;
            }
        }
        _ => {}
    }
}

fn collect_json_strings(bytes: &[u8], out: &mut Vec<String>) {
    let index = JsonIndex::build(bytes);
    visit_json(index.root(bytes), out);
}

fn collect_yaml_strings(bytes: &[u8], out: &mut Vec<String>) {
    let Ok(index) = YamlIndex::build(bytes) else {
        return;
    };
    let root = index.root(bytes);
    match root.value() {
        YamlValue::Sequence(mut docs) => {
            while let Some((doc, rest)) = docs.uncons_cursor() {
                visit_yaml(doc, out);
                docs = rest;
            }
        }
        _ => visit_yaml(root, out),
    }
}

/// Logical (unquoted, un-doubled) field content, matching what a real DSV
/// parse -> jq array -> `@csv`/`@dsv` round-trip would carry as a string value.
fn unquote_dsv_field(field: &[u8]) -> String {
    if field.first() == Some(&b'"') && field.len() >= 2 && field.last() == Some(&b'"') {
        let inner = &field[1..field.len() - 1];
        String::from_utf8_lossy(inner).replace("\"\"", "\"")
    } else {
        String::from_utf8_lossy(field).into_owned()
    }
}

fn collect_dsv_fields(bytes: &[u8], delimiter: u8, out: &mut Vec<String>) {
    let config = DsvConfig::default().with_delimiter(delimiter);
    let index = build_dsv_index(bytes, &config);
    for row in DsvRows::new(bytes, &index) {
        for field in row.fields() {
            out.push(unquote_dsv_field(field));
        }
    }
}

/// Extension -> (format, optional DSV delimiter), mirrors
/// `src/bin/succinctly/corpus_stats.rs::classify`.
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

// ---------------------------------------------------------------------------
// Supplementary real corpora: shell-argument literals and URLs already
// present in this repo (not fabricated, not downloaded).
// ---------------------------------------------------------------------------

/// Quoted string literals from this repo's own shell scripts -- real
/// argument-shaped text a human wrote for a real purpose, on-point for `@sh`.
///
/// Scoped to non-comment source lines and matched within a single line: a
/// naive whole-file quote scan mistakes apostrophes inside `#`-comment prose
/// (e.g. "bugs as \"correct\"") for open quotes and swallows hundreds of
/// bytes of unrelated comment text as one "literal" -- verified against this
/// corpus, where it inflated p99 to 1052B before this fix.
fn collect_shell_args(root: &Path, out: &mut Vec<String>) {
    for path in walk_files(root, &["sh"]) {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        for line in text.lines() {
            if line.trim_start().starts_with('#') {
                continue;
            }
            let bytes = line.as_bytes();
            let mut i = 0;
            while i < bytes.len() {
                if bytes[i] == b'"' || bytes[i] == b'\'' {
                    let quote = bytes[i];
                    let start = i + 1;
                    let mut j = start;
                    while j < bytes.len() && bytes[j] != quote {
                        if bytes[j] == b'\\' && quote == b'"' && j + 1 < bytes.len() {
                            j += 1;
                        }
                        j += 1;
                    }
                    if j > start && j <= bytes.len() {
                        if let Ok(s) = std::str::from_utf8(&bytes[start..j]) {
                            if s.len() >= 2 {
                                out.push(s.to_string());
                            }
                        }
                    }
                    i = j + 1;
                } else {
                    i += 1;
                }
            }
        }
    }
}

/// URLs referenced from this repo's own docs -- real, on-point for `@uri`
/// (though skewed toward already-valid URLs; see caveats in the write-up).
fn collect_urls(root: &Path, out: &mut Vec<String>) {
    for path in walk_files(root, &["md"]) {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        for token in text.split(|c: char| {
            c.is_whitespace() || matches!(c, '(' | ')' | '"' | '\'' | '<' | '>' | '[' | ']')
        }) {
            if token.starts_with("http://") || token.starts_with("https://") {
                let t = token.trim_end_matches(['.', ',', ';', ':']);
                if t.len() > 8 {
                    out.push(t.to_string());
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// main
// ---------------------------------------------------------------------------

fn main() {
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let corpus_dir = repo_root.join("data/bench/corpus");
    if !corpus_dir.exists() {
        eprintln!(
            "error: {} not found -- run ./scripts/sync-bench-corpus.sh first",
            corpus_dir.display()
        );
        std::process::exit(1);
    }

    let mut generic_strings = Vec::new(); // JSON string values + YAML scalars
    let mut dsv_fields = Vec::new(); // DSV/CSV row fields (unquoted)
    let mut json_files = 0u64;
    let mut yaml_files = 0u64;
    let mut dsv_files = 0u64;

    for path in walk_files(
        &corpus_dir,
        &[
            "json", "geojson", "ndjson", "yaml", "yml", "csv", "tsv", "psv",
        ],
    ) {
        let Some((fmt, delim)) = classify(&path) else {
            continue;
        };
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        match fmt {
            "json" => {
                collect_json_strings(&bytes, &mut generic_strings);
                json_files += 1;
            }
            "yaml" => {
                collect_yaml_strings(&bytes, &mut generic_strings);
                yaml_files += 1;
            }
            "dsv" => {
                collect_dsv_fields(&bytes, delim.unwrap(), &mut dsv_fields);
                dsv_files += 1;
            }
            _ => unreachable!(),
        }
    }

    let mut shell_args = Vec::new();
    collect_shell_args(&repo_root.join("scripts"), &mut shell_args);

    let mut urls = Vec::new();
    collect_urls(&repo_root.join("docs"), &mut urls);

    println!("# jq Format-Function Input Shape (#663)\n");
    println!(
        "Corpus: {json_files} JSON + {yaml_files} YAML files from `data/bench/corpus` (#301) \
-> {} string values; {dsv_files} DSV/CSV files -> {} row fields; \
{} shell-argument literals from `scripts/*.sh`; {} URLs from `docs/**/*.md`.\n",
        generic_strings.len(),
        dsv_fields.len(),
        shell_args.len(),
        urls.len(),
    );

    println!("## Primary pools\n");
    println!("{STATS_HEADER}");

    let mut uri = FormatStats::default();
    for s in &generic_strings {
        uri.record(s, is_uri_safe);
    }
    println!("{}", uri.row("@uri (generic string values)"));

    let mut html = FormatStats::default();
    for s in &generic_strings {
        html.record(s, is_html_safe);
    }
    println!("{}", html.row("@html (generic string values)"));

    let mut sh = FormatStats::default();
    for s in &generic_strings {
        sh.record(s, is_sh_safe);
    }
    println!("{}", sh.row("@sh (generic string values)"));

    let mut csv = FormatStats::default();
    for s in &dsv_fields {
        csv.record(s, is_csv_safe);
    }
    println!("{}", csv.row("@csv/@dsv (DSV row fields)"));

    println!("\n## Supplementary real corpora\n");
    println!("{STATS_HEADER}");

    let mut uri_urls = FormatStats::default();
    for s in &urls {
        uri_urls.record(s, is_uri_safe);
    }
    println!("{}", uri_urls.row("@uri (real URLs from docs/)"));

    let mut sh_args = FormatStats::default();
    for s in &shell_args {
        sh_args.record(s, is_sh_safe);
    }
    println!("{}", sh_args.row("@sh (real shell literals from scripts/)"));
}
