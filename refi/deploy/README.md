# Deploying `refi_service`

Target: a single Linux host (validated on the Oracle Cloud `ol-arm-free`
instance, Oracle Linux 10 / aarch64). The process speaks plain HTTP/1.1 on a
loopback port; TLS is terminated by the tunnel or reverse proxy in front of it,
so the service needs no certificate handling of its own.

## 1. Build

```sh
cd ~/Projects/australVM/refi
cargo build --release
```

The release build also needs the stock `austral` front-end at
`../austral` (already built in this repo) and `libaustral_cranelift_bridge.so`
next to it. The front-end is only used to turn `modules/RefiPolicy.aum` into
CPS; after the first start the artifact is cached in the state dir.

Prebuild the policy artifact (recommended, so a runtime never needs OCaml):

```sh
AUSTRAL_BIN=../austral ../austral compile modules/RefiPolicy.aum \
  --use-cps-jit --allow-all --target-type=tc \
  --emit-cps=$HOME/.local/share/refi/RefiPolicy.cps
```

## 2. State directory

`~/.local/share/refi/` (mode 0700) holds:

| file | contents | backup |
|---|---|---|
| `treasury.key` | ed25519 seed of the mint authority, 0600 | **yes, separately** |
| `admin.token` | bootstrap admin bearer token, 0600 | yes |
| `exchange.json` | exchange snapshot + daily-cap counters, 0600 | yes |
| `accounts.json` | accounts with custodial sandbox keys, 0600 | yes |
| `content.json` | site pages | yes |
| `proofs.json` | the proof ledger: submitted developments, states, Zenodo DOIs | yes |
| `RefiPolicy.cps` | compiled policy kernel | no (regenerated) |

Restoring `exchange.json` under a different `treasury.key` is refused at
startup — the snapshot records the treasury DID. Keep the key with the state,
or the money state is unreadable by design.

## 3. systemd (user unit)

```sh
mkdir -p ~/.config/refi ~/.local/share/refi
cp deploy/refi.env.example ~/.config/refi/refi.env
chmod 600 ~/.config/refi/refi.env
# edit the paths and base URL
mkdir -p ~/.config/systemd/user
cp deploy/refi.service ~/.config/systemd/user/refi.service
systemctl --user daemon-reload
systemctl --user enable --now refi.service
journalctl --user -u refi.service -f
```

The unit runs with `ProtectSystem=strict`, `ProtectHome=read-only`, a single
writable path (the state dir), `NoNewPrivileges` and `UMask=0077`.
`MemoryDenyWriteExecute` is **not** set: the Cranelift JIT maps its code pages
RWX, and denying that makes the policy kernel fail to compile at startup
(observed on this host: `unable to make memory readable+executable`). If the
bridge ever moves to a W^X mapping, enable the directive again.

Keep the listener on loopback. Put a TLS-terminating front in front of it (a
cloudflared/`ssh -R` tunnel, or nginx/caddy on the same host) and point it at
`127.0.0.1:8080`:

```sh
# in ~/.config/refi/refi.env
REFI_LISTEN=127.0.0.1:8080
REFI_BASE_URL=https://refi.example.org
```

The public origin in `REFI_BASE_URL` is what makes cookies `Secure`; it must be
the HTTPS URL users actually visit.

## 3a. The proof service (`mm0web`)

The exchange publishes mm0 developments through `mm0web`
(`unfer/mm0web/deploy/README.md`), which verifies them with `mm0-rs` + `mm0-c`
and archives them on Zenodo. Both run on loopback, side by side:

```sh
# ~/.config/mm0web/mm0web.env  (see unfer/mm0web/deploy/README.md)
# ~/.config/refi/refi.env
REFI_MM0WEB=http://127.0.0.1:8081
```

`REFI_MM0WEB` empty (or `--mm0web ""`) runs the exchange without a proof
backend: `/proofs` still lists what was archived, and submissions are refused
with `503` rather than silently dropped.

Without `ZENODO_API_KEY` on the mm0web side nothing is archived — mm0web
verifies and stores the proof, and the exchange records it as `verified` with
the reason, not as `published`. That distinction is the point: a verified proof
without a DOI is not a citation.

## 4. Public HTTPS on Oracle Cloud

Three separate gates, all required:

1. **Certificate.** Point a DNS name at the instance's public IP and issue a
   certificate for the front proxy (Let's Encrypt HTTP-01 on port 80, or
   DNS-01). A short-lived `sslip.io`-style name (`<public-ip>.sslip.io`) works
   for a test deployment but is not a production identity. With a tunnel
   instead of an open port, neither this nor the two gates below are needed —
   the tunnel terminates TLS and dials the loopback listener.
2. **Oracle security list / NSG.** Add ingress rules for TCP 80 (ACME) and 443
   (service) to the instance's VCN subnet; the default free-tier rule set only
   opens 22.
3. **Host firewall.** Oracle Linux images ship `firewalld` with only SSH open:

   ```sh
   sudo firewall-cmd --permanent --add-service=https
   sudo firewall-cmd --permanent --add-service=http   # only while issuing
   sudo firewall-cmd --reload
   ```

Verification from outside the VM:

```sh
curl -sS https://<name>/healthz
curl -sS https://<name>/ | head
```

Set `REFI_BASE_URL` to the same origin so pages and the API docs name the
right host; cookies become `Secure` automatically when it starts with
`https://`.

## 5. Operations

- **Health:** `GET /healthz` returns 503 if the fiat conservation audit breaks.
  That is the page-level alarm; wire it to whatever monitor you have.
- **Metrics:** `GET /metrics` (Prometheus text) — request/error/mutation
  counters, ledger supply, reserve and merchant totals, policy-loaded gauge.
- **Logs:** structured `tracing` lines on stderr → journald. The admin token is
  never logged.
- **Backup:** copy the state directory (or at least `exchange.json`,
  `accounts.json`, `content.json`, `treasury.key`). `exchange.json` is written
  atomically (temp file + fsync + rename + directory fsync), so a copy taken at
  any instant is a consistent snapshot; `treasury.key` must travel with it.
- **Upgrade:** rebuild, `systemctl --user restart refi.service`. The state file
  carries `version: 1`; a future format change must migrate on read.
- **Rotate the admin token:** stop the service, replace `admin.token` (any
  32+ char secret), start it again. Account tokens are unaffected; the
  bootstrap admin account's hash is re-derived from the new token on startup.
- **Revoke an account:** remove its entry from `accounts.json` while stopped
  (there is no delete endpoint yet); its DID remains on-ledger and its coins
  remain spendable only by a key nobody holds, which is the safe direction.

## 6. What this deployment does *not* include

- A real bank/Taler wire gateway. `SimulatedWireGateway` books and confirms
  wires in-process; the UI labels the deployment as sandbox.
- Multi-node replication of the exchange mirror (the op log is ready for it).
- Rate limiting shared across processes (in-memory, per-instance).
- KYC, sanctions screening, VASP licensing, or carbon-registry integration
  beyond the provenance reference string.
