"""Shared facts for the umbrella lanes: what is installed, what is being
tested, and how a child process actually behaved.

Nothing here decides whether a test passes; it reports measurements so a case
can compare them with its oracle. The two distinctions the harness exists to
keep are the ones U00 is accepted on:

  * a *gap* (a prerequisite this environment does not have) is not a pass and
    is not a failure either — strict mode is what turns it into an error;
  * a child exiting 0 is not evidence that the operation happened. Cases check
    the marker/oracle the child was supposed to produce.
"""

import hashlib
import json
import os
import platform
import shutil
import subprocess
import sys
import time

BRIDGE_LIB = "safestos/cranelift/target/release/libaustral_cranelift_bridge.so"

# The stages a prerequisite report must be able to name individually
# (BLD08: "specific prerequisite error with stage identity").
PREREQ_STAGES = ("cargo", "rustc", "linker", "frontend", "bridge")

PREREQ_REMEDY = {
    "cargo": "install the Rust toolchain, or run under the dev shell",
    "rustc": "install the Rust toolchain, or run under the dev shell",
    "linker": "install a native C toolchain (cc/ld)",
    "frontend": 'build the stock front-end: nix develop "git+file://$PWD#ocaml" -c make all',
    "bridge": 'build the release bridge: nix develop "git+file://$PWD#ocaml" -c make bridge',
}


class CaseFailure(Exception):
    """The case ran and its stated oracle did not hold."""


class Gap(Exception):
    """A required tool/feature is absent; not a pass, not a failure."""


def sha256_file(path):
    try:
        h = hashlib.sha256()
        with open(path, "rb") as fh:
            for chunk in iter(lambda: fh.read(1 << 20), b""):
                h.update(chunk)
        return h.hexdigest()
    except OSError:
        return None


def run_capture(argv, *, timeout, env=None, cwd=None):
    """Run a child, always returning a record — a timeout is data, not an
    exception, because the caller's oracle is what decides the verdict."""
    started = time.monotonic_ns()
    record = {
        "argv": list(argv),
        "cwd": str(cwd) if cwd is not None else None,
        "timed_out": False,
        "returncode": None,
        "signal": None,
        "stdout": "",
        "stderr": "",
        "duration_ns": 0,
    }
    try:
        proc = subprocess.run(
            argv,
            cwd=cwd,
            env=env,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            timeout=timeout,
        )
        record["returncode"] = proc.returncode
        record["stdout"] = proc.stdout.decode("utf-8", "replace")
        record["stderr"] = proc.stderr.decode("utf-8", "replace")
        if proc.returncode is not None and proc.returncode < 0:
            record["signal"] = -proc.returncode
    except subprocess.TimeoutExpired as exc:
        record["timed_out"] = True
        record["stdout"] = (exc.stdout or b"").decode("utf-8", "replace")
        record["stderr"] = (exc.stderr or b"").decode("utf-8", "replace")
    except OSError as exc:
        record["spawn_error"] = str(exc)
    record["duration_ns"] = time.monotonic_ns() - started
    return record


def _version_of(argv, env, timeout=15):
    rec = run_capture(argv, timeout=timeout, env=env)
    if rec["returncode"] != 0:
        return None
    first = rec["stdout"].strip().splitlines()
    return first[0] if first else None


def _which(name, env):
    path = (env or os.environ).get("PATH", os.defpath)
    return shutil.which(name, path=path)


def probe_prerequisites(root, env=None):
    """Report each prerequisite stage by name, with its own detail and remedy.

    The oracle is that a missing tool is identified *as that stage* rather
    than as a generic "environment broken" — that is what makes the failure
    actionable and what BLD08 asks for.
    """
    env = dict(os.environ if env is None else env)
    report = []

    def entry(stage, path, version=None):
        ok = path is not None and os.path.exists(path)
        report.append(
            {
                "stage": stage,
                "status": "ok" if ok else "missing",
                "path": str(path) if path is not None else None,
                "version": version if ok else None,
                "detail": (
                    "%s found at %s" % (stage, path)
                    if ok
                    else "%s not found: %s"
                    % (
                        stage,
                        {
                            "cargo": "no `cargo` on PATH",
                            "rustc": "no `rustc` on PATH",
                            "linker": "no native `cc` on PATH",
                            "frontend": "no stock `austral` front-end at %s"
                            % os.path.join(root, "austral"),
                            "bridge": "no release bridge at %s"
                            % os.path.join(root, BRIDGE_LIB),
                        }[stage],
                    )
                ),
                "remedy": None if ok else PREREQ_REMEDY[stage],
            }
        )

    cargo = _which("cargo", env)
    entry("cargo", cargo, _version_of([cargo, "--version"], env) if cargo else None)

    rustc = _which("rustc", env)
    entry("rustc", rustc, _version_of([rustc, "--version"], env) if rustc else None)

    linker = _which("cc", env)
    entry("linker", linker, _version_of([linker, "--version"], env) if linker else None)

    frontend = os.path.join(root, "austral")
    entry(
        "frontend",
        frontend if os.path.isfile(frontend) else None,
        _version_of([frontend, "--version"], env) if os.path.isfile(frontend) else None,
    )

    bridge = os.path.join(root, BRIDGE_LIB)
    entry("bridge", bridge if os.path.isfile(bridge) else None)

    return report


def _toml(path):
    try:
        import tomllib
    except ImportError:  # pragma: no cover - Python < 3.11
        return None
    try:
        with open(path, "rb") as fh:
            return tomllib.load(fh)
    except (OSError, ValueError):
        return None


def record_identity(root):
    """Compiler/bridge/umbrella identities, lock hashes, features, target and
    native linker — captured once per run, before any case executes."""
    identity = {"repo_root": os.path.realpath(root)}

    umb = _toml(os.path.join(root, "umbrella/Cargo.toml"))
    if umb is not None:
        pkg = umb.get("package", {})
        identity["umbrella"] = {
            "name": pkg.get("name"),
            "version": pkg.get("version"),
            "manifest_sha256": sha256_file(os.path.join(root, "umbrella/Cargo.toml")),
        }
        dep = umb.get("dependencies", {}).get("austral_cranelift_bridge", {})
        identity["umbrella"]["bridge_dependency"] = {
            "default_features": dep.get("default-features", True),
            "features": dep.get("features", []),
        }

    bridge = _toml(os.path.join(root, "safestos/cranelift/Cargo.toml"))
    if bridge is not None:
        feats = bridge.get("features", {})
        identity["bridge"] = {
            "version": bridge.get("package", {}).get("version"),
            "available_features": sorted(feats),
            "default_features": list(feats.get("default", [])),
        }

    locks = {}
    for rel in (
        "Cargo.lock",
        "umbrella/Cargo.lock",
        "umbrella/modules/rscrate/Cargo.lock",
        "safestos/cranelift/Cargo.lock",
    ):
        p = os.path.join(root, rel)
        if os.path.isfile(p):
            locks[rel] = sha256_file(p)
    identity["lockfiles"] = locks

    git = run_capture(
        ["git", "-C", root, "rev-parse", "HEAD"], timeout=15, env=os.environ
    )
    status = run_capture(
        ["git", "-C", root, "status", "--porcelain"], timeout=30, env=os.environ
    )
    dirty = [
        line for line in status["stdout"].splitlines() if line.strip()
    ]
    identity["git"] = {
        "rev": git["stdout"].strip() if git["returncode"] == 0 else None,
        "dirty_entries": len(dirty),
    }

    rustc = _which("rustc", os.environ)
    verbose = (
        run_capture([rustc, "-vV"], timeout=15, env=os.environ) if rustc else None
    )
    host = None
    if verbose is not None and verbose["returncode"] == 0:
        for line in verbose["stdout"].splitlines():
            if line.startswith("host: "):
                host = line[len("host: ") :].strip()
    identity["toolchain"] = {
        "python": sys.version.split()[0],
        "cargo": _version_of([_which("cargo", os.environ), "--version"], os.environ)
        if _which("cargo", os.environ)
        else None,
        "rustc": _version_of([rustc, "--version"], os.environ) if rustc else None,
        "rustc_host": host,
        "dune": _version_of([_which("dune", os.environ), "--version"], os.environ)
        if _which("dune", os.environ)
        else None,
    }

    cc = _which("cc", os.environ)
    identity["native"] = {
        "machine": platform.machine(),
        "system": platform.system(),
        "linker": cc,
        "linker_target": _version_of([cc, "-dumpmachine"], os.environ) if cc else None,
        "ld": _version_of([_which("ld", os.environ), "--version"], os.environ)
        if _which("ld", os.environ)
        else None,
    }
    return identity


def write_json(path, payload):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    tmp = path + ".tmp"
    with open(tmp, "w", encoding="utf-8") as fh:
        json.dump(payload, fh, indent=2, sort_keys=True)
        fh.write("\n")
    os.replace(tmp, path)


def read_json(path):
    with open(path, "r", encoding="utf-8") as fh:
        return json.load(fh)
