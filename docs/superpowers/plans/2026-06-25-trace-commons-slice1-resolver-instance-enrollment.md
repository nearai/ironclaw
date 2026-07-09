# Trace Commons Slice 1 — Credential resolver + instance enrollment Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

> **Target repo:** `ironclaw`. Independent of
> Slice 0 (it adds a resolver + admin-gated instance enrollment that writes an
> instance-level policy; the per-user `subject` it produces is *consumed* by
> Slice 2). Safe to build and merge before Slice 0 lands.

**Goal:** Add a trace-credential resolver that picks a user's own (personal-invite)
enrollment when present, else falls back to an admin-provisioned instance
enrollment with a per-user pseudonymous subject — and add the admin-gated
instance-enrollment entry point.

**Architecture:** Trace state today is keyed per-scope under
`trace_contribution_dir_for_scope(Some(scope))/policy.json`. We add an
*instance-level* policy (scope `None` → base dir `policy.json`) written only by an
admin path, and a resolver that returns `{ scope_for_state, subject }` so all
downstream callers consult one function instead of reading a per-scope policy
directly. Personal-invite enrollment wins; instance enrollment is the fallback
and carries a per-user subject = `local_pseudonymous_contributor_id("{tenant}:{user}")`.

**Tech Stack:** Rust, tokio, serde, anyhow. Spec:
`docs/superpowers/specs/2026-06-25-trace-commons-instance-enrollment-profiles-inspection-design.md`.

## Global Constraints

- No `.unwrap()`/`.expect()` in production code (tests are fine).
- Zero clippy warnings: `cargo clippy --all --benches --tests --examples --all-features`.
- Map errors with context per CLAUDE.md.
- Prompt/large strings are not involved here.
- Unit tests: `cargo test -p ironclaw_reborn_traces`. Crate-level admin tests:
  `cargo test` (and `--features integration` where DB is needed).
- "Test through the caller" (CLAUDE.md): the resolver gates a network side
  effect, so add a test that drives the *caller* (the first-party dispatch path
  in Slice 3/Task 4 here covers the admin enrollment caller), not only the
  resolver helper.

---

### Task 1: `TraceCredentialResolution` type + resolver

**Files:**
- Modify: `crates/ironclaw_reborn_traces/src/contribution.rs` (add resolver near
  `read_trace_policy_for_scope` at line 4058; reuse `trace_scope_key` line 4001,
  `local_pseudonymous_contributor_id` line 4013)
- Test: same file, test module.

**Interfaces:**
- Produces:
  ```rust
  pub struct TraceCredentialResolution {
      /// The scope string whose local state (policy, device key, credits) to use.
      pub state_scope: String,
      /// Per-user subject to send in upload-claim / login-link requests.
      /// `None` for the personal-invite model (device key already 1:1 with user).
      pub subject: Option<String>,
      /// The resolved enrollment policy.
      pub policy: StandingTraceContributionPolicy,
  }

  pub fn resolve_trace_credentials(
      tenant_id: &str,
      user_id: &str,
  ) -> anyhow::Result<Option<TraceCredentialResolution>>;
  ```
  Returns `None` when neither a personal nor an instance enrollment is enabled.
- Consumes: `read_trace_policy_for_scope`, `trace_scope_key`,
  `local_pseudonymous_contributor_id` (all existing).

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn resolver_prefers_personal_invite_enrollment_with_no_subject() {
    let dir = tempfile::tempdir().unwrap();
    crate::test_support::with_base_dir(dir.path(), || {
        let scope = trace_scope_key("tenant-a", "alice");
        let mut personal = StandingTraceContributionPolicy::default();
        personal.enabled = true;
        write_trace_policy_for_scope(Some(scope.as_str()), &personal).unwrap();

        let r = resolve_trace_credentials("tenant-a", "alice").unwrap().unwrap();
        assert_eq!(r.state_scope, scope);
        assert_eq!(r.subject, None, "personal invite carries no subject");
        assert!(r.policy.enabled);
    });
}

#[test]
fn resolver_falls_back_to_instance_enrollment_with_per_user_subject() {
    let dir = tempfile::tempdir().unwrap();
    crate::test_support::with_base_dir(dir.path(), || {
        // No personal policy; only the instance-level (scope None) policy.
        let mut instance = StandingTraceContributionPolicy::default();
        instance.enabled = true;
        write_trace_policy_for_scope(None, &instance).unwrap();

        let r = resolve_trace_credentials("tenant-a", "alice").unwrap().unwrap();
        let expected_scope = trace_scope_key("tenant-a", "alice");
        assert_eq!(r.subject, Some(local_pseudonymous_contributor_id(&expected_scope)));
        assert!(r.policy.enabled);
    });
}

#[test]
fn resolver_returns_none_when_unenrolled() {
    let dir = tempfile::tempdir().unwrap();
    crate::test_support::with_base_dir(dir.path(), || {
        assert!(resolve_trace_credentials("tenant-a", "alice").unwrap().is_none());
    });
}
```

> If `crate::test_support::with_base_dir` does not exist, the tests must instead
> set the base dir the way the existing onboarding/contribution tests do — check
> how `trace_contribution_dir_for_scope` resolves its base
> (`ironclaw_common::paths::ironclaw_base_dir()`) and mirror the existing
> base-dir override used by tests in this crate (search the test module for how
> other tests isolate the base dir; the e2e test uses `setup_base_dir()`).
> Replace the wrapper with that mechanism if needed — do NOT invent a new global.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p ironclaw_reborn_traces resolver_`
Expected: FAIL — `resolve_trace_credentials` not found.

- [ ] **Step 3: Implement the resolver**

```rust
/// Resolved Trace Commons credentials for a (tenant, user): which local-state
/// scope to use and the per-user subject (if any) to send to the server.
#[derive(Debug, Clone, PartialEq)]
pub struct TraceCredentialResolution {
    pub state_scope: String,
    pub subject: Option<String>,
    pub policy: StandingTraceContributionPolicy,
}

/// Pick the user's own (personal-invite) enrollment when present and enabled,
/// else fall back to the admin-provisioned instance enrollment (scope `None`)
/// with a per-user pseudonymous subject. Returns `None` when neither is enabled.
pub fn resolve_trace_credentials(
    tenant_id: &str,
    user_id: &str,
) -> anyhow::Result<Option<TraceCredentialResolution>> {
    let scope = trace_scope_key(tenant_id, user_id);

    let personal = read_trace_policy_for_scope(Some(scope.as_str()))?;
    if personal.enabled {
        return Ok(Some(TraceCredentialResolution {
            state_scope: scope,
            subject: None,
            policy: personal,
        }));
    }

    let instance = read_trace_policy_for_scope(None)?;
    if instance.enabled {
        return Ok(Some(TraceCredentialResolution {
            subject: Some(local_pseudonymous_contributor_id(&scope)),
            state_scope: scope,
            policy: instance,
        }));
    }

    Ok(None)
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p ironclaw_reborn_traces resolver_`
Expected: PASS (3 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/ironclaw_reborn_traces/src/contribution.rs
git commit -m "feat(traces): trace-credential resolver (personal invite wins, instance fallback w/ subject)"
```

---

### Task 2: Instance-enrollment write path (scope = None)

**Files:**
- Modify: `crates/ironclaw_reborn_traces/src/onboarding/mod.rs` (add a thin
  instance-enrollment entry that targets the base dir)
- Test: `crates/ironclaw_reborn_traces/src/onboarding/tests.rs`

**Interfaces:**
- Consumes: `onboard_at_dir_with_sink` (existing, line 207),
  `trace_contribution_dir_for_scope(None)` (existing, line 3991 in contribution.rs).
- Produces:
  ```rust
  pub async fn onboard_instance_with_sink(
      invite_url: &str,
      consents: OnboardConsents,
      sink: &dyn OnboardingHttpSink,
  ) -> Result<OnboardOutcome, OnboardError>;
  ```
  Writes the enrollment policy to the **instance-level** location
  (`trace_contribution_dir_for_scope(None)/policy.json`), making it the resolver's
  fallback (Task 1).

- [ ] **Step 1: Write the failing test**

```rust
#[tokio::test]
async fn instance_onboard_writes_instance_level_policy() {
    let base = tempfile::tempdir().unwrap();
    // Point the crate base dir at `base` the same way other tests in this file
    // isolate it (mirror `successful_onboard_writes_policy_and_promotes_key`'s
    // dir handling, but for scope None → base/trace_contributions/policy.json).
    let mock = spawn_mock_issuer(
        |addr| ok_response(addr, "https://ingest.example.com"),
        axum::http::StatusCode::OK,
    )
    .await;
    let invite_url = format!("http://127.0.0.1:{}/onboard#INVTEST01", mock.addr.port());

    let outcome = onboard_instance_with_sink(
        &invite_url,
        OnboardConsents::default(),
        &DefaultOnboardingHttpSink,
    )
    .await
    .expect("instance onboard succeeds");

    assert_eq!(outcome.tenant_id, "tenant-a");
    // Instance policy is at the base (scope None), not under users/<hash>/.
    let policy = read_trace_policy_for_scope(None).unwrap();
    assert!(policy.enabled);
    assert_eq!(policy.device_key_id.as_deref(), Some(outcome.device_key_id.as_str()));
}
```

> Use the same base-dir isolation the existing onboarding tests use. The assert
> that matters is "policy lands at scope `None`", which is what makes the
> resolver's instance fallback fire.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p ironclaw_reborn_traces instance_onboard_writes_instance_level_policy`
Expected: FAIL — `onboard_instance_with_sink` not found.

- [ ] **Step 3: Implement the instance-enrollment entry**

In `onboarding/mod.rs`, next to `onboard` (line 195):

```rust
/// Instance-wide enrollment: identical to [`onboard`] but writes the resulting
/// `StandingTraceContributionPolicy` to the instance-level location
/// (`trace_contribution_dir_for_scope(None)`), so all users without their own
/// personal-invite enrollment inherit it via `resolve_trace_credentials`.
///
/// This is an admin-only operation at the call boundary (the host gates it
/// behind `AdminScope`); the function itself only knows it targets the base dir.
pub async fn onboard_instance_with_sink(
    invite_url: &str,
    consents: OnboardConsents,
    sink: &dyn OnboardingHttpSink,
) -> Result<OnboardOutcome, OnboardError> {
    let dir = trace_contribution_dir_for_scope(None);
    onboard_at_dir_with_sink(&dir, invite_url, consents, sink).await
}
```

Ensure `trace_contribution_dir_for_scope` is imported (it already is for the
per-scope `onboard`).

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p ironclaw_reborn_traces instance_onboard_writes_instance_level_policy`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/ironclaw_reborn_traces/src/onboarding/mod.rs crates/ironclaw_reborn_traces/src/onboarding/tests.rs
git commit -m "feat(traces): instance-level enrollment write path (scope None)"
```

---

### Task 3: Admin-gated instance enrollment on `AdminScope` — SUPERSEDED, do not implement

> **Superseded in review (see note at the end of this task):** the
> `AdminScope::enroll_instance_trace_commons` wrapper described below was
> implemented and later REMOVED from `src/tenant.rs` — new Reborn features must
> not land in the retiring v1 monolith. The crate-side entry point is
> `ironclaw_reborn_traces::onboarding::onboard_instance_with_sink` (Task 2);
> an admin-gated Reborn surface will wrap it when instance enrollment gets a
> product entry point. That surface now exists: `ironclaw-reborn traces
> enroll-instance --invite <url>` wraps
> `onboarding::onboard_instance_at_base` (host-shell possession is the admin
> gate, matching `traces opt-in`'s trust boundary for the global policy). The original task text is retained below only as a
> historical record of what was planned.

**Files:**
- Modify: `src/tenant.rs:933-978` (add a method to `AdminScope`)
- Test: `src/tenant.rs` test module (mirror
  `test_admin_scope_new_returns_some_for_admin` at line 1195)

**Interfaces:**
- Consumes: `ironclaw_reborn_traces::onboarding::onboard_instance_with_sink`
  (Task 2) and the host egress onboarding sink. Because `AdminScope` has no
  egress handle, the method takes the sink as a parameter so the host wires it.
- Produces:
  ```rust
  impl AdminScope {
      pub async fn enroll_instance_trace_commons(
          &self,
          invite_url: &str,
          consents: ironclaw_reborn_traces::onboarding::OnboardConsents,
          sink: &dyn ironclaw_reborn_traces::onboarding::OnboardingHttpSink,
      ) -> Result<ironclaw_reborn_traces::onboarding::OnboardOutcome,
                  ironclaw_reborn_traces::onboarding::OnboardError>;
  }
  ```
  The mere existence of `&self: AdminScope` is the gate — it is unconstructable
  for non-admins (`AdminScope::new` returns `None`).

- [ ] **Step 1: Write the failing test**

```rust
#[tokio::test]
async fn admin_scope_exposes_instance_trace_enrollment() {
    // Compile-level gate test: a Regular identity cannot even obtain AdminScope,
    // so the method is unreachable for them. Confirm the method exists on the
    // admin-constructed scope (we don't perform real network here).
    let scope = AdminScope::new(admin_identity(), test_db().await)
        .expect("admin constructs scope");
    // The method is async + network-bound; assert it is addressable by taking a
    // function pointer (no call). This documents the gate without a live POST.
    let _f = AdminScope::enroll_instance_trace_commons;
    let _ = scope; // scope is the gate; non-admins never reach this method.
}
```

> The real network behavior is covered by Task 2's integration test and the
> Slice 3 dispatch caller test. This test asserts the gate placement (method
> lives on `AdminScope`, which non-admins cannot construct).

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test admin_scope_exposes_instance_trace_enrollment`
Expected: FAIL — method `enroll_instance_trace_commons` not found.

- [ ] **Step 3: Implement the method**

Add inside `impl AdminScope` (after `deactivate_user`, line 977):

```rust
    // === Trace Commons instance enrollment ===

    /// Enroll this IronClaw instance with Trace Commons using an operator invite
    /// link. Writes the instance-level enrollment policy that non-personally-
    /// enrolled users inherit via `resolve_trace_credentials`. Admin-only by
    /// construction: `AdminScope` is unconstructable for non-admin identities.
    /// The `sink` is supplied by the host so the POST routes through the
    /// deployment's network-egress policy.
    pub async fn enroll_instance_trace_commons(
        &self,
        invite_url: &str,
        consents: ironclaw_reborn_traces::onboarding::OnboardConsents,
        sink: &dyn ironclaw_reborn_traces::onboarding::OnboardingHttpSink,
    ) -> Result<
        ironclaw_reborn_traces::onboarding::OnboardOutcome,
        ironclaw_reborn_traces::onboarding::OnboardError,
    > {
        ironclaw_reborn_traces::onboarding::onboard_instance_with_sink(
            invite_url, consents, sink,
        )
        .await
    }
```

- [ ] **Step 4: Run test to verify it passes + clippy**

Run: `cargo test admin_scope_exposes_instance_trace_enrollment`
Then: `cargo clippy --all --tests`
Expected: PASS; zero warnings.

- [ ] **Step 5: Commit**

```bash
git add src/tenant.rs
git commit -m "feat(admin): AdminScope::enroll_instance_trace_commons (admin-gated instance enrollment)"
```

> **Superseded in review:** the `AdminScope::enroll_instance_trace_commons`
> wrapper was removed from `src/tenant.rs` — new Reborn features must not land
> in the retiring v1 monolith. The crate-side entry point is
> `ironclaw_reborn_traces::onboarding::onboard_instance_with_sink`; an
> admin-gated Reborn surface will wrap it when instance enrollment gets a
> product entry point. That surface now exists: `ironclaw-reborn traces
> enroll-instance --invite <url>` wraps
> `onboarding::onboard_instance_at_base` (host-shell possession is the admin
> gate, matching `traces opt-in`'s trust boundary for the global policy).

---

## Self-Review

- **Spec coverage:** Spec §"Two coexisting models, one resolver" + precedence →
  Task 1 (`resolve_trace_credentials`, personal wins, instance fallback w/
  subject, `None` when unenrolled). §"IronClaw changes" item 2 (instance
  enrollment, additive, instance-level policy) → Task 2 (`onboard_instance_with_sink`
  → scope `None`) + Task 3 (admin gate). Per-scope flow untouched: `onboard`
  (line 195) is not modified.
- **Placeholder scan:** The only deferred specifics are the crate's base-dir test
  isolation mechanism (named: mirror `successful_onboard_writes_policy_and_promotes_key`
  / `setup_base_dir`), not open TODOs. No "add validation"/"handle edge cases"
  steps.
- **Type consistency:** `TraceCredentialResolution { state_scope, subject, policy }`
  defined Task 1, not referenced by name in later tasks (Slice 2 consumes it).
  `onboard_instance_with_sink(invite_url, consents, sink)` signature identical in
  Task 2 (def) and Task 3 (call). `OnboardConsents`/`OnboardOutcome`/`OnboardError`
  paths match the extracted onboarding module.
- **Note for Slice 2/3/4:** downstream callers must switch from
  `read_trace_policy_for_scope(Some(scope))` to `resolve_trace_credentials(tenant,
  user)` and pass `resolution.subject` into claim/login-link requests, and use
  `resolution.state_scope` for local-state reads.
