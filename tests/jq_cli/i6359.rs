//! #6359: `path(..|nulls)`, `del(..|nulls)`, `(..|numbers) |= f` and the other type selectors
//! (`booleans`, `numbers`, `strings`, `arrays`, `objects`, `iterables`, `scalars`, `values`) ran
//! through the `path_mode` prelude interpreter (a function lookup and call per node), twice as
//! slow as the `select(type == ...)` spelling that `is_static_target` already resolved natively.
//! They are now static targets. The answers must not change, errors included: a selector followed
//! by a step that is not a path expression (`nulls|tostring`) still raises. Every expectation is
//! what `/usr/bin/jq` 1.7.1 printed (stdout and exit status).

use anyhow::Result;

use crate::run_jq_full;

const A: &str =
    r#"{"a":null,"b":[1,"x",null,{"c":null,"d":true}],"e":{"f":2.5,"g":"s","h":[]},"i":{}}"#;
const B: &str = r#"[null,[null,[null]],{"a":{"a":null}},"s",1,false,true]"#;

/// (stdin, filter, jq 1.7.1's stdout, jq 1.7.1's exit status).
const ROWS: &[(&str, &str, &str, i32)] = &[
    (A, "[path(..|nulls)]", "[[\"a\"],[\"b\",2],[\"b\",3,\"c\"]]\n", 0),
    (A, "[path(..|nulls)]|length", "3\n", 0),
    (A, "del(..|nulls)", "{\"b\":[1,\"x\",{\"d\":true}],\"e\":{\"f\":2.5,\"g\":\"s\",\"h\":[]},\"i\":{}}\n", 0),
    (A, "[path(.[]?|nulls)]", "[[\"a\"]]\n", 0),
    (A, "[path(..|nulls|.a?)]", "[[\"a\",\"a\"],[\"b\",2,\"a\"],[\"b\",3,\"c\",\"a\"]]\n", 0),
    (A, "(..|nulls) |= 1", "{\"a\":1,\"b\":[1,\"x\",1,{\"c\":1,\"d\":true}],\"e\":{\"f\":2.5,\"g\":\"s\",\"h\":[]},\"i\":{}}\n", 0),
    (A, "[path(..|nulls|select(.==null))]", "[[\"a\"],[\"b\",2],[\"b\",3,\"c\"]]\n", 0),
    (A, "[path(..|nulls)|length]", "[1,2,3]\n", 0),
    (A, "path(nulls)", "", 0),
    (A, "[path(.. | first(nulls))]", "[[\"a\"],[\"b\",2],[\"b\",3,\"c\"]]\n", 0),
    (A, "[path(..|try nulls)]", "[[\"a\"],[\"b\",2],[\"b\",3,\"c\"]]\n", 0),
    (A, "[path(..|(nulls//empty))]", "[]\n", 0),
    (A, "[path(..|nulls|nulls)]", "[[\"a\"],[\"b\",2],[\"b\",3,\"c\"]]\n", 0),
    (A, "delpaths([path(..|nulls)])", "{\"b\":[1,\"x\",{\"d\":true}],\"e\":{\"f\":2.5,\"g\":\"s\",\"h\":[]},\"i\":{}}\n", 0),
    (A, "[path(..|nulls|tostring)]", "", 5),
    (A, "[path(..|nulls|length)]", "", 5),
    (A, "try path(..|nulls|tostring) catch .", "\"Invalid path expression with result \\\"null\\\"\"\n", 0),
    (A, "[path(..|nulls, .)]", "[[],[\"a\"],[\"a\"],[\"b\"],[\"b\",0],[\"b\",1],[\"b\",2],[\"b\",2],[\"b\",3],[\"b\",3,\"c\"],[\"b\",3,\"c\"],[\"b\",3,\"d\"],[\"e\"],[\"e\",\"f\"],[\"e\",\"g\"],[\"e\",\"h\"],[\"i\"]]\n", 0),
    (A, "[path(..|if nulls then . else empty end)]", "[]\n", 0),
    (A, "to_entries? // [] | [path(.[]|nulls)]", "[]\n", 0),
    (A, "(.. | nulls) = 9", "{\"a\":9,\"b\":[1,\"x\",9,{\"c\":9,\"d\":true}],\"e\":{\"f\":2.5,\"g\":\"s\",\"h\":[]},\"i\":{}}\n", 0),
    (A, "[limit(3; path(..|nulls))]", "[[\"a\"],[\"b\",2],[\"b\",3,\"c\"]]\n", 0),
    (A, "[path(first(..|nulls))]", "[[\"a\"]]\n", 0),
    (A, "path(..|select(nulls))", "", 0),
    (A, r#"[path(getpath(["a"])|nulls)]"#, "[[\"a\"]]\n", 0),
    (A, "[path(.a?|nulls)]", "[[\"a\"]]\n", 0),
    (A, "[path(..|nulls) as $p | $p]", "[[\"a\"],[\"b\",2],[\"b\",3,\"c\"]]\n", 0),
    (A, "reduce path(..|nulls) as $p (0; .+1)", "3\n", 0),
    (A, "[path(..|strings)]", "[[\"b\",1],[\"e\",\"g\"]]\n", 0),
    (A, "[path(..|strings)]|length", "2\n", 0),
    (A, "del(..|strings)", "{\"a\":null,\"b\":[1,null,{\"c\":null,\"d\":true}],\"e\":{\"f\":2.5,\"h\":[]},\"i\":{}}\n", 0),
    (A, "[path(.[]?|strings)]", "[]\n", 0),
    (A, "[path(..|strings|.a?)]", "[]\n", 0),
    (A, "(..|strings) |= 1", "{\"a\":null,\"b\":[1,1,null,{\"c\":null,\"d\":true}],\"e\":{\"f\":2.5,\"g\":1,\"h\":[]},\"i\":{}}\n", 0),
    (A, "[path(..|strings|select(.==null))]", "[]\n", 0),
    (A, "[path(..|strings)|length]", "[2,2]\n", 0),
    (A, "path(strings)", "", 0),
    (A, "[path(.. | first(strings))]", "[[\"b\",1],[\"e\",\"g\"]]\n", 0),
    (A, "[path(..|try strings)]", "[[\"b\",1],[\"e\",\"g\"]]\n", 0),
    (A, "[path(..|(strings//empty))]", "[[\"b\",1],[\"e\",\"g\"]]\n", 0),
    (A, "[path(..|strings|strings)]", "[[\"b\",1],[\"e\",\"g\"]]\n", 0),
    (A, "delpaths([path(..|strings)])", "{\"a\":null,\"b\":[1,null,{\"c\":null,\"d\":true}],\"e\":{\"f\":2.5,\"h\":[]},\"i\":{}}\n", 0),
    (A, "[path(..|strings|tostring)]", "[[\"b\",1],[\"e\",\"g\"]]\n", 0),
    (A, "[path(..|strings|length)]", "", 5),
    (A, "try path(..|strings|tostring) catch .", "[\"b\",1]\n[\"e\",\"g\"]\n", 0),
    (A, "[path(..|strings, .)]", "[[],[\"a\"],[\"b\"],[\"b\",0],[\"b\",1],[\"b\",1],[\"b\",2],[\"b\",3],[\"b\",3,\"c\"],[\"b\",3,\"d\"],[\"e\"],[\"e\",\"f\"],[\"e\",\"g\"],[\"e\",\"g\"],[\"e\",\"h\"],[\"i\"]]\n", 0),
    (A, "[path(..|if strings then . else empty end)]", "[[\"b\",1],[\"e\",\"g\"]]\n", 0),
    (A, "to_entries? // [] | [path(.[]|strings)]", "[]\n", 0),
    (A, "(.. | strings) = 9", "{\"a\":null,\"b\":[1,9,null,{\"c\":null,\"d\":true}],\"e\":{\"f\":2.5,\"g\":9,\"h\":[]},\"i\":{}}\n", 0),
    (A, "[limit(3; path(..|strings))]", "[[\"b\",1],[\"e\",\"g\"]]\n", 0),
    (A, "[path(first(..|strings))]", "[[\"b\",1]]\n", 0),
    (A, "path(..|select(strings))", "[\"b\",1]\n[\"e\",\"g\"]\n", 0),
    (A, r#"[path(getpath(["a"])|strings)]"#, "[]\n", 0),
    (A, "[path(.a?|strings)]", "[]\n", 0),
    (A, "[path(..|strings) as $p | $p]", "[[\"b\",1],[\"e\",\"g\"]]\n", 0),
    (A, "reduce path(..|strings) as $p (0; .+1)", "2\n", 0),
    (B, "[path(..|nulls)]", "[[0],[1,0],[1,1,0],[2,\"a\",\"a\"]]\n", 0),
    (B, "[path(..|nulls|tostring)]", "", 5),
    (B, "try path(..|nulls|tostring) catch .", "\"Invalid path expression with result \\\"null\\\"\"\n", 0),
    (B, "[path(..|nulls, .)]", "[[],[0],[0],[1],[1,0],[1,0],[1,1],[1,1,0],[1,1,0],[2],[2,\"a\"],[2,\"a\",\"a\"],[2,\"a\",\"a\"],[3],[4],[5],[6]]\n", 0),
    (B, "reduce path(..|nulls) as $p (0; .+1)", "4\n", 0),
    (B, "(..|strings) |= 1", "[null,[null,[null]],{\"a\":{\"a\":null}},1,1,false,true]\n", 0),
    (B, "del(..|arrays)", "null\n", 0),
    (B, "[path(..|objects)]", "[[2],[2,\"a\"]]\n", 0),
    (B, "[path(..|iterables)]|length", "5\n", 0),
    (B, "[path(..|scalars)]", "[[0],[1,0],[1,1,0],[2,\"a\",\"a\"],[3],[4],[5],[6]]\n", 0),
    (B, "del(..|values)", "null\n", 0),
    (B, "[path(..|strings)]", "[[3]]\n", 0),
    (B, "[path(..|strings)]|length", "1\n", 0),
    (B, "del(..|strings)", "[null,[null,[null]],{\"a\":{\"a\":null}},1,false,true]\n", 0),
    (B, "[path(.[]?|strings)]", "[[3]]\n", 0),
    (B, "[path(..|strings|.a?)]", "[]\n", 0),
    (B, "[path(..|strings|select(.==null))]", "[]\n", 0),
    (B, "[path(..|strings)|length]", "[1]\n", 0),
    (B, "path(strings)", "", 0),
    (B, "[path(.. | first(strings))]", "[[3]]\n", 0),
    (B, "[path(..|try strings)]", "[[3]]\n", 0),
    (B, "[path(..|(strings//empty))]", "[[3]]\n", 0),
    (B, "[path(..|strings|strings)]", "[[3]]\n", 0),
    (B, "delpaths([path(..|strings)])", "[null,[null,[null]],{\"a\":{\"a\":null}},1,false,true]\n", 0),
];

#[test]
fn test_type_selectors_in_path_position_agree_with_jq_6359() -> Result<()> {
    for &(input, filter, expected, status) in ROWS {
        let (stdout, stderr, code) = run_jq_full(&["-c", filter], Some(input))?;
        assert_eq!(stdout, expected, "{input} | {filter}: {stderr}");
        assert_eq!(code, status, "{input} | {filter}: {stderr}");
    }
    Ok(())
}
