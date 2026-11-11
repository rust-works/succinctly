//! #6900: a `*` with one merge flag (`*+`, `*d`, `*?`, `*n`) keeps the presentation of the document
//! it merges into, as the plain `*` does and as yq does for all of them: the head and line comments,
//! the flow style of a collection, the left node's own style around an array the merge replaces
//! (plain `*` and `*?`), and the `&anchor` a merge that rebuilds a container drops (`*+` keeps it). A combination of flags (`*+d`) keeps the route that carries none, so its row
//! is the control that is still different from yq. Every other row is captured from yq v4.53.3.

use anyhow::Result;

use crate::run_yq_stdin;

/// `(document, filter, output)`; the last row is `*+d`, whose output yq and this build differ on.
const ROWS: &[(&str, &str, &str)] = &[
    ("# main\nname: web\nports:\n  - a: 1 # one\n  - a: 2\nenv: [x]\n", ". *+ {\"ports\": [{\"a\": 3}]}", "# main\nname: web\nports:\n  - a: 1 # one\n  - a: 2\n  - a: 3\nenv: [x]\n"),
    ("# main\nname: web\nports:\n  - a: 1 # one\n  - a: 2\nenv: [x]\n", ". *d {\"ports\": [{\"a\": 9}]}", "# main\nname: web\nports:\n  - a: 9 # one\n  - a: 2\nenv: [x]\n"),
    ("# main\nname: web\nports:\n  - a: 1 # one\n  - a: 2\nenv: [x]\n", ". *+ {\"env\": [\"y\"]}", "# main\nname: web\nports:\n  - a: 1 # one\n  - a: 2\nenv: [x, y]\n"),
    ("# main\nname: web\nports:\n  - a: 1 # one\n  - a: 2\nenv: [x]\n", ". *? {\"name\": \"z\"}", "# main\nname: z\nports:\n  - a: 1 # one\n  - a: 2\nenv: [x]\n"),
    ("# main\nname: web\nports:\n  - a: 1 # one\n  - a: 2\nenv: [x]\n", ". *n {\"name\": \"z\", \"new\": 1}", "# main\nname: web\nports:\n  - a: 1 # one\n  - a: 2\nenv: [x]\nnew: 1\n"),
    ("# main\nname: web\nports:\n  - a: 1 # one\n  - a: 2\nenv: [x]\n", ".ports *+ [{\"a\": 3}]", "- a: 1 # one\n- a: 2\n- a: 3\n"),
    ("# main\nname: web\nports:\n  - a: 1 # one\n  - a: 2\nenv: [x]\n", ". *d {\"ports\": [{\"a\": 9}, {\"b\": 1}, {\"a\": 5}]}", "# main\nname: web\nports:\n  - a: 9 # one\n  - a: 2\n    b: 1\n  - a: 5\nenv: [x]\n"),
    ("# main\nname: web\nports:\n  - a: 1 # one\n  - a: 2\nenv: [x]\n", ". *n {\"ports\": [{\"a\": 8}]}", "# main\nname: web\nports:\n  - a: 1 # one\n  - a: 2\nenv: [x]\n"),
    ("# main\nname: web\nports:\n  - a: 1 # one\n  - a: 2\nenv: [x]\n", ". *? {\"ports\": [{\"a\": 8}]}", "# main\nname: web\nports:\n  - a: 8\nenv: [x]\n"),
    ("# main\nname: web\nports:\n  - a: 1 # one\n  - a: 2\nenv: [x]\n", ". *? {\"env\": [\"y\", \"z\"]}", "# main\nname: web\nports:\n  - a: 1 # one\n  - a: 2\nenv: [y, z]\n"),
    ("# main\nname: web\nports:\n  - a: 1 # one\n  - a: 2\nenv: [x]\n", ". *n {\"env\": [\"y\"]}", "# main\nname: web\nports:\n  - a: 1 # one\n  - a: 2\nenv: [x]\n"),
    ("# main\nname: web\nports:\n  - a: 1 # one\n  - a: 2\nenv: [x]\n", ". *d {\"env\": [\"y\", \"z\"]}", "# main\nname: web\nports:\n  - a: 1 # one\n  - a: 2\nenv: [y, z]\n"),
    ("# main\nname: web\nports:\n  - a: 1 # one\n  - a: 2\nenv: [x]\n", ". *n {\"name\": null}", "# main\nname: web\nports:\n  - a: 1 # one\n  - a: 2\nenv: [x]\n"),
    ("# main\nname: web\nports:\n  - a: 1 # one\n  - a: 2\nenv: [x]\n", ". * {\"env\": [\"y\", \"z\"]}", "# main\nname: web\nports:\n  - a: 1 # one\n  - a: 2\nenv: [y, z]\n"),
    ("# main\nname: web\nports:\n  - a: 1 # one\n  - a: 2\nenv: [x]\n", ". * {\"name\": \"z\"}", "# main\nname: z\nports:\n  - a: 1 # one\n  - a: 2\nenv: [x]\n"),
    ("# top\nname: web # nm\nlabels: {app: a, tier: t} # lb\nports:\n  - {a: 1} # one\n  - a: 2\nenv: [x, y] # ev\nnest:\n  list: [1, 2, 3]\n  deep:\n    k: v # kc\n", ". * {\"nest\": {\"list\": [9]}}", "# top\nname: web # nm\nlabels: {app: a, tier: t} # lb\nports:\n  - {a: 1} # one\n  - a: 2\nenv: [x, y] # ev\nnest:\n  list: [9]\n  deep:\n    k: v # kc\n"),
    ("# top\nname: web # nm\nlabels: {app: a, tier: t} # lb\nports:\n  - {a: 1} # one\n  - a: 2\nenv: [x, y] # ev\nnest:\n  list: [1, 2, 3]\n  deep:\n    k: v # kc\n", ". *+ {\"nest\": {\"list\": [4]}}", "# top\nname: web # nm\nlabels: {app: a, tier: t} # lb\nports:\n  - {a: 1} # one\n  - a: 2\nenv: [x, y] # ev\nnest:\n  list: [1, 2, 3, 4]\n  deep:\n    k: v # kc\n"),
    ("# top\nname: web # nm\nlabels: {app: a, tier: t} # lb\nports:\n  - {a: 1} # one\n  - a: 2\nenv: [x, y] # ev\nnest:\n  list: [1, 2, 3]\n  deep:\n    k: v # kc\n", ". *+ {\"labels\": {\"x\": 1}}", "# top\nname: web # nm\nlabels: {app: a, tier: t, x: 1} # lb\nports:\n  - {a: 1} # one\n  - a: 2\nenv: [x, y] # ev\nnest:\n  list: [1, 2, 3]\n  deep:\n    k: v # kc\n"),
    ("# top\nname: web # nm\nlabels: {app: a, tier: t} # lb\nports:\n  - {a: 1} # one\n  - a: 2\nenv: [x, y] # ev\nnest:\n  list: [1, 2, 3]\n  deep:\n    k: v # kc\n", ". *d {\"nest\": {\"list\": [9, 8, 7, 6]}}", "# top\nname: web # nm\nlabels: {app: a, tier: t} # lb\nports:\n  - {a: 1} # one\n  - a: 2\nenv: [x, y] # ev\nnest:\n  list: [9, 8, 7, 6]\n  deep:\n    k: v # kc\n"),
    ("# top\nname: web # nm\nlabels: {app: a, tier: t} # lb\nports:\n  - {a: 1} # one\n  - a: 2\nenv: [x, y] # ev\nnest:\n  list: [1, 2, 3]\n  deep:\n    k: v # kc\n", ". *n {\"nest\": {\"list\": [9], \"deep\": {\"k\": \"w\", \"j\": 1}}}", "# top\nname: web # nm\nlabels: {app: a, tier: t} # lb\nports:\n  - {a: 1} # one\n  - a: 2\nenv: [x, y] # ev\nnest:\n  list: [1, 2, 3]\n  deep:\n    k: v # kc\n    j: 1\n"),
    ("# top\nname: web # nm\nlabels: {app: a, tier: t} # lb\nports:\n  - {a: 1} # one\n  - a: 2\nenv: [x, y] # ev\nnest:\n  list: [1, 2, 3]\n  deep:\n    k: v # kc\n", ". *? {\"nest\": {\"deep\": {\"k\": \"w\", \"j\": 1}}}", "# top\nname: web # nm\nlabels: {app: a, tier: t} # lb\nports:\n  - {a: 1} # one\n  - a: 2\nenv: [x, y] # ev\nnest:\n  list: [1, 2, 3]\n  deep:\n    k: w # kc\n"),
    ("# top\nname: web # nm\nlabels: {app: a, tier: t} # lb\nports:\n  - {a: 1} # one\n  - a: 2\nenv: [x, y] # ev\nnest:\n  list: [1, 2, 3]\n  deep:\n    k: v # kc\n", ".nest *+ {\"list\": [4]}", "list: [1, 2, 3, 4]\ndeep:\n  k: v # kc\n"),
    ("# top\nname: web # nm\nlabels: {app: a, tier: t} # lb\nports:\n  - {a: 1} # one\n  - a: 2\nenv: [x, y] # ev\nnest:\n  list: [1, 2, 3]\n  deep:\n    k: v # kc\n", ".nest.list *+ [4]", "[1, 2, 3, 4]\n"),
    ("# top\nname: web # nm\nlabels: {app: a, tier: t} # lb\nports:\n  - {a: 1} # one\n  - a: 2\nenv: [x, y] # ev\nnest:\n  list: [1, 2, 3]\n  deep:\n    k: v # kc\n", ".nest.list *d [9]", "[9, 2, 3]\n"),
    ("# top\nname: web # nm\nlabels: {app: a, tier: t} # lb\nports:\n  - {a: 1} # one\n  - a: 2\nenv: [x, y] # ev\nnest:\n  list: [1, 2, 3]\n  deep:\n    k: v # kc\n", ".labels *n {\"app\": \"z\", \"n\": 1}", "{app: a, tier: t, n: 1} # lb\n"),
    ("# top\nname: web # nm\nlabels: {app: a, tier: t} # lb\nports:\n  - {a: 1} # one\n  - a: 2\nenv: [x, y] # ev\nnest:\n  list: [1, 2, 3]\n  deep:\n    k: v # kc\n", ".labels *? {\"app\": \"z\", \"n\": 1}", "{app: z, tier: t} # lb\n"),
    ("# top\na: ~ # c1\nb: &x [2, 1] # cb\nc: {k: 1}\nd: [1, 2, 3]\ne: []\nf: &y {p: 1}\n", ". *n {\"a\": 1}", "# top\na: 1 # c1\nb: &x [2, 1] # cb\nc: {k: 1}\nd: [1, 2, 3]\ne: []\nf: &y {p: 1}\n"),
    ("# top\na: ~ # c1\nb: &x [2, 1] # cb\nc: {k: 1}\nd: [1, 2, 3]\ne: []\nf: &y {p: 1}\n", ". *+ {\"b\": [3]}", "# top\na: ~ # c1\nb: &x [2, 1, 3] # cb\nc: {k: 1}\nd: [1, 2, 3]\ne: []\nf: &y {p: 1}\n"),
    ("# top\na: ~ # c1\nb: &x [2, 1] # cb\nc: {k: 1}\nd: [1, 2, 3]\ne: []\nf: &y {p: 1}\n", ". *+ {\"b\": []}", "# top\na: ~ # c1\nb: &x [2, 1] # cb\nc: {k: 1}\nd: [1, 2, 3]\ne: []\nf: &y {p: 1}\n"),
    ("# top\na: ~ # c1\nb: &x [2, 1] # cb\nc: {k: 1}\nd: [1, 2, 3]\ne: []\nf: &y {p: 1}\n", ". *d {\"b\": [9, 8, 7]}", "# top\na: ~ # c1\nb: [9, 8, 7] # cb\nc: {k: 1}\nd: [1, 2, 3]\ne: []\nf: &y {p: 1}\n"),
    ("# top\na: ~ # c1\nb: &x [2, 1] # cb\nc: {k: 1}\nd: [1, 2, 3]\ne: []\nf: &y {p: 1}\n", ". *d {\"b\": []}", "# top\na: ~ # c1\nb: [2, 1] # cb\nc: {k: 1}\nd: [1, 2, 3]\ne: []\nf: &y {p: 1}\n"),
    ("# top\na: ~ # c1\nb: &x [2, 1] # cb\nc: {k: 1}\nd: [1, 2, 3]\ne: []\nf: &y {p: 1}\n", ". *? {\"b\": [7]}", "# top\na: ~ # c1\nb: [7] # cb\nc: {k: 1}\nd: [1, 2, 3]\ne: []\nf: &y {p: 1}\n"),
    ("# top\na: ~ # c1\nb: &x [2, 1] # cb\nc: {k: 1}\nd: [1, 2, 3]\ne: []\nf: &y {p: 1}\n", ". * {\"b\": [7]}", "# top\na: ~ # c1\nb: [7] # cb\nc: {k: 1}\nd: [1, 2, 3]\ne: []\nf: &y {p: 1}\n"),
    ("# top\na: ~ # c1\nb: &x [2, 1] # cb\nc: {k: 1}\nd: [1, 2, 3]\ne: []\nf: &y {p: 1}\n", ". *+ {\"e\": [1]}", "# top\na: ~ # c1\nb: &x [2, 1] # cb\nc: {k: 1}\nd: [1, 2, 3]\ne:\n  - 1\nf: &y {p: 1}\n"),
    ("# top\na: ~ # c1\nb: &x [2, 1] # cb\nc: {k: 1}\nd: [1, 2, 3]\ne: []\nf: &y {p: 1}\n", ". *d {\"e\": [1]}", "# top\na: ~ # c1\nb: &x [2, 1] # cb\nc: {k: 1}\nd: [1, 2, 3]\ne:\n  - 1\nf: &y {p: 1}\n"),
    ("# top\na: ~ # c1\nb: &x [2, 1] # cb\nc: {k: 1}\nd: [1, 2, 3]\ne: []\nf: &y {p: 1}\n", ". * {\"e\": [1]}", "# top\na: ~ # c1\nb: &x [2, 1] # cb\nc: {k: 1}\nd: [1, 2, 3]\ne:\n  - 1\nf: &y {p: 1}\n"),
    ("# top\na: ~ # c1\nb: &x [2, 1] # cb\nc: {k: 1}\nd: [1, 2, 3]\ne: []\nf: &y {p: 1}\n", ". *n {\"e\": [1]}", "# top\na: ~ # c1\nb: &x [2, 1] # cb\nc: {k: 1}\nd: [1, 2, 3]\ne: []\nf: &y {p: 1}\n"),
    ("# top\na: ~ # c1\nb: &x [2, 1] # cb\nc: {k: 1}\nd: [1, 2, 3]\ne: []\nf: &y {p: 1}\n", ". *+ {\"f\": {\"q\": 2}}", "# top\na: ~ # c1\nb: &x [2, 1] # cb\nc: {k: 1}\nd: [1, 2, 3]\ne: []\nf: {p: 1, q: 2}\n"),
    ("# top\na: ~ # c1\nb: &x [2, 1] # cb\nc: {k: 1}\nd: [1, 2, 3]\ne: []\nf: &y {p: 1}\n", ". * {\"f\": {\"q\": 2}}", "# top\na: ~ # c1\nb: &x [2, 1] # cb\nc: {k: 1}\nd: [1, 2, 3]\ne: []\nf: {p: 1, q: 2}\n"),
    ("# top\na: ~ # c1\nb: &x [2, 1] # cb\nc: {k: 1}\nd: [1, 2, 3]\ne: []\nf: &y {p: 1}\n", ". *n {\"c\": {\"k\": 5, \"z\": 1}}", "# top\na: ~ # c1\nb: &x [2, 1] # cb\nc: {k: 1, z: 1}\nd: [1, 2, 3]\ne: []\nf: &y {p: 1}\n"),
    ("# top\na: ~ # c1\nb: &x [2, 1] # cb\nc: {k: 1}\nd: [1, 2, 3]\ne: []\nf: &y {p: 1}\n", ". * {\"a\": 5}", "# top\na: 5 # c1\nb: &x [2, 1] # cb\nc: {k: 1}\nd: [1, 2, 3]\ne: []\nf: &y {p: 1}\n"),
    ("# top\nname: web # nm\nlabels: {app: a, tier: t} # lb\nports:\n  - {a: 1} # one\n  - a: 2\nenv: [x, y] # ev\nnest:\n  list: [1, 2, 3]\n  deep:\n    k: v # kc\n", ". *+d {\"labels\": {\"x\": 1}}", "# top\nname: web # nm\nlabels: {app: a, tier: t, x: 1} # lb\nports:\n  - {a: 1} # one\n  - a: 2\nenv: [x, y] # ev\nnest:\n  list: [1, 2, 3]\n  deep:\n    k: v # kc\n"),
];

#[test]
fn test_a_flagged_merge_keeps_the_left_documents_presentation_6900() -> Result<()> {
    let (flagged_both, rows) = ROWS.split_last().expect("rows");
    for (doc, filter, want) in rows {
        let (stdout, code) = run_yq_stdin(filter, doc, &[])?;
        assert_eq!(code, 0, "exit code of {filter:?}");
        assert_eq!(&stdout, want, "{filter:?} over {doc:?}");
    }
    // `*+d` is the same data as yq's and still loses the presentation (recorded).
    let (doc, filter, yq) = flagged_both;
    let (stdout, code) = run_yq_stdin(filter, doc, &[])?;
    assert_eq!(code, 0, "exit code of {filter:?}");
    assert_ne!(
        &stdout, yq,
        "{filter:?} now matches yq: drop it from the limitations"
    );
    assert!(!stdout.contains("# top"), "{stdout}");
    Ok(())
}
