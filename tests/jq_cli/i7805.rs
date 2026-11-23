//! #7805: a regex group name is read as Oniguruma's `fetch_name` reads one it defines. Its first
//! character must be a word character other than an ASCII digit (`(?<(>x)` is `invalid char in group
//! name <(>`, `(?<-a>x)` `invalid group name <-a>`, `(?<\u20ac>x)` is refused, `(?<\u00b2a>x)` is not),
//! every later character up to the closing delimiter is accepted and is not pattern text
//! (`(?<a[>a)` is a group named `a[`), and a `)` or the end of the pattern before the delimiter is
//! `invalid group name` with Oniguruma's cut of the name. Each row is captured from jq 1.7.1
//! (`/usr/bin/jq`), and jq 1.8.2 gives the same.

use anyhow::Result;

use crate::run_jq_full;

/// Every match of `$r.re` in `$r.s` with its groups' names, or the caught error.
const PROGRAM: &str = r#". as $r | $r.s | try ([match($r.re; "g") | [.offset, .length, (.captures | map(.name))]] | tojson) catch ."#;

/// `(row, jq 1.7.1's output)`.
const ROWS: &[(&str, &str)] = &[
    (
        r#"{"re": "(?<(>)", "s": "x"}"#,
        r#""Regex failure: invalid char in group name <(>""#,
    ),
    (
        r#"{"re": "(?<-a>x)", "s": "x"}"#,
        r#""Regex failure: invalid group name <-a>""#,
    ),
    (
        r#"{"re": "(?<(>x)", "s": "x"}"#,
        r#""Regex failure: invalid char in group name <(>""#,
    ),
    (
        r#"{"re": "(?<\u20ac>x)", "s": "x"}"#,
        r#""Regex failure: invalid char in group name <€>""#,
    ),
    (r#"{"re": "(?<a[>a)", "s": "a"}"#, r#""[[0,1,[\"a[\"]]]""#),
    (
        r#"{"re": "(?'a[')", "s": "a"}"#,
        r#""[[0,0,[\"a[\"]],[1,0,[\"a[\"]]]""#,
    ),
    (
        r#"{"re": "(?<!a>x)", "s": "x"}"#,
        r#""[[0,0,[]],[1,0,[]]]""#,
    ),
    (
        r#"{"re": "(?'!a'x)", "s": "x"}"#,
        r#""Regex failure: invalid char in group name <!a>""#,
    ),
    (r#"{"re": "(?<=a>x)", "s": "x"}"#, r#""[]""#),
    (
        r#"{"re": "(?'=a'x)", "s": "x"}"#,
        r#""Regex failure: invalid char in group name <=a>""#,
    ),
    (
        r#"{"re": "(?<#a>x)", "s": "x"}"#,
        r#""Regex failure: invalid char in group name <#a>""#,
    ),
    (
        r#"{"re": "(?'#a'x)", "s": "x"}"#,
        r#""Regex failure: invalid char in group name <#a>""#,
    ),
    (
        r#"{"re": "(?<$a>x)", "s": "x"}"#,
        r#""Regex failure: invalid char in group name <$a>""#,
    ),
    (
        r#"{"re": "(?'$a'x)", "s": "x"}"#,
        r#""Regex failure: invalid char in group name <$a>""#,
    ),
    (
        r#"{"re": "(?<.a>x)", "s": "x"}"#,
        r#""Regex failure: invalid char in group name <.a>""#,
    ),
    (
        r#"{"re": "(?'.a'x)", "s": "x"}"#,
        r#""Regex failure: invalid char in group name <.a>""#,
    ),
    (
        r#"{"re": "(?<[a>x)", "s": "x"}"#,
        r#""Regex failure: invalid char in group name <[a>""#,
    ),
    (
        r#"{"re": "(?'[a'x)", "s": "x"}"#,
        r#""Regex failure: invalid char in group name <[a>""#,
    ),
    (
        r#"{"re": "(?<]a>x)", "s": "x"}"#,
        r#""Regex failure: invalid char in group name <]a>""#,
    ),
    (
        r#"{"re": "(?']a'x)", "s": "x"}"#,
        r#""Regex failure: invalid char in group name <]a>""#,
    ),
    (
        r#"{"re": "(?<{a>x)", "s": "x"}"#,
        r#""Regex failure: invalid char in group name <{a>""#,
    ),
    (
        r#"{"re": "(?'{a'x)", "s": "x"}"#,
        r#""Regex failure: invalid char in group name <{a>""#,
    ),
    (
        r#"{"re": "(?<\\a>x)", "s": "x"}"#,
        r#""Regex failure: invalid char in group name <\\a>""#,
    ),
    (
        r#"{"re": "(?'\\a'x)", "s": "x"}"#,
        r#""Regex failure: invalid char in group name <\\a>""#,
    ),
    (
        r#"{"re": "(?<)a>x)", "s": "x"}"#,
        r#""Regex failure: invalid char in group name <)a>""#,
    ),
    (
        r#"{"re": "(?')a'x)", "s": "x"}"#,
        r#""Regex failure: invalid char in group name <)a>""#,
    ),
    (
        r#"{"re": "(?<'a>x)", "s": "x"}"#,
        r#""Regex failure: invalid char in group name <'a>""#,
    ),
    (
        r#"{"re": "(?''a'x)", "s": "x"}"#,
        r#""Regex failure: group name is empty""#,
    ),
    (
        r#"{"re": "(?<>a>x)", "s": "x"}"#,
        r#""Regex failure: group name is empty""#,
    ),
    (
        r#"{"re": "(?'>a'x)", "s": "x"}"#,
        r#""Regex failure: invalid char in group name <>a>""#,
    ),
    (
        r#"{"re": "(?<<a>x)", "s": "x"}"#,
        r#""Regex failure: invalid char in group name <<a>""#,
    ),
    (
        r#"{"re": "(?'<a'x)", "s": "x"}"#,
        r#""Regex failure: invalid char in group name <<a>""#,
    ),
    (
        r#"{"re": "(?< a>x)", "s": "x"}"#,
        r#""Regex failure: invalid char in group name < a>""#,
    ),
    (
        r#"{"re": "(?' a'x)", "s": "x"}"#,
        r#""Regex failure: invalid char in group name < a>""#,
    ),
    (
        r#"{"re": "(?<+a>x)", "s": "x"}"#,
        r#""Regex failure: invalid group name <+a>""#,
    ),
    (
        r#"{"re": "(?'+a'x)", "s": "x"}"#,
        r#""Regex failure: invalid group name <+a>""#,
    ),
    (
        r#"{"re": "(?<0a>x)", "s": "x"}"#,
        r#""Regex failure: invalid group name <0a>""#,
    ),
    (
        r#"{"re": "(?'0a'x)", "s": "x"}"#,
        r#""Regex failure: invalid group name <0a>""#,
    ),
    (r#"{"re": "(?<_a>x)", "s": "x"}"#, r#""[[0,1,[\"_a\"]]]""#),
    (r#"{"re": "(?'_a'x)", "s": "x"}"#, r#""[[0,1,[\"_a\"]]]""#),
    (
        r#"{"re": "(?<\u00a0a>x)", "s": "x"}"#,
        r#""Regex failure: invalid char in group name < a>""#,
    ),
    (
        r#"{"re": "(?'\u00a0a'x)", "s": "x"}"#,
        r#""Regex failure: invalid char in group name < a>""#,
    ),
    (
        r#"{"re": "(?<\u3000a>x)", "s": "x"}"#,
        r#""Regex failure: invalid char in group name <　a>""#,
    ),
    (
        r#"{"re": "(?'\u3000a'x)", "s": "x"}"#,
        r#""Regex failure: invalid char in group name <　a>""#,
    ),
    (
        r#"{"re": "(?<\u00e9a>x)", "s": "x"}"#,
        r#""[[0,1,[\"éa\"]]]""#,
    ),
    (
        r#"{"re": "(?'\u00e9a'x)", "s": "x"}"#,
        r#""[[0,1,[\"éa\"]]]""#,
    ),
    (
        r#"{"re": "(?<\u0301a>x)", "s": "x"}"#,
        r#""[[0,1,[\"́a\"]]]""#,
    ),
    (
        r#"{"re": "(?'\u0301a'x)", "s": "x"}"#,
        r#""[[0,1,[\"́a\"]]]""#,
    ),
    (
        r#"{"re": "(?<\u0663a>x)", "s": "x"}"#,
        r#""[[0,1,[\"٣a\"]]]""#,
    ),
    (
        r#"{"re": "(?'\u0663a'x)", "s": "x"}"#,
        r#""[[0,1,[\"٣a\"]]]""#,
    ),
    (
        r#"{"re": "(?<\u00b2a>x)", "s": "x"}"#,
        r#""[[0,1,[\"²a\"]]]""#,
    ),
    (
        r#"{"re": "(?'\u00b2a'x)", "s": "x"}"#,
        r#""[[0,1,[\"²a\"]]]""#,
    ),
    (
        r#"{"re": "(?<\u00bca>x)", "s": "x"}"#,
        r#""[[0,1,[\"¼a\"]]]""#,
    ),
    (
        r#"{"re": "(?'\u00bca'x)", "s": "x"}"#,
        r#""[[0,1,[\"¼a\"]]]""#,
    ),
    (
        r#"{"re": "(?<\ud83d\ude00a>x)", "s": "x"}"#,
        r#""Regex failure: invalid char in group name <😀a>""#,
    ),
    (
        r#"{"re": "(?'\ud83d\ude00a'x)", "s": "x"}"#,
        r#""Regex failure: invalid char in group name <😀a>""#,
    ),
    (
        r#"{"re": "(?<\u203fa>x)", "s": "x"}"#,
        r#""[[0,1,[\"‿a\"]]]""#,
    ),
    (
        r#"{"re": "(?'\u203fa'x)", "s": "x"}"#,
        r#""[[0,1,[\"‿a\"]]]""#,
    ),
    (
        r#"{"re": "(?<\u200ca>x)", "s": "x"}"#,
        r#""Regex failure: invalid char in group name <‌a>""#,
    ),
    (
        r#"{"re": "(?'\u200ca'x)", "s": "x"}"#,
        r#""Regex failure: invalid char in group name <‌a>""#,
    ),
    (
        r#"{"re": "(?<\u2160a>x)", "s": "x"}"#,
        r#""[[0,1,[\"Ⅰa\"]]]""#,
    ),
    (
        r#"{"re": "(?'\u2160a'x)", "s": "x"}"#,
        r#""[[0,1,[\"Ⅰa\"]]]""#,
    ),
    (r#"{"re": "(?<a[>x)", "s": "x"}"#, r#""[[0,1,[\"a[\"]]]""#),
    (r#"{"re": "(?'a[b'x)", "s": "x"}"#, r#""[[0,1,[\"a[b\"]]]""#),
    (r#"{"re": "(?<a(>x)", "s": "x"}"#, r#""[[0,1,[\"a(\"]]]""#),
    (r#"{"re": "(?'a(b'x)", "s": "x"}"#, r#""[[0,1,[\"a(b\"]]]""#),
    (r#"{"re": "(?<a$>x)", "s": "x"}"#, r#""[[0,1,[\"a$\"]]]""#),
    (r#"{"re": "(?'a$b'x)", "s": "x"}"#, r#""[[0,1,[\"a$b\"]]]""#),
    (r#"{"re": "(?<a{>x)", "s": "x"}"#, r#""[[0,1,[\"a{\"]]]""#),
    (r#"{"re": "(?'a{b'x)", "s": "x"}"#, r#""[[0,1,[\"a{b\"]]]""#),
    (
        r#"{"re": "(?<a\\>x)", "s": "x"}"#,
        r#""[[0,1,[\"a\\\\\"]]]""#,
    ),
    (
        r#"{"re": "(?'a\\b'x)", "s": "x"}"#,
        r#""[[0,1,[\"a\\\\b\"]]]""#,
    ),
    (r#"{"re": "(?<a->x)", "s": "x"}"#, r#""[[0,1,[\"a-\"]]]""#),
    (r#"{"re": "(?'a-b'x)", "s": "x"}"#, r#""[[0,1,[\"a-b\"]]]""#),
    (r#"{"re": "(?<a#>x)", "s": "x"}"#, r#""[[0,1,[\"a#\"]]]""#),
    (r#"{"re": "(?'a#b'x)", "s": "x"}"#, r#""[[0,1,[\"a#b\"]]]""#),
    (r#"{"re": "(?<a >x)", "s": "x"}"#, r#""[[0,1,[\"a \"]]]""#),
    (r#"{"re": "(?'a b'x)", "s": "x"}"#, r#""[[0,1,[\"a b\"]]]""#),
    (r#"{"re": "(?<a<>x)", "s": "x"}"#, r#""[[0,1,[\"a<\"]]]""#),
    (r#"{"re": "(?'a<b'x)", "s": "x"}"#, r#""[[0,1,[\"a<b\"]]]""#),
    (
        r#"{"re": "(?<a\u20ac>x)", "s": "x"}"#,
        r#""[[0,1,[\"a€\"]]]""#,
    ),
    (
        r#"{"re": "(?'a\u20acb'x)", "s": "x"}"#,
        r#""[[0,1,[\"a€b\"]]]""#,
    ),
    (r#"{"re": "(?<a]>x)", "s": "x"}"#, r#""[[0,1,[\"a]\"]]]""#),
    (r#"{"re": "(?'a]b'x)", "s": "x"}"#, r#""[[0,1,[\"a]b\"]]]""#),
    (
        r#"{"re": "(?<a", "s": "x"}"#,
        r#""Regex failure: invalid group name <a>""#,
    ),
    (
        r#"{"re": "(?<ab", "s": "x"}"#,
        r#""Regex failure: invalid group name <a>""#,
    ),
    (
        r#"{"re": "(?<\u00dfx", "s": "x"}"#,
        r#""Regex failure: invalid group name <ß>""#,
    ),
    (
        r#"{"re": "(?<a)", "s": "x"}"#,
        r#""Regex failure: invalid group name <a>""#,
    ),
    (
        r#"{"re": "(?<ab)x>", "s": "x"}"#,
        r#""Regex failure: invalid group name <ab>""#,
    ),
    (
        r#"{"re": "(?'ab", "s": "x"}"#,
        r#""Regex failure: invalid group name <a>""#,
    ),
    (
        r#"{"re": "(?'a)b'", "s": "x"}"#,
        r#""Regex failure: invalid group name <a>""#,
    ),
    (
        r#"{"re": "(?<(", "s": "x"}"#,
        r#""Regex failure: invalid char in group name <(>""#,
    ),
    (
        r#"{"re": "(?<(a", "s": "x"}"#,
        r#""Regex failure: invalid char in group name <(a>""#,
    ),
    (
        r#"{"re": "(?<(>", "s": "x"}"#,
        r#""Regex failure: invalid char in group name <(>>""#,
    ),
    (
        r#"{"re": "(?<(a)", "s": "x"}"#,
        r#""Regex failure: invalid char in group name <(a)>""#,
    ),
    (
        r#"{"re": "(?<)>x)", "s": "x"}"#,
        r#""Regex failure: invalid char in group name <)>""#,
    ),
    (
        r#"{"re": "(?<", "s": "x"}"#,
        r#""Regex failure: end pattern with unmatched parenthesis""#,
    ),
    (
        r#"{"re": "(?<>x)", "s": "x"}"#,
        r#""Regex failure: group name is empty""#,
    ),
    (
        r#"{"re": "(?''x)", "s": "x"}"#,
        r#""Regex failure: group name is empty""#,
    ),
    (
        r#"{"re": "(?<a[>x)[b]", "s": "xb"}"#,
        r#""[[0,2,[\"a[\"]]]""#,
    ),
    (
        r#"{"re": "(?<a[>x){0}y", "s": "xy"}"#,
        r#""[[1,1,[\"a[\"]]]""#,
    ),
    (r#"{"re": "(?<a$>x)$", "s": "x"}"#, r#""[[0,1,[\"a$\"]]]""#),
    (
        r#"{"re": "(?<a[>x)*y", "s": "xxy"}"#,
        r#""[[0,3,[\"a[\"]]]""#,
    ),
    (
        r#"{"re": "(?<a(>x)(y)", "s": "xy"}"#,
        r#""[[0,2,[\"a(\",null]]]""#,
    ),
    (r#"{"re": "(?<a]>x)]", "s": "x]"}"#, r#""[[0,2,[\"a]\"]]]""#),
    (
        r#"{"re": "(?<a[>(?<b]>x))", "s": "x"}"#,
        r#""[[0,1,[\"a[\",\"b]\"]]]""#,
    ),
    (
        r#"{"re": "(?<a{>x){2}", "s": "xx"}"#,
        r#""[[0,2,[\"a{\"]]]""#,
    ),
];

#[test]
fn a_group_name_is_read_as_oniguruma_reads_it_7805() -> Result<()> {
    let input: String = ROWS.iter().map(|(row, _)| format!("{row}\n")).collect();
    let (stdout, stderr, code) = run_jq_full(&["-c", PROGRAM], Some(&input))?;
    assert_eq!((code, stderr.as_str()), (0, ""));
    let got: Vec<&str> = stdout.lines().collect();
    assert_eq!(got.len(), ROWS.len(), "{stdout}");
    for ((row, expected), got) in ROWS.iter().zip(got) {
        assert_eq!(got, *expected, "{row}");
    }
    Ok(())
}
