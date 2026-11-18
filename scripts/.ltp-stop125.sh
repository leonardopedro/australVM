#!/usr/bin/env bash
# §13.36: the stop's prefix claim, re-stated against a *complete* theory. The
# earlier witness was a run that later died of MemoryError; this run stops at the
# same rule count (125 417) and its axiom payload is compared with the complete
# `.mm0`'s payload byte for byte.
set -u
D=/home/leo/.local/state/australVM-lean/ns-share
cd /home/leo/Projects/australVM || exit 1
ulimit -v 12582912
rm -rf "$D/stop125b"; mkdir -p "$D/stop125b"
export LTP_OBLIG_SHARE=1 LTP_OBLIG_CHUNK=2147483648 LTP_EXPRS_CACHE=20000
export LTP_STOP_RULES=125417 LTP_STOP_EMIT=1
python3 scripts/lean-typing-proof \
  mm0/exports/BookProof.Ns.ChapterNsOuterFockSingleTime.ndjson \
  nsOuterFock_farisLavine_timeIndependent_singleTime "$D/stop125b/out" --spine \
  > "$D/stop125b/run.log" 2>&1
echo "exit=$?" >> "$D/stop125b/run.log"
echo done
