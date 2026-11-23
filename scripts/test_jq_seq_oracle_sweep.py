#!/usr/bin/env python3
"""Unit tests for scripts/jq-seq-oracle-sweep.py's `artifact()` that need no build and no jq.
Run directly (`python3 scripts/test_jq_seq_oracle_sweep.py`) or via
`python3 -m unittest scripts/test_jq_seq_oracle_sweep.py`.

They pin #7256 and #7525: jq 1.7.1's `check_literal` compares `tokenbuf[1]` with `u` for a bare
`n`. When no earlier token has written index 1 that is uninitialised heap, which differs between
libc allocators (Linux aarch64 holds `u`; macOS and x86_64 glibc do not), and when an earlier
token left a `u` there 1.7.1 says "Invalid literal". jq 1.8.2 and `succinctly jq` never read the
byte (`tokenpos > 1`, jqlang/jq#3324), so against the pinned 1.7.1 the sweep has to skip exactly
those streams: skipping fewer leaves a flake, skipping more stops checking the case it can check,
a bare `n` after a byte that is not `u` (#1723, #6513).

Imported via `importlib` rather than `import`: the script's filename has hyphens.
"""

import importlib.util
import json
import pathlib
import unittest

_SCRIPT_PATH = pathlib.Path(__file__).parent / "jq-seq-oracle-sweep.py"
_spec = importlib.util.spec_from_file_location("jq_seq_oracle_sweep", _SCRIPT_PATH)
sweep = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(sweep)


class UnwrittenTokenbufTests(unittest.TestCase):
    def test_issue_repros_are_artifacts(self):
        # The two streams the Graviton4 leg reported, byte for byte.
        for data in (b"\x1en]b[tba:\x1e[,e\nb}\x1esaab\n", b"\x1e}n}t0n-\ni\nlt0ff:.\x1ea[ \n"):
            self.assertTrue(sweep.reads_unwritten_tokenbuf(data), data)
            self.assertTrue(sweep.artifact(data), data)

    def test_first_bare_n_in_any_position(self):
        for data in (b"\x1en", b"\x1en\n", b"\x1e n ", b"\x1e[n]", b"\x1e{n:1}", b"\x1e,n", b"\x1e\"a\"n"):
            self.assertTrue(sweep.reads_unwritten_tokenbuf(data), data)

    def test_one_byte_tokens_that_write_nothing_do_not_count(self):
        # `t`, `f`, a structure byte, white space and a string of a byte or less leave index 1 alone.
        for data in (b"\x1et n", b"\x1ef n", b"\x1e[ ] n", b"\x1e\"\" n", b"\x1e\"a\" n"):
            self.assertTrue(sweep.reads_unwritten_tokenbuf(data), data)

    def test_earlier_token_that_wrote_a_byte_other_than_u_is_checked_not_skipped(self):
        for data in (
            b"\x1enan n\n",  # a token of two or more bytes leaves its second byte
            b"\x1e12 n\n",
            b"\x1e1 n\n",  # a one-byte number NUL-terminates in place, at index 1
            b"\x1ex n\n",  # so does any other one-byte token off the keyword path
        ):
            self.assertFalse(sweep.reads_unwritten_tokenbuf(data), data)

    def test_earlier_token_that_left_a_u_is_skipped_7525(self):
        # 1.7.1 says "Invalid literal" for the `n`; 1.8.2 and succinctly say "Invalid numeric literal".
        for data in (
            b"\x1enul n\n",  # a literal whose second byte is `u`
            b"\x1enull n\n",
            b"\x1e\"iu\" n\n",  # the issue's repro: a string of two or more bytes may leave one
            b"\x1e\"ab\" n\n",  # (what it leaves is its decoded text, not worked out here)
        ):
            self.assertTrue(sweep.reads_unwritten_tokenbuf(data), data)

    def test_a_quote_that_ends_a_failing_token_opens_no_string_7806(self):
        # `iu` fails at the `"` that ends it; jq resets the parser there, so the `"` opens no string
        # and the `n` after it is a bare token, read against the `u` `iu` left (1.7.1 says
        # "Invalid literal", 1.8.2 and succinctly "Invalid numeric literal").
        for data in (b'\x1eiu"n\n', b'\x1eiu"n,', b'\x1eiu"n"', b'\x1e1 iu"n\n', b'\x1enul"n\n', b'\x1euu"n\n'):
            self.assertTrue(sweep.reads_unwritten_tokenbuf(data), data)
            self.assertTrue(sweep.artifact(data), data)

    def test_a_quote_that_ends_a_valid_word_still_opens_a_string_7806(self):
        # The word is a literal or a number, so nothing fails: the `"` opens a string whose text
        # (`n` and the newline) holds no token, and no bare `n` is read.
        for data in (b'\x1etrue"n\n', b'\x1enull"n\n', b'\x1e12"n\n', b'\x1e-1.5e3"n\n', b'\x1enan"n\n', b'\x1e1"n\n'):
            self.assertFalse(sweep.reads_unwritten_tokenbuf(data), data)

    def test_a_failing_token_that_wrote_a_byte_other_than_u_is_still_checked_7806(self):
        # `ix`, `tru` and `1.2.3` leave `x`, `r` and `.` at index 1: the `n` is checked, not skipped.
        for data in (b'\x1eix"n\n', b'\x1etru"n\n', b'\x1e1.2.3"n\n'):
            self.assertFalse(sweep.reads_unwritten_tokenbuf(data), data)

    def test_valid_words(self):
        for word in (b"true", b"false", b"null", b"nan", b"0", b"12", b"-1", b"+1", b".5", b"1.", b"1e5", b"1E-5", b"infinity", b"-Infinity", b"NaN"):
            self.assertTrue(sweep._valid_word(word), word)
        for word in (b"iu", b"n", b"t", b"tru", b"nul", b"x", b"-", b"1e", b"1.2.3", b"0x1", b"."):
            self.assertFalse(sweep._valid_word(word), word)

    def test_only_the_first_bare_n_is_exposed(self):
        # The first reads the heap; it leaves its own NUL at index 1 for the second.
        self.assertTrue(sweep.reads_unwritten_tokenbuf(b"\x1en n\n"))
        self.assertFalse(sweep.reads_unwritten_tokenbuf(b"\x1e1 n n\n"))
        self.assertFalse(sweep.reads_unwritten_tokenbuf(b"\x1e12 n n\n"))

    def test_string_writes_its_raw_bytes_and_an_rs_ends_it(self):
        # `"\\u0041"` decodes to one byte but leaves its raw `u` at index 1 (#6513).
        self.assertTrue(sweep.reads_unwritten_tokenbuf(b'\x1e"\\u0041" n\n'))
        self.assertTrue(sweep.reads_unwritten_tokenbuf(b'\x1e"a\\nbcd" n\n'))
        # An unclosed string was `tokenadd`ed all the same, up to the RS that ends it...
        self.assertTrue(sweep.reads_unwritten_tokenbuf(b'\x1e"ab\x1en'))
        # ...but a string of one byte before the RS wrote nothing, and the RS is not its content.
        self.assertTrue(sweep.reads_unwritten_tokenbuf(b'\x1e"a\x1en'))
        self.assertTrue(sweep.reads_unwritten_tokenbuf(b'\x1e"\\\x1en'))

    def test_a_parse_error_does_not_skip_what_follows(self):
        # jq resets the parser at an error and reads on, so the `xy` after `]` is tokenised.
        self.assertFalse(sweep.reads_unwritten_tokenbuf(b"\x1e]xy\x1en"))

    def test_streams_without_a_bare_n_are_not_artifacts(self):
        for data in (b"\x1e1\n", b"\x1enul\n", b"\x1enan\n", b"\x1enull\n", b"\x1e[1,2]\n", b"\x1ea n1\n"):
            self.assertFalse(sweep.reads_unwritten_tokenbuf(data), data)

    def test_bytes_before_the_first_rs_are_not_tokenised(self):
        # jq waits for an RS and reads none of it, so this `n` never reaches `check_literal`...
        self.assertFalse(sweep.reads_unwritten_tokenbuf(b"n\x1e1\n"))
        # ...unless a malformed BOM has put the parser back in `Normal`.
        self.assertTrue(sweep.reads_unwritten_tokenbuf(b"\xef[n]\x1e1\n"))

    def test_a_stream_with_no_rs_is_left_to_the_existing_rule(self):
        self.assertFalse(sweep.reads_unwritten_tokenbuf(b"n"))

    def test_artifact_is_what_expect_artifacts_turns_off(self):
        # `main()` calls `artifact()` only without `--expect-artifacts`; the stream still runs there.
        self.assertTrue(sweep.artifact(b"\x1en\n"))
        self.assertFalse(sweep.artifact(b"\x1enan n\n"))


class ReadsPastLastBufferTests(unittest.TestCase):
    """#7718: jq 1.8.2 reads one more, empty, buffer after the last bytes, and `succinctly jq` follows
    it, so against the pinned 1.7.1 the sweep skips the streams 1.7.1 reads in its last buffer: an
    RS scanned there without `-s` (its end-of-stream drop, #2998), and a byte-order mark that breaks
    off (whose EOF report 1.8.2 wipes)."""

    def test_the_issue_rows_are_skipped(self):
        for data in (b'\x1e"a"\n\x1e"b"', b"\x1e[1]\n\x1e[2]", b"\x1e1\n\x1etrue", b"\x1e\x1etrue"):
            self.assertTrue(sweep.reads_past_last_buffer(data, False), data)
            self.assertTrue(sweep.artifact(data), data)

    def test_a_last_buffer_that_is_empty_reads_alike(self):
        # (The last of these has data in its last buffer, and no RS there.)
        for data in (b"\x1e1\n\x1etrue\n", b"\x1e1\n", b"\x1e1\n" + b" " * 4094):
            self.assertFalse(sweep.reads_past_last_buffer(data, False), data)
        # A chunk that fills jq's 4095-byte buffer is followed by an empty read in 1.7.1 too.
        self.assertFalse(sweep.reads_past_last_buffer(b"\x1e1" + b" " * 4093, False))

    def test_the_first_rs_of_the_stream_only_opens_a_record(self):
        self.assertFalse(sweep.reads_past_last_buffer(b"\x1e1 2", False))
        self.assertFalse(sweep.reads_past_last_buffer(b"\x1e[1,", False))

    def test_slurp_never_dropped(self):
        self.assertFalse(sweep.reads_past_last_buffer(b"\x1e1\n\x1etrue", True))
        self.assertFalse(sweep.artifact(b"\x1e1\n\x1e2{", slurp=True))

    def test_a_mark_that_breaks_off_is_skipped_in_both_modes(self):
        for slurp in (False, True):
            self.assertTrue(sweep.reads_past_last_buffer(b"\xef\xbb1 2", slurp))
            self.assertFalse(sweep.reads_past_last_buffer(b"\xef\xbb1 2\n", slurp))

    def test_the_last_buffer_is_what_the_parser_sees(self):
        # A chunk with no newline ends at its first NUL (#7340), and an RS past it is never scanned.
        self.assertFalse(sweep.reads_past_last_buffer(b"\x1e1\n\x002\x1e3", False))
        self.assertEqual(sweep.final_buffer(b"\x1e1\nab"), (3, b"ab"))
        self.assertEqual(sweep.final_buffer(b"\x1e1\nab\n"), (6, b""))


class ProbeInputsTests(unittest.TestCase):
    """#7391: `comparison/jq/probes/*.tsv` feeds the same jq 1.7.1 parser, so a probe whose stdin has a bare
    `n` before any token has written `tokenbuf[1]` makes the reference answer from uninitialised heap: the
    `man-streaming-errors-example` probe (`["a",n]`) failed `Ledger drift` on one run in several and passed
    on the re-run. A probe may name a bare `n` only after a token that settled index 1, and the one
    shape that `reads_unwritten_tokenbuf` skips on purpose, a string of two or more bytes before it
    (`"iu" n`, whose second byte is a `u` that 1.7.1 reads deterministically), is listed here."""

    # The probes that put a `u` at index 1 on purpose (#7525): 1.7.1 says "Invalid literal", always.
    DELIBERATE = {"stream-bare-n-after-u", "stream-errors-bare-n-after-u", "inputs-bare-n-after-u"}

    @staticmethod
    def parsed_by_jq(args):
        # `-R` reads lines, which no JSON parser sees.
        return not any(
            isinstance(a, str) and (a == "--raw-input" or (a.startswith("-") and not a.startswith("--") and "R" in a))
            for a in args
        )

    def exposed_probes(self):
        found = set()
        for path in sorted((pathlib.Path(__file__).parent.parent / "comparison" / "jq" / "probes").glob("*.tsv")):
            for line in path.read_text(encoding="utf-8").splitlines():
                fields = line.split("\t")
                if line.startswith("#") or len(fields) < 4 or fields[3] == "-":
                    continue
                try:
                    args, stdin = json.loads(fields[2]), json.loads(fields[3])
                except ValueError:
                    continue
                if isinstance(stdin, str) and self.parsed_by_jq(args):
                    # An RS first: it puts the stream's parser at its start, as a stream's first byte does.
                    if sweep.reads_unwritten_tokenbuf(b"\x1e" + stdin.encode("utf-8")):
                        found.add(fields[0])
        return found

    def test_no_probe_reads_jqs_uninitialised_byte_but_the_deliberate_ones(self):
        self.assertEqual(self.exposed_probes(), self.DELIBERATE)

    def test_the_manual_example_probe_is_settled_by_an_earlier_token(self):
        self.assertTrue(sweep.reads_unwritten_tokenbuf(b'\x1e["a",n]'))
        self.assertFalse(sweep.reads_unwritten_tokenbuf(b'\x1e["a",1,n]'))


if __name__ == "__main__":
    unittest.main()
