//! #7117: `succinctly yq -o xml` writes each node's head, line and foot comment as yq's XML encoder does,
//! where it wrote none: `<!-- c -->` before an element, inside it after its text, after its end tag, and the
//! first document's leading comment block ahead of the first element. Every row was captured from yq
//! v4.53.3 (stdout, the first stderr line, the exit status) and kept only when both tools agree; the
//! inputs are what a random sweep of comment positions over twelve document shapes found (`scripts/`
//! `yq-xml-input-sweep.py --only xml` is the same check over XML input). What still differs is in
//! `docs/compliance/yq/limitations.md` (`-o=xml`): a comment block that a blank line splits, a write.

use anyhow::Result;

use crate::run_yq_stdin_with_stderr;

/// (flags after `-o xml`, filter, input, yq's stdout, yq's first stderr line, yq's exit status).
type Row = (
    &'static [&'static str],
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    i32,
);

#[rustfmt::skip]
const ROWS: &[Row] = &[
    (&[], ".", "# a\nx: 1 # l\n---\n# b\ny: 2 # m\n", "<!-- a -->\n<x>1<!-- l --></x>\n<!-- b -->\n<y>2<!-- m --></y>\n", "", 0),
    (&["-N"], ".", "# a\nx: 1 # l\n---\n# b\ny: 2 # m\n", "<!-- a -->\n<x>1<!-- l --></x>\n<!-- b -->\n<y>2<!-- m --></y>\n", "", 0),
    (&[], ".", "---\n# a\nx: 1\n", "<!-- $yqDocSeparator$\na -->\n<x>1</x>\n", "", 0),
    (&[], ".", "a: &x 1 # l\nb: *x # m\n", "<a>1<!-- l --></a>\n<b>1<!-- m --></b>\n", "", 0),
    (&[], ".", "a: &x\n  c: 1 # l\nb: *x # m\n", "<a>\n  <c>1<!-- l --></c>\n</a>\n<b>\n  <c>1<!-- l --></c>\n</b>\n", "", 0),
    (&[], ".", "a: {b: 1, c: 2} # l\n# h\nd: [1, 2] # m\n", "<a>\n  <b>1</b>\n  <c>2</c>\n</a><!-- h -->\n<!-- m -->\n<d>1</d>\n<d>2</d>\n", "", 0),
    (&[], ".", "[1, 2] # c\n", "", "Error: cannot encode !!seq to XML - only maps can be encoded", 1),
    (&[], ".", "# lead\n\n5 # c\n", "<!-- lead\n-->\n5", "", 0),
    (&[], ".", "5 # c\n", "5", "", 0),
    (&["-I", "4"], ".", "a:\n  b: 1 # l\n  # f\nc: 2\n", "<a>\n    <b>1<!-- l --></b><!-- f -->\n</a>\n<c>2</c>\n", "", 0),
    (&["-I", "0"], ".", "a:\n  b: 1 # l\n", "<a><b>1<!-- l --></b></a>\n", "", 0),
    (&[], ".", "a: 1 # a --> b\n", "", "Error: xml: EncodeToken of Comment containing --> marker", 1),
    (&[], ".", "a: 1 # a -- b\n", "<a>1<!-- a -- b --></a>\n", "", 0),
    (&[], ".", "# a & b < c\nx: 1 # d & e\n", "<!-- a & b < c -->\n<x>1<!-- d & e --></x>\n", "", 0),
    (&[], ".", "# h\u{e9}llo \u{2713}\nx: 1 # \u{fc}n\u{ef}\n", "<!-- h\u{e9}llo \u{2713} -->\n<x>1<!-- \u{fc}n\u{ef} --></x>\n", "", 0),
    (&[], ".", "a:\n  # h0\n  - x: 1 # l0\n  # h1\n  - x: 2\n    # f1\n  # f2\n", "<a><!-- h0 -->\n  <x>1<!-- l0 --></x>\n</a>\n<a><!-- h1 -->\n  <x>2</x><!-- f1 -->\n</a><!-- f2 -->\n", "", 0),
    (&[], ".", "a:\n  # h\n  - - 1 # l\n    - 2\n  - 3\n", "<!-- h --><a>1<!-- l --></a>\n<a>2</a>\n<a>3</a>\n", "", 0),
    (&[], ".", "a:\n  +content: t # c\n  # h\n  +@id: 1 # at\n  b: 1\n", "<a id=\"1\"><!-- c -->t<!-- h -->\n  <b>1</b>\n</a>\n", "", 0),
    (&[], ".", "# top\n+p_xml: version=\"1.0\" # d\n# h\na: 1\n", "<?xml version=\"1.0\"?>\n<!-- top -->\n<!-- h -->\n<a>1</a>\n", "", 0),
    (&[], ".", "# h\n+directive: DOCTYPE html # d\na: 1\n", "<!-- h -->\n<!DOCTYPE html>\n<a>1</a>\n", "", 0),
    (&[], ".", "a: {} # l\n", "<a></a>\n", "", 0),
    (&[], ".", "a: # l1\n  b: # l2\n    c: 1\n", "<!-- l1 -->\n<a><!-- l2 -->\n  <b>\n    <c>1</c>\n  </b>\n</a>\n", "", 0),
    (&[], ".", "a: 1\n\n# h\n\nb: 2\n", "<a>1</a><!-- h\n-->\n\n<b>2</b>\n", "", 0),
    (&[], ".", "a: \"x # not a comment\" # real\n", "<a>x # not a comment<!-- real --></a>\n", "", 0),
    (&[], ".", "a: |\n  text # not\n  more\n# after\nb: 1\n", "<a>text # not\nmore\n</a><!-- after -->\n\n<b>1</b>\n", "", 0),
    (&[], ".", "\u{feff}# h\na: 1 # l\n", "<!-- h -->\n<a>1<!-- l --></a>\n", "", 0),
    (&[], ".", "#\na: 1 #\n#\n", "<!-- -->\n<a>1</a>\n", "", 0),
    (&[], ".", "a: 1 # l  \n# h   \nb: 2\n", "<a>1<!-- l  --></a><!-- h   -->\n\n<b>2</b>\n", "", 0),
    (&[], ".", "a: 1\t# l\n", "<a>1<!-- l --></a>\n", "", 0),
    (&[], ".", "#  spaced\n\na:\n  # f\n  b: 1 #  l  \n  c: 2\nd: 3\n", "<!-- spaced\n-->\n<a><!-- f -->\n  <b>1<!--  l  --></b>\n  <c>2</c>\n</a>\n<d>3</d>\n", "", 0),
    (&[], ".", "# a b c\na: 1 #  l  \nb: 2 #  l  \n", "<!-- a b c -->\n<a>1<!--  l  --></a>\n<b>2<!--  l  --></b>\n", "", 0),
    (&[], ".", "r:\n  s:\n    t: 1\n# f1\n# f2\n    # a b c\n    u: 2\n  v: 3\nw: 4\n  # f1\n  # f2\n", "<r>\n  <s>\n    <t>1</t><!-- f1\nf2\na b c -->\n    <u>2</u>\n  </s>\n  <v>3</v>\n</r>\n<w>4</w><!-- f1\nf2 -->\n", "", 0),
    (&[], ".", "a: #l\n  - x: 1\n    #h\n  - x: 2 # a # b\nb: 3 #  l  \n  # f1\n  # f2\n", "<!--l -->\n<a>\n  <x>1</x><!--h -->\n</a>\n<a>\n  <x>2<!-- a # b --></x>\n</a>\n<b>3<!--  l  --></b><!-- f1\nf2 -->\n", "", 0),
    (&[], ".", "a:\n  b: 1\n    # x # y\n  d: 2 # a # b\n  c: 3\n#f\n# f\n", "<a>\n  <b>1</b><!-- x # y -->\n  <d>2<!-- a # b --></d>\n  <c>3</c>\n</a><!-- f\nf -->\n", "", 0),
    (&[], ".", "# x # y\n\na: 1\n#  spaced\nb: 2\n", "<!-- x # y\n-->\n<a>1</a><!--  spaced -->\n\n<b>2</b>\n", "", 0),
    (&[], ".", "a: #  l  \n  - 1\n  - 2\nb: 3\n", "<!--  l  -->\n<a>1</a>\n<a>2</a>\n<b>3</b>\n", "", 0),
    (&[], ".", "a: 1 #  l  \n# h\nb: 2\nc: 3\n#f\n", "<a>1<!--  l  --></a><!-- h -->\n\n<b>2</b>\n<c>3</c><!--f -->\n", "", 0),
    (&[], ".", "a: 1 #  l  \nb: 2\n", "<a>1<!--  l  --></a>\n<b>2</b>\n", "", 0),
    (&[], ".", "a: # a # b\n  # f1\n  # f2\n  +@id: 7\n  +content: t\n# f1\n# f2\n  c: 1\n", "<!-- a # b -->\n<a id=\"7\"><!-- f1\nf2 -->t<!-- f1\nf2 -->\n  <c>1</c>\n</a>\n", "", 0),
    (&[], ".", "x: # a # b\n  +p_target: data\n  y: 1\n  # f\n", "<!-- a # b -->\n<x><?target data?>\n  <y>1</y><!-- f -->\n</x>\n", "", 0),
    (&[], ".", "a: # l\n#f\n  b: 1\n  d: 2\n  # f1\n  # f2\n  c: 3\n", "<!-- l -->\n<a><!--f -->\n  <b>1</b>\n  <d>2</d><!-- f1\nf2 -->\n  <c>3</c>\n</a>\n", "", 0),
    (&[], ".", "#h\na: #  l  \n# f1\n# f2\n    # h1\n    # h2\n  - - 1 # a # b\n    - 2\n  # f\n  - 3\n", "<!--h -->\n<!--  l  -->\n<!-- f1\nf2\nh1\nh2 --><a>1<!-- a # b --></a>\n<a>2</a>\n<a><!-- f -->3</a>\n", "", 0),
    (&[], ".", "## two\n\n  # x # y\na: 5\nb:\n  c: 6 # a # b\n", "<!-- # two\n\nx # y -->\n<a>5</a>\n<b>\n  <c>6<!-- a # b --></c>\n</b>\n", "", 0),
    (&[], ".", "a: # l\n  +@id: 7\n  +content: t\n    ## two\n  c: 1 #l\n", "<!-- l -->\n<a id=\"7\">t<!-- two -->\n  <c>1<!--l --></c>\n</a>\n", "", 0),
    (&[], ".", "a:\n  b: 1\n  d: 2\n  c: 3\n", "<a>\n  <b>1</b>\n  <d>2</d>\n  <c>3</c>\n</a>\n", "", 0),
    (&[], ".", "  #  spaced\nr:\n  s:\n    t: 1\n    u: 2\n# f\n  v: 3\nw: 4\n", "<!--  spaced -->\n<r>\n  <s>\n    <t>1</t>\n    <u>2</u><!-- f -->\n  </s>\n  <v>3</v>\n</r>\n<w>4</w>\n", "", 0),
    (&[], ".", "a:\n#f\n  b: 1\n  c: 2\n# h1\n# h2\nd: 3\n", "<a><!--f -->\n  <b>1</b>\n  <c>2</c>\n</a><!-- h1\nh2 -->\n\n<d>3</d>\n", "", 0),
    (&[], ".", "# h1\n# h2\na:\n  - - 1\n    - 2\n  - 3\n", "<!-- h1\nh2 -->\n<a>1</a>\n<a>2</a>\n<a>3</a>\n", "", 0),
    (&[], ".", "a:\n  b: 1\n  d: 2\n  c: 3\n  #f\n", "<a>\n  <b>1</b>\n  <d>2</d>\n  <c>3</c><!--f -->\n</a>\n", "", 0),
    (&[], ".", "#h\n---\n  #  spaced\na:\n    # x # y\n  - x: 1\n  #h\n  - x: 2 # a # b\nb: 3 #l\n", "<!-- h\n$yqDocSeparator$\n spaced -->\n<a><!-- x # y -->\n  <x>1</x>\n</a>\n<a><!--h -->\n  <x>2<!-- a # b --></x>\n</a>\n<b>3<!--l --></b>\n", "", 0),
    (&[], ".", "#h\n# x # y\na:\n  - 1\n  - 2\nb: 3\n  # f1\n  # f2\n", "<!-- h\nx # y -->\n<a>1</a>\n<a>2</a>\n<b>3</b><!-- f1\nf2 -->\n", "", 0),
    (&[], ".", "a:\n    #h\n  - 1 # a # b\n    #f\n  - 2\nb: 3\n", "<a><!--h -->1<!-- a # b --></a>\n<a><!--f -->2</a>\n<b>3</b>\n", "", 0),
    (&[], ".", "a:\n# x # y\n  - - 1 # a # b\n    - 2\n  - 3\n", "<!-- x # y --><a>1<!-- a # b --></a>\n<a>2</a>\n<a>3</a>\n", "", 0),
    (&[], ".", "#\na:\n  - x: 1\n  - x: 2\nb: 3\n", "<!-- -->\n<a>\n  <x>1</x>\n</a>\n<a>\n  <x>2</x>\n</a>\n<b>3</b>\n", "", 0),
    (&[], ".", "# a b c\na: 1\n+p_xml: version=\"1.0\" # l\nb: 2\n", "<?xml version=\"1.0\"?>\n<!-- a b c -->\n<a>1</a>\n<b>2</b>\n", "", 0),
    (&[], ".", "a:\n## two\n  - - 1 #  l  \n  # h1\n  # h2\n    - 2\n  - 3\n", "<!-- two --><a>1<!--  l  --></a>\n<a><!-- h1\nh2 -->2</a>\n<a>3</a>\n", "", 0),
    (&[], ".", "r: # a # b\n  # x # y\n  s: #l\n    t: 1\n    u: 2 #  l  \n    #f\n  v: 3 # a # b\nw: 4\n", "<!-- a # b -->\n<r><!-- x # yl -->\n  <s>\n    <t>1</t>\n    <u>2<!--  l  --></u><!--f -->\n  </s>\n  <v>3<!-- a # b --></v>\n</r>\n<w>4</w>\n", "", 0),
    (&[], ".a", "a:\n  b: 1 # l\n  # f\nc: 2\n", "<b>1<!-- l --></b><!-- f -->\n", "", 0),
    (&[], ".b", "# h\na: 1 # l\nb:\n  - x # lx\n", "", "Error: cannot encode !!seq to XML - only maps can be encoded", 1),
];

#[test]
fn xml_output_writes_comments_as_yq_does_7117() -> Result<()> {
    for &(flags, filter, input, stdout, stderr, code) in ROWS {
        let mut args = vec!["-o", "xml"];
        args.extend_from_slice(flags);
        let (out, err, status) = run_yq_stdin_with_stderr(filter, input, &args)?;
        let row = format!("-o xml {flags:?} {filter:?} on {input:?}");
        assert_eq!(out, stdout, "stdout of {row}");
        assert_eq!(err.lines().next().unwrap_or(""), stderr, "stderr of {row}");
        assert_eq!(status, code, "exit status of {row}");
    }
    Ok(())
}
