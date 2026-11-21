//! #7433: `succinctly jq` refuses a non-ASCII identifier (see `tests/jq_cli/i7433.rs`), and
//! `succinctly yq` follows yq here, not jq. Live against yq v4.53.3: the unquoted path element
//! after a `.` takes any character but the delimiters, so `.é`, `.中` and `.a.é` read the keys
//! `é`, `中` and `a`/`é`; every other place a name is spelled (an object key, a `def`, a
//! `$variable`, a label, a bare word, `@format`) is a lexer error.

use anyhow::Result;

use crate::run_yq_stdin;

const DOC: &str = r#"{"é":1,"a":{"é":2},"中":3}"#;

#[test]
fn a_path_element_after_a_dot_takes_non_ascii_letters_7433() -> Result<()> {
    for (filter, want) in [
        (".é", "1\n"),
        (".a.é", "2\n"),
        (".中", "3\n"),
        (".é?", "1\n"),
        (".é | length", "1\n"),
        ("[.é]", "[1]\n"),
        (".é as $x | $x", "1\n"),
    ] {
        let (stdout, code) = run_yq_stdin(filter, DOC, &["-p", "json", "-o", "json", "-I0"])?;
        assert_eq!((stdout.as_str(), code), (want, 0), "{filter}");
    }
    Ok(())
}

#[test]
fn every_other_spelling_of_a_non_ascii_name_is_refused_7433() -> Result<()> {
    for filter in [
        "{é:1}",
        "def é: 1; é",
        ". as $é | $é",
        ".a as $é | 1",
        "label $é | 1",
        "é",
        "ünicode",
        ".[é]",
        "@é",
        "$é",
        ".a | é",
    ] {
        let (stdout, code) = run_yq_stdin(filter, DOC, &["-p", "json", "-o", "json", "-I0"])?;
        assert_ne!(code, 0, "{filter}: {stdout}");
        assert_eq!(stdout, "", "{filter}");
    }
    Ok(())
}
