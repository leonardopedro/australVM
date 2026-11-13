"""What the encoder alone costs on one declaration.

`lean-proof-scan` reports `emit: over 10 GiB` for `adjoint_aeval`, and the
growth trace shows the axiom text is only about a quarter of that at 1.4 GB.
Two cheaper explanations come first: the export's expression table (indexed
once, `array('q')` offsets, no dicts), and the encoded strings for the
theorem's own type and value. Both are measured here without deriving
anything — same loader, same `Enc`, same export, same declaration.
"""

import importlib.machinery
import importlib.util
import sys
import time
from pathlib import Path

AVM = Path(__file__).resolve().parents[3]
loader = importlib.machinery.SourceFileLoader("lean2mm0", str(AVM / "scripts" / "lean2mm0"))
spec = importlib.util.spec_from_loader("lean2mm0", loader)
l2m = importlib.util.module_from_spec(spec)
sys.modules["lean2mm0"] = l2m
loader.exec_module(l2m)

# `Enc` lives in the emitter script, which is not a module; exec it with
# `__name__ != "__main__"` so its `main()` does not run.
src = (AVM / "scripts" / "lean-typing-proof").read_text()
ns = {"__name__": "ltp_lib", "__file__": str(AVM / "scripts" / "lean-typing-proof")}
exec(compile(src, ns["__file__"], "exec"), ns)
Enc = ns["Enc"]


def rss_mb() -> int:
    with open("/proc/self/statm") as fh:
        return int(fh.read().split()[1]) * 4096 >> 20


def main() -> int:
    export = Path(sys.argv[1])
    want = sys.argv[2]
    t0 = time.time()
    ex = l2m.Export(export, lazy=True)
    print(f"indexed      : {rss_mb():6d} MB, {len(ex.exprs):>9} exprs, {time.time()-t0:6.1f} s")
    enc = Enc(ex)
    target = next(
        (nm for nm, (_k, rec, _i) in ex.decls.items() if want in nm and "value" in rec), None
    )
    if target is None:
        raise SystemExit(f"no declaration matching {want!r}")
    _kind, rec, _ = ex.decls[target]
    ty = enc.enc(rec["type"])
    print(f"type encoded : {rss_mb():6d} MB, {len(ty):>10} B, {time.time()-t0:6.1f} s")
    val = enc.enc(rec["value"])
    print(f"value encoded: {rss_mb():6d} MB, {len(val):>10} B, {time.time()-t0:6.1f} s")
    cache = ex.exprs._cache
    table = sum(sys.getsizeof(v) for v in cache.values())
    print(f"parse cache  : {rss_mb():6d} MB, {len(cache):>9} nodes cached ({table/1e6:.1f} MB of dicts, cap {ex.exprs.CACHE_MAX})")
    print(f"encoder memo : {enc._memo_bytes/1e6:8.1f} MB, {len(enc._memo)} entries")
    print(f"final rss    : {rss_mb():6d} MB, {time.time()-t0:6.1f} s")
    return 0


if __name__ == "__main__":
    sys.exit(main())
