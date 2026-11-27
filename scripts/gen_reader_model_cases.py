#!/usr/bin/env python3
"""Regenerate the reader model's differential table from jq 1.7.1 (ADR-0033, #7987 step 3).

Every row is a set of input files and what jq 1.7.1 (`/usr/bin/jq`, the pinned reference) prints for

    jq -nc '[limit(12; repeat(try [input, input_line_number, input_filename]
                              catch [., input_line_number, input_filename]))]' f0 f1 ...

run in a directory holding the files `f0`, `f1`, ..., with `--seq` added for a `seq` row (less the RS it
writes before the output). Each `input`
is one `jq_util_input_next_input` call, so the output names every value, fault and end of input
(`break`) the reader returns, with the position after it, and goes on past the end to show where the
position falls after it. `tests.rs` replays the rows through the model.

A row whose answer the model departs from by a recorded rule names it, and its expected answer is
what `succinctly jq` gives for it, captured with `--succinctly` from a build of `main` (the rule is
implemented there): E2's NUL outside a string (#7966), the bare `n` (#7525), the NaN payload (#7522)
and `--seq`'s read of every record (#7718). The script refuses a departure that jq 1.7.1 does not
show, or one that `succinctly jq` does not give.

    python3 scripts/gen_reader_model_cases.py --succinctly target/release/succinctly \\
        > src/bin/succinctly/jq_runner/reader_model/jq171_cases.rs
"""
import argparse
import os
import subprocess
import sys
import tempfile

JQ = "/usr/bin/jq"
PROGRAM = (
    "[limit(12; repeat(try [input, input_line_number, input_filename] "
    "catch [., input_line_number, input_filename]))]"
)
SP = b" "


def row(name, *files, seq=False, rule=None):
    return {"name": name, "files": list(files), "seq": seq, "rule": rule}


E2 = "E2, #7966: a NUL outside a string is `Invalid character` at its own column"
NO_CUT = "E2, #7966: a newline-less read is not cut at its first NUL (`strlen`, CVE-2026-33948)"
BARE_N = "#7525: a bare `n` is `Invalid numeric literal` whatever an earlier token left"
NAN = "#7522: a NaN with a payload is refused, as in jq 1.8"
SEQ_EVERY = "#7718: `--seq` reads every record, as jq 1.8.2 does"

ROWS = [
    # Values split across files: one parser for the whole run, never told where a file ends.
    row("split_number", b"1", b"2\n"),
    row("split_string", b'"ab', b'cd"\n'),
    row("split_array", b"[1,", b"2]"),
    row("split_literal", b"tr", b"ue\n"),
    row("split_key", b'{"a', b'b":1}\n'),
    row("split_number_over_empty_file", b"12", b"", b"3\n"),
    row("values_on_one_line_no_newline_two_files", b"1 2", b" 3 4"),
    # The end of the input and where the position stands after it.
    row("empty_file", b""),
    row("two_empty_files", b"", b""),
    row("only_newline", b"\n"),
    row("value_then_empty_file", b"1", b""),
    row("value_then_newline_file", b"1", b"\n"),
    row("no_trailing_newline", b"1\n2\n3"),
    row("trailing_newline", b"1\n2\n3\n"),
    row("trailing_space", b"1 2 3 "),
    row("blank_line_at_end", b"3\n\n"),
    row("two_clean_files", b"1\n2\n", b"3\n"),
    row("crlf", b"1\r\n2\r\n"),
    row("nested_object", b'{"a":[1,{"b":null}],"c":"d"}\n'),
    row("adjacent_strings_and_arrays", b'"a""b"1[2]{"c":3}\n'),
    # Read boundaries: `fgets` reads 4,095 bytes or through a newline.
    row("one_then_4093_spaces", b"1" + SP * 4093),
    row("one_then_4094_spaces", b"1" + SP * 4094),
    row("4094_spaces_then_one", SP * 4094 + b"1"),
    row("line_of_4095_with_its_newline", b"1" + SP * 4093 + b"\n2\n"),
    row("line_of_4095_then_newline", b"1" + SP * 4094 + b"\n2\n"),
    row("line_of_4096_then_newline", b"1" + SP * 4095 + b"\n2\n"),
    row("token_cut_by_the_read", SP * 4094 + b"12 3\n"),
    row("string_cut_by_the_read", SP * 4093 + b'"abcd"\n'),
    row("value_after_two_reads", SP * 8189 + b"7\n"),
    row("full_read_ends_the_file", SP * 4094 + b"\n", b"5"),
    row("whole_read_at_end_of_input", b"1" + SP * 4094),
    row("token_fills_the_last_read", SP * 4090 + b"12345"),
    row("long_line_values_across_reads", b"1 " + SP * 8000 + b"2\n"),
    # A fault, and the rest of its read thrown away: jq resumes at its next read.
    row("fault_then_resume_next_line", b"1 x 2\n3\n"),
    row("fault_in_full_read_resumes_at_4095", b"x" + SP * 4090 + b"5 7\n"),
    row("fault_in_full_read_then_line", b"x" + SP * 4094 + b"8\n"),
    row("fault_on_a_long_line_resumes_at_8190", b"1" + SP * 4999 + b"}" + SP * 3500 + b"2 3\n4\n"),
    row("fault_at_eof_on_a_long_line", SP * 5000 + b"tru"),
    row("fault_across_files", b"[1,", b"x]\n2\n"),
    row("fault_at_eof_number", b"1 x"),
    row("close_brace_after_number", b"1}"),
    row("close_brace_after_space", b"1 }"),
    row("close_brace_then_long_line", b"1 } 2" + SP * 6000 + b"3\n4\n"),
    row("unfinished_string_at_eof", b'"abc'),
    row("unfinished_array_at_eof", b"[1,2"),
    row("unfinished_object_at_eof", b'{"a":'),
    row("bad_literals", b"tru\n1\nnul\n2\n"),
    row("invalid_escape", b'"a\\x"\n1\n'),
    row("lone_high_surrogate", b'"\\ud800"\n2\n'),
    row("control_in_string", b'"a\tb"\n3\n'),
    row("string_over_newline", b'"a\n', b'b"\n4\n'),
    row("no_separator_in_array", b"[1 2]\n3\n"),
    row("key_without_colon", b'{"a" 1}\n4\n'),
    row("trailing_comma_array", b"[1,]\n5\n"),
    row("trailing_comma_object", b'{"a":1,}\n6\n'),
    row("unmatched_bracket", b"]\n1\n"),
    row("lone_comma", b",\n1\n"),
    row("lone_colon", b":\n1\n"),
    row("number_key", b"{1:2}\n7\n"),
    row("object_of_strings", b'{"a","b"}\n8\n'),
    row("colon_in_array", b'["a":1]\n9\n'),
    row("adjacent_values", b'1"a"tru\n'),
    row("single_quote", b"'a'\n1\n"),
    # The byte-order mark, matched a byte at a time across reads and files.
    row("bom_whole", b"\xef\xbb\xbf1\n"),
    row("bom_then_fault", b"\xef\xbb\xbf}\n1\n"),
    row("bom_across_files", b"\xef", b"\xbb\xbf1\n"),
    row("bom_across_three_files", b"\xef", b"\xbb", b"\xbf7\n8\n"),
    row("bom_partial_two_at_end", b"\xef\xbb"),
    row("bom_partial_one_at_end", b"\xef"),
    row("bom_malformed_after_one", b"\xef1\n2\n"),
    row("bom_malformed_after_two", b"\xef\xbb1\n"),
    row("bom_malformed_across_files", b"\xef", b"1\n"),
    row("bom_after_empty_file", b"", b"\xef\xbb\xbf5\n"),
    row("bom_in_second_file", b"1\n", b"\xef\xbb\xbf2\n"),
    row("bom_only", b"\xef\xbb\xbf"),
    row("bom_twice", b"\xef\xbb\xbf\xef\xbb\xbf1\n"),
    # The depth limit: 256 stack entries, an object taking two once its key is read.
    # jq prints a value nested past its print depth as `<skipped: too deep>`, so the rows that fill
    # the stack to 256 do it with objects, whose keys take a slot each.
    row("depth_256_objects", b'{"a":' * 128 + b"1" + b"}" * 128 + b"\n2\n"),
    row(
        "depth_256_arrays_and_objects",
        b"[" * 200 + b'{"a":' * 28 + b"1" + b"}" * 28 + b"]" * 200 + b"\n2\n",
    ),
    row("depth_257_arrays", b"[" * 257 + b"]" * 257 + b"\n1\n"),
    row("depth_257_objects", b'{"a":' * 129 + b"1" + b"}" * 129 + b"\n2\n"),
    # A NUL inside a string is the control-character error both releases give.
    row("nul_in_string", b'"a\x00b"\n1\n'),
    row("nul_in_string_without_newline", b'"a\x00b" 3', rule=NO_CUT),
    # E2: a NUL outside a string.
    row("nul_between_values", b"1 \x00 2\n3\n", rule=E2),
    row("nul_glued_to_number", b"12\x003\n4\n", rule=E2),
    row("nul_at_the_start", b"\x00\n5\n", rule=E2),
    row("nul_in_array", b"[1\x00x,2]\n7\n", rule=E2),
    row("nul_after_true", b"true\x00\n1\n", rule=E2),
    row("nul_in_a_newline_less_read", b"1 \x002", rule=E2),
    row("nul_after_a_long_line", b"1" + SP * 5000 + b"\x00 2\n3\n", rule=E2),
    # #7525 and #7522.
    row("bare_n_after_a_u", b'"au" n\n1\n', rule=BARE_N),
    row("bare_n_after_null", b"null n\n1\n", rule=BARE_N),
    row("bare_n_after_a_number", b'"au" 1 n\n2\n'),
    row("nan_payload", b"NaN5\n1\n", rule=NAN),
    # `--seq`.
    row("seq_records", b"\x1e1\n\x1e[2]\n", seq=True),
    row("seq_garbage_before_the_first_rs", b"x y\x1e3\n", seq=True),
    row("seq_fault_resyncs_and_reads_on", b"\x1e} 5\n\x1e6\n", seq=True),
    row("seq_truncated_number", b"\x1e12\x1e3\n", seq=True),
    row("seq_truncated_value", b"\x1e[1\x1e2\n", seq=True),
    row("seq_number_at_eof", b"\x1e12", seq=True),
    row("seq_abandoned_text_at_eof", b"ab", seq=True),
    row("seq_empty_record_in_the_last_buffer", b"\x1e1\n\x1etrue", seq=True, rule=SEQ_EVERY),
]


def run(argv, files, seq, cwd):
    flags = ["--seq"] if seq else []
    names = [f"f{i}" for i in range(len(files))]
    out = subprocess.run(
        [*argv, *flags, "-nc", PROGRAM, *names], cwd=cwd, capture_output=True, check=False
    )
    if out.returncode != 0:
        sys.exit(f"{argv} exited {out.returncode}: {out.stderr.decode(errors='replace')}")
    text = out.stdout.decode("utf-8").rstrip("\n")
    # `--seq` writes an RS before each output.
    return text.removeprefix("\x1e") if seq else text


def capture(case, succinctly):
    with tempfile.TemporaryDirectory() as tmp:
        for i, data in enumerate(case["files"]):
            with open(os.path.join(tmp, f"f{i}"), "wb") as f:
                f.write(data)
        jq171 = run([JQ], case["files"], case["seq"], tmp)
        expect = None
        if case["rule"] is not None:
            if succinctly is None:
                sys.exit(f"{case['name']}: a departure needs --succinctly")
            expect = run([succinctly, "jq"], case["files"], case["seq"], tmp)
            if expect == jq171:
                sys.exit(f"{case['name']}: listed as a departure, but succinctly jq agrees with jq 1.7.1")
    return jq171, expect


def rust_bytes(data):
    out = []
    for b in data:
        if b == 0x22:
            out.append('\\"')
        elif b == 0x5C:
            out.append("\\\\")
        elif 0x20 <= b < 0x7F:
            out.append(chr(b))
        elif b == 0x0A:
            out.append("\\n")
        elif b == 0x09:
            out.append("\\t")
        elif b == 0x0D:
            out.append("\\r")
        else:
            out.append(f"\\x{b:02x}")
    return 'b"' + "".join(out) + '"'


def pieces(data):
    """A file as literal pieces and runs of one byte (32 or more)."""
    out = []
    lit = bytearray()
    i = 0
    while i < len(data):
        j = i
        while j < len(data) and data[j] == data[i]:
            j += 1
        if j - i >= 32:
            if lit:
                out.append(f"Lit({rust_bytes(bytes(lit))})")
                lit = bytearray()
            out.append(f"Run(0x{data[i]:02x}, {j - i})")
        else:
            lit += data[i:j]
        i = j
    if lit or not out:
        out.append(f"Lit({rust_bytes(bytes(lit))})")
    return "&[" + ", ".join(out) + "]"


def rust_str(text):
    return '"' + text.replace("\\", "\\\\").replace('"', '\\"') + '"'


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("--succinctly", help="a build of succinctly from main, for the departures")
    args = parser.parse_args()
    version = subprocess.run([JQ, "--version"], capture_output=True, check=True).stdout.decode()
    # macOS ships 1.7.1 as `jq-1.7.1-apple`, the same sources.
    if version.strip() not in ("jq-1.7.1", "jq-1.7.1-apple"):
        sys.exit(f"{JQ} is {version.strip()}, not jq-1.7.1")
    names = [case["name"] for case in ROWS]
    if len(set(names)) != len(names):
        sys.exit("duplicate row names")
    # The rows run in a temporary directory, so a relative path to the build would not resolve.
    if args.succinctly:
        args.succinctly = os.path.abspath(args.succinctly)
    print("// Generated by `scripts/gen_reader_model_cases.py` from jq 1.7.1 (`/usr/bin/jq`); do not edit.")
    print("const CASES: &[Case] = &[")
    for case in ROWS:
        jq171, expect = capture(case, args.succinctly)
        files = ", ".join(pieces(data) for data in case["files"])
        print("    Case {")
        print(f"        name: {rust_str(case['name'])},")
        print(f"        seq: {'true' if case['seq'] else 'false'},")
        print(f"        files: &[{files}],")
        print(f"        jq171: {rust_str(jq171)},")
        if expect is None:
            print("        departs: None,")
        else:
            print(f"        departs: Some(({rust_str(case['rule'])}, {rust_str(expect)})),")
        print("    },")
    print("];")


if __name__ == "__main__":
    main()
