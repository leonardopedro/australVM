#!/usr/bin/env bash
# Three-way A/B for §13.35: default, fast escaping, deeper export cache.
# One `nix develop` for all three, because concurrent invocations deadlock on
# the input's eval-cache SQLite.
set -u
D=/home/leo/.local/state/australVM-lean/ns-share
cd /home/leo/Projects/australVM || exit 1
ulimit -v 10485760
run() {  # name, env assignment
  local n=$1 e=${2:-}
  mkdir -p "$D/$n"
  ( export LTP_STOP_RULES=44700 LTP_OBLIG_CHUNK=2147483648
    [ -n "$e" ] && export "$e"
    { time python3 scripts/lean-typing-proof \
        mm0/exports/BookProof.Ns.ChapterNsOuterFockSingleTime.ndjson \
        nsOuterFock_farisLavine_timeIndependent_singleTime "$D/$n/out" --spine
    } > "$D/$n/run.log" 2>&1 ) &
}
run dflt2 ""
run fastesc2 "LTP_ENC_FASTESC=1"
run cache "LTP_EXPRS_CACHE=4000000"
wait
echo done
