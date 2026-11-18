#!/usr/bin/env bash
# §13.36 candidate: does a smaller export cache move the memory ceiling? The
# completed-state run died of `MemoryError` at the 10 GiB address-space cap with
# 5 494 MiB peak RSS after 566 obligations; if `LazyExprs._cache` (200 000 parsed
# records) is the accumulator, bounding it at 20 000 should lower peak RSS and
# let the walk finish. RSS and free disk are sampled so a kill comes with numbers.
set -u
D=/home/leo/.local/state/australVM-lean/ns-share
cd /home/leo/Projects/australVM || exit 1
ulimit -v 12582912
rm -rf "$D/cap"; mkdir -p "$D/cap"
export LTP_OBLIG_SHARE=1 LTP_OBLIG_CHUNK=2147483648 LTP_EXPRS_CACHE=20000
python3 scripts/lean-typing-proof \
  mm0/exports/BookProof.Ns.ChapterNsOuterFockSingleTime.ndjson \
  nsOuterFock_farisLavine_timeIndependent_singleTime "$D/cap/out" --spine \
  > "$D/cap/run.log" 2>&1 &
job=$!
while kill -0 "$job" 2>/dev/null; do
  sleep 30
  # VmSize is what `ulimit -v` counts: at 10 GiB the walk died with VmRSS at
  # 4.7 GB, so the gap between the two is the thing to watch, not RSS alone.
  rss=$(awk '/VmRSS/{print $2}' /proc/"$job"/status 2>/dev/null)
  vsz=$(awk '/VmSize/{print $2}' /proc/"$job"/status 2>/dev/null)
  vdt=$(awk '/VmData/{print $2}' /proc/"$job"/status 2>/dev/null)
  avail=$(awk '/MemAvailable/{print $2}' /proc/meminfo)
  free=$(df -m --output=avail /home | tail -1 | tr -d ' ')
  printf '%s\t%s\t%s\t%s\t%s\t%s\n' "$(date +%s)" "${rss:-0}" "${vsz:-0}" "${vdt:-0}" "$avail" "$free" >> "$D/cap/rss.tsv"
done
wait "$job"
echo "exit=$?" >> "$D/cap/run.log"
echo done
