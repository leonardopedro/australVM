#!/usr/bin/env python3
"""A fake external tool for the umbrella harness lanes.

It exists to make "the tool ran, with exactly these arguments, in exactly
this environment" an observable fact instead of an assumption:

  * it records its own ``argv`` (verbatim, every element) and the
    ``UMBRELLA_TEST_*`` environment subset plus ``PATH`` to ``--record``;
  * it writes an invocation marker to ``--marker`` with ``--marker-text``.

A tool that prints PASS and exits 0 without doing those things is exactly the
false success U01/HAR02 and U00's acceptance criterion forbid, so the fake
tool also supports *not* writing the marker: callers simply omit ``--marker``.

Usage:
  fake_tool.py [--record PATH] [--marker PATH] [--marker-text TEXT]
               [--print TEXT] [--exit N] [--sleep SECONDS] [--stderr TEXT]
               [-- any other arguments are recorded verbatim]
Exit: the requested ``--exit`` code (default 0).
"""

import json
import os
import sys
import time

# Options this tool consumes. Everything else stays in `argv` untouched, so a
# caller can assert that the harness passed an argument with spaces or an
# embedded `--` through without re-splitting it.
VALUE_OPTS = {
    "--record",
    "--marker",
    "--marker-text",
    "--print",
    "--exit",
    "--sleep",
    "--stderr",
}


def parse(raw):
    opts = {}
    rest = []
    i = 0
    while i < len(raw):
        a = raw[i]
        name, sep, inline = a.partition("=")
        if sep and name in VALUE_OPTS:
            opts[name] = inline
        elif a in VALUE_OPTS:
            i += 1
            if i >= len(raw):
                raise SystemExit("%s needs a value" % a)
            opts[a] = raw[i]
        else:
            rest.append(a)
        i += 1
    return opts, rest


def main():
    opts, rest = parse(sys.argv[1:])

    print(opts.get("--print", ""), end="")
    if "--stderr" in opts:
        print(opts["--stderr"], file=sys.stderr, end="")
    sys.stdout.flush()
    sys.stderr.flush()

    if "--sleep" in opts:
        time.sleep(float(opts["--sleep"]))

    if "--record" in opts:
        env = {
            k: v
            for k, v in os.environ.items()
            if k.startswith("UMBRELLA_TEST_") or k == "PATH"
        }
        payload = {
            "argv": sys.argv[1:],
            "rest": rest,
            "env": env,
            "cwd": os.getcwd(),
        }
        with open(opts["--record"], "w", encoding="utf-8") as fh:
            json.dump(payload, fh, indent=2, sort_keys=True)

    if "--marker" in opts:
        with open(opts["--marker"], "w", encoding="utf-8") as fh:
            fh.write(opts.get("--marker-text", "invoked"))

    return int(opts.get("--exit", "0"))


if __name__ == "__main__":
    sys.exit(main())
