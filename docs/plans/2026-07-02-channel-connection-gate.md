# Channel-connection gate — put Slack pairing on GitHub's auth-gate rail

**Status:** in progress (branch `reborn/channel-connection-gate`)
**Owner:** Ben
**Supersedes the frontend-heuristic pairing panel of PR #5362.**

## Problem

When the model activates a connectable inbound channel (Slack, Telegram) and the
caller isn't yet connected, the backend returns a **completed** `extension_activate`
tool result with a `channel_connection_required` display card, and the WebChat
panel is *derived from that durable timeline card*. Because activation completes
the turn, there is **no durable "waiting for connection" state**: a finished thread
that merely contains a past card is indistinguishable from a live pending one, so
historical Slack chats resurrect the panel (especially after the channel is later
disconnected). The "resume" is faked in the browser by sending a literal
`"Slack is connected. Continue the previous request."` message.

GitHub/Gmail (OAuth) do **not** have this problem: they block the turn via
`auth_required_for_credentials` → `TurnStatus::BlockedAuth` → `pendingGate`
projection (reconnect-safe, rebuildable) → `resolve_gate` resume. Slack is the
only connectable channel not on that rail. **That divergence is the bug.**

## Decision (locked)

Reuse the **existing auth-gate primitive**. Do NOT add a new
`TurnStatus::BlockedConnection` / `BlockedReason::Connection`. Slack (and any
connectable inbound channel) blocks the turn the same way GitHub already does —
`FirstPartyCapabilityError::auth_required*` → `BlockedAuth` → existing
block/persist/project/resume rails. The only additions are **data**: a
`channel_connection` challenge kind on the auth prompt carrying the connection
requirement (channel, strategy, instructions, input_placeholder, submit_label,
error_message — the `/pair` copy), so the frontend renders a pairing card on the
auth-gate rail. Semantically sound: pairing IS per-user authorization to act as
the user on an external service, same category as GitHub's token.

Behavior change (intended): the model sees a parked turn awaiting connection
instead of concluding "I can't read Slack"; on pairing it **auto-resumes** and
finishes the original request. The fake "Slack is connected, continue" message
is deleted.

### Refinement (adopted) — challenge kind = interaction modality, not provider

Do NOT add a `channel_connection` challenge kind. Challenge kinds describe the
**interaction modality**; the specifics ride as context. We do NOT rename the
wire values — they stay the stable `oauth_url` / `manual_token`; the modality is
expressed only in comments/naming:
- `oauth_url`: browser OAuth relay.
- `manual_token`: paste a string. Covers GitHub PAT, API key, AND
  Slack/Telegram **pair code** — one card.

Slack pairing reuses `manual_token`. What differs between a PAT and a pair code is
only the **resolve route**, carried as gate *context* (data, not a new kind/turn-state):
- PAT → store credential → resolve with `credential_ref` (`resolve_auth_gate`).
- pair code → redeem (bind identity) → resolve → resume (`resolve_generic_gate` path).

So the gate carries a `ConnectionContext { channel, strategy, instructions,
input_placeholder, submit_label, error_message }` (optional, serde-default) on
`AuthPromptView` / `AuthPromptContextView` / `ProductProjectionItem::Gate`, plus a
resolve-route discriminator ("this paste is a pair code for channel X"). The one
paste card renders for both (copy from context); its submit + the backend resolve
route by that discriminator. Any future "paste a code to connect" channel or
"relay to OAuth" provider drops in with no new kind. The wire values stay stable
(`oauth_url` / `manual_token`), so persisted gates / in-flight events / the
frontend transition don't break; the enum readers (frontend `gates.js`, serde
tests) keep reading those exact strings.

## Per-layer plan (file:line seams from tracing)

### V1 — activation → gate → projection (backend)
- **Activation handler** `crates/ironclaw_reborn_composition/src/extension_lifecycle_capabilities.rs`
  `EXTENSION_ACTIVATE_CAPABILITY_ID` (:167). Today: credential gate at :183, else
  activate, then `channel_connection_display_preview` at :214 and return `Ok`.
  Change: after activation succeeds and payload carries `connection_required:
  Some(req)`, check per-user connection; if NOT connected → return an
  `auth_required`-family blocking error carrying `req` as `channel_connection`
  challenge context; if connected → `Ok` with NO card. Wire
  `ChannelConnectionFacade::caller_channel_connections`
  (`slack_connectable_channel.rs:162-183`) into `ExtensionLifecycleToolHandler`
  (fields :127-130; also its constructor/wiring). Keep the OAuth/credential gate
  at :183 byte-for-byte (GitHub/Gmail unaffected).
- **Requirement source:** `channel_connection_requirement()`
  (`extension_lifecycle.rs:1223-1249`), predicate
  `package_declares_inbound_product_adapter` (:1251-1256), set in
  `commit_activation` (:583-590). Prefer sourcing from the connectable-channel
  abstraction (`RebornConnectableChannelInfo`) so it's channel-generic and
  forward-compatible with #5107's manifest-driven `[[metadata.connectable.channels]]`.
- **Blocking error → outcome → loop:** `first_party.rs:179 auth_required()` /
  `:205 auth_required_for_credentials`; `production.rs:2011 auth_required_outcome`
  + `stable_auth_gate_id` :2025; `RuntimeAuthGate` `lib.rs:556`;
  `capability_port.rs:2608` maps to `CapabilityOutcome::AuthRequired`,
  `loop_gate_ref` :2963. Add a channel-connection challenge carrier on the auth
  gate (new field on `RuntimeAuthGate`/outcome, or context alongside
  `credential_requirements`).
- **Projection:** `crates/ironclaw_reborn_composition/src/projection/turn_events.rs:406`
  `blocked_prompt_payload` BlockedAuth arm → `AuthPromptView` with
  `challenge_kind = channel_connection`; `auth_prompt.rs:48 challenge_for_gate` /
  `:162 auth_prompt_from_credential_requirement`.
- **Wire contract:** `crates/ironclaw_product_adapters/src/outbound.rs` — add
  `AuthPromptChallengeKind::ChannelConnection` (:959, wire `channel_connection`)
  + a `ChannelConnectionPromptContext { channel, strategy, instructions,
  input_placeholder, submit_label, error_message }` optional field on
  `AuthPromptView` (:975) and `AuthPromptContextView` (:1007); thread through
  `ProductProjectionItem::Gate` (:1149). Additive + serde-default (backward safe);
  extend the challenge-kind serde table test (:1529).
- **Tests:** rewrite Slack/Telegram activation tests that assert `Ok`+card
  (`extension_lifecycle.rs:1651-1704`, `:1706-1768`; capabilities.rs :348-444)
  to assert the gate (unconnected → auth_required+channel_connection; connected →
  `Ok`, no card). Caller-level test through the handler.

### V2 — resume (backend)
- **Redeem endpoint** `slack_personal_binding_pairing_serve.rs:115-137`
  (`slack_personal_binding_pairing_redeem_handler`): on success, after binding the
  identity, resume the caller's turn(s) blocked on this channel's connection gate.
- **Resume model:** `auth_interaction/service.rs:163-193 resume_auth_gate` →
  `resume_turn(ResumeTurnRequest{ precondition: BlockedAuthGate })`. A connection
  gate has no credential_ref, so resume via the generic path
  (`reborn_services.rs:5367 resolve_generic_gate` resumes via `resume_turn`
  WITHOUT credential_ref) — model on that, not `resolve_auth_gate` (:5324).
  Locate the blocked run(s) by caller+channel (the cross-thread resume the
  frontend waiter bus fakes today).
- On resume the parked `extension_activate` re-dispatches; the per-user
  connection check now passes → activation completes → run continues → Completed.

### V3 — frontend (delete heuristics, render on auth rail)
- **gates.js** — add `channel_connection` branch in `gateFromEvent` (:6) and
  `gateFromProjectionGate` (:61) mapping the challenge → `{ kind, channel,
  strategy, instructions, inputPlaceholder, submitLabel, errorMessage, runId,
  gateRef }`.
- **useChatEvents.js** — add the status/case so the gate isn't treated stale:
  `PROMPT_RUN_STATUSES` (:244), `GATE_ACTIVE_RUN_STATUSES` (:250), `case
  "channel_connection"` (:124). (BlockedAuth status already listed — verify.)
- **chat.js** (:302-339) — add a `pendingGate.kind === "channel_connection"`
  branch rendering `OnboardingPairingCard`, `onSubmit=submitChannelConnectionPairing`,
  `onCancel=resolveGate("cancelled")`.
- **useChat.js** — replace `submitOnboardingPairing` with a `submitAuthToken`-shaped
  handler (redeem code → `resolveGate`). DELETE the history/session/localStorage
  derivation and waiter bus per the frontend inventory: `channelConnectionRequirementFromCard`,
  `latestChannelConnectionRequirement`, `connectionCardSourceIds`,
  `channelConnectionIsSatisfied`, the dismissed-onboarding localStorage machinery,
  `pendingOnboarding` state + refs + the big derive effect, `rememberChannelConnectionWaiter`/
  `resumeOnboardingAfterChannelConnected`/`dismissOnboardingPairing`, and the
  `channel-connection-events.js` waiter half (`channelConnectionContinuationMessage`,
  `rememberChannelConnectionWaiter`, `forgetChannelConnectionWaiter`,
  `resumeWaitingChannelConnections`, waiter localStorage). KEEP the notify/display
  half (`normalizeConnectionChannel`, `channelConnectionDisplayName`,
  `notifyChannelConnected` minus its waiter tail) shared with Settings.
- **Tests:** delete `channel-connection-events.test.mjs`; rewrite the
  card-derivation/waiter tests in `useChat-send.test.mjs`, `chat.test.mjs`; fix the
  `configure-modal.test.mjs` import of a removed symbol; keep
  `onboarding-pairing-card.test.mjs`.

## Regression surface
- GitHub/Gmail/Notion OAuth gate: **unaffected** (already gated; do not touch :183).
- Model-visible activation message change (V1): activation no longer emits the
  "activated + guidance" message for an unconnected channel — it parks. Update
  the tests pinning that prose (`extension_lifecycle.rs:1662-1675`).
- Wire-stable `AuthPromptChallengeKind` is additive; old rows deserialize fine
  (`serde(default)` on the new field). Per `.claude/rules/types.md`.
- `auth_required` is a BLOCKING gate, not a terminal `Err` — verify it maps to
  `BlockedAuth`, not `HostUnavailable` (`.claude/rules/agent-loop-capabilities.md`).

## Coordination
- Overlaps serrrfirat's in-flight #5107 (manifest-driven connectable channels via
  `RebornConnectableChannelInfo` / `ConnectableChannelsProductFacade`). No file
  conflict (V1-V3 touch none of #5107's files), but source the requirement from
  the connectable-channel abstraction so both compose. #5072 is ingress-only,
  orthogonal.
