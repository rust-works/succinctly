//! `report.md`, generated from the results (#5409). Never hand-edited.

use super::cli;
use super::features::{FeatureSummary, Status};
use super::harness::{Row, Verdict};
use super::ledger;
use super::perf::{self, Gate, PerfRow, ToolResult};
use super::robust;
use super::Results;
use std::fmt::Write;

fn geomean(xs: &[f64]) -> Option<f64> {
    (!xs.is_empty()).then(|| (xs.iter().map(|x| x.ln()).sum::<f64>() / xs.len() as f64).exp())
}

// `r`/`e`/`s`/`x` are the results, env, output and row throughout a flat formatter.
#[allow(clippy::many_single_char_names)]
pub fn render(results: &Results) -> String {
    let r = results;
    let e = &r.env;
    let mut s = String::new();
    let _ = writeln!(
        s,
        "# jq vs succinctly jq\n\n{} · {} · {} cores · `{}`{}\n",
        e.system.cpu,
        e.system.os_version,
        e.system.cpu_cores,
        e.git.commit_short,
        if e.git.dirty { " (dirty)" } else { "" },
    );
    if r.command.contains("--control") {
        let _ = writeln!(s, "> **CONTROL RUN:** the subject is jq itself, so every ratio shows the harness's noise floor, not a comparison.\n");
    }
    if !e.subject_is_harness {
        let _ = writeln!(
            s,
            "> Timed binary: a separate build (`{}`), not the harness (#5570).\n",
            e.succinctly_features.join(",")
        );
    }
    if !e.invalid_reasons.is_empty() {
        let _ = writeln!(s, "> **INVALID RUN — do not quote the timings:**");
        for why in &e.invalid_reasons {
            let _ = writeln!(s, "> - {why}");
        }
        s.push('\n');
    }

    let total = r.smoke.len();
    let matched = r
        .smoke
        .iter()
        .filter(|x| x.verdict == Verdict::Match)
        .count();
    let speedups: Vec<f64> = r.smoke.iter().filter_map(|x| x.speedup).collect();
    let _ = writeln!(s, "## Summary\n");
    let _ = writeln!(s, "| Metric                      | Value |");
    let _ = writeln!(s, "|-----------------------------|-------|");
    let _ = writeln!(s, "| Reference                   | {} |", e.jq_version);
    if total > 0 {
        let _ = writeln!(s, "| Smoke differential pass     | {matched}/{total} |");
        match geomean(&speedups) {
            Some(g) => {
                let lo = speedups.iter().copied().fold(f64::MAX, f64::min);
                let hi = speedups.iter().copied().fold(f64::MIN, f64::max);
                let _ = writeln!(
                    s,
                    "| Smoke geomean speedup       | {g:.2}x ({lo:.2}x–{hi:.2}x) |"
                );
            }
            None => {
                let _ = writeln!(s, "| Smoke geomean speedup       | n/a |");
            }
        }
        if let Some(f) = &r.features {
            let _ = writeln!(
                s,
                "| Builtins defined by succinctly | {:.1}% of {} probed ({} listed, {} not probed) |",
                f.implemented_pct, f.probed, f.total, f.not_probed
            );
            let _ = writeln!(
                s,
                "| Builtins agreeing with the reference | {:.1}% ({}/{}) |",
                f.matching_pct, f.matching, f.probed
            );
        }
    }
    if !r.syntax.is_empty() {
        let ok = r
            .syntax
            .iter()
            .filter(|x| x.verdict == cli::Verdict::Match)
            .count();
        let _ = writeln!(
            s,
            "| Syntax-form probes agreeing | {ok}/{} |",
            r.syntax.len()
        );
    }
    if !r.cli.is_empty() {
        let ok = r
            .cli
            .iter()
            .filter(|x| x.verdict == cli::Verdict::Match)
            .count();
        let _ = writeln!(s, "| CLI probes agreeing | {ok}/{} |", r.cli.len());
    }
    if !r.differential.is_empty() {
        let (all, ok) = r
            .differential
            .iter()
            .fold((0, 0), |(a, o), x| (a + x.total, o + x.matching));
        let _ = writeln!(s, "| Upstream jq test cases agreeing | {ok}/{all} |");
    }
    if let Some(i) = &r.input {
        let _ = writeln!(
            s,
            "| JSONTestSuite `y_` accepted / `n_` rejected (`--validate`) | {}/{} · {}/{} |",
            i.y.validate,
            i.y.total,
            i.n.total - i.n.validate,
            i.n.total
        );
    }
    if !r.robust.is_empty() {
        let ok = r.robust.iter().filter(|x| x.matches).count();
        let n = |c: robust::Class, f: fn(&robust::Row) -> robust::Class| {
            r.robust.iter().filter(|x| f(x) == c).count()
        };
        let _ = writeln!(
            s,
            "| Robustness probes agreeing | {ok}/{} |",
            r.robust.len()
        );
        let _ = writeln!(
            s,
            "| Robustness: crashes / timeouts, succinctly | {} / {} |",
            n(robust::Class::Crash, |x| x.subject),
            n(robust::Class::Timeout, |x| x.subject)
        );
        let _ = writeln!(
            s,
            "| Robustness: crashes / timeouts, jq | {} / {} |",
            n(robust::Class::Crash, |x| x.reference),
            n(robust::Class::Timeout, |x| x.reference)
        );
    }
    let timed: Vec<&PerfRow> = r
        .perf
        .iter()
        .filter(|x| x.speedup_primary.is_some())
        .collect();
    if !timed.is_empty() {
        let sp: Vec<f64> = timed.iter().filter_map(|x| x.speedup_primary).collect();
        let (lo, hi) = range(&sp);
        let _ = writeln!(
            s,
            "| Geomean speedup vs {} | {:.2}x ({lo:.2}x–{hi:.2}x, n={}) |",
            e.jq_version,
            geomean(&sp).unwrap_or(f64::NAN),
            sp.len()
        );
        let sec: Vec<f64> = timed.iter().filter_map(|x| x.speedup_secondary).collect();
        if let Some(g) = geomean(&sec) {
            let (lo, hi) = range(&sec);
            let _ = writeln!(s, "| Geomean speedup vs the secondary reference | {g:.2}x ({lo:.2}x–{hi:.2}x, n={}) |", sec.len());
        }
        let rss: Vec<f64> = timed.iter().filter_map(|x| x.rss_ratio_primary).collect();
        if let Some(g) = geomean(&rss) {
            let (lo, hi) = range(&rss);
            let _ = writeln!(
                s,
                "| Geomean peak RSS vs {} | {g:.2}x ({lo:.2}x–{hi:.2}x) |",
                e.jq_version
            );
        }
        let private: Vec<f64> = timed
            .iter()
            .filter_map(|x| x.private_ratio_primary)
            .collect();
        if let Some(g) = geomean(&private) {
            let (lo, hi) = range(&private);
            let _ = writeln!(
                s,
                "| Geomean private memory vs {} | {g:.2}x ({lo:.2}x–{hi:.2}x) |",
                e.jq_version
            );
        }
        let ir: Vec<f64> = timed.iter().filter_map(|x| x.ir_ratio_primary).collect();
        if let Some(g) = geomean(&ir) {
            let (lo, hi) = range(&ir);
            let _ = writeln!(
                s,
                "| Geomean instructions vs {} (cachegrind, n={}) | {g:.2}x ({lo:.2}x–{hi:.2}x) |",
                e.jq_version,
                ir.len()
            );
        }
        let _ = writeln!(
            s,
            "| Timing rows jq wins / noisy (CV > 5%) | {} / {} |",
            sp.iter().filter(|x| **x < 1.0).count(),
            timed.iter().filter(|x| x.noisy).count()
        );
    }
    if r.features.is_some() || !r.cli.is_empty() || !r.differential.is_empty() || r.input.is_some()
    {
        let _ = writeln!(
            s,
            "| Known divergences (ledger) | {} |",
            r.known_divergences.len()
        );
        let _ = writeln!(
            s,
            "| Ledger drift | {} unlisted, {} stale |",
            r.ledger.unlisted.len(),
            r.ledger.stale.len()
        );
    }
    s.push('\n');

    let _ = writeln!(s, "## Dimensions\n");
    let _ = writeln!(s, "| Dimension | Status |");
    let _ = writeln!(s, "|-----------|--------|");
    for d in &r.dimensions {
        let _ = writeln!(s, "| {} | {} |", d.name, d.status);
    }

    if total > 0 {
        // Deterministic section: no timings.
        let _ = writeln!(s, "\n## Smoke: correctness\n");
        let _ = writeln!(s, "| Query | Verdict | jq exit | succinctly exit |");
        let _ = writeln!(s, "|-------|---------|---------|-----------------|");
        for x in &r.smoke {
            let ex = |c: Option<i32>| c.map_or_else(|| "signal".into(), |c| c.to_string());
            let v = if x.verdict == Verdict::Match {
                "match"
            } else {
                "**DIFFERS**"
            };
            let _ = writeln!(
                s,
                "| {} | {v} | {} | {} |",
                x.id,
                ex(x.reference_exit),
                ex(x.subject_exit)
            );
        }

        let _ = writeln!(
            s,
            "\n## Smoke: timing (process spawn, ms; matching outputs only)\n"
        );
        let _ = writeln!(
            s,
            "| Query | jq median (MAD) | succinctly median (MAD) | speedup |"
        );
        let _ = writeln!(
            s,
            "|-------|-----------------|-------------------------|---------|"
        );
        let timed: Vec<&Row> = r.smoke.iter().filter(|x| x.speedup.is_some()).collect();
        for x in timed {
            let (a, b) = (x.reference.as_ref().unwrap(), x.subject.as_ref().unwrap());
            let _ = writeln!(
                s,
                "| {} | {:.2} ({:.2}) | {:.2} ({:.2}) | {:.2}x |",
                x.id,
                a.median_ms,
                a.mad_ms,
                b.median_ms,
                b.mad_ms,
                x.speedup.unwrap()
            );
        }
        let _ = writeln!(
            s,
            "\nn = {} interleaved repetitions per row. Startup dominates at this input size; \
             these rows measure process spawn, not throughput.",
            r.reps
        );
    }

    features_sections(&mut s, r);
    syntax_section(&mut s, &r.syntax, &r.known_divergences);
    cli_sections(&mut s, r);
    differential_section(&mut s, r);
    differential_secondary_section(&mut s, r);
    input_section(&mut s, r);
    robust_section(&mut s, r);
    operational_section(&mut s, r);
    perf_sections(&mut s, r);

    let _ = writeln!(s, "\n## Reproduce\n");
    let _ = writeln!(s, "```bash\ncargo build --release --features bench-runner");
    if !e.subject_is_harness {
        let _ = writeln!(
            s,
            "cargo build --release --features {} --target-dir target/subject",
            e.succinctly_features.join(",")
        );
    }
    let _ = writeln!(s, "{}\n```", r.command);
    let _ = writeln!(
        s,
        "\nmanifest sha256 `{}` · succinctly sha256 `{}` · jq sha256 `{}`",
        r.manifest_sha256, e.succinctly_sha256, e.jq_sha256
    );
    s
}

fn note_for(r: &Results, id: &str) -> String {
    ledger_note(&r.known_divergences, id)
}

fn ledger_note(known: &[ledger::Entry], id: &str) -> String {
    known.iter().find(|e| e.id == id).map_or_else(
        || "**unlisted**".into(),
        |e| format!("{}: {}", e.category, e.reason),
    )
}

fn feature_table(s: &mut String, r: &Results, suite: &str, f: &FeatureSummary) {
    let _ = writeln!(
        s,
        "{} builtins: {} match, {} differ, {} not implemented, {} not probed.\n",
        f.reference, f.matching, f.differs, f.not_implemented, f.not_probed
    );
    let _ = writeln!(s, "| Builtin | Status | Ledger |");
    let _ = writeln!(s, "|---------|--------|--------|");
    for x in f
        .rows
        .iter()
        .filter(|x| !matches!(x.status, Status::Match | Status::NotProbed))
    {
        let st = if x.status == Status::Differs {
            "differs"
        } else {
            "not implemented"
        };
        let _ = writeln!(
            s,
            "| `{}` | {st} | {} |",
            x.id,
            note_for(r, &format!("{suite}:{}", x.id))
        );
    }
    let skipped: Vec<String> = f
        .rows
        .iter()
        .filter(|x| x.status == Status::NotProbed)
        .map(|x| format!("`{}` ({})", x.id, x.note.as_deref().unwrap_or("")))
        .collect();
    if !skipped.is_empty() {
        let _ = writeln!(s, "\nNot probed: {}.", skipped.join(", "));
    }
}

fn features_sections(s: &mut String, r: &Results) {
    let Some(f) = &r.features else { return };
    let _ = writeln!(s, "\n## A. Builtin coverage\n");
    let _ = writeln!(
        s,
        "Inventory: `builtins` of the reference. Each `name/arity` runs on eight inputs under \
         `try ... catch \"E\"`; error wording is the error-probe suite's job.\n"
    );
    feature_table(s, r, "features", f);
    let _ = writeln!(s, "\n### Added after the primary reference\n");
    match (&r.since_secondary, &r.secondary_note) {
        (Some(x), _) => feature_table(s, r, "since-secondary", x),
        (None, Some(why)) => {
            let _ = writeln!(s, "Not run: {why}.");
        }
        (None, None) => {}
    }
}

/// A, the language's forms: the same stdout-and-exit comparison as G, grouped by area.
fn syntax_section(s: &mut String, rows: &[cli::CliRow], known: &[ledger::Entry]) {
    if rows.is_empty() {
        return;
    }
    let _ = writeln!(s, "\n### A. Syntax forms\n");
    let _ = writeln!(
        s,
        "Operators, paths and slices, construction, control flow, destructuring (`?//` included), functions, \
         string and format forms, assignment and modules, each run on a fixed document. Stdout and exit \
         status must be identical; error wording is not compared.\n"
    );
    let _ = writeln!(s, "| Area | Probes | Agree |");
    let _ = writeln!(s, "|------|--------|-------|");
    let mut areas: Vec<&str> = rows.iter().map(|x| x.area.as_str()).collect();
    areas.sort_unstable();
    areas.dedup();
    for a in areas {
        let all = rows.iter().filter(|x| x.area == a).count();
        let ok = rows
            .iter()
            .filter(|x| x.area == a && x.verdict == cli::Verdict::Match)
            .count();
        let _ = writeln!(s, "| {a} | {all} | {ok} |");
    }
    let mut differing = rows
        .iter()
        .filter(|x| x.verdict == cli::Verdict::Differs)
        .peekable();
    if differing.peek().is_none() {
        return;
    }
    let _ = writeln!(s, "\n| Probe | jq exit | succinctly exit | Ledger |");
    let _ = writeln!(s, "|-------|---------|-----------------|--------|");
    let ex = |c: Option<i32>| c.map_or_else(|| "signal".into(), |c| c.to_string());
    for x in differing {
        let _ = writeln!(
            s,
            "| `{}` | {} | {} | {} |",
            x.id,
            ex(x.reference_exit),
            ex(x.subject_exit),
            ledger_note(known, &format!("syntax:{}", x.id))
        );
    }
}

fn cli_sections(s: &mut String, r: &Results) {
    if r.cli.is_empty() {
        return;
    }
    let _ = writeln!(s, "\n## G. CLI & I/O compatibility\n");
    let _ = writeln!(
        s,
        "Stdout and exit status must be identical; stderr wording is not compared.\n"
    );
    let _ = writeln!(s, "| Area | Probes | Agree |");
    let _ = writeln!(s, "|------|--------|-------|");
    let mut areas: Vec<&str> = r.cli.iter().map(|x| x.area.as_str()).collect();
    areas.sort_unstable();
    areas.dedup();
    for a in areas {
        let all = r.cli.iter().filter(|x| x.area == a).count();
        let ok = r
            .cli
            .iter()
            .filter(|x| x.area == a && x.verdict == cli::Verdict::Match)
            .count();
        let _ = writeln!(s, "| {a} | {all} | {ok} |");
    }
    let _ = writeln!(s, "\n| Probe | jq exit | succinctly exit | Ledger |");
    let _ = writeln!(s, "|-------|---------|-----------------|--------|");
    let ex = |c: Option<i32>| c.map_or_else(|| "signal".into(), |c| c.to_string());
    for x in r.cli.iter().filter(|x| x.verdict == cli::Verdict::Differs) {
        let _ = writeln!(
            s,
            "| `{}` | {} | {} | {} |",
            x.id,
            ex(x.reference_exit),
            ex(x.subject_exit),
            note_for(r, &format!("cli:{}", x.id))
        );
    }
    let _ = writeln!(
        s,
        "\n### Succinctly-only surface (not counted as coverage)\n"
    );
    let _ = writeln!(s, "| Probe | Reference | Succinctly |");
    let _ = writeln!(s, "|-------|-----------|------------|");
    for x in &r.extensions {
        let who = if x.verdict == cli::Verdict::Extension {
            "rejects, as intended"
        } else {
            "**unexpected**"
        };
        let _ = writeln!(
            s,
            "| `{}` | exit {} ({who}) | exit {} |",
            x.id,
            ex(x.reference_exit),
            ex(x.subject_exit)
        );
    }
}

fn differential_section(s: &mut String, r: &Results) {
    if r.differential.is_empty() {
        return;
    }
    let _ = writeln!(s, "\n## B. Differential: upstream jq test files\n");
    let _ = writeln!(
        s,
        "Each case runs through both tools; stdout and exit status must be identical. \
         The files' own expected outputs are not consulted.\n"
    );
    let _ = writeln!(s, "| File | Cases | Agree |");
    let _ = writeln!(s, "|------|-------|-------|");
    for x in &r.differential {
        let _ = writeln!(s, "| {} | {} | {} |", x.suite, x.total, x.matching);
    }
    let _ = writeln!(s, "\n| Case | Program | Ledger |");
    let _ = writeln!(s, "|------|---------|--------|");
    for x in r
        .differential
        .iter()
        .flat_map(super::suites::SuiteSummary::differing)
    {
        let _ = writeln!(
            s,
            "| `{}` | `{}` | {} |",
            x.id,
            x.program,
            note_for(r, &x.id)
        );
    }
}

/// The secondary reference's own test files (#5622). Where `succinctly jq` follows jq 1.7.1, a
/// disagreement here is expected rather than a bug, so nothing in this table is a ledger entry
/// and none of it is counted above.
fn differential_secondary_section(s: &mut String, r: &Results) {
    if !r.secondary_suites_skipped.is_empty() {
        let _ = writeln!(s, "\n### B2. jq 1.8.1's own test files: not run\n");
        for why in &r.secondary_suites_skipped {
            let _ = writeln!(s, "- {why}");
        }
    }
    if r.differential_secondary.is_empty() {
        return;
    }
    let _ = writeln!(s, "\n### B2. jq 1.8.1's own test files (informational)\n");
    let _ = writeln!(
        s,
        "The same comparison against the secondary reference. `succinctly jq` follows jq 1.7.1, so a \
         disagreement here is a place where 1.8 changed, not necessarily a bug; these rows are not \
         in the ledger and not in the counts above.\n"
    );
    let _ = writeln!(s, "| File | Cases | Agree |");
    let _ = writeln!(s, "|------|-------|-------|");
    for x in &r.differential_secondary {
        let _ = writeln!(s, "| {} | {} | {} |", x.suite, x.total, x.matching);
    }
    let _ = writeln!(s, "\n| Case | Program |");
    let _ = writeln!(s, "|------|---------|");
    for x in r
        .differential_secondary
        .iter()
        .flat_map(super::suites::SuiteSummary::differing)
    {
        let _ = writeln!(s, "| `{}` | `{}` |", x.id, x.program);
    }
}

fn input_section(s: &mut String, r: &Results) {
    let Some(i) = &r.input else { return };
    let _ = writeln!(s, "\n## C. Input conformance: JSONTestSuite\n");
    let _ = writeln!(
        s,
        "Accepting is exit 0. `y_` must be accepted, `n_` rejected, `i_` is implementation-defined. \
         `succinctly` as shipped semi-indexes and validates little; `--validate` is strict RFC 8259.\n"
    );
    let _ = writeln!(
        s,
        "| Class | Cases | jq accepts | succinctly accepts | succinctly --validate accepts |"
    );
    let _ = writeln!(
        s,
        "|-------|-------|------------|--------------------|-------------------------------|"
    );
    for (name, t) in [("y_", &i.y), ("n_", &i.n), ("i_", &i.i)] {
        let _ = writeln!(
            s,
            "| {name} | {} | {} | {} | {} |",
            t.total, t.reference, t.default, t.validate
        );
    }
    let _ = writeln!(
        s,
        "\nThe ledger lists a case only when `--validate` breaks its `y_`/`n_` expectation or the \
         two tools print different values for an input both accept; jq itself accepts {} `n_` cases.",
        i.n.reference
    );
}

fn range(xs: &[f64]) -> (f64, f64) {
    (
        xs.iter().copied().fold(f64::MAX, f64::min),
        xs.iter().copied().fold(f64::MIN, f64::max),
    )
}

fn med(x: &PerfRow, i: usize) -> Option<String> {
    let t = x.tools.get(i)?;
    let (s, cv) = (t.stats.as_ref()?, t.cv_pct?);
    Some(format!("{:.1} ({cv:.0}%)", s.median_ms))
}

fn ratio(v: Option<f64>) -> String {
    v.map_or_else(|| "n/a".into(), |v| format!("{v:.2}x"))
}

/// A row's ratio, with † when it is of the minimum rather than the median.
fn row_ratio(x: &PerfRow, v: Option<f64>) -> String {
    let r = ratio(v);
    if x.ratio_from_min && v.is_some() {
        format!("{r}†")
    } else {
        r
    }
}

fn perf_sections(s: &mut String, r: &Results) {
    if r.perf.is_empty() {
        return;
    }
    let _ = writeln!(
        s,
        "\n## D. Performance (process spawn, ms; matching outputs only)\n"
    );
    let _ = writeln!(
        s,
        "Median of interleaved repetitions ({} requested, fewer at 100 MB; a row over {} ms is topped up \
         to {} where the time allows), CV in parentheses; the tool order rotates each repetition. A \
         speedup is reference ÷ succinctly, so below 1.00x jq is faster, and is a ratio of medians \
         except where marked †: a row that long is moved by run-to-run noise, so its ratio is of \
         each tool's fastest of the {} runs, which the noise moves least (the times beside it stay \
         medians). A row marked `CV>5%` is noise-dominated, and its ratio is not a finding on its \
         own. `1.8.1 ÷ 1.7.1` is the secondary reference's own change in speed, always of medians.{}\n",
        r.reps,
        perf::SLOW_ROW_MS,
        perf::SLOW_ROW_REPS,
        perf::SLOW_ROW_REPS,
        r.perf_note.as_deref().map_or(String::new(), |n| format!(" ({n}.)"))
    );
    let wrong: Vec<&PerfRow> = r.perf.iter().filter(|x| x.gate == Gate::Differs).collect();
    if !wrong.is_empty() {
        let _ = writeln!(
            s,
            "**Correctness rows (output differs from the primary reference; untimed):**\n"
        );
        for x in &wrong {
            let _ = writeln!(
                s,
                "- `{}` on `{}`: {}",
                x.query,
                x.input,
                note_for(r, &format!("perf:{}@{}", x.query, x.input))
            );
        }
        s.push('\n');
    }
    let timed: Vec<&PerfRow> = r
        .perf
        .iter()
        .filter(|x| x.speedup_primary.is_some())
        .collect();
    let losses: Vec<&&PerfRow> = timed
        .iter()
        .filter(|x| x.speedup_primary < Some(1.0))
        .collect();
    let _ = writeln!(s, "### Where jq is faster\n");
    if losses.is_empty() {
        let _ = writeln!(s, "None in this run.\n");
    } else {
        let _ = writeln!(
            s,
            "| Query | Input | jq 1.7.1 | succinctly | speedup | CV>5% |"
        );
        let _ = writeln!(
            s,
            "|-------|-------|----------|------------|---------|-------|"
        );
        for x in losses {
            let _ = writeln!(
                s,
                "| {} | {} | {} | {} | {} | {} |",
                x.query,
                x.input,
                med(x, 1).unwrap_or_default(),
                med(x, 0).unwrap_or_default(),
                row_ratio(x, x.speedup_primary),
                if x.noisy { "yes" } else { "" }
            );
        }
        s.push('\n');
    }
    let _ = writeln!(s, "### All timed rows\n");
    let _ = writeln!(s, "| Query | Input | jq 1.7.1 | succinctly | vs 1.7.1 | jq 1.8.1 | vs 1.8.1 | 1.8.1 ÷ 1.7.1 | CV>5% |");
    let _ = writeln!(s, "|-------|-------|----------|------------|----------|----------|----------|---------------|-------|");
    for x in &timed {
        let _ = writeln!(
            s,
            "| {} | {} | {} | {} | {} | {} | {} | {} | {} |",
            x.query,
            x.input,
            med(x, 1).unwrap_or_default(),
            med(x, 0).unwrap_or_default(),
            row_ratio(x, x.speedup_primary),
            med(x, 2).unwrap_or_else(|| "n/a".into()),
            row_ratio(x, x.speedup_secondary),
            ratio(x.secondary_over_primary),
            if x.noisy { "yes" } else { "" }
        );
    }
    memory_section(s, &timed);
    instruction_section(s, &timed);
}

/// Whether the instruction count and the wall-clock put succinctly on opposite sides of jq
/// (CLAUDE.md: they can disagree in sign). Both must be more than 5% from a tie, so noise
/// around 1.0 on either is not a finding.
fn signs_disagree(ir_ratio: f64, speedup: f64) -> bool {
    let wall_ratio = 1.0 / speedup;
    (ir_ratio < 1.0) != (wall_ratio < 1.0)
        && (ir_ratio - 1.0).abs() > 0.05
        && (wall_ratio - 1.0).abs() > 0.05
}

/// Dimension D's secondary signal: instructions executed under cachegrind (`--cachegrind`).
fn instruction_section(s: &mut String, timed: &[&PerfRow]) {
    let counted: Vec<&&PerfRow> = timed
        .iter()
        .filter(|x| x.tools.iter().any(|t| t.instructions.is_some()))
        .collect();
    if counted.is_empty() {
        return;
    }
    let _ = writeln!(
        s,
        "\n## D. Instruction counts (`valgrind --tool=cachegrind`, `I refs` of one untimed run)\n"
    );
    let _ = writeln!(
        s,
        "Instructions executed by the whole process, start-up included. Unlike a time they do not move with \
         machine load, but they are not time: a vectorised loop and a branchy one retire different numbers of \
         instructions per nanosecond, so the two signals can disagree in sign. A row is marked `differs` when \
         the instruction ratio and the wall-clock put succinctly on opposite sides of jq and both are more \
         than 5% from a tie. Only the rows up to 1 MB are counted (cachegrind runs ~50x slower); a tool \
         valgrind could not run reads `n/a`.\n"
    );
    let _ = writeln!(
        s,
        "| Query | Input | jq 1.7.1 | succinctly | ratio (succinctly ÷ jq) | wall-clock vs jq 1.7.1 | signs |"
    );
    let _ = writeln!(
        s,
        "|-------|-------|----------|------------|-------------------------|------------------------|-------|"
    );
    for x in counted {
        let ir = |i: usize| {
            x.tools
                .get(i)
                .and_then(|t| t.instructions)
                .map_or_else(|| "n/a".into(), |v| v.to_string())
        };
        let differs = matches!(
            (x.ir_ratio_primary, x.speedup_primary),
            (Some(a), Some(b)) if signs_disagree(a, b)
        );
        let _ = writeln!(
            s,
            "| {} | {} | {} | {} | {} | {} | {} |",
            x.query,
            x.input,
            ir(1),
            ir(0),
            ratio(x.ir_ratio_primary),
            row_ratio(x, x.speedup_primary),
            if differs { "differs" } else { "" }
        );
    }
}

/// Dimension E: peak RSS beside private memory, with the note that says why both are shown.
fn memory_section(s: &mut String, timed: &[&PerfRow]) {
    let _ = writeln!(
        s,
        "\n## E. Memory (peak RSS and private memory, KiB; separate untimed runs, median of 3)\n"
    );
    let _ = writeln!(
        s,
        "Peak RSS charges a process for the file-backed pages of its mapped executable and libraries, \
         which are shared and reclaimable, so a larger binary reads higher on every row with the same \
         heap (#5578). Private memory leaves them out: on Linux the largest `RssAnon` sampled while \
         the process ran (a lower bound, and coarser for a run of a few milliseconds, so its ratios and the \
         geomean carry the same caveat), on macOS the peak memory footprint. `n/a` where the platform \
         offers neither.\n"
    );
    let _ = writeln!(
        s,
        "| Query | Input | jq 1.7.1 RSS | succinctly RSS | RSS ratio (succinctly ÷ jq) | jq 1.7.1 private | succinctly private | private ratio |"
    );
    let _ = writeln!(
        s,
        "|-------|-------|--------------|----------------|-----------------------------|------------------|--------------------|---------------|"
    );
    for x in timed {
        let kb = |i: usize, f: fn(&ToolResult) -> Option<u64>| {
            x.tools
                .get(i)
                .and_then(f)
                .map_or_else(|| "n/a".into(), |v| v.to_string())
        };
        let rss = |t: &ToolResult| t.peak_rss_kb;
        let private = |t: &ToolResult| t.private_kb;
        let _ = writeln!(
            s,
            "| {} | {} | {} | {} | {} | {} | {} | {} |",
            x.query,
            x.input,
            kb(1, rss),
            kb(0, rss),
            ratio(x.rss_ratio_primary),
            kb(1, private),
            kb(0, private),
            ratio(x.private_ratio_primary)
        );
    }
}

fn robust_section(s: &mut String, r: &Results) {
    if r.robust.is_empty() {
        return;
    }
    let _ = writeln!(s, "\n## F. Robustness and resource limits\n");
    let _ = writeln!(
        s,
        "Each probe has a 10 s deadline. Class: `ok`, `error` (non-zero exit), `crash` (signal or panic), \
         `timeout`. Stdout and exit status must match jq's; a crash or hang of succinctly is listed \
         even when jq does the same.\n"
    );
    let mut areas: Vec<&str> = r.robust.iter().map(|x| x.area.as_str()).collect();
    areas.sort_unstable();
    areas.dedup();
    let _ = writeln!(s, "| Area | Probes | Agree |");
    let _ = writeln!(s, "|------|--------|-------|");
    for a in areas {
        let all = r.robust.iter().filter(|x| x.area == a).count();
        let ok = r.robust.iter().filter(|x| x.area == a && x.matches).count();
        let _ = writeln!(s, "| {a} | {all} | {ok} |");
    }
    let _ = writeln!(s, "\n| Probe | jq | succinctly | Ledger |");
    let _ = writeln!(s, "|-------|----|-------------|--------|");
    let cls = |c: robust::Class, exit: Option<i32>, ms: f64| {
        let e = exit.map_or_else(|| "signal".into(), |e| e.to_string());
        format!("{c:?} {e} ({ms:.0} ms)").to_lowercase()
    };
    for x in r.robust.iter().filter(|x| {
        !x.matches || matches!(x.subject, robust::Class::Crash | robust::Class::Timeout)
    }) {
        let _ = writeln!(
            s,
            "| `{}` | {} | {} | {} |",
            x.id,
            cls(x.reference, x.reference_exit, x.reference_ms),
            cls(x.subject, x.subject_exit, x.subject_ms),
            note_for(r, &format!("robust:{}", x.id))
        );
    }
}

fn operational_section(s: &mut String, r: &Results) {
    let Some(o) = &r.operational else { return };
    let _ = writeln!(s, "\n## H. Platform and operational properties\n");
    let _ = writeln!(
        s,
        "| Tool | Binary size | Dynamic libraries | CPU time ÷ wall time (`.` on 10 MB) |"
    );
    let _ = writeln!(
        s,
        "|------|-------------|-------------------|--------------------------------------|"
    );
    for b in &o.binaries {
        let par = o
            .parallelism
            .iter()
            .find(|p| p.tool == b.tool)
            .and_then(|p| p.cpu_over_wall)
            .map_or_else(|| "n/a".into(), |v| format!("{v:.2}"));
        let _ = writeln!(
            s,
            "| {} | {:.1} MiB | {} | {par} |",
            b.tool,
            b.bytes as f64 / 1_048_576.0,
            b.dynamic_libs.len()
        );
    }
    let _ = writeln!(
        s,
        "\nA ratio near 1.00 means one thread. Rust dependencies of the shipped feature set \
         (`cargo tree -e normal`): {}. Build time is not measured.\n",
        o.rust_dependencies
            .map_or_else(|| "n/a".into(), |n| n.to_string())
    );
    let _ = writeln!(s, "| Property | Value | Source |");
    let _ = writeln!(s, "|----------|-------|--------|");
    for f in &o.facts {
        let _ = writeln!(s, "| {} | {} | `{}` |", f.item, f.value, f.source);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool(name: &str, rss: Option<u64>, private: Option<u64>) -> ToolResult {
        ToolResult {
            tool: name.into(),
            stats: None,
            cv_pct: None,
            peak_rss_kb: rss,
            private_kb: private,
            instructions: None,
        }
    }

    fn row(tools: Vec<ToolResult>, rss: Option<f64>, private: Option<f64>) -> PerfRow {
        PerfRow {
            query: "startup".into(),
            input: "none".into(),
            tier: "startup".into(),
            gate: Gate::Match,
            gate_secondary: None,
            tools,
            reps: 3,
            ratio_from_min: false,
            speedup_primary: Some(1.0),
            speedup_secondary: None,
            secondary_over_primary: None,
            rss_ratio_primary: rss,
            private_ratio_primary: private,
            ir_ratio_primary: None,
            noisy: false,
        }
    }

    #[test]
    fn memory_table_shows_private_memory_beside_peak_rss() {
        // The #5578 measurement: a 3.4x RSS ratio over a 1.07x private one.
        let r = row(
            vec![
                tool("succinctly", Some(10128), Some(896)),
                tool("jq", Some(3008), Some(840)),
            ],
            Some(3.37),
            Some(1.07),
        );
        let mut s = String::new();
        memory_section(&mut s, &[&r]);
        assert!(s.contains("file-backed pages"), "{s}");
        assert!(
            s.contains("| startup | none | 3008 | 10128 | 3.37x | 840 | 896 | 1.07x |"),
            "{s}"
        );
    }

    #[test]
    fn memory_table_marks_a_platform_without_private_memory() {
        let r = row(
            vec![
                tool("succinctly", Some(10), None),
                tool("jq", Some(5), None),
            ],
            Some(2.0),
            None,
        );
        let mut s = String::new();
        memory_section(&mut s, &[&r]);
        assert!(s.contains("| 5 | 10 | 2.00x | n/a | n/a |"), "{s}");
    }

    fn counted(name: &str, ir: Option<u64>) -> ToolResult {
        ToolResult {
            instructions: ir,
            ..tool(name, None, None)
        }
    }

    #[test]
    fn instruction_table_flags_a_row_whose_signs_disagree_with_the_wall_clock() {
        // Fewer instructions (0.50x) yet jq is faster (speedup 0.80x): the two signals disagree.
        let mut r = row(
            vec![counted("succinctly", Some(500)), counted("jq", Some(1000))],
            None,
            None,
        );
        r.speedup_primary = Some(0.8);
        r.ir_ratio_primary = Some(0.5);
        let mut s = String::new();
        instruction_section(&mut s, &[&r]);
        assert!(
            s.contains("| startup | none | 1000 | 500 | 0.50x | 0.80x | differs |"),
            "{s}"
        );
        // The same instruction ratio with succinctly also faster agrees.
        r.speedup_primary = Some(1.6);
        let mut s = String::new();
        instruction_section(&mut s, &[&r]);
        assert!(s.contains("| 0.50x | 1.60x |  |"), "{s}");
    }

    #[test]
    fn a_tie_on_either_signal_is_not_a_disagreement() {
        assert!(
            !signs_disagree(0.98, 0.8),
            "instructions within 5% of a tie"
        );
        assert!(!signs_disagree(0.5, 0.98), "wall-clock within 5% of a tie");
        assert!(signs_disagree(1.5, 1.5), "more instructions, yet faster");
        assert!(!signs_disagree(1.5, 0.5), "more instructions and slower");
    }

    #[test]
    fn no_instruction_section_without_counts() {
        let r = row(vec![tool("succinctly", None, None)], None, None);
        let mut s = String::new();
        instruction_section(&mut s, &[&r]);
        assert!(s.is_empty(), "{s}");
    }
}

#[cfg(test)]
mod syntax_tests {
    use super::*;

    fn row(id: &str, area: &str, verdict: cli::Verdict) -> cli::CliRow {
        cli::CliRow {
            id: id.into(),
            area: area.into(),
            verdict,
            reference_exit: Some(0),
            subject_exit: if verdict == cli::Verdict::Match {
                Some(0)
            } else {
                Some(3)
            },
        }
    }

    fn entry(id: &str) -> ledger::Entry {
        ledger::Entry {
            id: id.into(),
            category: "to-triage".into(),
            reason: "the parser rejects it (#5590)".into(),
            issue: None,
            platform: None,
            volatile: false,
        }
    }

    fn render(rows: &[cli::CliRow], known: &[ledger::Entry]) -> String {
        let mut s = String::new();
        syntax_section(&mut s, rows, known);
        s
    }

    #[test]
    fn no_syntax_rows_renders_nothing() {
        assert_eq!(render(&[], &[]), "");
    }

    /// The per-area counts come from the rows, and a difference shows its exit codes and its
    /// ledger entry; one that is not listed says so rather than printing a blank.
    #[test]
    fn the_syntax_section_counts_per_area_and_names_each_difference_with_its_ledger_note() {
        let rows = [
            row("op-add", "operators", cli::Verdict::Match),
            row("op-sub", "operators", cli::Verdict::Match),
            row("def-ns", "functions", cli::Verdict::Differs),
            row("def-new", "functions", cli::Verdict::Differs),
        ];
        let out = render(&rows, &[entry("syntax:def-ns")]);
        assert!(out.contains("| operators | 2 | 2 |"), "{out}");
        assert!(out.contains("| functions | 2 | 0 |"), "{out}");
        assert!(
            out.contains("| `def-ns` | 0 | 3 | to-triage: the parser rejects it (#5590) |"),
            "{out}"
        );
        assert!(
            out.contains("| `def-new` | 0 | 3 | **unlisted** |"),
            "{out}"
        );
        assert!(
            !out.contains("`op-add`"),
            "a matching probe is not listed: {out}"
        );
    }

    #[test]
    fn an_all_agreeing_syntax_section_has_no_difference_table() {
        let out = render(&[row("op-add", "operators", cli::Verdict::Match)], &[]);
        assert!(out.contains("| operators | 1 | 1 |"), "{out}");
        assert!(!out.contains("| Probe |"), "{out}");
    }
}
