//! #7175: under `--eval-all`, yq's `del`, `sort_keys`, `map_values`, `sort` and the metadata assignments
//! (`. style = "flow"`, `. anchor = "x"`) write into the nodes they are given and return that context, so each
//! node keeps its `EvaluateTogether` flag: `del(.k) | [.]` is one array of every document, a binary operator
//! after them crosses the whole list, and `[.k]` reads read-only. `succinctly yq` dropped the flag after each
//! (the assignments kept it since #7137), so those read one document at a time.
//!
//! Every row was captured from yq v4.53.3 over the three files below (the third holds two documents) under
//! `--eval-all -o=json -I=0`. What still differs, listed in `limitations.md`: `. tag = "..."` under `--eval-all`
//! (refused, #2796), `pick`/`omit`/`explode` (their arguments cross the list), and a `//` after the write (#7173).

use anyhow::Result;
use tempfile::TempDir;

use crate::run_yq_files;

/// (filter, yq's stdout).
#[rustfmt::skip]
const ROWS: &[(&str, &str)] = &[
    ("del(.k) | [.]", "[{\"a\":1,\"l\":[1,2]},{\"c\":3,\"l\":[3],\"m\":{\"p\":1}},{\"e\":5,\"l\":[],\"m\":{\"q\":2}},{\"x\":9}]\n"),
    ("del(.l[0]) | [.] | length", "4\n"),
    ("del(.k, .l) | [.]", "[{\"a\":1},{\"c\":3,\"m\":{\"p\":1}},{\"e\":5,\"m\":{\"q\":2}},{\"x\":9}]\n"),
    ("sort_keys(.) | [.]", "[{\"a\":1,\"k\":10,\"l\":[1,2]},{\"c\":3,\"k\":20,\"l\":[3],\"m\":{\"p\":1}},{\"e\":5,\"k\":30,\"l\":[],\"m\":{\"q\":2}},{\"x\":9}]\n"),
    ("sort_keys(.) | [.k]", "[10,20,30]\n"),
    ("sort_keys(..) | [.]", "[{\"a\":1,\"k\":10,\"l\":[1,2]},{\"c\":3,\"k\":20,\"l\":[3],\"m\":{\"p\":1}},{\"e\":5,\"k\":30,\"l\":[],\"m\":{\"q\":2}},{\"x\":9}]\n"),
    ("map_values(.) | [.]", "[{\"a\":1,\"k\":10,\"l\":[1,2]},{\"c\":3,\"k\":20,\"l\":[3],\"m\":{\"p\":1}},{\"e\":5,\"k\":30,\"l\":[],\"m\":{\"q\":2}},{\"x\":9}]\n"),
    ("sort | [.]", "[{\"l\":[1,2],\"a\":1,\"k\":10},{\"l\":[3],\"m\":{\"p\":1},\"c\":3,\"k\":20},{\"l\":[],\"m\":{\"q\":2},\"e\":5,\"k\":30},{\"x\":9}]\n"),
    (". style=\"flow\" | [.]", "[{\"a\":1,\"k\":10,\"l\":[1,2]},{\"c\":3,\"k\":20,\"l\":[3],\"m\":{\"p\":1}},{\"e\":5,\"k\":30,\"l\":[],\"m\":{\"q\":2}},{\"x\":9}]\n"),
    (". anchor=\"x\" | [.]", "[{\"a\":1,\"k\":10,\"l\":[1,2]},{\"c\":3,\"k\":20,\"l\":[3],\"m\":{\"p\":1}},{\"e\":5,\"k\":30,\"l\":[],\"m\":{\"q\":2}},{\"x\":9}]\n"),
    (".k style=\"flow\" | [.]", "[{\"a\":1,\"k\":10,\"l\":[1,2]},{\"c\":3,\"k\":20,\"l\":[3],\"m\":{\"p\":1}},{\"e\":5,\"k\":30,\"l\":[],\"m\":{\"q\":2}},{\"x\":9,\"k\":null}]\n"),
    ("del(.k) | . + {\"z\": 1} | length", "3\n3\n3\n3\n4\n4\n4\n4\n4\n4\n4\n4\n2\n2\n2\n2\n"),
    ("del(.k) | [.k]", "[]\n"),
    ("del(.zz) | [.]", "[{\"a\":1,\"k\":10,\"l\":[1,2]},{\"c\":3,\"k\":20,\"l\":[3],\"m\":{\"p\":1}},{\"e\":5,\"k\":30,\"l\":[],\"m\":{\"q\":2}},{\"x\":9}]\n"),
    ("del(.. | select(. == 1)) | [.]", "[{\"k\":10,\"l\":[2]},{\"c\":3,\"k\":20,\"l\":[3],\"m\":{}},{\"e\":5,\"k\":30,\"l\":[],\"m\":{\"q\":2}},{\"x\":9}]\n"),
    ("(del(.k) | [.]) | length", "4\n"),
    ("del(.k) | .z = fi", "{\"a\":1,\"l\":[1,2],\"z\":2}\n{\"c\":3,\"l\":[3],\"m\":{\"p\":1},\"z\":2}\n{\"e\":5,\"l\":[],\"m\":{\"q\":2},\"z\":2}\n{\"x\":9,\"z\":2}\n"),
    (".z = 1 | del(.k) | [.]", "[{\"a\":1,\"l\":[1,2],\"z\":1},{\"c\":3,\"l\":[3],\"m\":{\"p\":1},\"z\":1},{\"e\":5,\"l\":[],\"m\":{\"q\":2},\"z\":1},{\"x\":9,\"z\":1}]\n"),
    ("del(.k) | .z = 1 | [.]", "[{\"a\":1,\"l\":[1,2],\"z\":1},{\"c\":3,\"l\":[3],\"m\":{\"p\":1},\"z\":1},{\"e\":5,\"l\":[],\"m\":{\"q\":2},\"z\":1},{\"x\":9,\"z\":1}]\n"),
    ("sort_keys(.) | .z = fi", "{\"a\":1,\"k\":10,\"l\":[1,2],\"z\":2}\n{\"c\":3,\"k\":20,\"l\":[3],\"m\":{\"p\":1},\"z\":2}\n{\"e\":5,\"k\":30,\"l\":[],\"m\":{\"q\":2},\"z\":2}\n{\"x\":9,\"z\":2}\n"),
    ("del(.k) | fi", "0\n1\n2\n2\n"),
    ("del(.k) | di", "0\n0\n0\n1\n"),
    ("sort_keys(.) | fi", "0\n1\n2\n2\n"),
    ("del(.k) | select(fi == 1)", "{\"c\":3,\"l\":[3],\"m\":{\"p\":1}}\n"),
    ("del(.k) | select(di == 1)", "{\"x\":9}\n"),
    ("del(.k) | length", "2\n3\n3\n1\n"),
    ("del(.k) | keys", "[\"a\",\"l\"]\n[\"c\",\"l\",\"m\"]\n[\"e\",\"l\",\"m\"]\n[\"x\"]\n"),
    ("del(.k) | .", "{\"a\":1,\"l\":[1,2]}\n{\"c\":3,\"l\":[3],\"m\":{\"p\":1}}\n{\"e\":5,\"l\":[],\"m\":{\"q\":2}}\n{\"x\":9}\n"),
];

#[test]
fn eval_all_writers_that_return_their_input_keep_the_whole_list_7175() -> Result<()> {
    let dir = TempDir::new()?;
    let texts = [
        "a: 1\nk: 10\nl: [1, 2]\n",
        "c: 3\nk: 20\nl: [3]\nm: {p: 1}\n",
        "e: 5\nk: 30\nl: []\nm: {q: 2}\n---\nx: 9\n",
    ];
    let paths: Vec<_> = texts
        .iter()
        .enumerate()
        .map(|(i, text)| {
            let path = dir.path().join(format!("f{}.yaml", i + 1));
            std::fs::write(&path, text).map(|()| path)
        })
        .collect::<std::io::Result<_>>()?;
    let refs: Vec<&std::path::Path> = paths.iter().map(std::path::PathBuf::as_path).collect();
    for &(filter, expected) in ROWS {
        let (stdout, stderr, status) = run_yq_files(filter, &refs, &["ea", "-o=json", "-I=0"])?;
        assert_eq!(
            (stdout.as_str(), stderr.as_str(), status),
            (expected, "", 0),
            "{filter}"
        );
    }
    Ok(())
}
