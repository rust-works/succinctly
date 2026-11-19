//! #7260: `map_values`, `to_entries` and `with_entries` over an object walk its fields in place of
//! a list of every field, when the object is well formed and repeats no key. These pin what the
//! list gave on both sides of that gate, over objects wider than the census's small-object and
//! prefilter cut-offs: the collapse of a repeated key (first position, last value), and a
//! malformed member raising before `f` has run on any field. Captured from `/usr/bin/jq` 1.7.1.

use anyhow::Result;

use crate::run_jq_full;

/// An object of `n` fields whose keys are `k{i % distinct}` and whose values are `i`, with
/// `first` in place of the first value and `tail` before the closing `}`.
fn wide(n: usize, distinct: usize, first: &str, tail: &str) -> String {
    let fields: Vec<String> = (0..n)
        .map(|i| {
            let value = if i == 0 {
                first.to_string()
            } else {
                i.to_string()
            };
            format!("\"k{}\":{value}", i % distinct)
        })
        .collect();
    format!("{{{}{tail}}}", fields.join(","))
}

/// The collapsed fields of [`wide`] without `first` or `tail`: each key at its first position,
/// with its last value, plus one.
fn collapsed_plus_one(n: usize, distinct: usize) -> Vec<(String, usize)> {
    let mut out: Vec<(String, usize)> = Vec::new();
    for i in 0..n {
        let key = format!("k{}", i % distinct);
        match out.iter_mut().find(|(k, _)| *k == key) {
            Some(slot) => slot.1 = i + 1,
            None => out.push((key, i + 1)),
        }
    }
    out
}

#[test]
fn wide_object_builtins_match_jq_with_and_without_repeated_keys_7260() -> Result<()> {
    // 300 distinct keys; then 300 fields over 250 keys (the last 50 repeat the first 50); then
    // 3,000 fields over 7 keys.
    for (n, distinct) in [(300, 300), (300, 250), (3000, 7)] {
        let input = wide(n, distinct, "0", "");
        let fields = collapsed_plus_one(n, distinct);
        let object = |value: &dyn Fn(usize) -> String| {
            let members: Vec<String> = fields
                .iter()
                .map(|(k, v)| format!("\"{k}\":{}", value(*v)))
                .collect();
            format!("{{{}}}\n", members.join(","))
        };
        let plus_one = object(&|v| v.to_string());
        let entries: Vec<String> = fields
            .iter()
            .map(|(k, v)| format!("{{\"key\":\"{k}\",\"value\":{}}}", v - 1))
            .collect();
        let entries = format!("[{}]\n", entries.join(","));
        for (filter, want) in [
            ("map_values(.+1)", plus_one.clone()),
            ("with_entries(.value+=1)", plus_one),
            ("to_entries", entries),
            ("to_entries|length", format!("{}\n", fields.len())),
        ] {
            let (stdout, stderr, code) = run_jq_full(&["-c", filter], Some(&input))?;
            assert_eq!(
                (stdout.as_str(), stderr.as_str(), code),
                (want.as_str(), "", 0),
                "{filter} over {n} fields, {distinct} keys"
            );
        }
    }
    Ok(())
}

#[test]
fn malformed_wide_object_raises_before_f_runs_7260() -> Result<()> {
    // The first value is a string, so `.+1` on it would raise "string ... cannot be added": the
    // malformed member must be reported instead, as jq's parser reports it before any filter runs.
    for distinct in [300, 250] {
        for (tail, message) in [
            (",", "Expected another key-value pair"),
            (",\"z\"", "Objects must consist of key:value pairs"),
        ] {
            let input = wide(300, distinct, "\"x\"", tail);
            let want = format!(
                "jq: parse error: {message} at line 1, column {}\n",
                input.len()
            );
            for filter in [
                "map_values(.+1)",
                "to_entries",
                "with_entries(.value+=1)",
                "try map_values(.+1) catch \"caught\"",
            ] {
                let (stdout, stderr, code) = run_jq_full(&["-c", filter], Some(&input))?;
                assert_eq!(
                    (stdout.as_str(), stderr.as_str(), code),
                    ("", want.as_str(), 5),
                    "{filter} over {distinct} keys, tail {tail:?}"
                );
            }
        }
    }
    Ok(())
}
