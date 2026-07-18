# Generic OAuth Denial Lifecycle Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make OAuth denial release the correct blocked run for every product surface while steering Slack personal OAuth to the configured workspace.

**Architecture:** Keep denial policy in the shared product-auth and product-workflow layers. An explicit product `auth deny` cancels the exact run, while a provider callback denial dispatches the existing canceled-auth continuation so the exact gate resumes with a denied disposition; channel adapters only parse and render. Slack-specific code contributes only a validated `team` authorization hint sourced from durable Slack setup.

**Tech Stack:** Rust 2024, Tokio, Axum, typed Reborn auth/turn ports, Cargo integration harnesses.

## Global Constraints

- Do not delete a thread, transcript, or unrelated run.
- Do not add Telegram-, Slack-, or WebUI-specific auth lifecycle policy.
- Preserve `Failed(ProviderDenied)` as the durable provider-cancel result and preserve the provider-denied HTTP response.
- Preserve exact `BlockedAuthGate` preconditions, actor scope checks, and idempotency markers.
- Keep Slack callback binding validation authoritative; `team` is only an authorization-page hint.
- Do not add `.unwrap()` or `.expect()` to production code.
- Do not reintroduce the OAuth expiry or supersession behavior reverted from PR #6130.

---

### Task 1: Explicit Product Denial Cancels the Exact Run

**Files:**
- Modify: `crates/ironclaw_turns/src/request.rs`
- Modify: `crates/ironclaw_turns/src/memory/mod.rs`
- Modify: `crates/ironclaw_product_workflow/tests/auth_interaction_contract.rs`
- Modify: `crates/ironclaw_product_workflow/src/auth_interaction/service.rs`

**Interfaces:**
- Consumes: `ResolveAuthInteractionRequest`, `TurnCoordinator::cancel_run`, `CancelRunPrecondition`, and `AuthFlowManager::cancel_flow`.
- Produces: `DefaultAuthInteractionService::cancel_auth_run(...) -> Result<ResolveAuthInteractionResponse, ProductWorkflowError>` and `ResolveAuthInteractionResponse::Canceled` for explicit denial.

- [ ] **Step 1: Change the caller-level tests to require cancellation**

  Rename the parked-gate and missing-flow tests to describe cancellation. Assert the response is `Canceled`, the flow is canceled once when present, `cancel_run` receives `SanitizedCancelReason::UserRequested`, and `resume_turn` is never called. Update replay tests to seed and assert the coordinator's cancel idempotency result instead of a resume result.

  ```rust
  assert!(matches!(response, ResolveAuthInteractionResponse::Canceled(_)));
  assert_eq!(flow_manager.cancellations().len(), 1);
  assert!(coordinator.resumes().is_empty());
  let cancellations = coordinator.cancellations();
  assert_eq!(cancellations.len(), 1);
  assert_eq!(cancellations[0].reason, SanitizedCancelReason::UserRequested);
  ```

- [ ] **Step 2: Run the focused contract tests and verify red**

  Run: `cargo test -p ironclaw_product_workflow --test auth_interaction_contract denied_auth -- --nocapture`

  Expected: FAIL because current production returns `Resumed` and records `resume_turn`.

- [ ] **Step 3: Replace denied-gate resume with exact run cancellation**

  Import `CancelRunRequest` and `SanitizedCancelReason`. Reserve the active auth flow first so callback completion and denial have one atomic winner. If denial wins, cancel the run with the exact `BlockedAuthGate` precondition, then finalize the flow cancellation. Roll the reservation back if run cancellation fails, including a stale-gate failure, so the flow stays usable. If completion wins, leave the flow and run on the completion path. Use the shared run-cancel helper for the missing-flow parked-gate path and for idempotent replays of a previously canceled flow.

  ```rust
  async fn cancel_auth_run(
      &self,
      request: ResolveAuthInteractionRequest,
      run_id: TurnRunId,
  ) -> Result<ResolveAuthInteractionResponse, ProductWorkflowError> {
      let response = self
          .turn_coordinator
          .cancel_run(CancelRunRequest {
              scope: request.scope,
          actor: request.actor,
          run_id,
          precondition: Some(CancelRunPrecondition::BlockedAuthGate {
              gate_ref: request.gate_ref,
          }),
          reason: SanitizedCancelReason::UserRequested,
              idempotency_key: request.idempotency_key,
          })
          .await
          .map_err(map_auth_resume_error)?;
      Ok(ResolveAuthInteractionResponse::Canceled(response))
  }
  ```

- [ ] **Step 4: Run the entire product-workflow auth contract**

  Run: `cargo test -p ironclaw_product_workflow --test auth_interaction_contract -- --nocapture`

  Expected: PASS, including stale completion, missing-flow, actor-scope, and idempotency cases.

### Task 2: Provider Popup Denial Resumes the Generic Gate

**Files:**
- Modify: `crates/ironclaw_channel_host/src/auth_continuation.rs`
- Modify: `crates/ironclaw_reborn_composition/src/product_auth/api/auth.rs`

**Interfaces:**
- Consumes: `AuthFlowManager::complete_oauth_callback`, `AuthFlowManager::get_flow`, `AuthFlowManager::mark_continuation_dispatched`, and `RebornAuthContinuationDispatcher::dispatch_canceled_auth_continuation`.
- Produces: a provider-denial validator plus one shared canceled-continuation
  dispatch-and-acknowledgement gateway used by provider denial and lifecycle
  cleanup.

- [ ] **Step 1: Add a recording provider-denial callback test**

  Add a test dispatcher that records canceled events in `Mutex<Vec<AuthContinuationEvent>>`. Create an awaiting-user OAuth flow with `AuthContinuationRef::TurnGateResume`, invoke `handle_oauth_callback` with `RebornOAuthCallbackOutcome::ProviderDenied`, and assert all of the following:

  ```rust
  assert_eq!(error.code, AuthErrorCode::ProviderDenied);
  assert_eq!(persisted.status, AuthFlowStatus::Failed);
  assert_eq!(persisted.error, Some(AuthErrorCode::ProviderDenied));
  assert!(persisted.continuation_emitted_at.is_some());
  assert_eq!(dispatcher.canceled_events().len(), 1);
  assert!(matches!(
      dispatcher.canceled_events()[0].continuation,
      AuthContinuationRef::TurnGateResume { .. }
  ));
  ```

  Add a duplicate-callback or reconciliation assertion that the event is not emitted a second time once `continuation_emitted_at` is present.

- [ ] **Step 2: Run the new callback test and verify red**

  Run: `cargo test -p ironclaw_reborn_composition --lib provider_denied_turn_gate -- --nocapture`

  Expected: FAIL because the durable flow becomes `Failed(ProviderDenied)` without dispatching a canceled-auth continuation.

- [ ] **Step 3: Dispatch only the generic turn-gate continuation**

  In the `ProviderDenied` callback arm, reload the exact scoped failed record, dispatch the canceled event only when its continuation is `TurnGateResume`, and mark the flow emitted after dispatch succeeds. Return the expected `AuthProductError::ProviderDenied` once that dispatch completes; surface reload or exhausted-dispatch failures rather than claiming completion. Route the dispatch-plus-marker sequence through the same private gateway as lifecycle cleanup and retry one transient backend failure inline. `SetupOnly`, `LifecycleActivation`, and `ProductActionResume` retain their current terminal behavior.

  ```rust
  let event = AuthContinuationEvent {
      flow_id: failed.id,
      scope: failed.scope.clone(),
      continuation: failed.continuation.clone(),
      provider: failed.provider.clone(),
      credential_account_id: failed.credential_account_id,
      emitted_at: Utc::now(),
  };
  self.continuation_dispatcher
      .dispatch_canceled_auth_continuation(event.clone())
      .await?;
  self.flow_manager
      .mark_continuation_dispatched(&event.scope, event.flow_id, event.emitted_at)
      .await
  ```

  Extend `reconcile_oauth_flow` to retry an unacknowledged `Failed(ProviderDenied)` turn-gate continuation, so a callback-side transient does not permanently park the thread.

- [ ] **Step 4: Clarify the channel-host port contract and run tests**

  Update `dispatch_canceled_auth_continuation` documentation to cover provider denial and lifecycle cancellation without naming a channel.

  Run: `cargo test -p ironclaw_reborn_composition --lib product_auth::api::auth::tests -- --nocapture`

  Expected: PASS with the existing completed, malformed, lifecycle, and callback-race cases unchanged.

### Task 3: Add Slack's Configured Workspace Hint

**Files:**
- Modify: `crates/ironclaw_reborn_composition/src/slack/slack_setup.rs`
- Modify: `crates/ironclaw_reborn_composition/src/product_auth/serve/mod.rs`
- Modify: `crates/ironclaw_reborn_composition/src/slack/slack_personal_oauth.rs`
- Modify: `crates/ironclaw_reborn_composition/src/product_auth/serve/oauth.rs`
- Modify: `crates/ironclaw_reborn_composition/tests/webui_v2_product_auth.rs`

**Interfaces:**
- Consumes: durable `SlackInstallationSetup.team_id`, validated `OAuthClientId`, and `OAuthExtraParam`.
- Produces: `SlackOAuthAuthorizationContext { client_id: OAuthClientId, team_id: SlackTeamId }` and exactly one `team=<configured team id>` query parameter in both Slack start paths.

- [ ] **Step 1: Extend both URL tests with an exact team assertion**

  Parse all `team` query pairs and require the configured fixture team exactly once:

  ```rust
  let teams = parsed
      .query_pairs()
      .filter_map(|(name, value)| (name == "team").then(|| value.into_owned()))
      .collect::<Vec<_>>();
  assert_eq!(teams, vec!["T123"]);
  ```

- [ ] **Step 2: Run the focused Slack URL tests and verify red**

  Run: `cargo test -p ironclaw_reborn_composition --features slack-v2-host-beta slack_personal_oauth_start_uses_server_scopes_not_client_supplied_scopes -- --nocapture`

  Run: `cargo test -p ironclaw_reborn_composition --features slack-v2-host-beta --test webui_v2_product_auth slack_personal_oauth_start_serves_through_composed_router -- --nocapture`

  Expected: both FAIL because the generated URL currently has no `team` parameter.

- [ ] **Step 3: Expose a non-secret authorization context and use it in both builders**

  Add a setup-owned context that validates the configured client and secret handle but returns only the client and team. In the two URL builders, create one typed extra parameter and pass it to the existing generic builder.

  ```rust
  let team = OAuthExtraParam::new("team", authorization.team_id.as_str())?;
  let extra_params = [team];
  let authorization_url = build_authorization_url_with_scope_param(
      OAuthAuthorizeUrlRequest {
          client_id: &authorization.client_id,
          extra_params: &extra_params,
          // existing endpoint, redirect, state, PKCE, and scopes unchanged
      },
      OAuthScopeParam::UserScope,
  )?;
  ```

  Keep `oauth_credentials()` unchanged for token exchange so raw client-secret material never enters the URL context.

- [ ] **Step 4: Run the Slack setup and URL suites**

  Run: `cargo test -p ironclaw_reborn_composition --features slack-v2-host-beta slack_personal_oauth -- --nocapture`

  Expected: PASS and both authorization paths contain the configured workspace hint exactly once.

### Task 4: Prove Channel-Neutral Behavior Through Telegram

**Files:**
- Modify: `tests/integration/telegram_journeys/scenario_decline_in_chat.rs`
- Add: `tests/integration/telegram_journeys/scenario_slack_oauth_cancel_resume.rs`
- Modify: `tests/integration/telegram_journeys/harness.rs`
- Modify: `tests/integration/telegram_journeys/main.rs`
- Modify: `docs/qa/telegram-coverage-map.md`

**Interfaces:**
- Consumes: the shared interaction-command grammar and `DefaultAuthInteractionService` through production composition.
- Produces: two whole-turn regressions proving both adapter-originated explicit
  denial and provider-popup denial free the originating thread, plus a
  production-seam assertion that Slack's authorization link targets the
  configured workspace.

- [ ] **Step 1: Strengthen the journey around model-response consumption**

  Retain the raw scripted model trace in the journey stack and resolve the
  gated run from the durable paired-DM transcript. After Telegram receives
  `Authentication canceled.`, wait for that exact run to reach `Cancelled`,
  then assert the call count is still exactly the two calls used to install
  and activate. Send the next user message and assert that it is the action
  that consumes and delivers the post-denial marker.

  ```rust
  assert_eq!(
      stack.model_trace.calls(),
      2,
      "explicit auth denial must not resume the canceled model run"
  );
  ```

- [ ] **Step 2: Run the journey against the old behavior and verify red**

  Run: `cargo test --test reborn_integration_telegram_journey telegram_dm_auth_deny_command_cancels_gate_and_frees_the_thread -- --nocapture`

  Expected before Task 1 implementation: FAIL because explicit denial resumes and consumes another scripted model response. Expected after Task 1: PASS.

- [ ] **Step 3: Add the provider-popup denial and workspace-targeting journey**

  From a paired Telegram DM, install and activate Slack through the real
  lifecycle capabilities. Parse the delivered Slack URL and assert exactly one
  configured `team`, the expected public redirect, PKCE, user scopes, and no
  client secret. Resolve the exact durable run and require `BlockedAuth`, then
  send `error=access_denied` through the real public Slack callback route with
  a browser `Accept` header. Require a sanitized failure page, durable
  `Failed(ProviderDenied)` with `continuation_emitted_at`, the exact run's
  transition to `Completed`, the resumed model reply in Telegram, and a normal
  second DM on the same thread.

- [ ] **Step 4: Run both named Telegram cancellation journeys**

  Run: `cargo test --test reborn_integration_telegram_journey telegram_dm_auth_deny_command_cancels_gate_and_frees_the_thread -- --nocapture`

  Run: `cargo test --test reborn_integration_telegram_journey telegram_dm_slack_oauth_targets_workspace_and_popup_cancel_resumes_thread -- --nocapture`

  Expected: both PASS through production composition. The provider-popup test
  must leave the original immediate-ACK delivery task alive while invoking the
  callback, matching the real webhook ordering.

### Task 5: Documentation and Verification

**Files:**
- Modify: `CHANGELOG.md`
- Verify: `docs/superpowers/specs/2026-07-18-generic-oauth-denial-lifecycle-design.md`

**Interfaces:**
- Consumes: all completed behavior and tests.
- Produces: release-facing description and a merge-ready branch.

- [ ] **Step 1: Add an Unreleased changelog entry**

  Add one `Fixed` item describing all user-visible outcomes:

  ```markdown
  - *(reborn)* make OAuth denial lifecycle channel-neutral: explicit auth denial cancels the blocked run, provider-popup denial resumes the exact gate as denied, and Slack personal OAuth targets the configured workspace.
  ```

- [ ] **Step 2: Run formatting and focused crate suites**

  Run: `cargo fmt --all -- --check`

  Run: `cargo test -p ironclaw_product_workflow`

  Run: `cargo test -p ironclaw_reborn_composition --features slack-v2-host-beta`

  Expected: all PASS.

- [ ] **Step 3: Run whole-path and lint checks**

  Run: `cargo test --test reborn_integration_telegram_journey telegram_dm_auth_deny_command_cancels_gate_and_frees_the_thread -- --nocapture`

  Run: `cargo clippy -p ironclaw_product_workflow --all-targets --all-features -- -D warnings`

  Run: `cargo clippy -p ironclaw_reborn_composition --all-targets --all-features -- -D warnings`

  Run the workspace-wide clippy command from `.claude/rules/review-discipline.md`, then run: `scripts/pre-commit-safety.sh`

  Expected: all PASS with zero warnings.

- [ ] **Step 4: Audit the final diff**

  Run: `git diff --check`

  Run: `rg -n "\.unwrap\(\)|\.expect\(" crates/ironclaw_product_workflow/src/auth_interaction/service.rs crates/ironclaw_channel_host/src/auth_continuation.rs crates/ironclaw_reborn_composition/src/product_auth/api/auth.rs crates/ironclaw_reborn_composition/src/slack/slack_setup.rs crates/ironclaw_reborn_composition/src/slack/slack_personal_oauth.rs crates/ironclaw_reborn_composition/src/product_auth/serve/mod.rs`

  Expected: no whitespace errors and no newly introduced production unwrap/expect calls.

- [ ] **Step 5: Commit and publish a draft pull request**

  ```bash
  git add -A
  git diff --cached --name-only
  git commit -m "fix(auth): separate explicit and provider denial"
  git push -u origin codex/fix-slack-oauth-cancel-gates
  gh pr create --draft --base main --head codex/fix-slack-oauth-cancel-gates
  ```
