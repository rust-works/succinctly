//! #6631: the nesting ceiling counts containers, as jq's parser does: a document of 256 arrays
//! around a scalar is read (the scalar is not a 257th level) and a filter over it answers, where
//! the cursor walk's guards refused the scalar ({`tojson`, `walk`, `==`, `sort`, `del`, `flatten`,
//! `group_by`, `tostream`, ...}), so one document answered differently by route (#6636). Every
//! row was captured from jq 1.7.1; the 257th array is refused by the reader before any filter runs.

use anyhow::Result;

use crate::run_jq_full;

fn nested(depth: usize) -> String {
    format!("{}1{}", "[".repeat(depth), "]".repeat(depth))
}

#[test]
fn filters_over_a_document_256_containers_deep_answer_as_jq_does_6631() -> Result<()> {
    // (filter, stdout at 255 arrays, stdout at 256 arrays)
    for (filter, at_255, at_256) in [
        ("tojson | length", "511\n", "513\n"),
        ("tostring | length", "511\n", "513\n"),
        ("@json | length", "511\n", "513\n"),
        ("@text | length", "511\n", "513\n"),
        ("walk(.) | type", "\"array\"\n", "\"array\"\n"),
        (". == .", "true\n", "true\n"),
        ("[., .] | sort | length", "2\n", "2\n"),
        ("del(.[1]) | type", "\"array\"\n", "\"array\"\n"),
        ("flatten", "[1]\n", "[1]\n"),
        ("group_by(.) | length", "1\n", "1\n"),
        ("tostream | length", "2\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n", "2\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n1\n"),
        ("ltrimstr(\"a\") | type", "\"array\"\n", "\"array\"\n"),
        ("tojson | fromjson | type", "\"array\"\n", "\"array\"\n"),
        (". as $y | [path($y)] | length", "1\n", "1\n"),
        (".[0] = 1 | type", "\"array\"\n", "\"array\"\n"),
        ("del(.[0]) | length", "0\n", "0\n"),
        ("reduce . as $x (0; . + 1)", "1\n", "1\n"),
        ("[limit(1; .)] | length", "1\n", "1\n"),
        ("unique | length", "1\n", "1\n"),
        ("min | type", "\"array\"\n", "\"array\"\n"),
        ("add | type", "\"array\"\n", "\"array\"\n"),
        ("getpath([0]) | type", "\"array\"\n", "\"array\"\n"),
        ("setpath([0]; 1) | type", "\"array\"\n", "\"array\"\n"),
        ("to_entries | length", "1\n", "1\n"),
        ("map(.) | length", "1\n", "1\n"),
        ("tojson | test(\"1\")", "true\n", "true\n"),
    ] {
        for (depth, want) in [(255, at_255), (256, at_256)] {
            let (stdout, stderr, code) = run_jq_full(&["-c", filter], Some(&nested(depth)))?;
            assert_eq!(
                (stdout.as_str(), stderr.as_str(), code),
                (want, "", 0),
                "`{filter}` @ {depth}"
            );
        }
    }
    Ok(())
}

#[test]
fn output_flags_render_a_document_256_containers_deep_6631() -> Result<()> {
    let deep = nested(256);
    for flags in [&["-c", "-a"][..], &["-c", "-S"][..], &["-c", "-C"][..]] {
        let mut args = flags.to_vec();
        args.push(".");
        let (stdout, stderr, code) = run_jq_full(&args, Some(&deep))?;
        assert_eq!((code, stderr.as_str()), (0, ""), "{flags:?}");
        assert!(
            stdout.contains(&"[".repeat(256)) || flags.contains(&"-C"),
            "{flags:?}"
        );
    }
    Ok(())
}

#[test]
fn a_document_257_containers_deep_is_refused_by_the_reader_6631() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(&["-c", "tojson | length"], Some(&nested(257)))?;
    assert_eq!(
        (stdout.as_str(), stderr.as_str(), code),
        (
            "",
            "jq: parse error: Exceeds depth limit for parsing at line 1, column 257\n",
            5
        )
    );
    Ok(())
}
