//! #7292: `.[]` and `map(f)` over a mapping collapse every collection key (a sequence or mapping
//! key) into one slot, as yq v4.53.3 does: its text is empty, so two different collection keys are
//! one key to the traversal, and so is a plain `""` key. The first such key keeps its position and
//! the last one's value stays. Each row is `(document, filter, stdout)` as yq answers with
//! `-o json -I0`; an alias key is read as the alias's name, so alias keys stay apart.

use anyhow::Result;

use crate::run_yq_stdin;

const ROWS: &[(&str, &str, &str)] = &[
    (
        "? [1, 2]\n: p\n? [1, 2]\n: q\nr: 1\n",
        "[.[]]",
        "[\"q\",1]\n",
    ),
    (
        "? [1, 2]\n: p\n? [1, 2]\n: q\nr: 1\n",
        "map(.)",
        "[\"q\",1]\n",
    ),
    ("? [1, 2]\n: p\n? [3]\n: q\nr: 1\n", "[.[]]", "[\"q\",1]\n"),
    ("? [1, 2]\n: p\n? [3]\n: q\nr: 1\n", "map(.)", "[\"q\",1]\n"),
    ("? {a: 1}\n: p\n? {a: 1}\n: q\n", "[.[]]", "[\"q\"]\n"),
    (
        "? [1, 2]\n: p\nr: 1\n? [1, 2]\n: q\n",
        "[.[]]",
        "[\"q\",1]\n",
    ),
    (
        "? [1, 2]\n: p\nr: 1\n? [1, 2]\n: q\n",
        "map(.)",
        "[\"q\",1]\n",
    ),
    (
        "o: {? [1, 2] : p, ? [1, 2] : q, r: 1}\n",
        ".o | [.[]]",
        "[\"q\",1]\n",
    ),
    (
        "? [1]\n: p\n? {x: 1}\n: q\n? [2]\n: r\n",
        "[.[]]",
        "[\"r\"]\n",
    ),
    // A plain `""` key is the same key as a collection key, in either order.
    ("? [1]\n: p\n\"\": q\nr: 1\n", "[.[]]", "[\"q\",1]\n"),
    ("\"\": q\n? [1]\n: p\nr: 1\n", "[.[]]", "[\"p\",1]\n"),
    (
        "a: 1\n? [1]\n: p\nb: 2\n? [2]\n: q\n",
        "[.[]]",
        "[1,\"q\",2]\n",
    ),
    ("? [1]\n: true\n? [2]\n: false\n", "[.[]]", "[false]\n"),
    ("? [1]\n: true\n? [2]\n: false\n", ".[]", "false\n"),
    // One collection key, and ordinary keys, are not a repeat.
    ("? [1]\n: p\nr: 1\n", "[.[]]", "[\"p\",1]\n"),
    // Only the traversal collapses: the count, like yq's, keeps every member.
    ("? [1]\n: p\n? [2]\n: q\nr: 1\n", "length", "3\n"),
    (
        "? [1]\n: p\n? [2]\n: q\nr: 1\n",
        "to_entries | length",
        "3\n",
    ),
    // An alias key is its alias's name, so two aliases of different collections are two keys.
    (
        "a: &x [1]\nb: &y [2]\n*x : 1\n*y : 2\n",
        "[.[]]",
        "[[1],[2],1,2]\n",
    ),
];

#[test]
fn test_collection_keys_collapse_under_iteration_7292() -> Result<()> {
    for &(document, filter, stdout) in ROWS {
        let (out, code) = run_yq_stdin(filter, document, &["-o", "json", "-I0"])?;
        assert_eq!(
            (out.as_str(), code),
            (stdout, 0),
            "{filter} on {document:?}"
        );
    }
    Ok(())
}

/// The same rule on objects past the eight keys the stack compares pairwise and the 64-key batch
/// the hashes are taken in, with the collection keys at both ends and in the middle.
#[test]
fn test_collection_keys_collapse_in_a_wide_mapping_7292() -> Result<()> {
    for width in [9usize, 64, 65, 130] {
        let mut document = String::from("? [0]\n: first\n");
        for i in 0..width {
            document.push_str(&format!("k{i}: {i}\n"));
            if i == width / 2 {
                document.push_str("? {m: 1}\n: middle\n");
            }
        }
        document.push_str("? [9]\n: last\n");
        // The collapsed slot is the first collection key's, holding the last one's value.
        let mut expected = vec!["\"last\"".to_string()];
        expected.extend((0..width).map(|i| i.to_string()));
        let expected = format!("[{}]\n", expected.join(","));
        for filter in ["[.[]]", "map(.)"] {
            let (out, code) = run_yq_stdin(filter, &document, &["-o", "json", "-I0"])?;
            assert_eq!(
                (out.as_str(), code),
                (expected.as_str(), 0),
                "{filter} at {width}"
            );
        }
    }
    Ok(())
}
