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

R001 stores no Matrix session. Access tokens remain in the host secret store,
and no Matrix session material is written to the channel workspace. Follow-up
session restore work should use host-owned encrypted credential/session storage
rather than plaintext WASM workspace files.

## Build and test

```bash
cargo test --manifest-path channels-src/matrix/Cargo.toml
cargo build --manifest-path channels-src/matrix/Cargo.toml --target wasm32-wasip2 --release
```

The build output is
`channels-src/matrix/target/wasm32-wasip2/release/matrix_channel.wasm`.
