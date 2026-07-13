# Matrix channel skeleton

This is the Matrix channel skeleton for IronClaw Reborn. It establishes the
WASM component shape, capability manifest, build path, and host callback
contract that later Matrix work will extend.

This skeleton intentionally does not implement live sync, message sending,
E2EE, homeserver discovery, media, or session restore. HTTP callbacks and
outbound responses fail closed with explicit skeleton errors until those
features are implemented in follow-up work.

## Security boundary

The component does not contain real credentials, homeserver URLs, user IDs,
device IDs, access tokens, or cryptographic keys. `matrix_access_token` is a
host-managed secret and is injected only by the host HTTP credential layer for
allowed Matrix API requests.

The `/webhook/matrix` callback is protected by a host-managed
`matrix_webhook_secret` using the `X-Matrix-Webhook-Secret` header. The skeleton
requires the host to validate that secret before the callback reaches WASM.

R001 keeps outbound HTTP egress restricted to `matrix.org` under `/_matrix/`.
Federated homeserver discovery requires explicit SSRF mitigations before this
can expand to arbitrary homeserver hosts. R002 must define HTTPS-only egress,
private/link-local IP rejection, DNS rebinding handling, redirect policy, and
per-homeserver credential scoping before enabling wildcard Matrix egress.

## Persistence

R001 stores no Matrix session and performs no session refresh. Access tokens
remain in the host secret store, and no Matrix session material is written to
the channel workspace or WASI filesystem.

On component restart, Matrix starts from the same static host-provided config
and has no resumable sync cursor, device state, or refresh-token state to
restore. Follow-up session restore work should use host-owned encrypted
credential/session storage, or an equivalent host-managed keyvalue service,
rather than plaintext WASM workspace files.

## Future interface evolution

The R001 skeleton does not change the shared WIT records for E2EE. R002 should
make E2EE additions additive by using optional fields or metadata keys for
sender device identity, encrypted-message content variants, undecryptable event
state, and room/device provenance. R002 should not require existing plaintext
channels to change their callback signatures.

## Supported callbacks

R001 supports the following WIT callbacks as skeleton entry points:

- `on_start`
- `on_http_request`
- `on_poll`
- `on_status`
- `on_shutdown`

`on_respond` and `on_broadcast` return descriptive unsupported-callback errors
until Matrix outbound delivery exists.

## Build and test

```bash
cargo test --manifest-path channels-src/matrix/Cargo.toml
cargo build --manifest-path channels-src/matrix/Cargo.toml --target wasm32-wasip2 --release
```

The build output is
`channels-src/matrix/target/wasm32-wasip2/release/matrix_channel.wasm`.
