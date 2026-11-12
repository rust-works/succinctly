#!/usr/bin/env python3
"""Check that src/jq/shell_nfkd.rs agrees with Python's own NFKD (#7092).

The table is generated from UnicodeData.txt 15.0.0 by scripts/gen_shell_nfkd.py, and a decomposition never
changes once assigned, so every code point the running Python's Unicode data knows must give the text the
table gives: what yq's `-o=shell` key rule leaves of the code point's NFKD (ASCII letters and digits and `_`
kept, other printable ASCII written `_`, everything else dropped).
"""

import os
import re
import unicodedata
import unittest

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def keep(ch):
    cp = ord(ch)
    if ch.isascii() and (ch.isalnum() or ch == "_"):
        return ch
    return "_" if 32 <= cp <= 126 else ""


def table():
    text = open(os.path.join(ROOT, "src/jq/shell_nfkd.rs"), encoding="utf-8").read()
    out = {}
    for start, count, value in re.findall(r'\(0x([0-9a-f]+), (\d+), "([^"]*)"\)', text):
        start, count = int(start, 16), int(count)
        for k in range(count):
            out[start + k] = value if count == 1 else chr(ord(value) + k)
    return out


class ShellNfkdTable(unittest.TestCase):
    def test_every_code_point_python_knows_agrees_with_the_table(self):
        rows = table()
        assigned = [cp for cp in range(0x80, 0x110000) if unicodedata.category(chr(cp)) != "Cn"]
        wrong = []
        for cp in assigned:
            want = "".join(keep(c) for c in unicodedata.normalize("NFKD", chr(cp)))
            if rows.get(cp, "") != want:
                wrong.append((hex(cp), rows.get(cp, ""), want))
        self.assertEqual(wrong[:10], [])

    def test_no_row_is_for_a_code_point_below_u0080(self):
        self.assertTrue(all(cp >= 0x80 for cp in table()))


if __name__ == "__main__":
    unittest.main()
