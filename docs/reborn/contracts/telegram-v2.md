# Telegram Channel (Reborn host)

> **SUPERSEDED (unified extension runtime, NEA-25).** The host architecture this
> document describes — the `ironclaw_channel_host` / `ironclaw_channel_delivery`
> vendor-neutral host crates, the `ironclaw_wasm_product_adapters` adapter
> runtime, and composition-owned Telegram mounting — was deleted with the V3
> manifest-driven extension runtime. Telegram now ships as an ordinary
> extension package (`crates/ironclaw_first_party_extensions/assets/telegram/`
> plus its `ChannelAdapter` in `crates/ironclaw_telegram_extension`), installed,
> readiness-derived, and dispatched by the generic runtime; see
> `docs/reborn/extension-runtime/overview.md`. This document is retained as the
> historical design record of the pre-runtime Telegram host; read it for the
> shape of the flows, not for current crate or type names.

**Status:** Superseded — see banner above. Historical text below describes the
retired host wiring: the CLI required runtime Telegram enablement —
`[telegram].enabled = true` /
`IRONCLAW_REBORN_TELEGRAM_ENABLED=true` — before the routes were mounted.
Supersedes the issue #3285 webhook-only
tracer bullet this document previously described (see "What changed from the
tracer-bullet contract" below).
**Host crate:** `crates/ironclaw_telegram_extension/` (the Telegram host domain and
facade-shaped host builder); composition keeps the thin extraction/mount/registration
adapter in `crates/ironclaw_reborn_composition/src/telegram/telegram_host_beta.rs`.
Vendor-neutral host contracts live in `crates/ironclaw_channel_host/`; generic live and
triggered delivery algorithms live in `crates/ironclaw_channel_delivery/`.
**Adapter crate:** `ironclaw_telegram_v2_adapter` (reused unchanged).
**Adapter runtime:** `ironclaw_wasm_product_adapters` (`NativeProductAdapterRunner`).
**Parent contract:** [`product-adapters.md`](product-adapters.md).
**Design spec:** `docs/superpowers/specs/2026-07-16-telegram-extension-design.md`.

## One extension, no tools

Telegram ships as a single user-visible extension. There is no hidden
operator companion (the retired Slack `slack_bot` model-B split) and no
`telegram.*` tool surface — Telegram is an **entrypoint only**; IronClaw
cannot read Telegram or send as the user. Anything shaped like
`telegram_bot` / `telegram_personal` / `telegram_channel` is retired
taxonomy, pinned to zero by
`crates/ironclaw_architecture/tests/telegram_extension_gates.rs`.

| Concept | Value |
|---|---|
| Extension id / package ref | `telegram` (visible in the user catalog; not `is_internal_extension_package_ref`) |
| Bot token credential handle | `telegram_bot_token` (stored revision-suffixed: `telegram_bot_token_<attempt-salted-hash>_v<revision>`) |
| Webhook secret credential handle | `telegram_webhook_secret` (same revision-suffixed shape; minted server-side, never operator-supplied) |
| Identity provider | `telegram` |
| Provider user key | `{installation}:{telegram_user_id}` → `tg-bot-<bot_id>:<telegram_user_id>` |
| Adapter id | `telegram_v2` |
| Actor kind | `telegram_user` (`TELEGRAM_USER_ACTOR_KIND`) |
| Adapter installation id | `tg-bot-<bot_id>` (from `getMe`) |
| Webhook route | `POST /webhooks/extensions/telegram/updates` (route id `telegram.updates`) |
| Admin routes | `GET/PUT/DELETE /api/webchat/v2/channels/telegram/setup` |
| Pairing routes | `POST/GET/DELETE /api/webchat/v2/channels/telegram/pairing` |
| Connect strategy | `RebornChannelConnectStrategy::WebGeneratedCode` |
| Tools | none (negative-pinned) |

The manifest is
`crates/ironclaw_first_party_extensions/assets/telegram/manifest.toml`
(`[[product_adapter.inbound.host_ingress]]` declares the route descriptor;
required credentials are the two handles above; egress is
`api.telegram.org` keyed by `telegram_bot_token`).

## Admin setup (`/api/webchat/v2/channels/telegram/setup`)

Operator-managed, one bot per deployment
(`setup/service.rs`, `setup/compensation.rs`, `channel_routes.rs`):

- **Authorization:** cross-tenant caller → 404 (anti-enumeration);
  same-tenant non-operator → 403. The optional `webhook_url` field passes
  the safety-layer admin-field scan before use.
- **`PUT` body:** `bot_token` (secret; blank/absent means "keep existing")
  and `webhook_url` (optional https override; default derives from the
  deployment public base URL plus the pinned route path).
- **Save pipeline** (fail-closed; nothing persists unless every step
  succeeds):
  1. `getMe` validates the token and captures `bot_id` + `bot_username`
     (persisted non-secret; deep links need the username, identity scoping
     needs the id).
  2. A **fresh webhook secret is minted per save revision** (OS CSPRNG).
  3. `setWebhook` uses the normalized `webhook_url` override when supplied;
     otherwise it derives
     `<public-base>/webhooks/extensions/telegram/updates`. In both cases it
     sends the fresh `secret_token` and `allowed_updates = ["message"]`. The
     default path is pinned to the unified-extension-runtime shape so
     registrations held server-side by Telegram survive the #6116 port with
     zero re-registration.
  4. Secrets persist under revision-suffixed, attempt-salted handles in the
     shared `SecretStore`; the setup record publishes through bounded CAS.
     Concurrent losers clean up only their own attempt's handles.
  5. The deployment channel host reconciles the new configured revision. A
     reconciliation failure rolls the record back to the previous save so
     stored administrator configuration and the mounted ingress never
     split-brain. This does not install Telegram for the administrator or any
     member; user membership remains independent.
  - Invalid token, rejected `setWebhook`, or missing public base URL all
    fail closed with a precise admin error; nothing is half-configured.
- **`GET`** returns a redacted status only
  (`configured`, `bot_username`, `bot_token_configured`, `webhook_url`,
  `revision`) — raw secrets are never echoed by any surface.
- **`DELETE`** first persists a fail-closed `clearing` lifecycle record, then
  requires confirmation of `deleteWebhook` and deletion of both secret handles
  before publishing a `cleared` tombstone. Any failure leaves enough durable
  metadata for a later request (including after restart) to retry cleanup.
  **Pairing records and history are retained** — ingress simply fails closed
  until the deployment is reconfigured; if the same bot returns, existing
  pairings work again.
- **Bot-swap semantics:** the installation identity is the bot
  (`tg-bot-<bot_id>`). Rotating the same bot's token bumps the revision
  (and webhook secret) but keeps the installation id, so pairings survive;
  pointing the deployment at a different bot re-scopes the installation and
  orphans prior pairings by design.

## Ingress (`POST /webhooks/extensions/telegram/updates`)

`ingress/route.rs` and `ingress/resolver.rs` compose the public webhook without binding listeners
(the host mounts the fragment through the WebUI public-route seam):

- **Descriptor is manifest-projected.** The route id, path, method, and
  policy come from the bundled manifest's
  `[[product_adapter.inbound.host_ingress]]` block via
  `host_ingress::bundled_host_ingress_descriptors`, and the axum mount is
  built from that descriptor — what axum serves cannot drift from what the
  manifest declares. Policy: `public_webhook` listener class, fail-closed
  auth floor (`required` / `webhook_signature`), host-resolved scope,
  1 MiB body limit, 12000/60s declared rate limit, `public_callback`
  audit, `product_workflow` effect path.
- **Verification first:** the `X-Telegram-Bot-Api-Secret-Token` header is
  compared in constant time (`SharedSecretHeaderAuth`) against the current
  revision's minted secret. Missing/wrong header → 401, no turn. An
  unconfigured deployment resolves no installation → 401. The dynamic
  resolver re-reads the setup record on every update (WebUI setup changes
  take effect on the next webhook, no restart) and caches the built
  verifier/adapter/runner chain per setup revision.
- **Rate limiting:** a per-installation token bucket (120/60s) applies
  after verification → 429.
- **Immediate-ack dispatch:** Telegram gets its 200 immediately; the turn
  runs async through `NativeProductAdapterRunner` wrapping the unchanged
  `TelegramV2Adapter` (2s intake timeout covering auth/parse/stamp/submit
  only, 64 in-flight cap).
- **DM-only admission** (the pairing-aware pre-router wraps the runner):

  | Verified update | Outcome |
  |---|---|
  | non-private chat (group/supergroup), `channel_post`, `edited_message`, sender is a bot | ignored — no turn, no reply |
  | private DM from an **unpaired** sender | fail closed, no turn; the bot replies with a **static** throttled pairing hint (never LLM-generated, at most once per chat per throttle window) |
  | private `/start <CODE>` or a bare message exactly matching a live code | pairing consume (below); `/start` is authorized by the Telegram manifest rather than built into the generic parser |
  | private bare `/start` (no payload) | paired sender: silent ack — no turn, no reply (re-opening the chat must not pitch pairing); unpaired sender: the static throttled pairing hint; pairedness-lookup outage: silent ack |
  | private text from a **paired** sender | workflow turn (continuous conversation) |

- **Identity per message:** `telegram_actor_identity.rs` resolves
  provider `telegram`, key `tg-bot-<bot_id>:<telegram_user_id>`, re-read on
  every update with the binding-epoch check — revocation is observed
  immediately (in-flight messages after unpair fail closed).

## Pairing (WebGeneratedCode)

Direction is web→Telegram: IronClaw issues the code; the bot never does
(`pairing/code.rs`, `pairing/service.rs`, `pairing/status.rs`):

- **Issue** (`POST /api/webchat/v2/channels/telegram/pairing`, any
  authenticated same-tenant member, self-scoped): fails closed when no
  admin setup exists ("an administrator must configure the Telegram bot
  first" — no code is ever minted first). Mints an **8-character** code
  from the unambiguous alphabet `ABCDEFGHJKLMNPQRSTUVWXYZ23456789`
  (OS CSPRNG), **15-minute TTL**, **single-use**, one live code per user
  per installation — re-request **rotates** the code and invalidates the
  prior one. Response carries `code`, the deep link
  `https://t.me/<bot_username>?start=<CODE>`, and `expires_at`; the panel
  renders the link, a QR of the same link, and the copyable code/username.
- **Status** (`GET`): `{ connected, pending? }` for the caller.
- **Consume** (over the verified webhook, private chat only): `/start
  <CODE>` (because Telegram declares `inbound_code_prefixes = ["/start"]`)
  or a bare live code, case-insensitive (uppercase-normalized). Undeclared
  command prefixes remain ordinary inbound text.
  Valid + live + unconsumed ⇒ bind
  `tg-bot-<bot_id>:<telegram_user_id>` → the code's user (**bind, never
  mint** — channel actors are not mintable), record the DM `chat_id` as the
  user's delivery target, mark the code consumed, confirm in-chat, and
  dispatch the auth continuation.
  - Telegram account already bound to a **different** user ⇒ explicit
    `AlreadyBoundToOtherUser` refusal; no silent re-bind and the rightful
    user's code remains live.
  - The code claim is atomic: concurrent webhook consumers produce exactly
    one pairing winner. A code presented under a foreign authenticated
    installation is indistinguishable from an unknown code and cannot burn it.
  - Same user re-pairing ⇒ idempotent success, binding unchanged.
  - Expired/consumed/unknown/malformed ⇒ `ExpiredOrUnknown`; no binding,
    no continuation dispatch; failed attempts are rate-limited per chat.
- **Disconnect** (`DELETE`, or extension remove): removes the caller's
  binding + DM target and invalidates any live code; only that user is
  affected; history retained. It also removes the external actor's current
  conversation pairing, so the same chat receives a fresh thread if the user
  later pairs it again.
- **Blocked-run resume:** consume durably records and dispatches an
  `AuthContinuationEvent` with `provider = telegram` and
  `AuthContinuationRef::LifecycleActivation`; the lifecycle continuation slot
  reconciles activation, then the standard `BlockedAuthResumeFanout` resumes
  every `BlockedAuth` run parked for that tenant+user on provider `telegram`.
  **Codes expire; gates
  don't** — the parked run is provider-keyed, not code-keyed, so pairing
  with the n-th rotated code still resumes it.

## The in-chat gate

In-chat `builtin.extension_install` with an unpaired caller joins membership
and parks the run
(`extension_host/extension_lifecycle.rs`):

- Telegram declares a provider-neutral account-setup descriptor with
  `provider = telegram`, `setup = RuntimeCredentialAccountSetup::Pairing`, and
  `requester_extension = telegram`. Generic lifecycle looks up that descriptor and its
  connection-status source through the `ExtensionId`-keyed account-setup registry. A
  disconnected caller receives the descriptor's `RuntimeCredentialAuthRequirement` and
  the run parks as `TurnStatus::BlockedAuth`; an unmounted/unregistered required setup
  fails closed instead of parking a run nothing can resume.
- The auth-prompt projection maps `Pairing` to
  `AuthPromptChallengeKind::Pairing`, which renders the same pairing panel
  the Extensions card uses (dual-surface parity). Bot credentials are
  **never requested in chat** — admin setup is a WebUI-only surface.
- The resumed run recomputes the requirement list; a paired caller yields
  none and the derived public state becomes `active` (the self-correcting
  `BlockedAuth` shape). There is no separate activation command or state.
- The connectable-channels facade (`telegram_connectable_channel.rs`)
  shows the operator an `admin_managed_channels` bot-setup card and every
  same-tenant member a `web_generated_code` pairing row once the bot is
  configured; per-caller pairedness and disconnect go through the
  channel-connection facade under the `"telegram"` key.

## Outbound & honest delivery

Egress targets only the declared `api.telegram.org` host; the bot token
travels as an opaque credential handle substituted into the URL path by the
mediated host egress (`{telegram_bot_token}` placeholder — token bytes never
appear in adapter-visible state, composition inputs, or logs).

Final replies render as plain text. Replies over Telegram's 4096-UTF-16-unit
message cap split into **ordered lossless `sendMessage` chunks** (never inside
a character), sent sequentially; a mid-sequence failure stops the remaining
chunks and records ONE honest failure status for the attempt — already-sent
chunks stand, and the attempt is never reported `Delivered` over a partial
reply. **Once any chunk has been delivered, that failure is terminal for the
attempt (`FailedPermanent`)** — automatic re-delivery restarts from chunk
zero and would duplicate user-visible text; only a first-chunk failure (which
delivered nothing) keeps the retryable/unauthorized mapping below. The
adapter's `DeliveryStatus` mapping is the honesty contract:

| Telegram response | DeliveryStatus |
|---|---|
| 2xx | `Delivered` |
| 5xx, 429 | `FailedRetryable` (the mediated egress first honors ONE declared `retry_after` ≤ 5s with an in-place resend; a longer flood wait or a second 429 surfaces immediately) |
| 401, 403 (user blocked the bot / token revoked) | `FailedUnauthorized` |
| other 4xx (e.g. 400), render errors | `FailedPermanent` |
| progress (typing) when not advertised, projection/keep-alive payloads | `Deferred` |

**Blocked-run prompts render — they are never deferred.** A run that parks
`BlockedAuth` with a link-shaped challenge gets its `AuthPrompt` delivered as
a plain-text `sendMessage` carrying the authorization URL (tap → browser →
provider consent → the OAuth callback resumes the parked run; nothing secret
enters the chat). Credential-entry challenges never reach the adapter — the
shared delivery driver's deny arm cancels the run and posts the
"set this up in the web app" notice directly. A `BlockedApproval` run's
`GatePrompt` renders with copy advertising the in-chat reply plus the web-app
fallback. In-chat gate commands (`approve`/`deny`/`approve gate:<ref>`/`deny
gate:<ref>`/`auth deny gate:<ref>`) parse through the channel-neutral grammar
in `ironclaw_product_adapters::interaction_commands` — the same grammar Slack
uses and the same commands the shared busy hints advertise; drift guards
round-trip the advertised copy through the parser at both the driver and
adapter tiers. Both prompts ride
the same chunking + honesty mapping above and record `Delivered` with the
originating `run_id`. (Regression 2026-07-17: the adapter used to record
these `Deferred`, so an auth-gated DM watched "thinking…" get deleted and
then silence.)

A failed send is recorded as failed — never optimistic `Delivered`. Paired
users' DM `chat_id`s (captured at consume time) are the deployment's
Telegram delivery targets for proactive sends.

**Status messages are wired.** The host-authored notices the delivery
machinery posts around the adapter render path — the working message
("Ironclaw is thinking..."), busy-thread hints, and blocked-run
approval/auth notices — ride the same policy-scoped egress as plain-text
`sendMessage` calls (`TelegramDeliveryProtocol::post_status_message`), and
the returned `message_id` handle lets the observer clean up its working
message via `deleteMessage` once the reply lands. Rejections map to a
stable `StatusMessage` error (HTTP status only; Telegram's free-text
`description` stays a bounded debug diagnostic).

## Durable host state

All host state lives in the concrete `state/` owner on the tenant-scoped filesystem plane,
restart-safe and CAS-guarded:
`/tenant-shared/telegram-setup/installation.json`,
`/tenant-shared/telegram-pairing/{codes,users}`,
`/tenant-shared/telegram-binding/{identities,users}`,
`/tenant-shared/telegram-dm-targets`.

## Adapter contract (unchanged)

The `ironclaw_telegram_v2_adapter` parse/render contract this document
previously froze still holds and is still pinned by the adapter's own
tests: reference normalization (`update_id` → `ExternalEventId`
`tg-<installation>-<update_id>`; `message.from.id` → actor ref of kind
`telegram_user`; chat/topic → conversation ref excluding
`reply_target_message_id` from the fingerprint), attachment descriptors
(`file_id`/mime/filename/size only — no downloads, no `source_url`, no
local paths, no raw bytes), duplicate `update_id` idempotency
(`ProductInboundAck::Duplicate` — duplicates stay 200 with no side
effects), and refusal to parse unverified auth evidence. The adapter's
group/supergroup trigger logic exists but is moot at the host tier: the
host admits private-chat messages only and wires `recognized_commands`
empty. `ExternalProgressPush` remains opt-in and is wired off.

## Exclusivity guard (v1 monolith arbitration)

`REBORN_TELEGRAM_V2_ENABLED` and
`ironclaw::config::validate_telegram_v1_v2_exclusivity` are retained with
the env name unchanged (config compat). Their meaning is now: **the Reborn
Telegram channel host owns the deployment bot — the v1 monolith Telegram
WASM channel (`channels-src/telegram`) must not activate for the same
installation.** The validator runs at env-resolve time and again at runtime
startup with the persisted-active channel set, and fails startup closed
when both would handle the same bot. The v1 channel keeps working (default
posture) until the monolith retires; this guard is the collision arbiter
while both exist.

## What changed from the tracer-bullet contract

The previous revision of this document described the #3285 first slice: the
adapter proven against recorded payloads and fake Reborn services, with no
mounted route and `REBORN_TELEGRAM_V2_ENABLED` gating hypothetical traffic.
Deltas now shipped:

- **Host wiring exists.** Admin setup, manifest-projected public webhook
  mount, pairing, identity binding, the in-chat `BlockedAuth` pairing gate,
  revision workflow and trigger decorator, and the WebUI facades live in
  `crates/ironclaw_telegram_extension/`, mounted/registered by
  `crates/ironclaw_reborn_composition/src/telegram/telegram_host_beta.rs`;
  the services below the workflow facade are real, not fakes.
- **The webhook route is pinned** to
  `/webhooks/extensions/telegram/updates` (the unified-extension-runtime
  path), registered with Telegram by the setup pipeline itself.
- **Pairing is IronClaw-issued** (WebGeneratedCode), DM-only admission,
  single `telegram` extension, no tools — the group-trigger and
  progress-push adapter capabilities stay dormant.
- **`REBORN_TELEGRAM_V2_ENABLED` is retained but re-pointed** at this
  implementation (see the exclusivity section above); it is no longer a
  tracer-bullet toggle.
- The deferred `ac<N>_*` acceptance-suite port from the tracer era is
  superseded by the host-tier coverage below; the recorded payload fixtures
  under the adapter crate's `tests/fixtures/` remain valid adapter-tier
  inputs.

## Test coverage

| Surface | Test location | Run |
|---|---|---|
| Setup pipeline (fail-closed order, rollback, rotation, clear), pairing state machine (issue/rotate/consume/refusals/unpair/continuation), Bot API envelopes, webhook auth/rate/errors, revision replacement, trigger-cache behavior, gate requirement shape | `crates/ironclaw_telegram_extension/src/` focused module tests, composition `telegram_host_beta_tests.rs` + `extension_host/extension_lifecycle.rs`, `crates/ironclaw_reborn_composition/tests/webui_v2_serve.rs` | `cargo test -p ironclaw_telegram_extension` + `cargo test -p ironclaw_reborn_composition telegram` |
| Retired-taxonomy zero + no v1 pairing-route literals in the Reborn context | `crates/ironclaw_architecture/tests/telegram_extension_gates.rs` | `cargo test -p ironclaw_architecture --test telegram_extension_gates` (wired into `scripts/reborn-e2e-rust.sh` architecture group) |
| v1↔v2 exclusivity arbitration + default-off posture | `tests/telegram_v2_default_off_integration.rs` (root crate) | `cargo test --test telegram_v2_default_off_integration` |
| Adapter parse/render/idempotency/delivery mapping | `ironclaw_telegram_v2_adapter` per-module `mod tests` | `cargo test -p ironclaw_telegram_v2_adapter --lib` |

Live proof rides the Reborn WebUI v2 live-QA lane
(`scripts/reborn_webui_v2_live_qa/`), whose Telegram cases drive admin
setup + pairing against a live bot and stay gated on live credentials.
