//! #5305: a jq-mode string interpolation reads its `\(...)` slots at the cursor, instead of
//! bridging the ambient value into an owned copy first. Rows were captured from jq 1.7.1
//! (`/usr/bin/jq`) unless a comment says otherwise.

use anyhow::Result;

use crate::{run_jq_full, run_jq_stdin};

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
            r#"[try ("\(.a[])\(error("late"))") catch .]"#,
            "[\"late\"]\n",
        ),
        (r#"[("\(.a[]?)" | length)]"#, "[1,1]\n"),
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-c", filter], Some(DOC))?;
        assert_eq!((stdout.as_str(), code), (want, 0), "{filter}: {stderr}");
    }
    Ok(())
}

/// A filter validates only what it reads (the rule of #2103, #3476 and #5308): the interpolation
/// reads `.name` and nothing else, as `.name` and `[.name]` already do. It used to bridge the
/// whole ambient object into an owned copy first, which decodes every sibling and so raised on
/// the one malformed beside it. The two routes are pinned separately: a lone interpolation is
/// collected, one behind a pipe is streamed. jq itself rejects this document at parse time with exit 2
/// (`docs/compliance/jq/limitations.md`); `--validate` rejects it here too.
#[test]
fn test_interpolation_reads_only_the_fields_its_slots_navigate_5305() -> Result<()> {
    for sibling in [r#""bad":tru"#, r#""bad":1e"#, r#""bad":"\x""#] {
        let input = format!(r#"{{"name":"a",{sibling}}}"#);
        for (filter, want) in [
            // The sink route (`eval_each_generic`).
            (r#""\(.name)""#, "\"a\"\n"),
            // The collect route (`eval_single`).
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

/// The rule's other half: a slot that reads the malformed field still raises.
#[test]
fn test_interpolation_raises_when_a_slot_reads_the_malformed_field_5305() -> Result<()> {
    for filter in [r#""\(.bad)""#, r#"["\(.bad)"]"#, r#""\(.)""#] {
        let (out, stderr, code) = run_jq_full(&["-c", filter], Some(r#"{"name":"a","bad":tru}"#))?;
        assert_eq!((out.as_str(), code), ("", 5), "{filter}: {stderr}");
    }
    Ok(())
}
