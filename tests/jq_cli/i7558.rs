//! #7558: a failure after a value of a `?//` bind in a builtin's argument retries the bind's next
//! alternative in jq, as it does after any other `as` binding: `in(xs)` (`def in(xs): . as $x |
//! xs | has($x);`), `halt_error/1`, and the arguments jq reads inside an array it collects,
//! `split`'s and `splits`' flags and `combinations`' count. The evaluators read those arguments to
//! the end before the builtin acted, so the first alternative's failure was reported, and
//! `in([[1], {"a":1}] as [$a] ?// [$b, $a] | $a)` on `"a"` raised where jq answers `true`.
//! #3487 and #7474 bind the other builtins' arguments; the last rows are their controls.
//!
//! Every row is jq 1.7.1's own output (`/usr/bin/jq -c`, stdin ending in a newline). jq 1.8.2
//! retries the same way; it differs on 26 rows for reasons outside this issue: it adds `"g"` after
//! the flags (`number (1) and string ("g") cannot be added`), previews the index in an error
//! (jqlang/jq#3478's rewording), and since jqlang/jq#3260's refactor binds `splits`' flags as a
//! `$flags` parameter, so several flags values split separately (`split(","; ("g", null))`).
//! That refactor is not a fix, so 1.7.1 stays the reference.

use anyhow::Result;

use crate::run_jq_full;

/// `(program, input, stdout, stderr, exit)`.
const ROWS: &[(&str, &str, &str, &str, i32)] = &[
    ("in(([[1], {\"a\":1}] as [$a] ?// [$b, $a] | $a))", "\"a\"", "true\n", "", 0),
    ("[in(([[1], {\"a\":1}] as [$a] ?// [$b, $a] | $a), {\"a\":0})]", "\"a\"", "[true,true]\n", "", 0),
    ("in(([[1], {\"a\":1}] as [$a] ?// [$b, $a] | $a)) | if . then error(\"later\") else . end", "\"a\"", "", "jq: error (at <stdin>:1): later\n", 5),
    ("try in(([[1], {\"a\":1}] as [$a] ?// [$b, $a] | $a)) catch .", "\"a\"", "true\n", "", 0),
    ("in(([{\"a\":1}, [1]] as [$a] ?// [$b, $a] | $a))", "\"a\"", "true\n", "", 0),
    ("[in(([{\"a\":1}, [1]] as [$a] ?// [$b, $a] | $a), {\"a\":0})]", "\"a\"", "[true,true]\n", "", 0),
    ("in(([{\"a\":1}, [1]] as [$a] ?// [$b, $a] | $a)) | if . then error(\"later\") else . end", "\"a\"", "", "jq: error (at <stdin>:1): Cannot check whether array has a string key\n", 5),
    ("try in(([{\"a\":1}, [1]] as [$a] ?// [$b, $a] | $a)) catch .", "\"a\"", "true\n", "", 0),
    ("in(([[1], [2]] as [$a] ?// [$b, $a] | $a))", "\"a\"", "", "jq: error (at <stdin>:1): Cannot check whether array has a string key\n", 5),
    ("[in(([[1], [2]] as [$a] ?// [$b, $a] | $a), {\"a\":0})]", "\"a\"", "", "jq: error (at <stdin>:1): Cannot check whether array has a string key\n", 5),
    ("in(([[1], [2]] as [$a] ?// [$b, $a] | $a)) | if . then error(\"later\") else . end", "\"a\"", "", "jq: error (at <stdin>:1): Cannot check whether array has a string key\n", 5),
    ("try in(([[1], [2]] as [$a] ?// [$b, $a] | $a)) catch .", "\"a\"", "\"Cannot check whether array has a string key\"\n", "", 0),
    ("in(([{\"b\":2}, {\"a\":1}] as [$a] ?// [$b, $a] | $a))", "\"a\"", "false\n", "", 0),
    ("[in(([{\"b\":2}, {\"a\":1}] as [$a] ?// [$b, $a] | $a), {\"a\":0})]", "\"a\"", "[false,true]\n", "", 0),
    ("in(([{\"b\":2}, {\"a\":1}] as [$a] ?// [$b, $a] | $a)) | if . then error(\"later\") else . end", "\"a\"", "false\n", "", 0),
    ("try in(([{\"b\":2}, {\"a\":1}] as [$a] ?// [$b, $a] | $a)) catch .", "\"a\"", "false\n", "", 0),
    ("in(([[1], {\"a\":1}] as [$a] ?// [$b, $a] | $a))", "\"b\"", "false\n", "", 0),
    ("[in(([[1], {\"a\":1}] as [$a] ?// [$b, $a] | $a), {\"a\":0})]", "\"b\"", "[false,false]\n", "", 0),
    ("in(([[1], {\"a\":1}] as [$a] ?// [$b, $a] | $a)) | if . then error(\"later\") else . end", "\"b\"", "false\n", "", 0),
    ("try in(([[1], {\"a\":1}] as [$a] ?// [$b, $a] | $a)) catch .", "\"b\"", "false\n", "", 0),
    ("in(([{\"a\":1}, [1]] as [$a] ?// [$b, $a] | $a))", "\"b\"", "false\n", "", 0),
    ("[in(([{\"a\":1}, [1]] as [$a] ?// [$b, $a] | $a), {\"a\":0})]", "\"b\"", "[false,false]\n", "", 0),
    ("in(([{\"a\":1}, [1]] as [$a] ?// [$b, $a] | $a)) | if . then error(\"later\") else . end", "\"b\"", "false\n", "", 0),
    ("try in(([{\"a\":1}, [1]] as [$a] ?// [$b, $a] | $a)) catch .", "\"b\"", "false\n", "", 0),
    ("in(([[1], [2]] as [$a] ?// [$b, $a] | $a))", "\"b\"", "", "jq: error (at <stdin>:1): Cannot check whether array has a string key\n", 5),
    ("[in(([[1], [2]] as [$a] ?// [$b, $a] | $a), {\"a\":0})]", "\"b\"", "", "jq: error (at <stdin>:1): Cannot check whether array has a string key\n", 5),
    ("in(([[1], [2]] as [$a] ?// [$b, $a] | $a)) | if . then error(\"later\") else . end", "\"b\"", "", "jq: error (at <stdin>:1): Cannot check whether array has a string key\n", 5),
    ("try in(([[1], [2]] as [$a] ?// [$b, $a] | $a)) catch .", "\"b\"", "\"Cannot check whether array has a string key\"\n", "", 0),
    ("in(([{\"b\":2}, {\"a\":1}] as [$a] ?// [$b, $a] | $a))", "\"b\"", "true\n", "", 0),
    ("[in(([{\"b\":2}, {\"a\":1}] as [$a] ?// [$b, $a] | $a), {\"a\":0})]", "\"b\"", "[true,false]\n", "", 0),
    ("in(([{\"b\":2}, {\"a\":1}] as [$a] ?// [$b, $a] | $a)) | if . then error(\"later\") else . end", "\"b\"", "false\n", "", 0),
    ("try in(([{\"b\":2}, {\"a\":1}] as [$a] ?// [$b, $a] | $a)) catch .", "\"b\"", "true\n", "", 0),
    ("[splits(\",\"; ([1, \"g\"] as [$a] ?// [$b, $a] | $a))]", "\"a,b\"", "[\"a\",\"b\"]\n", "", 0),
    ("split(\",\"; ([1, \"g\"] as [$a] ?// [$b, $a] | $a))", "\"a,b\"", "[\"a\",\"b\"]\n", "", 0),
    ("[splits(\"A\"; ([1, \"g\"] as [$a] ?// [$b, $a] | $a))]", "\"a,b\"", "[\"a,b\"]\n", "", 0),
    ("split(([1, \",\"] as [$a] ?// [$b, $a] | $a); ([1, \"g\"] as [$a] ?// [$b, $a] | $a))", "\"a,b\"", "[\"a\",\"b\"]\n", "", 0),
    ("[splits(\",\"; ([1, \"g\"] as [$a] ?// [$b, $a] | $a))]", "null", "", "jq: error (at <stdin>:1): null (null) cannot be matched, as it is not a string\n", 5),
    ("split(\",\"; ([1, \"g\"] as [$a] ?// [$b, $a] | $a))", "null", "", "jq: error (at <stdin>:1): null (null) cannot be matched, as it is not a string\n", 5),
    ("[splits(\"A\"; ([1, \"g\"] as [$a] ?// [$b, $a] | $a))]", "null", "", "jq: error (at <stdin>:1): null (null) cannot be matched, as it is not a string\n", 5),
    ("split(([1, \",\"] as [$a] ?// [$b, $a] | $a); ([1, \"g\"] as [$a] ?// [$b, $a] | $a))", "null", "", "jq: error (at <stdin>:1): null (null) cannot be matched, as it is not a string\n", 5),
    ("[splits(\",\"; ([1, \"g\"] as [$a] ?// [$b, $a] | $a))]", "\"A,b\"", "[\"A\",\"b\"]\n", "", 0),
    ("split(\",\"; ([1, \"g\"] as [$a] ?// [$b, $a] | $a))", "\"A,b\"", "[\"A\",\"b\"]\n", "", 0),
    ("[splits(\"A\"; ([1, \"g\"] as [$a] ?// [$b, $a] | $a))]", "\"A,b\"", "[\"\",\",b\"]\n", "", 0),
    ("split(([1, \",\"] as [$a] ?// [$b, $a] | $a); ([1, \"g\"] as [$a] ?// [$b, $a] | $a))", "\"A,b\"", "[\"A\",\"b\"]\n", "", 0),
    ("[splits(\",\"; ([1, null] as [$a] ?// [$b, $a] | $a))]", "\"a,b\"", "[\"a\",\"b\"]\n", "", 0),
    ("split(\",\"; ([1, null] as [$a] ?// [$b, $a] | $a))", "\"a,b\"", "[\"a\",\"b\"]\n", "", 0),
    ("[splits(\"A\"; ([1, null] as [$a] ?// [$b, $a] | $a))]", "\"a,b\"", "[\"a,b\"]\n", "", 0),
    ("split(([1, \",\"] as [$a] ?// [$b, $a] | $a); ([1, null] as [$a] ?// [$b, $a] | $a))", "\"a,b\"", "[\"a\",\"b\"]\n", "", 0),
    ("[splits(\",\"; ([1, null] as [$a] ?// [$b, $a] | $a))]", "null", "", "jq: error (at <stdin>:1): null (null) cannot be matched, as it is not a string\n", 5),
    ("split(\",\"; ([1, null] as [$a] ?// [$b, $a] | $a))", "null", "", "jq: error (at <stdin>:1): null (null) cannot be matched, as it is not a string\n", 5),
    ("[splits(\"A\"; ([1, null] as [$a] ?// [$b, $a] | $a))]", "null", "", "jq: error (at <stdin>:1): null (null) cannot be matched, as it is not a string\n", 5),
    ("split(([1, \",\"] as [$a] ?// [$b, $a] | $a); ([1, null] as [$a] ?// [$b, $a] | $a))", "null", "", "jq: error (at <stdin>:1): null (null) cannot be matched, as it is not a string\n", 5),
    ("[splits(\",\"; ([1, null] as [$a] ?// [$b, $a] | $a))]", "\"A,b\"", "[\"A\",\"b\"]\n", "", 0),
    ("split(\",\"; ([1, null] as [$a] ?// [$b, $a] | $a))", "\"A,b\"", "[\"A\",\"b\"]\n", "", 0),
    ("[splits(\"A\"; ([1, null] as [$a] ?// [$b, $a] | $a))]", "\"A,b\"", "[\"\",\",b\"]\n", "", 0),
    ("split(([1, \",\"] as [$a] ?// [$b, $a] | $a); ([1, null] as [$a] ?// [$b, $a] | $a))", "\"A,b\"", "[\"A\",\"b\"]\n", "", 0),
    ("[splits(\",\"; ([[1], \"x\"] as [$a] ?// [$b, $a] | $a))]", "\"a,b\"", "[\"a\",\"b\"]\n", "", 0),
    ("split(\",\"; ([[1], \"x\"] as [$a] ?// [$b, $a] | $a))", "\"a,b\"", "[\"a\",\"b\"]\n", "", 0),
    ("[splits(\"A\"; ([[1], \"x\"] as [$a] ?// [$b, $a] | $a))]", "\"a,b\"", "[\"a,b\"]\n", "", 0),
    ("split(([1, \",\"] as [$a] ?// [$b, $a] | $a); ([[1], \"x\"] as [$a] ?// [$b, $a] | $a))", "\"a,b\"", "[\"a\",\"b\"]\n", "", 0),
    ("[splits(\",\"; ([[1], \"x\"] as [$a] ?// [$b, $a] | $a))]", "null", "", "jq: error (at <stdin>:1): null (null) cannot be matched, as it is not a string\n", 5),
    ("split(\",\"; ([[1], \"x\"] as [$a] ?// [$b, $a] | $a))", "null", "", "jq: error (at <stdin>:1): null (null) cannot be matched, as it is not a string\n", 5),
    ("[splits(\"A\"; ([[1], \"x\"] as [$a] ?// [$b, $a] | $a))]", "null", "", "jq: error (at <stdin>:1): null (null) cannot be matched, as it is not a string\n", 5),
    ("split(([1, \",\"] as [$a] ?// [$b, $a] | $a); ([[1], \"x\"] as [$a] ?// [$b, $a] | $a))", "null", "", "jq: error (at <stdin>:1): null (null) cannot be matched, as it is not a string\n", 5),
    ("[splits(\",\"; ([[1], \"x\"] as [$a] ?// [$b, $a] | $a))]", "\"A,b\"", "[\"A\",\"b\"]\n", "", 0),
    ("split(\",\"; ([[1], \"x\"] as [$a] ?// [$b, $a] | $a))", "\"A,b\"", "[\"A\",\"b\"]\n", "", 0),
    ("[splits(\"A\"; ([[1], \"x\"] as [$a] ?// [$b, $a] | $a))]", "\"A,b\"", "[\"\",\",b\"]\n", "", 0),
    ("split(([1, \",\"] as [$a] ?// [$b, $a] | $a); ([[1], \"x\"] as [$a] ?// [$b, $a] | $a))", "\"A,b\"", "[\"A\",\"b\"]\n", "", 0),
    ("[splits(\",\"; ([\"q\", \"g\"] as [$a] ?// [$b, $a] | $a))]", "\"a,b\"", "[\"a\",\"b\"]\n", "", 0),
    ("split(\",\"; ([\"q\", \"g\"] as [$a] ?// [$b, $a] | $a))", "\"a,b\"", "[\"a\",\"b\"]\n", "", 0),
    ("[splits(\"A\"; ([\"q\", \"g\"] as [$a] ?// [$b, $a] | $a))]", "\"a,b\"", "[\"a,b\"]\n", "", 0),
    ("split(([1, \",\"] as [$a] ?// [$b, $a] | $a); ([\"q\", \"g\"] as [$a] ?// [$b, $a] | $a))", "\"a,b\"", "[\"a\",\"b\"]\n", "", 0),
    ("[splits(\",\"; ([\"q\", \"g\"] as [$a] ?// [$b, $a] | $a))]", "null", "", "jq: error (at <stdin>:1): null (null) cannot be matched, as it is not a string\n", 5),
    ("split(\",\"; ([\"q\", \"g\"] as [$a] ?// [$b, $a] | $a))", "null", "", "jq: error (at <stdin>:1): null (null) cannot be matched, as it is not a string\n", 5),
    ("[splits(\"A\"; ([\"q\", \"g\"] as [$a] ?// [$b, $a] | $a))]", "null", "", "jq: error (at <stdin>:1): null (null) cannot be matched, as it is not a string\n", 5),
    ("split(([1, \",\"] as [$a] ?// [$b, $a] | $a); ([\"q\", \"g\"] as [$a] ?// [$b, $a] | $a))", "null", "", "jq: error (at <stdin>:1): null (null) cannot be matched, as it is not a string\n", 5),
    ("[splits(\",\"; ([\"q\", \"g\"] as [$a] ?// [$b, $a] | $a))]", "\"A,b\"", "[\"A\",\"b\"]\n", "", 0),
    ("split(\",\"; ([\"q\", \"g\"] as [$a] ?// [$b, $a] | $a))", "\"A,b\"", "[\"A\",\"b\"]\n", "", 0),
    ("[splits(\"A\"; ([\"q\", \"g\"] as [$a] ?// [$b, $a] | $a))]", "\"A,b\"", "[\"\",\",b\"]\n", "", 0),
    ("split(([1, \",\"] as [$a] ?// [$b, $a] | $a); ([\"q\", \"g\"] as [$a] ?// [$b, $a] | $a))", "\"A,b\"", "[\"A\",\"b\"]\n", "", 0),
    ("[splits(\",\"; ([null, 1] as [$a] ?// [$b, $a] | $a))]", "\"a,b\"", "[\"a\",\"b\"]\n", "", 0),
    ("split(\",\"; ([null, 1] as [$a] ?// [$b, $a] | $a))", "\"a,b\"", "[\"a\",\"b\"]\n", "", 0),
    ("[splits(\"A\"; ([null, 1] as [$a] ?// [$b, $a] | $a))]", "\"a,b\"", "[\"a,b\"]\n", "", 0),
    ("split(([1, \",\"] as [$a] ?// [$b, $a] | $a); ([null, 1] as [$a] ?// [$b, $a] | $a))", "\"a,b\"", "[\"a\",\"b\"]\n", "", 0),
    ("[splits(\",\"; ([null, 1] as [$a] ?// [$b, $a] | $a))]", "null", "", "jq: error (at <stdin>:1): string (\"g\") and number (1) cannot be added\n", 5),
    ("split(\",\"; ([null, 1] as [$a] ?// [$b, $a] | $a))", "null", "", "jq: error (at <stdin>:1): string (\"g\") and number (1) cannot be added\n", 5),
    ("[splits(\"A\"; ([null, 1] as [$a] ?// [$b, $a] | $a))]", "null", "", "jq: error (at <stdin>:1): string (\"g\") and number (1) cannot be added\n", 5),
    ("split(([1, \",\"] as [$a] ?// [$b, $a] | $a); ([null, 1] as [$a] ?// [$b, $a] | $a))", "null", "", "jq: error (at <stdin>:1): string (\"g\") and number (1) cannot be added\n", 5),
    ("[splits(\",\"; ([null, 1] as [$a] ?// [$b, $a] | $a))]", "\"A,b\"", "[\"A\",\"b\"]\n", "", 0),
    ("split(\",\"; ([null, 1] as [$a] ?// [$b, $a] | $a))", "\"A,b\"", "[\"A\",\"b\"]\n", "", 0),
    ("[splits(\"A\"; ([null, 1] as [$a] ?// [$b, $a] | $a))]", "\"A,b\"", "[\"\",\",b\"]\n", "", 0),
    ("split(([1, \",\"] as [$a] ?// [$b, $a] | $a); ([null, 1] as [$a] ?// [$b, $a] | $a))", "\"A,b\"", "[\"A\",\"b\"]\n", "", 0),
    ("[splits(\",\"; ([1, {}] as [$a] ?// [$b, $a] | $a))]", "\"a,b\"", "", "jq: error (at <stdin>:1): string (\"g\") and object ({}) cannot be added\n", 5),
    ("split(\",\"; ([1, {}] as [$a] ?// [$b, $a] | $a))", "\"a,b\"", "", "jq: error (at <stdin>:1): string (\"g\") and object ({}) cannot be added\n", 5),
    ("[splits(\"A\"; ([1, {}] as [$a] ?// [$b, $a] | $a))]", "\"a,b\"", "", "jq: error (at <stdin>:1): string (\"g\") and object ({}) cannot be added\n", 5),
    ("split(([1, \",\"] as [$a] ?// [$b, $a] | $a); ([1, {}] as [$a] ?// [$b, $a] | $a))", "\"a,b\"", "", "jq: error (at <stdin>:1): string (\"g\") and object ({}) cannot be added\n", 5),
    ("[splits(\",\"; ([1, {}] as [$a] ?// [$b, $a] | $a))]", "null", "", "jq: error (at <stdin>:1): string (\"g\") and object ({}) cannot be added\n", 5),
    ("split(\",\"; ([1, {}] as [$a] ?// [$b, $a] | $a))", "null", "", "jq: error (at <stdin>:1): string (\"g\") and object ({}) cannot be added\n", 5),
    ("[splits(\"A\"; ([1, {}] as [$a] ?// [$b, $a] | $a))]", "null", "", "jq: error (at <stdin>:1): string (\"g\") and object ({}) cannot be added\n", 5),
    ("split(([1, \",\"] as [$a] ?// [$b, $a] | $a); ([1, {}] as [$a] ?// [$b, $a] | $a))", "null", "", "jq: error (at <stdin>:1): string (\"g\") and object ({}) cannot be added\n", 5),
    ("[splits(\",\"; ([1, {}] as [$a] ?// [$b, $a] | $a))]", "\"A,b\"", "", "jq: error (at <stdin>:1): string (\"g\") and object ({}) cannot be added\n", 5),
    ("split(\",\"; ([1, {}] as [$a] ?// [$b, $a] | $a))", "\"A,b\"", "", "jq: error (at <stdin>:1): string (\"g\") and object ({}) cannot be added\n", 5),
    ("[splits(\"A\"; ([1, {}] as [$a] ?// [$b, $a] | $a))]", "\"A,b\"", "", "jq: error (at <stdin>:1): string (\"g\") and object ({}) cannot be added\n", 5),
    ("split(([1, \",\"] as [$a] ?// [$b, $a] | $a); ([1, {}] as [$a] ?// [$b, $a] | $a))", "\"A,b\"", "", "jq: error (at <stdin>:1): string (\"g\") and object ({}) cannot be added\n", 5),
    ("[splits((\",\", \" \"); (\"g\", null))]", "\"a,b c\"", "[\"a\",\"\",\"b c\",\"a,b\",\"\",\"c\"]\n", "", 0),
    ("split(\",\"; (\"g\", null))", "\"a,b c\"", "[\"a\",\"\",\"b c\"]\n", "", 0),
    ("[splits(\",\"; ((\"g\", null) as $a ?// [$a] | $a))]", "\"a,b c\"", "[\"a\",\"\",\"b c\"]\n", "", 0),
    ("[splits(\",\"; ((1, \"g\") as $a ?// [$a] | $a))]", "\"a,b c\"", "", "jq: error (at <stdin>:1): Cannot index number with number\n", 5),
    ("split(\",\"; ((1, \"g\", null) as [$a] ?// $a | $a))", "\"a,b c\"", "", "jq: error (at <stdin>:1): string (\"g\") and number (1) cannot be added\n", 5),
    ("split(\",\"; ((\"g\", \"x\", null) as [$a] ?// $a | $a))", "\"a,b c\"", "[\"a\",\"\",\"\",\"b c\"]\n", "", 0),
    ("[splits(\",\"; empty)]", "\"a,b c\"", "[\"a,b c\"]\n", "", 0),
    ("split(\"x\"; (([1], \"i\") as [$a] ?// $a | $a))", "\"aXbxc\"", "", "jq: error (at <stdin>:1): string (\"g\") and array ([1]) cannot be added\n", 5),
    ("[splits(\"x\"; ((\"i\", 2) as $a ?// [$a] | $a), \"g\")]", "\"aXbxc\"", "", "jq: error (at <stdin>:1): Cannot index number with number\n", 5),
    ("[combinations(([\"x\", 1] as [$a] ?// [$b, $a] | $a))]", "[1,2]", "[[1],[2]]\n", "", 0),
    ("[combinations(([\"x\", 1] as [$a] ?// [$b, $a] | $a))]", "[[1],[2,3]]", "[[[1]],[[2,3]]]\n", "", 0),
    ("[combinations(([\"x\", 1] as [$a] ?// [$b, $a] | $a))]", "\"s\"", "", "jq: error (at <stdin>:1): Cannot iterate over string (\"s\")\n", 5),
    ("[combinations(([\"x\", 2] as [$a] ?// [$b, $a] | $a))]", "[1,2]", "[[1,1],[1,2],[2,1],[2,2]]\n", "", 0),
    ("[combinations(([\"x\", 2] as [$a] ?// [$b, $a] | $a))]", "[[1],[2,3]]", "[[[1],[1]],[[1],[2,3]],[[2,3],[1]],[[2,3],[2,3]]]\n", "", 0),
    ("[combinations(([\"x\", 2] as [$a] ?// [$b, $a] | $a))]", "\"s\"", "", "jq: error (at <stdin>:1): Cannot iterate over string (\"s\")\n", 5),
    ("[combinations(([[1], \"y\"] as [$a] ?// [$b, $a] | $a))]", "[1,2]", "", "jq: error (at <stdin>:1): Range bounds must be numeric\n", 5),
    ("[combinations(([[1], \"y\"] as [$a] ?// [$b, $a] | $a))]", "[[1],[2,3]]", "", "jq: error (at <stdin>:1): Range bounds must be numeric\n", 5),
    ("[combinations(([[1], \"y\"] as [$a] ?// [$b, $a] | $a))]", "\"s\"", "", "jq: error (at <stdin>:1): Range bounds must be numeric\n", 5),
    ("[combinations(([-1, 2] as [$a] ?// [$b, $a] | $a))]", "[1,2]", "[[]]\n", "", 0),
    ("[combinations(([-1, 2] as [$a] ?// [$b, $a] | $a))]", "[[1],[2,3]]", "[[]]\n", "", 0),
    ("[combinations(([-1, 2] as [$a] ?// [$b, $a] | $a))]", "\"s\"", "[[]]\n", "", 0),
    ("[combinations(([1, \"y\"] as [$a] ?// [$b, $a] | $a))]", "[1,2]", "[[1],[2]]\n", "", 0),
    ("[combinations(([1, \"y\"] as [$a] ?// [$b, $a] | $a))]", "[[1],[2,3]]", "[[[1]],[[2,3]]]\n", "", 0),
    ("[combinations(([1, \"y\"] as [$a] ?// [$b, $a] | $a))]", "\"s\"", "", "jq: error (at <stdin>:1): Cannot iterate over string (\"s\")\n", 5),
    ("[combinations((\"x\",1) as $a ?// [$a] | $a)]", "[1,2]", "", "jq: error (at <stdin>:1): Cannot index string with number\n", 5),
    ("[combinations((\"x\",1,2) as [$a] ?// $a | $a)] | length", "[1,2]", "", "jq: error (at <stdin>:1): Range bounds must be numeric\n", 5),
    ("[combinations((1,2) as [$a] ?// $a | $a)] | length", "[1,2]", "8\n", "", 0),
    ("[combinations(1,2)] | length", "[1,2]", "8\n", "", 0),
    ("halt_error(([\"x\", 3] as [$a] ?// [$b, $a] | $a))", "\"s\"", "", "s", 3),
    ("try halt_error(([\"x\", 3] as [$a] ?// [$b, $a] | $a)) catch .", "\"s\"", "", "s", 3),
    ("halt_error(([\"x\", 3] as [$a] ?// [$b, $a] | $a))", "{\"a\":1}", "", "{\"a\":1}\n", 3),
    ("try halt_error(([\"x\", 3] as [$a] ?// [$b, $a] | $a)) catch .", "{\"a\":1}", "", "{\"a\":1}\n", 3),
    ("halt_error(([\"x\", \"y\"] as [$a] ?// [$b, $a] | $a))", "\"s\"", "", "jq: error (at <stdin>:1): string (\"s\") halt_error/1: number required\n", 5),
    ("try halt_error(([\"x\", \"y\"] as [$a] ?// [$b, $a] | $a)) catch .", "\"s\"", "\"string (\\\"s\\\") halt_error/1: number required\"\n", "", 0),
    ("halt_error(([\"x\", \"y\"] as [$a] ?// [$b, $a] | $a))", "{\"a\":1}", "", "jq: error (at <stdin>:1): object ({\"a\":1}) halt_error/1: number required\n", 5),
    ("try halt_error(([\"x\", \"y\"] as [$a] ?// [$b, $a] | $a)) catch .", "{\"a\":1}", "\"object ({\\\"a\\\":1}) halt_error/1: number required\"\n", "", 0),
    ("halt_error(([[1], 0] as [$a] ?// [$b, $a] | $a))", "\"s\"", "", "s", 0),
    ("try halt_error(([[1], 0] as [$a] ?// [$b, $a] | $a)) catch .", "\"s\"", "", "s", 0),
    ("halt_error(([[1], 0] as [$a] ?// [$b, $a] | $a))", "{\"a\":1}", "", "{\"a\":1}\n", 0),
    ("try halt_error(([[1], 0] as [$a] ?// [$b, $a] | $a)) catch .", "{\"a\":1}", "", "{\"a\":1}\n", 0),
    ("halt_error(([1, 2] as [$a] ?// [$b, $a] | $a))", "\"s\"", "", "s", 1),
    ("try halt_error(([1, 2] as [$a] ?// [$b, $a] | $a)) catch .", "\"s\"", "", "s", 1),
    ("halt_error(([1, 2] as [$a] ?// [$b, $a] | $a))", "{\"a\":1}", "", "{\"a\":1}\n", 1),
    ("try halt_error(([1, 2] as [$a] ?// [$b, $a] | $a)) catch .", "{\"a\":1}", "", "{\"a\":1}\n", 1),
    ("nth(([\"x\", 1] as [$a] ?// [$b, $a] | $a))", "[5,6]", "6\n", "", 0),
    ("has(([1, \"a\"] as [$a] ?// [$b, $a] | $a))", "{\"a\":1}", "true\n", "", 0),
    ("setpath([0]; ([1, 2] as [$a] ?// [$b, $a] | $a)) | if . == [1] then error(\"later\") else . end", "[1]", "[2]\n", "", 0),
    ("sub(([1, \",\"] as [$a] ?// [$b, $a] | $a); \"X\"; ([1, \"g\"] as [$a] ?// [$b, $a] | $a))", "\"a,b\"", "\"aXb\"\n", "", 0),
];

#[test]
fn a_failure_after_a_builtin_argument_retries_its_alternative_bind_7558() -> Result<()> {
    let mut failures = Vec::new();
    for &(program, input, stdout, stderr, code) in ROWS {
        let got = run_jq_full(&["-c", program], Some(&format!("{input}\n")))?;
        if got != (stdout.to_string(), stderr.to_string(), code) {
            failures.push(format!("{program} on {input}: got {got:?}"));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} rows differ from jq 1.7.1:\n{}",
        failures.len(),
        ROWS.len(),
        failures.join("\n")
    );
    Ok(())
}

/// The compile errors of a call whose argument the parser settles first are reported as for the
/// call as written: its operands are closures, compile units of their own (#3391).
/// `(program, stderr)`, every one exit 3, input `"a"`. jq 1.8.2 reports the same names, with a
/// caret under the source instead of the padded line, a 1.8 presentation change.
const COMPILE_ERRORS: &[(&str, &str)] = &[
    ("def t: tm; t, split(\",\"; ua1 as [$a] ?// $a | $a)", "jq: error: tm/0 is not defined at <top-level>, line 1:\ndef t: tm; t, split(\",\"; ua1 as [$a] ?// $a | $a)       \njq: error: ua1/0 is not defined at <top-level>, line 1:\ndef t: tm; t, split(\",\"; ua1 as [$a] ?// $a | $a)                         \njq: 2 compile errors\n"),
    ("def t: tm; t, [splits(\",\"; ua1 as [$a] ?// $a | $a)]", "jq: error: tm/0 is not defined at <top-level>, line 1:\ndef t: tm; t, [splits(\",\"; ua1 as [$a] ?// $a | $a)]       \njq: error: ua1/0 is not defined at <top-level>, line 1:\ndef t: tm; t, [splits(\",\"; ua1 as [$a] ?// $a | $a)]                           \njq: 2 compile errors\n"),
    ("def t: tm; t, combinations(ua1 as [$a] ?// $a | $a)", "jq: error: tm/0 is not defined at <top-level>, line 1:\ndef t: tm; t, combinations(ua1 as [$a] ?// $a | $a)       \njq: error: ua1/0 is not defined at <top-level>, line 1:\ndef t: tm; t, combinations(ua1 as [$a] ?// $a | $a)                           \njq: 2 compile errors\n"),
    ("split(ub; ua1 as [$a] ?// $a | $a), tm", "jq: error: tm/0 is not defined at <top-level>, line 1:\nsplit(ub; ua1 as [$a] ?// $a | $a), tm                                    \njq: 1 compile error\n"),
    ("combinations(ua1 as [$a] ?// $a | $a), tm", "jq: error: tm/0 is not defined at <top-level>, line 1:\ncombinations(ua1 as [$a] ?// $a | $a), tm                                       \njq: 1 compile error\n"),
    ("def t: tm; t, in(ua1 as [$a] ?// $a | $a)", "jq: error: tm/0 is not defined at <top-level>, line 1:\ndef t: tm; t, in(ua1 as [$a] ?// $a | $a)       \njq: error: ua1/0 is not defined at <top-level>, line 1:\ndef t: tm; t, in(ua1 as [$a] ?// $a | $a)                 \njq: 2 compile errors\n"),
    ("in(ua1 as [$a] ?// $a | $a), tm", "jq: error: tm/0 is not defined at <top-level>, line 1:\nin(ua1 as [$a] ?// $a | $a), tm                             \njq: 1 compile error\n"),
    ("split($y; ($x as [$a] ?// $a | $a))", "jq: error: $y is not defined at <top-level>, line 1:\nsplit($y; ($x as [$a] ?// $a | $a))      \njq: error: $x is not defined at <top-level>, line 1:\nsplit($y; ($x as [$a] ?// $a | $a))           \njq: 2 compile errors\n"),
    ("in($x as [$a] ?// $a | $a)", "jq: error: $x is not defined at <top-level>, line 1:\nin($x as [$a] ?// $a | $a)   \njq: 1 compile error\n"),
    ("def t: tm; t, in(ua1)", "jq: error: tm/0 is not defined at <top-level>, line 1:\ndef t: tm; t, in(ua1)       \njq: error: ua1/0 is not defined at <top-level>, line 1:\ndef t: tm; t, in(ua1)                 \njq: 2 compile errors\n"),
    ("def t: tm; t, halt_error(ua1 as [$a] ?// $a | $a)", "jq: error: ua1/0 is not defined at <top-level>, line 1:\ndef t: tm; t, halt_error(ua1 as [$a] ?// $a | $a)                         \njq: 1 compile error\n"),
];

#[test]
fn a_settled_argument_reports_compile_errors_as_the_call_written_7558() -> Result<()> {
    for &(program, stderr) in COMPILE_ERRORS {
        assert_eq!(
            run_jq_full(&["-c", program], Some("\"a\"\n"))?,
            (String::new(), stderr.to_string(), 3),
            "{program}"
        );
    }
    Ok(())
}
