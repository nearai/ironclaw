# Web Push notifications for the web app (PWA)

**Date:** 2026-08-08 · **Branch:** `webapp-push-notifications` · **Status:** In progress

## Goal

Make the web app a real, selectable notification route for automations —
parity with Slack/Telegram — plus PWA installability. Users can pick any
combination of Slack / Telegram / Web app (or none) in the automations page's
notification-channels panel, and the web app route delivers actual browser
push notifications (W3C Push API: RFC 8030 transport, RFC 8291 encryption,
RFC 8292 VAPID) through a service worker, including when the app is closed.

## What exists today (verified against HEAD 30ae2d50f6)

- The notification-channels panel (`crates/product/ironclaw_webui/frontend/src/pages/automations/components/notification-channels-panel.tsx`)
  edits one account-wide `CommunicationPreferenceRecord.notification_targets`
  set (cap 8) via `GET/POST /api/webchat/v2/outbound/notification-channels`.
  Rows come from the owner-scoped outbound target catalog
  (`OutboundDeliveryTargetProvider` registry in `ironclaw_outbound`).
- "Web app" today is only a helper string shown when the set is empty
  (`automations.notificationChannels.webOnlyHelper`); nothing is delivered.
  The old `builtin:web_app` pseudo-target was deliberately deleted in #7157
  (it was an opt-out sentinel, not a destination) and
  `tests/integration/outbound_target.rs` pins that id as non-addressable.
- Two-lane delivery contract: lane 1 = final reply always lands in the run
  thread; lane 2 = model-called `builtin.outbound_deliver` to one catalog
  target; lane 3 = host-emitted background-run notices (gate/auth/failure —
  **not** Completed) fanned over the notification-channel set by
  `TriggeredRunDeliveryDriver` → `DeliveryCoordinator` → `ChannelAdapter.deliver`.
- PWA manifest + maskable icons already ship (`frontend/public/assets/site.webmanifest`,
  pinned by `root-paths.test.ts`); there is no service worker and no push code.

## Decision

Web push becomes a **bundled channel extension** (the repo's canonical shape:
"adding a channel means adding one capability surface of an extension … never
per-channel host code"), deployment-bound like Telegram, with the protocol
mechanics in a new domain crate. It is a genuinely external destination
(egress to browser push services), so it does not revive the retired
`builtin:web_app` in-app pseudo-target.

### New crates

1. **`crates/domains/ironclaw_web_push`** (family: domains, layer: substrates)
   - `PushSubscriptionRecord` grammar + `WebPushSubscriptionStore`: typed
     wrapper over `ScopedFilesystem` + `cas_update` (per database.md; one
     JSON doc per tenant/user, endpoint-unique entries, bounded count).
   - RFC 8291 `aes128gcm` payload encryption + RFC 8188 framing and the
     compact ES256 JWS builder for VAPID — all via `aws-lc-rs` (already in
     the lockfile via jsonwebtoken; zero new heavy deps): ECDH_P256,
     HKDF_SHA256, AES_128_GCM, EcdsaKeyPair. Deterministic-key seams for the
     RFC 8291 Appendix A test vector.
   - VAPID key material generation (PKCS#8) producing the generic
     `vapid_authorization` credential-material JSON (below). No secret
     persistence here; composition seeds/stores it.
   - Push endpoint allowlist (exact hosts, matching the manifest egress
     declarations) + subscribe-time endpoint validation. FCM
     (`fcm.googleapis.com`), Mozilla (`updates.push.services.mozilla.com`),
     Apple (`web.push.apple.com`). Windows WNS uses per-tenant subdomains and
     is out of scope v1 (documented, validated at enroll time with a clear
     error).
   - Pure push request builder: `(subscription, payload, ttl, urgency)` →
     `{host, path, headers, ciphertext}`. No HTTP, no network/secrets deps —
     the domains-family "no transport sends" rule holds.
   - `OutboundDeliveryTargetProvider` impl over the store: one owner-scoped
     entry per user ("Web app — browser notifications"), channel `web-push`,
     capabilities `{final_replies: true, gate_prompts: true, auth_prompts: true}`,
     destination = codec-grammar binding ref. Listed regardless of enrolled
     device count (frontend surfaces the device state).

2. **`crates/extensions/packages/web-push`** (`ironclaw_web_push_extension`,
   layer: products)
   - `manifest.toml` (v3): `[channel]` outbound-only (no ingress section, no
     connection/pairing — the WebUI session is the identity), `[[channel.egress]]`
     entries for the exact push-service hosts with
     `injection = { type = "vapid_authorization" }` and tight body caps.
   - `WebPushChannelAdapter`: `deliver()` loads the target user's
     subscriptions (via the runtime slot, below), encrypts per subscription
     through the domain crate, sends one `RestrictedEgress` POST per
     subscription (TTL 24h; Urgency high when an `AuthPrompt` part is
     present), classifies outcomes (≥1 accepted ⇒ `Sent { vendor_message_ref: None }`,
     404/410 prunes the subscription, all-retryable ⇒ `Retryable`, else
     `Permanent`), renders `AuthPrompt` parts via `render_channel_auth_prompt`.
     `inbound()` returns `ChannelError::Unsupported`.
   - `WebPushPreferenceTargetCodec`: binding-ref grammar; every target is a
     personal direct message (admits OAuth/auth prompts), actor = the user id.
   - Package dep set = the channel-package four **plus `ironclaw_web_push`** —
     justified the same way memory provider packages reach their domain crate:
     the vendor-side state (the subscription registry — web push's analogue of
     Slack's workspace) is host-local by nature.
   - Runtime slot: the adapter is constructed by the CLI before storage
     exists, so it holds a `WebPushRuntimeSlot` (from the domain crate) that
     composition fills with the store handle at assembly (same late-bind shape
     as the buffered trigger-poller hook). Until filled, deliver fails closed
     with `ChannelError::Configuration`.

### Generic host change (one, standard-based)

A new declared egress credential-injection kind **`vapid_authorization`**
beside `header`/`query`/`path_placeholder`: the host transport resolves the
channel secret (JSON `{es256_private_key_pkcs8_b64url, public_key_b64url, subject}`),
computes `Authorization: vapid t=<ES256 JWT aud=<scheme://host of this request>,
exp=now+12h, sub=subject>, k=<public_key>` per RFC 8292, and injects it
host-side. The adapter never sees key bytes (it cannot even name the
`Authorization` header — `EgressHeader` forbids it). This is recipe
vocabulary for an IETF-standard auth scheme, not vendor code, implemented
once in the generic transport with `aws-lc-rs`.

### Wiring

- CLI binding table (`native_extensions.rs`): third `ChannelExtensionBinding`
  (adapter + codec); slot handed to composition input. CLI dep-list
  architecture pin + `crates/app/ironclaw_cli/AGENTS.md` sentence updated.
- Composition: seed the VAPID key material on boot when absent (generated by
  the domain crate, stored where channel egress credentials resolve from);
  build the subscription store on the filesystem plane; fill the slot;
  `MutableOutboundDeliveryTargetRegistry::register_provider("web-push", …)`.
- Assistant: `web_push.subscribe` / `web_push.unsubscribe` product commands +
  a `web_push_status` view (VAPID public key, enrolled-device summary),
  following the `set_notification_channels` descriptor/service shape.
- WebUI: three routes under `/api/webchat/v2/web-push/…` (descriptors +
  handlers under the `outbound` charter owner + CONTRACT.md rows).
- Frontend: `public/sw.js` (push + notificationclick, deep link to the app),
  SW registration, notification panel: web-push row rendered like other
  channels plus a per-browser "enable notifications in this browser" flow
  (`Notification.requestPermission` → `pushManager.subscribe(VAPID)` →
  subscribe command); device-state badges (unsupported / permission denied /
  enrolled). `worker-src 'self'` added to the shell CSP. i18n keys in all 11
  locales.
- Model steering: `TRIGGER_CREATE_DESCRIPTION` currently says no web-app
  target exists — update (+ its pinned `tool_surface_contract` test) and the
  `delivery.md` prompt so routines can be steered to the web-push target.

### Contract/doc corrections (dated)

- `docs/internal/reborn/contracts/communication-delivery-resolution.md` §5: "empty set
  means web app only" → empty set means **no external notification route**;
  the web app is selectable as the `web-push` catalog target.
- `docs/internal/reborn/extension-runtime/overview.md` §5.4 note: the no-pseudo-target
  statement stays true; add the web-push real-target clarification.
- `FEATURE_PARITY.md` PWA/Web Push row.
- New package README + family/crate table rows + target-tree doc for the two
  new workspace members.

## Two-lane contract: unchanged

Completed fires still deliver nothing from the notifier (lane 1 owns the
result). Selecting the web-push notification channel yields gate/auth/failure
notices as pushes; "push me the result" routines pin the web-push catalog
target and the model delivers via `builtin.outbound_deliver` (lane 2) exactly
as with Slack/Telegram. Push acceptance (201/202 from the push service) is
transport acknowledgment, not device receipt; the adapter reports
`Sent { vendor_message_ref: None }` and never fabricates stronger evidence.

## Test plan

- Domain crate: RFC 8291 Appendix A vector; VAPID JWT round-trip verify;
  store contract tests (CAS conflict, endpoint uniqueness, prune, scope
  isolation); builder goldens; endpoint validation.
- Host transport: `vapid_authorization` injection unit + contract tests
  (header shape, aud derivation, no key leak to adapter surface).
- Package: adapter conformance suite + deliver classification (Sent/prune/
  retry/permanent) over recording egress.
- Integration (`tests/integration/`): subscribe via product command → gated
  trigger fire → notice push POST captured on the recording network substrate
  (endpoint, `Authorization: vapid`, `Content-Encoding: aes128gcm`) + durable
  delivery attempt; 410 response prunes the subscription; web-push target
  selectable/deselectable through the notification-channels commands; the
  `empty_notification_set_keeps_blocked_fire_in_app_only` scenario updated for
  the new row's existence.
- Frontend vitest: panel row + enroll flow (mocked `navigator`), api client,
  SW registration guard; root-paths/manifest pins.
