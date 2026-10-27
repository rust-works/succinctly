//! #5003: `pick(...)` keeps what the document wrote about the nodes it keeps: their trailing
//! comments, flow style, quoting, anchors, tags and block-scalar header comments, their keys' own
//! quoting, and the document's head comment, as yq's does. The pick copies the document's nodes,
//! so nothing is reconstructed from the values.
//!
//! Every output is captured from yq v4.53.3.

use anyhow::Result;

use crate::run_yq_stdin;

/// (document, filter, extra arguments, `yq` output).
const ROWS: &[(&str, &str, &[&str], &str)] = &[
    ("# head\na: x # ac\n# between\n\"b\": &anc [1, 2] # bc\nc:\n  d: 1 # dc\n  e: 2\n  # foot-e\nf: |  # fh\n  text\n# end\n", "pick([\"a\"])", &[], "# head\na: x # ac\n"),
    ("# head\na: x # ac\n# between\n\"b\": &anc [1, 2] # bc\nc:\n  d: 1 # dc\n  e: 2\n  # foot-e\nf: |  # fh\n  text\n# end\n", "pick([\"a\"])", &["-P"], "# head\na: x # ac\n"),
    ("# head\na: x # ac\n# between\n\"b\": &anc [1, 2] # bc\nc:\n  d: 1 # dc\n  e: 2\n  # foot-e\nf: |  # fh\n  text\n# end\n", "pick([\"b\"])", &[], "# head\n# between\n\"b\": &anc [1, 2] # bc\n"),
    ("# head\na: x # ac\n# between\n\"b\": &anc [1, 2] # bc\nc:\n  d: 1 # dc\n  e: 2\n  # foot-e\nf: |  # fh\n  text\n# end\n", "pick([\"b\"])", &["-P"], "# head\n# between\nb: &anc\n  - 1\n  - 2\n"),
    ("# head\na: x # ac\n# between\n\"b\": &anc [1, 2] # bc\nc:\n  d: 1 # dc\n  e: 2\n  # foot-e\nf: |  # fh\n  text\n# end\n", "pick([\"c\"])", &[], "# head\nc:\n  d: 1 # dc\n  e: 2\n  # foot-e\n"),
    ("# head\na: x # ac\n# between\n\"b\": &anc [1, 2] # bc\nc:\n  d: 1 # dc\n  e: 2\n  # foot-e\nf: |  # fh\n  text\n# end\n", "pick([\"c\"])", &["-P"], "# head\nc:\n  d: 1 # dc\n  e: 2\n  # foot-e\n"),
    ("# head\na: x # ac\n# between\n\"b\": &anc [1, 2] # bc\nc:\n  d: 1 # dc\n  e: 2\n  # foot-e\nf: |  # fh\n  text\n# end\n", "pick([\"f\"])", &[], "# head\nf: | # fh\n  text\n# end\n"),
    ("# head\na: x # ac\n# between\n\"b\": &anc [1, 2] # bc\nc:\n  d: 1 # dc\n  e: 2\n  # foot-e\nf: |  # fh\n  text\n# end\n", "pick([\"f\"])", &["-P"], "# head\nf: | # fh\n  text\n# end\n"),
    ("# head\na: x # ac\n# between\n\"b\": &anc [1, 2] # bc\nc:\n  d: 1 # dc\n  e: 2\n  # foot-e\nf: |  # fh\n  text\n# end\n", "pick([\"f\",\"a\"])", &[], "# head\nf: | # fh\n  text\n# end\n\na: x # ac\n"),
    ("# head\na: x # ac\n# between\n\"b\": &anc [1, 2] # bc\nc:\n  d: 1 # dc\n  e: 2\n  # foot-e\nf: |  # fh\n  text\n# end\n", "pick([\"f\",\"a\"])", &["-P"], "# head\nf: | # fh\n  text\n# end\n\na: x # ac\n"),
    ("# head\na: x # ac\n# between\n\"b\": &anc [1, 2] # bc\nc:\n  d: 1 # dc\n  e: 2\n  # foot-e\nf: |  # fh\n  text\n# end\n", "pick([\"c\",\"a\"])", &[], "# head\nc:\n  d: 1 # dc\n  e: 2\n  # foot-e\na: x # ac\n"),
    ("# head\na: x # ac\n# between\n\"b\": &anc [1, 2] # bc\nc:\n  d: 1 # dc\n  e: 2\n  # foot-e\nf: |  # fh\n  text\n# end\n", "pick([\"c\",\"a\"])", &["-P"], "# head\nc:\n  d: 1 # dc\n  e: 2\n  # foot-e\na: x # ac\n"),
    ("# head\na: x # ac\n# between\n\"b\": &anc [1, 2] # bc\nc:\n  d: 1 # dc\n  e: 2\n  # foot-e\nf: |  # fh\n  text\n# end\n", "pick([\"other\"])", &[], "# head\n{}\n"),
    ("# head\na: x # ac\n# between\n\"b\": &anc [1, 2] # bc\nc:\n  d: 1 # dc\n  e: 2\n  # foot-e\nf: |  # fh\n  text\n# end\n", "pick([\"other\"])", &["-P"], "# head\n{}\n"),
    ("# head\na: x # ac\n# between\n\"b\": &anc [1, 2] # bc\nc:\n  d: 1 # dc\n  e: 2\n  # foot-e\nf: |  # fh\n  text\n# end\n", "pick([\"zz\"])", &[], "# head\n{}\n"),
    ("# head\na: x # ac\n# between\n\"b\": &anc [1, 2] # bc\nc:\n  d: 1 # dc\n  e: 2\n  # foot-e\nf: |  # fh\n  text\n# end\n", "pick([\"zz\"])", &["-P"], "# head\n{}\n"),
    ("# head\na: x # ac\n# between\n\"b\": &anc [1, 2] # bc\nc:\n  d: 1 # dc\n  e: 2\n  # foot-e\nf: |  # fh\n  text\n# end\n", "pick([])", &[], "# head\n{}\n"),
    ("# head\na: x # ac\n# between\n\"b\": &anc [1, 2] # bc\nc:\n  d: 1 # dc\n  e: 2\n  # foot-e\nf: |  # fh\n  text\n# end\n", "pick([])", &["-P"], "# head\n{}\n"),
    ("a: {x: 1, y: 2} # fc\nb: [1, 2]\nc: &k {z: 1}\nd: *k\n", "pick([\"a\"])", &[], "a: {x: 1, y: 2} # fc\n"),
    ("a: {x: 1, y: 2} # fc\nb: [1, 2]\nc: &k {z: 1}\nd: *k\n", "pick([\"a\"])", &["-P"], "a:\n  x: 1\n  y: 2\n"),
    ("a: {x: 1, y: 2} # fc\nb: [1, 2]\nc: &k {z: 1}\nd: *k\n", "pick([\"b\"])", &[], "b: [1, 2]\n"),
    ("a: {x: 1, y: 2} # fc\nb: [1, 2]\nc: &k {z: 1}\nd: *k\n", "pick([\"b\"])", &["-P"], "b:\n  - 1\n  - 2\n"),
    ("a: {x: 1, y: 2} # fc\nb: [1, 2]\nc: &k {z: 1}\nd: *k\n", "pick([\"c\"])", &[], "c: &k {z: 1}\n"),
    ("a: {x: 1, y: 2} # fc\nb: [1, 2]\nc: &k {z: 1}\nd: *k\n", "pick([\"c\"])", &["-P"], "c: &k\n  z: 1\n"),
    ("a: {x: 1, y: 2} # fc\nb: [1, 2]\nc: &k {z: 1}\nd: *k\n", "pick([\"f\"])", &[], "{}\n"),
    ("a: {x: 1, y: 2} # fc\nb: [1, 2]\nc: &k {z: 1}\nd: *k\n", "pick([\"f\"])", &["-P"], "{}\n"),
    ("a: {x: 1, y: 2} # fc\nb: [1, 2]\nc: &k {z: 1}\nd: *k\n", "pick([\"f\",\"a\"])", &[], "a: {x: 1, y: 2} # fc\n"),
    ("a: {x: 1, y: 2} # fc\nb: [1, 2]\nc: &k {z: 1}\nd: *k\n", "pick([\"f\",\"a\"])", &["-P"], "a:\n  x: 1\n  y: 2\n"),
    ("a: {x: 1, y: 2} # fc\nb: [1, 2]\nc: &k {z: 1}\nd: *k\n", "pick([\"c\",\"a\"])", &[], "c: &k {z: 1}\na: {x: 1, y: 2} # fc\n"),
    ("a: {x: 1, y: 2} # fc\nb: [1, 2]\nc: &k {z: 1}\nd: *k\n", "pick([\"c\",\"a\"])", &["-P"], "c: &k\n  z: 1\na:\n  x: 1\n  y: 2\n"),
    ("a: {x: 1, y: 2} # fc\nb: [1, 2]\nc: &k {z: 1}\nd: *k\n", "pick([\"other\"])", &[], "{}\n"),
    ("a: {x: 1, y: 2} # fc\nb: [1, 2]\nc: &k {z: 1}\nd: *k\n", "pick([\"other\"])", &["-P"], "{}\n"),
    ("a: {x: 1, y: 2} # fc\nb: [1, 2]\nc: &k {z: 1}\nd: *k\n", "pick([\"zz\"])", &[], "{}\n"),
    ("a: {x: 1, y: 2} # fc\nb: [1, 2]\nc: &k {z: 1}\nd: *k\n", "pick([\"zz\"])", &["-P"], "{}\n"),
    ("a: {x: 1, y: 2} # fc\nb: [1, 2]\nc: &k {z: 1}\nd: *k\n", "pick([])", &[], "{}\n"),
    ("a: {x: 1, y: 2} # fc\nb: [1, 2]\nc: &k {z: 1}\nd: *k\n", "pick([])", &["-P"], "{}\n"),
    ("items:\n  - n: 1 # one\n    m: 2\n  - n: 3\nother: !foo bar # t\n", "pick([\"a\"])", &[], "{}\n"),
    ("items:\n  - n: 1 # one\n    m: 2\n  - n: 3\nother: !foo bar # t\n", "pick([\"a\"])", &["-P"], "{}\n"),
    ("items:\n  - n: 1 # one\n    m: 2\n  - n: 3\nother: !foo bar # t\n", "pick([\"b\"])", &[], "{}\n"),
    ("items:\n  - n: 1 # one\n    m: 2\n  - n: 3\nother: !foo bar # t\n", "pick([\"b\"])", &["-P"], "{}\n"),
    ("items:\n  - n: 1 # one\n    m: 2\n  - n: 3\nother: !foo bar # t\n", "pick([\"c\"])", &[], "{}\n"),
    ("items:\n  - n: 1 # one\n    m: 2\n  - n: 3\nother: !foo bar # t\n", "pick([\"c\"])", &["-P"], "{}\n"),
    ("items:\n  - n: 1 # one\n    m: 2\n  - n: 3\nother: !foo bar # t\n", "pick([\"f\"])", &[], "{}\n"),
    ("items:\n  - n: 1 # one\n    m: 2\n  - n: 3\nother: !foo bar # t\n", "pick([\"f\"])", &["-P"], "{}\n"),
    ("items:\n  - n: 1 # one\n    m: 2\n  - n: 3\nother: !foo bar # t\n", "pick([\"f\",\"a\"])", &[], "{}\n"),
    ("items:\n  - n: 1 # one\n    m: 2\n  - n: 3\nother: !foo bar # t\n", "pick([\"f\",\"a\"])", &["-P"], "{}\n"),
    ("items:\n  - n: 1 # one\n    m: 2\n  - n: 3\nother: !foo bar # t\n", "pick([\"c\",\"a\"])", &[], "{}\n"),
    ("items:\n  - n: 1 # one\n    m: 2\n  - n: 3\nother: !foo bar # t\n", "pick([\"c\",\"a\"])", &["-P"], "{}\n"),
    ("items:\n  - n: 1 # one\n    m: 2\n  - n: 3\nother: !foo bar # t\n", "pick([\"other\"])", &[], "other: !foo bar # t\n"),
    ("items:\n  - n: 1 # one\n    m: 2\n  - n: 3\nother: !foo bar # t\n", "pick([\"other\"])", &["-P"], "other: !foo bar # t\n"),
    ("items:\n  - n: 1 # one\n    m: 2\n  - n: 3\nother: !foo bar # t\n", "pick([\"zz\"])", &[], "{}\n"),
    ("items:\n  - n: 1 # one\n    m: 2\n  - n: 3\nother: !foo bar # t\n", "pick([\"zz\"])", &["-P"], "{}\n"),
    ("items:\n  - n: 1 # one\n    m: 2\n  - n: 3\nother: !foo bar # t\n", "pick([])", &[], "{}\n"),
    ("items:\n  - n: 1 # one\n    m: 2\n  - n: 3\nother: !foo bar # t\n", "pick([])", &["-P"], "{}\n"),
    ("a: # kc\n  x: 1\nb: 2\n", "pick([\"a\"])", &[], "a: # kc\n  x: 1\n"),
    ("a: # kc\n  x: 1\nb: 2\n", "pick([\"a\"])", &["-P"], "a: # kc\n  x: 1\n"),
    ("a: # kc\n  x: 1\nb: 2\n", "pick([\"b\"])", &[], "b: 2\n"),
    ("a: # kc\n  x: 1\nb: 2\n", "pick([\"b\"])", &["-P"], "b: 2\n"),
    ("a: # kc\n  x: 1\nb: 2\n", "pick([\"c\"])", &[], "{}\n"),
    ("a: # kc\n  x: 1\nb: 2\n", "pick([\"c\"])", &["-P"], "{}\n"),
    ("a: # kc\n  x: 1\nb: 2\n", "pick([\"f\"])", &[], "{}\n"),
    ("a: # kc\n  x: 1\nb: 2\n", "pick([\"f\"])", &["-P"], "{}\n"),
    ("a: # kc\n  x: 1\nb: 2\n", "pick([\"f\",\"a\"])", &[], "a: # kc\n  x: 1\n"),
    ("a: # kc\n  x: 1\nb: 2\n", "pick([\"f\",\"a\"])", &["-P"], "a: # kc\n  x: 1\n"),
    ("a: # kc\n  x: 1\nb: 2\n", "pick([\"c\",\"a\"])", &[], "a: # kc\n  x: 1\n"),
    ("a: # kc\n  x: 1\nb: 2\n", "pick([\"c\",\"a\"])", &["-P"], "a: # kc\n  x: 1\n"),
    ("a: # kc\n  x: 1\nb: 2\n", "pick([\"other\"])", &[], "{}\n"),
    ("a: # kc\n  x: 1\nb: 2\n", "pick([\"other\"])", &["-P"], "{}\n"),
    ("a: # kc\n  x: 1\nb: 2\n", "pick([\"zz\"])", &[], "{}\n"),
    ("a: # kc\n  x: 1\nb: 2\n", "pick([\"zz\"])", &["-P"], "{}\n"),
    ("a: # kc\n  x: 1\nb: 2\n", "pick([])", &[], "{}\n"),
    ("a: # kc\n  x: 1\nb: 2\n", "pick([])", &["-P"], "{}\n"),
    ("'a': !!str 1\n? b\n: 2\nc: !foo\n  d: 1\n", "pick([\"a\"])", &[], "'a': !!str 1\n"),
    ("'a': !!str 1\n? b\n: 2\nc: !foo\n  d: 1\n", "pick([\"a\"])", &["-P"], "a: \"1\"\n"),
    ("'a': !!str 1\n? b\n: 2\nc: !foo\n  d: 1\n", "pick([\"b\"])", &[], "b: 2\n"),
    ("'a': !!str 1\n? b\n: 2\nc: !foo\n  d: 1\n", "pick([\"b\"])", &["-P"], "b: 2\n"),
    ("'a': !!str 1\n? b\n: 2\nc: !foo\n  d: 1\n", "pick([\"c\"])", &[], "c: !foo\n  d: 1\n"),
    ("'a': !!str 1\n? b\n: 2\nc: !foo\n  d: 1\n", "pick([\"c\"])", &["-P"], "c: !foo\n  d: 1\n"),
    ("'a': !!str 1\n? b\n: 2\nc: !foo\n  d: 1\n", "pick([\"f\"])", &[], "{}\n"),
    ("'a': !!str 1\n? b\n: 2\nc: !foo\n  d: 1\n", "pick([\"f\"])", &["-P"], "{}\n"),
    ("'a': !!str 1\n? b\n: 2\nc: !foo\n  d: 1\n", "pick([\"f\",\"a\"])", &[], "'a': !!str 1\n"),
    ("'a': !!str 1\n? b\n: 2\nc: !foo\n  d: 1\n", "pick([\"f\",\"a\"])", &["-P"], "a: \"1\"\n"),
    ("'a': !!str 1\n? b\n: 2\nc: !foo\n  d: 1\n", "pick([\"c\",\"a\"])", &[], "c: !foo\n  d: 1\n'a': !!str 1\n"),
    ("'a': !!str 1\n? b\n: 2\nc: !foo\n  d: 1\n", "pick([\"c\",\"a\"])", &["-P"], "c: !foo\n  d: 1\na: \"1\"\n"),
    ("'a': !!str 1\n? b\n: 2\nc: !foo\n  d: 1\n", "pick([\"other\"])", &[], "{}\n"),
    ("'a': !!str 1\n? b\n: 2\nc: !foo\n  d: 1\n", "pick([\"other\"])", &["-P"], "{}\n"),
    ("'a': !!str 1\n? b\n: 2\nc: !foo\n  d: 1\n", "pick([\"zz\"])", &[], "{}\n"),
    ("'a': !!str 1\n? b\n: 2\nc: !foo\n  d: 1\n", "pick([\"zz\"])", &["-P"], "{}\n"),
    ("'a': !!str 1\n? b\n: 2\nc: !foo\n  d: 1\n", "pick([])", &[], "{}\n"),
    ("'a': !!str 1\n? b\n: 2\nc: !foo\n  d: 1\n", "pick([])", &["-P"], "{}\n"),
    ("a: 1\nb: 2\n# trailing\n", "pick([\"a\"])", &[], "a: 1\n"),
    ("a: 1\nb: 2\n# trailing\n", "pick([\"a\"])", &["-P"], "a: 1\n"),
    ("a: 1\nb: 2\n# trailing\n", "pick([\"b\"])", &[], "b: 2\n# trailing\n"),
    ("a: 1\nb: 2\n# trailing\n", "pick([\"b\"])", &["-P"], "b: 2\n# trailing\n"),
    ("a: 1\nb: 2\n# trailing\n", "pick([\"c\"])", &[], "{}\n"),
    ("a: 1\nb: 2\n# trailing\n", "pick([\"c\"])", &["-P"], "{}\n"),
    ("a: 1\nb: 2\n# trailing\n", "pick([\"f\"])", &[], "{}\n"),
    ("a: 1\nb: 2\n# trailing\n", "pick([\"f\"])", &["-P"], "{}\n"),
    ("a: 1\nb: 2\n# trailing\n", "pick([\"f\",\"a\"])", &[], "a: 1\n"),
    ("a: 1\nb: 2\n# trailing\n", "pick([\"f\",\"a\"])", &["-P"], "a: 1\n"),
    ("a: 1\nb: 2\n# trailing\n", "pick([\"c\",\"a\"])", &[], "a: 1\n"),
    ("a: 1\nb: 2\n# trailing\n", "pick([\"c\",\"a\"])", &["-P"], "a: 1\n"),
    ("a: 1\nb: 2\n# trailing\n", "pick([\"other\"])", &[], "{}\n"),
    ("a: 1\nb: 2\n# trailing\n", "pick([\"other\"])", &["-P"], "{}\n"),
    ("a: 1\nb: 2\n# trailing\n", "pick([\"zz\"])", &[], "{}\n"),
    ("a: 1\nb: 2\n# trailing\n", "pick([\"zz\"])", &["-P"], "{}\n"),
    ("a: 1\nb: 2\n# trailing\n", "pick([])", &[], "{}\n"),
    ("a: 1\nb: 2\n# trailing\n", "pick([])", &["-P"], "{}\n"),
    ("a:\n  b:\n    c: 1 # cc\n    d: 2\n  e: 3\nf: 4\n", "pick([\"a\"])", &[], "a:\n  b:\n    c: 1 # cc\n    d: 2\n  e: 3\n"),
    ("a:\n  b:\n    c: 1 # cc\n    d: 2\n  e: 3\nf: 4\n", "pick([\"a\"])", &["-P"], "a:\n  b:\n    c: 1 # cc\n    d: 2\n  e: 3\n"),
    ("a:\n  b:\n    c: 1 # cc\n    d: 2\n  e: 3\nf: 4\n", "pick([\"b\"])", &[], "{}\n"),
    ("a:\n  b:\n    c: 1 # cc\n    d: 2\n  e: 3\nf: 4\n", "pick([\"b\"])", &["-P"], "{}\n"),
    ("a:\n  b:\n    c: 1 # cc\n    d: 2\n  e: 3\nf: 4\n", "pick([\"c\"])", &[], "{}\n"),
    ("a:\n  b:\n    c: 1 # cc\n    d: 2\n  e: 3\nf: 4\n", "pick([\"c\"])", &["-P"], "{}\n"),
    ("a:\n  b:\n    c: 1 # cc\n    d: 2\n  e: 3\nf: 4\n", "pick([\"f\"])", &[], "f: 4\n"),
    ("a:\n  b:\n    c: 1 # cc\n    d: 2\n  e: 3\nf: 4\n", "pick([\"f\"])", &["-P"], "f: 4\n"),
    ("a:\n  b:\n    c: 1 # cc\n    d: 2\n  e: 3\nf: 4\n", "pick([\"f\",\"a\"])", &[], "f: 4\na:\n  b:\n    c: 1 # cc\n    d: 2\n  e: 3\n"),
    ("a:\n  b:\n    c: 1 # cc\n    d: 2\n  e: 3\nf: 4\n", "pick([\"f\",\"a\"])", &["-P"], "f: 4\na:\n  b:\n    c: 1 # cc\n    d: 2\n  e: 3\n"),
    ("a:\n  b:\n    c: 1 # cc\n    d: 2\n  e: 3\nf: 4\n", "pick([\"c\",\"a\"])", &[], "a:\n  b:\n    c: 1 # cc\n    d: 2\n  e: 3\n"),
    ("a:\n  b:\n    c: 1 # cc\n    d: 2\n  e: 3\nf: 4\n", "pick([\"c\",\"a\"])", &["-P"], "a:\n  b:\n    c: 1 # cc\n    d: 2\n  e: 3\n"),
    ("a:\n  b:\n    c: 1 # cc\n    d: 2\n  e: 3\nf: 4\n", "pick([\"other\"])", &[], "{}\n"),
    ("a:\n  b:\n    c: 1 # cc\n    d: 2\n  e: 3\nf: 4\n", "pick([\"other\"])", &["-P"], "{}\n"),
    ("a:\n  b:\n    c: 1 # cc\n    d: 2\n  e: 3\nf: 4\n", "pick([\"zz\"])", &[], "{}\n"),
    ("a:\n  b:\n    c: 1 # cc\n    d: 2\n  e: 3\nf: 4\n", "pick([\"zz\"])", &["-P"], "{}\n"),
    ("a:\n  b:\n    c: 1 # cc\n    d: 2\n  e: 3\nf: 4\n", "pick([])", &[], "{}\n"),
    ("a:\n  b:\n    c: 1 # cc\n    d: 2\n  e: 3\nf: 4\n", "pick([])", &["-P"], "{}\n"),
    ("a: 1\nb: 2\nc: 3\n", "pick([\"a\"])", &[], "a: 1\n"),
    ("a: 1\nb: 2\nc: 3\n", "pick([\"a\"])", &["-P"], "a: 1\n"),
    ("a: 1\nb: 2\nc: 3\n", "pick([\"b\"])", &[], "b: 2\n"),
    ("a: 1\nb: 2\nc: 3\n", "pick([\"b\"])", &["-P"], "b: 2\n"),
    ("a: 1\nb: 2\nc: 3\n", "pick([\"c\"])", &[], "c: 3\n"),
    ("a: 1\nb: 2\nc: 3\n", "pick([\"c\"])", &["-P"], "c: 3\n"),
    ("a: 1\nb: 2\nc: 3\n", "pick([\"f\"])", &[], "{}\n"),
    ("a: 1\nb: 2\nc: 3\n", "pick([\"f\"])", &["-P"], "{}\n"),
    ("a: 1\nb: 2\nc: 3\n", "pick([\"f\",\"a\"])", &[], "a: 1\n"),
    ("a: 1\nb: 2\nc: 3\n", "pick([\"f\",\"a\"])", &["-P"], "a: 1\n"),
    ("a: 1\nb: 2\nc: 3\n", "pick([\"c\",\"a\"])", &[], "c: 3\na: 1\n"),
    ("a: 1\nb: 2\nc: 3\n", "pick([\"c\",\"a\"])", &["-P"], "c: 3\na: 1\n"),
    ("a: 1\nb: 2\nc: 3\n", "pick([\"other\"])", &[], "{}\n"),
    ("a: 1\nb: 2\nc: 3\n", "pick([\"other\"])", &["-P"], "{}\n"),
    ("a: 1\nb: 2\nc: 3\n", "pick([\"zz\"])", &[], "{}\n"),
    ("a: 1\nb: 2\nc: 3\n", "pick([\"zz\"])", &["-P"], "{}\n"),
    ("a: 1\nb: 2\nc: 3\n", "pick([])", &[], "{}\n"),
    ("a: 1\nb: 2\nc: 3\n", "pick([])", &["-P"], "{}\n"),
    ("- a: 1 # x\n  b: 2\n- a: 3 # z\n  b: 4\n", "pick([0])", &[], "- a: 1 # x\n  b: 2\n"),
    ("- a: 1 # x\n  b: 2\n- a: 3 # z\n  b: 4\n", "pick([0])", &["-P"], "- a: 1 # x\n  b: 2\n"),
    ("- a: 1 # x\n  b: 2\n- a: 3 # z\n  b: 4\n", "pick([1])", &[], "- a: 3 # z\n  b: 4\n"),
    ("- a: 1 # x\n  b: 2\n- a: 3 # z\n  b: 4\n", "pick([1])", &["-P"], "- a: 3 # z\n  b: 4\n"),
    ("- a: 1 # x\n  b: 2\n- a: 3 # z\n  b: 4\n", "pick([1,0])", &[], "- a: 3 # z\n  b: 4\n- a: 1 # x\n  b: 2\n"),
    ("- a: 1 # x\n  b: 2\n- a: 3 # z\n  b: 4\n", "pick([1,0])", &["-P"], "- a: 3 # z\n  b: 4\n- a: 1 # x\n  b: 2\n"),
    ("- a: 1 # x\n  b: 2\n- a: 3 # z\n  b: 4\n", "pick([5])", &[], "[]\n"),
    ("- a: 1 # x\n  b: 2\n- a: 3 # z\n  b: 4\n", "pick([5])", &["-P"], "[]\n"),
    ("- a: 1 # x\n  b: 2\n- a: 3 # z\n  b: 4\n", "pick([])", &[], "[]\n"),
    ("- a: 1 # x\n  b: 2\n- a: 3 # z\n  b: 4\n", "pick([])", &["-P"], "[]\n"),
    ("- 1 # x\n- 1 # y\n- 2\n", "pick([0])", &[], "- 1 # x\n"),
    ("- 1 # x\n- 1 # y\n- 2\n", "pick([0])", &["-P"], "- 1 # x\n"),
    ("- 1 # x\n- 1 # y\n- 2\n", "pick([1])", &[], "- 1 # y\n"),
    ("- 1 # x\n- 1 # y\n- 2\n", "pick([1])", &["-P"], "- 1 # y\n"),
    ("- 1 # x\n- 1 # y\n- 2\n", "pick([2,0])", &[], "- 2\n- 1 # x\n"),
    ("- 1 # x\n- 1 # y\n- 2\n", "pick([2,0])", &["-P"], "- 2\n- 1 # x\n"),
    ("- 1 # x\n- 1 # y\n- 2\n", "pick([2])", &[], "- 2\n"),
    ("- 1 # x\n- 1 # y\n- 2\n", "pick([2])", &["-P"], "- 2\n"),
    ("[a, b, c] # t\n", "pick([0])", &[], "[a] # t\n"),
    ("[a, b, c] # t\n", "pick([0])", &["-P"], "- a\n"),
    ("[a, b, c] # t\n", "pick([2,0])", &[], "[c, a] # t\n"),
    ("[a, b, c] # t\n", "pick([2,0])", &["-P"], "- c\n- a\n"),
    ("[a, b, c] # t\n", "pick([1,1])", &[], "[b, b] # t\n"),
    ("[a, b, c] # t\n", "pick([1,1])", &["-P"], "- b\n- b\n"),
    ("---\n# h\na: 1\nb:\n  x: 1\n", "pick([\"a\"])", &[], "---\n# h\na: 1\n"),
    ("---\n# h\na: 1\nb:\n  x: 1\n", "pick([\"a\"])", &["-P"], "---\n# h\na: 1\n"),
];

#[test]
fn test_5003_a_pick_keeps_the_presentation_of_what_it_keeps() -> Result<()> {
    for (input, filter, args, want) in ROWS {
        let (out, code) = run_yq_stdin(filter, input, args)?;
        assert_eq!(code, 0, "{filter} {args:?} on {input:?}");
        assert_eq!(&out, want, "{filter} {args:?} on {input:?}");
    }
    Ok(())
}

/// The value is the same with or without the presentation.
#[test]
fn test_5003_b_json_output_is_unchanged() -> Result<()> {
    let (out, code) = run_yq_stdin("pick([\"a\"])", "a: x # ac\nb: 2\n", &["-o=json", "-I=0"])?;
    assert_eq!((out.as_str(), code), ("{\"a\":\"x\"}\n", 0));
    Ok(())
}

/// Shapes where this tool still differs from yq, pinned so a change is seen
/// (`docs/compliance/yq/limitations.md`): a key listed twice is printed once where yq prints the
/// entry twice, and a mapping cannot hold it; with a `---` marker over a comment line, a later
/// entry's pick loses the comment that yq keeps under the marker.
#[test]
fn test_5003_c_recorded_differences() -> Result<()> {
    for (input, filter, want) in [
        ("a: 1 # x\nb: 2\n", "pick([\"a\", \"a\"])", "a: 1 # x\n"),
        (
            "---\n# h\na: 1\nb:\n  x: 1\n",
            "pick([\"b\"])",
            "---\nb:\n  x: 1\n",
        ),
        (
            "x: &x {k: 1}\nc:\n  <<: *x\n  z: 2\n",
            "pick([\"c\"])",
            "c:\n  k: 1\n  z: 2\n",
        ),
    ] {
        let (out, code) = run_yq_stdin(filter, input, &[])?;
        assert_eq!(code, 0, "{filter} on {input:?}");
        assert_eq!(out, want, "{filter} on {input:?}");
    }
    Ok(())
}

/// Past 1 MiB the replay is not made (it converts the whole document, where the plain route reads
/// only what is picked), so `pick` prints the result without the kept nodes' comments.
#[test]
fn test_5003_d_a_document_past_the_replay_bound_prints_plain() -> Result<()> {
    let padding = "pad: xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx\n"
        .repeat(13_000);
    let input = format!("a: x # ac\n{padding}");
    assert!(input.len() > 1 << 20);
    let (out, code) = run_yq_stdin("pick([\"a\"])", &input, &[])?;
    assert_eq!((out.as_str(), code), ("a: x\n", 0));
    let small = format!("a: x # ac\n{}", &padding[..padding.len() / 13_000 * 10]);
    let (out, code) = run_yq_stdin("pick([\"a\"])", &small, &[])?;
    assert_eq!((out.as_str(), code), ("a: x # ac\n", 0));
    Ok(())
}
