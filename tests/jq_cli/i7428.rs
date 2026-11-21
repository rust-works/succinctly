//! #7428, #7430: jq's module search list is one list. `-L` replaces the default list
//! (`~/.jq`, `$ORIGIN/../lib/jq`, `$ORIGIN/../lib`) instead of being searched ahead of it, and
//! the entries of whichever list is in force are expanded: `~/` to `$HOME`, `$ORIGIN/` to the
//! directory jq was run from. `~` and `$ORIGIN` with no slash are literal. Every expected value
//! was captured from jq 1.7.1 (`/usr/bin/jq`) and from jq 1.8.2, which agree on all of them; the
//! `$ORIGIN` rows ran against a copy of each binary beside a `lib/` directory.

use anyhow::Result;

use std::process::Command;

use crate::{spawn_with_signal_retry, succinctly_bin};

/// A scratch tree: `home/.jq/m.jq` (`hm`), `home/x/m.jq` (`hx`), `home/m.jq` (`hroot`),
/// `cwd/a/m.jq` (`a`), `cwd/c.jq` (`cwd`), and a `jq` symlink at `root/bin/jq` with
/// `root/lib/m.jq` (`lib`), `root/lib/jq/m.jq` (`libjq`), `root/lib/d.json` and `root/lib/jq/n.jq`.
struct Tree {
    dir: tempfile::TempDir,
}

impl Tree {
    fn new(dot_jq_dir: bool) -> Result<Self> {
        let dir = tempfile::tempdir()?;
        let p = dir.path();
        for sub in ["home/x", "cwd/a", "root/bin", "root/lib/jq"] {
            std::fs::create_dir_all(p.join(sub))?;
        }
        if dot_jq_dir {
            std::fs::create_dir_all(p.join("home/.jq"))?;
            std::fs::write(p.join("home/.jq/m.jq"), "def f: \"hm\";")?;
            std::fs::write(p.join("home/.jq/d.json"), "\"data-hm\"")?;
        }
        std::fs::write(p.join("home/x/m.jq"), "def f: \"hx\";")?;
        std::fs::write(p.join("home/m.jq"), "def f: \"hroot\";")?;
        std::fs::write(p.join("cwd/a/m.jq"), "def f: \"a\";")?;
        std::fs::write(p.join("cwd/c.jq"), "def f: \"cwd\";")?;
        std::fs::write(p.join("root/lib/m.jq"), "def f: \"lib\";")?;
        std::fs::write(p.join("root/lib/jq/m.jq"), "def f: \"libjq\";")?;
        std::fs::write(p.join("root/lib/jq/n.jq"), "def f: \"nlibjq\";")?;
        std::fs::write(p.join("root/lib/d.json"), "\"data-lib\"")?;
        #[cfg(unix)]
        std::os::unix::fs::symlink(succinctly_bin(), p.join("root/bin/jq"))?;
        Ok(Self { dir })
    }

    /// Run `jq -nc <l...> <program>` as `root/bin/jq` (so `$ORIGIN` is `root/bin`), in `cwd/`,
    /// with `HOME` at `home/`.
    fn run(&self, l: &[&str], program: &str) -> Result<(String, String, i32)> {
        let p = self.dir.path();
        let (output, code) = spawn_with_signal_retry(
            || {
                let mut command = Command::new(p.join("root/bin/jq"));
                command.arg("-nc");
                for entry in l {
                    command.args(["-L", entry]);
                }
                command
                    .arg(program)
                    .current_dir(p.join("cwd"))
                    .env("HOME", p.join("home"));
                command
            },
            None,
        )?;
        Ok((
            String::from_utf8(output.stdout)?,
            String::from_utf8(output.stderr)?,
            code,
        ))
    }
}

const IMPORT: &str = "import \"m\" as m; m::f";
const NOT_FOUND_M: &str = "jq: error: module not found: m\n\njq: 1 compile error\n";

/// One row: the `-L` list, the program, and jq's stdout, or `None` for "module not found".
type Row<'a> = (&'a [&'a str], &'a str, Option<&'a str>);

fn check(tree: &Tree, rows: &[Row<'_>]) -> Result<()> {
    for (l, program, want) in rows {
        let (stdout, stderr, code) = tree.run(l, program)?;
        match want {
            Some(want) => assert_eq!(
                (stdout.as_str(), code),
                (*want, 0),
                "-L {l:?} {program}: {stderr}"
            ),
            None => {
                // A compile error is 3; `modulemeta` raises an error of the filter, 5.
                let status = if program.contains("modulemeta") { 5 } else { 3 };
                assert_eq!(code, status, "-L {l:?} {program}: {stdout}{stderr}");
                assert!(stdout.is_empty(), "-L {l:?} {program}: {stdout}");
                assert!(
                    stderr.contains("module not found: "),
                    "-L {l:?} {program}: {stderr}"
                );
            }
        }
    }
    Ok(())
}

#[cfg(unix)]
#[test]
fn test_l_replaces_the_default_search_list_7428() -> Result<()> {
    let tree = Tree::new(true)?;
    check(
        &tree,
        &[
            // No `-L`: `~/.jq` first, then `$ORIGIN/../lib/jq`, then `$ORIGIN/../lib`.
            (&[], IMPORT, Some("\"hm\"\n")),
            (&[], "import \"n\" as m; m::f", Some("\"nlibjq\"\n")),
            (&[], "import \"d\" as $d; $d", Some("[\"data-hm\"]\n")),
            // Any `-L` turns every default directory off: the module is only in `~/.jq`
            // and in `$ORIGIN/../lib`.
            (&["a"], IMPORT, Some("\"a\"\n")),
            (&["missing"], IMPORT, None),
            (&["missing"], "include \"m\"; f", None),
            (&["missing"], "import \"d\" as $d; $d", None),
            (&["missing"], "\"m\"|modulemeta|.defs", None),
            (&["a"], "\"n\"|modulemeta|.defs", None),
            // The list reported is the list searched.
            (&["missing"], "get_search_list", Some("[\"missing\"]\n")),
        ],
    )
}

#[cfg(unix)]
#[test]
fn test_l_entries_expand_tilde_and_origin_7430() -> Result<()> {
    let tree = Tree::new(true)?;
    check(
        &tree,
        &[
            (&["~/x"], IMPORT, Some("\"hx\"\n")),
            (&["~/x"], "\"m\"|modulemeta|.defs", Some("[\"f/0\"]\n")),
            (&["~/"], IMPORT, Some("\"hroot\"\n")),
            (&["~/.jq"], IMPORT, Some("\"hm\"\n")),
            (&["~/nope"], IMPORT, None),
            // `~` and `$ORIGIN` without a slash are literal.
            (&["~"], IMPORT, None),
            (&["$ORIGIN"], IMPORT, None),
            (&["$ORIGIN/"], IMPORT, None),
            (&["$ORIGIN/../lib"], IMPORT, Some("\"lib\"\n")),
            (&["$ORIGIN/../lib/jq"], IMPORT, Some("\"libjq\"\n")),
            (
                &["$ORIGIN/../lib"],
                "import \"d\" as $d; $d",
                Some("[\"data-lib\"]\n"),
            ),
            (
                &["$ORIGIN/../lib"],
                "\"m\"|modulemeta|.defs",
                Some("[\"f/0\"]\n"),
            ),
            (&["$ORIGIN/../nope"], IMPORT, None),
            // In order, the first directory holding the module wins; an unexpandable-to-a-module
            // entry before it is skipped.
            (&["~/x", "a"], IMPORT, Some("\"hx\"\n")),
            (&["a", "~/x"], IMPORT, Some("\"a\"\n")),
            // The list reported is the entries as given.
            (&["~/x"], "get_search_list", Some("[\"~/x\"]\n")),
        ],
    )
}

#[cfg(unix)]
#[test]
fn test_default_origin_entries_are_searched_7430() -> Result<()> {
    // `~/.jq` is a file of definitions here, not a directory: the module is found through
    // `$ORIGIN/../lib/jq`, then `$ORIGIN/../lib`.
    let tree = Tree::new(false)?;
    check(
        &tree,
        &[
            (&[], IMPORT, Some("\"libjq\"\n")),
            (&[], "import \"d\" as $d; $d", Some("[\"data-lib\"]\n")),
            (&[], "\"m\"|modulemeta|.defs", Some("[\"f/0\"]\n")),
            (&["missing"], IMPORT, None),
        ],
    )
}

#[cfg(unix)]
#[test]
fn test_empty_l_entry_is_not_the_working_directory_7428() -> Result<()> {
    // `c.jq` is in the working directory: an `import` finds it there whatever `-L` says, but
    // `modulemeta` does not search the working directory, and `-L ""` does not add it.
    let tree = Tree::new(true)?;
    check(
        &tree,
        &[
            (&[""], "import \"c\" as c; c::f", Some("\"cwd\"\n")),
            (&[""], "\"c\"|modulemeta|.defs", None),
        ],
    )?;
    let (stdout, stderr, code) = tree.run(&[""], "\"c\"|modulemeta|.defs")?;
    assert_eq!(
        (stdout.as_str(), stderr.as_str(), code),
        ("", "jq: error (at <unknown>): module not found: c\n", 5)
    );
    let (_, stderr, _) = tree.run(&["missing"], IMPORT)?;
    assert_eq!(stderr, NOT_FOUND_M);
    Ok(())
}
