//! #5305: a jq-mode string interpolation of plain navigation reads its `\(...)` slots at the
//! cursor, instead of bridging the ambient value into an owned copy first. Rows were captured
//! from jq 1.7.1 (`/usr/bin/jq`) unless a comment says otherwise.

use anyhow::Result;

use crate::{assert_jq_raises, run_jq_full, run_jq_stdin};

const DOC: &str = r#"{"a":[1,2],"b":[3,4],"s":"x","n":null,"o":{"k":[true]},"u":"hé\n\"q\""}"#;

#[test]
fn test_interpolation_over_a_document_matches_jq_5305() -> Result<()> {
    for (filter, want) in [
        // Scalars, null and a container rendered into the string.
        (r#""\(.s)-\(.n)-\(.o)""#, "\"x-null-{\\\"k\\\":[true]}\"\n"),
        (r#""\(.u)""#, "\"h\u{e9}\\n\\\"q\\\"\"\n"),
        // The first slot varies fastest, the last slowest.
        (
            r#""\(.a[])-\(.b[])""#,
            "\"1-3\"\n\"2-3\"\n\"1-4\"\n\"2-4\"\n",
        ),
        (
            r#"[("\(.a[])-\(.b[])-\(.a[])")]"#,
            "[\"1-3-1\",\"2-3-1\",\"1-4-1\",\"2-4-1\",\"1-3-2\",\"2-3-2\",\"1-4-2\",\"2-4-2\"]\n",
        ),
        (
            r#"[.a[] as $x | "\($x)/\(.b[])"]"#,
            "[\"1/3\",\"1/4\",\"2/3\",\"2/4\"]\n",
        ),
        (r#"[.a[] | "<\(.)>"]"#, "[\"<1>\",\"<2>\"]\n"),
        (r#"[.a[] | "\(.)\(.)" | length]"#, "[2,2]\n"),
        // Demand: a consumer that stops early stops the fan-out.
        (
            r#"[limit(3; "\(.a[])-\(.b[])")]"#,
            "[\"1-3\",\"2-3\",\"1-4\"]\n",
        ),
        (r#"first("\(.a[])-\(.b[])")"#, "\"1-3\"\n"),
        (r#"isempty("\(.a[])")"#, "false\n"),
        (
            r#"[label $f | "\(.a[] | if . > 1 then break $f else . end)-\(.b[])"]"#,
            "[\"1-3\"]\n",
        ),
        // A slot that yields nothing yields nothing.
        (r#""\(empty)x""#, ""),
        // A slot's error escapes after the combinations already built.
        (
            r#"[try "\(.a[], error("x"))" catch .]"#,
            "[\"1\",\"2\",\"x\"]\n",
        ),
        (
            r#"[try ("\(.a[])\(1, error("late"))") catch .]"#,
            "[\"11\",\"21\",\"late\"]\n",
        ),
        (r#"[("\(.s[]?)" | length)]"#, "[]\n"),
        // An earlier slot's error escapes through the slots around it, streamed and collected.
        (
            r#"[try "\(.s.x)-\(.b[])" catch .]"#,
            "[\"Cannot index string with string \\\"x\\\"\"]\n",
        ),
        (
            r#"[.b[] | try "\(.s.x)-\(.)" catch "c"]"#,
            "[\"c\",\"c\"]\n",
        ),
        // The collected and object-value forms, and the per-element builtins that take them.
        (r#"[.a[] | {k: "\(.)"}]"#, "[{\"k\":\"1\"},{\"k\":\"2\"}]\n"),
        (r#".a | map("<\(.)>")"#, "[\"<1>\",\"<2>\"]\n"),
        (r#".a | map("\(.)\(.)")"#, "[\"11\",\"22\"]\n"),
        (
            r#".a | to_entries | map("\(.key)=\(.value)")"#,
            "[\"0=1\",\"1=2\"]\n",
        ),
        (r#""\(.a[0])\(.b[1])\(.s)\("lit")""#, "\"14xlit\"\n"),
        (r#""\(.a[-1])""#, "\"2\"\n"),
        // A slot that is not navigation keeps the owned route and its answers.
        (
            r#"def r(n): if n == 0 then "x" else "\(r(n-1))" end; r(1500) | length"#,
            "1\n",
        ),
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-c", filter], Some(DOC))?;
        assert_eq!((stdout.as_str(), code), (want, 0), "{filter}: {stderr}");
    }
    Ok(())
}

/// A filter validates only what it reads (the rule of #2103, #3476 and #5308): the interpolation
/// reads `.name` and nothing else, as `.name` and `[.name]` already do. It used to bridge the
/// whole ambient object into an owned copy first, which decodes every sibling and so raised on
/// the one malformed beside it. Both routes are pinned: an interpolation behind `.[]` is
/// streamed, one inside `[...]` is collected. jq itself rejects this document at parse time with exit 5
/// (`docs/compliance/jq/limitations.md`); `--validate` rejects it here too.
#[test]
fn test_interpolation_reads_only_the_fields_its_slots_navigate_5305() -> Result<()> {
    for sibling in [r#""bad":tru"#, r#""bad":1e"#, r#""bad":"\x""#] {
        let input = format!(r#"{{"name":"a",{sibling}}}"#);
        for (filter, want) in [
            // Streamed.
            (r#""\(.name)""#, "\"a\"\n"),
            // Collected.
            (r#"["\(.name)"]"#, "[\"a\"]\n"),
            (r#"[("\(.name)", "z")]"#, "[\"a\",\"z\"]\n"),
            // The same read through a field access, the control.
            (".name", "\"a\"\n"),
        ] {
            let (out, code) = run_jq_stdin(filter, &input, &["-c"])?;
            assert_eq!((out.as_str(), code), (want, 0), "{filter} over {input}");
        }
        // Behind a pipe the interpolation is a stage of a stream, which the sink route drives.
        let rows = format!("[{input}]");
        for (filter, want) in [
            (r#".[] | "\(.name)""#, "\"a\"\n"),
            (r#"first(.[] | "\(.name)")"#, "\"a\"\n"),
            (r#".[] | "\(.name)\(.name)""#, "\"aa\"\n"),
        ] {
            let (out, code) = run_jq_stdin(filter, &rows, &["-c"])?;
            assert_eq!((out.as_str(), code), (want, 0), "{filter} over {rows}");
        }
    }
    Ok(())
}

/// The rule's other half: a slot that reads the malformed field still raises, and says why.
#[test]
fn test_interpolation_raises_when_a_slot_reads_the_malformed_field_5305() -> Result<()> {
    for filter in [
        r#""\(.bad)""#,
        r#"["\(.bad)"]"#,
        r#""\(.)""#,
        r#".. | "\(.)""#,
    ] {
        assert_jq_raises(
            "5305",
            filter,
            &["-c"],
            filter,
            r#"{"name":"a","bad":tru}"#,
            5,
            "invalid boolean",
        );
    }
    Ok(())
}

/// A snapshot of the current limitation, not a contract: extending the cursor path to more slot
/// kinds should update these rows. An interpolation with a slot that is not plain navigation keeps
/// the owned copy, so it still
/// decodes the whole ambient value first (the verdict of a string depends on how it is spelled,
/// `docs/compliance/jq/limitations.md`), and a slot that reads a position keeps its answer.
#[test]
fn test_interpolation_with_a_call_slot_keeps_the_copy_5305() -> Result<()> {
    let doc = r#"{"name":"a","bad":tru}"#;
    for filter in [
        r#""\(.name | ascii_upcase)""#,
        r#"["\(.name | ascii_upcase)"]"#,
        r#""\(.name)\(.name | length)""#,
    ] {
        assert_jq_raises("5305", filter, &["-c"], filter, doc, 5, "invalid boolean");
    }
    for (filter, want) in [
        // Captured from the pre-#5305 binary: the `key` stubs are unchanged.
        (r#".a | "\(map_values(key))""#, "\"{\\\"b\\\":null}\"\n"),
        (r#""\(.a | map(key))""#, "\"[\\\"b\\\"]\"\n"),
    ] {
        let (out, code) = run_jq_stdin(filter, r#"{"a":{"b":1}}"#, &["-c"])?;
        assert_eq!((out.as_str(), code), (want, 0), "{filter}");
    }
    Ok(())
}

/// `inputs` in a slot drains the queue lazily, so a later `[inputs]` still sees the rest. The
/// cursor path has no lazy `inputs`; the slot keeps the owned route (captured from the pre-#5305
/// binary: `line` is a succinctly builtin, so jq itself cannot run this program).
#[test]
fn test_interpolation_of_inputs_leaves_the_rest_of_the_queue_5305() -> Result<()> {
    let (out, stderr, code) = run_jq_full(
        &[
            "-cn",
            r#"(label $f | "\(inputs)" | ., break $f), line, [inputs]"#,
        ],
        Some("1\n2\n3\n"),
    )?;
    assert_eq!(
        (out.as_str(), code),
        ("\"1\"\n1\n[2,3]\n", 0),
        "stderr: {stderr}"
    );
    Ok(())
}

/// The cursor path covers at most 64 parts; a longer interpolation keeps the owned copy.
#[test]
fn test_interpolation_cursor_path_stops_after_sixty_four_parts_5305() -> Result<()> {
    let doc = r#"{"name":"a","bad":tru}"#;
    let program = |slots: usize| format!("\"{}\"", r"\(.name)".repeat(slots));
    let (out, code) = run_jq_stdin(&program(64), doc, &["-c"])?;
    assert_eq!(
        (out.as_str(), code),
        (format!("\"{}\"\n", "a".repeat(64)).as_str(), 0)
    );
    assert_jq_raises(
        "5305",
        "65 parts",
        &["-c"],
        &program(65),
        doc,
        5,
        "invalid boolean",
    );
    Ok(())
}
