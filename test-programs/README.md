# Test Programs

End-to-end tests of the compiler.

The file hierarchy is:

```
test-programs/
    suites/
        XXX-suite/      # Represents a test suite
            YYY-test/      # Represents a test
                Test.aui   # Module interface
                Test.aum   # Module body
                output.txt # Expected program output, if the program is meant to compile.
                error.txt  # Expected compiler output, if the program is meant to compile.
```

The `suites` directory has numbered subdirectories, each representing a test suite: a collection of tests covering specific compiler features.

Each test directory contains some Austral source files (in the simplest cases, `Test.aui` and `Test.aum`) and some control files:

1. `austral-stderr.txt` is the compiler's error message if the test is expected to fail.

2. `program-stdout.txt` is the compiled program's stdout if the compiler is expected to succeed and the program is expected to produce stdout and return normally.

3. If neither file is present, the test is expected to compile and run successfully but produce no output.

4. `source-copy.txt` names a fixture directory, relative to the repository root, whose `.aui`/`.aum` sources this test copies. The runner adds a `<test>/copy` case that fails when a copied module is missing, extra, or differs in bytes from the fixture, so the copy cannot drift from its example.

Run a single suite with `./test-programs/runner.py <suite>`; a second argument filters test names.

The no-argument run compiles the suites named in `runner.DEFAULT_SUITES`, in that order. Wiring a suite is therefore an explicit edit: `collect_tests` refuses a directory under `suites/` that the tuple does not name, and refuses a name that has no directory, so neither a forgotten suite nor a stale entry can pass quietly. `python3 test-programs/test_runner.py` holds that rule to scratch directories (no compiler needed).

Suite and test directories are numbered so that tests run in a predictable order.

The `runner.py` script runs the tests.
