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

#[test]
fn a_string_builtin_over_a_string_input_answers_as_jq_5816() -> Result<()> {
    for (filter, text, expected) in ROWS {
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
