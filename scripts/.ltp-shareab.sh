set -u
D=/home/leo/.local/state/australVM-lean/share-ab-def
cd /home/leo/Projects/australVM || exit 1
ulimit -v 12582912
export LTP_PROBE_LOOKUP=1
for ver in head cur; do
  [ "$ver" = head ] && script=scripts/.ltp-head2.py || script=scripts/lean-typing-proof
  rm -rf "$D/$ver"; mkdir -p "$D/$ver"
  /usr/bin/time -f "$ver %e s %M KiB" python3 "$script" \
    mm0/exports/BookProof.Toolbox.ChapterFockPairPerturbation.ndjson \
    abs_re_inner_pairVec_le "$D/$ver/out" --spine > "$D/$ver/log" 2>&1
  grep -E "probe|^$ver " "$D/$ver/log"
done
for f in "$D/head/out"/*; do b=$(basename "$f"); cmp -s "$f" "$D/cur/out/$b" && echo "same $b" || echo "DIFF $b"; done
