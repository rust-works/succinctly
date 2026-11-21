//! #7438: `succinctly jq` parsed its command line with clap's grammar, which accepts shapes jq
//! refuses (`--indent=3`, `--args=`, `--jsonargs=1`) and refuses shapes jq accepts (`-V1`). It now
//! reads argv as jq's own option loop does (`walk_jq_argv`), and clap parses only the options found.
//!
//! jq 1.7.1 and 1.8.2 differ on short-option bundles: 1.7.1 counted each distinct letter once and
//! refused a repeat (`-nn`) and `-L` inside a bundle (`-nL lib`), and named the whole word
//! (`Unknown option -fprog.jq`). jq 1.8's in-order parser (jqlang/jq#3194, commit ff4acba769,
//! "Parse short options in order given") lists those refusals as the bugs it fixes and names the
//! first bad letter (`Unknown option -p`), so those rows follow 1.8.2. Every row was captured from
//! `/usr/bin/jq` (1.7.1) and `/opt/homebrew/bin/jq` (1.8.2). The usage hint after `Unknown option`
//! is 1.7.1's, as in every other usage error of `succinctly jq`.

use anyhow::Result;

use crate::run_jq_in_dir;

/// `prog.jq` (`.a`), `a.json` (`{"a":1}`) and an empty directory `lib`.
fn fixture() -> Result<tempfile::TempDir> {
    let dir = tempfile::tempdir()?;
    std::fs::write(dir.path().join("prog.jq"), ".a\n")?;
    std::fs::write(dir.path().join("a.json"), "{\"a\":1}\n")?;
    std::fs::create_dir(dir.path().join("lib"))?;
    Ok(dir)
}

/// jq's refusal of `word`, with the usage hint.
fn unknown(word: &str) -> String {
    format!(
        "jq: Unknown option {word}\n\
         Use jq --help for help with command-line options,\n\
         or see the jq manpage, or online docs  at https://jqlang.github.io/jq\n"
    )
}

fn run(dir: &tempfile::TempDir, argv: &str, input: &str) -> Result<(String, String, i32)> {
    let words: Vec<&str> = argv.split(' ').collect();
    run_jq_in_dir(dir.path(), &words, Some(input))
}

/// A letter may repeat in a bundle, and `-L` may end one, taking the rest of the word or the next
/// word. jq 1.8.2's stdout and exit status; jq 1.7.1 refuses every row (`Unknown option`, exit 2).
#[test]
fn test_7438_a_bundle_is_read_a_letter_at_a_time_as_in_jq_1_8_2() -> Result<()> {
    let dir = fixture()?;
    for (argv, stdout, code) in [
        ("-cc .", "[1]\n", 0),
        ("-nn 1", "1\n", 0),
        ("-ss .", "[\n  [\n    1\n  ]\n]\n", 0),
        ("-ee -n .", "null\n", 1),
        // `1` is the library directory and the program is the default `.`.
        ("-nL 1", "null\n", 0),
        // jq 1.8.2's own regression tests for jqlang/jq#3194.
        ("-nL. 42", "42\n", 0),
        ("-nL . 42", "42\n", 0),
        ("-nLlib 1", "1\n", 0),
        ("-RsjL lib .", "[1]", 0),
        ("-ff prog.jq a.json", "1\n", 0),
    ] {
        let (out, err, status) = run(&dir, argv, "[1]")?;
        assert_eq!(
            (out.as_str(), err.as_str(), status),
            (stdout, "", code),
            "jq {argv}"
        );
    }
    Ok(())
}

/// jq has no `--name=value` syntax and refuses a letter it does not define; both jq versions exit 2
/// with `Unknown option` and nothing on stdout. Before #7438 the first five rows ran the program.
#[test]
fn test_7438_what_jq_refuses_is_refused_with_its_words() -> Result<()> {
    let dir = fixture()?;
    for (argv, named) in [
        ("--indent=3 .", "--indent=3"),
        ("--args= .", "--args="),
        ("--jsonargs=1 .", "--jsonargs=1"),
        ("-n --arg=x 1", "--arg=x"),
        ("--tab=1 .", "--tab=1"),
        ("--from-file=prog.jq a.json", "--from-file=prog.jq"),
        // jq 1.8.2 names the first letter it does not define, jq 1.7.1 the whole word.
        ("-fprog.jq a.json", "-p"),
        ("-n -library-path", "-l"),
        ("-c1 .", "-1"),
        ("-n- .", "--"),
        ("-xV", "-x"),
        ("--compac .", "--compac"),
    ] {
        let (out, err, status) = run(&dir, argv, "[1]")?;
        assert_eq!(
            (out.as_str(), err.as_str(), status),
            ("", unknown(named).as_str(), 2),
            "jq {argv}"
        );
    }
    Ok(())
}

/// jq exits at a help or version option, so nothing after it is read: `-V1`, `-V=1` and `-nVx`
/// print the version (both jq versions), and of `-hV` and `-Vh` the first letter wins (jq 1.8.2's
/// regression tests for jqlang/jq#3194; jq 1.7.1 always chose help). The version and help text are
/// succinctly's own (the ledger's `cli:version` and `cli:help-exit`).
#[test]
fn test_7438_help_and_version_end_the_command_line() -> Result<()> {
    let dir = fixture()?;
    let (version, _, _) = run(&dir, "-V", "")?;
    let (help, _, _) = run(&dir, "-h", "")?;
    assert!(version.starts_with("succinctly jq"), "{version}");
    assert_ne!(help, version);
    for (argv, want) in [
        ("-V1", &version),
        ("-V=1", &version),
        ("-nVx --bogus", &version),
        ("-Vh", &version),
        ("-V -h", &version),
        ("-hV", &help),
        ("-h -V", &help),
        ("-ch --indent=3", &help),
    ] {
        let (out, err, status) = run(&dir, argv, "")?;
        assert_eq!(
            (out.as_str(), err.as_str(), status),
            (want.as_str(), "", 0),
            "jq {argv}"
        );
    }
    Ok(())
}

/// `--binary` is in jq 1.7.1's own `--help`, but 1.7.1 refused it outside Windows; jq 1.8 fixed that
/// (jqlang/jq#3131), so it is the no-op it is there. `--library-path` is the long spelling of `-L`
/// that jq 1.8 added (jqlang/jq@0558e77e8c: a new option, so 1.7.1, which refuses it, stays the
/// reference), and a succinctly-only option keeps clap's `--name=value` form.
#[test]
fn test_7438_binary_library_path_and_extension_options() -> Result<()> {
    let dir = fixture()?;
    let (out, err, status) = run(&dir, "--binary .", "[1]")?;
    assert_eq!((out.as_str(), err.as_str(), status), ("[\n  1\n]\n", "", 0));
    let (out, err, status) = run(&dir, "-n --library-path . 1", "")?;
    assert_eq!(
        (out.as_str(), err.as_str(), status),
        ("", unknown("--library-path").as_str(), 2)
    );
    let (out, err, status) = run(&dir, "-c --on-error=stop .a", "{\"a\":1}")?;
    assert_eq!((out.as_str(), err.as_str(), status), ("1\n", "", 0));
    Ok(())
}

/// The `sjq` and `jq` multi-call aliases read argv the same way.
#[test]
#[cfg(unix)]
fn test_7438_the_aliases_read_argv_as_jq_does() -> Result<()> {
    let alias_dir = tempfile::tempdir()?;
    for name in ["sjq", "jq"] {
        let alias = alias_dir.path().join(name);
        std::os::unix::fs::symlink(crate::succinctly_bin(), &alias)?;
        for (argv, stdout, stderr, code) in [
            (&["-cc", "."][..], "[1]\n", String::new(), 0),
            (&["--indent=3", "."][..], "", unknown("--indent=3"), 2),
            (&["-fprog.jq", "a.json"][..], "", unknown("-p"), 2),
        ] {
            let (output, status) = crate::spawn_with_signal_retry(
                || {
                    let mut command = std::process::Command::new(&alias);
                    command.args(argv);
                    command
                },
                Some(b"[1]"),
            )?;
            assert_eq!(
                (
                    String::from_utf8(output.stdout)?.as_str(),
                    String::from_utf8(output.stderr)?,
                    status
                ),
                (stdout, stderr, code),
                "{name} {argv:?}"
            );
        }
    }
    Ok(())
}
