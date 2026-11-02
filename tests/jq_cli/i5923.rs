//! #5923: after a parse error that `input` raises, jq drops the rest of the `fgets` read the fault was
//! in and never counts the dropped bytes, so the position of the next fault starts where the first
//! left the parser, not where it sits in the text, and the counters carry on from one file to the
//! next. `}\n}\n` reports line 1, column 1, then line 1, column 2. Every expected string was captured
//! from jq 1.7.1 (`/usr/bin/jq`).

use anyhow::Result;

use crate::run_jq_full;

const FILTER: &str = "[limit(8; repeat(try input catch .))]";

#[test]
fn a_fault_after_a_fault_is_placed_from_where_the_parser_stopped_5923() -> Result<()> {
    for (input, want) in [
        ("}\n}\n[4]\n", "[\"Unmatched '}' at line 1, column 1\",\"Unmatched '}' at line 1, column 2\",[4],\"break\",\"break\",\"break\",\"break\",\"break\"]\n"),
        ("[1]\n[2]\n[3]\n}\n}\n[4]\n", "[[1],[2],[3],\"Unmatched '}' at line 4, column 1\",\"Unmatched '}' at line 4, column 2\",[4],\"break\",\"break\"]\n"),
        ("]\n}\n]\n[7]\n", "[\"Unmatched ']' at line 1, column 1\",\"Unmatched '}' at line 1, column 2\",\"Unmatched ']' at line 1, column 3\",[7],\"break\",\"break\",\"break\",\"break\"]\n"),
        ("}}\n}\n", "[\"Unmatched '}' at line 1, column 1\",\"Unmatched '}' at line 1, column 2\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\"]\n"),
        ("}\n\n}\n[1]\n", "[\"Unmatched '}' at line 1, column 1\",\"Unmatched '}' at line 2, column 1\",[1],\"break\",\"break\",\"break\",\"break\",\"break\"]\n"),
        ("[1,\n}\n}\n2\n", "[\"Unmatched '}' at line 2, column 1\",\"Unmatched '}' at line 2, column 2\",2,\"break\",\"break\",\"break\",\"break\",\"break\"]\n"),
        ("tru\n}\n}\n", "[\"Invalid literal at line 2, column 0\",\"Unmatched '}' at line 2, column 1\",\"Unmatched '}' at line 2, column 2\",\"break\",\"break\",\"break\",\"break\",\"break\"]\n"),
        ("1e\n}\n]\n3\n", "[\"Invalid numeric literal at line 2, column 0\",\"Unmatched '}' at line 2, column 1\",\"Unmatched ']' at line 2, column 2\",3,\"break\",\"break\",\"break\",\"break\"]\n"),
        (",\ntru\n12 34\n@\n}}\n", "[\"Expected value before ',' at line 1, column 1\",\"Invalid literal at line 2, column 0\",12,34,\"Invalid numeric literal at line 4, column 0\",\"Unmatched '}' at line 4, column 1\",\"break\",\"break\"]\n"),
        ("@\n@\n@\n@\n", "[\"Invalid numeric literal at line 2, column 0\",\"Invalid numeric literal at line 3, column 0\",\"Invalid numeric literal at line 4, column 0\",\"Invalid numeric literal at line 5, column 0\",\"break\",\"break\",\"break\",\"break\"]\n"),
        ("{\"a\"}\n}\n", "[\"Objects must consist of key:value pairs at line 1, column 5\",\"Unmatched '}' at line 1, column 6\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\"]\n"),
    ] {
        let (stdout, _, code) = run_jq_full(&["-nc", FILTER], Some(input))?;
        assert_eq!((stdout.as_str(), code), (want, 0), "{input:?}");
    }
    Ok(())
}

/// jq reads every file as one stream, so the parser's counters go on from one file to the next, and
/// a fault in a later file is placed from where the earlier ones left them.
#[test]
fn the_parser_position_carries_from_one_file_to_the_next_5923() -> Result<()> {
    for (files, want) in [
        (vec!["}\n}\n", "}\n}\n"], "[\"Unmatched '}' at line 1, column 1\",\"Unmatched '}' at line 1, column 2\",\"Unmatched '}' at line 1, column 3\",\"Unmatched '}' at line 1, column 4\",\"break\",\"break\",\"break\",\"break\"]\n"),
        (vec!["}\n}\n", "[1]\n}\n"], "[\"Unmatched '}' at line 1, column 1\",\"Unmatched '}' at line 1, column 2\",[1],\"Unmatched '}' at line 2, column 1\",\"break\",\"break\",\"break\",\"break\"]\n"),
        (vec!["[1]\n}\n", "}\n[2]\n"], "[[1],\"Unmatched '}' at line 2, column 1\",\"Unmatched '}' at line 2, column 2\",[2],\"break\",\"break\",\"break\",\"break\"]\n"),
        (vec!["}\n}\n", "}\n}\n", "[1]\n}\n", "}\n[2]\n"], "[\"Unmatched '}' at line 1, column 1\",\"Unmatched '}' at line 1, column 2\",\"Unmatched '}' at line 1, column 3\",\"Unmatched '}' at line 1, column 4\",[1],\"Unmatched '}' at line 2, column 1\",\"Unmatched '}' at line 2, column 2\",[2]]\n"),
        (vec!["}\n", "\n", "}\n", "x\n", "}\n"], "[\"Unmatched '}' at line 1, column 1\",\"Unmatched '}' at line 2, column 1\",\"Invalid numeric literal at line 3, column 0\",\"Unmatched '}' at line 3, column 1\",\"break\",\"break\",\"break\",\"break\"]\n"),
    ] {
        let dir = tempfile::tempdir()?;
        let mut args = vec!["-nc".to_string(), FILTER.to_string()];
        for (i, text) in files.iter().enumerate() {
            let path = dir.path().join(format!("f{i}"));
            std::fs::write(&path, text)?;
            args.push(path.to_string_lossy().into_owned());
        }
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let (stdout, _, code) = run_jq_full(&args, None)?;
        assert_eq!((stdout.as_str(), code), (want, 0), "{files:?}");
    }
    Ok(())
}

/// jq's depth refusal is a fault like any other, so the faults around it are placed by the counters
/// it leaves: it reports the column of the bracket that crossed the limit, and the next fault goes on
/// from there.
#[test]
fn a_depth_refusal_between_faults_moves_the_counters_like_any_fault_5923() -> Result<()> {
    let deep = "[".repeat(300);
    for (input, want) in [
        (["}\n", &deep, "\n}\n[5]\n"].concat(), "[\"Unmatched '}' at line 1, column 1\",\"Exceeds depth limit for parsing at line 1, column 258\",\"Unmatched '}' at line 1, column 259\",[5],\"break\",\"break\",\"break\",\"break\"]\n".to_string()),
        (["}\n}\n", &deep, "1\n}\n[5]\n"].concat(), "[\"Unmatched '}' at line 1, column 1\",\"Unmatched '}' at line 1, column 2\",\"Exceeds depth limit for parsing at line 1, column 259\",\"Unmatched '}' at line 1, column 260\",[5],\"break\",\"break\",\"break\"]\n".to_string()),
        (["[1]\n", &deep, "\n}\n}\n[6]\n"].concat(), "[[1],\"Exceeds depth limit for parsing at line 2, column 257\",\"Unmatched '}' at line 2, column 258\",\"Unmatched '}' at line 2, column 259\",[6],\"break\",\"break\",\"break\"]\n".to_string()),
    ] {
        let (stdout, _, code) = run_jq_full(&["-nc", FILTER], Some(&input))?;
        assert_eq!((stdout.as_str(), code), (want.as_str(), 0), "{}", &input[..12]);
    }
    Ok(())
}
