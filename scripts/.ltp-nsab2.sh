set -u
B=/home/leo/.local/state/australVM-lean/ns-ab2
cd /home/leo/Projects/australVM || exit 1
ulimit -v 10485760
export LTP_OBLIG_SHARE=1 LTP_STOP_RULES=60000 LTP_EXPRS_CACHE=20000
for ver in head cur; do
  [ "$ver" = head ] && script=scripts/.ltp-head2.py || script=scripts/lean-typing-proof
  rm -rf "$B/$ver"; mkdir -p "$B/$ver"
  /usr/bin/time -f "$ver %e s %M KiB" python3 "$script" \
    mm0/exports/BookProof.Ns.ChapterNsOuterFockSingleTime.ndjson \
    nsOuterFock_farisLavine_timeIndependent_singleTime "$B/$ver/out" --spine \
    > "$B/$ver/log" 2>&1
  tail -1 "$B/$ver/log"
done
for f in "$B/head/out"/*; do b=$(basename "$f")
  cmp -s "$f" "$B/cur/out/$b" && echo "same $b $(stat -c%s "$f")" || echo "DIFF $b"; done
