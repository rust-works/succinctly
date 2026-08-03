//! Guards `.omni-dev/scopes.yaml` against the drift found in #568: 11 of 27
//! `file_patterns` pointed at paths that no longer existed, three real modules
//! (`src/dsv/`, `src/text/`, `src/util/`) had no scope at all, and the scope list
//! was duplicated across four files, three of which disagreed with `scopes.yaml`.
//!
//! `scopes.yaml` is parsed by hand rather than with a YAML crate: its shape (a
//! flat list of single-line-array maps) doesn't need one, this test must also
//! run under the plain `cargo test --verbose` CI step where `regex` isn't
//! enabled, and the project already hand-rolls its own YAML/JSON parsers.
//!
//! No off-the-shelf glob crate is used either: #568 found that naive recursive
//! globbing (Python's `glob.glob('src/simd/**', recursive=True)`) can report a
//! match even when the concrete directory doesn't exist, because `**` also
//! matches the empty string. The existence checks below always strip the `/**`
//! tail and `stat` the literal prefix directly instead of trusting any glob
//! library's semantics.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

const REPO_ROOT: &str = env!("CARGO_MANIFEST_DIR");

#[derive(Debug, Clone)]
struct Scope {
    name: String,
    description: String,
    file_patterns: Vec<String>,
}

fn omni_dev_dir() -> PathBuf {
    PathBuf::from(REPO_ROOT).join(".omni-dev")
}

fn scopes_yaml_text() -> String {
    let path = omni_dev_dir().join("scopes.yaml");
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

fn commit_guidelines_text() -> String {
    let path = omni_dev_dir().join("commit-guidelines.md");
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// Strips a leading/trailing `"` pair, if present.
fn unquote(s: &str) -> String {
    s.trim().trim_matches('"').to_string()
}

/// Splits a `["a", "b, c", "d"]`-shaped value into its quoted elements,
/// ignoring commas that fall inside a quoted string (none do in `scopes.yaml`
/// today, but a future entry shouldn't be free to silently mis-split).
fn split_bracketed_list(s: &str) -> Vec<String> {
    let inner = s.trim().trim_start_matches('[').trim_end_matches(']');
    split_top_level_commas(inner)
        .into_iter()
        .map(unquote)
        .filter(|s| !s.is_empty())
        .collect()
}

fn split_top_level_commas(s: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut in_quotes = false;
    for (i, ch) in s.char_indices() {
        match ch {
            '"' => in_quotes = !in_quotes,
            ',' if !in_quotes => {
                parts.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(&s[start..]);
    parts
}

/// Hand-rolled parser for `scopes.yaml`'s fixed shape: a top-level `scopes:`
/// list of maps, each with single-line `name`, `description`, `examples`, and
/// `file_patterns` fields. Not a general YAML parser — it only needs to
/// survive the exact structure this file uses.
fn parse_scopes_yaml(text: &str) -> Vec<Scope> {
    let mut scopes = Vec::new();
    let mut current: Option<Scope> = None;
    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("- name:") {
            if let Some(scope) = current.take() {
                scopes.push(scope);
            }
            current = Some(Scope {
                name: unquote(rest),
                description: String::new(),
                file_patterns: Vec::new(),
            });
        } else if let Some(rest) = trimmed.strip_prefix("description:") {
            if let Some(scope) = current.as_mut() {
                scope.description = unquote(rest);
            }
        } else if let Some(rest) = trimmed.strip_prefix("file_patterns:") {
            if let Some(scope) = current.as_mut() {
                scope.file_patterns = split_bracketed_list(rest);
            }
        }
    }
    if let Some(scope) = current.take() {
        scopes.push(scope);
    }
    scopes
}

/// Whether `pattern` (a `file_patterns` entry, forward-slash separated)
/// matches at least one real path under `root`.
fn pattern_matches_something(root: &Path, pattern: &str) -> bool {
    if let Some(prefix) = pattern.strip_suffix("/**") {
        return matches_dir_glob(root, prefix);
    }
    if !pattern.contains('/') && pattern.contains('*') {
        return matches_bare_wildcard(root, pattern);
    }
    root.join(pattern).exists()
}

/// Resolves a `/**`-stripped prefix like `src/*/simd` against real
/// directories, one `*` wildcard segment matching any single directory name.
fn matches_dir_glob(root: &Path, prefix: &str) -> bool {
    let mut candidates = vec![root.to_path_buf()];
    for segment in prefix.split('/') {
        let mut next = Vec::new();
        for dir in &candidates {
            if segment == "*" {
                if let Ok(entries) = fs::read_dir(dir) {
                    next.extend(
                        entries
                            .filter_map(Result::ok)
                            .map(|e| e.path())
                            .filter(|p| p.is_dir()),
                    );
                }
            } else {
                let candidate = dir.join(segment);
                if candidate.is_dir() {
                    next.push(candidate);
                }
            }
        }
        candidates = next;
        if candidates.is_empty() {
            return false;
        }
    }
    !candidates.is_empty()
}

/// Matches a bare `*.ext` pattern (no `/`) against `root`'s top-level entries
/// only — deliberately non-recursive, since nothing in `scopes.yaml` needs
/// recursive extension matching.
fn matches_bare_wildcard(root: &Path, pattern: &str) -> bool {
    let suffix = pattern
        .strip_prefix('*')
        .unwrap_or_else(|| panic!("unsupported bare wildcard pattern: {pattern:?}"));
    let Ok(entries) = fs::read_dir(root) else {
        return false;
    };
    entries
        .filter_map(Result::ok)
        .any(|e| e.path().is_file() && e.file_name().to_string_lossy().ends_with(suffix))
}

/// Whether `pattern` (from a scope's `file_patterns`) covers `target_dir`
/// (a repo-relative directory like `"src/bits"`), by segment shape alone —
/// no filesystem access, since the caller already knows `target_dir` is real.
///
/// Only `/**`-suffixed patterns can cover a directory: the pattern's stripped
/// segments must be a prefix of the target's segments, `*` standing for
/// exactly one segment.
fn pattern_covers_dir(pattern: &str, target_dir: &str) -> bool {
    let Some(prefix) = pattern.strip_suffix("/**") else {
        return false;
    };
    let pattern_segs: Vec<&str> = prefix.split('/').collect();
    let target_segs: Vec<&str> = target_dir.split('/').collect();
    if pattern_segs.len() > target_segs.len() {
        return false;
    }
    pattern_segs
        .iter()
        .zip(&target_segs)
        .all(|(p, t)| *p == "*" || p == t)
}

/// Real, one-level directory names under `dir`, as `"<dir's own name>/<name>"`
/// relative paths (e.g. `"src/bits"`), sorted for stable failure messages.
fn one_level_subdirs(dir: &Path) -> Vec<String> {
    let dir_name = dir.file_name().unwrap().to_string_lossy().into_owned();
    let mut names: Vec<String> = fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("read {}: {e}", dir.display()))
        .filter_map(Result::ok)
        .filter(|e| e.path().is_dir())
        .map(|e| format!("{dir_name}/{}", e.file_name().to_string_lossy()))
        .collect();
    names.sort();
    names
}

/// Returns the body of the `## <heading>` section, up to (not including) the
/// next `## ` heading. `### ` subheadings do not terminate the slice.
fn section<'a>(markdown: &'a str, heading: &str) -> &'a str {
    let marker = format!("\n## {heading}\n");
    let start = markdown
        .find(&marker)
        .unwrap_or_else(|| panic!("commit-guidelines.md has no `## {heading}` section"))
        + marker.len();
    let end = markdown[start..]
        .find("\n## ")
        .map_or(markdown.len(), |offset| start + offset);
    &markdown[start..end]
}

/// Parses `- \`name\` - description` bullets into name -> description.
fn markdown_scopes(body: &str) -> BTreeMap<String, String> {
    body.lines()
        .filter_map(|line| {
            let rest = line.trim_end().strip_prefix("- `")?;
            let (name, rest) = rest.split_once('`')?;
            let description = rest.strip_prefix(" - ")?;
            is_scope_name(name).then(|| (name.to_string(), description.trim().to_string()))
        })
        .collect()
}

fn is_scope_name(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// Finds `type(scope):` (optionally `type(scope)!:`) header lines inside the
/// `## Examples` section and returns each `(scope, containing line)` pair.
fn scopes_used_in_examples(examples_section: &str) -> Vec<(String, String)> {
    let mut found = Vec::new();
    for line in examples_section.lines() {
        let line = line.trim();
        let Some(open) = line.find('(') else { continue };
        let Some(close_rel) = line[open..].find(')') else {
            continue;
        };
        let close = open + close_rel;
        let head = &line[..open];
        if head.is_empty() || !head.chars().all(|c| c.is_ascii_lowercase()) {
            continue;
        }
        let after_close = &line[close + 1..];
        let after_bang = after_close.strip_prefix('!').unwrap_or(after_close);
        if !after_bang.starts_with(':') {
            continue;
        }
        for scope in line[open + 1..close].split(',') {
            found.push((scope.trim().to_string(), line.to_string()));
        }
    }
    found
}

#[test]
fn file_patterns_match_real_paths() {
    let root = PathBuf::from(REPO_ROOT);
    let scopes = parse_scopes_yaml(&scopes_yaml_text());

    let mut dead: Vec<String> = scopes
        .iter()
        .flat_map(|scope| {
            scope
                .file_patterns
                .iter()
                .filter(|pattern| !pattern_matches_something(&root, pattern))
                .map(|pattern| format!("{}: {pattern:?}", scope.name))
        })
        .collect();
    dead.sort();

    assert!(
        dead.is_empty(),
        "the following file_patterns in .omni-dev/scopes.yaml match no real path (#568):\n  {}",
        dead.join("\n  ")
    );
}

#[test]
fn every_src_subdirectory_has_a_scope() {
    let src = PathBuf::from(REPO_ROOT).join("src");
    let scopes = parse_scopes_yaml(&scopes_yaml_text());
    let subdirs = one_level_subdirs(&src);

    let mut uncovered: Vec<String> = subdirs
        .into_iter()
        .filter(|dir| {
            !scopes.iter().any(|scope| {
                scope
                    .file_patterns
                    .iter()
                    .any(|p| pattern_covers_dir(p, dir))
            })
        })
        .collect();
    uncovered.sort();

    assert!(
        uncovered.is_empty(),
        "the following src/* subdirectories are matched by no scope in .omni-dev/scopes.yaml \
         (#568) — add a file_patterns entry that covers them:\n  {}",
        uncovered.join("\n  ")
    );
}

#[test]
fn example_scopes_are_defined_in_scopes_yaml() {
    let yaml_scopes = parse_scopes_yaml(&scopes_yaml_text());
    let known: Vec<String> = yaml_scopes.into_iter().map(|s| s.name).collect();
    let guidelines = commit_guidelines_text();
    let examples = section(&guidelines, "Examples");

    let mut undefined: Vec<String> = scopes_used_in_examples(examples)
        .into_iter()
        .filter(|(scope, _)| !known.contains(scope))
        .map(|(scope, line)| format!("`{scope}` used in `{line}`"))
        .collect();
    undefined.sort();

    assert!(
        undefined.is_empty(),
        "the `## Examples` section of .omni-dev/commit-guidelines.md uses scopes that are not \
         defined in .omni-dev/scopes.yaml (#568):\n  {}",
        undefined.join("\n  ")
    );
}

#[test]
fn skill_and_style_guide_point_at_scopes_yaml_instead_of_hardcoding_it() {
    for path in [
        ".claude/skills/commit-msg/SKILL.md",
        "docs/STYLE_GUIDE.md",
        ".omni-dev/commit-guidelines.md",
    ] {
        let full_path = PathBuf::from(REPO_ROOT).join(path);
        let text = fs::read_to_string(&full_path)
            .unwrap_or_else(|e| panic!("read {}: {e}", full_path.display()));
        assert!(
            text.contains(".omni-dev/scopes.yaml"),
            "{path} should point at .omni-dev/scopes.yaml as the source of truth for commit \
             scopes, not restate them (#568, #588)"
        );
    }

    let skill_md =
        fs::read_to_string(PathBuf::from(REPO_ROOT).join(".claude/skills/commit-msg/SKILL.md"))
            .expect("read SKILL.md");
    assert!(
        !skill_md.contains("## Scopes for This Project"),
        "SKILL.md has reintroduced a hardcoded scope table (#568) — point at \
         .omni-dev/scopes.yaml instead of a competing list"
    );

    let guidelines = commit_guidelines_text();
    let markdown = markdown_scopes(section(&guidelines, "Scopes"));
    assert!(
        markdown.is_empty(),
        "commit-guidelines.md's `## Scopes` section has reintroduced a hardcoded scope list \
         (#588) — point at .omni-dev/scopes.yaml instead of a competing, drift-prone copy:\n  {}",
        markdown.keys().cloned().collect::<Vec<_>>().join(", ")
    );
}

#[test]
fn uncovered_dirs_flags_a_new_module_with_no_scope() {
    let tmp = tempfile::tempdir().expect("create tempdir");
    let src = tmp.path().join("src");
    fs::create_dir_all(src.join("bits")).unwrap();
    fs::create_dir_all(src.join("newmodule")).unwrap();

    let subdirs = one_level_subdirs(&src);
    let scopes = [Scope {
        name: "bitvec".to_string(),
        description: String::new(),
        file_patterns: vec!["src/bits/**".to_string()],
    }];

    let uncovered: Vec<String> = subdirs
        .into_iter()
        .filter(|dir| {
            !scopes.iter().any(|scope| {
                scope
                    .file_patterns
                    .iter()
                    .any(|p| pattern_covers_dir(p, dir))
            })
        })
        .collect();

    assert_eq!(uncovered, vec!["src/newmodule".to_string()]);
}

#[test]
fn pattern_covers_dir_requires_full_prefix_match() {
    assert!(pattern_covers_dir("src/bits/**", "src/bits"));
    assert!(pattern_covers_dir("src/*/simd/**", "src/json/simd"));
    assert!(!pattern_covers_dir("src/*/simd/**", "src/bits"));
    assert!(!pattern_covers_dir("src/bits/**", "src/trees"));
    assert!(!pattern_covers_dir("src/bits", "src/bits")); // no `/**` tail: not a directory glob
}

#[test]
fn pattern_matches_something_stats_the_concrete_prefix() {
    let tmp = tempfile::tempdir().expect("create tempdir");
    let root = tmp.path();
    fs::create_dir_all(root.join("src/bits")).unwrap();
    fs::create_dir_all(root.join("src/json/simd")).unwrap();
    fs::write(root.join("Cargo.toml"), "").unwrap();
    fs::write(root.join("CLAUDE.md"), "").unwrap();

    assert!(pattern_matches_something(root, "Cargo.toml"));
    assert!(!pattern_matches_something(root, "Cargo.lock"));
    assert!(pattern_matches_something(root, "src/bits/**"));
    assert!(
        !pattern_matches_something(root, "src/simd/**"),
        "a /** tail must not report a match for a directory that doesn't exist"
    );
    assert!(pattern_matches_something(root, "src/*/simd/**"));
    assert!(pattern_matches_something(root, "*.md"));
    assert!(!pattern_matches_something(root, "*.rs"));
}

#[test]
fn section_stops_at_the_next_heading_but_not_subheadings() {
    let markdown = "# Title\n\n## One\n\nbody\n\n### Sub\n\nmore\n\n## Two\n\nother\n";
    assert_eq!(section(markdown, "One"), "\nbody\n\n### Sub\n\nmore\n");
    assert_eq!(section(markdown, "Two"), "\nother\n");
}

#[test]
fn markdown_scopes_ignores_prose() {
    let body = "\n- `cli` - Command-line interface tool\n\
                \nIn addition to the scopes above, `cargo` is also accepted.\n";
    let parsed = markdown_scopes(body);
    assert_eq!(parsed.len(), 1);
    assert_eq!(
        parsed.get("cli").map(String::as_str),
        Some("Command-line interface tool")
    );
}

#[test]
fn split_top_level_commas_ignores_commas_inside_quotes() {
    let parts = split_top_level_commas(r#""a, b", "c""#);
    assert_eq!(parts, vec![r#""a, b""#, r#" "c""#]);
}
