//! #7093: `--unwrapScalar[=BOOL]` and `-r[=BOOL]` take a value after `=` as yq's other boolean flags do,
//! and the value yq resolves (its own when given, else true for YAML and properties output and false
//! for the rest) decides how `-o=shell` quotes a value, whether `-o=props` and `@props` put a value with a
//! space in it in Go's quotes, and whether a top-level scalar of a JSON, KYAML or other encoder is
//! written raw. Every answer was captured from yq v4.53.3. The one row that still differs, a top-level
//! YAML scalar under `=false`, is in `docs/compliance/yq/limitations.md`.

use crate::run_yq_stdin;

const INPUT: &str = "name: Mike Wazowski\nq: \"a b\"\ne: \"\"\nn: 5\ns: hello\nl: [x y, z]\n";

/// (flags, filter, yq's stdout over [`INPUT`]).
#[rustfmt::skip]
const ROWS: &[(&[&str], &str, &str)] = &[
    (&["-o=props"], ".", "name = Mike Wazowski\nq = a b\ne = \nn = 5\ns = hello\nl.0 = x y\nl.1 = z\n"),
    (&["-o=props", "--unwrapScalar=false"], ".", "name = \"Mike Wazowski\"\nq = \"a b\"\ne = \nn = 5\ns = hello\nl.0 = \"x y\"\nl.1 = z\n"),
    (&["-o=props", "--unwrapScalar=true"], ".", "name = Mike Wazowski\nq = a b\ne = \nn = 5\ns = hello\nl.0 = x y\nl.1 = z\n"),
    (&["-o=props", "-r"], ".", "name = Mike Wazowski\nq = a b\ne = \nn = 5\ns = hello\nl.0 = x y\nl.1 = z\n"),
    (&["-o=props", "-r=false"], ".", "name = \"Mike Wazowski\"\nq = \"a b\"\ne = \nn = 5\ns = hello\nl.0 = \"x y\"\nl.1 = z\n"),
    (&["-o=props", "-r=true"], ".", "name = Mike Wazowski\nq = a b\ne = \nn = 5\ns = hello\nl.0 = x y\nl.1 = z\n"),
    (&["-o=props", "-r", "--unwrapScalar=false"], ".", "name = \"Mike Wazowski\"\nq = \"a b\"\ne = \nn = 5\ns = hello\nl.0 = \"x y\"\nl.1 = z\n"),
    (&["-o=shell"], ".", "name='Mike Wazowski'\nq='a b'\ne=\nn=5\ns=hello\nl_0='x y'\nl_1=z\n"),
    (&["-o=shell", "--unwrapScalar=false"], ".", "name='Mike Wazowski'\nq='a b'\ne=\nn=5\ns=hello\nl_0='x y'\nl_1=z\n"),
    (&["-o=shell", "--unwrapScalar=true"], ".", "name=Mike Wazowski\nq=a b\ne=\nn=5\ns=hello\nl_0=x y\nl_1=z\n"),
    (&["-o=shell", "-r"], ".", "name=Mike Wazowski\nq=a b\ne=\nn=5\ns=hello\nl_0=x y\nl_1=z\n"),
    (&["-o=shell", "-r=false"], ".", "name='Mike Wazowski'\nq='a b'\ne=\nn=5\ns=hello\nl_0='x y'\nl_1=z\n"),
    (&["-o=shell", "-r=true"], ".", "name=Mike Wazowski\nq=a b\ne=\nn=5\ns=hello\nl_0=x y\nl_1=z\n"),
    (&["-o=shell", "-r", "--unwrapScalar=false"], ".", "name='Mike Wazowski'\nq='a b'\ne=\nn=5\ns=hello\nl_0='x y'\nl_1=z\n"),
    (&["-o=json"], ".", "{\n  \"name\": \"Mike Wazowski\",\n  \"q\": \"a b\",\n  \"e\": \"\",\n  \"n\": 5,\n  \"s\": \"hello\",\n  \"l\": [\n    \"x y\",\n    \"z\"\n  ]\n}\n"),
    (&["-o=json", "--unwrapScalar=false"], ".", "{\n  \"name\": \"Mike Wazowski\",\n  \"q\": \"a b\",\n  \"e\": \"\",\n  \"n\": 5,\n  \"s\": \"hello\",\n  \"l\": [\n    \"x y\",\n    \"z\"\n  ]\n}\n"),
    (&["-o=json", "--unwrapScalar=true"], ".", "{\n  \"name\": \"Mike Wazowski\",\n  \"q\": \"a b\",\n  \"e\": \"\",\n  \"n\": 5,\n  \"s\": \"hello\",\n  \"l\": [\n    \"x y\",\n    \"z\"\n  ]\n}\n"),
    (&["-o=json", "-r"], ".", "{\n  \"name\": \"Mike Wazowski\",\n  \"q\": \"a b\",\n  \"e\": \"\",\n  \"n\": 5,\n  \"s\": \"hello\",\n  \"l\": [\n    \"x y\",\n    \"z\"\n  ]\n}\n"),
    (&["-o=json", "-r=false"], ".", "{\n  \"name\": \"Mike Wazowski\",\n  \"q\": \"a b\",\n  \"e\": \"\",\n  \"n\": 5,\n  \"s\": \"hello\",\n  \"l\": [\n    \"x y\",\n    \"z\"\n  ]\n}\n"),
    (&["-o=json", "-r=true"], ".", "{\n  \"name\": \"Mike Wazowski\",\n  \"q\": \"a b\",\n  \"e\": \"\",\n  \"n\": 5,\n  \"s\": \"hello\",\n  \"l\": [\n    \"x y\",\n    \"z\"\n  ]\n}\n"),
    (&["-o=json", "-r", "--unwrapScalar=false"], ".", "{\n  \"name\": \"Mike Wazowski\",\n  \"q\": \"a b\",\n  \"e\": \"\",\n  \"n\": 5,\n  \"s\": \"hello\",\n  \"l\": [\n    \"x y\",\n    \"z\"\n  ]\n}\n"),
    (&["-o=csv"], "[.q, .e, .n]", "a b,,5\n"),
    (&["-o=csv", "--unwrapScalar=false"], "[.q, .e, .n]", "a b,,5\n"),
    (&["-o=csv", "--unwrapScalar=true"], "[.q, .e, .n]", "a b,,5\n"),
    (&["-o=csv", "-r"], "[.q, .e, .n]", "a b,,5\n"),
    (&["-o=csv", "-r=false"], "[.q, .e, .n]", "a b,,5\n"),
    (&["-o=csv", "-r=true"], "[.q, .e, .n]", "a b,,5\n"),
    (&["-o=csv", "-r", "--unwrapScalar=false"], "[.q, .e, .n]", "a b,,5\n"),
    (&["-o=tsv"], "[.q, .e, .n]", "a b\t\t5\n"),
    (&["-o=tsv", "--unwrapScalar=false"], "[.q, .e, .n]", "a b\t\t5\n"),
    (&["-o=tsv", "--unwrapScalar=true"], "[.q, .e, .n]", "a b\t\t5\n"),
    (&["-o=tsv", "-r"], "[.q, .e, .n]", "a b\t\t5\n"),
    (&["-o=tsv", "-r=false"], "[.q, .e, .n]", "a b\t\t5\n"),
    (&["-o=tsv", "-r=true"], "[.q, .e, .n]", "a b\t\t5\n"),
    (&["-o=tsv", "-r", "--unwrapScalar=false"], "[.q, .e, .n]", "a b\t\t5\n"),
    (&["-o=xml"], ".", "<name>Mike Wazowski</name>\n<q>a b</q>\n<e></e>\n<n>5</n>\n<s>hello</s>\n<l>x y</l>\n<l>z</l>\n"),
    (&["-o=xml", "--unwrapScalar=false"], ".", "<name>Mike Wazowski</name>\n<q>a b</q>\n<e></e>\n<n>5</n>\n<s>hello</s>\n<l>x y</l>\n<l>z</l>\n"),
    (&["-o=xml", "--unwrapScalar=true"], ".", "<name>Mike Wazowski</name>\n<q>a b</q>\n<e></e>\n<n>5</n>\n<s>hello</s>\n<l>x y</l>\n<l>z</l>\n"),
    (&["-o=xml", "-r"], ".", "<name>Mike Wazowski</name>\n<q>a b</q>\n<e></e>\n<n>5</n>\n<s>hello</s>\n<l>x y</l>\n<l>z</l>\n"),
    (&["-o=xml", "-r=false"], ".", "<name>Mike Wazowski</name>\n<q>a b</q>\n<e></e>\n<n>5</n>\n<s>hello</s>\n<l>x y</l>\n<l>z</l>\n"),
    (&["-o=xml", "-r=true"], ".", "<name>Mike Wazowski</name>\n<q>a b</q>\n<e></e>\n<n>5</n>\n<s>hello</s>\n<l>x y</l>\n<l>z</l>\n"),
    (&["-o=xml", "-r", "--unwrapScalar=false"], ".", "<name>Mike Wazowski</name>\n<q>a b</q>\n<e></e>\n<n>5</n>\n<s>hello</s>\n<l>x y</l>\n<l>z</l>\n"),
    (&["-o=toml"], ".", "name = \"Mike Wazowski\"\nq = \"a b\"\ne = \"\"\nn = 5\ns = \"hello\"\nl = [\"x y\", \"z\"]\n"),
    (&["-o=toml", "--unwrapScalar=false"], ".", "name = \"Mike Wazowski\"\nq = \"a b\"\ne = \"\"\nn = 5\ns = \"hello\"\nl = [\"x y\", \"z\"]\n"),
    (&["-o=toml", "--unwrapScalar=true"], ".", "name = \"Mike Wazowski\"\nq = \"a b\"\ne = \"\"\nn = 5\ns = \"hello\"\nl = [\"x y\", \"z\"]\n"),
    (&["-o=toml", "-r"], ".", "name = \"Mike Wazowski\"\nq = \"a b\"\ne = \"\"\nn = 5\ns = \"hello\"\nl = [\"x y\", \"z\"]\n"),
    (&["-o=toml", "-r=false"], ".", "name = \"Mike Wazowski\"\nq = \"a b\"\ne = \"\"\nn = 5\ns = \"hello\"\nl = [\"x y\", \"z\"]\n"),
    (&["-o=toml", "-r=true"], ".", "name = \"Mike Wazowski\"\nq = \"a b\"\ne = \"\"\nn = 5\ns = \"hello\"\nl = [\"x y\", \"z\"]\n"),
    (&["-o=toml", "-r", "--unwrapScalar=false"], ".", "name = \"Mike Wazowski\"\nq = \"a b\"\ne = \"\"\nn = 5\ns = \"hello\"\nl = [\"x y\", \"z\"]\n"),
    (&["-o=lua"], ".", "return {\n\t[\"name\"] = \"Mike Wazowski\";\n\t[\"q\"] = \"a b\";\n\t[\"e\"] = \"\";\n\t[\"n\"] = 5;\n\t[\"s\"] = \"hello\";\n\t[\"l\"] = {\n\t\t\"x y\",\n\t\t\"z\",\n\t};\n};\n"),
    (&["-o=lua", "--unwrapScalar=false"], ".", "return {\n\t[\"name\"] = \"Mike Wazowski\";\n\t[\"q\"] = \"a b\";\n\t[\"e\"] = \"\";\n\t[\"n\"] = 5;\n\t[\"s\"] = \"hello\";\n\t[\"l\"] = {\n\t\t\"x y\",\n\t\t\"z\",\n\t};\n};\n"),
    (&["-o=lua", "--unwrapScalar=true"], ".", "return {\n\t[\"name\"] = \"Mike Wazowski\";\n\t[\"q\"] = \"a b\";\n\t[\"e\"] = \"\";\n\t[\"n\"] = 5;\n\t[\"s\"] = \"hello\";\n\t[\"l\"] = {\n\t\t\"x y\",\n\t\t\"z\",\n\t};\n};\n"),
    (&["-o=lua", "-r"], ".", "return {\n\t[\"name\"] = \"Mike Wazowski\";\n\t[\"q\"] = \"a b\";\n\t[\"e\"] = \"\";\n\t[\"n\"] = 5;\n\t[\"s\"] = \"hello\";\n\t[\"l\"] = {\n\t\t\"x y\",\n\t\t\"z\",\n\t};\n};\n"),
    (&["-o=lua", "-r=false"], ".", "return {\n\t[\"name\"] = \"Mike Wazowski\";\n\t[\"q\"] = \"a b\";\n\t[\"e\"] = \"\";\n\t[\"n\"] = 5;\n\t[\"s\"] = \"hello\";\n\t[\"l\"] = {\n\t\t\"x y\",\n\t\t\"z\",\n\t};\n};\n"),
    (&["-o=lua", "-r=true"], ".", "return {\n\t[\"name\"] = \"Mike Wazowski\";\n\t[\"q\"] = \"a b\";\n\t[\"e\"] = \"\";\n\t[\"n\"] = 5;\n\t[\"s\"] = \"hello\";\n\t[\"l\"] = {\n\t\t\"x y\",\n\t\t\"z\",\n\t};\n};\n"),
    (&["-o=lua", "-r", "--unwrapScalar=false"], ".", "return {\n\t[\"name\"] = \"Mike Wazowski\";\n\t[\"q\"] = \"a b\";\n\t[\"e\"] = \"\";\n\t[\"n\"] = 5;\n\t[\"s\"] = \"hello\";\n\t[\"l\"] = {\n\t\t\"x y\",\n\t\t\"z\",\n\t};\n};\n"),
    (&["-o=kyaml"], ".", "{\n  name: \"Mike Wazowski\",\n  q: \"a b\",\n  e: \"\",\n  n: 5,\n  s: \"hello\",\n  l: [\n    \"x y\",\n    \"z\",\n  ],\n}\n"),
    (&["-o=kyaml", "--unwrapScalar=false"], ".", "{\n  name: \"Mike Wazowski\",\n  q: \"a b\",\n  e: \"\",\n  n: 5,\n  s: \"hello\",\n  l: [\n    \"x y\",\n    \"z\",\n  ],\n}\n"),
    (&["-o=kyaml", "--unwrapScalar=true"], ".", "{\n  name: \"Mike Wazowski\",\n  q: \"a b\",\n  e: \"\",\n  n: 5,\n  s: \"hello\",\n  l: [\n    \"x y\",\n    \"z\",\n  ],\n}\n"),
    (&["-o=kyaml", "-r"], ".", "{\n  name: \"Mike Wazowski\",\n  q: \"a b\",\n  e: \"\",\n  n: 5,\n  s: \"hello\",\n  l: [\n    \"x y\",\n    \"z\",\n  ],\n}\n"),
    (&["-o=kyaml", "-r=false"], ".", "{\n  name: \"Mike Wazowski\",\n  q: \"a b\",\n  e: \"\",\n  n: 5,\n  s: \"hello\",\n  l: [\n    \"x y\",\n    \"z\",\n  ],\n}\n"),
    (&["-o=kyaml", "-r=true"], ".", "{\n  name: \"Mike Wazowski\",\n  q: \"a b\",\n  e: \"\",\n  n: 5,\n  s: \"hello\",\n  l: [\n    \"x y\",\n    \"z\",\n  ],\n}\n"),
    (&["-o=kyaml", "-r", "--unwrapScalar=false"], ".", "{\n  name: \"Mike Wazowski\",\n  q: \"a b\",\n  e: \"\",\n  n: 5,\n  s: \"hello\",\n  l: [\n    \"x y\",\n    \"z\",\n  ],\n}\n"),
    (&["-o=json", "--unwrapScalar=true"], ".q", "a b\n"),
    (&["-o=json", "-r"], ".q", "a b\n"),
    (&["-o=json", "-r=true"], ".q", "a b\n"),
    (&["-o=json", "--unwrapScalar=false"], ".q", "\"a b\"\n"),
    (&["-o=yaml", "--unwrapScalar=true"], ".q", "a b\n"),
    (&["-o=yaml", "-r"], ".q", "a b\n"),
    (&["-o=yaml"], ".q", "a b\n"),
    (&["-o=json"], ".q", "\"a b\"\n"),
    (&["-o=yaml"], "{\"a\":\"x y\",\"b\":\"z\"} | @props", "a = x y\nb = z\n\n"),
    (&["-o=yaml"], ".l | @props", "0 = x y\n1 = z\n\n"),
    (&["-o=json"], "{\"a\":\"x y\",\"b\":\"z\"} | @props", "\"a = \\\"x y\\\"\\nb = z\\n\"\n"),
    (&["-o=json"], ".l | @props", "\"0 = \\\"x y\\\"\\n1 = z\\n\"\n"),
    (&["-o=props"], "{\"a\":\"x y\",\"b\":\"z\"} | @props", "a = x y\nb = z\n\n"),
    (&["-o=props"], ".l | @props", "0 = x y\n1 = z\n\n"),
    (&["-o=shell"], "{\"a\":\"x y\",\"b\":\"z\"} | @props", "value='a = \"x y\"\nb = z\n'\n"),
    (&["-o=shell"], ".l | @props", "value='0 = \"x y\"\n1 = z\n'\n"),
    (&["-o=json", "--unwrapScalar=false"], "{\"a\":\"x y\",\"b\":\"z\"} | @props", "\"a = \\\"x y\\\"\\nb = z\\n\"\n"),
    (&["-o=json", "--unwrapScalar=false"], ".l | @props", "\"0 = \\\"x y\\\"\\n1 = z\\n\"\n"),
    (&["-o=props", "--unwrapScalar=false"], "{\"a\":\"x y\",\"b\":\"z\"} | @props", "a = \"x y\"\nb = z\n\n"),
    (&["-o=props", "--unwrapScalar=false"], ".l | @props", "0 = \"x y\"\n1 = z\n\n"),
    (&["-o=shell", "--unwrapScalar=false"], "{\"a\":\"x y\",\"b\":\"z\"} | @props", "value='a = \"x y\"\nb = z\n'\n"),
    (&["-o=shell", "--unwrapScalar=false"], ".l | @props", "value='0 = \"x y\"\n1 = z\n'\n"),
    (&["-o=yaml", "--unwrapScalar=true"], "{\"a\":\"x y\",\"b\":\"z\"} | @props", "a = x y\nb = z\n\n"),
    (&["-o=yaml", "--unwrapScalar=true"], ".l | @props", "0 = x y\n1 = z\n\n"),
    (&["-o=json", "--unwrapScalar=true"], "{\"a\":\"x y\",\"b\":\"z\"} | @props", "a = x y\nb = z\n\n"),
    (&["-o=json", "--unwrapScalar=true"], ".l | @props", "0 = x y\n1 = z\n\n"),
    (&["-o=props", "--unwrapScalar=true"], "{\"a\":\"x y\",\"b\":\"z\"} | @props", "a = x y\nb = z\n\n"),
    (&["-o=props", "--unwrapScalar=true"], ".l | @props", "0 = x y\n1 = z\n\n"),
    (&["-o=shell", "--unwrapScalar=true"], "{\"a\":\"x y\",\"b\":\"z\"} | @props", "value=a = x y\nb = z\n\n"),
    (&["-o=shell", "--unwrapScalar=true"], ".l | @props", "value=0 = x y\n1 = z\n\n"),
];

#[test]
fn unwrap_scalar_takes_a_value_and_resolves_like_yq_7093() {
    for &(flags, filter, stdout) in ROWS {
        let (out, code) = run_yq_stdin(filter, INPUT, flags).expect("runs");
        assert_eq!((out.as_str(), code), (stdout, 0), "{flags:?} {filter:?}");
    }
}

/// A value that is not a Go boolean is refused where yq refuses it.
#[test]
fn unwrap_scalar_refuses_a_value_that_is_not_a_boolean_7093() {
    let (out, code) = run_yq_stdin(".", INPUT, &["--unwrapScalar=maybe"]).expect("runs");
    assert_eq!((out.as_str(), code), ("", 2));
}
