//! #7710: an interpolated `import`/`include` path is jq's `Import path must be constant`, reported
//! at the start of the path's string (its format prefix, when there is one) and counted with the
//! errors around it, as jq's `ImportFrom` action does; it used to be `invalid escape sequence '\('`,
//! and for an `import` a second, spurious `expected 'as' in import directive`. This is upstream's
//! own `%%FAIL` case (`include "\(a)"; 0`, `tests/jq.test` at jq-1.7.1 and jq-1.8.2).
//!
//! Every row was captured from jq 1.7.1 (`/usr/bin/jq`, jq-1.7.1-apple) with `-L L -n`, the
//! library [`library`] writes; jq 1.8.2 prints the same messages in the same order (with its
//! `, column N` and caret line, its presentation of every compile error, not followed here).
//! [`FIRST_AND_COUNT`] holds the rows whose other error is a bison syntax error, whose wording is
//! succinctly's own (`docs/compliance/jq/limitations.md`): they pin jq's first line (only its
//! place, for a syntax error) and its count.

use anyhow::Result;

/// (program, stdout, stderr with `<R>` for the run's canonical root, exit status), from jq 1.7.1.
const EXACT: &[(&str, &str, &str, i32)] = &[
    ("include \"\\(1)\"; 0", "", "jq: error: Import path must be constant at <top-level>, line 1:\ninclude \"\\(1)\"; 0        \njq: 1 compile error\n", 3),
    ("import \"\\(\"x\")\" as $d; 0", "", "jq: error: Import path must be constant at <top-level>, line 1:\nimport \"\\(\"x\")\" as $d; 0       \njq: 1 compile error\n", 3),
    ("import \"\\(1)\" as m; 0", "", "jq: error: Import path must be constant at <top-level>, line 1:\nimport \"\\(1)\" as m; 0       \njq: 1 compile error\n", 3),
    ("include \"a\\(1)b\"; 0", "", "jq: error: Import path must be constant at <top-level>, line 1:\ninclude \"a\\(1)b\"; 0        \njq: 1 compile error\n", 3),
    ("include \"\\q\\(1)\"; 0", "", "jq: error: Invalid escape at line 1, column 4 (while parsing '\"\\q\"') at <top-level>, line 1:\ninclude \"\\q\\(1)\"; 0         \njq: error: Import path must be constant at <top-level>, line 1:\ninclude \"\\q\\(1)\"; 0        \njq: 2 compile errors\n", 3),
    ("include \"\\(1)\\q\"; 0", "", "jq: error: Invalid escape at line 1, column 4 (while parsing '\"\\q\"') at <top-level>, line 1:\ninclude \"\\(1)\\q\"; 0             \njq: error: Import path must be constant at <top-level>, line 1:\ninclude \"\\(1)\\q\"; 0        \njq: 2 compile errors\n", 3),
    ("include \"\\(1)\" {search:\".\"}; 0", "", "jq: error: Import path must be constant at <top-level>, line 1:\ninclude \"\\(1)\" {search:\".\"}; 0        \njq: 1 compile error\n", 3),
    ("include @base64 \"\\(1)\"; 0", "", "jq: error: Import path must be constant at <top-level>, line 1:\ninclude @base64 \"\\(1)\"; 0        \njq: 1 compile error\n", 3),
    ("include \"\\(1)\"; include \"\\(2)\"; 0", "", "jq: error: Import path must be constant at <top-level>, line 1:\ninclude \"\\(1)\"; include \"\\(2)\"; 0        \njq: error: Import path must be constant at <top-level>, line 1:\ninclude \"\\(1)\"; include \"\\(2)\"; 0                        \njq: 2 compile errors\n", 3),
    ("include \"\\(1)\"; import \"\\(2)\" as $x; 0", "", "jq: error: Import path must be constant at <top-level>, line 1:\ninclude \"\\(1)\"; import \"\\(2)\" as $x; 0        \njq: error: Import path must be constant at <top-level>, line 1:\ninclude \"\\(1)\"; import \"\\(2)\" as $x; 0                       \njq: 2 compile errors\n", 3),
    ("include \"\\(1)\"; {(0):1}", "", "jq: error: Import path must be constant at <top-level>, line 1:\ninclude \"\\(1)\"; {(0):1}        \njq: error: Cannot use number (0) as object key at <top-level>, line 1:\ninclude \"\\(1)\"; {(0):1}                 \njq: 2 compile errors\n", 3),
    ("include \"\\(.)\"; 0", "", "jq: error: Import path must be constant at <top-level>, line 1:\ninclude \"\\(.)\"; 0        \njq: 1 compile error\n", 3),
    ("include \"\\(1)\"; nosuchfn", "", "jq: error: Import path must be constant at <top-level>, line 1:\ninclude \"\\(1)\"; nosuchfn        \njq: 1 compile error\n", 3),
    ("import \"\\(1)\" as m; m::f", "", "jq: error: Import path must be constant at <top-level>, line 1:\nimport \"\\(1)\" as m; m::f       \njq: 1 compile error\n", 3),
    ("import \"\\(1)\" as m; import \"ip\" as i; 0", "", "jq: error: Import path must be constant at <top-level>, line 1:\nimport \"\\(1)\" as m; import \"ip\" as i; 0       \njq: 1 compile error\n", 3),
    ("import \"ip\" as i; 0", "", "jq: error: Import path must be constant at <R>/L/ip.jq, line 1:\ninclude \"\\(1)\"; def f: 1;        \njq: 1 compile error\n", 3),
    ("include \"m\"; include \"\\(1)\"; f", "", "jq: error: Import path must be constant at <top-level>, line 1:\ninclude \"m\"; include \"\\(1)\"; f                     \njq: 1 compile error\n", 3),
    ("include \"\\(\"m\")\"; f", "", "jq: error: Import path must be constant at <top-level>, line 1:\ninclude \"\\(\"m\")\"; f        \njq: 1 compile error\n", 3),
    ("module {a:1}; include \"\\(1)\"; 0", "", "jq: error: Import path must be constant at <top-level>, line 1:\nmodule {a:1}; include \"\\(1)\"; 0                      \njq: 1 compile error\n", 3),
    ("include \"\\(1)\" ; \"\\q\"", "", "jq: error: Import path must be constant at <top-level>, line 1:\ninclude \"\\(1)\" ; \"\\q\"        \njq: error: Invalid escape at line 1, column 4 (while parsing '\"\\q\"') at <top-level>, line 1:\ninclude \"\\(1)\" ; \"\\q\"                  \njq: 2 compile errors\n", 3),
];

/// (program, jq's first stderr line, its last (the count), exit status), from jq 1.7.1.
const FIRST_AND_COUNT: &[(&str, &str, &str, i32)] = &[
    ("include \"\\(1)\" {search:.}; 0", "jq: error: Import path must be constant at <top-level>, line 1:", "jq: 2 compile errors", 3),
    ("include \"\\(1)\"; 1 +", "jq: error: Import path must be constant at <top-level>, line 1:", "jq: 2 compile errors", 3),
    ("include \"\\(1;\"; 0", "jq: error: syntax error, unexpected ';' (Unix shell quoting issues?) at <top-level>, line 1:", "jq: 1 compile error", 3),
    ("include \"\\(1)\" 0", "jq: error: Import path must be constant at <top-level>, line 1:", "jq: 2 compile errors", 3),
    ("import \"\\(1)\" as $d", "jq: error: Import path must be constant at <top-level>, line 1:", "jq: 2 compile errors", 3),
    ("include \"\\(1)\"", "jq: error: Import path must be constant at <top-level>, line 1:", "jq: 2 compile errors", 3),
    ("import \"\\(1)\" as $__loc__; 0", "jq: error: Import path must be constant at <top-level>, line 1:", "jq: 2 compile errors", 3),
];

/// The module library under `root/L`: `ip.jq` includes an interpolated path, `ip2.jq` two, and
/// `m.jq` defines `f`; `prog.jq` is a program file whose interpolated `include` is on line 3.
fn library(root: &std::path::Path) -> Result<()> {
    let lib = root.join("L");
    std::fs::create_dir_all(&lib)?;
    std::fs::write(lib.join("ip.jq"), "include \"\\(1)\"; def f: 1;\n")?;
    std::fs::write(
        lib.join("ip2.jq"),
        "include \"\\(1)\"; include \"\\(2)\"; def f: 1;\n",
    )?;
    std::fs::write(lib.join("m.jq"), "def f: 1;\n")?;
    std::fs::write(root.join("prog.jq"), "\n\ninclude \"x\\(1)\";\n.\n")?;
    Ok(())
}

fn run(root: &std::path::Path, args: &[&str]) -> Result<(String, String, Option<i32>)> {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .arg("jq")
        .args(["-L", "L", "-n"])
        .args(args)
        .current_dir(root)
        .env("HOME", root)
        .env_remove("JQ_LIBRARY_PATH")
        .output()?;
    Ok((
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).replace(&*root.to_string_lossy(), "<R>"),
        output.status.code(),
    ))
}

#[test]
fn an_interpolated_import_path_is_not_constant_7710() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let root = std::fs::canonicalize(dir.path())?;
    library(&root)?;
    for &(program, stdout, stderr, status) in EXACT {
        assert_eq!(
            run(&root, &[program])?,
            (stdout.to_string(), stderr.to_string(), Some(status)),
            "{program}"
        );
    }
    for &(program, first, count, status) in FIRST_AND_COUNT {
        let (stdout, stderr, code) = run(&root, &[program])?;
        let lines: Vec<&str> = stderr.lines().filter(|l| l.starts_with("jq: ")).collect();
        // A bison syntax error first (`"\(1;"`): its place, not its wording.
        let bison = first.starts_with("jq: error: syntax error, ");
        let shown = |line: &str| {
            if bison {
                line.rsplit_once(" at ")
                    .map_or("", |(_, at)| at)
                    .to_string()
            } else {
                line.to_string()
            }
        };
        assert_eq!(
            (
                stdout.as_str(),
                lines.first().map(|l| shown(l)),
                lines.last().copied(),
                code
            ),
            ("", Some(shown(first)), Some(count), Some(status)),
            "{program}"
        );
    }
    // A program file: the line is the path's own.
    assert_eq!(
        run(&root, &["-f", "prog.jq"])?,
        (
            String::new(),
            "jq: error: Import path must be constant at <top-level>, line 3:\ninclude \"x\\(1)\";        \njq: 1 compile error\n".to_string(),
            Some(3)
        )
    );
    Ok(())
}
