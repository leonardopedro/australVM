#!/usr/bin/env bash
#
# Negative control for the execution step of run.sh.
#
# A green run.sh proves the cell executed; it does not prove the check would
# notice the failure it was written for. This script takes the one defect the
# execution step found — `link_with` subtracting the rel32 field size in
# addition to Cranelift's -4 addend — puts that line back, rebuilds avm-pkg,
# and requires `avm-pkg load --exec` to stop returning the right value. It
# then restores the file and requires the value back. It also requires the
# Cedar gate to refuse a cell with `Execute` denied, so a job whose gate is
# wired to nothing cannot stay green either.
#
# Needs the cells and executor produced by `run.sh` (same default work dir).
# The linker source is restored by an EXIT trap; the control refuses to start
# on a dirty `linker.rs`, because restoring with `git checkout` over someone's
# edit would be a destructive way to make a test pass.
#
#   examples/nix-pkg/run.sh
#   examples/nix-pkg/linker-control.sh
#
set -euo pipefail

DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
ROOT=$(cd "$DIR/../.." && pwd)
PKG=$ROOT/safestos/pkg
LINKER=$PKG/avm-exec/src/linker.rs
WORK=${AVM_NIXPKG_WORK:-$ROOT/target/nix-pkg}
CELL=$WORK/cells/apply.avm
EXEC=$WORK/exec-image
AVM_PKG=${AVM_PKG:-$PKG/target/release/avm-pkg}
POLICY=$ROOT/safestos/pkg/policies/allow.cedar

for f in "$CELL" "$EXEC" "$POLICY"; do
  [ -f "$f" ] || { echo "linker-control: missing $f — run examples/nix-pkg/run.sh first" >&2; exit 2; }
done
# The control saves and restores the exact bytes, so an uncommitted fix is
# fine; it only needs the fixed line to be the one in the file.
if ! grep -q -- '- site as i128;' "$LINKER"; then
  echo "linker-control: the fixed PC-relative line is not in $LINKER — nothing to mutate" >&2
  exit 2
fi

SAVE=$WORK/linker.rs.control-orig
cp "$LINKER" "$SAVE"
restore() {
  cp "$SAVE" "$LINKER"
  rm -f "$SAVE"
}
trap restore EXIT

# The value the fixed linker must produce (examples/nix-pkg/apply.nix = 42).
load_rax() {
  local out
  if ! out=$("$AVM_PKG" load "$CELL" --policy "$POLICY" --base 0x10000000 \
       --resolve 'Austral.Pervasive::trappingAdd=0x20000000' \
       --resolve 'Austral.Pervasive::trappingMultiply=0x20000010' \
       --resolve 'au_fadd=0x20000020' --resolve 'au_fsub=0x20000040' \
       --resolve 'au_f64_add=0x20000020' --resolve 'au_f64_subtract=0x20000040' \
       --resolve 'au_f64_multiply=0x20000060' --resolve 'au_f64_divide=0x20000080' \
       --resolve 'au_fmul=0x20000060' --resolve 'au_fdiv=0x20000080' \
       --exec "$EXEC" 2>&1); then
    return 1
  fi
  printf '%s' "$out" | sed -n 's/.*"stdout":"rax=\(-\{0,1\}[0-9]*\) f64_bits=.*/\1/p'
}

# 1. The gate half: a policy that denies Execute must refuse before linking.
DENY=$ROOT/safestos/pkg/policies/deny-execute.cedar
set +e
deny_out=$("$AVM_PKG" load "$CELL" --policy "$DENY" 2>&1)
deny_rc=$?
set -e
if [ "$deny_rc" -eq 0 ] || ! printf '%s' "$deny_out" | grep -q 'EPERM: capability denied for action "Execute"'; then
  echo "linker-control: [fail] Execute denial did not refuse (exit $deny_rc):" >&2
  printf '%s\n' "$deny_out" >&2
  exit 1
fi
echo "  [ok] Cedar gate: Execute denied -> EPERM (exit $deny_rc)"

# 2. Put the double-subtraction back and require the execution check to fail.
sed -i 's/- site as i128;/- (site as i128 + 4);/' "$LINKER"
if ! grep -q -- '- (site as i128 + 4);' "$LINKER"; then
  echo "linker-control: [fail] mutation did not apply to $LINKER" >&2
  exit 1
fi
(cd "$PKG" && cargo build --release -p avm-cli >/dev/null 2>&1) ||
  { echo "linker-control: [fail] rebuild with the mutation failed" >&2; exit 1; }

set +e
mut_rax=$(load_rax)
mut_rc=$?
set -e
if [ "$mut_rc" -eq 0 ] && [ "$mut_rax" = 42 ]; then
  echo "linker-control: [fail] pre-fix linker still returned 42 — the check is vacuous" >&2
  exit 1
fi
echo "  [ok] pre-fix linker rejected: load/exec did not return 42 (exit $mut_rc, rax=${mut_rax:-none})"

# 3. Restore, rebuild, and require the check to pass again.
cp "$SAVE" "$LINKER"
(cd "$PKG" && cargo build --release -p avm-cli >/dev/null 2>&1) ||
  { echo "linker-control: [fail] rebuild after restore failed" >&2; exit 1; }
fixed=$(load_rax) || { echo "linker-control: [fail] load failed after restore" >&2; exit 1; }
if [ "$fixed" != 42 ]; then
  echo "linker-control: [fail] restored linker returned ${fixed:-none}, want 42" >&2
  exit 1
fi
echo "  [ok] restored linker returns 42"
echo "linker-control: the execution step fails on a broken linker and on a dead gate"
