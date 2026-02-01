//! Shared utilities for JSON SIMD benchmarks.
//!
//! This module contains common code used by the split json_simd benchmarks:
//! - json_simd_indexing
//! - json_simd_cursor
//! - json_simd_full

use std::path::PathBuf;

/// Discover all JSON files in data/bench/generated/ directory.
/// Returns a sorted list of (display_name, path, file_size) tuples.
pub fn discover_json_files() -> Vec<(String, PathBuf, u64)> {
    let base_dir = PathBuf::from("data/bench/generated");
    let mut files = Vec::new();

    if !base_dir.exists() {
        eprintln!(
            "Warning: {} does not exist. Run `cargo run --features cli -- json generate-suite` first.",
            base_dir.display()
        );
        return files;
    }

    // Iterate through pattern directories
    if let Ok(patterns) = std::fs::read_dir(&base_dir) {
        for pattern_entry in patterns.flatten() {
            let pattern_path = pattern_entry.path();
            if !pattern_path.is_dir() {
                continue;
            }

            let pattern_name = pattern_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown");

            // Iterate through size files in each pattern directory
            if let Ok(size_files) = std::fs::read_dir(&pattern_path) {
                for file_entry in size_files.flatten() {
                    let file_path = file_entry.path();
                    if file_path.extension().is_some_and(|e| e == "json") {
                        let size_name = file_path
                            .file_stem()
                            .and_then(|n| n.to_str())
                            .unwrap_or("unknown");

                        let file_size = std::fs::metadata(&file_path).map(|m| m.len()).unwrap_or(0);

                        // Create display name: pattern/size (e.g., "comprehensive/10mb")
                        let display_name = format!("{}/{}", pattern_name, size_name);
                        files.push((display_name, file_path, file_size));
                    }
                }
            }
        }
    }

    // Sort by pattern name, then by file size
    files.sort_by(|a, b| {
        let (a_pattern, a_size) = a.0.split_once('/').unwrap_or((&a.0, ""));
        let (b_pattern, b_size) = b.0.split_once('/').unwrap_or((&b.0, ""));

        a_pattern.cmp(b_pattern).then_with(|| {
            // Parse sizes for proper numerical ordering
            parse_size_order(a_size).cmp(&parse_size_order(b_size))
        })
    });

    files
}

/// Parse size string to a comparable number for ordering.
pub fn parse_size_order(s: &str) -> u64 {
    let s = s.to_lowercase();
    if s.ends_with("gb") {
        s.trim_end_matches("gb")
            .parse::<u64>()
            .unwrap_or(0)
            .saturating_mul(1024 * 1024 * 1024)
    } else if s.ends_with("mb") {
        s.trim_end_matches("mb")
            .parse::<u64>()
            .unwrap_or(0)
            .saturating_mul(1024 * 1024)
    } else if s.ends_with("kb") {
        s.trim_end_matches("kb")
            .parse::<u64>()
            .unwrap_or(0)
            .saturating_mul(1024)
    } else {
        s.parse::<u64>().unwrap_or(0)
    }
}

/// Filter files by maximum size (skip files larger than this).
pub fn filter_by_max_size(
    files: Vec<(String, PathBuf, u64)>,
    max_bytes: u64,
) -> Vec<(String, PathBuf, u64)> {
    files
        .into_iter()
        .filter(|(_, _, size)| *size <= max_bytes)
        .collect()
}
