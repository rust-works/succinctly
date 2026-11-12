#!/usr/bin/env python3
"""Check that src/jq/shell_nfkd.rs agrees with Python's own NFKD (#7092).

The table is generated from UnicodeData.txt 15.0.0 by scripts/gen_shell_nfkd.py, and a decomposition never
changes once assigned, so every row must give what yq's `-o=shell` key rule leaves of the code point's NFKD
(ASCII letters and digits and `_` kept, other printable ASCII written `_`, everything else dropped), whatever
Unicode version the running Python has; a Python no newer than 15.0.0 can say no row is missing, and one no older can say none is wrong.
"""

import os
import re
import unicodedata
import unittest

VERSION = tuple(int(n) for n in unicodedata.unidata_version.split("."))
ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def keep(ch):
    cp = ord(ch)
    if ch.isascii() and (ch.isalnum() or ch == "_"):
        return ch
    return "_" if 32 <= cp <= 126 else ""


def table():
    with open(os.path.join(ROOT, "src/jq/shell_nfkd.rs"), encoding="utf-8") as f:
        text = f.read()
    out = {}
    for start, count, value in re.findall(r'\(0x([0-9a-f]+), (\d+), "([^"]*)"\)', text):
        start, count = int(start, 16), int(count)
        for k in range(count):
            out[start + k] = value if count == 1 else chr(ord(value) + k)
    return out


class ShellNfkdTable(unittest.TestCase):
    def test_every_row_agrees_with_pythons_nfkd(self):
        """A decomposition never changes once assigned, so a Python at 15.0.0 or newer agrees with every row."""
        if VERSION < (15, 0, 0):
            self.skipTest("Python's Unicode data (%s) is older than the table's" % unicodedata.unidata_version)
        wrong = []
        for cp, text in table().items():
            want = "".join(keep(c) for c in unicodedata.normalize("NFKD", chr(cp)))
            if text != want:
                wrong.append((hex(cp), text, want))
        self.assertEqual(wrong[:10], [])

    def test_no_code_point_is_missing_that_the_tables_unicode_version_knows(self):
        """Only a Python whose Unicode data is no newer than the table's can say a row is missing: a newer
        one also knows the characters a later release added, which the table (15.0.0) cannot have."""
        if VERSION > (15, 0, 0):
            self.skipTest("Python's Unicode data (%s) is newer than the table's" % unicodedata.unidata_version)
        rows = table()
        missing = []
        for cp in range(0x80, 0x110000):
            if unicodedata.category(chr(cp)) == "Cn":
                continue
            want = "".join(keep(c) for c in unicodedata.normalize("NFKD", chr(cp)))
            if want and cp not in rows:
                missing.append((hex(cp), want))
        self.assertEqual(missing[:10], [])

    def test_no_row_is_for_a_code_point_below_u0080(self):
        self.assertTrue(all(cp >= 0x80 for cp in table()))


if __name__ == "__main__":
    unittest.main()
