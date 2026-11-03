//! #6091: yq's `upcase` and `downcase` map by Go's simple Unicode case mapping, not ASCII only.
//! Every expected output is what yq v4.53.3 prints. The mapping was also compared with yq over every
//! printable code point of the basic and supplementary planes: `ß` stays, `İ` lowercases to `i`, the
//! Greek letters with an iota subscript uppercase to their titlecase letter, and a case pair that
//! Unicode 16 added (`ɤ`/`Ɤ`, `ƛ`/`Ƛ`, the Garay letters) maps neither way, as Go's tables are
//! Unicode 15.

use anyhow::Result;

use crate::run_yq_stdin;

/// (name, document, filter, expected stdout).
const ORACLE_ROWS: &[(&str, &str, &str, &str)] = &[
    (
        "latin_upcase",
        "a: ágUA ǆ ǅ Ǆ ÿ ŉ\n",
        ".a | upcase",
        "ÁGUA Ǆ Ǆ Ǆ Ÿ ŉ\n",
    ),
    (
        "latin_downcase",
        "a: ágUA ǆ ǅ Ǆ ÿ ŉ\n",
        ".a | downcase",
        "água ǆ ǆ ǆ ÿ ŉ\n",
    ),
    (
        "sharp_s_dotted_i_upcase",
        "a: ß ẞ İ ı I i ǰ\n",
        ".a | upcase",
        "ß ẞ İ I I I ǰ\n",
    ),
    (
        "sharp_s_dotted_i_downcase",
        "a: ß ẞ İ ı I i ǰ\n",
        ".a | downcase",
        "ß ß i ı i i ǰ\n",
    ),
    (
        "greek_upcase",
        "a: ΑΣ Σίσυφος ὈΔΥΣΣΕΎΣ ᾀ ᾈ ᾳ ῃ ῳ ᾼ\n",
        ".a | upcase",
        "ΑΣ ΣΊΣΥΦΟΣ ὈΔΥΣΣΕΎΣ ᾈ ᾈ ᾼ ῌ ῼ ᾼ\n",
    ),
    (
        "greek_downcase",
        "a: ΑΣ Σίσυφος ὈΔΥΣΣΕΎΣ ᾀ ᾈ ᾳ ῃ ῳ ᾼ\n",
        ".a | downcase",
        "ασ σίσυφος ὀδυσσεύσ ᾀ ᾀ ᾳ ῃ ῳ ᾳ\n",
    ),
    (
        "cyrillic_armenian_georgian_upcase",
        "a: привет ՈՒ ა Ა\n",
        ".a | upcase",
        "ПРИВЕТ ՈՒ Ა Ა\n",
    ),
    (
        "cyrillic_armenian_georgian_downcase",
        "a: привет ՈՒ ა Ა\n",
        ".a | downcase",
        "привет ու ა ა\n",
    ),
    (
        "astral_upcase",
        "a: 𐐨𐐀 𞤢𞤀 𐓘𐒰\n",
        ".a | upcase",
        "𐐀𐐀 𞤀𞤀 𐒰𐒰\n",
    ),
    (
        "astral_downcase",
        "a: 𐐨𐐀 𞤢𞤀 𐓘𐒰\n",
        ".a | downcase",
        "𐐨𐐨 𞤢𞤢 𐓘𐓘\n",
    ),
    (
        "unicode16_pairs_upcase",
        "a: \"\\u019B \\u0264 \\uA7CD \\uA7DB \\uA7DC \\uA7CB\"\n",
        ".a | upcase",
        "ƛ ɤ ꟍ ꟛ Ƛ Ɤ\n",
    ),
    (
        "unicode16_pairs_downcase",
        "a: \"\\u019B \\u0264 \\uA7CD \\uA7DB \\uA7DC \\uA7CB\"\n",
        ".a | downcase",
        "ƛ ɤ ꟍ ꟛ Ƛ Ɤ\n",
    ),
    (
        "mixed_upcase",
        "a: Hello, Wörld! 123 ñandú\n",
        ".a | upcase",
        "HELLO, WÖRLD! 123 ÑANDÚ\n",
    ),
    (
        "mixed_downcase",
        "a: Hello, Wörld! 123 ñandú\n",
        ".a | downcase",
        "hello, wörld! 123 ñandú\n",
    ),
    (
        "map_values_upcase",
        "a: é\nb: [ü, ß]\n",
        ".. |= (select(tag == \"!!str\") | upcase)",
        "a: É\nb: [Ü, ß]\n",
    ),
    ("upcase_a_number", "a: 1\n", ".a | upcase", ""),
    ("downcase_null", "a: ~\n", ".a | downcase", ""),
];

#[test]
fn upcase_and_downcase_map_unicode_as_yq_does_6091() -> Result<()> {
    for &(name, document, filter, expected) in ORACLE_ROWS {
        let (output, code) = run_yq_stdin(filter, document, &[])?;
        // A number or a null cannot change case: yq refuses and prints nothing, and so do we.
        let refused = name.starts_with("upcase_a_number") || name.starts_with("downcase_null");
        assert_eq!(code != 0, refused, "{name}");
        assert_eq!(output, expected, "{name}: `{filter}` over {document:?}");
    }
    Ok(())
}

/// jq's `ascii_upcase`/`ascii_downcase` stay ASCII, in jq mode and under `--jq-extensions`.
#[test]
fn ascii_case_builtins_stay_ascii_6091() -> Result<()> {
    for (args, expected) in [
        (vec!["jq", "-n", r#""áB" | ascii_upcase"#], "\"áB\"\n"),
        (vec!["jq", "-n", r#""ÁB" | ascii_downcase"#], "\"Áb\"\n"),
        (
            vec!["yq", "--jq-extensions", "-n", r#""áB" | ascii_upcase"#],
            "áB\n",
        ),
    ] {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_succinctly"))
            .args(&args)
            .output()?;
        assert_eq!(String::from_utf8(output.stdout)?, expected, "{args:?}");
    }
    Ok(())
}
