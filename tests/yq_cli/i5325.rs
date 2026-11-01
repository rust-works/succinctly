//! #5325: yq's `fromjson` skips up to two leading UTF-8 byte-order marks before it reads the text:
//! yq's reader drops one and go-yaml another, so a third is a character of the text. Each
//! expectation is a live yq v4.53.3 capture (`yq -n -o=json -I0`). What the same decoder does with
//! trailing content, with text that is not JSON, and with whitespace alone is a wider gap that this
//! file does not pin.

use anyhow::Result;

use crate::run_yq_stdin;

const BOM: &str = "\u{feff}";

#[test]
fn test_fromjson_skips_one_or_two_leading_byte_order_marks_as_yq_does_5325() -> Result<()> {
    for (text, expected) in [
        ("[1]", "[1]\n"),
        ("{\\\"a\\\":1}", "{\"a\":1}\n"),
        ("1", "1\n"),
        ("\\\"x\\\"", "\"x\"\n"),
        ("null", "null\n"),
        ("true", "true\n"),
        ("  [1]", "[1]\n"),
        ("\\n[1]", "[1]\n"),
    ] {
        for boms in [1, 2] {
            let filter = format!("\"{}{text}\" | fromjson", BOM.repeat(boms));
            let (stdout, code) = run_yq_stdin(&filter, "", &["-n", "-o=json", "-I0"])?;
            assert_eq!(
                (stdout.as_str(), code),
                (expected, 0),
                "{boms} BOM(s) + {text}"
            );
        }
    }
    Ok(())
}

#[test]
fn test_fromjson_of_a_byte_order_mark_with_no_json_after_it_is_an_error_as_in_yq_5325() -> Result<()>
{
    for text in ["", "[1,"] {
        for boms in [1, 2] {
            let filter = format!("\"{}{text}\" | fromjson", BOM.repeat(boms));
            let (stdout, code) = run_yq_stdin(&filter, "", &["-n", "-o=json", "-I0"])?;
            assert_eq!((stdout.as_str(), code), ("", 1), "{boms} BOM(s) + {text:?}");
        }
    }
    Ok(())
}
