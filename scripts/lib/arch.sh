#!/usr/bin/env bash
# arch.sh — one answer to "was this artifact's host-native object digest
# recorded on this host?"
#
# `mm0/flux1d_certificate.mm0` records the architecture beside `obj=` in its
# `-- code-status:` line. That digest is an AVCE capture and is host-native, so
# a gate or control on another architecture must report the comparison as a
# stated gap: never a silent pass, never a false failure. Sourced by
# `scripts/verify-invariants` and `scripts/obj-drift-control` so both read the
# field and word the mismatch the same way.

avm_recorded_arch() { # avm_recorded_arch <artifact>
    sed -n 's/.*arch=\([a-z0-9_]*\)).*/\1/p' "$1" 2>/dev/null | head -1
}

avm_host_arch() {
    uname -m
}

avm_arch_gap() { # avm_arch_gap <artifact> [host-arch]
    # Prints "recorded arch=…, this host is …" and returns 0 when the recorded
    # arch is known and differs from the host's. Returns 1, printing nothing,
    # when they match or either side is unknown.
    local recorded host
    recorded="$(avm_recorded_arch "$1")"
    host="${2:-$(avm_host_arch)}"
    [ -n "$recorded" ] && [ -n "$host" ] && [ "$recorded" != "$host" ] || return 1
    printf 'recorded arch=%s, this host is %s' "$recorded" "$host"
}
