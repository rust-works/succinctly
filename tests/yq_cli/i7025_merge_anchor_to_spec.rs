//! #7025: `--yaml-fix-merge-anchor-to-spec` reads every `<<` by the YAML merge-key spec, as yq does.
//! An own key beats a merged one wherever the `<<` is written (`.t.k`, `.t[]` and the rendering all
//! agree), of the sources of `<<: [*a, *b]` the earlier-listed one wins, and a source's own `<<` is
//! expanded. Without the flag the reading is yq's default (the last write wins), which these rows pin
//! next to the flagged ones so that neither moves the other. Each row is `yq [--yaml-fix-merge-anchor-to-spec]
//! -o FORMAT -I 2 FILTER` over one of four documents, captured from yq v4.53.3 (stdout and exit status);
//! the rows that make `explode` and the JSON rendering of a malformed `<<` fail are not here (limitations.md).

use anyhow::Result;

use crate::run_yq_stdin;

const DOCS: [&str; 4] = [
    r"base: &base {p: 1, q: 2}
other: &other {q: 20, r: 30}
nested: &nested {<<: *base, n: 5}
a: {<<: *nested, p: 7}
b: {z: 0, <<: *base, q: 9, <<: *other}
c: {<<: [*nested, *other]}
d: {<<: {q: inline}, p: own}
h: {<<: [], p: 1}
i: {w: 1, <<: [*other, *base], q: 0}
",
    r"b: &b {x: 1, y: 2}
o: &o {y: 20, z: 30}
e: {y: 4, <<: *b}
f: {<<: [*b, *o]}
h: {w: 0, <<: [*o, *b], y: 9}
",
    r"s0: &s0 {a: 2}
s1: &s1 {c: 50, b: 46, a: 76, <<: *s0, d: 31}
s2: &s2 {a: 23, e: 5}
t0: {<<: [*s2, *s0]}
t1: {c: 24, c: 82, <<: *s1, b: 61}
t2: {e: 51, <<: *s1, <<: *s0, c: 93}
t3: {<<: [*s1, {a: 55, z: 1}], <<: *s2}
t4:
  x: 1
  <<: *s2
  nested:
    <<: *s1
    a: 0
seq:
  - <<: *s0
    k: 1
  - {k: 2, <<: *s2}
",
    r"foo: &foo
  a: foo_a
  thing: foo_thing
  c: foo_c
bar: &bar
  b: bar_b
  thing: bar_thing
  c: bar_c
foobarList:
  b: foobarList_b
  !!merge <<:
    - *foo
    - *bar
  c: foobarList_c
foobar:
  c: foobar_c
  !!merge <<: *foo
  thing: foobar_thing
",
];

/// (document, flag, filter, output format, yq's stdout, exit status).
#[rustfmt::skip]
const ROWS: &[(usize, bool, &str, &str, &str, i32)] = &[
    (0, true, ".a", "json", "{\n  \"q\": 2,\n  \"n\": 5,\n  \"p\": 7\n}\n", 0),
    (0, false, ".a", "json", "{\n  \"<<\": {\n    \"p\": 1,\n    \"q\": 2\n  },\n  \"n\": 5,\n  \"p\": 7\n}\n", 0),
    (0, true, ".a", "yaml", "{!!merge <<: *nested, p: 7}\n", 0),
    (0, false, ".a", "yaml", "{!!merge <<: *nested, p: 7}\n", 0),
    (0, true, ".b", "json", "{\n  \"z\": 0,\n  \"p\": 1,\n  \"q\": 9,\n  \"r\": 30\n}\n", 0),
    (0, false, ".b", "json", "{\n  \"z\": 0,\n  \"p\": 1,\n  \"q\": 20,\n  \"r\": 30\n}\n", 0),
    (0, true, ".b", "yaml", "{z: 0, !!merge <<: *base, q: 9, !!merge <<: *other}\n", 0),
    (0, false, ".b", "yaml", "{z: 0, !!merge <<: *base, q: 9, !!merge <<: *other}\n", 0),
    (0, true, ".c", "json", "{\n  \"p\": 1,\n  \"q\": 2,\n  \"n\": 5,\n  \"r\": 30\n}\n", 0),
    (0, false, ".c", "json", "{\n  \"q\": 20,\n  \"r\": 30,\n  \"<<\": {\n    \"p\": 1,\n    \"q\": 2\n  },\n  \"n\": 5\n}\n", 0),
    (0, true, ".c", "yaml", "{!!merge <<: [*nested, *other]}\n", 0),
    (0, false, ".c", "yaml", "{!!merge <<: [*nested, *other]}\n", 0),
    (0, true, ".d", "json", "{\n  \"q\": \"inline\",\n  \"p\": \"own\"\n}\n", 0),
    (0, false, ".d", "json", "{\n  \"p\": \"own\"\n}\n", 0),
    (0, true, ".d", "yaml", "{!!merge <<: {q: inline}, p: own}\n", 0),
    (0, false, ".d", "yaml", "{!!merge <<: {q: inline}, p: own}\n", 0),
    (0, true, ".h", "json", "{\n  \"p\": 1\n}\n", 0),
    (0, false, ".h", "json", "{\n  \"p\": 1\n}\n", 0),
    (0, true, ".h", "yaml", "{!!merge <<: [], p: 1}\n", 0),
    (0, false, ".h", "yaml", "{!!merge <<: [], p: 1}\n", 0),
    (0, true, ".i", "json", "{\n  \"w\": 1,\n  \"r\": 30,\n  \"p\": 1,\n  \"q\": 0\n}\n", 0),
    (0, false, ".i", "json", "{\n  \"w\": 1,\n  \"p\": 1,\n  \"q\": 0,\n  \"r\": 30\n}\n", 0),
    (0, true, ".i", "yaml", "{w: 1, !!merge <<: [*other, *base], q: 0}\n", 0),
    (0, false, ".i", "yaml", "{w: 1, !!merge <<: [*other, *base], q: 0}\n", 0),
    (0, true, ".a.p", "json", "7\n", 0),
    (0, false, ".a.p", "json", "7\n", 0),
    (0, true, ".a.p", "yaml", "7\n", 0),
    (0, false, ".a.p", "yaml", "7\n", 0),
    (0, true, ".a.q", "json", "2\n", 0),
    (0, false, ".a.q", "json", "2\n", 0),
    (0, true, ".a.q", "yaml", "2\n", 0),
    (0, false, ".a.q", "yaml", "2\n", 0),
    (0, true, ".a.n", "json", "5\n", 0),
    (0, false, ".a.n", "json", "5\n", 0),
    (0, true, ".a.n", "yaml", "5\n", 0),
    (0, false, ".a.n", "yaml", "5\n", 0),
    (0, true, ".b.q", "json", "9\n", 0),
    (0, false, ".b.q", "json", "20\n", 0),
    (0, true, ".b.q", "yaml", "9\n", 0),
    (0, false, ".b.q", "yaml", "20\n", 0),
    (0, true, ".b.p", "json", "1\n", 0),
    (0, false, ".b.p", "json", "1\n", 0),
    (0, true, ".b.p", "yaml", "1\n", 0),
    (0, false, ".b.p", "yaml", "1\n", 0),
    (0, true, ".c.q", "json", "2\n", 0),
    (0, false, ".c.q", "json", "20\n", 0),
    (0, true, ".c.q", "yaml", "2\n", 0),
    (0, false, ".c.q", "yaml", "20\n", 0),
    (0, true, ".c.p", "json", "1\n", 0),
    (0, false, ".c.p", "json", "1\n", 0),
    (0, true, ".c.p", "yaml", "1\n", 0),
    (0, false, ".c.p", "yaml", "1\n", 0),
    (0, true, ".d.q", "json", "\"inline\"\n", 0),
    (0, false, ".d.q", "json", "\"inline\"\n", 0),
    (0, true, ".d.q", "yaml", "inline\n", 0),
    (0, false, ".d.q", "yaml", "inline\n", 0),
    (0, true, ".i.q", "json", "0\n", 0),
    (0, false, ".i.q", "json", "0\n", 0),
    (0, true, ".i.q", "yaml", "0\n", 0),
    (0, false, ".i.q", "yaml", "0\n", 0),
    (0, true, ".i.p", "json", "1\n", 0),
    (0, false, ".i.p", "json", "1\n", 0),
    (0, true, ".i.p", "yaml", "1\n", 0),
    (0, false, ".i.p", "yaml", "1\n", 0),
    (0, true, ".i.r", "json", "30\n", 0),
    (0, false, ".i.r", "json", "30\n", 0),
    (0, true, ".i.r", "yaml", "30\n", 0),
    (0, false, ".i.r", "yaml", "30\n", 0),
    (0, true, ".a[]", "json", "7\n2\n5\n", 0),
    (0, true, ".a[]", "yaml", "7\n2\n5\n", 0),
    (0, true, ".b[]", "json", "9\n30\n1\n0\n", 0),
    (0, false, ".b[]", "json", "0\n1\n20\n30\n", 0),
    (0, true, ".b[]", "yaml", "9\n30\n1\n0\n", 0),
    (0, false, ".b[]", "yaml", "0\n1\n20\n30\n", 0),
    (0, true, ".c[]", "json", "2\n30\n1\n5\n", 0),
    (0, true, ".c[]", "yaml", "2\n30\n1\n5\n", 0),
    (0, true, ".d[]", "json", "\"inline\"\n\"own\"\n", 0),
    (0, false, ".d[]", "json", "\"inline\"\n\"own\"\n", 0),
    (0, true, ".d[]", "yaml", "inline\nown\n", 0),
    (0, false, ".d[]", "yaml", "inline\nown\n", 0),
    (0, true, ".i[]", "json", "1\n0\n30\n1\n", 0),
    (0, false, ".i[]", "json", "1\n0\n30\n1\n", 0),
    (0, true, ".i[]", "yaml", "1\n0\n30\n1\n", 0),
    (0, false, ".i[]", "yaml", "1\n0\n30\n1\n", 0),
    (0, true, "[.c[] | key]", "json", "[\n  \"q\",\n  \"r\",\n  \"p\",\n  \"n\"\n]\n", 0),
    (0, true, "[.c[] | key]", "yaml", "- q\n- r\n- p\n- n\n", 0),
    (0, true, "[.i[] | key]", "json", "[\n  \"p\",\n  \"q\",\n  \"r\",\n  \"w\"\n]\n", 0),
    (0, false, "[.i[] | key]", "json", "[\n  \"w\",\n  \"q\",\n  \"r\",\n  \"p\"\n]\n", 0),
    (0, true, "[.i[] | key]", "yaml", "- p\n- q\n- r\n- w\n", 0),
    (0, false, "[.i[] | key]", "yaml", "- w\n- q\n- r\n- p\n", 0),
    (0, true, ".a | tojson", "json", "\"{\\n  \\\"q\\\": 2,\\n  \\\"n\\\": 5,\\n  \\\"p\\\": 7\\n}\\n\"\n", 0),
    (0, false, ".a | tojson", "json", "\"{\\n  \\\"<<\\\": {\\n    \\\"p\\\": 1,\\n    \\\"q\\\": 2\\n  },\\n  \\\"n\\\": 5,\\n  \\\"p\\\": 7\\n}\\n\"\n", 0),
    (0, true, ".a | tojson", "yaml", "{\n  \"q\": 2,\n  \"n\": 5,\n  \"p\": 7\n}\n\n", 0),
    (0, false, ".a | tojson", "yaml", "{\n  \"<<\": {\n    \"p\": 1,\n    \"q\": 2\n  },\n  \"n\": 5,\n  \"p\": 7\n}\n\n", 0),
    (0, true, ".c | tojson", "json", "\"{\\n  \\\"p\\\": 1,\\n  \\\"q\\\": 2,\\n  \\\"n\\\": 5,\\n  \\\"r\\\": 30\\n}\\n\"\n", 0),
    (0, false, ".c | tojson", "json", "\"{\\n  \\\"q\\\": 20,\\n  \\\"r\\\": 30,\\n  \\\"<<\\\": {\\n    \\\"p\\\": 1,\\n    \\\"q\\\": 2\\n  },\\n  \\\"n\\\": 5\\n}\\n\"\n", 0),
    (0, true, ".c | tojson", "yaml", "{\n  \"p\": 1,\n  \"q\": 2,\n  \"n\": 5,\n  \"r\": 30\n}\n\n", 0),
    (0, false, ".c | tojson", "yaml", "{\n  \"q\": 20,\n  \"r\": 30,\n  \"<<\": {\n    \"p\": 1,\n    \"q\": 2\n  },\n  \"n\": 5\n}\n\n", 0),
    (0, true, "explode(.)", "json", "{\n  \"base\": {\n    \"p\": 1,\n    \"q\": 2\n  },\n  \"other\": {\n    \"q\": 20,\n    \"r\": 30\n  },\n  \"nested\": {\n    \"p\": 1,\n    \"q\": 2,\n    \"n\": 5\n  },\n  \"a\": {\n    \"q\": 2,\n    \"n\": 5,\n    \"p\": 7\n  },\n  \"b\": {\n    \"z\": 0,\n    \"p\": 1,\n    \"q\": 9,\n    \"r\": 30\n  },\n  \"c\": {\n    \"p\": 1,\n    \"q\": 2,\n    \"n\": 5,\n    \"r\": 30\n  },\n  \"d\": {\n    \"q\": \"inline\",\n    \"p\": \"own\"\n  },\n  \"h\": {\n    \"p\": 1\n  },\n  \"i\": {\n    \"w\": 1,\n    \"r\": 30,\n    \"p\": 1,\n    \"q\": 0\n  }\n}\n", 0),
    (0, true, "explode(.)", "yaml", "base: {p: 1, q: 2}\nother: {q: 20, r: 30}\nnested: {p: 1, q: 2, n: 5}\na: {q: 2, n: 5, p: 7}\nb: {z: 0, p: 1, q: 9, r: 30}\nc: {p: 1, q: 2, n: 5, r: 30}\nd: {q: inline, p: own}\nh: {p: 1}\ni: {w: 1, r: 30, p: 1, q: 0}\n", 0),
    (0, true, ".", "json", "{\n  \"base\": {\n    \"p\": 1,\n    \"q\": 2\n  },\n  \"other\": {\n    \"q\": 20,\n    \"r\": 30\n  },\n  \"nested\": {\n    \"p\": 1,\n    \"q\": 2,\n    \"n\": 5\n  },\n  \"a\": {\n    \"q\": 2,\n    \"n\": 5,\n    \"p\": 7\n  },\n  \"b\": {\n    \"z\": 0,\n    \"p\": 1,\n    \"q\": 9,\n    \"r\": 30\n  },\n  \"c\": {\n    \"p\": 1,\n    \"q\": 2,\n    \"n\": 5,\n    \"r\": 30\n  },\n  \"d\": {\n    \"q\": \"inline\",\n    \"p\": \"own\"\n  },\n  \"h\": {\n    \"p\": 1\n  },\n  \"i\": {\n    \"w\": 1,\n    \"r\": 30,\n    \"p\": 1,\n    \"q\": 0\n  }\n}\n", 0),
    (0, true, ".", "yaml", "base: &base {p: 1, q: 2}\nother: &other {q: 20, r: 30}\nnested: &nested {!!merge <<: *base, n: 5}\na: {!!merge <<: *nested, p: 7}\nb: {z: 0, !!merge <<: *base, q: 9, !!merge <<: *other}\nc: {!!merge <<: [*nested, *other]}\nd: {!!merge <<: {q: inline}, p: own}\nh: {!!merge <<: [], p: 1}\ni: {w: 1, !!merge <<: [*other, *base], q: 0}\n", 0),
    (0, false, ".", "yaml", "base: &base {p: 1, q: 2}\nother: &other {q: 20, r: 30}\nnested: &nested {!!merge <<: *base, n: 5}\na: {!!merge <<: *nested, p: 7}\nb: {z: 0, !!merge <<: *base, q: 9, !!merge <<: *other}\nc: {!!merge <<: [*nested, *other]}\nd: {!!merge <<: {q: inline}, p: own}\nh: {!!merge <<: [], p: 1}\ni: {w: 1, !!merge <<: [*other, *base], q: 0}\n", 0),
    (0, true, ".. | select(tag == \"!!int\")", "json", "1\n2\n20\n30\n5\n7\n0\n9\n1\n1\n0\n", 0),
    (0, false, ".. | select(tag == \"!!int\")", "json", "1\n2\n20\n30\n5\n7\n0\n9\n1\n1\n0\n", 0),
    (0, true, ".. | select(tag == \"!!int\")", "yaml", "1\n2\n20\n30\n5\n7\n0\n9\n1\n1\n0\n", 0),
    (0, false, ".. | select(tag == \"!!int\")", "yaml", "1\n2\n20\n30\n5\n7\n0\n9\n1\n1\n0\n", 0),
    (1, true, ".e", "json", "{\n  \"y\": 4,\n  \"x\": 1\n}\n", 0),
    (1, false, ".e", "json", "{\n  \"y\": 2,\n  \"x\": 1\n}\n", 0),
    (1, true, ".e", "yaml", "{y: 4, !!merge <<: *b}\n", 0),
    (1, false, ".e", "yaml", "{y: 4, !!merge <<: *b}\n", 0),
    (1, true, ".f", "json", "{\n  \"x\": 1,\n  \"y\": 2,\n  \"z\": 30\n}\n", 0),
    (1, false, ".f", "json", "{\n  \"y\": 2,\n  \"z\": 30,\n  \"x\": 1\n}\n", 0),
    (1, true, ".f", "yaml", "{!!merge <<: [*b, *o]}\n", 0),
    (1, false, ".f", "yaml", "{!!merge <<: [*b, *o]}\n", 0),
    (1, true, ".h", "json", "{\n  \"w\": 0,\n  \"z\": 30,\n  \"x\": 1,\n  \"y\": 9\n}\n", 0),
    (1, false, ".h", "json", "{\n  \"w\": 0,\n  \"x\": 1,\n  \"y\": 9,\n  \"z\": 30\n}\n", 0),
    (1, true, ".h", "yaml", "{w: 0, !!merge <<: [*o, *b], y: 9}\n", 0),
    (1, false, ".h", "yaml", "{w: 0, !!merge <<: [*o, *b], y: 9}\n", 0),
    (1, true, ".e.y", "json", "4\n", 0),
    (1, false, ".e.y", "json", "2\n", 0),
    (1, true, ".e.y", "yaml", "4\n", 0),
    (1, false, ".e.y", "yaml", "2\n", 0),
    (1, true, ".f.y", "json", "2\n", 0),
    (1, false, ".f.y", "json", "20\n", 0),
    (1, true, ".f.y", "yaml", "2\n", 0),
    (1, false, ".f.y", "yaml", "20\n", 0),
    (1, true, ".h.y", "json", "9\n", 0),
    (1, false, ".h.y", "json", "9\n", 0),
    (1, true, ".h.y", "yaml", "9\n", 0),
    (1, false, ".h.y", "yaml", "9\n", 0),
    (1, true, ".e.x", "json", "1\n", 0),
    (1, false, ".e.x", "json", "1\n", 0),
    (1, true, ".e.x", "yaml", "1\n", 0),
    (1, false, ".e.x", "yaml", "1\n", 0),
    (1, true, ".f.x", "json", "1\n", 0),
    (1, false, ".f.x", "json", "1\n", 0),
    (1, true, ".f.x", "yaml", "1\n", 0),
    (1, false, ".f.x", "yaml", "1\n", 0),
    (1, true, ".e[]", "json", "1\n4\n", 0),
    (1, false, ".e[]", "json", "2\n1\n", 0),
    (1, true, ".e[]", "yaml", "1\n4\n", 0),
    (1, false, ".e[]", "yaml", "2\n1\n", 0),
    (1, true, ".f[]", "json", "2\n30\n1\n", 0),
    (1, false, ".f[]", "json", "1\n20\n30\n", 0),
    (1, true, ".f[]", "yaml", "2\n30\n1\n", 0),
    (1, false, ".f[]", "yaml", "1\n20\n30\n", 0),
    (1, true, ".h[]", "json", "1\n9\n30\n0\n", 0),
    (1, false, ".h[]", "json", "0\n9\n30\n1\n", 0),
    (1, true, ".h[]", "yaml", "1\n9\n30\n0\n", 0),
    (1, false, ".h[]", "yaml", "0\n9\n30\n1\n", 0),
    (1, true, "explode(.)", "json", "{\n  \"b\": {\n    \"x\": 1,\n    \"y\": 2\n  },\n  \"o\": {\n    \"y\": 20,\n    \"z\": 30\n  },\n  \"e\": {\n    \"y\": 4,\n    \"x\": 1\n  },\n  \"f\": {\n    \"x\": 1,\n    \"y\": 2,\n    \"z\": 30\n  },\n  \"h\": {\n    \"w\": 0,\n    \"z\": 30,\n    \"x\": 1,\n    \"y\": 9\n  }\n}\n", 0),
    (1, false, "explode(.)", "json", "{\n  \"b\": {\n    \"x\": 1,\n    \"y\": 2\n  },\n  \"o\": {\n    \"y\": 20,\n    \"z\": 30\n  },\n  \"e\": {\n    \"y\": 2,\n    \"x\": 1\n  },\n  \"f\": {\n    \"y\": 2,\n    \"z\": 30,\n    \"x\": 1\n  },\n  \"h\": {\n    \"w\": 0,\n    \"x\": 1,\n    \"y\": 9,\n    \"z\": 30\n  }\n}\n", 0),
    (1, true, "explode(.)", "yaml", "b: {x: 1, y: 2}\no: {y: 20, z: 30}\ne: {y: 4, x: 1}\nf: {x: 1, y: 2, z: 30}\nh: {w: 0, z: 30, x: 1, y: 9}\n", 0),
    (1, false, "explode(.)", "yaml", "b: {x: 1, y: 2}\no: {y: 20, z: 30}\ne: {y: 2, x: 1}\nf: {y: 2, z: 30, x: 1}\nh: {w: 0, x: 1, y: 9, z: 30}\n", 0),
    (2, true, ".t0", "json", "{\n  \"a\": 23,\n  \"e\": 5\n}\n", 0),
    (2, false, ".t0", "json", "{\n  \"a\": 23,\n  \"e\": 5\n}\n", 0),
    (2, true, ".t0", "yaml", "{!!merge <<: [*s2, *s0]}\n", 0),
    (2, false, ".t0", "yaml", "{!!merge <<: [*s2, *s0]}\n", 0),
    (2, true, ".t1", "json", "{\n  \"c\": 24,\n  \"c\": 82,\n  \"a\": 76,\n  \"d\": 31,\n  \"b\": 61\n}\n", 0),
    (2, false, ".t1", "json", "{\n  \"c\": 50,\n  \"a\": 76,\n  \"<<\": {\n    \"a\": 2\n  },\n  \"d\": 31,\n  \"b\": 61\n}\n", 0),
    (2, true, ".t1", "yaml", "{c: 24, c: 82, !!merge <<: *s1, b: 61}\n", 0),
    (2, false, ".t1", "yaml", "{c: 24, c: 82, !!merge <<: *s1, b: 61}\n", 0),
    (2, true, ".t2", "json", "{\n  \"e\": 51,\n  \"b\": 46,\n  \"a\": 76,\n  \"d\": 31,\n  \"c\": 93\n}\n", 0),
    (2, true, ".t2", "yaml", "{e: 51, !!merge <<: *s1, !!merge <<: *s0, c: 93}\n", 0),
    (2, false, ".t2", "yaml", "{e: 51, !!merge <<: *s1, !!merge <<: *s0, c: 93}\n", 0),
    (2, true, ".t3", "json", "{\n  \"c\": 50,\n  \"b\": 46,\n  \"a\": 76,\n  \"d\": 31,\n  \"z\": 1,\n  \"e\": 5\n}\n", 0),
    (2, true, ".t3", "yaml", "{!!merge <<: [*s1, {a: 55, z: 1}], !!merge <<: *s2}\n", 0),
    (2, false, ".t3", "yaml", "{!!merge <<: [*s1, {a: 55, z: 1}], !!merge <<: *s2}\n", 0),
    (2, true, ".t4", "json", "{\n  \"x\": 1,\n  \"a\": 23,\n  \"e\": 5,\n  \"nested\": {\n    \"c\": 50,\n    \"b\": 46,\n    \"d\": 31,\n    \"a\": 0\n  }\n}\n", 0),
    (2, false, ".t4", "json", "{\n  \"x\": 1,\n  \"a\": 23,\n  \"e\": 5,\n  \"nested\": {\n    \"c\": 50,\n    \"b\": 46,\n    \"<<\": {\n      \"a\": 2\n    },\n    \"d\": 31,\n    \"a\": 0\n  }\n}\n", 0),
    (2, true, ".t4", "yaml", "x: 1\n!!merge <<: *s2\nnested:\n  !!merge <<: *s1\n  a: 0\n", 0),
    (2, false, ".t4", "yaml", "x: 1\n!!merge <<: *s2\nnested:\n  !!merge <<: *s1\n  a: 0\n", 0),
    (2, true, ".t0.a", "json", "23\n", 0),
    (2, false, ".t0.a", "json", "2\n", 0),
    (2, true, ".t0.a", "yaml", "23\n", 0),
    (2, false, ".t0.a", "yaml", "2\n", 0),
    (2, true, ".t1.c", "json", "82\n", 0),
    (2, false, ".t1.c", "json", "50\n", 0),
    (2, true, ".t1.c", "yaml", "82\n", 0),
    (2, false, ".t1.c", "yaml", "50\n", 0),
    (2, true, ".t1.b", "json", "61\n", 0),
    (2, false, ".t1.b", "json", "61\n", 0),
    (2, true, ".t1.b", "yaml", "61\n", 0),
    (2, false, ".t1.b", "yaml", "61\n", 0),
    (2, true, ".t2.c", "json", "93\n", 0),
    (2, false, ".t2.c", "json", "93\n", 0),
    (2, true, ".t2.c", "yaml", "93\n", 0),
    (2, false, ".t2.c", "yaml", "93\n", 0),
    (2, true, ".t2.e", "json", "51\n", 0),
    (2, false, ".t2.e", "json", "51\n", 0),
    (2, true, ".t2.e", "yaml", "51\n", 0),
    (2, false, ".t2.e", "yaml", "51\n", 0),
    (2, true, ".t2.a", "json", "76\n", 0),
    (2, false, ".t2.a", "json", "2\n", 0),
    (2, true, ".t2.a", "yaml", "76\n", 0),
    (2, false, ".t2.a", "yaml", "2\n", 0),
    (2, true, ".t3.a", "json", "76\n", 0),
    (2, false, ".t3.a", "json", "23\n", 0),
    (2, true, ".t3.a", "yaml", "76\n", 0),
    (2, false, ".t3.a", "yaml", "23\n", 0),
    (2, true, ".t3.z", "json", "1\n", 0),
    (2, false, ".t3.z", "json", "1\n", 0),
    (2, true, ".t3.z", "yaml", "1\n", 0),
    (2, false, ".t3.z", "yaml", "1\n", 0),
    (2, true, ".t3.c", "json", "50\n", 0),
    (2, false, ".t3.c", "json", "50\n", 0),
    (2, true, ".t3.c", "yaml", "50\n", 0),
    (2, false, ".t3.c", "yaml", "50\n", 0),
    (2, true, ".t4.nested.a", "json", "0\n", 0),
    (2, false, ".t4.nested.a", "json", "0\n", 0),
    (2, true, ".t4.nested.a", "yaml", "0\n", 0),
    (2, false, ".t4.nested.a", "yaml", "0\n", 0),
    (2, true, ".t4.nested.c", "json", "50\n", 0),
    (2, false, ".t4.nested.c", "json", "50\n", 0),
    (2, true, ".t4.nested.c", "yaml", "50\n", 0),
    (2, false, ".t4.nested.c", "yaml", "50\n", 0),
    (2, true, ".t4.x", "json", "1\n", 0),
    (2, false, ".t4.x", "json", "1\n", 0),
    (2, true, ".t4.x", "yaml", "1\n", 0),
    (2, false, ".t4.x", "yaml", "1\n", 0),
    (2, true, ".t4.e", "json", "5\n", 0),
    (2, false, ".t4.e", "json", "5\n", 0),
    (2, true, ".t4.e", "yaml", "5\n", 0),
    (2, false, ".t4.e", "yaml", "5\n", 0),
    (2, true, ".t0[]", "json", "23\n5\n", 0),
    (2, false, ".t0[]", "json", "2\n5\n", 0),
    (2, true, ".t0[]", "yaml", "23\n5\n", 0),
    (2, false, ".t0[]", "yaml", "2\n5\n", 0),
    (2, true, ".t1[]", "json", "76\n82\n61\n31\n", 0),
    (2, true, ".t1[]", "yaml", "76\n82\n61\n31\n", 0),
    (2, true, ".t2[]", "json", "76\n93\n46\n31\n51\n", 0),
    (2, true, ".t2[]", "yaml", "76\n93\n46\n31\n51\n", 0),
    (2, true, ".t3[]", "json", "76\n5\n1\n50\n46\n31\n", 0),
    (2, true, ".t3[]", "yaml", "76\n5\n1\n50\n46\n31\n", 0),
    (2, true, ".t4[]", "json", "23\n5\n1\n{\n  \"c\": 50,\n  \"b\": 46,\n  \"d\": 31,\n  \"a\": 0\n}\n", 0),
    (2, false, ".t4[]", "json", "1\n23\n5\n{\n  \"c\": 50,\n  \"b\": 46,\n  \"<<\": {\n    \"a\": 2\n  },\n  \"d\": 31,\n  \"a\": 0\n}\n", 0),
    (2, true, ".t4[]", "yaml", "23\n5\n1\n!!merge <<: *s1\na: 0\n", 0),
    (2, false, ".t4[]", "yaml", "1\n23\n5\n!!merge <<: *s1\na: 0\n", 0),
    (2, true, ".t4.nested[]", "json", "0\n50\n46\n31\n", 0),
    (2, true, ".t4.nested[]", "yaml", "0\n50\n46\n31\n", 0),
    (2, true, ".seq", "json", "[\n  {\n    \"a\": 2,\n    \"k\": 1\n  },\n  {\n    \"k\": 2,\n    \"a\": 23,\n    \"e\": 5\n  }\n]\n", 0),
    (2, false, ".seq", "json", "[\n  {\n    \"a\": 2,\n    \"k\": 1\n  },\n  {\n    \"k\": 2,\n    \"a\": 23,\n    \"e\": 5\n  }\n]\n", 0),
    (2, true, ".seq", "yaml", "- !!merge <<: *s0\n  k: 1\n- {k: 2, !!merge <<: *s2}\n", 0),
    (2, false, ".seq", "yaml", "- !!merge <<: *s0\n  k: 1\n- {k: 2, !!merge <<: *s2}\n", 0),
    (2, true, ".seq[0]", "json", "{\n  \"a\": 2,\n  \"k\": 1\n}\n", 0),
    (2, false, ".seq[0]", "json", "{\n  \"a\": 2,\n  \"k\": 1\n}\n", 0),
    (2, true, ".seq[0]", "yaml", "!!merge <<: *s0\nk: 1\n", 0),
    (2, false, ".seq[0]", "yaml", "!!merge <<: *s0\nk: 1\n", 0),
    (2, true, ".seq[1]", "json", "{\n  \"k\": 2,\n  \"a\": 23,\n  \"e\": 5\n}\n", 0),
    (2, false, ".seq[1]", "json", "{\n  \"k\": 2,\n  \"a\": 23,\n  \"e\": 5\n}\n", 0),
    (2, true, ".seq[1]", "yaml", "{k: 2, !!merge <<: *s2}\n", 0),
    (2, false, ".seq[1]", "yaml", "{k: 2, !!merge <<: *s2}\n", 0),
    (2, true, ".seq[].k", "json", "1\n2\n", 0),
    (2, false, ".seq[].k", "json", "1\n2\n", 0),
    (2, true, ".seq[].k", "yaml", "1\n2\n", 0),
    (2, false, ".seq[].k", "yaml", "1\n2\n", 0),
    (2, true, ".seq[] | keys", "json", "[\n  \"<<\",\n  \"k\"\n]\n[\n  \"k\",\n  \"<<\"\n]\n", 0),
    (2, false, ".seq[] | keys", "json", "[\n  \"<<\",\n  \"k\"\n]\n[\n  \"k\",\n  \"<<\"\n]\n", 0),
    (2, true, "[.t3[] | key]", "json", "[\n  \"a\",\n  \"e\",\n  \"z\",\n  \"c\",\n  \"b\",\n  \"d\"\n]\n", 0),
    (2, true, "[.t3[] | key]", "yaml", "- a\n- e\n- z\n- c\n- b\n- d\n", 0),
    (2, true, ".t3 | tojson", "json", "\"{\\n  \\\"c\\\": 50,\\n  \\\"b\\\": 46,\\n  \\\"a\\\": 76,\\n  \\\"d\\\": 31,\\n  \\\"z\\\": 1,\\n  \\\"e\\\": 5\\n}\\n\"\n", 0),
    (2, true, ".t3 | tojson", "yaml", "{\n  \"c\": 50,\n  \"b\": 46,\n  \"a\": 76,\n  \"d\": 31,\n  \"z\": 1,\n  \"e\": 5\n}\n\n", 0),
    (2, true, "explode(.t3)", "json", "{\n  \"s0\": {\n    \"a\": 2\n  },\n  \"s1\": {\n    \"c\": 50,\n    \"b\": 46,\n    \"a\": 76,\n    \"d\": 31\n  },\n  \"s2\": {\n    \"a\": 23,\n    \"e\": 5\n  },\n  \"t0\": {\n    \"a\": 23,\n    \"e\": 5\n  },\n  \"t1\": {\n    \"c\": 24,\n    \"c\": 82,\n    \"a\": 76,\n    \"d\": 31,\n    \"b\": 61\n  },\n  \"t2\": {\n    \"e\": 51,\n    \"b\": 46,\n    \"a\": 76,\n    \"d\": 31,\n    \"c\": 93\n  },\n  \"t3\": {\n    \"c\": 50,\n    \"b\": 46,\n    \"a\": 76,\n    \"d\": 31,\n    \"z\": 1,\n    \"e\": 5\n  },\n  \"t4\": {\n    \"x\": 1,\n    \"a\": 23,\n    \"e\": 5,\n    \"nested\": {\n      \"c\": 50,\n      \"b\": 46,\n      \"d\": 31,\n      \"a\": 0\n    }\n  },\n  \"seq\": [\n    {\n      \"a\": 2,\n      \"k\": 1\n    },\n    {\n      \"k\": 2,\n      \"a\": 23,\n      \"e\": 5\n    }\n  ]\n}\n", 0),
    (2, true, "explode(.)", "json", "{\n  \"s0\": {\n    \"a\": 2\n  },\n  \"s1\": {\n    \"c\": 50,\n    \"b\": 46,\n    \"a\": 76,\n    \"d\": 31\n  },\n  \"s2\": {\n    \"a\": 23,\n    \"e\": 5\n  },\n  \"t0\": {\n    \"a\": 23,\n    \"e\": 5\n  },\n  \"t1\": {\n    \"c\": 24,\n    \"c\": 82,\n    \"a\": 76,\n    \"d\": 31,\n    \"b\": 61\n  },\n  \"t2\": {\n    \"e\": 51,\n    \"b\": 46,\n    \"a\": 76,\n    \"d\": 31,\n    \"c\": 93\n  },\n  \"t3\": {\n    \"c\": 50,\n    \"b\": 46,\n    \"a\": 76,\n    \"d\": 31,\n    \"z\": 1,\n    \"e\": 5\n  },\n  \"t4\": {\n    \"x\": 1,\n    \"a\": 23,\n    \"e\": 5,\n    \"nested\": {\n      \"c\": 50,\n      \"b\": 46,\n      \"d\": 31,\n      \"a\": 0\n    }\n  },\n  \"seq\": [\n    {\n      \"a\": 2,\n      \"k\": 1\n    },\n    {\n      \"k\": 2,\n      \"a\": 23,\n      \"e\": 5\n    }\n  ]\n}\n", 0),
    (2, true, "explode(.)", "yaml", "s0: {a: 2}\ns1: {c: 50, b: 46, a: 76, d: 31}\ns2: {a: 23, e: 5}\nt0: {a: 23, e: 5}\nt1: {c: 24, c: 82, a: 76, d: 31, b: 61}\nt2: {e: 51, b: 46, a: 76, d: 31, c: 93}\nt3: {c: 50, b: 46, a: 76, d: 31, z: 1, e: 5}\nt4:\n  x: 1\n  a: 23\n  e: 5\n  nested:\n    c: 50\n    b: 46\n    d: 31\n    a: 0\nseq:\n  - a: 2\n    k: 1\n  - {k: 2, a: 23, e: 5}\n", 0),
    (3, true, ".foobar.c", "json", "\"foobar_c\"\n", 0),
    (3, false, ".foobar.c", "json", "\"foo_c\"\n", 0),
    (3, true, ".foobar.c", "yaml", "foobar_c\n", 0),
    (3, false, ".foobar.c", "yaml", "foo_c\n", 0),
    (3, true, ".foobarList.thing", "json", "\"foo_thing\"\n", 0),
    (3, false, ".foobarList.thing", "json", "\"bar_thing\"\n", 0),
    (3, true, ".foobarList.thing", "yaml", "foo_thing\n", 0),
    (3, false, ".foobarList.thing", "yaml", "bar_thing\n", 0),
    (3, true, ".foobar[]", "json", "\"foo_a\"\n\"foobar_thing\"\n\"foobar_c\"\n", 0),
    (3, false, ".foobar[]", "json", "\"foo_c\"\n\"foo_a\"\n\"foobar_thing\"\n", 0),
    (3, true, ".foobar[]", "yaml", "foo_a\nfoobar_thing\nfoobar_c\n", 0),
    (3, false, ".foobar[]", "yaml", "foo_c\nfoo_a\nfoobar_thing\n", 0),
    (3, true, ".foobarList[]", "json", "\"foobarList_b\"\n\"foo_thing\"\n\"foobarList_c\"\n\"foo_a\"\n", 0),
    (3, false, ".foobarList[]", "json", "\"bar_b\"\n\"foo_a\"\n\"bar_thing\"\n\"foobarList_c\"\n", 0),
    (3, true, ".foobarList[]", "yaml", "foobarList_b\nfoo_thing\nfoobarList_c\nfoo_a\n", 0),
    (3, false, ".foobarList[]", "yaml", "bar_b\nfoo_a\nbar_thing\nfoobarList_c\n", 0),
    (3, true, ".foobar", "json", "{\n  \"c\": \"foobar_c\",\n  \"a\": \"foo_a\",\n  \"thing\": \"foobar_thing\"\n}\n", 0),
    (3, false, ".foobar", "json", "{\n  \"c\": \"foo_c\",\n  \"a\": \"foo_a\",\n  \"thing\": \"foobar_thing\"\n}\n", 0),
    (3, true, ".foobar", "yaml", "c: foobar_c\n!!merge <<: *foo\nthing: foobar_thing\n", 0),
    (3, false, ".foobar", "yaml", "c: foobar_c\n!!merge <<: *foo\nthing: foobar_thing\n", 0),
    (3, true, ".foobarList", "json", "{\n  \"b\": \"foobarList_b\",\n  \"a\": \"foo_a\",\n  \"thing\": \"foo_thing\",\n  \"c\": \"foobarList_c\"\n}\n", 0),
    (3, false, ".foobarList", "json", "{\n  \"b\": \"bar_b\",\n  \"thing\": \"foo_thing\",\n  \"c\": \"foobarList_c\",\n  \"a\": \"foo_a\"\n}\n", 0),
    (3, true, ".foobarList", "yaml", "b: foobarList_b\n!!merge <<:\n  - *foo\n  - *bar\nc: foobarList_c\n", 0),
    (3, false, ".foobarList", "yaml", "b: foobarList_b\n!!merge <<:\n  - *foo\n  - *bar\nc: foobarList_c\n", 0),
    (3, true, "explode(.)", "json", "{\n  \"foo\": {\n    \"a\": \"foo_a\",\n    \"thing\": \"foo_thing\",\n    \"c\": \"foo_c\"\n  },\n  \"bar\": {\n    \"b\": \"bar_b\",\n    \"thing\": \"bar_thing\",\n    \"c\": \"bar_c\"\n  },\n  \"foobarList\": {\n    \"b\": \"foobarList_b\",\n    \"a\": \"foo_a\",\n    \"thing\": \"foo_thing\",\n    \"c\": \"foobarList_c\"\n  },\n  \"foobar\": {\n    \"c\": \"foobar_c\",\n    \"a\": \"foo_a\",\n    \"thing\": \"foobar_thing\"\n  }\n}\n", 0),
    (3, false, "explode(.)", "json", "{\n  \"foo\": {\n    \"a\": \"foo_a\",\n    \"thing\": \"foo_thing\",\n    \"c\": \"foo_c\"\n  },\n  \"bar\": {\n    \"b\": \"bar_b\",\n    \"thing\": \"bar_thing\",\n    \"c\": \"bar_c\"\n  },\n  \"foobarList\": {\n    \"b\": \"bar_b\",\n    \"thing\": \"foo_thing\",\n    \"c\": \"foobarList_c\",\n    \"a\": \"foo_a\"\n  },\n  \"foobar\": {\n    \"c\": \"foo_c\",\n    \"a\": \"foo_a\",\n    \"thing\": \"foobar_thing\"\n  }\n}\n", 0),
    (3, true, "explode(.)", "yaml", "foo:\n  a: foo_a\n  thing: foo_thing\n  c: foo_c\nbar:\n  b: bar_b\n  thing: bar_thing\n  c: bar_c\nfoobarList:\n  b: foobarList_b\n  a: foo_a\n  thing: foo_thing\n  c: foobarList_c\nfoobar:\n  c: foobar_c\n  a: foo_a\n  thing: foobar_thing\n", 0),
    (3, false, "explode(.)", "yaml", "foo:\n  a: foo_a\n  thing: foo_thing\n  c: foo_c\nbar:\n  b: bar_b\n  thing: bar_thing\n  c: bar_c\nfoobarList:\n  b: bar_b\n  thing: foo_thing\n  c: foobarList_c\n  a: foo_a\nfoobar:\n  c: foo_c\n  a: foo_a\n  thing: foobar_thing\n", 0),
    (3, true, ".foobarList.c", "json", "\"foobarList_c\"\n", 0),
    (3, false, ".foobarList.c", "json", "\"foobarList_c\"\n", 0),
    (3, true, ".foobarList.c", "yaml", "foobarList_c\n", 0),
    (3, false, ".foobarList.c", "yaml", "foobarList_c\n", 0),
    (3, true, ".foobarList.b", "json", "\"foobarList_b\"\n", 0),
    (3, false, ".foobarList.b", "json", "\"bar_b\"\n", 0),
    (3, true, ".foobarList.b", "yaml", "foobarList_b\n", 0),
    (3, false, ".foobarList.b", "yaml", "bar_b\n", 0),
    (3, true, ".foobarList.a", "json", "\"foo_a\"\n", 0),
    (3, false, ".foobarList.a", "json", "\"foo_a\"\n", 0),
    (3, true, ".foobarList.a", "yaml", "foo_a\n", 0),
    (3, false, ".foobarList.a", "yaml", "foo_a\n", 0),
];

#[test]
fn merge_keys_read_as_yq_does_under_the_spec_flag_7025() -> Result<()> {
    for (doc, flag, filter, format, stdout, status) in ROWS {
        let mut args = vec!["-o", format, "-I", "2"];
        if *flag {
            args.push("--yaml-fix-merge-anchor-to-spec");
        }
        let (out, code) = run_yq_stdin(filter, DOCS[*doc], &args)?;
        assert_eq!(
            (out.as_str(), code),
            (*stdout, *status),
            "{filter} (document {doc}, flag {flag}, -o {format})"
        );
    }
    Ok(())
}

/// A boolean flag of yq's takes its value after `=` only, and the last one given wins, so
/// `=false` is the default reading and `--flag=false --flag` is the spec one.
#[test]
fn spec_flag_takes_a_go_bool_after_equals_7025() -> Result<()> {
    let doc = DOCS[0];
    let on = run_yq_stdin(".b.q", doc, &["--yaml-fix-merge-anchor-to-spec"])?;
    let off = run_yq_stdin(".b.q", doc, &["--yaml-fix-merge-anchor-to-spec=false"])?;
    assert_eq!((on.0.as_str(), on.1), ("9\n", 0));
    assert_eq!((off.0.as_str(), off.1), ("20\n", 0));
    for flags in [
        ["--yaml-fix-merge-anchor-to-spec=true"],
        ["--yaml-fix-merge-anchor-to-spec=T"],
        ["--yaml-fix-merge-anchor-to-spec=1"],
    ] {
        assert_eq!(run_yq_stdin(".b.q", doc, &flags)?, on, "{flags:?}");
    }
    let last_wins = run_yq_stdin(
        ".b.q",
        doc,
        &[
            "--yaml-fix-merge-anchor-to-spec=false",
            "--yaml-fix-merge-anchor-to-spec",
        ],
    )?;
    assert_eq!(last_wins, on);
    // A value that is no Go bool is refused where yq refuses it, at the flag.
    let (out, err, code) =
        crate::run_yq_stdin_with_stderr(".b.q", doc, &["--yaml-fix-merge-anchor-to-spec=maybe"])?;
    assert_eq!((out.as_str(), code), ("", 2), "{err}");
    Ok(())
}

/// A quoted `"<<"` is a merge key in yq's default reading and an ordinary string key under the flag
/// (yq tests the key's tag there): `.t` keeps the entry and `.t[]` yields the mapping it names.
#[rustfmt::skip]
const QUOTED_ROWS: &[(&str, &str, &str, i32)] = &[
    (".t", "json", "{\n  \"<<\": {\n    \"x\": 1\n  },\n  \"y\": 2\n}\n", 0),
    (".t", "yaml", "{\"<<\": *a, y: 2}\n", 0),
    (".t[]", "json", "{\n  \"x\": 1\n}\n2\n", 0),
    (".t.x", "json", "null\n", 0),
    (".t.x", "yaml", "null\n", 0),
    (".t.y", "json", "2\n", 0),
    (".t.y", "yaml", "2\n", 0),
    ("explode(.)", "json", "{\n  \"a\": {\n    \"x\": 1\n  },\n  \"t\": {\n    \"<<\": {\n      \"x\": 1\n    },\n    \"y\": 2\n  },\n  \"u\": {\n    \"x\": 1,\n    \"<<\": {\n      \"z\": 3\n    },\n    \"y\": 2\n  }\n}\n", 0),
    ("explode(.)", "yaml", "a: {x: 1}\nt: {\"<<\": {x: 1}, y: 2}\nu: {x: 1, \"<<\": {z: 3}, y: 2}\n", 0),
    (".u", "json", "{\n  \"x\": 1,\n  \"<<\": {\n    \"z\": 3\n  },\n  \"y\": 2\n}\n", 0),
    (".u", "yaml", "{!!merge <<: *a, \"<<\": {z: 3}, y: 2}\n", 0),
    (".u[]", "json", "1\n{\n  \"z\": 3\n}\n2\n", 0),
    (".u[]", "yaml", "1\n{z: 3}\n2\n", 0),
    (".u.x", "json", "1\n", 0),
    (".u.x", "yaml", "1\n", 0),
    (".u.z", "json", "null\n", 0),
    (".u.z", "yaml", "null\n", 0),
    ("[.u[]|key]", "json", "[\n  \"x\",\n  \"<<\",\n  \"y\"\n]\n", 0),
    ("[.u[]|key]", "yaml", "- x\n- \"<<\"\n- y\n", 0),
];

#[test]
fn a_quoted_merge_key_is_a_plain_key_under_the_spec_flag_7025() -> Result<()> {
    let doc = "a: &a {x: 1}\nt: {\"<<\": *a, y: 2}\nu: {<<: *a, \"<<\": {z: 3}, y: 2}\n";
    for (filter, format, stdout, status) in QUOTED_ROWS {
        let args = ["-o", format, "-I", "2", "--yaml-fix-merge-anchor-to-spec"];
        let (out, code) = run_yq_stdin(filter, doc, &args)?;
        assert_eq!(
            (out.as_str(), code),
            (*stdout, *status),
            "{filter} (-o {format})"
        );
    }
    Ok(())
}
