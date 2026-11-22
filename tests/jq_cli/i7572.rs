//! #7572: a runtime error raised while `fromstream(inputs)`, `inputs[.]` or `in(inputs)` consume the
//! input named the end of the input (`(at FILE:3)`, or `<unknown>` once the last document had been
//! read), where jq 1.7.1 names the line of the input the error was raised on. All three collected
//! every output of their generator before looking at the first, so they read the whole input first;
//! jq runs them as `foreach f as $i`, `E | .[$k]` and `xs | has($x)` do, one output at a time. The
//! inputs jq has not read when the error ends the program are left to the main loop and to `try`, so
//! the exit status and what a `catch` reads afterwards moved too.
//!
//! Every expectation is captured from jq 1.7.1 over a generated matrix (each program over each input,
//! on stdin and from a file). jq 1.8.2 agrees on every location and exit status but one: it names the
//! line (`:0`) where 1.7.1 says `<unknown>` for an error raised after the last document was read (the
//! second error of `--stream '[.] | in(inputs)'` over `[1,2,3]`), a change to every program that errs
//! after its input ends (`[inputs], error("x")` too), so 1.7.1 stays the reference there. Its caught
//! messages carry the 1.8 value preview (jqlang/jq#3478), which keeps 1.7.1 as the reference.

use anyhow::Result;

use crate::spawn_jq;

/// (stdin or file, flags, program, input, jq 1.7.1's stdout, its stderr with `FILE` for the file's
/// path, its exit status).
type Row = (
    &'static str,
    &'static [&'static str],
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    i32,
);

const ROWS: &[Row] = &[
    ("stdin", &["-n", "-c"], "fromstream(inputs)", "1\n2\n", "", "jq: error (at <stdin>:1): Cannot index number with number\n", 5),
    ("file", &["-n", "-c"], "fromstream(inputs)", "1\n2\n", "", "jq: error (at FILE:1): Cannot index number with number\n", 5),
    ("stdin", &["-n", "-c"], "fromstream(inputs)", "1\n[\n", "", "jq: error (at <stdin>:1): Cannot index number with number\n", 5),
    ("file", &["-n", "-c"], "fromstream(inputs)", "1\n[\n", "", "jq: error (at FILE:1): Cannot index number with number\n", 5),
    ("stdin", &["-n", "-c"], "fromstream(inputs)", "1\n\"x\"\n", "", "jq: error (at <stdin>:1): Cannot index number with number\n", 5),
    ("file", &["-n", "-c"], "fromstream(inputs)", "1\n\"x\"\n", "", "jq: error (at FILE:1): Cannot index number with number\n", 5),
    ("stdin", &["-n", "-c"], "fromstream(inputs)", "1\n\n\n", "", "jq: error (at <stdin>:1): Cannot index number with number\n", 5),
    ("file", &["-n", "-c"], "fromstream(inputs)", "1\n\n\n", "", "jq: error (at FILE:1): Cannot index number with number\n", 5),
    ("stdin", &["-n", "-c"], "fromstream(inputs)", "[1,2,3]", "null\n", "", 0),
    ("file", &["-n", "-c"], "fromstream(inputs)", "[1,2,3]", "null\n", "", 0),
    ("stdin", &["-n", "-c"], "fromstream(inputs)", "1 2 3", "", "jq: error (at <stdin>:0): Cannot index number with number\n", 5),
    ("file", &["-n", "-c"], "fromstream(inputs)", "1 2 3", "", "jq: error (at FILE:0): Cannot index number with number\n", 5),
    ("stdin", &["-n", "-c"], "fromstream(inputs)", "[1,2,3]\n", "null\n", "", 0),
    ("file", &["-n", "-c"], "fromstream(inputs)", "[1,2,3]\n", "null\n", "", 0),
    ("stdin", &["-n", "-c"], "fromstream(inputs)", "{\"a\":1}\n[2]\n3", "", "jq: error (at <stdin>:1): Cannot index object with number\n", 5),
    ("file", &["-n", "-c"], "fromstream(inputs)", "{\"a\":1}\n[2]\n3", "", "jq: error (at FILE:1): Cannot index object with number\n", 5),
    ("stdin", &["-n", "-c"], "fromstream(inputs)", "[[0],1]\n[[0]]\n5\n", "[1]\n", "jq: error (at <stdin>:3): Cannot index number with number\n", 5),
    ("file", &["-n", "-c"], "fromstream(inputs)", "[[0],1]\n[[0]]\n5\n", "[1]\n", "jq: error (at FILE:3): Cannot index number with number\n", 5),
    ("stdin", &["-c"], "fromstream(inputs)", "1\n2\n", "", "jq: error (at <stdin>:2): Cannot index number with number\n", 5),
    ("file", &["-c"], "fromstream(inputs)", "1\n2\n", "", "jq: error (at FILE:2): Cannot index number with number\n", 5),
    ("stdin", &["-c"], "fromstream(inputs)", "1\n[\n", "", "jq: error (at <stdin>:2): Unfinished JSON term at EOF at line 3, column 0\n", 5),
    ("file", &["-c"], "fromstream(inputs)", "1\n[\n", "", "jq: error (at FILE:2): Unfinished JSON term at EOF at line 3, column 0\n", 5),
    ("stdin", &["-c"], "fromstream(inputs)", "1\n\"x\"\n", "", "jq: error (at <stdin>:2): Cannot index string with number\n", 5),
    ("file", &["-c"], "fromstream(inputs)", "1\n\"x\"\n", "", "jq: error (at FILE:2): Cannot index string with number\n", 5),
    ("stdin", &["-c"], "fromstream(inputs)", "1\n\n\n", "", "", 0),
    ("file", &["-c"], "fromstream(inputs)", "1\n\n\n", "", "", 0),
    ("stdin", &["-c"], "fromstream(inputs)", "[1,2,3]", "", "", 0),
    ("file", &["-c"], "fromstream(inputs)", "[1,2,3]", "", "", 0),
    ("stdin", &["-c"], "fromstream(inputs)", "1 2 3", "", "jq: error (at <stdin>:0): Cannot index number with number\n", 0),
    ("file", &["-c"], "fromstream(inputs)", "1 2 3", "", "jq: error (at FILE:0): Cannot index number with number\n", 0),
    ("stdin", &["-c"], "fromstream(inputs)", "[1,2,3]\n", "", "", 0),
    ("file", &["-c"], "fromstream(inputs)", "[1,2,3]\n", "", "", 0),
    ("stdin", &["-c"], "fromstream(inputs)", "{\"a\":1}\n[2]\n3", "", "jq: error (at <stdin>:2): Cannot index number with number\n", 5),
    ("file", &["-c"], "fromstream(inputs)", "{\"a\":1}\n[2]\n3", "", "jq: error (at FILE:2): Cannot index number with number\n", 5),
    ("stdin", &["-c"], "fromstream(inputs)", "[[0],1]\n[[0]]\n5\n", "null\n", "jq: error (at <stdin>:3): Cannot index number with number\n", 5),
    ("file", &["-c"], "fromstream(inputs)", "[[0],1]\n[[0]]\n5\n", "null\n", "jq: error (at FILE:3): Cannot index number with number\n", 5),
    ("stdin", &["-n", "-c"], "try fromstream(inputs) catch ., [inputs]", "1\n2\n", "\"Cannot index number with number\"\n[2]\n", "", 0),
    ("file", &["-n", "-c"], "try fromstream(inputs) catch ., [inputs]", "1\n2\n", "\"Cannot index number with number\"\n[2]\n", "", 0),
    ("stdin", &["-n", "-c"], "try fromstream(inputs) catch ., [inputs]", "1\n[\n", "\"Cannot index number with number\"\n", "jq: error (at <stdin>:2): Unfinished JSON term at EOF at line 3, column 0\n", 5),
    ("file", &["-n", "-c"], "try fromstream(inputs) catch ., [inputs]", "1\n[\n", "\"Cannot index number with number\"\n", "jq: error (at FILE:2): Unfinished JSON term at EOF at line 3, column 0\n", 5),
    ("stdin", &["-n", "-c"], "try fromstream(inputs) catch ., [inputs]", "1\n\"x\"\n", "\"Cannot index number with number\"\n[\"x\"]\n", "", 0),
    ("file", &["-n", "-c"], "try fromstream(inputs) catch ., [inputs]", "1\n\"x\"\n", "\"Cannot index number with number\"\n[\"x\"]\n", "", 0),
    ("stdin", &["-n", "-c"], "try fromstream(inputs) catch ., [inputs]", "1\n\n\n", "\"Cannot index number with number\"\n[]\n", "", 0),
    ("file", &["-n", "-c"], "try fromstream(inputs) catch ., [inputs]", "1\n\n\n", "\"Cannot index number with number\"\n[]\n", "", 0),
    ("stdin", &["-n", "-c"], "try fromstream(inputs) catch ., [inputs]", "[1,2,3]", "null\n[]\n", "", 0),
    ("file", &["-n", "-c"], "try fromstream(inputs) catch ., [inputs]", "[1,2,3]", "null\n[]\n", "", 0),
    ("stdin", &["-n", "-c"], "try fromstream(inputs) catch ., [inputs]", "1 2 3", "\"Cannot index number with number\"\n[2,3]\n", "", 0),
    ("file", &["-n", "-c"], "try fromstream(inputs) catch ., [inputs]", "1 2 3", "\"Cannot index number with number\"\n[2,3]\n", "", 0),
    ("stdin", &["-n", "-c"], "try fromstream(inputs) catch ., [inputs]", "[1,2,3]\n", "null\n[]\n", "", 0),
    ("file", &["-n", "-c"], "try fromstream(inputs) catch ., [inputs]", "[1,2,3]\n", "null\n[]\n", "", 0),
    ("stdin", &["-n", "-c"], "try fromstream(inputs) catch ., [inputs]", "{\"a\":1}\n[2]\n3", "\"Cannot index object with number\"\n[[2],3]\n", "", 0),
    ("file", &["-n", "-c"], "try fromstream(inputs) catch ., [inputs]", "{\"a\":1}\n[2]\n3", "\"Cannot index object with number\"\n[[2],3]\n", "", 0),
    ("stdin", &["-n", "-c"], "try fromstream(inputs) catch ., [inputs]", "[[0],1]\n[[0]]\n5\n", "[1]\n\"Cannot index number with number\"\n[]\n", "", 0),
    ("file", &["-n", "-c"], "try fromstream(inputs) catch ., [inputs]", "[[0],1]\n[[0]]\n5\n", "[1]\n\"Cannot index number with number\"\n[]\n", "", 0),
    ("stdin", &["-n", "-c"], "[inputs[.]]", "1\n2\n", "", "jq: error (at <stdin>:1): Cannot index number with null\n", 5),
    ("file", &["-n", "-c"], "[inputs[.]]", "1\n2\n", "", "jq: error (at FILE:1): Cannot index number with null\n", 5),
    ("stdin", &["-n", "-c"], "[inputs[.]]", "1\n[\n", "", "jq: error (at <stdin>:1): Cannot index number with null\n", 5),
    ("file", &["-n", "-c"], "[inputs[.]]", "1\n[\n", "", "jq: error (at FILE:1): Cannot index number with null\n", 5),
    ("stdin", &["-n", "-c"], "[inputs[.]]", "1\n\"x\"\n", "", "jq: error (at <stdin>:1): Cannot index number with null\n", 5),
    ("file", &["-n", "-c"], "[inputs[.]]", "1\n\"x\"\n", "", "jq: error (at FILE:1): Cannot index number with null\n", 5),
    ("stdin", &["-n", "-c"], "[inputs[.]]", "1\n\n\n", "", "jq: error (at <stdin>:1): Cannot index number with null\n", 5),
    ("file", &["-n", "-c"], "[inputs[.]]", "1\n\n\n", "", "jq: error (at FILE:1): Cannot index number with null\n", 5),
    ("stdin", &["-n", "-c"], "[inputs[.]]", "[1,2,3]", "", "jq: error (at <stdin>:0): Cannot index array with null\n", 5),
    ("file", &["-n", "-c"], "[inputs[.]]", "[1,2,3]", "", "jq: error (at FILE:0): Cannot index array with null\n", 5),
    ("stdin", &["-n", "-c"], "[inputs[.]]", "1 2 3", "", "jq: error (at <stdin>:0): Cannot index number with null\n", 5),
    ("file", &["-n", "-c"], "[inputs[.]]", "1 2 3", "", "jq: error (at FILE:0): Cannot index number with null\n", 5),
    ("stdin", &["-n", "-c"], "[inputs[.]]", "[1,2,3]\n", "", "jq: error (at <stdin>:1): Cannot index array with null\n", 5),
    ("file", &["-n", "-c"], "[inputs[.]]", "[1,2,3]\n", "", "jq: error (at FILE:1): Cannot index array with null\n", 5),
    ("stdin", &["-n", "-c"], "[inputs[.]]", "{\"a\":1}\n[2]\n3", "", "jq: error (at <stdin>:1): Cannot index object with null\n", 5),
    ("file", &["-n", "-c"], "[inputs[.]]", "{\"a\":1}\n[2]\n3", "", "jq: error (at FILE:1): Cannot index object with null\n", 5),
    ("stdin", &["-n", "-c"], "[inputs[.]]", "[[0],1]\n[[0]]\n5\n", "", "jq: error (at <stdin>:1): Cannot index array with null\n", 5),
    ("file", &["-n", "-c"], "[inputs[.]]", "[[0],1]\n[[0]]\n5\n", "", "jq: error (at FILE:1): Cannot index array with null\n", 5),
    ("stdin", &["-n", "-c"], "[inputs[null]]", "1\n2\n", "", "jq: error (at <stdin>:1): Cannot index number with null\n", 5),
    ("file", &["-n", "-c"], "[inputs[null]]", "1\n2\n", "", "jq: error (at FILE:1): Cannot index number with null\n", 5),
    ("stdin", &["-n", "-c"], "[inputs[null]]", "1\n[\n", "", "jq: error (at <stdin>:1): Cannot index number with null\n", 5),
    ("file", &["-n", "-c"], "[inputs[null]]", "1\n[\n", "", "jq: error (at FILE:1): Cannot index number with null\n", 5),
    ("stdin", &["-n", "-c"], "[inputs[null]]", "1\n\"x\"\n", "", "jq: error (at <stdin>:1): Cannot index number with null\n", 5),
    ("file", &["-n", "-c"], "[inputs[null]]", "1\n\"x\"\n", "", "jq: error (at FILE:1): Cannot index number with null\n", 5),
    ("stdin", &["-n", "-c"], "[inputs[null]]", "1\n\n\n", "", "jq: error (at <stdin>:1): Cannot index number with null\n", 5),
    ("file", &["-n", "-c"], "[inputs[null]]", "1\n\n\n", "", "jq: error (at FILE:1): Cannot index number with null\n", 5),
    ("stdin", &["-n", "-c"], "[inputs[null]]", "[1,2,3]", "", "jq: error (at <stdin>:0): Cannot index array with null\n", 5),
    ("file", &["-n", "-c"], "[inputs[null]]", "[1,2,3]", "", "jq: error (at FILE:0): Cannot index array with null\n", 5),
    ("stdin", &["-n", "-c"], "[inputs[null]]", "1 2 3", "", "jq: error (at <stdin>:0): Cannot index number with null\n", 5),
    ("file", &["-n", "-c"], "[inputs[null]]", "1 2 3", "", "jq: error (at FILE:0): Cannot index number with null\n", 5),
    ("stdin", &["-n", "-c"], "[inputs[null]]", "[1,2,3]\n", "", "jq: error (at <stdin>:1): Cannot index array with null\n", 5),
    ("file", &["-n", "-c"], "[inputs[null]]", "[1,2,3]\n", "", "jq: error (at FILE:1): Cannot index array with null\n", 5),
    ("stdin", &["-n", "-c"], "[inputs[null]]", "{\"a\":1}\n[2]\n3", "", "jq: error (at <stdin>:1): Cannot index object with null\n", 5),
    ("file", &["-n", "-c"], "[inputs[null]]", "{\"a\":1}\n[2]\n3", "", "jq: error (at FILE:1): Cannot index object with null\n", 5),
    ("stdin", &["-n", "-c"], "[inputs[null]]", "[[0],1]\n[[0]]\n5\n", "", "jq: error (at <stdin>:1): Cannot index array with null\n", 5),
    ("file", &["-n", "-c"], "[inputs[null]]", "[[0],1]\n[[0]]\n5\n", "", "jq: error (at FILE:1): Cannot index array with null\n", 5),
    ("stdin", &["-n", "-c"], "[inputs[0]]", "1\n2\n", "", "jq: error (at <stdin>:1): Cannot index number with number\n", 5),
    ("file", &["-n", "-c"], "[inputs[0]]", "1\n2\n", "", "jq: error (at FILE:1): Cannot index number with number\n", 5),
    ("stdin", &["-n", "-c"], "[inputs[0]]", "1\n[\n", "", "jq: error (at <stdin>:1): Cannot index number with number\n", 5),
    ("file", &["-n", "-c"], "[inputs[0]]", "1\n[\n", "", "jq: error (at FILE:1): Cannot index number with number\n", 5),
    ("stdin", &["-n", "-c"], "[inputs[0]]", "1\n\"x\"\n", "", "jq: error (at <stdin>:1): Cannot index number with number\n", 5),
    ("file", &["-n", "-c"], "[inputs[0]]", "1\n\"x\"\n", "", "jq: error (at FILE:1): Cannot index number with number\n", 5),
    ("stdin", &["-n", "-c"], "[inputs[0]]", "1\n\n\n", "", "jq: error (at <stdin>:1): Cannot index number with number\n", 5),
    ("file", &["-n", "-c"], "[inputs[0]]", "1\n\n\n", "", "jq: error (at FILE:1): Cannot index number with number\n", 5),
    ("stdin", &["-n", "-c"], "[inputs[0]]", "[1,2,3]", "[1]\n", "", 0),
    ("file", &["-n", "-c"], "[inputs[0]]", "[1,2,3]", "[1]\n", "", 0),
    ("stdin", &["-n", "-c"], "[inputs[0]]", "1 2 3", "", "jq: error (at <stdin>:0): Cannot index number with number\n", 5),
    ("file", &["-n", "-c"], "[inputs[0]]", "1 2 3", "", "jq: error (at FILE:0): Cannot index number with number\n", 5),
    ("stdin", &["-n", "-c"], "[inputs[0]]", "[1,2,3]\n", "[1]\n", "", 0),
    ("file", &["-n", "-c"], "[inputs[0]]", "[1,2,3]\n", "[1]\n", "", 0),
    ("stdin", &["-n", "-c"], "[inputs[0]]", "{\"a\":1}\n[2]\n3", "", "jq: error (at <stdin>:1): Cannot index object with number\n", 5),
    ("file", &["-n", "-c"], "[inputs[0]]", "{\"a\":1}\n[2]\n3", "", "jq: error (at FILE:1): Cannot index object with number\n", 5),
    ("stdin", &["-n", "-c"], "[inputs[0]]", "[[0],1]\n[[0]]\n5\n", "", "jq: error (at <stdin>:3): Cannot index number with number\n", 5),
    ("file", &["-n", "-c"], "[inputs[0]]", "[[0],1]\n[[0]]\n5\n", "", "jq: error (at FILE:3): Cannot index number with number\n", 5),
    ("stdin", &["-c"], "[inputs[.]]", "1\n2\n", "", "jq: error (at <stdin>:2): Cannot index number with number\n", 5),
    ("file", &["-c"], "[inputs[.]]", "1\n2\n", "", "jq: error (at FILE:2): Cannot index number with number\n", 5),
    ("stdin", &["-c"], "[inputs[.]]", "1\n[\n", "", "jq: error (at <stdin>:2): Unfinished JSON term at EOF at line 3, column 0\n", 5),
    ("file", &["-c"], "[inputs[.]]", "1\n[\n", "", "jq: error (at FILE:2): Unfinished JSON term at EOF at line 3, column 0\n", 5),
    ("stdin", &["-c"], "[inputs[.]]", "1\n\"x\"\n", "", "jq: error (at <stdin>:2): Cannot index string with number\n", 5),
    ("file", &["-c"], "[inputs[.]]", "1\n\"x\"\n", "", "jq: error (at FILE:2): Cannot index string with number\n", 5),
    ("stdin", &["-c"], "[inputs[.]]", "1\n\n\n", "[]\n", "", 0),
    ("file", &["-c"], "[inputs[.]]", "1\n\n\n", "[]\n", "", 0),
    ("stdin", &["-c"], "[inputs[.]]", "[1,2,3]", "[]\n", "", 0),
    ("file", &["-c"], "[inputs[.]]", "[1,2,3]", "[]\n", "", 0),
    ("stdin", &["-c"], "[inputs[.]]", "1 2 3", "[]\n", "jq: error (at <stdin>:0): Cannot index number with number\n", 0),
    ("file", &["-c"], "[inputs[.]]", "1 2 3", "[]\n", "jq: error (at FILE:0): Cannot index number with number\n", 0),
    ("stdin", &["-c"], "[inputs[.]]", "[1,2,3]\n", "[]\n", "", 0),
    ("file", &["-c"], "[inputs[.]]", "[1,2,3]\n", "[]\n", "", 0),
    ("stdin", &["-c"], "[inputs[.]]", "{\"a\":1}\n[2]\n3", "[]\n", "jq: error (at <stdin>:2): Array/string slice indices must be integers\n", 0),
    ("file", &["-c"], "[inputs[.]]", "{\"a\":1}\n[2]\n3", "[]\n", "jq: error (at FILE:2): Array/string slice indices must be integers\n", 0),
    ("stdin", &["-c"], "[inputs[.]]", "[[0],1]\n[[0]]\n5\n", "", "jq: error (at <stdin>:3): Cannot index number with array\n", 5),
    ("file", &["-c"], "[inputs[.]]", "[[0],1]\n[[0]]\n5\n", "", "jq: error (at FILE:3): Cannot index number with array\n", 5),
    ("stdin", &["-n", "-c"], "try [inputs[null]] catch ., [inputs]", "1\n2\n", "\"Cannot index number with null\"\n[2]\n", "", 0),
    ("file", &["-n", "-c"], "try [inputs[null]] catch ., [inputs]", "1\n2\n", "\"Cannot index number with null\"\n[2]\n", "", 0),
    ("stdin", &["-n", "-c"], "try [inputs[null]] catch ., [inputs]", "1\n[\n", "\"Cannot index number with null\"\n", "jq: error (at <stdin>:2): Unfinished JSON term at EOF at line 3, column 0\n", 5),
    ("file", &["-n", "-c"], "try [inputs[null]] catch ., [inputs]", "1\n[\n", "\"Cannot index number with null\"\n", "jq: error (at FILE:2): Unfinished JSON term at EOF at line 3, column 0\n", 5),
    ("stdin", &["-n", "-c"], "try [inputs[null]] catch ., [inputs]", "1\n\"x\"\n", "\"Cannot index number with null\"\n[\"x\"]\n", "", 0),
    ("file", &["-n", "-c"], "try [inputs[null]] catch ., [inputs]", "1\n\"x\"\n", "\"Cannot index number with null\"\n[\"x\"]\n", "", 0),
    ("stdin", &["-n", "-c"], "try [inputs[null]] catch ., [inputs]", "1\n\n\n", "\"Cannot index number with null\"\n[]\n", "", 0),
    ("file", &["-n", "-c"], "try [inputs[null]] catch ., [inputs]", "1\n\n\n", "\"Cannot index number with null\"\n[]\n", "", 0),
    ("stdin", &["-n", "-c"], "try [inputs[null]] catch ., [inputs]", "[1,2,3]", "\"Cannot index array with null\"\n[]\n", "", 0),
    ("file", &["-n", "-c"], "try [inputs[null]] catch ., [inputs]", "[1,2,3]", "\"Cannot index array with null\"\n[]\n", "", 0),
    ("stdin", &["-n", "-c"], "try [inputs[null]] catch ., [inputs]", "1 2 3", "\"Cannot index number with null\"\n[2,3]\n", "", 0),
    ("file", &["-n", "-c"], "try [inputs[null]] catch ., [inputs]", "1 2 3", "\"Cannot index number with null\"\n[2,3]\n", "", 0),
    ("stdin", &["-n", "-c"], "try [inputs[null]] catch ., [inputs]", "[1,2,3]\n", "\"Cannot index array with null\"\n[]\n", "", 0),
    ("file", &["-n", "-c"], "try [inputs[null]] catch ., [inputs]", "[1,2,3]\n", "\"Cannot index array with null\"\n[]\n", "", 0),
    ("stdin", &["-n", "-c"], "try [inputs[null]] catch ., [inputs]", "{\"a\":1}\n[2]\n3", "\"Cannot index object with null\"\n[[2],3]\n", "", 0),
    ("file", &["-n", "-c"], "try [inputs[null]] catch ., [inputs]", "{\"a\":1}\n[2]\n3", "\"Cannot index object with null\"\n[[2],3]\n", "", 0),
    ("stdin", &["-n", "-c"], "try [inputs[null]] catch ., [inputs]", "[[0],1]\n[[0]]\n5\n", "\"Cannot index array with null\"\n[[[0]],5]\n", "", 0),
    ("file", &["-n", "-c"], "try [inputs[null]] catch ., [inputs]", "[[0],1]\n[[0]]\n5\n", "\"Cannot index array with null\"\n[[[0]],5]\n", "", 0),
    ("stdin", &["-n", "-c", "--stream"], "[inputs[.]]", "1\n2\n", "", "jq: error (at <stdin>:1): Cannot index array with null\n", 5),
    ("file", &["-n", "-c", "--stream"], "[inputs[.]]", "1\n2\n", "", "jq: error (at FILE:1): Cannot index array with null\n", 5),
    ("stdin", &["-n", "-c", "--stream"], "[inputs[.]]", "1\n[\n", "", "jq: error (at <stdin>:1): Cannot index array with null\n", 5),
    ("file", &["-n", "-c", "--stream"], "[inputs[.]]", "1\n[\n", "", "jq: error (at FILE:1): Cannot index array with null\n", 5),
    ("stdin", &["-n", "-c", "--stream"], "[inputs[.]]", "1\n\"x\"\n", "", "jq: error (at <stdin>:1): Cannot index array with null\n", 5),
    ("file", &["-n", "-c", "--stream"], "[inputs[.]]", "1\n\"x\"\n", "", "jq: error (at FILE:1): Cannot index array with null\n", 5),
    ("stdin", &["-n", "-c", "--stream"], "[inputs[.]]", "1\n\n\n", "", "jq: error (at <stdin>:1): Cannot index array with null\n", 5),
    ("file", &["-n", "-c", "--stream"], "[inputs[.]]", "1\n\n\n", "", "jq: error (at FILE:1): Cannot index array with null\n", 5),
    ("stdin", &["-n", "-c", "--stream"], "[inputs[.]]", "[1,2,3]", "", "jq: error (at <stdin>:0): Cannot index array with null\n", 5),
    ("file", &["-n", "-c", "--stream"], "[inputs[.]]", "[1,2,3]", "", "jq: error (at FILE:0): Cannot index array with null\n", 5),
    ("stdin", &["-n", "-c", "--stream"], "[inputs[.]]", "1 2 3", "", "jq: error (at <stdin>:0): Cannot index array with null\n", 5),
    ("file", &["-n", "-c", "--stream"], "[inputs[.]]", "1 2 3", "", "jq: error (at FILE:0): Cannot index array with null\n", 5),
    ("stdin", &["-n", "-c", "--stream"], "[inputs[.]]", "[1,2,3]\n", "", "jq: error (at <stdin>:1): Cannot index array with null\n", 5),
    ("file", &["-n", "-c", "--stream"], "[inputs[.]]", "[1,2,3]\n", "", "jq: error (at FILE:1): Cannot index array with null\n", 5),
    ("stdin", &["-n", "-c", "--stream"], "[inputs[.]]", "{\"a\":1}\n[2]\n3", "", "jq: error (at <stdin>:1): Cannot index array with null\n", 5),
    ("file", &["-n", "-c", "--stream"], "[inputs[.]]", "{\"a\":1}\n[2]\n3", "", "jq: error (at FILE:1): Cannot index array with null\n", 5),
    ("stdin", &["-n", "-c", "--stream"], "[inputs[.]]", "[[0],1]\n[[0]]\n5\n", "", "jq: error (at <stdin>:1): Cannot index array with null\n", 5),
    ("file", &["-n", "-c", "--stream"], "[inputs[.]]", "[[0],1]\n[[0]]\n5\n", "", "jq: error (at FILE:1): Cannot index array with null\n", 5),
    ("stdin", &["-c"], "[.] | in(inputs)", "1\n2\n", "", "jq: error (at <stdin>:2): Cannot check whether number has a array key\n", 5),
    ("file", &["-c"], "[.] | in(inputs)", "1\n2\n", "", "jq: error (at FILE:2): Cannot check whether number has a array key\n", 5),
    ("stdin", &["-c"], "[.] | in(inputs)", "1\n[\n", "", "jq: error (at <stdin>:2): Unfinished JSON term at EOF at line 3, column 0\n", 5),
    ("file", &["-c"], "[.] | in(inputs)", "1\n[\n", "", "jq: error (at FILE:2): Unfinished JSON term at EOF at line 3, column 0\n", 5),
    ("stdin", &["-c"], "[.] | in(inputs)", "1\n\"x\"\n", "", "jq: error (at <stdin>:2): Cannot check whether string has a array key\n", 5),
    ("file", &["-c"], "[.] | in(inputs)", "1\n\"x\"\n", "", "jq: error (at FILE:2): Cannot check whether string has a array key\n", 5),
    ("stdin", &["-c"], "[.] | in(inputs)", "1\n\n\n", "", "", 0),
    ("file", &["-c"], "[.] | in(inputs)", "1\n\n\n", "", "", 0),
    ("stdin", &["-c"], "[.] | in(inputs)", "[1,2,3]", "", "", 0),
    ("file", &["-c"], "[.] | in(inputs)", "[1,2,3]", "", "", 0),
    ("stdin", &["-c"], "[.] | in(inputs)", "1 2 3", "", "jq: error (at <stdin>:0): Cannot check whether number has a array key\n", 0),
    ("file", &["-c"], "[.] | in(inputs)", "1 2 3", "", "jq: error (at FILE:0): Cannot check whether number has a array key\n", 0),
    ("stdin", &["-c"], "[.] | in(inputs)", "[1,2,3]\n", "", "", 0),
    ("file", &["-c"], "[.] | in(inputs)", "[1,2,3]\n", "", "", 0),
    ("stdin", &["-c"], "[.] | in(inputs)", "{\"a\":1}\n[2]\n3", "", "jq: error (at <stdin>:2): Cannot check whether array has a array key\n", 0),
    ("file", &["-c"], "[.] | in(inputs)", "{\"a\":1}\n[2]\n3", "", "jq: error (at FILE:2): Cannot check whether array has a array key\n", 0),
    ("stdin", &["-c"], "[.] | in(inputs)", "[[0],1]\n[[0]]\n5\n", "", "jq: error (at <stdin>:2): Cannot check whether array has a array key\n", 0),
    ("file", &["-c"], "[.] | in(inputs)", "[[0],1]\n[[0]]\n5\n", "", "jq: error (at FILE:2): Cannot check whether array has a array key\n", 0),
    ("stdin", &["-c", "--stream"], "[.] | in(inputs)", "1\n2\n", "", "jq: error (at <stdin>:2): Cannot check whether array has a array key\n", 5),
    ("file", &["-c", "--stream"], "[.] | in(inputs)", "1\n2\n", "", "jq: error (at FILE:2): Cannot check whether array has a array key\n", 5),
    ("stdin", &["-c", "--stream"], "[.] | in(inputs)", "1\n[\n", "", "jq: error (at <stdin>:2): Unfinished JSON term at EOF at line 3, column 0\n", 5),
    ("file", &["-c", "--stream"], "[.] | in(inputs)", "1\n[\n", "", "jq: error (at FILE:2): Unfinished JSON term at EOF at line 3, column 0\n", 5),
    ("stdin", &["-c", "--stream"], "[.] | in(inputs)", "1\n\"x\"\n", "", "jq: error (at <stdin>:2): Cannot check whether array has a array key\n", 5),
    ("file", &["-c", "--stream"], "[.] | in(inputs)", "1\n\"x\"\n", "", "jq: error (at FILE:2): Cannot check whether array has a array key\n", 5),
    ("stdin", &["-c", "--stream"], "[.] | in(inputs)", "1\n\n\n", "", "", 0),
    ("file", &["-c", "--stream"], "[.] | in(inputs)", "1\n\n\n", "", "", 0),
    ("stdin", &["-c", "--stream"], "[.] | in(inputs)", "[1,2,3]", "", "jq: error (at <stdin>:0): Cannot check whether array has a array key\njq: error (at <unknown>): Cannot check whether array has a array key\n", 5),
    ("file", &["-c", "--stream"], "[.] | in(inputs)", "[1,2,3]", "", "jq: error (at FILE:0): Cannot check whether array has a array key\njq: error (at <unknown>): Cannot check whether array has a array key\n", 5),
    ("stdin", &["-c", "--stream"], "[.] | in(inputs)", "1 2 3", "", "jq: error (at <stdin>:0): Cannot check whether array has a array key\n", 0),
    ("file", &["-c", "--stream"], "[.] | in(inputs)", "1 2 3", "", "jq: error (at FILE:0): Cannot check whether array has a array key\n", 0),
    ("stdin", &["-c", "--stream"], "[.] | in(inputs)", "[1,2,3]\n", "", "jq: error (at <stdin>:1): Cannot check whether array has a array key\njq: error (at <stdin>:1): Cannot check whether array has a array key\n", 5),
    ("file", &["-c", "--stream"], "[.] | in(inputs)", "[1,2,3]\n", "", "jq: error (at FILE:1): Cannot check whether array has a array key\njq: error (at FILE:1): Cannot check whether array has a array key\n", 5),
    ("stdin", &["-c", "--stream"], "[.] | in(inputs)", "{\"a\":1}\n[2]\n3", "", "jq: error (at <stdin>:1): Cannot check whether array has a array key\njq: error (at <stdin>:2): Cannot check whether array has a array key\n", 0),
    ("file", &["-c", "--stream"], "[.] | in(inputs)", "{\"a\":1}\n[2]\n3", "", "jq: error (at FILE:1): Cannot check whether array has a array key\njq: error (at FILE:2): Cannot check whether array has a array key\n", 0),
    ("stdin", &["-c", "--stream"], "[.] | in(inputs)", "[[0],1]\n[[0]]\n5\n", "", "jq: error (at <stdin>:1): Cannot check whether array has a array key\njq: error (at <stdin>:1): Cannot check whether array has a array key\njq: error (at <stdin>:2): Cannot check whether array has a array key\njq: error (at <stdin>:3): Cannot check whether array has a array key\n", 5),
    ("file", &["-c", "--stream"], "[.] | in(inputs)", "[[0],1]\n[[0]]\n5\n", "", "jq: error (at FILE:1): Cannot check whether array has a array key\njq: error (at FILE:1): Cannot check whether array has a array key\njq: error (at FILE:2): Cannot check whether array has a array key\njq: error (at FILE:3): Cannot check whether array has a array key\n", 5),
    ("stdin", &["-n", "-c"], "try ([1] | in(inputs)) catch ., [inputs]", "1\n2\n", "\"Cannot check whether number has a array key\"\n[2]\n", "", 0),
    ("file", &["-n", "-c"], "try ([1] | in(inputs)) catch ., [inputs]", "1\n2\n", "\"Cannot check whether number has a array key\"\n[2]\n", "", 0),
    ("stdin", &["-n", "-c"], "try ([1] | in(inputs)) catch ., [inputs]", "1\n[\n", "\"Cannot check whether number has a array key\"\n", "jq: error (at <stdin>:2): Unfinished JSON term at EOF at line 3, column 0\n", 5),
    ("file", &["-n", "-c"], "try ([1] | in(inputs)) catch ., [inputs]", "1\n[\n", "\"Cannot check whether number has a array key\"\n", "jq: error (at FILE:2): Unfinished JSON term at EOF at line 3, column 0\n", 5),
    ("stdin", &["-n", "-c"], "try ([1] | in(inputs)) catch ., [inputs]", "1\n\"x\"\n", "\"Cannot check whether number has a array key\"\n[\"x\"]\n", "", 0),
    ("file", &["-n", "-c"], "try ([1] | in(inputs)) catch ., [inputs]", "1\n\"x\"\n", "\"Cannot check whether number has a array key\"\n[\"x\"]\n", "", 0),
    ("stdin", &["-n", "-c"], "try ([1] | in(inputs)) catch ., [inputs]", "1\n\n\n", "\"Cannot check whether number has a array key\"\n[]\n", "", 0),
    ("file", &["-n", "-c"], "try ([1] | in(inputs)) catch ., [inputs]", "1\n\n\n", "\"Cannot check whether number has a array key\"\n[]\n", "", 0),
    ("stdin", &["-n", "-c"], "try ([1] | in(inputs)) catch ., [inputs]", "[1,2,3]", "\"Cannot check whether array has a array key\"\n[]\n", "", 0),
    ("file", &["-n", "-c"], "try ([1] | in(inputs)) catch ., [inputs]", "[1,2,3]", "\"Cannot check whether array has a array key\"\n[]\n", "", 0),
    ("stdin", &["-n", "-c"], "try ([1] | in(inputs)) catch ., [inputs]", "1 2 3", "\"Cannot check whether number has a array key\"\n[2,3]\n", "", 0),
    ("file", &["-n", "-c"], "try ([1] | in(inputs)) catch ., [inputs]", "1 2 3", "\"Cannot check whether number has a array key\"\n[2,3]\n", "", 0),
    ("stdin", &["-n", "-c"], "try ([1] | in(inputs)) catch ., [inputs]", "[1,2,3]\n", "\"Cannot check whether array has a array key\"\n[]\n", "", 0),
    ("file", &["-n", "-c"], "try ([1] | in(inputs)) catch ., [inputs]", "[1,2,3]\n", "\"Cannot check whether array has a array key\"\n[]\n", "", 0),
    ("stdin", &["-n", "-c"], "try ([1] | in(inputs)) catch ., [inputs]", "{\"a\":1}\n[2]\n3", "\"Cannot check whether object has a array key\"\n[[2],3]\n", "", 0),
    ("file", &["-n", "-c"], "try ([1] | in(inputs)) catch ., [inputs]", "{\"a\":1}\n[2]\n3", "\"Cannot check whether object has a array key\"\n[[2],3]\n", "", 0),
    ("stdin", &["-n", "-c"], "try ([1] | in(inputs)) catch ., [inputs]", "[[0],1]\n[[0]]\n5\n", "\"Cannot check whether array has a array key\"\n[[[0]],5]\n", "", 0),
    ("file", &["-n", "-c"], "try ([1] | in(inputs)) catch ., [inputs]", "[[0],1]\n[[0]]\n5\n", "\"Cannot check whether array has a array key\"\n[[[0]],5]\n", "", 0),
];

fn run(args: &[&str], stdin: &[u8]) -> Result<(String, String, i32)> {
    let (output, status) = spawn_jq(args, Some(stdin))?;
    Ok((
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        status,
    ))
}

#[test]
fn an_error_while_consuming_inputs_names_the_line_jq_names_7572() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let file = dir.path().join("in.json");
    let path = file.to_str().expect("a UTF-8 temporary path");
    let mut failures = Vec::new();
    for &(mode, flags, program, input, stdout, stderr, status) in ROWS {
        let mut args: Vec<&str> = flags.to_vec();
        args.push(program);
        let got = if mode == "file" {
            std::fs::write(&file, input)?;
            args.push(path);
            run(&args, b"")?
        } else {
            run(&args, input.as_bytes())?
        };
        let want = (stdout.to_string(), stderr.replace("FILE", path), status);
        if got != want {
            failures.push(format!(
                "{mode} {flags:?} {program:?} over {input:?}: got {got:?}, want {want:?}"
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} rows differ:\n{}",
        failures.len(),
        ROWS.len(),
        failures.join("\n")
    );
    Ok(())
}

/// (program over `null`, jq 1.7.1's stdout, its stderr, its exit status): the effect of a later output
/// of the generator (`stderr`, `debug`) is not reached once an earlier one raises.
const EFFECTS: &[(&str, &str, &str, i32)] = &[
    (
        "[(1, (\"x\"|stderr))[null]]",
        "",
        "jq: error (at <stdin>:0): Cannot index number with null\n",
        5,
    ),
    (
        "[(1, (\"x\"|stderr))[.]]",
        "",
        "jq: error (at <stdin>:0): Cannot index number with null\n",
        5,
    ),
    (
        "fromstream(1, (\"x\"|stderr))",
        "",
        "jq: error (at <stdin>:0): Cannot index number with number\n",
        5,
    ),
    (
        "[fromstream(([[0],1],[[0]]), (\"x\"|stderr), 2)]",
        "",
        "xjq: error (at <stdin>:0): Cannot index string with number\n",
        5,
    ),
    (
        "[1] | in(1, (\"x\"|stderr))",
        "",
        "jq: error (at <stdin>:0): Cannot check whether number has a array key\n",
        5,
    ),
    (
        "try ([1] | in(1, (\"x\"|stderr))) catch .",
        "\"Cannot check whether number has a array key\"\n",
        "",
        0,
    ),
    (
        "[((1, (\"x\"|stderr)) | tostring)[0:1]]",
        "[\"1\",\"x\"]\n",
        "x",
        0,
    ),
    (
        "[(1, (\"x\"|debug))[.]]",
        "",
        "jq: error (at <stdin>:0): Cannot index number with null\n",
        5,
    ),
    ("[(1, 2)[null]?]", "[]\n", "", 0),
    (
        "[try ((1, (\"x\"|stderr))[null]) catch .]",
        "[\"Cannot index number with null\"]\n",
        "",
        0,
    ),
];

#[test]
fn a_later_output_s_effect_is_not_reached_after_an_earlier_one_raises_7572() -> Result<()> {
    for &(program, stdout, stderr, status) in EFFECTS {
        let got = run(&["-c", program], b"null")?;
        assert_eq!(
            got,
            (stdout.to_string(), stderr.to_string(), status),
            "{program}"
        );
    }
    Ok(())
}
