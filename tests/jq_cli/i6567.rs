//! #6567: a fold step whose right side reads the state through a computed key or a `//` default
//! (`.[$k] = (.[$k] // 0) + 1`, the counting idiom) was not a shape the owned fold knew, so the
//! whole assignment went to the evaluator and the state was written out and indexed again every
//! step: 0.20x of jq 1.7.1 at a hundred keys, where the `+=` spelling of the same fold is several
//! times faster than jq. The pure operand grammar reads `.[K]` for a string or integer key and
//! `a // b` now, as an operand only, and declines where a read raises. Every row was captured from
//! jq 1.7.1 (`-n -c`).

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn a_fold_reading_its_state_by_a_computed_key_or_default_is_jqs_answer_6567() -> Result<()> {
    for (filter, expected) in [
        ("reduce range(40) as $i ({}; .[($i % 7 | tostring)] = (.[($i % 7 | tostring)] // 0) + 1)", "{\"0\":6,\"1\":6,\"2\":6,\"3\":6,\"4\":6,\"5\":5,\"6\":5}\n"),
        ("reduce range(40) as $i ({}; .[($i % 7 | tostring)] = (.[($i % 7 | tostring)] // 0) + 1) | length", "7\n"),
        ("reduce range(10) as $i ({}; .x = ((.x // 0) + 1))", "{\"x\":10}\n"),
        ("reduce range(10) as $i ({}; .b = (.a // 0) + 1 | .a = $i)", "{\"b\":9,\"a\":9}\n"),
        ("reduce range(12) as $i ([0,0,0]; .[$i % 3] = (.[$i % 3] // 0) + $i)", "[18,22,26]\n"),
        ("reduce range(12) as $i ([]; .[$i % 3] = (.[$i % 3] // 0) + 1)", "[4,4,4]\n"),
        ("reduce (\"a\",\"b\",\"a\",\"c\",\"a\") as $w ({}; .[$w] = (.[$w] // 0) + 1)", "{\"a\":3,\"b\":1,\"c\":1}\n"),
        ("reduce (\"a\",\"b\",\"a\") as $w ({}; .[$w] = (.[$w] // []) + [$w])", "{\"a\":[\"a\",\"a\"],\"b\":[\"b\"]}\n"),
        ("[foreach range(5) as $i ({}; .[($i % 2 | tostring)] = (.[($i % 2 | tostring)] // 0) + 1; .)]", "[{\"0\":1},{\"0\":1,\"1\":1},{\"0\":2,\"1\":1},{\"0\":2,\"1\":2},{\"0\":3,\"1\":2}]\n"),
        ("reduce range(8) as $i ({c: {}}; .c[($i % 3 | tostring)] = (.c[($i % 3 | tostring)] // 0) + $i)", "{\"c\":{\"0\":9,\"1\":12,\"2\":7}}\n"),
        ("reduce range(4) as $i (null; .[\"k\"] = (.[\"k\"] // 0) + 1)", "{\"k\":4}\n"),
        ("reduce (false, null, 0, 1) as $v ({}; .a = (.a // 10) + 1 | .b = $v)", "{\"a\":14,\"b\":1}\n"),
        ("reduce range(6) as $i ({\"a\": false}; .a = (.a // 7) + $i)", "{\"a\":22}\n"),
        ("reduce range(5) as $i ({\"a\": 0}; .a = (.a // 7) + $i)", "{\"a\":10}\n"),
        ("reduce range(5) as $i ({}; .[($i | tostring)] = (.[(($i - 1) | tostring)] // 100) + 1)", "{\"0\":101,\"1\":102,\"2\":103,\"3\":104,\"4\":105}\n"),
        ("reduce range(6) as $i ([5,6,7]; .[0] = (.[($i % 3)] // 0) + 1)", "[8,6,7]\n"),
        ("reduce range(6) as $i ([5,6,7]; .[0] = (.[(0 - $i)] // 0) + 1)", "[1,6,7]\n"),
        ("reduce range(5) as $i ({}; .a = (.a // 0) + 0.5)", "{\"a\":2.5}\n"),
        ("reduce range(5) as $i ({\"a\": 1.5}; .a = (.[\"a\"] // 0) * 2)", "{\"a\":48}\n"),
        ("[range(6) | {a: .}] | map(select((.a // 9) > 2))", "[{\"a\":3},{\"a\":4},{\"a\":5}]\n"),
        ("[range(6) | {a: (if . % 2 == 0 then . else null end)}] | map(select((.a // 9) > 4)) | length", "3\n"),
        ("[range(6) | {a: .}] | map(select(.[\"a\"] > 2))", "[{\"a\":3},{\"a\":4},{\"a\":5}]\n"),
        ("[range(5) | [., . * 2]] | map(select((.[1] // 0) > 3)) | length", "3\n"),
        ("[range(5) | {k: \"a\", a: .}] | map(select(.[.k] > 2))", "[{\"k\":\"a\",\"a\":3},{\"k\":\"a\",\"a\":4}]\n"),
        ("[range(5) | {k: \"a\", a: .}] | map(select((.[\"zz\"] // 0) > 2)) | length", "0\n"),
        ("reduce range(4) as $i ({a:{b:{}}}; .a.b[($i | tostring)] = (.a.b[($i | tostring)] // 0) + 1)", "{\"a\":{\"b\":{\"0\":1,\"1\":1,\"2\":1,\"3\":1}}}\n"),
        ("reduce range(3) as $i ({}; .a = ((.b[(\"k\")]) // 0) + 1)", "{\"a\":1}\n"),
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-n", "-c", filter], None)?;
        assert_eq!(stdout, expected, "{filter}");
        assert_eq!(stderr, "", "{filter}");
        assert_eq!(code, 0, "{filter}");
    }
    Ok(())
}

#[test]
fn a_fold_whose_state_read_raises_keeps_jqs_error_6567() -> Result<()> {
    for (filter, message) in [
        (
            "reduce range(3) as $i (1; .x = (.[(\"k\")] // 0) + 1)",
            "jq: error (at <unknown>): Cannot index number with string \"k\"\n",
        ),
        (
            "reduce range(3) as $i ({}; .[$i] = (.[\"k\"] // 0) + 1)",
            "jq: error (at <unknown>): Cannot index object with number\n",
        ),
        (
            "reduce range(3) as $i ([]; .a = (.[(\"k\")] // 0) + 1)",
            "jq: error (at <unknown>): Cannot index array with string \"k\"\n",
        ),
        (
            "reduce range(3) as $i ({}; .a = (.b // 0) + \"x\")",
            "jq: error (at <unknown>): number (0) and string (\"x\") cannot be added\n",
        ),
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-n", "-c", filter], None)?;
        assert_eq!(stdout, "", "{filter}");
        assert_eq!(stderr, message, "{filter}");
        assert_eq!(code, 5, "{filter}");
    }
    Ok(())
}
