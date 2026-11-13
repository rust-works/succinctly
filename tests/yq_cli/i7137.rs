//! #7137: under `--eval-all` real yq's `assignUpdateOperator` evaluates a plain or compound assignment's right
//! side **once**, against the whole list of documents, and applies its last output to every node: `yq ea '.k = fi'
//! a1 a2 a3` is `k: 2` in all three (the last file's index), `.k = .k` is the last document's own `k` everywhere,
//! `.k += fi` adds the last file's index to each `k`, and `.k = [fi]` is the list `[0, 1, 2]` in each. `|=`, which
//! runs its filter once per node, and a right side that is a literal are per document. `succinctly yq` evaluated
//! the right side once per document.
//!
//! Every row was captured from yq v4.53.3 over the three files below, as (files, filter, stdout): `plain` is
//! `a: 1` / `c: 3` / `e: 5`, `keyed` the same with `k: 10` / `k: 20` / `k: 30` added. What still differs, listed
//! in `limitations.md`: a right side that holds the context list and is written into the same list (`.k = [.]`,
//! where yq's array holds the live nodes, so a later document shows the earlier ones' new `k`), and the separator
//! between results (#7019: `.k = 1 | length` is `2\n2\n2` in yq).

use anyhow::Result;
use tempfile::TempDir;

use crate::run_yq_files;

/// (files, filter, yq's stdout), with `@DIR@` for the directory the files are in (`filename`).
#[rustfmt::skip]
const ROWS: &[(&str, &str, &str)] = &[
    ("plain", ".k = fi", "a: 1\nk: 2\n---\nc: 3\nk: 2\n---\ne: 5\nk: 2\n"),
    ("keyed", ".k = fi", "a: 1\nk: 2\n---\nc: 3\nk: 2\n---\ne: 5\nk: 2\n"),
    ("plain", ".k = filename", "a: 1\nk: @DIR@/f3.yaml\n---\nc: 3\nk: @DIR@/f3.yaml\n---\ne: 5\nk: @DIR@/f3.yaml\n"),
    ("keyed", ".k = filename", "a: 1\nk: @DIR@/f3.yaml\n---\nc: 3\nk: @DIR@/f3.yaml\n---\ne: 5\nk: @DIR@/f3.yaml\n"),
    ("plain", ".k = file_index", "a: 1\nk: 2\n---\nc: 3\nk: 2\n---\ne: 5\nk: 2\n"),
    ("keyed", ".k = file_index", "a: 1\nk: 2\n---\nc: 3\nk: 2\n---\ne: 5\nk: 2\n"),
    ("plain", ".k = (fi)", "a: 1\nk: 2\n---\nc: 3\nk: 2\n---\ne: 5\nk: 2\n"),
    ("keyed", ".k = (fi)", "a: 1\nk: 2\n---\nc: 3\nk: 2\n---\ne: 5\nk: 2\n"),
    ("plain", ".k += fi", "a: 1\nk: 2\n---\nc: 3\nk: 2\n---\ne: 5\nk: 2\n"),
    ("keyed", ".k += fi", "a: 1\nk: 12\n---\nc: 3\nk: 22\n---\ne: 5\nk: 32\n"),
    ("plain", ".k = fi + 1", "a: 1\nk: 3\n---\nc: 3\nk: 3\n---\ne: 5\nk: 3\n"),
    ("keyed", ".k = fi + 1", "a: 1\nk: 3\n---\nc: 3\nk: 3\n---\ne: 5\nk: 3\n"),
    ("plain", ".k = [fi]", "a: 1\nk:\n  - 0\n  - 1\n  - 2\n---\nc: 3\nk:\n  - 0\n  - 1\n  - 2\n---\ne: 5\nk:\n  - 0\n  - 1\n  - 2\n"),
    ("keyed", ".k = [fi]", "a: 1\nk:\n  - 0\n  - 1\n  - 2\n---\nc: 3\nk:\n  - 0\n  - 1\n  - 2\n---\ne: 5\nk:\n  - 0\n  - 1\n  - 2\n"),
    ("plain", ".k = {\"x\": fi}", "a: 1\nk:\n  x: 2\n---\nc: 3\nk:\n  x: 2\n---\ne: 5\nk:\n  x: 2\n"),
    ("keyed", ".k = {\"x\": fi}", "a: 1\nk:\n  x: 2\n---\nc: 3\nk:\n  x: 2\n---\ne: 5\nk:\n  x: 2\n"),
    ("plain", ".k = (. | fi)", "a: 1\nk: 2\n---\nc: 3\nk: 2\n---\ne: 5\nk: 2\n"),
    ("keyed", ".k = (. | fi)", "a: 1\nk: 2\n---\nc: 3\nk: 2\n---\ne: 5\nk: 2\n"),
    ("plain", ".k = ([.] | .[0] | fi)", "a: 1\nk: 0\n---\nc: 3\nk: 0\n---\ne: 5\nk: 0\n"),
    ("keyed", ".k = ([.] | .[0] | fi)", "a: 1\nk: 0\n---\nc: 3\nk: 0\n---\ne: 5\nk: 0\n"),
    ("plain", ".k = (fi | . + 1)", "a: 1\nk: 3\n---\nc: 3\nk: 3\n---\ne: 5\nk: 3\n"),
    ("keyed", ".k = (fi | . + 1)", "a: 1\nk: 3\n---\nc: 3\nk: 3\n---\ne: 5\nk: 3\n"),
    ("plain", ".k = (select(fi == 0) | fi)", "a: 1\nk: 0\n---\nc: 3\nk: 0\n---\ne: 5\nk: 0\n"),
    ("keyed", ".k = (select(fi == 0) | fi)", "a: 1\nk: 0\n---\nc: 3\nk: 0\n---\ne: 5\nk: 0\n"),
    ("plain", ".k = (select(fi == 2) | fi)", "a: 1\nk: 2\n---\nc: 3\nk: 2\n---\ne: 5\nk: 2\n"),
    ("keyed", ".k = (select(fi == 2) | fi)", "a: 1\nk: 2\n---\nc: 3\nk: 2\n---\ne: 5\nk: 2\n"),
    ("plain", ".k = (.a | fi)", "a: 1\nk: 0\n---\nc: 3\nk: 0\n---\ne: 5\nk: 0\n"),
    ("keyed", ".k = (.a | fi)", "a: 1\nk: 0\n---\nc: 3\nk: 0\n---\ne: 5\nk: 0\n"),
    ("plain", ".k = (.c | fi)", "a: 1\nk: 1\n---\nc: 3\nk: 1\n---\ne: 5\nk: 1\n"),
    ("keyed", ".k = (.c | fi)", "a: 1\nk: 1\n---\nc: 3\nk: 1\n---\ne: 5\nk: 1\n"),
    ("plain", ".k = (.e | fi)", "a: 1\nk: 2\n---\nc: 3\nk: 2\n---\ne: 5\nk: 2\n"),
    ("keyed", ".k = (.e | fi)", "a: 1\nk: 2\n---\nc: 3\nk: 2\n---\ne: 5\nk: 2\n"),
    ("plain", ".[ \"k\" ] = fi", "a: 1\nk: 2\n---\nc: 3\nk: 2\n---\ne: 5\nk: 2\n"),
    ("keyed", ".[ \"k\" ] = fi", "a: 1\nk: 2\n---\nc: 3\nk: 2\n---\ne: 5\nk: 2\n"),
    ("plain", ".a = fi", "a: 2\n---\nc: 3\na: 2\n---\ne: 5\na: 2\n"),
    ("keyed", ".a = fi", "a: 2\nk: 10\n---\nc: 3\nk: 20\na: 2\n---\ne: 5\nk: 30\na: 2\n"),
    ("plain", ".k = filename + \"x\"", "a: 1\nk: @DIR@/f3.yamlx\n---\nc: 3\nk: @DIR@/f3.yamlx\n---\ne: 5\nk: @DIR@/f3.yamlx\n"),
    ("keyed", ".k = filename + \"x\"", "a: 1\nk: @DIR@/f3.yamlx\n---\nc: 3\nk: @DIR@/f3.yamlx\n---\ne: 5\nk: @DIR@/f3.yamlx\n"),
    ("plain", ".k = ([fi, di])", "a: 1\nk:\n  - 0\n  - 0\n  - 1\n  - 0\n  - 2\n  - 0\n---\nc: 3\nk:\n  - 0\n  - 0\n  - 1\n  - 0\n  - 2\n  - 0\n---\ne: 5\nk:\n  - 0\n  - 0\n  - 1\n  - 0\n  - 2\n  - 0\n"),
    ("keyed", ".k = ([fi, di])", "a: 1\nk:\n  - 0\n  - 0\n  - 1\n  - 0\n  - 2\n  - 0\n---\nc: 3\nk:\n  - 0\n  - 0\n  - 1\n  - 0\n  - 2\n  - 0\n---\ne: 5\nk:\n  - 0\n  - 0\n  - 1\n  - 0\n  - 2\n  - 0\n"),
    ("plain", ".k = (fi * 10)", "a: 1\nk: 20\n---\nc: 3\nk: 20\n---\ne: 5\nk: 20\n"),
    ("keyed", ".k = (fi * 10)", "a: 1\nk: 20\n---\nc: 3\nk: 20\n---\ne: 5\nk: 20\n"),
    ("plain", ".k = (fi == 1)", "a: 1\nk: false\n---\nc: 3\nk: false\n---\ne: 5\nk: false\n"),
    ("keyed", ".k = (fi == 1)", "a: 1\nk: false\n---\nc: 3\nk: false\n---\ne: 5\nk: false\n"),
    ("plain", ".k = (fi | tostring)", "a: 1\nk: \"2\"\n---\nc: 3\nk: \"2\"\n---\ne: 5\nk: \"2\"\n"),
    ("keyed", ".k = (fi | tostring)", "a: 1\nk: \"2\"\n---\nc: 3\nk: \"2\"\n---\ne: 5\nk: \"2\"\n"),
    ("plain", ".k = (fi // 9)", "a: 1\nk: 2\n---\nc: 3\nk: 2\n---\ne: 5\nk: 2\n"),
    ("keyed", ".k = (fi // 9)", "a: 1\nk: 2\n---\nc: 3\nk: 2\n---\ne: 5\nk: 2\n"),
    ("plain", ".k = (fi | select(. > 0))", "a: 1\nk: 2\n---\nc: 3\nk: 2\n---\ne: 5\nk: 2\n"),
    ("keyed", ".k = (fi | select(. > 0))", "a: 1\nk: 2\n---\nc: 3\nk: 2\n---\ne: 5\nk: 2\n"),
    ("plain", ".k = (. as $d | fi)", "a: 1\nk: 2\n---\nc: 3\nk: 2\n---\ne: 5\nk: 2\n"),
    ("keyed", ".k = (. as $d | fi)", "a: 1\nk: 2\n---\nc: 3\nk: 2\n---\ne: 5\nk: 2\n"),
    ("plain", ".k = (fi as $f | $f)", "a: 1\nk: 2\n---\nc: 3\nk: 2\n---\ne: 5\nk: 2\n"),
    ("keyed", ".k = (fi as $f | $f)", "a: 1\nk: 2\n---\nc: 3\nk: 2\n---\ne: 5\nk: 2\n"),
    ("plain", ".k = (.. | select(tag == \"!!int\") | fi)", "a: 1\nk: 2\n---\nc: 3\nk: 2\n---\ne: 5\nk: 2\n"),
    ("keyed", ".k = (.. | select(tag == \"!!int\") | fi)", "a: 1\nk: 2\n---\nc: 3\nk: 2\n---\ne: 5\nk: 2\n"),
    ("plain", ".k = ([fi] | length)", "a: 1\nk: 3\n---\nc: 3\nk: 3\n---\ne: 5\nk: 3\n"),
    ("keyed", ".k = ([fi] | length)", "a: 1\nk: 3\n---\nc: 3\nk: 3\n---\ne: 5\nk: 3\n"),
    ("plain", ".k -= fi", "a: 1\nk: 2\n---\nc: 3\nk: 2\n---\ne: 5\nk: 2\n"),
    ("keyed", ".k -= fi", "a: 1\nk: 8\n---\nc: 3\nk: 18\n---\ne: 5\nk: 28\n"),
    ("keyed", ".k *= fi", "a: 1\nk: 20\n---\nc: 3\nk: 40\n---\ne: 5\nk: 60\n"),
    ("plain", ".k = .k + fi", "a: 1\nk: 2\n---\nc: 3\nk: 2\n---\ne: 5\nk: 2\n"),
    ("keyed", ".k = .k + fi", "a: 1\nk: 32\n---\nc: 3\nk: 32\n---\ne: 5\nk: 32\n"),
    ("keyed", ".k = (.k | . + 1)", "a: 1\nk: 31\n---\nc: 3\nk: 31\n---\ne: 5\nk: 31\n"),
    ("keyed", ".k += .k", "a: 1\nk: 40\n---\nc: 3\nk: 50\n---\ne: 5\nk: 60\n"),
    ("keyed", ".k = .k", "a: 1\nk: 30\n---\nc: 3\nk: 30\n---\ne: 5\nk: 30\n"),
    ("keyed", ".k = ([.k] | .[0])", "a: 1\nk: 10\n---\nc: 3\nk: 10\n---\ne: 5\nk: 10\n"),
    ("plain", ".k = ([.k] | length)", "a: 1\nk: 3\n---\nc: 3\nk: 3\n---\ne: 5\nk: 3\n"),
    ("keyed", ".k = ([.k] | length)", "a: 1\nk: 3\n---\nc: 3\nk: 3\n---\ne: 5\nk: 3\n"),
    ("plain", ".z = (.k, fi)", "a: 1\nz: 2\n---\nc: 3\nz: 2\n---\ne: 5\nz: 2\n"),
    ("keyed", ".z = (.k, fi)", "a: 1\nk: 10\nz: 2\n---\nc: 3\nk: 20\nz: 2\n---\ne: 5\nk: 30\nz: 2\n"),
    ("keyed", ".k = (.k + 1)", "a: 1\nk: 31\n---\nc: 3\nk: 31\n---\ne: 5\nk: 31\n"),
    ("plain", ".k = keys", "a: 1\nk:\n  - e\n  - k\n---\nc: 3\nk:\n  - e\n  - k\n---\ne: 5\nk:\n  - e\n  - k\n"),
    ("keyed", ".k = keys", "a: 1\nk:\n  - e\n  - k\n---\nc: 3\nk:\n  - e\n  - k\n---\ne: 5\nk:\n  - e\n  - k\n"),
    ("plain", ".k = (keys | .[0])", "a: 1\nk: e\n---\nc: 3\nk: e\n---\ne: 5\nk: e\n"),
    ("keyed", ".k = (keys | .[0])", "a: 1\nk: e\n---\nc: 3\nk: e\n---\ne: 5\nk: e\n"),
    ("plain", ".k = .a", "a: 1\nk: 1\n---\nc: 3\nk: 1\n---\ne: 5\nk: 1\n"),
    ("keyed", ".k = .a", "a: 1\nk: 1\n---\nc: 3\nk: 1\n---\ne: 5\nk: 1\n"),
    ("plain", ".k = .c", "a: 1\nk: 3\n---\nc: 3\nk: 3\n---\ne: 5\nk: 3\n"),
    ("keyed", ".k = .c", "a: 1\nk: 3\n---\nc: 3\nk: 3\n---\ne: 5\nk: 3\n"),
    ("plain", ".k = ([.] | length)", "a: 1\nk: 3\n---\nc: 3\nk: 3\n---\ne: 5\nk: 3\n"),
    ("keyed", ".k = ([.] | length)", "a: 1\nk: 3\n---\nc: 3\nk: 3\n---\ne: 5\nk: 3\n"),
    ("plain", ".k = ([.[]] | length)", "a: 1\nk: 6\n---\nc: 3\nk: 6\n---\ne: 5\nk: 6\n"),
    ("keyed", ".k = ([.[]] | length)", "a: 1\nk: 6\n---\nc: 3\nk: 6\n---\ne: 5\nk: 6\n"),
    ("plain", ".k = 1 | [.]", "- a: 1\n  k: 1\n- c: 3\n  k: 1\n- e: 5\n  k: 1\n"),
    ("keyed", ".k = 1 | [.]", "- a: 1\n  k: 1\n- c: 3\n  k: 1\n- e: 5\n  k: 1\n"),
    ("plain", ".k |= 1 | [.]", "- a: 1\n  k: 1\n- c: 3\n  k: 1\n- e: 5\n  k: 1\n"),
    ("keyed", ".k |= 1 | [.]", "- a: 1\n  k: 1\n- c: 3\n  k: 1\n- e: 5\n  k: 1\n"),
    ("plain", "(.k = fi) | [.]", "- a: 1\n  k: 2\n- c: 3\n  k: 2\n- e: 5\n  k: 2\n"),
    ("keyed", "(.k = fi) | [.]", "- a: 1\n  k: 2\n- c: 3\n  k: 2\n- e: 5\n  k: 2\n"),
    ("plain", ".k += 1 | [.] | length", "3\n"),
    ("keyed", ".k += 1 | [.] | length", "3\n"),
    ("plain", ".k = 1 | .m = fi", "a: 1\nk: 1\nm: 2\n---\nc: 3\nk: 1\nm: 2\n---\ne: 5\nk: 1\nm: 2\n"),
    ("keyed", ".k = 1 | .m = fi", "a: 1\nk: 1\nm: 2\n---\nc: 3\nk: 1\nm: 2\n---\ne: 5\nk: 1\nm: 2\n"),
    ("plain", ".k = fi | .m = fi", "a: 1\nk: 2\nm: 2\n---\nc: 3\nk: 2\nm: 2\n---\ne: 5\nk: 2\nm: 2\n"),
    ("keyed", ".k = fi | .m = fi", "a: 1\nk: 2\nm: 2\n---\nc: 3\nk: 2\nm: 2\n---\ne: 5\nk: 2\nm: 2\n"),
    ("plain", ".k = fi | .k = .k + 1", "a: 1\nk: 3\n---\nc: 3\nk: 3\n---\ne: 5\nk: 3\n"),
    ("keyed", ".k = fi | .k = .k + 1", "a: 1\nk: 3\n---\nc: 3\nk: 3\n---\ne: 5\nk: 3\n"),
    ("keyed", ".a = (.k + 1)", "a: 31\nk: 10\n---\nc: 3\nk: 20\na: 31\n---\ne: 5\nk: 30\na: 31\n"),
    ("keyed", ".k = (.k // 5)", "a: 1\nk: 30\n---\nc: 3\nk: 30\n---\ne: 5\nk: 30\n"),
    ("keyed", ".k = (.k | select(. > 15))", "a: 1\nk: 30\n---\nc: 3\nk: 30\n---\ne: 5\nk: 30\n"),
    ("keyed", ".k = {\"x\": .k}", "a: 1\nk:\n  x: 30\n---\nc: 3\nk:\n  x: 30\n---\ne: 5\nk:\n  x: 30\n"),
    ("keyed", ".k = \"v\" + (.k | tostring)", "a: 1\nk: v30\n---\nc: 3\nk: v30\n---\ne: 5\nk: v30\n"),
    ("plain", "(.k, .z) = fi", "a: 1\nk: 2\nz: 2\n---\nc: 3\nk: 2\nz: 2\n---\ne: 5\nk: 2\nz: 2\n"),
    ("keyed", "(.k, .z) = fi", "a: 1\nk: 2\nz: 2\n---\nc: 3\nk: 2\nz: 2\n---\ne: 5\nk: 2\nz: 2\n"),
    ("plain", ".[] = fi", "a: 2\n---\nc: 2\n---\ne: 2\n"),
    ("keyed", ".[] = fi", "a: 2\nk: 2\n---\nc: 2\nk: 2\n---\ne: 2\nk: 2\n"),
    ("plain", ".k[0] = fi", "a: 1\nk:\n  - 2\n---\nc: 3\nk:\n  - 2\n---\ne: 5\nk:\n  - 2\n"),
    ("plain", ".n.m = fi", "a: 1\nn:\n  m: 2\n---\nc: 3\nn:\n  m: 2\n---\ne: 5\nn:\n  m: 2\n"),
    ("keyed", ".n.m = fi", "a: 1\nk: 10\nn:\n  m: 2\n---\nc: 3\nk: 20\nn:\n  m: 2\n---\ne: 5\nk: 30\nn:\n  m: 2\n"),
    ("keyed", ".k = (. as $d | .k)", "a: 1\nk: 30\n---\nc: 3\nk: 30\n---\ne: 5\nk: 30\n"),
    ("keyed", ".k = .k * 2", "a: 1\nk: 60\n---\nc: 3\nk: 60\n---\ne: 5\nk: 60\n"),
    ("plain", ".k = ([1, 2])", "a: 1\nk:\n  - 1\n  - 2\n  - 1\n  - 2\n  - 1\n  - 2\n---\nc: 3\nk:\n  - 1\n  - 2\n  - 1\n  - 2\n  - 1\n  - 2\n---\ne: 5\nk:\n  - 1\n  - 2\n  - 1\n  - 2\n  - 1\n  - 2\n"),
    ("keyed", ".k = ([1, 2])", "a: 1\nk:\n  - 1\n  - 2\n  - 1\n  - 2\n  - 1\n  - 2\n---\nc: 3\nk:\n  - 1\n  - 2\n  - 1\n  - 2\n  - 1\n  - 2\n---\ne: 5\nk:\n  - 1\n  - 2\n  - 1\n  - 2\n  - 1\n  - 2\n"),
    ("plain", ".k = [1, 2]", "a: 1\nk:\n  - 1\n  - 2\n  - 1\n  - 2\n  - 1\n  - 2\n---\nc: 3\nk:\n  - 1\n  - 2\n  - 1\n  - 2\n  - 1\n  - 2\n---\ne: 5\nk:\n  - 1\n  - 2\n  - 1\n  - 2\n  - 1\n  - 2\n"),
    ("keyed", ".k = [1, 2]", "a: 1\nk:\n  - 1\n  - 2\n  - 1\n  - 2\n  - 1\n  - 2\n---\nc: 3\nk:\n  - 1\n  - 2\n  - 1\n  - 2\n  - 1\n  - 2\n---\ne: 5\nk:\n  - 1\n  - 2\n  - 1\n  - 2\n  - 1\n  - 2\n"),
    ("plain", ".k = [1]", "a: 1\nk:\n  - 1\n  - 1\n  - 1\n---\nc: 3\nk:\n  - 1\n  - 1\n  - 1\n---\ne: 5\nk:\n  - 1\n  - 1\n  - 1\n"),
    ("keyed", ".k = [1]", "a: 1\nk:\n  - 1\n  - 1\n  - 1\n---\nc: 3\nk:\n  - 1\n  - 1\n  - 1\n---\ne: 5\nk:\n  - 1\n  - 1\n  - 1\n"),
    ("keyed", ".k = .k | .m = .k", "a: 1\nk: 30\nm: 30\n---\nc: 3\nk: 30\nm: 30\n---\ne: 5\nk: 30\nm: 30\n"),
    ("plain", ".k = di", "a: 1\nk: 0\n---\nc: 3\nk: 0\n---\ne: 5\nk: 0\n"),
    ("keyed", ".k = di", "a: 1\nk: 0\n---\nc: 3\nk: 0\n---\ne: 5\nk: 0\n"),
    ("plain", ".k = document_index", "a: 1\nk: 0\n---\nc: 3\nk: 0\n---\ne: 5\nk: 0\n"),
    ("keyed", ".k = document_index", "a: 1\nk: 0\n---\nc: 3\nk: 0\n---\ne: 5\nk: 0\n"),
    ("plain", ".k |= fi", "a: 1\nk: 0\n---\nc: 3\nk: 1\n---\ne: 5\nk: 2\n"),
    ("keyed", ".k |= fi", "a: 1\nk: 0\n---\nc: 3\nk: 1\n---\ne: 5\nk: 2\n"),
    ("plain", ".k |= di", "a: 1\nk: 0\n---\nc: 3\nk: 0\n---\ne: 5\nk: 0\n"),
    ("keyed", ".k |= di", "a: 1\nk: 0\n---\nc: 3\nk: 0\n---\ne: 5\nk: 0\n"),
    ("plain", ".k = (fi, di)", "a: 1\nk: 0\n---\nc: 3\nk: 0\n---\ne: 5\nk: 0\n"),
    ("keyed", ".k = (fi, di)", "a: 1\nk: 0\n---\nc: 3\nk: 0\n---\ne: 5\nk: 0\n"),
    ("plain", ".k += (fi, di)", "a: 1\nk: 0\n---\nc: 3\nk: 0\n---\ne: 5\nk: 0\n"),
    ("keyed", ".k += (fi, di)", "a: 1\nk: 10\n---\nc: 3\nk: 20\n---\ne: 5\nk: 30\n"),
];

fn files(dir: &TempDir, which: &str) -> Result<Vec<std::path::PathBuf>> {
    let texts: [&str; 3] = match which {
        "plain" => ["a: 1\n", "c: 3\n", "e: 5\n"],
        "keyed" => ["a: 1\nk: 10\n", "c: 3\nk: 20\n", "e: 5\nk: 30\n"],
        other => unreachable!("no file set {other}"),
    };
    texts
        .iter()
        .enumerate()
        .map(|(i, text)| {
            let path = dir.path().join(format!("f{}.yaml", i + 1));
            std::fs::write(&path, text)?;
            Ok(path)
        })
        .collect()
}

/// The right side of `=`, `+=`, `-=`, `*=` is read over the whole list, and the last output wins.
#[test]
fn eval_all_assignment_reads_its_right_side_over_the_list_7137() -> Result<()> {
    for &(which, filter, expected) in ROWS {
        let dir = TempDir::new()?;
        let paths = files(&dir, which)?;
        let refs: Vec<&std::path::Path> = paths.iter().map(std::path::PathBuf::as_path).collect();
        let (stdout, stderr, status) = run_yq_files(filter, &refs, &["ea"])?;
        assert_eq!(
            (stdout.as_str(), stderr.as_str(), status),
            (
                expected
                    .replace("@DIR@", &dir.path().display().to_string())
                    .as_str(),
                "",
                0
            ),
            "{filter} over {which}"
        );
    }
    Ok(())
}

/// Without `--eval-all` each file is its own evaluation, so the right side sees one document.
#[test]
fn plain_multi_file_assignment_stays_per_file_7137() -> Result<()> {
    let dir = TempDir::new()?;
    let paths = files(&dir, "keyed")?;
    let refs: Vec<&std::path::Path> = paths.iter().map(std::path::PathBuf::as_path).collect();
    for (filter, expected) in [
        (".k = fi", "a: 1\nk: 0\n---\nc: 3\nk: 1\n---\ne: 5\nk: 2\n"),
        (
            ".k += fi",
            "a: 1\nk: 10\n---\nc: 3\nk: 21\n---\ne: 5\nk: 32\n",
        ),
        (
            ".k = .k",
            "a: 1\nk: 10\n---\nc: 3\nk: 20\n---\ne: 5\nk: 30\n",
        ),
    ] {
        let (stdout, stderr, status) = run_yq_files(filter, &refs, &[])?;
        assert_eq!(
            (stdout.as_str(), stderr.as_str(), status),
            (expected, "", 0),
            "{filter}"
        );
    }
    Ok(())
}
