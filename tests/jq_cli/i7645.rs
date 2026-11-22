//! #7645: a data import whose metadata says `raw: true` (`import "r" as $r {raw: true};`) binds
//! `$r` to the file's text, one string, as jq 1.7.1 and 1.8.2 do, where it was parsed as JSON: a
//! file that was not JSON was `error loading data file` (exit 3), and one that was imported as
//! `[value, ...]`. jq's `process_dependencies` reads `raw` as raw only when it is `true` itself, and
//! reads the file then as `--rawfile` reads its file (`jv_load_file(path, 1)`, #7436).
//!
//! Every expectation was captured from jq 1.7.1 (`/usr/bin/jq`); jq 1.8.2 agrees on every row. A
//! row with exit 3 is the parse error of a file read as JSON, whose wording is #2703's recorded
//! divergence, so only its first words are pinned.

use anyhow::Result;

/// (file, the import's metadata, which access, jq's stdout, jq's exit status). The three files at
/// the 4,096-byte read boundary run only with `raw` true: read as JSON they are not this issue's.
const ROWS: &[(usize, &str, usize, &str, i32)] = &[
    (
        0,
        "{raw: true}",
        0,
        "[\"string\",7,[32,97,98,99,100,101,102]]\n",
        0,
    ),
    (0, "{raw: true}", 1, "[\"string\",7]\n", 0),
    (
        0,
        "{\"raw\": true}",
        0,
        "[\"string\",7,[32,97,98,99,100,101,102]]\n",
        0,
    ),
    (0, "{\"raw\": true}", 1, "[\"string\",7]\n", 0),
    (
        0,
        "{raw: false, raw: true}",
        0,
        "[\"string\",7,[32,97,98,99,100,101,102]]\n",
        0,
    ),
    (0, "{raw: false, raw: true}", 1, "[\"string\",7]\n", 0),
    (
        0,
        "{raw: (1 == 1)}",
        0,
        "[\"string\",7,[32,97,98,99,100,101,102]]\n",
        0,
    ),
    (0, "{raw: (1 == 1)}", 1, "[\"string\",7]\n", 0),
    (
        0,
        "{raw: true, search: \".\"}",
        0,
        "[\"string\",7,[32,97,98,99,100,101,102]]\n",
        0,
    ),
    (0, "{raw: true, search: \".\"}", 1, "[\"string\",7]\n", 0),
    (0, "", 0, "", 3),
    (0, "", 1, "", 3),
    (0, "{raw: false}", 0, "", 3),
    (0, "{raw: false}", 1, "", 3),
    (0, "{raw: null}", 0, "", 3),
    (0, "{raw: null}", 1, "", 3),
    (0, "{raw: 1}", 0, "", 3),
    (0, "{raw: 1}", 1, "", 3),
    (0, "{raw: \"true\"}", 0, "", 3),
    (0, "{raw: \"true\"}", 1, "", 3),
    (0, "{raw: [true]}", 0, "", 3),
    (0, "{raw: [true]}", 1, "", 3),
    (0, "{raw: true, raw: false}", 0, "", 3),
    (0, "{raw: true, raw: false}", 1, "", 3),
    (1, "{raw: true}", 0, "[\"string\",3,[49,91,93]]\n", 0),
    (1, "{raw: true}", 1, "[\"string\",3]\n", 0),
    (1, "{\"raw\": true}", 0, "[\"string\",3,[49,91,93]]\n", 0),
    (1, "{\"raw\": true}", 1, "[\"string\",3]\n", 0),
    (
        1,
        "{raw: false, raw: true}",
        0,
        "[\"string\",3,[49,91,93]]\n",
        0,
    ),
    (1, "{raw: false, raw: true}", 1, "[\"string\",3]\n", 0),
    (1, "{raw: (1 == 1)}", 0, "[\"string\",3,[49,91,93]]\n", 0),
    (1, "{raw: (1 == 1)}", 1, "[\"string\",3]\n", 0),
    (
        1,
        "{raw: true, search: \".\"}",
        0,
        "[\"string\",3,[49,91,93]]\n",
        0,
    ),
    (1, "{raw: true, search: \".\"}", 1, "[\"string\",3]\n", 0),
    (1, "", 0, "[\"array\",1,[[1]]]\n", 0),
    (1, "", 1, "[\"array\",1]\n", 0),
    (1, "{raw: false}", 0, "[\"array\",1,[[1]]]\n", 0),
    (1, "{raw: false}", 1, "[\"array\",1]\n", 0),
    (1, "{raw: null}", 0, "[\"array\",1,[[1]]]\n", 0),
    (1, "{raw: null}", 1, "[\"array\",1]\n", 0),
    (1, "{raw: 1}", 0, "[\"array\",1,[[1]]]\n", 0),
    (1, "{raw: 1}", 1, "[\"array\",1]\n", 0),
    (1, "{raw: \"true\"}", 0, "[\"array\",1,[[1]]]\n", 0),
    (1, "{raw: \"true\"}", 1, "[\"array\",1]\n", 0),
    (1, "{raw: [true]}", 0, "[\"array\",1,[[1]]]\n", 0),
    (1, "{raw: [true]}", 1, "[\"array\",1]\n", 0),
    (1, "{raw: true, raw: false}", 0, "[\"array\",1,[[1]]]\n", 0),
    (1, "{raw: true, raw: false}", 1, "[\"array\",1]\n", 0),
    (
        2,
        "{raw: true}",
        0,
        "[\"string\",9,[32,34,49,50,58,107,123,125]]\n",
        0,
    ),
    (2, "{raw: true}", 1, "[\"string\",9]\n", 0),
    (
        2,
        "{\"raw\": true}",
        0,
        "[\"string\",9,[32,34,49,50,58,107,123,125]]\n",
        0,
    ),
    (2, "{\"raw\": true}", 1, "[\"string\",9]\n", 0),
    (
        2,
        "{raw: false, raw: true}",
        0,
        "[\"string\",9,[32,34,49,50,58,107,123,125]]\n",
        0,
    ),
    (2, "{raw: false, raw: true}", 1, "[\"string\",9]\n", 0),
    (
        2,
        "{raw: (1 == 1)}",
        0,
        "[\"string\",9,[32,34,49,50,58,107,123,125]]\n",
        0,
    ),
    (2, "{raw: (1 == 1)}", 1, "[\"string\",9]\n", 0),
    (
        2,
        "{raw: true, search: \".\"}",
        0,
        "[\"string\",9,[32,34,49,50,58,107,123,125]]\n",
        0,
    ),
    (2, "{raw: true, search: \".\"}", 1, "[\"string\",9]\n", 0),
    (2, "", 0, "[\"array\",2,[{\"k\":1},2]]\n", 0),
    (2, "", 1, "[\"array\",2]\n", 0),
    (2, "{raw: false}", 0, "[\"array\",2,[{\"k\":1},2]]\n", 0),
    (2, "{raw: false}", 1, "[\"array\",2]\n", 0),
    (2, "{raw: null}", 0, "[\"array\",2,[{\"k\":1},2]]\n", 0),
    (2, "{raw: null}", 1, "[\"array\",2]\n", 0),
    (2, "{raw: 1}", 0, "[\"array\",2,[{\"k\":1},2]]\n", 0),
    (2, "{raw: 1}", 1, "[\"array\",2]\n", 0),
    (2, "{raw: \"true\"}", 0, "[\"array\",2,[{\"k\":1},2]]\n", 0),
    (2, "{raw: \"true\"}", 1, "[\"array\",2]\n", 0),
    (2, "{raw: [true]}", 0, "[\"array\",2,[{\"k\":1},2]]\n", 0),
    (2, "{raw: [true]}", 1, "[\"array\",2]\n", 0),
    (
        2,
        "{raw: true, raw: false}",
        0,
        "[\"array\",2,[{\"k\":1},2]]\n",
        0,
    ),
    (2, "{raw: true, raw: false}", 1, "[\"array\",2]\n", 0),
    (3, "{raw: true}", 0, "[\"string\",0,[]]\n", 0),
    (3, "{raw: true}", 1, "[\"string\",0]\n", 0),
    (3, "{\"raw\": true}", 0, "[\"string\",0,[]]\n", 0),
    (3, "{\"raw\": true}", 1, "[\"string\",0]\n", 0),
    (3, "{raw: false, raw: true}", 0, "[\"string\",0,[]]\n", 0),
    (3, "{raw: false, raw: true}", 1, "[\"string\",0]\n", 0),
    (3, "{raw: (1 == 1)}", 0, "[\"string\",0,[]]\n", 0),
    (3, "{raw: (1 == 1)}", 1, "[\"string\",0]\n", 0),
    (3, "{raw: true, search: \".\"}", 0, "[\"string\",0,[]]\n", 0),
    (3, "{raw: true, search: \".\"}", 1, "[\"string\",0]\n", 0),
    (3, "", 0, "[\"array\",0,[]]\n", 0),
    (3, "", 1, "[\"array\",0]\n", 0),
    (3, "{raw: false}", 0, "[\"array\",0,[]]\n", 0),
    (3, "{raw: false}", 1, "[\"array\",0]\n", 0),
    (3, "{raw: null}", 0, "[\"array\",0,[]]\n", 0),
    (3, "{raw: null}", 1, "[\"array\",0]\n", 0),
    (3, "{raw: 1}", 0, "[\"array\",0,[]]\n", 0),
    (3, "{raw: 1}", 1, "[\"array\",0]\n", 0),
    (3, "{raw: \"true\"}", 0, "[\"array\",0,[]]\n", 0),
    (3, "{raw: \"true\"}", 1, "[\"array\",0]\n", 0),
    (3, "{raw: [true]}", 0, "[\"array\",0,[]]\n", 0),
    (3, "{raw: [true]}", 1, "[\"array\",0]\n", 0),
    (3, "{raw: true, raw: false}", 0, "[\"array\",0,[]]\n", 0),
    (3, "{raw: true, raw: false}", 1, "[\"array\",0]\n", 0),
    (4, "{raw: true}", 0, "[\"string\",2,[10,120]]\n", 0),
    (4, "{raw: true}", 1, "[\"string\",2]\n", 0),
    (4, "{\"raw\": true}", 0, "[\"string\",2,[10,120]]\n", 0),
    (4, "{\"raw\": true}", 1, "[\"string\",2]\n", 0),
    (
        4,
        "{raw: false, raw: true}",
        0,
        "[\"string\",2,[10,120]]\n",
        0,
    ),
    (4, "{raw: false, raw: true}", 1, "[\"string\",2]\n", 0),
    (4, "{raw: (1 == 1)}", 0, "[\"string\",2,[10,120]]\n", 0),
    (4, "{raw: (1 == 1)}", 1, "[\"string\",2]\n", 0),
    (
        4,
        "{raw: true, search: \".\"}",
        0,
        "[\"string\",2,[10,120]]\n",
        0,
    ),
    (4, "{raw: true, search: \".\"}", 1, "[\"string\",2]\n", 0),
    (4, "", 0, "", 3),
    (4, "", 1, "", 3),
    (4, "{raw: false}", 0, "", 3),
    (4, "{raw: false}", 1, "", 3),
    (4, "{raw: null}", 0, "", 3),
    (4, "{raw: null}", 1, "", 3),
    (4, "{raw: 1}", 0, "", 3),
    (4, "{raw: 1}", 1, "", 3),
    (4, "{raw: \"true\"}", 0, "", 3),
    (4, "{raw: \"true\"}", 1, "", 3),
    (4, "{raw: [true]}", 0, "", 3),
    (4, "{raw: [true]}", 1, "", 3),
    (4, "{raw: true, raw: false}", 0, "", 3),
    (4, "{raw: true, raw: false}", 1, "", 3),
    (5, "{raw: true}", 0, "[\"string\",3,[0,97,98]]\n", 0),
    (5, "{raw: true}", 1, "[\"string\",3]\n", 0),
    (5, "{\"raw\": true}", 0, "[\"string\",3,[0,97,98]]\n", 0),
    (5, "{\"raw\": true}", 1, "[\"string\",3]\n", 0),
    (
        5,
        "{raw: false, raw: true}",
        0,
        "[\"string\",3,[0,97,98]]\n",
        0,
    ),
    (5, "{raw: false, raw: true}", 1, "[\"string\",3]\n", 0),
    (5, "{raw: (1 == 1)}", 0, "[\"string\",3,[0,97,98]]\n", 0),
    (5, "{raw: (1 == 1)}", 1, "[\"string\",3]\n", 0),
    (
        5,
        "{raw: true, search: \".\"}",
        0,
        "[\"string\",3,[0,97,98]]\n",
        0,
    ),
    (5, "{raw: true, search: \".\"}", 1, "[\"string\",3]\n", 0),
    (5, "", 0, "", 3),
    (5, "", 1, "", 3),
    (5, "{raw: false}", 0, "", 3),
    (5, "{raw: false}", 1, "", 3),
    (5, "{raw: null}", 0, "", 3),
    (5, "{raw: null}", 1, "", 3),
    (5, "{raw: 1}", 0, "", 3),
    (5, "{raw: 1}", 1, "", 3),
    (5, "{raw: \"true\"}", 0, "", 3),
    (5, "{raw: \"true\"}", 1, "", 3),
    (5, "{raw: [true]}", 0, "", 3),
    (5, "{raw: [true]}", 1, "", 3),
    (5, "{raw: true, raw: false}", 0, "", 3),
    (5, "{raw: true, raw: false}", 1, "", 3),
    (6, "{raw: true}", 0, "[\"string\",5,[97,98,65533]]\n", 0),
    (6, "{raw: true}", 1, "[\"string\",5]\n", 0),
    (6, "{\"raw\": true}", 0, "[\"string\",5,[97,98,65533]]\n", 0),
    (6, "{\"raw\": true}", 1, "[\"string\",5]\n", 0),
    (
        6,
        "{raw: false, raw: true}",
        0,
        "[\"string\",5,[97,98,65533]]\n",
        0,
    ),
    (6, "{raw: false, raw: true}", 1, "[\"string\",5]\n", 0),
    (6, "{raw: (1 == 1)}", 0, "[\"string\",5,[97,98,65533]]\n", 0),
    (6, "{raw: (1 == 1)}", 1, "[\"string\",5]\n", 0),
    (
        6,
        "{raw: true, search: \".\"}",
        0,
        "[\"string\",5,[97,98,65533]]\n",
        0,
    ),
    (6, "{raw: true, search: \".\"}", 1, "[\"string\",5]\n", 0),
    (6, "", 0, "", 3),
    (6, "", 1, "", 3),
    (6, "{raw: false}", 0, "", 3),
    (6, "{raw: false}", 1, "", 3),
    (6, "{raw: null}", 0, "", 3),
    (6, "{raw: null}", 1, "", 3),
    (6, "{raw: 1}", 0, "", 3),
    (6, "{raw: 1}", 1, "", 3),
    (6, "{raw: \"true\"}", 0, "", 3),
    (6, "{raw: \"true\"}", 1, "", 3),
    (6, "{raw: [true]}", 0, "", 3),
    (6, "{raw: [true]}", 1, "", 3),
    (6, "{raw: true, raw: false}", 0, "", 3),
    (6, "{raw: true, raw: false}", 1, "", 3),
    (7, "{raw: true}", 0, "[\"string\",3,[34,65533]]\n", 0),
    (7, "{raw: true}", 1, "[\"string\",3]\n", 0),
    (7, "{\"raw\": true}", 0, "[\"string\",3,[34,65533]]\n", 0),
    (7, "{\"raw\": true}", 1, "[\"string\",3]\n", 0),
    (
        7,
        "{raw: false, raw: true}",
        0,
        "[\"string\",3,[34,65533]]\n",
        0,
    ),
    (7, "{raw: false, raw: true}", 1, "[\"string\",3]\n", 0),
    (7, "{raw: (1 == 1)}", 0, "[\"string\",3,[34,65533]]\n", 0),
    (7, "{raw: (1 == 1)}", 1, "[\"string\",3]\n", 0),
    (
        7,
        "{raw: true, search: \".\"}",
        0,
        "[\"string\",3,[34,65533]]\n",
        0,
    ),
    (7, "{raw: true, search: \".\"}", 1, "[\"string\",3]\n", 0),
    (7, "", 0, "[\"array\",1,[\"�\"]]\n", 0),
    (7, "", 1, "[\"array\",1]\n", 0),
    (7, "{raw: false}", 0, "[\"array\",1,[\"�\"]]\n", 0),
    (7, "{raw: false}", 1, "[\"array\",1]\n", 0),
    (7, "{raw: null}", 0, "[\"array\",1,[\"�\"]]\n", 0),
    (7, "{raw: null}", 1, "[\"array\",1]\n", 0),
    (7, "{raw: 1}", 0, "[\"array\",1,[\"�\"]]\n", 0),
    (7, "{raw: 1}", 1, "[\"array\",1]\n", 0),
    (7, "{raw: \"true\"}", 0, "[\"array\",1,[\"�\"]]\n", 0),
    (7, "{raw: \"true\"}", 1, "[\"array\",1]\n", 0),
    (7, "{raw: [true]}", 0, "[\"array\",1,[\"�\"]]\n", 0),
    (7, "{raw: [true]}", 1, "[\"array\",1]\n", 0),
    (
        7,
        "{raw: true, raw: false}",
        0,
        "[\"array\",1,[\"�\"]]\n",
        0,
    ),
    (7, "{raw: true, raw: false}", 1, "[\"array\",1]\n", 0),
    (8, "{raw: true}", 0, "[\"string\",4,[50,91,93,65279]]\n", 0),
    (8, "{raw: true}", 1, "[\"string\",4]\n", 0),
    (
        8,
        "{\"raw\": true}",
        0,
        "[\"string\",4,[50,91,93,65279]]\n",
        0,
    ),
    (8, "{\"raw\": true}", 1, "[\"string\",4]\n", 0),
    (
        8,
        "{raw: false, raw: true}",
        0,
        "[\"string\",4,[50,91,93,65279]]\n",
        0,
    ),
    (8, "{raw: false, raw: true}", 1, "[\"string\",4]\n", 0),
    (
        8,
        "{raw: (1 == 1)}",
        0,
        "[\"string\",4,[50,91,93,65279]]\n",
        0,
    ),
    (8, "{raw: (1 == 1)}", 1, "[\"string\",4]\n", 0),
    (
        8,
        "{raw: true, search: \".\"}",
        0,
        "[\"string\",4,[50,91,93,65279]]\n",
        0,
    ),
    (8, "{raw: true, search: \".\"}", 1, "[\"string\",4]\n", 0),
    (8, "", 0, "[\"array\",1,[[2]]]\n", 0),
    (8, "", 1, "[\"array\",1]\n", 0),
    (8, "{raw: false}", 0, "[\"array\",1,[[2]]]\n", 0),
    (8, "{raw: false}", 1, "[\"array\",1]\n", 0),
    (8, "{raw: null}", 0, "[\"array\",1,[[2]]]\n", 0),
    (8, "{raw: null}", 1, "[\"array\",1]\n", 0),
    (8, "{raw: 1}", 0, "[\"array\",1,[[2]]]\n", 0),
    (8, "{raw: 1}", 1, "[\"array\",1]\n", 0),
    (8, "{raw: \"true\"}", 0, "[\"array\",1,[[2]]]\n", 0),
    (8, "{raw: \"true\"}", 1, "[\"array\",1]\n", 0),
    (8, "{raw: [true]}", 0, "[\"array\",1,[[2]]]\n", 0),
    (8, "{raw: [true]}", 1, "[\"array\",1]\n", 0),
    (8, "{raw: true, raw: false}", 0, "[\"array\",1,[[2]]]\n", 0),
    (8, "{raw: true, raw: false}", 1, "[\"array\",1]\n", 0),
    (9, "{raw: true}", 0, "[\"string\",4097,[33,97,233]]\n", 0),
    (9, "{raw: true}", 1, "[\"string\",4097]\n", 0),
    (
        9,
        "{\"raw\": true}",
        0,
        "[\"string\",4097,[33,97,233]]\n",
        0,
    ),
    (9, "{\"raw\": true}", 1, "[\"string\",4097]\n", 0),
    (
        9,
        "{raw: false, raw: true}",
        0,
        "[\"string\",4097,[33,97,233]]\n",
        0,
    ),
    (9, "{raw: false, raw: true}", 1, "[\"string\",4097]\n", 0),
    (
        9,
        "{raw: (1 == 1)}",
        0,
        "[\"string\",4097,[33,97,233]]\n",
        0,
    ),
    (9, "{raw: (1 == 1)}", 1, "[\"string\",4097]\n", 0),
    (
        9,
        "{raw: true, search: \".\"}",
        0,
        "[\"string\",4097,[33,97,233]]\n",
        0,
    ),
    (9, "{raw: true, search: \".\"}", 1, "[\"string\",4097]\n", 0),
    (10, "{raw: true}", 0, "[\"string\",4097,[33,97,65533]]\n", 0),
    (10, "{raw: true}", 1, "[\"string\",4097]\n", 0),
    (
        10,
        "{\"raw\": true}",
        0,
        "[\"string\",4097,[33,97,65533]]\n",
        0,
    ),
    (10, "{\"raw\": true}", 1, "[\"string\",4097]\n", 0),
    (
        10,
        "{raw: false, raw: true}",
        0,
        "[\"string\",4097,[33,97,65533]]\n",
        0,
    ),
    (10, "{raw: false, raw: true}", 1, "[\"string\",4097]\n", 0),
    (
        10,
        "{raw: (1 == 1)}",
        0,
        "[\"string\",4097,[33,97,65533]]\n",
        0,
    ),
    (10, "{raw: (1 == 1)}", 1, "[\"string\",4097]\n", 0),
    (
        10,
        "{raw: true, search: \".\"}",
        0,
        "[\"string\",4097,[33,97,65533]]\n",
        0,
    ),
    (
        10,
        "{raw: true, search: \".\"}",
        1,
        "[\"string\",4097]\n",
        0,
    ),
    (
        11,
        "{raw: true}",
        0,
        "[\"string\",4096,[97,8364,65533]]\n",
        0,
    ),
    (11, "{raw: true}", 1, "[\"string\",4096]\n", 0),
    (
        11,
        "{\"raw\": true}",
        0,
        "[\"string\",4096,[97,8364,65533]]\n",
        0,
    ),
    (11, "{\"raw\": true}", 1, "[\"string\",4096]\n", 0),
    (
        11,
        "{raw: false, raw: true}",
        0,
        "[\"string\",4096,[97,8364,65533]]\n",
        0,
    ),
    (11, "{raw: false, raw: true}", 1, "[\"string\",4096]\n", 0),
    (
        11,
        "{raw: (1 == 1)}",
        0,
        "[\"string\",4096,[97,8364,65533]]\n",
        0,
    ),
    (11, "{raw: (1 == 1)}", 1, "[\"string\",4096]\n", 0),
    (
        11,
        "{raw: true, search: \".\"}",
        0,
        "[\"string\",4096,[97,8364,65533]]\n",
        0,
    ),
    (
        11,
        "{raw: true, search: \".\"}",
        1,
        "[\"string\",4096]\n",
        0,
    ),
];

fn contents() -> Vec<Vec<u8>> {
    vec![
        b"abc def".to_vec(),
        b"[1]".to_vec(),
        b"{\"k\":1} 2".to_vec(),
        Vec::new(),
        b"x\n".to_vec(),
        b"a\0b".to_vec(),
        b"\xff\xfeab\xc3".to_vec(),
        b"\"\xff\"".to_vec(),
        b"\xef\xbb\xbf[2]".to_vec(),
        [vec![b'a'; 4095], "é!".as_bytes().to_vec()].concat(),
        [vec![b'a'; 4095], b"\xe2\x82!".to_vec()].concat(),
        [vec![b'a'; 4094], b"\xe2\x82\xac\xff".to_vec()].concat(),
    ]
}
const ACCESSES: [&str; 2] = [
    "$r | [type, length, (if type == \"string\" then explode | unique else . end)]",
    "$r::r | [type, length]",
];

fn run(dir: &std::path::Path, args: &[&str]) -> Result<(String, String, i32)> {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .arg("jq")
        .args(args)
        .current_dir(dir)
        .env("LC_ALL", "C")
        .output()?;
    Ok((
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code().unwrap_or(-1),
    ))
}

#[test]
fn a_raw_data_import_binds_the_files_text_7645() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let contents = contents();
    for &(file, meta, access, want, status) in ROWS {
        std::fs::write(dir.path().join("r.json"), &contents[file])?;
        let program = format!("import \"r\" as $r {meta}; {}", ACCESSES[access]);
        let got = run(dir.path(), &["-nc", "-L", ".", &program])?;
        assert_eq!(
            (got.0.as_str(), got.2),
            (want, status),
            "file {file}, {program}: {}",
            got.1
        );
        if status == 0 {
            assert_eq!(got.1, "", "file {file}, {program}");
        } else {
            assert!(
                got.1.starts_with("jq: error loading data file "),
                "file {file}, {program}: {}",
                got.1
            );
        }
    }
    Ok(())
}

/// The issue's own two files, a raw import inside a module, the same file imported raw and parsed
/// (only the parsed import fails: jq reads each data import on its own), and `raw` on a library
/// import or include, where it changes nothing.
#[test]
fn a_raw_data_import_in_the_issues_shapes_7645() -> Result<()> {
    let dir = tempfile::tempdir()?;
    std::fs::write(dir.path().join("r.json"), "abc def")?;
    std::fs::write(dir.path().join("a.json"), "[1]")?;
    std::fs::write(dir.path().join("lib.jq"), "def f: 1;")?;
    std::fs::write(
        dir.path().join("m.jq"),
        "import \"r\" as $r {raw: true}; def g: $r;",
    )?;
    let ok = |program: &str, want: &str| -> Result<()> {
        let got = run(dir.path(), &["-nc", "-L", ".", program])?;
        assert_eq!(
            (got.0.as_str(), got.1.as_str(), got.2),
            (want, "", 0),
            "{program}"
        );
        Ok(())
    };
    ok("import \"r\" as $r {raw: true}; $r", "\"abc def\"\n")?;
    ok("import \"a\" as $r {raw: true}; $r", "\"[1]\"\n")?;
    ok(
        "import \"r\" as $r {raw: true}; [$r, $r::r]",
        "[\"abc def\",\"abc def\"]\n",
    )?;
    ok(
        "import \"r\" as $r {raw: true}; import \"a\" as $a; [$r, $a]",
        "[\"abc def\",[[1]]]\n",
    )?;
    ok("import \"m\" as m; m::g", "\"abc def\"\n")?;
    ok("import \"lib\" as l {raw: true}; l::f", "1\n")?;
    ok("include \"lib\" {raw: true}; f", "1\n")?;
    ok(
        "\"m\" | modulemeta",
        "{\"deps\":[{\"raw\":true,\"as\":\"r\",\"is_data\":true,\"relpath\":\"r\"}],\"defs\":[\"g/0\"]}\n",
    )?;

    let got = run(
        dir.path(),
        &[
            "-nc",
            "-L",
            ".",
            "import \"r\" as $r {raw: true}; import \"r\" as $q; [$r]",
        ],
    )?;
    assert_eq!((got.0.as_str(), got.2), ("", 3));
    assert_eq!(
        got.1.matches("jq: error loading data file ").count(),
        1,
        "{}",
        got.1
    );
    assert!(got.1.ends_with("\njq: 1 compile error\n"), "{}", got.1);
    Ok(())
}
