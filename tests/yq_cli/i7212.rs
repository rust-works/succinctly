//! #7212: a document read with `-p xml` and written as XML has every comment on the node yq's decoder gave
//! it. The YAML text the read hands the run attributes a comment by where it sits, which put a comment after
//! a closing tag on another element (`<x/><!-- c -->` read back as the next one's), after a nested block on
//! its last child, and a trailing blank line in a foot comment (`<!-- c\n-->`). Every row was captured from yq
//! v4.53.3 (stdout, the first stderr line, the exit status).
use anyhow::Result;

use crate::run_yq_stdin_with_stderr;

/// (filter, stdin, yq's stdout, yq's first stderr line, yq's exit status).
type Row = (&'static str, &'static str, &'static str, &'static str, i32);

#[rustfmt::skip]
const ROWS: &[Row] = &[
    (".", "<r><x/><!-- c --><y>2</y></r>", "<r>\n  <x></x><!-- c -->\n  <y>2</y>\n</r>\n", "", 0),
    (".", "<r><x><k>1</k></x><!-- c --></r><!-- d -->", "<r>\n  <x>\n    <k>1</k>\n  </x><!-- c -->\n</r><!-- d -->\n", "", 0),
    (".", "<a>1</a><!-- d -->", "<a>1</a><!-- d -->\n", "", 0),
    (".", "<r><x>1</x><!-- c --><x>2</x><!-- e --></r>", "<r>\n  <x>1</x><!-- c -->\n  <x>2</x><!-- e -->\n</r>\n", "", 0),
    (".r.y = 3", "<r><x><k>1</k></x><!-- c --><y>2</y></r>", "<r>\n  <x>\n    <k>1</k>\n  </x><!-- c -->\n  <y>3</y>\n</r>\n", "", 0),
    ("del(.r.y)", "<r><x/><!-- c --><y>2</y></r>", "<r>\n  <x></x><!-- c -->\n</r>\n", "", 0),
    (".", "<?xml version=\"1.0\"?>\n<plist><dict><k>a</k><s>1</s><!-- mid --></dict><x/></plist>\n", "<?xml version=\"1.0\"?>\n<plist>\n  <dict>\n    <k>a</k>\n    <s>1</s><!-- mid -->\n  </dict>\n  <x></x>\n</plist>\n", "", 0),
    ("del(.r.x[0])", "<r><x>1</x><!-- c --><x>2</x><!-- e --></r>", "<r>\n  <x>2</x><!-- e -->\n</r>\n", "", 0),
    (".r.x += [\"3\"]", "<r><x>1</x><!-- c --><x>2</x><!-- e --></r>", "<r>\n  <x>1</x><!-- c -->\n  <x>2</x><!-- e -->\n  <x>3</x>\n</r>\n", "", 0),
];

#[test]
fn xml_written_back_as_xml_keeps_each_comment_on_the_decoders_node_7212() -> Result<()> {
    for &(filter, input, stdout, stderr, code) in ROWS {
        let (out, err, status) =
            run_yq_stdin_with_stderr(filter, input, &["-p", "xml", "-o", "xml"])?;
        let row = format!("-p xml -o xml {filter:?} on {input:?}");
        assert_eq!(out, stdout, "stdout of {row}");
        assert_eq!(err.lines().next().unwrap_or(""), stderr, "stderr of {row}");
        assert_eq!(status, code, "exit status of {row}");
    }
    Ok(())
}
