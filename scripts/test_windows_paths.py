#!/usr/bin/env python3
"""Self-tests for scripts/windows_paths.py (#7953)."""

import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import windows_paths  # noqa: E402


class WindowsPathsTest(unittest.TestCase):
    def test_the_file_that_broke_the_release(self):
        self.assertEqual(
            windows_paths.bad_paths(["src/util/simd/nul.rs"]),
            [("src/util/simd/nul.rs", "`nul.rs` is a reserved device name")],
        )

    def test_reserved_names_in_any_case_with_or_without_an_extension(self):
        for name in ["CON", "prn.txt", "Aux.rs", "nul", "COM1.log", "lpt9", "com¹.x", "NUL.tar.gz"]:
            self.assertIsNotNone(windows_paths.problem(name), name)

    def test_a_reserved_name_as_a_directory(self):
        self.assertEqual(len(windows_paths.bad_paths(["docs/aux/readme.md"])), 1)

    def test_names_that_only_contain_a_reserved_word_are_fine(self):
        for name in ["nul_byte.rs", "i5654_nul.rs", "null.rs", "console.rs", "com10", "lpt", "auxiliary.md", "conf.d"]:
            self.assertIsNone(windows_paths.problem(name), name)

    def test_forbidden_characters_and_trailing_dot_or_space(self):
        for name in ["a:b", "what?", "star*", 'q"t', "pipe|x", "lt<", "gt>", "back\\slash", "tab\tx", "dot.", "space "]:
            self.assertIsNotNone(windows_paths.problem(name), repr(name))

    def test_the_repository_passes(self):
        self.assertEqual(windows_paths.bad_paths(windows_paths.tracked_paths()), [])


if __name__ == "__main__":
    unittest.main()
