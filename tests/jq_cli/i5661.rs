//! #5661: `-n` with `input`/`inputs` over a clean plain-JSON stream converts each document when the
//! program reads it, not up front. What the program sees is what it saw when the whole stream was
//! read first, and what jq 1.7.1 answers (expected values captured from `/usr/bin/jq`): the
//! documents, `input_line_number` after each, the marker and exit status of an error, and the
//! streams that keep the up-front route (a parse error, a document nested past the lazy limit).

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn test_a_stream_read_under_null_input_matches_jq_5661() -> Result<()> {
    for (stream, program, expected_stdout, expected_stderr, expected_code) in [
        (
            r"1
2
3
",
            r"[inputs]",
            r"[1,2,3]
",
            r"",
            0,
        ),
        (
            r"1
2
3
", r"input", r"1
", r"", 0,
        ),
        (
            r"1
2
3
",
            r"[input, input]",
            r"[1,2]
",
            r"",
            0,
        ),
        (
            r"1
2
3
",
            r"first(inputs)",
            r"1
",
            r"",
            0,
        ),
        (
            r"1
2
3
",
            r"[limit(2; inputs)]",
            r"[1,2]
",
            r"",
            0,
        ),
        (
            r"1
2
3
",
            r"[inputs | input_line_number]",
            r"[1,2,3]
",
            r"",
            0,
        ),
        (
            r"1
2
3
",
            r#"try input catch "E""#,
            r"1
",
            r"",
            0,
        ),
        (
            r"1
2
3
",
            r"[inputs, input_line_number]",
            r"[1,2,3,3]
",
            r"",
            0,
        ),
        (
            r"1
2
3
",
            r#"[range(5) | try input catch "X"]"#,
            r#"[1,2,3,"X","X"]
"#,
            r"",
            0,
        ),
        (
            r"1
2
3
",
            r"last(inputs)",
            r"3
",
            r"",
            0,
        ),
        (
            r"1
2
3
",
            r"isempty(inputs)",
            r"false
",
            r"",
            0,
        ),
        (
            r"1
2
3
",
            r"reduce inputs as $x (0; . + 1)",
            r"3
",
            r"",
            0,
        ),
        (
            r"1
2
3
",
            r#"(input | type), (try input catch "end"), (try input catch "end2"), input_line_number"#,
            r#""number"
2
3
3
"#,
            r"",
            0,
        ),
        (
            r"1
2
3",
            r"[inputs]",
            r"[1,2,3]
",
            r"",
            0,
        ),
        (
            r"1
2
3", r"input", r"1
", r"", 0,
        ),
        (
            r"1
2
3",
            r"[input, input]",
            r"[1,2]
",
            r"",
            0,
        ),
        (
            r"1
2
3",
            r"first(inputs)",
            r"1
",
            r"",
            0,
        ),
        (
            r"1
2
3",
            r"[limit(2; inputs)]",
            r"[1,2]
",
            r"",
            0,
        ),
        (
            r"1
2
3",
            r"[inputs | input_line_number]",
            r"[1,2,2]
",
            r"",
            0,
        ),
        (
            r"1
2
3",
            r#"try input catch "E""#,
            r"1
",
            r"",
            0,
        ),
        (
            r"1
2
3",
            r"[inputs, input_line_number]",
            r"[1,2,3,0]
",
            r"",
            0,
        ),
        (
            r"1
2
3",
            r#"[range(5) | try input catch "X"]"#,
            r#"[1,2,3,"X","X"]
"#,
            r"",
            0,
        ),
        (
            r"1
2
3",
            r"last(inputs)",
            r"3
",
            r"",
            0,
        ),
        (
            r"1
2
3",
            r"isempty(inputs)",
            r"false
",
            r"",
            0,
        ),
        (
            r"1
2
3",
            r"reduce inputs as $x (0; . + 1)",
            r"3
",
            r"",
            0,
        ),
        (
            r"1
2
3",
            r#"(input | type), (try input catch "end"), (try input catch "end2"), input_line_number"#,
            r#""number"
2
3
2
"#,
            r"",
            0,
        ),
        (
            r"1 2 3",
            r"[inputs]",
            r"[1,2,3]
",
            r"",
            0,
        ),
        (
            r"1 2 3", r"input", r"1
", r"", 0,
        ),
        (
            r"1 2 3",
            r"[input, input]",
            r"[1,2]
",
            r"",
            0,
        ),
        (
            r"1 2 3",
            r"first(inputs)",
            r"1
",
            r"",
            0,
        ),
        (
            r"1 2 3",
            r"[limit(2; inputs)]",
            r"[1,2]
",
            r"",
            0,
        ),
        (
            r"1 2 3",
            r"[inputs | input_line_number]",
            r"[0,0,0]
",
            r"",
            0,
        ),
        (
            r"1 2 3",
            r#"try input catch "E""#,
            r"1
",
            r"",
            0,
        ),
        (
            r"1 2 3",
            r"[inputs, input_line_number]",
            r"[1,2,3,0]
",
            r"",
            0,
        ),
        (
            r"1 2 3",
            r#"[range(5) | try input catch "X"]"#,
            r#"[1,2,3,"X","X"]
"#,
            r"",
            0,
        ),
        (
            r"1 2 3",
            r"last(inputs)",
            r"3
",
            r"",
            0,
        ),
        (
            r"1 2 3",
            r"isempty(inputs)",
            r"false
",
            r"",
            0,
        ),
        (
            r"1 2 3",
            r"reduce inputs as $x (0; . + 1)",
            r"3
",
            r"",
            0,
        ),
        (
            r"1 2 3",
            r#"(input | type), (try input catch "end"), (try input catch "end2"), input_line_number"#,
            r#""number"
2
3
0
"#,
            r"",
            0,
        ),
        (
            r"1

2


3

",
            r"[inputs]",
            r"[1,2,3]
",
            r"",
            0,
        ),
        (
            r"1

2


3

",
            r"input",
            r"1
",
            r"",
            0,
        ),
        (
            r"1

2


3

",
            r"[input, input]",
            r"[1,2]
",
            r"",
            0,
        ),
        (
            r"1

2


3

",
            r"first(inputs)",
            r"1
",
            r"",
            0,
        ),
        (
            r"1

2


3

",
            r"[limit(2; inputs)]",
            r"[1,2]
",
            r"",
            0,
        ),
        (
            r"1

2


3

",
            r"[inputs | input_line_number]",
            r"[1,3,6]
",
            r"",
            0,
        ),
        (
            r"1

2


3

",
            r#"try input catch "E""#,
            r"1
",
            r"",
            0,
        ),
        (
            r"1

2


3

",
            r"[inputs, input_line_number]",
            r"[1,2,3,7]
",
            r"",
            0,
        ),
        (
            r"1

2


3

",
            r#"[range(5) | try input catch "X"]"#,
            r#"[1,2,3,"X","X"]
"#,
            r"",
            0,
        ),
        (
            r"1

2


3

",
            r"last(inputs)",
            r"3
",
            r"",
            0,
        ),
        (
            r"1

2


3

",
            r"isempty(inputs)",
            r"false
",
            r"",
            0,
        ),
        (
            r"1

2


3

",
            r"reduce inputs as $x (0; . + 1)",
            r"3
",
            r"",
            0,
        ),
        (
            r"1

2


3

",
            r#"(input | type), (try input catch "end"), (try input catch "end2"), input_line_number"#,
            r#""number"
2
3
6
"#,
            r"",
            0,
        ),
        (
            r"1
2
3
",
            r"[inputs]",
            r"[1,2,3]
",
            r"",
            0,
        ),
        (
            r"1
2
3
", r"input", r"1
", r"", 0,
        ),
        (
            r"1
2
3
",
            r"[input, input]",
            r"[1,2]
",
            r"",
            0,
        ),
        (
            r"1
2
3
",
            r"first(inputs)",
            r"1
",
            r"",
            0,
        ),
        (
            r"1
2
3
",
            r"[limit(2; inputs)]",
            r"[1,2]
",
            r"",
            0,
        ),
        (
            r"1
2
3
",
            r"[inputs | input_line_number]",
            r"[1,2,3]
",
            r"",
            0,
        ),
        (
            r"1
2
3
",
            r#"try input catch "E""#,
            r"1
",
            r"",
            0,
        ),
        (
            r"1
2
3
",
            r"[inputs, input_line_number]",
            r"[1,2,3,3]
",
            r"",
            0,
        ),
        (
            r"1
2
3
",
            r#"[range(5) | try input catch "X"]"#,
            r#"[1,2,3,"X","X"]
"#,
            r"",
            0,
        ),
        (
            r"1
2
3
",
            r"last(inputs)",
            r"3
",
            r"",
            0,
        ),
        (
            r"1
2
3
",
            r"isempty(inputs)",
            r"false
",
            r"",
            0,
        ),
        (
            r"1
2
3
",
            r"reduce inputs as $x (0; . + 1)",
            r"3
",
            r"",
            0,
        ),
        (
            r"1
2
3
",
            r#"(input | type), (try input catch "end"), (try input catch "end2"), input_line_number"#,
            r#""number"
2
3
3
"#,
            r"",
            0,
        ),
        (
            r#"{"a":1}
{"a":2}
{"a":3}
"#,
            r"[inputs]",
            r#"[{"a":1},{"a":2},{"a":3}]
"#,
            r"",
            0,
        ),
        (
            r#"{"a":1}
{"a":2}
{"a":3}
"#,
            r"input",
            r#"{"a":1}
"#,
            r"",
            0,
        ),
        (
            r#"{"a":1}
{"a":2}
{"a":3}
"#,
            r"[input, input]",
            r#"[{"a":1},{"a":2}]
"#,
            r"",
            0,
        ),
        (
            r#"{"a":1}
{"a":2}
{"a":3}
"#,
            r"first(inputs)",
            r#"{"a":1}
"#,
            r"",
            0,
        ),
        (
            r#"{"a":1}
{"a":2}
{"a":3}
"#,
            r"[limit(2; inputs)]",
            r#"[{"a":1},{"a":2}]
"#,
            r"",
            0,
        ),
        (
            r#"{"a":1}
{"a":2}
{"a":3}
"#,
            r"[inputs | input_line_number]",
            r"[1,2,3]
",
            r"",
            0,
        ),
        (
            r#"{"a":1}
{"a":2}
{"a":3}
"#,
            r#"try input catch "E""#,
            r#"{"a":1}
"#,
            r"",
            0,
        ),
        (
            r#"{"a":1}
{"a":2}
{"a":3}
"#,
            r"[inputs, input_line_number]",
            r#"[{"a":1},{"a":2},{"a":3},3]
"#,
            r"",
            0,
        ),
        (
            r#"{"a":1}
{"a":2}
{"a":3}
"#,
            r#"[range(5) | try input catch "X"]"#,
            r#"[{"a":1},{"a":2},{"a":3},"X","X"]
"#,
            r"",
            0,
        ),
        (
            r#"{"a":1}
{"a":2}
{"a":3}
"#,
            r"last(inputs)",
            r#"{"a":3}
"#,
            r"",
            0,
        ),
        (
            r#"{"a":1}
{"a":2}
{"a":3}
"#,
            r"isempty(inputs)",
            r"false
",
            r"",
            0,
        ),
        (
            r#"{"a":1}
{"a":2}
{"a":3}
"#,
            r"reduce inputs as $x (0; . + 1)",
            r"3
",
            r"",
            0,
        ),
        (
            r#"{"a":1}
{"a":2}
{"a":3}
"#,
            r#"(input | type), (try input catch "end"), (try input catch "end2"), input_line_number"#,
            r#""object"
{"a":2}
{"a":3}
3
"#,
            r"",
            0,
        ),
        (
            r#"{
  "a": 1,
  "b": [1,
 2]
}
{"c":
3}
"#,
            r"[inputs]",
            r#"[{"a":1,"b":[1,2]},{"c":3}]
"#,
            r"",
            0,
        ),
        (
            r#"{
  "a": 1,
  "b": [1,
 2]
}
{"c":
3}
"#,
            r"input",
            r#"{"a":1,"b":[1,2]}
"#,
            r"",
            0,
        ),
        (
            r#"{
  "a": 1,
  "b": [1,
 2]
}
{"c":
3}
"#,
            r"[input, input]",
            r#"[{"a":1,"b":[1,2]},{"c":3}]
"#,
            r"",
            0,
        ),
        (
            r#"{
  "a": 1,
  "b": [1,
 2]
}
{"c":
3}
"#,
            r"first(inputs)",
            r#"{"a":1,"b":[1,2]}
"#,
            r"",
            0,
        ),
        (
            r#"{
  "a": 1,
  "b": [1,
 2]
}
{"c":
3}
"#,
            r"[limit(2; inputs)]",
            r#"[{"a":1,"b":[1,2]},{"c":3}]
"#,
            r"",
            0,
        ),
        (
            r#"{
  "a": 1,
  "b": [1,
 2]
}
{"c":
3}
"#,
            r"[inputs | input_line_number]",
            r"[5,7]
",
            r"",
            0,
        ),
        (
            r#"{
  "a": 1,
  "b": [1,
 2]
}
{"c":
3}
"#,
            r#"try input catch "E""#,
            r#"{"a":1,"b":[1,2]}
"#,
            r"",
            0,
        ),
        (
            r#"{
  "a": 1,
  "b": [1,
 2]
}
{"c":
3}
"#,
            r"[inputs, input_line_number]",
            r#"[{"a":1,"b":[1,2]},{"c":3},7]
"#,
            r"",
            0,
        ),
        (
            r#"{
  "a": 1,
  "b": [1,
 2]
}
{"c":
3}
"#,
            r#"[range(5) | try input catch "X"]"#,
            r#"[{"a":1,"b":[1,2]},{"c":3},"X","X","X"]
"#,
            r"",
            0,
        ),
        (
            r#"{
  "a": 1,
  "b": [1,
 2]
}
{"c":
3}
"#,
            r"last(inputs)",
            r#"{"c":3}
"#,
            r"",
            0,
        ),
        (
            r#"{
  "a": 1,
  "b": [1,
 2]
}
{"c":
3}
"#,
            r"isempty(inputs)",
            r"false
",
            r"",
            0,
        ),
        (
            r#"{
  "a": 1,
  "b": [1,
 2]
}
{"c":
3}
"#,
            r"reduce inputs as $x (0; . + 1)",
            r"2
",
            r"",
            0,
        ),
        (
            r#"{
  "a": 1,
  "b": [1,
 2]
}
{"c":
3}
"#,
            r#"(input | type), (try input catch "end"), (try input catch "end2"), input_line_number"#,
            r#""object"
{"c":3}
"end2"
7
"#,
            r"",
            0,
        ),
        (
            r#"{"only":true}
"#,
            r"[inputs]",
            r#"[{"only":true}]
"#,
            r"",
            0,
        ),
        (
            r#"{"only":true}
"#,
            r"input",
            r#"{"only":true}
"#,
            r"",
            0,
        ),
        (
            r#"{"only":true}
"#,
            r"[input, input]",
            r"",
            r"jq: error (at <stdin>:1): break
",
            5,
        ),
        (
            r#"{"only":true}
"#,
            r"first(inputs)",
            r#"{"only":true}
"#,
            r"",
            0,
        ),
        (
            r#"{"only":true}
"#,
            r"[limit(2; inputs)]",
            r#"[{"only":true}]
"#,
            r"",
            0,
        ),
        (
            r#"{"only":true}
"#,
            r"[inputs | input_line_number]",
            r"[1]
",
            r"",
            0,
        ),
        (
            r#"{"only":true}
"#,
            r#"try input catch "E""#,
            r#"{"only":true}
"#,
            r"",
            0,
        ),
        (
            r#"{"only":true}
"#,
            r"[inputs, input_line_number]",
            r#"[{"only":true},1]
"#,
            r"",
            0,
        ),
        (
            r#"{"only":true}
"#,
            r#"[range(5) | try input catch "X"]"#,
            r#"[{"only":true},"X","X","X","X"]
"#,
            r"",
            0,
        ),
        (
            r#"{"only":true}
"#,
            r"last(inputs)",
            r#"{"only":true}
"#,
            r"",
            0,
        ),
        (
            r#"{"only":true}
"#,
            r"isempty(inputs)",
            r"false
",
            r"",
            0,
        ),
        (
            r#"{"only":true}
"#,
            r"reduce inputs as $x (0; . + 1)",
            r"1
",
            r"",
            0,
        ),
        (
            r#"{"only":true}
"#,
            r#"(input | type), (try input catch "end"), (try input catch "end2"), input_line_number"#,
            r#""object"
"end"
"end2"
0
"#,
            r"",
            0,
        ),
        (
            r"",
            r"[inputs]",
            r"[]
",
            r"",
            0,
        ),
        (
            r"",
            r"input",
            r"",
            r"jq: error (at <stdin>:0): break
",
            5,
        ),
        (
            r"",
            r"[input, input]",
            r"",
            r"jq: error (at <stdin>:0): break
",
            5,
        ),
        (r"", r"first(inputs)", r"", r"", 0),
        (
            r"",
            r"[limit(2; inputs)]",
            r"[]
",
            r"",
            0,
        ),
        (
            r"",
            r"[inputs | input_line_number]",
            r"[]
",
            r"",
            0,
        ),
        (
            r"",
            r#"try input catch "E""#,
            r#""E"
"#,
            r"",
            0,
        ),
        (
            r"",
            r"[inputs, input_line_number]",
            r"[0]
",
            r"",
            0,
        ),
        (
            r"",
            r#"[range(5) | try input catch "X"]"#,
            r#"["X","X","X","X","X"]
"#,
            r"",
            0,
        ),
        (
            r"",
            r"last(inputs)",
            r"null
",
            r"",
            0,
        ),
        (
            r"",
            r"isempty(inputs)",
            r"true
",
            r"",
            0,
        ),
        (
            r"",
            r"reduce inputs as $x (0; . + 1)",
            r"0
",
            r"",
            0,
        ),
        (
            r"",
            r#"(input | type), (try input catch "end"), (try input catch "end2"), input_line_number"#,
            r"",
            r"jq: error (at <stdin>:0): break
",
            5,
        ),
        (
            r"

  1
2
",
            r"[inputs]",
            r"[1,2]
",
            r"",
            0,
        ),
        (
            r"

  1
2
",
            r"input",
            r"1
",
            r"",
            0,
        ),
        (
            r"

  1
2
",
            r"[input, input]",
            r"[1,2]
",
            r"",
            0,
        ),
        (
            r"

  1
2
",
            r"first(inputs)",
            r"1
",
            r"",
            0,
        ),
        (
            r"

  1
2
",
            r"[limit(2; inputs)]",
            r"[1,2]
",
            r"",
            0,
        ),
        (
            r"

  1
2
",
            r"[inputs | input_line_number]",
            r"[3,4]
",
            r"",
            0,
        ),
        (
            r"

  1
2
",
            r#"try input catch "E""#,
            r"1
",
            r"",
            0,
        ),
        (
            r"

  1
2
",
            r"[inputs, input_line_number]",
            r"[1,2,4]
",
            r"",
            0,
        ),
        (
            r"

  1
2
",
            r#"[range(5) | try input catch "X"]"#,
            r#"[1,2,"X","X","X"]
"#,
            r"",
            0,
        ),
        (
            r"

  1
2
",
            r"last(inputs)",
            r"2
",
            r"",
            0,
        ),
        (
            r"

  1
2
",
            r"isempty(inputs)",
            r"false
",
            r"",
            0,
        ),
        (
            r"

  1
2
",
            r"reduce inputs as $x (0; . + 1)",
            r"2
",
            r"",
            0,
        ),
        (
            r"

  1
2
",
            r#"(input | type), (try input catch "end"), (try input catch "end2"), input_line_number"#,
            r#""number"
2
"end2"
4
"#,
            r"",
            0,
        ),
        (
            r#"{"k":"é€😀"}
"日本語"
"#,
            r"[inputs]",
            r#"[{"k":"é€😀"},"日本語"]
"#,
            r"",
            0,
        ),
        (
            r#"{"k":"é€😀"}
"日本語"
"#,
            r"input",
            r#"{"k":"é€😀"}
"#,
            r"",
            0,
        ),
        (
            r#"{"k":"é€😀"}
"日本語"
"#,
            r"[input, input]",
            r#"[{"k":"é€😀"},"日本語"]
"#,
            r"",
            0,
        ),
        (
            r#"{"k":"é€😀"}
"日本語"
"#,
            r"first(inputs)",
            r#"{"k":"é€😀"}
"#,
            r"",
            0,
        ),
        (
            r#"{"k":"é€😀"}
"日本語"
"#,
            r"[limit(2; inputs)]",
            r#"[{"k":"é€😀"},"日本語"]
"#,
            r"",
            0,
        ),
        (
            r#"{"k":"é€😀"}
"日本語"
"#,
            r"[inputs | input_line_number]",
            r"[1,2]
",
            r"",
            0,
        ),
        (
            r#"{"k":"é€😀"}
"日本語"
"#,
            r#"try input catch "E""#,
            r#"{"k":"é€😀"}
"#,
            r"",
            0,
        ),
        (
            r#"{"k":"é€😀"}
"日本語"
"#,
            r"[inputs, input_line_number]",
            r#"[{"k":"é€😀"},"日本語",2]
"#,
            r"",
            0,
        ),
        (
            r#"{"k":"é€😀"}
"日本語"
"#,
            r#"[range(5) | try input catch "X"]"#,
            r#"[{"k":"é€😀"},"日本語","X","X","X"]
"#,
            r"",
            0,
        ),
        (
            r#"{"k":"é€😀"}
"日本語"
"#,
            r"last(inputs)",
            r#""日本語"
"#,
            r"",
            0,
        ),
        (
            r#"{"k":"é€😀"}
"日本語"
"#,
            r"isempty(inputs)",
            r"false
",
            r"",
            0,
        ),
        (
            r#"{"k":"é€😀"}
"日本語"
"#,
            r"reduce inputs as $x (0; . + 1)",
            r"2
",
            r"",
            0,
        ),
        (
            r#"{"k":"é€😀"}
"日本語"
"#,
            r#"(input | type), (try input catch "end"), (try input catch "end2"), input_line_number"#,
            r#""object"
"日本語"
"end2"
2
"#,
            r"",
            0,
        ),
        (
            r"100000000000000000000
1.0
1e1000
-0
0.1
",
            r"[inputs]",
            r"[100000000000000000000,1.0,1E+1000,-0,0.1]
",
            r"",
            0,
        ),
        (
            r"100000000000000000000
1.0
1e1000
-0
0.1
",
            r"input",
            r"100000000000000000000
",
            r"",
            0,
        ),
        (
            r"100000000000000000000
1.0
1e1000
-0
0.1
",
            r"[input, input]",
            r"[100000000000000000000,1.0]
",
            r"",
            0,
        ),
        (
            r"100000000000000000000
1.0
1e1000
-0
0.1
",
            r"first(inputs)",
            r"100000000000000000000
",
            r"",
            0,
        ),
        (
            r"100000000000000000000
1.0
1e1000
-0
0.1
",
            r"[limit(2; inputs)]",
            r"[100000000000000000000,1.0]
",
            r"",
            0,
        ),
        (
            r"100000000000000000000
1.0
1e1000
-0
0.1
",
            r"[inputs | input_line_number]",
            r"[1,2,3,4,5]
",
            r"",
            0,
        ),
        (
            r"100000000000000000000
1.0
1e1000
-0
0.1
",
            r#"try input catch "E""#,
            r"100000000000000000000
",
            r"",
            0,
        ),
        (
            r"100000000000000000000
1.0
1e1000
-0
0.1
",
            r"[inputs, input_line_number]",
            r"[100000000000000000000,1.0,1E+1000,-0,0.1,5]
",
            r"",
            0,
        ),
        (
            r"100000000000000000000
1.0
1e1000
-0
0.1
",
            r#"[range(5) | try input catch "X"]"#,
            r"[100000000000000000000,1.0,1E+1000,-0,0.1]
",
            r"",
            0,
        ),
        (
            r"100000000000000000000
1.0
1e1000
-0
0.1
",
            r"last(inputs)",
            r"0.1
",
            r"",
            0,
        ),
        (
            r"100000000000000000000
1.0
1e1000
-0
0.1
",
            r"isempty(inputs)",
            r"false
",
            r"",
            0,
        ),
        (
            r"100000000000000000000
1.0
1e1000
-0
0.1
",
            r"reduce inputs as $x (0; . + 1)",
            r"5
",
            r"",
            0,
        ),
        (
            r"100000000000000000000
1.0
1e1000
-0
0.1
",
            r#"(input | type), (try input catch "end"), (try input catch "end2"), input_line_number"#,
            r#""number"
1.0
1E+1000
3
"#,
            r"",
            0,
        ),
        (
            r"[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[1]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]
1
",
            r"[inputs]",
            r"[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[1]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]],1]
",
            r"",
            0,
        ),
        (
            r"[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[1]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]
1
",
            r"input",
            r"[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[1]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]
",
            r"",
            0,
        ),
        (
            r"[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[1]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]
1
",
            r"[input, input]",
            r"[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[1]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]],1]
",
            r"",
            0,
        ),
        (
            r"[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[1]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]
1
",
            r"first(inputs)",
            r"[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[1]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]
",
            r"",
            0,
        ),
        (
            r"[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[1]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]
1
",
            r"[limit(2; inputs)]",
            r"[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[1]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]],1]
",
            r"",
            0,
        ),
        (
            r"[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[1]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]
1
",
            r"[inputs | input_line_number]",
            r"[1,2]
",
            r"",
            0,
        ),
        (
            r"[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[1]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]
1
",
            r#"try input catch "E""#,
            r"[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[1]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]
",
            r"",
            0,
        ),
        (
            r"[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[1]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]
1
",
            r"[inputs, input_line_number]",
            r"[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[1]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]],1,2]
",
            r"",
            0,
        ),
        (
            r"[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[1]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]
1
",
            r#"[range(5) | try input catch "X"]"#,
            r#"[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[1]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]],1,"X","X","X"]
"#,
            r"",
            0,
        ),
        (
            r"[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[1]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]
1
",
            r"last(inputs)",
            r"1
",
            r"",
            0,
        ),
        (
            r"[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[1]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]
1
",
            r"isempty(inputs)",
            r"false
",
            r"",
            0,
        ),
        (
            r"[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[1]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]
1
",
            r"reduce inputs as $x (0; . + 1)",
            r"2
",
            r"",
            0,
        ),
        (
            r"[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[1]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]
1
",
            r#"(input | type), (try input catch "end"), (try input catch "end2"), input_line_number"#,
            r#""array"
1
"end2"
2
"#,
            r"",
            0,
        ),
        (
            r"1
2
}
",
            r"[inputs]",
            r"",
            r"jq: error (at <stdin>:3): Unmatched '}' at line 3, column 1
",
            5,
        ),
        (
            r"1
2
}
", r"input", r"1
", r"", 0,
        ),
        (
            r"1
2
}
",
            r"[input, input]",
            r"[1,2]
",
            r"",
            0,
        ),
        (
            r"1
2
}
",
            r"first(inputs)",
            r"1
",
            r"",
            0,
        ),
        (
            r"1
2
}
",
            r"[limit(2; inputs)]",
            r"[1,2]
",
            r"",
            0,
        ),
        (
            r"1
2
}
",
            r"[inputs | input_line_number]",
            r"",
            r"jq: error (at <stdin>:3): Unmatched '}' at line 3, column 1
",
            5,
        ),
        (
            r"1
2
}
",
            r#"try input catch "E""#,
            r"1
",
            r"",
            0,
        ),
        (
            r"1
2
}
",
            r"[inputs, input_line_number]",
            r"",
            r"jq: error (at <stdin>:3): Unmatched '}' at line 3, column 1
",
            5,
        ),
        (
            r"1
2
}
",
            r#"[range(5) | try input catch "X"]"#,
            r#"[1,2,"X","X","X"]
"#,
            r"",
            0,
        ),
        (
            r"1
2
}
",
            r"last(inputs)",
            r"",
            r"jq: error (at <stdin>:3): Unmatched '}' at line 3, column 1
",
            5,
        ),
        (
            r"1
2
}
",
            r"isempty(inputs)",
            r"false
",
            r"",
            0,
        ),
        (
            r"1
2
}
",
            r"reduce inputs as $x (0; . + 1)",
            r"",
            r"jq: error (at <stdin>:3): Unmatched '}' at line 3, column 1
",
            5,
        ),
        (
            r"1
2
}
",
            r#"(input | type), (try input catch "end"), (try input catch "end2"), input_line_number"#,
            r#""number"
2
"end2"
3
"#,
            r"",
            0,
        ),
        (
            r#"1
{"a":
2
3
"#,
            r"[inputs]",
            r"",
            r"jq: error (at <stdin>:4): Expected separator between values at line 5, column 0
",
            5,
        ),
        (
            r#"1
{"a":
2
3
"#,
            r"input",
            r"1
",
            r"",
            0,
        ),
        (
            r#"1
{"a":
2
3
"#,
            r"[input, input]",
            r"",
            r"jq: error (at <stdin>:4): Expected separator between values at line 5, column 0
",
            5,
        ),
        (
            r#"1
{"a":
2
3
"#,
            r"first(inputs)",
            r"1
",
            r"",
            0,
        ),
        (
            r#"1
{"a":
2
3
"#,
            r"[limit(2; inputs)]",
            r"",
            r"jq: error (at <stdin>:4): Expected separator between values at line 5, column 0
",
            5,
        ),
        (
            r#"1
{"a":
2
3
"#,
            r"[inputs | input_line_number]",
            r"",
            r"jq: error (at <stdin>:4): Expected separator between values at line 5, column 0
",
            5,
        ),
        (
            r#"1
{"a":
2
3
"#,
            r#"try input catch "E""#,
            r"1
",
            r"",
            0,
        ),
        (
            r#"1
{"a":
2
3
"#,
            r"[inputs, input_line_number]",
            r"",
            r"jq: error (at <stdin>:4): Expected separator between values at line 5, column 0
",
            5,
        ),
        (
            r#"1
{"a":
2
3
"#,
            r#"[range(5) | try input catch "X"]"#,
            r#"[1,"X","X","X","X"]
"#,
            r"",
            0,
        ),
        (
            r#"1
{"a":
2
3
"#,
            r"last(inputs)",
            r"",
            r"jq: error (at <stdin>:4): Expected separator between values at line 5, column 0
",
            5,
        ),
        (
            r#"1
{"a":
2
3
"#,
            r"isempty(inputs)",
            r"false
",
            r"",
            0,
        ),
        (
            r#"1
{"a":
2
3
"#,
            r"reduce inputs as $x (0; . + 1)",
            r"",
            r"jq: error (at <stdin>:4): Expected separator between values at line 5, column 0
",
            5,
        ),
        (
            r#"1
{"a":
2
3
"#,
            r#"(input | type), (try input catch "end"), (try input catch "end2"), input_line_number"#,
            r#""number"
"end"
"end2"
4
"#,
            r"",
            0,
        ),
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-nc", program], Some(stream))?;
        assert_eq!(stdout, expected_stdout, "{program} over {stream:?}");
        assert_eq!(stderr, expected_stderr, "{program} over {stream:?}");
        assert_eq!(code, expected_code, "{program} over {stream:?}");
    }
    Ok(())
}

/// A stream long enough that a program stopping early would have noticed reading all of it: the
/// answers are the same whether the documents are converted up front or when read.
#[test]
fn test_a_long_stream_stopped_early_or_read_whole_5661() -> Result<()> {
    const DOCUMENTS: usize = 50_000;
    let stream: String = (0..DOCUMENTS)
        .map(|id| format!("{{\"id\":{id},\"s\":\"u{id}\"}}\n"))
        .collect();
    for (program, expected) in [
        ("first(inputs) | .id", "0".to_string()),
        ("[limit(3; inputs) | .id]", "[0,1,2]".to_string()),
        ("[range(5) | input | .id]", "[0,1,2,3,4]".to_string()),
        ("input | input_line_number", "1".to_string()),
        ("[inputs] | length", DOCUMENTS.to_string()),
        ("last(inputs) | .id", (DOCUMENTS - 1).to_string()),
        (
            "reduce inputs as $r (0; . + $r.id)",
            (DOCUMENTS * (DOCUMENTS - 1) / 2).to_string(),
        ),
        ("[inputs | input_line_number] | last", DOCUMENTS.to_string()),
        (
            "first(inputs), input_line_number",
            format!("{{\"id\":0,\"s\":\"u0\"}}\n1"),
        ),
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-nc", program], Some(&stream))?;
        assert_eq!(code, 0, "{program}: {stderr:?}");
        assert_eq!(stdout.trim_end(), expected, "{program}");
    }
    Ok(())
}
