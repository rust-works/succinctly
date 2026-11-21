//! #7458: `$__loc__` is its own token in jq, not a `BINDING`, so it names no `label`, no
//! `break` target and no data import: each is a syntax error (exit 3). #3029 rejected it at the
//! other binding sites (`def` parameters, `as`, destructuring, `{$__loc__}`); `label`, `break`
//! and `import "m" as $name` still accepted it. Every row was captured from jq 1.7.1
//! (`/usr/bin/jq`); jq 1.8.2 agrees on each exit status and output (only its syntax-error layout
//! differs, and syntax-error wording is a recorded divergence, #2703, so a refused program pins the
//! exit status and the empty output).

use std::process::Command;

use anyhow::Result;

use crate::{run_jq_full, spawn_with_signal_retry, succinctly_bin};

/// `succinctly jq -nc -L <dir> <program>`, with `a.json` (`{"k":1}`) and `a.jq` (`def f: 1;`) in
/// `<dir>`.
fn run_with_module_dir(program: &str) -> Result<(String, i32)> {
    let temp_dir = tempfile::tempdir()?;
    std::fs::write(temp_dir.path().join("a.json"), "{\"k\":1}\n")?;
    std::fs::write(temp_dir.path().join("a.jq"), "def f: 1;\n")?;
    let (output, code) = spawn_with_signal_retry(
        || {
            let mut command = Command::new(succinctly_bin());
            command
                .args(["jq", "-nc", "-L"])
                .arg(temp_dir.path())
                .arg(program);
            command
        },
        None,
    )?;
    Ok((String::from_utf8(output.stdout)?, code))
}

#[test]
fn test_loc_is_not_a_label_or_break_name_7458() -> Result<()> {
    for program in [
        "label $__loc__|1",
        "label $__loc__ | 1, break $__loc__, 2",
        "break $__loc__",
        "label $x | break $__loc__",
        "def f: break $__loc__; 1",
    ] {
        let (out, _err, code) = run_jq_full(&["-nc", program], None)?;
        assert_eq!((out.as_str(), code), ("", 3), "{program}");
    }
    // Names that only look like it, and `$__loc__` as a value, are ordinary.
    for (program, expected) in [
        ("label $__loc | 1", "1\n"),
        ("label $_loc__ | 1", "1\n"),
        ("label $__LOC__ | 1", "1\n"),
        ("label $__loc__x|1", "1\n"),
        (
            "label $x|$__loc__",
            "{\"file\":\"<top-level>\",\"line\":1}\n",
        ),
        ("[label $f | 1, break $f, 2]", "[1]\n"),
    ] {
        let (out, err, code) = run_jq_full(&["-nc", program], None)?;
        assert_eq!((out.as_str(), code), (expected, 0), "{program}: {err}");
    }
    Ok(())
}

#[test]
fn test_loc_is_not_a_data_import_name_7458() -> Result<()> {
    for program in [
        "import \"a\" as $__loc__; 1",
        "import \"a\" as $__loc__ {search:\"./\"}; 1",
    ] {
        let (out, code) = run_with_module_dir(program)?;
        assert_eq!((out.as_str(), code), ("", 3), "{program}");
    }
    for (program, expected) in [
        ("import \"a\" as $d; $d", "[{\"k\":1}]\n"),
        ("import \"a\" as $a; $a::a", "[{\"k\":1}]\n"),
        ("import \"a\" as $__loc__x; 1", "1\n"),
        ("import \"a\" as $_loc__; 1", "1\n"),
        ("import \"a\" as __loc__; 1", "1\n"),
        ("import \"a\" as a; a::f", "1\n"),
        ("include \"a\"; f", "1\n"),
    ] {
        let (out, code) = run_with_module_dir(program)?;
        assert_eq!((out.as_str(), code), (expected, 0), "{program}");
    }
    Ok(())
}
