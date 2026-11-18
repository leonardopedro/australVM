set -u
B=/home/leo/.local/state/australVM-lean/rawab
cd /home/leo/Projects/australVM || exit 1
ulimit -v 12582912
for mode in default shared; do
  if [ "$mode" = shared ]; then export LTP_OBLIG_SHARE=1; else unset LTP_OBLIG_SHARE; fi
  for ver in head cur; do
    [ "$ver" = head ] && script=scripts/.ltp-head2.py || script=scripts/lean-typing-proof
    rm -rf "$B/$mode-$ver"; mkdir -p "$B/$mode-$ver"
    /usr/bin/time -f "$mode-$ver %e s %M KiB" python3 "$script" \
      mm0/exports/BookProof.Toolbox.ChapterFockPairPerturbation.ndjson \
      abs_re_inner_pairVec_le "$B/$mode-$ver/out" --spine > "$B/$mode-$ver/log" 2>&1
    tail -1 "$B/$mode-$ver/log"
  done
  for f in "$B/$mode-head/out"/*; do b=$(basename "$f")
    cmp -s "$f" "$B/$mode-cur/out/$b" || echo "DIFF $mode $b"; done
done
echo compared
