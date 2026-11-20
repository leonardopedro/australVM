#!/usr/bin/env python3
# Part of the Austral project, under the Apache License v2.0 with LLVM Exceptions.
# See LICENSE file for details.
#
# SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
"""
This script runs the end-to-end tests of the compiler.
"""
import os
import subprocess
import time
from concurrent.futures import ProcessPoolExecutor, Future

#
# Constants
#

# The `test-programs/` directory.
DIR: str = "test-programs/"

# `./austral` links the cranelift bridge as a shared library. Callers used to
# have to export `LD_LIBRARY_PATH` themselves; without it every fixture died at
# the loader with `libaustral_cranelift_bridge.so: cannot open shared object
# file`, which says nothing about the fixture. The runner now finds the bridge
# itself: both the tree root and the bridge's release directory, ahead of
# whatever the caller already had (a caller's entry stays usable, it just does
# not have to carry ours).
_ROOT: str = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
_BRIDGE_DIRS: list[str] = [
    _ROOT,
    os.path.join(_ROOT, "safestos", "cranelift", "target", "release"),
]
os.environ["LD_LIBRARY_PATH"] = os.pathsep.join(
    _BRIDGE_DIRS
    + ([os.environ["LD_LIBRARY_PATH"]] if os.environ.get("LD_LIBRARY_PATH") else [])
)


#
# Error Reporting
#


def report(properties, outputs):
    print("\n\n--- BEGIN ERROR ---")
    for name, value in properties:
        print(f"{name}: {value}")
    for name, value in outputs:
        print(f"--- BEGIN {name} ---")
        print(value)
        print(f"--- END {name} ---\n")
    print("--- END ERROR ---")
    exit(-1)


#
# Classes
#


class Test(object):
    """
    Base class of test objects.
    """

    name: str
    suite_name: str
    directory: str
    c_path: str
    bin_path: str

    def __init__(self, name: str, suite_name: str, directory: str, cli: str | None):
        self.name = name
        self.suite_name = suite_name
        self.directory = directory
        self.cli = cli
        ident: str = suite_name + "_" + name
        self.c_path = "/tmp/austral_e2e_" + ident + ".c"
        self.bin_path = "/tmp/austral_e2e_" + ident + ".bin"


class TestSuccess(Test):
    """
    A test that is expected to succeed.
    """

    expected_output: str | None

    def __init__(
        self,
        name: str,
        suite_name: str,
        directory: str,
        cli: str | None,
        expected_output: str | None,
    ):
        super().__init__(name, suite_name, directory, cli)
        self.expected_output = expected_output


class TestFailure(Test):
    """
    A test that is expected to fail when compiling the code.
    """

    expected_compiler_error: str

    def __init__(
        self,
        name: str,
        suite_name: str,
        directory: str,
        cli: str | None,
        expected_compiler_error: str,
    ):
        super().__init__(name, suite_name, directory, cli)
        self.expected_compiler_error = expected_compiler_error


class TestProgramFailure(Test):
    """
    A test that is expected to compile successfully, but the compiled program
    should return a failure exit code, and print to stderr.
    """

    expected_program_stderr: str

    def __init__(
        self,
        name: str,
        suite_name: str,
        directory: str,
        cli: str | None,
        expected_program_stderr: str,
    ):
        super().__init__(name, suite_name, directory, cli)
        self.expected_program_stderr = expected_program_stderr


class TestFixtureCopy(Test):
    """
    A case that checks a fixture copy still matches the fixture it came from.

    A suite that copies a module (rather than compiling the example in place)
    cannot see the example change: the copy keeps compiling the old program and
    the suite goes on passing. `source-copy.txt` names the origin directory
    relative to the repository root; this case compares the set of `*.aui` and
    `*.aum` names on both sides and each file's bytes. An origin with no module
    sources is a failure, so the comparison cannot pass vacuously.
    """

    source_dir: str

    def __init__(
        self, name: str, suite_name: str, directory: str, source_dir: str
    ):
        super().__init__(name=name, suite_name=suite_name, directory=directory, cli=None)
        self.source_dir = source_dir


class TestLoaderPath(Test):
    """
    A case with no source: it checks the runner's own loader setup.

    The suite's other cases all prove something about Austral code; this one
    proves that `./austral` starts because the runner put the bridge on the
    loader's path, not because whoever invoked the runner had exported
    `LD_LIBRARY_PATH`.
    """

    note: str

    def __init__(self, name: str, suite_name: str, directory: str, note: str):
        super().__init__(name, suite_name, directory, cli=None)
        self.note = note


class TestResult(object):
    """
    Base class of test results.
    """

    test: Test

    def __init__(self, test: Test):
        self.test = test


class TestPass(TestResult):
    """
    Represents a passing test.
    """

    def __init__(self, test: Test):
        super().__init__(test)


class TestFail(TestResult):
    """
    Represents a test that failed.
    """

    austral_cmd: list[str] | None
    cc_cmd: list[str] | None
    reason: str
    outputs: list[tuple[str, str]]

    def __init__(
        self,
        test: Test,
        austral_cmd: list[str] | None,
        cc_cmd: list[str] | None,
        reason: str,
        outputs: list[tuple[str, str]],
    ):
        super().__init__(test)
        self.austral_cmd = austral_cmd
        self.cc_cmd = cc_cmd
        self.reason = reason
        self.outputs = outputs


#
# Reporting
#


def report_test_results(results: list[TestResult]):
    fails: list[TestFail] = []

    for result in results:
        if isinstance(result, TestPass):
            report_pass(result)
        elif isinstance(result, TestFail):
            report_fail(result)
            fails.append(result)
        else:
            raise ValueError("Unknown test result type: ", result)

    if fails:
        with open("FAILURES.txt", "w") as stream:
            for fail in fails:
                report_fail_details(fail, stream)

        print("\n\nDetailed failures written to FAILURES.txt")


def report_pass(result: TestPass):
    suite: str = result.test.suite_name
    name: str = result.test.name
    print(f"{suite.ljust(45)}  {name.ljust(45)}  PASS")


def report_fail(result: TestFail):
    suite: str = result.test.suite_name
    name: str = result.test.name
    print(f"{suite.ljust(45)}  {name.ljust(45)}  FAIL")


def report_fail_details(result: TestFail, stream):
    print("Suite:", result.test.suite_name, file=stream)
    print("Test:", result.test.name, file=stream)
    print("C File:", result.test.c_path, file=stream)
    print("Bin File:", result.test.bin_path, file=stream)
    print("Austral command:", try_join(result.austral_cmd), file=stream)
    print("GCC command:", try_join(result.cc_cmd), file=stream)
    print("Reason:", result.reason, file=stream)
    for key, value in result.outputs:
        print(key, file=stream)
        for line in value.split("\n"):
            print(f"\t{line}", file=stream)
        print(file=stream)
    print("\n\n\n\n\n", file=stream)


def try_join(l: list[str] | None) -> str:
    if l is None:
        return ""
    else:
        return " ".join(l)


#
# Collection
#


def collect_tests() -> list[Test]:
    """
    Get the list of all test suites.
    """
    # Result aggregator.
    tests: list[Test] = []
    # Find the `suites/`` directory.
    suites_dir: str = os.path.join(DIR, "suites")
    # Find the test suites.
    suite_names: list = sorted(os.listdir(suites_dir))
    # Iterate over each test suite.
    for suite_name in suite_names:
        # Find the tests in this suite.
        suite_dir: str = os.path.join(suites_dir, suite_name)
        test_names: list = sorted(
            [name for name in os.listdir(suite_dir) if name != "README.md"]
        )
        # Iterate over each test.
        for test_name in test_names:
            test_dir: str = os.path.join(suite_dir, test_name)
            expected_error: str | None = _get_file_contents(
                test_dir, "austral-stderr.txt"
            )
            expected_output: str | None = _get_file_contents(
                test_dir, "program-stdout.txt"
            )
            program_stderr: str | None = _get_file_contents(
                test_dir, "program-stderr.txt"
            )
            cli: str | None = _get_file_contents(test_dir, "cli.txt")
            loader_note: str | None = _get_file_contents(test_dir, "loader-path.txt")
            source_copy: str | None = _get_file_contents(test_dir, "source-copy.txt")
            if source_copy is not None:
                # An extra case beside the program: the runner checks the
                # copied sources still match their origin, then the metadata
                # below decides what the program itself is expected to do.
                tests.append(
                    TestFixtureCopy(
                        name=test_name + "/copy",
                        suite_name=suite_name,
                        directory=test_dir,
                        source_dir=source_copy,
                    )
                )
            if loader_note is not None:
                # Not a program: the runner checks its own environment.
                tests.append(
                    TestLoaderPath(
                        name=test_name,
                        suite_name=suite_name,
                        directory=test_dir,
                        note=loader_note,
                    )
                )
                continue
            if (expected_error is not None) and (expected_output is not None):
                raise ValueError(
                    "Can't have both `austral-stderr.txt` and `program-stdout.txt` in the same test."
                )
            elif (expected_error is not None) and (expected_output is None):
                # The test should fail to compile, and the compiler output must
                # match the contents of the file.
                tests.append(
                    TestFailure(
                        name=test_name,
                        suite_name=suite_name,
                        directory=test_dir,
                        cli=cli,
                        expected_compiler_error=expected_error,
                    )
                )
            elif (expected_error is None) and (expected_output is not None):
                # The program should compile, run successfully, produce stdout
                # that matches the contents of the file.
                tests.append(
                    TestSuccess(
                        name=test_name,
                        suite_name=suite_name,
                        directory=test_dir,
                        cli=cli,
                        expected_output=expected_output,
                    )
                )
            # There is neither an `austral-stderr.txt` nor an `program-stdout.txt `file.
            else:
                if program_stderr is None:
                    # The program should compile and run successfully and produce no
                    # stdout.
                    tests.append(
                        TestSuccess(
                            name=test_name,
                            suite_name=suite_name,
                            directory=test_dir,
                            cli=cli,
                            expected_output=None,
                        )
                    )
                else:
                    # The program should compile successfully, return a failure
                    # exit code, and print stderr.
                    tests.append(
                        TestProgramFailure(
                            name=test_name,
                            suite_name=suite_name,
                            directory=test_dir,
                            cli=cli,
                            expected_program_stderr=program_stderr,
                        )
                    )
    return tests


def _get_file_contents(test_dir: str, filename: str) -> str | None:
    if os.path.isfile(os.path.join(test_dir, filename)):
        with open(os.path.join(test_dir, filename), "r") as stream:
            data: str = stream.read().strip()
            if not data:
                raise ValueError(f"`{filename}` exists, but it is empty.")
            return data
    else:
        return None


#
# Test Execution
#


def run_test(test: Test, replace_stderr: bool) -> TestResult | None:
    try:
        result: TestResult = run_test_inner(test, replace_stderr)
        return result
    except Exception as e:
        print(e)
        return None


def run_test_inner(test: Test, replace_stderr: bool) -> TestResult:
    if isinstance(test, TestSuccess):
        return _run_success_test(test)
    elif isinstance(test, TestFailure):
        return _run_failure_test(test, replace_stderr)
    elif isinstance(test, TestProgramFailure):
        return _run_program_failure_test(test)
    elif isinstance(test, TestFixtureCopy):
        return _run_fixture_copy_test(test)
    elif isinstance(test, TestLoaderPath):
        return _run_loader_path_test(test)
    else:
        raise ValueError("Unknown test type.")


def _test_cmd(test: Test) -> list[str]:
    if test.cli:
        cli: str = test.cli
        cli: str = cli.replace("$DIR", test.directory)
        cli: str = cli.replace("$C_PATH", test.c_path)
        return cli.split(" ")
    else:
        body_path: str = os.path.join(test.directory, "Test.aum")
        return [
            "./austral",
            "compile",
            f"{body_path}",
            "--entrypoint=Test:main",
            "--target-type=c",
            f"--output={test.c_path}",
            "--error-format=json",
        ]


def _run_success_test(test: TestSuccess) -> TestResult:
    # Find the source files.
    expected_output = test.expected_output
    suite_name: str = test.suite_name
    test_name: str = test.name
    # Construct the compiler command.
    compile_cmd: list[str] = _test_cmd(test)
    # Call the compiler.
    result: subprocess.CompletedProcess = subprocess.run(
        compile_cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE
    )
    code: int = result.returncode
    if code != 0:
        # Compilation failed.
        return TestFail(
            test=test,
            austral_cmd=compile_cmd,
            cc_cmd=None,
            reason="Austral compilation failed, but was expected to succeed.",
            outputs=[
                ("AUSTRAL STDOUT", result.stdout.decode("utf-8")),
                ("AUSTRAL STDERR", result.stderr.decode("utf-8")),
            ],
        )
    # The compiler executed successfully. Compile the program with GCC.
    gcc_cmd: list = [
        "gcc",
        "-fwrapv",  # Modular arithmetic semantics
        "-Wno-builtin-declaration-mismatch",
        test.c_path,
        "-lm",  # Math stdlib,
        "-o",
        test.bin_path,
    ]
    # Call GCC.
    result: subprocess.CompletedProcess = subprocess.run(
        gcc_cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE
    )
    code: int = result.returncode
    if code != 0:
        # GCC compilation failed.
        return TestFail(
            test=test,
            austral_cmd=compile_cmd,
            cc_cmd=gcc_cmd,
            reason="GCC compilation failed, but was expected to succeed.",
            outputs=[
                ("GCC STDOUT", result.stdout.decode("utf-8")),
                ("GCC STDERR", result.stderr.decode("utf-8")),
            ],
        )
    # GCC compilation succeeded. Run the program.
    result: subprocess.CompletedProcess = subprocess.run(
        [test.bin_path],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    code: int = result.returncode
    if code != 0:
        # Program did not terminate successfully.
        return TestFail(
            test=test,
            austral_cmd=compile_cmd,
            cc_cmd=gcc_cmd,
            reason="Program terminated abnormally.",
            outputs=[
                ("STDOUT", result.stdout.decode("utf-8")),
                ("STDERR", result.stderr.decode("utf-8")),
            ],
        )
    # Program ran successfully. Did it produce output?
    stdout: str = result.stdout.decode("utf-8").strip()
    if stdout and expected_output:
        # Have output, expected output. Check they match.
        if stdout == expected_output:
            # Output matches. Pass.
            return TestPass(test=test)
        else:
            # Output mismatch. Error.
            return TestFail(
                test=test,
                austral_cmd=compile_cmd,
                cc_cmd=gcc_cmd,
                reason="Program produced the wrong stdout.",
                outputs=[
                    ("ACTUAL STDOUT", stdout),
                    ("EXPECTED STDOUT", expected_output),
                ],
            )
    elif stdout and (not expected_output):
        # Have output, did not expect it. Error.
        return TestFail(
            test=test,
            austral_cmd=compile_cmd,
            cc_cmd=gcc_cmd,
            reason="Program produced stdout, which we did not expect.",
            outputs=[
                ("STDOUT", stdout),
            ],
        )
    elif (not stdout) and expected_output:
        # Don't have output, expected it. Error.
        return TestFail(
            test=test,
            austral_cmd=compile_cmd,
            cc_cmd=gcc_cmd,
            reason="Program did not produce stdout, but we expected it.",
            outputs=[
                ("EXPECTED OUTPUT", expected_output),
            ],
        )
    else:
        # Don't have output, didn't expect it. Pass.
        return TestPass(test=test)

    return TestPass(test=test)


def trim_lines(text):
    return "\n".join(
        [line if line.strip() else "" for line in text.strip().split("\n")]
    )


def _run_failure_test(test: TestFailure, replace_stderr: bool) -> TestResult:
    suite_name: str = test.suite_name
    test_name: str = test.name
    # Construct the compiler command.
    compile_cmd: list[str] = _test_cmd(test)
    # Call the compiler.
    result: subprocess.CompletedProcess = subprocess.run(
        compile_cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE
    )
    code: int = result.returncode
    if code == 0:
        # Compilation succeeded.
        return TestFail(
            test=test,
            austral_cmd=compile_cmd,
            cc_cmd=None,
            reason="Austral compiler succeeded, but was expected to fail.",
            outputs=[
                ("EXPECTED STDERR", test.expected_compiler_error),
            ],
        )
    # Compilation failed. Does the actual stderr match expected?
    if replace_stderr:
        # If replace_stderr is true, just take the actual stderr and overwrite the expected stderr.
        stderr: str = trim_lines(result.stderr.decode("utf-8"))
        with open(test.directory + "/austral-stderr.txt", "w") as stream:
            stream.write(stderr)
    else:
        stderr: str = trim_lines(result.stderr.decode("utf-8"))
        if stderr != trim_lines(test.expected_compiler_error):
            return TestFail(
                test=test,
                austral_cmd=compile_cmd,
                cc_cmd=None,
                reason="Austral compiler failed, but compiler output does not match what we expected.",
                outputs=[
                    ("EXPECTED STDERR", test.expected_compiler_error),
                    ("ACTUAL STDERR", stderr),
                ],
            )
    # Compilation failed and output matches.
    return TestPass(test=test)


def _run_program_failure_test(test: TestProgramFailure) -> TestResult:
    # Find the source files.
    expected_stderr: str = test.expected_program_stderr
    suite_name: str = test.suite_name
    test_name: str = test.name
    # Construct the compiler command.
    compile_cmd: list[str] = _test_cmd(test)
    # Call the compiler.
    result: subprocess.CompletedProcess = subprocess.run(
        compile_cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE
    )
    code: int = result.returncode
    if code != 0:
        # Compilation failed: print the output.
        return TestFail(
            test=test,
            austral_cmd=compile_cmd,
            cc_cmd=None,
            reason="Austral compilation failed, but was expected to succeed.",
            outputs=[
                ("AUSTRAL STDOUT", result.stdout.decode("utf-8")),
                ("AUSTRAL STDERR", result.stderr.decode("utf-8")),
            ],
        )
    # The compiler executed successfully. Compile the program with GCC.
    gcc_cmd: list[str] = [
        "gcc",
        "-fwrapv",  # Modular arithmetic semantics
        "-Wno-builtin-declaration-mismatch",
        test.c_path,
        "-lm",  # Math stdlib,
        "-o",
        test.bin_path,
    ]
    # Call GCC.
    result: subprocess.CompletedProcess = subprocess.run(
        gcc_cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE
    )
    code: int = result.returncode
    if code != 0:
        # GCC compilation failed.
        return TestFail(
            test=test,
            austral_cmd=compile_cmd,
            cc_cmd=gcc_cmd,
            reason="GCC compilation failed, but was expected to succeed.",
            outputs=[
                ("GCC STDOUT", result.stdout.decode("utf-8")),
                ("GCC STDERR", result.stderr.decode("utf-8")),
            ],
        )
    # GCC compilation succeeded. Run the program.
    result: subprocess.CompletedProcess = subprocess.run(
        [test.bin_path],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    code: int = result.returncode
    if code == 0:
        # Program did terminated successfully, but we didn't want that.
        return TestFail(
            test=test,
            austral_cmd=compile_cmd,
            cc_cmd=gcc_cmd,
            reason="Program terminated successfully, but we expected a failure.",
            outputs=[
                ("STDOUT", result.stdout.decode("utf-8")),
                ("STDERR", result.stderr.decode("utf-8")),
            ],
        )
    # Did it produce output?
    stderr: str = result.stderr.decode("utf-8").strip()
    if stderr:
        # Have stderr, expected stderr. Check they match.
        if stderr == expected_stderr:
            # Output matches. Pass.
            return TestPass(test=test)
        else:
            # Output mismatch. Error.
            return TestFail(
                test=test,
                austral_cmd=compile_cmd,
                cc_cmd=gcc_cmd,
                reason="Program produced stderr, but it was not what we expected",
                outputs=[
                    ("ACTUAL STDERR", stderr),
                    ("EXPECTED STDERR", expected_stderr),
                ],
            )
    else:
        # Don't have output, expected it. Error.
        return TestFail(
            test=test,
            austral_cmd=compile_cmd,
            cc_cmd=gcc_cmd,
            reason="Program did not produce stderr, but we expected stderr",
            outputs=[
                ("EXPECTED STDERR", expected_stderr),
            ],
        )

    return TestPass(test=test)


BRIDGE_LIBRARY = "libaustral_cranelift_bridge.so"


def _run_fixture_copy_test(test: TestFixtureCopy) -> TestResult:
    """
    Compares the copy with its declared origin, both ways.

    Both directions matter: a missing copy means the suite is compiling
    something else (or nothing), and an extra module beside it means the case
    runs code the origin never had.
    """
    origin = os.path.join(_ROOT, test.source_dir)
    compile_cmd: list[str] = [
        "diff",
        "-r",
        origin,
        test.directory,
    ]

    def module_sources(directory: str) -> list[str]:
        return sorted(
            name
            for name in os.listdir(directory)
            if name.endswith(".aui") or name.endswith(".aum")
        )

    if not os.path.isdir(origin):
        return TestFail(
            test=test,
            austral_cmd=compile_cmd,
            cc_cmd=None,
            reason=f"the declared copy origin `{test.source_dir}` is not a directory",
            outputs=[("ORIGIN", origin), ("COPY", test.directory)],
        )
    wanted: list[str] = module_sources(origin)
    if not wanted:
        return TestFail(
            test=test,
            austral_cmd=compile_cmd,
            cc_cmd=None,
            reason=f"the declared copy origin `{test.source_dir}` holds no "
            "`.aui`/`.aum` module, so this case could not detect drift",
            outputs=[("ORIGIN", origin)],
        )
    present: list[str] = module_sources(test.directory)
    missing: list[str] = [name for name in wanted if name not in present]
    extra: list[str] = [name for name in present if name not in wanted]
    differing: list[str] = []
    for name in wanted:
        if name not in present:
            continue
        with open(os.path.join(origin, name), "rb") as stream:
            original: bytes = stream.read()
        with open(os.path.join(test.directory, name), "rb") as stream:
            copied: bytes = stream.read()
        if original != copied:
            differing.append(name)
    if missing or extra or differing:
        outputs: list[tuple[str, str]] = [
            ("ORIGIN", origin),
            ("COPY", test.directory),
        ]
        if missing:
            outputs.append(("MISSING FROM COPY", "\n".join(missing)))
        if extra:
            outputs.append(("NOT IN ORIGIN", "\n".join(extra)))
        if differing:
            outputs.append(("DIFFERENT BYTES", "\n".join(differing)))
        return TestFail(
            test=test,
            austral_cmd=compile_cmd,
            cc_cmd=None,
            reason=f"the copy no longer matches `{test.source_dir}`; refresh the "
            "copied sources or compile the example in place",
            outputs=outputs,
        )
    return TestPass(test=test)


def _run_loader_path_test(test: TestLoaderPath) -> TestResult:
    """
    Two runs of the compiler, and the pair is the oracle:

    * in the runner's own environment `./austral` must start, and
    * with `LD_LIBRARY_PATH` removed it must fail at the *loader*, naming the
      bridge.

    The second run is what makes the first mean something. If the runner ever
    stops exporting the path, the first run reports this same loader error; if
    the loader ever stops needing it (an rpath appears, the library gets
    installed), the second run starts and this case says it can no longer
    distinguish the runner's contribution from the environment's.
    """
    compile_cmd: list[str] = ["./austral", "--version"]

    def run(env: dict[str, str]) -> subprocess.CompletedProcess:
        return subprocess.run(
            compile_cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=env
        )

    owned: subprocess.CompletedProcess = run(dict(os.environ))
    if owned.returncode != 0:
        return TestFail(
            test=test,
            austral_cmd=compile_cmd,
            cc_cmd=None,
            reason="the compiler does not start in the runner's own environment; "
            "the runner is expected to put the bridge on the loader's path",
            outputs=[
                ("AUSTRAL STDOUT", owned.stdout.decode("utf-8")),
                ("AUSTRAL STDERR", owned.stderr.decode("utf-8")),
                ("LD_LIBRARY_PATH", os.environ.get("LD_LIBRARY_PATH", "<unset>")),
            ],
        )

    stripped_env: dict[str, str] = dict(os.environ)
    stripped_env.pop("LD_LIBRARY_PATH", None)
    stripped: subprocess.CompletedProcess = run(stripped_env)
    stripped_stderr: str = stripped.stderr.decode("utf-8")
    if stripped.returncode == 0 or BRIDGE_LIBRARY not in stripped_stderr:
        return TestFail(
            test=test,
            austral_cmd=compile_cmd,
            cc_cmd=None,
            reason="the compiler starts without LD_LIBRARY_PATH and without "
            f"naming {BRIDGE_LIBRARY}, so this case cannot tell the runner's "
            "path from the environment's (the loader no longer needs it)",
            outputs=[
                ("AUSTRAL STDOUT", stripped.stdout.decode("utf-8")),
                ("AUSTRAL STDERR", stripped_stderr),
                ("EXPECTED IN STDERR", BRIDGE_LIBRARY),
            ],
        )

    return TestPass(test=test)


#
# Test Runner
#


def filter_tests(
    tests: list[Test],
    suite_pattern: str | None,
    name_pattern: str | None,
) -> list[Test]:
    filtered: list[Test] = []
    for test in tests:
        if suite_pattern is not None:
            if test.suite_name.find(suite_pattern) == -1:
                continue

        if name_pattern is not None:
            if test.name.find(name_pattern) == -1:
                continue

        filtered.append(test)

    return filtered


def run_all_tests(
    tests: list[Test],
    suite_pattern: str | None,
    name_pattern: str | None,
    replace_stderr: bool,
) -> list[TestResult]:
    """
    Run the given suites.

    If suite_pattern is given, only tests in suites containing the given string are considered.
    If name_pattern is given, only tests that have names containing the given string are run.
    An empty pattern means match all.
    """
    filtered: list[Test] = filter_tests(tests, suite_pattern, name_pattern)
    return run_tests_parallel(filtered, replace_stderr)


def run_tests_parallel(
    tests: list[Test],
    replace_stderr: bool,
) -> list[TestResult]:
    with ProcessPoolExecutor() as executor:
        futures: list[Future[TestResult | None]] = [
            executor.submit(run_test, test, replace_stderr)
            for test in tests
        ]
    results: list[TestResult] = [r for f in futures if (r := f.result()) is not None]
    assert len(results) == len(tests)

    return sorted(results, key=lambda r: r.test.suite_name + "_" + r.test.name)


#
# Entrypoint
#


def have_failures(l: list[TestResult]) -> bool:
    for result in results:
        if isinstance(result, TestFail):
            return True
    return False


if __name__ == "__main__":
    import sys

    replace_stderr: bool = "--replace-stderr" in sys.argv
    args: list[str] = [arg for arg in sys.argv if not arg.startswith("--")]
    suite_pattern: str | None = args[1] if len(args) > 1 else None
    name_pattern: str | None = args[2] if len(args) > 2 else None
    try:
        print("Collecting...")
        tests: list[Test] = collect_tests()
        print("Running...")
        results: list[TestResult] = run_all_tests(
            tests,
            suite_pattern,
            name_pattern,
            replace_stderr,
        )
        report_test_results(results)
        if have_failures(results):
            print("\nSome tests failed")
            exit(-1)
        else:
            print("\nAll tests passed")
            exit(0)
    except ValueError as e:
        print(e)
        exit(-1)
