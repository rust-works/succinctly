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


if __name__ == "__main__":
    unittest.main()
