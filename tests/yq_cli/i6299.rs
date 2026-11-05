//! #6299: a plain scalar under a tag of the secondary handle that names no core type is its text in yq:
//! `a: !!foo 1` is the string `"1"`, and so are `!!binary 1`, `!!set 1` and `!!str: 1`, where the `:` belongs
//! to the tag and not to a mapping. A primary-handle custom tag (`!tag 1`) resolves as the untagged text
//! does, and the core tags (`!!int`, `!!str`, `!!null`, `!!bool`, `!!float`) force their own kind. Every row
//! is a live yq v4.53.3 capture (`-o=json -I0`).
//! Left out: `a: !x!y 1`, which yq refuses (`found undefined tag handle`) and this reads.

use anyhow::Result;

use crate::run_yq_stdin;

/// (document, filter, expected stdout).
const ROWS: &[(&str, &str, &str)] = &[
    ("a: !!foo 1\n", ".", "{\"a\":\"1\"}\n"),
    ("a: !!foo 1\n", ".a | type", "\"!!foo\"\n"),
    ("a: !!strx 1\n", ".", "{\"a\":\"1\"}\n"),
    ("a: !!strx 1\n", ".a | type", "\"!!strx\"\n"),
    ("a: !!int2 5\n", ".", "{\"a\":\"5\"}\n"),
    ("a: !!int2 5\n", ".a | type", "\"!!int2\"\n"),
    ("a: !!str: 1\n", ".", "{\"a\":\"1\"}\n"),
    ("a: !!str: 1\n", ".a | type", "\"!!str:\"\n"),
    ("[!!str: 1]\n", ".", "[\"1\"]\n"),
    ("[!!str: 1]\n", ".[] | type", "\"!!str:\"\n"),
    ("!!str: 1\n", ".", "\"1\"\n"),
    ("!!str: 1\n", ".a | type", ""),
    ("[!!int: 1]\n", ".", "[\"1\"]\n"),
    ("[!!int: 1]\n", ".[] | type", "\"!!int:\"\n"),
    ("a: !!str.x 1\n", ".", "{\"a\":\"1\"}\n"),
    ("a: !!str.x 1\n", ".a | type", "\"!!str.x\"\n"),
    ("a: !!set 1\n", ".", "{\"a\":\"1\"}\n"),
    ("a: !!set 1\n", ".a | type", "\"!!set\"\n"),
    ("a: !!omap 1\n", ".", "{\"a\":\"1\"}\n"),
    ("a: !!omap 1\n", ".a | type", "\"!!omap\"\n"),
    ("a: !!merge 1\n", ".", "{\"a\":\"1\"}\n"),
    ("a: !!merge 1\n", ".a | type", "\"!!merge\"\n"),
    ("a: !!value 1\n", ".", "{\"a\":\"1\"}\n"),
    ("a: !!value 1\n", ".a | type", "\"!!value\"\n"),
    ("a: !!yaml 1\n", ".", "{\"a\":\"1\"}\n"),
    ("a: !!yaml 1\n", ".a | type", "\"!!yaml\"\n"),
    ("a: !!foo true\n", ".", "{\"a\":\"true\"}\n"),
    ("a: !!foo true\n", ".a | type", "\"!!foo\"\n"),
    ("a: !!foo null\n", ".", "{\"a\":\"null\"}\n"),
    ("a: !!foo null\n", ".a | type", "\"!!foo\"\n"),
    ("a: !!foo ~\n", ".", "{\"a\":\"~\"}\n"),
    ("a: !!foo ~\n", ".a | type", "\"!!foo\"\n"),
    ("a: !!int: 0x1f\n", ".", "{\"a\":\"0x1f\"}\n"),
    ("a: !!int: 0x1f\n", ".a | type", "\"!!int:\"\n"),
    ("a: !!float: 1\n", ".", "{\"a\":\"1\"}\n"),
    ("a: !!float: 1\n", ".a | type", "\"!!float:\"\n"),
    ("a: !!bool: yes\n", ".", "{\"a\":\"yes\"}\n"),
    ("a: !!bool: yes\n", ".a | type", "\"!!bool:\"\n"),
    ("a: !!null: null\n", ".", "{\"a\":\"null\"}\n"),
    ("a: !!null: null\n", ".a | type", "\"!!null:\"\n"),
    ("a: !!binary 1\n", ".", "{\"a\":\"1\"}\n"),
    ("a: !!binary 1\n", ".a | type", "\"!!binary\"\n"),
    ("a: !!seq 1\n", ".", "{\"a\":\"1\"}\n"),
    ("a: !!seq 1\n", ".a | type", "\"!!seq\"\n"),
    ("a: !!map 1\n", ".", "{\"a\":\"1\"}\n"),
    ("a: !!map 1\n", ".a | type", "\"!!map\"\n"),
    ("a: !!foo 0x1f\n", ".", "{\"a\":\"0x1f\"}\n"),
    ("a: !!foo 0x1f\n", ".a | type", "\"!!foo\"\n"),
    ("a: !!foo 1.5\n", ".", "{\"a\":\"1.5\"}\n"),
    ("a: !!foo 1.5\n", ".a | type", "\"!!foo\"\n"),
    ("a: !!foo .inf\n", ".", "{\"a\":\".inf\"}\n"),
    ("a: !!foo .inf\n", ".a | type", "\"!!foo\"\n"),
    ("a: !tag 1\n", ".", "{\"a\":1}\n"),
    ("a: !tag 1\n", ".a | type", "\"!tag\"\n"),
    ("a: !tag true\n", ".", "{\"a\":true}\n"),
    ("a: !tag true\n", ".a | type", "\"!tag\"\n"),
    ("a: !tag: 1\n", ".", "{\"a\":1}\n"),
    ("a: !tag: 1\n", ".a | type", "\"!tag:\"\n"),
    ("a: !!int 5\n", ".", "{\"a\":5}\n"),
    ("a: !!int 5\n", ".a | type", "\"!!int\"\n"),
    ("a: !!str 5\n", ".", "{\"a\":\"5\"}\n"),
    ("a: !!str 5\n", ".a | type", "\"!!str\"\n"),
    ("a: !!null x\n", ".", "{\"a\":null}\n"),
    ("a: !!null x\n", ".a | type", "\"!!null\"\n"),
    ("a: !!bool yes\n", ".", "{\"a\":true}\n"),
    ("a: !!bool yes\n", ".a | type", "\"!!bool\"\n"),
    ("a: !!float 5\n", ".", "{\"a\":5.0}\n"),
    ("a: !!float 5\n", ".a | type", "\"!!float\"\n"),
    (
        "a: !!foo: 1\nb: !!foo 2\n",
        ".",
        "{\"a\":\"1\",\"b\":\"2\"}\n",
    ),
    ("a: !!foo: 1\nb: !!foo 2\n", ".a | type", "\"!!foo:\"\n"),
    ("- !!str: 1\n- !!str: 2\n", ".", "[\"1\",\"2\"]\n"),
    (
        "- !!str: 1\n- !!str: 2\n",
        ".[] | type",
        "\"!!str:\"\n\"!!str:\"\n",
    ),
    ("{a: !!foo 1}\n", ".", "{\"a\":\"1\"}\n"),
    ("{a: !!foo 1}\n", ".[] | type", "\"!!foo\"\n"),
    ("[!!foo 1, 2]\n", ".", "[\"1\",2]\n"),
    ("[!!foo 1, 2]\n", ".[] | type", "\"!!foo\"\n\"!!int\"\n"),
    ("{!!str: 1}\n", ".", "{\"1\":null}\n"),
    ("{!!str: 1}\n", ".[] | type", "\"!!null\"\n"),
    ("[!!str : 1]\n", ".", "[{\"\":1}]\n"),
    ("[!!str : 1]\n", ".[] | type", "\"!!map\"\n"),
    ("a: !!foo \"q\"\n", ".", "{\"a\":\"q\"}\n"),
    ("a: !!foo \"q\"\n", ".a | type", "\"!!foo\"\n"),
    ("a: !!foo [1]\n", ".", "{\"a\":[1]}\n"),
    ("a: !!foo [1]\n", ".a | type", "\"!!foo\"\n"),
    ("a: !!foo\n", ".", "{\"a\":\"\"}\n"),
    ("a: !!foo\n", ".a | type", "\"!!foo\"\n"),
    ("a: !!str:\n", ".", "{\"a\":\"\"}\n"),
    ("a: !!str:\n", ".a | type", "\"!!str:\"\n"),
];

#[test]
fn a_scalar_under_an_unknown_secondary_handle_tag_is_its_text_6299() -> Result<()> {
    for (document, filter, expected) in ROWS {
        let (stdout, code) = run_yq_stdin(filter, document, &["-o", "json", "-I0"])?;
        assert_eq!(stdout, *expected, "{document:?} | {filter}");
        assert_eq!(code, 0, "{document:?} | {filter}");
    }
    Ok(())
}
