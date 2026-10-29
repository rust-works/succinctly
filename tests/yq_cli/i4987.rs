//! #4987: yq's JSON rendering of a mapping expands a `<<` source only when it is spelled as an alias. An inline
//! mapping, an anchored inline mapping and `{q: *a}` contribute nothing to it, while a lookup (`.item.q`) and an
//! iteration (`.[] | key`) still see their keys.

use anyhow::Result;

use crate::run_yq_stdin;

/// Every row is captured from yq v4.53.3 (`-o=json -I=0`) over nine merge shapes: an inline source alone, in a
/// sequence before and after an alias, anchored, holding an alias value, two inline sources, written before an own
/// key, an alias source (which still renders), and a sequence of mappings each with its own `<<`.
#[test]
fn test_4987_json_rendering_drops_a_merge_source_that_is_not_an_alias() -> Result<()> {
    let rows: &[(&str, &str, &str)] = &[
        ("item:\n  <<: {q: 1}\n  k: 0\n", ".item", "{\"k\":0}"),
        ("item:\n  <<: {q: 1}\n  k: 0\n", ".", "{\"item\":{\"k\":0}}"),
        (
            "item:\n  <<: {q: 1}\n  k: 0\n",
            "[.item|.[]|key]",
            "[\"q\",\"k\"]",
        ),
        ("item:\n  <<: {q: 1}\n  k: 0\n", ".item.q", "1"),
        (
            "a: &a {x: 1, z: 9}\nitem:\n  <<: [*a, {q: 1}]\n  k: 0\n",
            ".item",
            "{\"x\":1,\"z\":9,\"k\":0}",
        ),
        (
            "a: &a {x: 1, z: 9}\nitem:\n  <<: [*a, {q: 1}]\n  k: 0\n",
            ".",
            "{\"a\":{\"x\":1,\"z\":9},\"item\":{\"x\":1,\"z\":9,\"k\":0}}",
        ),
        (
            "a: &a {x: 1, z: 9}\nitem:\n  <<: [*a, {q: 1}]\n  k: 0\n",
            "[.item|.[]|key]",
            "[\"x\",\"z\",\"q\",\"k\"]",
        ),
        (
            "a: &a {x: 1, z: 9}\nitem:\n  <<: [*a, {q: 1}]\n  k: 0\n",
            ".item.q",
            "1",
        ),
        (
            "a: &a {x: 1, z: 9}\nitem:\n  <<: [{q: 1}, *a]\n  k: 0\n",
            ".item",
            "{\"x\":1,\"z\":9,\"k\":0}",
        ),
        (
            "a: &a {x: 1, z: 9}\nitem:\n  <<: [{q: 1}, *a]\n  k: 0\n",
            ".",
            "{\"a\":{\"x\":1,\"z\":9},\"item\":{\"x\":1,\"z\":9,\"k\":0}}",
        ),
        (
            "a: &a {x: 1, z: 9}\nitem:\n  <<: [{q: 1}, *a]\n  k: 0\n",
            "[.item|.[]|key]",
            "[\"q\",\"x\",\"z\",\"k\"]",
        ),
        (
            "a: &a {x: 1, z: 9}\nitem:\n  <<: [{q: 1}, *a]\n  k: 0\n",
            ".item.q",
            "1",
        ),
        ("item:\n  <<: &m {q: 1}\n  k: 0\n", ".item", "{\"k\":0}"),
        (
            "item:\n  <<: &m {q: 1}\n  k: 0\n",
            ".",
            "{\"item\":{\"k\":0}}",
        ),
        (
            "item:\n  <<: &m {q: 1}\n  k: 0\n",
            "[.item|.[]|key]",
            "[\"q\",\"k\"]",
        ),
        ("item:\n  <<: &m {q: 1}\n  k: 0\n", ".item.q", "1"),
        (
            "a: &a {x: 1}\nitem:\n  <<: {q: *a}\n  k: 0\n",
            ".item",
            "{\"k\":0}",
        ),
        (
            "a: &a {x: 1}\nitem:\n  <<: {q: *a}\n  k: 0\n",
            ".",
            "{\"a\":{\"x\":1},\"item\":{\"k\":0}}",
        ),
        (
            "a: &a {x: 1}\nitem:\n  <<: {q: *a}\n  k: 0\n",
            "[.item|.[]|key]",
            "[\"q\",\"k\"]",
        ),
        (
            "a: &a {x: 1}\nitem:\n  <<: {q: *a}\n  k: 0\n",
            ".item.q",
            "{\"x\":1}",
        ),
        (
            "a: &a {x: 1, z: 9}\nitem:\n  <<: *a\n  k: 0\n",
            ".item",
            "{\"x\":1,\"z\":9,\"k\":0}",
        ),
        (
            "a: &a {x: 1, z: 9}\nitem:\n  <<: *a\n  k: 0\n",
            ".",
            "{\"a\":{\"x\":1,\"z\":9},\"item\":{\"x\":1,\"z\":9,\"k\":0}}",
        ),
        (
            "a: &a {x: 1, z: 9}\nitem:\n  <<: *a\n  k: 0\n",
            "[.item|.[]|key]",
            "[\"x\",\"z\",\"k\"]",
        ),
        (
            "a: &a {x: 1, z: 9}\nitem:\n  <<: *a\n  k: 0\n",
            ".item.q",
            "null",
        ),
        (
            "item:\n  <<: [{q: 1}, {r: 2}]\n  k: 0\n",
            ".item",
            "{\"k\":0}",
        ),
        (
            "item:\n  <<: [{q: 1}, {r: 2}]\n  k: 0\n",
            ".",
            "{\"item\":{\"k\":0}}",
        ),
        (
            "item:\n  <<: [{q: 1}, {r: 2}]\n  k: 0\n",
            "[.item|.[]|key]",
            "[\"q\",\"r\",\"k\"]",
        ),
        ("item:\n  <<: [{q: 1}, {r: 2}]\n  k: 0\n", ".item.q", "1"),
        ("item:\n  k: 0\n  <<: {k: 5, q: 1}\n", ".item", "{\"k\":0}"),
        (
            "item:\n  k: 0\n  <<: {k: 5, q: 1}\n",
            ".",
            "{\"item\":{\"k\":0}}",
        ),
        (
            "item:\n  k: 0\n  <<: {k: 5, q: 1}\n",
            "[.item|.[]|key]",
            "[\"k\",\"q\"]",
        ),
        ("item:\n  k: 0\n  <<: {k: 5, q: 1}\n", ".item.q", "1"),
        (
            "a: &a {x: 1, z: 9}\nlist:\n  - <<: [*a, {q: 1}]\n    k: 0\n  - <<: {r: 2}\n    k: 1\n",
            ".item",
            "null",
        ),
        (
            "a: &a {x: 1, z: 9}\nlist:\n  - <<: [*a, {q: 1}]\n    k: 0\n  - <<: {r: 2}\n    k: 1\n",
            ".",
            "{\"a\":{\"x\":1,\"z\":9},\"list\":[{\"x\":1,\"z\":9,\"k\":0},{\"k\":1}]}",
        ),
        (
            "a: &a {x: 1, z: 9}\nlist:\n  - <<: [*a, {q: 1}]\n    k: 0\n  - <<: {r: 2}\n    k: 1\n",
            "[.item|.[]|key]",
            "[]",
        ),
        (
            "a: &a {x: 1, z: 9}\nlist:\n  - <<: [*a, {q: 1}]\n    k: 0\n  - <<: {r: 2}\n    k: 1\n",
            ".item.q",
            "null",
        ),
    ];
    for (input, filter, want) in rows {
        let (out, code) = run_yq_stdin(filter, input, &["-o=json", "-I=0"])?;
        assert_eq!(code, 0, "{filter} on {input:?}: {out:?}");
        assert_eq!(out.trim_end(), *want, "{filter} on {input:?}");
    }
    Ok(())
}
