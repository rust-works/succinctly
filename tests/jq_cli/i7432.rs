//! #7432: `modulemeta` lists each name/arity once, at the position of its first `def`.
//!
//! jq lists the module block's bindings, one per name and arity, so a module that defines the
//! same function twice (`def f: 1; def f: 2;`) has one `f/0` in `defs`, where `succinctly jq`
//! listed one per `def`. A different arity is a different entry. Every expected value is the
//! answer of `/usr/bin/jq` 1.7.1 (`jq -nc -L lib '"m" | modulemeta'`).

use anyhow::Result;

use crate::run_jq_in_dir;

/// `(module source, jq's defs)`.
const MODULES: &[(&str, &str)] = &[
    ("def f: 1; def g: 1; def f: 3;", r#"["f/0","g/0"]"#),
    ("def f: 1; def f: 2;", r#"["f/0"]"#),
    ("def f: 1; def f: 2; def f: 3;", r#"["f/0"]"#),
    (
        "def f: 1; def f(a): 2; def f: 3; def f(a): 4; def f(a;b): 5;",
        r#"["f/0","f/1","f/2"]"#,
    ),
    ("def f(a;b): 1; def f(c;d): 2;", r#"["f/2"]"#),
    ("def f(a): 1; def f($a): 2;", r#"["f/1"]"#),
    ("def f(g): 1; def f(g): 2;", r#"["f/1"]"#),
    (
        "def b: 1; def a: 2; def b: 3; def c: 4; def a: 5;",
        r#"["b/0","a/0","c/0"]"#,
    ),
    ("def f: def g: 1; g; def g: 2;", r#"["f/0","g/0"]"#),
    ("def length: 1; def length: 2;", r#"["length/0"]"#),
    // Controls: no repeat, or a different arity, lists every def as before.
    ("def f: 1; def g: 2;", r#"["f/0","g/0"]"#),
    ("def f: 1; def f(a): 2;", r#"["f/0","f/1"]"#),
];

#[test]
fn test_modulemeta_lists_each_name_and_arity_once_7432() -> Result<()> {
    let dir = tempfile::tempdir()?;
    std::fs::create_dir_all(dir.path().join("lib"))?;
    for &(source, defs) in MODULES {
        std::fs::write(dir.path().join("lib/m.jq"), source)?;
        let (stdout, stderr, code) = run_jq_in_dir(
            dir.path(),
            &["-nc", "-L", "lib", r#""m" | modulemeta"#],
            None,
        )?;
        assert_eq!(
            (stdout.as_str(), stderr.as_str(), code),
            (format!("{{\"deps\":[],\"defs\":{defs}}}\n").as_str(), "", 0),
            "{source}"
        );
    }
    Ok(())
}

/// The header metadata and the directives are untouched by the repeat.
#[test]
fn test_modulemeta_repeated_def_keeps_the_header_and_deps_7432() -> Result<()> {
    let dir = tempfile::tempdir()?;
    std::fs::create_dir_all(dir.path().join("lib"))?;
    for (source, want) in [
        (
            "module {a:1}; import \"x\" as x; def f: 1; def f: 2;",
            r#"{"a":1,"deps":[{"as":"x","is_data":false,"relpath":"x"}],"defs":["f/0"]}"#,
        ),
        (
            "include \"x\"; def f: 1; def f: 2;",
            r#"{"deps":[{"is_data":false,"relpath":"x"}],"defs":["f/0"]}"#,
        ),
    ] {
        std::fs::write(dir.path().join("lib/m.jq"), source)?;
        let (stdout, stderr, code) = run_jq_in_dir(
            dir.path(),
            &["-nc", "-L", "lib", r#""m" | modulemeta"#],
            None,
        )?;
        assert_eq!(
            (stdout.as_str(), stderr.as_str(), code),
            (format!("{want}\n").as_str(), "", 0),
            "{source}"
        );
    }
    Ok(())
}
