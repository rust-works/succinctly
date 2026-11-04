//! #6199: a jq-mode `path |= f` of a static path that reaches most of the document is answered
//! on the document's cursor, not on one owned copy of the whole document. Whatever the filter
//! does not answer with one output (nothing, an error), a path that does not land, and a document
//! that is malformed anywhere still take the whole-document route, so every answer is the one it
//! always was. Every row was captured from jq 1.7.1 (`/usr/bin/jq -c`) and agrees with the
//! whole-document route before the change.

use anyhow::Result;

use crate::run_jq_stdin;

#[test]
fn test_a_path_update_answers_as_jq_does_6199() -> Result<()> {
    for (input, filter, expected, code) in [
        (
            r#"{"users":[{"id":1,"age":3},{"id":2,"age":4},{"id":3,"age":5}],"n":7}"#,
            r".users |= .[:2]",
            "{\"users\":[{\"id\":1,\"age\":3},{\"id\":2,\"age\":4}],\"n\":7}\n",
            0,
        ),
        (
            r#"{"users":[{"id":1,"age":3},{"id":2,"age":4},{"id":3,"age":5}],"n":7}"#,
            r".users |= .[1:]",
            "{\"users\":[{\"id\":2,\"age\":4},{\"id\":3,\"age\":5}],\"n\":7}\n",
            0,
        ),
        (
            r#"{"users":[{"id":1,"age":3},{"id":2,"age":4},{"id":3,"age":5}],"n":7}"#,
            r".users |= map(.age + 1)",
            "{\"users\":[4,5,6],\"n\":7}\n",
            0,
        ),
        (
            r#"{"users":[{"id":1,"age":3},{"id":2,"age":4},{"id":3,"age":5}],"n":7}"#,
            r".users |= map(.age)",
            "{\"users\":[3,4,5],\"n\":7}\n",
            0,
        ),
        (
            r#"{"users":[{"id":1,"age":3},{"id":2,"age":4},{"id":3,"age":5}],"n":7}"#,
            r".users |= length",
            "{\"users\":3,\"n\":7}\n",
            0,
        ),
        (
            r#"{"users":[{"id":1,"age":3},{"id":2,"age":4},{"id":3,"age":5}],"n":7}"#,
            r".users |= first",
            "{\"users\":{\"id\":1,\"age\":3},\"n\":7}\n",
            0,
        ),
        (
            r#"{"users":[{"id":1,"age":3},{"id":2,"age":4},{"id":3,"age":5}],"n":7}"#,
            r".users |= add",
            "{\"users\":{\"id\":3,\"age\":5},\"n\":7}\n",
            0,
        ),
        (
            r#"{"users":[{"id":1,"age":3},{"id":2,"age":4},{"id":3,"age":5}],"n":7}"#,
            r".users |= limit(2; .[])",
            "{\"users\":{\"id\":1,\"age\":3},\"n\":7}\n",
            0,
        ),
        (
            r#"{"users":[{"id":1,"age":3},{"id":2,"age":4},{"id":3,"age":5}],"n":7}"#,
            r".users |= .[0]",
            "{\"users\":{\"id\":1,\"age\":3},\"n\":7}\n",
            0,
        ),
        (
            r#"{"users":[{"id":1,"age":3},{"id":2,"age":4},{"id":3,"age":5}],"n":7}"#,
            r".users |= (map(.age) | add)",
            "{\"users\":12,\"n\":7}\n",
            0,
        ),
        (
            r#"{"users":[{"id":1,"age":3},{"id":2,"age":4},{"id":3,"age":5}],"n":7}"#,
            r".users |= map(select(.age > 3))",
            "{\"users\":[{\"id\":2,\"age\":4},{\"id\":3,\"age\":5}],\"n\":7}\n",
            0,
        ),
        (
            r#"{"users":[{"id":1,"age":3},{"id":2,"age":4},{"id":3,"age":5}],"n":7}"#,
            r".users |= sort_by(-.age)",
            "{\"users\":[{\"id\":3,\"age\":5},{\"id\":2,\"age\":4},{\"id\":1,\"age\":3}],\"n\":7}\n",
            0,
        ),
        (
            r#"{"users":[{"id":1,"age":3},{"id":2,"age":4},{"id":3,"age":5}],"n":7}"#,
            r".users |= empty",
            "{\"n\":7}\n",
            0,
        ),
        (
            r#"{"users":[{"id":1,"age":3},{"id":2,"age":4},{"id":3,"age":5}],"n":7}"#,
            r".users |= map(empty)",
            "{\"users\":[],\"n\":7}\n",
            0,
        ),
        (
            r#"{"users":[{"id":1,"age":3},{"id":2,"age":4},{"id":3,"age":5}],"n":7}"#,
            r#".users |= error("boom")"#,
            r"",
            5,
        ),
        (
            r#"{"users":[{"id":1,"age":3},{"id":2,"age":4},{"id":3,"age":5}],"n":7}"#,
            r#".users |= (.[0], error("later"))"#,
            "{\"users\":{\"id\":1,\"age\":3},\"n\":7}\n",
            0,
        ),
        (
            r#"{"users":[{"id":1,"age":3},{"id":2,"age":4},{"id":3,"age":5}],"n":7}"#,
            r".users |= (.[:1], .[1:])",
            "{\"users\":[{\"id\":1,\"age\":3}],\"n\":7}\n",
            0,
        ),
        (
            r#"{"users":[{"id":1,"age":3},{"id":2,"age":4},{"id":3,"age":5}],"n":7}"#,
            r".users[0] |= length",
            "{\"users\":[2,{\"id\":2,\"age\":4},{\"id\":3,\"age\":5}],\"n\":7}\n",
            0,
        ),
        (
            r#"{"users":[{"id":1,"age":3},{"id":2,"age":4},{"id":3,"age":5}],"n":7}"#,
            r".users[1].age |= . * 10",
            "{\"users\":[{\"id\":1,\"age\":3},{\"id\":2,\"age\":40},{\"id\":3,\"age\":5}],\"n\":7}\n",
            0,
        ),
        (
            r#"{"users":[{"id":1,"age":3},{"id":2,"age":4},{"id":3,"age":5}],"n":7}"#,
            r".users[5] |= 1",
            "{\"users\":[{\"id\":1,\"age\":3},{\"id\":2,\"age\":4},{\"id\":3,\"age\":5},null,null,1],\"n\":7}\n",
            0,
        ),
        (
            r#"{"users":[{"id":1,"age":3},{"id":2,"age":4},{"id":3,"age":5}],"n":7}"#,
            r".users[-1] |= 1",
            "{\"users\":[{\"id\":1,\"age\":3},{\"id\":2,\"age\":4},1],\"n\":7}\n",
            0,
        ),
        (
            r#"{"users":[{"id":1,"age":3},{"id":2,"age":4},{"id":3,"age":5}],"n":7}"#,
            r".nope |= 5",
            "{\"users\":[{\"id\":1,\"age\":3},{\"id\":2,\"age\":4},{\"id\":3,\"age\":5}],\"n\":7,\"nope\":5}\n",
            0,
        ),
        (
            r#"{"users":[{"id":1,"age":3},{"id":2,"age":4},{"id":3,"age":5}],"n":7}"#,
            r".nope.deeper |= length",
            "{\"users\":[{\"id\":1,\"age\":3},{\"id\":2,\"age\":4},{\"id\":3,\"age\":5}],\"n\":7,\"nope\":{\"deeper\":0}}\n",
            0,
        ),
        (
            r#"{"users":[{"id":1,"age":3},{"id":2,"age":4},{"id":3,"age":5}],"n":7}"#,
            r".n |= .[0]?",
            "{\"users\":[{\"id\":1,\"age\":3},{\"id\":2,\"age\":4},{\"id\":3,\"age\":5}]}\n",
            0,
        ),
        (
            r#"{"users":[{"id":1,"age":3},{"id":2,"age":4},{"id":3,"age":5}],"n":7}"#,
            r"(.users |= length) | .users",
            "3\n",
            0,
        ),
        (
            r#"{"users":[{"id":1,"age":3},{"id":2,"age":4},{"id":3,"age":5}],"n":7}"#,
            r"[.users |= length] | length",
            "1\n",
            0,
        ),
        (
            r#"{"users":[{"id":1,"age":3},{"id":2,"age":4},{"id":3,"age":5}],"n":7}"#,
            r".users |= map(.age) | .users",
            "[3,4,5]\n",
            0,
        ),
        (
            r#"{"a":1,"a":[1,2,3],"b":2}"#,
            r".a |= length",
            "{\"a\":3,\"b\":2}\n",
            0,
        ),
        (
            r#"{"a":[1,2,3],"a":4}"#,
            r".a |= length",
            "{\"a\":4}\n",
            0,
        ),
        (
            r#"{"a":[1,2,3],"b":null}"#,
            r".b |= length",
            "{\"a\":[1,2,3],\"b\":0}\n",
            0,
        ),
        (
            r#"{"a":{"b":[1,2,3]},"c":1}"#,
            r".a.b |= length",
            "{\"a\":{\"b\":3},\"c\":1}\n",
            0,
        ),
        (
            r#"{"a":{"b":[1,2,3]},"c":1}"#,
            r".a |= length",
            "{\"a\":1,\"c\":1}\n",
            0,
        ),
        (
            r"[[1,2,3],[4]]",
            r".[0] |= length",
            "[3,[4]]\n",
            0,
        ),
        (
            r#"{"a":[1,2,3]}"#,
            r".a |= map(.+1)?",
            "{\"a\":[2,3,4]}\n",
            0,
        ),
        (
            r#"{"a":"x"}"#,
            r".a |= length",
            "{\"a\":1}\n",
            0,
        ),
        (
            r#"{"a":[1,"x"]}"#,
            r".a |= map(.+1)",
            r"",
            5,
        ),
        (
            r#"{"a":[1,2,3],"b":[4,5]}"#,
            r".a |= length | .b |= length",
            "{\"a\":3,\"b\":2}\n",
            0,
        ),
        (
            r#"{"a":[1,2,3]}"#,
            r".a |= input?",
            "{}\n",
            0,
        ),
        (
            r#"{"a":[1,2,3"#,
            r".a |= length",
            r"",
            5,
        ),
        (
            r#"{"a":[1,2,]}"#,
            r".a |= length",
            r"",
            5,
        ),
        (
            r#"{"a":[1,"\q"],"b":1}"#,
            r".a |= length",
            r"",
            5,
        ),
        (
            r#"{"a":[1,2,3],"b":"\q"}"#,
            r".a |= length",
            r"",
            5,
        ),
    ] {
        let (stdout, status) = run_jq_stdin(filter, input, &["-c"])?;
        assert_eq!((stdout.as_str(), status), (expected, code), "{filter} over {input}");
    }
    Ok(())
}

/// The same updates on a document big enough for the cursor route to take them (the rows above are
/// below its size floor). The expected text is built from the records, and every row was checked
/// against jq 1.7.1.
#[test]
fn test_a_path_update_of_a_large_target_answers_as_jq_does_6199() -> Result<()> {
    let ages: Vec<usize> = (1..=40).map(|i| i % 7 + 3).collect();
    let users = |items: &[String]| items.join(",");
    let record = |i: usize| format!(r#"{{"id":{i},"age":{}}}"#, ages[i - 1]);
    let all: Vec<String> = (1..=40).map(record).collect();
    let input = format!(r#"{{"users":[{}],"n":7,"tail":[1,2]}}"#, users(&all));
    let doc = |value: &str| format!(r#"{{"users":{value},"n":7,"tail":[1,2]}}"#);
    let list = |items: Vec<String>| format!("[{}]", items.join(","));
    for (filter, expected) in [
        (".users |= length", doc("40")),
        (".users |= first", doc(&record(1))),
        (".users |= .[0]", doc(&record(1))),
        (".users |= .[:2]", doc(&list(vec![record(1), record(2)]))),
        (".users |= .[38:]", doc(&list(vec![record(39), record(40)]))),
        (
            ".users |= map(.age)",
            doc(&list(ages.iter().map(ToString::to_string).collect())),
        ),
        (
            ".users |= map(.age + 1)",
            doc(&list(ages.iter().map(|a| (a + 1).to_string()).collect())),
        ),
        (
            ".users |= (map(.age) | add)",
            doc(&ages.iter().sum::<usize>().to_string()),
        ),
        (".users |= limit(2; .[])", doc(&record(1))),
        (".users |= map(select(.age > 100))", doc("[]")),
        (
            ".users |= sort_by(-.id) | .users | length",
            "40".to_string(),
        ),
        (".users |= error(\"boom\")", String::new()),
    ] {
        let (stdout, status) = run_jq_stdin(filter, &input, &["-c"])?;
        let code = i32::from(expected.is_empty()) * 5;
        assert_eq!(
            (stdout.trim_end(), status),
            (expected.as_str(), code),
            "{filter}"
        );
    }
    Ok(())
}
