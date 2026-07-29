# Alpaca sidecar

Wraps Ledger's Alpaca `CoinModuleApi` as a local service the IronClaw Rust
backend calls (attested-signing Phase E).

## What it is authoritative for: nothing

| Decision | Authority | This sidecar |
|---|---|---|
| Canonical signing bytes, render, `ApprovedTxHash` | **Rust** (`ironclaw_attestation`) | proposes a crafted tx; if Rust cannot decode it, the raise fails closed |
| Signer identity | **Rust** (gate-bound `SigningContext`) | never consulted |
| Grant claim / one-shot | **Rust** (sealed-grant CAS) | never consulted |
| Bytes handed to `combine` | **Rust**, reconstructed from the binding | mechanically attaches a signature |
| Broadcast admission | **Rust** idempotency ledger CAS | executes the RPC submit |
| Fees, balances, chain height | sidecar | advisory only |

A compromised sidecar can propose a malicious transaction — and the human
clear-signing the *Rust-derived* render on the device is what catches it. It
holds no keys, sees no grants, and cannot alter the bytes the device signs.

## Transport

Unix domain socket in a `0700` directory, plus a per-boot token the Rust parent
generates and passes on stdin. Every request must carry the token. No inbound
port on any external interface, ever. Localhost TCP is the Windows/dev fallback
with the same token requirement.

## The stdin link

Stdin carries two things over one pipe, and the parent keeps its write end open
for the child's whole life:

1. **The token**, as the first newline-terminated line — stdin rather than argv
   (visible in `ps`) or the environment (visible in a crash dump).
2. **Liveness.** EOF means the parent is gone, by clean exit, panic, or SIGKILL
   alike. It is the only signal that survives a parent the OS killed without
   warning, and it is why the token is a *line*: reading to EOF would spend the
   signal at startup. On EOF the sidecar unlinks its socket and exits, so it can
   never outlive the process that vouched for it and sit holding the signing
   socket as an orphan.

## Running it

The Rust parent spawns and supervises it (`ATTESTED_ALPACA_SIDECAR=managed`).
Standalone, for development — note the FIFO rather than a here-string, so the
pipe stays open and the sidecar does not immediately see EOF and exit:

```sh
mkfifo /tmp/alpaca.stdin
ALPACA_SOCKET_PATH=/tmp/alpaca.sock \
  node --experimental-strip-types src/server.ts < /tmp/alpaca.stdin &
exec 3> /tmp/alpaca.stdin   # holding fd 3 open holds the sidecar up
echo "$TOKEN" >&3
```
