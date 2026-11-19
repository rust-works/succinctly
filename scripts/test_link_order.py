#!/usr/bin/env python3
"""Self-tests for scripts/link-order/link.py, the release binary's function-order linker wrapper (#7260).

No linker, no build: the decisions `link.py` makes from rustc's arguments, the name normalization
that lets one list serve GNU and LLVM demanglers, and the resolution against an object's symbols.
"""
import os
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "link-order"))
import link  # noqa: E402


def args_for(out, *extra):
    return ["-m64", "x.o", "-fuse-ld=lld", *extra, "-o", out]


class WantsOrder(unittest.TestCase):
    def setUp(self):
        self.env = {}

    def test_release_binary_on_linux_with_lld_is_ordered(self):
        a = args_for("/t/target/release/deps/succinctly-0123abcd")
        self.assertEqual(link.wants_order(a, "linux", self.env), "-Wl,--symbol-ordering-file={}")

    def test_cross_target_release_dir_is_ordered(self):
        a = args_for("/t/target/x86_64-unknown-linux-gnu/release/deps/succinctly-0123abcd")
        self.assertIsNotNone(link.wants_order(a, "linux", self.env))

    def test_macos_uses_order_file(self):
        a = ["-arch", "arm64", "x.o", "-o", "/t/target/release/deps/succinctly-0123abcd"]
        self.assertEqual(link.wants_order(a, "darwin", self.env), "-Wl,-order_file,{}")

    def test_gnu_ld_gets_no_option(self):
        # GNU ld has no --symbol-ordering-file; passing it would fail the link.
        a = ["x.o", "-o", "/t/target/release/deps/succinctly-0123abcd"]
        self.assertIsNone(link.wants_order(a, "linux", self.env))

    def test_debug_profile_is_left_alone(self):
        a = args_for("/t/target/debug/deps/succinctly-0123abcd")
        self.assertIsNone(link.wants_order(a, "linux", self.env))

    def test_other_outputs_are_left_alone(self):
        for out in ("/t/target/release/deps/libserde_derive-01.so", "/t/target/release/deps/jq_cli_tests-01",
                    "/t/target/release/build/foo-01/build_script_build-01"):
            self.assertIsNone(link.wants_order(args_for(out), "linux", self.env), out)

    def test_shared_library_and_response_file_are_left_alone(self):
        out = "/t/target/release/deps/succinctly-0123abcd"
        self.assertIsNone(link.wants_order(args_for(out, "-shared"), "linux", self.env))
        self.assertIsNone(link.wants_order(args_for(out, "@/tmp/args"), "linux", self.env))

    def test_opt_out(self):
        a = args_for("/t/target/release/deps/succinctly-0123abcd")
        self.assertIsNone(link.wants_order(a, "linux", {"SUCCINCTLY_LINK_ORDER": "0"}))

    def test_windows_is_left_alone(self):
        a = args_for("/t/target/release/deps/succinctly-0123abcd")
        self.assertIsNone(link.wants_order(a, "win32", self.env))


class Norm(unittest.TestCase):
    def test_gnu_and_llvm_spellings_agree(self):
        # GNU c++filt (libiberty) prints crate hashes and const-generic types; LLVM's does not.
        gnu = "<alloc[12ca63220423dfc2]::string::String as core[a9090a07a7fa7517]::fmt::Write>::write_str"
        self.assertEqual(link.norm(gnu), "<alloc::string::String as core::fmt::Write>::write_str")
        self.assertEqual(link.norm("<x::FatAVX2<1: usize>>::new"), "<x::FatAVX2<1>>::new")
        self.assertEqual(link.norm("f::<a::B, false: bool>"), "f::<a::B, false>")

    def test_lto_suffix_is_dropped(self):
        self.assertEqual(link.norm("core::fmt::write (.837)"), "core::fmt::write")
        self.assertEqual(link.norm("a::b (.specialized.4)"), "a::b")

    def test_slices_and_arrays_keep_their_brackets(self):
        self.assertEqual(link.norm("<[f32]>::sort"), "<[f32]>::sort")
        self.assertEqual(link.norm("<[u8; 4] as a::B>::c"), "<[u8; 4] as a::B>::c")


class Resolve(unittest.TestCase):
    def test_every_copy_is_ordered_once_and_missing_names_are_counted(self):
        mangled = ["_RNv1a", "_RNv1a.llvm.9", "_RNv1b", "_RNv1c"]
        table = link.demangled_to_mangled(mangled, ["a::f", "a::f", "a::g", "a::h"])
        order, missing = link.resolve(["a::g", "gone::x", "a::f", "a::g"], table)
        self.assertEqual(order, ["_RNv1b", "_RNv1a", "_RNv1a.llvm.9"])
        self.assertEqual(missing, 1)

    def test_comments_and_blank_lines_are_not_names(self):
        with tempfile.NamedTemporaryFile("w", suffix=".txt", delete=False) as f:
            f.write("# header\n\na::f\n# -- end of startup block --\na::g\n")
        try:
            self.assertEqual(link.read_list(f.name), ["a::f", "a::g"])
        finally:
            os.unlink(f.name)

    def test_only_object_files_are_read(self):
        with tempfile.TemporaryDirectory() as d:
            o, rlib = os.path.join(d, "a.o"), os.path.join(d, "libb.rlib")
            for p in (o, rlib):
                open(p, "w").close()
            self.assertEqual(link.objects(["-o", "out", o, rlib, os.path.join(d, "absent.o")]), [o])


if __name__ == "__main__":
    unittest.main()
