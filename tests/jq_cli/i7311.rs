//! #7311: an object that repeats a key is printed, iterated and mapped by streaming its fields
//! with a plan of the few fields jq's rule changes (a repeated key keeps its first position and
//! takes its last value; its later occurrences go), instead of materializing every field and
//! owning every key. Each row is `(document, arguments, stdout)` as jq 1.7.1 (`/usr/bin/jq`)
//! answers, read from stdin: escaped spellings of one key, a repeat whose last value is a
//! container, repeats nested in arrays and objects, and two different keys whose 64-bit hashes are
//! equal (`key-one-padding!` and `kwjfpwvojvWuiqDf`, with `i7f35QhA5RFytU=l` and
//! `MO7swvZKnn*|^JFk` on the same hash; see `i7290.rs`), alone and repeated.

use anyhow::Result;

use crate::run_jq_full;

const ROWS: &[(&str, &[&str], &str)] = &[
    (r#"{"a":1,"b":2,"a":3}"#, &["."], "{\n  \"a\": 3,\n  \"b\": 2\n}\n"),
    (r#"{"a":1,"b":2,"a":3}"#, &["-c", "."], "{\"a\":3,\"b\":2}\n"),
    (r#"{"a":1,"b":2,"a":3}"#, &["--indent", "1", "."], "{\n \"a\": 3,\n \"b\": 2\n}\n"),
    (r#"{"a":1,"b":2,"a":3}"#, &["--tab", "."], "{\n\t\"a\": 3,\n\t\"b\": 2\n}\n"),
    (r#"{"a":1,"b":2,"a":3}"#, &["-c", "map(.)"], "[3,2]\n"),
    (r#"{"a":1,"b":2,"a":3}"#, &["-c", "select(true)"], "{\"a\":3,\"b\":2}\n"),
    (r#"{"a":1,"b":2,"a":3}"#, &["-c", ".[]"], "3\n2\n"),
    (r#"{"a":1,"b":2,"a":3}"#, &["-c", "[.[]]"], "[3,2]\n"),
    (r#"{"a":1,"b":2,"a":{"c":[1,2]}}"#, &["."], "{\n  \"a\": {\n    \"c\": [\n      1,\n      2\n    ]\n  },\n  \"b\": 2\n}\n"),
    (r#"{"a":1,"b":2,"a":{"c":[1,2]}}"#, &["-c", "."], "{\"a\":{\"c\":[1,2]},\"b\":2}\n"),
    (r#"{"a":1,"b":2,"a":{"c":[1,2]}}"#, &["--indent", "1", "."], "{\n \"a\": {\n  \"c\": [\n   1,\n   2\n  ]\n },\n \"b\": 2\n}\n"),
    (r#"{"a":1,"b":2,"a":{"c":[1,2]}}"#, &["--tab", "."], "{\n\t\"a\": {\n\t\t\"c\": [\n\t\t\t1,\n\t\t\t2\n\t\t]\n\t},\n\t\"b\": 2\n}\n"),
    (r#"{"a":1,"b":2,"a":{"c":[1,2]}}"#, &["-c", "map(.)"], "[{\"c\":[1,2]},2]\n"),
    (r#"{"a":1,"b":2,"a":{"c":[1,2]}}"#, &["-c", "select(true)"], "{\"a\":{\"c\":[1,2]},\"b\":2}\n"),
    (r#"{"a":1,"b":2,"a":{"c":[1,2]}}"#, &["-c", ".[]"], "{\"c\":[1,2]}\n2\n"),
    (r#"{"a":1,"b":2,"a":{"c":[1,2]}}"#, &["-c", "[.[]]"], "[{\"c\":[1,2]},2]\n"),
    (r#"{"b":1,"a":2,"b":3,"a":4,"b":5}"#, &["."], "{\n  \"b\": 5,\n  \"a\": 4\n}\n"),
    (r#"{"b":1,"a":2,"b":3,"a":4,"b":5}"#, &["-c", "."], "{\"b\":5,\"a\":4}\n"),
    (r#"{"b":1,"a":2,"b":3,"a":4,"b":5}"#, &["--indent", "1", "."], "{\n \"b\": 5,\n \"a\": 4\n}\n"),
    (r#"{"b":1,"a":2,"b":3,"a":4,"b":5}"#, &["--tab", "."], "{\n\t\"b\": 5,\n\t\"a\": 4\n}\n"),
    (r#"{"b":1,"a":2,"b":3,"a":4,"b":5}"#, &["-c", "map(.)"], "[5,4]\n"),
    (r#"{"b":1,"a":2,"b":3,"a":4,"b":5}"#, &["-c", "select(true)"], "{\"b\":5,\"a\":4}\n"),
    (r#"{"b":1,"a":2,"b":3,"a":4,"b":5}"#, &["-c", ".[]"], "5\n4\n"),
    (r#"{"b":1,"a":2,"b":3,"a":4,"b":5}"#, &["-c", "[.[]]"], "[5,4]\n"),
    (r#"{"a\/b":1,"c":2,"a/b":3}"#, &["."], "{\n  \"a/b\": 3,\n  \"c\": 2\n}\n"),
    (r#"{"a\/b":1,"c":2,"a/b":3}"#, &["-c", "."], "{\"a/b\":3,\"c\":2}\n"),
    (r#"{"a\/b":1,"c":2,"a/b":3}"#, &["--indent", "1", "."], "{\n \"a/b\": 3,\n \"c\": 2\n}\n"),
    (r#"{"a\/b":1,"c":2,"a/b":3}"#, &["--tab", "."], "{\n\t\"a/b\": 3,\n\t\"c\": 2\n}\n"),
    (r#"{"a\/b":1,"c":2,"a/b":3}"#, &["-c", "map(.)"], "[3,2]\n"),
    (r#"{"a\/b":1,"c":2,"a/b":3}"#, &["-c", "select(true)"], "{\"a/b\":3,\"c\":2}\n"),
    (r#"{"a\/b":1,"c":2,"a/b":3}"#, &["-c", ".[]"], "3\n2\n"),
    (r#"{"a\/b":1,"c":2,"a/b":3}"#, &["-c", "[.[]]"], "[3,2]\n"),
    (r#"{"\u0061":1,"a":2}"#, &["."], "{\n  \"a\": 2\n}\n"),
    (r#"{"\u0061":1,"a":2}"#, &["-c", "."], "{\"a\":2}\n"),
    (r#"{"\u0061":1,"a":2}"#, &["--indent", "1", "."], "{\n \"a\": 2\n}\n"),
    (r#"{"\u0061":1,"a":2}"#, &["--tab", "."], "{\n\t\"a\": 2\n}\n"),
    (r#"{"\u0061":1,"a":2}"#, &["-c", "map(.)"], "[2]\n"),
    (r#"{"\u0061":1,"a":2}"#, &["-c", "select(true)"], "{\"a\":2}\n"),
    (r#"{"\u0061":1,"a":2}"#, &["-c", ".[]"], "2\n"),
    (r#"{"\u0061":1,"a":2}"#, &["-c", "[.[]]"], "[2]\n"),
    (r#"[{"x":1,"x":2},{"y":[{"z":1,"z":{"w":1,"w":2}}]}]"#, &["."], "[\n  {\n    \"x\": 2\n  },\n  {\n    \"y\": [\n      {\n        \"z\": {\n          \"w\": 2\n        }\n      }\n    ]\n  }\n]\n"),
    (r#"[{"x":1,"x":2},{"y":[{"z":1,"z":{"w":1,"w":2}}]}]"#, &["-c", "."], "[{\"x\":2},{\"y\":[{\"z\":{\"w\":2}}]}]\n"),
    (r#"[{"x":1,"x":2},{"y":[{"z":1,"z":{"w":1,"w":2}}]}]"#, &["--indent", "1", "."], "[\n {\n  \"x\": 2\n },\n {\n  \"y\": [\n   {\n    \"z\": {\n     \"w\": 2\n    }\n   }\n  ]\n }\n]\n"),
    (r#"[{"x":1,"x":2},{"y":[{"z":1,"z":{"w":1,"w":2}}]}]"#, &["--tab", "."], "[\n\t{\n\t\t\"x\": 2\n\t},\n\t{\n\t\t\"y\": [\n\t\t\t{\n\t\t\t\t\"z\": {\n\t\t\t\t\t\"w\": 2\n\t\t\t\t}\n\t\t\t}\n\t\t]\n\t}\n]\n"),
    (r#"[{"x":1,"x":2},{"y":[{"z":1,"z":{"w":1,"w":2}}]}]"#, &["-c", "map(.)"], "[{\"x\":2},{\"y\":[{\"z\":{\"w\":2}}]}]\n"),
    (r#"[{"x":1,"x":2},{"y":[{"z":1,"z":{"w":1,"w":2}}]}]"#, &["-c", "select(true)"], "[{\"x\":2},{\"y\":[{\"z\":{\"w\":2}}]}]\n"),
    (r#"[{"x":1,"x":2},{"y":[{"z":1,"z":{"w":1,"w":2}}]}]"#, &["-c", ".[]"], "{\"x\":2}\n{\"y\":[{\"z\":{\"w\":2}}]}\n"),
    (r#"[{"x":1,"x":2},{"y":[{"z":1,"z":{"w":1,"w":2}}]}]"#, &["-c", "[.[]]"], "[{\"x\":2},{\"y\":[{\"z\":{\"w\":2}}]}]\n"),
    (r#"{"key-one-padding!":1,"kwjfpwvojvWuiqDf":2}"#, &["."], "{\n  \"key-one-padding!\": 1,\n  \"kwjfpwvojvWuiqDf\": 2\n}\n"),
    (r#"{"key-one-padding!":1,"kwjfpwvojvWuiqDf":2}"#, &["-c", "."], "{\"key-one-padding!\":1,\"kwjfpwvojvWuiqDf\":2}\n"),
    (r#"{"key-one-padding!":1,"kwjfpwvojvWuiqDf":2}"#, &["--indent", "1", "."], "{\n \"key-one-padding!\": 1,\n \"kwjfpwvojvWuiqDf\": 2\n}\n"),
    (r#"{"key-one-padding!":1,"kwjfpwvojvWuiqDf":2}"#, &["--tab", "."], "{\n\t\"key-one-padding!\": 1,\n\t\"kwjfpwvojvWuiqDf\": 2\n}\n"),
    (r#"{"key-one-padding!":1,"kwjfpwvojvWuiqDf":2}"#, &["-c", "map(.)"], "[1,2]\n"),
    (r#"{"key-one-padding!":1,"kwjfpwvojvWuiqDf":2}"#, &["-c", "select(true)"], "{\"key-one-padding!\":1,\"kwjfpwvojvWuiqDf\":2}\n"),
    (r#"{"key-one-padding!":1,"kwjfpwvojvWuiqDf":2}"#, &["-c", ".[]"], "1\n2\n"),
    (r#"{"key-one-padding!":1,"kwjfpwvojvWuiqDf":2}"#, &["-c", "[.[]]"], "[1,2]\n"),
    (r#"{"key-one-padding!":1,"kwjfpwvojvWuiqDf":2,"key-one-padding!":3}"#, &["."], "{\n  \"key-one-padding!\": 3,\n  \"kwjfpwvojvWuiqDf\": 2\n}\n"),
    (r#"{"key-one-padding!":1,"kwjfpwvojvWuiqDf":2,"key-one-padding!":3}"#, &["-c", "."], "{\"key-one-padding!\":3,\"kwjfpwvojvWuiqDf\":2}\n"),
    (r#"{"key-one-padding!":1,"kwjfpwvojvWuiqDf":2,"key-one-padding!":3}"#, &["--indent", "1", "."], "{\n \"key-one-padding!\": 3,\n \"kwjfpwvojvWuiqDf\": 2\n}\n"),
    (r#"{"key-one-padding!":1,"kwjfpwvojvWuiqDf":2,"key-one-padding!":3}"#, &["--tab", "."], "{\n\t\"key-one-padding!\": 3,\n\t\"kwjfpwvojvWuiqDf\": 2\n}\n"),
    (r#"{"key-one-padding!":1,"kwjfpwvojvWuiqDf":2,"key-one-padding!":3}"#, &["-c", "map(.)"], "[3,2]\n"),
    (r#"{"key-one-padding!":1,"kwjfpwvojvWuiqDf":2,"key-one-padding!":3}"#, &["-c", "select(true)"], "{\"key-one-padding!\":3,\"kwjfpwvojvWuiqDf\":2}\n"),
    (r#"{"key-one-padding!":1,"kwjfpwvojvWuiqDf":2,"key-one-padding!":3}"#, &["-c", ".[]"], "3\n2\n"),
    (r#"{"key-one-padding!":1,"kwjfpwvojvWuiqDf":2,"key-one-padding!":3}"#, &["-c", "[.[]]"], "[3,2]\n"),
    (r#"{"key-one-padding!":1,"kwjfpwvojvWuiqDf":2,"i7f35QhA5RFytU=l":3,"MO7swvZKnn*|^JFk":4,"kwjfpwvojvWuiqDf":5,"MO7swvZKnn*|^JFk":6,"key-one-padding!":7}"#, &["."], "{\n  \"key-one-padding!\": 7,\n  \"kwjfpwvojvWuiqDf\": 5,\n  \"i7f35QhA5RFytU=l\": 3,\n  \"MO7swvZKnn*|^JFk\": 6\n}\n"),
    (r#"{"key-one-padding!":1,"kwjfpwvojvWuiqDf":2,"i7f35QhA5RFytU=l":3,"MO7swvZKnn*|^JFk":4,"kwjfpwvojvWuiqDf":5,"MO7swvZKnn*|^JFk":6,"key-one-padding!":7}"#, &["-c", "."], "{\"key-one-padding!\":7,\"kwjfpwvojvWuiqDf\":5,\"i7f35QhA5RFytU=l\":3,\"MO7swvZKnn*|^JFk\":6}\n"),
    (r#"{"key-one-padding!":1,"kwjfpwvojvWuiqDf":2,"i7f35QhA5RFytU=l":3,"MO7swvZKnn*|^JFk":4,"kwjfpwvojvWuiqDf":5,"MO7swvZKnn*|^JFk":6,"key-one-padding!":7}"#, &["--indent", "1", "."], "{\n \"key-one-padding!\": 7,\n \"kwjfpwvojvWuiqDf\": 5,\n \"i7f35QhA5RFytU=l\": 3,\n \"MO7swvZKnn*|^JFk\": 6\n}\n"),
    (r#"{"key-one-padding!":1,"kwjfpwvojvWuiqDf":2,"i7f35QhA5RFytU=l":3,"MO7swvZKnn*|^JFk":4,"kwjfpwvojvWuiqDf":5,"MO7swvZKnn*|^JFk":6,"key-one-padding!":7}"#, &["--tab", "."], "{\n\t\"key-one-padding!\": 7,\n\t\"kwjfpwvojvWuiqDf\": 5,\n\t\"i7f35QhA5RFytU=l\": 3,\n\t\"MO7swvZKnn*|^JFk\": 6\n}\n"),
    (r#"{"key-one-padding!":1,"kwjfpwvojvWuiqDf":2,"i7f35QhA5RFytU=l":3,"MO7swvZKnn*|^JFk":4,"kwjfpwvojvWuiqDf":5,"MO7swvZKnn*|^JFk":6,"key-one-padding!":7}"#, &["-c", "map(.)"], "[7,5,3,6]\n"),
    (r#"{"key-one-padding!":1,"kwjfpwvojvWuiqDf":2,"i7f35QhA5RFytU=l":3,"MO7swvZKnn*|^JFk":4,"kwjfpwvojvWuiqDf":5,"MO7swvZKnn*|^JFk":6,"key-one-padding!":7}"#, &["-c", "select(true)"], "{\"key-one-padding!\":7,\"kwjfpwvojvWuiqDf\":5,\"i7f35QhA5RFytU=l\":3,\"MO7swvZKnn*|^JFk\":6}\n"),
    (r#"{"key-one-padding!":1,"kwjfpwvojvWuiqDf":2,"i7f35QhA5RFytU=l":3,"MO7swvZKnn*|^JFk":4,"kwjfpwvojvWuiqDf":5,"MO7swvZKnn*|^JFk":6,"key-one-padding!":7}"#, &["-c", ".[]"], "7\n5\n3\n6\n"),
    (r#"{"key-one-padding!":1,"kwjfpwvojvWuiqDf":2,"i7f35QhA5RFytU=l":3,"MO7swvZKnn*|^JFk":4,"kwjfpwvojvWuiqDf":5,"MO7swvZKnn*|^JFk":6,"key-one-padding!":7}"#, &["-c", "[.[]]"], "[7,5,3,6]\n"),
];

#[test]
fn test_repeated_keys_collapse_on_every_streaming_route_7311() -> Result<()> {
    for &(document, args, stdout) in ROWS {
        let (out, err, code) = run_jq_full(args, Some(document))?;
        assert_eq!(
            (out.as_str(), err.as_str(), code),
            (stdout, "", 0),
            "{args:?} on {document}"
        );
    }
    Ok(())
}

/// An object as `(key, value)` pairs, as JSON text.
fn object(fields: &[(String, u32)]) -> String {
    let body: Vec<String> = fields.iter().map(|(k, v)| format!("\"{k}\":{v}")).collect();
    format!("{{{}}}", body.join(","))
}

/// `fields` under jq's rule: the first position of each key, holding its last value.
fn collapsed(fields: &[(String, u32)]) -> Vec<(String, u32)> {
    let mut out: Vec<(String, u32)> = Vec::new();
    for (key, value) in fields {
        match out.iter_mut().find(|(k, _)| k == key) {
            Some(slot) => slot.1 = *value,
            None => out.push((key.clone(), *value)),
        }
    }
    out
}

/// Objects wider than the pairwise checks and the hash prefilter's 128-key floor, so the hash
/// lists decide: one repeat at the front, one at the end, a colliding pair among them with and
/// without a repeat, and eight keys repeated 500 times each. Every route prints each one exactly
/// as it prints the collapsed object, which repeats nothing and so is echoed from its text.
#[test]
fn test_wide_repeated_keys_print_as_their_collapsed_object_7311() -> Result<()> {
    let numbered =
        |n: u32| -> Vec<(String, u32)> { (0..n).map(|i| (format!("k{i}"), i)).collect() };
    let mut front = numbered(300);
    front.insert(2, ("k0".to_string(), 1000));
    let mut end = numbered(300);
    end.push(("k0".to_string(), 1000));
    let mut colliding = numbered(300);
    colliding.push(("key-one-padding!".to_string(), 1001));
    colliding.push(("kwjfpwvojvWuiqDf".to_string(), 1002));
    let mut colliding_repeat = colliding.clone();
    colliding_repeat.push(("key-one-padding!".to_string(), 1003));
    colliding_repeat.push(("k7".to_string(), 1004));
    let many: Vec<(String, u32)> = (0..4000).map(|i| (format!("k{}", i % 8), i)).collect();

    let routes: [&[&str]; 8] = [
        &["."],
        &["-c", "."],
        &["--indent", "1", "."],
        &["--tab", "."],
        &["-C", "."],
        &["-c", "map(.)"],
        &["-c", "select(true)"],
        &["-c", ".[]"],
    ];
    for fields in [&front, &end, &colliding, &colliding_repeat, &many] {
        let document = object(fields);
        let expected = object(&collapsed(fields));
        for args in routes {
            let got = run_jq_full(args, Some(&document))?;
            let want = run_jq_full(args, Some(&expected))?;
            assert_eq!(got.2, 0, "{args:?}: {}", got.1);
            assert_eq!(got, want, "{args:?} over {} fields", fields.len());
        }
    }
    // The collapsed objects themselves, so the comparison above is not between two wrong answers.
    let (out, _, _) = run_jq_full(&["-c", "[.[]] | .[:3], length"], Some(&object(&end)))?;
    assert_eq!(out, "[1000,1,2]\n300\n");
    let (out, _, _) = run_jq_full(&["-c", "[.[]] | .[:4], length"], Some(&object(&front)))?;
    assert_eq!(out, "[1000,1,2,3]\n300\n");
    let (out, _, _) = run_jq_full(
        &["-c", "[.[]] | .[-3:], length"],
        Some(&object(&colliding_repeat)),
    )?;
    assert_eq!(out, "[299,1003,1002]\n302\n");
    let (out, _, _) = run_jq_full(&["-c", "."], Some(&object(&many)))?;
    assert_eq!(
        out,
        "{\"k0\":3992,\"k1\":3993,\"k2\":3994,\"k3\":3995,\"k4\":3996,\"k5\":3997,\"k6\":3998,\"k7\":3999}\n"
    );
    Ok(())
}
