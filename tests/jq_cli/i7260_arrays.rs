//! #7260: an array's storage is one heap block (count, length, capacity, elements) instead of an
//! `Rc` around a `Vec`. Nothing it prints may change: every write that grows, shrinks or reorders
//! an array now goes through the block's own `Vec`-like methods, every conversion from and to a
//! `Vec` moves the elements across, and identity (`jv_identical`, which `path($x)` reads) is still
//! the block plus the window. Every row was captured from jq 1.7.1.

use anyhow::Result;

use crate::run_jq_stdin;

const DOC: &str = r#"{"a":[1,2,3,[4,5],{"b":[6]}],"e":[]}"#;

/// `(filter, stdout)` for `-c` over [`DOC`].
const ROWS: &[(&str, &str)] = &[
    (
        r#".a[1:3] = ["x","y","z"]"#,
        r#"{"a":[1,"x","y","z",[4,5],{"b":[6]}],"e":[]}"#,
    ),
    (
        ".a[1:3] |= map(tostring)",
        r#"{"a":[1,"2","3",[4,5],{"b":[6]}],"e":[]}"#,
    ),
    ("del(.a[0,2])", r#"{"a":[2,[4,5],{"b":[6]}],"e":[]}"#),
    ("del(.a[1:3])", r#"{"a":[1,[4,5],{"b":[6]}],"e":[]}"#),
    ("del(.a[-1], .a[0])", r#"{"a":[2,3,[4,5]],"e":[]}"#),
    (
        ".a[7] = 9",
        r#"{"a":[1,2,3,[4,5],{"b":[6]},null,null,9],"e":[]}"#,
    ),
    (
        r#"setpath(["e",2]; 1)"#,
        r#"{"a":[1,2,3,[4,5],{"b":[6]}],"e":[null,null,1]}"#,
    ),
    (
        r#"delpaths([["a",3,0],["a",4,"b",0]])"#,
        r#"{"a":[1,2,3,[5],{"b":[]}],"e":[]}"#,
    ),
    (".a | [.[1:][1:][0], .[-2:][0]]", "[3,[4,5]]"),
    (
        ".a | path(.[1:3] as $y | .[1:3] | $y)",
        r#"[{"start":1,"end":3}]"#,
    ),
    (
        r#".a | try path(.[1:1] as $y | .[1:1] | $y) catch "refused""#,
        r#""refused""#,
    ),
    ("reduce range(6) as $i (.a; .[1:] + [$i])", "[1,2,3,4,5]"),
    (
        ".a | [foreach .[] as $x ([]; . + [$x]; length)]",
        "[1,2,3,4,5]",
    ),
    (
        ".e += [1,2] | .e[0] = 0",
        r#"{"a":[1,2,3,[4,5],{"b":[6]}],"e":[0,2]}"#,
    ),
    ("[.a[] | arrays] | add", "[4,5]"),
    (".a | to_entries | map(.key)", "[0,1,2,3,4]"),
    ("[limit(2; .a[])]", "[1,2]"),
    ("fromstream(tostream) == .", "true"),
    (
        ".a | (.[0], .[3][1]) |= . * 10",
        r#"[10,2,3,[4,50],{"b":[6]}]"#,
    ),
    // jq copies a slice that starts 65,536 or more elements into its array, so the copy is a
    // fresh node; one element earlier is still a view, the same node each time.
    (
        "[range(70000)] | .[65536:] | [length, .[0]]",
        "[4464,65536]",
    ),
    (
        r#"[range(70000)] | try path(.[65536:] as $y | .[65536:] | $y) catch "refused""#,
        r#""refused""#,
    ),
    (
        "[range(70000)] | path(.[65535:] as $y | .[65535:] | $y)",
        r#"[{"start":65535,"end":null}]"#,
    ),
];

#[test]
fn array_writes_and_slices_print_as_jq_7260() -> Result<()> {
    for (filter, want) in ROWS {
        let (out, code) = run_jq_stdin(filter, DOC, &["-c"])?;
        assert_eq!(code, 0, "{filter}");
        assert_eq!(out.trim_end(), *want, "{filter}");
    }
    Ok(())
}
