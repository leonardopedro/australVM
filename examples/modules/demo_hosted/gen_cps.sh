#!/usr/bin/env bash
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OUT="$HERE/module.cps"

# CPS v1 binary: one function "run" that returns 42.
# Format: magic(u32) func_count(u32) [name(str) params(u32) ret(u8) body_len(u32) body]*
# Body: 0x07(Return) 0x01(IntLit) 42(i64)
printf '\x31\x53\x50\x43' > "$OUT"   # magic 0x43505331 LE
printf '\x01\x00\x00\x00' >> "$OUT"  # func_count = 1
printf '\x03\x00\x00\x00' >> "$OUT"  # name_len = 3
printf 'run'              >> "$OUT"  # name
printf '\x00\x00\x00\x00' >> "$OUT"  # param_count = 0
printf '\x00'             >> "$OUT"  # ret_type = 0 (Unit/I64)
printf '\x0a\x00\x00\x00' >> "$OUT"  # body_len = 10
printf '\x07'             >> "$OUT"  # Return
printf '\x01'             >> "$OUT"  # IntLit
printf '\x2a\x00\x00\x00\x00\x00\x00\x00' >> "$OUT"  # 42 as i64 LE

echo "Generated $OUT ($(wc -c < "$OUT") bytes)"
