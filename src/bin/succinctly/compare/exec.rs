//! Run one tool invocation with a deadline and capture all three results (#5409).

use super::harness::Tool;
use anyhow::{Context, Result};
use serde::Serialize;
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const DEADLINE: Duration = Duration::from_secs(10);

/// The perf gate runs whole-corpus queries that legitimately take minutes in jq.
pub const PERF_DEADLINE: Duration = Duration::from_secs(900);

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Exec {
    /// `None` when killed by a signal or by the deadline.
    pub exit: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub timed_out: bool,
}

impl Exec {
    /// Both tools print jq's own `name/arity is not defined` at compile time.
    pub fn is_undefined(&self) -> bool {
        self.exit == Some(3) && String::from_utf8_lossy(&self.stderr).contains("is not defined")
    }
}

/// Run `tool` with `args` and `stdin` in `cwd`. The environment is fixed
/// (`TZ=UTC`, `LC_ALL=C`) so a locale or zone cannot read as a divergence.
pub fn exec(tool: &Tool, args: &[String], stdin: impl AsRef<[u8]>, cwd: &Path) -> Result<Exec> {
    exec_within(tool, args, stdin, cwd, DEADLINE)
}

pub fn exec_within(
    tool: &Tool,
    args: &[String],
    stdin: impl AsRef<[u8]>,
    cwd: &Path,
    deadline: Duration,
) -> Result<Exec> {
    let mut child = Command::new(&tool.program)
        .args(&tool.prefix)
        .args(args)
        .current_dir(cwd)
        .env("TZ", "UTC")
        .env("LC_ALL", "C")
        .env_remove("JQ_COLORS")
        .env_remove("NO_COLOR")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("spawning {} ({})", tool.name, tool.program))?;
    let mut sin = child.stdin.take().expect("piped");
    let input = stdin.as_ref().to_vec();
    let writer = std::thread::spawn(move || {
        let _ = sin.write_all(&input);
    });
    let (mut so, mut se) = (child.stdout.take().unwrap(), child.stderr.take().unwrap());
    let ro = std::thread::spawn(move || {
        let mut v = vec![];
        let _ = so.read_to_end(&mut v);
        v
    });
    let re = std::thread::spawn(move || {
        let mut v = vec![];
        let _ = se.read_to_end(&mut v);
        v
    });
    let start = Instant::now();
    let (status, timed_out) = loop {
        if let Some(s) = child.try_wait()? {
            break (s.code(), false);
        }
        if start.elapsed() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            break (None, true);
        }
        std::thread::sleep(Duration::from_millis(1));
    };
    let _ = writer.join();
    Ok(Exec {
        exit: status,
        stdout: ro.join().unwrap_or_default(),
        stderr: re.join().unwrap_or_default(),
        timed_out,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sh(script: &str) -> Tool {
        Tool {
            name: "sh".into(),
            program: "sh".into(),
            prefix: vec!["-c".into(), script.into(), "sh".into()],
        }
    }

    #[test]
    fn captures_stdout_stderr_and_exit() {
        let e = exec(&sh("cat; echo err >&2; exit 4"), &[], "in", Path::new(".")).unwrap();
        assert_eq!(e.exit, Some(4));
        assert_eq!(
            (e.stdout.as_slice(), e.stderr.as_slice()),
            (&b"in"[..], &b"err\n"[..])
        );
    }

    #[test]
    fn undefined_is_recognised_only_with_exit_3_and_the_message() {
        let msg = "echo 'jq: error: f/0 is not defined' >&2; exit 3";
        assert!(exec(&sh(msg), &[], "", Path::new("."))
            .unwrap()
            .is_undefined());
        let other = "echo 'jq: error: f/0 is not defined' >&2; exit 5";
        assert!(!exec(&sh(other), &[], "", Path::new("."))
            .unwrap()
            .is_undefined());
    }

    #[test]
    fn environment_is_fixed() {
        let e = exec(&sh("echo $TZ $LC_ALL"), &[], "", Path::new(".")).unwrap();
        assert_eq!(e.stdout, b"UTC C\n");
    }
}
