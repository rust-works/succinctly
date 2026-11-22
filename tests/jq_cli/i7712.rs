//! #7712: jq reads its command line left to right and stops at the first error, so a bad
//! `--argjson` text, an unreadable `--slurpfile`/`--rawfile` file or a `--jsonargs` word that is
//! not JSON is reported where it stands: before a later `-h`/`-V`/`--build-configuration`, which
//! then never runs, and before a later unknown option, missing value or bad `--indent`.
//! `succinctly jq` printed the help or version with exit 0 there, and reported a later unknown
//! option, missing value or `--indent` first.
//!
//! Every row was captured from `/usr/bin/jq` (1.7.1) with `argv[0]` `jq`; jq 1.8.2
//! (`/opt/homebrew/bin/jq`) gives the same exit status and message on each (with its own URL in
//! the usage hint). Three rows keep succinctly's recorded wording: a refused `--indent` value is
//! clap's (`docs/compliance/jq/limitations.md`, #7501), and `-nL` is jq 1.8.2's `-L takes a
//! parameter`, where jq 1.7.1 refused `-L` in a bundle (#7438). A help, version or
//! build-configuration row checks only that it printed with exit 0: the text is succinctly's own.

use anyhow::Result;

use crate::run_jq_in_dir;

/// `ok.json` holds `[1]`; `nope.json`, `nope2.json` and `nope.jq` do not exist.
fn fixture() -> Result<tempfile::TempDir> {
    let dir = tempfile::tempdir()?;
    std::fs::write(dir.path().join("ok.json"), "[1]\n")?;
    Ok(dir)
}

fn run(dir: &tempfile::TempDir, argv: &str) -> Result<(String, String, i32)> {
    let words: Vec<&str> = argv.split(' ').collect();
    run_jq_in_dir(dir.path(), &words, Some(""))
}

/// The first error in argv order, with jq's message and exit status; `None` is a help, version or
/// build-configuration option that jq reaches first, printed on standard output with exit 0.
#[test]
fn test_7712_the_first_option_error_in_argv_order_is_reported() -> Result<()> {
    let dir = fixture()?;
    let mut failures = Vec::new();
    for (argv, stderr, code) in [
        ("-n --argjson a { -h", Some("jq: invalid JSON text passed to --argjson\nUse jq --help for help with command-line options,\nor see the jq manpage, or online docs  at https://jqlang.github.io/jq\n"), 2),
        ("-n --argjson a { --help", Some("jq: invalid JSON text passed to --argjson\nUse jq --help for help with command-line options,\nor see the jq manpage, or online docs  at https://jqlang.github.io/jq\n"), 2),
        ("-n --slurpfile a nope.json -V", Some("jq: Bad JSON in --slurpfile a nope.json: Could not open nope.json: No such file or directory\n"), 2),
        ("-n --rawfile a nope.json --version", Some("jq: Bad JSON in --rawfile a nope.json: Could not open nope.json: No such file or directory\n"), 2),
        ("-n --slurpfile a nope.json --build-configuration", Some("jq: Bad JSON in --slurpfile a nope.json: Could not open nope.json: No such file or directory\n"), 2),
        ("-n --jsonargs . nope -h", Some("jq: invalid JSON text passed to --jsonargs\nUse jq --help for help with command-line options,\nor see the jq manpage, or online docs  at https://jqlang.github.io/jq\n"), 2),
        ("-n --argjson a { -ch", Some("jq: invalid JSON text passed to --argjson\nUse jq --help for help with command-line options,\nor see the jq manpage, or online docs  at https://jqlang.github.io/jq\n"), 2),
        ("-n -h --argjson a {", None, 0),
        ("-n -V --slurpfile a nope.json", None, 0),
        ("-n --build-configuration --jsonargs . nope", None, 0),
        ("-n --argjson a { . -x", Some("jq: invalid JSON text passed to --argjson\nUse jq --help for help with command-line options,\nor see the jq manpage, or online docs  at https://jqlang.github.io/jq\n"), 2),
        ("-n --argjson a { . --bogus", Some("jq: invalid JSON text passed to --argjson\nUse jq --help for help with command-line options,\nor see the jq manpage, or online docs  at https://jqlang.github.io/jq\n"), 2),
        ("-n --slurpfile a nope.json -cx .", Some("jq: Bad JSON in --slurpfile a nope.json: Could not open nope.json: No such file or directory\n"), 2),
        ("-n --jsonargs . { --bogus", Some("jq: invalid JSON text passed to --jsonargs\nUse jq --help for help with command-line options,\nor see the jq manpage, or online docs  at https://jqlang.github.io/jq\n"), 2),
        ("-n -x --argjson a {", Some("jq: Unknown option -x\nUse jq --help for help with command-line options,\nor see the jq manpage, or online docs  at https://jqlang.github.io/jq\n"), 2),
        ("-n --bogus --slurpfile a nope.json .", Some("jq: Unknown option --bogus\nUse jq --help for help with command-line options,\nor see the jq manpage, or online docs  at https://jqlang.github.io/jq\n"), 2),
        ("-n --indent 9 -x .", Some("error: invalid value '9' for '--indent <N>': --indent takes a number between -1 and 7\n\nFor more information, try '--help'.\n"), 2),
        ("-n -x --indent 9 .", Some("jq: Unknown option -x\nUse jq --help for help with command-line options,\nor see the jq manpage, or online docs  at https://jqlang.github.io/jq\n"), 2),
        ("-n --slurpfile a nope.json . --arg", Some("jq: Bad JSON in --slurpfile a nope.json: Could not open nope.json: No such file or directory\n"), 2),
        ("-n --argjson a { . --arg x", Some("jq: invalid JSON text passed to --argjson\nUse jq --help for help with command-line options,\nor see the jq manpage, or online docs  at https://jqlang.github.io/jq\n"), 2),
        ("-n . --arg", Some("jq: --arg takes two parameters (e.g. --arg varname value)\nUse jq --help for help with command-line options,\nor see the jq manpage, or online docs  at https://jqlang.github.io/jq\n"), 2),
        ("-n . --arg x", Some("jq: --arg takes two parameters (e.g. --arg varname value)\nUse jq --help for help with command-line options,\nor see the jq manpage, or online docs  at https://jqlang.github.io/jq\n"), 2),
        ("-n . --argjson x", Some("jq: --argjson takes two parameters (e.g. --argjson varname text)\nUse jq --help for help with command-line options,\nor see the jq manpage, or online docs  at https://jqlang.github.io/jq\n"), 2),
        ("-n . --slurpfile", Some("jq: --slurpfile takes two parameters (e.g. --slurpfile varname filename)\nUse jq --help for help with command-line options,\nor see the jq manpage, or online docs  at https://jqlang.github.io/jq\n"), 2),
        ("-n . --rawfile x", Some("jq: --rawfile takes two parameters (e.g. --rawfile varname filename)\nUse jq --help for help with command-line options,\nor see the jq manpage, or online docs  at https://jqlang.github.io/jq\n"), 2),
        ("-n . --indent", Some("jq: --indent takes one parameter\nUse jq --help for help with command-line options,\nor see the jq manpage, or online docs  at https://jqlang.github.io/jq\n"), 2),
        ("-n . -L", Some("-L takes a parameter: (e.g. -L /search/path or -L/search/path)\nUse jq --help for help with command-line options,\nor see the jq manpage, or online docs  at https://jqlang.github.io/jq\n"), 2),
        ("-nL", Some("-L takes a parameter: (e.g. -L /search/path or -L/search/path)\nUse jq --help for help with command-line options,\nor see the jq manpage, or online docs  at https://jqlang.github.io/jq\n"), 2),
        ("-n --argjson a { --indent 9 .", Some("jq: invalid JSON text passed to --argjson\nUse jq --help for help with command-line options,\nor see the jq manpage, or online docs  at https://jqlang.github.io/jq\n"), 2),
        ("-n --indent 9 --argjson a { .", Some("error: invalid value '9' for '--indent <N>': --indent takes a number between -1 and 7\n\nFor more information, try '--help'.\n"), 2),
        ("-n --slurpfile a nope.json --argjson b { .", Some("jq: Bad JSON in --slurpfile a nope.json: Could not open nope.json: No such file or directory\n"), 2),
        ("-n --argjson b { --slurpfile a nope.json .", Some("jq: invalid JSON text passed to --argjson\nUse jq --help for help with command-line options,\nor see the jq manpage, or online docs  at https://jqlang.github.io/jq\n"), 2),
        ("-n --rawfile a nope.json --slurpfile b nope2.json .", Some("jq: Bad JSON in --rawfile a nope.json: Could not open nope.json: No such file or directory\n"), 2),
        ("-n --jsonargs . { --argjson a {", Some("jq: invalid JSON text passed to --jsonargs\nUse jq --help for help with command-line options,\nor see the jq manpage, or online docs  at https://jqlang.github.io/jq\n"), 2),
        ("-n --jsonargs . { --slurpfile a nope.json", Some("jq: invalid JSON text passed to --jsonargs\nUse jq --help for help with command-line options,\nor see the jq manpage, or online docs  at https://jqlang.github.io/jq\n"), 2),
        ("-n --argjson a { --jsonargs . {", Some("jq: invalid JSON text passed to --argjson\nUse jq --help for help with command-line options,\nor see the jq manpage, or online docs  at https://jqlang.github.io/jq\n"), 2),
        ("-n --slurpfile a nope.json --jsonargs . {", Some("jq: Bad JSON in --slurpfile a nope.json: Could not open nope.json: No such file or directory\n"), 2),
        ("-n --jsonargs . 1 -- {", Some("jq: invalid JSON text passed to --jsonargs\nUse jq --help for help with command-line options,\nor see the jq manpage, or online docs  at https://jqlang.github.io/jq\n"), 2),
        ("-f nope.jq --argjson a {", Some("jq: invalid JSON text passed to --argjson\nUse jq --help for help with command-line options,\nor see the jq manpage, or online docs  at https://jqlang.github.io/jq\n"), 2),
        ("--argjson a { -f nope.jq", Some("jq: invalid JSON text passed to --argjson\nUse jq --help for help with command-line options,\nor see the jq manpage, or online docs  at https://jqlang.github.io/jq\n"), 2),
        (". nope.json --argjson a {", Some("jq: invalid JSON text passed to --argjson\nUse jq --help for help with command-line options,\nor see the jq manpage, or online docs  at https://jqlang.github.io/jq\n"), 2),
        ("-f nope.jq --slurpfile a ok.json", Some("jq: Could not open nope.jq: No such file or directory\n"), 2),
        ("-n --argjson a { --run-tests", Some("jq: invalid JSON text passed to --argjson\nUse jq --help for help with command-line options,\nor see the jq manpage, or online docs  at https://jqlang.github.io/jq\n"), 2),
    ] {
        let (out, err, status) = run(&dir, argv)?;
        let ok = match stderr {
            Some(want) => err == want && status == code && out.is_empty(),
            None => err.is_empty() && status == code && !out.is_empty(),
        };
        if !ok {
            failures.push(format!(
                "{argv}: exit {status}, stderr {err:?}, stdout {:?}",
                out.chars().take(60).collect::<String>()
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    Ok(())
}

/// A name already bound keeps its first value, and jq reads no later value for it: neither the
/// `--argjson` text nor the `--slurpfile`/`--rawfile` file, so neither can fail.
#[test]
fn test_7712_a_bound_name_is_not_read_again() -> Result<()> {
    let dir = fixture()?;
    for (argv, stdout) in [
        ("-n --arg a 1 --argjson a { $a", "\"1\"\n"),
        ("-n --arg a 1 --slurpfile a nope.json $a", "\"1\"\n"),
        (
            "-nc --slurpfile a ok.json --rawfile a nope.json $a",
            "[[1]]\n",
        ),
    ] {
        assert_eq!(
            run(&dir, argv)?,
            (stdout.to_string(), String::new(), 0),
            "{argv}"
        );
    }
    Ok(())
}
