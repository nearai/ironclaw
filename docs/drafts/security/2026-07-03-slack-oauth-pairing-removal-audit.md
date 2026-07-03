# Slack OAuth Pairing Removal Audit

Date: 2026-07-03

PR: https://github.com/nearai/ironclaw/pull/5604

Current base audited: `origin/main` (`f92c658c943b7844e15605e8761a393c4685449c`, includes merged PR #5362)

Initial audit comparison: `origin/codex/remove-chat-connect-shortcut` (`e95ead4e711b3b0e726b49d12b4d0108fc440fe7`, PR #5362 before merge) to `origin/codex/slack-oauth-pairing-integration` (`4a1264a22a960a745cc8445a0a325b26ba647676`)

Working branch: `codex/slack-oauth-pairing-integration`

## Scope

This audit checks whether the branch correctly removes the old Slack pairing-code flow and replaces the user journey with Slack OAuth:

1. User installs the single user-visible Slack extension.
2. User configures Slack from the Extensions page or from an in-chat connection-required card.
3. Configuration starts Slack personal OAuth.
4. Slack OAuth binds the authenticated WebUI user to the Slack user returned by Slack.
5. After OAuth, the user can message the Slack bot as an entrypoint and can use Slack personal tools from WebUI or bot DM with per-user token isolation.

This audit was originally commissioned relative to PR #5362. After #5362 merged, the branch was replayed onto `origin/main` and the post-replay verification below was rerun from that base.

## Discovery Method

The repo requires probing the codebase graph first for cross-crate flow questions. `bash scripts/codebase-graph.sh status` returned:

```text
graph:   MISSING (no .codebase-memory/artifact.json)
action:  build it once - call index_repository(repo_path=".") via the codebase-memory MCP
```

The `codebase-memory` MCP tools were not available in this session, so the audit followed the documented fallback: Reborn orientation notes, targeted `rg`, direct source reads, local diff inspection, focused tests, and independent subagent review.

## Executive Summary

The original branch direction is right: Slack should be modeled as one user-visible Slack extension, with the Slack bot as the entrypoint/channel and Slack personal OAuth as the user identity/tool credential layer. However, the first pushed PR was not safe to merge as-is.

The isolated audits found real blockers:

- Stale `/pair` tests still referenced deleted Slack pairing code and broke `slack-v2-host-beta` test compilation.
- Chat OAuth completion cleared the connection UI but did not resume the originating chat thread.
- Chat OAuth relied only on same-origin browser callback signals, so local WebUI plus public callback origin could leave the card stuck.
- Extension-page reconnect polling stopped as soon as Slack closed the popup, before a fresh server state read proved the user was connected.
- Static legacy Slack config could still seed or preserve Slack user identity without OAuth.
- Slack route serving had been broadened to "always enabled", contradicting config/docs and mounting Slack without explicit enablement.
- Slack OAuth start trusted client-supplied provider/scopes instead of deriving the Slack scope set from the server-side package contract.
- Current tests were mostly hook/unit simulations and did not yet include a served browser/API replacement path for Slack OAuth.

Several blockers have been fixed in response to this audit. Local targeted verification is now green; the remaining merge gates are unresolved CI status plus the explicitly open architecture/test-scope decisions below.

## Findings

### F-001: Stale Slack `/pair` Slash-Command Tests

Severity: High

Status: Fixed locally

Evidence:

- `crates/ironclaw_reborn_composition/src/slack_serve.rs` still had `pair_command_tests` importing deleted `slack_personal_binding_pairing` symbols.
- `cargo test -p ironclaw_reborn_composition --features slack-v2-host-beta --tests --no-run` failed before the fix.

Resolution:

- Removed the stale `/pair` caller test block.
- Re-ran `cargo test -p ironclaw_reborn_composition --features slack-v2-host-beta --tests --no-run`; it compiled after this removal.

Why this matters:

The production `/pair` route was no longer mounted, but stale tests create false confidence and break the relevant feature build.

### F-002: In-Chat OAuth Did Not Resume the Source Thread

Severity: High

Status: Fixed locally

Evidence:

- The chat OAuth completion effect cleared `pendingOnboarding` and called `notifyChannelConnected`.
- The shared channel-connection event bus intentionally skips `sourceThreadId` waiters, because the source path is expected to send its own continuation.
- Manual pairing already sent `channelConnectionContinuationMessage(...)` through `send(..., { bypassPendingOnboarding: true })`.
- OAuth did not do the same.

Regression test added:

- `useChat: Slack OAuth completion consumes the in-chat connection card`
- The test now asserts both UI clearance and a real send to the source thread with `Slack is connected. Continue the previous request.`

Resolution:

- Chat OAuth completion now routes through the same continuation send used by manual pairing.
- It clears the card only after the source continuation is accepted.

### F-003: In-Chat OAuth Had No Server-State Fallback

Severity: High

Status: Fixed locally

Evidence:

- Local testing commonly uses `127.0.0.1` for WebUI while OAuth callback can complete on a public/ngrok origin.
- BroadcastChannel and localStorage completion signals are same-origin browser signals.
- If the callback origin differs, the local chat cannot observe the browser signal.

Regression test added:

- `useChat: Slack OAuth completion polls per-user extension state when callback storage is unavailable`

Resolution:

- The chat OAuth watcher now polls the per-user extension snapshot and completes when Slack becomes authenticated/configured, even if no callback storage event is visible.

### F-004: Extension OAuth Reconnect Stopped Polling When Popup Closed

Severity: Medium

Status: Fixed locally

Evidence:

- Slack can close the OAuth popup before the Extensions page has observed the refreshed setup state.
- The watcher stopped when `popup.closed` was true, even for callback-required reconnects.

Regression test added:

- `useOauthSetup keeps polling reconnect after Slack closes the OAuth popup`

Resolution:

- For callback-required reconnects, popup closure no longer stops polling. The watcher continues until configured state is observed or the OAuth timeout expires.

### F-005: Legacy Static Slack User Config Bypassed OAuth Identity Proof

Severity: High

Status: Fixed locally for the audited paths

Evidence:

- `[slack].slack_user_id` was accepted as legacy setup in `ironclaw-reborn serve`.
- The runtime setup path could seed `RebornUserIdentityBinding` directly from config.
- `SlackHostBetaConfigInput` also carried an optional static Slack actor mapping.

Impact:

A stale or wrong config value could make a Slack actor appear connected to a Reborn user without the Slack OAuth callback proving Slack user/team/app identity for the authenticated user.

Resolution:

- `ironclaw-reborn serve` now rejects deprecated Slack setup fields, including `[slack].slack_user_id`, with a message telling the operator to configure Slack through WebUI and OAuth.
- The composition-side legacy setup seeding no longer writes a personal Slack identity binding.
- `SlackHostBetaConfigInput` and `SlackHostBetaConfig` no longer carry a static `slack_user_id` / `slack_actor` mapping.
- Inbound actor resolution now relies on durable identity bindings, which are created by Slack OAuth.

Residual note:

Slack user IDs still exist as OAuth-returned provider identity values and as DM provisioning targets. That is expected. The removed behavior is config-sourced Slack user identity as proof of ownership.

### F-006: Slack Host Route Was Always Enabled

Severity: High

Status: Fixed locally

Evidence:

- `resolve_slack_config_for_serve(None, ...)` returned `Some(runtime_config)`.
- Tests asserted "Slack always enabled".
- Docs/config say Slack should mount only when `[slack].enabled = true` or `IRONCLAW_REBORN_SLACK_ENABLED=true`.

Resolution:

- Slack host config now returns `None` unless enabled by config or env.
- Added a regression test for the no-section disabled case.
- Added a regression test that rejects legacy static user binding.

### F-007: Slack OAuth Start Trusted Client-Supplied Scopes

Severity: High

Status: Fixed locally

Evidence:

- The extension OAuth start handler routed Slack based on `request.provider`.
- Slack OAuth start turned `request.scopes` into provider scopes and sent them to Slack.
- The browser got scopes from setup projection, then echoed them back.

Impact:

A modified authenticated browser client could request broader Slack user scopes than the package declares. Slack user consent is still required, so this is not direct cross-user leakage, but it breaks backend least privilege and package-scope isolation.

Resolution:

- Slack personal OAuth start now only accepts the public Slack package as requester.
- It ignores client-supplied scopes and uses the canonical server-side Slack OAuth scope list from the available-extension catalog.

Regression tests added:

- `slack_personal_oauth_start_uses_server_scopes_not_client_supplied_scopes`
- `slack_personal_oauth_start_rejects_non_slack_requester_extension`

### F-008: Setup "Configured" State Could Ignore OAuth Client Credentials

Severity: Medium

Status: Open

Evidence:

- One architecture audit reported that Slack setup status could treat bot token + signing secret as configured without Slack personal OAuth client credentials.

Risk:

The UI could show Slack as configured while OAuth start cannot work.

Needed follow-up:

- Split setup status into bot entrypoint readiness and personal OAuth readiness, or require OAuth client id/secret for the user-facing "configured" state.
- Add a caller-level setup summary test.

### F-009: Served Slack OAuth Replacement Path Is Under-Tested

Severity: High

Status: Open

Evidence:

- The new coverage is strong at hook/component/direct-handler level.
- There is not yet a served WebUI/API test that starts from Extensions or Chat, starts Slack OAuth, completes the callback, sees Slack activate, closes the modal/card, and resumes the source chat.

Needed follow-up:

- Add a served `webui_v2_product_auth` or Playwright scenario using fake Slack OAuth responses.
- Include both entrypoints: Extensions page and in-chat connection-required card.

### F-010: Slack Personal Tool Token Injection Needs Runtime Dispatch Proof

Severity: High

Status: Open

Evidence:

- The `slack_user` WASM tool declares a `slack_personal` product-auth credential.
- Tests cover manifest metadata and OAuth token parsing, but not a full runtime capability dispatch proving the fake Slack request receives the per-user `xoxp` Authorization header.

Needed follow-up:

- Add a host-runtime dispatch test for `slack_user.search_messages` or `slack_user.send_message`.
- Assert the selected credential belongs to the authenticated Reborn user/scope and is not the bot token.

### F-011: Generic Pairing Artifacts Still Exist For Non-Slack Channels

Severity: Informational / scope decision

Status: Deliberately not removed in this pass

Evidence:

- `pairing-api.js`, `PairingSection`, and generic manual pairing tests still exist.
- These are used for non-Slack channels such as Telegram-style manual-code pairing.

Conclusion:

This branch should remove Slack pairing artifacts, not delete generic manual pairing for every channel unless product explicitly decides to remove all non-Slack pairing flows. The audit found no production Reborn Slack `/pair` route still mounted after the stale test removal.

### F-012: PR Scope Contains Potentially Unrelated Changes

Severity: Medium

Status: Open / needs owner decision

Evidence from diff-hygiene audit:

- Telegram group keyword filtering appears unrelated to Slack OAuth/pairing removal.
- Some v1 `src/` pairing-tool removals may affect legacy Slack relay behavior.
- The new `slack_user` tool is larger than UI pairing removal, though it may be necessary for the intended "read/search/send Slack as me" product flow.

Needed follow-up:

- Decide whether Telegram and v1 pairing changes stay in this PR or split out.
- If v1 behavior is intentionally deprecated, document that explicitly and add deprecation coverage.

### F-013: Targeted Clippy Failed On Auth / Lifecycle Additions

Severity: Medium

Status: Fixed locally

Evidence:

- `cargo clippy -p ironclaw_reborn_cli -p ironclaw_reborn_composition --features slack-v2-host-beta --tests -- -D warnings` initially failed.
- The main correctness-adjacent failure was `ProviderCallbackOutcome` carrying a large `OAuthProviderExchange` enum variant by value.
- The same run also caught new composition style failures in the provider-identity cleanup path, runtime credential requirement merge predicates, and Slack companion lifecycle glue.

Resolution:

- Boxed the `ProviderCallbackOutcome::Authorized` exchange payload and updated both durable and in-memory callback completion consumers.
- Updated contract tests and Reborn product-auth callback completion to construct the boxed payload explicitly.
- Collapsed the provider-identity check path without changing token cleanup or fail-closed behavior.
- Reworked Slack companion lifecycle code through small private helpers and converted merge predicates to `matches!`.

Verification:

- The same targeted clippy command now exits successfully.

## Multi-Tenant Isolation Review

The desired tenant model is:

- Slack bot token/signing secret are installation/app-level entrypoint credentials.
- Slack personal OAuth token is user-scoped product-auth credential.
- Inbound Slack DM/app mention is an entrypoint. It resolves the Slack actor to a Reborn user through durable identity binding.
- That binding is created only after Slack OAuth callback proves Slack user/team/app identity.
- Slack personal tools resolve credentials through product-auth owner scope and extension requirements.

Findings:

- OAuth callback already checks Slack provider identity and rejects wrong app/team/foreign tenant paths in direct-handler tests.
- Slack OAuth token parser intentionally uses `authed_user.access_token`, not the top-level bot `access_token`.
- Static config identity binding was the main leakage risk found. It has been removed from the audited paths.
- The remaining important proof gap is runtime dispatch: we still need a test that a Slack personal tool invocation injects the correct per-user token and never the bot token or another user's token.

## Verification Log

Red tests observed before fixes:

```text
node --test crates/ironclaw_webui_v2/static/js/pages/chat/lib/useChat-send.test.mjs --test-name-pattern "Slack OAuth completion"
# failed: OAuth completion did not send the continuation; server-state fallback did not clear the card

node --test crates/ironclaw_webui_v2/static/js/pages/extensions/hooks/useExtensions-oauth.test.mjs --test-name-pattern "popup|polling"
# failed: reconnect did not complete after popup closed before fresh configured state
```

Green tests after fixes:

```text
node --test crates/ironclaw_webui_v2/static/js/pages/chat/lib/useChat-send.test.mjs --test-name-pattern "Slack OAuth completion"
# 60/60 pass

node --test crates/ironclaw_webui_v2/static/js/pages/extensions/hooks/useExtensions-oauth.test.mjs --test-name-pattern "popup|polling"
# 4/4 pass

node --check crates/ironclaw_webui_v2/static/js/pages/chat/hooks/useChat.js
node --check crates/ironclaw_webui_v2/static/js/pages/extensions/hooks/useExtensions.js
# pass
```

Fresh Rust verification after local fixes:

```text
cargo test -p ironclaw_auth
# 14 unit tests pass; 83 auth product contract tests pass; doc tests pass

cargo test -p ironclaw_reborn_cli
# 110 unit tests pass; 5 extension tests pass; 87 smoke tests pass

cargo test -p ironclaw_reborn_cli --features slack-v2-host-beta serve_slack -- --nocapture
# 3/3 serve_slack tests pass

cargo test -p ironclaw_reborn_composition --features slack-v2-host-beta slack_personal_oauth_start -- --nocapture
# 2/2 Slack OAuth start tests pass

cargo test -p ironclaw_reborn_composition --features slack-v2-host-beta seed_legacy_slack_setup -- --nocapture
# 2/2 legacy Slack setup tests pass

cargo test -p ironclaw_reborn_composition --features slack-v2-host-beta --tests --no-run
# all composition test binaries compile

cargo build -p ironclaw_reborn_cli --features slack-v2-host-beta --bin ironclaw-reborn
# pass

cargo clippy -p ironclaw_reborn_cli -p ironclaw_reborn_composition --features slack-v2-host-beta --tests -- -D warnings
# pass

cargo fmt --check
git diff --check
# pass
```

Post-#5362-merge replay verification on top of `origin/main`:

```text
cargo test -p ironclaw_reborn_composition --features slack-v2-host-beta slack_personal_oauth_start -- --nocapture
# 2/2 Slack OAuth start tests pass; command exits 0

cargo test -p ironclaw_reborn_composition --features slack-v2-host-beta seed_legacy_slack_setup -- --nocapture
# 2/2 legacy Slack setup tests pass; command exits 0

cargo test -p ironclaw_reborn_composition --features slack-v2-host-beta --tests --no-run
# all composition test binaries compile; command exits 0

cargo build -p ironclaw_reborn_cli --features slack-v2-host-beta --bin ironclaw-reborn
# pass; command exits 0

cargo clippy -p ironclaw_reborn_cli -p ironclaw_reborn_composition --features slack-v2-host-beta --tests -- -D warnings
# pass; command exits 0
```

Notes:

- Cargo prints `warning: unused config key net.retries` from the local `.cargo/config.toml`; this is environment/config noise, not a crate warning.
- The untracked generated WebUI bundle at `crates/ironclaw_webui_v2/static/dist/` remains intentionally unstaged and must not be committed.

## Subagent Audit Inputs

### UI/OAuth UX Auditor

Verdict: Not merge-ready before fixes.

Key findings:

- In-chat OAuth cleared UI without resuming source chat.
- Cross-origin callback signals could be invisible to local WebUI.
- Extension polling could stop on stale cache when popup closed.
- Modal close during OAuth could orphan watcher.
- Popup blocker path lacked useful handling.

### Pairing Artifact Auditor

Verdict: Not clean before fixes.

Key findings:

- Stale `/pair` slash-command tests broke Slack feature compilation.
- Generic pairing redemption client/tests still exist.
- No production Reborn Slack pairing-code route appeared mounted.
- v1 generic pairing API/CLI can still syntactically accept `slack`, but no Slack minting path was found.

### Backend Security / Multi-Tenant Auditor

Verdict: Needs changes before fixes.

Key findings:

- Legacy `[slack].slack_user_id` could create connected state without OAuth.
- Slack OAuth start trusted client provider/scopes.
- Slack callback uses user token from `authed_user.access_token`, not bot token.
- Outbound target facade had tenant/caller ownership checks for channel routes and personal DMs.

### Architecture / Modeling Auditor

Verdict: Request changes before fixes.

Key findings:

- Slack setup configured state may not require OAuth client credentials.
- Public Slack entrypoint and hidden Slack personal tool are glued together with Slack-specific lifecycle branches.
- OAuth routing remains provider-string branching rather than a provider strategy model.
- Slack setup rollback may omit OAuth client secret cleanup.

### Test / Verification Auditor

Verdict: Unsafe before fixes.

Key findings:

- Lacked served caller-level Slack OAuth coverage.
- Slack personal token scoping not proven through runtime dispatch.
- Slack reconnect/multi-user isolation only partially covered.
- One ConfigureModal pairing test mixed Telegram modal setup with Slack provider assertions.

### Diff Hygiene Auditor

Verdict: Block as-is before fixes.

Key findings:

- Slack serve enablement/defaults regressed.
- Unrelated Telegram keyword filtering may need split.
- v1 pairing removal may be a legacy regression or needs explicit deprecation.
- Untracked generated WebUI bundle exists locally and should not be committed.

## Merge Recommendation

The local fixes close the concrete blocker findings found by the audit. Do not merge until:

1. CI is green or every failure is understood and unrelated.
2. Owner decides whether Telegram/v1 pairing changes belong in this PR.
3. A served Slack OAuth replacement path test is added or explicitly accepted as a follow-up risk.
4. A runtime dispatch test proves Slack personal tools receive the correct per-user token.

The branch is now much closer to security-defensible, but the remaining open risks are real. The fixed invariants must stay enforced by tests:

- no config-sourced Slack user identity binding;
- no browser-controlled Slack OAuth scope escalation;
- in-chat OAuth resumes the source thread;
- callback-origin mismatch can recover from server state;
- Slack entrypoint and Slack personal tool credentials remain separate layers.
