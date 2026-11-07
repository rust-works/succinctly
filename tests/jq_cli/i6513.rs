//! #6513: after a string whose escapes decode to two bytes or more, jq's token buffer no longer
//! holds a `u` at index 1, so a bare `n` in the next `--seq` record is read as a number ("Invalid
//! numeric literal") and not as a keyword ("Invalid literal"). The reader models jq's in-place
//! decode of a string, the wording follows from the buffer, and the stdout and exit status were
//! always the same. Rows, messages and exit statuses were captured from jq 1.7.1 (`/usr/bin/jq`);
//! the unit test `a_decoded_string_leaves_its_second_byte_at_index_one_6513` pins the rule itself.

use anyhow::Result;

use crate::run_jq_full;

/// `(stdin, stdout, stderr, exit status)` under `jq -c --seq .`.
const ROWS: &[(&str, &str, &str, i32)] = &[
    ("\x1e\"\\udc00\"\n\x1en\n", "\x1e\"�\"\n", "jq: ignoring parse error: Invalid numeric literal at line 3, column 0 (need RS to resync)\n", 0),
    ("\x1e\"\\udc00\"\n\x1enu\n", "\x1e\"�\"\n", "jq: ignoring parse error: Invalid literal at line 3, column 0 (need RS to resync)\n", 0),
    ("\x1e\"\\ud83d\\ude00\"\n\x1en\n", "\x1e\"😀\"\n", "jq: ignoring parse error: Invalid numeric literal at line 3, column 0 (need RS to resync)\n", 0),
    ("\x1e\"\\u00e9\"\n\x1en\n", "\x1e\"é\"\n", "jq: ignoring parse error: Invalid numeric literal at line 3, column 0 (need RS to resync)\n", 0),
    ("\x1e\"\\u00e9\"\n\x1enu\n", "\x1e\"é\"\n", "jq: ignoring parse error: Invalid literal at line 3, column 0 (need RS to resync)\n", 0),
    ("\x1e\"\\u0041a\"\n\x1en\n", "\x1e\"Aa\"\n", "jq: ignoring parse error: Invalid numeric literal at line 3, column 0 (need RS to resync)\n", 0),
    ("\x1e\"\\u0041\"\n\x1en\n", "\x1e\"A\"\n", "jq: ignoring parse error: Invalid literal at line 3, column 0 (need RS to resync)\n", 0),
    ("\x1e\"\\u0041\\u0042\"\n\x1en\n", "\x1e\"AB\"\n", "jq: ignoring parse error: Invalid numeric literal at line 3, column 0 (need RS to resync)\n", 0),
    ("\x1e\"\\u00e9\\ud800\"\n\x1en\n", "", "jq: ignoring parse error: Invalid \\uXXXX\\uXXXX surrogate pair escape at line 1, column 15 (need RS to resync)\njq: ignoring parse error: Invalid numeric literal at line 3, column 0 (need RS to resync)\n", 0),
    ("\x1e\"\\udc00x\"\n\x1en\n", "\x1e\"�x\"\n", "jq: ignoring parse error: Invalid numeric literal at line 3, column 0 (need RS to resync)\n", 0),
    ("\x1e\"x\\u0075\"\n\x1en\n", "\x1e\"xu\"\n", "jq: ignoring parse error: Invalid literal at line 3, column 0 (need RS to resync)\n", 0),
    ("\x1e\"\\u0000\"\n\x1en\n", "\x1e\"\\u0000\"\n", "jq: ignoring parse error: Invalid literal at line 3, column 0 (need RS to resync)\n", 0),
];

#[test]
fn test_a_bare_n_after_a_decoded_string_is_read_as_jq_reads_it_6513() -> Result<()> {
    for (stdin, stdout, stderr, code) in ROWS {
        let args = ["-c", "--seq", "."];
        let (out, err, status) = run_jq_full(&args, Some(stdin))?;
        assert_eq!(
            (out.as_str(), err.as_str(), status),
            (*stdout, *stderr, *code),
            "{args:?} {stdin:?}"
        );
    }
    Ok(())
}
