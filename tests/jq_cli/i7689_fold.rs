//! Found by the wider regex fuzz of #7689: under `i` (the flag or an inline `(?i)`), Oniguruma
//! negates a negated property or POSIX bracket inside a class (`\P{Lu}`, `\p{^Lu}`, `[:^upper:]`)
//! before it case-folds the class, so `"A" | test("[\\P{Lu}]"; "i")` is `true` in jq; the `regex`
//! crate folded the property first and then negated it, which left out every cased letter. Such an
//! item is now written as its complement's ranges. The rows are a sample of a 4,830-row matrix
//! (eight properties and seven POSIX names in nine forms, 14 strings, five flag spellings), each
//! captured from jq 1.7.1 (`/usr/bin/jq`) and the same in jq 1.8.2: 35 of 42 fail before the
//! change, the rest are controls.

use anyhow::Result;

use crate::run_jq_full;

/// Each row's matches: offset, length, text, or the caught error.
const PROGRAM: &str = r#". as $r | $r.s | try ([if $r.fl == null then match($r.re) else match($r.re; $r.fl) end | [.offset, .length, .string]]) catch "ERR: \(.)""#;

/// `(row, jq 1.7.1's output)`.
const ROWS: &[(&str, &str)] = &[
    (r#"{"re":"(?i)[^\\P{Ll}]","fl":null,"s":"ǅ"}"#, "[]"),
    (
        r#"{"re":"(?i:[\\P{Lowercase}\\d]+)","fl":null,"s":"ς"}"#,
        r#"[[0,1,"ς"]]"#,
    ),
    (
        r#"{"re":"[\\P{Ll}\\d]+","fl":"gi","s":"Σ"}"#,
        r#"[[0,1,"Σ"]]"#,
    ),
    (r#"{"re":"[^\\p{^Lt}x]","fl":"gi","s":"ǅ"}"#, "[]"),
    (
        r#"{"re":"(?i:[\\P{Lowercase}])","fl":null,"s":"ǅ"}"#,
        r#"[[0,1,"ǅ"]]"#,
    ),
    (
        r#"{"re":"[\\p{^Lowercase}]","fl":"gi","s":"ß"}"#,
        r#"[[0,1,"ß"]]"#,
    ),
    (r#"{"re":"(?i)[^[:^lower:]]","fl":null,"s":"Σ"}"#, "[]"),
    (
        r#"{"re":"[\\P{Lowercase}]","fl":"gi","s":"σ"}"#,
        r#"[[0,1,"σ"]]"#,
    ),
    (r#"{"re":"[^\\P{Lowercase}]","fl":"gi","s":"A"}"#, "[]"),
    (
        r#"{"re":"[[:^upper:]1]+","fl":"gi","s":"Σ"}"#,
        r#"[[0,1,"Σ"]]"#,
    ),
    (
        r#"{"re":"[\\P{Lowercase}\\d]+","fl":"i","s":"ß"}"#,
        r#"[[0,1,"ß"]]"#,
    ),
    (r#"{"re":"(?i:[^\\P{Lowercase}])","fl":null,"s":"é"}"#, "[]"),
    (
        r#"{"re":"(?i:[b\\P{Upper}])","fl":null,"s":"É"}"#,
        r#"[[0,1,"É"]]"#,
    ),
    (
        r#"{"re":"(?i)[^\\p{^Lowercase}x]","fl":null,"s":"ß"}"#,
        "[]",
    ),
    (
        r#"{"re":"(?i:[\\P{Lu}\\d]+)","fl":null,"s":"é"}"#,
        r#"[[0,1,"é"]]"#,
    ),
    (
        r#"{"re":"[[:^upper:]]","fl":"i","s":"A"}"#,
        r#"[[0,1,"A"]]"#,
    ),
    (
        r#"{"re":"(?i)[\\P{Lu}\\d]+","fl":null,"s":"ß"}"#,
        r#"[[0,1,"ß"]]"#,
    ),
    (r#"{"re":"(?i:[^\\P{Upper}])","fl":null,"s":"Σ"}"#, "[]"),
    (
        r#"{"re":"(?i)[\\p{^Lu}]","fl":null,"s":"A"}"#,
        r#"[[0,1,"A"]]"#,
    ),
    (r#"{"re":"[\\P{Lu}]","fl":"gi","s":"ǅ"}"#, r#"[[0,1,"ǅ"]]"#),
    (
        r#"{"re":"(?i)[\\p{^Lowercase}]","fl":null,"s":"ß"}"#,
        r#"[[0,1,"ß"]]"#,
    ),
    (
        r#"{"re":"[[:^lower:]1]+","fl":"i","s":"É"}"#,
        r#"[[0,1,"É"]]"#,
    ),
    (
        r#"{"re":"(?i)[\\P{Upper}\\d]+","fl":null,"s":"ß"}"#,
        r#"[[0,1,"ß"]]"#,
    ),
    (r#"{"re":"[^\\p{^Upper}x]","fl":"gi","s":"ǅ"}"#, "[]"),
    (r#"{"re":"[b\\P{Ll}]","fl":"gi","s":"ǅ"}"#, r#"[[0,1,"ǅ"]]"#),
    (r#"{"re":"[^\\P{Upper}]","fl":"i","s":"ß"}"#, "[]"),
    (r#"{"re":"(?i:[^[:^lower:]])","fl":null,"s":"ς"}"#, "[]"),
    (
        r#"{"re":"(?i:[[:^upper:]])","fl":null,"s":"σ"}"#,
        r#"[[0,1,"σ"]]"#,
    ),
    (
        r#"{"re":"[b\\P{Lowercase}]","fl":"gi","s":"a"}"#,
        r#"[[0,1,"a"]]"#,
    ),
    (r#"{"re":"(?i)[^[:^upper:]]","fl":null,"s":"É"}"#, "[]"),
    (
        r#"{"re":"[[:^lower:]]","fl":"gi","s":"é"}"#,
        r#"[[0,1,"é"]]"#,
    ),
    (
        r#"{"re":"(?i)[[:^lower:]1]+","fl":null,"s":"σ"}"#,
        r#"[[0,1,"σ"]]"#,
    ),
    (
        r#"{"re":"(?i)[\\p{^Upper}]","fl":null,"s":"É"}"#,
        r#"[[0,1,"É"]]"#,
    ),
    (
        r#"{"re":"(?i:[^\\p{^Lowercase}x])","fl":null,"s":"ς"}"#,
        "[]",
    ),
    (
        r#"{"re":"[\\P{Lu}\\d]+","fl":"i","s":"A"}"#,
        r#"[[0,1,"A"]]"#,
    ),
    (
        r#"{"re":"[^\\p{^Greek}x]","fl":"gi","s":"Σ"}"#,
        r#"[[0,1,"Σ"]]"#,
    ),
    (r#"{"re":"[b\\P{Lu}]","fl":null,"s":"σ"}"#, r#"[[0,1,"σ"]]"#),
    (r#"{"re":"[[:^alpha:]1]+","fl":"gi","s":"A"}"#, "[]"),
    (r#"{"re":"[b\\P{L}]","fl":"i","s":"ı"}"#, "[]"),
    (r#"{"re":"[^\\p{^L}x]","fl":"i","s":"ı"}"#, r#"[[0,1,"ı"]]"#),
    (r#"{"re":"(?i)[[:^alpha:]1]+","fl":null,"s":"Σ"}"#, "[]"),
    (r#"{"re":"[\\p{^L}]","fl":"gi","s":"1"}"#, r#"[[0,1,"1"]]"#),
];

#[test]
fn a_negated_property_in_a_class_is_folded_after_it_is_negated_7689() -> Result<()> {
    let input: String = ROWS.iter().map(|(row, _)| format!("{row}\n")).collect();
    let (stdout, stderr, code) = run_jq_full(&["-c", PROGRAM], Some(&input))?;
    assert_eq!((code, stderr.as_str()), (0, ""));
    let got: Vec<&str> = stdout.lines().collect();
    assert_eq!(got.len(), ROWS.len(), "{stdout}");
    for ((row, expected), got) in ROWS.iter().zip(got) {
        assert_eq!(got, *expected, "{row}");
    }
    Ok(())
}

/// The rows of the report: jq 1.7.1 and 1.8.2 print `true`, `true`, `false`.
#[test]
fn the_reported_rows_answer_as_jq_7689() -> Result<()> {
    for (program, want) in [
        (r#""A" | test("[\\P{Lu}]"; "i")"#, "true\n"),
        (r#""a" | test("[[:^upper:]]"; "i")"#, "true\n"),
        (r#""a" | test("[^\\P{Lu}]"; "i")"#, "false\n"),
    ] {
        let out = run_jq_full(&["-nc", program], None)?;
        assert_eq!(out, (want.to_string(), String::new(), 0), "{program}");
    }
    Ok(())
}
