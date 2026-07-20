# Slack Bot / Tools Remodel — Design Spec

**Date:** 2026-07-05
**Status:** Approved design; implementation pending
**Branches (stacked on `split/4-slack-legacy-config-rejection`):**
`split/5-slack-bot-tools-remodel` → `split/6-slack-least-privilege-scopes` → `split/7-slack-oauth-durability`
**Related:** Slack personal-OAuth stack #5643–#5646; design review 2026-07-05.

## 1. Motivation

The Slack personal-OAuth stack (#5644/#5645) introduced a hidden "companion"
extension (`slack_user`) auto-activated alongside the visible bot channel
(`slack`). A design review verified, against live code:

- **Activation-gap bug** — after `slack_personal` OAuth, the hidden companion is
  activated only by a best-effort frontend call; it stays connected-but-inactive
  on direct callback / closed popup / cross-device / restart. (`SetupOnly`
  continuation; the `LifecycleActivation` variant is an unwired no-op; tool
  visibility is gated on the in-memory active registry.)
- **Companion special-casing** — ~150 lines of Slack-named lifecycle in
  `ironclaw_reborn_composition` under a plan-less `large_file` arch-exempt.
- **Least-privilege scopes** — every user tool, including read-only ones,
  requests the full 11-scope union with `chat:write`.
- **Durability deferrals** — the PKCE verifier is process-local; the Slack
  conversation binding is in-memory. Both are documented, intentional deferrals.

Rather than *generalize* the companion, we adopt a cleaner product model
("model B") that **dissolves** the companion and the activation bug.

## 2. Model B

The Slack integration has two runtime-incompatible packages — a native bot
service and a WASM tool bundle — and that split is forced and unchanged. What
changes is *which* package is the user-installable, and the removal of the
hidden-companion coupling.

| Role | Was | Becomes | Provisioned by |
|---|---|---|---|
| Entrypoint (bot) | `slack` (visible, installable) | `slack_bot` — hidden from user catalog, operator infra | operator config (`[slack].enabled` + bot token / signing secret) |
| Installable tools | `slack_user` (hidden companion) | `slack` — the visible user-installable extension | user install + `slack_personal` OAuth |
| Identity link | companion OAuth side-effect | the tools extension's own OAuth setup | `slack_personal` (binds Slack id → Reborn user) |

**Verified architectural fact:** the bot channel is mounted purely from operator
config (`serve_slack.rs` → `slack_serve.rs`), authenticates on the Slack request
signature, and resolves the installation from host config. It **never consults
extension-install state** — the bot receives DMs with zero extensions installed.
The only bot↔tools coupling is at the identity layer: a user's DM runs *as them*
only if they have completed `slack_personal` OAuth.

**States:**
- Unbound user → bot replies with a connect/install nudge; **no turn runs**.
  *(New. Today a first-contact unbound DM is silently dropped.)*
- OAuth done, tools not activated → bot recognizes the user; no read/write
  tools. A valid state.
- OAuth done + tools active → full functionality.

## 3. Scope — three stacked PRs on `split/4`

### PR5 — `split/5-slack-bot-tools-remodel`

**Commit 1 — mechanical rename, no behavior change.** `slack` → `slack_bot`,
`slack_user` → `slack`. ~200 lines across 26 files, concentrated in
`ironclaw_reborn_composition` (id constants `SLACK_EXTENSION_ID` /
`SLACK_USER_EXTENSION_ID` in `available_extensions.rs`; asset dirs
`assets/slack`→`slack_bot`, `assets/slack_user`→`slack`; string literals).
Unchanged: `slack_bot_token`, `slack_user_token`, `slack_personal`.

**Commit 2+ — behavior.**
- **Visibility flip** — hide `slack_bot` from the user catalog
  (`is_internal_extension_package_ref`); un-hide `slack` (tools).
- **Delete companion coupling** — remove `activate_slack_with_companion`,
  `ensure_slack_user_companion_installed`,
  `append_slack_user_companion_credential_requirements`,
  `*_companion_package`, `append_companion_visible_capabilities`, and the
  companion gate in `activate_with_credential_gate`. The tools extension
  activates via the standard lifecycle (like GitHub/Gmail). Removes the
  companion clause of the `extension_lifecycle.rs` arch-exempt.
- **Move Configure/connect affordance** to the tools extension
  (`channel_unconnected` → `SetupRequired`; WebUI Configure card /
  `useChannelOnboarding`).
- **Unbound-user greeting** — emit a connect/install nudge on
  `Rejected(BindingRequired)` for `UserMessage` payloads
  (`slack_delivery.rs` `observe_workflow_ack` / rejection-hint path,
  ~1232–1256 and ~1577–1610). Canned reply only; no turn runs.

**Consequence:** the activation-gap bug dissolves (no hidden companion);
~150 lines deleted.

### PR6 — `split/6-slack-least-privilege-scopes`
- Read-only tools (`search_messages`, `list_conversations`,
  `get_conversation_history`, `get_user_info`) drop `chat:write` from their
  `runtime_credentials`; only `send_message` keeps it.
- `SLACK_PERSONAL_OAUTH_SETUP_SCOPES` becomes computed from enabled
  capabilities' declared scopes rather than a fixed union.
- **Open decision (settle in this PR):** whether write is a separate opt-in
  (Slack user OAuth is one consent per account) or activation grants the union.

### PR7 — `split/7-slack-oauth-durability`
- **Durable PKCE store** — replace the process-local `ExpiringLruCache` with a
  `PkceVerifierStore` port (`ironclaw_auth`) + an encrypted durable impl
  (AES-256-GCM secret store), wired at composition. Two impls (durable +
  in-memory test) so the trait earns its keep.
- **Durable conversation binding** — replace `InMemoryConversationServices`
  with a filesystem/DB store mirroring `FilesystemSlackHostState` (the
  security-critical *identity* binding is already durable).

## 4. Testing (test-first)
- **PR5:** unbound DM → nudge (was silent); tools extension installs/activates
  via the standard flow; bot receives inbound with no extension installed;
  catalog surfaces `slack` (tools), not `slack_bot`; companion functions gone.
- **PR6:** a read-only OAuth request omits `chat:write`; per-tool scopes are
  truthful.
- **PR7:** start flow → drop in-memory state → callback recovers the verifier
  from the durable store; conversation binding survives a restart.
- **Per PR:** `cargo fmt`; `cargo clippy --all --benches --tests --examples
  --all-features`; `cargo test`; `cargo test -p ironclaw_architecture`; the
  webui descriptor-contract test if routes change; e2e / live-test skill and
  setup docs updated.

## 5. Migration & non-goals
- **Migration:** the rename touches persisted install records keyed on
  `slack`/`slack_user`. The feature is opt-in beta (`slack-v2-host-beta`) with
  few/no real installs → low risk; include a note / one-shot for any dev/test
  data.
- **Non-goals:** do not modify the existing four PRs (#5643–#5646); do not
  de-extension-ify the bot channel (it stays a product-adapter, just
  operator-provisioned and hidden from the user catalog); provider
  `slack_personal` and the credential handles are unchanged.

## 6. Rollout
- Each PR: implement test-first, run the full quality gate, commit locally,
  merge forward (`split/5 → 6 → 7`). **Hold all pushes to origin until the
  diffs have been reviewed.**
