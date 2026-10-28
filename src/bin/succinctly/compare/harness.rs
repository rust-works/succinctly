//! Running one filter through two tools: output gate and interleaved timing (#5409).

use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::io::Write;
use std::process::{Command, Stdio};
use std::time::Instant;

#[derive(Debug, Clone)]
pub struct Tool {
    pub name: String,
    pub program: String,
    /// Arguments placed before the flags (`["jq"]` for `succinctly jq`).
    pub prefix: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Query {
    pub id: String,
    pub flags: Vec<String>,
    pub filter: String,
    pub input: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Outcome {
    pub exit: Option<i32>,
    pub stdout: Vec<u8>,
}

/// Parse a `*.tsv` query file: `id<TAB>flags<TAB>filter<TAB>input`.
pub fn parse_queries(text: &str) -> Result<Vec<Query>> {
    let mut out = vec![];
    for (n, line) in text.lines().enumerate() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let cols: Vec<&str> = line.splitn(4, '\t').collect();
        if cols.len() != 4 {
            bail!("query line {}: expected 4 tab-separated columns", n + 1);
        }
        out.push(Query {
            id: cols[0].to_string(),
            flags: if cols[1] == "-" {
                vec![]
            } else {
                cols[1].split(' ').map(String::from).collect()
            },
            filter: cols[2].to_string(),
            input: cols[3].to_string(),
        });
    }
    Ok(out)
}

/// Run `q` once; returns the outcome and wall time in milliseconds.
pub fn run_once(tool: &Tool, q: &Query) -> Result<(Outcome, f64)> {
    let mut child = Command::new(&tool.program)
        .args(&tool.prefix)
        .args(&q.flags)
        .arg(&q.filter)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("spawning {} ({})", tool.name, tool.program))?;
    let mut stdin = child.stdin.take().expect("piped");
    let input = q.input.clone();
    let writer = std::thread::spawn(move || {
        let _ = stdin.write_all(input.as_bytes());
    });
    let start = Instant::now();
    let out = child.wait_with_output()?;
    let ms = start.elapsed().as_secs_f64() * 1e3;
    let _ = writer.join();
    Ok((
        Outcome {
            exit: out.status.code(),
            stdout: out.stdout,
        },
        ms,
    ))
}

#[derive(Debug, Clone, Serialize)]
pub struct Stats {
    pub n: usize,
    pub min_ms: f64,
    pub median_ms: f64,
    /// Median absolute deviation.
    pub mad_ms: f64,
}

fn median(sorted: &[f64]) -> f64 {
    let n = sorted.len();
    if n % 2 == 1 {
        sorted[n / 2]
    } else {
        (sorted[n / 2 - 1] + sorted[n / 2]) / 2.0
    }
}

pub fn stats(samples: &[f64]) -> Stats {
    let mut s = samples.to_vec();
    s.sort_by(f64::total_cmp);
    let med = median(&s);
    let mut dev: Vec<f64> = s.iter().map(|x| (x - med).abs()).collect();
    dev.sort_by(f64::total_cmp);
    Stats {
        n: s.len(),
        min_ms: s[0],
        median_ms: med,
        mad_ms: median(&dev),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Verdict {
    Match,
    Differs,
}

#[derive(Debug, Clone, Serialize)]
pub struct Row {
    pub id: String,
    pub verdict: Verdict,
    pub reference_exit: Option<i32>,
    pub subject_exit: Option<i32>,
    /// Present only when the outputs match: a speedup of differing output is
    /// a correctness row, not a timing one (docs/guides/benchmarking.md).
    pub reference: Option<Stats>,
    pub subject: Option<Stats>,
    pub speedup: Option<f64>,
}

/// Gate on output identity, then time `reps` interleaved repetitions
/// (the order within a repetition alternates, so neither tool always runs warm).
pub fn compare(reference: &Tool, subject: &Tool, q: &Query, reps: usize) -> Result<Row> {
    let (r, _) = run_once(reference, q)?;
    let (s, _) = run_once(subject, q)?;
    let verdict = if r == s {
        Verdict::Match
    } else {
        Verdict::Differs
    };
    let mut row = Row {
        id: q.id.clone(),
        verdict,
        reference_exit: r.exit,
        subject_exit: s.exit,
        reference: None,
        subject: None,
        speedup: None,
    };
    if verdict == Verdict::Differs || reps == 0 {
        return Ok(row);
    }
    let (mut rt, mut st) = (vec![], vec![]);
    for i in 0..reps {
        if i % 2 == 0 {
            rt.push(run_once(reference, q)?.1);
            st.push(run_once(subject, q)?.1);
        } else {
            st.push(run_once(subject, q)?.1);
            rt.push(run_once(reference, q)?.1);
        }
    }
    let (rs, ss) = (stats(&rt), stats(&st));
    row.speedup = Some(rs.median_ms / ss.median_ms);
    row.reference = Some(rs);
    row.subject = Some(ss);
    Ok(row)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sh(name: &str, script: &str) -> Tool {
        Tool {
            name: name.into(),
            program: "sh".into(),
            prefix: vec!["-c".into(), script.into(), "sh".into()],
        }
    }

    fn q() -> Query {
        Query {
            id: "t".into(),
            flags: vec![],
            filter: ".".into(),
            input: "1".into(),
        }
    }

    #[test]
    fn identical_tools_match_and_get_timed() {
        let t = sh("a", "cat >/dev/null; echo ok");
        let row = compare(&t, &t, &q(), 3).unwrap();
        assert_eq!(row.verdict, Verdict::Match);
        assert_eq!(row.reference.unwrap().n, 3);
        assert!(row.speedup.is_some());
    }

    /// The gate must be able to fail: a differing output is reported as such
    /// and is never given a speedup.
    #[test]
    fn differing_output_is_caught_and_untimed() {
        let a = sh("a", "cat >/dev/null; echo 1");
        let b = sh("b", "cat >/dev/null; echo 2");
        let row = compare(&a, &b, &q(), 3).unwrap();
        assert_eq!(row.verdict, Verdict::Differs);
        assert!(row.speedup.is_none() && row.subject.is_none());
    }

    #[test]
    fn differing_exit_status_is_caught() {
        let a = sh("a", "cat >/dev/null; exit 0");
        let b = sh("b", "cat >/dev/null; exit 5");
        assert_eq!(compare(&a, &b, &q(), 0).unwrap().verdict, Verdict::Differs);
    }

    #[test]
    fn stats_are_robust_to_one_outlier() {
        let s = stats(&[1.0, 1.0, 1.0, 1.0, 100.0]);
        assert_eq!((s.min_ms, s.median_ms, s.mad_ms), (1.0, 1.0, 0.0));
    }

    #[test]
    fn query_file_parses_and_rejects_short_lines() {
        let qs = parse_queries("# c\na\t-c\t.x\t{\"x\":1}\n").unwrap();
        assert_eq!(
            (qs[0].flags.clone(), qs[0].filter.as_str()),
            (vec!["-c".to_string()], ".x")
        );
        assert!(parse_queries("a\tb\n").is_err());
    }

    #[test]
    fn checked_in_smoke_queries_parse() {
        let text = include_str!("../../../../comparison/jq/queries/smoke.tsv");
        assert!(parse_queries(text).unwrap().len() >= 10);
    }
}
