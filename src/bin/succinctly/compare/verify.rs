//! `--verify results.json`: re-run the deterministic dimensions and spot-check
//! timing rows against a recorded run (#5409, acceptance criteria 2 and 3).
//!
//! The recorded and the fresh results are reduced to a fingerprint of the
//! sections that must be byte-stable on one machine (A, B, C, F, G and the
//! ledger); a differing row id is a failure. Timing rows are only expected to
//! stay within a tolerance that widens with their own measured noise, so a
//! genuine slowdown trips the check and ordinary jitter does not.

use serde_json::Value;
use std::collections::BTreeMap;

/// section → row id → canonical text of what must not change.
pub type Fingerprint = BTreeMap<String, BTreeMap<String, String>>;

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or("")
}

fn rows<'a>(v: &'a Value, path: &[&str]) -> Vec<&'a Value> {
    let mut cur = v;
    for k in path {
        match cur.get(*k) {
            Some(n) => cur = n,
            None => return vec![],
        }
    }
    cur.as_array()
        .map(|a| a.iter().collect())
        .unwrap_or_default()
}

/// Notes and timings are machine-specific or noisy and are left out; ids and
/// outcomes are what a rerun must reproduce.
pub fn fingerprint(r: &Value) -> Fingerprint {
    let mut f = Fingerprint::new();
    let mut put = |section: &str, id: String, text: String| {
        f.entry(section.to_string()).or_default().insert(id, text);
    };
    for (section, path) in [
        ("features", ["features", "rows"]),
        ("since-secondary", ["since_secondary", "rows"]),
    ] {
        for x in rows(r, &path) {
            put(section, s(x, "id").into(), s(x, "status").into());
        }
    }
    for (section, key) in [("cli", "cli"), ("extensions", "extensions")] {
        for x in rows(r, &[key]) {
            put(section, s(x, "id").into(), s(x, "verdict").into());
        }
    }
    for suite in rows(r, &["differential"]) {
        for x in rows(suite, &["rows"]) {
            put("differential", s(x, "id").into(), x["matches"].to_string());
        }
    }
    for x in rows(r, &["input", "rows"]) {
        let text = format!(
            "{} {} {} {} {}",
            s(x, "expect"),
            x["reference_accepts"],
            x["default_accepts"],
            x["validate_accepts"],
            x["output_differs"]
        );
        put("input", s(x, "id").into(), text);
    }
    for x in rows(r, &["robust"]) {
        let text = format!(
            "{} {} {} {} {}",
            x["matches"],
            s(x, "reference"),
            s(x, "subject"),
            x["reference_exit"],
            x["subject_exit"]
        );
        put("robust", s(x, "id").into(), text);
    }
    for x in rows(r, &["known_divergences"]) {
        put("ledger", s(x, "id").into(), s(x, "category").into());
    }
    f
}

/// Human-readable differences between two fingerprints; empty when equal.
pub fn diff(old: &Fingerprint, new: &Fingerprint) -> Vec<String> {
    let mut out = vec![];
    let empty = BTreeMap::new();
    let sections: std::collections::BTreeSet<&String> = old.keys().chain(new.keys()).collect();
    for sec in sections {
        let (a, b) = (
            old.get(sec).unwrap_or(&empty),
            new.get(sec).unwrap_or(&empty),
        );
        let ids: std::collections::BTreeSet<&String> = a.keys().chain(b.keys()).collect();
        for id in ids {
            match (a.get(id), b.get(id)) {
                (Some(x), Some(y)) if x == y => {}
                (Some(x), Some(y)) => out.push(format!("{sec}:{id} was `{x}`, now `{y}`")),
                (Some(_), None) => out.push(format!("{sec}:{id} is no longer produced")),
                (None, _) => out.push(format!("{sec}:{id} is new")),
            }
        }
    }
    out
}

/// `n` timed rows picked at an even stride through the recorded ones, so the
/// same file always yields the same spot check.
pub fn spot_rows(r: &Value, n: usize) -> Vec<(String, String)> {
    let timed: Vec<&Value> = rows(r, &["perf"])
        .into_iter()
        .filter(|x| x["speedup_primary"].is_number())
        .collect();
    if timed.is_empty() || n == 0 {
        return vec![];
    }
    let step = (timed.len() as f64 / n.min(timed.len()) as f64).max(1.0);
    (0..n.min(timed.len()))
        .map(|i| timed[(i as f64 * step) as usize])
        .map(|x| (s(x, "query").to_string(), s(x, "input").to_string()))
        .collect()
}

/// Largest CV (percent) among a row's tools.
pub fn max_cv(row: &Value) -> f64 {
    rows(row, &["tools"])
        .iter()
        .filter_map(|t| t["cv_pct"].as_f64())
        .fold(0.0, f64::max)
}

/// Do two speedups agree? The tolerance is the larger of 25% and three times
/// the combined measured CV, in log space, so a noisy row is not failed for its noise.
pub fn timing_agrees(old: f64, new: f64, old_cv_pct: f64, new_cv_pct: f64) -> bool {
    let tol = 1.25_f64.ln().max(3.0 * (old_cv_pct + new_cv_pct) / 100.0);
    (new / old).ln().abs() <= tol
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample() -> Value {
        json!({
            "features": {"rows": [{"id": "length/0", "status": "match", "note": "/Users/x/path"}]},
            "cli": [{"id": "compact", "verdict": "match"}],
            "differential": [{"rows": [{"id": "jq.test:3", "matches": true}]}],
            "robust": [{"id": "r", "matches": true, "reference": "ok", "subject": "ok",
                        "reference_exit": 0, "subject_exit": 0, "reference_ms": 1.0, "subject_ms": 2.0}],
            "known_divergences": [{"id": "cli:x", "category": "by-design", "reason": "r"}],
            "perf": [
                {"query": "a", "input": "u", "speedup_primary": 2.0, "tools": [{"cv_pct": 1.0}, {"cv_pct": 3.0}]},
                {"query": "b", "input": "u", "speedup_primary": 1.0, "tools": []},
                {"query": "c", "input": "u", "speedup_primary": null, "tools": []},
                {"query": "d", "input": "u", "speedup_primary": 3.0, "tools": []}
            ]
        })
    }

    #[test]
    fn identical_results_have_no_differences_and_timings_and_notes_are_ignored() {
        let a = sample();
        let mut b = sample();
        b["features"]["rows"][0]["note"] = json!("/other/path");
        b["robust"][0]["subject_ms"] = json!(999.0);
        let none: Vec<String> = vec![];
        assert_eq!(diff(&fingerprint(&a), &fingerprint(&b)), none);
    }

    /// The check must be able to fail: each kind of change is reported by id.
    #[test]
    fn a_changed_added_or_removed_row_is_reported() {
        let a = fingerprint(&sample());
        let mut changed = sample();
        changed["cli"][0]["verdict"] = json!("differs");
        let d = diff(&a, &fingerprint(&changed));
        assert_eq!(d, ["cli:compact was `match`, now `differs`"]);
        let mut dropped = sample();
        dropped["differential"][0]["rows"] = json!([]);
        assert_eq!(
            diff(&a, &fingerprint(&dropped)),
            ["differential:jq.test:3 is no longer produced"]
        );
        let mut added = sample();
        added["known_divergences"] = json!([]);
        assert_eq!(diff(&fingerprint(&added), &a), ["ledger:cli:x is new"]);
        let mut crash = sample();
        crash["robust"][0]["subject"] = json!("crash");
        assert_eq!(diff(&a, &fingerprint(&crash)).len(), 1);
    }

    #[test]
    fn spot_rows_are_deterministic_skip_untimed_rows_and_are_capped() {
        let r = sample();
        let spots = spot_rows(&r, 2);
        assert_eq!(spots, spot_rows(&r, 2));
        assert_eq!(spots.len(), 2);
        assert!(spots.iter().all(|(q, _)| q != "c"));
        assert_eq!(spot_rows(&r, 99).len(), 3);
        assert!(spot_rows(&r, 0).is_empty() && spot_rows(&json!({}), 5).is_empty());
    }

    #[test]
    fn max_cv_takes_the_noisiest_tool() {
        assert_eq!(max_cv(&sample()["perf"][0]), 3.0);
        assert_eq!(max_cv(&sample()["perf"][1]), 0.0);
    }

    /// The injected-slowdown acceptance test: a halved speedup must fail at
    /// ordinary noise, ordinary jitter must pass, and a very noisy row is
    /// allowed more room than a quiet one.
    #[test]
    fn a_slowdown_trips_the_check_and_jitter_does_not() {
        assert!(timing_agrees(2.0, 2.2, 2.0, 2.0));
        assert!(timing_agrees(2.0, 1.8, 2.0, 2.0));
        assert!(
            !timing_agrees(2.0, 1.0, 2.0, 2.0),
            "a halved speedup must fail"
        );
        assert!(
            !timing_agrees(2.0, 4.0, 2.0, 2.0),
            "a doubled speedup must fail too"
        );
        assert!(!timing_agrees(2.0, 1.4, 1.0, 1.0));
        assert!(
            timing_agrees(2.0, 1.4, 12.0, 12.0),
            "noisy rows get wider tolerance"
        );
    }
}
