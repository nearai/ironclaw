# Telegram Linked Device — Executive Overview

**Status:** Proposal, revision 6 (re-verified against `origin/main` @ `0f771d4915`; **sign-off still withheld** — PROPOSAL §14.3–14.4) · **Authored against:** `boom-python` @ 2026-08-07
**Documents:** this overview · [PROPOSAL.md](PROPOSAL.md) (specification and
per-crate change inventory) · [PLAN.md](PLAN.md) (execution order and PR slicing)
· [CHECKLIST.md](CHECKLIST.md) (definition of done)
· [ADR-device-link-auth-hook.md](ADR-device-link-auth-hook.md) (the auth-hook decision and what it costs)
· [AUTO-CHANNEL-IDENTITY.md](AUTO-CHANNEL-IDENTITY.md) (approved follow-on:
device linking becomes the Telegram bot channel identity ceremony)
**Supersedes for Telegram:** [`../2026-08-07-linked-accounts-design.md`](../2026-08-07-linked-accounts-design.md),
which remains the reference for WhatsApp/Signal and the deferred inbound/mirror
work.

> **Revision history.** Two adversarial reviews rejected revision 1; revision 3
> then verified every deferred vendor claim against source rather than punting
> it to a spike. Corrected across those passes: the footprint (12 crates, not
> 9); custody placement (`ironclaw_auth`, after revision 1 put it in composition
> and revision 2 over-corrected to a new crate); `send_message` returning
> `id == 0`, which revision 1 mapped in a way that would make the agent
> double-send; the adapter contract, which could not express its own phone
> fallback; ghost-device leaks after QR acceptance; and a factual error about
> Telegram Premium search. Details in
> [PROPOSAL §14](PROPOSAL.md#14-review-log).

---

## 1. What ships

A user links their **personal Telegram account** to IronClaw as a real linked
device — what Telegram Desktop is, visible and revocable in Telegram's
*Settings → Devices*. The agent then reads their conversations and acts as them
through model-callable tools bound to the standard messaging operations.

**We keep no message content.** Telegram is a cloud messenger: history lives on
Telegram's servers and a linked device fetches it on demand. Every read is a
live call.

*Precisely* what we do persist is the session credential — auth keys, DC
routing, an update cursor, **and grammers' peer cache**, which is a partial map
of the accounts and chats the session has seen. That is a contact-graph
fragment, not message content, and product copy must not overclaim.

**v1 is pull-shaped.** The agent acts when asked. It does not consume the live
update stream for messages, so it cannot yet "notice a message and react" — a
deliberate, sequenced follow-on (§6).

## 2. Why this shape

Three findings collapsed the design from the earlier network-generic sketch:

1. **Telegram needs no message store.** `iter_messages`, `search_messages`,
   `search_all_messages`, `iter_dialogs`, and `get_messages_by_id` are live
   RPCs. The mirror crate, FTS plane, retention policy, and dual-backend parity
   suite all leave v1.
2. **No message-update consumption means no ingress work.** The hardest seam in
   the earlier design — minting session-sourced verified-inbound evidence past a
   23-test ratchet — is not on this path.
3. **The session lives in the existing package.** A `ToolAdapter` is built once
   per registry generation and reused across every call, so it can hold a
   per-user connection pool (keyed by credential account, not `scope.user_id` —
   §3.3).

## 3. System architecture

```text
 ┌────────────────────────────────────────────────────────────────────────┐
 │ WebUI · AuthDeviceLinkCard (QR ⇄ phone fallback, 2FA, status poll)     │
 └───────────────┬────────────────────────────────────────────────────────┘
 ┌───────────────▼────────────────────────────────────────────────────────┐
 │ ironclaw_auth — device_link method                                     │
 │   owns: step state machine, revision CAS, TTLs, credential lifecycle   │
 │   owns NO vendor mechanics; calls out through DeviceLinkDriver         │
 └───────────────┬────────────────────────────────────────────────────────┘
 ┌───────────────▼────────────────────────────────────────────────────────┐
 │ ironclaw_extension_host — the glue                                     │
 │   implements DeviceLinkDriver by resolving extension → bound adapter   │
 │   scopes LinkedSessionPort per (extension, user); applies rate limits  │
 └───────────────┬────────────────────────────────────────────────────────┘
 ┌───────────────▼────────────────────────────────────────────────────────┐
 │ crates/extensions/packages/telegram   ← ALL vendor code, and the only  │
 │                                          place a decrypted key resides │
 │   PendingLinks   flow_id → parked Client + runner (polls for scan)     │
 │   SessionPool    LinkedAccountRef+revision → Client (cache; shared)    │
 │   IronclawSession   impl grammers Session + DC validation (only seam)  │
 │   TelegramLinkAdapter  (DeviceLinkAdapter)                             │
 │   TelegramToolAdapter  (ToolAdapter — standard ops, live calls)        │
 │   TelegramChannelAdapter  (unchanged bot channel)                      │
 └───────────────┬────────────────────────────────┬───────────────────────┘
 ┌───────────────▼──────────────────┐  ┌──────────▼───────────────────────┐
 │ ironclaw_auth + ironclaw_secrets │  │ Telegram datacenters             │
 │ CredentialAccount + encrypted    │  │ raw MTProto — declared carve-out │
 │ blob behind access_secret (+CAS) │  │ (§4.4)                           │
 └──────────────────────────────────┘  └──────────────────────────────────┘
```

**Security posture, stated accurately** (revision 1's summary got this wrong):
custody is host-side and encrypted at rest, but the package **does** hold the
decrypted session key in memory while connected, and the user's 2FA password
passes through package code during linking. That is unavoidable — grammers must
have the key to speak the protocol — and it is the core concession this design
makes. See §4.4 and PROPOSAL §3.4.

## 4. The four decisions

**4.1 Reads are live; nothing is mirrored.** Telegram answers history and search
itself.

**4.2 Device-link is an auth method with a narrow adapter hook.** Every existing
method is data the host executes; MTProto login is a protocol handshake that no
descriptor can express. The extension-runtime spec named this exact revisit
trigger — *"a vendor defeats the descriptor (add a narrow hook)"* — and the auth
crate's own charter requires **an ADR** for it, which PR 2 writes. The hook also
revokes a real security claim ("no third-party code executes inside an auth
flow"); that revocation is argued, not assumed.

**4.3 The session lives in the package; custody extends `ironclaw_auth`.** The
pool is a cache destroyed on every reactivation. A linked account *is* a
`CredentialAccount` whose `access_secret` points at the session blob; the only
genuinely new surface is a compare-and-swap write path for a mutable binary
secret, because a clobbered auth key silently kills the link.

**4.4 The raw MTProto socket is a declared carve-out.** No mechanical gate blocks
it, but three written charters do, and it must be argued with dated amendments
on the `mem0` model.

## 5. Footprint

**Seventeen crates modified. No new crates.** The count has moved five times: 9 → 12 → 16 → 18 → 17, each correction from an audit
that actually ran the greps. Revision 4 adds `ironclaw_assistant` (two
compile-breaking exhaustive matches), `ironclaw_secrets` (the CAS path is a
substrate change), and the decorator/impl fan-out those imply. Revision 2's
new-crate detour was withdrawn — `CredentialAccount` already models custody
(PROPOSAL §3.3).

| Crate | Change | Size |
| --- | --- | --- |
| `extensions/packages/telegram` | **All vendor code** — session, pool, login, tools | Large |
| `product/ironclaw_webui` (+ frontend) | Device-link card, wire fields | Medium |
| `domains/ironclaw_auth` | `device_link` method, step machine, driver port, CAS blob write, ADR | Medium |
| `extensions/ironclaw_extension_host` | `DeviceLinkDriver` impl, bindings, `check_binding` | Medium |
| `contracts/ironclaw_extension_contracts` | Adapter trait, recipe variant, prompt kind | Small |
| `app/ironclaw_composition` | Wiring only | Small |
| `contracts/ironclaw_host_api` | Setup variant **+ the whole §6.2 schema graduation** | Medium |
| `contracts/ironclaw_product_contracts` | Lifecycle credential-setup variant | Tiny |
| `extensions/ironclaw_extension_registry` | Recipe projection arms | Tiny |
| `extensions/ironclaw_extension_manager` | Exhaustive-match arm | Tiny |
| `product/ironclaw_assistant` | Credential-setup + auth-challenge exhaustive matches (**compile-breaking, twice**) | Small |
| `substrates/ironclaw_secrets` | The CAS write path is a substrate change, not only an auth one | Small |
| `kernel/ironclaw_host_runtime` | Secret-store decorator fan-out + the §6.2 output validator | Small |
| `extensions/ironclaw_extension_support` | `CredentialAccountService` impl sites must still compile | Tiny |
| `app/ironclaw_cli` | Entrypoint binds the new adapters | Tiny |
| `app/ironclaw_architecture_tests` | Size ceilings, location scans, the new lockfile gate | Small |
| `tests/` (`ironclaw_integration_tests`) | Harness profile, flow helpers, secret-store double | Medium |

No new runtime lane. No change to the dispatcher's logic, the sealed evidence
mint, the ingress router, or the process supervisor — the loop-tier enforcement
earlier revisions required was withdrawn once upstream made owner == actor
(PROPOSAL §3.3).

## 6. Explicitly not in v1

| Not built | Why | Revisit when |
| --- | --- | --- |
| Message-update consumption | Pull-shaped v1 needs no inbound path | Proactive behavior is wanted |
| Session-sourced inbound evidence (T2′ ADR) | Only needed with updates | Same trigger |
| Message mirror, FTS, retention | Telegram serves reads live | A network that cannot (WhatsApp, Signal) |
| Supervised session fleet | An adapter-held pool suffices when sessions only serve tool traffic | Sessions must outlive tool traffic |
| Media content | Metadata suffices for v1 tools | A tool needs media bytes |
| Secret chats | Device-local by protocol | Never |

## 7. Risks

1. **grammers is unaudited and runs in-process, and we are shipping anyway** — a
   recorded, accepted risk (PROPOSAL §11.1). The exposure is not just its crypto:
   an in-process dependency can reach every other user's key and the secrets
   master key. Revisit trigger is **N = 2 linked accounts**, not GA.
2. **Blast radius, stated fully.** A full account credential per linked user —
   read, write and delete across their entire history — **plus** the 2FA cloud
   password, which survives unlink and enables permanent takeover; **plus**
   impersonation to that user's entire contact graph, who never consented to any
   of this; **plus** fleet-wide correlation, since one process holds every
   linked user's key at once.
3. **Read content is attacker-controlled.** Any Telegram user on earth can DM a
   personal account, and unsolicited DMs reach the agent through dialog and
   history reads. See PROPOSAL §6.4.
4. **QR login uses raw TL**, explicitly outside the crate's semver guarantee.
   Exact-pin `=0.10.0`; isolate raw TL to one module.
5. **The `=0.10.0` pin is a security control.** DC-address validation works only
   because 0.10.0 never persists server-pushed addresses; the upstream fix for
   that lands after this release. Re-verify validation at every upgrade.
6. **Ghost devices.** A QR scan authorizes the device immediately; any abort
   after that leaves a live authorization IronClaw has forgotten. Mitigated by
   logout-on-abort, with a residual crash window that must be disclosed.
7. **ToS exposure.** Personal-account automation is a grey area; bans are a
   support liability.
8. **Per-session cost.** Each live session is one runner task plus sockets;
   bound and idle-evict from day one.

## 8. Where to start

[PLAN.md](PLAN.md) sequences eight PRs. There is no spike step: the vendor
mechanics were verified directly against the grammers 0.10.0 sources and the
reference QR implementations, and are recorded as decided fact in
[PROPOSAL §14.1](PROPOSAL.md#141-revision-3--claims-verified-against-source-2026-08-07).
