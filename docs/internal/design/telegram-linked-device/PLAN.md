# Telegram Linked Device — Execution Plan

Revision 6 (re-verified against a moved `main`, 2026-08-10). Read
[PROPOSAL.md](PROPOSAL.md) for the specification.

**Shape:** eight PRs. **There is no spike step** — every vendor-mechanics claim
the earlier revision wanted to defer has been verified directly against the
grammers 0.10.0 sources and the reference QR implementations, and folded into
PROPOSAL §14.1 as decided fact. What remains is implementation.

**Dependency edges:**

```
PR1 contracts ─► PR2 auth ─► PR3 ext-host ─► PR5 package (fake) ─► PR6 transport ─► PR7 tools ─► PR8 hardening
                    │                             ▲
                    └──► PR4 frontend ────────────┘
```

PR 4 (frontend) depends on **PR 2**, not PR 1 — it renders the step state PR 2
defines — and must land **before** PR 5, whose risk-gate proof drives the flow
through the real UI.

---

## PR 1 — Contracts, schema amendment, and auth custody

**Ships:** `DeviceLinkAdapter` (with `mode` and typed `DeviceLinkInput`),
`VendorAuthRecipe::DeviceLink` and every arm,
`AuthPromptChallengeKind::DeviceLink` + prompt views,
`RuntimeCredentialAccountSetup::DeviceLink`,
`LifecycleExtensionCredentialSetup` variant + the extension-manager arm, the
`ironclaw_extension_registry` recipe→setup projection arms, the
`VendorAuthRecipe` arm in composition's `factory/auth_engine_assembly.rs`,
the `LinkedSessionPort` family declaration (**no `ToolPorts` change** — the
custody handle arrives via a bind-time factory, so none of its six construction
sites break). **No new crate** — session custody extends
`ironclaw_auth` with `link_revision` and a CAS-bearing opaque-material write;
`SessionBytes` and `MAX_LINKED_SESSION_BYTES` live in **contracts** (§5.1).

**Also ships the schema graduation** — a **new** `send_message.output.v2.json`
carrying the `sent_unverified` branch, plus version-plural `StandardOpContract`,
`.v2` arms in `resolve_standard_schema_ref` and `CapabilityProfileSchemaRef`, and
an explicit decision on the op-keyed runtime validator (superset test, or make it
version-keyed). `.v1` is never edited. The model-facing sentence goes in
Telegram's **vendor addendum**, not the shared core. This is a contracts-tier
workstream — scope it as such, not as a one-file edit. `send_message` cannot be
bound until it lands.

`LinkedSessionPort` is declared here, in `ironclaw_extension_contracts` beside
`RestrictedEgress`. Add the dated clarification to `crates/contracts/AGENTS.md`
separating a record-owning store trait from a pre-scoped capability port.

**Scope warning:** this is titled "contracts" but is a **9+-crate PR**. Adding
`VendorAuthRecipe::DeviceLink` forces arms in every exhaustive match at once
(auth ×3, extension_host ×2, registry, composition), and
`AuthPromptChallengeKind::DeviceLink` breaks `ironclaw_assistant`'s prompt match.
Land placeholder arms where behavior arrives later, and budget 2–3 weeks.

**Also touches, and the footprint must say so:** `ironclaw_secrets` (the CAS
write path is a *substrate* change, not only an auth one) and
`ironclaw_assistant` (the prompt arm — which forces a product decision nobody has
made: what does an in-channel prompt say when a run parks on a device-link gate
that cannot be completed in-channel?).

**Watch:**
- Ship `RuntimeCredentialAccountSetup::DeviceLink` and emit it from nowhere.
- `link_revision` on `CredentialAccount` needs `#[serde(default)]` and a
  rehydration test — it is a persisted record.
- The supply-chain controls (§11.1) ship with **the first grammers crate** —
  which is PR 5, not this PR and not hardening.
- Three shape decisions must land here or later PRs build on sand:
  `LinkedAccountRef` reachable from contracts (§3.3) and grant-vs-bare-id on the
  factory (§5.1). *(The owner-only enforcement decision is withdrawn — upstream
  #7397 made owner == actor; see §3.3.)*
  `#[serde(other)] Retired` means an older binary folds an unknown kind to
  unserviceable.
- The two silent recipe arms: `keepalive_idle_threshold → None`, and the
  explicit `(DeviceLink, DeviceLink)` arm on `compatible_for_shared_vendor`
  (its `_ => false` fails at *activation*, not at compile time).
- Raise the contracts size ceiling; update the contract location scan.

**Done when:** architecture tests green; wire round-trips for both new enum
variants; a test proves an unknown future setup kind still folds to `Retired`;
auth-side tests cover the CAS write path (a concurrent write conflict is rejected
with the current version, not last-writer-wins; the semantic merge is tested
package-side), the size ceiling, and revision gating.

---

## PR 2 — Auth: the device-link method, and the ADR

**Ships first, because everything downstream programs against it:** a signature
block for `DeviceLinkDriver` — methods, error surface, the
`DeviceLinkStep` → `AuthChallenge::DeviceLinkStep` projection, which clock each
TTL belongs to, and what it returns when no binding exists. Revision 3 described
this port only by name and responsibility; it is the largest
design-it-yourself hole in the plan.

Note the vocabulary trap: "driver" means two things — the auth-side step engine
(here) and the host-side `DeviceLinkDriver` impl (PR 3). Inside `ironclaw_auth`
only the *port* can be faked; the adapter is invisible behind it by design.

**Ships:** `AuthFlowStepState`, `AuthFlowStatus::AwaitingVendor` (+ the explicit
`Authenticating` projection arm), `AuthChallenge::DeviceLinkStep`,
`advance_flow_step` with revision CAS, the `DeviceLinkDriver` port, the driver,
step expiry, `product_prompt.rs` projection, fakes, conformance — and **the
ADR** the auth charter requires for a quirk hook.

**The ADR is drafted** — [ADR-device-link-auth-hook.md](ADR-device-link-auth-hook.md).
Land it in this PR. It states the real compensation set (code review + trust in
grammers; **no host-side control can detect a substituted login**), adds the
post-completion device-confirmation detection control, and records the
in-process-vs-sidecar trade as *the* security decision. Note it makes the
supply-chain pins a **PR 5** deliverable — the PR that first adds a grammers
crate — not hardening.

**Watch:**
- Every new `src/**/*.rs` gets a sub-owner row in `AGENTS.md` in the same commit.
- The two module halves may not name each other. The probe strips comments and
  strings, so only real paths trip it — a module named `device_link_engine::`
  would. Route through the crate-root `provider.rs` port.
- The driver must **never re-invoke the adapter for a transition that already
  ran**; on CAS loss it reloads and reconciles.

**Done when:** `cargo test -p ironclaw_auth` green including `module_charter`;
the driver walks a fake adapter through display → input → complete; the ADR is
merged.

---

## PR 3 — Extension host: bindings and the driver implementation

**Ships:** the `device_link` binding slot on `ExtensionBindings` and its
`check_binding` arms; **retirement of `auth_never_binds_is_not_a_binding_field`**
with a written rationale; the `DeviceLinkDriver` implementation resolving
extension → bound adapter and constructing a **pre-scoped**
`DeviceLinkContext`; rate limits and TTL enforcement; `LinkedSessionPortFactory` supplied on `BindContext`; the onboarding-copy arm.

*Revision 1 had no PR for this — the glue was unowned and the bindings were
believed to live in contracts.*

**Watch:**
- The retired test encodes a security claim. Its replacement rationale belongs
  in the same commit and should reference the PR 2 ADR.
- Scoping is the security boundary: the adapter receives a store it cannot
  re-address.

**Done when:** a declared device-link extension binds and an undeclared one is
refused; the driver drives a fake adapter end to end through the auth engine.

---

## PR 4 — Frontend

**Ships:** extraction of the QR/countdown/poll presentation from
`pairing-web-code-panel.tsx` into a shared panel; `auth-device-link-card.tsx`
with the **QR ⇄ phone-number switch**; the `challengeKind === "device_link"`
branch; `gates.ts` normalizer; extension-card affordance; i18n; the additive
flow-status wire fields; and the backend route that submits user input to the
device-link driver.

**Watch:**
- Recompose the existing pairing panel over the extracted component; two QR
  implementations will drift.
- Ignore stale-revision poll responses; stop polling on terminal states.
- A `Failed { restartable: true }` (process restart, TTL reap) must offer
  "start again" rather than dead-ending.

**Done when:** frontend vitest green; `webui_v2_descriptors_contract` green if
routes changed; every step renders including both input kinds.

---

## PR 5 — Package scaffold with a fake vendor

**Ships:** `src/linked/` tree; shared `SessionPool` (used by both adapters);
`PendingLinks`; `IronclawSession` over `LinkedSessionPort`; a
**scripted-fake** `DeviceLinkAdapter` and a fake transport for tools;
`TelegramToolAdapter` routing; CLI entrypoint binding all three adapters;
composition wiring. The manifest gains `[auth.telegram]` and **exactly one**
fake-backed tool binding — not the full set.

*Revision 1 shipped the full 15-tool manifest here, two PRs before
implementations, contradicting its own "bind only what is implemented" rule.*

**Proves — the risk gate:** link → tool call → unlink through the real UI, real
auth engine, real driver, fake vendor. Session survives pool eviction. Two users
isolated.

**Decide before starting — the vendor seam.** `PendingLink` and `SessionPool`
are specified in `grammers-client` types that arrive in PR 6, so this PR must
build against an abstraction the design has not defined. Pick the level and write
it down: an **op-level** `TelegramTransport` trait (easy to fake, but PR 6/7 grow
it to 15 methods and rewrite the pool internals around real runner tasks anyway),
or a connection/RPC-level seam (fights grammers' generic `invoke<R>` for object
safety). Say explicitly **what of `pool.rs`/`PendingLinks` is expected to survive
PR 6** — the honest reading is that this PR retires the *host-plumbing* risk, and
the vendor-side lifecycle code will be substantially rewritten.

**Decide before starting — how a fake vendor ships dark.** This PR puts
`[auth.telegram]` and a tool binding into the **production manifest**, bound by
the **production entrypoint**: between PR 5 and PR 6 a real deployment would
offer "Link my account" backed by a scripted fake. No flag or feature gate exists
for this. Either put the fake behind a non-default cargo feature the harness
enables, or land the manifest auth section in PR 6 and inject a test manifest
here.

**Budget the harness work — it is this PR's critical path, not the package code.**
`tests/CLAUDE.md` records that **Telegram has no group-tier lifecycle scenario**
because its setup resolves through a pairing mechanism the bare harness does not
mount. The integration proof needs a new harness profile that mounts the native
package with admin config satisfied, helpers that drive a multi-step device-link
flow through the product-auth services, tool dispatch as a specific user, unlink
through credential cleanup, the two-user variant, a `[[test]]` registration, and
a `tests/CLAUDE.md` row.

**Watch:**
- `grammers-session` and `grammers-tl-types` (types only, no sockets) arrive here
  for `IronclawSession`; socket-bearing `grammers-client` waits for PR 6. **The
  first grammers dependency entering the tree means `cargo deny`, licence review,
  and the §11.1 supply-chain pins land in THIS PR**, not PR 6.
- The integration proof runs through the **real harness**, not a real UI — the
  Rust tier has no UI. Real-UI proof would be a served-binary E2E fixture, which
  is unscoped.
- Lazy init only; never hold the pool lock across an await.
- Test the reactivation path explicitly: admin-config edit → adapters rebuilt →
  next call reconnects from the blob.
- Rewrite `telegram_factory_binds_a_channel_and_no_tools`.
- Name the test double so it does not match `struct InMemory*Store`.
- 999-line budget counts inline test modules.

**Done when:** the integration test links, calls, and unlinks against the fake;
architecture tests green.

---

## PR 6 — grammers transport and real login

**Ships:** `grammers-client`; `transport.rs`; the real `DeviceLinkAdapter` — raw
TL QR with **poll-driven acceptance**, phone path, 2FA; DC validation in
`IronclawSession`; logout-on-abort; the carve-out documentation and the three
charter amendments (named, not gestured at).

**Watch:**
- `tokio::spawn(runner.run())` is required. `drop(updates)` is **global** — the
  login client polls too and needs no updates.
- QR poll: same session, identical `except_ids`, `min(3s, expires − serverNow)`,
  repaint only on byte change, correct for **server** time, exponential backoff
  1 → 60 s on export errors. 2FA arrives as `SESSION_PASSWORD_NEEDED` **on the
  export call**.
- Set `NoRetries` as the client retry policy and retry explicitly in our wrapper
  — `AutoSleep` would re-send a write once after an I/O error, and no custom
  policy can prevent it (the request is invisible to the policy on `Io`).
- `Dropped` on a write means **outcome unknown**, never "not executed" —
  surface as sent-unverified, not failure.
- Quiesce the old pool before the new one connects on reactivation: two pools
  over one session cause auth-key last-write-wins and update-state interleaving
  (no corruption, but avoidable).
- Per-link mutex serializes vendor operations; `poll` is a pure read while
  awaiting input.
- Post-acceptance abort ⇒ `auth.logOut` before drop, TTL reap included.
- Completion order: store blob → mint account → report completed.
- `MigrateTo` via `invoke_in_dc`; persist the new home DC; cache the self-peer.
- `Dropped` ⇒ evict, rehydrate, retry **reads only**.
- `PooledClient::Drop` aborts its runner and flushes.
- Exact-pin `=0.10.0`; isolate raw TL to one module.

**Done when — split by tier, because most of this cannot run in CI:**

*Automatable:* DC validation rejects a poisoned address; `NoRetries` installed;
no write is retried; per-link mutex serializes; store-then-mint ordering (test by
failing the store); `PendingLinks` bounded and reaped.

*Live smoke, human-assisted, scripted protocol:* a real account links by QR and
by phone; a restart resumes without re-linking; an aborted post-acceptance link
leaves **no** device on the account. The abort proof needs something to *accept*
the QR mid-test — a human with a phone, or an automation rig holding a second
logged-in session that calls `auth.acceptLoginToken` and enumerates devices.
**That rig is unscoped work; scope it or accept a manual protocol.**

---

## PR 7 — Standard-op tools

**Ships:** the 15 op implementations, canonical mapping, error mapping, prompt
docs, permissions posture, and the manifest's full tool set.

**Watch:**
- `id == 0` ⇒ sent-but-unverified, terminal, **never retried**.
- Empty `text` rule for media/service messages, or the first sticker fails a
  whole history call.
- Date bounds on search; name resolution beyond @usernames; `MAX_PAGES_PER_CALL`.
- Our wrapper's flood-wait sleep budget sits below `TOOL_CALL_TIMEOUT`.
- Retry-after can ride only `model_visible_cause` prose today — do not imply a
  structured value (§6.6).
- The full §6.6 error table is implemented, not just the catch-all.
- Supergroup migration and `CHANNEL_PRIVATE` ⇒ `messaging.unknown_conversation`.
- Credential failures ride `AuthRequired`, not the messaging taxonomy.
- Groups and DMs are both in scope (decision 5) — `list_members` stays bound.

**Done when:** messaging conformance green for every bound op including the
evidence loop; the gated approve → resume integration test passes.

---

## PR 8 — Hardening and release readiness

**Ships:** bounds tuning against **200 concurrent sessions**, the flood-wait
backoff policy (§7.1), a per-user link-state surface (no message content, no
identifiers beyond opaque ids), live smoke script, docs updates
(`2026-07-16-telegram-extension-design.md` non-goals; the linked-accounts doc),
and the internal feature security review
(`/security-review`) — distinct from the grammers-crypto review, which §11.1
deliberately defers.

**Watch:**
- Prove a failed session does not reconnect until `link_revision` changes.
- Prove a Telegram-side revocation parks the run and re-linking resumes it.
- Verify — do not assume — that an un-linked user sees no tools.

**Done when:** [CHECKLIST.md](CHECKLIST.md) is fully ticked.

---

## Estimate

**4.5–6 months** for one engineer, PR 1 through PR 8. Revision 3 said 3.5–4;
the implementability audit re-estimated after counting PR 1's true blast radius
(9+ crates), PR 5's unscoped harness work, and the governance latency the plan
never budgeted — the PR 2 ADR, the PR 3 test-retirement rationale, and the PR 6
charter amendments each need *other people* to sign off.

**The likely schedule-killer is PR 6.** Every real-login bug is debuggable only
against live Telegram with a human scanning QR codes — a minutes-per-iteration
loop, rate-limited by flood waits, on infrastructure no PR provisions. Second
place is PR 5's harness work, because it blocks the risk gate the plan hinges
on.

## Failure modes that would change the plan

| If implementation (or the live smoke) shows… | Then… |
| --- | --- |
| Pull-only stalls in practice | Message updates must be consumed → the T2′ ingress ADR returns; roughly doubles the estimate |
| Re-export invalidates a displayed QR in practice, contradicting the Web K evidence | Fall back to consuming `updateLoginToken` on the login client; if that is also unreliable, ship phone-only linking for v1 |
| The PR 1 schema amendment is rejected in review | Do not bind `send_message` until an accepted representation lands — never fabricate a ref |
| Parked logins cannot survive realistic deploy cadence | Accept that a deploy cancels in-flight links **and** logs them out, or make the link durable |
| The raw-socket carve-out is rejected | Only an out-of-process sidecar remains — a quarter-scale project |
| Provisioning is missing | `api_id`/`api_hash` per developer, Telegram test accounts, and a second-device rig for accept/revoke testing are prerequisites no PR provisions — sort them before PR 6 |
