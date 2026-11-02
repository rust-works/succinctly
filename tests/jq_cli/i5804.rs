//! #5804: a string that opens with part of the UTF-8 BOM and then departs from it is `Malformed
//! BOM` to jq's parser, which `fromjson` and `tonumber` hand their string to. Every character from
//! U+F000 to U+FFFF starts with `EF`, so U+FFFD (what `@base64d` puts for an invalid byte) reaches
//! it. Answers captured from jq 1.7.1 (`/usr/bin/jq`).

use anyhow::Result;

use crate::run_jq_null;

fn answer(filter: &str) -> Result<String> {
    let (stdout, code) = run_jq_null(filter, &["-c"])?;
    assert_eq!(code, 0, "{filter}");
    Ok(stdout)
}

#[test]
fn test_a_partial_bom_is_malformed_in_fromjson_and_tonumber_5804() -> Result<()> {
    for (filter, expected) in [
        (
            r#""\ufffd" | try fromjson catch ."#,
            "\"Malformed BOM (while parsing '\u{fffd}')\"",
        ),
        (
            r#""\ufffd" | try tonumber catch ."#,
            "\"Malformed BOM (while parsing '\u{fffd}')\"",
        ),
        (
            r#""\ufffde" | try fromjson catch ."#,
            "\"Malformed BOM (while parsing '\u{fffd}e')\"",
        ),
        (
            r#""\uf000" | try fromjson catch ."#,
            "\"Malformed BOM (while parsing '\u{f000}')\"",
        ),
        (
            r#""\uffff1" | try fromjson catch ."#,
            "\"Malformed BOM (while parsing '\u{ffff}1')\"",
        ),
        (
            r#""\ufec0" | try fromjson catch ."#,
            "\"Malformed BOM (while parsing '\u{fec0}')\"",
        ),
        // The quotation ends at a NUL, as for any other fault; `tonumber` takes trailing text.
        (
            r#""\ufffd\u0000x" | try fromjson catch ."#,
            "\"Malformed BOM (while parsing '\u{fffd}')\"",
        ),
        (
            r#""\ufffd 1" | try tonumber catch ."#,
            "\"Malformed BOM (while parsing '\u{fffd} 1')\"",
        ),
        // The fuzzer's finding: `@base64d` of `null` is two U+FFFD and a letter.
        (
            r#"(null | @base64d) | try fromjson catch . | startswith("Malformed BOM")"#,
            "true",
        ),
        // Not a partial BOM: a whole one is skipped, a second one or one behind other text is an
        // ordinary fault, and `EE 80 80` does not start `EF`.
        (r#""\ufeff1" | fromjson"#, "1"),
        (
            r#""\ufeff\ufffd" | try fromjson catch ."#,
            "\"Invalid numeric literal at EOF at line 1, column 3 (while parsing '\u{feff}\u{fffd}')\"",
        ),
        (
            r#""1\ufffd" | try fromjson catch ."#,
            "\"Invalid numeric literal at EOF at line 1, column 4 (while parsing '1\u{fffd}')\"",
        ),
        (
            r#""\ue0001" | try fromjson catch . | startswith("Malformed BOM")"#,
            "false",
        ),
    ] {
        assert_eq!(answer(filter)?, format!("{expected}\n"), "{filter}");
    }
    Ok(())
}
