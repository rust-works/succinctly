//! #5923: after a parse error that `input` raises, jq drops the rest of the `fgets` read it was in
//! and never counts the dropped bytes, so the position of the next fault starts where the first
//! left the parser, not where it sits in the text. `}\n}\n` reports line 1, column 1, then line 1,
//! column 2. Every expected string was captured from jq 1.7.1 (`/usr/bin/jq`).

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn a_fault_after_a_fault_is_placed_from_where_the_parser_stopped_5923() -> Result<()> {
    for (input, filter, want, want_code) in [
        ("}\n}\n[4]\n", r"[limit(8; repeat(try input catch .))]", "[\"Unmatched '}' at line 1, column 1\",\"Unmatched '}' at line 1, column 2\",[4],\"break\",\"break\",\"break\",\"break\",\"break\"]\n", 0),
        ("[1]\n[2]\n[3]\n}\n}\n[4]\n", r"[limit(8; repeat(try input catch .))]", "[[1],[2],[3],\"Unmatched '}' at line 4, column 1\",\"Unmatched '}' at line 4, column 2\",[4],\"break\",\"break\"]\n", 0),
        ("]\n}\n]\n[7]\n", r"[limit(8; repeat(try input catch .))]", "[\"Unmatched ']' at line 1, column 1\",\"Unmatched '}' at line 1, column 2\",\"Unmatched ']' at line 1, column 3\",[7],\"break\",\"break\",\"break\",\"break\"]\n", 0),
        ("}}\n}\n", r"[limit(8; repeat(try input catch .))]", "[\"Unmatched '}' at line 1, column 1\",\"Unmatched '}' at line 1, column 2\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\"]\n", 0),
        ("}\n\n}\n[1]\n", r"[limit(8; repeat(try input catch .))]", "[\"Unmatched '}' at line 1, column 1\",\"Unmatched '}' at line 2, column 1\",[1],\"break\",\"break\",\"break\",\"break\",\"break\"]\n", 0),
        ("[1,\n}\n}\n2\n", r"[limit(8; repeat(try input catch .))]", "[\"Unmatched '}' at line 2, column 1\",\"Unmatched '}' at line 2, column 2\",2,\"break\",\"break\",\"break\",\"break\",\"break\"]\n", 0),
        ("tru\n}\n}\n", r"[limit(8; repeat(try input catch .))]", "[\"Invalid literal at line 2, column 0\",\"Unmatched '}' at line 2, column 1\",\"Unmatched '}' at line 2, column 2\",\"break\",\"break\",\"break\",\"break\",\"break\"]\n", 0),
        ("1e\n}\n]\n3\n", r"[limit(8; repeat(try input catch .))]", "[\"Invalid numeric literal at line 2, column 0\",\"Unmatched '}' at line 2, column 1\",\"Unmatched ']' at line 2, column 2\",3,\"break\",\"break\",\"break\",\"break\"]\n", 0),
        (",\ntru\n12 34\n@\n}}\n", r"[limit(8; repeat(try input catch .))]", "[\"Expected value before ',' at line 1, column 1\",\"Invalid literal at line 2, column 0\",12,34,\"Invalid numeric literal at line 4, column 0\",\"Unmatched '}' at line 4, column 1\",\"break\",\"break\"]\n", 0),
        ("@\n@\n@\n@\n", r"[limit(8; repeat(try input catch .))]", "[\"Invalid numeric literal at line 2, column 0\",\"Invalid numeric literal at line 3, column 0\",\"Invalid numeric literal at line 4, column 0\",\"Invalid numeric literal at line 5, column 0\",\"break\",\"break\",\"break\",\"break\"]\n", 0),
        ("{\"a\"}\n}\n", r"[limit(8; repeat(try input catch .))]", "[\"Objects must consist of key:value pairs at line 1, column 5\",\"Unmatched '}' at line 1, column 6\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\"]\n", 0),
    ] {
        let (stdout, _, code) = run_jq_full(&["-nc", filter], Some(input))?;
        assert_eq!((stdout.as_str(), code), (want, want_code), "{input:?}");
    }
    Ok(())
}
