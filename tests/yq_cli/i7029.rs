//! #7029: `--split-exp` names a file after the first value its expression produces, and makes the
//! directories the name needs, as yq v4.53.3 does. yq reads the node's scalar *value*: a string as it
//! is, a number, boolean or `null` as it prints (`1.yml`, `true.yml`, `null.yml`, `0x10.yml`), and nothing
//! for a mapping, a sequence or no value at all, so those write `.yml` rather than failing. Later values
//! are ignored. Every row was captured from yq (`-s EXPR . f.yaml`, run in an empty directory).
//!
//! Not covered because it differs for a reason of its own: a source `~` is `null` here (#2802).

use std::process::{Command, Stdio};

use anyhow::Result;
use tempfile::TempDir;

/// The document every row reads.
const DOC: &str = "a: 1\nb: x\nc: true\nd: 1.5\ne: null\nf: [1]\ng: {k: 1}\nh: 0x10\nj: 007\nk: !!str 1\nl: \"\"\n";

/// `(split expression, files written)`.
const ROWS: &[(&str, &[&str])] = &[
    (".a", &["1.yml"]),
    (".b", &["x.yml"]),
    (".c", &["true.yml"]),
    (".d", &["1.5"]),
    (".e", &["null.yml"]),
    (".f", &[".yml"]),
    (".g", &[".yml"]),
    (".h", &["0x10.yml"]),
    (".j", &["007.yml"]),
    (".k", &["1.yml"]),
    (".l", &[".yml"]),
    (".zz", &["null.yml"]),
    (".a, .b", &["1.yml"]),
    (".b, .b", &["x.yml"]),
    ("select(false)", &[".yml"]),
    (".a + 1", &["2.yml"]),
    ("\"x\", \"y\"", &["x.yml"]),
    ("[.a]", &[".yml"]),
    (".f[]", &["1.yml"]),
    ("\"\" + .a", &["1.yml"]),
    (".a|tostring", &["1.yml"]),
    (".d|tostring", &["1.5"]),
    (".", &[".yml"]),
    ("$index", &["0.yml"]),
    ("$index + 1", &["1.yml"]),
    (".g.k", &["1.yml"]),
    (".f[0]", &["1.yml"]),
    (".d + 1", &["2.5"]),
    ("null", &["null.yml"]),
    ("true", &["true.yml"]),
    ("\"a\" + \"b\"", &["ab.yml"]),
    ("\"a.b\"", &["a.b"]),
    ("\"d/x\"", &["d/x.yml"]),
    ("\"d/e/x\"", &["d/e/x.yml"]),
    ("\"d/\"", &["d/.yml"]),
    ("\"./x\"", &["x.yml"]),
    ("\"d//x\"", &["d/x.yml"]),
    ("\"d/x/\"", &["d/x/.yml"]),
    ("\"d/.h\"", &["d/.h"]),
    ("\"dir.d/f\"", &["dir.d/f.yml"]),
    ("\"a/b/c/d/e/f\"", &["a/b/c/d/e/f.yml"]),
    ("\"d/x.txt\"", &["d/x.txt"]),
    ("\"d/e/\"", &["d/e/.yml"]),
];

/// Run `succinctly yq --split-exp EXPR . f.yaml` in a fresh directory holding `f.yaml`; return the
/// directory (kept alive by the caller), the exit code and stderr.
fn run_split(expr: &str, doc: &str, extra: &[&str]) -> Result<(TempDir, i32, String)> {
    let dir = TempDir::new()?;
    std::fs::write(dir.path().join("f.yaml"), doc)?;
    let output = Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .current_dir(dir.path())
        .arg("yq")
        .args(extra)
        .args(["--split-exp", expr, ".", "f.yaml"])
        .stdin(Stdio::null())
        .output()?;
    let code = output.status.code().unwrap_or(-1);
    Ok((dir, code, String::from_utf8(output.stderr)?))
}

/// Every file under `dir` except `f.yaml`, as `/`-separated paths relative to it, sorted.
fn written(dir: &std::path::Path) -> Result<Vec<String>> {
    fn walk(root: &std::path::Path, at: &std::path::Path, out: &mut Vec<String>) -> Result<()> {
        for entry in std::fs::read_dir(at)? {
            let path = entry?.path();
            if path.is_dir() {
                walk(root, &path, out)?;
            } else {
                let rel = path
                    .strip_prefix(root)?
                    .to_string_lossy()
                    .replace('\\', "/");
                if rel != "f.yaml" {
                    out.push(rel);
                }
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out)?;
    out.sort();
    Ok(out)
}

#[test]
fn split_exp_names_the_file_after_the_first_values_scalar_text_7029() -> Result<()> {
    for &(expr, want) in ROWS {
        let (dir, code, stderr) = run_split(expr, DOC, &[])?;
        assert_eq!(code, 0, "{expr}: {stderr}");
        let names = written(dir.path())?;
        let names: Vec<&str> = names.iter().map(String::as_str).collect();
        assert_eq!(names, want, "{expr}");
    }
    Ok(())
}

/// The file holds the result, not the name: `.a` over `a: 1` names the file `1.yml` and writes the
/// document.
#[test]
fn split_exp_writes_the_result_to_the_file_it_names_7029() -> Result<()> {
    let (dir, code, stderr) = run_split(".a", "a: 1\n", &[])?;
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(std::fs::read_to_string(dir.path().join("1.yml"))?, "a: 1\n");
    // A mapping name is empty: the result goes to `.yml`.
    let (dir, code, stderr) = run_split(".", "a: 1\n", &[])?;
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(std::fs::read_to_string(dir.path().join(".yml"))?, "a: 1\n");
    Ok(())
}

/// A name with no value at all (here the expression has no output under `--jq-extensions`, which
/// yq's lexer refuses) is an empty name too, and so is `select(false)` above.
#[test]
fn split_exp_an_expression_with_no_output_names_the_extension_alone_7029() -> Result<()> {
    let (dir, code, stderr) = run_split("empty", "a: 1\n", &["--jq-extensions"])?;
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(written(dir.path())?, [".yml"]);
    Ok(())
}

/// A name with two values uses the first, and a later error is not raised when the first already
/// named the file; `$index` is a number.
#[test]
fn split_exp_uses_the_first_value_only_7029() -> Result<()> {
    let (dir, code, stderr) = run_split("$index, 1", "a: 1\n", &[])?;
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(written(dir.path())?, ["0.yml"]);
    Ok(())
}

/// `d/../x` has the directory `.` once cleaned (Go's `filepath.Dir`), so no directory is made and the
/// write fails on the missing `d`, as yq's does.
#[test]
fn split_exp_cleans_the_directory_lexically_before_creating_it_7029() -> Result<()> {
    let (dir, code, _) = run_split("\"d/../x\"", "a: 1\n", &[])?;
    assert_eq!(code, 1);
    assert_eq!(written(dir.path())?, Vec::<String>::new());
    assert!(!dir.path().join("d").exists());
    Ok(())
}

/// A path component that exists and is not a directory is an error and nothing is written.
#[test]
fn split_exp_a_file_where_a_directory_is_needed_is_an_error_7029() -> Result<()> {
    let (dir, code, stderr) = run_split("\"f.yaml/x\"", "a: 1\n", &[])?;
    assert_eq!(code, 1, "{stderr}");
    assert!(
        stderr.contains("failed to create --split-exp directory"),
        "{stderr}"
    );
    assert_eq!(written(dir.path())?, Vec::<String>::new());
    Ok(())
}

/// yq creates the directories with mode `0750`; the umask can only remove bits from that.
#[cfg(unix)]
#[test]
fn split_exp_creates_directories_with_mode_0750_7029() -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let (dir, code, stderr) = run_split("\"m/n/x\"", "a: 1\n", &[])?;
    assert_eq!(code, 0, "{stderr}");
    for sub in ["m", "m/n"] {
        let mode = std::fs::metadata(dir.path().join(sub))?
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode & !0o750, 0, "{sub}: {mode:o} grants more than 0750");
        assert_eq!(mode & 0o700, 0o700, "{sub}: {mode:o}");
    }
    Ok(())
}

/// An absolute name makes its directories wherever they are, and a leading `..` is kept by the lexical
/// clean (`../out/x` from `inner` is `out/x` beside it): the directory `Dir` names is the one made.
#[test]
fn split_exp_makes_the_directories_of_an_absolute_and_a_parent_relative_name_7029() -> Result<()> {
    let root = TempDir::new()?;
    let inner = root.path().join("inner");
    std::fs::create_dir(&inner)?;
    std::fs::write(inner.join("f.yaml"), "a: 1\n")?;
    let absolute = format!("\"{}/abs/deep/x\"", root.path().display());
    for expr in [absolute.as_str(), "\"../out/x\""] {
        let output = Command::new(env!("CARGO_BIN_EXE_succinctly"))
            .current_dir(&inner)
            .args(["yq", "--split-exp", expr, ".", "f.yaml"])
            .stdin(Stdio::null())
            .output()?;
        assert_eq!(
            output.status.code(),
            Some(0),
            "{expr}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    assert_eq!(
        std::fs::read_to_string(root.path().join("abs/deep/x.yml"))?,
        "a: 1\n"
    );
    assert_eq!(
        std::fs::read_to_string(root.path().join("out/x.yml"))?,
        "a: 1\n"
    );
    assert!(!inner.join("out").exists());
    Ok(())
}
