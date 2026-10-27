//! `report.md`, generated from the results (#5409). Never hand-edited.

use super::harness::{Row, Verdict};
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
    s.push('\n');

    let _ = writeln!(s, "## Dimensions\n");
    let _ = writeln!(s, "| Dimension | Status |");
    let _ = writeln!(s, "|-----------|--------|");
    for d in &r.dimensions {
        let _ = writeln!(s, "| {} | {} |", d.name, d.status);
    }

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

    let _ = writeln!(s, "\n## Reproduce\n");
    let _ = writeln!(s, "```bash\ncargo build --release --features bench-runner");
    let _ = writeln!(s, "{}\n```", r.command);
    let _ = writeln!(
        s,
        "\nmanifest sha256 `{}` · succinctly sha256 `{}` · jq sha256 `{}`",
        r.manifest_sha256, e.succinctly_sha256, e.jq_sha256
    );
    s
}
