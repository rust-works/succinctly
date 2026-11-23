#!/usr/bin/env python3
"""Unit tests for scripts/perf-guard.py that need no binary, valgrind, or
built fixtures -- pure checks on the script's own logic. Run directly
(`python3 scripts/test_perf_guard.py`) or via `python3 -m unittest
scripts/test_perf_guard.py`; wired into CI's `perf-guard` job (#3169) so a
regression here fails fast, before either matrix leg's expensive
valgrind-based measurement even starts.

Imported via `importlib` rather than `import perf_guard`: the script's
filename has a hyphen, which is not a valid Python module identifier.
"""

import contextlib
import importlib.util
import io
import json
import os
import pathlib
import tempfile
import unittest
from unittest import mock

_SCRIPT_PATH = pathlib.Path(__file__).parent / "perf-guard.py"
_spec = importlib.util.spec_from_file_location("perf_guard", _SCRIPT_PATH)
perf_guard = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(perf_guard)


class FailureAdviceModeTests(unittest.TestCase):
    """#3169: the closing failure advice must depend on whether this run
    compared against `--baseline-binary` (CI's mode on every PR/push run,
    where `--update-baseline` is both rejected outright and would not
    consult the checked-in baseline it can't be run against anyway) or the
    checked-in baseline file (a local `--check`, where `--update-baseline`
    is the correct and only advice)."""

    def test_baseline_binary_mode_never_recommends_update_baseline(self):
        # The advice may still *mention* --update-baseline to say it can't
        # help (which it does, deliberately) -- what must never appear is
        # the recommending phrase itself.
        advice = perf_guard.failure_advice(baseline_binary=True)
        self.assertNotIn("re-run with --update-baseline", advice)

    def test_baseline_binary_mode_points_at_query_thresholds(self):
        advice = perf_guard.failure_advice(baseline_binary=True)
        self.assertIn("QUERY_THRESHOLDS", advice)

    def test_checked_in_baseline_mode_advises_update_baseline(self):
        advice = perf_guard.failure_advice(baseline_binary=False)
        self.assertIn("--update-baseline", advice)

    def test_checked_in_baseline_mode_does_not_mention_query_thresholds(self):
        # Not because an override is wrong in this mode too, but because
        # --update-baseline is the direct, correct fix here -- unlike
        # --baseline-binary mode, where it's rejected outright (#3169).
        advice = perf_guard.failure_advice(baseline_binary=False)
        self.assertNotIn("QUERY_THRESHOLDS", advice)


class RawInputRowTests(unittest.TestCase):
    """#3479: the `-R` rows read a derived one-record-per-line fixture and pass
    `-R`; a row listed in one place and not the other would measure a shape it
    does not name (a line-oriented filter over the pretty-printed document, or
    the document through `-R` as one string), and still report a number."""

    def test_every_lines_row_is_a_yq_row_passing_dash_r(self):
        queries = {q[0]: q for q in perf_guard.QUERIES}
        self.assertTrue(perf_guard.LINES_FIXTURE_ROWS)
        for query_id in perf_guard.LINES_FIXTURE_ROWS:
            self.assertIn(query_id, queries)
            self.assertEqual(queries[query_id][3], "yq", query_id)
            self.assertIn("-R", perf_guard.QUERY_FLAGS[query_id], query_id)

    def test_every_dash_r_row_gets_the_lines_fixture(self):
        raw = {k for k, flags in perf_guard.QUERY_FLAGS.items() if "-R" in flags}
        self.assertEqual(raw, set(perf_guard.LINES_FIXTURE_ROWS))

    def test_every_flagged_row_is_a_query(self):
        ids = {q[0] for q in perf_guard.QUERIES}
        self.assertLessEqual(set(perf_guard.QUERY_FLAGS), ids)

    def test_the_partial_slice_row_that_3535_missed_stays_in_the_guard(self):
        # A presence ratchet only: it stops the row being deleted or reworded
        # away, and cannot tell whether `.[0:3]` still reaches the evaluator's
        # catch-all arm (a native slice arm would keep this green while the row
        # measured a different path). That is `registered_document_bridges_
        # without_reindexing_3479`'s job, not this file's.
        filters = {q[4] for q in perf_guard.QUERIES if q[0] in perf_guard.LINES_FIXTURE_ROWS}
        self.assertIn(".[0:3]", filters)


class SeqLfLedRowTests(unittest.TestCase):
    """#7260: the `--seq` rows read their stream behind one LF, so they keep reaching the route that
    walks a stream whole (a lone `--seq` file of 64 KiB or more is otherwise read a window at a
    time). A row left on the generated stream would still report a number, for another route."""

    def test_the_lf_led_rows_are_the_seq_rows_of_the_stream(self):
        seq_rows = {q[0] for q in perf_guard.QUERIES if (q[1], q[2]) == perf_guard.SEQ_LF_LED_SOURCE}
        self.assertEqual(seq_rows, set(perf_guard.SEQ_LF_LED_ROWS))
        for query_id in perf_guard.SEQ_LF_LED_ROWS:
            self.assertIn("--seq", perf_guard.QUERY_FLAGS[query_id], query_id)

    def test_the_fixture_is_the_stream_behind_one_lf(self):
        with tempfile.TemporaryDirectory() as tmp:
            source = os.path.join(tmp, "s.json")
            out = os.path.join(tmp, "o.json")
            with open(source, "wb") as f:
                f.write(b'\x1e{"id":1}\n\x1e{"id":')
            perf_guard.write_lf_led_fixture(source, out)
            with open(out, "rb") as f:
                self.assertEqual(f.read(), b'\n\x1e{"id":1}\n\x1e{"id":')

    def test_measure_all_hands_the_seq_rows_the_lf_led_fixture(self):
        seen = {}

        def fake_generate(binary, pattern, size, seed, out_path):
            with open(out_path, "wb") as f:
                f.write(b'\x1e{"id":1}\n' if pattern == "json-seq-bad-last" else b"{}")

        def fake_derive(binary, source_path, out_path):
            with open(out_path, "w") as f:
                f.write('{"users":[{"age":1,"score":1}]}')

        def fake_measure(valgrind_bin, binary, mode, flags, filter_expr, fixture_paths, reps):
            if fixture_paths:
                with open(fixture_paths[0], "rb") as f:
                    seen[(filter_expr, tuple(flags))] = f.read(2)
            return 1

        with contextlib.ExitStack() as stack:
            for name, fn in {
                "generate_fixture": fake_generate,
                "generate_compact_fixture": fake_derive,
                "generate_lines_fixture": fake_derive,
                "measure_query": fake_measure,
            }.items():
                stack.enter_context(mock.patch.object(perf_guard, name, fn))
            stack.enter_context(contextlib.redirect_stdout(io.StringIO()))
            perf_guard.measure_all("bin", "valgrind", 1)
        queries = {q[0]: q for q in perf_guard.QUERIES}
        for query_id in perf_guard.SEQ_LF_LED_ROWS:
            key = (queries[query_id][4], tuple(perf_guard.QUERY_FLAGS[query_id]))
            self.assertEqual(seen[key], b"\n\x1e", query_id)


class CensusRowTests(unittest.TestCase):
    """#4168: `wide_length` and `mid512_map_length_add` exist to see `census`
    (#3343/#4167), which no other row isolates. The mid-width row's fixture is
    synthesized here rather than generated by the binary, so what it contains
    is this file's to pin: a fixture of the wrong shape would still report an
    instruction count."""

    @classmethod
    def setUpClass(cls):
        cls.tmp = tempfile.TemporaryDirectory()
        cls.addClassCleanup(cls.tmp.cleanup)
        cls.shared = cls.fixture("shared.json")

    @classmethod
    def fixture(cls, name):
        path = os.path.join(cls.tmp.name, name)
        perf_guard.generate_mid_width_fixture(path)
        return path

    def test_both_rows_run_the_filters_the_issue_measured(self):
        queries = {q[0]: q for q in perf_guard.QUERIES}
        self.assertEqual(queries["wide_length"][1:], ("wide", "2mb", "jq", "length"))
        row = queries[perf_guard.MID_WIDTH_ROW]
        self.assertEqual(row[1], perf_guard.MID_WIDTH_PATTERN)
        self.assertEqual(row[3:], ("jq", "map(length) | add"))

    def test_the_synthetic_pattern_is_only_used_by_its_own_row(self):
        # `measure_all` skips `json generate` for the sentinel pattern, so any
        # other row carrying it would be measured against the wrong fixture.
        users = [q[0] for q in perf_guard.QUERIES if q[1] == perf_guard.MID_WIDTH_PATTERN]
        self.assertEqual(users, [perf_guard.MID_WIDTH_ROW])

    def test_fixture_is_equal_objects_of_512_distinct_keys(self):
        with open(self.shared) as f:
            objects = json.load(f)
        self.assertEqual(len(objects), 663)
        keys = list(objects[0])
        self.assertEqual(len(keys), 512)
        self.assertEqual(len(set(keys)), 512)
        for obj in objects:
            self.assertEqual(list(obj), keys)

    def test_fixture_is_about_4_mb(self):
        size = os.path.getsize(self.shared)
        self.assertTrue(3_900_000 <= size <= 4_200_000, size)

    def test_measure_all_gives_the_row_the_synthesized_fixture(self):
        # The wiring, not the fixture: `measure_all` must not ask the binary
        # to `json generate` the sentinel pattern, and must hand the row the
        # file this script wrote (a `dict.get` default evaluated eagerly
        # raised `KeyError` here before the lookup was made lazy).
        generated = []
        seen = {}

        def fake_generate(binary, pattern, size, seed, out_path):
            generated.append(pattern)
            with open(out_path, "w") as f:
                f.write("{}")

        def fake_derive(binary, source_path, out_path):
            with open(out_path, "w") as f:
                f.write('{"users":[{"age":1,"score":1}]}')

        def fake_measure(valgrind_bin, binary, mode, flags, filter_expr, fixture_paths, reps):
            # The first row with the filter: #7798's split rows run `length` too, after `wide_length`.
            if fixture_paths:
                seen.setdefault(filter_expr, (fixture_paths[0], os.path.getsize(fixture_paths[0])))
            return 1

        with contextlib.ExitStack() as stack:
            for name, fn in {
                "generate_fixture": fake_generate,
                "generate_compact_fixture": fake_derive,
                "generate_lines_fixture": fake_derive,
                "measure_query": fake_measure,
            }.items():
                stack.enter_context(mock.patch.object(perf_guard, name, fn))
            stack.enter_context(contextlib.redirect_stdout(io.StringIO()))
            measured = perf_guard.measure_all("bin", "valgrind", 1)
        self.assertNotIn(perf_guard.MID_WIDTH_PATTERN, generated)
        self.assertIn(perf_guard.MID_WIDTH_ROW, measured)
        path, size = seen["map(length) | add"]
        self.assertTrue(path.endswith("mid512_4mb.json"), path)
        self.assertGreater(size, 3_900_000)
        # `wide_length` still reads the generated `wide` fixture.
        self.assertTrue(seen["length"][0].endswith("wide_2mb.json"), seen["length"][0])

    def test_fixture_is_deterministic(self):
        with open(self.shared, "rb") as a, open(self.fixture("again.json"), "rb") as b:
            self.assertEqual(a.read(), b.read())


class NewPatternRowTests(unittest.TestCase):
    """#6619: a PR that adds a generator pattern and a guard row over it is compared with its
    merge-base build, which cannot generate that fixture. The row is new in the PR: reported, not
    compared, and nothing else about the comparison changes."""

    CLAP = "error: invalid value 'json-seq-bad-last' for '--pattern <PATTERN>'\n"

    def test_only_the_unknown_pattern_error_counts_as_unknown(self):
        self.assertTrue(perf_guard.pattern_is_unknown(self.CLAP, "json-seq-bad-last"))
        self.assertFalse(perf_guard.pattern_is_unknown(self.CLAP, "users"))
        self.assertFalse(perf_guard.pattern_is_unknown("error: disk full\n", "json-seq-bad-last"))
        self.assertFalse(perf_guard.pattern_is_unknown("", "json-seq-bad-last"))

    def _generate(self, returncode, stderr):
        result = mock.Mock(returncode=returncode, stderr=stderr)
        with mock.patch.object(perf_guard.subprocess, "run", return_value=result):
            return perf_guard.generate_fixture_if_known(
                "bin", "json-seq-bad-last", "2mb", 1, "out.json"
            )

    def test_a_baseline_that_does_not_know_the_pattern_is_skipped(self):
        self.assertFalse(self._generate(2, self.CLAP))

    def test_a_baseline_that_generates_it_is_used(self):
        self.assertTrue(self._generate(0, ""))

    def test_any_other_generation_failure_is_still_fatal(self):
        with self.assertRaises(SystemExit):
            self._generate(1, "error: disk full\n")

    def test_the_binary_under_test_must_know_every_pattern(self):
        result = mock.Mock(returncode=2, stderr=self.CLAP)
        with mock.patch.object(perf_guard.subprocess, "run", return_value=result):
            with self.assertRaises(SystemExit):
                perf_guard.generate_fixture("bin", "json-seq-bad-last", "2mb", 1, "out.json")

    def test_measure_all_leaves_out_the_row_a_baseline_cannot_generate(self):
        measured_filters = []

        def fake_known(binary, pattern, size, seed, out_path):
            if pattern == "json-seq-bad-last":
                return False
            with open(out_path, "w") as f:
                f.write("{}")
            return True

        def fake_derive(binary, source_path, out_path):
            with open(out_path, "w") as f:
                f.write('{"users":[{"age":1,"score":1}]}')

        def fake_measure(valgrind_bin, binary, mode, flags, filter_expr, fixture_paths, reps):
            measured_filters.append(filter_expr)
            return 1

        with contextlib.ExitStack() as stack:
            for name, fn in {
                "generate_fixture_if_known": fake_known,
                "generate_compact_fixture": fake_derive,
                "generate_lines_fixture": fake_derive,
                "measure_query": fake_measure,
            }.items():
                stack.enter_context(mock.patch.object(perf_guard, name, fn))
            stack.enter_context(contextlib.redirect_stdout(io.StringIO()))
            measured = perf_guard.measure_all("bin", "valgrind", 1, baseline_run=True)
        # Every row over the pattern is left out (#6620 added two beside `seq_name_bad_last`).
        seq_rows = {q[0] for q in perf_guard.QUERIES if q[1] == "json-seq-bad-last"}
        self.assertIn("seq_name_bad_last", seq_rows)
        self.assertGreaterEqual(len(seq_rows), 3)
        for name in seq_rows:
            self.assertNotIn(name, measured)
        self.assertIn("wide_length", measured)
        self.assertEqual(len(measured), len(perf_guard.QUERIES) - len(seq_rows))

    def _drift(self, baseline, measured):
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            failed = perf_guard.report_drift(baseline, measured, 5.0)
        return failed, out.getvalue()

    def test_a_new_row_is_reported_and_not_compared(self):
        measured = {q[0]: 100.0 for q in perf_guard.QUERIES}
        baseline = {k: v for k, v in measured.items() if k != "seq_name_bad_last"}
        failed, text = self._drift(baseline, measured)
        self.assertEqual(failed, [])
        line = next(l for l in text.splitlines() if l.startswith("seq_name_bad_last"))
        self.assertIn("(new row)", line)
        self.assertIn("not compared", line)

    def test_every_other_row_is_still_compared_beside_a_new_one(self):
        measured = {q[0]: 100.0 for q in perf_guard.QUERIES}
        baseline = {k: v for k, v in measured.items() if k != "seq_name_bad_last"}
        baseline["wide_identity"] = 80.0  # +25% now
        failed, _ = self._drift(baseline, measured)
        self.assertEqual([f[0] for f in failed], ["wide_identity"])

    def test_the_new_rows_own_threshold_applies_once_it_has_a_baseline(self):
        # Whatever the row's threshold is now (#6619 set 3.0, #6620 raises it while the change it
        # speeds up is not in the merge-base), drift over it fails and drift under it passes.
        threshold = perf_guard.QUERY_THRESHOLDS["seq_name_bad_last"]
        self.assertGreater(threshold, 0)
        for drift, fails in ((threshold + 0.5, True), (threshold - 0.5, False)):
            measured = {q[0]: 100.0 for q in perf_guard.QUERIES}
            baseline = dict(measured)
            baseline["seq_name_bad_last"] = 100.0 / (1 + drift / 100)
            failed, _ = self._drift(baseline, measured)
            self.assertEqual([f[0] for f in failed], ["seq_name_bad_last"] if fails else [])

    def test_the_new_rows_threshold_is_tighter_than_the_default(self):
        # #6619's point: a revert of #6594 reads +4.5%, which the default 5.0 lets through.
        # #6620, #7263 and #7260's field-list object map raised the row while the changes they
        # speed it up with were not yet in the merge-base; they are now, and the row is back to 3.0.
        self.assertLess(perf_guard.QUERY_THRESHOLDS["seq_name_bad_last"], 4.5)


class SyntheticFixtureRowTests(unittest.TestCase):
    """#7798: the per-record, multi-file, escape-heavy and fold rows read fixtures this script
    writes. A fixture of the wrong shape (one file where the row names a split, a cut on a line
    boundary, too few backslashes, a file under the lazy route's prefix) would still report an
    instruction count, for a shape the row does not name."""

    @classmethod
    def setUpClass(cls):
        cls.tmp = tempfile.TemporaryDirectory()
        cls.addClassCleanup(cls.tmp.cleanup)
        cls.paths = perf_guard.write_synthetic_fixtures(cls.tmp.name)

    def read(self, pattern):
        return [pathlib.Path(p).read_bytes() for p in self.paths[pattern]]

    def rows(self):
        return [q for q in perf_guard.QUERIES if q[1] in perf_guard.SYNTHETIC_PATTERNS]

    def test_every_pattern_has_a_row_and_every_row_a_written_pattern(self):
        used = {q[1] for q in self.rows()}
        self.assertEqual(used, set(perf_guard.SYNTHETIC_PATTERNS))
        self.assertEqual(set(self.paths), set(perf_guard.SYNTHETIC_PATTERNS))

    def test_fixtures_are_deterministic(self):
        with tempfile.TemporaryDirectory() as other:
            again = perf_guard.write_synthetic_fixtures(other)
            for pattern, paths in self.paths.items():
                self.assertEqual([pathlib.Path(p).read_bytes() for p in again[pattern]],
                                 self.read(pattern), pattern)

    def test_the_records_are_past_the_lazy_prefix(self):
        # 256 KiB is `LAZY_PREFIX_BYTES` (src/bin/succinctly/jq_runner.rs): a shorter file is read
        # whole, where a 10 MB file of records is read a window at a time.
        (ndjson,) = self.read("records-ndjson")
        self.assertGreater(len(ndjson), 256 * 1024)
        lines = ndjson.decode().splitlines()
        self.assertEqual(len(lines), perf_guard.RECORDS_COUNT)
        for line in lines[:50]:
            self.assertEqual(list(json.loads(line)), ["id", "name", "age", "active", "score"])
        (doc,) = self.read("records-doc")
        self.assertEqual(len(json.loads(doc)["users"]), perf_guard.RECORDS_COUNT)

    def test_a_split_document_concatenates_back_and_is_cut_inside_a_value(self):
        whole = b"".join(self.read("records-array-2"))
        array = json.loads(whole)
        self.assertEqual(len(array), perf_guard.RECORDS_COUNT)
        for pattern, parts in (("records-array-2", 2), ("records-array-16", 16)):
            texts = self.read(pattern)
            self.assertEqual(len(texts), parts, pattern)
            self.assertEqual(b"".join(texts), whole, pattern)
            for left, right in zip(texts, texts[1:]):
                # Each boundary falls inside a record, never between two, so every file but the
                # last ends inside a value and the run carries an open tail across each boundary.
                self.assertFalse(left.endswith((b"\n", b" ", b",")), pattern)
                self.assertFalse(right.startswith(b"\n"), pattern)
                self.assertRaises(json.JSONDecodeError, json.loads, left)
            # In order: the paths sort as the parts run.
            self.assertEqual(self.paths[pattern], sorted(self.paths[pattern]))

    def test_the_escapes_fixture_is_dense_in_backslashes(self):
        (text,) = self.read("escapes-ndjson")
        self.assertGreaterEqual(len(text), perf_guard.ESCAPES_BYTES)
        self.assertGreater(text.count(b"\\") * 16, len(text))
        for line in text.decode().splitlines():
            json.loads(line)

    def test_a_row_without_a_file_passes_dash_n(self):
        for query_id, pattern, _, _, _ in self.rows():
            if pattern == "none":
                self.assertIn("-n", perf_guard.QUERY_FLAGS[query_id], query_id)
        self.assertEqual(self.paths["none"], [])

    def test_the_e_row_differs_from_its_twin_only_by_dash_e(self):
        queries = {q[0]: q for q in perf_guard.QUERIES}
        self.assertEqual(queries["nd_e_iterate"][1:], queries["nd_iterate"][1:])
        self.assertEqual(perf_guard.QUERY_FLAGS["nd_e_iterate"],
                         ["-e"] + perf_guard.QUERY_FLAGS["nd_iterate"])

    def fake_cachegrind(self, fixture_paths):
        seen = {}

        def fake_run(cmd, **kwargs):
            seen["cmd"], seen["stdin"] = cmd, kwargs.get("stdin")
            log = next(a for a in cmd if a.startswith("--log-file=")).split("=", 1)[1]
            pathlib.Path(log).write_text("==1== I   refs:      1,234\n")
            return mock.Mock(returncode=0, stdout="1\n", stderr="")

        with mock.patch.object(perf_guard.subprocess, "run", fake_run):
            ir = perf_guard.run_cachegrind_once("valgrind", "bin", "jq", ["-n"], ".", fixture_paths)
        self.assertEqual(ir, 1234)
        return seen

    def test_a_row_without_a_file_reads_an_empty_stdin(self):
        seen = self.fake_cachegrind([])
        self.assertEqual(seen["cmd"][-3:], ["jq", "-n", "."])
        self.assertIs(seen["stdin"], perf_guard.subprocess.DEVNULL)

    def test_a_row_with_files_passes_them_in_order_and_keeps_its_stdin(self):
        seen = self.fake_cachegrind(["a.json", "b.json"])
        self.assertEqual(seen["cmd"][-2:], ["a.json", "b.json"])
        self.assertIsNone(seen["stdin"])

    def test_the_synthetic_rows_take_one_rep_and_the_rest_the_reps_asked_for(self):
        reps = {}

        def fake_measure(valgrind_bin, binary, mode, flags, filter_expr, fixture_paths, n):
            reps.setdefault(n, 0)
            reps[n] += 1
            return 1

        with contextlib.ExitStack() as stack:
            for name, fn in {
                "generate_fixture": lambda *a: pathlib.Path(a[-1]).write_text("{}"),
                "generate_compact_fixture": lambda *a: pathlib.Path(a[-1]).write_text(
                    '{"users":[{"age":1,"score":1}]}'),
                "generate_lines_fixture": lambda *a: pathlib.Path(a[-1]).write_text("{}"),
                "measure_query": fake_measure,
            }.items():
                stack.enter_context(mock.patch.object(perf_guard, name, fn))
            stack.enter_context(contextlib.redirect_stdout(io.StringIO()))
            perf_guard.measure_all("bin", "valgrind", 3)
        self.assertEqual(reps, {3: len(perf_guard.QUERIES) - len(self.rows()),
                                perf_guard.SYNTHETIC_REPS: len(self.rows())})

    def test_the_tight_thresholds_are_below_the_costs_they_were_added_for(self):
        # #7798's measured costs (cachegrind, AMD Ryzen 9 7950X, each branch against its base): the
        # smallest one each row was added to catch. A threshold at or above it lets that cost back.
        caught = {
            "nd_pipe30": 2.29,          # #7719, first branch
            "nd_two_defs": 2.74,        # #7719, first branch (#7787: +4.62)
            "nd_select_and_or": 2.92,   # #7719, first branch (#7787: +5.08)
            "nd_obj_interp_def": 2.20,  # #7787
            "doc_select_and_or": 4.28,  # #7787
            "doc_and_def": 3.10,        # #7787
        }
        for query_id, cost in caught.items():
            threshold = perf_guard.QUERY_THRESHOLDS.get(query_id, perf_guard.DEFAULT_THRESHOLD)
            self.assertLess(threshold, cost, query_id)

    def test_measure_all_hands_each_row_its_files(self):
        calls = []

        def fake_measure(valgrind_bin, binary, mode, flags, filter_expr, fixture_paths, reps):
            calls.append(list(fixture_paths))
            return 1

        def fake_generate(binary, pattern, size, seed, out_path):
            pathlib.Path(out_path).write_text("{}")

        def fake_derive(binary, source_path, out_path):
            pathlib.Path(out_path).write_text('{"users":[{"age":1,"score":1}]}')

        with contextlib.ExitStack() as stack:
            for name, fn in {
                "generate_fixture": fake_generate,
                "generate_compact_fixture": fake_derive,
                "generate_lines_fixture": fake_derive,
                "measure_query": fake_measure,
            }.items():
                stack.enter_context(mock.patch.object(perf_guard, name, fn))
            stack.enter_context(contextlib.redirect_stdout(io.StringIO()))
            perf_guard.measure_all("bin", "valgrind", 1)
        self.assertEqual(len(calls), len(perf_guard.QUERIES))
        for (query_id, pattern, _, _, _), paths in zip(perf_guard.QUERIES, calls):
            if pattern in perf_guard.SYNTHETIC_PATTERNS:
                names = [os.path.basename(p) for p in paths]
                self.assertEqual(names, [os.path.basename(p) for p in self.paths[pattern]], query_id)
            else:
                self.assertEqual(len(paths), 1, query_id)


class MeasuredPathTests(unittest.TestCase):
    """#6153: valgrind starts every measured binary from a path of one length."""

    def run_measure_all(self, binary):
        started = []

        def fake_measure(valgrind_bin, run_binary, mode, flags, filter_expr, fixture_paths, reps):
            started.append(run_binary)
            return 1

        def fake_generate(binary, pattern, size, seed, out_path):
            with open(out_path, "w") as f:
                f.write("{}")

        def fake_derive(binary, source_path, out_path):
            with open(out_path, "w") as f:
                f.write('{"users":[{"age":1,"score":1}]}')

        with contextlib.ExitStack() as stack:
            for name, fn in {
                "generate_fixture": fake_generate,
                "generate_compact_fixture": fake_derive,
                "generate_lines_fixture": fake_derive,
                "measure_query": fake_measure,
            }.items():
                stack.enter_context(mock.patch.object(perf_guard, name, fn))
            stack.enter_context(contextlib.redirect_stdout(io.StringIO()))
            perf_guard.measure_all(binary, "valgrind", 1)
        return started

    def test_binaries_spelled_with_paths_of_different_lengths_start_from_one_length(self):
        # The two spellings the CI job used: the current binary relative, a baseline binary
        # reused from main's push run absolute and longer.
        short = self.run_measure_all("target/release/succinctly")
        long = self.run_measure_all("/tmp/perf-guard-baseline-bin/succinctly")
        self.assertTrue(short and long)
        self.assertEqual({len(p) for p in short + long}, {len(short[0])})

    def test_the_started_path_stays_out_of_the_slow_lengths_wherever_tmpdir_points(self):
        # The length is the same on every machine and outside the window that costs
        # `arrays_map_iterate` 5.8%: `$TMPDIR` does not reach it.
        lengths = set()
        for tmpdir in (None, "/var/tmp", "/home/runner/work/_temp", "/a/very/long/tmpdir/" + "x" * 30):
            with mock.patch.object(tempfile, "tempdir", tmpdir):
                lengths.update(len(p) for p in self.run_measure_all("target/release/succinctly"))
        self.assertEqual(len(lengths), 1, lengths)
        (length,) = lengths
        self.assertNotIn(length, perf_guard.SLOW_PATH_LENGTHS)
        self.assertEqual(length, len("/tmp/tmpXXXXXXXX/bin/succinctly"))

    def test_the_started_path_is_a_symlink_to_the_binary_named_succinctly(self):
        seen = []

        def fake_measure(valgrind_bin, run_binary, *rest):
            seen.append((os.path.basename(run_binary), os.path.islink(run_binary),
                         os.readlink(run_binary)))
            return 1

        with contextlib.ExitStack() as stack:
            stack.enter_context(mock.patch.object(perf_guard, "measure_query", fake_measure))
            for name in ("generate_fixture", "generate_compact_fixture", "generate_lines_fixture"):
                stack.enter_context(mock.patch.object(
                    perf_guard, name,
                    lambda *a: pathlib.Path(a[-1]).write_text('{"users":[{"age":1,"score":1}]}')))
            stack.enter_context(contextlib.redirect_stdout(io.StringIO()))
            perf_guard.measure_all("some/relative/bin", "valgrind", 1)
        name, is_link, target = seen[0]
        self.assertEqual(name, "succinctly")
        self.assertTrue(is_link)
        self.assertEqual(target, os.path.abspath("some/relative/bin"))


if __name__ == "__main__":
    unittest.main()
