# google_gmail_send_approved replay snapshot

Replay snapshot for the Phase 6 Gmail approved send path.

`send_message.trace.json` is a recorded request/response/handler-output trace
for the `gmail.send_message` capability, captured after the descriptor-level
approval gate (`PermissionMode::Ask`) granted the write. It is the crate-level
stand-in for a `scripts/replay-snap.sh` capture: that wrapper drives cargo-insta
against a live agent session, which is out of scope for a crate-only package.
The trace pins the approved-write contract (request shape and the whitelisted,
redacted handler output) so the acceptance item is satisfied without a live
LLM.

The exercised, runnable approval/send-path coverage lives in
`crates/ironclaw_native_extensions/tests/google_gmail_approval.rs`, driven by a
fake `RuntimeHttpEgress` over the fixtures in
`crates/ironclaw_native_extensions/tests/fixtures/google_api/gmail/`.
