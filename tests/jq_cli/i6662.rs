//! #6662: `isvalid(f)` is a succinctly extension (ADR-0018 rule 5), not a jq builtin: jq 1.6, 1.7.1
//! and 1.8.2 all refuse `isvalid/1` with `isvalid/1 is not defined` (exit 3). An extension is
//! allowed where it is marked as one and changes no program the reference accepts, so these rows pin
//! both halves: the call is accepted, and everything jq accepts around the name still answers as
//! jq does (a user `def` of the name wins, other arities are jq's own compile error, an object key
//! of that name is a key, and `builtins` does not list it).

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn isvalid_one_is_an_extension_that_answers_true_or_false_6662() -> Result<()> {
    for (filter, want) in [
        ("isvalid(.)", "true\n"),
        ("isvalid(error)", "false\n"),
        ("isvalid(empty)", "false\n"),
        ("[isvalid(1, 2)]", "[true]\n"),
    ] {
        let (out, err, code) = run_jq_full(&["-c", filter], Some("1"))?;
        assert_eq!(
            (out.as_str(), err.as_str(), code),
            (want, "", 0),
            "{filter}"
        );
    }
    Ok(())
}

#[test]
fn a_user_definition_of_isvalid_wins_exactly_as_in_jq_6662() -> Result<()> {
    // Each program was run on `/usr/bin/jq` 1.7.1 and printed the same.
    for (filter, want) in [
        ("def isvalid(f): 7; isvalid(.)", "7\n"),
        ("def isvalid(f): 7; isvalid(error)", "7\n"),
        ("def isvalid: 9; isvalid", "9\n"),
        ("def isvalid(f; g): 5; isvalid(1; 2)", "5\n"),
        ("{isvalid: 1} | .isvalid", "1\n"),
    ] {
        let (out, err, code) = run_jq_full(&["-c", filter], Some("1"))?;
        assert_eq!(
            (out.as_str(), err.as_str(), code),
            (want, "", 0),
            "{filter}"
        );
    }
    Ok(())
}

#[test]
fn isvalid_at_another_arity_is_jqs_compile_error_6662() -> Result<()> {
    // jq 1.7.1: `jq: error: isvalid/<n> is not defined at <top-level>, line 1:` then the program
    // text and `jq: 1 compile error`, exit 3.
    for (filter, name) in [("isvalid", "isvalid/0"), ("isvalid(1; 2)", "isvalid/2")] {
        let (out, err, code) = run_jq_full(&["-c", filter], Some("1"))?;
        assert_eq!(out, "", "{filter}");
        assert_eq!(code, 3, "{filter}");
        assert!(
            err.starts_with(&format!(
                "jq: error: {name} is not defined at <top-level>, line 1:\n{filter}\n"
            )),
            "{filter}: {err}"
        );
        assert!(err.ends_with("jq: 1 compile error\n"), "{filter}: {err}");
    }
    Ok(())
}

#[test]
fn builtins_does_not_list_the_isvalid_extension_6662() -> Result<()> {
    let (out, err, code) = run_jq_full(
        &["-nc", "[builtins[] | select(startswith(\"isvalid\"))]"],
        None,
    )?;
    assert_eq!((out.as_str(), err.as_str(), code), ("[]\n", "", 0));
    Ok(())
}
