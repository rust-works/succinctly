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
            // An Arm Linux kernel prints no `model name`: name the core from its MIDR
            // fields instead of reporting "Unknown CPU" (#6873).
            if let Some(name) = arm_core_name(&content) {
                return name;
            }
        }
    }

    "Unknown CPU".to_string()
}

/// The name of the first Arm core in a `/proc/cpuinfo` dump that has no `model name`
/// line (every aarch64 Linux kernel): `ARM Neoverse-V2` for implementer `0x41`, part
/// `0xd4f`. The parts are the `ARM_CPU_PART_NEOVERSE_*` values of the kernel's
/// `arch/arm64/include/asm/cputype.h`; a core not in that list is named by its two
/// fields, so the report still says which CPU it ran on. `None` when the dump has no
/// Arm implementer and part, as on x86.
#[cfg(any(target_os = "linux", test))]
fn arm_core_name(cpuinfo: &str) -> Option<String> {
    let field = |key: &str| {
        cpuinfo.lines().find_map(|l| {
            let (k, v) = l.split_once(':')?;
            (k.trim() == key).then(|| v.trim().to_ascii_lowercase())
        })
    };
    let (implementer, part) = (field("CPU implementer")?, field("CPU part")?);
    let core = match (implementer.as_str(), part.as_str()) {
        ("0x41", "0xd0c") => "Neoverse-N1",
        ("0x41", "0xd40") => "Neoverse-V1",
        ("0x41", "0xd49") => "Neoverse-N2",
        ("0x41", "0xd4f") => "Neoverse-V2",
        ("0x41", "0xd84") => "Neoverse-V3",
        ("0x41", "0xd8e") => "Neoverse-N3",
        _ => return Some(format!("Arm core (implementer {implementer}, part {part})")),
    };
    Some(format!("ARM {core}"))
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

    /// The head of `/proc/cpuinfo` on an AWS Graviton 4 (c8g.large) instance: no
    /// `model name`, a tab before the colon on most keys and none on `CPU architecture`.
    const GRAVITON4: &str = "processor\t: 0\nBogoMIPS\t: 2000.00\n\
        Features\t: fp asimd evtstrm aes sve sve2\nCPU implementer\t: 0x41\n\
        CPU architecture: 8\nCPU variant\t: 0x0\nCPU part\t: 0xd4f\nCPU revision\t: 1\n\n\
        processor\t: 1\nCPU implementer\t: 0x41\nCPU part\t: 0xd4f\n";

    #[test]
    fn a_graviton4_is_named_from_its_midr_fields_6873() {
        assert_eq!(arm_core_name(GRAVITON4).as_deref(), Some("ARM Neoverse-V2"));
    }

    #[test]
    fn each_neoverse_part_has_its_own_name_6873() {
        let name =
            |part: &str| arm_core_name(&format!("CPU implementer\t: 0x41\nCPU part\t: {part}\n"));
        assert_eq!(name("0xd0c").as_deref(), Some("ARM Neoverse-N1"));
        assert_eq!(name("0xd40").as_deref(), Some("ARM Neoverse-V1"));
        assert_eq!(name("0xd49").as_deref(), Some("ARM Neoverse-N2"));
        assert_eq!(name("0xd4f").as_deref(), Some("ARM Neoverse-V2"));
        assert_eq!(name("0xd84").as_deref(), Some("ARM Neoverse-V3"));
        assert_eq!(name("0xd8e").as_deref(), Some("ARM Neoverse-N3"));
        // The kernel prints the part in lower case, but a hand-edited dump may not.
        assert_eq!(name("0xD4F").as_deref(), Some("ARM Neoverse-V2"));
    }

    #[test]
    fn an_unlisted_arm_core_is_named_by_its_fields_6873() {
        let dump = "CPU implementer\t: 0x41\nCPU part\t: 0xd0b\n";
        assert_eq!(
            arm_core_name(dump).as_deref(),
            Some("Arm core (implementer 0x41, part 0xd0b)")
        );
        let other = "CPU implementer\t: 0x51\nCPU part\t: 0xd4f\n";
        assert_eq!(
            arm_core_name(other).as_deref(),
            Some("Arm core (implementer 0x51, part 0xd4f)"),
            "part 0xd4f is Neoverse-V2 only under implementer 0x41"
        );
    }

    #[test]
    fn a_dump_without_the_arm_fields_names_no_core_6873() {
        assert_eq!(arm_core_name("model name\t: AMD Ryzen 9 7950X\n"), None);
        assert_eq!(arm_core_name("CPU implementer\t: 0x41\n"), None);
        assert_eq!(arm_core_name("CPU part\t: 0xd4f\n"), None);
        assert_eq!(arm_core_name(""), None);
    }
}
