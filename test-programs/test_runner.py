# Part of the Austral project, under the Apache License v2.0 with LLVM Exceptions.
# See LICENSE file for details.
#
# SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
"""Tests for the runner's suite list.

The default run is `DEFAULT_SUITES`, and `collect_tests` refuses a `suites/`
directory the tuple does not name (a suite nobody compiles) and a tuple entry
with no directory (a stale name). Both refusals are only worth having if they
can fail, so each is held to a scratch pair here, and the repository's own pair
is checked to agree.

Run with `python3 test-programs/test_runner.py`.
"""

import os
import tempfile
import unittest

import runner


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
