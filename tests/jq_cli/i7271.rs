//! #7271: an indented render of a document the canonical scan certifies is written out a chunk at a
//! time instead of being held whole first. The whole-value buffer existed so that a document that
//! turns out malformed late prints nothing, as jq 1.7.1 does (it parses a document fully before
//! printing it): a fault declines the certificate, so these rows pin that it still prints nothing,
//! that the documents before it in the same file still print, and that a certified document larger
//! than several chunks prints exactly what jq prints, for each indentation flag. Every expected text,
//! column and exit status was captured from `/usr/bin/jq` 1.7.1 over the same bytes.

use anyhow::Result;

use crate::run_jq_full;

/// 238,898 bytes of canonical compact JSON (raw UTF-8, canonical numbers, keys in order), so the
/// scan certifies it and its indented render is several 64 KiB chunks.
fn certified() -> String {
    let items: Vec<String> = (0..4000)
        .map(|i| format!(r#"{{"id":{i},"n":[1,-2.5,null,true],"s":"é😀\nxxxxxxxxxx"}}"#))
        .collect();
    format!(r#"{{"xs":[{}]}}"#, items.join(","))
}

/// jq's indented spelling of [`certified`]: `unit` repeated `width` times per level.
fn pretty(unit: &str) -> String {
    let i = |n: usize| unit.repeat(n);
    let items: Vec<String> = (0..4000)
        .map(|n| {
            format!(
                "{o}{{\n{f}\"id\": {n},\n{f}\"n\": [\n{e}1,\n{e}-2.5,\n{e}null,\n{e}true\n{f}],\n{f}\"s\": \"é😀\\nxxxxxxxxxx\"\n{o}}}",
                o = i(2),
                f = i(3),
                e = i(4),
            )
        })
        .collect();
    format!(
        "{{\n{}\"xs\": [\n{}\n{}]\n}}\n",
        i(1),
        items.join(",\n"),
        i(1)
    )
}

/// [`certified`] with the first digit of its last `id` replaced by `@`, 51 bytes from its end.
fn late_fault() -> String {
    let doc = certified();
    let at = doc.rfind(r#""id":"#).unwrap() + 5;
    format!("{}@{}", &doc[..at], &doc[at + 1..])
}

fn run(flags: &[&str], text: &str) -> Result<(String, String, i32)> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("in.json");
    std::fs::write(&path, text)?;
    let mut args = flags.to_vec();
    args.extend([".", path.to_str().expect("utf-8 temp path")]);
    run_jq_full(&args, None)
}

#[test]
fn test_a_certified_document_prints_what_jq_prints_7271() -> Result<()> {
    let doc = certified();
    assert_eq!(doc.len(), 238_898);
    for (flags, unit) in [
        (&[][..], "  "),
        (&["--indent", "1"][..], " "),
        (&["--tab"][..], "\t"),
        (&["--indent", "7"][..], "       "),
        (&["-S"][..], "  "),
    ] {
        let want = pretty(unit);
        assert_eq!(run(flags, &doc)?, (want, String::new(), 0), "{flags:?}");
    }
    Ok(())
}

#[test]
fn test_a_late_fault_still_prints_nothing_7271() -> Result<()> {
    let numeric = "jq: parse error: Invalid numeric literal at line 1, column 238848\n";
    let trailing = format!("{},]}}", &certified()[..238_896]);
    let element = "jq: parse error: Expected another array element at line 1, column 238898\n";
    for flags in [&[][..], &["--indent", "1"], &["--tab"], &["-S"]] {
        let got = run(flags, &late_fault())?;
        assert_eq!(got, (String::new(), numeric.to_string(), 5), "{flags:?}");
        let got = run(flags, &trailing)?;
        assert_eq!(got, (String::new(), element.to_string(), 5), "{flags:?}");
    }
    Ok(())
}

#[test]
fn test_documents_before_a_late_fault_still_print_7271() -> Result<()> {
    let text = format!("{{\"a\":1}}\n[2,3]\n{}\n{{\"c\":3}}\n", late_fault());
    let stderr = "jq: parse error: Invalid numeric literal at line 3, column 238848\n";
    for (flags, stdout) in [
        (&[][..], "{\n  \"a\": 1\n}\n[\n  2,\n  3\n]\n"),
        (&["--tab"][..], "{\n\t\"a\": 1\n}\n[\n\t2,\n\t3\n]\n"),
    ] {
        let got = run(flags, &text)?;
        assert_eq!(
            got,
            (stdout.to_string(), stderr.to_string(), 5),
            "{flags:?}"
        );
    }
    Ok(())
}
