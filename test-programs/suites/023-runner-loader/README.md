# 023 — the runner's own loader path

The compiler binary links the cranelift bridge as a shared library. Callers
used to have to export `LD_LIBRARY_PATH`, and without it every fixture died
with `libaustral_cranelift_bridge.so: cannot open shared object file` — a
message about the bridge for a failure in whichever fixture ran first.

`runner.py` now prepends the tree root and the bridge's release directory to
its own `LD_LIBRARY_PATH`. This suite is the check that the guarantee holds
from inside: no caller-set variable is needed (run `python3
test-programs/runner.py 023` with `LD_LIBRARY_PATH` unset), and the case
itself asserts both halves — the compiler starts, and it still fails at the
loader when the variable is taken away.
