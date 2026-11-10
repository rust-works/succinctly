//! #2435 (the numeric-index slice): a numeric index that lands on an existing mapping member names
//! it by the matched key's own value, as a name does (#2801): `key` and `path` answer the string
//! for a quoted key (`"5":`) and the number for a plain one (`7:`), and the index when no member
//! has that text. Every row is captured from yq v4.53.3 (`-o=json -I=0`).

use anyhow::Result;

use crate::run_yq_stdin;

const DOC: &str = "a: {k: 1, \"5\": five}\nb: {k: 1}\nq: {\"5\": five, 7: seven, 0x10: h, \"x\": y}\nf: {1.5: x, 1.0: y, 1e3: z}\nt: {!!str 5: s, !!int \"6\": i}\n";
const ARGS: &[&str] = &["-o=json", "-I=0"];

/// `(filter, output)`.
const ROWS: &[(&str, &str)] = &[
    // A quoted key is a string: `key`, `path`, and the tag of each.
    (".a[5] | key", "\"5\"\n"),
    (".a[5] | path", "[\"a\",\"5\"]\n"),
    (".a | .[5] | path", "[\"a\",\"5\"]\n"),
    (".a[5] | [key, path]", "[\"5\",[\"a\",\"5\"]]\n"),
    (".a[5] | key | tag", "\"!!str\"\n"),
    (".q[5] | path", "[\"q\",\"5\"]\n"),
    ("(.a[5], .q[7]) | key", "\"5\"\n7\n"),
    // A plain number key stays the number, whatever its spelling (`0x10` is 16).
    (".q[7] | key", "7\n"),
    (".q[7] | path", "[\"q\",7]\n"),
    (".q[16] | path", "[\"q\",16]\n"),
    // A float key is a string to `path` (its spelling), as a name is; a tag decides the rest.
    (".f[1.5] | path", "[\"f\",\"1.5\"]\n"),
    (".f[1.0] | path", "[\"f\",\"1.0\"]\n"),
    (".f[1e3] | path", "[\"f\",\"1e3\"]\n"),
    (".t[5] | key", "\"5\"\n"),
    (".t[5] | path", "[\"t\",\"5\"]\n"),
    (".t[6] | path", "[\"t\",6]\n"),
    // No member with that text: the index is the component, as before.
    (".b[5] | key", "5\n"),
    (".b[5] | path", "[\"b\",5]\n"),
    // What a component is not: the position, the value and a write still find the member.
    (".a[5]", "\"five\"\n"),
    (".a[5] | parent | key", "\"a\"\n"),
    (".a[5] |= (. | key)", "{\"a\":{\"k\":1,\"5\":\"5\"},\"b\":{\"k\":1},\"q\":{\"5\":\"five\",\"7\":\"seven\",\"0x10\":\"h\",\"x\":\"y\"},\"f\":{\"1.5\":\"x\",\"1.0\":\"y\",\"1e3\":\"z\"},\"t\":{\"5\":\"s\",\"6\":\"i\"}}\n"),
    // The bound form always read the key node.
    (".a[5] as $x | $x | key", "\"5\"\n"),
];

#[test]
fn test_a_numeric_index_names_the_matched_key_by_its_own_value_2435() -> Result<()> {
    for (filter, want) in ROWS {
        let (stdout, code) = run_yq_stdin(filter, DOC, ARGS)?;
        assert_eq!(code, 0, "exit code of {filter:?}");
        assert_eq!(&stdout, want, "{filter:?}");
    }
    Ok(())
}

/// What still differs, pinned so a change to it is a decision: a mapping a filter built, or one read
/// inside `map(...)` or behind an `as` binding after a pipe, keeps no key types, so a numeric index
/// into it names its member by the index (yq: `"5"` for a quoted key).
/// Each row is `(filter, what succinctly prints, what yq prints)`.
#[test]
fn test_a_computed_mappings_key_types_stay_recorded_2435() -> Result<()> {
    let rows: &[(&str, &str, &str)] = &[
        ("(.a | . + {}) | .[5] | key", "5\n", "\"5\"\n"),
        (
            "(.a | . + {}) | .[5] | path",
            "[\"a\",5]\n",
            "[\"a\",\"5\"]\n",
        ),
        ("[.a] | map(.[5] | key)", "[5]\n", "[\"5\"]\n"),
        (".a | .[5] as $v | $v | key", "5\n", "\"5\"\n"),
    ];
    for (filter, ours, yq) in rows {
        let (stdout, code) = run_yq_stdin(filter, DOC, ARGS)?;
        assert_eq!(code, 0, "exit code of {filter:?}");
        assert_eq!(&stdout, ours, "{filter:?}");
        assert_ne!(
            ours, yq,
            "{filter:?} now matches yq: drop it from the limitations"
        );
    }
    Ok(())
}
