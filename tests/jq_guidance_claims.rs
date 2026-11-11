//! #6916: the claims of `docs/guides/choosing-jq.md` hold in the committed comparison reports.
//!
//! The guide says where `succinctly jq` or jq is the better choice and quotes the figures behind each
//! recommendation. Those figures come from `docs/compare/jq-<platform>.json`, which `succinctly dev compare jq
//! --write-docs` regenerates. This test reads both reports, checks every claim against the threshold the guide
//! states for it, and checks that the guide carries a `<!-- claim: id -->` marker for every claim and no marker
//! for a claim that is not here, so a report that stops backing a sentence fails the build instead of leaving the
//! sentence behind.
//!
//! A threshold is the bound the guide's wording needs ("at least 5x"), not the figure measured: a re-run moves
//! the figure by a few percent and must not fail the test, while a regression that changes the advice must.

use serde_json::Value;

const PLATFORMS: [&str; 2] = ["linux-x86_64", "macos-aarch64"];

fn report(platform: &str) -> Value {
    let path = format!(
        "{}/docs/compare/jq-{platform}.json",
        env!("CARGO_MANIFEST_DIR")
    );
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{path}: {e}"))
}

fn perf_rows(report: &Value) -> &Vec<Value> {
    report["perf"].as_array().expect("perf rows")
}

/// The row of `query` over `input`.
fn row<'a>(report: &'a Value, query: &str, input: &str) -> &'a Value {
    perf_rows(report)
        .iter()
        .find(|r| r["query"] == query && r["input"] == input)
        .unwrap_or_else(|| panic!("no perf row {query} over {input}"))
}

fn speedup(report: &Value, query: &str, input: &str) -> f64 {
    row(report, query, input)["speedup_primary"]
        .as_f64()
        .unwrap_or_else(|| panic!("{query} over {input} has no speedup"))
}

fn rss_ratio(report: &Value, query: &str, input: &str) -> f64 {
    row(report, query, input)["rss_ratio_primary"]
        .as_f64()
        .unwrap_or_else(|| panic!("{query} over {input} has no memory ratio"))
}

/// The geomean speedup over every timed row whose query `select`s.
fn geomean(report: &Value, select: impl Fn(&str) -> bool) -> f64 {
    let speedups: Vec<f64> = perf_rows(report)
        .iter()
        .filter(|r| select(r["query"].as_str().unwrap_or("")))
        .filter_map(|r| r["speedup_primary"].as_f64())
        .collect();
    assert!(!speedups.is_empty(), "no rows in the group");
    (speedups.iter().map(|s| s.ln()).sum::<f64>() / speedups.len() as f64).exp()
}

fn binary_bytes(report: &Value, tool: &str) -> f64 {
    report["operational"]["binaries"]
        .as_array()
        .expect("binaries")
        .iter()
        .find(|b| b["tool"] == tool)
        .and_then(|b| b["bytes"].as_f64())
        .unwrap_or_else(|| panic!("no binary size for {tool}"))
}

fn suite<'a>(report: &'a Value, key: &str, name: &str) -> &'a Value {
    report[key]
        .as_array()
        .expect("suites")
        .iter()
        .find(|s| s["suite"] == name)
        .unwrap_or_else(|| panic!("no suite {name} in {key}"))
}

// The groups of the guide's table, by query name.
fn pass_through(q: &str) -> bool {
    matches!(q, "identity" | "identity_compact")
}
fn navigation(q: &str) -> bool {
    matches!(
        q,
        "users_field"
            | "users_length"
            | "users_names"
            | "users_to_entries"
            | "wide_keys"
            | "users_three_files"
    )
}
fn early_exit(q: &str) -> bool {
    matches!(q, "users_first_early_exit" | "users_limit_early_exit")
}
fn transforms(q: &str) -> bool {
    const PREFIXES: [&str; 21] = [
        "users_map",
        "users_sort",
        "users_group",
        "users_unique",
        "users_reduce",
        "users_csv",
        "users_regex",
        "users_ascii",
        "users_assign",
        "users_del",
        "users_paths",
        "users_tojson",
        "users_walk",
        "numbers_",
        "strings_",
        "wide_to",
        "wide_map",
        "wide_add",
        "wide_with",
        "nested_",
        "comprehensive_",
    ];
    PREFIXES.iter().any(|prefix| q.starts_with(prefix))
}
fn streams(q: &str) -> bool {
    q.starts_with("ndjson_") || q.starts_with("seq_")
}
fn small_inputs(q: &str) -> bool {
    matches!(q, "startup" | "args_many") || q.starts_with("corpus_")
}

/// A group of the category table: its name and the query names it holds.
type Group = (&'static str, fn(&str) -> bool);

/// One claim: its marker id, and whether it holds on a platform's report.
type Claim = (&'static str, fn(&Value) -> bool);

const CLAIMS: &[Claim] = &[
    // The groups: each is the guide's headline for its row of the table.
    ("category-pass-through", |r| geomean(r, pass_through) > 2.5),
    ("category-navigation", |r| geomean(r, navigation) > 3.0),
    ("category-early-exit", |r| geomean(r, early_exit) > 3.5),
    ("category-transforms", |r| geomean(r, transforms) > 1.8),
    // Streams are the group the guide calls "same speed for most filters": a lead, but a small one.
    ("category-streams", |r| {
        let g = geomean(r, streams);
        g > 1.1 && g < 2.5
    }),
    // And small inputs are where there is no measurable gain: about 1x either side.
    ("category-small-inputs", |r| {
        let g = geomean(r, small_inputs);
        g > 0.8 && g < 1.3
    }),
    // One large document: 8x for `-c .`, 2x for the pretty-printed identity, 5x for a field read.
    ("pass-through-large", |r| {
        speedup(r, "identity_compact", "users-100mb") > 5.0
            && speedup(r, "identity", "users-100mb") > 2.0
    }),
    ("navigation-large", |r| {
        speedup(r, "users_field", "users-100mb") > 5.0
            && speedup(r, "users_length", "users-100mb") > 5.0
    }),
    // Under a fifth of jq's memory for a field read of 100 MB.
    ("memory-single-document", |r| {
        rss_ratio(r, "users_field", "users-100mb") < 0.2
    }),
    ("early-exit-large", |r| {
        speedup(r, "users_first_early_exit", "users-100mb") > 5.0
            && speedup(r, "users_limit_early_exit", "users-100mb") > 5.0
    }),
    ("strings-upcase", |r| {
        ["strings-1mb", "strings-10mb", "strings-100mb"]
            .iter()
            .all(|input| speedup(r, "strings_upcase", input) > 25.0)
    }),
    // jq starts faster, and loses its lead on the small real documents.
    ("startup", |r| {
        speedup(r, "startup", "none") < 1.0
            && speedup(r, "corpus_tabular_group", "corpus-tabular") < 1.0
    }),
    // A stream is held whole: more than 30x jq's resident set at 100 MB, for every stream row named.
    ("stream-memory", |r| {
        [
            ("ndjson_identity", "ndjson-100mb"),
            ("ndjson_field", "ndjson-100mb"),
            ("ndjson_select", "ndjson-100mb"),
            ("ndjson_inputs_reduce", "ndjson-100mb"),
            ("seq_identity", "jsonseq-100mb"),
        ]
        .iter()
        .all(|(query, input)| rss_ratio(r, query, input) > 30.0)
    }),
    // And two stream shapes where jq is the faster tool.
    ("stream-abandon", |r| {
        speedup(r, "ndjson_inputs_first", "ndjson-100mb") < 1.0
            && speedup(r, "users_seq_output", "users-10mb") < 1.0
    }),
    // jq 1.8.2's own tests: the guide says 475 of 550 `jq.test` programs agree, and 447 of 447 against 1.7.1.
    ("jq-1-8", |r| {
        let secondary = suite(r, "differential_secondary", "upstream-1.8.2-jq.test");
        let primary = suite(r, "differential", "upstream-jq.test");
        secondary["total"] == 550
            && secondary["matching"] == 475
            && primary["total"] == 447
            && primary["matching"] == 447
    }),
    // 11x to 22x: more than ten times the size of jq 1.7.1.
    ("binary-size", |r| {
        binary_bytes(r, "succinctly jq") > 10.0 * binary_bytes(r, "jq-1.7.1")
    }),
    // Of the 188 `n_` cases: 25 for jq, 39 for `succinctly jq`, none with `--validate`.
    ("validation", |r| {
        let n = &r["input"]["n"];
        n["total"] == 188 && n["reference"] == 25 && n["default"] == 39 && n["validate"] == 0
    }),
];

#[test]
fn every_claim_of_the_guide_holds_on_both_platforms_6916() {
    let reports: Vec<(&str, Value)> = PLATFORMS.iter().map(|p| (*p, report(p))).collect();
    let mut failed = Vec::new();
    for (id, holds) in CLAIMS {
        for (platform, report) in &reports {
            if !holds(report) {
                failed.push(format!("{id} on {platform}"));
            }
        }
    }
    assert!(
        failed.is_empty(),
        "the committed reports no longer back docs/guides/choosing-jq.md on: {failed:?}. Change the claim, the \
         threshold here and the sentence in the guide together."
    );
}

#[test]
fn the_guide_carries_a_marker_for_every_claim_and_no_other_6916() {
    let guide = std::fs::read_to_string(format!(
        "{}/docs/guides/choosing-jq.md",
        env!("CARGO_MANIFEST_DIR")
    ))
    .expect("the guide");
    let mut marked: Vec<&str> = guide
        .match_indices("<!-- claim: ")
        .map(|(at, _)| {
            let rest = &guide[at + "<!-- claim: ".len()..];
            &rest[..rest.find(" -->").expect("a closed marker")]
        })
        .collect();
    marked.sort_unstable();
    let mut claims: Vec<&str> = CLAIMS.iter().map(|(id, _)| *id).collect();
    claims.sort_unstable();
    assert_eq!(
        marked, claims,
        "the guide's markers and this test's claims differ"
    );
}

/// The numbers the guide's category table prints are the ones the reports give, to the digit it prints.
#[test]
fn the_category_table_matches_the_reports_6916() {
    let guide = std::fs::read_to_string(format!(
        "{}/docs/guides/choosing-jq.md",
        env!("CARGO_MANIFEST_DIR")
    ))
    .expect("the guide");
    let groups: [Group; 6] = [
        ("Pass-through and reformatting", pass_through),
        ("Field access and navigation", navigation),
        ("Early exit", early_exit),
        ("Transforms", transforms),
        ("Streams (NDJSON, `--seq`)", streams),
        ("Startup-bound", small_inputs),
    ];
    let reports: Vec<Value> = PLATFORMS.iter().map(|p| report(p)).collect();
    for (name, select) in groups {
        let cells: Vec<&str> = guide
            .lines()
            .map(|l| l.split('|').map(str::trim).collect::<Vec<_>>())
            .find(|cells| cells.get(1) == Some(&name))
            .unwrap_or_else(|| panic!("no table row for {name}"));
        // | name | queries | x86_64 | aarch64 |
        for (cell, report) in [cells[3], cells[4]].iter().zip(&reports) {
            let printed: f64 = cell.trim_end_matches('x').parse().expect("a figure");
            let measured = geomean(report, select);
            assert!(
                (printed - measured).abs() < 0.05,
                "{name}: the guide prints {printed}x, the report gives {measured:.2}x"
            );
        }
    }
}
