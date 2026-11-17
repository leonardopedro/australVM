#!/usr/bin/env bash
# §13.35 part three: the lookup timer at the 44 700-rule budget, alone — the
# host cannot hold two walks (the first pair was killed at ~3 min).
set -u
D=/home/leo/.local/state/australVM-lean/ns-share
cd /home/leo/Projects/australVM || exit 1
ulimit -v 10485760
rm -rf "$D/probe/out"; mkdir -p "$D/probe"
export LTP_PROBE_LOOKUP=1 LTP_STOP_RULES=44700 LTP_OBLIG_CHUNK=2147483648
{ time python3 scripts/lean-typing-proof \
    mm0/exports/BookProof.Ns.ChapterNsOuterFockSingleTime.ndjson \
    nsOuterFock_farisLavine_timeIndependent_singleTime "$D/probe/out" --spine
} > "$D/probe/run.log" 2>&1
echo "exit=$?" >> "$D/probe/run.log"
echo done
