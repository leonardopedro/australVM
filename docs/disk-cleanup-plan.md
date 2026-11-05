# Cleaning the disk

Measured 2026-11-05, root filesystem `/dev/nvme0n1p5`: **331 G total, 313 G used,
397 M free (100%)**. It has already cost sweep verdicts: 34 modules were recorded
`failed` with `no space left on device` from nanoda and lean4export, which is a
statement about the disk, not about those chapters.

## Where the space went

| | size |
|---|---|
| `Projects/australVM/mm0/exports` | 91 G |
| `/nix/store` | 42 G |
| `Projects/australVM/mm0/statements` | 29 G |
| `Projects/math/lean/.lake` | 18 G |
| `/swapfile` | 16 G |
| `Projects/timepiece/.lake` | 12 G |
| `~/.elan` | 12 G |
| `/tmp` | 7.9 G |
| `~/.local` (openshell overlays) | 7.2 G |
| `~/.rustup` | 5.2 G |
| `/var/lib/waydroid` | 4.4 G |
| `~/.cargo`, `~/.config`, `~/.cache`, `~/.opam` | 2.6 + 2.3 + 1.8 + 1.6 G |

The corpus artifacts dominate: `mm0/` is 118 G by itself. Of that, **43 G belongs
to the 63 modules that are not `ok`** — 23.9 G of `.mm0`/`.mm1` statements and
19.5 G of `.ndjson` exports that produced no accepted output and can be
regenerated from source at will.

## Phase 0 — stop making it worse (minutes)

The sweep is still running and still writing. Any further ENOSPC turns into more
false verdicts.

```bash
pkill -f 'lean[-]corpus'                    # not 'lean-corpus-mm0': that matches the invoking shell
```

Quarantine the poisoned rows so the retry pass re-attempts those modules
honestly instead of trusting a disk error:

```bash
grep -l 'No space left\|os error 28\|resource exhausted' mm0/sweep-logs/*.log \
  | sed 's#.*/##; s/\.log$//' | sort > /tmp/enospc.mods
awk -F'\t' 'NR==FNR{bad[$0]; next} !($2 in bad)' /tmp/enospc.mods mm0/sweep-state.tsv > /tmp/s.tsv
awk -F'\t' '!seen[$2]++' /tmp/s.tsv > mm0/sweep-state.tsv   # also drops the 5+ duplicate rows
find mm0/exports -size 0 -delete                             # 110 aborted exports
```

## Phase 1 — reclaim with no rebuild cost (~45 G)

1. **Artifacts of non-ok modules: 43 G.** Exports and statements only exist for
   modules that produced accepted output. For the rest they are debris.

   ```bash
   awk -F'\t' '$1!="ok"{print $2}' mm0/sweep-state.tsv | sort -u > /tmp/bad.mods
   while read -r m; do
     rm -f "mm0/exports/$m.ndjson" mm0/statements/"$m".mm0 mm0/statements/"$m".mm1
   done < /tmp/bad.mods
   ```

2. **`/tmp`: 7.9 G.** `pv-backup-pre-scrub.git` and `pv-backup-pre-rewrite.git`
   (1.5 G, from an old history rewrite), `negtest` (523 M), `lean4export` (228 M),
   `l4e428` (211 M), `tmp.udxrqodKNH` (166 M), `openshell-vm-driver-leo-vm-dev`
   (371 M). Move the two git backups to an external disk if they are still wanted,
   otherwise delete.

3. **Unused Lean toolchains: 6.0 G.** Every `lean-toolchain` under `Projects/`
   was checked: timepiece and lean4export pin v4.28.0; math and lean4export-v434
   pin v4.34.1; physlib pins v4.35.0; arctic_authority pins v4.31.0. Installed are
   4.28.0, 4.33.1, 4.34.1, **4.36.0-rc1** — the last two are pinned by nothing, so
   both go (3.1 G + 2.9 G).

   ```bash
   elan toolchain uninstall leanprover/lean4:v4.36.0-rc1
   elan toolchain uninstall leanprover/lean4:v4.33.1
   ```

4. **Old Rust toolchain: 1.7 G.** `rustup toolchain uninstall 1.95.0-x86_64-unknown-linux-gnu`
   (keep 1.99.0 and nightly), then `rm -rf ~/.cargo/registry/cache/*`.

5. **Journal: ~0.5 G.** `journalctl --vacuum-size=200M`.

## Phase 2 — reclaim with a rebuild cost (~40 G)

6. **Nix store: 42 G, 20 system generations.** `sudo nix-collect-garbage -d`
   (expect 15–25 G). This is the largest structural win after `mm0/`.
7. **`Projects/math/lean/.lake`: 18 G.** Rebuildable oleans; only do it if that
   project is idle, since a rebuild is hours.
8. **Waydroid: 4.4 G** if Android-in-a-container is not in use
   (`/var/lib/waydroid`, needs root).
9. **Long tail:** `~/.opam` 1.6 G, `~/.kani` 485 M, `~/Downloads` 459 M,
   `~/.aliyun` 109 M (leftover cloud CLI packaging).

**Do not touch while converting:** `timepiece/.lake` (12 G of oleans the sweep
reads), `/swapfile` (16 G — it is what keeps the memory-heavy modules from
reaching the OOM killer), and `mm0/exports/*` for `ok` modules.

## Phase 3 — make this not recur

10. **`mm0/exports` is scratch, not a deliverable.** 91 G of `.ndjson`
    intermediates for 670 modules, kept forever by default. Keep statements
    (29 G, the actual `mm0`/`mm1` output) and treat exports as a cache: purge per
    module after acceptance, or cap the directory (`mtime +7 -delete`).

11. **The sweep needs a disk preflight, and ENOSPC is `deferred`, not `failed`.**
    A full disk is the exact twin of the memory floor already handled inside
    `run_module()`: a host condition that must not become a verdict about a
    module. Concretely: refuse to start below `SWEEP_MIN_FREE_GIB` (say 8), print
    free space in the header, and map ENOSPC in the module log to `deferred` so
    the module is re-attempted rather than counted as broken. Until then, every
    run risks re-inventing these 34 phantom failures.

12. **Move `mm0/` off the root filesystem** (external NVMe or a second disk). The
    corpus artifacts are the only thing here that grows without bound: 118 G for
    883 modules is ~130 M/module average and ~2 G for the heaviest chapters.

## Verification

After Phase 1 and 2: `df -h /` should show >100 G free, and
`du -sh mm0 mm0/exports mm0/statements` should show the exports directory near
zero and statements near 5 G (only `ok` modules). Then restart the sweep; the
quarantined modules are unrecorded, so pass 1 picks them up again by itself.

## Status 2026-11-05 (phases 0 and 1 executed)

Free space went from **397 M** (measured during the incident) / 12 G to **41 G**:
`/` is at 88%. Reclaimed: 20.2 G of artifacts from the 29 non-`ok` modules (42
files), 110 zero-byte exports, 6.5 G of unused Lean toolchains (4.36.0-rc1 and
4.33.1), 1.7 G of the 1.95.0 Rust toolchain, and 1.3 G of `/tmp` debris
(`negtest`, `l4e428`, `tmp.udxrqodKNH`, `openshell-vm-driver-leo-vm-dev`).

Also done: the 34 ENOSPC-poisoned rows were quarantined and the state deduped to
533 rows (504 `ok`, 10 `failed`, 13 `deferred`, 6 `empty`, 0 duplicates), leaving
352 modules unrecorded for the next pass.

One casualty, now repaired: `/tmp/l4e428` was the driver's default
`LEAN_EXPORTER`, so removing it broke the pipeline. `scripts/lean-corpus-mm0` now
resolves `../lean4export` first and keeps `/tmp` builds only as a fallback;
verified by re-converting `BookProof.Bell.ChapterBell` (15 declarations → 293
lines, mm0-c OK).

Still open: `/tmp/pv-backup-pre-{scrub,rewrite}.git` (1.5 G — they hold pre-rewrite
history of `verifiedUniqueAliases`; delete or move to external storage),
`journalctl --vacuum` (needs root), and all of phase 2 — `nix-collect-garbage -d`
also needs root — plus 85 G of exports still held for `ok` modules (phase 3).
