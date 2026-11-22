//! #7789: `del(.. | f)` and `delpaths([path(.. | f)])` held a multiple of jq's memory. A stage
//! after a descent rebuilt every result's whole path chain instead of sharing the node it
//! extends, the delete trie gave every node two maps and three lists, and `delpaths`' argument
//! held its paths three times over. These rows pin that the answers did not move: every one was
//! captured from `/usr/bin/jq` 1.7.1, and `/opt/homebrew/bin/jq` 1.8.2 answers the same, but for
//! the value preview in the `Cannot index string with number` message (jqlang/jq#3478, where
//! 1.7.1 stays the reference). The memory itself is pinned by the unit tests beside the code
//! (`del_path_memory_tests_7789`).

use anyhow::Result;

use crate::run_jq_full;

const DOC: &str = r#"[{"id":1,"pad":"p","n":{"a":[1,2,{"b":"x","c":null,"pad":3}]}},{"id":2,"n":{"pad":[4]}},"s",null,[{"pad":null}]]"#;

/// (filter, stdout, exit status) over [`DOC`].
const ROWS: &[(&str, &str, i32)] = &[
    (
        "del(.. | .pad?)",
        "[{\"id\":1,\"n\":{\"a\":[1,2,{\"b\":\"x\",\"c\":null}]}},{\"id\":2,\"n\":{}},\"s\",null,[{}]]\n",
        0,
    ),
    (
        "del(..|strings)",
        "[{\"id\":1,\"n\":{\"a\":[1,2,{\"c\":null,\"pad\":3}]}},{\"id\":2,\"n\":{\"pad\":[4]}},null,[{\"pad\":null}]]\n",
        0,
    ),
    (
        "del(.. | select(. == null))",
        "[{\"id\":1,\"pad\":\"p\",\"n\":{\"a\":[1,2,{\"b\":\"x\",\"pad\":3}]}},{\"id\":2,\"n\":{\"pad\":[4]}},\"s\",[{}]]\n",
        0,
    ),
    (
        "delpaths([path(.. | .pad?)])",
        "[{\"id\":1,\"n\":{\"a\":[1,2,{\"b\":\"x\",\"c\":null}]}},{\"id\":2,\"n\":{}},\"s\",null,[{}]]\n",
        0,
    ),
    (
        "delpaths([path(..|numbers)])",
        "[{\"pad\":\"p\",\"n\":{\"a\":[{\"b\":\"x\",\"c\":null}]}},{\"n\":{\"pad\":[]}},\"s\",null,[{\"pad\":null}]]\n",
        0,
    ),
    (
        "del(.. | .pad?, .id?)",
        "[{\"n\":{\"a\":[1,2,{\"b\":\"x\",\"c\":null}]}},{\"n\":{}},\"s\",null,[{}]]\n",
        0,
    ),
    ("try delpaths([path(.[0].pad, error(\"boom\"))]) catch .", "\"boom\"\n", 0),
    (
        "try delpaths([path(.[0].pad, .[2][0])]) catch .",
        "\"Cannot index string with number\"\n",
        0,
    ),
    ("[path(.. | .pad?)] | length", "9\n", 0),
];

#[test]
fn deletes_through_a_descent_answer_as_jq_7789() -> Result<()> {
    for (filter, stdout, status) in ROWS {
        let (out, _, code) = run_jq_full(&["-c", filter], Some(DOC))?;
        assert_eq!((out.as_str(), code), (*stdout, *status), "{filter}");
    }
    Ok(())
}

/// Three chains of 40 nested objects: the paths a descent names there run up to 81 steps, each
/// sharing all but its last with the path of its parent.
#[test]
fn deletes_through_a_deep_descent_answer_as_jq_7789() -> Result<()> {
    let chain = format!("{}null{}", "{\"n\":1,\"a\":".repeat(40), "}".repeat(40));
    let doc = format!("[{}]", vec![chain; 3].join(","));
    for (filter, stdout) in [
        ("del(.. | .n?) | tojson | length", "736\n"),
        ("del(.. | numbers) | [.. | numbers] | length", "0\n"),
        ("delpaths([path(.. | .n?)]) | tojson | length", "736\n"),
        ("del(.[] | .. | .n?) | tojson | length", "736\n"),
        (
            "del(.[1:][] | .. | numbers) | [.. | numbers] | length",
            "40\n",
        ),
        ("[paths] | length", "243\n"),
    ] {
        let (out, _, code) = run_jq_full(&["-c", filter], Some(&doc))?;
        assert_eq!((out.as_str(), code), (stdout, 0), "{filter}");
    }
    Ok(())
}
