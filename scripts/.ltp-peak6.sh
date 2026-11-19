set -u
D=/home/leo/.local/state/australVM-lean/ns-share
cd /home/leo/Projects/australVM || exit 1
ulimit -v 6291456          # 6 GiB
rm -rf "$D/peak6"; mkdir -p "$D/peak6"
export LTP_OBLIG_SHARE=1 LTP_OBLIG_CHUNK=2147483648 LTP_EXPRS_CACHE=20000
python3 scripts/lean-typing-proof \
  mm0/exports/BookProof.Ns.ChapterNsOuterFockSingleTime.ndjson \
  nsOuterFock_farisLavine_timeIndependent_singleTime "$D/peak6/out" --spine \
  > "$D/peak6/run.log" 2>&1 &
job=$!
while kill -0 "$job" 2>/dev/null; do
  s=$(cat /proc/"$job"/status 2>/dev/null)
  printf '%s\t%s\t%s\n' "$(date +%s)" \
    "$(echo "$s" | awk '/VmRSS/{print $2}')" "$(echo "$s" | awk '/VmSize/{print $2}')" \
    >> "$D/peak6/rss.tsv"
  sleep 1
done
wait "$job"
echo "exit=$?" >> "$D/peak6/run.log"
