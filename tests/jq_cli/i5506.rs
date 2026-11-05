//! #5506: `recurse(.[]?; cond)` takes the cursor walk for the `cond`s the allow-list admits, and the
//! allow-list gained `tojson`, `test`, `any`, `all` and `isempty`. `succinctly jq` shares it, so each
//! row pins the answer (or the refusal) to jq 1.7.1's (`/usr/bin/jq -c`).

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn test_recurse_cond_of_the_gained_builtins_answers_as_jq_does_5506() -> Result<()> {
    for (program, doc, code, expected) in [
        (
            r#"[recurse(.[]?; tojson != "")]"#,
            r#"{"a":{"x":1,"y":[1,2]},"c":{"x":1,"y":[1,2]}}"#,
            0,
            r#"[{"a":{"x":1,"y":[1,2]},"c":{"x":1,"y":[1,2]}},{"x":1,"y":[1,2]},1,[1,2],1,2,{"x":1,"y":[1,2]},1,[1,2],1,2]"#,
        ),
        (
            r#"[recurse(.[]?; test("a"))]"#,
            r#"{"a":{"x":1,"y":[1,2]},"c":{"x":1,"y":[1,2]}}"#,
            5,
            r"",
        ),
        (
            r#"[recurse(.[]?; (strings | test("b")))]"#,
            r#"{"a":{"x":1,"y":[1,2]},"c":{"x":1,"y":[1,2]}}"#,
            0,
            r#"[{"a":{"x":1,"y":[1,2]},"c":{"x":1,"y":[1,2]}}]"#,
        ),
        (
            r"[recurse(.[]?; any)]",
            r#"{"a":{"x":1,"y":[1,2]},"c":{"x":1,"y":[1,2]}}"#,
            5,
            r"",
        ),
        (
            r"[recurse(.[]?; all)]",
            r#"{"a":{"x":1,"y":[1,2]},"c":{"x":1,"y":[1,2]}}"#,
            5,
            r"",
        ),
        (
            r#"[recurse(.[]?; tojson | test("1"))]"#,
            r#"{"a":{"x":1,"y":[1,2]},"c":{"x":1,"y":[1,2]}}"#,
            0,
            r#"[{"a":{"x":1,"y":[1,2]},"c":{"x":1,"y":[1,2]}},{"x":1,"y":[1,2]},1,[1,2],1,{"x":1,"y":[1,2]},1,[1,2],1]"#,
        ),
        (
            r"[recurse(.[]?; (try any catch false))]",
            r#"{"a":{"x":1,"y":[1,2]},"c":{"x":1,"y":[1,2]}}"#,
            0,
            r#"[{"a":{"x":1,"y":[1,2]},"c":{"x":1,"y":[1,2]}},{"x":1,"y":[1,2]},[1,2],{"x":1,"y":[1,2]},[1,2]]"#,
        ),
        (
            r#"[recurse(.[]?; test("A"; "i"))]"#,
            r#"[[1,[2,3]],"s",{"k":[]},null,[[]],{"a":{"b":{}}}]"#,
            5,
            r"",
        ),
        (
            r"[recurse(.[]?; (arrays | any))]",
            r#"[[1,[2,3]],"s",{"k":[]},null,[[]],{"a":{"b":{}}}]"#,
            0,
            r#"[[[1,[2,3]],"s",{"k":[]},null,[[]],{"a":{"b":{}}}],[1,[2,3]],[2,3],[[]]]"#,
        ),
        (
            r#"[recurse(.[]?; (arrays | all(type == "number")))]"#,
            r#"[[1,[2,3]],"s",{"k":[]},null,[[]],{"a":{"b":{}}}]"#,
            0,
            r#"[[[1,[2,3]],"s",{"k":[]},null,[[]],{"a":{"b":{}}}]]"#,
        ),
        (
            r"[recurse(.[]?; isempty(empty))]",
            r#"[[1,[2,3]],"s",{"k":[]},null,[[]],{"a":{"b":{}}}]"#,
            0,
            r#"[[[1,[2,3]],"s",{"k":[]},null,[[]],{"a":{"b":{}}}],[1,[2,3]],1,[2,3],2,3,"s",{"k":[]},[],null,[[]],[],{"a":{"b":{}}},{"b":{}},{}]"#,
        ),
        (
            r"[recurse(.[]?; isempty(.[]?, error))]",
            r#"[[1,[2,3]],"s",{"k":[]},null,[[]],{"a":{"b":{}}}]"#,
            5,
            r"",
        ),
        (
            r"[recurse(.[]?; (tojson|length) > 3)]",
            r#"{"s":"abc","l":["x",["y"]],"n":null,"t":true}"#,
            0,
            r#"[{"s":"abc","l":["x",["y"]],"n":null,"t":true},"abc",["x",["y"]],["y"],null,true]"#,
        ),
        (
            r"[recurse(.[]?; isempty(.[]?))]",
            r#"{"s":"abc","l":["x",["y"]],"n":null,"t":true}"#,
            0,
            r#"[{"s":"abc","l":["x",["y"]],"n":null,"t":true},"abc",null,true]"#,
        ),
        (
            r"[recurse(.[]?; isempty(.[]?) | not)]",
            r#"{"s":"abc","l":["x",["y"]],"n":null,"t":true}"#,
            0,
            r#"[{"s":"abc","l":["x",["y"]],"n":null,"t":true},["x",["y"]],["y"]]"#,
        ),
    ] {
        let (stdout, stderr, status) = run_jq_full(&["-c", program], Some(doc))?;
        let want = if code == 0 { 0 } else { 5 };
        assert_eq!(status, want, "{program} on {doc}: {stderr}");
        assert_eq!(stdout.trim_end(), expected, "{program} on {doc}");
    }
    Ok(())
}
