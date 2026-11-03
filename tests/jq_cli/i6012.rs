//! #6012: `delpaths` over a string, or a `null`, with a path of two or more components words its
//! error as jq does. jq reads each key it has more path under with `jv_get` before it reaches the
//! next component: on a string an object key is a slice descriptor (so a bad one is `Array/string
//! slice indices must be integers`, and a good one names a substring the rest of the path cannot
//! delete from, `Cannot delete fields from string`), and on a `null` root a `null`, boolean or array
//! key is `Cannot index null with <kind>`. A `null` child never gets that far, because jq skips it
//! before it descends. Every answer was captured from jq 1.7.1 (`/usr/bin/jq`).

use anyhow::Result;

use crate::run_jq_null;

#[test]
fn delpaths_reads_the_first_key_of_a_longer_path_as_jv_get_does() -> Result<()> {
    for (filter, want) in [
        ("\"abc\" | try delpaths([[{\"a\":1},{\"a\":2}]]) catch .", "\"Array/string slice indices must be integers\"\n"),
        ("\"abc\" | try delpaths([[{\"a\":1},0]]) catch .", "\"Array/string slice indices must be integers\"\n"),
        ("\"abc\" | try delpaths([[{\"a\":1},\"x\"]]) catch .", "\"Array/string slice indices must be integers\"\n"),
        ("\"abc\" | try delpaths([[{\"start\":1,\"end\":2},0]]) catch .", "\"Cannot delete fields from string\"\n"),
        ("\"abc\" | try delpaths([[{\"start\":1,\"end\":2},{\"start\":0,\"end\":1}]]) catch .", "\"Cannot delete fields from string\"\n"),
        ("\"abc\" | try delpaths([[{\"start\":null,\"end\":2},0]]) catch .", "\"Cannot delete fields from string\"\n"),
        ("\"abc\" | try delpaths([[{\"start\":1,\"end\":2},\"x\"]]) catch .", "\"Cannot delete fields from string\"\n"),
        ("\"abc\" | try delpaths([[{\"start\":\"a\",\"end\":2},0]]) catch .", "\"Array/string slice indices must be integers\"\n"),
        ("\"abc\" | try delpaths([[{},0]]) catch .", "\"Array/string slice indices must be integers\"\n"),
        ("\"abc\" | try delpaths([[{\"start\":1},0]]) catch .", "\"Array/string slice indices must be integers\"\n"),
        ("\"abc\" | try delpaths([[{\"start\":0,\"end\":3},0]]) catch .", "\"Cannot delete fields from string\"\n"),
        ("\"abc\" | try delpaths([[{\"start\":1,\"end\":2},{\"start\":0,\"end\":1},0]]) catch .", "\"Cannot delete fields from string\"\n"),
        ("\"abc\" | try delpaths([[{\"a\":1},0,0]]) catch .", "\"Array/string slice indices must be integers\"\n"),
        ("\"abc\" | try delpaths([[{\"start\":5,\"end\":9},0]]) catch .", "\"Cannot delete fields from string\"\n"),
        ("\"abc\" | try delpaths([[{\"start\":1.5,\"end\":2.5},0]]) catch .", "\"Cannot delete fields from string\"\n"),
        ("\"abc\" | try delpaths([[{\"start\":1,\"end\":2},0],[{\"start\":0,\"end\":1},0]]) catch .", "\"Cannot delete fields from string\"\n"),
        ("\"abc\" | try delpaths([[{\"a\":1}]]) catch .", "\"Cannot delete fields from string\"\n"),
        ("\"abc\" | try delpaths([[0,0]]) catch .", "\"Cannot index string with number\"\n"),
        ("\"abc\" | try delpaths([[{\"start\":1,\"end\":2}]]) catch .", "\"Cannot delete fields from string\"\n"),
        ("\"\" | try delpaths([[{\"a\":1},0]]) catch .", "\"Array/string slice indices must be integers\"\n"),
        ("\"héllo\" | try delpaths([[{\"start\":1,\"end\":3},0]]) catch .", "\"Cannot delete fields from string\"\n"),
        ("\"héllo\" | try delpaths([[{\"start\":\"x\",\"end\":3},0]]) catch .", "\"Array/string slice indices must be integers\"\n"),
        ("null | try delpaths([[null,0]]) catch .", "\"Cannot index null with null\"\n"),
        ("null | try delpaths([[[1],0]]) catch .", "\"Cannot index null with array\"\n"),
        ("null | try delpaths([[true,0]]) catch .", "\"Cannot index null with boolean\"\n"),
        ("null | try delpaths([[false,\"a\"]]) catch .", "\"Cannot index null with boolean\"\n"),
        ("null | try delpaths([[null,\"a\"]]) catch .", "\"Cannot index null with null\"\n"),
        ("null | try delpaths([[\"a\",null,0]]) catch .", "null\n"),
        ("null | try delpaths([[null]]) catch .", "null\n"),
        ("null | try delpaths([[true],[null,0]]) catch .", "\"Cannot index null with null\"\n"),
        ("null | try delpaths([[[1]],[0,0]]) catch .", "null\n"),
        ("null | try delpaths([[{\"a\":1},0]]) catch .", "null\n"),
        ("null | try delpaths([[0,0]]) catch .", "null\n"),
        ("null | try delpaths([[\"a\",0]]) catch .", "null\n"),
        ("[null,[1]] | try delpaths([[0,[1],0]]) catch .", "[null,[1]]\n"),
        ("{\"a\":null} | try delpaths([[\"a\",[1],0]]) catch .", "{\"a\":null}\n"),
        ("[1,2,3] | try delpaths([[{\"a\":1},0]]) catch .", "\"Array/string slice indices must be integers\"\n"),
        ("1 | try delpaths([[{\"a\":1},0]]) catch .", "\"Cannot index number with object\"\n"),
        ("{\"a\":\"abc\"} | try delpaths([[\"a\",{\"a\":1},0]]) catch .", "\"Array/string slice indices must be integers\"\n"),
        ("{\"a\":\"abc\"} | try delpaths([[\"a\",{\"start\":1,\"end\":2},0]]) catch .", "\"Cannot delete fields from string\"\n"),
        ("[[\"abc\"]] | try delpaths([[0,0,{\"a\":1},0]]) catch .", "\"Array/string slice indices must be integers\"\n"),
        ("\"abc\" | try del(.[{\"a\":1}][0]) catch .", "\"Array/string slice indices must be integers\"\n"),
        ("\"abc\" | try del(.[1:2][0]) catch .", "\"Cannot index string with number\"\n"),
        ("null | try del(.[null].a) catch .", "\"Cannot index null with null\"\n"),
        ("null | try del(.[true][0]) catch .", "\"Cannot index null with boolean\"\n"),
        ("null | try del(.[[1]][0]) catch .", "\"Cannot index null with array\"\n"),
        ("{\"a\":null} | try delpaths([[\"a\",null,0]]) catch .", "{\"a\":null}\n"),
        ("[null] | try delpaths([[0,null,0]]) catch .", "[null]\n"),
    ] {
        let (out, code) = run_jq_null(filter, &["-c"])?;
        assert_eq!((out.as_str(), code), (want, 0), "{filter}");
    }
    Ok(())
}

/// The uncaught spelling, which is what the fuzz found (seed 9207).
#[test]
fn an_uncaught_delpaths_error_names_the_slice_wording() -> Result<()> {
    let (_, code) = run_jq_null(r#""abc" | delpaths([[{"a":1},{"a":2}]])"#, &["-c"])?;
    assert_eq!(code, 5);
    Ok(())
}
