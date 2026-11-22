//! #7663: a module or data import whose name resolves to a directory, or to a file that will not
//! open, is that module, as in jq's `find_lib` (`stat` finds something there): reading it fails with
//! `error loading data file <path>: Could not open <path>: It's a directory` (or `Permission
//! denied`), where a directory used to read as `module not found` and an unreadable file lacked
//! jq's `Could not open <path>:`. Only a spelling `stat` calls absent moves on to the next spelling;
//! a symlink loop moves on to the next search directory. `modulemeta` on a module that will not read
//! is `null` with nothing on stderr.
//!
//! Every row was captured from jq 1.7.1 (`/usr/bin/jq`) by `.ai/scratch/7663-capture.py` (not
//! committed; it builds the same layouts), with `-nc -L ../a -L ../b` run in `w/`; jq 1.8.2 agrees on
//! every row, and so does jq 1.7.1 on x86_64 Linux.

use anyhow::Result;

/// The layouts, as (path under the run's root, what is there): `dir` is a directory, `->t` a
/// symlink to `t`, `!text` a file holding `text` that the user cannot read, anything else a file
/// holding that text.
const LAYOUTS: &[(&str, &[(&str, &str)])] = &[
    ("dir", &[("a/m.json", "dir"), ("a/m.jq", "dir")]),
    (
        "dir_then_file",
        &[
            ("a/m.json", "dir"),
            ("a/m.jq", "dir"),
            ("b/m.json", "[1]"),
            ("b/m.jq", "def f: 1;"),
        ],
    ),
    (
        "dir_at_main",
        &[
            ("a/m/jq/main.json", "dir"),
            ("a/m/jq/main.jq", "dir"),
            ("a/m/m.json", "[2]"),
            ("a/m/m.jq", "def f: 2;"),
        ],
    ),
    (
        "dir_at_base",
        &[
            ("a/m/m.json", "dir"),
            ("a/m/m.jq", "dir"),
            ("b/m.json", "[3]"),
            ("b/m.jq", "def f: 3;"),
        ],
    ),
    (
        "loop",
        &[
            ("a/m.json", "->m.json"),
            ("a/m.jq", "->m.jq"),
            ("a/m/jq/main.json", "[4]"),
            ("a/m/jq/main.jq", "def f: 4;"),
            ("b/m.json", "[5]"),
            ("b/m.jq", "def f: 5;"),
        ],
    ),
    (
        "dangling",
        &[
            ("a/m.json", "->nowhere"),
            ("a/m.jq", "->nowhere"),
            ("a/m/jq/main.json", "[6]"),
            ("a/m/jq/main.jq", "def f: 6;"),
        ],
    ),
    (
        "unreadable",
        &[
            ("a/m.json", "!def f: 7;"),
            ("a/m.jq", "!def f: 7;"),
            ("b/m.json", "[8]"),
            ("b/m.jq", "def f: 8;"),
        ],
    ),
];

const PROGRAMS: &[&str] = &[
    r#"import "m" as $d; $d"#,
    r#"import "m" as lib; lib::f"#,
    r#"include "m"; f"#,
    r#"import "m" as $d; include "m"; [$d, f]"#,
    r#""m" | modulemeta | .defs"#,
    r#""m" | try modulemeta catch "caught""#,
];

/// (layout, program, stdout, stderr with `<R>` for the run's canonical root, exit status), from jq.
const ROWS: &[(&str, usize, &str, &str, i32)] = &[
    ("dir", 0, "", "jq: error loading data file <R>/a/m.json: Could not open <R>/a/m.json: It's a directory\n\njq: 1 compile error\n", 3),
    ("dir", 1, "", "jq: error loading data file <R>/a/m.jq: Could not open <R>/a/m.jq: It's a directory\n\njq: 1 compile error\n", 3),
    ("dir", 2, "", "jq: error loading data file <R>/a/m.jq: Could not open <R>/a/m.jq: It's a directory\n\njq: 1 compile error\n", 3),
    ("dir", 3, "", "jq: error loading data file <R>/a/m.jq: Could not open <R>/a/m.jq: It's a directory\n\njq: error loading data file <R>/a/m.json: Could not open <R>/a/m.json: It's a directory\n\njq: 2 compile errors\n", 3),
    ("dir", 4, "null\n", "", 0),
    ("dir", 5, "null\n", "", 0),
    ("dir_then_file", 0, "", "jq: error loading data file <R>/a/m.json: Could not open <R>/a/m.json: It's a directory\n\njq: 1 compile error\n", 3),
    ("dir_then_file", 1, "", "jq: error loading data file <R>/a/m.jq: Could not open <R>/a/m.jq: It's a directory\n\njq: 1 compile error\n", 3),
    ("dir_then_file", 2, "", "jq: error loading data file <R>/a/m.jq: Could not open <R>/a/m.jq: It's a directory\n\njq: 1 compile error\n", 3),
    ("dir_then_file", 3, "", "jq: error loading data file <R>/a/m.jq: Could not open <R>/a/m.jq: It's a directory\n\njq: error loading data file <R>/a/m.json: Could not open <R>/a/m.json: It's a directory\n\njq: 2 compile errors\n", 3),
    ("dir_then_file", 4, "null\n", "", 0),
    ("dir_then_file", 5, "null\n", "", 0),
    ("dir_at_main", 0, "", "jq: error loading data file <R>/a/m/jq/main.json: Could not open <R>/a/m/jq/main.json: It's a directory\n\njq: 1 compile error\n", 3),
    ("dir_at_main", 1, "", "jq: error loading data file <R>/a/m/jq/main.jq: Could not open <R>/a/m/jq/main.jq: It's a directory\n\njq: 1 compile error\n", 3),
    ("dir_at_main", 2, "", "jq: error loading data file <R>/a/m/jq/main.jq: Could not open <R>/a/m/jq/main.jq: It's a directory\n\njq: 1 compile error\n", 3),
    ("dir_at_main", 3, "", "jq: error loading data file <R>/a/m/jq/main.jq: Could not open <R>/a/m/jq/main.jq: It's a directory\n\njq: error loading data file <R>/a/m/jq/main.json: Could not open <R>/a/m/jq/main.json: It's a directory\n\njq: 2 compile errors\n", 3),
    ("dir_at_main", 4, "null\n", "", 0),
    ("dir_at_main", 5, "null\n", "", 0),
    ("dir_at_base", 0, "", "jq: error loading data file <R>/a/m/m.json: Could not open <R>/a/m/m.json: It's a directory\n\njq: 1 compile error\n", 3),
    ("dir_at_base", 1, "", "jq: error loading data file <R>/a/m/m.jq: Could not open <R>/a/m/m.jq: It's a directory\n\njq: 1 compile error\n", 3),
    ("dir_at_base", 2, "", "jq: error loading data file <R>/a/m/m.jq: Could not open <R>/a/m/m.jq: It's a directory\n\njq: 1 compile error\n", 3),
    ("dir_at_base", 3, "", "jq: error loading data file <R>/a/m/m.jq: Could not open <R>/a/m/m.jq: It's a directory\n\njq: error loading data file <R>/a/m/m.json: Could not open <R>/a/m/m.json: It's a directory\n\njq: 2 compile errors\n", 3),
    ("dir_at_base", 4, "null\n", "", 0),
    ("dir_at_base", 5, "null\n", "", 0),
    ("loop", 0, "[[5]]\n", "", 0),
    ("loop", 1, "5\n", "", 0),
    ("loop", 2, "5\n", "", 0),
    ("loop", 3, "[[[5]],5]\n", "", 0),
    ("loop", 4, "[\"f/0\"]\n", "", 0),
    ("loop", 5, "{\"deps\":[],\"defs\":[\"f/0\"]}\n", "", 0),
    ("dangling", 0, "[[6]]\n", "", 0),
    ("dangling", 1, "6\n", "", 0),
    ("dangling", 2, "6\n", "", 0),
    ("dangling", 3, "[[[6]],6]\n", "", 0),
    ("dangling", 4, "[\"f/0\"]\n", "", 0),
    ("dangling", 5, "{\"deps\":[],\"defs\":[\"f/0\"]}\n", "", 0),
    ("unreadable", 0, "", "jq: error loading data file <R>/a/m.json: Could not open <R>/a/m.json: Permission denied\n\njq: 1 compile error\n", 3),
    ("unreadable", 1, "", "jq: error loading data file <R>/a/m.jq: Could not open <R>/a/m.jq: Permission denied\n\njq: 1 compile error\n", 3),
    ("unreadable", 2, "", "jq: error loading data file <R>/a/m.jq: Could not open <R>/a/m.jq: Permission denied\n\njq: 1 compile error\n", 3),
    ("unreadable", 3, "", "jq: error loading data file <R>/a/m.jq: Could not open <R>/a/m.jq: Permission denied\n\njq: error loading data file <R>/a/m.json: Could not open <R>/a/m.json: Permission denied\n\njq: 2 compile errors\n", 3),
    ("unreadable", 4, "null\n", "", 0),
    ("unreadable", 5, "null\n", "", 0),
];

/// Build `layout` under `root`; whether every unreadable file really is unreadable to this user
/// (root reads them anyway).
#[cfg(unix)]
fn build(root: &std::path::Path, layout: &[(&str, &str)]) -> Result<bool> {
    use std::os::unix::fs::PermissionsExt;
    let mut binds = true;
    for dir in ["a", "b", "w"] {
        std::fs::create_dir_all(root.join(dir))?;
    }
    for &(path, what) in layout {
        let full = root.join(path);
        std::fs::create_dir_all(full.parent().expect("a parent"))?;
        if what == "dir" {
            std::fs::create_dir(&full)?;
        } else if let Some(target) = what.strip_prefix("->") {
            std::os::unix::fs::symlink(target, &full)?;
        } else if let Some(text) = what.strip_prefix('!') {
            std::fs::write(&full, format!("{text}\n"))?;
            std::fs::set_permissions(&full, std::fs::Permissions::from_mode(0o000))?;
            binds &= std::fs::read(&full).is_err();
        } else {
            std::fs::write(&full, format!("{what}\n"))?;
        }
    }
    Ok(binds)
}

#[cfg(unix)]
#[test]
fn a_module_found_as_a_directory_or_unreadable_file_reports_as_jq_7663() -> Result<()> {
    let mut checked = 0;
    for &(name, layout) in LAYOUTS {
        let root = tempfile::tempdir()?;
        if !build(root.path(), layout)? {
            eprintln!("skipped {name}: chmod 000 does not bind this user");
            continue;
        }
        let canonical = std::fs::canonicalize(root.path())?;
        for &(row_layout, program, stdout, stderr, status) in ROWS {
            if row_layout != name {
                continue;
            }
            let output = std::process::Command::new(env!("CARGO_BIN_EXE_succinctly"))
                .args(["jq", "-nc", "-L", "../a", "-L", "../b", PROGRAMS[program]])
                .current_dir(root.path().join("w"))
                .env("LC_ALL", "C")
                .env("HOME", &canonical)
                .env_remove("JQ_LIBRARY_PATH")
                .output()?;
            let got_stderr = String::from_utf8_lossy(&output.stderr)
                .replace(&*canonical.to_string_lossy(), "<R>");
            assert_eq!(
                (
                    String::from_utf8_lossy(&output.stdout).as_ref(),
                    got_stderr.as_str(),
                    output.status.code()
                ),
                (stdout, stderr, Some(status)),
                "{name}: {}",
                PROGRAMS[program]
            );
            checked += 1;
        }
    }
    assert!(checked >= ROWS.len() - PROGRAMS.len(), "{checked} rows run");
    Ok(())
}
