//! #6833: a call to a user `def` with no parameters whose body is a string's `length` (or another
//! count the string door answers) is answered from the string, as the same body written inline is,
//! instead of through the reindex bridge once per call. Every row is `jq -nc PROGRAM` against what
//! jq 1.7.1 (`/usr/bin/jq`) prints, with exit status 0: the shadowing, redefinition, parameter,
//! non-string, error and closure-capturing rows are the ones a door that read the body too loosely
//! would change.

use anyhow::Result;

use crate::run_jq_full;

const ROWS: &[(&str, &str)] = &[
    (
        r#"def f: length; "héllo" | ascii_downcase | . as $t | [range(3) | ($t | f)]"#,
        "[5,5,5]\n",
    ),
    (
        r#"def f: utf8bytelength; "héllo" | ascii_downcase | . as $t | [range(3) | ($t | f)]"#,
        "[6,6,6]\n",
    ),
    (
        r#"def f: length > 3; "héllo" | ascii_downcase | . as $t | [range(3) | ($t | f)]"#,
        "[true,true,true]\n",
    ),
    (
        r#"def f: startswith("h"); "héllo" | ascii_downcase | . as $t | [range(3) | ($t | f)]"#,
        "[true,true,true]\n",
    ),
    (
        r#"def f: if length > 3 then "long" else "short" end; "héllo" | ascii_downcase | . as $t | [range(3) | ($t | f)]"#,
        "[\"long\",\"long\",\"long\"]\n",
    ),
    (
        r#"def length: 5; def f: length; "héllo" | ascii_downcase | . as $t | [range(2) | ($t | f)]"#,
        "[5,5]\n",
    ),
    (
        r#"def f: length; def length: 9; "héllo" | ascii_downcase | . as $t | [range(2) | ($t | f)]"#,
        "[5,5]\n",
    ),
    (
        r#"def f: length; def g: f; def f: 7; "héllo" | ascii_downcase | . as $t | [range(2) | ($t | g)]"#,
        "[5,5]\n",
    ),
    (
        r#"def f: length; def g: f; def f: 7; "héllo" | ascii_downcase | . as $t | [range(2) | ($t | f)]"#,
        "[7,7]\n",
    ),
    (
        r#"def f(x): length; "héllo" | ascii_downcase | . as $t | [range(2) | ($t | f(1))]"#,
        "[5,5]\n",
    ),
    (
        r#"def g: length; def f: g; "héllo" | ascii_downcase | . as $t | [range(2) | ($t | f)]"#,
        "[5,5]\n",
    ),
    (
        r"def f: length; [1,2,3] | map(.) | . as $t | [range(2) | ($t | f)]",
        "[3,3]\n",
    ),
    (
        r#"def f: length; {"a":1,"b":2} | with_entries(.) | . as $t | [range(2) | ($t | f)]"#,
        "[2,2]\n",
    ),
    (
        r"def f: length; null | . as $t | [range(2) | ($t | f)]",
        "[0,0]\n",
    ),
    (
        r"def f: length; -5 | . as $t | [range(2) | ($t | f)]",
        "[5,5]\n",
    ),
    (
        r"def f: length; true | . as $t | [range(2) | try ($t | f) catch .]",
        "[\"boolean (true) has no length\",\"boolean (true) has no length\"]\n",
    ),
    (
        r#"def f: if length > 0 then 0 else 1 end; "" | ascii_downcase | . as $t | [range(2) | ($t | f)]"#,
        "[1,1]\n",
    ),
    (
        r#"def f: [length]; "héllo" | ascii_downcase | . as $t | [range(2) | ($t | f)]"#,
        "[[5],[5]]\n",
    ),
    (
        r#"def f: length, length; "héllo" | ascii_downcase | . as $t | [range(2) | ($t | f)]"#,
        "[5,5,5,5]\n",
    ),
    (
        r#"def f: length; "héllo" | ascii_downcase | . as $t | reduce range(3) as $i (0; . + ($t | f))"#,
        "15\n",
    ),
    (
        r#"def f: length; "héllo" | ascii_downcase | . as $t | [range(2) | ($t | f | . + 1)]"#,
        "[6,6]\n",
    ),
    (
        r#"def f: length; "héllo" | ascii_downcase | . as $t | [range(2) | ($t | try f catch "e")]"#,
        "[5,5]\n",
    ),
    (
        r#"def f: length; "héllo" | ascii_downcase | . as $t | [range(2) | path($t | f)]?"#,
        "",
    ),
    (
        r#"def f: length; "héllo" | ascii_downcase | . as $t | [limit(2; range(5) | ($t | f))]"#,
        "[5,5]\n",
    ),
    (
        r#"def f: length; "héllo" | ascii_downcase | . as $t | [range(2) | . as $w | ($t | f)]"#,
        "[5,5]\n",
    ),
    (r#"def f: length; "héllo" | ascii_downcase | f"#, "5\n"),
    (
        r#"def f: length; "aé😀" | ascii_downcase | . as $t | [range(2) | ($t | f)]"#,
        "[3,3]\n",
    ),
    // A nested def that captures an enclosing parameter or variable is not the door's to answer.
    (
        r#"def g(x): def f: length > x; "héllo" | ascii_downcase | . as $t | [range(2) | ($t | f)]; g(3)"#,
        "[true,true]\n",
    ),
    (
        r#"def g(x): def f: length > x; "héllo" | ascii_downcase | . as $t | [range(2) | ($t | f)]; g(9)"#,
        "[false,false]\n",
    ),
    (
        r#"def g($x): def f: length > $x; "héllo" | ascii_downcase | . as $t | [range(2) | ($t | f)]; g(3)"#,
        "[true,true]\n",
    ),
    (
        r#"3 as $n | def f: length > $n; "héllo" | ascii_downcase | . as $t | [range(2) | ($t | f)]"#,
        "[true,true]\n",
    ),
    (
        r#"9 as $n | def f: length > $n; "héllo" | ascii_downcase | . as $t | [range(2) | ($t | f)]"#,
        "[false,false]\n",
    ),
    (
        r#"def g(x): def f: length + x; "héllo" | ascii_downcase | . as $t | [range(2) | ($t | f)]; g(10)"#,
        "[15,15]\n",
    ),
    (
        r#"def g(x): def f: startswith(x); "héllo" | ascii_downcase | . as $t | [range(2) | ($t | f)]; g("h")"#,
        "[true,true]\n",
    ),
    (
        r#"def g(x): def f: startswith(x); "héllo" | ascii_downcase | . as $t | [range(2) | ($t | f)]; g("z")"#,
        "[false,false]\n",
    ),
    (
        r#""h" as $p | def f: startswith($p); "héllo" | ascii_downcase | . as $t | [range(2) | ($t | f)]"#,
        "[true,true]\n",
    ),
    (
        r#"def g(x): def f: if length > x then "long" else "short" end; "héllo" | ascii_downcase | . as $t | reduce range(2) as $i ([]; . + [$t | f]); g(9)"#,
        "[\"short\",\"short\"]\n",
    ),
    (
        r#"def f: length; def g: def f: 1; f; "héllo" | ascii_downcase | . as $t | [range(2) | ($t | g)]"#,
        "[1,1]\n",
    ),
];

#[test]
fn test_6833_a_def_of_length_answers_as_jq_does() -> Result<()> {
    for (program, stdout) in ROWS {
        let (out, err, code) = run_jq_full(&["-nc", program], None)?;
        assert_eq!(
            (out.as_str(), err.as_str(), code),
            (*stdout, "", 0),
            "{program}"
        );
    }
    Ok(())
}
