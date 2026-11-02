//! #5816: a string builtin over a string read as `-R`/`-Rs` input, or a `--arg` string, is
//! answered from the string itself (`test`, `index`, `rindex`, `inside` of a string literal) and
//! not by writing the string out as JSON, indexing it and decoding it back once per read. Every
//! row is a filter and a text with what jq 1.7.1 (`/usr/bin/jq`) prints and exits with, over
//! `-Rs` (the whole text), `-R` (the text as one line) and `--arg s` (`$s | FILTER`); an erroring
//! program stays the evaluator's (a bad pattern, bad flags, a non-string needle).

use anyhow::Result;

use crate::run_jq_full;

/// `(filter, text, [-Rs, -R, --arg] each (stdout, exit status))`.
type Row = (&'static str, &'static str, [(&'static str, i32); 3]);

const ROWS: &[Row] = &[
    (
        "test(\"zzz\")",
        "",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "test(\"zzz\")",
        "a",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "test(\"zzz\")",
        "hello zzz world",
        [("true\n", 0), ("true\n", 0), ("true\n", 0)],
    ),
    (
        "test(\"zzz\")",
        "héllo",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "test(\"zzz\")",
        "日本語",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "test(\"zzz\")",
        "a\nb",
        [("false\n", 0), ("false\nfalse\n", 0), ("false\n", 0)],
    ),
    (
        "test(\"zzz\")",
        "q\"b\\s",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "test(\"zzz\")",
        "zzzabc",
        [("true\n", 0), ("true\n", 0), ("true\n", 0)],
    ),
    (
        "test(\"zzz\")",
        "abczzz",
        [("true\n", 0), ("true\n", 0), ("true\n", 0)],
    ),
    (
        "test(\"zzz\")",
        "ZZZ",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "test(\"Z\";\"i\")",
        "",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "test(\"Z\";\"i\")",
        "a",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "test(\"Z\";\"i\")",
        "hello zzz world",
        [("true\n", 0), ("true\n", 0), ("true\n", 0)],
    ),
    (
        "test(\"Z\";\"i\")",
        "héllo",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "test(\"Z\";\"i\")",
        "日本語",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "test(\"Z\";\"i\")",
        "a\nb",
        [("false\n", 0), ("false\nfalse\n", 0), ("false\n", 0)],
    ),
    (
        "test(\"Z\";\"i\")",
        "q\"b\\s",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "test(\"Z\";\"i\")",
        "zzzabc",
        [("true\n", 0), ("true\n", 0), ("true\n", 0)],
    ),
    (
        "test(\"Z\";\"i\")",
        "abczzz",
        [("true\n", 0), ("true\n", 0), ("true\n", 0)],
    ),
    (
        "test(\"Z\";\"i\")",
        "ZZZ",
        [("true\n", 0), ("true\n", 0), ("true\n", 0)],
    ),
    (
        "test(\"h.l\";\"x\")",
        "",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "test(\"h.l\";\"x\")",
        "a",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "test(\"h.l\";\"x\")",
        "hello zzz world",
        [("true\n", 0), ("true\n", 0), ("true\n", 0)],
    ),
    (
        "test(\"h.l\";\"x\")",
        "héllo",
        [("true\n", 0), ("true\n", 0), ("true\n", 0)],
    ),
    (
        "test(\"h.l\";\"x\")",
        "日本語",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "test(\"h.l\";\"x\")",
        "a\nb",
        [("false\n", 0), ("false\nfalse\n", 0), ("false\n", 0)],
    ),
    (
        "test(\"h.l\";\"x\")",
        "q\"b\\s",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "test(\"h.l\";\"x\")",
        "zzzabc",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "test(\"h.l\";\"x\")",
        "abczzz",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "test(\"h.l\";\"x\")",
        "ZZZ",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "test(\"é\")",
        "",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "test(\"é\")",
        "a",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "test(\"é\")",
        "hello zzz world",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "test(\"é\")",
        "héllo",
        [("true\n", 0), ("true\n", 0), ("true\n", 0)],
    ),
    (
        "test(\"é\")",
        "日本語",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "test(\"é\")",
        "a\nb",
        [("false\n", 0), ("false\nfalse\n", 0), ("false\n", 0)],
    ),
    (
        "test(\"é\")",
        "q\"b\\s",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "test(\"é\")",
        "zzzabc",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "test(\"é\")",
        "abczzz",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "test(\"é\")",
        "ZZZ",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "test(\"^h\")",
        "",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "test(\"^h\")",
        "a",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "test(\"^h\")",
        "hello zzz world",
        [("true\n", 0), ("true\n", 0), ("true\n", 0)],
    ),
    (
        "test(\"^h\")",
        "héllo",
        [("true\n", 0), ("true\n", 0), ("true\n", 0)],
    ),
    (
        "test(\"^h\")",
        "日本語",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "test(\"^h\")",
        "a\nb",
        [("false\n", 0), ("false\nfalse\n", 0), ("false\n", 0)],
    ),
    (
        "test(\"^h\")",
        "q\"b\\s",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "test(\"^h\")",
        "zzzabc",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "test(\"^h\")",
        "abczzz",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "test(\"^h\")",
        "ZZZ",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    ("test(\"[\")", "", [("", 5), ("", 5), ("", 5)]),
    ("test(\"[\")", "a", [("", 5), ("", 5), ("", 5)]),
    (
        "test(\"[\")",
        "hello zzz world",
        [("", 5), ("", 5), ("", 5)],
    ),
    ("test(\"[\")", "héllo", [("", 5), ("", 5), ("", 5)]),
    ("test(\"[\")", "日本語", [("", 5), ("", 5), ("", 5)]),
    ("test(\"[\")", "a\nb", [("", 5), ("", 5), ("", 5)]),
    ("test(\"[\")", "q\"b\\s", [("", 5), ("", 5), ("", 5)]),
    ("test(\"[\")", "zzzabc", [("", 5), ("", 5), ("", 5)]),
    ("test(\"[\")", "abczzz", [("", 5), ("", 5), ("", 5)]),
    ("test(\"[\")", "ZZZ", [("", 5), ("", 5), ("", 5)]),
    ("test(\"a\";\"q\")", "", [("", 5), ("", 5), ("", 5)]),
    ("test(\"a\";\"q\")", "a", [("", 5), ("", 5), ("", 5)]),
    (
        "test(\"a\";\"q\")",
        "hello zzz world",
        [("", 5), ("", 5), ("", 5)],
    ),
    ("test(\"a\";\"q\")", "héllo", [("", 5), ("", 5), ("", 5)]),
    ("test(\"a\";\"q\")", "日本語", [("", 5), ("", 5), ("", 5)]),
    ("test(\"a\";\"q\")", "a\nb", [("", 5), ("", 5), ("", 5)]),
    ("test(\"a\";\"q\")", "q\"b\\s", [("", 5), ("", 5), ("", 5)]),
    ("test(\"a\";\"q\")", "zzzabc", [("", 5), ("", 5), ("", 5)]),
    ("test(\"a\";\"q\")", "abczzz", [("", 5), ("", 5), ("", 5)]),
    ("test(\"a\";\"q\")", "ZZZ", [("", 5), ("", 5), ("", 5)]),
    (
        "test([\"a\",\"i\"])",
        "",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "test([\"a\",\"i\"])",
        "a",
        [("true\n", 0), ("true\n", 0), ("true\n", 0)],
    ),
    (
        "test([\"a\",\"i\"])",
        "hello zzz world",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "test([\"a\",\"i\"])",
        "héllo",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "test([\"a\",\"i\"])",
        "日本語",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "test([\"a\",\"i\"])",
        "a\nb",
        [("true\n", 0), ("true\nfalse\n", 0), ("true\n", 0)],
    ),
    (
        "test([\"a\",\"i\"])",
        "q\"b\\s",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "test([\"a\",\"i\"])",
        "zzzabc",
        [("true\n", 0), ("true\n", 0), ("true\n", 0)],
    ),
    (
        "test([\"a\",\"i\"])",
        "abczzz",
        [("true\n", 0), ("true\n", 0), ("true\n", 0)],
    ),
    (
        "test([\"a\",\"i\"])",
        "ZZZ",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "test(\"a\";null)",
        "",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "test(\"a\";null)",
        "a",
        [("true\n", 0), ("true\n", 0), ("true\n", 0)],
    ),
    (
        "test(\"a\";null)",
        "hello zzz world",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "test(\"a\";null)",
        "héllo",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "test(\"a\";null)",
        "日本語",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "test(\"a\";null)",
        "a\nb",
        [("true\n", 0), ("true\nfalse\n", 0), ("true\n", 0)],
    ),
    (
        "test(\"a\";null)",
        "q\"b\\s",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "test(\"a\";null)",
        "zzzabc",
        [("true\n", 0), ("true\n", 0), ("true\n", 0)],
    ),
    (
        "test(\"a\";null)",
        "abczzz",
        [("true\n", 0), ("true\n", 0), ("true\n", 0)],
    ),
    (
        "test(\"a\";null)",
        "ZZZ",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "index(\"z\")",
        "",
        [("null\n", 0), ("null\n", 0), ("null\n", 0)],
    ),
    (
        "index(\"z\")",
        "a",
        [("null\n", 0), ("null\n", 0), ("null\n", 0)],
    ),
    (
        "index(\"z\")",
        "hello zzz world",
        [("6\n", 0), ("6\n", 0), ("6\n", 0)],
    ),
    (
        "index(\"z\")",
        "héllo",
        [("null\n", 0), ("null\n", 0), ("null\n", 0)],
    ),
    (
        "index(\"z\")",
        "日本語",
        [("null\n", 0), ("null\n", 0), ("null\n", 0)],
    ),
    (
        "index(\"z\")",
        "a\nb",
        [("null\n", 0), ("null\nnull\n", 0), ("null\n", 0)],
    ),
    (
        "index(\"z\")",
        "q\"b\\s",
        [("null\n", 0), ("null\n", 0), ("null\n", 0)],
    ),
    (
        "index(\"z\")",
        "zzzabc",
        [("0\n", 0), ("0\n", 0), ("0\n", 0)],
    ),
    (
        "index(\"z\")",
        "abczzz",
        [("3\n", 0), ("3\n", 0), ("3\n", 0)],
    ),
    (
        "index(\"z\")",
        "ZZZ",
        [("null\n", 0), ("null\n", 0), ("null\n", 0)],
    ),
    (
        "rindex(\"z\")",
        "",
        [("null\n", 0), ("null\n", 0), ("null\n", 0)],
    ),
    (
        "rindex(\"z\")",
        "a",
        [("null\n", 0), ("null\n", 0), ("null\n", 0)],
    ),
    (
        "rindex(\"z\")",
        "hello zzz world",
        [("8\n", 0), ("8\n", 0), ("8\n", 0)],
    ),
    (
        "rindex(\"z\")",
        "héllo",
        [("null\n", 0), ("null\n", 0), ("null\n", 0)],
    ),
    (
        "rindex(\"z\")",
        "日本語",
        [("null\n", 0), ("null\n", 0), ("null\n", 0)],
    ),
    (
        "rindex(\"z\")",
        "a\nb",
        [("null\n", 0), ("null\nnull\n", 0), ("null\n", 0)],
    ),
    (
        "rindex(\"z\")",
        "q\"b\\s",
        [("null\n", 0), ("null\n", 0), ("null\n", 0)],
    ),
    (
        "rindex(\"z\")",
        "zzzabc",
        [("2\n", 0), ("2\n", 0), ("2\n", 0)],
    ),
    (
        "rindex(\"z\")",
        "abczzz",
        [("5\n", 0), ("5\n", 0), ("5\n", 0)],
    ),
    (
        "rindex(\"z\")",
        "ZZZ",
        [("null\n", 0), ("null\n", 0), ("null\n", 0)],
    ),
    (
        "index(\"\")",
        "",
        [("null\n", 0), ("null\n", 0), ("null\n", 0)],
    ),
    (
        "index(\"\")",
        "a",
        [("null\n", 0), ("null\n", 0), ("null\n", 0)],
    ),
    (
        "index(\"\")",
        "hello zzz world",
        [("null\n", 0), ("null\n", 0), ("null\n", 0)],
    ),
    (
        "index(\"\")",
        "héllo",
        [("null\n", 0), ("null\n", 0), ("null\n", 0)],
    ),
    (
        "index(\"\")",
        "日本語",
        [("null\n", 0), ("null\n", 0), ("null\n", 0)],
    ),
    (
        "index(\"\")",
        "a\nb",
        [("null\n", 0), ("null\nnull\n", 0), ("null\n", 0)],
    ),
    (
        "index(\"\")",
        "q\"b\\s",
        [("null\n", 0), ("null\n", 0), ("null\n", 0)],
    ),
    (
        "index(\"\")",
        "zzzabc",
        [("null\n", 0), ("null\n", 0), ("null\n", 0)],
    ),
    (
        "index(\"\")",
        "abczzz",
        [("null\n", 0), ("null\n", 0), ("null\n", 0)],
    ),
    (
        "index(\"\")",
        "ZZZ",
        [("null\n", 0), ("null\n", 0), ("null\n", 0)],
    ),
    (
        "rindex(\"é\")",
        "",
        [("null\n", 0), ("null\n", 0), ("null\n", 0)],
    ),
    (
        "rindex(\"é\")",
        "a",
        [("null\n", 0), ("null\n", 0), ("null\n", 0)],
    ),
    (
        "rindex(\"é\")",
        "hello zzz world",
        [("null\n", 0), ("null\n", 0), ("null\n", 0)],
    ),
    (
        "rindex(\"é\")",
        "héllo",
        [("1\n", 0), ("1\n", 0), ("1\n", 0)],
    ),
    (
        "rindex(\"é\")",
        "日本語",
        [("null\n", 0), ("null\n", 0), ("null\n", 0)],
    ),
    (
        "rindex(\"é\")",
        "a\nb",
        [("null\n", 0), ("null\nnull\n", 0), ("null\n", 0)],
    ),
    (
        "rindex(\"é\")",
        "q\"b\\s",
        [("null\n", 0), ("null\n", 0), ("null\n", 0)],
    ),
    (
        "rindex(\"é\")",
        "zzzabc",
        [("null\n", 0), ("null\n", 0), ("null\n", 0)],
    ),
    (
        "rindex(\"é\")",
        "abczzz",
        [("null\n", 0), ("null\n", 0), ("null\n", 0)],
    ),
    (
        "rindex(\"é\")",
        "ZZZ",
        [("null\n", 0), ("null\n", 0), ("null\n", 0)],
    ),
    (
        "index(\"語\")",
        "",
        [("null\n", 0), ("null\n", 0), ("null\n", 0)],
    ),
    (
        "index(\"語\")",
        "a",
        [("null\n", 0), ("null\n", 0), ("null\n", 0)],
    ),
    (
        "index(\"語\")",
        "hello zzz world",
        [("null\n", 0), ("null\n", 0), ("null\n", 0)],
    ),
    (
        "index(\"語\")",
        "héllo",
        [("null\n", 0), ("null\n", 0), ("null\n", 0)],
    ),
    (
        "index(\"語\")",
        "日本語",
        [("6\n", 0), ("6\n", 0), ("6\n", 0)],
    ),
    (
        "index(\"語\")",
        "a\nb",
        [("null\n", 0), ("null\nnull\n", 0), ("null\n", 0)],
    ),
    (
        "index(\"語\")",
        "q\"b\\s",
        [("null\n", 0), ("null\n", 0), ("null\n", 0)],
    ),
    (
        "index(\"語\")",
        "zzzabc",
        [("null\n", 0), ("null\n", 0), ("null\n", 0)],
    ),
    (
        "index(\"語\")",
        "abczzz",
        [("null\n", 0), ("null\n", 0), ("null\n", 0)],
    ),
    (
        "index(\"語\")",
        "ZZZ",
        [("null\n", 0), ("null\n", 0), ("null\n", 0)],
    ),
    (
        "inside(\"hzzzello world\")",
        "",
        [("true\n", 0), ("true\n", 0), ("true\n", 0)],
    ),
    (
        "inside(\"hzzzello world\")",
        "a",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "inside(\"hzzzello world\")",
        "hello zzz world",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "inside(\"hzzzello world\")",
        "héllo",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "inside(\"hzzzello world\")",
        "日本語",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "inside(\"hzzzello world\")",
        "a\nb",
        [("false\n", 0), ("false\nfalse\n", 0), ("false\n", 0)],
    ),
    (
        "inside(\"hzzzello world\")",
        "q\"b\\s",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "inside(\"hzzzello world\")",
        "zzzabc",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "inside(\"hzzzello world\")",
        "abczzz",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "inside(\"hzzzello world\")",
        "ZZZ",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "inside(\"\")",
        "",
        [("true\n", 0), ("true\n", 0), ("true\n", 0)],
    ),
    (
        "inside(\"\")",
        "a",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "inside(\"\")",
        "hello zzz world",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "inside(\"\")",
        "héllo",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "inside(\"\")",
        "日本語",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "inside(\"\")",
        "a\nb",
        [("false\n", 0), ("false\nfalse\n", 0), ("false\n", 0)],
    ),
    (
        "inside(\"\")",
        "q\"b\\s",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "inside(\"\")",
        "zzzabc",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "inside(\"\")",
        "abczzz",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "inside(\"\")",
        "ZZZ",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "inside(\"héllo\")",
        "",
        [("true\n", 0), ("true\n", 0), ("true\n", 0)],
    ),
    (
        "inside(\"héllo\")",
        "a",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "inside(\"héllo\")",
        "hello zzz world",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "inside(\"héllo\")",
        "héllo",
        [("true\n", 0), ("true\n", 0), ("true\n", 0)],
    ),
    (
        "inside(\"héllo\")",
        "日本語",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "inside(\"héllo\")",
        "a\nb",
        [("false\n", 0), ("false\nfalse\n", 0), ("false\n", 0)],
    ),
    (
        "inside(\"héllo\")",
        "q\"b\\s",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "inside(\"héllo\")",
        "zzzabc",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "inside(\"héllo\")",
        "abczzz",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "inside(\"héllo\")",
        "ZZZ",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "[test(\"a\"), test(\"b\")]",
        "",
        [
            ("[false,false]\n", 0),
            ("[false,false]\n", 0),
            ("[false,false]\n", 0),
        ],
    ),
    (
        "[test(\"a\"), test(\"b\")]",
        "a",
        [
            ("[true,false]\n", 0),
            ("[true,false]\n", 0),
            ("[true,false]\n", 0),
        ],
    ),
    (
        "[test(\"a\"), test(\"b\")]",
        "hello zzz world",
        [
            ("[false,false]\n", 0),
            ("[false,false]\n", 0),
            ("[false,false]\n", 0),
        ],
    ),
    (
        "[test(\"a\"), test(\"b\")]",
        "héllo",
        [
            ("[false,false]\n", 0),
            ("[false,false]\n", 0),
            ("[false,false]\n", 0),
        ],
    ),
    (
        "[test(\"a\"), test(\"b\")]",
        "日本語",
        [
            ("[false,false]\n", 0),
            ("[false,false]\n", 0),
            ("[false,false]\n", 0),
        ],
    ),
    (
        "[test(\"a\"), test(\"b\")]",
        "a\nb",
        [
            ("[true,true]\n", 0),
            ("[true,false]\n[false,true]\n", 0),
            ("[true,true]\n", 0),
        ],
    ),
    (
        "[test(\"a\"), test(\"b\")]",
        "q\"b\\s",
        [
            ("[false,true]\n", 0),
            ("[false,true]\n", 0),
            ("[false,true]\n", 0),
        ],
    ),
    (
        "[test(\"a\"), test(\"b\")]",
        "zzzabc",
        [
            ("[true,true]\n", 0),
            ("[true,true]\n", 0),
            ("[true,true]\n", 0),
        ],
    ),
    (
        "[test(\"a\"), test(\"b\")]",
        "abczzz",
        [
            ("[true,true]\n", 0),
            ("[true,true]\n", 0),
            ("[true,true]\n", 0),
        ],
    ),
    (
        "[test(\"a\"), test(\"b\")]",
        "ZZZ",
        [
            ("[false,false]\n", 0),
            ("[false,false]\n", 0),
            ("[false,false]\n", 0),
        ],
    ),
    ("select(test(\"a\"))", "", [("", 0), ("", 0), ("", 0)]),
    (
        "select(test(\"a\"))",
        "a",
        [("\"a\"\n", 0), ("\"a\"\n", 0), ("\"a\"\n", 0)],
    ),
    (
        "select(test(\"a\"))",
        "hello zzz world",
        [("", 0), ("", 0), ("", 0)],
    ),
    ("select(test(\"a\"))", "héllo", [("", 0), ("", 0), ("", 0)]),
    ("select(test(\"a\"))", "日本語", [("", 0), ("", 0), ("", 0)]),
    (
        "select(test(\"a\"))",
        "a\nb",
        [("\"a\\nb\"\n", 0), ("\"a\"\n", 0), ("\"a\\nb\"\n", 0)],
    ),
    (
        "select(test(\"a\"))",
        "q\"b\\s",
        [("", 0), ("", 0), ("", 0)],
    ),
    (
        "select(test(\"a\"))",
        "zzzabc",
        [
            ("\"zzzabc\"\n", 0),
            ("\"zzzabc\"\n", 0),
            ("\"zzzabc\"\n", 0),
        ],
    ),
    (
        "select(test(\"a\"))",
        "abczzz",
        [
            ("\"abczzz\"\n", 0),
            ("\"abczzz\"\n", 0),
            ("\"abczzz\"\n", 0),
        ],
    ),
    ("select(test(\"a\"))", "ZZZ", [("", 0), ("", 0), ("", 0)]),
    (
        "test(\"a\") and test(\"b\")",
        "",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "test(\"a\") and test(\"b\")",
        "a",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "test(\"a\") and test(\"b\")",
        "hello zzz world",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "test(\"a\") and test(\"b\")",
        "héllo",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "test(\"a\") and test(\"b\")",
        "日本語",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "test(\"a\") and test(\"b\")",
        "a\nb",
        [("true\n", 0), ("false\nfalse\n", 0), ("true\n", 0)],
    ),
    (
        "test(\"a\") and test(\"b\")",
        "q\"b\\s",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "test(\"a\") and test(\"b\")",
        "zzzabc",
        [("true\n", 0), ("true\n", 0), ("true\n", 0)],
    ),
    (
        "test(\"a\") and test(\"b\")",
        "abczzz",
        [("true\n", 0), ("true\n", 0), ("true\n", 0)],
    ),
    (
        "test(\"a\") and test(\"b\")",
        "ZZZ",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "if index(\"a\") then 1 else 2 end",
        "",
        [("2\n", 0), ("2\n", 0), ("2\n", 0)],
    ),
    (
        "if index(\"a\") then 1 else 2 end",
        "a",
        [("1\n", 0), ("1\n", 0), ("1\n", 0)],
    ),
    (
        "if index(\"a\") then 1 else 2 end",
        "hello zzz world",
        [("2\n", 0), ("2\n", 0), ("2\n", 0)],
    ),
    (
        "if index(\"a\") then 1 else 2 end",
        "héllo",
        [("2\n", 0), ("2\n", 0), ("2\n", 0)],
    ),
    (
        "if index(\"a\") then 1 else 2 end",
        "日本語",
        [("2\n", 0), ("2\n", 0), ("2\n", 0)],
    ),
    (
        "if index(\"a\") then 1 else 2 end",
        "a\nb",
        [("1\n", 0), ("1\n2\n", 0), ("1\n", 0)],
    ),
    (
        "if index(\"a\") then 1 else 2 end",
        "q\"b\\s",
        [("2\n", 0), ("2\n", 0), ("2\n", 0)],
    ),
    (
        "if index(\"a\") then 1 else 2 end",
        "zzzabc",
        [("1\n", 0), ("1\n", 0), ("1\n", 0)],
    ),
    (
        "if index(\"a\") then 1 else 2 end",
        "abczzz",
        [("1\n", 0), ("1\n", 0), ("1\n", 0)],
    ),
    (
        "if index(\"a\") then 1 else 2 end",
        "ZZZ",
        [("2\n", 0), ("2\n", 0), ("2\n", 0)],
    ),
    (
        "try test(\"[\") catch \"c\"",
        "",
        [("\"c\"\n", 0), ("\"c\"\n", 0), ("\"c\"\n", 0)],
    ),
    (
        "try test(\"[\") catch \"c\"",
        "a",
        [("\"c\"\n", 0), ("\"c\"\n", 0), ("\"c\"\n", 0)],
    ),
    (
        "try test(\"[\") catch \"c\"",
        "hello zzz world",
        [("\"c\"\n", 0), ("\"c\"\n", 0), ("\"c\"\n", 0)],
    ),
    (
        "try test(\"[\") catch \"c\"",
        "héllo",
        [("\"c\"\n", 0), ("\"c\"\n", 0), ("\"c\"\n", 0)],
    ),
    (
        "try test(\"[\") catch \"c\"",
        "日本語",
        [("\"c\"\n", 0), ("\"c\"\n", 0), ("\"c\"\n", 0)],
    ),
    (
        "try test(\"[\") catch \"c\"",
        "a\nb",
        [("\"c\"\n", 0), ("\"c\"\n\"c\"\n", 0), ("\"c\"\n", 0)],
    ),
    (
        "try test(\"[\") catch \"c\"",
        "q\"b\\s",
        [("\"c\"\n", 0), ("\"c\"\n", 0), ("\"c\"\n", 0)],
    ),
    (
        "try test(\"[\") catch \"c\"",
        "zzzabc",
        [("\"c\"\n", 0), ("\"c\"\n", 0), ("\"c\"\n", 0)],
    ),
    (
        "try test(\"[\") catch \"c\"",
        "abczzz",
        [("\"c\"\n", 0), ("\"c\"\n", 0), ("\"c\"\n", 0)],
    ),
    (
        "try test(\"[\") catch \"c\"",
        "ZZZ",
        [("\"c\"\n", 0), ("\"c\"\n", 0), ("\"c\"\n", 0)],
    ),
    (
        "test(\"a\")?",
        "",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "test(\"a\")?",
        "a",
        [("true\n", 0), ("true\n", 0), ("true\n", 0)],
    ),
    (
        "test(\"a\")?",
        "hello zzz world",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "test(\"a\")?",
        "héllo",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "test(\"a\")?",
        "日本語",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "test(\"a\")?",
        "a\nb",
        [("true\n", 0), ("true\nfalse\n", 0), ("true\n", 0)],
    ),
    (
        "test(\"a\")?",
        "q\"b\\s",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "test(\"a\")?",
        "zzzabc",
        [("true\n", 0), ("true\n", 0), ("true\n", 0)],
    ),
    (
        "test(\"a\")?",
        "abczzz",
        [("true\n", 0), ("true\n", 0), ("true\n", 0)],
    ),
    (
        "test(\"a\")?",
        "ZZZ",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "[.,test(\"z\")]",
        "",
        [
            ("[\"\",false]\n", 0),
            ("[\"\",false]\n", 0),
            ("[\"\",false]\n", 0),
        ],
    ),
    (
        "[.,test(\"z\")]",
        "a",
        [
            ("[\"a\",false]\n", 0),
            ("[\"a\",false]\n", 0),
            ("[\"a\",false]\n", 0),
        ],
    ),
    (
        "[.,test(\"z\")]",
        "hello zzz world",
        [
            ("[\"hello zzz world\",true]\n", 0),
            ("[\"hello zzz world\",true]\n", 0),
            ("[\"hello zzz world\",true]\n", 0),
        ],
    ),
    (
        "[.,test(\"z\")]",
        "héllo",
        [
            ("[\"héllo\",false]\n", 0),
            ("[\"héllo\",false]\n", 0),
            ("[\"héllo\",false]\n", 0),
        ],
    ),
    (
        "[.,test(\"z\")]",
        "日本語",
        [
            ("[\"日本語\",false]\n", 0),
            ("[\"日本語\",false]\n", 0),
            ("[\"日本語\",false]\n", 0),
        ],
    ),
    (
        "[.,test(\"z\")]",
        "a\nb",
        [
            ("[\"a\\nb\",false]\n", 0),
            ("[\"a\",false]\n[\"b\",false]\n", 0),
            ("[\"a\\nb\",false]\n", 0),
        ],
    ),
    (
        "[.,test(\"z\")]",
        "q\"b\\s",
        [
            ("[\"q\\\"b\\\\s\",false]\n", 0),
            ("[\"q\\\"b\\\\s\",false]\n", 0),
            ("[\"q\\\"b\\\\s\",false]\n", 0),
        ],
    ),
    (
        "[.,test(\"z\")]",
        "zzzabc",
        [
            ("[\"zzzabc\",true]\n", 0),
            ("[\"zzzabc\",true]\n", 0),
            ("[\"zzzabc\",true]\n", 0),
        ],
    ),
    (
        "[.,test(\"z\")]",
        "abczzz",
        [
            ("[\"abczzz\",true]\n", 0),
            ("[\"abczzz\",true]\n", 0),
            ("[\"abczzz\",true]\n", 0),
        ],
    ),
    (
        "[.,test(\"z\")]",
        "ZZZ",
        [
            ("[\"ZZZ\",false]\n", 0),
            ("[\"ZZZ\",false]\n", 0),
            ("[\"ZZZ\",false]\n", 0),
        ],
    ),
    (
        "length > 3 and test(\"z\")",
        "",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "length > 3 and test(\"z\")",
        "a",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "length > 3 and test(\"z\")",
        "hello zzz world",
        [("true\n", 0), ("true\n", 0), ("true\n", 0)],
    ),
    (
        "length > 3 and test(\"z\")",
        "héllo",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "length > 3 and test(\"z\")",
        "日本語",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "length > 3 and test(\"z\")",
        "a\nb",
        [("false\n", 0), ("false\nfalse\n", 0), ("false\n", 0)],
    ),
    (
        "length > 3 and test(\"z\")",
        "q\"b\\s",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "length > 3 and test(\"z\")",
        "zzzabc",
        [("true\n", 0), ("true\n", 0), ("true\n", 0)],
    ),
    (
        "length > 3 and test(\"z\")",
        "abczzz",
        [("true\n", 0), ("true\n", 0), ("true\n", 0)],
    ),
    (
        "length > 3 and test(\"z\")",
        "ZZZ",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "tostring|test(\"z\")",
        "",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "tostring|test(\"z\")",
        "a",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "tostring|test(\"z\")",
        "hello zzz world",
        [("true\n", 0), ("true\n", 0), ("true\n", 0)],
    ),
    (
        "tostring|test(\"z\")",
        "héllo",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "tostring|test(\"z\")",
        "日本語",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "tostring|test(\"z\")",
        "a\nb",
        [("false\n", 0), ("false\nfalse\n", 0), ("false\n", 0)],
    ),
    (
        "tostring|test(\"z\")",
        "q\"b\\s",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "tostring|test(\"z\")",
        "zzzabc",
        [("true\n", 0), ("true\n", 0), ("true\n", 0)],
    ),
    (
        "tostring|test(\"z\")",
        "abczzz",
        [("true\n", 0), ("true\n", 0), ("true\n", 0)],
    ),
    (
        "tostring|test(\"z\")",
        "ZZZ",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        ".[1:]|test(\"z\")",
        "",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        ".[1:]|test(\"z\")",
        "a",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        ".[1:]|test(\"z\")",
        "hello zzz world",
        [("true\n", 0), ("true\n", 0), ("true\n", 0)],
    ),
    (
        ".[1:]|test(\"z\")",
        "héllo",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        ".[1:]|test(\"z\")",
        "日本語",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        ".[1:]|test(\"z\")",
        "a\nb",
        [("false\n", 0), ("false\nfalse\n", 0), ("false\n", 0)],
    ),
    (
        ".[1:]|test(\"z\")",
        "q\"b\\s",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        ".[1:]|test(\"z\")",
        "zzzabc",
        [("true\n", 0), ("true\n", 0), ("true\n", 0)],
    ),
    (
        ".[1:]|test(\"z\")",
        "abczzz",
        [("true\n", 0), ("true\n", 0), ("true\n", 0)],
    ),
    (
        ".[1:]|test(\"z\")",
        "ZZZ",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "test(\"a$\")",
        "a\n",
        [("true\n", 0), ("true\nfalse\n", 0), ("true\n", 0)],
    ),
    (
        "test(\"a$\")",
        "a\nb\n",
        [("false\n", 0), ("true\nfalse\nfalse\n", 0), ("false\n", 0)],
    ),
    (
        "test(\"a$\")",
        "\n",
        [("false\n", 0), ("false\nfalse\n", 0), ("false\n", 0)],
    ),
    (
        "test(\"b$\")",
        "a\n",
        [("false\n", 0), ("false\nfalse\n", 0), ("false\n", 0)],
    ),
    (
        "test(\"b$\")",
        "a\nb\n",
        [("true\n", 0), ("false\ntrue\nfalse\n", 0), ("true\n", 0)],
    ),
    (
        "test(\"b$\")",
        "\n",
        [("false\n", 0), ("false\nfalse\n", 0), ("false\n", 0)],
    ),
    (
        "test(\"^b\")",
        "a\n",
        [("false\n", 0), ("false\nfalse\n", 0), ("false\n", 0)],
    ),
    (
        "test(\"^b\")",
        "a\nb\n",
        [("false\n", 0), ("false\ntrue\nfalse\n", 0), ("false\n", 0)],
    ),
    (
        "test(\"^b\")",
        "\n",
        [("false\n", 0), ("false\nfalse\n", 0), ("false\n", 0)],
    ),
    (
        "test(\"a$\";\"x\")",
        "a\n",
        [("true\n", 0), ("true\nfalse\n", 0), ("true\n", 0)],
    ),
    (
        "test(\"a$\";\"x\")",
        "a\nb\n",
        [("false\n", 0), ("true\nfalse\nfalse\n", 0), ("false\n", 0)],
    ),
    (
        "test(\"a$\";\"x\")",
        "\n",
        [("false\n", 0), ("false\nfalse\n", 0), ("false\n", 0)],
    ),
];

fn check(rows: &[Row]) -> Result<()> {
    for (filter, text, expected) in rows {
        let arg_filter = format!("$s | {filter}");
        let line = format!("{text}\n");
        let runs: [(Vec<&str>, Option<&str>); 3] = [
            (vec!["-Rsc", filter], Some(text)),
            (vec!["-Rc", filter], Some(&line)),
            (vec!["-nc", "--arg", "s", text, &arg_filter], None),
        ];
        for (mode, ((args, input), (stdout, code))) in runs.iter().zip(expected).enumerate() {
            let (out, err, status) = run_jq_full(args, *input)?;
            assert_eq!(
                (out.as_str(), status),
                (*stdout, *code),
                "mode {mode} {filter} over {text:?}: {err}"
            );
        }
    }
    Ok(())
}

#[test]
fn a_string_builtin_over_a_string_input_answers_as_jq_5816() -> Result<()> {
    check(ROWS)
}

/// #5816: the string-to-string stages (`ltrimstr`, `rtrimstr`, `tostring`, `. + "lit"`) of a string
/// input, alone and as the head of a pipe, over the same three input modes, with a non-string
/// argument included.
const STAGE_ROWS: &[Row] = &[
    (
        "ltrimstr(\"x\")|length",
        "",
        [("0\n", 0), ("0\n", 0), ("0\n", 0)],
    ),
    (
        "ltrimstr(\"x\")|length",
        "a",
        [("1\n", 0), ("1\n", 0), ("1\n", 0)],
    ),
    (
        "ltrimstr(\"x\")|length",
        "hello zzz world",
        [("15\n", 0), ("15\n", 0), ("15\n", 0)],
    ),
    (
        "ltrimstr(\"x\")|length",
        "héllo",
        [("5\n", 0), ("5\n", 0), ("5\n", 0)],
    ),
    (
        "ltrimstr(\"x\")|length",
        "日本語",
        [("3\n", 0), ("3\n", 0), ("3\n", 0)],
    ),
    (
        "ltrimstr(\"x\")|length",
        "a\nb",
        [("3\n", 0), ("1\n1\n", 0), ("3\n", 0)],
    ),
    (
        "ltrimstr(\"x\")|length",
        "q\"b\\s",
        [("5\n", 0), ("5\n", 0), ("5\n", 0)],
    ),
    (
        "ltrimstr(\"x\")|length",
        "zzzabc",
        [("6\n", 0), ("6\n", 0), ("6\n", 0)],
    ),
    (
        "ltrimstr(\"x\")|length",
        "abczzz",
        [("6\n", 0), ("6\n", 0), ("6\n", 0)],
    ),
    (
        "ltrimstr(\"x\")|length",
        "zzz",
        [("3\n", 0), ("3\n", 0), ("3\n", 0)],
    ),
    (
        "ltrimstr(\"x\")|length",
        "hé",
        [("2\n", 0), ("2\n", 0), ("2\n", 0)],
    ),
    (
        "ltrimstr(\"h\")|length",
        "",
        [("0\n", 0), ("0\n", 0), ("0\n", 0)],
    ),
    (
        "ltrimstr(\"h\")|length",
        "a",
        [("1\n", 0), ("1\n", 0), ("1\n", 0)],
    ),
    (
        "ltrimstr(\"h\")|length",
        "hello zzz world",
        [("14\n", 0), ("14\n", 0), ("14\n", 0)],
    ),
    (
        "ltrimstr(\"h\")|length",
        "héllo",
        [("4\n", 0), ("4\n", 0), ("4\n", 0)],
    ),
    (
        "ltrimstr(\"h\")|length",
        "日本語",
        [("3\n", 0), ("3\n", 0), ("3\n", 0)],
    ),
    (
        "ltrimstr(\"h\")|length",
        "a\nb",
        [("3\n", 0), ("1\n1\n", 0), ("3\n", 0)],
    ),
    (
        "ltrimstr(\"h\")|length",
        "q\"b\\s",
        [("5\n", 0), ("5\n", 0), ("5\n", 0)],
    ),
    (
        "ltrimstr(\"h\")|length",
        "zzzabc",
        [("6\n", 0), ("6\n", 0), ("6\n", 0)],
    ),
    (
        "ltrimstr(\"h\")|length",
        "abczzz",
        [("6\n", 0), ("6\n", 0), ("6\n", 0)],
    ),
    (
        "ltrimstr(\"h\")|length",
        "zzz",
        [("3\n", 0), ("3\n", 0), ("3\n", 0)],
    ),
    (
        "ltrimstr(\"h\")|length",
        "hé",
        [("1\n", 0), ("1\n", 0), ("1\n", 0)],
    ),
    (
        "ltrimstr(\"h\")",
        "",
        [("\"\"\n", 0), ("\"\"\n", 0), ("\"\"\n", 0)],
    ),
    (
        "ltrimstr(\"h\")",
        "a",
        [("\"a\"\n", 0), ("\"a\"\n", 0), ("\"a\"\n", 0)],
    ),
    (
        "ltrimstr(\"h\")",
        "hello zzz world",
        [
            ("\"ello zzz world\"\n", 0),
            ("\"ello zzz world\"\n", 0),
            ("\"ello zzz world\"\n", 0),
        ],
    ),
    (
        "ltrimstr(\"h\")",
        "héllo",
        [("\"éllo\"\n", 0), ("\"éllo\"\n", 0), ("\"éllo\"\n", 0)],
    ),
    (
        "ltrimstr(\"h\")",
        "日本語",
        [
            ("\"日本語\"\n", 0),
            ("\"日本語\"\n", 0),
            ("\"日本語\"\n", 0),
        ],
    ),
    (
        "ltrimstr(\"h\")",
        "a\nb",
        [
            ("\"a\\nb\"\n", 0),
            ("\"a\"\n\"b\"\n", 0),
            ("\"a\\nb\"\n", 0),
        ],
    ),
    (
        "ltrimstr(\"h\")",
        "q\"b\\s",
        [
            ("\"q\\\"b\\\\s\"\n", 0),
            ("\"q\\\"b\\\\s\"\n", 0),
            ("\"q\\\"b\\\\s\"\n", 0),
        ],
    ),
    (
        "ltrimstr(\"h\")",
        "zzzabc",
        [
            ("\"zzzabc\"\n", 0),
            ("\"zzzabc\"\n", 0),
            ("\"zzzabc\"\n", 0),
        ],
    ),
    (
        "ltrimstr(\"h\")",
        "abczzz",
        [
            ("\"abczzz\"\n", 0),
            ("\"abczzz\"\n", 0),
            ("\"abczzz\"\n", 0),
        ],
    ),
    (
        "ltrimstr(\"h\")",
        "zzz",
        [("\"zzz\"\n", 0), ("\"zzz\"\n", 0), ("\"zzz\"\n", 0)],
    ),
    (
        "ltrimstr(\"h\")",
        "hé",
        [("\"é\"\n", 0), ("\"é\"\n", 0), ("\"é\"\n", 0)],
    ),
    (
        "rtrimstr(\"o\")",
        "",
        [("\"\"\n", 0), ("\"\"\n", 0), ("\"\"\n", 0)],
    ),
    (
        "rtrimstr(\"o\")",
        "a",
        [("\"a\"\n", 0), ("\"a\"\n", 0), ("\"a\"\n", 0)],
    ),
    (
        "rtrimstr(\"o\")",
        "hello zzz world",
        [
            ("\"hello zzz world\"\n", 0),
            ("\"hello zzz world\"\n", 0),
            ("\"hello zzz world\"\n", 0),
        ],
    ),
    (
        "rtrimstr(\"o\")",
        "héllo",
        [("\"héll\"\n", 0), ("\"héll\"\n", 0), ("\"héll\"\n", 0)],
    ),
    (
        "rtrimstr(\"o\")",
        "日本語",
        [
            ("\"日本語\"\n", 0),
            ("\"日本語\"\n", 0),
            ("\"日本語\"\n", 0),
        ],
    ),
    (
        "rtrimstr(\"o\")",
        "a\nb",
        [
            ("\"a\\nb\"\n", 0),
            ("\"a\"\n\"b\"\n", 0),
            ("\"a\\nb\"\n", 0),
        ],
    ),
    (
        "rtrimstr(\"o\")",
        "q\"b\\s",
        [
            ("\"q\\\"b\\\\s\"\n", 0),
            ("\"q\\\"b\\\\s\"\n", 0),
            ("\"q\\\"b\\\\s\"\n", 0),
        ],
    ),
    (
        "rtrimstr(\"o\")",
        "zzzabc",
        [
            ("\"zzzabc\"\n", 0),
            ("\"zzzabc\"\n", 0),
            ("\"zzzabc\"\n", 0),
        ],
    ),
    (
        "rtrimstr(\"o\")",
        "abczzz",
        [
            ("\"abczzz\"\n", 0),
            ("\"abczzz\"\n", 0),
            ("\"abczzz\"\n", 0),
        ],
    ),
    (
        "rtrimstr(\"o\")",
        "zzz",
        [("\"zzz\"\n", 0), ("\"zzz\"\n", 0), ("\"zzz\"\n", 0)],
    ),
    (
        "rtrimstr(\"o\")",
        "hé",
        [("\"hé\"\n", 0), ("\"hé\"\n", 0), ("\"hé\"\n", 0)],
    ),
    (
        "rtrimstr(\"o\")|length",
        "",
        [("0\n", 0), ("0\n", 0), ("0\n", 0)],
    ),
    (
        "rtrimstr(\"o\")|length",
        "a",
        [("1\n", 0), ("1\n", 0), ("1\n", 0)],
    ),
    (
        "rtrimstr(\"o\")|length",
        "hello zzz world",
        [("15\n", 0), ("15\n", 0), ("15\n", 0)],
    ),
    (
        "rtrimstr(\"o\")|length",
        "héllo",
        [("4\n", 0), ("4\n", 0), ("4\n", 0)],
    ),
    (
        "rtrimstr(\"o\")|length",
        "日本語",
        [("3\n", 0), ("3\n", 0), ("3\n", 0)],
    ),
    (
        "rtrimstr(\"o\")|length",
        "a\nb",
        [("3\n", 0), ("1\n1\n", 0), ("3\n", 0)],
    ),
    (
        "rtrimstr(\"o\")|length",
        "q\"b\\s",
        [("5\n", 0), ("5\n", 0), ("5\n", 0)],
    ),
    (
        "rtrimstr(\"o\")|length",
        "zzzabc",
        [("6\n", 0), ("6\n", 0), ("6\n", 0)],
    ),
    (
        "rtrimstr(\"o\")|length",
        "abczzz",
        [("6\n", 0), ("6\n", 0), ("6\n", 0)],
    ),
    (
        "rtrimstr(\"o\")|length",
        "zzz",
        [("3\n", 0), ("3\n", 0), ("3\n", 0)],
    ),
    (
        "rtrimstr(\"o\")|length",
        "hé",
        [("2\n", 0), ("2\n", 0), ("2\n", 0)],
    ),
    (
        "rtrimstr(\"\")",
        "",
        [("\"\"\n", 0), ("\"\"\n", 0), ("\"\"\n", 0)],
    ),
    (
        "rtrimstr(\"\")",
        "a",
        [("\"a\"\n", 0), ("\"a\"\n", 0), ("\"a\"\n", 0)],
    ),
    (
        "rtrimstr(\"\")",
        "hello zzz world",
        [
            ("\"hello zzz world\"\n", 0),
            ("\"hello zzz world\"\n", 0),
            ("\"hello zzz world\"\n", 0),
        ],
    ),
    (
        "rtrimstr(\"\")",
        "héllo",
        [("\"héllo\"\n", 0), ("\"héllo\"\n", 0), ("\"héllo\"\n", 0)],
    ),
    (
        "rtrimstr(\"\")",
        "日本語",
        [
            ("\"日本語\"\n", 0),
            ("\"日本語\"\n", 0),
            ("\"日本語\"\n", 0),
        ],
    ),
    (
        "rtrimstr(\"\")",
        "a\nb",
        [
            ("\"a\\nb\"\n", 0),
            ("\"a\"\n\"b\"\n", 0),
            ("\"a\\nb\"\n", 0),
        ],
    ),
    (
        "rtrimstr(\"\")",
        "q\"b\\s",
        [
            ("\"q\\\"b\\\\s\"\n", 0),
            ("\"q\\\"b\\\\s\"\n", 0),
            ("\"q\\\"b\\\\s\"\n", 0),
        ],
    ),
    (
        "rtrimstr(\"\")",
        "zzzabc",
        [
            ("\"zzzabc\"\n", 0),
            ("\"zzzabc\"\n", 0),
            ("\"zzzabc\"\n", 0),
        ],
    ),
    (
        "rtrimstr(\"\")",
        "abczzz",
        [
            ("\"abczzz\"\n", 0),
            ("\"abczzz\"\n", 0),
            ("\"abczzz\"\n", 0),
        ],
    ),
    (
        "rtrimstr(\"\")",
        "zzz",
        [("\"zzz\"\n", 0), ("\"zzz\"\n", 0), ("\"zzz\"\n", 0)],
    ),
    (
        "rtrimstr(\"\")",
        "hé",
        [("\"hé\"\n", 0), ("\"hé\"\n", 0), ("\"hé\"\n", 0)],
    ),
    (
        "ltrimstr(\"\")|length",
        "",
        [("0\n", 0), ("0\n", 0), ("0\n", 0)],
    ),
    (
        "ltrimstr(\"\")|length",
        "a",
        [("1\n", 0), ("1\n", 0), ("1\n", 0)],
    ),
    (
        "ltrimstr(\"\")|length",
        "hello zzz world",
        [("15\n", 0), ("15\n", 0), ("15\n", 0)],
    ),
    (
        "ltrimstr(\"\")|length",
        "héllo",
        [("5\n", 0), ("5\n", 0), ("5\n", 0)],
    ),
    (
        "ltrimstr(\"\")|length",
        "日本語",
        [("3\n", 0), ("3\n", 0), ("3\n", 0)],
    ),
    (
        "ltrimstr(\"\")|length",
        "a\nb",
        [("3\n", 0), ("1\n1\n", 0), ("3\n", 0)],
    ),
    (
        "ltrimstr(\"\")|length",
        "q\"b\\s",
        [("5\n", 0), ("5\n", 0), ("5\n", 0)],
    ),
    (
        "ltrimstr(\"\")|length",
        "zzzabc",
        [("6\n", 0), ("6\n", 0), ("6\n", 0)],
    ),
    (
        "ltrimstr(\"\")|length",
        "abczzz",
        [("6\n", 0), ("6\n", 0), ("6\n", 0)],
    ),
    (
        "ltrimstr(\"\")|length",
        "zzz",
        [("3\n", 0), ("3\n", 0), ("3\n", 0)],
    ),
    (
        "ltrimstr(\"\")|length",
        "hé",
        [("2\n", 0), ("2\n", 0), ("2\n", 0)],
    ),
    (
        "tostring",
        "",
        [("\"\"\n", 0), ("\"\"\n", 0), ("\"\"\n", 0)],
    ),
    (
        "tostring",
        "a",
        [("\"a\"\n", 0), ("\"a\"\n", 0), ("\"a\"\n", 0)],
    ),
    (
        "tostring",
        "hello zzz world",
        [
            ("\"hello zzz world\"\n", 0),
            ("\"hello zzz world\"\n", 0),
            ("\"hello zzz world\"\n", 0),
        ],
    ),
    (
        "tostring",
        "héllo",
        [("\"héllo\"\n", 0), ("\"héllo\"\n", 0), ("\"héllo\"\n", 0)],
    ),
    (
        "tostring",
        "日本語",
        [
            ("\"日本語\"\n", 0),
            ("\"日本語\"\n", 0),
            ("\"日本語\"\n", 0),
        ],
    ),
    (
        "tostring",
        "a\nb",
        [
            ("\"a\\nb\"\n", 0),
            ("\"a\"\n\"b\"\n", 0),
            ("\"a\\nb\"\n", 0),
        ],
    ),
    (
        "tostring",
        "q\"b\\s",
        [
            ("\"q\\\"b\\\\s\"\n", 0),
            ("\"q\\\"b\\\\s\"\n", 0),
            ("\"q\\\"b\\\\s\"\n", 0),
        ],
    ),
    (
        "tostring",
        "zzzabc",
        [
            ("\"zzzabc\"\n", 0),
            ("\"zzzabc\"\n", 0),
            ("\"zzzabc\"\n", 0),
        ],
    ),
    (
        "tostring",
        "abczzz",
        [
            ("\"abczzz\"\n", 0),
            ("\"abczzz\"\n", 0),
            ("\"abczzz\"\n", 0),
        ],
    ),
    (
        "tostring",
        "zzz",
        [("\"zzz\"\n", 0), ("\"zzz\"\n", 0), ("\"zzz\"\n", 0)],
    ),
    (
        "tostring",
        "hé",
        [("\"hé\"\n", 0), ("\"hé\"\n", 0), ("\"hé\"\n", 0)],
    ),
    ("tostring|length", "", [("0\n", 0), ("0\n", 0), ("0\n", 0)]),
    ("tostring|length", "a", [("1\n", 0), ("1\n", 0), ("1\n", 0)]),
    (
        "tostring|length",
        "hello zzz world",
        [("15\n", 0), ("15\n", 0), ("15\n", 0)],
    ),
    (
        "tostring|length",
        "héllo",
        [("5\n", 0), ("5\n", 0), ("5\n", 0)],
    ),
    (
        "tostring|length",
        "日本語",
        [("3\n", 0), ("3\n", 0), ("3\n", 0)],
    ),
    (
        "tostring|length",
        "a\nb",
        [("3\n", 0), ("1\n1\n", 0), ("3\n", 0)],
    ),
    (
        "tostring|length",
        "q\"b\\s",
        [("5\n", 0), ("5\n", 0), ("5\n", 0)],
    ),
    (
        "tostring|length",
        "zzzabc",
        [("6\n", 0), ("6\n", 0), ("6\n", 0)],
    ),
    (
        "tostring|length",
        "abczzz",
        [("6\n", 0), ("6\n", 0), ("6\n", 0)],
    ),
    (
        "tostring|length",
        "zzz",
        [("3\n", 0), ("3\n", 0), ("3\n", 0)],
    ),
    (
        "tostring|length",
        "hé",
        [("2\n", 0), ("2\n", 0), ("2\n", 0)],
    ),
    (
        ". + \"x\"",
        "",
        [("\"x\"\n", 0), ("\"x\"\n", 0), ("\"x\"\n", 0)],
    ),
    (
        ". + \"x\"",
        "a",
        [("\"ax\"\n", 0), ("\"ax\"\n", 0), ("\"ax\"\n", 0)],
    ),
    (
        ". + \"x\"",
        "hello zzz world",
        [
            ("\"hello zzz worldx\"\n", 0),
            ("\"hello zzz worldx\"\n", 0),
            ("\"hello zzz worldx\"\n", 0),
        ],
    ),
    (
        ". + \"x\"",
        "héllo",
        [
            ("\"héllox\"\n", 0),
            ("\"héllox\"\n", 0),
            ("\"héllox\"\n", 0),
        ],
    ),
    (
        ". + \"x\"",
        "日本語",
        [
            ("\"日本語x\"\n", 0),
            ("\"日本語x\"\n", 0),
            ("\"日本語x\"\n", 0),
        ],
    ),
    (
        ". + \"x\"",
        "a\nb",
        [
            ("\"a\\nbx\"\n", 0),
            ("\"ax\"\n\"bx\"\n", 0),
            ("\"a\\nbx\"\n", 0),
        ],
    ),
    (
        ". + \"x\"",
        "q\"b\\s",
        [
            ("\"q\\\"b\\\\sx\"\n", 0),
            ("\"q\\\"b\\\\sx\"\n", 0),
            ("\"q\\\"b\\\\sx\"\n", 0),
        ],
    ),
    (
        ". + \"x\"",
        "zzzabc",
        [
            ("\"zzzabcx\"\n", 0),
            ("\"zzzabcx\"\n", 0),
            ("\"zzzabcx\"\n", 0),
        ],
    ),
    (
        ". + \"x\"",
        "abczzz",
        [
            ("\"abczzzx\"\n", 0),
            ("\"abczzzx\"\n", 0),
            ("\"abczzzx\"\n", 0),
        ],
    ),
    (
        ". + \"x\"",
        "zzz",
        [("\"zzzx\"\n", 0), ("\"zzzx\"\n", 0), ("\"zzzx\"\n", 0)],
    ),
    (
        ". + \"x\"",
        "hé",
        [("\"héx\"\n", 0), ("\"héx\"\n", 0), ("\"héx\"\n", 0)],
    ),
    (". + \"x\"|length", "", [("1\n", 0), ("1\n", 0), ("1\n", 0)]),
    (
        ". + \"x\"|length",
        "a",
        [("2\n", 0), ("2\n", 0), ("2\n", 0)],
    ),
    (
        ". + \"x\"|length",
        "hello zzz world",
        [("16\n", 0), ("16\n", 0), ("16\n", 0)],
    ),
    (
        ". + \"x\"|length",
        "héllo",
        [("6\n", 0), ("6\n", 0), ("6\n", 0)],
    ),
    (
        ". + \"x\"|length",
        "日本語",
        [("4\n", 0), ("4\n", 0), ("4\n", 0)],
    ),
    (
        ". + \"x\"|length",
        "a\nb",
        [("4\n", 0), ("2\n2\n", 0), ("4\n", 0)],
    ),
    (
        ". + \"x\"|length",
        "q\"b\\s",
        [("6\n", 0), ("6\n", 0), ("6\n", 0)],
    ),
    (
        ". + \"x\"|length",
        "zzzabc",
        [("7\n", 0), ("7\n", 0), ("7\n", 0)],
    ),
    (
        ". + \"x\"|length",
        "abczzz",
        [("7\n", 0), ("7\n", 0), ("7\n", 0)],
    ),
    (
        ". + \"x\"|length",
        "zzz",
        [("4\n", 0), ("4\n", 0), ("4\n", 0)],
    ),
    (
        ". + \"x\"|length",
        "hé",
        [("3\n", 0), ("3\n", 0), ("3\n", 0)],
    ),
    (". + \"\"|length", "", [("0\n", 0), ("0\n", 0), ("0\n", 0)]),
    (". + \"\"|length", "a", [("1\n", 0), ("1\n", 0), ("1\n", 0)]),
    (
        ". + \"\"|length",
        "hello zzz world",
        [("15\n", 0), ("15\n", 0), ("15\n", 0)],
    ),
    (
        ". + \"\"|length",
        "héllo",
        [("5\n", 0), ("5\n", 0), ("5\n", 0)],
    ),
    (
        ". + \"\"|length",
        "日本語",
        [("3\n", 0), ("3\n", 0), ("3\n", 0)],
    ),
    (
        ". + \"\"|length",
        "a\nb",
        [("3\n", 0), ("1\n1\n", 0), ("3\n", 0)],
    ),
    (
        ". + \"\"|length",
        "q\"b\\s",
        [("5\n", 0), ("5\n", 0), ("5\n", 0)],
    ),
    (
        ". + \"\"|length",
        "zzzabc",
        [("6\n", 0), ("6\n", 0), ("6\n", 0)],
    ),
    (
        ". + \"\"|length",
        "abczzz",
        [("6\n", 0), ("6\n", 0), ("6\n", 0)],
    ),
    (
        ". + \"\"|length",
        "zzz",
        [("3\n", 0), ("3\n", 0), ("3\n", 0)],
    ),
    (
        ". + \"\"|length",
        "hé",
        [("2\n", 0), ("2\n", 0), ("2\n", 0)],
    ),
    (
        "ltrimstr(\"h\")|startswith(\"é\")",
        "",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "ltrimstr(\"h\")|startswith(\"é\")",
        "a",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "ltrimstr(\"h\")|startswith(\"é\")",
        "hello zzz world",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "ltrimstr(\"h\")|startswith(\"é\")",
        "héllo",
        [("true\n", 0), ("true\n", 0), ("true\n", 0)],
    ),
    (
        "ltrimstr(\"h\")|startswith(\"é\")",
        "日本語",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "ltrimstr(\"h\")|startswith(\"é\")",
        "a\nb",
        [("false\n", 0), ("false\nfalse\n", 0), ("false\n", 0)],
    ),
    (
        "ltrimstr(\"h\")|startswith(\"é\")",
        "q\"b\\s",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "ltrimstr(\"h\")|startswith(\"é\")",
        "zzzabc",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "ltrimstr(\"h\")|startswith(\"é\")",
        "abczzz",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "ltrimstr(\"h\")|startswith(\"é\")",
        "zzz",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "ltrimstr(\"h\")|startswith(\"é\")",
        "hé",
        [("true\n", 0), ("true\n", 0), ("true\n", 0)],
    ),
    (
        "ltrimstr(1)",
        "",
        [("\"\"\n", 0), ("\"\"\n", 0), ("\"\"\n", 0)],
    ),
    (
        "ltrimstr(1)",
        "a",
        [("\"a\"\n", 0), ("\"a\"\n", 0), ("\"a\"\n", 0)],
    ),
    (
        "ltrimstr(1)",
        "hello zzz world",
        [
            ("\"hello zzz world\"\n", 0),
            ("\"hello zzz world\"\n", 0),
            ("\"hello zzz world\"\n", 0),
        ],
    ),
    (
        "ltrimstr(1)",
        "héllo",
        [("\"héllo\"\n", 0), ("\"héllo\"\n", 0), ("\"héllo\"\n", 0)],
    ),
    (
        "ltrimstr(1)",
        "日本語",
        [
            ("\"日本語\"\n", 0),
            ("\"日本語\"\n", 0),
            ("\"日本語\"\n", 0),
        ],
    ),
    (
        "ltrimstr(1)",
        "a\nb",
        [
            ("\"a\\nb\"\n", 0),
            ("\"a\"\n\"b\"\n", 0),
            ("\"a\\nb\"\n", 0),
        ],
    ),
    (
        "ltrimstr(1)",
        "q\"b\\s",
        [
            ("\"q\\\"b\\\\s\"\n", 0),
            ("\"q\\\"b\\\\s\"\n", 0),
            ("\"q\\\"b\\\\s\"\n", 0),
        ],
    ),
    (
        "ltrimstr(1)",
        "zzzabc",
        [
            ("\"zzzabc\"\n", 0),
            ("\"zzzabc\"\n", 0),
            ("\"zzzabc\"\n", 0),
        ],
    ),
    (
        "ltrimstr(1)",
        "abczzz",
        [
            ("\"abczzz\"\n", 0),
            ("\"abczzz\"\n", 0),
            ("\"abczzz\"\n", 0),
        ],
    ),
    (
        "ltrimstr(1)",
        "zzz",
        [("\"zzz\"\n", 0), ("\"zzz\"\n", 0), ("\"zzz\"\n", 0)],
    ),
    (
        "ltrimstr(1)",
        "hé",
        [("\"hé\"\n", 0), ("\"hé\"\n", 0), ("\"hé\"\n", 0)],
    ),
    (
        "ltrimstr(1)|length",
        "",
        [("0\n", 0), ("0\n", 0), ("0\n", 0)],
    ),
    (
        "ltrimstr(1)|length",
        "a",
        [("1\n", 0), ("1\n", 0), ("1\n", 0)],
    ),
    (
        "ltrimstr(1)|length",
        "hello zzz world",
        [("15\n", 0), ("15\n", 0), ("15\n", 0)],
    ),
    (
        "ltrimstr(1)|length",
        "héllo",
        [("5\n", 0), ("5\n", 0), ("5\n", 0)],
    ),
    (
        "ltrimstr(1)|length",
        "日本語",
        [("3\n", 0), ("3\n", 0), ("3\n", 0)],
    ),
    (
        "ltrimstr(1)|length",
        "a\nb",
        [("3\n", 0), ("1\n1\n", 0), ("3\n", 0)],
    ),
    (
        "ltrimstr(1)|length",
        "q\"b\\s",
        [("5\n", 0), ("5\n", 0), ("5\n", 0)],
    ),
    (
        "ltrimstr(1)|length",
        "zzzabc",
        [("6\n", 0), ("6\n", 0), ("6\n", 0)],
    ),
    (
        "ltrimstr(1)|length",
        "abczzz",
        [("6\n", 0), ("6\n", 0), ("6\n", 0)],
    ),
    (
        "ltrimstr(1)|length",
        "zzz",
        [("3\n", 0), ("3\n", 0), ("3\n", 0)],
    ),
    (
        "ltrimstr(1)|length",
        "hé",
        [("2\n", 0), ("2\n", 0), ("2\n", 0)],
    ),
    (
        "rtrimstr(null)",
        "",
        [("\"\"\n", 0), ("\"\"\n", 0), ("\"\"\n", 0)],
    ),
    (
        "rtrimstr(null)",
        "a",
        [("\"a\"\n", 0), ("\"a\"\n", 0), ("\"a\"\n", 0)],
    ),
    (
        "rtrimstr(null)",
        "hello zzz world",
        [
            ("\"hello zzz world\"\n", 0),
            ("\"hello zzz world\"\n", 0),
            ("\"hello zzz world\"\n", 0),
        ],
    ),
    (
        "rtrimstr(null)",
        "héllo",
        [("\"héllo\"\n", 0), ("\"héllo\"\n", 0), ("\"héllo\"\n", 0)],
    ),
    (
        "rtrimstr(null)",
        "日本語",
        [
            ("\"日本語\"\n", 0),
            ("\"日本語\"\n", 0),
            ("\"日本語\"\n", 0),
        ],
    ),
    (
        "rtrimstr(null)",
        "a\nb",
        [
            ("\"a\\nb\"\n", 0),
            ("\"a\"\n\"b\"\n", 0),
            ("\"a\\nb\"\n", 0),
        ],
    ),
    (
        "rtrimstr(null)",
        "q\"b\\s",
        [
            ("\"q\\\"b\\\\s\"\n", 0),
            ("\"q\\\"b\\\\s\"\n", 0),
            ("\"q\\\"b\\\\s\"\n", 0),
        ],
    ),
    (
        "rtrimstr(null)",
        "zzzabc",
        [
            ("\"zzzabc\"\n", 0),
            ("\"zzzabc\"\n", 0),
            ("\"zzzabc\"\n", 0),
        ],
    ),
    (
        "rtrimstr(null)",
        "abczzz",
        [
            ("\"abczzz\"\n", 0),
            ("\"abczzz\"\n", 0),
            ("\"abczzz\"\n", 0),
        ],
    ),
    (
        "rtrimstr(null)",
        "zzz",
        [("\"zzz\"\n", 0), ("\"zzz\"\n", 0), ("\"zzz\"\n", 0)],
    ),
    (
        "rtrimstr(null)",
        "hé",
        [("\"hé\"\n", 0), ("\"hé\"\n", 0), ("\"hé\"\n", 0)],
    ),
    (
        "[ltrimstr(\"h\"), rtrimstr(\"o\")]",
        "",
        [
            ("[\"\",\"\"]\n", 0),
            ("[\"\",\"\"]\n", 0),
            ("[\"\",\"\"]\n", 0),
        ],
    ),
    (
        "[ltrimstr(\"h\"), rtrimstr(\"o\")]",
        "a",
        [
            ("[\"a\",\"a\"]\n", 0),
            ("[\"a\",\"a\"]\n", 0),
            ("[\"a\",\"a\"]\n", 0),
        ],
    ),
    (
        "[ltrimstr(\"h\"), rtrimstr(\"o\")]",
        "hello zzz world",
        [
            ("[\"ello zzz world\",\"hello zzz world\"]\n", 0),
            ("[\"ello zzz world\",\"hello zzz world\"]\n", 0),
            ("[\"ello zzz world\",\"hello zzz world\"]\n", 0),
        ],
    ),
    (
        "[ltrimstr(\"h\"), rtrimstr(\"o\")]",
        "héllo",
        [
            ("[\"éllo\",\"héll\"]\n", 0),
            ("[\"éllo\",\"héll\"]\n", 0),
            ("[\"éllo\",\"héll\"]\n", 0),
        ],
    ),
    (
        "[ltrimstr(\"h\"), rtrimstr(\"o\")]",
        "日本語",
        [
            ("[\"日本語\",\"日本語\"]\n", 0),
            ("[\"日本語\",\"日本語\"]\n", 0),
            ("[\"日本語\",\"日本語\"]\n", 0),
        ],
    ),
    (
        "[ltrimstr(\"h\"), rtrimstr(\"o\")]",
        "a\nb",
        [
            ("[\"a\\nb\",\"a\\nb\"]\n", 0),
            ("[\"a\",\"a\"]\n[\"b\",\"b\"]\n", 0),
            ("[\"a\\nb\",\"a\\nb\"]\n", 0),
        ],
    ),
    (
        "[ltrimstr(\"h\"), rtrimstr(\"o\")]",
        "q\"b\\s",
        [
            ("[\"q\\\"b\\\\s\",\"q\\\"b\\\\s\"]\n", 0),
            ("[\"q\\\"b\\\\s\",\"q\\\"b\\\\s\"]\n", 0),
            ("[\"q\\\"b\\\\s\",\"q\\\"b\\\\s\"]\n", 0),
        ],
    ),
    (
        "[ltrimstr(\"h\"), rtrimstr(\"o\")]",
        "zzzabc",
        [
            ("[\"zzzabc\",\"zzzabc\"]\n", 0),
            ("[\"zzzabc\",\"zzzabc\"]\n", 0),
            ("[\"zzzabc\",\"zzzabc\"]\n", 0),
        ],
    ),
    (
        "[ltrimstr(\"h\"), rtrimstr(\"o\")]",
        "abczzz",
        [
            ("[\"abczzz\",\"abczzz\"]\n", 0),
            ("[\"abczzz\",\"abczzz\"]\n", 0),
            ("[\"abczzz\",\"abczzz\"]\n", 0),
        ],
    ),
    (
        "[ltrimstr(\"h\"), rtrimstr(\"o\")]",
        "zzz",
        [
            ("[\"zzz\",\"zzz\"]\n", 0),
            ("[\"zzz\",\"zzz\"]\n", 0),
            ("[\"zzz\",\"zzz\"]\n", 0),
        ],
    ),
    (
        "[ltrimstr(\"h\"), rtrimstr(\"o\")]",
        "hé",
        [
            ("[\"é\",\"hé\"]\n", 0),
            ("[\"é\",\"hé\"]\n", 0),
            ("[\"é\",\"hé\"]\n", 0),
        ],
    ),
    (
        "ltrimstr(\"h\")|ltrimstr(\"é\")|length",
        "",
        [("0\n", 0), ("0\n", 0), ("0\n", 0)],
    ),
    (
        "ltrimstr(\"h\")|ltrimstr(\"é\")|length",
        "a",
        [("1\n", 0), ("1\n", 0), ("1\n", 0)],
    ),
    (
        "ltrimstr(\"h\")|ltrimstr(\"é\")|length",
        "hello zzz world",
        [("14\n", 0), ("14\n", 0), ("14\n", 0)],
    ),
    (
        "ltrimstr(\"h\")|ltrimstr(\"é\")|length",
        "héllo",
        [("3\n", 0), ("3\n", 0), ("3\n", 0)],
    ),
    (
        "ltrimstr(\"h\")|ltrimstr(\"é\")|length",
        "日本語",
        [("3\n", 0), ("3\n", 0), ("3\n", 0)],
    ),
    (
        "ltrimstr(\"h\")|ltrimstr(\"é\")|length",
        "a\nb",
        [("3\n", 0), ("1\n1\n", 0), ("3\n", 0)],
    ),
    (
        "ltrimstr(\"h\")|ltrimstr(\"é\")|length",
        "q\"b\\s",
        [("5\n", 0), ("5\n", 0), ("5\n", 0)],
    ),
    (
        "ltrimstr(\"h\")|ltrimstr(\"é\")|length",
        "zzzabc",
        [("6\n", 0), ("6\n", 0), ("6\n", 0)],
    ),
    (
        "ltrimstr(\"h\")|ltrimstr(\"é\")|length",
        "abczzz",
        [("6\n", 0), ("6\n", 0), ("6\n", 0)],
    ),
    (
        "ltrimstr(\"h\")|ltrimstr(\"é\")|length",
        "zzz",
        [("3\n", 0), ("3\n", 0), ("3\n", 0)],
    ),
    (
        "ltrimstr(\"h\")|ltrimstr(\"é\")|length",
        "hé",
        [("0\n", 0), ("0\n", 0), ("0\n", 0)],
    ),
    (
        "tostring|tostring|length",
        "",
        [("0\n", 0), ("0\n", 0), ("0\n", 0)],
    ),
    (
        "tostring|tostring|length",
        "a",
        [("1\n", 0), ("1\n", 0), ("1\n", 0)],
    ),
    (
        "tostring|tostring|length",
        "hello zzz world",
        [("15\n", 0), ("15\n", 0), ("15\n", 0)],
    ),
    (
        "tostring|tostring|length",
        "héllo",
        [("5\n", 0), ("5\n", 0), ("5\n", 0)],
    ),
    (
        "tostring|tostring|length",
        "日本語",
        [("3\n", 0), ("3\n", 0), ("3\n", 0)],
    ),
    (
        "tostring|tostring|length",
        "a\nb",
        [("3\n", 0), ("1\n1\n", 0), ("3\n", 0)],
    ),
    (
        "tostring|tostring|length",
        "q\"b\\s",
        [("5\n", 0), ("5\n", 0), ("5\n", 0)],
    ),
    (
        "tostring|tostring|length",
        "zzzabc",
        [("6\n", 0), ("6\n", 0), ("6\n", 0)],
    ),
    (
        "tostring|tostring|length",
        "abczzz",
        [("6\n", 0), ("6\n", 0), ("6\n", 0)],
    ),
    (
        "tostring|tostring|length",
        "zzz",
        [("3\n", 0), ("3\n", 0), ("3\n", 0)],
    ),
    (
        "tostring|tostring|length",
        "hé",
        [("2\n", 0), ("2\n", 0), ("2\n", 0)],
    ),
    (
        ". + \"a\" + \"b\"|length",
        "",
        [("2\n", 0), ("2\n", 0), ("2\n", 0)],
    ),
    (
        ". + \"a\" + \"b\"|length",
        "a",
        [("3\n", 0), ("3\n", 0), ("3\n", 0)],
    ),
    (
        ". + \"a\" + \"b\"|length",
        "hello zzz world",
        [("17\n", 0), ("17\n", 0), ("17\n", 0)],
    ),
    (
        ". + \"a\" + \"b\"|length",
        "héllo",
        [("7\n", 0), ("7\n", 0), ("7\n", 0)],
    ),
    (
        ". + \"a\" + \"b\"|length",
        "日本語",
        [("5\n", 0), ("5\n", 0), ("5\n", 0)],
    ),
    (
        ". + \"a\" + \"b\"|length",
        "a\nb",
        [("5\n", 0), ("3\n3\n", 0), ("5\n", 0)],
    ),
    (
        ". + \"a\" + \"b\"|length",
        "q\"b\\s",
        [("7\n", 0), ("7\n", 0), ("7\n", 0)],
    ),
    (
        ". + \"a\" + \"b\"|length",
        "zzzabc",
        [("8\n", 0), ("8\n", 0), ("8\n", 0)],
    ),
    (
        ". + \"a\" + \"b\"|length",
        "abczzz",
        [("8\n", 0), ("8\n", 0), ("8\n", 0)],
    ),
    (
        ". + \"a\" + \"b\"|length",
        "zzz",
        [("5\n", 0), ("5\n", 0), ("5\n", 0)],
    ),
    (
        ". + \"a\" + \"b\"|length",
        "hé",
        [("4\n", 0), ("4\n", 0), ("4\n", 0)],
    ),
    (
        "select(ltrimstr(\"h\")|length>2)",
        "",
        [("", 0), ("", 0), ("", 0)],
    ),
    (
        "select(ltrimstr(\"h\")|length>2)",
        "a",
        [("", 0), ("", 0), ("", 0)],
    ),
    (
        "select(ltrimstr(\"h\")|length>2)",
        "hello zzz world",
        [
            ("\"hello zzz world\"\n", 0),
            ("\"hello zzz world\"\n", 0),
            ("\"hello zzz world\"\n", 0),
        ],
    ),
    (
        "select(ltrimstr(\"h\")|length>2)",
        "héllo",
        [("\"héllo\"\n", 0), ("\"héllo\"\n", 0), ("\"héllo\"\n", 0)],
    ),
    (
        "select(ltrimstr(\"h\")|length>2)",
        "日本語",
        [
            ("\"日本語\"\n", 0),
            ("\"日本語\"\n", 0),
            ("\"日本語\"\n", 0),
        ],
    ),
    (
        "select(ltrimstr(\"h\")|length>2)",
        "a\nb",
        [("\"a\\nb\"\n", 0), ("", 0), ("\"a\\nb\"\n", 0)],
    ),
    (
        "select(ltrimstr(\"h\")|length>2)",
        "q\"b\\s",
        [
            ("\"q\\\"b\\\\s\"\n", 0),
            ("\"q\\\"b\\\\s\"\n", 0),
            ("\"q\\\"b\\\\s\"\n", 0),
        ],
    ),
    (
        "select(ltrimstr(\"h\")|length>2)",
        "zzzabc",
        [
            ("\"zzzabc\"\n", 0),
            ("\"zzzabc\"\n", 0),
            ("\"zzzabc\"\n", 0),
        ],
    ),
    (
        "select(ltrimstr(\"h\")|length>2)",
        "abczzz",
        [
            ("\"abczzz\"\n", 0),
            ("\"abczzz\"\n", 0),
            ("\"abczzz\"\n", 0),
        ],
    ),
    (
        "select(ltrimstr(\"h\")|length>2)",
        "zzz",
        [("\"zzz\"\n", 0), ("\"zzz\"\n", 0), ("\"zzz\"\n", 0)],
    ),
    (
        "select(ltrimstr(\"h\")|length>2)",
        "hé",
        [("", 0), ("", 0), ("", 0)],
    ),
    (
        "ltrimstr(\"z\")|index(\"z\")",
        "",
        [("null\n", 0), ("null\n", 0), ("null\n", 0)],
    ),
    (
        "ltrimstr(\"z\")|index(\"z\")",
        "a",
        [("null\n", 0), ("null\n", 0), ("null\n", 0)],
    ),
    (
        "ltrimstr(\"z\")|index(\"z\")",
        "hello zzz world",
        [("6\n", 0), ("6\n", 0), ("6\n", 0)],
    ),
    (
        "ltrimstr(\"z\")|index(\"z\")",
        "héllo",
        [("null\n", 0), ("null\n", 0), ("null\n", 0)],
    ),
    (
        "ltrimstr(\"z\")|index(\"z\")",
        "日本語",
        [("null\n", 0), ("null\n", 0), ("null\n", 0)],
    ),
    (
        "ltrimstr(\"z\")|index(\"z\")",
        "a\nb",
        [("null\n", 0), ("null\nnull\n", 0), ("null\n", 0)],
    ),
    (
        "ltrimstr(\"z\")|index(\"z\")",
        "q\"b\\s",
        [("null\n", 0), ("null\n", 0), ("null\n", 0)],
    ),
    (
        "ltrimstr(\"z\")|index(\"z\")",
        "zzzabc",
        [("0\n", 0), ("0\n", 0), ("0\n", 0)],
    ),
    (
        "ltrimstr(\"z\")|index(\"z\")",
        "abczzz",
        [("3\n", 0), ("3\n", 0), ("3\n", 0)],
    ),
    (
        "ltrimstr(\"z\")|index(\"z\")",
        "zzz",
        [("0\n", 0), ("0\n", 0), ("0\n", 0)],
    ),
    (
        "ltrimstr(\"z\")|index(\"z\")",
        "hé",
        [("null\n", 0), ("null\n", 0), ("null\n", 0)],
    ),
    (
        "ltrimstr(\"a\")|test(\"b\")",
        "",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "ltrimstr(\"a\")|test(\"b\")",
        "a",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "ltrimstr(\"a\")|test(\"b\")",
        "hello zzz world",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "ltrimstr(\"a\")|test(\"b\")",
        "héllo",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "ltrimstr(\"a\")|test(\"b\")",
        "日本語",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "ltrimstr(\"a\")|test(\"b\")",
        "a\nb",
        [("true\n", 0), ("false\ntrue\n", 0), ("true\n", 0)],
    ),
    (
        "ltrimstr(\"a\")|test(\"b\")",
        "q\"b\\s",
        [("true\n", 0), ("true\n", 0), ("true\n", 0)],
    ),
    (
        "ltrimstr(\"a\")|test(\"b\")",
        "zzzabc",
        [("true\n", 0), ("true\n", 0), ("true\n", 0)],
    ),
    (
        "ltrimstr(\"a\")|test(\"b\")",
        "abczzz",
        [("true\n", 0), ("true\n", 0), ("true\n", 0)],
    ),
    (
        "ltrimstr(\"a\")|test(\"b\")",
        "zzz",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (
        "ltrimstr(\"a\")|test(\"b\")",
        "hé",
        [("false\n", 0), ("false\n", 0), ("false\n", 0)],
    ),
    (". + \"é\"|length", "", [("1\n", 0), ("1\n", 0), ("1\n", 0)]),
    (
        ". + \"é\"|length",
        "a",
        [("2\n", 0), ("2\n", 0), ("2\n", 0)],
    ),
    (
        ". + \"é\"|length",
        "hello zzz world",
        [("16\n", 0), ("16\n", 0), ("16\n", 0)],
    ),
    (
        ". + \"é\"|length",
        "héllo",
        [("6\n", 0), ("6\n", 0), ("6\n", 0)],
    ),
    (
        ". + \"é\"|length",
        "日本語",
        [("4\n", 0), ("4\n", 0), ("4\n", 0)],
    ),
    (
        ". + \"é\"|length",
        "a\nb",
        [("4\n", 0), ("2\n2\n", 0), ("4\n", 0)],
    ),
    (
        ". + \"é\"|length",
        "q\"b\\s",
        [("6\n", 0), ("6\n", 0), ("6\n", 0)],
    ),
    (
        ". + \"é\"|length",
        "zzzabc",
        [("7\n", 0), ("7\n", 0), ("7\n", 0)],
    ),
    (
        ". + \"é\"|length",
        "abczzz",
        [("7\n", 0), ("7\n", 0), ("7\n", 0)],
    ),
    (
        ". + \"é\"|length",
        "zzz",
        [("4\n", 0), ("4\n", 0), ("4\n", 0)],
    ),
    (
        ". + \"é\"|length",
        "hé",
        [("3\n", 0), ("3\n", 0), ("3\n", 0)],
    ),
    (
        "rtrimstr(\"zzz\")|length",
        "",
        [("0\n", 0), ("0\n", 0), ("0\n", 0)],
    ),
    (
        "rtrimstr(\"zzz\")|length",
        "a",
        [("1\n", 0), ("1\n", 0), ("1\n", 0)],
    ),
    (
        "rtrimstr(\"zzz\")|length",
        "hello zzz world",
        [("15\n", 0), ("15\n", 0), ("15\n", 0)],
    ),
    (
        "rtrimstr(\"zzz\")|length",
        "héllo",
        [("5\n", 0), ("5\n", 0), ("5\n", 0)],
    ),
    (
        "rtrimstr(\"zzz\")|length",
        "日本語",
        [("3\n", 0), ("3\n", 0), ("3\n", 0)],
    ),
    (
        "rtrimstr(\"zzz\")|length",
        "a\nb",
        [("3\n", 0), ("1\n1\n", 0), ("3\n", 0)],
    ),
    (
        "rtrimstr(\"zzz\")|length",
        "q\"b\\s",
        [("5\n", 0), ("5\n", 0), ("5\n", 0)],
    ),
    (
        "rtrimstr(\"zzz\")|length",
        "zzzabc",
        [("6\n", 0), ("6\n", 0), ("6\n", 0)],
    ),
    (
        "rtrimstr(\"zzz\")|length",
        "abczzz",
        [("3\n", 0), ("3\n", 0), ("3\n", 0)],
    ),
    (
        "rtrimstr(\"zzz\")|length",
        "zzz",
        [("0\n", 0), ("0\n", 0), ("0\n", 0)],
    ),
    (
        "rtrimstr(\"zzz\")|length",
        "hé",
        [("2\n", 0), ("2\n", 0), ("2\n", 0)],
    ),
    (
        "ltrimstr(\"zzz\")|rtrimstr(\"abc\")",
        "",
        [("\"\"\n", 0), ("\"\"\n", 0), ("\"\"\n", 0)],
    ),
    (
        "ltrimstr(\"zzz\")|rtrimstr(\"abc\")",
        "a",
        [("\"a\"\n", 0), ("\"a\"\n", 0), ("\"a\"\n", 0)],
    ),
    (
        "ltrimstr(\"zzz\")|rtrimstr(\"abc\")",
        "hello zzz world",
        [
            ("\"hello zzz world\"\n", 0),
            ("\"hello zzz world\"\n", 0),
            ("\"hello zzz world\"\n", 0),
        ],
    ),
    (
        "ltrimstr(\"zzz\")|rtrimstr(\"abc\")",
        "héllo",
        [("\"héllo\"\n", 0), ("\"héllo\"\n", 0), ("\"héllo\"\n", 0)],
    ),
    (
        "ltrimstr(\"zzz\")|rtrimstr(\"abc\")",
        "日本語",
        [
            ("\"日本語\"\n", 0),
            ("\"日本語\"\n", 0),
            ("\"日本語\"\n", 0),
        ],
    ),
    (
        "ltrimstr(\"zzz\")|rtrimstr(\"abc\")",
        "a\nb",
        [
            ("\"a\\nb\"\n", 0),
            ("\"a\"\n\"b\"\n", 0),
            ("\"a\\nb\"\n", 0),
        ],
    ),
    (
        "ltrimstr(\"zzz\")|rtrimstr(\"abc\")",
        "q\"b\\s",
        [
            ("\"q\\\"b\\\\s\"\n", 0),
            ("\"q\\\"b\\\\s\"\n", 0),
            ("\"q\\\"b\\\\s\"\n", 0),
        ],
    ),
    (
        "ltrimstr(\"zzz\")|rtrimstr(\"abc\")",
        "zzzabc",
        [("\"\"\n", 0), ("\"\"\n", 0), ("\"\"\n", 0)],
    ),
    (
        "ltrimstr(\"zzz\")|rtrimstr(\"abc\")",
        "abczzz",
        [
            ("\"abczzz\"\n", 0),
            ("\"abczzz\"\n", 0),
            ("\"abczzz\"\n", 0),
        ],
    ),
    (
        "ltrimstr(\"zzz\")|rtrimstr(\"abc\")",
        "zzz",
        [("\"\"\n", 0), ("\"\"\n", 0), ("\"\"\n", 0)],
    ),
    (
        "ltrimstr(\"zzz\")|rtrimstr(\"abc\")",
        "hé",
        [("\"hé\"\n", 0), ("\"hé\"\n", 0), ("\"hé\"\n", 0)],
    ),
];

#[test]
fn a_string_stage_over_a_string_input_answers_as_jq_5816() -> Result<()> {
    check(STAGE_ROWS)
}

/// #5816: a string stage in path position keeps the evaluator's route and its answer (jq refuses
/// `path(ltrimstr("h"))`, answers `path(tostring)` with `[]`).
const PATH_ROWS: &[Row] = &[
    (
        "[path(ltrimstr(\"h\"))]",
        "",
        [("[[]]\n", 0), ("[[]]\n", 0), ("[[]]\n", 0)],
    ),
    (
        "[path(ltrimstr(\"h\"))]",
        "a",
        [("[[]]\n", 0), ("[[]]\n", 0), ("[[]]\n", 0)],
    ),
    (
        "[path(ltrimstr(\"h\"))]",
        "hello zzz world",
        [("", 5), ("", 5), ("", 5)],
    ),
    (
        "[path(ltrimstr(\"h\"))]",
        "héllo",
        [("", 5), ("", 5), ("", 5)],
    ),
    (
        "[path(ltrimstr(\"h\"))]",
        "日本語",
        [("[[]]\n", 0), ("[[]]\n", 0), ("[[]]\n", 0)],
    ),
    (
        "[path(ltrimstr(\"h\"))]",
        "a\nb",
        [("[[]]\n", 0), ("[[]]\n[[]]\n", 0), ("[[]]\n", 0)],
    ),
    (
        "[path(ltrimstr(\"h\"))]",
        "q\"b\\s",
        [("[[]]\n", 0), ("[[]]\n", 0), ("[[]]\n", 0)],
    ),
    (
        "[path(ltrimstr(\"h\"))]",
        "zzzabc",
        [("[[]]\n", 0), ("[[]]\n", 0), ("[[]]\n", 0)],
    ),
    (
        "[path(ltrimstr(\"h\"))]",
        "abczzz",
        [("[[]]\n", 0), ("[[]]\n", 0), ("[[]]\n", 0)],
    ),
    (
        "[path(ltrimstr(\"h\"))]",
        "zzz",
        [("[[]]\n", 0), ("[[]]\n", 0), ("[[]]\n", 0)],
    ),
    ("[path(ltrimstr(\"h\"))]", "hé", [("", 5), ("", 5), ("", 5)]),
    (
        "[path(tostring)]",
        "",
        [("[[]]\n", 0), ("[[]]\n", 0), ("[[]]\n", 0)],
    ),
    (
        "[path(tostring)]",
        "a",
        [("[[]]\n", 0), ("[[]]\n", 0), ("[[]]\n", 0)],
    ),
    (
        "[path(tostring)]",
        "hello zzz world",
        [("[[]]\n", 0), ("[[]]\n", 0), ("[[]]\n", 0)],
    ),
    (
        "[path(tostring)]",
        "héllo",
        [("[[]]\n", 0), ("[[]]\n", 0), ("[[]]\n", 0)],
    ),
    (
        "[path(tostring)]",
        "日本語",
        [("[[]]\n", 0), ("[[]]\n", 0), ("[[]]\n", 0)],
    ),
    (
        "[path(tostring)]",
        "a\nb",
        [("[[]]\n", 0), ("[[]]\n[[]]\n", 0), ("[[]]\n", 0)],
    ),
    (
        "[path(tostring)]",
        "q\"b\\s",
        [("[[]]\n", 0), ("[[]]\n", 0), ("[[]]\n", 0)],
    ),
    (
        "[path(tostring)]",
        "zzzabc",
        [("[[]]\n", 0), ("[[]]\n", 0), ("[[]]\n", 0)],
    ),
    (
        "[path(tostring)]",
        "abczzz",
        [("[[]]\n", 0), ("[[]]\n", 0), ("[[]]\n", 0)],
    ),
    (
        "[path(tostring)]",
        "zzz",
        [("[[]]\n", 0), ("[[]]\n", 0), ("[[]]\n", 0)],
    ),
    (
        "[path(tostring)]",
        "hé",
        [("[[]]\n", 0), ("[[]]\n", 0), ("[[]]\n", 0)],
    ),
    (
        "try path(. + \"x\") catch \"c\"",
        "",
        [("\"c\"\n", 0), ("\"c\"\n", 0), ("\"c\"\n", 0)],
    ),
    (
        "try path(. + \"x\") catch \"c\"",
        "a",
        [("\"c\"\n", 0), ("\"c\"\n", 0), ("\"c\"\n", 0)],
    ),
    (
        "try path(. + \"x\") catch \"c\"",
        "hello zzz world",
        [("\"c\"\n", 0), ("\"c\"\n", 0), ("\"c\"\n", 0)],
    ),
    (
        "try path(. + \"x\") catch \"c\"",
        "héllo",
        [("\"c\"\n", 0), ("\"c\"\n", 0), ("\"c\"\n", 0)],
    ),
    (
        "try path(. + \"x\") catch \"c\"",
        "日本語",
        [("\"c\"\n", 0), ("\"c\"\n", 0), ("\"c\"\n", 0)],
    ),
    (
        "try path(. + \"x\") catch \"c\"",
        "a\nb",
        [("\"c\"\n", 0), ("\"c\"\n\"c\"\n", 0), ("\"c\"\n", 0)],
    ),
    (
        "try path(. + \"x\") catch \"c\"",
        "q\"b\\s",
        [("\"c\"\n", 0), ("\"c\"\n", 0), ("\"c\"\n", 0)],
    ),
    (
        "try path(. + \"x\") catch \"c\"",
        "zzzabc",
        [("\"c\"\n", 0), ("\"c\"\n", 0), ("\"c\"\n", 0)],
    ),
    (
        "try path(. + \"x\") catch \"c\"",
        "abczzz",
        [("\"c\"\n", 0), ("\"c\"\n", 0), ("\"c\"\n", 0)],
    ),
    (
        "try path(. + \"x\") catch \"c\"",
        "zzz",
        [("\"c\"\n", 0), ("\"c\"\n", 0), ("\"c\"\n", 0)],
    ),
    (
        "try path(. + \"x\") catch \"c\"",
        "hé",
        [("\"c\"\n", 0), ("\"c\"\n", 0), ("\"c\"\n", 0)],
    ),
];

/// A string stage in path position answers as jq 1.7.1 does.
#[test]
fn a_string_stage_in_path_position_answers_as_jq_5816() -> Result<()> {
    check(PATH_ROWS)
}
