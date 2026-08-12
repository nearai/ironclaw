# web-push — Browser notifications

The web app's browser-notification channel: outbound-only Web Push
(RFC 8030/8291/8292) to the user's enrolled browsers.

- **Extension id:** `web-push` · **Surfaces:** channel only (outbound-only —
  no ingress, no tools, no auth recipe) · **Runtime:** first_party ·
  **Code:** crate `ironclaw_web_push_extension`
- **Deployment-bound** like Telegram: the binary's binding table links the
  adapter and codec; there is no pairing flow (browser enrollment happens in
  the authenticated WebUI session via the web-push product commands).
- **Credentials:** the `web_push_vapid` handle holds auto-generated VAPID key
  material (`VapidCredentialMaterialV1`), seeded by composition at boot —
  never operator-typed. The RFC 8292 `Authorization: vapid` header is
  computed host-side by the `vapid_authorization` egress injection; the
  adapter never sees key bytes.
- **State:** per-user subscription records live in the
  `ironclaw_web_push` domain crate (this package's "vendor side" is our own
  database). 404/410 responses prune the dead subscription.
- **Evidence:** push services acknowledge acceptance (2xx) without a
  readable message reference, so delivery reports `Sent` with no vendor ref —
  acceptance by the push service, not device receipt.

The `[[channel.egress]]` hosts in `manifest.toml` are the deployment's
push-service allowlist: composition reads them back at boot and hands them to
enrollment validation (`PushEndpoint::validate_against_push_services`), so the
same list bounds both which endpoints may enroll and where deliveries may go.
`tests/manifest_lockstep.rs` pins the egress declarations' shape (https-only,
VAPID credential, `vapid_authorization` injection).

## Validation

- `cargo test -p ironclaw_web_push_extension`
- `cargo clippy -p ironclaw_web_push_extension --all-targets --all-features -- -D warnings`
