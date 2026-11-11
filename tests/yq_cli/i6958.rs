//! #6958: an underscored `!!float` (`!!float 1_0.5`, `1_000`, `1_0e1_0`) keeps its text, as an underscored
//! `!!int` does: `tostring`, YAML output and a copy print the text and carry the `!!float` tag where the text
//! alone would read as an integer, and the JSON writers print the number as a float (`1000.0`, not `1000`).
//! Every row was captured from yq v4.53.3. The forms yq cannot encode (`1__0.5`, `1_.5`, `0x1_0`) are
//! recorded in `docs/compliance/yq/limitations.md` (#6938).

use anyhow::Result;

use crate::run_yq_stdin;

fn assert_yq_rows(rows: &[(&str, &[&str], &str, &str)]) -> Result<()> {
    for (doc, flags, filter, expected) in rows {
        let (out, code) = run_yq_stdin(filter, doc, flags)?;
        assert_eq!(code, 0, "{flags:?} {filter} over {doc:?}");
        assert_eq!(&out, expected, "{flags:?} {filter} over {doc:?}");
    }
    Ok(())
}

#[test]
fn test_an_underscored_float_keeps_its_text_6958() -> Result<()> {
    assert_yq_rows(&[
        (
            "a: !!float 1_0.5\n",
            &["-o=json", "-I0"],
            ".a | tostring",
            "\"1_0.5\"\n",
        ),
        ("a: !!float 1_0.5\n", &[], ".a | tostring", "1_0.5\n"),
        (
            "a: !!float 1_0.5\n",
            &["-o=json", "-I0"],
            ".a * 2",
            "21.0\n",
        ),
        ("a: !!float 1_0.5\n", &[], ".a * 2", "21\n"),
        (
            "a: !!float 1_0.5\n",
            &["-o=json", "-I0"],
            ".b = .a",
            "{\"a\":10.5,\"b\":10.5}\n",
        ),
        (
            "a: !!float 1_0.5\n",
            &[],
            ".b = .a",
            "a: !!float 1_0.5\nb: !!float 1_0.5\n",
        ),
        (
            "a: !!float 1_0.5\n",
            &["-o=json", "-I0"],
            "with_entries(.)",
            "{\"a\":10.5}\n",
        ),
        (
            "a: !!float 1_0.5\n",
            &[],
            "with_entries(.)",
            "a: !!float 1_0.5\n",
        ),
        (
            "a: !!float 1_0.5\n",
            &["-o=json", "-I0"],
            "[.a] | tojson",
            "\"[\\n  10.5\\n]\\n\"\n",
        ),
        (
            "a: !!float 1_0.5\n",
            &[],
            "[.a] | tojson",
            "[\n  10.5\n]\n\n",
        ),
        (
            "a: !!float 1_000\n",
            &["-o=json", "-I0"],
            ".a | tostring",
            "\"1_000\"\n",
        ),
        ("a: !!float 1_000\n", &[], ".a | tostring", "1_000\n"),
        (
            "a: !!float 1_000\n",
            &["-o=json", "-I0"],
            ".a * 2",
            "2000.0\n",
        ),
        ("a: !!float 1_000\n", &[], ".a * 2", "2000\n"),
        (
            "a: !!float 1_000\n",
            &["-o=json", "-I0"],
            ".b = .a",
            "{\"a\":1000.0,\"b\":1000.0}\n",
        ),
        (
            "a: !!float 1_000\n",
            &[],
            ".b = .a",
            "a: !!float 1_000\nb: !!float 1_000\n",
        ),
        (
            "a: !!float 1_000\n",
            &["-o=json", "-I0"],
            "with_entries(.)",
            "{\"a\":1000.0}\n",
        ),
        (
            "a: !!float 1_000\n",
            &[],
            "with_entries(.)",
            "a: !!float 1_000\n",
        ),
        (
            "a: !!float 1_000\n",
            &["-o=json", "-I0"],
            "[.a] | tojson",
            "\"[\\n  1000.0\\n]\\n\"\n",
        ),
        (
            "a: !!float 1_000\n",
            &[],
            "[.a] | tojson",
            "[\n  1000.0\n]\n\n",
        ),
        (
            "a: !!float 0_8\n",
            &["-o=json", "-I0"],
            ".a | tostring",
            "\"0_8\"\n",
        ),
        ("a: !!float 0_8\n", &[], ".a | tostring", "0_8\n"),
        ("a: !!float 0_8\n", &["-o=json", "-I0"], ".a * 2", "16.0\n"),
        ("a: !!float 0_8\n", &[], ".a * 2", "16\n"),
        (
            "a: !!float 0_8\n",
            &["-o=json", "-I0"],
            ".b = .a",
            "{\"a\":8.0,\"b\":8.0}\n",
        ),
        (
            "a: !!float 0_8\n",
            &[],
            ".b = .a",
            "a: !!float 0_8\nb: !!float 0_8\n",
        ),
        (
            "a: !!float 0_8\n",
            &["-o=json", "-I0"],
            "with_entries(.)",
            "{\"a\":8.0}\n",
        ),
        (
            "a: !!float 0_8\n",
            &[],
            "with_entries(.)",
            "a: !!float 0_8\n",
        ),
        (
            "a: !!float 0_8\n",
            &["-o=json", "-I0"],
            "[.a] | tojson",
            "\"[\\n  8.0\\n]\\n\"\n",
        ),
        ("a: !!float 0_8\n", &[], "[.a] | tojson", "[\n  8.0\n]\n\n"),
        (
            "a: !!float 1_0e1_0\n",
            &["-o=json", "-I0"],
            ".a | tostring",
            "\"1_0e1_0\"\n",
        ),
        ("a: !!float 1_0e1_0\n", &[], ".a | tostring", "1_0e1_0\n"),
        (
            "a: !!float 1_0e1_0\n",
            &["-o=json", "-I0"],
            ".a * 2",
            "2e+11\n",
        ),
        ("a: !!float 1_0e1_0\n", &[], ".a * 2", "2e+11\n"),
        (
            "a: !!float 1_0e1_0\n",
            &["-o=json", "-I0"],
            ".b = .a",
            "{\"a\":100000000000.0,\"b\":100000000000.0}\n",
        ),
        (
            "a: !!float 1_0e1_0\n",
            &[],
            ".b = .a",
            "a: !!float 1_0e1_0\nb: !!float 1_0e1_0\n",
        ),
        (
            "a: !!float 1_0e1_0\n",
            &["-o=json", "-I0"],
            "with_entries(.)",
            "{\"a\":100000000000.0}\n",
        ),
        (
            "a: !!float 1_0e1_0\n",
            &[],
            "with_entries(.)",
            "a: !!float 1_0e1_0\n",
        ),
        (
            "a: !!float 1_0e1_0\n",
            &["-o=json", "-I0"],
            "[.a] | tojson",
            "\"[\\n  100000000000.0\\n]\\n\"\n",
        ),
        (
            "a: !!float 1_0e1_0\n",
            &[],
            "[.a] | tojson",
            "[\n  100000000000.0\n]\n\n",
        ),
        (
            "a: !!float +1_0.5\n",
            &["-o=json", "-I0"],
            ".a | tostring",
            "\"+1_0.5\"\n",
        ),
        ("a: !!float +1_0.5\n", &[], ".a | tostring", "+1_0.5\n"),
        (
            "a: !!float +1_0.5\n",
            &["-o=json", "-I0"],
            ".a * 2",
            "21.0\n",
        ),
        ("a: !!float +1_0.5\n", &[], ".a * 2", "21\n"),
        (
            "a: !!float +1_0.5\n",
            &["-o=json", "-I0"],
            ".b = .a",
            "{\"a\":10.5,\"b\":10.5}\n",
        ),
        (
            "a: !!float +1_0.5\n",
            &[],
            ".b = .a",
            "a: !!float +1_0.5\nb: !!float +1_0.5\n",
        ),
        (
            "a: !!float +1_0.5\n",
            &["-o=json", "-I0"],
            "with_entries(.)",
            "{\"a\":10.5}\n",
        ),
        (
            "a: !!float +1_0.5\n",
            &[],
            "with_entries(.)",
            "a: !!float +1_0.5\n",
        ),
        (
            "a: !!float +1_0.5\n",
            &["-o=json", "-I0"],
            "[.a] | tojson",
            "\"[\\n  10.5\\n]\\n\"\n",
        ),
        (
            "a: !!float +1_0.5\n",
            &[],
            "[.a] | tojson",
            "[\n  10.5\n]\n\n",
        ),
        (
            "a: !!float -1_0.5\n",
            &["-o=json", "-I0"],
            ".a | tostring",
            "\"-1_0.5\"\n",
        ),
        ("a: !!float -1_0.5\n", &[], ".a | tostring", "-1_0.5\n"),
        (
            "a: !!float -1_0.5\n",
            &["-o=json", "-I0"],
            ".a * 2",
            "-21.0\n",
        ),
        ("a: !!float -1_0.5\n", &[], ".a * 2", "-21\n"),
        (
            "a: !!float -1_0.5\n",
            &["-o=json", "-I0"],
            ".b = .a",
            "{\"a\":-10.5,\"b\":-10.5}\n",
        ),
        (
            "a: !!float -1_0.5\n",
            &[],
            ".b = .a",
            "a: !!float -1_0.5\nb: !!float -1_0.5\n",
        ),
        (
            "a: !!float -1_0.5\n",
            &["-o=json", "-I0"],
            "with_entries(.)",
            "{\"a\":-10.5}\n",
        ),
        (
            "a: !!float -1_0.5\n",
            &[],
            "with_entries(.)",
            "a: !!float -1_0.5\n",
        ),
        (
            "a: !!float -1_0.5\n",
            &["-o=json", "-I0"],
            "[.a] | tojson",
            "\"[\\n  -10.5\\n]\\n\"\n",
        ),
        (
            "a: !!float -1_0.5\n",
            &[],
            "[.a] | tojson",
            "[\n  -10.5\n]\n\n",
        ),
        (
            "a: !!float 1_0.\n",
            &["-o=json", "-I0"],
            ".a | tostring",
            "\"1_0.\"\n",
        ),
        ("a: !!float 1_0.\n", &[], ".a | tostring", "1_0.\n"),
        ("a: !!float 1_0.\n", &["-o=json", "-I0"], ".a * 2", "20.0\n"),
        ("a: !!float 1_0.\n", &[], ".a * 2", "20\n"),
        (
            "a: !!float 1_0.\n",
            &["-o=json", "-I0"],
            ".b = .a",
            "{\"a\":10.0,\"b\":10.0}\n",
        ),
        (
            "a: !!float 1_0.\n",
            &[],
            ".b = .a",
            "a: !!float 1_0.\nb: !!float 1_0.\n",
        ),
        (
            "a: !!float 1_0.\n",
            &["-o=json", "-I0"],
            "with_entries(.)",
            "{\"a\":10.0}\n",
        ),
        (
            "a: !!float 1_0.\n",
            &[],
            "with_entries(.)",
            "a: !!float 1_0.\n",
        ),
        (
            "a: !!float 1_0.\n",
            &["-o=json", "-I0"],
            "[.a] | tojson",
            "\"[\\n  10.0\\n]\\n\"\n",
        ),
        (
            "a: !!float 1_0.\n",
            &[],
            "[.a] | tojson",
            "[\n  10.0\n]\n\n",
        ),
        (
            "a: !!float .5_5\n",
            &["-o=json", "-I0"],
            ".a | tostring",
            "\".5_5\"\n",
        ),
        ("a: !!float .5_5\n", &[], ".a | tostring", ".5_5\n"),
        ("a: !!float .5_5\n", &["-o=json", "-I0"], ".a * 2", "1.1\n"),
        ("a: !!float .5_5\n", &[], ".a * 2", "1.1\n"),
        (
            "a: !!float .5_5\n",
            &["-o=json", "-I0"],
            ".b = .a",
            "{\"a\":0.55,\"b\":0.55}\n",
        ),
        (
            "a: !!float .5_5\n",
            &[],
            ".b = .a",
            "a: !!float .5_5\nb: !!float .5_5\n",
        ),
        (
            "a: !!float .5_5\n",
            &["-o=json", "-I0"],
            "with_entries(.)",
            "{\"a\":0.55}\n",
        ),
        (
            "a: !!float .5_5\n",
            &[],
            "with_entries(.)",
            "a: !!float .5_5\n",
        ),
        (
            "a: !!float .5_5\n",
            &["-o=json", "-I0"],
            "[.a] | tojson",
            "\"[\\n  0.55\\n]\\n\"\n",
        ),
        (
            "a: !!float .5_5\n",
            &[],
            "[.a] | tojson",
            "[\n  0.55\n]\n\n",
        ),
        (
            "a: !!float 00_1.5\n",
            &["-o=json", "-I0"],
            ".a | tostring",
            "\"00_1.5\"\n",
        ),
        ("a: !!float 00_1.5\n", &[], ".a | tostring", "00_1.5\n"),
        (
            "a: !!float 00_1.5\n",
            &["-o=json", "-I0"],
            ".a * 2",
            "3.0\n",
        ),
        ("a: !!float 00_1.5\n", &[], ".a * 2", "3\n"),
        (
            "a: !!float 00_1.5\n",
            &["-o=json", "-I0"],
            ".b = .a",
            "{\"a\":1.5,\"b\":1.5}\n",
        ),
        (
            "a: !!float 00_1.5\n",
            &[],
            ".b = .a",
            "a: !!float 00_1.5\nb: !!float 00_1.5\n",
        ),
        (
            "a: !!float 00_1.5\n",
            &["-o=json", "-I0"],
            "with_entries(.)",
            "{\"a\":1.5}\n",
        ),
        (
            "a: !!float 00_1.5\n",
            &[],
            "with_entries(.)",
            "a: !!float 00_1.5\n",
        ),
        (
            "a: !!float 00_1.5\n",
            &["-o=json", "-I0"],
            "[.a] | tojson",
            "\"[\\n  1.5\\n]\\n\"\n",
        ),
        (
            "a: !!float 00_1.5\n",
            &[],
            "[.a] | tojson",
            "[\n  1.5\n]\n\n",
        ),
        (
            "a: !!float -0_0\n",
            &["-o=json", "-I0"],
            ".a | tostring",
            "\"-0_0\"\n",
        ),
        ("a: !!float -0_0\n", &[], ".a | tostring", "-0_0\n"),
        ("a: !!float -0_0\n", &["-o=json", "-I0"], ".a * 2", "-0.0\n"),
        ("a: !!float -0_0\n", &[], ".a * 2", "-0\n"),
        (
            "a: !!float -0_0\n",
            &["-o=json", "-I0"],
            ".b = .a",
            "{\"a\":-0.0,\"b\":-0.0}\n",
        ),
        (
            "a: !!float -0_0\n",
            &[],
            ".b = .a",
            "a: !!float -0_0\nb: !!float -0_0\n",
        ),
        (
            "a: !!float -0_0\n",
            &["-o=json", "-I0"],
            "with_entries(.)",
            "{\"a\":-0.0}\n",
        ),
        (
            "a: !!float -0_0\n",
            &[],
            "with_entries(.)",
            "a: !!float -0_0\n",
        ),
        (
            "a: !!float -0_0\n",
            &["-o=json", "-I0"],
            "[.a] | tojson",
            "\"[\\n  -0.0\\n]\\n\"\n",
        ),
        (
            "a: !!float -0_0\n",
            &[],
            "[.a] | tojson",
            "[\n  -0.0\n]\n\n",
        ),
        (
            "a: !!float 1_0\n",
            &["-o=json", "-I0"],
            ".a | tostring",
            "\"1_0\"\n",
        ),
        ("a: !!float 1_0\n", &[], ".a | tostring", "1_0\n"),
        ("a: !!float 1_0\n", &["-o=json", "-I0"], ".a * 2", "20.0\n"),
        ("a: !!float 1_0\n", &[], ".a * 2", "20\n"),
        (
            "a: !!float 1_0\n",
            &["-o=json", "-I0"],
            ".b = .a",
            "{\"a\":10.0,\"b\":10.0}\n",
        ),
        (
            "a: !!float 1_0\n",
            &[],
            ".b = .a",
            "a: !!float 1_0\nb: !!float 1_0\n",
        ),
        (
            "a: !!float 1_0\n",
            &["-o=json", "-I0"],
            "with_entries(.)",
            "{\"a\":10.0}\n",
        ),
        (
            "a: !!float 1_0\n",
            &[],
            "with_entries(.)",
            "a: !!float 1_0\n",
        ),
        (
            "a: !!float 1_0\n",
            &["-o=json", "-I0"],
            "[.a] | tojson",
            "\"[\\n  10.0\\n]\\n\"\n",
        ),
        ("a: !!float 1_0\n", &[], "[.a] | tojson", "[\n  10.0\n]\n\n"),
    ])
}

#[test]
fn test_the_underscored_float_neighbours_are_unchanged_6958() -> Result<()> {
    assert_yq_rows(&[
        (
            "a: !!int 1_000\n",
            &["-o=json", "-I0"],
            ".a | tostring",
            "\"1_000\"\n",
        ),
        (
            "a: !!int 1_000\n",
            &[],
            ".b = .a",
            "a: !!int 1_000\nb: !!int 1_000\n",
        ),
        ("a: !!float 1__0.5\n", &[], ".a | tostring", "1__0.5\n"),
        ("a: !!float 1.5_\n", &[], ".a | tostring", "1.5_\n"),
        (
            "a: !!float 1_0.5\nb: 2\n",
            &[],
            ".b = .a | .b | tostring",
            "1_0.5\n",
        ),
        (
            "a: [!!float 1_0.5, !!float 1_000]\n",
            &[],
            ".[] |= .",
            "a: [!!float 1_0.5, !!float 1_000]\n",
        ),
    ])
}
