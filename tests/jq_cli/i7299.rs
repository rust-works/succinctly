//! #7299: on x86_64 with AVX2 the JSON index is built by a branch-free table kernel that
//! skips 32-byte blocks lying wholly inside a string. [`input`] puts strings of every length
//! from 0 to 69, each with an escaped quote, an escaped backslash and a `\n` escape, at every
//! offset of a block, so escapes and skipped blocks meet at block edges. Every expected
//! answer below is what jq 1.7.1 prints for this input.

use anyhow::Result;

use crate::run_jq_full;

/// 200 compact objects; the strings grow, so their escapes slide across block edges.
fn input() -> String {
    let objects: Vec<String> = (0..200)
        .map(|i| {
            let s = format!(
                r#"{}\"{}\\\n"#,
                "a".repeat(i % 70),
                "b".repeat((i * 7) % 50)
            );
            format!(r#"{{"i":{i},"s":"{s}","v":[true,null,-15.5,{{}},[]]}}"#)
        })
        .collect();
    format!("[{}]", objects.join(","))
}

#[test]
fn compact_identity_reproduces_the_document() -> Result<()> {
    let input = input();
    let (out, err, code) = run_jq_full(&["-c", "."], Some(&input))?;
    assert_eq!((err.as_str(), code), ("", 0));
    assert_eq!(out, format!("{input}\n"));
    Ok(())
}

#[test]
fn every_string_and_value_is_found_after_the_escapes() -> Result<()> {
    let input = input();
    let rows: &[(&str, String)] = &[
        (
            "[.[] | .s | length] | add",
            // a-run + `"` + b-run + `\` + newline per object.
            format!(
                "{}\n",
                (0..200)
                    .map(|i| i % 70 + 1 + (i * 7) % 50 + 2)
                    .sum::<usize>()
            ),
        ),
        ("[.[] | .i] | add", "19900\n".to_string()),
        ("[.[] | .v[2]] | add", "-3100\n".to_string()),
        (
            ".[69].s",
            format!("\"{}\\\"{}\\\\\\n\"\n", "a".repeat(69), "b".repeat(33)),
        ),
        (
            ".[199] | [.i, .v[0], .v[3], .v[4]]",
            "[199,true,{},[]]\n".to_string(),
        ),
        (
            "[.[] | .s | test(\"^a*\\\"b*\\\\\\\\\\n$\")] | all",
            "true\n".to_string(),
        ),
    ];
    for (filter, expected) in rows {
        let (out, err, code) = run_jq_full(&["-c", filter], Some(&input))?;
        assert_eq!(
            (out.as_str(), err.as_str(), code),
            (expected.as_str(), "", 0),
            "{filter}"
        );
    }
    Ok(())
}
