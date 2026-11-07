//! Shared utilities for benchmark runners.
//!
//! Consolidated from jq_bench.rs, yq_bench.rs, and dsv_bench.rs.
//! Some functions are not currently used but are provided for future use.

#![allow(dead_code)] // STYLE-0005: bench-only helper module (see module doc above)

use anyhow::{Context, Result};
use std::process::{Command, Stdio};

/// Get CPU information for the current system.
pub fn get_cpu_info() -> String {
    #[cfg(target_os = "macos")]
    {
        if let Ok(output) = Command::new("sysctl")
            .args(["-n", "machdep.cpu.brand_string"])
            .output()
        {
            if output.status.success() {
                return String::from_utf8_lossy(&output.stdout).trim().to_string();
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        if let Ok(content) = std::fs::read_to_string("/proc/cpuinfo") {
            for line in content.lines() {
                if line.starts_with("model name") {
                    if let Some(name) = line.split(':').nth(1) {
                        return name.trim().to_string();
                    }
                }
            }
        }
    }

    "Unknown CPU".to_string()
}

/// Get OS version information.
pub fn get_os_version() -> String {
    #[cfg(target_os = "macos")]
    {
        if let Ok(output) = Command::new("sw_vers").args(["-productVersion"]).output() {
            if output.status.success() {
                return format!("macOS {}", String::from_utf8_lossy(&output.stdout).trim());
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        if let Ok(content) = std::fs::read_to_string("/etc/os-release") {
            for line in content.lines() {
                if line.starts_with("PRETTY_NAME=") {
                    return line
                        .strip_prefix("PRETTY_NAME=")
                        .unwrap_or("")
                        .trim_matches('"')
                        .to_string();
                }
            }
        }
        // Fallback to uname
        if let Ok(output) = Command::new("uname").arg("-r").output() {
            if output.status.success() {
                return format!("Linux {}", String::from_utf8_lossy(&output.stdout).trim());
            }
        }
    }

    std::env::consts::OS.to_string()
}

/// Get total RAM in gigabytes.
pub fn get_total_ram_gb() -> f64 {
    #[cfg(target_os = "macos")]
    {
        if let Ok(output) = Command::new("sysctl").args(["-n", "hw.memsize"]).output() {
            if output.status.success() {
                if let Ok(bytes) = String::from_utf8_lossy(&output.stdout)
                    .trim()
                    .parse::<u64>()
                {
                    return bytes as f64 / (1024.0 * 1024.0 * 1024.0);
                }
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        if let Ok(content) = std::fs::read_to_string("/proc/meminfo") {
            for line in content.lines() {
                if line.starts_with("MemTotal:") {
                    if let Some(kb_str) = line.split_whitespace().nth(1) {
                        if let Ok(kb) = kb_str.parse::<u64>() {
                            return kb as f64 / (1024.0 * 1024.0);
                        }
                    }
                }
            }
        }
    }

    0.0
}

/// Format bytes to human-readable string with decimals.
pub fn format_bytes(bytes: u64) -> String {
    if bytes >= 1024 * 1024 * 1024 {
        format!("{:.2} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    } else if bytes >= 1024 * 1024 {
        format!("{:.2} MB", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{:.2} KB", bytes as f64 / 1024.0)
    } else {
        format!("{bytes} bytes")
    }
}

/// Format memory with fixed width (7 chars: 4 for number + space + 2 for unit).
pub fn format_memory_fixed(bytes: u64) -> String {
    if bytes >= 1024 * 1024 * 1024 {
        format!("{:>4.0} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    } else if bytes >= 1024 * 1024 {
        format!("{:>4.0} MB", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{:>4.0} KB", bytes as f64 / 1024.0)
    } else {
        format!("{bytes:>4} B ")
    }
}

/// Format time with fixed width (8 chars total).
pub fn format_time_fixed(ms: f64) -> String {
    if ms >= 1000.0 {
        format!("{:>7.2}s", ms / 1000.0)
    } else {
        let ms_str = format!("{ms:.1}ms");
        format!("{ms_str:>8}")
    }
}

/// Format duration in seconds to human-readable.
pub fn format_duration(seconds: f64) -> String {
    if seconds >= 3600.0 {
        let hours = (seconds / 3600.0).floor();
        let mins = ((seconds % 3600.0) / 60.0).floor();
        format!("{hours:.0}h {mins:.0}m")
    } else if seconds >= 60.0 {
        let mins = (seconds / 60.0).floor();
        let secs = seconds % 60.0;
        format!("{mins:.0}m {secs:.0}s")
    } else {
        format!("{seconds:.1}s")
    }
}

/// Run a command and capture output with timing using /usr/bin/time.
pub fn run_command_with_timing(program: &str, args: &[&str]) -> Result<TimingResult> {
    use std::time::Instant;

    let start = Instant::now();

    let (output, peak_memory, user_time, sys_time) = if cfg!(target_os = "macos") {
        run_with_time_macos(program, args)?
    } else if cfg!(target_os = "linux") {
        run_with_time_linux(program, args)?
    } else {
        let output = Command::new(program)
            .args(args)
            .output()
            .with_context(|| format!("Failed to run {program}"))?;
        (output.stdout, 0, 0.0, 0.0)
    };

    let wall_time = start.elapsed();

    Ok(TimingResult {
        stdout: output,
        peak_memory_bytes: peak_memory,
        wall_time_ms: wall_time.as_secs_f64() * 1000.0,
        user_time_ms: user_time * 1000.0,
        sys_time_ms: sys_time * 1000.0,
    })
}

/// Result from running a command with timing.
pub struct TimingResult {
    pub stdout: Vec<u8>,
    pub peak_memory_bytes: u64,
    pub wall_time_ms: f64,
    pub user_time_ms: f64,
    pub sys_time_ms: f64,
}

/// Run command with /usr/bin/time on macOS.
fn run_with_time_macos(program: &str, args: &[&str]) -> Result<(Vec<u8>, u64, f64, f64)> {
    let mut time_args = vec!["-l", program];
    time_args.extend(args);

    let output = Command::new("/usr/bin/time")
        .args(&time_args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .with_context(|| "Failed to run /usr/bin/time")?;

    let stderr = String::from_utf8_lossy(&output.stderr);

    let peak_memory = parse_macos_memory(&stderr);
    let (user_time, sys_time) = parse_macos_times(&stderr);

    Ok((output.stdout, peak_memory, user_time, sys_time))
}

/// Run command with /usr/bin/time on Linux.
fn run_with_time_linux(program: &str, args: &[&str]) -> Result<(Vec<u8>, u64, f64, f64)> {
    let mut time_args = vec!["-v", program];
    time_args.extend(args);

    let output = Command::new("/usr/bin/time")
        .args(&time_args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .with_context(|| "Failed to run /usr/bin/time")?;

    let stderr = String::from_utf8_lossy(&output.stderr);

    let peak_memory = parse_linux_memory(&stderr);
    let (user_time, sys_time) = parse_linux_times(&stderr);

    Ok((output.stdout, peak_memory, user_time, sys_time))
}

/// Parse peak memory from macOS time output (bytes).
fn parse_macos_memory(stderr: &str) -> u64 {
    for line in stderr.lines() {
        if line.contains("maximum resident set size") {
            if let Some(num) = line.split_whitespace().next() {
                if let Ok(bytes) = num.parse::<u64>() {
                    return bytes;
                }
            }
        }
    }
    0
}

/// Parse user/sys time from macOS time output (seconds).
fn parse_macos_times(stderr: &str) -> (f64, f64) {
    let mut user = 0.0;
    let mut sys = 0.0;

    for line in stderr.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 2 {
            if parts[1] == "user" || line.ends_with(" user") {
                if let Ok(t) = parts[0].parse::<f64>() {
                    user = t;
                }
            } else if parts[1] == "sys" || line.ends_with(" sys") {
                if let Ok(t) = parts[0].parse::<f64>() {
                    sys = t;
                }
            }
        }
    }

    (user, sys)
}

/// Parse peak memory from Linux time output (KB -> bytes).
fn parse_linux_memory(stderr: &str) -> u64 {
    for line in stderr.lines() {
        if line.contains("Maximum resident set size") {
            if let Some(num_str) = line.split(':').next_back() {
                if let Ok(kb) = num_str.trim().parse::<u64>() {
                    return kb * 1024;
                }
            }
        }
    }
    0
}

/// Parse user/sys time from Linux time output (seconds).
fn parse_linux_times(stderr: &str) -> (f64, f64) {
    let mut user = 0.0;
    let mut sys = 0.0;

    for line in stderr.lines() {
        if line.contains("User time") {
            if let Some(num_str) = line.split(':').next_back() {
                if let Ok(t) = num_str.trim().parse::<f64>() {
                    user = t;
                }
            }
        } else if line.contains("System time") {
            if let Some(num_str) = line.split(':').next_back() {
                if let Ok(t) = num_str.trim().parse::<f64>() {
                    sys = t;
                }
            }
        }
    }

    (user, sys)
}

/// Which tool a repetition runs: the reference (jq or yq) or succinctly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tool {
    Reference,
    Succinctly,
}

/// Which tool runs first in repetition `rep`: the reference on even reps, succinctly on odd ones,
/// so neither tool always runs second (#5409, #6417; benchmarking.md rule 1).
pub(crate) fn rep_order(rep: usize) -> [Tool; 2] {
    if rep % 2 == 0 {
        [Tool::Reference, Tool::Succinctly]
    } else {
        [Tool::Succinctly, Tool::Reference]
    }
}

/// Run `warmup` untimed and then `runs` timed repetitions, each running both tools in
/// [`rep_order`]'s order. One counter spans the warmups and the timed reps, so the order keeps
/// alternating across the boundary rather than running one tool twice in a row there. With an odd
/// total one tool leads one rep more often; no order of an odd count can balance that. Returns the
/// timed results per tool (reference first), in rep order. A warmup's error is ignored; a timed
/// rep's error aborts the row.
pub(crate) fn interleaved_reps<T>(
    warmup: usize,
    runs: usize,
    mut run_reference: impl FnMut() -> Result<T>,
    mut run_succinctly: impl FnMut() -> Result<T>,
) -> Result<(Vec<T>, Vec<T>)> {
    for rep in 0..warmup {
        for tool in rep_order(rep) {
            let _ = match tool {
                Tool::Reference => run_reference(),
                Tool::Succinctly => run_succinctly(),
            };
        }
    }
    let mut reference = Vec::with_capacity(runs);
    let mut succinctly = Vec::with_capacity(runs);
    for rep in warmup..warmup + runs {
        for tool in rep_order(rep) {
            match tool {
                Tool::Reference => reference.push(run_reference()?),
                Tool::Succinctly => succinctly.push(run_succinctly()?),
            }
        }
    }
    Ok((reference, succinctly))
}

/// Fail the row when a timed tool exits non-zero (`/usr/bin/time` exits with the child's status):
/// a tool that rejects the query would otherwise be timed on its near-instant error (#5409, #6417).
pub(crate) fn ensure_success(
    program: &str,
    status: std::process::ExitStatus,
    stderr: &str,
) -> Result<()> {
    if status.success() {
        return Ok(());
    }
    let first = stderr.lines().next().unwrap_or("").trim();
    anyhow::bail!("`{program}` exited with {status}: {first}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_bytes() {
        assert_eq!(format_bytes(1024), "1.00 KB");
        assert_eq!(format_bytes(1024 * 1024), "1.00 MB");
        assert_eq!(format_bytes(1024 * 1024 * 1024), "1.00 GB");
    }

    #[test]
    fn test_format_time_fixed() {
        assert_eq!(format_time_fixed(1500.0), "   1.50s");
        assert_eq!(format_time_fixed(500.0), " 500.0ms");
        assert_eq!(format_time_fixed(6.1), "   6.1ms");
    }

    #[test]
    fn test_format_memory_fixed() {
        assert_eq!(format_memory_fixed(1024 * 1024), "   1 MB");
        assert_eq!(format_memory_fixed(512 * 1024 * 1024), " 512 MB");
    }

    #[test]
    fn test_format_duration() {
        assert_eq!(format_duration(30.0), "30.0s");
        assert_eq!(format_duration(90.0), "1m 30s");
        assert_eq!(format_duration(3700.0), "1h 1m");
    }

    #[test]
    fn test_get_cpu_info() {
        let cpu = get_cpu_info();
        assert!(!cpu.is_empty(), "{cpu:?}");
    }
    /// #5409, #6417: each repetition runs both tools, the order flipping every rep, warmups
    /// included, and only the timed reps are returned (benchmarking.md rule 1).
    #[test]
    fn reps_interleave_the_tools_and_alternate_the_order() {
        use std::cell::RefCell;
        let log = RefCell::new(Vec::new());
        let (reference, succ) = interleaved_reps(
            2,
            3,
            || {
                let mut l = log.borrow_mut();
                l.push("ref");
                Ok(l.len())
            },
            || {
                let mut l = log.borrow_mut();
                l.push("succ");
                Ok(l.len())
            },
        )
        .unwrap();
        assert_eq!(
            log.into_inner(),
            [
                // warmup reps 0 and 1
                "ref", "succ", "succ", "ref", //
                // timed reps 2, 3 and 4: the order carries on across the boundary
                "ref", "succ", "succ", "ref", "ref", "succ",
            ]
        );
        // The values are the call positions: the warmups' four are discarded.
        assert_eq!(reference, [5, 8, 9]);
        assert_eq!(succ, [6, 7, 10]);
        // After one warmup (reference first) the first timed rep leads with succinctly.
        let log = RefCell::new(Vec::new());
        interleaved_reps(
            1,
            1,
            || -> Result<()> {
                log.borrow_mut().push("ref");
                Ok(())
            },
            || {
                log.borrow_mut().push("succ");
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(log.into_inner(), ["ref", "succ", "succ", "ref"]);
    }

    #[test]
    fn a_failed_timed_rep_aborts_and_a_failed_warmup_does_not() {
        let mut calls = 0;
        let err = interleaved_reps(
            1,
            2,
            || -> Result<()> {
                calls += 1;
                if calls == 1 {
                    anyhow::bail!("warmup failure is ignored")
                }
                anyhow::bail!("timed failure")
            },
            || Ok(()),
        )
        .unwrap_err();
        assert_eq!(err.to_string(), "timed failure");
        assert_eq!(calls, 2, "the row stops at the first timed failure");
    }

    #[cfg(unix)]
    #[test]
    fn a_non_zero_exit_names_the_program_and_its_first_stderr_line() {
        let status = Command::new("sh").args(["-c", "exit 3"]).status().unwrap();
        let err = ensure_success("sh", status, "boom\nmore").unwrap_err();
        assert!(err.to_string().contains("`sh` exited with"), "{err}");
        assert!(err.to_string().ends_with(": boom"), "{err}");
        let ok = Command::new("true").status().unwrap();
        assert!(ensure_success("true", ok, "").is_ok());
    }
}
