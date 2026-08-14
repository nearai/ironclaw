# web-app — Browser notifications

The web app's browser-notification channel: Web Push (RFC 8030/8291/8292) to
the user's enrolled browsers. The manifest also selects host-owned
authenticated-session ingress and stream replies, but this package binds only
`ChannelDelivery`.

- **Extension id:** `web-app` · **Surfaces:** channel only
  (`authenticated_session` ingress + stream reply + push delivery; no tools or
  auth recipe) · **Runtime:** first_party ·
  **Code:** crate `ironclaw_web_app_extension`
- **Deployment-bound** like Telegram: the binary's binding table links the
  delivery capability and codec; there is no pairing flow. The authenticated
  WebUI session drives the generic host-owned delivery-registration routes.
- **Credentials:** the `web_push_vapid` handle (constant `WEB_APP_VAPID_CREDENTIAL_HANDLE`, whose value deliberately keeps the pre-rename spelling — renaming it would rotate the VAPID identity and break every existing subscription) holds auto-generated VAPID key
  material (`VapidCredentialMaterialV1`), seeded by composition at boot —
  never operator-typed. The RFC 8292 `Authorization: vapid` header is
  computed host-side by the `vapid_authorization` egress injection; the
  adapter never sees key bytes.
- **State:** per-user registrations live in
  `ironclaw_auth::delivery_registrations`, not in the adapter. The package
  receives bounded opaque registrations only at delivery time; 404/410
  responses tell the host which registrations to prune.
- **Evidence:** push services acknowledge acceptance (2xx) without a
  readable message reference, so delivery reports `Sent` with no vendor ref —
  acceptance by the push service, not device receipt.

The `[[channel.egress]]` hosts in `manifest.toml` are the deployment's single
push-service allowlist. Generic product orchestration reads those resolved
hosts and rejects any registration endpoint outside them *before storage*;
restricted egress enforces the same declarations at send time. The adapter
parses Web Push key material only when it is used. `tests/manifest_lockstep.rs`
pins the egress declarations' shape (https-only, VAPID credential,
`vapid_authorization` injection).

## Validation

- `cargo test -p ironclaw_web_app_extension`
- `cargo clippy -p ironclaw_web_app_extension --all-targets --all-features -- -D warnings`
