# refi — ReFi exchange web service

A web service for the ReFi exchange framework that already exists in this
workspace: GNU Taler-style settlement (reserves → withdraw → deposit → peg-out)
over the `unfer` carbon-certificate ledger (`unfer_consensus::certs`), with
UNFCCC provenance on mints.

Two things are deliberately unusual about it:

1. **The business policy is Austral.** Every mutating command is routed through
   `modules/RefiPolicy.aum`, compiled by the stock `austral` front-end and
   executed in-process by australVM's Cranelift bridge. Authorization, amount
   bounds, denomination checks, deposit shape, daily caps and the post-op audit
   gate live in that module, not in the HTTP layer. The host supplies the
   mechanism as `rs_*` primitives (`src/engine.rs`), gated by
   `module.toml` — an ungranted primitive fails JIT compilation with UK-4001.
2. **The site is served by the same process.** A small HTTP/1.1 front
   (`src/server.rs`) serves the JSON API, the public site and a small typed
   content console (pages with a Markdown subset, draft/publish, revisions) —
   an EmDash-shaped CMS with no second runtime, no D1/R2, and no client-side
   JavaScript. The CSP is `default-src 'none'`.
3. **Proofs are archived, not asserted.** An operator submits an mm0
   development (spec + proofs) from the console; the exchange forwards it to
   `mm0web` (`unfer/mm0web`), which verifies it with `mm0-rs` and then `mm0-c`
   — a warning is a proof hole — and publishes the verified artifact to a
   Zenodo project deposit. The DOI that comes back is recorded in
   `proofs.json` and shown on `/proofs`, so a claim is a citation, not a
   promise — and an operator can mint a certificate whose provenance *is* that
   DOI (`POST /api/v1/proofs/<id>/mint` or the proof's console page), which the
   registry lists as `kind: attested`. One flow, two ledgers: the proof earns
   an archive, the archive backs a certificate.

```
HTTP ─ server.rs (sockets) ─ Service (src/service.rs)
                                      │ auth, rate limits, routes, content, proofs
                                      ├──────────────► mm0web (verify + Zenodo DOI)
                                      │
                                      ▼
                               Engine thread (src/engine.rs)
                                      │ Austral policy kernel (JIT)
                                      │ rs_* primitives → TalerExchange
                                      ▼
                        exchange.json (atomic, fsync) + accounts.json
```

## Status and honest limits

- **Sandbox settlement.** The bank seam is `SimulatedWireGateway`: a peg-in is
  a wire the service books and confirms itself; a peg-out is a reservation an
  operator confirms. No fiat moves. The running site says so on every page.
- **Custodial sandbox wallets.** Account ed25519 keys are generated and kept
  in `accounts.json` (0600). A non-custodial deployment would take signed
  transactions from clients instead; that is a deliberate v1 boundary, not an
  oversight.
- **Single node.** The exchange's certificate mirror is local. Replicating it
  onto the existing `unfer_consensus` network (`--features network`) is the
  next step; the op log is already the replayable input.
- **Not a regulated financial service.** A live deployment needs VASP
  registration, custody and commodity-backed e-coin advice; none of that is
  implied by this code.
- **The page renderer is a subset** (headings, bullets, bold, code, http(s)
  links). It escapes everything before applying tags; there is no raw-HTML
  escape hatch.

## Build and run

```sh
# from australVM/refi
cargo build --release

# first run creates state/, treasury.key, admin.token (all 0600)
./target/release/refi_service \
  --listen 127.0.0.1:8080 \
  --state-dir ./state \
  --base-url http://127.0.0.1:8080 \
  --open-registration \
  --check            # validates state + compiles the policy kernel, prints status

./target/release/refi_service --listen 127.0.0.1:8080 --state-dir ./state \
  --open-registration
```

The policy kernel is compiled to `state/RefiPolicy.cps` on first start (via
`../austral compile ... --emit-cps`; set `AUSTRAL_BIN` if the front-end lives
elsewhere, and keep `libaustral_cranelift_bridge.so` next to it). After that the
CPS artifact is reused, so the OCaml front-end is not needed at runtime.

Prebuild the artifact explicitly:

```sh
AUSTRAL_BIN=../austral ../austral compile modules/RefiPolicy.aum \
  --use-cps-jit --allow-all --target-type=tc --emit-cps=state/RefiPolicy.cps
```

## API

Public: `GET /api/v1/status`, `/registry`, `/denominations`, `/ops?limit=`.
Auth: `Authorization: Bearer <token>`.

| method | path | who |
|---|---|---|
| POST | `/api/v1/accounts` | anyone (when registration is open) |
| GET | `/api/v1/me` | account |
| POST | `/api/v1/reserves` | account |
| POST | `/api/v1/reserves/{id}/peg-in` | reserve owner |
| POST | `/api/v1/reserves/{id}/withdraw` | reserve owner |
| POST | `/api/v1/deposits` | coin owner |
| POST | `/api/v1/transfers` | coin owner |
| POST | `/api/v1/peg-outs` | merchant |
| POST | `/api/v1/peg-outs/{wire}/confirm` | admin |
| POST | `/api/v1/peg-outs/{wire}/cancel` | merchant or admin |
| GET | `/api/v1/audit` | admin |
| POST | `/api/v1/admin/accounts` | admin |
| POST | `/api/v1/admin/denominations` | admin |
| POST | `/api/v1/admin/mint` | admin |

Errors are uniform: `{"error":{"code":"UK-7102","name":"…","message":"…"}}`.
`/healthz` fails (503) if the fiat conservation audit breaks; `/metrics`
serves Prometheus text.

End-to-end sandbox flow:

```sh
BASE=http://127.0.0.1:8080
TOKEN=$(curl -s $BASE/api/v1/accounts -d '{"name":"alice"}' | jq -r .token)
RES=$(curl -s $BASE/api/v1/reserves -H "Authorization: Bearer $TOKEN" | jq -r .reserve_id)
curl -s $BASE/api/v1/reserves/$RES/peg-in  -H "Authorization: Bearer $TOKEN" -d '{"amount":1000}'
COIN=$(curl -s $BASE/api/v1/reserves/$RES/withdraw -H "Authorization: Bearer $TOKEN" -d '{"amount":500}' | jq -r .coin_id)
curl -s $BASE/api/v1/deposits -H "Authorization: Bearer $TOKEN" \
  -d "{\"coins\":[\"$COIN\"],\"merchant\":\"$MERCHANT_DID\"}"
```

Carbon certificates: `POST /api/v1/admin/mint` with
`{"amount":10,"owner":"did:unfer:…","source":"unfccc:vc:<order>"}`; the
provenance prefix is enforced. `GET /api/v1/registry` joins every mint in the
op log against the live unspent set, so `status` is `unspent` or `spent`.

## Site and admin

`/` home, `/registry`, `/pages/<slug>`, `/api` reference. The console is at
`/admin`: sign in with the contents of `state/admin.token`; manage pages,
issue denominations, create accounts (the token is shown once). Forms carry
the admin token as a hidden field (double-submit against the `HttpOnly`,
`SameSite=Strict` session cookie), and every stored page body is escaped
before the Markdown subset is applied.

## Tests

```sh
cargo test          # unit + HTTP-shaped end-to-end through the real policy
```

The integration tests compile the `.aum` kernel with the stock front-end and
fail-soft with a `SKIP:` line when `../austral` is absent. They cover the full
sandbox lifecycle, authorization and ownership denials, unsupported
denominations, double deposits, wire cancel/confirm, conservation, daily caps,
provenance validation, certificate transfer conservation, restart durability,
admin session/forms and HTML injection.

## Deploy notes

See `deploy/README.md` for the systemd unit, state layout and the Oracle Cloud
path (TLS-terminating front + security list + host firewall + a certificate).
