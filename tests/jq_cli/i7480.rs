//! #7480: a program reading `input`/`inputs` over several files, with a document cut across a file's end,
//! reads the files the way it reads them with no cut: the cut document is carried onto the front of the
//! next file and the run goes on reading a window at a time, where the rest of the stream was handed to
//! the eager reader, which turned every later document into a value before the first was read (995 MiB
//! for 100 MB). Every row was captured from jq 1.7.1 on the same files (jq 1.8.2 answers the same, but
//! for the read past the end after the last row's fault, which names the last file there). That the run
//! hands nothing over and holds a window is pinned in `jq_runner.rs`
//! (`a_document_cut_across_files_is_carried_on_lazily_7480`); these rows pin what it reads.

use anyhow::Result;

use crate::spawn_jq;

/// Records in the stream: about 1.3 MB, past the 1 MiB from which several files are read a window at a
/// time.
const RECORDS: usize = 40_000;

/// Record `i`: one in seven an object over four lines, one in eleven a bare number, the rest an object on
/// one line.
fn record(i: usize) -> Vec<u8> {
    if i % 7 == 3 {
        format!(
            "{{\"id\":{i},\n \"p\":[1,\n2],\n \"s\":\"{}\"}}\n",
            "y".repeat(i % 13)
        )
    } else if i % 11 == 5 {
        format!("{i}\n")
    } else {
        format!("{{\"id\":{i},\"s\":\"{}\"}}\n", "x".repeat(i % 50))
    }
    .into_bytes()
}

/// Where a file ends: `At(record, byte)` that many bytes into a record, or a file of its own text
/// between two others.
enum Cut {
    At(usize, usize),
    Blank(&'static [u8]),
}

struct Row {
    name: &'static str,
    cuts: &'static [Cut],
    /// Records replaced, to put a fault near a cut.
    corrupt: &'static [(usize, &'static [u8])],
    /// [`COUNTED`]'s stdout, stderr and exit status under jq 1.7.1.
    counted: (&'static str, &'static str, i32),
    /// [`READ_PAST`]'s.
    read_past: (&'static str, &'static str, i32),
}

/// Every input, and where the stream ends.
const COUNTED: &str = "reduce inputs as $x (0; . + 1), input_line_number, input_filename";

/// The records around each cut and every fault, each with its file and line, reading past the faults.
const READ_PAST: &str = "[label $done | foreach range(40010) as $k (null; try input catch (\"E:\" + .)) | (if . == \"E:break\" then (., break $done) else . end) | (if type == \"object\" then .id else . end) as $i | select(if type == \"string\" then true else ($cuts | any(. - 1 <= $i and $i <= . + 1)) end) | [$i, input_filename, input_line_number]]";

const ROWS: &[Row] = &[
    Row {
        name: "issue",
        cuts: &[Cut::At(10000, 7), Cut::At(25000, 13)],
        corrupt: &[],
        counted: ("40000\n21428\n\"f2.json\"\n", "", 0),
        read_past: ("[[9999,\"f0.json\",14287],[10000,\"f1.json\",1],[10001,\"f1.json\",2],[24999,\"f1.json\",21426],[25000,\"f2.json\",3],[25001,\"f2.json\",4],[\"E:break\",\"f2.json\",21428]]\n", "", 0),
    },
    Row {
        name: "pretty",
        cuts: &[Cut::At(20002, 12)],
        corrupt: &[],
        counted: ("40000\n28569\n\"f1.json\"\n", "", 0),
        read_past: ("[[20001,\"f0.json\",28573],[20002,\"f1.json\",4],[20003,\"f1.json\",5],[\"E:break\",\"f1.json\",28569]]\n", "", 0),
    },
    Row {
        name: "pretty_after_newline",
        cuts: &[Cut::At(20002, 13)],
        corrupt: &[],
        counted: ("40000\n28568\n\"f1.json\"\n", "", 0),
        read_past: ("[[20001,\"f0.json\",28573],[20002,\"f1.json\",3],[20003,\"f1.json\",4],[\"E:break\",\"f1.json\",28568]]\n", "", 0),
    },
    Row {
        name: "bare_number",
        cuts: &[Cut::At(15009, 2)],
        corrupt: &[],
        counted: ("40000\n35701\n\"f1.json\"\n", "", 0),
        read_past: ("[[15008,\"f0.json\",21441],[15009,\"f1.json\",1],[15010,\"f1.json\",2],[\"E:break\",\"f1.json\",35701]]\n", "", 0),
    },
    Row {
        name: "bare_number_whole",
        cuts: &[Cut::At(15009, 0), Cut::At(15010, 0)],
        corrupt: &[],
        counted: ("40000\n35700\n\"f2.json\"\n", "", 0),
        read_past: ("[[15008,\"f0.json\",21441],[15009,\"f1.json\",1],[15010,\"f2.json\",1],[15011,\"f2.json\",5],[\"E:break\",\"f2.json\",35700]]\n", "", 0),
    },
    Row {
        name: "number_then_newline",
        cuts: &[Cut::At(15009, 5)],
        corrupt: &[],
        counted: ("40000\n35701\n\"f1.json\"\n", "", 0),
        read_past: ("[[15008,\"f0.json\",21441],[15009,\"f1.json\",1],[15010,\"f1.json\",2],[\"E:break\",\"f1.json\",35701]]\n", "", 0),
    },
    Row {
        name: "opener_only",
        cuts: &[Cut::At(30021, 1)],
        corrupt: &[],
        counted: ("40000\n14254\n\"f1.json\"\n", "", 0),
        read_past: ("[[30020,\"f0.json\",42888],[30021,\"f1.json\",1],[30022,\"f1.json\",2],[\"E:break\",\"f1.json\",14254]]\n", "", 0),
    },
    Row {
        name: "in_string",
        cuts: &[Cut::At(30021, 20)],
        corrupt: &[],
        counted: ("40000\n14254\n\"f1.json\"\n", "", 0),
        read_past: ("[[30020,\"f0.json\",42888],[30021,\"f1.json\",1],[30022,\"f1.json\",2],[\"E:break\",\"f1.json\",14254]]\n", "", 0),
    },
    Row {
        name: "before_close",
        cuts: &[Cut::At(30021, 39)],
        corrupt: &[],
        counted: ("40000\n14254\n\"f1.json\"\n", "", 0),
        read_past: ("[[30020,\"f0.json\",42888],[30021,\"f1.json\",1],[30022,\"f1.json\",2],[\"E:break\",\"f1.json\",14254]]\n", "", 0),
    },
    Row {
        name: "before_newline",
        cuts: &[Cut::At(30021, 40)],
        corrupt: &[],
        counted: ("40000\n14254\n\"f1.json\"\n", "", 0),
        read_past: ("[[30020,\"f0.json\",42888],[30021,\"f0.json\",42888],[30022,\"f1.json\",2],[\"E:break\",\"f1.json\",14254]]\n", "", 0),
    },
    Row {
        name: "all_tail_file",
        cuts: &[Cut::At(30021, 3), Cut::At(30021, 9), Cut::At(30021, 15)],
        corrupt: &[],
        counted: ("40000\n14254\n\"f3.json\"\n", "", 0),
        read_past: ("[[30020,\"f0.json\",42888],[30021,\"f3.json\",1],[30022,\"f3.json\",2],[\"E:break\",\"f3.json\",14254]]\n", "", 0),
    },
    Row {
        name: "empty_file",
        cuts: &[Cut::At(30021, 4), Cut::At(30021, 4)],
        corrupt: &[],
        counted: ("40000\n14254\n\"f2.json\"\n", "", 0),
        read_past: ("[[30020,\"f0.json\",42888],[30021,\"f2.json\",1],[30022,\"f2.json\",2],[\"E:break\",\"f2.json\",14254]]\n", "", 0),
    },
    Row {
        name: "blank_file",
        cuts: &[Cut::At(30021, 1), Cut::Blank(b"\n \n")],
        corrupt: &[],
        counted: ("40000\n14254\n\"f2.json\"\n", "", 0),
        read_past: ("[[30020,\"f0.json\",42888],[30021,\"f2.json\",1],[30022,\"f2.json\",2],[\"E:break\",\"f2.json\",14254]]\n", "", 0),
    },
    Row {
        name: "blank_in_string",
        cuts: &[Cut::At(30021, 18), Cut::Blank(b"\n")],
        corrupt: &[],
        counted: ("", "jq: error (at f2.json:1): Invalid string: control characters from U+0000 through U+001F must be escaped at line 42890, column 21\n", 5),
        read_past: ("[[30020,\"f0.json\",42888],[\"E:Invalid string: control characters from U+0000 through U+001F must be escaped at line 42890, column 21\",\"f2.json\",1],[30022,\"f2.json\",2],[\"E:break\",\"f2.json\",14254]]\n", "", 0),
    },
    Row {
        name: "five_files",
        cuts: &[Cut::At(5000, 3), Cut::At(12000, 30), Cut::At(20002, 5), Cut::At(33000, 9)],
        corrupt: &[],
        counted: ("40000\n10000\n\"f4.json\"\n", "", 0),
        read_past: ("[[4999,\"f0.json\",7142],[5000,\"f1.json\",1],[5001,\"f1.json\",5],[11999,\"f1.json\",10000],[12000,\"f1.json\",10001],[12001,\"f2.json\",4],[20001,\"f2.json\",11430],[20002,\"f3.json\",4],[20003,\"f3.json\",5],[32999,\"f3.json\",18569],[33000,\"f4.json\",1],[33001,\"f4.json\",5],[\"E:break\",\"f4.json\",10000]]\n", "", 0),
    },
    Row {
        name: "fault_after",
        cuts: &[Cut::At(30021, 6)],
        corrupt: &[(30022, b"{\"id\":}\n")],
        counted: ("", "jq: error (at f1.json:2): Unmatched '}' at line 42890, column 7\n", 5),
        read_past: ("[[30020,\"f0.json\",42888],[30021,\"f1.json\",1],[\"E:Unmatched '}' at line 42890, column 7\",\"f1.json\",2],[\"E:break\",\"f1.json\",14254]]\n", "", 0),
    },
    Row {
        name: "fault_in_moved",
        cuts: &[Cut::At(30021, 9)],
        corrupt: &[(30021, b"{\"id\":30021,,\"s\":\"q\"}\n")],
        counted: ("", "jq: error (at f1.json:1): Expected value before ',' at line 42889, column 13\n", 5),
        read_past: ("[[30020,\"f0.json\",42888],[\"E:Expected value before ',' at line 42889, column 13\",\"f1.json\",1],[30022,\"f1.json\",2],[\"E:break\",\"f1.json\",14254]]\n", "", 0),
    },
    Row {
        name: "fault_in_next_own",
        cuts: &[Cut::At(30021, 9)],
        corrupt: &[(30021, b"{\"id\":30021,\"s\":\"q\",}\n")],
        counted: ("", "jq: error (at f1.json:1): Expected another key-value pair at line 42889, column 21\n", 5),
        read_past: ("[[30020,\"f0.json\",42888],[\"E:Expected another key-value pair at line 42889, column 21\",\"f1.json\",1],[30022,\"f1.json\",2],[\"E:break\",\"f1.json\",14254]]\n", "", 0),
    },
    Row {
        name: "unfinished_last",
        cuts: &[Cut::At(30021, 9)],
        corrupt: &[(39999, b"{\"id\":39999,\"s\":\"open\n")],
        counted: ("", "jq: error (at f1.json:14254): Unfinished string at EOF at line 57143, column 0\n", 5),
        read_past: ("[[30020,\"f0.json\",42888],[30021,\"f1.json\",1],[30022,\"f1.json\",2],[\"E:Unfinished string at EOF at line 57143, column 0\",\"f1.json\",14254],[\"E:break\",null,0]]\n", "", 0),
    },
];

/// The row's files, and the records it cuts (`$cuts`).
fn files(row: &Row) -> (Vec<Vec<u8>>, Vec<usize>) {
    let mut starts = Vec::with_capacity(RECORDS);
    let mut data = Vec::new();
    for i in 0..RECORDS {
        starts.push(data.len());
        match row.corrupt.iter().find(|(at, _)| *at == i) {
            Some((_, text)) => data.extend_from_slice(text),
            None => data.extend(record(i)),
        }
    }
    let (mut pieces, mut prev, mut cut) = (Vec::new(), 0, Vec::new());
    for c in row.cuts {
        match *c {
            Cut::Blank(text) => pieces.push(text.to_vec()),
            Cut::At(record, byte) => {
                let at = starts[record] + byte;
                pieces.push(data[prev..at].to_vec());
                prev = at;
                cut.push(record);
            }
        }
    }
    pieces.push(data[prev..].to_vec());
    cut.sort_unstable();
    cut.dedup();
    (pieces, cut)
}

#[test]
fn a_document_cut_across_files_reads_as_jq_reads_it_7480() -> Result<()> {
    for row in ROWS {
        let dir = tempfile::tempdir()?;
        let (pieces, cut) = files(row);
        assert!(pieces.iter().map(Vec::len).sum::<usize>() > 1 << 20);
        let mut names = Vec::new();
        for (k, piece) in pieces.iter().enumerate() {
            let path = dir.path().join(format!("f{k}.json"));
            std::fs::write(&path, piece)?;
            names.push(path.to_string_lossy().into_owned());
        }
        let cuts = format!("{cut:?}");
        let prefix = format!("{}/", dir.path().to_string_lossy());
        for (program, expected) in [(COUNTED, row.counted), (READ_PAST, row.read_past)] {
            let mut argv = vec!["-nc", "--argjson", "cuts", &cuts, program];
            argv.extend(names.iter().map(String::as_str));
            let (out, status) = spawn_jq(&argv, None)?;
            let seen = (
                String::from_utf8_lossy(&out.stdout).replace(&prefix, ""),
                String::from_utf8_lossy(&out.stderr).replace(&prefix, ""),
                status,
            );
            assert_eq!(
                (seen.0.as_str(), seen.1.as_str(), seen.2),
                expected,
                "row {} program {program}",
                row.name
            );
        }
    }
    Ok(())
}
