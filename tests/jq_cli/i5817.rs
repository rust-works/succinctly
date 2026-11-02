//! #5817: jq leaves the quoted key out of `Cannot index <type> with string "<key>"` when the key
//! is 30 bytes or longer (a 29-byte key is quoted whole). The limit counts bytes. Answers captured
//! from jq 1.7.1 (`/usr/bin/jq`).

use anyhow::Result;

use crate::{run_jq_full, run_jq_null};

fn caught(filter: &str) -> Result<String> {
    let (stdout, code) = run_jq_null(filter, &["-c"])?;
    assert_eq!(code, 0, "{filter}");
    Ok(stdout)
}

#[test]
fn a_key_of_thirty_bytes_or_more_is_left_out_of_the_index_error_5817() -> Result<()> {
    let short = "k".repeat(29);
    let long = "k".repeat(40);
    for (filter, expected) in [
        (
            format!("[1] | try .[\"{short}\"] catch ."),
            format!("\"Cannot index array with string \\\"{short}\\\"\""),
        ),
        (
            format!("[1] | try .[\"{}\"] catch .", "k".repeat(30)),
            "\"Cannot index array with string\"".to_string(),
        ),
        (
            format!("1 | try .[\"{long}\"] catch ."),
            "\"Cannot index number with string\"".to_string(),
        ),
        (
            format!("\"s\" | try .[\"{long}\"] catch ."),
            "\"Cannot index string with string\"".to_string(),
        ),
        (
            format!("true | try .[\"{long}\"] catch ."),
            "\"Cannot index boolean with string\"".to_string(),
        ),
        // The field spelling and a delete reach the same message.
        (
            format!("[1] | try .{long} catch ."),
            "\"Cannot index array with string\"".to_string(),
        ),
        (
            format!("[1] | try del(.[\"{long}\"]) catch ."),
            "\"Cannot index array with string\"".to_string(),
        ),
        // Bytes, not characters: 14 two-byte characters are 28 bytes, 15 are 30.
        (
            format!("[1] | try .[\"{}\"] catch .", "\u{e9}".repeat(14)),
            format!(
                "\"Cannot index array with string \\\"{}\\\"\"",
                "\u{e9}".repeat(14)
            ),
        ),
        (
            format!("[1] | try .[\"{}\"] catch .", "\u{e9}".repeat(15)),
            "\"Cannot index array with string\"".to_string(),
        ),
    ] {
        assert_eq!(caught(&filter)?, format!("{expected}\n"), "{filter}");
    }
    Ok(())
}

/// Over a file, where the key is the document's own text (the fuzzer's finding), and uncaught.
#[test]
fn a_long_key_computed_from_the_document_is_left_out_5817() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(
        &["-c", "try .[(. | tojson)] catch ."],
        Some("[{\"a\":\"a\",\"b\":1,\"c\":null},{\"b\":\"\",\"a\":false}]"),
    )?;
    assert_eq!(
        (stdout.as_str(), code),
        ("\"Cannot index array with string\"\n", 0),
        "{stderr}"
    );
    let (_, stderr, code) = run_jq_full(
        &["-c", ".[(. | tojson)]"],
        Some("[{\"a\":\"a\",\"b\":1,\"c\":null},{\"b\":\"\",\"a\":false}]"),
    )?;
    assert_eq!(code, 5);
    assert!(
        stderr
            .trim_end()
            .ends_with("Cannot index array with string"),
        "{stderr}"
    );
    Ok(())
}
