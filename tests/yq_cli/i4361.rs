//! #4361: in yq mode a bracketed string key read straight off a `+` or `*` result answers nothing
//! when the mapping lacks it, where the dotted and piped spellings answer `null`. Every output is
//! captured from yq v4.53.3.

use anyhow::Result;

use crate::run_yq_stdin;

const DOC: &str = "a: {x: 1}\nb: {}\nn: [1, 2]\n";

#[test]
fn test_absent_bracket_key_on_a_sum_or_merge_is_empty_4361() -> Result<()> {
    for (filter, want) in [
        ("(.a + {})[\"z\"]", ""),
        ("(.a * {\"q\": 1})[\"z\"]", ""),
        ("(.a + {})[\"a\"]", ""),
        ("(.a + {})[\"x\"]", "1\n"),
        ("(.a *+ {\"q\": [1]})[\"z\"]", ""),
        ("(.b + {})[\"z\"]", ""),
        ("((.a + {}))[\"z\"]", ""),
        ("(.a + {})[\"z\"]?", ""),
        ("(.a + {})[\"z\"] // \"d\"", "d\n"),
        ("[(.a + {})[\"z\"]]", "[]\n"),
        ("(.a + {})[\"z\"] | length", ""),
        ("(.a + {})[\"z\"][\"y\"]", ""),
        ("\"z\" as $k | (.a + {})[$k]", ""),
        ("\"x\" as $k | (.a + {})[$k]", "1\n"),
        ("(.a + {})[(\"z\", \"x\")]", "1\n"),
        ("(.a + {}).z", "null\n"),
        ("(.a + {}).[\"z\"]", "null\n"),
        ("(.a + {}) | .[\"z\"]", "null\n"),
        ("(.a + {}) as $o | $o[\"z\"]", "null\n"),
        ("(.a // .b)[\"z\"]", "null\n"),
        (".a[\"z\"]", "null\n"),
        ("(.a | .)[\"z\"]", "null\n"),
        ("(.n + [3])[9]", "null\n"),
        ("(1 + 2)[\"z\"]", ""),
        ("(.a + {})[\"z\"] = 1", "a: {x: 1}\nb: {}\nn: [1, 2]\n"),
        ("(.a + {})[\"z\"] |= 1", "a: {x: 1}\nb: {}\nn: [1, 2]\n"),
        ("del((.a + {})[\"z\"])", "a: {x: 1}\nb: {}\nn: [1, 2]\n"),
        ("0 as $k | (.a + {})[$k]", ""),
        ("true as $k | (.a * {\"q\": 1})[$k]", ""),
        ("((.a + {})[\"k\"]).x = 1", "a: {x: 1}\nb: {}\nn: [1, 2]\n"),
        (
            "(.a + {})[\"k\"][\"m\"] |= 1",
            "a: {x: 1}\nb: {}\nn: [1, 2]\n",
        ),
        ("(.a * {})[\"x\"][0] += 1", "a: {x: 1}\nb: {}\nn: [1, 2]\n"),
    ] {
        let (stdout, code) = run_yq_stdin(filter, DOC, &[])?;
        assert_eq!((stdout.as_str(), code), (want, 0), "`{filter}`");
    }
    Ok(())
}
