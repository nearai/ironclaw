# ironclaw_web_app

Web Push (RFC 8030/8291/8292) domain crate: the record grammar and pure
mechanics behind the web app's browser-notification channel.

## What it owns

- `PushSubscriptionRecord` / `PushEndpoint` / `PushSubscriptionKeys` — the
  validated per-browser enrollment grammar. The endpoint's push-service host
  is checked at enrollment against the deployment-supplied allowlist
  (`PushEndpoint::validate_against_push_services`), whose hosts composition
  resolves from the web-app manifest `[[channel.egress]]` entries — one
  source of truth with the egress path.
- `WebAppSubscriptionStore` + the scoped-filesystem implementation — one
  CAS-updated JSON document per (tenant, user), capped at
  `MAX_SUBSCRIPTIONS_PER_USER` browsers.
- RFC 8291 `aes128gcm` payload encryption (`encrypt_payload`), pinned to the
  RFC's Appendix A vector.
- VAPID key material generation (`generate_vapid_key_material`) producing the
  `VapidCredentialMaterialV1` JSON the host egress injector consumes.
- `build_push_request` — subscription + payload → a transport-free
  `WebAppRequestPlan` (host, origin-form path, protocol headers, encrypted
  body).
- The channel identity grammar (`web-app` extension id, the constant
  owner-scoped `web-app` target id, and the `web-app/v1/<tenant>/<user>`
  reply-target binding-ref format).
- VAPID material stays a protocol document here; the binary-owned channel
  initializer stores it through composition's neutral credential context and
  publishes only its public key as an opaque bootstrap document.

## When you want a different crate

- Sending the planned request: the channel package
  (`crates/extensions/packages/web-app`) over restricted egress.
- The `Authorization: vapid` header: computed host-side at the egress
  credential boundary (`ironclaw_host_runtime`), never here.
- Notification routing/policy: `ironclaw_outbound`.
- Subscribe/unsubscribe product surface: `ironclaw_assistant` + WebUI.

## Validation

- `cargo test -p ironclaw_web_app`
- `cargo clippy -p ironclaw_web_app --all-targets --all-features -- -D warnings`
