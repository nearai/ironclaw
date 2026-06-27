# Slack Pairing Recovery (`/pair`) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. Exact Rust is validated against the compiler during TDD — treat code blocks as faithful direction, run `cargo` after every change.

**Goal:** Give Slack users a self-service way to recover a lost/expired pairing code via a `/pair` slash command, and stop silently dropping unpaired users' DMs.

**Architecture:** New signed Slack slash-command ingress (`/webhooks/slack/commands`) that force-mints a fresh pairing code and returns it ephemerally; a force-mint seam on the pairing store/service; a rate-limited discoverability nudge replacing the silent message-drop; plus web redeem copy. Reuses the existing HMAC verify + installation-resolution substrate; no change to redeem/binding/TTL.

**Tech Stack:** Rust (axum, async-trait, tokio, serde), `ironclaw_reborn_composition` crate; JS (Preact/htm) for webui_v2 static; Python/Playwright for e2e.

## Scope reduction (shipped PR)

This plan was authored for the full feature; the **shipped PR is narrowed**:

- **Task 1** (store `reissue_challenge` force-mint) — shipped.
- **Task 2** — only the `reissue_challenge` **service** method ships. The
  `send_unlinked_nudge` notifier method and the `SlackUnlinkedNudgeNotification`
  type are **CUT**.
- **Task 3** (discoverability nudge in the resolver) — **CUT**. The deduped branch
  keeps its existing silent `Ok(None)`; no nudge, no `nudge_dedup` cooldown cache.
- **Tasks 4–6** (slash ingress, route/handler/ephemeral response, mount) — shipped.
- **Task 7** (web redeem copy) — shipped, including both the instructions copy and
  the error copy, both pointing at `/pair`.
- **Task 8** — docs shipped; the standalone `test_slack_pair_recovery.py` e2e is
  **CUT** (Rust handler/integration/contract tests cover the behavior instead).

The nudge stays a documented future enhancement; the Goal line's "stop silently
dropping unpaired users' DMs" is deferred with it.

## Global Constraints

- No `.unwrap()`/`.expect()` in production code (tests fine). Existing `NonZero*::new(..).unwrap()` consts with `// safety:` are the established pattern.
- Errors via `thiserror`; map with context; never `map_err(|_| …)` (carry the cause). No internal paths/IDs/the code value in user-facing output or logs.
- Strong types: reuse `AdapterInstallationId`, `SlackUserId`, `SlackPersonalBindingPairingCode`, `TenantId`.
- Prompt/large strings live in files, not Rust — N/A here (all copy is single-line).
- Pairing code: 8 char, single-use, `DEFAULT_PAIRING_TTL` = 10 min. Do not change.
- Test through the caller (`.claude/rules/testing.md`): handler/service-level tests, not just helpers.
- `info!`/`warn!` reserved; use `debug!` for diagnostics.
- Reborn composition guardrails: facade-shaped handles only; route crates don't bind listeners; reuse `ironclaw_auth`/adapter ports.

---

### Task 1: Store `reissue_challenge` — force-mint fresh, invalidate prior

**Files:**
- Modify: `crates/ironclaw_reborn_composition/src/slack_personal_binding_pairing.rs` (trait `SlackPersonalBindingPairingChallengeStore`, ~line 102)
- Modify: `crates/ironclaw_reborn_composition/src/slack_host_state.rs` (impl, near `issue_challenge` ~919; cleanup helpers ~1286-1329)
- Test: `slack_host_state.rs` `#[cfg(test)] mod tests` (alongside existing `issue_challenge` tests ~1978+)

**Interfaces:**
- Produces: `async fn reissue_challenge(&self, challenge: SlackPersonalBindingPairingChallenge) -> Result<IssuedSlackPersonalBindingPairingChallenge, SlackPersonalBindingPairingError>` on the store trait. Always expires/cleans any existing actor+code record for the `(installation_id, slack_user_id)` then mints a brand-new code. Default trait method may delegate, but the FS impl overrides for atomicity under the actor lock.

- [ ] **Step 1 — failing test:** `reissue_challenge_mints_fresh_and_invalidates_prior`: issue a challenge → capture code A; `reissue_challenge` → code B; assert `A != B`; assert `get_challenge(A)` → `ChallengeNotFound`; assert `get_challenge(B)` → Ok.
- [ ] **Step 2 — run, expect FAIL** (method missing): `cargo test -p ironclaw_reborn_composition reissue_challenge_mints_fresh -- --nocapture`
- [ ] **Step 3 — implement:** add trait method; FS impl: take the actor lock (`self.lock_for("pairing-actor:{inst}:{user}")`), read existing actor record, if present call `cleanup_actor_pairing_code_record` (delete code file) + `cleanup_pairing_actor_record` (expire actor record), then run the existing mint loop (factor the loop body of `issue_challenge` lines 947-1004 into `mint_fresh_challenge(&self, challenge, &actor_path, existing_actor_version) ` and call from both `issue_challenge` (no existing-active reuse path change) and `reissue_challenge`). Keep `issue_challenge` behavior identical.
- [ ] **Step 4 — run, expect PASS.**
- [ ] **Step 5 — concurrency test:** `reissue_challenge_is_cas_safe`: two concurrent `reissue_challenge` calls → both Ok, final `get_challenge` of the surviving code Ok, exactly one actor record. Run, fix if needed.
- [ ] **Step 6 — `cargo clippy -p ironclaw_reborn_composition --all-targets` clean; commit** (deferred — batch commit at end).

---

### Task 2: Service `reissue_challenge` + notifier nudge

**Files:**
- Modify: `slack_personal_binding_pairing.rs` (notifier trait ~120; service ~237)
- Modify: `slack_pairing_notifier.rs` (impl ~40)
- Test: `slack_personal_binding_pairing.rs` tests + `slack_pairing_notifier.rs` tests

**Interfaces:**
- Produces on `SlackPersonalBindingPairingNotifier`: `async fn send_unlinked_nudge(&self, notification: SlackUnlinkedNudgeNotification) -> Result<(), …>` where `SlackUnlinkedNudgeNotification { installation_id, slack_user_id }` (no code).
- Produces on `SlackPersonalBindingPairingService`: `async fn reissue_challenge(&self, installation_id, slack_user_id) -> Result<IssuedSlackPersonalBindingPairingChallenge, …>` (calls `challenge_store.reissue_challenge` then `notifier.send_pairing_challenge`) and `async fn send_unlinked_nudge(&self, installation_id, slack_user_id) -> Result<(), …>` (delegates to notifier).

- [ ] **Step 1 — failing test (notifier):** a fake `ProtocolHttpEgress` capturing the posted body; assert `send_unlinked_nudge` posts to `/api/chat.postMessage` with text containing "/pair" and NOT containing any code. (Mirror existing notifier tests at `slack_pairing_notifier.rs:191+`.)
- [ ] **Step 2 — run, expect FAIL.**
- [ ] **Step 3 — implement:** add `SlackUnlinkedNudgeNotification` struct; add trait method (default `Ok(())`? No — required, so all impls implement); impl on `SlackPairingChallengeHttpNotifier` reusing `open_dm_channel` + `send_slack_request`; host-authored fixed text: `"You're not linked to Ironclaw yet. Run `/pair` in Slack to get a fresh pairing code."` (mrkdwn:false so backticks are literal — or set a plain-text variant).
- [ ] **Step 4 — run, expect PASS.**
- [ ] **Step 5 — service test:** `reissue_challenge_service_sends_dm_and_returns_fresh`: fake store + fake notifier; assert store.reissue called, notifier.send_pairing_challenge called with the new code. Plus `send_unlinked_nudge_service_delegates`.
- [ ] **Step 6 — run PASS; clippy clean.**

---

### Task 3: Discoverability nudge in resolver (replace silent drop)

**Files:**
- Modify: `slack_personal_binding_pairing.rs` `SlackPairingActorResolver::resolve_product_actor_user` (~395-443) + cache (`pending_challenge_cache`, `reserve_pairing_challenge` ~349, `clear_pairing_challenge_reservation` ~367)
- Test: same file's tests (~575+, e.g. `resolver_suppresses_duplicate_pairing_challenges_during_cooldown`)

**Interfaces:**
- Consumes: Task 2 `service.send_unlinked_nudge`.
- Behavior: on the deduped branch (currently `reserve_pairing_challenge` returns false → silent `Ok(None)`), instead fire `self.pairing.send_unlinked_nudge(...)` at most once per nudge-cooldown window, then return `Ok(None)`. Keep first-contact code issuance unchanged. Don't turn unpaired msgs into turns.

- [ ] **Step 1 — failing test:** `unpaired_dm_within_cooldown_sends_single_nudge`: lookup returns None; first call issues challenge (existing); second call within cooldown → exactly ONE nudge sent (fake notifier counts), returns `Ok(None)`; third within cooldown → no extra nudge.
- [ ] **Step 2 — run, expect FAIL.**
- [ ] **Step 3 — implement:** add a `nudge_dedup` cache (or reuse `pending_challenge_cache` with a second key namespace) keyed by `(installation, slack_user)` with its own TTL const `SLACK_UNLINKED_NUDGE_COOLDOWN` (e.g. 5 min). On deduped branch: if nudge not in cooldown, spawn/await `send_unlinked_nudge` (await is fine; it's cheap) and record cooldown; swallow+log any nudge error with `debug!`. Return `Ok(None)`.
- [ ] **Step 4 — run, expect PASS;** ensure existing `resolver_suppresses_duplicate_pairing_challenges_during_cooldown` still passes (adjust if it asserted total silence — it should now assert no new *challenge*, nudge allowed).
- [ ] **Step 5 — clippy clean.**

---

### Task 4: Slash-command ingress resolution

**Files:**
- Modify: `slack_serve/installation.rs` (add `ResolvedSlackCommand`, `resolve_command_ingress`; reuse `verify_candidates`, `resolved_installation`, `ensure_candidate_budget`)
- Modify: `slack_serve.rs` (re-export `ResolvedSlackCommand`)
- Test: `installation.rs` tests

**Interfaces:**
- Produces: on `SlackInstallationResolver` trait, `fn resolve_command_ingress<'a>(&'a self, headers, body) -> Pin<Box<dyn Future<Output = Result<ResolvedSlackCommand, SlackIngressError>> + Send + 'a>>`. `ResolvedSlackCommand { installation: ResolvedSlackInstallation, command: String, slack_user_id: SlackUserId, response_url: String }`.
- Form parse: `application/x-www-form-urlencoded` → fields `command`, `team_id`, `api_app_id`, `enterprise_id` (opt), `user_id`, `response_url`. Build a `SlackInstallationSelector`-matchable view: add `SlackInstallationSelector::matches_command(&self, team_id, api_app_id, enterprise_id)` OR construct a minimal `SlackEnvelopeMetadata` from the form and reuse `.matches`. Prefer a small `SlackCommandContext { team_id, api_app_id, enterprise_id }` + `selector.matches_command(&ctx)`.

- [ ] **Step 1 — failing test:** `resolve_command_ingress_verifies_and_extracts_user`: a `StaticSlackInstallationResolver` with one record whose dispatcher verifies a known signature; POST body = urlencoded `command=/pair&team_id=T1&api_app_id=A1&user_id=U1&response_url=https://...`; assert returns `ResolvedSlackCommand` with `slack_user_id == U1`, `command == "/pair"`. Plus `resolve_command_ingress_rejects_bad_signature` (dispatcher rejects → `SlackIngressError::Runner(AuthenticationFailed)`), and `…_rejects_unknown_team` → `InstallationNotFound`.
- [ ] **Step 2 — run, expect FAIL.**
- [ ] **Step 3 — implement:** parse form (use `form_urlencoded::parse` or `serde_urlencoded`); resolve candidates via selector match on team/app/enterprise; `verify_candidates`; build `ResolvedSlackCommand`. URL-decode user_id/command.
- [ ] **Step 4 — run, expect PASS; clippy clean.**

---

### Task 5: Slash route, state, handler, ephemeral response

**Files:**
- Modify: `slack_serve.rs` (add `SLACK_COMMANDS_PATH`, descriptor, `SlackCommandsRouteState`, `slack_commands_handler`, `SlackSlashResponse`, `slack_commands_route_mount`)
- Test: `slack_serve/handler_tests.rs`

**Interfaces:**
- Consumes: Task 4 `resolve_command_ingress`; Task 2 `service.reissue_challenge`; `RebornUserIdentityLookup::resolve_user_identity` (already-paired check).
- `SlackCommandsRouteState { ingress: SlackIngressService, pairing: SlackPersonalBindingPairingService, lookup: Arc<dyn RebornUserIdentityLookup> }`.
- `SlackSlashResponse { response_type: "ephemeral", text: String }` (serde).
- Produces: `pub fn slack_commands_route_mount(state) -> PublicRouteMount`, `pub const SLACK_COMMANDS_PATH`.

- [ ] **Step 1 — failing test:** `slack_pair_command_returns_fresh_code_ephemerally`: signed `/pair` form → 200 JSON `{response_type:"ephemeral", text: contains the freshly-minted code}`. `slack_pair_twice_returns_different_codes_and_invalidates_first`. `slack_pair_already_linked_returns_connected_message` (lookup returns Some → no code in text). `slack_pair_bad_signature_returns_401`.
- [ ] **Step 2 — run, expect FAIL.**
- [ ] **Step 3 — implement:** handler: `resolve_command_ingress` → on err `ingress_error_response`/401; rate-limit per installation (reuse `installation_rate_limiter`); already-paired? ephemeral "You're already connected to Ironclaw."; else `pairing.reissue_challenge(installation_id, slack_user_id)` → ephemeral with code; map service errors to ephemeral "Pairing is temporarily unavailable — try again." + `debug!` the cause (no code, no paths). Never log the code.
- [ ] **Step 4 — run, expect PASS; clippy clean.**

---

### Task 6: Mount the commands route in `serve_slack`

**Files:**
- Modify: `crates/ironclaw_reborn_cli/src/commands/serve_slack.rs` (where `slack_events_route_mount` is built/mounted; thread the pairing service + identity lookup that already exist in host-beta composition)
- Test: existing serve_slack tests + a smoke assertion the route is mounted (or covered by Task 5 caller tests via `oneshot`)

- [ ] **Step 1 — locate** the events mount + the `SlackPersonalBindingPairingService` / identity-lookup handles in host-beta composition (`slack_host_beta.rs`).
- [ ] **Step 2 — implement:** build `SlackCommandsRouteState` from the same ingress resolver + pairing service + lookup; add `slack_commands_route_mount(...)` to the mounted public routes next to events. Gate behind the same `[slack].host_ingress_mode` / enablement as events.
- [ ] **Step 3 — `cargo build -p ironclaw_reborn_cli`; run any serve_slack tests; clippy clean.**

---

### Task 7: Web redeem copy references `/pair`

**Files:**
- Modify: `crates/ironclaw_webui_v2_static/static/js/components/slack-pairing-section.js` (copy ~68-78)
- Modify: the i18n source defining `pairing.slackInstructions` / `pairing.slackError` (locate via `rg "pairing.slackInstructions"`)
- Rebuild: `static/dist/app.js` per the repo's bundle step
- Test: existing webui_v2_serve static source-shape test pattern

- [ ] **Step 1:** update instructions copy to: "Run `/pair` in Slack to get a code, then paste it here. Codes expire in 10 minutes." Update error fallback to mention running `/pair` again.
- [ ] **Step 2:** rebuild dist bundle (follow `crates/ironclaw_webui_v2_static` build instructions / the concat step used by prior commits like `8c334705c`).
- [ ] **Step 3:** run the static-shape test if one asserts this copy; otherwise add a minimal assertion.

---

### Task 8: Docs + e2e

**Files:**
- Modify: `serve_slack` docs / `crates/ironclaw_reborn_cli` README or `docs/` Slack setup — document registering the `/pair` slash command (Request URL `<host>/webhooks/slack/commands`, `commands` scope) for prod + local.
- Create: `tests/e2e/scenarios/test_slack_pair_recovery.py` (gated like existing Slack e2e on a signing secret)

- [ ] **Step 1:** write the manifest/setup docs section.
- [ ] **Step 2:** port an e2e scenario: cold DM → nudge observed; `/pair` → ephemeral code; redeem in web → bound. Gate/skip if no Slack signing secret in the harness (mirror existing Slack e2e gating).
- [ ] **Step 3:** `cargo fmt --all`; full `cargo clippy --all --tests` clean; `cargo test -p ironclaw_reborn_composition`.

---

## Final gate
- [ ] `cargo fmt`
- [ ] `cargo clippy --all --benches --tests --examples --all-features` (zero warnings)
- [ ] `cargo test` (+ `--features integration` where relevant)
- [ ] Self-review diff; update spec if design drifted.
