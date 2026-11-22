//! #7450: a NUL byte inside a string that jq prints with a C `%s`.
//!
//! Two message families, each pinned to the jq version that is the reference for it.
//!
//! A module path with a NUL (`include "a\u0000b"`) is refused with `Module path contains a NUL
//! byte`, as jq 1.8.2 refuses it (CVE-2026-43895, upstream commit 9d223f153c): jq 1.7.1 looked the
//! path up as a C string cut at the NUL (`module not found: a`), which could open a different file
//! from the one the program names. Those rows were captured from jq 1.8.2.
//!
//! `Cannot index <type> with string "<key>"` writes the key with `%s`, so a key `a\u0000b` shows as
//! `a`, the closing quote and the rest of the message kept, and so does the string a `try` catches.
//! jq 1.8.2 rewords every `Cannot index` message on purpose (jqlang/jq#3478), so 1.7.1 stays the
//! reference for these, and its rows were captured from `/usr/bin/jq`.

use anyhow::Result;

use crate::run_jq_full;

const NUL_PATH_REFUSED: &str =
    "jq: error: Module path contains a NUL byte\n\njq: 1 compile error\n";

/// Programs whose module path holds a NUL: jq 1.8.2 refuses each with `Module path contains a NUL
/// byte`, ahead of its other checks on the path (the backslash, `..` and equal-components ones).
const NUL_PATHS: &[&str] = &[
    r#"include "a\u0000b"; 1"#,
    r#"include "\u0000"; 1"#,
    r#"include "a\u0000"; 1"#,
    r#"import "a\u0000b" as x; 1"#,
    r#"import "a\u0000b" as $x; $x"#,
    r#"import "a\u0000b" as x {search:"."}; 1"#,
    r#"include "a\u0000b\\c"; 1"#,
    r#"include "../a\u0000b"; 1"#,
    r#"include "a/..\u0000"; 1"#,
    r#"include "b/b\u0000"; 1"#,
    // Directives are walked last first, so the NUL one is reached before `zz` here.
    r#"include "zz"; include "a\u0000b"; 1"#,
];

#[test]
fn a_module_path_with_a_nul_is_refused_7450() -> Result<()> {
    for program in NUL_PATHS {
        let (out, err, status) = run_jq_full(&["-nc", program], None)?;
        assert_eq!(
            (out.as_str(), err.as_str(), status),
            ("", NUL_PATH_REFUSED, 3),
            "{program}"
        );
    }
    // The directive before it in the program is reached first, and fails on its own.
    let (out, err, status) = run_jq_full(&["-nc", r#"include "a\u0000b"; include "zz"; 1"#], None)?;
    assert_eq!(
        (out.as_str(), err.as_str(), status),
        (
            "",
            "jq: error: module not found: zz\n\njq: 1 compile error\n",
            3
        )
    );
    Ok(())
}

#[test]
fn modulemeta_refuses_a_module_path_with_a_nul_7450() -> Result<()> {
    for name in ["a\\u0000b", "\\u0000", "a/a\\u0000", "../a\\u0000b"] {
        let uncaught = format!(r#""{name}"|modulemeta"#);
        let (out, err, status) = run_jq_full(&["-nc", &uncaught], None)?;
        assert_eq!(
            (out.as_str(), err.as_str(), status),
            (
                "",
                "jq: error (at <unknown>): Module path contains a NUL byte\n",
                5
            ),
            "{uncaught}"
        );
        let caught = format!(r#"try ("{name}"|modulemeta) catch ."#);
        let (out, err, status) = run_jq_full(&["-nc", &caught], None)?;
        assert_eq!(
            (out.as_str(), err.as_str(), status),
            ("\"Module path contains a NUL byte\"\n", "", 0),
            "{caught}"
        );
    }
    Ok(())
}

#[test]
fn a_nul_in_a_modules_own_import_is_refused_too_7450() -> Result<()> {
    let dir = tempfile::tempdir()?;
    std::fs::write(dir.path().join("m.jq"), r#"include "a\u0000b"; def f: 1;"#)?;
    let lib = dir.path().to_str().unwrap_or_default();
    let (out, err, status) = run_jq_full(&["-nc", "-L", lib, r#"import "m" as m; m::f"#], None)?;
    assert_eq!(
        (out.as_str(), err.as_str(), status),
        ("", NUL_PATH_REFUSED, 3)
    );
    Ok(())
}

/// `[] | .[<key>]` and the other routes to the same message, with jq 1.7.1's caught string.
#[test]
fn an_index_error_cuts_the_key_at_its_nul_7450() -> Result<()> {
    let key = r#""a\u0000b""#;
    for (program, want) in [
        (
            format!("[] | try .[{key}] catch ."),
            "Cannot index array with string \\\"a\\\"",
        ),
        (
            format!("1 | try .[{key}] catch ."),
            "Cannot index number with string \\\"a\\\"",
        ),
        (
            format!(r#""abc" | try .[{key}] catch ."#),
            "Cannot index string with string \\\"a\\\"",
        ),
        (
            format!("true | try .[{key}] catch ."),
            "Cannot index boolean with string \\\"a\\\"",
        ),
        (
            format!("[] | try getpath([{key}]) catch ."),
            "Cannot index array with string \\\"a\\\"",
        ),
        (
            format!("1 | try setpath([{key}]; 1) catch ."),
            "Cannot index number with string \\\"a\\\"",
        ),
        (
            format!("[] | try (.[{key}] = 1) catch ."),
            "Cannot index array with string \\\"a\\\"",
        ),
        (
            format!("[] | try (.[{key}] |= 1) catch ."),
            "Cannot index array with string \\\"a\\\"",
        ),
        (
            format!("[] | try del(.[{key}]) catch ."),
            "Cannot index array with string \\\"a\\\"",
        ),
        (
            r#"[] | try .["\u0000"] catch ."#.to_string(),
            "Cannot index array with string \\\"\\\"",
        ),
        (
            r#"[] | try .["\u0000x"] catch ."#.to_string(),
            "Cannot index array with string \\\"\\\"",
        ),
        (
            r#"[] | try .["é\u0000"] catch ."#.to_string(),
            "Cannot index array with string \\\"é\\\"",
        ),
        // The length that leaves the key out is the whole key's, NUL and what follows it included.
        (
            format!(r#"[] | try .["a\u0000{}"] catch ."#, "b".repeat(27)),
            "Cannot index array with string \\\"a\\\"",
        ),
        (
            format!(r#"[] | try .["a\u0000{}"] catch ."#, "b".repeat(28)),
            "Cannot index array with string",
        ),
    ] {
        let (out, err, status) = run_jq_full(&["-nc", &program], None)?;
        assert_eq!(
            (out.as_str(), err.as_str(), status),
            (format!("\"{want}\"\n").as_str(), "", 0),
            "{program}"
        );
    }
    Ok(())
}

#[test]
fn an_uncaught_index_error_is_cut_at_the_nul_7450() -> Result<()> {
    let (out, err, status) = run_jq_full(&["-nc", r#"[] | .["a\u0000b"]"#], None)?;
    assert_eq!(
        (out.as_str(), err.as_str(), status),
        (
            "",
            "jq: error (at <unknown>): Cannot index array with string \"a\"\n",
            5
        )
    );
    Ok(())
}
