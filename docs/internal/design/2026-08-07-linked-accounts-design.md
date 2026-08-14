# Linked Accounts — personal-account sessions as a generic capability

**Status:** Design proposal (not yet approved). **Date:** 2026-08-07.
**Scope:** Let a user link their personal messaging account (Telegram first,
then WhatsApp, Signal) so IronClaw can read their conversations and act as
them, with model-callable tools bound to the standard messaging operations.
**Prior research:** external-capability study 2026-08-07 (Telegram Business vs
MTProto). This document assumes the linked-device direction has been chosen and
specifies what must change.

---

## 1. The shape of the answer

A **linked account** is a long-lived, authenticated client session that
IronClaw holds *as the user* — architecturally a linked device, exactly like
Telegram Desktop or WhatsApp Web. It is not a channel and not a bot.

The design adds **one generic capability with six seams**, plus **one small
crate per network**. Everything vendor-specific lives in the per-network
package; every seam below is vendor-blind, which is what makes WhatsApp and
Signal additive rather than a second build.

```
[auth.<vendor>] method = "device_link"      ← seam 2: how a user links
        │  (recipe DATA: steps, endpoints, pointers)
        ▼
LinkedAccountRecord  (session blob, CAS-versioned, envelope-encrypted)
        │                                    ← seam 5: custody
        ▼
SessionFleet — ProcessKind::ExtensionDefined("network_session")
        │  one supervised, never-returning executor per (user, network)
        │                                    ← seam 3: lifecycle
        ├──► outbound + reads: first-party capability handlers resolve the
        │    live session  →  standard messaging ops   ← seam 6: tools
        │
        └──► inbound: session events → verified-inbound evidence →
             the EXISTING GenericChannelInboundSink path  ← seam 4: ingress
                     │                        [DEFERRED — not needed for Telegram v1]
                     ▼
             MessageMirror (conversations, messages, users, FTS)
                     [DEFERRED — per-network, only where reads can't be live]
```

### Read model: live-first, mirror only where the network forces it

**This is the correction that sizes the project.** Telegram is a cloud
messenger: the servers hold full history and any linked device fetches it on
demand. `messages.getHistory` is among the cheapest MTProto calls, and per-chat
`messages.search` runs server-side. **For Telegram we store no messages at
all** — every read op is a live call, exactly as Telegram Desktop works.

WhatsApp and Signal are structurally different: end-to-end encrypted, with no
server-side plaintext to query. A companion device receives a **one-time
history bundle at link time** (WhatsApp: recent chats, transferred from the
primary; Signal: opt-in transfer plus the last 45 days of media) and after that
only live messages. There is no "ask the server for older" call. If reads must
outlive what the device happens to hold, that network must persist.

So read source is a **per-network capability declaration**, not a global
architecture decision. Verified 2026-08-07 against the protocol/library
surfaces:

| Network | Can a client re-fetch history on demand? | Storage required? |
| --- | --- | --- |
| **Slack** | Yes — `conversations.history` / `search.messages`, server-side | **No** (already proven: our Slack tools read live, no mirror) |
| **Telegram** | Yes — `messages.getHistory` / `messages.search`, server-side, full history | **No** |
| **Discord** | Yes for bots, server-side — personal-account form is out of scope | **No** |
| **WhatsApp** | Partially — `BuildHistorySyncRequest` requests history **from the user's primary phone** (not the server), anchored on a known message, ~50 at a time, and is documented as unreliable for companion devices | **Not strictly; practically yes** for deep history |
| **Signal** | **No** — the transfer happens only at link time; no post-link history request exists | **Yes**, for anything before the session |

Details behind the two hard cases:

- **WhatsApp.** `Client.BuildHistorySyncRequest(lastKnownMessageInfo, count)`
  builds a peer message "to request additional history from the user's primary
  device", sent via `SendPeerMessage`; the reply arrives as
  `events.HistorySync` of type `ON_DEMAND` containing `count` messages
  immediately before the anchor. So a companion *can* walk backwards without
  persisting — but it needs an anchor message it already holds, the phone must
  be online, the recommended page size is 50, and whatsmeow's own issue tracker
  (and Baileys') record WhatsApp **silently dropping on-demand requests from
  companion devices**. Treat it as best-effort, not a query API.
- **Signal.** The link-time transfer (chats plus the last 45 days of media) is
  a one-shot at pairing; Signal documents no mechanism to request older history
  afterwards, no client library exposes one, and the servers hold nothing after
  delivery (mailbox retention is for *undelivered* messages only). **The
  link-time bundle is therefore irreversible: if we discard it, that history is
  gone for good.**

  The same delete-on-delivery rule applies to the *ongoing* stream, which is
  the sharper operational consequence: once our linked device receives and acks
  a message, the server drops it and the user's phone will not hand it back. So
  on Signal, "don't persist" does not merely mean "no back-catalogue" — it
  means **every process restart permanently erases everything received since
  the last one.** On a server product where deploys are routine, the effective
  memory window is "since the last deploy", which is too fragile to build a
  read feature on. Signal support without storage is therefore a *send-and-
  react-live* product, not a searchable one.

  Worth stating plainly for the privacy discussion: a linked device that stores
  messages locally **is** Signal's own model — Signal Desktop keeps an
  encrypted local database for exactly this reason. The real question is not
  protocol deviation but *where that device lives*: ours runs on a server
  rather than the user's laptop. That is a hosting and consent question, not a
  protocol one, and it should be argued on those terms.

`SessionAdapter` declares which standard read ops it serves live. The mirror is
an **optional component built when the first network needs it** — which, on
this evidence, means Signal, not WhatsApp.

### The load-bearing insight

Most of this machinery already exists and is unused. The work is
disproportionately **wiring and hardening**, not invention:

| Capability | Status today |
| --- | --- |
| Never-returning supervised executor with lease renewal | **Exists** — `ProcessSupervisor` polls executors as futures; heartbeat renews a 90 s lease every 30 s |
| A free process kind | **Exists, zero incumbents** — `ProcessKind::ExtensionDefined(String)` |
| "Auth rejected → park until credentials change" | **Exists, unused** — `ProcessSuspensionKind::ExtensionDefined` + `ProcessSuspension.credential_requirements` |
| Multi-node claim safety | **Exists** — journal claim is a transactional CAS; restart-resume is free via `relinquish → Queued` |
| QR render + countdown + status polling UI | **Exists** — `pairing-web-code-panel.tsx` |
| Full-text search across both backends | **Exists, first-class** — `IndexKind::Fts` (libSQL fts5 / Postgres GIN+tsquery) |
| Transport-neutral inbound path | **Exists** — everything below `InboundAdmission` is already transport-free |
| Standard messaging op vocabulary + host-enforced output evidence | **Exists** — 16 core ops, schema-validated at one choke point |

What genuinely does not exist: a **vendor-poll primitive** in auth, a
**multi-round auth flow**, a **session-sourced evidence mint**, a **mutable
credential custody model**, and a **message mirror**.

---

## 2. Seam 1 — Contracts vocabulary

New, vendor-neutral, no execution. Placement passes the contracts admission
test (boundary concept, ≥2 consumers, no framework deps).

**`crates/contracts/ironclaw_host_api/src/session.rs`** (new)

- `NetworkId` — validated newtype (`.claude/rules/types.md` template).
- `SessionKey { tenant_id, user_id, network }`.
- `SessionState { Connecting, Connected, AuthRequired, BackingOff, Stopped }`.
- `SessionRegistry` — `resolve(&SessionKey) -> Arc<dyn SessionHandle>`, `state(&SessionKey)`.
- `SessionHandle` — `call(SessionOperation) -> Value`.
- `SessionResolveError` — **must** carry
  `AuthRequired { credential_requirements: Vec<RuntimeCredentialAuthRequirement> }`
  so the adapter can mint `DispatchError::AuthRequired` verbatim and the
  existing re-auth gate fires unchanged.

**`crates/contracts/ironclaw_extension_contracts/src/session_adapter.rs`** (new)

- `SessionAdapter::connect(&SessionContext, &dyn RestrictedEgress) -> Box<dyn SessionConnection>`.
- `SessionConnection` — `next_event() -> Option<InboundOutcome>` (reuse the
  channel outcome type), `call(SessionOperation)`, `close()` for graceful
  teardown.

**`crates/contracts/ironclaw_extension_contracts/src/linked_session.rs`** (new)

- `LinkedSessionPort` — `load() -> Option<SessionSnapshot>`,
  `store(expected: SessionVersion, blob) -> SessionVersion`.
  The package receives *this*, never `SecretStorePort` — the same inversion
  `RestrictedEgress` already uses for bot tokens.

**Second-implementation test (architecture review checklist #1):** every trait
here has ≥2 real implementations at Phase 9 (Telegram + WhatsApp), and
`SessionRegistry` is a dependency-inversion port whose impl must live up-layer.
`LinkedSessionPort` additionally enforces a privilege boundary — it keeps
`SecretStorePort` out of `products`-layer package crates.

Raise `reborn_contracts_crates_carry_a_checked_size_ceiling` in the same PR.

---

## 3. Seam 2 — Device-link auth

### Why it is an auth method, not a channel connection strategy

A personal session is a **credential**, not a channel. `[channel.connection]`
presupposes an installation, an inbound webhook, and an identity binding; a
linked account may exist with no channel surface at all.

**Do not unpin `ChannelConnectStrategy::QrCode`.** That variant is unreachable
dead vocabulary on the product side with no manifest producer, and its
lifecycle copy ("open the app, get the pairing code, and paste it here") is
wrong for a vendor-minted QR — nothing is pasted into IronClaw. Filing a
separate cleanup to delete `QrCode`/`InboundProofCode` is recommended.

### The recipe

`VendorAuthRecipe` gains `DeviceLink(Box<DeviceLinkRecipe>)`
(`crates/contracts/ironclaw_extension_contracts/src/recipe.rs:193`). Step
order, endpoints, JSON pointers, and copy are **recipe data** — the auth engine
charter forbids vendor branches.

**Eight compile-error seams** (exhaustive matches) and **six silent seams**
(`if let` / `_ =>`) must be updated. The silent ones are the dangerous ones:

- `recipe.rs:241` `compatible_for_shared_vendor` has a `_ => false` arm — two
  device-link recipes for one vendor would silently become "incompatible" and
  fail activation with `VendorRecipeConflict`. **Add the matching arm.**
- `recipe.rs:226` `keepalive_idle_threshold` **must return `None`** — otherwise
  the keepalive sweep drives `refresh_token` against a vendor with no token
  endpoint.
- `auth_engine_assembly.rs:217` warns on non-OAuth recipes; add an explicit
  "no static client credentials for this method" branch.

**Rollout hazard:** `RuntimeCredentialAccountSetup` carries
`#[serde(other)] Retired`. An older binary reading a newer persisted
`device_link` requirement silently folds it to `Retired`, which
`product_prompt.rs` treats as "no serviceable challenge". **Ship the enum
variant before any producer.**

### Multi-round flow state

`AuthFlowRecord.challenge` is written exactly once today; there is no
re-challenge path anywhere. Add:

- `AuthFlowStepState { index, kind, revision, vendor_cursor: Option<SecretHandle>, step_expires_at, last_polled_at, poll_attempts }`
  on `AuthFlowRecord` as `Option`, `#[serde(default)]` (every persisted record
  predates it).
- `AuthFlowStatus::AwaitingVendor`. Note `account_state.rs:96` has a `_` arm
  that would silently project it as `Disconnected` — add the explicit
  `Authenticating` arm.
- `AuthChallenge::DeviceLinkStep { interaction_id, provider, step, display, expires_at, revision }`
  with `DeviceLinkDisplay { Qr | Code | Waiting | Secret }`.
- `AuthFlowManager::advance_flow_step(AdvanceFlowStepInput { flow_id, from_revision, transition })`
  — revision-CAS under the existing per-flow lock, so a duplicated vendor poll
  is idempotent (loser gets `Ok` with the already-advanced record).

**Two clocks.** A Telegram QR rotates every ~30 s; a 2FA password takes a
minute to type. The *step* clock expiring re-mints the step; only the *flow*
clock terminalizes, extendable up to a declared ceiling.

**The second factor reuses the existing interaction store** —
`SecretSubmitRequest` already carries exactly one secret. Add
`flow_binding: Option<FlowStepBinding>` and a `submit_flow_step_secret` method
returning a non-`Serialize` `FlowStepSecretClaim`.

### The vendor poller, and the severance trap

`crates/domains/ironclaw_auth/tests/module_charter.rs` enforces that
`src/engine/**` contains no `product_auth::` and `src/product_auth/**` contains
no `engine::` — as **bare substring probes**. Any module path ending in
`engine::` in product-auth code fails the gate.

Resolution is existing precedent: declare `DeviceLinkProviderClient` in
`src/provider.rs` (crate-root vocabulary), implement it on `AuthEngine` in the
new `src/engine/device_link.rs`, and have product-auth hold
`Arc<dyn DeviceLinkProviderClient>` — exactly how `AuthProviderClient` works
today. Where the engine type must be named, spell it `crate::AuthEngine`.

**Every new `src/**/*.rs` file must be added to exactly one row of the
sub-owner map in `crates/domains/ironclaw_auth/AGENTS.md` in the same commit**,
or `module_charter` fails. Do not reflow the table.

**Drive the poll on read for v1.** The frontend already polls status every 2 s;
have the status route drive one vendor poll per request, rate-limited by the
route policy and by `last_polled_at`. This needs **zero** new worker lifecycle.
A background sweep modelled on `engine/keepalive.rs` (settings → leader lock →
handle → `tick_once`) is a strictly additive upgrade if a closed browser tab
stalling a link proves unacceptable.

### Frontend

Extract the presentation half of `pairing-web-code-panel.tsx` into
`components/link-payload-panel.tsx` (QR render, countdown, copy, renew) and
compose it from both the existing pairing panel and a new
`auth-device-link-card.tsx`. Reuse `AuthGateShell` and `AuthTokenCard`
(the 2FA step) unchanged. Add `DeviceLinkPromptView` to `AuthPromptView` /
`AuthPromptContextView` as additive optional fields, and a
`challengeKind === "device_link"` branch in the chat card selector.

**Open decision — QR payload custody.** A `tg://login?token=…` is
account-takeover material. Either (a) carry it in the serializable challenge as
a bounded, validated `DeviceLinkPayload` newtype (precedent:
`AuthChallenge::OAuthUrl` already embeds `state`), or (b) park it behind a
`SecretHandle` and lease it through a dedicated owner-authenticated route.
Decide explicitly; do not let it default.

---

## 4. Seam 3 — The session fleet

### Placement

`ProcessKind::ExtensionDefined("network_session")` on the existing
`ProcessSupervisor`. **No enum change.** The `turn_scheduler.rs` pattern
(293 lines: config newtype, executor adapter, wake notifier, handle) is the
template to copy.

Own the fleet in a module under `crates/extensions/ironclaw_extension_host`
for v1 — it must start and stop with extension activation, and
`GenericChannelHostAssembly`'s snapshot-reconcile loop (holding a `Weak<Self>`
so drop ends it) is already that shape. Promote to
`crates/loop/ironclaw_session_fleet` only if the module outgrows its host.

**Composition gets ≤ ~40 LOC** — construct, store on `RebornRuntime`, drain,
set a readiness bool. The budget has ~150 LOC and ~15 `Arc<dyn>` of headroom.

### Six changes the supervisor needs

1. **Its own config.** `max_concurrent_processes` defaults to **4**, and a
   session holds a semaphore permit for its entire life. Either raise it for
   this supervisor or add a `long_lived` mode that skips the semaphore —
   permits are the wrong shape for infinite-duration work.
2. **Graceful teardown.** `shutdown_supervisor` calls `JoinSet::shutdown()`,
   which *aborts* — no chance to send a logout or close frame. Add cooperative
   cancellation (the crate already owns `ProcessCancellationRegistry`) plus a
   bounded grace period before abort.
3. **Startup jitter.** The claim path grabs up to 128 permits and fires with
   zero jitter — 2000 queued sessions become 2000 simultaneous handshakes on
   restart. Copy the keepalive sweep's 300 s startup spread.
4. **Backoff that survives restart.** `ProcessFailureRecovery` has only
   `Terminal` and `RedriveIfCheckpointless`. Add `RedriveAfter { not_before }`
   (or a queue-row `not_before`), or in-process backoff state resets on every
   deploy.
5. **Per-kind reclaim budget.** `MAX_CRASH_RECOVERY_RECLAIMS = 3` means three
   network blips permanently fail a session. Either make it per-`ProcessKind`
   or have the executor never return `Err` for transient faults and loop
   internally with backoff (the lease stays alive via heartbeats).
6. **Auth-rejection parking.** On auth failure, `suspend_process` with
   `ProcessSuspensionKind::ExtensionDefined` carrying `credential_requirements`
   — `Suspended` is not claimable, so it cannot hot-loop, and an explicit
   `resume_process` from the credential-update path is the only exit. This is
   precisely what `.claude/rules/lifecycle.md` demands and it already exists.

### Drain order

Insert `session_fleet.shutdown(timeout)` in `RebornRuntime::shutdown`
**between** the keepalive sweep and `turn_scheduler.shutdown()` — after
periodic workers stop generating work, before the turn scheduler stops, since a
session teardown may need to write through a turn path.

### In-process, not a sidecar (for v1)

`SandboxCommandTransport::run_command` is one-shot request/response with a
mandatory timeout; a session needs duplex framing. The sandbox lane has **no
production execution backend at all**, and its Docker test gate is wired to
nothing (#7081). A sidecar is a new contracts port, a new lane, a new IPC
protocol, and a new trust boundary — roughly a quarter of work before the first
message is sent.

Mitigate in-process blast radius instead: `catch_unwind` is already applied;
add a per-session task budget and a hard cap on decoded frame size; keep the
protocol library behind `SessionAdapter` so a later move out-of-process swaps
one impl rather than a rewrite.

**Revisit the sidecar when a network's client cannot be safely hosted
in-process** — which is exactly the Signal case (see §8).

---

## 5. Seam 4 — Session-sourced inbound (the ADR)

### What the wall actually is

Two facts define it. `evidence_mint_for_verification` returns `None` for any
channel without an HTTP-header-shaped secret, so the route registers nothing
and fails closed. And `TrustedInboundContext::from_verified_evidence…` demands
a `Verified` `ProtocolAuthEvidence`, with no other door into
`ProductInboundEnvelope`.

The seal is three-layer: a private `HostAuthSeal(())`, a `VerifiedInboundGrant`
whose only constructor is a provided trait method that compiles solely inside
`ironclaw_host_api`, and a `Deserialize` impl that rejects anything but
`failed`. **`impl ChannelIngressVerifier for X {}` alone confers minting
power**, and a 23-test ratchet asserts exactly one implementor, in
`ironclaw_extension_host`.

### Recommended shape

Keep the session path flowing through `InboundAdmission` →
`GenericChannelInboundSink` → `ChannelInboundProductSurface`, because
everything below `InboundAdmission` is **already transport-neutral** —
idempotency, conversation binding, command/approval/auth classification,
attachments, and turn submission all work unchanged. Then:

1. Add `AuthRequirement::HeldClientSession { network, session_ref }`.
2. Add `mark_client_session_verified(grant, network, subject)` and **register
   it in `CHANNEL_MINT_FNS`** — otherwise the census test silently stops
   governing it.
3. Add `VerifiedEvidenceMint::ClientSession { network }` so the *sole*
   `ChannelIngressVerifier` implementor stays sole and the ratchet is untouched
   in shape.
4. Add a **sibling** `ChannelSessionIngressDescriptor` rather than mutating
   `ChannelIngressDescriptor`/`ChannelIngressMethod`, and relax
   `inbound && ingress.is_none()` to accept either descriptor. Transport is
   implementation, never taxonomy — do not add a "kind" discriminator.
5. **Run an `ironclaw_safety` scan inside the mint path** for session-sourced
   text, so "this text passed the scan" is a property of the evidence. This
   copies the trusted-trigger discipline exactly: the scan moved to the type
   constructor precisely because living in one *implementation* meant swapping
   the submitter silently dropped the guard.
6. Extend `untrusted_ingress_paths_cannot_submit_host_trusted_inbound` with the
   new session symbols — extend the gate, never sidestep it.

### The trust argument for the ADR

The architecture proposal defines **T2** as "raw webhook → verified inbound"
and already has a **T1′** variant where public webhook routes skip host
authentication because a *different* proof applies. This ADR defines **T2′ —
held client session → verified inbound**: a sibling transition, not a widening.

| Webhook proves | Session proves |
| --- | --- |
| Shared-secret possession (HMAC / header) | Session-key possession established at device link |
| Replay window on a timestamp header | Transport sequencing + the durable idempotency ledger on `event_id` |
| Exactly one of ≤8 candidate installations verified | The session *is* the installation — no candidate ambiguity at all |
| Byte integrity of one body | Integrity of the whole authenticated connection |
| "The vendor sent this" | "We received this on a connection we hold" |

The affirmative argument: **no unauthenticated public HTTP surface exists at
all** on this path. Route enumeration, replay against `/webhooks/extensions/*`,
body-limit DoS, and rate-limit exhaustion do not apply.

The honest residual: the session sees *everything* the account sees, so the
"was this event for us?" filter moves from the vendor to us.
`ProductTriggerReason` classification becomes load-bearing security rather than
mere routing. State this in the ADR residuals.

Four crate guidance files say "do not add a third grant" or "do not widen"
(`ironclaw_host_api`, `ironclaw_extension_contracts`, `crates/extensions`,
`crates/domains`). The recommended shape adds **no third grant** — it adds a
third mint function behind the existing grant, held by the existing sole
implementor. Engage each file's sentence directly in the ADR anyway.

Claim or add a row in the extension-runtime "Deliberately not built" table —
that table is the sanctioned mechanism for "we said no; here is the trigger
that fired."

### Inbound messages are not turns

Mirrored personal messages feed the **mirror**, not the agent loop. Only
explicitly-addressed messages (a configured self-chat, or an explicit trigger)
should become turns. Getting this wrong turns every message the user receives
into an agent run.

---

## 6. Seam 5 — Custody and the mirror

### Session custody: split the key from the blob

`ironclaw_secrets` is the wrong home for the blob and the right home for the
key. Measured constraints: `SecretMaterial = SecretString` and decryption hard-
rejects non-UTF-8, so binary cannot round-trip; `put` uses
`CasExpectation::Any`, i.e. **last-writer-wins with no lost-update protection**
(two concurrent writers during a key rotation would silently clobber, and the
loser's session is dead); there is **no size ceiling anywhere**; `expires_at`
semantics do not fit a session that has validity rather than expiry; and the
`lease_once`/`consume` model is read-exactly-once while a session writes back
many times per connection. Every one of the twelve production `put` callers
today stores a small immutable-until-replaced token.

**Therefore:**

- **`ironclaw_secrets` holds a 32-byte data key** (hex, immutable-until-rotated)
  under a revision-suffixed handle — the `admincfg-r{revision}-{sha256}` shape.
- **New `crates/domains/ironclaw_linked_accounts`** (layer `substrates`) holds
  `LinkedAccountRecord` — the envelope-encrypted session blob, CAS-versioned via
  the shared bounded `cas_update` helper, with a **declared `MAX_SESSION_BYTES`**
  and a new AAD domain binding `(tenant, user, extension_id, link_id,
  link_revision)` so a rolled-back ciphertext fails decryption.

### `link_revision` — the missing concept

`.claude/rules/lifecycle.md` requires that auth rejection "stops reconnect
attempts **until the credential revision changes**." Grep confirms **no
`credential_revision` type exists anywhere in the tree** — the rule is prose
only. `LinkedAccountRecord.link_revision: u64`, bumped on every relink and key
rotation, is that concept; the fleet refuses to reconnect unless the observed
revision differs from the one it failed on. Introducing it is part of this
feature.

Unlink must revoke vendor-side **first**, then delete the blob and key, and
record a quarantine reason when revocation fails — copy
`SecretCleanupQuarantineReason::{RevokeFailed, …}`. Silently succeeding on a
failed logout leaves a live session on the vendor side.

### What we actually store (Telegram v1)

Exactly two things, and neither is message content:

1. **The session blob** — auth keys, datacenter routing, and the update state
   (`pts`/`seq`) needed to resume without gaps. This is a credential.
2. **Nothing else durable.** A bounded in-memory cache within a turn is fine and
   is not a store.

**One honest caveat that must reach the product copy:** anything the agent
*reads* enters the model context and therefore the turn transcript, which *is*
durable and is retained under the never-delete invariant. So the accurate claim
is "IronClaw does not keep a copy of your Telegram history — it reads it live —
but messages it actually reads become part of that conversation's transcript."
That is true of every tool result; it just matters more here.

### The mirror — deferred, and per-network when it lands

Everything below applies to the **first network that cannot serve reads live**
(WhatsApp), not to Telegram. It is specified here so the seam is designed
correctly, not because it is v1 work.

**New `crates/domains/ironclaw_message_mirror`** (layer `substrates`, deps
`ironclaw_filesystem` + `ironclaw_host_api` **only** — any `domains → domains`
edge trips the same-layer inventory ratchet).

It cannot live in any existing crate: `ironclaw_threads` owns the *agent*
transcript and its `MessageKind` has no room for third-party human messages;
`ironclaw_conversations` forbids raw content by charter and says outright "it is
not the transcript"; `extension_host`'s stores are FIFO-evicting and TTL-
expiring by design; projections "cannot link a durable writer"; and a
per-package mirror would violate the runtime's Addition test (adding WhatsApp
must require no generic source changes).

Records map **losslessly** onto the canonical messaging schemas —
`is_self` is non-`Option` (schema-required, never fabricated `true`),
`conversation_info.kind` is exactly `dm | group_dm | channel | other` with
`counterpart` required when `dm`, and `next_cursor` is a base64url encoding of
the fabric's `OrderedQueryCursor`.

Four index specs, all partition-key-first:
`mirror_conversation_activity_v1`, `mirror_message_sequence_v1`,
`mirror_message_thread_v1`, and `mirror_message_text_v1` as `IndexKind::Fts`.

**FTS needs no new story** — it is first-class on both backends (libSQL fts5,
Postgres GIN + `plainto_tsquery`), with a shared normalizer guaranteeing plain
user text can never become query syntax. Mount `/message-mirror` as a runtime
storage plane advertising `Capability::IndexFts` (the `/memory` precedent);
the `/tenants` plane does not advertise it.

**Open decision — `search_messages` relevance.** Neither backend exposes a
score (`bm25()` / `ts_rank_cd` are not surfaced). Either serve
`sort: "relevance"` as timestamp order and say so in the vendor addendum, or
add a ranked FTS variant to `ironclaw_filesystem` with its own parity tests.
Decide before the manifest binds `search_messages` — outputs are
`additionalProperties: false`, so there is no room to signal degraded ranking
except the `vendor` passthrough. Omit `total`; the fabric has no
count-without-fetch.

### Retention

The repo invariant is "LLM data is never deleted … mark with timestamps and
make filterable." It is enforced by convention only — 13 prose sites, zero
architecture tests, and `tests/CLAUDE.md` names the coverage gap explicitly.

- **Vendor deletes a message** → set `status = VendorDeleted`, stamp
  `vendor_deleted_at`, clear `text`/`attachments` in parity with transcript
  redaction, keep the row. `get_message` on it returns
  `messaging.unknown_message` — the vendor's answer, honestly reproduced,
  without destroying our record.
- **Vendor edits** → CAS-update in place, set `edited = true`. Never append a
  second row; `message_ref` must stay unique for `get_message` to be total.
- **Unlink** → delete the credential (session + key), **retain the mirror**,
  mark the link `Unlinked`. A separate, explicitly authorized "purge mirror"
  operation is the only path to deletion, with scope isolation tests.

### Media: metadata only in v1

Store `MirroredAttachmentRef { vendor_ref, mime, size_bytes, file_name, kind }`
— enough to render "[photo, 2.1 MB]" and to fetch on demand later — and **no
bytes**. Byte mirroring needs its own egress budget (the per-inbound-message
10-file/10 MiB budget cannot serve a history backfill), its own storage plane
(landed media currently goes to the model-listable project workspace), and — if
dedup matters — a content-addressed substrate that does not exist in the tree.
Vendor refs are short-lived by design, so metadata-only will return dead links
for old media; that is the accepted v1 trade.

---

## 7. Seam 6 — Outbound tools

**No new runtime lane.** A first-party capability handler resolves the caller's
live session: `FirstPartyCapabilityRequest` already carries `scope` and
`authenticated_actor_user_id`, which is exactly the `SessionKey`. The handler
holds an `Arc<dyn SessionRegistry>` captured at registration.

This costs **zero** changes to `RuntimeLane`, `RuntimeKind`,
`RuntimeLaneExecutor`, both tool resolvers, the dispatcher, and the policy
planner — versus ~16 files for a fifth lane. Every existing gate stays green.
The `FirstParty` lane is `#[serde(skip_deserializing)]`, i.e. host-assigned, so
a third-party manifest cannot request it — correct for personal-account
credentials.

The handler **must fail closed**: no live session → `DispatchError::Backend`;
invalid session → `DispatchError::AuthRequired` carrying
`credential_requirements`, so the existing park-and-re-auth gate fires
unchanged. Credential problems deliberately stay out of the `messaging.*`
taxonomy.

Tools bind standard ops in the package manifest with tool id exactly
`<extension_id>.<op_name>`, no schema refs (the host synthesizes them), and
`external_write` on every write. The six read ops are served by **one generic
mirror-backed executor** in `crates/extensions/ironclaw_extension_support` —
the chartered home for "executors that serve many packages at once", and the
one scan-exempt place vendor names may appear. That single executor is what
makes WhatsApp and Signal reads free.

---

## 8. Generalization: which networks, and how

The per-network work is one package crate implementing `SessionAdapter` plus a
manifest. Everything else is shared. But the networks are **not** equivalent:

| Network | Library | License / source | Linking | Verdict |
| --- | --- | --- | --- | --- |
| **Telegram** | `grammers-client` 0.10 | MIT/Apache-2.0, crates.io | QR (`tg://login`) + phone/2FA | **In-process. Ship first.** |
| **WhatsApp** | `whatsapp-rust` 0.7 | MIT, crates.io | QR + pair code + passkey | **In-process. Phase 9 proof.** |
| **Signal** | `presage` / `libsignal` | **AGPL-3.0, git-only** | Provisioning QR | **Blocked in-process** — see below |
| **Discord** | — | user tokens must be scraped | none | **Out of scope** — see below |

**Signal is blocked by our own policy, twice.** `deny.toml` allows permissive
licenses only (no AGPL) and sets `unknown-git = "deny"` with a tiny pinned
allowlist. Linking `presage` in-process would make the combined program AGPL
and fail `cargo-deny` in CI. Signal therefore requires the **sidecar
workstream** (§4) — which is also the license-correct posture, since an
arm's-length separate process with an IPC protocol is the standard way to avoid
derivative-work linkage. Treat Signal as the trigger that justifies building
the sidecar transport, not as a Phase 9 add-on.

**Discord has no linked-device concept for third parties.** Automating a user
account ("self-bots") is explicitly forbidden and results in account
termination, and the token must be extracted from browser storage. Do not build
it. Discord-shaped networks stay on the existing OAuth/bot path.

**A note on Telegram library risk:** `grammers` is light and permissive but
explicitly unaudited ("if security is critical, review `grammers-crypto` and
the authentication part of `grammers-mtproto`"), cuts releases rarely, and does
not guarantee trait compatibility across versions. The official TDLib is
audited but has documented unbounded memory growth across many accounts, which
disqualifies it for a fleet. Budget a security review of the crypto and
authentication paths before Telegram ships to real users; this is a genuine
risk being accepted, not one being dismissed.

**ToS posture.** Personal-account automation sits in a grey zone on every
network. Telegram's guidance is to pre-notify `recover@telegram.org` describing
the use case before first login; bans are triggered by datacenter-IP logins,
protocol fingerprint anomalies, and unusual call density. Beeper operates this
model commercially at scale, so it is viable — but user account bans are a
support liability the product must consciously accept, and the connect UX must
set that expectation.

---

## 9. Phasing

Each phase is independently reviewable and leaves the tree green.

### Track A — Telegram, pull-shaped (the v1)

The agent reads and acts **when asked**. No mirror, no live update consumption,
therefore **no ingress ADR on the critical path**.

| # | Phase | Ships | Est. |
| --- | --- | --- | --- |
| 0 | **Contracts** | Vocabulary crates, arch-test registrations. No behavior. (The T2′ ADR moves to Track B.) | 1–2 wk |
| 1 | **Session lifecycle on a fake adapter** | Connect → heartbeat → graceful close → restart resume → auth-suspend that does *not* resume on a timer. Pure kernel/loop, zero vendor. | 2 wk |
| 2 | **Device-link auth** | Recipe method, multi-round flow, poll-on-read, frontend card — end-to-end against a fake vendor. **The irreducible piece.** | 3–4 wk |
| 3 | **Custody** | `ironclaw_linked_accounts`, data-key split, `link_revision`, unlink + revocation quarantine. | 2 wk |
| 4 | **Telegram package** | `grammers` behind `SessionAdapter`; real link, connection, reconnect, update-state resume. | 3–4 wk |
| 5 | **Tools — reads and writes, all live** | First-party handlers + `SessionRegistry`; `send/edit/delete/whoami` gated, plus the six reads served by direct MTProto calls. | 3 wk |
| 6 | **Hardening** | Connection pooling + idle eviction, flood-wait backoff, fleet status view, load test. | 2 wk |

**Telegram to a trustworthy alpha: ~3–3.5 months.**

### Track B — what the second network adds

Only start this when WhatsApp (or proactive behavior) is actually wanted.

| Phase | Ships | Est. |
| --- | --- | --- |
| **Inbound ingress** | The T2′ mint + ADR, session ingress descriptor, safety-scan-at-mint, gate extension. | 2–3 wk |
| **Mirror + local reads** | `ironclaw_message_mirror`, FTS plane, retention policy, dual-backend parity suite. | 3–4 wk |
| **WhatsApp package** | Second network — the proof the abstraction holds. | 2–3 wk |

If Track A's seams were drawn right, WhatsApp itself is package-only work; if it
takes materially longer, that is the signal they were drawn wrong. Signal is a
separate sidecar program (§8), not an increment.

### Proactive behavior is the real trigger for Track B

Track A gives "read and act on my Telegram when I ask." It does **not** give
"notice a message and act." The moment that is wanted, the live update stream
must be consumed, which pulls in the ingress ADR — and consuming updates makes
a mirror cheap to add, since the data is already flowing. Sequence them
together rather than separately.

---

## 10. Scale ceiling, stated honestly

The extension-runtime overview documents — as an assumption, not an engineered
property — **one serving process per deployment**, with serving-leader leases
and fencing tokens explicitly not built. The session fleet inherits that. What
a 1000-user × 2-network deployment collides with:

- `max_concurrent_processes` defaults to **4** (Phase 8 fix).
- ~2000 live sessions × 2 tasks each, plus 2000 open TLS sockets, is RSS the
  process has never carried.
- Heartbeats at 30 s × 2000 sessions ≈ **67 journal writes/s** through the same
  single-writer group-commit funnel that serves every turn; the funnel's
  1024-command queue converts sustained pressure into backpressure on **turn
  admission**.
- Postgres `pool_max_size` defaults to **2**.
- No shard or drain model — session placement is arbitrary, and "drain node A"
  has no expression.

**Recommendation:** target low hundreds of sessions per deployment for v1,
measure the funnel under load in Phase 8, and treat multi-replica session
placement as its own ADR when it is real. Do not let the fleet quietly become
the thing that makes multi-replica serving mandatory.

---

## 11. Decisions required before Phase 0 closes

1. **QR payload custody** — serializable challenge vs. leased handle (§3).
2. ~~**Global search on Telegram** — restricted to Premium.~~ **Withdrawn
   2026-08-07: the premise was false.** The paid/star-gated method is
   `channels.searchPosts` (public-post discovery). `messages.searchGlobal` —
   searching the user's *own* chats — carries no Premium restriction. Bind
   global search normally.
3. **Telegram crypto review** — who reviews `grammers-crypto` and the auth
   path, and before which phase ships to real users (§8).
4. **ToS posture** — whether to pre-notify Telegram, and what the connect UX
   tells users about account-ban risk (§8).
5. **Read-transcript copy** — how the product explains that live reads are not
   stored but *do* enter the conversation transcript (§6).

Deferred to Track B, not needed now: `search_messages` relevance ranking, turn
admission policy for inbound messages, and mirror retention on unlink.

---

## 12. Gates that will fire

Run `cargo test -p ironclaw_architecture_tests` at every phase. Specifically:

- **Layer matrix** — both new crates need `[package.metadata.ironclaw] layer`;
  `LAYER_MATRIX_EXCEPTIONS` is empty and ratcheted.
- **Same-layer edge inventory** — pinned as an *equality* and shrink-only.
  Prefer zero new `domains → domains` edges; the mirror's dep list is
  deliberately `filesystem` + `host_api` only.
- **Specificity** — a `telegram` package directory makes `telegram`, `mtproto`,
  and the vendor host forbidden vocabulary in every generic crate. The
  allowlist is shrink-only. Vendor names live in the package and in
  `extension_support` (scan-exempt) and nowhere else.
- **Sealed evidence mint ratchet (23 tests)** — register any new mint fn in
  `CHANNEL_MINT_FNS` or the census silently stops governing it.
- **`ironclaw_auth` module charter** — sub-owner map row per new file; the
  bare-substring severance probes.
- **`assert_workspace_deps_exactly`** — the CLI dep set is an exact equality;
  update it in the same PR.
- **`check-target-tree.py`** — 64 → 66 documented packages.
- **Composition budget** — 40,423 LOC / 814 `Arc<dyn>`, small tolerances.
- **Manifest v3 binding validations** — tool id, absent schema refs,
  `external_write` floor, one binding per op.
- **`telegram_extension_gates.rs`** — `telegram_personal` / `telegram_bot` must
  stay dead. One `telegram` extension id; the linked account is a *surface* of
  it, never a companion identity.
- **ADR 0001's scope fence** — it excluded multi-account channel surfaces
  pending "a conversation-attribution design." This feature fires that trigger
  and must supply that design.

Also update the Telegram extension design doc, whose non-goals still say "no
acting on the user's behalf (no MTProto/link-device)" and which pins an empty
tool surface with a negative test.

---

## 13. Testing obligations

Per `.claude/rules/testing.md` and `.claude/rules/lifecycle.md`, this feature's
required coverage is **denial, cancellation, restart, conflict, redaction,
scope isolation, partial failure**, plus:

- Authentication failure must prove **reconnect does not resume without an
  updated credential revision** (not merely that it failed).
- Restart mid-session must prove re-claim and resume without duplicate ingest.
- Two-user scope isolation on both the session store and the mirror.
- Standard-op conformance via `ironclaw_host_api::test_support::messaging_conformance`,
  including the evidence loop (a write's `message_ref` feeding an edit/delete).
- Mirror parity across in-memory / libSQL / Postgres, modelled on
  `crates/loop/ironclaw_hooks/tests/parity_matrix.rs` — cross-assert every
  backend *and* an independent oracle, so a shared bug cannot pass. FTS
  tokenization parity must be explicit.
- An integration proof that a session-sourced inbound message reaches the
  mirror and does **not** create a turn.
