//! #5612: a postfix (`.x`, `[0]`, `["x"]`, `[]`, `[1:]`) after `first`, `last`, `limit`, `range`,
//! `repeat`, `until`, `while`, `error` and `JOIN` binds to the call, as it does after every other
//! jq builtin. These nine were the only names the parser refused it after. Expected values were
//! captured from jq 1.7.1 (`/usr/bin/jq`).

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn test_postfix_after_a_special_form_matches_jq_5612() -> Result<()> {
    for (program, input, expected, code) in [
        (r"first(.a[]).id", r#"{"a":[{"id":1},{"id":2}]}"#, "1\n", 0),
        (r"first(.[]).x", r#"[{"x":7},{"x":8}]"#, "7\n", 0),
        (r"last(.a[]).id", r#"{"a":[{"id":1},{"id":2}]}"#, "2\n", 0),
        (r"first.x", r#"[{"x":1},{"x":2}]"#, "1\n", 0),
        (r"last[0]", r"[[1,2],[3,4]]", "3\n", 0),
        (r"first[0]", r"[[1,2],[3]]", "1\n", 0),
        (
            r"[limit(2;.[]).x]",
            r#"[{"x":1},{"x":2},{"x":3}]"#,
            "[1,2]\n",
            0,
        ),
        (r"[limit(2;.[])[0]]", r"[[1],[2],[3]]", "[1,2]\n", 0),
        (r"[range(3)]|.[1]", r"null", "1\n", 0),
        (r"[range(2;4).x?]", r"null", "[]\n", 0),
        (r#"try range(3).x catch "E""#, r"null", "\"E\"\n", 0),
        (r#"try range(1;3)[0] catch "E""#, r"null", "\"E\"\n", 0),
        (r#"try range(0;6;2).x catch "E""#, r"null", "\"E\"\n", 0),
        (r#"try error("x").y catch ."#, r"null", "\"x\"\n", 0),
        (r"try error.y catch .", r#""boom""#, "\"boom\"\n", 0),
        (r#"try error("x")[0] catch ."#, r"null", "\"x\"\n", 0),
        (r"[first(.[]).x?]", r#"[1,{"x":5}]"#, "[]\n", 0),
        (r"first(.[]).x?", r#"[1,{"x":5}]"#, r"", 0),
        (r#"[limit(3;repeat({"x":1})).x]"#, r"null", "[1,1,1]\n", 0),
        (r"[until(.>3;.+1)[]?]", r"1", "[]\n", 0),
        (r#"try until(.>3;.+1).x catch "E""#, r"1", "\"E\"\n", 0),
        (r"[limit(3;while(.<9;.*2)).x?]", r"1", "[]\n", 0),
        (
            r"[first(.[]).x, last(.[]).x]",
            r#"[{"x":1},{"x":2}]"#,
            "[1,2]\n",
            0,
        ),
        (r"first(.[]) .x", r#"[{"x":1}]"#, "1\n", 0),
        ("first(.[])\n.x", r#"[{"x":1}]"#, "1\n", 0),
        (
            r#"def first(f): "s"; try first(.).x catch "E""#,
            r#"[{"x":1}]"#,
            "\"E\"\n",
            0,
        ),
        (r#"def range(a): {"x":a}; range(3).x"#, r"null", "3\n", 0),
        (
            r#"def limit(n;f): {"x":n}; limit(2;.).x"#,
            r"null",
            "2\n",
            0,
        ),
        (r#"first(.[])["x"]"#, r#"[{"x":1}]"#, "1\n", 0),
        (r"first(.[])[]", r"[[1,2]]", "1\n2\n", 0),
        (
            r#"try JOIN({};.;.).x catch "E""#,
            r#"[{"x":1}]"#,
            "\"E\"\n",
            0,
        ),
        (
            r#"JOIN({"a":1};.[];.x)[0]"#,
            r#"[{"x":"a"}]"#,
            "{\"x\":\"a\"}\n",
            0,
        ),
        (r"[first(.[])[1:]]", r"[[1,2,3]]", "[[2,3]]\n", 0),
        (r"first(.[]).x as $v | $v + 1", r#"[{"x":1}]"#, "2\n", 0),
        (
            r"[.[] | first(.a, .b).c]",
            r#"[{"a":{"c":1},"b":{"c":2}}]"#,
            "[1]\n",
            0,
        ),
    ] {
        let (stdout, _, exit) = run_jq_full(&["-c", program], Some(input))?;
        assert_eq!((stdout.as_str(), exit), (expected, code), "{program}");
    }
    Ok(())
}
