//! #6099: `sort_by(key)` on a mapping sorts its entries by the key, as yq's "Sort a map by keys"
//! example does. The key filter ran on each entry's value, where `key` names nothing, so the order
//! came back unchanged. Every row is what yq v4.53.3 printed for the same program (`-o=json -I0`).

use anyhow::Result;

use crate::run_yq_stdin;

fn check(rows: &[(&str, &str, &str)]) -> Result<()> {
    for (doc, filter, want) in rows {
        let (stdout, code) = run_yq_stdin(filter, doc, &["-o=json", "-I0"])?;
        assert_eq!(code, 0, "`{filter}` failed");
        assert_eq!(&stdout, want, "`{filter}` on {doc:?}");
    }
    Ok(())
}

#[test]
fn a_mapping_sorts_by_its_string_keys() -> Result<()> {
    check(&[
        (
            "z: 1\nY: 2\nx: 3\nww: 4\n",
            "sort_by(key)",
            "{\"Y\":2,\"ww\":4,\"x\":3,\"z\":1}\n",
        ),
        (
            "z: 1\nY: 2\nx: 3\nww: 4\n",
            "sort_by(key | downcase)",
            "{\"ww\":4,\"x\":3,\"Y\":2,\"z\":1}\n",
        ),
        (
            "z: 1\nY: 2\nx: 3\nww: 4\n",
            "sort_by(key | length)",
            "{\"z\":1,\"Y\":2,\"x\":3,\"ww\":4}\n",
        ),
        (
            "z: 1\nY: 2\nx: 3\nww: 4\n",
            "sort_by((key | length), key)",
            "{\"Y\":2,\"x\":3,\"z\":1,\"ww\":4}\n",
        ),
        (
            "z: 1\nY: 2\nx: 3\nww: 4\n",
            "sort_by(key, .)",
            "{\"Y\":2,\"ww\":4,\"x\":3,\"z\":1}\n",
        ),
    ])
}

#[test]
fn a_mapping_sorts_by_its_typed_keys() -> Result<()> {
    check(&[
        (
            "10: a\n9: b\nx: c\ntrue: d\n1.5: e\nnull: f\n",
            "sort_by(key)",
            "{\"null\":\"f\",\"true\":\"d\",\"1.5\":\"e\",\"9\":\"b\",\"10\":\"a\",\"x\":\"c\"}\n",
        ),
        (
            "10: a\n9: b\nx: c\ntrue: d\n1.5: e\nnull: f\n",
            "sort_by(key | tag)",
            "{\"true\":\"d\",\"1.5\":\"e\",\"10\":\"a\",\"9\":\"b\",\"null\":\"f\",\"x\":\"c\"}\n",
        ),
        (
            "10: a\n9: b\nx: c\ntrue: d\n1.5: e\nnull: f\n",
            "sort_by(key, .)",
            "{\"null\":\"f\",\"true\":\"d\",\"1.5\":\"e\",\"9\":\"b\",\"10\":\"a\",\"x\":\"c\"}\n",
        ),
        (
            "10: a\n9: b\nx: c\ntrue: d\n1.5: e\nnull: f\n",
            "sort_by(key | length)",
            "{\"null\":\"f\",\"9\":\"b\",\"x\":\"c\",\"10\":\"a\",\"1.5\":\"e\",\"true\":\"d\"}\n",
        ),
    ])
}

#[test]
fn a_sort_by_nested_in_an_update_reads_the_key() -> Result<()> {
    check(&[(
        "a: {z: 1, y: 2}\n",
        ".a |= sort_by(key)",
        "{\"a\":{\"y\":2,\"z\":1}}\n",
    )])
}

#[test]
fn a_key_below_another_step_is_not_the_entrys() -> Result<()> {
    check(&[
        (
            "z: 1\nY: 2\nx: 3\nww: 4\n",
            "sort_by(.a | key)",
            "{\"z\":1,\"Y\":2,\"x\":3,\"ww\":4}\n",
        ),
        (
            "z: 1\nY: 2\nx: 3\nww: 4\n",
            "sort_by(key | key)",
            "{\"z\":1,\"Y\":2,\"x\":3,\"ww\":4}\n",
        ),
    ])
}

#[test]
fn sorting_by_the_value_is_unchanged() -> Result<()> {
    check(&[(
        "z: 1\nY: 2\nx: 3\nww: 4\n",
        "sort_by(.)",
        "{\"z\":1,\"Y\":2,\"x\":3,\"ww\":4}\n",
    )])
}

/// Comments, an anchor and its alias, a tag and a flow mapping travel with their entries.
#[test]
fn the_presentation_of_each_entry_follows_it() -> Result<()> {
    let doc = "# top\nz: 1 # one\nY: &x 2\nx: *x\nww: !!str 4\n\"q\": {a: 1}\n";
    for (filter, want) in [
        (
            "sort_by(key)",
            "# top\nY: &x 2\n\"q\": {a: 1}\nww: !!str 4\nx: *x\nz: 1 # one\n",
        ),
        (
            "sort_by(key | length)",
            "# top\nz: 1 # one\nY: &x 2\nx: *x\n\"q\": {a: 1}\nww: !!str 4\n",
        ),
        (
            "sort_by(key | tag)",
            "# top\nz: 1 # one\nY: &x 2\nx: *x\nww: !!str 4\n\"q\": {a: 1}\n",
        ),
    ] {
        let (stdout, code) = run_yq_stdin(filter, doc, &[])?;
        assert_eq!(code, 0, "`{filter}` failed");
        assert_eq!(stdout, want, "`{filter}`");
    }
    Ok(())
}

/// A comma binds the `key` stages it can and leaves the ones below another step.
#[test]
fn a_comma_binds_the_keys_it_can() -> Result<()> {
    check(&[
        (
            "z: 1\nY: 2\nx: 3\nww: 4\n",
            "sort_by(key, (.a | key))",
            "{\"Y\":2,\"ww\":4,\"x\":3,\"z\":1}\n",
        ),
        (
            "z: 1\nY: 2\nx: 3\nww: 4\n",
            "sort_by((.a | key), key)",
            "{\"Y\":2,\"ww\":4,\"x\":3,\"z\":1}\n",
        ),
        (
            "z: 1\nY: 2\nx: 3\nww: 4\n",
            "sort_by(.b, key)",
            "{\"Y\":2,\"ww\":4,\"x\":3,\"z\":1}\n",
        ),
        (
            "z: 1\nY: 2\nx: 3\nww: 4\n",
            "sort_by(key, length)",
            "{\"Y\":2,\"ww\":4,\"x\":3,\"z\":1}\n",
        ),
    ])
}
