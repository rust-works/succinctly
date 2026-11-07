//! #6595: `recurse(f)` with `f` a static path ending in `.[]?` (`.children[]?`, `.a.b[]?`) reads
//! each node's children off the owned tree, where every node used to be written out as text and
//! indexed again to read them. Every expected string was captured from jq 1.7.1 (`/usr/bin/jq`).

use anyhow::Result;

use crate::run_jq_full;

/// `[recurse(f)]` over each input: stdout, stderr and the exit status, as jq 1.7.1 answers them.
/// The inputs hold non-array, `null` and missing `children`, scalar and array nodes (where jq's
/// `.children` raises: its `?` guards only the iteration) and number spellings.
#[test]
fn recurse_bodies_answer_as_jq_6595() -> Result<()> {
    for (body, input, want, want_err, want_code) in [
        (
            r#".children[]?"#,
            r#"{"id":1,"children":[{"id":2,"children":[{"id":3}]},{"id":4,"children":null},{"id":5,"children":7},{"id":6,"children":{"b":{"id":8},"a":{"id":9,"children":[]}}}]}"#,
            "[{\"id\":1,\"children\":[{\"id\":2,\"children\":[{\"id\":3}]},{\"id\":4,\"children\":null},{\"id\":5,\"children\":7},{\"id\":6,\"children\":{\"b\":{\"id\":8},\"a\":{\"id\":9,\"children\":[]}}}]},{\"id\":2,\"children\":[{\"id\":3}]},{\"id\":3},{\"id\":4,\"children\":null},{\"id\":5,\"children\":7},{\"id\":6,\"children\":{\"b\":{\"id\":8},\"a\":{\"id\":9,\"children\":[]}}},{\"id\":8},{\"id\":9,\"children\":[]}]\n",
            "",
            0,
        ),
        (
            r#".children[]?"#,
            r#"{"id":1,"children":[1,"s",null]}"#,
            "",
            "jq: error (at <stdin>:0): Cannot index number with string \"children\"\n",
            5,
        ),
        (
            r#".children[]?"#,
            r#"[{"children":[{"x":1}]},{"y":2}]"#,
            "",
            "jq: error (at <stdin>:0): Cannot index array with string \"children\"\n",
            5,
        ),
        (
            r#".children[]?"#,
            r#"{"children":[{"children":[[1,2]]}]}"#,
            "",
            "jq: error (at <stdin>:0): Cannot index array with string \"children\"\n",
            5,
        ),
        (
            r#".children[]?"#,
            r#"{"a":{"b":[{"a":{"b":[]}},3]}}"#,
            "[{\"a\":{\"b\":[{\"a\":{\"b\":[]}},3]}}]\n",
            "",
            0,
        ),
        (
            r#".children[]?"#,
            r#"{"children":[1.50,100000000000000000000,{"children":[]}]}"#,
            "",
            "jq: error (at <stdin>:0): Cannot index number with string \"children\"\n",
            5,
        ),
        (
            r#".children[]?"#,
            r#"null"#,
            "[null]\n",
            "",
            0,
        ),
        (
            r#".children[]?"#,
            r#"5"#,
            "",
            "jq: error (at <stdin>:0): Cannot index number with string \"children\"\n",
            5,
        ),
        (
            r#".[]?"#,
            r#"{"id":1,"children":[{"id":2,"children":[{"id":3}]},{"id":4,"children":null},{"id":5,"children":7},{"id":6,"children":{"b":{"id":8},"a":{"id":9,"children":[]}}}]}"#,
            "[{\"id\":1,\"children\":[{\"id\":2,\"children\":[{\"id\":3}]},{\"id\":4,\"children\":null},{\"id\":5,\"children\":7},{\"id\":6,\"children\":{\"b\":{\"id\":8},\"a\":{\"id\":9,\"children\":[]}}}]},1,[{\"id\":2,\"children\":[{\"id\":3}]},{\"id\":4,\"children\":null},{\"id\":5,\"children\":7},{\"id\":6,\"children\":{\"b\":{\"id\":8},\"a\":{\"id\":9,\"children\":[]}}}],{\"id\":2,\"children\":[{\"id\":3}]},2,[{\"id\":3}],{\"id\":3},3,{\"id\":4,\"children\":null},4,null,{\"id\":5,\"children\":7},5,7,{\"id\":6,\"children\":{\"b\":{\"id\":8},\"a\":{\"id\":9,\"children\":[]}}},6,{\"b\":{\"id\":8},\"a\":{\"id\":9,\"children\":[]}},{\"id\":8},8,{\"id\":9,\"children\":[]},9,[]]\n",
            "",
            0,
        ),
        (
            r#".[]?"#,
            r#"{"id":1,"children":[1,"s",null]}"#,
            "[{\"id\":1,\"children\":[1,\"s\",null]},1,[1,\"s\",null],1,\"s\",null]\n",
            "",
            0,
        ),
        (
            r#".[]?"#,
            r#"[{"children":[{"x":1}]},{"y":2}]"#,
            "[[{\"children\":[{\"x\":1}]},{\"y\":2}],{\"children\":[{\"x\":1}]},[{\"x\":1}],{\"x\":1},1,{\"y\":2},2]\n",
            "",
            0,
        ),
        (
            r#".[]?"#,
            r#"{"children":[{"children":[[1,2]]}]}"#,
            "[{\"children\":[{\"children\":[[1,2]]}]},[{\"children\":[[1,2]]}],{\"children\":[[1,2]]},[[1,2]],[1,2],1,2]\n",
            "",
            0,
        ),
        (
            r#".[]?"#,
            r#"{"a":{"b":[{"a":{"b":[]}},3]}}"#,
            "[{\"a\":{\"b\":[{\"a\":{\"b\":[]}},3]}},{\"b\":[{\"a\":{\"b\":[]}},3]},[{\"a\":{\"b\":[]}},3],{\"a\":{\"b\":[]}},{\"b\":[]},[],3]\n",
            "",
            0,
        ),
        (
            r#".[]?"#,
            r#"{"children":[1.50,100000000000000000000,{"children":[]}]}"#,
            "[{\"children\":[1.50,100000000000000000000,{\"children\":[]}]},[1.50,100000000000000000000,{\"children\":[]}],1.50,100000000000000000000,{\"children\":[]},[]]\n",
            "",
            0,
        ),
        (
            r#".[]?"#,
            r#"null"#,
            "[null]\n",
            "",
            0,
        ),
        (
            r#".[]?"#,
            r#"5"#,
            "[5]\n",
            "",
            0,
        ),
        (
            r#".children[]? | select(.id != 4)"#,
            r#"{"id":1,"children":[{"id":2,"children":[{"id":3}]},{"id":4,"children":null},{"id":5,"children":7},{"id":6,"children":{"b":{"id":8},"a":{"id":9,"children":[]}}}]}"#,
            "[{\"id\":1,\"children\":[{\"id\":2,\"children\":[{\"id\":3}]},{\"id\":4,\"children\":null},{\"id\":5,\"children\":7},{\"id\":6,\"children\":{\"b\":{\"id\":8},\"a\":{\"id\":9,\"children\":[]}}}]},{\"id\":2,\"children\":[{\"id\":3}]},{\"id\":3},{\"id\":5,\"children\":7},{\"id\":6,\"children\":{\"b\":{\"id\":8},\"a\":{\"id\":9,\"children\":[]}}},{\"id\":8},{\"id\":9,\"children\":[]}]\n",
            "",
            0,
        ),
        (
            r#".children[]? | select(.id != 4)"#,
            r#"{"id":1,"children":[1,"s",null]}"#,
            "",
            "jq: error (at <stdin>:0): Cannot index number with string \"id\"\n",
            5,
        ),
        (
            r#".children[]? | select(.id != 4)"#,
            r#"[{"children":[{"x":1}]},{"y":2}]"#,
            "",
            "jq: error (at <stdin>:0): Cannot index array with string \"children\"\n",
            5,
        ),
        (
            r#".children[]? | select(.id != 4)"#,
            r#"{"children":[{"children":[[1,2]]}]}"#,
            "",
            "jq: error (at <stdin>:0): Cannot index array with string \"id\"\n",
            5,
        ),
        (
            r#".children[]? | select(.id != 4)"#,
            r#"{"a":{"b":[{"a":{"b":[]}},3]}}"#,
            "[{\"a\":{\"b\":[{\"a\":{\"b\":[]}},3]}}]\n",
            "",
            0,
        ),
        (
            r#".children[]? | select(.id != 4)"#,
            r#"{"children":[1.50,100000000000000000000,{"children":[]}]}"#,
            "",
            "jq: error (at <stdin>:0): Cannot index number with string \"id\"\n",
            5,
        ),
        (
            r#".children[]? | select(.id != 4)"#,
            r#"null"#,
            "[null]\n",
            "",
            0,
        ),
        (
            r#".children[]? | select(.id != 4)"#,
            r#"5"#,
            "",
            "jq: error (at <stdin>:0): Cannot index number with string \"children\"\n",
            5,
        ),
        (
            r#".a.b[]?"#,
            r#"{"id":1,"children":[{"id":2,"children":[{"id":3}]},{"id":4,"children":null},{"id":5,"children":7},{"id":6,"children":{"b":{"id":8},"a":{"id":9,"children":[]}}}]}"#,
            "[{\"id\":1,\"children\":[{\"id\":2,\"children\":[{\"id\":3}]},{\"id\":4,\"children\":null},{\"id\":5,\"children\":7},{\"id\":6,\"children\":{\"b\":{\"id\":8},\"a\":{\"id\":9,\"children\":[]}}}]}]\n",
            "",
            0,
        ),
        (
            r#".a.b[]?"#,
            r#"{"id":1,"children":[1,"s",null]}"#,
            "[{\"id\":1,\"children\":[1,\"s\",null]}]\n",
            "",
            0,
        ),
        (
            r#".a.b[]?"#,
            r#"[{"children":[{"x":1}]},{"y":2}]"#,
            "",
            "jq: error (at <stdin>:0): Cannot index array with string \"a\"\n",
            5,
        ),
        (
            r#".a.b[]?"#,
            r#"{"children":[{"children":[[1,2]]}]}"#,
            "[{\"children\":[{\"children\":[[1,2]]}]}]\n",
            "",
            0,
        ),
        (
            r#".a.b[]?"#,
            r#"{"a":{"b":[{"a":{"b":[]}},3]}}"#,
            "",
            "jq: error (at <stdin>:0): Cannot index number with string \"a\"\n",
            5,
        ),
        (
            r#".a.b[]?"#,
            r#"{"children":[1.50,100000000000000000000,{"children":[]}]}"#,
            "[{\"children\":[1.50,100000000000000000000,{\"children\":[]}]}]\n",
            "",
            0,
        ),
        (
            r#".a.b[]?"#,
            r#"null"#,
            "[null]\n",
            "",
            0,
        ),
        (
            r#".a.b[]?"#,
            r#"5"#,
            "",
            "jq: error (at <stdin>:0): Cannot index number with string \"a\"\n",
            5,
        ),
        (
            r#".children[0][]?"#,
            r#"{"id":1,"children":[{"id":2,"children":[{"id":3}]},{"id":4,"children":null},{"id":5,"children":7},{"id":6,"children":{"b":{"id":8},"a":{"id":9,"children":[]}}}]}"#,
            "",
            "jq: error (at <stdin>:0): Cannot index number with string \"children\"\n",
            5,
        ),
        (
            r#".children[0][]?"#,
            r#"{"id":1,"children":[1,"s",null]}"#,
            "[{\"id\":1,\"children\":[1,\"s\",null]}]\n",
            "",
            0,
        ),
        (
            r#".children[0][]?"#,
            r#"[{"children":[{"x":1}]},{"y":2}]"#,
            "",
            "jq: error (at <stdin>:0): Cannot index array with string \"children\"\n",
            5,
        ),
        (
            r#".children[0][]?"#,
            r#"{"children":[{"children":[[1,2]]}]}"#,
            "",
            "jq: error (at <stdin>:0): Cannot index array with string \"children\"\n",
            5,
        ),
        (
            r#".children[0][]?"#,
            r#"{"a":{"b":[{"a":{"b":[]}},3]}}"#,
            "[{\"a\":{\"b\":[{\"a\":{\"b\":[]}},3]}}]\n",
            "",
            0,
        ),
        (
            r#".children[0][]?"#,
            r#"{"children":[1.50,100000000000000000000,{"children":[]}]}"#,
            "[{\"children\":[1.50,100000000000000000000,{\"children\":[]}]}]\n",
            "",
            0,
        ),
        (
            r#".children[0][]?"#,
            r#"null"#,
            "[null]\n",
            "",
            0,
        ),
        (
            r#".children[0][]?"#,
            r#"5"#,
            "",
            "jq: error (at <stdin>:0): Cannot index number with string \"children\"\n",
            5,
        ),
        (
            r#".children | .[]?"#,
            r#"{"id":1,"children":[{"id":2,"children":[{"id":3}]},{"id":4,"children":null},{"id":5,"children":7},{"id":6,"children":{"b":{"id":8},"a":{"id":9,"children":[]}}}]}"#,
            "[{\"id\":1,\"children\":[{\"id\":2,\"children\":[{\"id\":3}]},{\"id\":4,\"children\":null},{\"id\":5,\"children\":7},{\"id\":6,\"children\":{\"b\":{\"id\":8},\"a\":{\"id\":9,\"children\":[]}}}]},{\"id\":2,\"children\":[{\"id\":3}]},{\"id\":3},{\"id\":4,\"children\":null},{\"id\":5,\"children\":7},{\"id\":6,\"children\":{\"b\":{\"id\":8},\"a\":{\"id\":9,\"children\":[]}}},{\"id\":8},{\"id\":9,\"children\":[]}]\n",
            "",
            0,
        ),
        (
            r#".children | .[]?"#,
            r#"{"id":1,"children":[1,"s",null]}"#,
            "",
            "jq: error (at <stdin>:0): Cannot index number with string \"children\"\n",
            5,
        ),
        (
            r#".children | .[]?"#,
            r#"[{"children":[{"x":1}]},{"y":2}]"#,
            "",
            "jq: error (at <stdin>:0): Cannot index array with string \"children\"\n",
            5,
        ),
        (
            r#".children | .[]?"#,
            r#"{"children":[{"children":[[1,2]]}]}"#,
            "",
            "jq: error (at <stdin>:0): Cannot index array with string \"children\"\n",
            5,
        ),
        (
            r#".children | .[]?"#,
            r#"{"a":{"b":[{"a":{"b":[]}},3]}}"#,
            "[{\"a\":{\"b\":[{\"a\":{\"b\":[]}},3]}}]\n",
            "",
            0,
        ),
        (
            r#".children | .[]?"#,
            r#"{"children":[1.50,100000000000000000000,{"children":[]}]}"#,
            "",
            "jq: error (at <stdin>:0): Cannot index number with string \"children\"\n",
            5,
        ),
        (
            r#".children | .[]?"#,
            r#"null"#,
            "[null]\n",
            "",
            0,
        ),
        (
            r#".children | .[]?"#,
            r#"5"#,
            "",
            "jq: error (at <stdin>:0): Cannot index number with string \"children\"\n",
            5,
        ),
        (
            r#". | .children[]?"#,
            r#"{"id":1,"children":[{"id":2,"children":[{"id":3}]},{"id":4,"children":null},{"id":5,"children":7},{"id":6,"children":{"b":{"id":8},"a":{"id":9,"children":[]}}}]}"#,
            "[{\"id\":1,\"children\":[{\"id\":2,\"children\":[{\"id\":3}]},{\"id\":4,\"children\":null},{\"id\":5,\"children\":7},{\"id\":6,\"children\":{\"b\":{\"id\":8},\"a\":{\"id\":9,\"children\":[]}}}]},{\"id\":2,\"children\":[{\"id\":3}]},{\"id\":3},{\"id\":4,\"children\":null},{\"id\":5,\"children\":7},{\"id\":6,\"children\":{\"b\":{\"id\":8},\"a\":{\"id\":9,\"children\":[]}}},{\"id\":8},{\"id\":9,\"children\":[]}]\n",
            "",
            0,
        ),
        (
            r#". | .children[]?"#,
            r#"{"id":1,"children":[1,"s",null]}"#,
            "",
            "jq: error (at <stdin>:0): Cannot index number with string \"children\"\n",
            5,
        ),
        (
            r#". | .children[]?"#,
            r#"[{"children":[{"x":1}]},{"y":2}]"#,
            "",
            "jq: error (at <stdin>:0): Cannot index array with string \"children\"\n",
            5,
        ),
        (
            r#". | .children[]?"#,
            r#"{"children":[{"children":[[1,2]]}]}"#,
            "",
            "jq: error (at <stdin>:0): Cannot index array with string \"children\"\n",
            5,
        ),
        (
            r#". | .children[]?"#,
            r#"{"a":{"b":[{"a":{"b":[]}},3]}}"#,
            "[{\"a\":{\"b\":[{\"a\":{\"b\":[]}},3]}}]\n",
            "",
            0,
        ),
        (
            r#". | .children[]?"#,
            r#"{"children":[1.50,100000000000000000000,{"children":[]}]}"#,
            "",
            "jq: error (at <stdin>:0): Cannot index number with string \"children\"\n",
            5,
        ),
        (
            r#". | .children[]?"#,
            r#"null"#,
            "[null]\n",
            "",
            0,
        ),
        (
            r#". | .children[]?"#,
            r#"5"#,
            "",
            "jq: error (at <stdin>:0): Cannot index number with string \"children\"\n",
            5,
        ),
    ] {
        let filter = format!("[recurse({body})]");
        let (stdout, stderr, code) = run_jq_full(&["-c", &filter], Some(input))?;
        assert_eq!(
            (stdout.as_str(), stderr.as_str(), code),
            (want, want_err, want_code),
            "{filter} on {input}"
        );
    }
    Ok(())
}

/// A chain 40 nodes deep, past the levels the walk descends natively, so the queued route reads
/// the children too; and a scalar sibling after it, which raises once the chain is visited.
#[test]
fn a_deep_chain_recurses_as_jq_6595() -> Result<()> {
    let mut chain = String::from("[]");
    for id in (1..=40).rev() {
        chain = if id == 40 {
            format!(r#"{{"id":{id},"children":[]}}"#)
        } else {
            format!(r#"{{"id":{id},"children":[{chain}]}}"#)
        };
    }
    let (stdout, stderr, code) =
        run_jq_full(&["-c", "[recurse(.children[]?) | .id]"], Some(&chain))?;
    assert_eq!((stdout.as_str(), stderr.as_str(), code), ("[1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18,19,20,21,22,23,24,25,26,27,28,29,30,31,32,33,34,35,36,37,38,39,40]\n", "", 0));
    let forked = format!(r#"{{"id":0,"children":[{chain},5]}}"#);
    let (stdout, stderr, code) =
        run_jq_full(&["-c", "[recurse(.children[]?) | .id]"], Some(&forked))?;
    assert_eq!(
        (stdout.as_str(), stderr.as_str(), code),
        (
            "",
            "jq: error (at <stdin>:0): Cannot index number with string \"id\"\n",
            5
        )
    );
    Ok(())
}
