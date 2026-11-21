//! #7520: a capture group that takes no part in an *empty* overall match is unmatched
//! (`{"offset":-1,"string":null,"length":0}`; `null` in `capture`, `scan` and `sub`'s `.`), as jq
//! 1.8.2 reports it (jqlang/jq#3238). jq 1.7.1 wrote every group of an empty match as an empty
//! string at the match's offset, so `"a" | match("(b)?") | .captures[0].offset` was 0. A group that
//! took part in the empty match still reports an empty match at the whole match's offset. Every row
//! below was captured from jq 1.8.2 (`/opt/homebrew/bin/jq -nc`); the difference from the pinned
//! 1.7.1 is recorded in the limitations page.

use anyhow::Result;

use crate::run_jq_full;

/// (filter, expected `-nc` stdout).
const ROWS: &[(&str, &str)] = &[
    ("\"a\" | match(\"(b)?\") | .captures", "[{\"offset\":-1,\"string\":null,\"length\":0,\"name\":null}]\n"),
    ("\"xyz\" | [match(\"(a)?\"; \"g\") | .captures[0].offset]", "[-1,-1,-1,-1]\n"),
    ("\"ac\" | match(\"a(b)?c\") | .captures", "[{\"offset\":-1,\"string\":null,\"length\":0,\"name\":null}]\n"),
    ("\"a\" | capture(\"(?<x>b)?\")", "{\"x\":null}\n"),
    ("\"a\" | [scan(\"(b)?\")]", "[[null],[null]]\n"),
    ("\"a\" | sub(\"(?<x>b)?\"; \"[\\(.x)]\")", "\"[null]a\"\n"),
    ("\"a\" | gsub(\"(?<x>b)?\"; \"[\\(.x)]\")", "\"[null]a[null]\"\n"),
    ("\"a\" | [match(\"(b)?(c)?\"; \"g\") | .captures]", "[[{\"offset\":-1,\"string\":null,\"length\":0,\"name\":null},{\"offset\":-1,\"string\":null,\"length\":0,\"name\":null}],[{\"offset\":-1,\"string\":null,\"length\":0,\"name\":null},{\"offset\":-1,\"string\":null,\"length\":0,\"name\":null}]]\n"),
    ("\"abc\" | [match(\"( )*\"; \"g\")]", "[{\"offset\":0,\"length\":0,\"string\":\"\",\"captures\":[{\"offset\":-1,\"string\":null,\"length\":0,\"name\":null}]},{\"offset\":1,\"length\":0,\"string\":\"\",\"captures\":[{\"offset\":-1,\"string\":null,\"length\":0,\"name\":null}]},{\"offset\":2,\"length\":0,\"string\":\"\",\"captures\":[{\"offset\":-1,\"string\":null,\"length\":0,\"name\":null}]},{\"offset\":3,\"length\":0,\"string\":\"\",\"captures\":[{\"offset\":-1,\"string\":null,\"length\":0,\"name\":null}]}]\n"),
    ("\"c\" | match(\"(?<x>a)?b?\") | .captures", "[{\"offset\":-1,\"string\":null,\"length\":0,\"name\":\"x\"}]\n"),
    ("\"b\" | match(\"(?<x>a)?b?\") | .captures", "[{\"offset\":-1,\"string\":null,\"length\":0,\"name\":\"x\"}]\n"),
    ("\"a\",\"b\",\"c\" | capture(\"(?<x>a)?b?\")", "{\"x\":\"a\"}\n{\"x\":null}\n{\"x\":null}\n"),
    ("\"\" | match(\"(a)?\") | .captures", "[{\"offset\":-1,\"string\":null,\"length\":0,\"name\":null}]\n"),
    ("\"\" | [match(\"(a)?(b)?\"; \"g\")]", "[{\"offset\":0,\"length\":0,\"string\":\"\",\"captures\":[{\"offset\":-1,\"string\":null,\"length\":0,\"name\":null},{\"offset\":-1,\"string\":null,\"length\":0,\"name\":null}]}]\n"),
    ("\"abc\" | [match(\"(x)?\"; \"g\") | {offset,length,captures}]", "[{\"offset\":0,\"length\":0,\"captures\":[{\"offset\":-1,\"string\":null,\"length\":0,\"name\":null}]},{\"offset\":1,\"length\":0,\"captures\":[{\"offset\":-1,\"string\":null,\"length\":0,\"name\":null}]},{\"offset\":2,\"length\":0,\"captures\":[{\"offset\":-1,\"string\":null,\"length\":0,\"name\":null}]},{\"offset\":3,\"length\":0,\"captures\":[{\"offset\":-1,\"string\":null,\"length\":0,\"name\":null}]}]\n"),
    ("\"abc\" | match(\"((x)?)\") | .captures", "[{\"offset\":0,\"string\":\"\",\"length\":0,\"name\":null},{\"offset\":-1,\"string\":null,\"length\":0,\"name\":null}]\n"),
    ("\"abc\" | match(\"(x)*\") | .captures", "[{\"offset\":-1,\"string\":null,\"length\":0,\"name\":null}]\n"),
    ("\"abc\" | match(\"(x){0}\") | .captures", "[{\"offset\":-1,\"string\":null,\"length\":0,\"name\":null}]\n"),
    ("\"abc\" | match(\"(?:(x)|y)?\") | .captures", "[{\"offset\":-1,\"string\":null,\"length\":0,\"name\":null}]\n"),
    ("\"abc\" | match(\"(?<n>x|)\") | .captures", "[{\"offset\":0,\"string\":\"\",\"length\":0,\"name\":\"n\"}]\n"),
    ("\"abc\" | match(\"(x|)\") | .captures", "[{\"offset\":0,\"string\":\"\",\"length\":0,\"name\":null}]\n"),
    ("\"abc\" | match(\"(a)?\") | .captures", "[{\"offset\":0,\"length\":1,\"string\":\"a\",\"name\":null}]\n"),
    ("\"bc\" | match(\"(a)?\") | .captures", "[{\"offset\":-1,\"string\":null,\"length\":0,\"name\":null}]\n"),
    ("\"abc\" | [scan(\"(x)?\")]", "[[null],[null],[null],[null]]\n"),
    ("\"abc\" | [scan(\"(x)?(y)?\")]", "[[null,null],[null,null],[null,null],[null,null]]\n"),
    ("\"abc\" | [splits(\"(x)?\")]", "[\"\",\"a\",\"b\",\"c\",\"\"]\n"),
    ("\"abc\" | [match(\"(?<a>x)?(?<b>y)?\"; \"g\") | .captures | map(.name)]", "[[\"a\",\"b\"],[\"a\",\"b\"],[\"a\",\"b\"],[\"a\",\"b\"]]\n"),
    ("\"abc\" | test(\"(x)?\")", "true\n"),
    ("\"abc\" | capture(\"(?<a>x)?(?<b>y)?\")", "{\"a\":null,\"b\":null}\n"),
    ("\"aXbX\" | gsub(\"(?<v>x)?\"; \"<\\(.v)>\"; \"i\")", "\"<null>a<X><null>b<X><null>\"\n"),
    ("\"abc\" | sub(\"(?<v>x)?\"; \"<\\(.v)>\")", "\"<null>abc\"\n"),
    ("\"abc\" | [match(\"(x)?\"; \"gn\")]", "[]\n"),
    ("\"abc\" | [match(\"(x)?$\"; \"g\") | .captures]", "[[{\"offset\":-1,\"string\":null,\"length\":0,\"name\":null}]]\n"),
    ("\"ab\" | [match(\"(?=(x)?)\"; \"g\") | .captures]", "[[{\"offset\":-1,\"string\":null,\"length\":0,\"name\":null}],[{\"offset\":-1,\"string\":null,\"length\":0,\"name\":null}],[{\"offset\":-1,\"string\":null,\"length\":0,\"name\":null}]]\n"),
    ("\"日本\" | [match(\"(語)?\"; \"g\") | {offset, captures}]", "[{\"offset\":0,\"captures\":[{\"offset\":-1,\"string\":null,\"length\":0,\"name\":null}]},{\"offset\":1,\"captures\":[{\"offset\":-1,\"string\":null,\"length\":0,\"name\":null}]},{\"offset\":1,\"captures\":[{\"offset\":-1,\"string\":null,\"length\":0,\"name\":null}]},{\"offset\":1,\"captures\":[{\"offset\":-1,\"string\":null,\"length\":0,\"name\":null}]},{\"offset\":2,\"captures\":[{\"offset\":-1,\"string\":null,\"length\":0,\"name\":null}]},{\"offset\":2,\"captures\":[{\"offset\":-1,\"string\":null,\"length\":0,\"name\":null}]},{\"offset\":2,\"captures\":[{\"offset\":-1,\"string\":null,\"length\":0,\"name\":null}]}]\n"),
    ("\"abc\" | [match(\"()\"; \"g\") | .captures]", "[[{\"offset\":0,\"string\":\"\",\"length\":0,\"name\":null}],[{\"offset\":1,\"string\":\"\",\"length\":0,\"name\":null}],[{\"offset\":2,\"string\":\"\",\"length\":0,\"name\":null}],[{\"offset\":3,\"string\":\"\",\"length\":0,\"name\":null}]]\n"),
    ("\"abc\" | [match(\"(x)?|b\"; \"g\") | .captures]", "[[{\"offset\":-1,\"string\":null,\"length\":0,\"name\":null}],[{\"offset\":-1,\"string\":null,\"length\":0,\"name\":null}],[{\"offset\":-1,\"string\":null,\"length\":0,\"name\":null}],[{\"offset\":-1,\"string\":null,\"length\":0,\"name\":null}]]\n"),
    ("\"abc\" | [match(\"b|(x)?\"; \"g\") | .captures]", "[[{\"offset\":-1,\"string\":null,\"length\":0,\"name\":null}],[{\"offset\":-1,\"string\":null,\"length\":0,\"name\":null}],[{\"offset\":-1,\"string\":null,\"length\":0,\"name\":null}],[{\"offset\":-1,\"string\":null,\"length\":0,\"name\":null}]]\n"),
    ("\"aaa\" | [match(\"(a)*\"; \"g\") | .captures]", "[[{\"offset\":2,\"length\":1,\"string\":\"a\",\"name\":null}],[{\"offset\":-1,\"string\":null,\"length\":0,\"name\":null}]]\n"),
    ("\"baaa\" | [match(\"(a)*\"; \"g\") | .captures]", "[[{\"offset\":-1,\"string\":null,\"length\":0,\"name\":null}],[{\"offset\":3,\"length\":1,\"string\":\"a\",\"name\":null}],[{\"offset\":-1,\"string\":null,\"length\":0,\"name\":null}]]\n"),
    ("\"abab\" | [match(\"(?:(a)|(b))*\"; \"g\") | .captures]", "[[{\"offset\":2,\"length\":1,\"string\":\"a\",\"name\":null},{\"offset\":3,\"length\":1,\"string\":\"b\",\"name\":null}],[{\"offset\":-1,\"string\":null,\"length\":0,\"name\":null},{\"offset\":-1,\"string\":null,\"length\":0,\"name\":null}]]\n"),
    ("\"xyz\" | [match(\"(a)*\"; \"g\") | .captures]", "[[{\"offset\":-1,\"string\":null,\"length\":0,\"name\":null}],[{\"offset\":-1,\"string\":null,\"length\":0,\"name\":null}],[{\"offset\":-1,\"string\":null,\"length\":0,\"name\":null}],[{\"offset\":-1,\"string\":null,\"length\":0,\"name\":null}]]\n"),
    ("\"xyz\" | [match(\"(a*)\"; \"g\") | .captures]", "[[{\"offset\":0,\"string\":\"\",\"length\":0,\"name\":null}],[{\"offset\":1,\"string\":\"\",\"length\":0,\"name\":null}],[{\"offset\":2,\"string\":\"\",\"length\":0,\"name\":null}],[{\"offset\":3,\"string\":\"\",\"length\":0,\"name\":null}]]\n"),
    ("\"xyz\" | [match(\"(a|)\"; \"g\") | .captures]", "[[{\"offset\":0,\"string\":\"\",\"length\":0,\"name\":null}],[{\"offset\":1,\"string\":\"\",\"length\":0,\"name\":null}],[{\"offset\":2,\"string\":\"\",\"length\":0,\"name\":null}],[{\"offset\":3,\"string\":\"\",\"length\":0,\"name\":null}]]\n"),
    ("\"xyz\" | [match(\"(?<n>a)?(?<m>b*)\"; \"g\") | .captures]", "[[{\"offset\":-1,\"string\":null,\"length\":0,\"name\":\"n\"},{\"offset\":0,\"string\":\"\",\"length\":0,\"name\":\"m\"}],[{\"offset\":-1,\"string\":null,\"length\":0,\"name\":\"n\"},{\"offset\":1,\"string\":\"\",\"length\":0,\"name\":\"m\"}],[{\"offset\":-1,\"string\":null,\"length\":0,\"name\":\"n\"},{\"offset\":2,\"string\":\"\",\"length\":0,\"name\":\"m\"}],[{\"offset\":-1,\"string\":null,\"length\":0,\"name\":\"n\"},{\"offset\":3,\"string\":\"\",\"length\":0,\"name\":\"m\"}]]\n"),
    ("\"é\" | [match(\"(a)?\"; \"g\") | {offset, captures}]", "[{\"offset\":0,\"captures\":[{\"offset\":-1,\"string\":null,\"length\":0,\"name\":null}]},{\"offset\":1,\"captures\":[{\"offset\":-1,\"string\":null,\"length\":0,\"name\":null}]},{\"offset\":1,\"captures\":[{\"offset\":-1,\"string\":null,\"length\":0,\"name\":null}]}]\n"),
    ("\"éa\" | [match(\"(a*)\"; \"g\") | {offset, captures}]", "[{\"offset\":0,\"captures\":[{\"offset\":0,\"string\":\"\",\"length\":0,\"name\":null}]},{\"offset\":1,\"captures\":[{\"offset\":1,\"string\":\"\",\"length\":0,\"name\":null}]},{\"offset\":1,\"captures\":[{\"offset\":1,\"length\":1,\"string\":\"a\",\"name\":null}]},{\"offset\":2,\"captures\":[{\"offset\":2,\"string\":\"\",\"length\":0,\"name\":null}]}]\n"),
    ("\"日本語\" | [match(\"(x)?(y*)\"; \"g\") | {offset, captures}]", "[{\"offset\":0,\"captures\":[{\"offset\":-1,\"string\":null,\"length\":0,\"name\":null},{\"offset\":0,\"string\":\"\",\"length\":0,\"name\":null}]},{\"offset\":1,\"captures\":[{\"offset\":-1,\"string\":null,\"length\":0,\"name\":null},{\"offset\":1,\"string\":\"\",\"length\":0,\"name\":null}]},{\"offset\":1,\"captures\":[{\"offset\":-1,\"string\":null,\"length\":0,\"name\":null},{\"offset\":1,\"string\":\"\",\"length\":0,\"name\":null}]},{\"offset\":1,\"captures\":[{\"offset\":-1,\"string\":null,\"length\":0,\"name\":null},{\"offset\":1,\"string\":\"\",\"length\":0,\"name\":null}]},{\"offset\":2,\"captures\":[{\"offset\":-1,\"string\":null,\"length\":0,\"name\":null},{\"offset\":2,\"string\":\"\",\"length\":0,\"name\":null}]},{\"offset\":2,\"captures\":[{\"offset\":-1,\"string\":null,\"length\":0,\"name\":null},{\"offset\":2,\"string\":\"\",\"length\":0,\"name\":null}]},{\"offset\":2,\"captures\":[{\"offset\":-1,\"string\":null,\"length\":0,\"name\":null},{\"offset\":2,\"string\":\"\",\"length\":0,\"name\":null}]},{\"offset\":3,\"captures\":[{\"offset\":-1,\"string\":null,\"length\":0,\"name\":null},{\"offset\":3,\"string\":\"\",\"length\":0,\"name\":null}]},{\"offset\":3,\"captures\":[{\"offset\":-1,\"string\":null,\"length\":0,\"name\":null},{\"offset\":3,\"string\":\"\",\"length\":0,\"name\":null}]},{\"offset\":3,\"captures\":[{\"offset\":-1,\"string\":null,\"length\":0,\"name\":null},{\"offset\":3,\"string\":\"\",\"length\":0,\"name\":null}]}]\n"),
    ("\"日本語\" | [capture(\"(?<p>x)?(?<q>y*)\"; \"g\")]", "[{\"p\":null,\"q\":\"\"},{\"p\":null,\"q\":\"\"},{\"p\":null,\"q\":\"\"},{\"p\":null,\"q\":\"\"},{\"p\":null,\"q\":\"\"},{\"p\":null,\"q\":\"\"},{\"p\":null,\"q\":\"\"},{\"p\":null,\"q\":\"\"},{\"p\":null,\"q\":\"\"},{\"p\":null,\"q\":\"\"}]\n"),
    ("\"日本語\" | [scan(\"(x)?\")]", "[[null],[null],[null],[null],[null],[null],[null],[null],[null],[null]]\n"),
    ("\"日本語\" | gsub(\"(?<p>x)?\"; \"-\\(.p)-\")", "\"-null-日-null--null--null-本-null--null--null-語-null--null--null-\"\n"),
    ("\"abc\" | gsub(\"(?<p>x)?\"; \"-\")", "\"-a-b-c-\"\n"),
    ("\"abc\" | gsub(\"\"; \"-\")", "\"-a-b-c-\"\n"),
    ("\"abc\" | gsub(\"(?<p>)\"; \"-\\(.p)\")", "\"-a-b-c-\"\n"),
    ("\"abc\" | [match(\"(?<p>)\"; \"g\") | .captures[0] ]", "[{\"offset\":0,\"string\":\"\",\"length\":0,\"name\":\"p\"},{\"offset\":1,\"string\":\"\",\"length\":0,\"name\":\"p\"},{\"offset\":2,\"string\":\"\",\"length\":0,\"name\":\"p\"},{\"offset\":3,\"string\":\"\",\"length\":0,\"name\":\"p\"}]\n"),
    ("\"abc\" | [match(\"\\\\b(x)?\"; \"g\") | .captures[0] ]", "[{\"offset\":-1,\"string\":null,\"length\":0,\"name\":null},{\"offset\":-1,\"string\":null,\"length\":0,\"name\":null}]\n"),
    ("\"abc\" | [match(\"(?=(a))\"; \"g\") | .captures ]", "[[{\"offset\":0,\"string\":\"\",\"length\":0,\"name\":null}]]\n"),
    ("\"abc\" | [match(\"(?=(a))|(b)\"; \"g\") | .captures ]", "[[{\"offset\":0,\"string\":\"\",\"length\":0,\"name\":null},{\"offset\":-1,\"string\":null,\"length\":0,\"name\":null}],[{\"offset\":-1,\"string\":null,\"length\":0,\"name\":null},{\"offset\":1,\"length\":1,\"string\":\"b\",\"name\":null}]]\n"),
    ("\"abc\" | [match(\"^(x)?\"; \"g\") | .captures ]", "[[{\"offset\":-1,\"string\":null,\"length\":0,\"name\":null}]]\n"),
    ("\"abc\" | [match(\"(x)?\"; \"gl\") | .captures ]", "[[{\"offset\":-1,\"string\":null,\"length\":0,\"name\":null}],[{\"offset\":-1,\"string\":null,\"length\":0,\"name\":null}],[{\"offset\":-1,\"string\":null,\"length\":0,\"name\":null}],[{\"offset\":-1,\"string\":null,\"length\":0,\"name\":null}]]\n"),
    ("\"abc\" | [match(\"(x)?\"; \"gx\") | .captures ]", "[[{\"offset\":-1,\"string\":null,\"length\":0,\"name\":null}],[{\"offset\":-1,\"string\":null,\"length\":0,\"name\":null}],[{\"offset\":-1,\"string\":null,\"length\":0,\"name\":null}],[{\"offset\":-1,\"string\":null,\"length\":0,\"name\":null}]]\n"),
    ("\"abc\" | [match(\"(x)?\"; \"gn\") | .captures ]", "[]\n"),
    ("\"abc\" | [match(\"(x)?(?=c)\"; \"g\") | .captures ]", "[[{\"offset\":-1,\"string\":null,\"length\":0,\"name\":null}]]\n"),
    ("\"abc\" | [match(\"(?<=(b))(x)?\"; \"g\") | .captures ]", "[[{\"offset\":2,\"string\":\"\",\"length\":0,\"name\":null},{\"offset\":-1,\"string\":null,\"length\":0,\"name\":null}]]\n"),
    ("\"abc\" | [splits(\"(x)?\")]", "[\"\",\"a\",\"b\",\"c\",\"\"]\n"),
    ("\"abc\" | ascii_downcase | [match(\"(x)?\"; null)] | length", "1\n"),
    ("\"abc\" | test(\"(x)?\")", "true\n"),
    ("\"abc\" | [match([\"(x)?\", \"g\"]) | .captures]", "[[{\"offset\":-1,\"string\":null,\"length\":0,\"name\":null}],[{\"offset\":-1,\"string\":null,\"length\":0,\"name\":null}],[{\"offset\":-1,\"string\":null,\"length\":0,\"name\":null}],[{\"offset\":-1,\"string\":null,\"length\":0,\"name\":null}]]\n"),
    ("\"a\" | [match(\"(x)?(a)?\"; \"g\") | .captures]", "[[{\"offset\":-1,\"string\":null,\"length\":0,\"name\":null},{\"offset\":0,\"length\":1,\"string\":\"a\",\"name\":null}],[{\"offset\":-1,\"string\":null,\"length\":0,\"name\":null},{\"offset\":-1,\"string\":null,\"length\":0,\"name\":null}]]\n"),
];

#[test]
fn an_unmatched_group_in_an_empty_match_is_unmatched_7520() -> Result<()> {
    for (filter, expected) in ROWS {
        let (stdout, stderr, code) = run_jq_full(&["-nc", filter], None)?;
        assert_eq!(
            (stdout.as_str(), code),
            (*expected, 0),
            "{filter}: stderr {stderr}"
        );
    }
    Ok(())
}
