//! #6258: a literal-bound slice of an owned array or string (a variable, a built value) is cut in place
//! instead of written out as JSON, indexed and decoded again. The answers must not move: every row was
//! captured from jq 1.7.1 (`/usr/bin/jq -c`).

use anyhow::Result;

use crate::run_jq_stdin;

#[test]
fn a_literal_slice_of_an_owned_value_answers_as_jq_does_6258() -> Result<()> {
    for (input, filter, expected) in [
        (
            r#"[1, 2.5, 3.0, 100000000000000000000, null, {"k":[7]}, "x"]"#,
            r". as $a | $a[:2], $a[1:3], $a[-2:], $a[2:1], $a[5:10], $a[0:], $a[-100:100]",
            r#"[1,2.5]
[2.5,3.0]
[{"k":[7]},"x"]
[]
[{"k":[7]},"x"]
[1,2.5,3.0,100000000000000000000,null,{"k":[7]},"x"]
[1,2.5,3.0,100000000000000000000,null,{"k":[7]},"x"]
"#,
        ),
        (
            r"[1, 2.5, 3.0, 100000000000000000000]",
            r". as $a | [$a[2:4][] | tojson]",
            r#"["3.0","100000000000000000000"]
"#,
        ),
        (
            r"[1.0, 1e1000, 0.10]",
            r". as $a | $a[:2], $a[1:]",
            r"[1.0,1E+1000]
[1E+1000,0.10]
",
        ),
        (
            r#""héllo wörld 😀!""#,
            r". as $s | $s[:3], $s[1:-1], $s[-3:], $s[4:2], $s[20:30]",
            r#""hél"
"éllo wörld 😀"
" 😀!"
""
""
"#,
        ),
        (
            r"null",
            r". as $n | $n[1:2]",
            r"null
",
        ),
        (
            r"[]",
            r". as $a | $a[:2], $a[0:]",
            r"[]
[]
",
        ),
        (
            r"[[1,2],[3,4],[5,6]]",
            r". as $a | $a[1:] | .[0][1]",
            r"4
",
        ),
        (
            r#"{"a":1}"#,
            r". as $o | try $o[:1] catch .",
            r#""Cannot index object with object"
"#,
        ),
        (
            r"5",
            r". as $n | try $n[:1] catch .",
            r#""Cannot index number with object"
"#,
        ),
        (
            r"[1,2,3]",
            r". as $a | reduce range(3) as $i (0; . + ($a[$i:] | length))",
            r"6
",
        ),
        (
            r"[1,2,3,4]",
            r". as $a | [limit(3; repeat($a[1:3] | add))]",
            r"[5,5,5]
",
        ),
        (
            r"[0,1,2,3,4,5]",
            r#". as $a | $a[1.5:3.5], $a[-1.5:], $a[null:2], $a[1:null], $a[{"start":1,"end":2}], $a[nan:2], $a[1:infinite]"#,
            r"[1,2,3]
[4,5]
[0,1]
[1,2,3,4,5]
[1]
[0,1]
[1,2,3,4,5]
",
        ),
    ] {
        let (stdout, code) = run_jq_stdin(filter, input, &["-c"])?;
        assert_eq!(
            (stdout.as_str(), code),
            (expected, 0),
            "{filter} on {input}"
        );
    }
    Ok(())
}
