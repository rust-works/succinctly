#!/usr/bin/env python3
"""Self-tests for scripts/link-order/link.py, the release binary's function-order linker wrapper (#7260).

No linker, no build: the decisions `link.py` makes from rustc's arguments, the name normalization
that lets one list serve GNU and LLVM demanglers, the resolution against an object's symbols, and
`generate.py`'s staleness check of the wrapper's log (and its `--warn`) and its macOS pass (#7894,
#7826), and `watch.py`'s reading of the perf-guard log.
"""
import os
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "link-order"))
import generate  # noqa: E402
import link  # noqa: E402
import watch  # noqa: E402


def args_for(out, *extra):
    return ["-m64", "x.o", "-fuse-ld=lld", *extra, "-o", out]


class WantsOrder(unittest.TestCase):
    def setUp(self):
        self.env = {}

    def test_release_binary_on_linux_with_lld_is_ordered(self):
        a = args_for("/t/target/release/deps/succinctly-0123abcd")
        self.assertEqual(link.wants_order(a, "linux", self.env)[:2], ("-Wl,--symbol-ordering-file={}", []))

    def test_cross_target_release_dir_is_ordered(self):
        a = args_for("/t/target/x86_64-unknown-linux-gnu/release/deps/succinctly-0123abcd")
        self.assertIsNotNone(link.wants_order(a, "linux", self.env)[0])

    def test_cargo_1_100_build_dir_layout_is_ordered(self):
        # cargo 1.100 links into build/<pkg>/<hash>/out/ and copies the binary up (#7260's CI).
        a = args_for("/t/target/release/build/succinctly/745a72d7e8e86ffb/out/succinctly")
        self.assertIsNotNone(link.wants_order(a, "linux", self.env)[0])
        a = args_for("/t/target/debug/build/succinctly/745a72d7e8e86ffb/out/succinctly")
        self.assertIsNone(link.wants_order(a, "linux", self.env)[0])
        a = args_for("/t/target/release/build/succinctly/194701fa9bc3b178/out/generate_pfsm_tables")
        self.assertIsNone(link.wants_order(a, "linux", self.env)[0])

    def test_macos_uses_order_file(self):
        a = ["-arch", "arm64", "x.o", "-o", "/t/target/release/deps/succinctly-0123abcd"]
        self.assertEqual(link.wants_order(a, "darwin", self.env)[0], "-Wl,-order_file,{}")

    def test_gnu_ld_gets_no_option(self):
        # GNU ld has no --symbol-ordering-file; passing it would fail the link.
        a = ["x.o", "-o", "/t/target/release/deps/succinctly-0123abcd"]
        self.assertIsNone(link.wants_order(a, "linux", self.env)[0])

    def test_rustup_lld_is_added_when_rustc_names_no_linker(self):
        with tempfile.TemporaryDirectory() as root:
            lib = os.path.join(root, "lib", "rustlib", "x86_64-unknown-linux-gnu", "lib")
            gcc_ld = os.path.join(root, "lib", "rustlib", "x86_64-unknown-linux-gnu", "bin", "gcc-ld")
            os.makedirs(lib)
            os.makedirs(gcc_ld)
            open(os.path.join(gcc_ld, "ld.lld"), "w").close()
            a = ["x.o", "-L", lib, "-o", "/t/target/release/deps/succinctly-0123abcd"]
            opt, driver, _ = link.wants_order(a, "linux", self.env)
            self.assertEqual(opt, "-Wl,--symbol-ordering-file={}")
            self.assertEqual(driver, ["-B" + gcc_ld, "-fuse-ld=lld"])

    def test_another_chosen_linker_is_respected(self):
        a = ["x.o", "-fuse-ld=mold", "-o", "/t/target/release/deps/succinctly-0123abcd"]
        self.assertIsNone(link.wants_order(a, "linux", self.env)[0])

    def test_debug_profile_is_left_alone(self):
        a = args_for("/t/target/debug/deps/succinctly-0123abcd")
        self.assertIsNone(link.wants_order(a, "linux", self.env)[0])

    def test_other_outputs_are_left_alone(self):
        for out in ("/t/target/release/deps/libserde_derive-01.so", "/t/target/release/deps/jq_cli_tests-01",
                    "/t/target/release/build/foo-01/build_script_build-01"):
            self.assertIsNone(link.wants_order(args_for(out), "linux", self.env)[0], out)

    def test_shared_library_is_left_alone(self):
        out = "/t/target/release/deps/succinctly-0123abcd"
        self.assertIsNone(link.wants_order(args_for(out, "-shared"), "linux", self.env)[0])

    def test_response_file_is_read(self):
        with tempfile.NamedTemporaryFile("w", delete=False) as f:
            f.write("x.o\n-fuse-ld=lld\n-o\n/t/target/release/deps/succinctly-01\n/a\\ b/c.o\n")
        try:
            args = link.expand(["-m64", "@" + f.name])
            self.assertEqual(args, ["-m64", "x.o", "-fuse-ld=lld", "-o", "/t/target/release/deps/succinctly-01", "/a b/c.o"])
            self.assertIsNotNone(link.wants_order(args, "linux", self.env)[0])
        finally:
            os.unlink(f.name)

    def test_opt_out(self):
        a = args_for("/t/target/release/deps/succinctly-0123abcd")
        self.assertIsNone(link.wants_order(a, "linux", {"SUCCINCTLY_LINK_ORDER": "0"})[0])

    def test_windows_is_left_alone(self):
        a = args_for("/t/target/release/deps/succinctly-0123abcd")
        self.assertIsNone(link.wants_order(a, "win32", self.env)[0])


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
        order, missing = link.resolve(["a::g", "gone::x", "a::f", "a::g"], table, "darwin")
        self.assertEqual(order, ["_RNv1b", "_RNv1a", "_RNv1a.llvm.9"])
        self.assertEqual(missing, 1)

    def test_linux_orders_the_c_runtime_start_up_code_first_7412(self):
        # `_start` and crtbeginS.o's functions are in no Rust object, so they are passed by name.
        table = link.demangled_to_mangled(["_RNv1a", "main"], ["a::f", "main"])
        order, missing = link.resolve(["_start", "main", "a::f"], table, "linux")
        self.assertEqual(order, link.CRT_STARTUP + ["main", "_RNv1a"])
        self.assertEqual(order[0], "_start")
        self.assertEqual(missing, 0)
        order, _ = link.resolve(["_start", "main"], table, "darwin")
        self.assertEqual(order, ["main"])

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


def write(path, text):
    with open(path, "w") as f:
        f.write(text)


class Stale(unittest.TestCase):
    """`generate.py stale`: a list too many of whose names no longer resolve fails the build (#7894)."""

    FRESH = "1454 listed, 1405 found, 1458 symbols ordered (rustc links with lld)\n"
    DRIFTED = "1455 listed, 1339 found, 1446 symbols ordered (rustc links with lld)\n"

    def run_stale(self, path, limit=generate.STALE_PERCENT):
        import contextlib
        import io
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            ok = generate.stale(path, limit)
        return ok, out.getvalue()

    def test_share_is_read_from_the_wrappers_log_line(self):
        listed, found, share = generate.stale_share(self.FRESH)
        self.assertEqual((listed, found), (1454, 1405))
        self.assertAlmostEqual(share, 3.37, places=2)
        self.assertIsNone(generate.stale_share("plain link: no lld: GNU ld has no --symbol-ordering-file\n"))
        self.assertIsNone(generate.stale_share("0 listed, 0 found, 5 symbols ordered (ld64)\n"))

    def test_the_log_format_is_the_one_link_py_writes(self):
        # link.py's summary line and this parser must not drift apart: read link.py's own format string.
        with open(os.path.join(os.path.dirname(link.__file__), "link.py")) as f:
            src = f.read()
        self.assertIn('listed, {len(names) - missing} found, {len(order)} symbols "', src)
        self.assertIsNotNone(generate.stale_share("7 listed, 6 found, 11 symbols ordered (ld64)"))

    def test_the_list_as_generated_passes_and_the_list_of_7894_fails(self):
        # 3.4% unresolved on the day the list was generated, 8.0% after 222 merges (#7894).
        with tempfile.TemporaryDirectory() as d:
            log = os.path.join(d, "succinctly-01.link-order.log")
            write(log, self.FRESH)
            ok, out = self.run_stale(log)
            self.assertTrue(ok, out)
            self.assertIn("1454 listed, 1405 found, 49 no longer resolve (3.4%", out)
            write(log, self.DRIFTED)
            ok, out = self.run_stale(log)
            self.assertFalse(ok, out)
            self.assertIn("116 no longer resolve (8.0%", out)
            self.assertIn("generate.py linux", out)
            self.assertTrue(self.run_stale(log, 10)[0])

    def test_a_plain_link_or_no_log_fails_instead_of_passing_unchecked(self):
        with tempfile.TemporaryDirectory() as d:
            ok, out = self.run_stale(d)
            self.assertFalse(ok)
            self.assertIn("no link-order log", out)
            log = os.path.join(d, "x.link-order.log")
            write(log, "plain link: rustc chose another linker: -fuse-ld=mold\n")
            ok, out = self.run_stale(log)
            self.assertFalse(ok)
            self.assertIn("not ordered", out)

    def test_the_newest_log_of_either_cargo_layout_is_read(self):
        with tempfile.TemporaryDirectory() as d:
            deps = os.path.join(d, "deps")
            out_dir = os.path.join(d, "build", "succinctly", "745a72d7e8e86ffb", "out")
            os.makedirs(deps)
            os.makedirs(out_dir)
            old, new = os.path.join(deps, "succinctly-01.link-order.log"), os.path.join(out_dir, "succinctly.link-order.log")
            write(old, self.DRIFTED)
            write(new, self.FRESH)
            os.utime(old, (1, 1))
            self.assertEqual(generate.newest_log(d), new)
            os.utime(old, None)
            os.utime(new, (1, 1))
            self.assertEqual(generate.newest_log(d), old)
            # another binary's log (a test executable is never ordered, but be exact about the name)
            write(os.path.join(deps, "jq_cli_tests-02.link-order.log"), self.FRESH)
            os.utime(new, None)
            os.utime(old, (1, 1))
            self.assertEqual(generate.newest_log(d), new)


class StaleWarn(unittest.TestCase):
    """`stale --warn` (pull requests): a warning annotation, and a pass."""

    def run_stale(self, log, warn=False):
        import contextlib
        import io
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            ok = generate.stale(log, generate.STALE_PERCENT, warn)
        return ok, out.getvalue()

    def test_warn_passes_a_stale_list_with_an_annotation(self):
        with tempfile.TemporaryDirectory() as d:
            log = os.path.join(d, "succinctly-01.link-order.log")
            write(log, Stale.DRIFTED)
            ok, out = self.run_stale(log, warn=True)
            self.assertTrue(ok, out)
            self.assertIn("::warning title=link-order list is stale (8.0% > 5%)::", out)
            ok, out = self.run_stale(log)
            self.assertFalse(ok, out)
            self.assertNotIn("::warning", out)
            write(log, Stale.FRESH)
            ok, out = self.run_stale(log, warn=True)
            self.assertTrue(ok)
            self.assertNotIn("::warning", out)

    def test_warn_does_not_pass_a_plain_link(self):
        # `--warn` excuses a stale list, not a link the wrapper did not order.
        with tempfile.TemporaryDirectory() as d:
            log = os.path.join(d, "succinctly-01.link-order.log")
            write(log, "plain link: no symbols.txt\n")
            ok, _ = self.run_stale(log, warn=True)
            self.assertFalse(ok)

    def test_the_command_line_takes_warn_anywhere(self):
        with tempfile.TemporaryDirectory() as d:
            log = os.path.join(d, "succinctly-01.link-order.log")
            write(log, Stale.DRIFTED)
            import contextlib
            import io
            for argv, code in ((["stale", log], 1), (["stale", log, "--warn"], 0), (["stale", "--warn", log], 0),
                               (["stale", log, "9"], 0)):
                with contextlib.redirect_stdout(io.StringIO()), self.assertRaises(SystemExit) as e:
                    generate.main(argv)
                self.assertEqual(e.exception.code, code, argv)


class Watch(unittest.TestCase):
    LOG = ("2026-11-26T06:40:00.1Z link-order list: 1536 listed, 1468 found, 68 no longer resolve "
           "(4.4%, limit 5%)\n")

    def test_the_line_stale_prints_is_parsed(self):
        self.assertEqual(watch.parse(self.LOG), (1536, 1468, 68, 4.4, 5.0))
        self.assertIsNone(watch.parse("1455 listed, 1338 found, 1442 symbols ordered (rustc links with lld)"))

    def test_the_parser_reads_the_line_stale_writes(self):
        import contextlib
        import io
        with tempfile.TemporaryDirectory() as d:
            log = os.path.join(d, "succinctly-01.link-order.log")
            write(log, "1536 listed, 1468 found, 1572 symbols ordered (rustc links with lld)\n")
            out = io.StringIO()
            with contextlib.redirect_stdout(out):
                generate.stale(log, generate.STALE_PERCENT)
        self.assertEqual(watch.parse(out.getvalue()), (1536, 1468, 68, 4.4, 5.0))

    def test_an_issue_is_opened_once_and_then_edited(self):
        self.assertIsNone(watch.decide(3.9, 4.0, []))
        self.assertIsNone(watch.decide(4.0, 4.0, [12]))
        self.assertEqual(watch.decide(4.4, 4.0, []), "create")
        self.assertEqual(watch.decide(4.4, 4.0, [12]), "edit")
        run = {"head_sha": "0dac2e2d7fc9", "html_url": "https://example.invalid/run/1"}
        text = watch.body(run, watch.parse(self.LOG), 4.0)
        self.assertIn("68 no longer resolve, 4.4%", text)
        self.assertIn("generate.py linux", text)


class MacosPass(unittest.TestCase):
    """`generate.py macos` adds every platform-named function the list lacks: it is not a sample (#7826)."""

    def test_every_platform_named_function_the_list_lacks_is_added(self):
        demangled = ["succinctly::json::simd::neon::process_chunk_standard", "core::fmt::write",
                     "<&core::core_arch::arm_shared::neon::uint8x16_t as core::fmt::Debug>::fmt",
                     "std::sys::thread_local::guard::apple::enable::run_dtors",
                     "memchr::arch::aarch64::neon::memchr::memchr_raw", "succinctly::json::simd::avx2::f"]
        have = {link.norm("memchr::arch::aarch64::neon::memchr::memchr_raw"), "core::fmt::write"}
        self.assertEqual(generate.platform_names(demangled, have), [
            "<&core::core_arch::arm_shared::neon::uint8x16_t as core::fmt::Debug>::fmt",
            "std::sys::thread_local::guard::apple::enable::run_dtors",
            "succinctly::json::simd::neon::process_chunk_standard"])

    def test_additions_close_the_startup_block_and_the_rest_keeps_its_order(self):
        syms = [(0, 4, "_a", "b::neon::k"), (4, 4, "_b", "a::plain"), (8, 4, "_c", "a::aarch64::z"), (12, 4, "_d", "x::listed_neon")]
        real = generate.nm_pairs
        generate.nm_pairs = lambda binary: syms
        try:
            with tempfile.NamedTemporaryFile("w", suffix=".txt", delete=False) as f:
                f.write("# header\nmain\n" + generate.MARK + "\nx::listed_neon\nhot::f\n")
            import contextlib
            import io
            err = io.StringIO()
            with contextlib.redirect_stderr(err):
                out = generate.macos("target/release/succinctly", f.name)
        finally:
            generate.nm_pairs = real
            os.unlink(f.name)
        self.assertEqual(out, ["# header", "main", "a::aarch64::z", "b::neon::k", generate.MARK, "x::listed_neon", "hot::f"])
        self.assertIn("added 2 platform-named functions", err.getvalue())
        self.assertIn("does not sample", err.getvalue())


if __name__ == "__main__":
    unittest.main()
