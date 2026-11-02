//! #5794: `walk(f)` on an object reads one output of `f` per member, as jq's `map_values(w)`
//! (`.[] |= w`) does, so a control `f` raises after its first output on a member is never
//! reached. The evaluator took the whole of `f` on each member and raised the error in place of
//! the output the parent would have produced first. Each row reads a file and then the same text
//! from stdin; the answers and exit statuses are captured from jq 1.7.1 (`/usr/bin/jq`).

use anyhow::Result;

use crate::run_jq_full;

/// `(input, filter, stdout, exit status)`.
const ROWS: &[(&str, &str, &str, i32)] = &[
    // The member keeps its first output; the error then comes from the parent's own `f`.
    ("{\"a\":1}", "walk((.,error(\"x\")))", "{\"a\":1}\n", 5),
    ("{\"a\":{}}", "walk((.,error(\"x\")))", "{\"a\":{}}\n", 5),
    ("{\"a\":1}", "walk(1,error(\"x\"))", "1\n", 5),
    ("{}", "walk((.,error(\"x\")))", "{}\n", 5),
    // An object whose members alone raise: the parent's `f` is clean, so nothing is raised.
    (
        "{\"a\":1}",
        "walk(if type==\"number\" then (.,error(\"x\")) else . end)",
        "{\"a\":1}\n",
        0,
    ),
    (
        "{\"a\":1}",
        "walk(if type==\"number\" then (.,error(\"x\"),.) else . end)",
        "{\"a\":1}\n",
        0,
    ),
    (
        "{\"a\":1}",
        "walk(if type==\"object\" then . else (.,error(\"x\")) end)",
        "{\"a\":1}\n",
        0,
    ),
    // An object inside an array: the array collects every output of its element, the nested
    // object's member reads one.
    (
        "[{\"a\":1}]",
        "walk(if type==\"number\" then (.,error(\"x\")) else . end)",
        "[{\"a\":1}]\n",
        0,
    ),
    // A `break` after the member's first output is never reached either.
    (
        "{\"a\":1}",
        "label $o | walk(if type==\"number\" then (.,break $o) else . end)",
        "{\"a\":1}\n",
        0,
    ),
    // Nothing is produced before the control: it is raised, or the member is deleted first.
    ("{\"a\":1}", "walk((error(\"x\"),.))", "", 5),
    (
        "{\"a\":1}",
        "walk(if type==\"number\" then empty,error(\"x\") else . end)",
        "",
        5,
    ),
    (
        "{\"a\":{}}",
        "walk(if type==\"number\" then empty,error(\"x\") else . end)",
        "{\"a\":{}}\n",
        0,
    ),
    // An array does not read one output per element: `map(w)` collects them all, so the error
    // is raised.
    ("[1]", "walk((.,error(\"x\")))", "", 5),
    ("[{\"a\":1}]", "walk((.,error(\"x\")))", "", 5),
    (
        "{\"a\":1,\"b\":[2,{\"c\":3}]}",
        "walk(if type==\"number\" then (.,error(\"x\")) else . end)",
        "",
        5,
    ),
];

#[test]
fn walk_reads_one_output_per_object_member_5794() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("in.json");
    for (input, filter, stdout, code) in ROWS {
        std::fs::write(&path, format!("{input}\n"))?;
        let (out, err, status) = run_jq_full(&["-c", filter, path.to_str().unwrap()], None)?;
        assert_eq!(
            (out.as_str(), status),
            (*stdout, *code),
            "{filter} over {input} (file): {err}"
        );
        let (out, err, status) = run_jq_full(&["-c", filter], Some(&format!("{input}\n")))?;
        assert_eq!(
            (out.as_str(), status),
            (*stdout, *code),
            "{filter} over {input} (stdin): {err}"
        );
    }
    Ok(())
}
