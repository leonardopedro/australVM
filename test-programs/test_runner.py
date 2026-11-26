# Part of the Austral project, under the Apache License v2.0 with LLVM Exceptions.
# See LICENSE file for details.
#
# SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
"""Tests for the runner's suite list and its copy-source fixtures.

The default run is `DEFAULT_SUITES`, and `collect_tests` refuses a `suites/`
directory the tuple does not name (a suite nobody compiles) and a tuple entry
with no directory (a stale name). Both refusals are only worth having if they
can fail, so each is held to a scratch pair here, and the repository's own pair
is checked to agree.

A suite that copies an example (`source-copy.txt`) is checked the same way:
every `examples/…` module the origin declares must be in the copy. That is the
same comparison the e2e case runs, but here it needs no compiler, so a dropped
module is named in seconds instead of after a full `make test`.

Run with `python3 test-programs/test_runner.py`.
"""

import os
import tempfile
import unittest

import runner


def _write(directory: str, name: str, text: str) -> None:
    with open(os.path.join(directory, name), "w", encoding="utf-8") as stream:
        stream.write(text)


class SuiteListTest(unittest.TestCase):
    def test_the_repository_list_matches_the_tree(self) -> None:
        tests = runner.collect_tests()
        suites = {test.suite_name for test in tests}
        self.assertEqual(suites, set(runner.DEFAULT_SUITES))
        self.assertEqual(len(runner.DEFAULT_SUITES), len(suites))
        self.assertIn("027-handle-pragma", suites)

    def test_the_handle_pragma_suite_collects_both_refusals(self) -> None:
        tests = [
            test
            for test in runner.collect_tests()
            if test.suite_name == "027-handle-pragma"
        ]
        self.assertEqual(
            sorted(test.name for test in tests),
            ["000-dispose-signature", "001-not-foreign"],
        )
        # Both are compiler-error fixtures: `austral-stderr.txt` is what makes
        # the runner a TestFailure rather than a program that must compile.
        for test in tests:
            self.assertIsInstance(test, runner.TestFailure)

    def test_a_directory_without_a_list_entry_is_refused(self) -> None:
        with tempfile.TemporaryDirectory() as suites_dir:
            os.makedirs(os.path.join(suites_dir, "999-unwired"))
            with self.assertRaises(ValueError) as caught:
                runner.collect_tests(suites_dir=suites_dir, suites=())
        message = str(caught.exception)
        self.assertIn("999-unwired", message)
        self.assertIn("not wired into DEFAULT_SUITES", message)

    def test_a_list_entry_without_a_directory_is_refused(self) -> None:
        with tempfile.TemporaryDirectory() as suites_dir:
            with self.assertRaises(ValueError) as caught:
                runner.collect_tests(
                    suites_dir=suites_dir, suites=("999-ghost",)
                )
        message = str(caught.exception)
        self.assertIn("999-ghost", message)
        self.assertIn("no directory", message)

    def test_the_pool_is_bounded_by_the_selection_and_the_machine(self) -> None:
        # `ProcessPoolExecutor()` defaults to one worker per CPU, which is what
        # exhausted memory on a full run. The bound is a real cap, not a hint.
        self.assertEqual(runner.worker_count([]), 1)
        self.assertLessEqual(
            runner.worker_count(list(range(1000))), runner.MAX_WORKERS
        )
        self.assertLessEqual(
            runner.worker_count([object()] * 1000), os.cpu_count() or 1
        )
        self.assertEqual(
            runner.worker_count([object()] * 3), min(3, os.cpu_count() or 1)
        )

    def test_every_copy_source_fixture_still_holds_its_origin_modules(self) -> None:
        # The e2e case compiles each suite, so a copy that lost a module was
        # only found after `make test` and the rest of the run; the same
        # comparison here is file IO and fails with the dropped name. It uses
        # the runner's own case, so the fast check and the compiled one cannot
        # disagree about what a copy must hold.
        fixtures = [
            test
            for test in runner.collect_tests()
            if isinstance(test, runner.TestFixtureCopy)
        ]
        self.assertGreaterEqual(len(fixtures), 2, "no copy-source fixture to check")
        for test in fixtures:
            result = runner._run_fixture_copy_test(test)
            detail = ""
            if isinstance(result, runner.TestFail):
                detail = result.reason + "\n" + "\n".join(
                    f"{name}: {value}" for name, value in result.outputs
                )
            self.assertIsInstance(
                result,
                runner.TestPass,
                f"{test.suite_name}/{test.name} does not match `{test.source_dir}`:\n{detail}",
            )

    def test_a_copy_that_drops_a_module_is_refused(self) -> None:
        with tempfile.TemporaryDirectory() as scratch:
            origin = os.path.join(scratch, "examples", "origin")
            copy = os.path.join(scratch, "suite", "case")
            os.makedirs(origin)
            os.makedirs(copy)
            _write(origin, "Alpha.aui", "interface Alpha is\nend interface.\n")
            _write(origin, "Beta.aum", "module body Beta is\nend module body.\n")
            _write(copy, "Alpha.aui", "interface Alpha is\nend interface.\n")
            test = runner.TestFixtureCopy(
                name="case/copy",
                suite_name="999-scratch",
                directory=copy,
                source_dir=origin,
            )
            result = runner._run_fixture_copy_test(test)
            self.assertIsInstance(result, runner.TestFail)
            dropped = "\n".join(
                f"{label}: {value}" for label, value in result.outputs
            )
            self.assertIn("Beta.aum", dropped)
            self.assertIn("MISSING FROM COPY", dropped)

    def test_a_wired_but_empty_suite_is_not_a_disagreement(self) -> None:
        with tempfile.TemporaryDirectory() as suites_dir:
            os.makedirs(os.path.join(suites_dir, "998-empty"))
            self.assertEqual(
                runner.collect_tests(
                    suites_dir=suites_dir, suites=("998-empty",)
                ),
                [],
            )


if __name__ == "__main__":
    unittest.main()
