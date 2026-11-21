//! #7427: a malformed value under a key that a later member repeats is refused by every route
//! that reads the values an object keeps, as jq 1.7.1 refuses the document, instead of being
//! dropped by the duplicate-key collapse ("first position, last value") unread.
//!
//! The first test's stderr lines and exit status were captured from jq 1.7.1 (`/usr/bin/jq`). The
//! last test pins what is *not* jq's: a filter that reads no value of the object (`length`,
//! `keys`, `has`) or only the value that wins (`.a`) still answers, as it does for a malformed
//! value no key shadows (`docs/compliance/jq/limitations.md`, #3035).

use anyhow::Result;

use crate::{run_jq_over_files, run_jq_stdin, run_jq_stdin_streams};

#[test]
fn test_a_malformed_shadowed_value_is_refused_where_the_object_is_read_7427() -> Result<()> {
    // The document, and jq's parse error for it.
    let documents = [
        (
            r#"{"a":tru,"b":2,"a":1}"#,
            "jq: parse error: Invalid literal at line 1, column 9\n",
        ),
        (
            r#"{"a":[1,,2],"a":1}"#,
            "jq: parse error: Expected value before ',' at line 1, column 9\n",
        ),
        (
            r#"{"a":{x},"a":1}"#,
            "jq: parse error: Invalid numeric literal at line 1, column 8\n",
        ),
        (
            r#"{"a":1,"a":tru,"a":2}"#,
            "jq: parse error: Invalid literal at line 1, column 15\n",
        ),
        (
            r#"{"b":{"a":tru,"a":1}}"#,
            "jq: parse error: Invalid literal at line 1, column 14\n",
        ),
    ];
    let filters: [(&str, &[&str]); 8] = [
        (".", &["-c"]),
        (".", &[]),
        (".[]", &["-c"]),
        ("[.[]]", &["-c"]),
        ("to_entries", &["-c"]),
        ("with_entries(.)", &["-c"]),
        ("map_values(.)", &["-c"]),
        ("tojson", &["-c"]),
    ];
    for (document, jq_error) in documents {
        for (filter, args) in filters {
            let (stdout, stderr, status) = run_jq_stdin_streams(filter, document, args)?;
            assert_eq!(
                (stdout.as_str(), stderr.as_str(), status),
                ("", jq_error, 5),
                "{args:?} {filter} on {document}"
            );
        }
        // A slurped file is read again for jq's own words; slurped stdin keeps the evaluator's,
        // as for any malformed member (`docs/compliance/jq/limitations.md`, #5576).
        let (stdout, stderr, status, _) = run_jq_over_files(&["-c", "-s", "."], &[document])?;
        assert_eq!(
            (stdout.as_str(), stderr.as_str(), status),
            ("", jq_error, 5),
            "-s . on a file holding {document}"
        );
        let (stdout, stderr, status) = run_jq_stdin_streams(".", document, &["-c", "-s"])?;
        assert_eq!((stdout.as_str(), status), ("", 5), "-s . on {document}");
        assert!(stderr.starts_with("jq: error (at <stdin>:0): "), "{stderr}");
    }
    Ok(())
}

#[test]
fn test_a_well_formed_shadowed_value_still_collapses_7427() -> Result<()> {
    // Captured from jq 1.7.1: first position, last value.
    let pad: String = (0..200)
        .map(|i| format!(r#""k{i}":[{i},{{"x":null}}],"#))
        .collect();
    let wide = format!(r#"{{"a":{{"deep":[1,2,{{"z":"s"}}]}},{pad}"a":7}}"#);
    let (out, status) = run_jq_stdin(".a, length", &wide, &["-c"])?;
    assert_eq!((out.as_str(), status), ("7\n201\n", 0));
    for (filter, document, want) in [
        (
            ".",
            r#"{"a":[1,{"b":null}],"b":2,"a":3}"#,
            "{\"a\":3,\"b\":2}\n",
        ),
        ("[.[]]", r#"{"a":"A","a":"x"}"#, "[\"x\"]\n"),
        (
            "to_entries",
            r#"{"a":1,"a":2}"#,
            "[{\"key\":\"a\",\"value\":2}]\n",
        ),
    ] {
        let (out, status) = run_jq_stdin(filter, document, &["-c"])?;
        assert_eq!((out.as_str(), status), (want, 0), "{filter} on {document}");
    }
    Ok(())
}

#[test]
fn test_a_filter_that_reads_no_shadowed_value_still_answers_7427() -> Result<()> {
    // Not jq's: jq 1.7.1 refuses the document whatever the filter. These read no value, or
    // only the one that wins, so they answer as they do on `{"a":tru,"b":2}`.
    let document = r#"{"a":tru,"b":2,"a":1}"#;
    for (filter, want) in [
        ("length", "2\n"),
        ("keys", "[\"a\",\"b\"]\n"),
        ("has(\"a\")", "true\n"),
        (".a", "1\n"),
    ] {
        let (out, status) = run_jq_stdin(filter, document, &["-c"])?;
        assert_eq!((out.as_str(), status), (want, 0), "{filter}");
    }
    Ok(())
}
