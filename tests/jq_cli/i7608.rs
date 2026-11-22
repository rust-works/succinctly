//! #7608: with `HOME` unset jq cannot expand a `~/` search entry, the `~/.jq` of its default list
//! included, and its `module not found` says so: ` (Could not expand ~/.jq. (Could not find home
//! directory.))`. `build_lib_search_chain` keeps the error of the last entry it could not expand,
//! from `-L`, from the default list, or from a directive's `search` metadata, and `find_lib` appends
//! it; `modulemeta` too. An empty `HOME` expands (to `/...`), so it has no note.
//!
//! Every expectation was captured from jq 1.7.1 (`/usr/bin/jq`); jq 1.8.2 agrees on every row.

use anyhow::Result;

/// `(HOME, or None to unset it; arguments; stdout; stderr; exit status)`.
type Row = (
    Option<&'static str>,
    &'static [&'static str],
    &'static str,
    &'static str,
    i32,
);

const ROWS: &[Row] = &[
    (None, &["-n", "import \"zz\" as m; 1"], "", "jq: error: module not found: zz (Could not expand ~/.jq. (Could not find home directory.))\n\njq: 1 compile error\n", 3),
    (None, &["-n", "include \"zz\"; 1"], "", "jq: error: module not found: zz (Could not expand ~/.jq. (Could not find home directory.))\n\njq: 1 compile error\n", 3),
    (None, &["-n", "import \"zz\" as $d; 1"], "", "jq: error: module not found: zz (Could not expand ~/.jq. (Could not find home directory.))\n\njq: 1 compile error\n", 3),
    (None, &["-L", "~/a", "-L", "~/b", "-n", "import \"zz\" as m; 1"], "", "jq: error: module not found: zz (Could not expand ~/b. (Could not find home directory.))\n\njq: 1 compile error\n", 3),
    (None, &["-L", "~/a", "-L", "/nonexistent", "-n", "import \"zz\" as m; 1"], "", "jq: error: module not found: zz (Could not expand ~/a. (Could not find home directory.))\n\njq: 1 compile error\n", 3),
    (None, &["-L", "/nonexistent", "-n", "import \"zz\" as m; 1"], "", "jq: error: module not found: zz\n\njq: 1 compile error\n", 3),
    (None, &["-n", "import \"zz\" as m {search: \"~/m\"}; 1"], "", "jq: error: module not found: zz (Could not expand ~/m. (Could not find home directory.))\n\njq: 1 compile error\n", 3),
    (None, &["-n", "import \"zz\" as m {search: [\"~/a\", \"/nonexistent\", \"~/b\"]}; 1"], "", "jq: error: module not found: zz (Could not expand ~/b. (Could not find home directory.))\n\njq: 1 compile error\n", 3),
    (None, &["-n", "import \"zz\" as m {search: [\"~/a\", \"./x\"]}; 1"], "", "jq: error: module not found: zz (Could not expand ~/a. (Could not find home directory.))\n\njq: 1 compile error\n", 3),
    (None, &["-n", "include \"zz\" {search: \"/nonexistent\"}; 1"], "", "jq: error: module not found: zz\n\njq: 1 compile error\n", 3),
    (None, &["-n", "import \"zz\" as m {search: []}; 1"], "", "jq: error: module not found: zz\n\njq: 1 compile error\n", 3),
    (None, &["-n", "import \"zz\" as m {search: \"~\"}; 1"], "", "jq: error: module not found: zz\n\njq: 1 compile error\n", 3),
    (None, &["-n", "\"zz\" | modulemeta"], "", "jq: error (at <unknown>): module not found: zz (Could not expand ~/.jq. (Could not find home directory.))\n", 5),
    (None, &["-n", "\"zz\" | try modulemeta catch ."], "\"module not found: zz (Could not expand ~/.jq. (Could not find home directory.))\"\n", "", 0),
    (None, &["-L", "lib", "-n", "import \"mm\" as m; 1"], "", "jq: error: module not found: qq (Could not expand ~/x. (Could not find home directory.))\n\njq: 1 compile error\n", 3),
    (None, &["-L", "lib", "-n", "import \"zz\" as m {search: \"~/m\"}; 1"], "", "jq: error: module not found: zz (Could not expand ~/m. (Could not find home directory.))\n\njq: 1 compile error\n", 3),
    (None, &["-L", "~", "-n", "import \"zz\" as m; 1"], "", "jq: error: module not found: zz\n\njq: 1 compile error\n", 3),
    (None, &["-L", "", "-n", "import \"zz\" as m; 1"], "", "jq: error: module not found: zz\n\njq: 1 compile error\n", 3),
    (None, &["-n", "import \"zz\" as m; import \"yy\" as n; 1"], "", "jq: error: module not found: yy (Could not expand ~/.jq. (Could not find home directory.))\n\njq: 1 compile error\n", 3),
    (None, &["-n", "import \"zz\" as $d {search: \"~/m\"}; 1"], "", "jq: error: module not found: zz (Could not expand ~/m. (Could not find home directory.))\n\njq: 1 compile error\n", 3),
    (Some(""), &["-n", "import \"zz\" as m; 1"], "", "jq: error: module not found: zz\n\njq: 1 compile error\n", 3),
    (Some(""), &["-n", "import \"zz\" as m {search: \"~/m\"}; 1"], "", "jq: error: module not found: zz\n\njq: 1 compile error\n", 3),
    (Some("/nonexistent-home"), &["-n", "import \"zz\" as m; 1"], "", "jq: error: module not found: zz\n\njq: 1 compile error\n", 3),
];

#[test]
fn module_not_found_names_the_search_entry_jq_cannot_expand_7608() -> Result<()> {
    let dir = tempfile::tempdir()?;
    std::fs::create_dir(dir.path().join("lib"))?;
    std::fs::write(
        dir.path().join("lib/mm.jq"),
        "import \"qq\" as q {search: \"~/x\"}; def f: 1;",
    )?;
    for &(home, args, stdout, stderr, code) in ROWS {
        let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_succinctly"));
        command
            .arg("jq")
            .args(args)
            .current_dir(dir.path())
            .env("LC_ALL", "C")
            .env_remove("JQ_LIBRARY_PATH");
        match home {
            Some(home) => command.env("HOME", home),
            None => command.env_remove("HOME"),
        };
        let output = command.output()?;
        assert_eq!(
            (
                String::from_utf8_lossy(&output.stdout).as_ref(),
                String::from_utf8_lossy(&output.stderr).as_ref(),
                output.status.code()
            ),
            (stdout, stderr, Some(code)),
            "HOME {home:?}, {args:?}"
        );
    }
    Ok(())
}
