//! #6792: a `(PATH | key)` branch of a union of metadata-write targets (`((.a | key), .b)`) was not
//! read as a key target, so its text was dropped without a word and the document printed with
//! exit 0. Each branch is resolved on its own now. A key node among value targets is refused (the
//! head of a value is the next key's, and the two interact in an order go-yaml's emitter decides),
//! and several key nodes, and nothing else, are written. The written rows are captured from yq
//! v4.53.3 (and are goldens too, `meta_assign_keys_*_6792`).

use anyhow::Result;

use crate::run_yq_stdin_with_stderr;

const DOC: &str = "a: 1\nb: 2\nc: 3\n";

#[test]
fn test_a_key_among_value_targets_is_refused_not_dropped_6792() -> Result<()> {
    for filter in [
        r#"((.a | key), .b) head_comment = "x""#,
        r#"(.a, (.b | key)) foot_comment = "x""#,
        r#"((.b | key), .a) head_comment = "x""#,
        r#"((.a | key), .b) line_comment = "x""#,
        r#"((.a | key), .b) comments = "x""#,
    ] {
        let (stdout, stderr, code) = run_yq_stdin_with_stderr(filter, DOC, &[])?;
        assert_eq!(code, 1, "{filter}: {stdout:?} {stderr}");
        assert_eq!(stdout, "", "{filter}");
        assert!(stderr.contains("not yet supported"), "{filter}: {stderr}");
    }
    // The slots a key node does not take stay refused in a union too.
    let (stdout, stderr, code) =
        run_yq_stdin_with_stderr(r#"((.a | key), .b) style = "double""#, DOC, &[])?;
    assert_eq!((stdout.as_str(), code), ("", 1));
    assert!(
        stderr.contains("only supported with the comment slots"),
        "{stderr}"
    );
    Ok(())
}

#[test]
fn test_several_key_targets_are_written_6792() -> Result<()> {
    for (filter, want) in [
        (
            r#"(.[] | key) head_comment = "x""#,
            "# x\na: 1\n# x\nb: 2\n# x\nc: 3\n",
        ),
        (
            r#"((.a | key), (.c | key)) head_comment = "x""#,
            "# x\na: 1\nb: 2\n# x\nc: 3\n",
        ),
        (
            r#"(.[] | key) line_comment = "x""#,
            "a: 1 # x\nb: 2 # x\nc: 3 # x\n",
        ),
        (
            r#"(.[] | key) head_comment |= "x""#,
            "# x\na: 1\n# x\nb: 2\n# x\nc: 3\n",
        ),
    ] {
        let (stdout, stderr, code) = run_yq_stdin_with_stderr(filter, DOC, &[])?;
        assert_eq!((stdout.as_str(), code), (want, 0), "{filter}: {stderr}");
    }
    Ok(())
}

#[test]
fn test_a_union_of_value_targets_and_the_clearing_form_are_as_before_6792() -> Result<()> {
    // No key branch: the union is one target expression, as it was.
    let (stdout, _, code) = run_yq_stdin_with_stderr(r#"(.a, .c) head_comment = "x""#, DOC, &[])?;
    assert_eq!((stdout.as_str(), code), ("a: 1\n# x\nb: 2\nc: 3\n# x\n", 0));
    // Clearing in a union is written as yq writes it: `# h` is the document's header, and `# m`
    // is `b`'s own key head, which only a key target of `b` clears.
    for (filter, want) in [
        (
            r#"((.a | key), .b) head_comment = """#,
            "# h\na: 1\n# m\nb: 2\n",
        ),
        (
            r#"((.a | key), (.b | key)) head_comment = """#,
            "# h\na: 1\nb: 2\n",
        ),
    ] {
        let (stdout, _, code) = run_yq_stdin_with_stderr(filter, "# h\na: 1\n# m\nb: 2\n", &[])?;
        assert_eq!((stdout.as_str(), code), (want, 0), "{filter}");
    }
    Ok(())
}
