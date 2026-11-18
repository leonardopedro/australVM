set -u
D=/home/leo/.local/state/australVM-lean/share-ab
cd /home/leo/Projects/australVM || exit 1
ulimit -v 12582912
export LTP_OBLIG_SHARE=1
rm -rf "$D/prof"; mkdir -p "$D/prof"
python3 -c "
import cProfile, pstats, sys, runpy
sys.argv=['lean-typing-proof','mm0/exports/BookProof.Toolbox.ChapterFockPairPerturbation.ndjson','abs_re_inner_pairVec_le','$D/prof/out','--spine']
cProfile.run('runpy.run_path(\"scripts/lean-typing-proof\", run_name=\"__main__\")','$D/prof/prof')
p=pstats.Stats('$D/prof/prof'); p.sort_stats('tottime')
p.print_stats('__getitem__')
p.print_callers('__getitem__')
" 2>&1 | head -60
