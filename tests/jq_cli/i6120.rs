//! #6120: a module name that starts with `/` is a path relative to the search directories in jq,
//! joined with a `/` (`lib` + `/` + `/a` is `lib//a.jq`, which the file system reads as
//! `lib/a.jq`). `succinctly jq` took it as an absolute file system path, so it found a module jq
//! cannot and missed one jq finds, for `import`, `include`, a data import and `modulemeta` alike.
//! Expected values captured live against jq 1.7.1 (`/usr/bin/jq`).

use anyhow::Result;

use crate::run_jq_in_dir;

/// `lib/a.jq` (`def f: "a.f";`) and `lib/d.json` (`{"k":[1,2]}`).
fn fixture() -> Result<tempfile::TempDir> {
    let dir = tempfile::tempdir()?;
    let lib = dir.path().join("lib");
    std::fs::create_dir_all(&lib)?;
    std::fs::write(lib.join("a.jq"), "def f: \"a.f\";")?;
    std::fs::write(lib.join("d.json"), "{\"k\":[1,2]}")?;
    Ok(dir)
}

fn run(dir: &tempfile::TempDir, args: &[&str]) -> Result<(String, String, i32)> {
    run_jq_in_dir(dir.path(), args, None)
}

fn answers(dir: &tempfile::TempDir, args: &[&str], want: &str) -> Result<()> {
    let (stdout, stderr, code) = run(dir, args)?;
    assert_eq!((stdout.as_str(), code), (want, 0), "{args:?}: {stderr}");
    Ok(())
}

fn not_found(dir: &tempfile::TempDir, args: &[&str], name: &str) -> Result<()> {
    let (stdout, stderr, code) = run(dir, args)?;
    assert_eq!(
        (stdout.as_str(), stderr.as_str(), code),
        (
            "",
            format!("jq: error: module not found: {name}\n\njq: 1 compile error\n").as_str(),
            3
        ),
        "{args:?}"
    );
    Ok(())
}

#[test]
fn a_leading_slash_is_joined_onto_each_search_directory() -> Result<()> {
    let dir = fixture()?;
    answers(
        &dir,
        &["-L", "lib", "-nc", r#"import "/a" as a; a::f"#],
        "\"a.f\"\n",
    )?;
    answers(
        &dir,
        &["-L", "lib", "-nc", r#"include "/a"; f"#],
        "\"a.f\"\n",
    )?;
    answers(
        &dir,
        &["-L", "lib", "-nc", r#"import "/d" as $d; $d"#],
        "[{\"k\":[1,2]}]\n",
    )?;
    answers(
        &dir,
        &["-L", "lib", "-nc", r#""/a" | modulemeta"#],
        "{\"deps\":[],\"defs\":[\"f/0\"]}\n",
    )?;
    // A `search` entry is a directory (an absolute one as it stands), the name is joined onto it.
    let search = format!(
        r#"import "/a" as a {{search:"{}/lib"}}; a::f"#,
        dir.path().display()
    );
    answers(&dir, &["-nc", &search], "\"a.f\"\n")?;
    Ok(())
}

#[cfg(unix)]
#[test]
fn an_absolute_path_is_not_a_module_name_and_a_missing_one_is_not_found() -> Result<()> {
    let dir = fixture()?;
    let abs = dir.path().join("lib").join("a");
    let program = format!(r#"import "{}" as a; a::f"#, abs.display());
    let name = abs.display().to_string();
    not_found(&dir, &["-nc", &program], &name)?;
    // `-L` on the same directory does not make it relative: the name is joined onto it.
    let lib = dir.path().join("lib").display().to_string();
    not_found(&dir, &["-L", &lib, "-nc", &program], &name)?;
    not_found(
        &dir,
        &["-L", "lib", "-nc", r#"import "/b" as a; a::f"#],
        "/b",
    )?;
    Ok(())
}

#[cfg(unix)]
#[test]
fn the_root_directory_as_a_search_directory_still_reaches_an_absolute_name() -> Result<()> {
    let dir = fixture()?;
    let abs = dir.path().join("lib").join("a");
    let program = format!(r#"import "{}" as a; a::f"#, abs.display());
    answers(&dir, &["-L", "/", "-nc", &program], "\"a.f\"\n")?;
    Ok(())
}

/// The directory-module spellings (`b/b.jq`, `p/jq/main.jq`) are joined the same way, and a name
/// that is only a slash, or ends in one, is refused or not found as it is without the slash.
#[test]
fn the_directory_module_spellings_follow_a_leading_slash() -> Result<()> {
    let dir = fixture()?;
    for (sub, file, text) in [
        ("lib/b", "b.jq", "def f: \"b/b\";"),
        ("lib/p/jq", "main.jq", "def f: \"p/jq/main\";"),
    ] {
        std::fs::create_dir_all(dir.path().join(sub))?;
        std::fs::write(dir.path().join(sub).join(file), text)?;
    }
    answers(
        &dir,
        &["-L", "lib", "-nc", r#"import "/b" as a; a::f"#],
        "\"b/b\"\n",
    )?;
    answers(
        &dir,
        &["-L", "lib", "-nc", r#"import "/p" as a; a::f"#],
        "\"p/jq/main\"\n",
    )?;
    let (stdout, stderr, code) = run(&dir, &["-L", "lib", "-nc", r#"import "/" as a; a::f"#])?;
    assert_eq!(
        (stdout.as_str(), stderr.as_str(), code),
        (
            "",
            "jq: error: module names must not have equal consecutive components: /\n\njq: 1 compile error\n",
            3
        )
    );
    not_found(
        &dir,
        &["-L", "lib", "-nc", r#"import "a/" as a; a::f"#],
        "a/",
    )?;
    Ok(())
}
