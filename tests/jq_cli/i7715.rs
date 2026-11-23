//! #7715: jq 1.7.1 and 1.8.2 run a file of jq test cases with `--run-tests [file]`, which
//! `succinctly jq` does not implement. It refused it as `Unknown option --run-tests`; it now says the
//! option is not supported (exit 2), as `docs/compliance/jq/limitations.md` records. The
//! `Unknown option` rows were captured from `/usr/bin/jq` (1.7.1) and agree with jq 1.8.2 apart
//! from its usage-hint URL.

use anyhow::Result;

use crate::run_jq_in_dir;

const REFUSED: &str =
    "jq: --run-tests is not supported: succinctly jq does not run jq's test files\n";

fn unknown(word: &str) -> String {
    format!(
        "jq: Unknown option {word}\n\
         Use jq --help for help with command-line options,\n\
         or see the jq manpage, or online docs  at https://jqlang.github.io/jq\n"
    )
}

#[test]
fn test_7715_run_tests_is_refused_as_unsupported() -> Result<()> {
    let dir = tempfile::tempdir()?;
    std::fs::write(dir.path().join("t.test"), ".a\n{\"a\":1}\n1\n")?;
    for (argv, stderr) in [
        ("--run-tests t.test", REFUSED.to_string()),
        // Without a file jq reads the cases from standard input.
        ("--run-tests", REFUSED.to_string()),
        ("-n --run-tests t.test", REFUSED.to_string()),
        // The words after it are the test runner's, not options.
        ("--run-tests -x", REFUSED.to_string()),
        ("-x --run-tests", unknown("-x")),
        ("--run-tests=x", unknown("--run-tests=x")),
        // jq reads the named arguments before it first.
        (
            "--argjson a { --run-tests",
            "jq: invalid JSON text passed to --argjson\n\
             Use jq --help for help with command-line options,\n\
             or see the jq manpage, or online docs  at https://jqlang.github.io/jq\n"
                .to_string(),
        ),
    ] {
        let words: Vec<&str> = argv.split(' ').collect();
        let got = run_jq_in_dir(dir.path(), &words, Some(""))?;
        assert_eq!(got, (String::new(), stderr, 2), "{argv}");
    }
    Ok(())
}
