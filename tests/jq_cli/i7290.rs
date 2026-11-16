//! #7290: `.[]` over an object hands on its value cursors without keeping the field list, and
//! `[.[]] | add` adds those nodes from their cursors instead of decoding the whole array first.
//! Each row is `(document, filter, stdout, stderr, exit status)` as jq 1.7.1 (`/usr/bin/jq`)
//! answers, read from stdin: a repeated key keeps its first position and its last value, the
//! first number of a sum prints as written (`1.0`, `1.50` after a `null`), a `-0` adds as
//! `-0.0`, a sum past the doubles and an operand of another type answer as before, and so do
//! `any`/`all` over an object with a repeated key.

use anyhow::Result;

use crate::run_jq_full;

const ROWS: &[(&str, &str, &str, &str, i32)] = &[
    (r#"{"a":1.0}"#, "[.[]]|add", "1.0\n", "", 0),
    (r#"{"a":1.0,"b":2}"#, "[.[]]|add", "3\n", "", 0),
    (r#"{"a":null,"b":1.50}"#, "[.[]]|add", "1.50\n", "", 0),
    (
        r#"{"a":null,"b":null,"c":1.10}"#,
        "[.[]]|add",
        "1.10\n",
        "",
        0,
    ),
    (r#"{"a":-0}"#, "[.[]]|add", "-0\n", "", 0),
    (r#"{"a":-0,"b":-0}"#, "[.[]]|add", "-0\n", "", 0),
    (r#"{"a":0,"b":-0}"#, "[.[]]|add", "0\n", "", 0),
    (
        r#"{"a":100000000000000000001}"#,
        "[.[]]|add",
        "100000000000000000001\n",
        "",
        0,
    ),
    (
        r#"{"a":100000000000000000001,"b":0}"#,
        "[.[]]|add",
        "1e+20\n",
        "",
        0,
    ),
    (
        r#"{"a":1e1000,"b":1}"#,
        "[.[]]|add",
        "1.7976931348623157e+308\n",
        "",
        0,
    ),
    (
        r#"{"a":0.1,"b":0.2}"#,
        "[.[]]|add",
        "0.30000000000000004\n",
        "",
        0,
    ),
    ("{}", "[.[]]|add", "null\n", "", 0),
    (r#"{"a":null}"#, "[.[]]|add", "null\n", "", 0),
    (
        r#"{"a":"x","b":"y","a":"z"}"#,
        "[.[]]|add",
        "\"zy\"\n",
        "",
        0,
    ),
    (r#"{"b":1,"a":2,"b":3}"#, "[.[]]|add", "5\n", "", 0),
    (
        r#"{"a":1,"b":2,"a":"q"}"#,
        "[.[]]|add",
        "",
        "jq: error (at <stdin>:0): string (\"q\") and number (2) cannot be added\n",
        5,
    ),
    (r#"{"a":[1],"b":[2]}"#, "[.[]]|add", "[1,2]\n", "", 0),
    (
        r#"{"a":{"x":1},"b":{"y":2},"a":{"z":3}}"#,
        "[.[]]|add",
        "{\"z\":3,\"y\":2}\n",
        "",
        0,
    ),
    (
        r#"{"a":1,"b":"x"}"#,
        "[.[]]|add",
        "",
        "jq: error (at <stdin>:0): number (1) and string (\"x\") cannot be added\n",
        5,
    ),
    (
        r#"{"a":3,"b":[1]}"#,
        "[.[]]|add",
        "",
        "jq: error (at <stdin>:0): number (3) and array ([1]) cannot be added\n",
        5,
    ),
    (r#"{"a":1,"b":2}"#, "[.[]]|add?", "3\n", "", 0),
    (r#"{"a":1,"b":"x"}"#, "[.[]]|add?", "", "", 0),
    (
        r#"{"a":1,"b":"x"}"#,
        "try ([.[]]|add) catch .",
        "\"number (1) and string (\\\"x\\\") cannot be added\"\n",
        "",
        0,
    ),
    (r#"{"a":1,"b":2,"a":3}"#, "[.[]]", "[3,2]\n", "", 0),
    (
        r#"{"a":1,"b":2,"a":3}"#,
        "[.[] | . * 10]",
        "[30,20]\n",
        "",
        0,
    ),
    (r#"{"a":true,"b":false,"a":true}"#, "any", "true\n", "", 0),
    (r#"{"a":false,"b":true,"b":false}"#, "any", "false\n", "", 0),
    (r#"{"a":true,"b":true,"a":false}"#, "all", "false\n", "", 0),
    (r#"{"a":1,"b":2,"a":3}"#, "any(. == 1)", "false\n", "", 0),
    (r#"{"a":1,"b":2,"a":3}"#, "all(. > 1)", "true\n", "", 0),
    (r#"{"a":1,"b":2}"#, "[.[]] as $v | $v | add", "3\n", "", 0),
    (r#"{"a":1,"b":2}"#, "[.[], .[]]|add", "6\n", "", 0),
];

#[test]
fn test_object_values_and_their_sum_match_jq_7290() -> Result<()> {
    for (document, filter, stdout, stderr, status) in ROWS {
        let (out, err, code) = run_jq_full(&["-c", filter], Some(document))?;
        assert_eq!(
            (out.as_str(), err.as_str(), code),
            (*stdout, *stderr, *status),
            "{filter} over {document:?}"
        );
    }
    Ok(())
}

/// A wide object with a few repeated keys, and one whose keys all repeat: the sum and the
/// values are those of the collapsed object (first position, last value), as in jq 1.7.1.
#[test]
fn test_wide_object_with_repeated_keys_sums_its_collapsed_values_7290() -> Result<()> {
    // 3,000 fields over 2,990 distinct keys: k0..k2989, then k0..k9 again with value 1000.
    let mut few = String::from("{");
    for i in 0..2990 {
        few.push_str(&format!("\"k{i}\":{},", i % 7));
    }
    for i in 0..10 {
        few.push_str(&format!("\"k{i}\":1000,"));
    }
    few.pop();
    few.push('}');
    // sum of i % 7 over 0..2990, less the first ten keys' own values, plus their last 1000s.
    let base: i64 = (0..2990).map(|i| i % 7).sum();
    let first_ten: i64 = (0..10).map(|i| i % 7).sum();
    let expected = base - first_ten + 10 * 1000;
    let (out, err, code) = run_jq_full(&["-c", "[.[]]|add"], Some(&few))?;
    assert_eq!(
        (out, err, code),
        (format!("{expected}\n"), String::new(), 0)
    );
    let (out, _, _) = run_jq_full(&["-c", "[.[]][:12]"], Some(&few))?;
    assert_eq!(
        out,
        "[1000,1000,1000,1000,1000,1000,1000,1000,1000,1000,3,4]\n"
    );

    // Eight keys, each repeated 500 times: the last value of each, in first-position order.
    let mut all = String::from("{");
    for i in 0..4000 {
        all.push_str(&format!("\"k{}\":{},", i % 8, i));
    }
    all.pop();
    all.push('}');
    let (out, _, code) = run_jq_full(&["-c", "[.[]]"], Some(&all))?;
    assert_eq!(
        (out.as_str(), code),
        ("[3992,3993,3994,3995,3996,3997,3998,3999]\n", 0)
    );
    let (out, _, _) = run_jq_full(&["-c", "[.[]]|add"], Some(&all))?;
    assert_eq!(out, format!("{}\n", (3992..4000).sum::<i64>()));
    Ok(())
}

/// Two different keys whose 64-bit hashes are equal (`key-one-padding!` and `kwjfpwvojvWuiqDf`,
/// constructed by inverting the hash's per-word step): the hashes repeat, so the collapse walk
/// runs, and it compares the keys themselves, so nothing collapses that jq 1.7.1 keeps apart.
/// Alone, with a real repeat beside them, and among 200 other keys (past the hash prefilter's
/// 128-key floor).
#[test]
fn test_colliding_key_hashes_collapse_nothing_7290() -> Result<()> {
    let pair = r#"{"key-one-padding!":1,"kwjfpwvojvWuiqDf":2}"#;
    let repeat = r#"{"key-one-padding!":1,"kwjfpwvojvWuiqDf":2,"key-one-padding!":3}"#;
    // The second key of a colliding pair repeated; four, three and five keys on the one hash
    // (`i7f35QhA5RFytU=l`, `MO7swvZKnn*|^JFk` and `QpymxnKzS~a*(k]#` share it too), with repeats.
    let second = r#"{"key-one-padding!":1,"kwjfpwvojvWuiqDf":2,"kwjfpwvojvWuiqDf":3}"#;
    let four = r#"{"key-one-padding!":1,"kwjfpwvojvWuiqDf":2,"i7f35QhA5RFytU=l":3,"MO7swvZKnn*|^JFk":4,"kwjfpwvojvWuiqDf":5,"MO7swvZKnn*|^JFk":6,"key-one-padding!":7}"#;
    let three = r#"{"i7f35QhA5RFytU=l":1,"MO7swvZKnn*|^JFk":2,"QpymxnKzS~a*(k]#":3,"i7f35QhA5RFytU=l":4,"QpymxnKzS~a*(k]#":5}"#;
    // Ten fields, past the pairwise check: five colliding keys, each twice.
    let keys = [
        "key-one-padding!",
        "kwjfpwvojvWuiqDf",
        "i7f35QhA5RFytU=l",
        "MO7swvZKnn*|^JFk",
        "QpymxnKzS~a*(k]#",
    ];
    let five = format!(
        "{{{}}}",
        (0..10)
            .map(|i| format!("\"{}\":{}", keys[i % 5], i + 1))
            .collect::<Vec<_>>()
            .join(",")
    );
    let mut wide = String::from("{");
    for i in 0..200 {
        wide.push_str(&format!("\"k{i}\":{i},"));
    }
    wide.push_str(r#""key-one-padding!":1,"kwjfpwvojvWuiqDf":2}"#);
    for (document, filter, stdout) in [
        (pair, "[.[]]", "[1,2]\n"),
        (pair, "[.[]]|add", "3\n"),
        (pair, "any(. > 1)", "true\n"),
        (repeat, "[.[]]", "[3,2]\n"),
        (repeat, "[.[]]|add", "5\n"),
        (wide.as_str(), "[.[]]|add", "19903\n"),
        (wide.as_str(), "[.[]]|length", "202\n"),
        (second, "[.[]]", "[1,3]\n"),
        (second, "[.[]]|add", "4\n"),
        (four, "[.[]]", "[7,5,3,6]\n"),
        (four, "[.[]]|add", "21\n"),
        (three, "[.[]]", "[4,2,5]\n"),
        (five.as_str(), "[.[]]", "[6,7,8,9,10]\n"),
        (five.as_str(), "[.[]]|add", "40\n"),
    ] {
        let (out, err, code) = run_jq_full(&["-c", filter], Some(document))?;
        assert_eq!(
            (out.as_str(), err.as_str(), code),
            (stdout, "", 0),
            "{filter}"
        );
    }
    Ok(())
}
