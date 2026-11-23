//! #7709: a code module that cannot be opened (a directory named `dm.jq`, or a `.jq` file with
//! mode 000) is reported at every `import`/`include` that reaches it, and each report is counted,
//! as in jq 1.7.1 and 1.8.2: their `load_library` (`src/linker.c`) returns before it enters a file
//! it could not open in its table of loaded modules, so the next directive that resolves to the same
//! path opens it again and fails again. A module that opens is still reached once.
//!
//! Every row was captured from jq 1.7.1 (`/usr/bin/jq`, jq-1.7.1-apple) with `-L L -n` in the
//! layout [`build`] makes, by a 192-program matrix ({directory, mode 000, syntax error, missing} x
//! {`import as name`, `import as $data`, `include`} x {once, twice, three times, through two
//! modules, the same module twice, mixed forms, a module that loads or fails between}); jq 1.8.2
//! agrees on every row kept here. The syntax-error rows are left out, since their wording is
//! succinctly's own (`docs/compliance/jq/limitations.md`), and #7535 records how often a module that
//! does not parse is reported.

use anyhow::Result;

/// (program, stdout, stderr with `<R>` for the run's canonical root, exit status), from jq 1.7.1.
const ROWS: &[(&str, &str, &str, i32)] = &[
    ("import \"dm\" as a1; 1", "", "jq: error loading data file <R>/L/dm.jq: Could not open <R>/L/dm.jq: It's a directory\n\njq: 1 compile error\n", 3),
    ("import \"dm\" as a1;import \"dm\" as a2; 1", "", "jq: error loading data file <R>/L/dm.jq: Could not open <R>/L/dm.jq: It's a directory\n\njq: error loading data file <R>/L/dm.jq: Could not open <R>/L/dm.jq: It's a directory\n\njq: 2 compile errors\n", 3),
    ("import \"dm\" as a1;import \"dm\" as a2;import \"dm\" as a3; 1", "", "jq: error loading data file <R>/L/dm.jq: Could not open <R>/L/dm.jq: It's a directory\n\njq: error loading data file <R>/L/dm.jq: Could not open <R>/L/dm.jq: It's a directory\n\njq: error loading data file <R>/L/dm.jq: Could not open <R>/L/dm.jq: It's a directory\n\njq: 3 compile errors\n", 3),
    ("import \"n1dm\" as x; import \"n2dm\" as y; 1", "", "jq: error loading data file <R>/L/dm.jq: Could not open <R>/L/dm.jq: It's a directory\n\njq: error loading data file <R>/L/dm.jq: Could not open <R>/L/dm.jq: It's a directory\n\njq: 2 compile errors\n", 3),
    ("import \"n1dm\" as x; import \"n1dm\" as y; 1", "", "jq: error loading data file <R>/L/dm.jq: Could not open <R>/L/dm.jq: It's a directory\n\njq: 1 compile error\n", 3),
    ("include \"n1dm\"; include \"n2dm\"; 1", "", "jq: error loading data file <R>/L/dm.jq: Could not open <R>/L/dm.jq: It's a directory\n\njq: error loading data file <R>/L/dm.jq: Could not open <R>/L/dm.jq: It's a directory\n\njq: 2 compile errors\n", 3),
    ("import \"n1dm\" as x; import \"dm\" as a1; 1", "", "jq: error loading data file <R>/L/dm.jq: Could not open <R>/L/dm.jq: It's a directory\n\njq: error loading data file <R>/L/dm.jq: Could not open <R>/L/dm.jq: It's a directory\n\njq: 2 compile errors\n", 3),
    ("import \"dm\" as a1; import \"n1dm\" as x; 1", "", "jq: error loading data file <R>/L/dm.jq: Could not open <R>/L/dm.jq: It's a directory\n\njq: error loading data file <R>/L/dm.jq: Could not open <R>/L/dm.jq: It's a directory\n\njq: 2 compile errors\n", 3),
    ("import \"dm\" as a1; import \"ok\" as o; import \"dm\" as a2; 1", "", "jq: error loading data file <R>/L/dm.jq: Could not open <R>/L/dm.jq: It's a directory\n\njq: error loading data file <R>/L/dm.jq: Could not open <R>/L/dm.jq: It's a directory\n\njq: 2 compile errors\n", 3),
    ("import \"dm\" as a1;import \"perm\" as a2;import \"dm\" as a3; 1", "", "jq: error loading data file <R>/L/dm.jq: Could not open <R>/L/dm.jq: It's a directory\n\njq: error loading data file <R>/L/perm.jq: Could not open <R>/L/perm.jq: Permission denied\n\njq: error loading data file <R>/L/dm.jq: Could not open <R>/L/dm.jq: It's a directory\n\njq: 3 compile errors\n", 3),
    ("import \"dm\" as a1;import \"miss\" as a2;import \"dm\" as a3; 1", "", "jq: error loading data file <R>/L/dm.jq: Could not open <R>/L/dm.jq: It's a directory\n\njq: error: module not found: miss\n\njq: 1 compile error\n", 3),
    ("import \"dm\" as $a1; 1", "", "jq: error loading data file <R>/L/dm.json: Could not open <R>/L/dm.json: It's a directory\n\njq: 1 compile error\n", 3),
    ("import \"dm\" as $a1;import \"dm\" as $a2; 1", "", "jq: error loading data file <R>/L/dm.json: Could not open <R>/L/dm.json: It's a directory\n\njq: error loading data file <R>/L/dm.json: Could not open <R>/L/dm.json: It's a directory\n\njq: 2 compile errors\n", 3),
    ("import \"dm\" as $a1;import \"dm\" as $a2;import \"dm\" as $a3; 1", "", "jq: error loading data file <R>/L/dm.json: Could not open <R>/L/dm.json: It's a directory\n\njq: error loading data file <R>/L/dm.json: Could not open <R>/L/dm.json: It's a directory\n\njq: error loading data file <R>/L/dm.json: Could not open <R>/L/dm.json: It's a directory\n\njq: 3 compile errors\n", 3),
    ("import \"d1dm\" as x; import \"d2dm\" as y; 1", "", "jq: error loading data file <R>/L/dm.json: Could not open <R>/L/dm.json: It's a directory\n\njq: error loading data file <R>/L/dm.json: Could not open <R>/L/dm.json: It's a directory\n\njq: 2 compile errors\n", 3),
    ("import \"d1dm\" as x; import \"d1dm\" as y; 1", "", "jq: error loading data file <R>/L/dm.json: Could not open <R>/L/dm.json: It's a directory\n\njq: 1 compile error\n", 3),
    ("include \"d1dm\"; include \"d2dm\"; 1", "", "jq: error loading data file <R>/L/dm.json: Could not open <R>/L/dm.json: It's a directory\n\njq: error loading data file <R>/L/dm.json: Could not open <R>/L/dm.json: It's a directory\n\njq: 2 compile errors\n", 3),
    ("import \"d1dm\" as x; import \"dm\" as $a1; 1", "", "jq: error loading data file <R>/L/dm.json: Could not open <R>/L/dm.json: It's a directory\n\njq: error loading data file <R>/L/dm.json: Could not open <R>/L/dm.json: It's a directory\n\njq: 2 compile errors\n", 3),
    ("import \"dm\" as $a1; import \"d1dm\" as x; 1", "", "jq: error loading data file <R>/L/dm.json: Could not open <R>/L/dm.json: It's a directory\n\njq: error loading data file <R>/L/dm.json: Could not open <R>/L/dm.json: It's a directory\n\njq: 2 compile errors\n", 3),
    ("import \"dm\" as $a1; import \"ok\" as o; import \"dm\" as $a2; 1", "", "jq: error loading data file <R>/L/dm.json: Could not open <R>/L/dm.json: It's a directory\n\njq: error loading data file <R>/L/dm.json: Could not open <R>/L/dm.json: It's a directory\n\njq: 2 compile errors\n", 3),
    ("import \"dm\" as $a1;import \"perm\" as $a2;import \"dm\" as $a3; 1", "", "jq: error loading data file <R>/L/dm.json: Could not open <R>/L/dm.json: It's a directory\n\njq: error loading data file <R>/L/perm.json: Could not open <R>/L/perm.json: Permission denied\n\njq: error loading data file <R>/L/dm.json: Could not open <R>/L/dm.json: It's a directory\n\njq: 3 compile errors\n", 3),
    ("import \"dm\" as $a1;import \"miss\" as $a2;import \"dm\" as $a3; 1", "", "jq: error loading data file <R>/L/dm.json: Could not open <R>/L/dm.json: It's a directory\n\njq: error: module not found: miss\n\njq: 1 compile error\n", 3),
    ("include \"dm\"; 1", "", "jq: error loading data file <R>/L/dm.jq: Could not open <R>/L/dm.jq: It's a directory\n\njq: 1 compile error\n", 3),
    ("include \"dm\";include \"dm\"; 1", "", "jq: error loading data file <R>/L/dm.jq: Could not open <R>/L/dm.jq: It's a directory\n\njq: error loading data file <R>/L/dm.jq: Could not open <R>/L/dm.jq: It's a directory\n\njq: 2 compile errors\n", 3),
    ("include \"dm\";include \"dm\";include \"dm\"; 1", "", "jq: error loading data file <R>/L/dm.jq: Could not open <R>/L/dm.jq: It's a directory\n\njq: error loading data file <R>/L/dm.jq: Could not open <R>/L/dm.jq: It's a directory\n\njq: error loading data file <R>/L/dm.jq: Could not open <R>/L/dm.jq: It's a directory\n\njq: 3 compile errors\n", 3),
    ("import \"i1dm\" as x; import \"i2dm\" as y; 1", "", "jq: error loading data file <R>/L/dm.jq: Could not open <R>/L/dm.jq: It's a directory\n\njq: error loading data file <R>/L/dm.jq: Could not open <R>/L/dm.jq: It's a directory\n\njq: 2 compile errors\n", 3),
    ("import \"i1dm\" as x; import \"i1dm\" as y; 1", "", "jq: error loading data file <R>/L/dm.jq: Could not open <R>/L/dm.jq: It's a directory\n\njq: 1 compile error\n", 3),
    ("include \"i1dm\"; include \"i2dm\"; 1", "", "jq: error loading data file <R>/L/dm.jq: Could not open <R>/L/dm.jq: It's a directory\n\njq: error loading data file <R>/L/dm.jq: Could not open <R>/L/dm.jq: It's a directory\n\njq: 2 compile errors\n", 3),
    ("import \"i1dm\" as x; include \"dm\"; 1", "", "jq: error loading data file <R>/L/dm.jq: Could not open <R>/L/dm.jq: It's a directory\n\njq: error loading data file <R>/L/dm.jq: Could not open <R>/L/dm.jq: It's a directory\n\njq: 2 compile errors\n", 3),
    ("include \"dm\"; import \"i1dm\" as x; 1", "", "jq: error loading data file <R>/L/dm.jq: Could not open <R>/L/dm.jq: It's a directory\n\njq: error loading data file <R>/L/dm.jq: Could not open <R>/L/dm.jq: It's a directory\n\njq: 2 compile errors\n", 3),
    ("include \"dm\"; import \"ok\" as o; include \"dm\"; 1", "", "jq: error loading data file <R>/L/dm.jq: Could not open <R>/L/dm.jq: It's a directory\n\njq: error loading data file <R>/L/dm.jq: Could not open <R>/L/dm.jq: It's a directory\n\njq: 2 compile errors\n", 3),
    ("include \"dm\";include \"perm\";include \"dm\"; 1", "", "jq: error loading data file <R>/L/dm.jq: Could not open <R>/L/dm.jq: It's a directory\n\njq: error loading data file <R>/L/perm.jq: Could not open <R>/L/perm.jq: Permission denied\n\njq: error loading data file <R>/L/dm.jq: Could not open <R>/L/dm.jq: It's a directory\n\njq: 3 compile errors\n", 3),
    ("include \"dm\";include \"miss\";include \"dm\"; 1", "", "jq: error loading data file <R>/L/dm.jq: Could not open <R>/L/dm.jq: It's a directory\n\njq: error: module not found: miss\n\njq: 1 compile error\n", 3),
    ("import \"dm\" as a1;import \"dm\" as $a2; 1", "", "jq: error loading data file <R>/L/dm.json: Could not open <R>/L/dm.json: It's a directory\n\njq: error loading data file <R>/L/dm.jq: Could not open <R>/L/dm.jq: It's a directory\n\njq: 2 compile errors\n", 3),
    ("import \"dm\" as a1;import \"dm\" as $a2;import \"dm\" as a3; 1", "", "jq: error loading data file <R>/L/dm.jq: Could not open <R>/L/dm.jq: It's a directory\n\njq: error loading data file <R>/L/dm.json: Could not open <R>/L/dm.json: It's a directory\n\njq: error loading data file <R>/L/dm.jq: Could not open <R>/L/dm.jq: It's a directory\n\njq: 3 compile errors\n", 3),
    ("import \"dm\" as a1;include \"dm\"; 1", "", "jq: error loading data file <R>/L/dm.jq: Could not open <R>/L/dm.jq: It's a directory\n\njq: error loading data file <R>/L/dm.jq: Could not open <R>/L/dm.jq: It's a directory\n\njq: 2 compile errors\n", 3),
    ("import \"dm\" as a1;include \"dm\";import \"dm\" as a3; 1", "", "jq: error loading data file <R>/L/dm.jq: Could not open <R>/L/dm.jq: It's a directory\n\njq: error loading data file <R>/L/dm.jq: Could not open <R>/L/dm.jq: It's a directory\n\njq: error loading data file <R>/L/dm.jq: Could not open <R>/L/dm.jq: It's a directory\n\njq: 3 compile errors\n", 3),
    ("import \"dm\" as $a1;import \"dm\" as a2; 1", "", "jq: error loading data file <R>/L/dm.jq: Could not open <R>/L/dm.jq: It's a directory\n\njq: error loading data file <R>/L/dm.json: Could not open <R>/L/dm.json: It's a directory\n\njq: 2 compile errors\n", 3),
    ("import \"dm\" as $a1;import \"dm\" as a2;import \"dm\" as $a3; 1", "", "jq: error loading data file <R>/L/dm.json: Could not open <R>/L/dm.json: It's a directory\n\njq: error loading data file <R>/L/dm.jq: Could not open <R>/L/dm.jq: It's a directory\n\njq: error loading data file <R>/L/dm.json: Could not open <R>/L/dm.json: It's a directory\n\njq: 3 compile errors\n", 3),
    ("import \"dm\" as $a1;include \"dm\"; 1", "", "jq: error loading data file <R>/L/dm.jq: Could not open <R>/L/dm.jq: It's a directory\n\njq: error loading data file <R>/L/dm.json: Could not open <R>/L/dm.json: It's a directory\n\njq: 2 compile errors\n", 3),
    ("import \"dm\" as $a1;include \"dm\";import \"dm\" as $a3; 1", "", "jq: error loading data file <R>/L/dm.json: Could not open <R>/L/dm.json: It's a directory\n\njq: error loading data file <R>/L/dm.jq: Could not open <R>/L/dm.jq: It's a directory\n\njq: error loading data file <R>/L/dm.json: Could not open <R>/L/dm.json: It's a directory\n\njq: 3 compile errors\n", 3),
    ("include \"dm\";import \"dm\" as a2; 1", "", "jq: error loading data file <R>/L/dm.jq: Could not open <R>/L/dm.jq: It's a directory\n\njq: error loading data file <R>/L/dm.jq: Could not open <R>/L/dm.jq: It's a directory\n\njq: 2 compile errors\n", 3),
    ("include \"dm\";import \"dm\" as a2;include \"dm\"; 1", "", "jq: error loading data file <R>/L/dm.jq: Could not open <R>/L/dm.jq: It's a directory\n\njq: error loading data file <R>/L/dm.jq: Could not open <R>/L/dm.jq: It's a directory\n\njq: error loading data file <R>/L/dm.jq: Could not open <R>/L/dm.jq: It's a directory\n\njq: 3 compile errors\n", 3),
    ("include \"dm\";import \"dm\" as $a2; 1", "", "jq: error loading data file <R>/L/dm.json: Could not open <R>/L/dm.json: It's a directory\n\njq: error loading data file <R>/L/dm.jq: Could not open <R>/L/dm.jq: It's a directory\n\njq: 2 compile errors\n", 3),
    ("include \"dm\";import \"dm\" as $a2;include \"dm\"; 1", "", "jq: error loading data file <R>/L/dm.jq: Could not open <R>/L/dm.jq: It's a directory\n\njq: error loading data file <R>/L/dm.json: Could not open <R>/L/dm.json: It's a directory\n\njq: error loading data file <R>/L/dm.jq: Could not open <R>/L/dm.jq: It's a directory\n\njq: 3 compile errors\n", 3),
    ("import \"perm\" as a1; 1", "", "jq: error loading data file <R>/L/perm.jq: Could not open <R>/L/perm.jq: Permission denied\n\njq: 1 compile error\n", 3),
    ("import \"perm\" as a1;import \"perm\" as a2; 1", "", "jq: error loading data file <R>/L/perm.jq: Could not open <R>/L/perm.jq: Permission denied\n\njq: error loading data file <R>/L/perm.jq: Could not open <R>/L/perm.jq: Permission denied\n\njq: 2 compile errors\n", 3),
    ("import \"perm\" as a1;import \"perm\" as a2;import \"perm\" as a3; 1", "", "jq: error loading data file <R>/L/perm.jq: Could not open <R>/L/perm.jq: Permission denied\n\njq: error loading data file <R>/L/perm.jq: Could not open <R>/L/perm.jq: Permission denied\n\njq: error loading data file <R>/L/perm.jq: Could not open <R>/L/perm.jq: Permission denied\n\njq: 3 compile errors\n", 3),
    ("import \"n1perm\" as x; import \"n2perm\" as y; 1", "", "jq: error loading data file <R>/L/perm.jq: Could not open <R>/L/perm.jq: Permission denied\n\njq: error loading data file <R>/L/perm.jq: Could not open <R>/L/perm.jq: Permission denied\n\njq: 2 compile errors\n", 3),
    ("import \"n1perm\" as x; import \"n1perm\" as y; 1", "", "jq: error loading data file <R>/L/perm.jq: Could not open <R>/L/perm.jq: Permission denied\n\njq: 1 compile error\n", 3),
    ("include \"n1perm\"; include \"n2perm\"; 1", "", "jq: error loading data file <R>/L/perm.jq: Could not open <R>/L/perm.jq: Permission denied\n\njq: error loading data file <R>/L/perm.jq: Could not open <R>/L/perm.jq: Permission denied\n\njq: 2 compile errors\n", 3),
    ("import \"n1perm\" as x; import \"perm\" as a1; 1", "", "jq: error loading data file <R>/L/perm.jq: Could not open <R>/L/perm.jq: Permission denied\n\njq: error loading data file <R>/L/perm.jq: Could not open <R>/L/perm.jq: Permission denied\n\njq: 2 compile errors\n", 3),
    ("import \"perm\" as a1; import \"n1perm\" as x; 1", "", "jq: error loading data file <R>/L/perm.jq: Could not open <R>/L/perm.jq: Permission denied\n\njq: error loading data file <R>/L/perm.jq: Could not open <R>/L/perm.jq: Permission denied\n\njq: 2 compile errors\n", 3),
    ("import \"perm\" as a1; import \"ok\" as o; import \"perm\" as a2; 1", "", "jq: error loading data file <R>/L/perm.jq: Could not open <R>/L/perm.jq: Permission denied\n\njq: error loading data file <R>/L/perm.jq: Could not open <R>/L/perm.jq: Permission denied\n\njq: 2 compile errors\n", 3),
    ("import \"perm\" as a1;import \"dm\" as a2;import \"perm\" as a3; 1", "", "jq: error loading data file <R>/L/perm.jq: Could not open <R>/L/perm.jq: Permission denied\n\njq: error loading data file <R>/L/dm.jq: Could not open <R>/L/dm.jq: It's a directory\n\njq: error loading data file <R>/L/perm.jq: Could not open <R>/L/perm.jq: Permission denied\n\njq: 3 compile errors\n", 3),
    ("import \"perm\" as a1;import \"miss\" as a2;import \"perm\" as a3; 1", "", "jq: error loading data file <R>/L/perm.jq: Could not open <R>/L/perm.jq: Permission denied\n\njq: error: module not found: miss\n\njq: 1 compile error\n", 3),
    ("import \"perm\" as $a1; 1", "", "jq: error loading data file <R>/L/perm.json: Could not open <R>/L/perm.json: Permission denied\n\njq: 1 compile error\n", 3),
    ("import \"perm\" as $a1;import \"perm\" as $a2; 1", "", "jq: error loading data file <R>/L/perm.json: Could not open <R>/L/perm.json: Permission denied\n\njq: error loading data file <R>/L/perm.json: Could not open <R>/L/perm.json: Permission denied\n\njq: 2 compile errors\n", 3),
    ("import \"perm\" as $a1;import \"perm\" as $a2;import \"perm\" as $a3; 1", "", "jq: error loading data file <R>/L/perm.json: Could not open <R>/L/perm.json: Permission denied\n\njq: error loading data file <R>/L/perm.json: Could not open <R>/L/perm.json: Permission denied\n\njq: error loading data file <R>/L/perm.json: Could not open <R>/L/perm.json: Permission denied\n\njq: 3 compile errors\n", 3),
    ("import \"d1perm\" as x; import \"d2perm\" as y; 1", "", "jq: error loading data file <R>/L/perm.json: Could not open <R>/L/perm.json: Permission denied\n\njq: error loading data file <R>/L/perm.json: Could not open <R>/L/perm.json: Permission denied\n\njq: 2 compile errors\n", 3),
    ("import \"d1perm\" as x; import \"d1perm\" as y; 1", "", "jq: error loading data file <R>/L/perm.json: Could not open <R>/L/perm.json: Permission denied\n\njq: 1 compile error\n", 3),
    ("include \"d1perm\"; include \"d2perm\"; 1", "", "jq: error loading data file <R>/L/perm.json: Could not open <R>/L/perm.json: Permission denied\n\njq: error loading data file <R>/L/perm.json: Could not open <R>/L/perm.json: Permission denied\n\njq: 2 compile errors\n", 3),
    ("import \"d1perm\" as x; import \"perm\" as $a1; 1", "", "jq: error loading data file <R>/L/perm.json: Could not open <R>/L/perm.json: Permission denied\n\njq: error loading data file <R>/L/perm.json: Could not open <R>/L/perm.json: Permission denied\n\njq: 2 compile errors\n", 3),
    ("import \"perm\" as $a1; import \"d1perm\" as x; 1", "", "jq: error loading data file <R>/L/perm.json: Could not open <R>/L/perm.json: Permission denied\n\njq: error loading data file <R>/L/perm.json: Could not open <R>/L/perm.json: Permission denied\n\njq: 2 compile errors\n", 3),
    ("import \"perm\" as $a1; import \"ok\" as o; import \"perm\" as $a2; 1", "", "jq: error loading data file <R>/L/perm.json: Could not open <R>/L/perm.json: Permission denied\n\njq: error loading data file <R>/L/perm.json: Could not open <R>/L/perm.json: Permission denied\n\njq: 2 compile errors\n", 3),
    ("import \"perm\" as $a1;import \"dm\" as $a2;import \"perm\" as $a3; 1", "", "jq: error loading data file <R>/L/perm.json: Could not open <R>/L/perm.json: Permission denied\n\njq: error loading data file <R>/L/dm.json: Could not open <R>/L/dm.json: It's a directory\n\njq: error loading data file <R>/L/perm.json: Could not open <R>/L/perm.json: Permission denied\n\njq: 3 compile errors\n", 3),
    ("import \"perm\" as $a1;import \"miss\" as $a2;import \"perm\" as $a3; 1", "", "jq: error loading data file <R>/L/perm.json: Could not open <R>/L/perm.json: Permission denied\n\njq: error: module not found: miss\n\njq: 1 compile error\n", 3),
    ("include \"perm\"; 1", "", "jq: error loading data file <R>/L/perm.jq: Could not open <R>/L/perm.jq: Permission denied\n\njq: 1 compile error\n", 3),
    ("include \"perm\";include \"perm\"; 1", "", "jq: error loading data file <R>/L/perm.jq: Could not open <R>/L/perm.jq: Permission denied\n\njq: error loading data file <R>/L/perm.jq: Could not open <R>/L/perm.jq: Permission denied\n\njq: 2 compile errors\n", 3),
    ("include \"perm\";include \"perm\";include \"perm\"; 1", "", "jq: error loading data file <R>/L/perm.jq: Could not open <R>/L/perm.jq: Permission denied\n\njq: error loading data file <R>/L/perm.jq: Could not open <R>/L/perm.jq: Permission denied\n\njq: error loading data file <R>/L/perm.jq: Could not open <R>/L/perm.jq: Permission denied\n\njq: 3 compile errors\n", 3),
    ("import \"i1perm\" as x; import \"i2perm\" as y; 1", "", "jq: error loading data file <R>/L/perm.jq: Could not open <R>/L/perm.jq: Permission denied\n\njq: error loading data file <R>/L/perm.jq: Could not open <R>/L/perm.jq: Permission denied\n\njq: 2 compile errors\n", 3),
    ("import \"i1perm\" as x; import \"i1perm\" as y; 1", "", "jq: error loading data file <R>/L/perm.jq: Could not open <R>/L/perm.jq: Permission denied\n\njq: 1 compile error\n", 3),
    ("include \"i1perm\"; include \"i2perm\"; 1", "", "jq: error loading data file <R>/L/perm.jq: Could not open <R>/L/perm.jq: Permission denied\n\njq: error loading data file <R>/L/perm.jq: Could not open <R>/L/perm.jq: Permission denied\n\njq: 2 compile errors\n", 3),
    ("import \"i1perm\" as x; include \"perm\"; 1", "", "jq: error loading data file <R>/L/perm.jq: Could not open <R>/L/perm.jq: Permission denied\n\njq: error loading data file <R>/L/perm.jq: Could not open <R>/L/perm.jq: Permission denied\n\njq: 2 compile errors\n", 3),
    ("include \"perm\"; import \"i1perm\" as x; 1", "", "jq: error loading data file <R>/L/perm.jq: Could not open <R>/L/perm.jq: Permission denied\n\njq: error loading data file <R>/L/perm.jq: Could not open <R>/L/perm.jq: Permission denied\n\njq: 2 compile errors\n", 3),
    ("include \"perm\"; import \"ok\" as o; include \"perm\"; 1", "", "jq: error loading data file <R>/L/perm.jq: Could not open <R>/L/perm.jq: Permission denied\n\njq: error loading data file <R>/L/perm.jq: Could not open <R>/L/perm.jq: Permission denied\n\njq: 2 compile errors\n", 3),
    ("include \"perm\";include \"dm\";include \"perm\"; 1", "", "jq: error loading data file <R>/L/perm.jq: Could not open <R>/L/perm.jq: Permission denied\n\njq: error loading data file <R>/L/dm.jq: Could not open <R>/L/dm.jq: It's a directory\n\njq: error loading data file <R>/L/perm.jq: Could not open <R>/L/perm.jq: Permission denied\n\njq: 3 compile errors\n", 3),
    ("include \"perm\";include \"miss\";include \"perm\"; 1", "", "jq: error loading data file <R>/L/perm.jq: Could not open <R>/L/perm.jq: Permission denied\n\njq: error: module not found: miss\n\njq: 1 compile error\n", 3),
    ("import \"perm\" as a1;import \"perm\" as $a2; 1", "", "jq: error loading data file <R>/L/perm.json: Could not open <R>/L/perm.json: Permission denied\n\njq: error loading data file <R>/L/perm.jq: Could not open <R>/L/perm.jq: Permission denied\n\njq: 2 compile errors\n", 3),
    ("import \"perm\" as a1;import \"perm\" as $a2;import \"perm\" as a3; 1", "", "jq: error loading data file <R>/L/perm.jq: Could not open <R>/L/perm.jq: Permission denied\n\njq: error loading data file <R>/L/perm.json: Could not open <R>/L/perm.json: Permission denied\n\njq: error loading data file <R>/L/perm.jq: Could not open <R>/L/perm.jq: Permission denied\n\njq: 3 compile errors\n", 3),
    ("import \"perm\" as a1;include \"perm\"; 1", "", "jq: error loading data file <R>/L/perm.jq: Could not open <R>/L/perm.jq: Permission denied\n\njq: error loading data file <R>/L/perm.jq: Could not open <R>/L/perm.jq: Permission denied\n\njq: 2 compile errors\n", 3),
    ("import \"perm\" as a1;include \"perm\";import \"perm\" as a3; 1", "", "jq: error loading data file <R>/L/perm.jq: Could not open <R>/L/perm.jq: Permission denied\n\njq: error loading data file <R>/L/perm.jq: Could not open <R>/L/perm.jq: Permission denied\n\njq: error loading data file <R>/L/perm.jq: Could not open <R>/L/perm.jq: Permission denied\n\njq: 3 compile errors\n", 3),
    ("import \"perm\" as $a1;import \"perm\" as a2; 1", "", "jq: error loading data file <R>/L/perm.jq: Could not open <R>/L/perm.jq: Permission denied\n\njq: error loading data file <R>/L/perm.json: Could not open <R>/L/perm.json: Permission denied\n\njq: 2 compile errors\n", 3),
    ("import \"perm\" as $a1;import \"perm\" as a2;import \"perm\" as $a3; 1", "", "jq: error loading data file <R>/L/perm.json: Could not open <R>/L/perm.json: Permission denied\n\njq: error loading data file <R>/L/perm.jq: Could not open <R>/L/perm.jq: Permission denied\n\njq: error loading data file <R>/L/perm.json: Could not open <R>/L/perm.json: Permission denied\n\njq: 3 compile errors\n", 3),
    ("import \"perm\" as $a1;include \"perm\"; 1", "", "jq: error loading data file <R>/L/perm.jq: Could not open <R>/L/perm.jq: Permission denied\n\njq: error loading data file <R>/L/perm.json: Could not open <R>/L/perm.json: Permission denied\n\njq: 2 compile errors\n", 3),
    ("import \"perm\" as $a1;include \"perm\";import \"perm\" as $a3; 1", "", "jq: error loading data file <R>/L/perm.json: Could not open <R>/L/perm.json: Permission denied\n\njq: error loading data file <R>/L/perm.jq: Could not open <R>/L/perm.jq: Permission denied\n\njq: error loading data file <R>/L/perm.json: Could not open <R>/L/perm.json: Permission denied\n\njq: 3 compile errors\n", 3),
    ("include \"perm\";import \"perm\" as a2; 1", "", "jq: error loading data file <R>/L/perm.jq: Could not open <R>/L/perm.jq: Permission denied\n\njq: error loading data file <R>/L/perm.jq: Could not open <R>/L/perm.jq: Permission denied\n\njq: 2 compile errors\n", 3),
    ("include \"perm\";import \"perm\" as a2;include \"perm\"; 1", "", "jq: error loading data file <R>/L/perm.jq: Could not open <R>/L/perm.jq: Permission denied\n\njq: error loading data file <R>/L/perm.jq: Could not open <R>/L/perm.jq: Permission denied\n\njq: error loading data file <R>/L/perm.jq: Could not open <R>/L/perm.jq: Permission denied\n\njq: 3 compile errors\n", 3),
    ("include \"perm\";import \"perm\" as $a2; 1", "", "jq: error loading data file <R>/L/perm.json: Could not open <R>/L/perm.json: Permission denied\n\njq: error loading data file <R>/L/perm.jq: Could not open <R>/L/perm.jq: Permission denied\n\njq: 2 compile errors\n", 3),
    ("include \"perm\";import \"perm\" as $a2;include \"perm\"; 1", "", "jq: error loading data file <R>/L/perm.jq: Could not open <R>/L/perm.jq: Permission denied\n\njq: error loading data file <R>/L/perm.json: Could not open <R>/L/perm.json: Permission denied\n\njq: error loading data file <R>/L/perm.jq: Could not open <R>/L/perm.jq: Permission denied\n\njq: 3 compile errors\n", 3),
    ("import \"miss\" as a1;import \"dm\" as a2;import \"miss\" as a3; 1", "", "jq: error: module not found: miss\n\njq: 1 compile error\n", 3),
    ("import \"miss\" as a1;import \"perm\" as a2;import \"miss\" as a3; 1", "", "jq: error: module not found: miss\n\njq: 1 compile error\n", 3),
    ("import \"miss\" as $a1;import \"dm\" as $a2;import \"miss\" as $a3; 1", "", "jq: error: module not found: miss\n\njq: 1 compile error\n", 3),
    ("import \"miss\" as $a1;import \"perm\" as $a2;import \"miss\" as $a3; 1", "", "jq: error: module not found: miss\n\njq: 1 compile error\n", 3),
    ("include \"miss\";include \"dm\";include \"miss\"; 1", "", "jq: error: module not found: miss\n\njq: 1 compile error\n", 3),
    ("include \"miss\";include \"perm\";include \"miss\"; 1", "", "jq: error: module not found: miss\n\njq: 1 compile error\n", 3),
];

/// The library under `root/L`: `dm.jq`/`dm.json` are directories, `perm.jq`/`perm.json` files this
/// user cannot read, `bad.jq`/`bad.json` do not parse, `miss` is absent, and `n1K`/`n2K` (`import`),
/// `d1K`/`d2K` (data `import`) and `i1K`/`i2K` (`include`) reach module `K` from a module of their
/// own. Whether the mode-000 files really are unreadable to this user (root reads them anyway).
#[cfg(unix)]
fn build(root: &std::path::Path) -> Result<bool> {
    use std::os::unix::fs::PermissionsExt;
    let lib = root.join("L");
    std::fs::create_dir_all(lib.join("dm.jq"))?;
    std::fs::create_dir_all(lib.join("dm.json"))?;
    std::fs::write(lib.join("perm.jq"), "def p: 1;\n")?;
    std::fs::write(lib.join("perm.json"), "1\n")?;
    std::fs::write(lib.join("bad.jq"), "def f: (;\n")?;
    std::fs::write(lib.join("bad.json"), "{\"a\":\n")?;
    std::fs::write(lib.join("ok.jq"), "def ok: 1;\n")?;
    for k in ["dm", "perm", "bad", "miss"] {
        for (file, text) in [
            ("n1", format!("import \"{k}\" as a; def n1: 1;\n")),
            ("n2", format!("import \"{k}\" as b; def n2: 2;\n")),
            ("d1", format!("import \"{k}\" as $a; def n1: 1;\n")),
            ("d2", format!("import \"{k}\" as $b; def n2: 2;\n")),
            ("i1", format!("include \"{k}\"; def n1: 1;\n")),
            ("i2", format!("include \"{k}\"; def n2: 2;\n")),
        ] {
            std::fs::write(lib.join(format!("{file}{k}.jq")), text)?;
        }
    }
    let mut binds = true;
    for file in ["perm.jq", "perm.json"] {
        let path = lib.join(file);
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000))?;
        binds &= std::fs::read(&path).is_err();
    }
    Ok(binds)
}

#[cfg(unix)]
#[test]
fn an_unopenable_module_is_reported_at_every_directive_that_reaches_it_7709() -> Result<()> {
    let root = tempfile::tempdir()?;
    let binds = build(root.path())?;
    let canonical = std::fs::canonicalize(root.path())?;
    let mut checked = 0;
    for &(program, stdout, stderr, status) in ROWS {
        if !binds && program.contains("perm") {
            continue;
        }
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_succinctly"))
            .args(["jq", "-L", "L", "-n", program])
            .current_dir(&canonical)
            .env("LC_ALL", "C")
            .env("HOME", &canonical)
            .env_remove("JQ_LIBRARY_PATH")
            .output()?;
        let got_stderr =
            String::from_utf8_lossy(&output.stderr).replace(&*canonical.to_string_lossy(), "<R>");
        assert_eq!(
            (
                String::from_utf8_lossy(&output.stdout).as_ref(),
                got_stderr.as_str(),
                output.status.code()
            ),
            (stdout, stderr, Some(status)),
            "{program}"
        );
        checked += 1;
    }
    if !binds {
        eprintln!("skipped the mode-000 rows: chmod 000 does not bind this user");
    }
    assert!(checked >= ROWS.len() / 2, "{checked} rows run");
    Ok(())
}
