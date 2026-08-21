# Unified channel model — target architecture

**Status:** Shipped, with one accepted deviation from the original plan.
Originally a target design (approved direction, 2026-08-10); the
channel-adapter contract described here is built (as the split
`ChannelIngress`/`ChannelReply`/`ChannelDelivery` traits, see
`crates/contracts/ironclaw_extension_contracts/src/channel_adapter.rs`) and
this document is still cited as the design rationale from
`delivery_coordinator.rs` and `inbound_turn.rs`. The old shape this design
replaced (channel-specific routes, two inbound cores, bespoke `/web-push/*`
enrollment) is gone — there is no production `/web-push/*` route. **The one
place the shipped code diverges from §3 below:** the web-app channel
(`crates/extensions/packages/web-app/src/channel.rs`) implements only
`ChannelDelivery`. Its authenticated-session inbound and its streaming reply
are host-owned, not adapter-implemented — a deliberate exception (the
adapter may never mint the session's trust, and the reply sink is the
existing SSE/projection path), not leftover migration debt. See the note at
the end of §3.
**Audience:** Any agent touching channel inbound, outbound/reply, notifications,
notification enrollment/setup, or the web-app/web-push surface. **Read this
first** — the old shape (channel-specific routes, two inbound cores, bespoke
`/web-push/*` enrollment) is a known smell and must not be reintroduced.

Owning families: `crates/extensions/` (adapters + host), `crates/contracts/ironclaw_extension_contracts`
(the `ChannelAdapter` + descriptor vocabulary), `crates/product/` (the shared
inbound/reply/notification core). Cross-ref:
`docs/internal/reborn/target-architecture/families/extensions.md`,
`.claude/rules/events.md`, `.claude/rules/safety-and-sandbox.md`.

---

## 0. The hard invariant (read this even if you read nothing else)

**Nothing in the codebase is specific to a given channel — for inbound, outbound,
or notifications. Period.**

- Every HTTP route is **generic and `extension_id`-parameterized**. There is no
  `/web-push/*` route, no web-app-specific message route, no per-channel handler.
- Every host / product / WebUI code path for inbound, reply, notifications, and
  notification setup is channel-agnostic and dispatches by `extension_id`.
- **Channel-specific behavior exists in exactly one place: the channel's
  `ChannelAdapter` in `crates/extensions/packages/<name>/`.** Parsing a protocol,
  rendering a reply, signing a push, storing a push subscription — all adapter.
- This is the existing "no vendor name in generic code" rule
  (`reborn_extension_specificity.rs`) **extended from names to routes and
  operations.** A grep of the generic crates for a channel name (`web_push`,
  `slack`, `telegram`, …) or a channel-specific route must return zero.

If you find yourself adding a route, a handler branch, a config field, or a
store keyed to one channel outside `packages/*`, stop — it belongs behind a
generic contract that the adapter implements.

## 1. The one-sentence goal

**Every channel — web-app, Slack, Telegram — is a `ChannelAdapter` that implements
inbound, outbound/reply, and notifications (including notification *setup*) the
same way. The only per-channel variation is (a) the *entrypoint* it declares
(how a request arrives) and (b) the *delivery/setup capabilities* it declares
(streaming vs batched, max message size, whether notifications need enrollment,
markdown…). Everything between the entrypoint and delivery is one abstract,
channel-agnostic core.**

## 2. Why (the smell being removed)

1. **Two post-ingress cores.** Webhook channels enter
   `ChannelInboundProductSurface::admit_channel_inbound`
   (`crates/contracts/ironclaw_product_contracts/src/surface.rs:76`) →
   `DefaultProductSurface::submit_inbound_inner`
   (`crates/product/ironclaw_assistant/src/workflow.rs:344`); the browser +
   OpenAI-compat enter `ProductSurface::invoke(SUBMIT_TURN_COMMAND)` →
   `RebornServices::submit_turn`
   (`crates/product/ironclaw_assistant/src/reborn_services.rs:3693`). Both run the
   same idempotency → bind → `TurnCoordinator::submit_turn`
   (`crates/kernel/ironclaw_turns/src/coordinator.rs:130`) against different
   ports — a parallel pipeline where one copy always lags
   (`.claude/rules/architecture.md` §4).
2. **The web-app is special-cased on both ends.** Inbound via a bespoke browser
   route; replies via the durable-event/projection stream while Slack/Telegram
   go through the `DeliveryCoordinator`.
3. **Channel-specific routes exist.** `/web-push/{subscribe,unsubscribe,status}`
   (enrollment) and the web-app message route are channel-specific host code —
   exactly what the invariant in §0 forbids.

The web-app should just be a channel; enrollment should just be generic
notification setup; the two cores should be one.

## 3. The `ChannelAdapter` — one contract, all channel behavior

`ChannelAdapter` (`crates/contracts/ironclaw_extension_contracts/src/channel_adapter.rs`)
is the *only* channel-specific code. Every channel (web-app included) implements
it. Its responsibilities:

1. **Inbound** — parse the raw protocol payload into a `NormalizedInboundMessage`.
   Pure, panic-isolated, no host authority (cannot forge trust — §9).
2. **Outbound / reply** — consume the core's abstract reply-event stream and
   deliver it per the channel's declared reply mode (§5).
3. **Notifications (`deliver_notification`)** — the channel-specific *send* of a
   notification (a blocked-automation notice, or any out-of-band notice), gated
   on the `notifications` capability. This is where a channel's notification
   logic lives. It is called by the generic `DeliveryCoordinator` facade (§7a),
   never directly by feature code.
4. **Notification setup** — perform/report the per-user enrollment a channel
   needs before notifications can be delivered (§7b). Web-app: register/remove a
   browser push subscription and report how many browsers are enrolled.

Auth has no adapter (the manifest recipe engine runs it); only channels get an
adapter. The host runs verification, credential injection, binding, idempotency,
turn submission, notification routing, and notification-setup dispatch
generically — the adapter only ever sees already-verified inbound, already-
authorized egress, and its own opaque setup payload.

**Accepted exception: the authenticated-session channel implements only
responsibilities 3 and 4.** The web-app channel's `[channel.ingress]` uses the
`authenticated_session` recipe and `[channel.reply] transport = "stream"`
(manifest, `crates/extensions/packages/web-app/manifest.toml`); for that
combination, responsibilities 1 and 2 above have no adapter code —
`WebAppChannelAdapter` implements only `ChannelDelivery`. Trust for a session
caller is the host transport's own authentication, which an adapter must never
mint from a payload (§9), and the reply is the existing durable
SSE/projection stream, which an adapter is never called to re-implement. Any
future `authenticated_session` + `stream` channel gets the same two
absences; a webhook or batched-reply channel still implements all four.

## 4. Entrypoints are declared, thin, generic

An entrypoint = *how a request arrives*, declared by `[channel.ingress]`. Two
jobs, then hand off to the one core: **establish trust class** + **normalize**
(via `adapter.inbound`). Kinds (via `IngressVerificationRecipe`,
`crates/contracts/ironclaw_extension_contracts/src/recipe.rs`):

| Entrypoint | Recipe kind | Trust | Generic route (parameterized by `extension_id`) |
| --- | --- | --- | --- |
| Webhook | `hmac_sha256` / `shared_secret_header` / `none` | T2 verified-inbound | `/webhooks/extensions/{extension_id}/{route_suffix}` (exists) |
| Authenticated session (POST + bearer) | `authenticated_session` | T1 (host transport verified) | generic session-inbound route keyed by `{extension_id}` — **replaces** the web-app-specific message route |
| API key | `authenticated_session` (API-key → caller projection) | T1 | the transport adapter's wire routes (e.g. OpenAI-compat's `/v1/*`) — a protocol adapter, the allowed home for its own wire shape |

`route_suffix` is present only for webhook entrypoints and forbidden for
`authenticated_session` — already enforced fail-closed
(`ChannelDescriptor::validate`, the webhook verifier). A browser request can
never reach a webhook mount.

## 5. The abstract core (inbound) and the reply model (outbound)

### Inbound core (channel-agnostic)
```
  normalized message + trust class + binding inputs
    → idempotency        (one durable mechanism for all channels)
    → bind               (enum: OwnedThread{thread_id}  — session owns its thread
                                | ExternalRef{...}        — webhook resolves-or-creates)
    → TurnCoordinator::submit_turn
```
`bind` and trust are enum arms, not two implementations
(`SessionCaller` (T1) | `VerifiedInbound` (T2 + installation tenant + external-
actor pairing)). The webhook trust/pairing machinery must never run for a session
channel and vice versa; everything below `submit_turn` is shared unchanged.

### Reply model (symmetric — abstract events, channel-declared delivery)
Turn execution emits **abstract, durable reply events** (`thinking`,
partial/token, tool-activity, `final`) through the event log → projections
(`.claude/rules/events.md`), so replay/reconnect are preserved. The
channel declares a **reply mode**; the adapter's **reply sink** consumes the
(live + replayed) stream per mode:

| Channel | Reply mode | Reply sink behavior |
| --- | --- | --- |
| web-app | `streaming` | forward the live/replayed event stream to the frontend (the existing SSE/WebSocket path *is* this sink) |
| Slack | `batched` | first event → generic "IronClaw is thinking…"; `final` → send, split to `max_message_chars`; duplicate → "your request is processing" |
| Telegram | `batched` | same shape, Telegram rendering + its own `max_message_chars` |

**Load-bearing layering rule:** the reply sink sits *on top of* the durable
event/projection pipeline — a consumer of durable reply events, never a
replacement. The `DeliveryCoordinator` becomes the batched sink's delivery
mechanism, not a separate reply path.

## 6. Channel-declared capabilities (the manifest surface)

The channel declares its behavior; the generic core reads it and adapts:

- **`inbound` / `outbound` / `notifications`** (bool).
- **`ingress`** — the entrypoint (recipe kind + optional `route_suffix`, §4).
  Required iff `inbound`.
- **`egress`** — declared vendor hosts + host-side credential injection for
  outbound/notification delivery (never in the adapter).
- **reply mode** — `streaming` | `batched` (new). Drives the reply sink.
- **`max_message_chars`** — **optional** (evolve `ChannelPresentation.max_message_chars`
  from required-with-default to optional). **Undeclared = unlimited → never
  batch/split** (web-app). Declared (Slack ~1000, Telegram ~2000) → the batched
  sink splits to fit. Streaming ignores it (chunking hint at most).
- **notification setup requirement** (new) — whether notifications need per-user
  enrollment before delivery (§7). Web-app: yes (browser enrollment). Slack/
  Telegram: no (a connected DM is deliverable as-is).
- **`supports_markdown` / `supports_threads`**, **`conversation_model`** — as today.

## 7. Notifications are generic — a send facade + a setup surface, both over the adapter

Two notification concerns. Both are channel-agnostic in every caller and
channel-specific only inside the adapter.

### 7a. Send — a generic, any-caller facade over `ChannelAdapter::deliver_notification`

- **The adapter owns the channel-specific send:**
  `ChannelAdapter::deliver_notification(target, content) -> DeliveryReport`
  (Slack DM, Telegram DM, web-push to enrolled browsers). One method; every
  channel implements it. This is where a channel's notification *logic* lives.
- **The generic facade is the `DeliveryCoordinator`** — already the single send
  path, and already *not* routine-specific: its callers today are the routine
  `TriggeredRunDeliveryDriver` **and** the model's `builtin.outbound_deliver`. It
  resolves the target and dispatches by `extension_id` to that channel's adapter.
  **Delivery is therefore already adapter-based — do not rebuild it.**
- **Any subsystem sends a notification by calling the facade** — routines are one
  caller among several. A "notify the user out-of-band" feature is simply a new
  caller of the *same* facade. Callers decide WHEN and WHAT; they never decide
  HOW and never name a channel (no `slack.…` at a call site — that violates §0).
- Two generic targeting shapes, both → coordinator → adapter:
  `notify_user(user, content)` fans out to the user's configured notification
  channels (the picker set + `resolve_notification_target`, gated on the
  `notifications` capability); `notify(target, content)` targets one channel.
- Mediated, evidence-returning like every outbound: returns the provider ref /
  `DeliveredUnconfirmed`, never a fabricated "sent"
  (`.claude/rules/tool-evidence.md`).
- **Stays generic (NOT in the adapter):** the coordinator, target resolution,
  capability gating, the user's channel preferences. **Moves INTO the adapter:**
  only the per-channel send mechanics (`deliver_notification`).

### 7b. Setup / enrollment is generic (this replaces `/web-push/*`)

Some channels can't deliver a notification until the user has *set them up* for
this account/browser. Web-app needs a browser push subscription; a future channel
might need a token. **This is a generic channel operation, not a per-channel
route.**

- The channel **declares** that notifications require setup, and the adapter
  implements three operations behind a generic contract:
  - **status** — is notification delivery enabled for this user? + opaque details
    (web-app: enrolled-browser count).
  - **enable** — perform setup with a channel-defined, host-opaque payload
    (web-app: `{endpoint, p256dh, auth, user_agent}` → store a subscription).
  - **disable** — tear down (web-app: unsubscribe).
- These are exposed through **one generic surface**, parameterized by
  `extension_id`, and dispatched to the adapter — the same shape as channel
  auth/config setup. Generic code never mentions VAPID, push endpoints, or
  subscriptions; those live only in the web-app adapter + its store.
- **`/web-push/{subscribe,unsubscribe,status}` are deleted** and replaced by this
  generic surface. The subscription store moves behind the adapter.
- The notification-channels picker reads **generic per-channel notification
  status** to drive selectability: a channel that needs setup and isn't set up
  is shown non-selectable with its generic "enable" affordance. (The shipped
  web-app readiness fix — currently keyed on `web-push`/`subscription_count` — is
  an interim; it generalizes to reading this per-channel status, so the picker
  and the frontend carry no channel name either.)

## 8. Routes — all generic, `extension_id`-parameterized

| Concern | Generic route | Replaces |
| --- | --- | --- |
| Webhook inbound | `/webhooks/extensions/{extension_id}/{route_suffix}` | (already generic) |
| Session inbound | generic session-inbound route keyed by `{extension_id}` | the web-app-specific message route |
| Notification setup | generic notification-setup surface keyed by `{extension_id}` (status/enable/disable) | `/web-push/{subscribe,unsubscribe,status}` |
| Reply delivery | none — replies flow through the durable event stream + the adapter reply sink | web-app-specific SSE wiring becomes the generic streaming sink |

No route names a channel. Enforced by extending the specificity gate (§13).

## 9. Trust and security invariants (non-negotiable)

- Trust is established at the entrypoint, once, carried as a typed class.
  Adapters never mint trust (verified-inbound is sealed by
  `reborn_sealed_evidence_mint_ratchet.rs`; session evidence is minted only by
  the host transport).
- A session channel has no webhook mount (validation + verifier fail-closed).
- Tenant/actor come from trusted config, never the payload.
- Reply events are durable before delivery; the reply sink never invents
  un-replayable state (`.claude/rules/events.md`).
- Egress credentials + notification-setup secrets are host-managed; the adapter
  gets references/mediated effects, never raw material
  (`.claude/rules/safety-and-sandbox.md`). Web-app VAPID stays host-seeded.
- Notification-setup payloads are untrusted input: validate + bound before
  storage (the existing web-push endpoint validation moves behind the adapter's
  `enable`).

## 10. What is NOT a channel concern (stays on `ProductSurface`)

The web-app is *also* a rich client: thread list/management, settings, gate
resolution, file browsing, extension management, projections. Those are
`ProductSurface` query/command operations and stay there — Slack/Telegram don't
have them. **Only the message-in / reply-out / notification path of the web-app
becomes the unified channel model.** Notification *setup* is generic (§7), not a
web-app ProductSurface special case.

## 11. The web-app extension (concrete)

One extension, one channel. The extension **id becomes `web-app`** (it is the web
app, and the `web-push`-named ids/routes are being removed regardless — keeping
the string only re-introduces a channel name the invariant forbids):

```toml
id = "web-app"
name = "Web app"

[channel]
id = "web-app"
display_name = "Web app"
inbound = true                  # browser chat (session entrypoint)
outbound = true                 # replies (streaming) + push
notifications = true
conversation_model = "isolated" # browser owns native threads
reply_mode = "streaming"        # NEW — reply sink streams to the frontend
notifications_require_setup = true   # NEW — browser enrollment via the generic setup surface
# max_message_chars: UNDECLARED → unlimited, never batch

[channel.ingress]               # NEW — session entrypoint, no webhook, no route
method = "post"
[channel.ingress.verification]
kind = "authenticated_session"

[[channel.egress]]              # outbound push (VAPID, host-signed) — unchanged mechanics, now behind the adapter
host = "fcm.googleapis.com"     # (+ mozilla, apple)
injection = { type = "vapid_authorization" }
```

Slack/Telegram manifests gain `reply_mode = "batched"`; their `max_message_chars`
becomes the declared split bound; `notifications_require_setup = false`.

## 12. Migration from today (concrete deltas)

Historical record of the steps taken to ship §0–§11 — kept for the same
reason a changelog is kept, not as an open TODO list. Step 10 (deleting
`/web-push/{subscribe,unsubscribe,status}`) is done — no such route exists
in production. Re-verify any individual step against the named files before
treating it as still open; do not read this section as a live plan.

Inbound unification:
1. Generalize the inbound entry DTO (`ChannelInboundSurfaceRequest` /
   `ProductInboundEnvelope`): `evidence` → trust-class enum
   (`VerifiedInbound` | `SessionCaller`); binding inputs → enum
   (`ExternalRef` | `OwnedThread`). (`workflow.rs:282`, `conversation_binding.rs:430`.)
2. Route the browser + OpenAI-compat through the one core: re-plumb
   `RebornServices::submit_turn` (`reborn_services.rs:3693`) to build the neutral
   request and call `admit_channel_inbound` / `submit_inbound_inner`; collapse
   the two idempotency mechanisms onto the durable ledger.
3. Delete the duplicate tail (`RebornServices::submit_turn` body + its
   `AcceptedWebUiMessage` / `replay_webui_send_message` / … helpers). The command
   constant stays; only its implementation re-routes.
4. Replace the web-app-specific message route with a generic session-inbound
   route keyed by `extension_id`.

Reply/outbound unification:
5. Give the web-app a `ChannelAdapter`: inbound normalization; a `streaming`
   reply sink that *is* the current SSE/projection forward; the web-push egress.
6. Introduce reply-mode + reply-sink; Slack/Telegram batched sink via
   `DeliveryCoordinator`, split by `max_message_chars`, thinking indicator.
7. Make `max_message_chars` optional and load-bearing for the batched sink.

Notification generalization:
8. Send facade: make `ChannelAdapter::deliver_notification(target, content)` the
   channel's notification send, and have the `DeliveryCoordinator` (the generic
   facade — already called by `TriggeredRunDeliveryDriver` + the model tool)
   dispatch to it by `extension_id`. Expose the facade so any subsystem can send
   (`notify_user(user, content)` / `notify(target, content)`). Delivery is
   already adapter-based — this is naming/exposing the facade + the adapter
   method, not a rebuild.
9. Setup: add the generic notification-setup surface (status/enable/disable by
   `extension_id`) dispatching to the adapter; move the web-push subscription
   store + VAPID + endpoint validation behind the web-app adapter.
10. **Delete `/web-push/{subscribe,unsubscribe,status}`**; point the frontend at
    the generic surface; generalize the picker's readiness (per-channel status,
    no channel name in the frontend).

Cleanup:
11. Rename the extension id/package/crate/route constants `web-push` → `web-app`
    and remove the `WEB_PUSH_*` id constants (the routes they named are gone).
    Internal crate rename (`ironclaw_web_push*`) is mechanical and may trail the
    behavior in the same PR.

Order: 1–4 (inbound, highest-traffic path — test-first against the existing
send-message/inbound contract, preserve caller-owns-thread + `client_action_id`
replay + no thread auto-creation), then 5–7 (reply), then 8–10 (notifications:
send facade, setup, delete bespoke routes), then 11 (rename).

## 13. Enforcement (so future agents can't regress it)

- Extend `reborn_extension_specificity.rs` (or a sibling gate) so the generic
  crates (`ironclaw_webui`, `ironclaw_assistant`, `ironclaw_extension_host`,
  composition) contain **no channel name and no channel-specific route** —
  `web_push`/`web-push` in generic code becomes a zero-occurrence assertion, like
  the retired-taxonomy gate.
- The generic notification-setup + session-inbound routes are asserted generic
  (parameterized by `extension_id`), with no per-channel handler.

## 14. Non-goals

- **No behavior change to what a turn does** — this unifies *transport/channel
  plumbing* around the same `TurnCoordinator::submit_turn`.
- Reply-mode taxonomy starts at `streaming | batched`; richer delivery
  (edit-in-place, thinking-but-batched) is expressed as declared capabilities,
  not new pipelines.
- Exact reply-event vocabulary is pinned against the existing
  `WebChatV2EventFrame` projection so the streaming sink is a no-behavior-change
  wrapper on day one.
