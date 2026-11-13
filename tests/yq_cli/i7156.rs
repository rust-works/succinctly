//! #7156: `succinctly yq -o xml` keeps the comments through a write and a rebuilt value: the filter runs
//! without its encoder and each result is encoded with the presentation tree it came with, as the YAML
//! printer prints it. Every row was captured from yq v4.53.3 (stdout, the first stderr line, the exit
//! status) and kept only when both tools agree; the documents are the comment positions of a random sweep
//! (`.zz = "n"`, `del(.a)`, `.a = "v"`, `sort_keys(.)`, `.a |= .`, a nested write, a deep `del`, a built
//! mapping around the document and a copy between keys). What still differs is in
//! `docs/compliance/yq/limitations.md` (`-o=xml`): `with_entries`, `to_entries | from_entries`, `. * {..}`.

use anyhow::Result;
use tempfile::TempDir;

use crate::{run_yq_files, run_yq_stdin_with_stderr};

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
    (&[], "del(.. | select(. == 2))", "a: 1\nb: 2\n", "<a>1</a>\n", "", 0),
    (&[], "sort_keys(.)", "r:\n  s: #l\n    t: 1\n    u: 2 # a # b\n# f\n## two\n  v: 3\nw: 4 # a # b\n", "<r><!--l -->\n  <s>\n    <t>1</t>\n    <u>2<!-- a # b --></u><!-- f\n# two -->\n  </s>\n  <v>3</v>\n</r>\n<w>4<!-- a # b --></w>\n", "", 0),
    (&[], "del(.. | select(. == 2))", "a: #l\n#h\n  - - 1\n    - 2\n  - 3 #  l  \n# f\n", "<!--l -->\n<!--h --><a>1</a>\n<a>3<!--  l  --></a><!-- f -->\n", "", 0),
    (&[], "del(.. | select(. == 2))", "#  spaced\nr: #  l  \n  s:\n    t: 1\n    u: 2\n  v: 3 # a # b\nw: 4\n", "<!--  spaced -->\n<!--  l  -->\n<r>\n  <s>\n    <t>1</t>\n  </s>\n  <v>3<!-- a # b --></v>\n</r>\n<w>4</w>\n", "", 0),
    (&[], ".zz = \"n\"", "r:\n#\n  s:\n# f\n    t: 1\n    # f\n    #h\n    u: 2 # a # b\n  v: 3\nw: 4\n#f\n", "<r>\n  <s><!-- f -->\n    <t>1</t><!-- f\nh -->\n    <u>2<!-- a # b --></u>\n  </s>\n  <v>3</v>\n</r>\n<w>4</w><!--f -->\n<zz>n</zz>\n", "", 0),
    (&[], ".a |= .", "  # x # y\na: # l\n    #h\n  - x: 1\n  - x: 2\n  #h\nb: 3\n  # f1\n  # f2\n", "<!-- x # y -->\n<!-- l -->\n<a><!--h -->\n  <x>1</x>\n</a>\n<a>\n  <x>2</x><!--h -->\n</a>\n<b>3</b><!-- f1\nf2 -->\n", "", 0),
    (&[], "sort_keys(.)", "a:\n# f1\n# f2\n  +@id: 7\n  +content: t\n  # f1\n  # f2\n  #\n  c: 1\n  # f1\n  # f2\n", "<a id=\"7\"><!-- f1\nf2 -->t<!-- f1\nf2\n-->\n  <c>1</c><!-- f1\nf2 -->\n</a>\n", "", 0),
    (&[], "{\"x\": .}", "#\na: # l\n    ## two\n  b: 1\n  # x # y\n  c: 2\nd: 3\n", "<x><!-- l -->\n  <a><!-- two -->\n    <b>1</b><!-- x # y -->\n    <c>2</c>\n  </a>\n  <d>3</d>\n</x>\n", "", 0),
    (&[], ".a = .b", "#h\na: 5 #l\nb:\n#  spaced\n  c: 6 #  l  \n  # f\n", "<!--h -->\n<a><!--  spaced -->\n  <c>6<!--  l  --></c><!-- f -->\n</a>\n<b><!--  spaced -->\n  <c>6<!--  l  --></c><!-- f -->\n</b>\n", "", 0),
    (&[], "del(.. | select(. == 2))", "# x # y\na: 1 #  l  \n+p_xml: version=\"1.0\"\nb: 2\n", "<?xml version=\"1.0\"?>\n<!-- x # y -->\n<a>1<!--  l  --></a>\n", "", 0),
    (&[], ".zz = \"n\"", "a:\n  #\n  b: 1 #l\n  c: 2\n# x # y\nd: 3\n  # f\n", "<a>\n  <b>1<!--l --></b>\n  <c>2</c>\n</a><!-- x # y -->\n\n<d>3</d><!-- f -->\n<zz>n</zz>\n", "", 0),
    (&[], ".a = .b", "# h\n  #\na:\n  b: 1\n  c: 2 #l\n  # h\nd: 3\n  # f1\n  # f2\n", "<!-- h\n-->\n<a>\n  <b>1</b>\n  <c>2<!--l --></c><!-- h -->\n</a>\n<d>3</d><!-- f1\nf2 -->\n", "", 0),
    (&[], "{\"x\": .}", "# a b c\na: 5\nb: # l\n  c: 6\n", "<x>\n  <a>5</a><!-- l -->\n  <b>\n    <c>6</c>\n  </b>\n</x>\n", "", 0),
    (&[], "sort_keys(.)", "#\na: 1\n#f\nb: 2\nc: 3\n", "<!-- -->\n<a>1</a><!--f -->\n\n<b>2</b>\n<c>3</c>\n", "", 0),
    (&[], ".a = \"v\"", "#h\na:\n    # x # y\n  b: 1 # l\n  c: 2\nd: 3\n", "<!--h -->\n<a>v</a>\n<d>3</d>\n", "", 0),
    (&[], ".a |= .", "# h1\n# h2\n---\n  #h\na:\n  +@id: 7 #l\n  # h1\n  # h2\n  +content: t\n  c: 1\n  #f\n  # f\n", "<!-- h1\nh2\n$yqDocSeparator$\nh -->\n<a id=\"7\"><!-- h1\nh2 -->t\n  <c>1</c><!-- f\nf -->\n</a>\n", "", 0),
    (&[], ".zz.yy = 1", "## two\na: 1\n# x # y\nb: 2 #l\nc: 3\n", "<!--# two -->\n<a>1</a><!-- x # y -->\n\n<b>2<!--l --></b>\n<c>3</c>\n<zz>\n  <yy>1</yy>\n</zz>\n", "", 0),
    (&[], ".zz = \"n\"", "  # h1\n  # h2\na: # l\n  b: 1 #l\n  #h\n  d: 2 # a # b\n# f1\n# f2\n#\n  c: 3\n# f\n", "<!-- h1\nh2 -->\n<!-- l -->\n<a>\n  <b>1<!--l --></b><!--h -->\n  <d>2<!-- a # b --></d><!-- f1\nf2\n-->\n  <c>3</c>\n</a><!-- f -->\n<zz>n</zz>\n", "", 0),
    (&[], ".a = \"v\"", "# x # y\na: 5\nb:\n# f1\n# f2\n  c: 6\n# f\n", "<!-- x # y -->\n<a>v</a>\n<b><!-- f1\nf2 -->\n  <c>6</c>\n</b><!-- f -->\n", "", 0),
    (&[], ".zz.yy = 1", "# a b c\na: # l\n#\n  +@id: 7\n  # f1\n  # f2\n  +content: t\n  c: 1\n", "<!-- a b c -->\n<!-- l -->\n<a id=\"7\"><!-- f1\nf2 -->t\n  <c>1</c>\n</a>\n<zz>\n  <yy>1</yy>\n</zz>\n", "", 0),
    (&[], "del(.. | select(. == 2))", "a:\n  b: 1\n  #  spaced\n  c: 2\nd: 3\n", "<a>\n  <b>1</b>\n</a>\n<d>3</d>\n", "", 0),
    (&[], ".a |= .", "# a b c\na: 1\nb: 2 #l\nc: 3 # l\n", "<!-- a b c -->\n<a>1</a>\n<b>2<!--l --></b>\n<c>3<!-- l --></c>\n", "", 0),
    (&[], "del(.a)", "## two\na:\n  b: 1\n  d: 2\n  c: 3\n  # f\n", "<!--# two -->\n\n", "", 0),
    (&[], "{\"x\": .}", "# a b c\n\na: 5\nb:\n    ## two\n  c: 6\n#f\n", "<x>\n  <a>5</a>\n  <b><!-- two -->\n    <c>6</c>\n  </b><!--f -->\n</x>\n", "", 0),
    (&[], ".a = \"v\"", "a: 5\n  # x # y\nb:\n  # f1\n  # f2\n  c: 6 #l\n#f\n", "<a>v</a><!-- x # y -->\n\n<b><!-- f1\nf2 -->\n  <c>6<!--l --></c>\n</b><!--f -->\n", "", 0),
    (&[], ".a = \"v\"", "a: 1\nb: 2\n", "<a>v</a>\n<b>2</b>\n", "", 0),
    (&[], "del(.a)", "#\na: 5\n# f1\n# f2\nb:\n  # h\n  c: 6\n  #f\n", "<!-- -->\n<!-- f1\nf2 -->\n<b><!-- h -->\n  <c>6</c><!--f -->\n</b>\n", "", 0),
    (&[], "{\"x\": .}", "# x # y\n# x # y\na:\n  - - 1\n    - 2 # a # b\n  - 3\n  # f1\n  # f2\n", "<x>\n  <a>1</a>\n  <a>2<!-- a # b --></a>\n  <a>3</a><!-- f1\nf2 -->\n</x>\n", "", 0),
    (&[], ".a = .b", "a:\n  b: 1\n  d: 2\n  c: 3\n", "<a>\n  <b>1</b>\n  <d>2</d>\n  <c>3</c>\n</a>\n", "", 0),
    (&[], ".a = .b", "#h\n\na: 1 #  l  \n  #f\n#\n+p_xml: version=\"1.0\"\nb: 2\n#f\n", "<?xml version=\"1.0\"?>\n<!-- h\n-->\n<a>2<!--  l  --></a><!-- f\n-->\n\n<b>2</b><!--f -->\n", "", 0),
    (&[], ".a = .b", "#\na: 1\n# x # y\nb: 2 # a # b\n", "<!-- -->\n<a>2<!-- a # b --></a><!-- x # y -->\n\n<b>2<!-- a # b --></b>\n", "", 0),
    (&[], "sort_keys(.)", "a:\n  - - 1\n  # x # y\n    - 2 # l\n  - 3 #l\n", "<a>1</a>\n<a><!-- x # y -->2<!-- l --></a>\n<a>3<!--l --></a>\n", "", 0),
    (&[], ".zz = \"n\"", "a: # l\n# f\n  - x: 1\n  - x: 2\nb: 3\n", "<!-- l -->\n<a><!-- f -->\n  <x>1</x>\n</a>\n<a>\n  <x>2</x>\n</a>\n<b>3</b>\n<zz>n</zz>\n", "", 0),
    (&[], ".zz = \"n\"", "# h1\n# h2\na:\n# f\n  ## two\n  - - 1\n    - 2 # l\n  - 3\n", "<!-- h1\nh2 -->\n<!-- f\n# two --><a>1</a>\n<a>2<!-- l --></a>\n<a>3</a>\n<zz>n</zz>\n", "", 0),
    (&[], ".zz.yy = 1", "# x # y\na:\n  - - 1\n    - 2\n  - 3\n", "<!-- x # y -->\n<a>1</a>\n<a>2</a>\n<a>3</a>\n<zz>\n  <yy>1</yy>\n</zz>\n", "", 0),
    (&[], "del(.a)", "a:\n  #\n  b: 1\n  d: 2 # l\n  c: 3 # l\n", "\n", "", 0),
    (&[], "{\"x\": .}", "# x # y\n\n---\na: 5 # l\n  # x # y\nb:\n  c: 6\n", "<x>\n  <a>5<!-- l --></a><!-- x # y -->\n  <b>\n    <c>6</c>\n  </b>\n</x>\n", "", 0),
    (&[], "del(.a)", "a:\n  - x: 1\n    #h\n  - x: 2\n  ## two\nb: 3\n", "<b>3</b>\n", "", 0),
    (&[], ".zz.yy = 1", "a:\n  - 1\n  - 2\nb: 3\n# f1\n# f2\n", "<a>1</a>\n<a>2</a>\n<b>3</b><!-- f1\nf2 -->\n<zz>\n  <yy>1</yy>\n</zz>\n", "", 0),
    (&[], ".a = \"v\"", "#h\na: # l\n  - 1\n    # h\n  - 2 #l\nb: 3\n# f1\n# f2\n", "<!--h -->\n<!-- l -->\n<a>v</a>\n<b>3</b><!-- f1\nf2 -->\n", "", 0),
    (&[], ".zz.yy = 1", "a:\n  - x: 1\n  - x: 2\nb: 3\n  # f1\n  # f2\n", "<a>\n  <x>1</x>\n</a>\n<a>\n  <x>2</x>\n</a>\n<b>3</b><!-- f1\nf2 -->\n<zz>\n  <yy>1</yy>\n</zz>\n", "", 0),
    (&[], ".a |= .", "#\n---\na:\n  - x: 1 #l\n  - x: 2\nb: 3\n", "<!--\n$yqDocSeparator$ -->\n<a>\n  <x>1<!--l --></x>\n</a>\n<a>\n  <x>2</x>\n</a>\n<b>3</b>\n", "", 0),
    (&[], "del(.a)", "a: #  l  \n  #\n  - 1\n  # a b c\n  - 2\nb: 3\n# f\n", "<b>3</b><!-- f -->\n", "", 0),
    (&[], "sort_keys(.)", "a: # a # b\n  - x: 1\n  - x: 2 #  l  \nb: 3\n", "<!-- a # b -->\n<a>\n  <x>1</x>\n</a>\n<a>\n  <x>2<!--  l  --></x>\n</a>\n<b>3</b>\n", "", 0),
    (&[], ".a |= .", "## two\na: # l\n  b: 1\n  d: 2\n# f1\n# f2\n  c: 3 # l\n", "<!--# two -->\n<!-- l -->\n<a>\n  <b>1</b>\n  <d>2</d><!-- f1\nf2 -->\n  <c>3<!-- l --></c>\n</a>\n", "", 0),
    (&["-I", "4"], ".a.b = 5", "a:\n  b: 1 # l\n  # f\nc: 2\n", "<a>\n    <b>5<!-- l --></b><!-- f -->\n</a>\n<c>2</c>\n", "", 0),
    (&["-N"], ".c = 3", "# top\n\na: 1 # l\n# f\n", "<!-- top\n-->\n<a>1<!-- l --></a>\n<c>3</c><!-- f -->\n", "", 0),
    (&[], ".zz = 1", "a: 1\n\n# h\n\nb: 2\n", "<a>1</a><!-- h\n-->\n\n<b>2</b>\n<zz>1</zz>\n", "", 0),
    (&[], "del(.a)", "a: 1\n\n# h\n\nb: 2\n", "<!-- h\n-->\n<b>2</b>\n", "", 0),
    (&[], "del(.a)", "## two\n---\na: 1\n# a b c\nb: 2 # a # b\n", "<!-- # two\n$yqDocSeparator$ -->\n<!-- a b c -->\n<b>2<!-- a # b --></b>\n", "", 0),
    (&[], "sort_keys(.)", "# h1\n\n# h2\n\nb: 2\na: 1 # l\n", "<!-- h1\n\nh2\n-->\n<a>1<!-- l --></a>\n<b>2</b>\n", "", 0),
    (&[], ".b = 3", "# lead\n\na: 1\n# f\nb: 2 # l\n# tail\n", "<!-- lead\n-->\n<a>1</a><!-- f -->\n\n<b>3<!-- l --></b><!-- tail -->\n", "", 0),
    (&[], "del(.a)", "# lead\n---\n# first\na: 1\nb: 2\n", "<!-- lead\n$yqDocSeparator$\nfirst -->\n<b>2</b>\n", "", 0),
    (&[], ".b = 3", "a: 1 # a --> b\nb: 2\n", "", "Error: xml: EncodeToken of Comment containing --> marker", 1),
];

#[test]
fn xml_output_keeps_the_comments_through_a_write_7156() -> Result<()> {
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

/// What `-i` writes back to an `.xml` file, as yq writes it: the file keeps its comments.
const POM: &str = "<?xml version=\"1.0\"?>\n<!-- top -->\n<project>\n  <!-- deps -->\n  <dependencies>\n    <dependency>\n      <artifactId>a</artifactId> <!-- first -->\n      <version>1</version>\n    </dependency>\n    <!-- logging -->\n    <dependency>\n      <artifactId>b</artifactId>\n    </dependency>\n  </dependencies>\n</project>\n";

#[rustfmt::skip]
const IN_PLACE: &[(&str, &str)] = &[
    (".project.dependencies.dependency[0].version = \"2\"",
     "<?xml version=\"1.0\"?>\n<!-- top -->\n<project><!-- deps -->\n  <dependencies>\n    <dependency>\n      <artifactId>a</artifactId><!-- first -->\n      <version>2</version>\n    </dependency>\n    <dependency>\n      <artifactId>b</artifactId>\n    </dependency><!-- logging -->\n  </dependencies>\n</project>\n"),
    ("del(.project.dependencies.dependency[0])",
     "<?xml version=\"1.0\"?>\n<!-- top -->\n<project><!-- deps -->\n  <dependencies>\n    <dependency>\n      <artifactId>b</artifactId>\n    </dependency><!-- logging -->\n  </dependencies>\n</project>\n"),
    (".project.name = \"x\"",
     "<?xml version=\"1.0\"?>\n<!-- top -->\n<project><!-- deps -->\n  <dependencies>\n    <dependency>\n      <artifactId>a</artifactId><!-- first -->\n      <version>1</version>\n    </dependency>\n    <dependency>\n      <artifactId>b</artifactId>\n    </dependency><!-- logging -->\n  </dependencies>\n  <name>x</name>\n</project>\n"),
];

#[test]
fn inplace_edit_of_an_xml_file_keeps_its_comments_7156() -> Result<()> {
    let dir = TempDir::new()?;
    let path = dir.path().join("pom.xml");
    for &(filter, expected) in IN_PLACE {
        std::fs::write(&path, POM)?;
        let (stdout, stderr, status) = run_yq_files(filter, &[path.as_path()], &["-i"])?;
        assert_eq!(
            (stdout.as_str(), stderr.as_str(), status),
            ("", "", 0),
            "{filter}"
        );
        assert_eq!(std::fs::read_to_string(&path)?, expected, "{filter}");
    }
    Ok(())
}
