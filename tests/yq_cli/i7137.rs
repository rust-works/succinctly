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
        "scalar" => ["5\n", "6\n", ""],
        "multi" => ["a: [1, 2]\nb: 7\n", "a: [3]\nb: 8\n", ""],
        "nested" => ["a:\n  x: 1\nl: [1]\n", "a:\n  x: 2\nl: [2, 3]\n", "b: 3\n"],
        other => unreachable!("no file set {other}"),
    };
    texts
        .iter()
        .filter(|text| !text.is_empty())
        .enumerate()
        .map(|(i, text)| {
            let path = dir.path().join(format!("f{}.yaml", i + 1));
            std::fs::write(&path, text)?;
            Ok(path)
        })
        .collect()
}

/// (files, filter, yq's stdout under `-o=json -I=0`): targets that are no-ops, several at once, and nested.
#[rustfmt::skip]
const JSON_ROWS: &[(&str, &str, &str)] = &[
    ("scalar", ".a = error(\"x\")", "5\n6\n"),
    ("scalar", ".a.b = fi", "5\n6\n"),
    ("scalar", ".[0] = fi", "5\n6\n"),
    ("multi", ".[0] = fi", "{\"a\":[1,2],\"b\":7,\"0\":1}\n{\"a\":[3],\"b\":8,\"0\":1}\n"),
    ("multi", ".a[] = fi", "{\"a\":[1,1],\"b\":7}\n{\"a\":[1],\"b\":8}\n"),
    ("multi", "(.a, .b) = fi", "{\"a\":1,\"b\":1}\n{\"a\":1,\"b\":1}\n"),
    ("multi", ".a[] += fi", "{\"a\":[2,3],\"b\":7}\n{\"a\":[4],\"b\":8}\n"),
    ("multi", ".a[0] = .b", "{\"a\":[8,2],\"b\":7}\n{\"a\":[8],\"b\":8}\n"),
    ("multi", ".a[] = .b", "{\"a\":[8,8],\"b\":7}\n{\"a\":[8],\"b\":8}\n"),
    ("multi", ".[] = fi", "{\"a\":1,\"b\":1}\n{\"a\":1,\"b\":1}\n"),
    ("multi", "(.b | select(. > 7)) = fi", "{\"a\":[1,2],\"b\":7}\n{\"a\":[3],\"b\":1}\n"),
    ("multi", ".c.d.e = .b", "{\"a\":[1,2],\"b\":7,\"c\":{\"d\":{\"e\":8}}}\n{\"a\":[3],\"b\":8,\"c\":{\"d\":{\"e\":8}}}\n"),
    ("multi", ".a += .a", "{\"a\":[1,2,3],\"b\":7}\n{\"a\":[3,3],\"b\":8}\n"),
    ("multi", ".a = .a + .a", "{\"a\":[3,3],\"b\":7}\n{\"a\":[3,3],\"b\":8}\n"),
    ("multi", ".a = .b", "{\"a\":8,\"b\":7}\n{\"a\":8,\"b\":8}\n"),
    ("multi", ".b = .a", "{\"a\":[1,2],\"b\":[3]}\n{\"a\":[3],\"b\":[3]}\n"),
    ("multi", ".b += .b", "{\"a\":[1,2],\"b\":15}\n{\"a\":[3],\"b\":16}\n"),
    ("multi", ".b -= fi", "{\"a\":[1,2],\"b\":6}\n{\"a\":[3],\"b\":7}\n"),
    ("multi", ".b *= .b", "{\"a\":[1,2],\"b\":56}\n{\"a\":[3],\"b\":64}\n"),
    ("multi", ".b = (.b | length)", "{\"a\":[1,2],\"b\":1}\n{\"a\":[3],\"b\":1}\n"),
    ("multi", ".b = ([.b] | length)", "{\"a\":[1,2],\"b\":2}\n{\"a\":[3],\"b\":2}\n"),
    ("multi", ".b = ([.a[]] | length)", "{\"a\":[1,2],\"b\":3}\n{\"a\":[3],\"b\":3}\n"),
    ("multi", ".c = keys", "{\"a\":[1,2],\"b\":7,\"c\":[\"a\",\"b\",\"c\"]}\n{\"a\":[3],\"b\":8,\"c\":[\"a\",\"b\",\"c\"]}\n"),
    ("multi", ".a[1] = fi", "{\"a\":[1,1],\"b\":7}\n{\"a\":[3,1],\"b\":8}\n"),
    ("multi", ".a[5] = fi", "{\"a\":[1,2,null,null,null,1],\"b\":7}\n{\"a\":[3,null,null,null,null,1],\"b\":8}\n"),
    ("multi", ".b = (.a | length)", "{\"a\":[1,2],\"b\":1}\n{\"a\":[3],\"b\":1}\n"),
    ("multi", ".b = (.a[0] // 9)", "{\"a\":[1,2],\"b\":3}\n{\"a\":[3],\"b\":3}\n"),
    ("multi", ".b = .a[0]", "{\"a\":[1,2],\"b\":3}\n{\"a\":[3],\"b\":3}\n"),
    ("nested", ".a.x = fi", "{\"a\":{\"x\":2},\"l\":[1]}\n{\"a\":{\"x\":2},\"l\":[2,3]}\n{\"b\":3,\"a\":{\"x\":2}}\n"),
    ("nested", ".a = .a", "{\"a\":null,\"l\":[1]}\n{\"a\":null,\"l\":[2,3]}\n{\"b\":3,\"a\":null}\n"),
    ("nested", ".a.x = .a.x", "{\"a\":{\"x\":null},\"l\":[1]}\n{\"a\":{\"x\":null},\"l\":[2,3]}\n{\"b\":3,\"a\":{\"x\":null}}\n"),
    ("nested", ".l = .l", "{\"a\":{\"x\":1},\"l\":null}\n{\"a\":{\"x\":2},\"l\":null}\n{\"b\":3,\"l\":null}\n"),
    ("nested", ".l += .l", "{\"a\":{\"x\":1},\"l\":[1]}\n{\"a\":{\"x\":2},\"l\":[2,3]}\n{\"b\":3,\"l\":null}\n"),
    ("nested", ".l[] = fi", "{\"a\":{\"x\":1},\"l\":[2]}\n{\"a\":{\"x\":2},\"l\":[2,2]}\n{\"b\":3,\"l\":[]}\n"),
    ("nested", ".a.y = .a.x", "{\"a\":{\"x\":1,\"y\":2},\"l\":[1]}\n{\"a\":{\"x\":2,\"y\":2},\"l\":[2,3]}\n{\"b\":3,\"a\":{\"y\":2}}\n"),
    ("nested", ".n = (.a | length)", "{\"a\":{\"x\":1},\"l\":[1],\"n\":1}\n{\"a\":{\"x\":2},\"l\":[2,3],\"n\":1}\n{\"b\":3,\"n\":1}\n"),
    ("nested", ".n = ([.a] | length)", "{\"a\":{\"x\":1},\"l\":[1],\"n\":2}\n{\"a\":{\"x\":2},\"l\":[2,3],\"n\":2}\n{\"b\":3,\"n\":2}\n"),
    ("nested", ".a |= . + {\"z\": fi}", "{\"a\":{\"x\":1,\"z\":0},\"l\":[1]}\n{\"a\":{\"x\":2,\"z\":1},\"l\":[2,3]}\n{\"b\":3,\"a\":{\"z\":2}}\n"),
    ("nested", ".a = (.a + {\"z\": fi})", "{\"a\":{\"z\":2},\"l\":[1]}\n{\"a\":{\"z\":2},\"l\":[2,3]}\n{\"b\":3,\"a\":{\"z\":2}}\n"),
    ("nested", ".m = [.l[]]", "{\"a\":{\"x\":1},\"l\":[1],\"m\":[1,2,3]}\n{\"a\":{\"x\":2},\"l\":[2,3],\"m\":[1,2,3]}\n{\"b\":3,\"m\":[1,2,3]}\n"),
    ("nested", ".m = (.l | length)", "{\"a\":{\"x\":1},\"l\":[1],\"m\":2}\n{\"a\":{\"x\":2},\"l\":[2,3],\"m\":2}\n{\"b\":3,\"m\":2}\n"),
    ("nested", ".m = .b", "{\"a\":{\"x\":1},\"l\":[1],\"m\":3}\n{\"a\":{\"x\":2},\"l\":[2,3],\"m\":3}\n{\"b\":3,\"m\":3}\n"),
    ("nested", ".a.x = .b", "{\"a\":{\"x\":3},\"l\":[1]}\n{\"a\":{\"x\":3},\"l\":[2,3]}\n{\"b\":3,\"a\":{\"x\":3}}\n"),
    ("nested", ".[] = .b", "{\"a\":3,\"l\":3}\n{\"a\":3,\"l\":3}\n{\"b\":3}\n"),
];

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

/// A document whose target is a no-op never runs the right side (`5 | .a = error("x")` is `5`), several
/// targets share the one right-side value, and the structure under `-o=json` is yq's.
#[test]
fn eval_all_assignment_targets_noop_multiple_and_nested_7137() -> Result<()> {
    for &(which, filter, expected) in JSON_ROWS {
        let dir = TempDir::new()?;
        let paths = files(&dir, which)?;
        let refs: Vec<&std::path::Path> = paths.iter().map(std::path::PathBuf::as_path).collect();
        let (stdout, stderr, status) = run_yq_files(filter, &refs, &["ea", "-o=json", "-I=0"])?;
        assert_eq!(
            (stdout.as_str(), stderr.as_str(), status),
            (expected, "", 0),
            "{filter} over {which}"
        );
    }
    Ok(())
}
