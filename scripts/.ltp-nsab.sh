set -u
B=/home/leo/.local/state/australVM-lean/ns-ab
cd /home/leo/Projects/australVM || exit 1
ulimit -v 12582912
export LTP_OBLIG_SHARE=1 LTP_PROBE_LOOKUP=1 LTP_STOP_RULES=20000 LTP_EXPRS_CACHE=20000
for ver in head cur; do
  [ "$ver" = head ] && script=scripts/.ltp-head2.py || script=scripts/lean-typing-proof
  rm -rf "$B/$ver"; mkdir -p "$B/$ver"
  /usr/bin/time -f "$ver %e s %M KiB" python3 "$script" \
    mm0/exports/BookProof.Ns.ChapterNsOuterFockSingleTime.ndjson \
    nsOuterFock_farisLavine_timeIndependent_singleTime "$B/$ver/out" --spine \
    > "$B/$ver/log" 2>&1
  grep -E "probe|^$ver " "$B/$ver/log"
done
