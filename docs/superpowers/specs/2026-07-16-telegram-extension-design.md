# Telegram Extension — Design Spec

> **SUPERSEDED by the unified manifest-driven extension runtime.** This file is
> retained as the pre-unification design record. The shipping public lifecycle
> is `uninstalled -> setup_needed -> active`, derived from membership plus the
> manifest-declared connection recipe. There is no public Activate command,
> endpoint, or durable per-user activation toggle; references below use current
> install/setup terminology where they describe user-visible behavior.

- **Date:** 2026-07-16
- **Status:** Superseded — see `docs/reborn/extension-runtime/overview.md`
- **Target:** Reborn stack on `main`, shippable before PR #6116 merges, portable onto #6116 with zero behavior change

## Summary

Ship Telegram as a first-class IronClaw entrypoint on the Reborn stack:

- **Admins** configure one Telegram bot per deployment (bot token, Channels tab), exactly parallel to the Slack bot setup.
- **Users** install the single `telegram` extension (registry tab or in-chat
  install). Membership immediately derives `setup_needed` while the caller is
  unpaired, and the pairing gate issues a short-lived code presented as a deep
  link `https://t.me/<bot_username>?start=<CODE>`; tapping Start pairs
  automatically, resumes blocked threads, and makes the derived state `active`.
- Once paired, DMs to the bot are a full IronClaw entrypoint (continuous conversation, proactive delivery in). **No `telegram.*` tools; IronClaw cannot act on the user's behalf** — that is the future link-device flow under the same `telegram` extension identity.

The implementation clones the proven `crates/ironclaw_reborn_composition/src/slack/**` host-module shape (as `telegram/**`, cargo feature `telegram-v2-host-beta`), reuses the existing, unwired `crates/ironclaw_telegram_v2_adapter` untouched, and pins every externally observable name/route/semantic to the shapes PR #6116 already ships for its reference Telegram extension — so porting later means deleting the composition module while the behavior contract (and this QA suite) carries over 1:1.

## Decisions log (owner: Ben, 2026-07-16)

1. **Pairing direction:** IronClaw issues the code; deep-link auto-pair; typed-code-to-bot fallback in the same direction. (Supersedes v1's bot-issues-code flow; maps to `RebornChannelConnectStrategy::WebGeneratedCode`.)
2. **Transport:** webhook only. IronClaw calls `setWebhook` itself on admin save; verifies `X-Telegram-Bot-Api-Secret-Token` on every inbound. No long-polling in v1.
3. **Scope:** DMs only. Group messages, `channel_post`, edited messages, inline queries are ignored/fail closed. Unpaired DM fails closed with a static pairing hint.
4. **Approach:** Slack-shaped composition module, **single `telegram` extension** (no hidden `telegram_bot` companion), target-shaped naming, #6116 webhook path, pairing resumed via the existing `BlockedAuth` fanout.
5. **Legacy scope:** Reborn-scoped purge. Every legacy-shaped Telegram artifact in the Reborn context is rewritten or removed as part of this feature (see §8); the v1 monolith implementation keeps working until the monolith itself retires (tracked follow-up under the roadmap's "Clean up old architecture"), and the v1/v2 exclusivity guard survives as the collision arbiter while both exist.

## Non-goals (v1)

- No `telegram.*` tools, no acting on the user's behalf (no MTProto/link-device). A negative test pins the empty tool surface.
- No group-chat routing/admission, no admin subject-routes for Telegram.
- No long-poll transport, no bot-issues-code direction. QR as a *separate connect strategy* is out of scope — but the pairing panel does render a QR **of the deep link** as pure presentation (§4.2), since cross-device (desktop browser + phone Telegram) is the dominant real deployment shape.
- No behavioral changes to the v1 monolith Telegram (`channels-src/telegram/`, `tools-src/telegram/`, v1 pairing machinery) — it keeps working until the monolith retires; deleting it rides that retirement, not this feature. The v1↔v2 exclusivity guard (`REBORN_TELEGRAM_V2_ENABLED`) is retained and re-pointed at the new implementation (§8). The reborn binary never runs v1 channels. Legacy-shaped Telegram code *inside the Reborn context* is NOT exempt — §8 removes it.
- No multi-bot / multi-workspace support (one bot per deployment; Slack has the same single-installation shape).

## 1. Identity & naming (the porting contract's foundation)

One user-visible extension: **`telegram`**. Never `telegram_bot` / `telegram_channel` / `telegram_personal` in any new identifier — #6116's `reborn_retired_taxonomy` gate pins those forms to zero, and its `reborn_extension_specificity` gate forbids `telegram` strings in generic crates (all concrete strings live in the telegram module/crates only).

| Concept | Value (identical on main and post-port) |
|---|---|
| Extension id / `ExtensionName` | `telegram` |
| Bot token credential handle | `telegram_bot_token` |
| Webhook secret handle | `telegram_webhook_secret` |
| Identity provider id | `telegram` |
| Provider user key | `{bot_id}:{telegram_user_id}` |
| Webhook route | `POST /webhooks/extensions/telegram/updates` |
| Connect strategy | `RebornChannelConnectStrategy::WebGeneratedCode` (already in main's enum) |
| Actor kind | `telegram_user` (`TELEGRAM_USER_ACTOR_KIND`, already in the adapter crate) |
| Admin routes | `GET/PUT /api/webchat/v2/channels/telegram/setup` |
| Cargo feature | `telegram-v2-host-beta` |

The webhook route is deliberately the **#6116 path** (`/webhooks/extensions/{id}/{suffix}` with suffix `updates`), not main's Slack convention (`/webhooks/slack/events`), so `setWebhook` registrations held server-side by Telegram survive the port with zero re-registration.

Scoping pairings by `bot_id` (from `getMe`) gives correct bot-swap semantics: rotating the same bot's token keeps all pairings; pointing the deployment at a different bot orphans them (users must `/start` the new bot anyway before it can message them).

## 2. Admin setup (Channels tab)

Mirror of Slack's operator flow (`slack_channel_routes/*`, `slack_setup.rs`, `slack-setup-panel.tsx`), reduced to one secret field:

- The Extensions page **Channels tab** (`pages/extensions/components/channels-tab.tsx`) shows a Telegram card for the operator: the telegram connectable-channels facade returns an `admin_managed_channels` action only when the caller is the operator (same rule as `slack_admin_managed_channel_connectable_channel()`).
- New `telegram-setup-panel.tsx` + `lib/telegram-setup-api.ts`. Fields: **bot token** (secret, required), **public webhook URL override** (non-secret, optional; default derived from the deployment's configured public base — the same source Slack personal OAuth uses for its hosted callback). Blank secret on save means "keep existing" (Slack convention).
- Backend: `GET/PUT /api/webchat/v2/channels/telegram/setup`, mounted composition-side via a `WebuiServeConfig::with_telegram_channel_routes(...)` analog, gated by the same `ensure_authorized_operator` (cross-tenant → 404 anti-enumeration, non-operator → 403), each field passed through the safety-layer scan (`scan_route_admin_field`).
- **Save pipeline** (persist-then-reconcile with rollback, per the setup service):
  1. `getMe` — validates the token; captures `bot_username` + `bot_id` (persisted non-secret in the setup record; deep links need the username, identity scoping needs the id).
  2. Generate a fresh random `telegram_webhook_secret`.
  3. `setWebhook(url = <public-base>/webhooks/extensions/telegram/updates, secret_token = <generated>, allowed_updates = ["message"])`.
  4. Secrets into `SecretStore` under `telegram_bot_token*` / `telegram_webhook_secret*` handles (revision-suffixed like Slack), operator/tenant `ResourceScope`; record into telegram host state (`/tenant-shared/telegram-setup/`, a `FilesystemSlackHostState` analog).
  5. Reconcile the deployment channel host against the newly configured revision.
  - Any step failing ⇒ roll back to the previous saved state and return a precise admin error. Invalid token / unreachable `api.telegram.org` / missing public base URL all **fail closed**; nothing half-configured.
- `GET /setup` returns a **redacted status** (`bot_token_configured: bool`, `bot_username`, webhook state) — raw secrets are never echoed anywhere (UI, logs, API responses).
- **Reconfigure** re-runs the pipeline (token rotation of the same bot preserves pairings; a different bot's token re-scopes `bot_id` and orphans them by design).
- **Clear/remove setup**: `deleteWebhook` (best-effort — if the token is already revoked provider-side, proceed), make the deployment channel unavailable, and purge the secret handles. Pairing records and all history are **retained** (ingress simply fails closed until reconfigured; if the same bot returns, pairings work again).

## 3. Ingress & message flow

- **Mount**: public webhook composed like `slack_serve.rs` — `ListenerClass::PublicWebhook` with the fail-closed auth floor (`IngressAuthPolicy::Required`), served outside bearer auth but inside the CORS/body-limit stack. Route descriptor projected from the telegram manifest's `[[product_adapter.inbound.host_ingress]]` block via `host_ingress::bundled_host_ingress_descriptors`, matching the shape already fixture-tested in `ironclaw_product_adapter_registry`.
- **Verification**: constant-time `X-Telegram-Bot-Api-Secret-Token` comparison via the existing `SharedSecretHeaderAuth` (the adapter already declares `AuthRequirement::SharedSecretHeader`). Missing/wrong header ⇒ 401, no turn. 1 MiB body limit; malformed JSON ⇒ 4xx, no turn; per-installation rate limit (Slack's 12000/60s shape).
- **Dispatch**: immediate-ack (200 to Telegram right away) + async dispatch through `NativeProductAdapterRunner` wrapping the unchanged `TelegramV2Adapter::parse_inbound`. No setup saved ⇒ installation resolver rejects (Slack's `InstallationNotFound` → 401).
- **Replay/idempotency**: a duplicate `update_id` must not create a duplicate turn — reuse the same workflow idempotency seam Slack relies on for its E5 duplicate-delivery guarantee (exact seam pinned during planning).
- **Admission (DM-only)**: only private-chat `message` updates from paired users start turns. `allowed_updates=["message"]` filters server-side; defensively, anything else that still arrives (groups, `channel_post`, `edited_message`, service messages, messages from bots) is ignored — no turn, no reply. Unpaired-user DM ⇒ fail closed (`BindingRequired`, no turn), bot replies with a **static** throttled pairing hint ("Pair your account from IronClaw → Extensions → Telegram") — never LLM-generated, at most once per chat per throttle window.
- **Identity per message**: a `ProductActorUserResolver` analog of `slack_actor_identity.rs` — provider `telegram`, key `{bot_id}:{telegram_user_id}`, **re-read on every update** with binding-epoch check, so revocation is observed immediately (mid-flight messages after unpair fail closed).
- **Conversation**: `conversation_model = continuous`; a paired DM chat maps to a durable conversation binding via `pair_external_actor` — thread continuity across messages, matching the #6116 manifest.
- **Outbound**: plain text (no MarkdownV2 escaping in v1 — matches target `supports_markdown = false`), chunked at 4096 chars into sequential `sendMessage` calls. **Honest delivery**: Telegram API error or user-blocked-bot 403 records `Failed`, never optimistic `Delivered` (the T1 channel-lifecycle rule); one retry honoring `retry_after` on 429, then fail honestly. The bot token reaches egress only through the credential handle (URL-path substitution — `path_placeholder` semantics; token bytes never in adapter-visible state or logs).
- **Proactive delivery**: an `OutboundDeliveryTargetProvider` analog of `slack_outbound_targets.rs` registers each paired user's DM `chat_id` (captured at pairing) as a delivery target, so routines/heartbeat/triggers can deliver into Telegram (Slack C9 parity).

## 4. Pairing (WebGeneratedCode)

All pairing state in telegram host state (`/tenant-shared/telegram-pairing/`). The code store is Reborn-side and new — v1's `pairing_requests`/`channel_identities` machinery is **not** reused (wrong direction, wrong identity store), but its conventions carry over where noted.

1. **Issue.** Triggered by Connect on the Extensions card or by an in-chat install that derives `setup_needed`. Preconditions: tenant bot configured, else fail closed with "an administrator must configure the Telegram bot first" (no code minted). Mint: 8 chars from the v1 unambiguous alphabet (`ABCDEFGHJKLMNPQRSTUVWXYZ23456789`), OS CSPRNG, **15-minute TTL**, **single-use**, **one live code per user per installation** — re-request rotates the code and invalidates the prior one (v1 `upsert_pairing_request` semantics). Codes are bound to `(tenant, ironclaw_user_id, bot_id)`.
2. **Present.** One shared panel contract for both surfaces (in-chat blocked card and Extensions card — RC-10 parity), offering a **three-rung fallback ladder** so pairing works regardless of where Telegram lives:
   1. **Deep link** `https://t.me/<bot_username>?start=<CODE>` (8-char code is valid `start` payload) — same-device happy path.
   2. **QR code of that same deep link**, rendered client-side in the panel (presentation only: same URL, same code — no new backend, not the separate QR strategy) — the cross-device path: IronClaw in a desktop browser, Telegram on the phone.
   3. **Manual**: the bot username as searchable copy-text (`@<bot_username>`) plus the copyable code — "open Telegram anywhere, find the bot, send this code." First-class, not an afterthought: some Telegram clients don't reliably re-send a `?start=` payload when a chat with the bot already exists, so the typed rung is the guaranteed one. A "Don't have Telegram?" line covers the no-account case.

   The panel shows the expiry countdown and live connection status (poll or SSE — pinned in planning to whatever the existing channel-connection panel does). **Renewal is self-service on both surfaces**: when the countdown lapses (or anytime), a "Get a new code" action re-requests through the same issue endpoint with step-1 rotate semantics — the in-chat blocked card carries the same affordance as the Extensions panel, and the card is interactive (like the OAuth card), not a static snapshot. **Codes expire; gates don't**: the parked run waits indefinitely (like an OAuth gate) and is keyed by `provider = telegram`, not by any specific code, so renewal never disturbs the blocked thread — pairing with the *n*-th code still resumes it. In chat, the run parks on the existing **`BlockedAuth` gate** with a requirement carrying `provider = telegram`, `requester_extension = telegram`; the display preview renders the pairing card via the `channel_connection_required` output kind with render chrome stripped from model-visible output. No new blocking machinery.
3. **Consume.** The webhook receives, from a private chat, either `/start <CODE>` (deep link) or a bare message exactly matching a live code (typed fallback; case-insensitive, uppercase-normalized). Valid + live + unconsumed ⇒
   - bind `{bot_id}:{telegram_user_id}` → the code's IronClaw user in the identity binding store (**bind, never mint** — channel actors are not mintable, the Reborn identity invariant);
   - record the DM `chat_id` as the user's Telegram delivery target;
   - mark the code consumed;
   - reply "✅ Paired to <display name>. You can talk to IronClaw right here.";
   - dispatch the auth-continuation completion event (provider `telegram`) — the existing `BlockedAuthResumeFanout` resumes every `BlockedAuth` run for that tenant+user whose requirements include provider `telegram`; membership plus the now-satisfied pairing requirement derives `active` without a second lifecycle command;
   - the panel flips to Connected via its status mechanism.
4. **Refuse.**
   - Expired/consumed/unknown code ⇒ static "that code has expired — get a fresh link from IronClaw" reply; no binding; invalid-code attempts rate-limited per chat.
   - Telegram account already bound to a **different** IronClaw user ⇒ explicit refusal ("this Telegram account is already paired to another IronClaw user"); no silent re-bind (`ProviderIdentityAlreadyBound` rule).
   - Same user re-pairing their own account ⇒ idempotent success (fresh confirmation, binding unchanged).
   - `/start` with no payload from an unpaired chat ⇒ the static pairing hint (§3), never a code (codes only originate in IronClaw).
5. **Unpair.** User disconnects from the card, or removes the extension: delete that user's binding + delivery target, bump the binding epoch (in-flight messages fail closed), invalidate any live pairing code. Only that user is affected; the tenant channel keeps running. History preserved.

## 5. Extension lifecycle & the two install surfaces

- `telegram` is **visible** in the user catalog (`available_extensions.rs` entry behind the `telegram-v2-host-beta` feature; **not** in `is_internal_extension_package_ref` — that hidden-companion pattern is the retired `slack_bot` taxonomy).
- Admin setup configures the deployment channel (§2) without installing it for any user. Per-user state is derived from membership plus the **connection** (pairing): member + caller not personally connected ⇒ `setup_needed`; paired member ⇒ `active`.
- **Registry tab**: the card's Connect/Configure action opens the pairing panel (§4.2). Install before admin config fails closed with the admin-config message.
- **In-chat**: search and install establish membership; installing as an unpaired caller parks the run and renders the manifest-driven pairing card. Bot credentials are **never requested in chat** — admin-only surface. (This supersedes the old QA expectation `qa-install:A-4b` of an in-chat bot-token panel.)
- **No tools**: zero tool capabilities declared. "Read my Telegram / send as me" ⇒ honest not-supported answer; a negative test pins the empty tool surface so nothing grows one accidentally before link-device.
- **Remove semantics**: user remove = leave membership and unpair (§4.5), others unaffected. Admin clears setup = channel stops for everyone (fail-closed ingress) and deployment secrets are purged; user memberships, pairings, and history are retained. Removal during a pending pairing invalidates the code (RA-1 analog); removal mid-inbound must not resurrect the channel (RM-H8's in-flight rule).

## 6. Security invariants

- Constant-time webhook secret comparison; fail-closed floor on the public listener (no `IngressAuthPolicy::None`).
- Secrets never echoed: setup status is booleans/handles; safety-layer scan on admin fields; token injected into egress URLs by handle substitution only; redaction covers `/bot<token>/` URL paths in logs and error messages.
- Pairing codes: CSPRNG, 15-min TTL, single-use, rotation-on-reissue, per-chat rate limit on failed attempts, only consumable through the verified webhook (an attacker must control a Telegram account AND hold a live code).
- Unknown users can never start a turn or mint an identity; bindings re-checked per message with epoch semantics.
- Oversized (>1 MiB), malformed, or unverified payloads are rejected before any parse/dispatch.
- All UI-initiated mutations route through the standard dispatch/facade paths (Everything Goes Through Tools; no direct store access in handlers).

## 7. Porting contract (#6116)

**Carries over 1:1** (the behavior contract): every name/route/handle in §1, the admin one-field setup UX, the pairing UX + refusal semantics, DM-only admission, plain-text/4096 presentation, honest-delivery rules, and the entire QA suite (it pins behavior, not implementation).

**Deleted/replaced at port time:**
- `crates/ironclaw_reborn_composition/src/telegram/**` → absorbed by the generic extension runtime.
- Main-schema manifest (`[[product_adapter.inbound.host_ingress]]` shape) → replaced by the v3 manifest that already exists at `f8e7c72c3:crates/ironclaw_first_party_extensions/assets/telegram/manifest.toml` (same id, same handles, same route suffix, same presentation).
- `ironclaw_telegram_v2_adapter` diffs against its #6116 descendant `ironclaw_telegram_extension` (the reconciliation already contains that port); keep main-side adapter changes at zero or near-zero.

**#6116-side follow-up (completed in the fold):** the target derives `WebGeneratedCode` directly from manifest data, with no per-extension branch. Unsupported pasted-proof and QR-only strategies were removed from the public vocabulary rather than retained as unimplemented promises.

**Gates to respect now** (so the fold is mechanical): no retired-taxonomy names in anything new; no `telegram` strings outside the telegram module/crates/inventory (mirror the specificity-gate exemption boundaries even though the gate itself isn't on main).

## 8. Legacy retirement (Reborn scope)

After this feature, the Reborn context — `crates/**`, the webui_v2 frontend, `docs/reborn/**`, `tests/integration/` + reborn-tier root tests, and the reborn live-QA/canary scripts — contains exactly **one** Telegram model: this one. Disposition of the existing Telegram-touching artifacts:

**Rewritten as part of this feature:**

| Artifact | Disposition |
|---|---|
| webui_v2 v1-pairing UI (`pages/extensions/lib/pairing-api.ts`, `pairing-section.tsx` + test, `chat/components/onboarding-pairing-card.tsx` + test, `useExtensions-pairing.test.ts`, telegram paths in `useChannelOnboarding.ts`) | Replaced by the WebGeneratedCode pairing panel + its tests. **Caveat (verify at planning):** these components may also serve v1-mounted webui_v2 flows (`test_reborn_webui_v2_legacy_extensions.py` suggests dual hosting). If a v1 consumer exists, the legacy components stay only for that consumer, clearly quarantined, and no reborn path routes to them; if reborn-only, they are deleted. |
| `docs/reborn/contracts/telegram-v2.md` | Rewritten to the shipped contract (single extension, admin setup, WebGeneratedCode pairing, DM-only). Per house pattern it names its test file + run command and is wired into `scripts/reborn-e2e-rust.sh`. |
| `tests/telegram_v2_default_off_integration.rs` | Replaced by a new-model gating test: the `telegram-v2-host-beta` feature/default posture, and the exclusivity guard still blocking v1 telegram activation when the reborn channel owns the bot. |
| Telegram legs in `tests/reborn_qa_connect_flows.rs`, `tests/staging_regression_fixes.rs`, `crates/ironclaw_reborn_composition/tests/webui_v2_serve.rs`, `crates/ironclaw_webui_v2/tests/webui_v2_handlers_contract.rs` | Rewritten to the new model (pairing connect action, `WebGeneratedCode` strategy payloads). |
| Stale `telegram` references in composition (`extension_host/extension_removal_cleanup.rs`, `extension_host/extension_lifecycle.rs`, `outbound/outbound_preferences.rs`, `root/communication_context.rs`, mention in `slack/slack_actor_identity.rs`) | Reconciled into the new `telegram/**` module — no orphaned pre-feature hooks left behind. |
| `scripts/reborn_webui_v2_live_qa/` telegram cases; `scripts/live-canary` / `scripts/live_canary` telegram registry entries | Updated to drive the new model (admin setup + pairing), becoming its live proof. |
| Telegram-flavored logic in shared reborn crates (`ironclaw_common/src/{platform,event,attachment,identity}.rs`, `ironclaw_product_adapter_registry` fixtures, `ironclaw_wasm_product_adapters` examples) | Inventoried at planning: doc-comments/examples stay; any v1-behavior-bearing logic is aligned to the new model. |

**Kept (v1 monolith, out of scope, removed with monolith retirement):** `channels-src/telegram/`, `tools-src/telegram/` (MTProto), `registry/{channels,tools}/telegram*`, `src/channels/wasm/telegram_host_config.rs` + v1 WASM host paths, v1 pairing machinery (`src/pairing/`, `/api/pairing/{channel}`, `ironclaw pairing` CLI), v1 telegram tests (Rust + Python e2e + `fake_telegram_api.py`), `scripts/telegram_smoke/`, `workflow_canary` v1 telegram scenarios, user-facing v1 docs (`docs/channels/telegram.mdx`, translations).

**Exclusivity guard:** `REBORN_TELEGRAM_V2_ENABLED` + `validate_telegram_v1_v2_exclusivity()` are retained with the env name unchanged (config compat); their documented meaning becomes "the reborn Telegram channel owns the bot — v1 telegram must not activate." Comments/docs updated; a new-model test pins the arbitration.

**No-legacy gate:** planning adds a mechanical check (arch-tier test or CI grep) asserting reborn-context files reference no v1 pairing routes (`/api/pairing/`), no v1 telegram config keys (`wasm_channel_owner_ids`-era), and no bot-issues-code pairing flow — so legacy can't creep back in ahead of the #6116 fold.

## 9. Feature flag & config

- New cargo feature `telegram-v2-host-beta` on `ironclaw_reborn_composition` (module gate + `lib.rs` re-exports) and `ironclaw_reborn_cli` (serve wiring), **declared in every workspace manifest that references it** and threaded through CI aggregate jobs, reborn-e2e, live-canary, and the QA runner flag sets — the S1 merge-hygiene lesson: no undeclared features.
- Serve wiring mirrors `serve.rs` lines ~497–619 + `serve_slack.rs`: build telegram mounts, install facades (connectable channels, channel connection), `with_public_route_mount(telegram_mounts.events)`, `with_telegram_channel_routes(...)`, register the outbound delivery target provider.
- Public base URL: required for `setWebhook`; sourced from the existing deployment public-origin config (same as OAuth callbacks), overridable per §2. Absent ⇒ admin save fails closed with a precise message.

## 10. Testing & QA deliverable

**First artifact: manual QA journey** — new journey **"Use IronClaw in Telegram"** in `~/ironclaw-manual-qa` (manifest.json + validator totals bumped + re-rendered), mirroring the Slack journey's pack structure (~10 packs, ~55–65 tests):

1. `before-telegram-is-connected` — bot unconfigured; configured-but-unpaired DM fails closed + static hint; forged/missing secret header pre/post config; `/start` with no code; group message ignored.
2. `configure-and-connect-telegram` — admin happy path (save → `getMe` → `setWebhook` → first paired inbound answers, no restart); invalid token fail-closed; `setWebhook` failure rollback; token rotation (same bot: pairings survive); bot swap (different bot: pairings orphaned by design); missing public base URL.
3. `install-and-connect-telegram` — registry-tab install→pair; in-chat "set up Telegram" → search/install → blocked pairing card (supersedes old A-4b); dual-surface parity (RC-10 analog); install before admin config fails closed.
4. `pair-a-personal-telegram-account` — deep-link happy path + thread auto-resume; typed-code fallback; expired code; consumed-code reuse; re-request rotates; already-bound-to-other-user refusal; same-user idempotent re-pair; abandon panel and retry cleanly; disconnect/unpair.
5. `messages-and-continuity` — DM happy path; multi-message continuity; long response chunking (>4096); formatting (plain text); inbound media/attachment handled gracefully; `/start` when already paired; unicode/control chars; edited message ignored.
6. `multiple-users-and-isolation` — two users pair to the same bot, no bleed; unknown user fail-closed; per-message binding re-read (revoked mid-flight rejected); same Telegram account cannot bind to two IronClaw users; one user's unpair leaves others untouched.
7. `restart-reconfigure-and-remove` — restart survival (setup, pairings, webhook registration all durable); user remove = unpair only; admin clear = channel stops for all, history retained; reconfigure while active; remove during pending pairing invalidates the code; message after removal (E11 analog).
8. `telegram-failure-and-recovery` — Telegram API down on send → honest `Failed`; duplicate `update_id`; rapid burst/rate limit; user blocks the bot → 403 honest failure; adapter panic isolation; webhook secret rotation; non-message update types ignored.
9. `telegram-ingress-security` — forged/missing secret token header; oversized payload; malformed JSON; replayed update; wrong-extension-id path; secrets never echoed in UI/logs/status.
10. `telegram-delivery-and-no-tools` — proactive delivery into Telegram (routine/heartbeat); channel-initiated action audit parity (`qa-agentturn-tools:3.3` already names Telegram); **negative: no `telegram.*` tools exist**, "send a Telegram message as me" gets an honest not-supported.

Plus rewrite the 3 stale Telegram tests in `connect-and-use-other-integrations/telegram.md` (old per-user bot-token model): `qa-install:A-4b` → pairing-card flow; `qa-remove-reconfigure:RM-H8` → split admin-clear vs user-unpair semantics; functional smoke stays, procedure updated.

**Repo-side (implementation phase, test-first per `.claude/rules/testing.md`):** red-first `tests/integration/` coverage driven through the harness at real seams — webhook → verified ingress → turn; pairing consume → binding → `BlockedAuth` resume; unpaired fail-closed; honest delivery statuses; restart survival through reopened stores; admin save rollback on reconciliation failure. Consolidate into the channel-lifecycle test shapes from #6105/#6113 rather than proliferating new files. Crate-tier only where the integration tier can't reach (justify in the PR).

## 11. Implementation map

**New (all additive, feature-gated):**

| Piece | Mirror of |
|---|---|
| `crates/ironclaw_reborn_composition/src/telegram/mod.rs` (+ submodules below) | `src/slack/mod.rs` |
| `telegram_setup.rs` — setup service, `getMe`/`setWebhook` client, secret handles | `slack_setup.rs` |
| `telegram_host_state.rs` — setup/pairing/binding/target stores under `/tenant-shared/telegram-*` | `slack_host_state.rs` |
| `telegram_channel_routes/` — admin `GET/PUT /setup`, operator auth, safety scan, reconcile-with-rollback | `slack_channel_routes/*` |
| `telegram_serve.rs` — public webhook mount, installation resolver, immediate-ack dispatch | `slack_serve.rs` |
| `telegram_actor_identity.rs` — per-message user resolution, provider `telegram` | `slack_actor_identity.rs` |
| `telegram_pairing.rs` — code store, issue/rotate/consume, continuation dispatch | new (v1 `src/pairing/` conventions, Reborn stores) |
| `telegram_connectable_channel.rs` — admin card + `WebGeneratedCode` connect action | `slack_connectable_channel.rs` |
| `telegram_outbound_targets.rs` — delivery target provider | `slack_outbound_targets.rs` |
| `telegram_host_beta/runtime_setup.rs` — `build_runtime_mounts` composition point | `slack_host_beta/runtime_setup.rs` |
| `assets/telegram/manifest.toml` (main schema, `[[product_adapter.inbound.host_ingress]]`) + `available_extensions.rs` entry | `assets/slack_bot/manifest.toml` (route/auth shape only — visibility differs) |
| `serve.rs` wiring + `WebuiServeConfig::with_telegram_channel_routes` | `serve_slack.rs` |
| Frontend: `telegram-setup-panel.tsx`, `lib/telegram-setup-api.ts`, `WebGeneratedCode` pairing panel, channels-tab branch, i18n keys | `slack-setup-panel.tsx`, `slack-setup-api.ts` |

**Reused untouched:** `ironclaw_telegram_v2_adapter` (`ProductAdapter` impl), `NativeProductAdapterRunner` + `SharedSecretHeaderAuth`, `BlockedAuth` gate + `BlockedAuthResumeFanout`, `channel_connection_required` display path, `builtin.extension_*` tools, `SecretStore`, lifecycle port, conversation binding (`pair_external_actor`).

**Reference files** (read before implementing): `slack_serve.rs`, `slack_host_beta/runtime_setup.rs`, `slack_setup.rs`, `slack_channel_routes/setup.rs`, `slack_actor_identity.rs`, `slack_personal_binding.rs`, `slack_connectable_channel.rs`, `slack_channel_connection.rs`, `slack_host_state.rs`, `conversation_binding.rs`, `blocked_auth_resume.rs`, `extension_lifecycle_capabilities.rs`, `ironclaw_wasm_product_adapters/src/{runner_immediate_ack,auth_verifier}.rs`, `serve.rs`/`serve_slack.rs`, `channels-tab.tsx`, `docs/reborn/contracts/telegram-v2.md`, and `f8e7c72c3:crates/ironclaw_first_party_extensions/assets/telegram/manifest.toml`.

## 12. Planning-time verifications (pin before building)

1. **Gate seam for channel connection**: an in-chat install by an unpaired channel caller must park the run through the manifest-declared channel-connection requirement (provider `telegram`). It ends as `TurnStatus::BlockedAuth` + fanout-resumable; satisfying the gate derives `active` without a public activation action.
2. **Continuation event for pairing**: confirm `AuthContinuationEvent` (or the dispatcher entry point) can be emitted by a non-OAuth completion with provider `telegram` and `AuthContinuationRef::SetupOnly`/`LifecycleActivation` — the fanout itself is provider-keyed and agnostic.
3. **Frontend `web_generated_code` handling**: route this strategy directly from the manifest to the host-generated code/deep-link/QR panel. Pin the completion signal (poll vs SSE) to the existing panel mechanism.
4. **Single-manifest feasibility**: confirm main's manifest schema accepts a visible extension that declares `[[product_adapter.inbound.host_ingress]]` (visibility is only the `is_internal_extension_package_ref` code list — expected yes).
5. **Idempotency seam** for duplicate `update_id` (what exactly dedups Slack retries today — workflow-level accepted-message idempotency vs runner-level).
6. **Existing `telegram` references** in composition (`extension_removal_cleanup.rs`, `slack_actor_identity.rs`, `outbound_preferences.rs`, `communication_context.rs`) — reconcile rather than duplicate.
7. **Where `RebornChannelConnectAction` needs extension** — current shape is input-oriented (`input_placeholder`/`submit_label`); the WebGeneratedCode panel needs code + link + expiry + status fields (additive DTO change in `reborn_services/types.rs` + webui contract test).
8. **webui_v2 v1-pairing component consumers** — determine whether `pairing-section`/`onboarding-pairing-card`/`pairing-api` serve v1-mounted webui_v2 (the `test_reborn_webui_v2_legacy_*` suites suggest dual hosting) before choosing delete vs quarantine (§8).
9. **Shared-crate telegram references** — classify every `telegram` hit in `ironclaw_common`, `ironclaw_product_adapter_registry`, `ironclaw_wasm_product_adapters` as doc-example (keep) vs behavior (align), per §8's inventory rule.

## Appendix: exploration provenance

Grounded in four exploration reports (2026-07-16) over main + `f8e7c72c3` (PR #6116 head): Slack admin/ingress/lifecycle map, the then-current extension install/activation/gate map, Telegram + pairing prior art (three generations), and the #6116 target characterization (v3 manifest, recipes, retired-taxonomy + specificity gates, reference Telegram extension). The public activation terminology in those source snapshots is historical; the superseding contract is the manifest-driven derived lifecycle linked above.
