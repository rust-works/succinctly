//! #7260: two per-record walks of an object were cut down. An array of field reads (`[.id, .name]`)
//! walks the object's members once for every name, where each read walked them all again, and jq's
//! `keys` of an object whose keys are all escape-free strings sorts them and drops a repeat, without
//! the hash probe that collapsed a repeat as the walk went. Every expectation here was captured from
//! jq 1.7.1.

use anyhow::Result;

use crate::run_jq_full;

fn jq(filter: &str, input: &str) -> Result<(String, String, i32)> {
    run_jq_full(&["-c", filter], Some(input))
}

/// What each read answers on its own: the last of a repeated key, `null` for a missing one, a name read
/// twice, a container value, more names than one walk takes, a key spelled with an escape, and every
/// document of a stream.
#[test]
fn an_array_of_field_reads_answers_each_name_as_its_own_read_7260() -> Result<()> {
    let rows: &[(&str, &str, &str)] = &[
        ("[.a,.b]", r#"{"b":1,"a":2,"b":3}"#, "[2,3]\n"),
        ("[.a,.zz,.a]", r#"{"a":1}"#, "[1,null,1]\n"),
        ("[.a,.b]", "null", "[null,null]\n"),
        (
            "[.a,.b]",
            r#"{"a":"x","b":{"c":[1,2]}}"#,
            "[\"x\",{\"c\":[1,2]}]\n",
        ),
        (
            "[.a,.b,.c,.d,.e,.f,.g,.h,.i]",
            r#"{"i":9,"a":1,"h":8}"#,
            "[1,null,null,null,null,null,null,8,9]\n",
        ),
        ("[.a,.b]", r#"{"\u0061":1,"a":2,"\u0062":3}"#, "[2,3]\n"),
        ("[.a,.b]", r#"{"a":1} {"a":2,"b":3}"#, "[1,null]\n[2,3]\n"),
        ("[.a,.b]?", "3", ""),
    ];
    for (filter, input, expected) in rows {
        assert_eq!(
            jq(filter, input)?,
            (String::from(*expected), String::new(), 0),
            "{filter} over {input}"
        );
    }
    Ok(())
}

/// A wide object (64 members or more) is registered for the key index by the walk that answers the
/// array, and the reads after it answer the same.
#[test]
fn an_array_of_field_reads_of_a_wide_object_answers_each_name_7260() -> Result<()> {
    let members: Vec<String> = (0..70).map(|i| format!("\"k{i}\":{i}")).collect();
    let input = format!("{{{}}}", members.join(","));
    assert_eq!(
        jq("[.k0,.k69], .k5, [.k1,.k68]", &input)?,
        ("[0,69]\n5\n[1,68]\n".to_string(), String::new(), 0)
    );
    Ok(())
}

/// The errors each read would raise: a type error for an input that is not an object, and jq's parse
/// error for a malformed object, whichever member is at fault.
#[test]
fn an_array_of_field_reads_raises_what_a_read_raises_7260() -> Result<()> {
    let rows: &[(&str, &str)] = &[
        (
            "[1]",
            "jq: error (at <stdin>:0): Cannot index array with string \"a\"\n",
        ),
        (
            "3",
            "jq: error (at <stdin>:0): Cannot index number with string \"a\"\n",
        ),
        (
            r#"{"a":1,}"#,
            "jq: parse error: Expected another key-value pair at line 1, column 8\n",
        ),
        (
            "{,}",
            "jq: parse error: Expected value before ',' at line 1, column 2\n",
        ),
        (
            r#"{"a":1,123:2}"#,
            "jq: parse error: Object keys must be strings at line 1, column 11\n",
        ),
        (
            r#"{"a" 1,"b":2}"#,
            "jq: parse error: Expected separator between values at line 1, column 7\n",
        ),
    ];
    for (input, stderr) in rows {
        assert_eq!(
            jq("[.a,.b]", input)?,
            (String::new(), String::from(*stderr), 5),
            "{input}"
        );
    }
    Ok(())
}

/// `keys` sorted by code point with a repeated key once, whether the repeat is spelled the same or
/// with an escape (which the escape-free walk leaves to the collapsing one), and a key holding an
/// escaped NUL or a DEL, which is printed escaped.
#[test]
fn keys_sorts_and_collapses_as_jq_does_7260() -> Result<()> {
    let rows: &[(&str, &str, &str)] = &[
        ("keys", r#"{"b":1,"a":2,"b":3}"#, "[\"a\",\"b\"]\n"),
        (
            "keys",
            r#"{"é":1,"e":2,"z":3,"E":4}"#,
            "[\"E\",\"e\",\"z\",\"é\"]\n",
        ),
        ("keys", r#"{"\u0062":1,"b":2,"a":3}"#, "[\"a\",\"b\"]\n"),
        (
            "keys",
            r#"{"a":1,"a":2,"a\u0000":3}"#,
            "[\"a\",\"a\\u0000\"]\n",
        ),
        ("keys", "{\"\x7f\":1,\"\":2}", "[\"\",\"\\u007f\"]\n"),
        ("keys", "{}", "[]\n"),
        (
            "[keys[]], (keys|length)",
            r#"{"b":1,"a":2,"b":3}"#,
            "[\"a\",\"b\"]\n2\n",
        ),
    ];
    for (filter, input, expected) in rows {
        assert_eq!(
            jq(filter, input)?,
            (String::from(*expected), String::new(), 0),
            "{filter} over {input}"
        );
    }
    let faults: &[(&str, &str)] = &[
        (
            r#"{"a":1,}"#,
            "jq: parse error: Expected another key-value pair at line 1, column 8\n",
        ),
        (
            r#"{"a" 1}"#,
            "jq: parse error: Expected separator between values at line 1, column 7\n",
        ),
    ];
    for (input, stderr) in faults {
        assert_eq!(
            jq("keys", input)?,
            (String::new(), String::from(*stderr), 5),
            "{input}"
        );
    }
    Ok(())
}
