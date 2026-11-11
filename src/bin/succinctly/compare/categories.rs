//! The per-category geomeans beside the all-rows one in the generated summary (#6916).
//!
//! The all-rows geomean counts every `identity` row over every input shape, which is
//! `succinctly jq`'s strongest case. The groups below are the ones the hand-written table
//! of `docs/guides/choosing-jq.md` uses (its names, and the query predicates of
//! `tests/jq_guidance_claims.rs`), so the summary and the guide say the same thing; a test
//! here holds this table's figures to the ones the guide prints, since a predicate that
//! lives in two places diverges silently otherwise.

use super::docs::{geomean, table};
use serde_json::Value;

type Group = (&'static str, fn(&str) -> bool);

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

fn startup_bound(q: &str) -> bool {
    matches!(q, "startup" | "args_many") || q.starts_with("corpus_")
}

/// The groups, named as the rows of the guide's table are.
const GROUPS: [Group; 6] = [
    ("Pass-through and reformatting", pass_through),
    ("Field access and navigation", navigation),
    ("Early exit", early_exit),
    ("Transforms", transforms),
    ("Streams (NDJSON, `--seq`)", streams),
    ("Startup-bound", startup_bound),
];

fn cell(v: &Value, keep: &dyn Fn(&str) -> bool) -> String {
    let xs: Vec<f64> = v["perf"]
        .as_array()
        .map_or(&[][..], Vec::as_slice)
        .iter()
        .filter(|r| keep(r["query"].as_str().unwrap_or("")))
        .filter_map(|r| r["speedup_primary"].as_f64())
        .collect();
    geomean(&xs).map_or_else(
        || "n/a".to_string(),
        |g| {
            format!(
                "{g:.2}x (jq faster on {} of {})",
                xs.iter().filter(|&&x| x < 1.0).count(),
                xs.len()
            )
        },
    )
}

/// One row per group and two all-rows lines (a group does not cover every row), one
/// column per committed platform. Every cell also says on how many of its rows jq wins,
/// so a group whose geomean hides a loss shows it.
pub fn table_for(runs: &[(String, Value)]) -> String {
    let mut rows: Vec<Vec<String>> = GROUPS
        .iter()
        .map(|&(name, keep)| {
            let mut row = vec![name.to_string()];
            row.extend(runs.iter().map(|(_, v)| cell(v, &keep)));
            row
        })
        .collect();
    for (label, keep) in [
        (
            "All rows without pass-through",
            &(|q: &str| !pass_through(q)) as &dyn Fn(&str) -> bool,
        ),
        ("All rows", &|_: &str| true),
    ] {
        let mut row = vec![label.to_string()];
        row.extend(runs.iter().map(|(_, v)| cell(v, keep)));
        rows.push(row);
    }
    let cpus: Vec<String> = runs
        .iter()
        .map(|(p, v)| {
            format!(
                "{} ({p})",
                v["env"]["system"]["cpu"].as_str().unwrap_or("?")
            )
        })
        .collect();
    let mut header = vec!["Group"];
    header.extend(cpus.iter().map(String::as_str));
    table(&header, &rows, 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn row(query: &str, speedup: f64) -> Value {
        json!({"query": query, "speedup_primary": speedup})
    }

    #[test]
    fn each_group_gets_its_own_geomean_and_the_losses_are_counted() {
        let v = json!({
            "env": {"system": {"cpu": "Test CPU"}},
            "perf": [
                row("identity", 8.0),
                row("identity_compact", 2.0),
                row("users_field", 0.5),
                {"query": "users_walk", "speedup_primary": null},
                row("users_slurp", 3.0),
            ]
        });
        let t = table_for(&[("p".into(), v)]);
        let line = |label: &str| t.lines().find(|l| l.contains(label)).unwrap().to_string();
        assert!(
            line("Pass-through").contains("4.00x (jq faster on 0 of 2)"),
            "{t}"
        );
        assert!(
            line("Field access").contains("0.50x (jq faster on 1 of 1)"),
            "{t}"
        );
        // An untimed row counts nowhere, and a group with no timed row says so.
        assert!(line("Transforms").contains("n/a"), "{t}");
        // `users_slurp` is in no group: only the all-rows lines count it.
        assert!(
            line("All rows without").contains("1.22x (jq faster on 1 of 2)"),
            "{t}"
        );
        assert!(
            line("| All rows   ").contains("2.21x (jq faster on 1 of 4)"),
            "{t}"
        );
        assert!(t.lines().next().unwrap().contains("Test CPU (p)"), "{t}");
    }

    #[test]
    fn the_groups_do_not_overlap() {
        let committed = super::super::docs::committed_runs(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/compare"),
            None,
        )
        .unwrap();
        for (platform, v) in committed {
            for r in v["perf"].as_array().unwrap() {
                let q = r["query"].as_str().unwrap();
                let n = GROUPS.iter().filter(|(_, keep)| keep(q)).count();
                assert!(n <= 1, "{platform}: `{q}` is in {n} groups");
            }
        }
    }

    /// The summary's figures are the ones `docs/guides/choosing-jq.md` prints for the same
    /// groups, to the digits it prints: the guide's table is typed by hand and checked
    /// against the reports by `tests/jq_guidance_claims.rs`, this table is generated, and
    /// a group defined differently in the two would show here.
    #[test]
    fn the_summary_agrees_with_the_guides_category_table() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let runs = super::super::docs::committed_runs(&root.join("docs/compare"), None).unwrap();
        let platforms: Vec<&str> = runs.iter().map(|(p, _)| p.as_str()).collect();
        assert_eq!(
            platforms,
            ["linux-aarch64", "linux-x86_64", "macos-aarch64"],
            "the guide's table has one column per platform, in this order"
        );
        let guide = std::fs::read_to_string(root.join("docs/guides/choosing-jq.md")).unwrap();
        for (name, keep) in GROUPS {
            let cells: Vec<&str> = guide
                .lines()
                .map(|l| l.split('|').map(str::trim).collect::<Vec<_>>())
                .find(|cells| cells.get(1) == Some(&name))
                .unwrap_or_else(|| panic!("the guide has no `{name}` row"))
                .into_iter()
                .collect();
            for (i, (platform, v)) in runs.iter().enumerate() {
                let generated = cell(v, &keep);
                let printed = cells[3 + i];
                assert!(
                    generated.starts_with(printed),
                    "{name} on {platform}: the guide prints {printed}, the summary {generated}"
                );
            }
        }
    }
}
