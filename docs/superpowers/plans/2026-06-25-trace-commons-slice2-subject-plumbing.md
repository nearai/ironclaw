# Trace Commons Slice 2 — Per-user subject plumbing Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

> **Target repo:** `ironclaw`. **Depends on Slice 0** (server accepting the
> `subject` field) for end-to-end effect, and on **Slice 1** (`TraceCredentialResolution.subject`).
> The client change is safe to merge before Slice 0: a server that ignores the
> extra field still works; the subject simply has no effect until Slice 0 lands.

**Goal:** Carry the resolver's per-user `subject` into the trace upload-claim
request so submissions under the shared instance device key are attributed
per-user.

**Architecture:** `TraceUploadClaimContext` gains an optional `subject`; the
request builder serializes it (only in `DeviceKey` auth mode, mirroring the
`invite_code` precedent); the submission path obtains the subject from
`resolve_trace_credentials` (Slice 1) and threads it into the context.

**Tech Stack:** Rust, serde, reqwest, tokio. Spec:
`docs/superpowers/specs/2026-06-25-trace-commons-instance-enrollment-profiles-inspection-design.md`.

## Global Constraints

- No `.unwrap()`/`.expect()` in production code.
- Zero clippy warnings.
- Backward compatible: when `subject` is `None`, the serialized request body is
  byte-identical to today (field omitted via `skip_serializing_if`).
- Wire field name MUST be `subject` to match the Slice 0 server struct field.
- Tests: `cargo test -p ironclaw_reborn_traces`.

---

### Task 1: Add `subject` to the upload-claim request body

**Files:**
- Modify: `crates/ironclaw_reborn_traces/src/contribution.rs:4546-4568`
  (`TraceUploadClaimIssuerRequest`) and `:5076-5102`
  (`build_trace_upload_claim_issuer_request`) and `:4467-4478`
  (`TraceUploadClaimContext`)
- Test: same file, test module (mirror
  `fetch_trace_upload_claim_from_issuer_accepts_loopback_dev_issuer` at 12558).

**Interfaces:**
- Produces: `TraceUploadClaimContext.subject: Option<String>` and
  `TraceUploadClaimIssuerRequest.subject: Option<String>` (`skip_serializing_if = "Option::is_none"`).
  The builder copies context→request only in `DeviceKey` auth mode.

- [ ] **Step 1: Write the failing test**

Add a test that asserts the serialized request includes `subject` when set in
`DeviceKey` mode and omits it when `None`. The cleanest assertion is on the
builder output (pure function, no network):

```rust
#[test]
fn upload_claim_request_includes_subject_in_device_key_mode() {
    let policy = StandingTraceContributionPolicy {
        enabled: true,
        auth_mode: TraceUploadAuthMode::DeviceKey,
        upload_token_tenant_id: Some("tenant-a".to_string()),
        ..Default::default()
    };
    let ctx = TraceUploadClaimContext {
        trace_id: None,
        submission_id: None,
        consent_scopes: vec![ConsentScope::DebuggingEvaluation],
        allowed_uses: Vec::new(),
        scope_dir: None,
        subject: Some("sha256:deadbeef".to_string()),
    };
    let req = build_trace_upload_claim_issuer_request(&policy, &ctx);
    let json = serde_json::to_value(&req).unwrap();
    assert_eq!(json["subject"], "sha256:deadbeef");
}

#[test]
fn upload_claim_request_omits_subject_when_none() {
    let policy = StandingTraceContributionPolicy {
        enabled: true,
        auth_mode: TraceUploadAuthMode::DeviceKey,
        ..Default::default()
    };
    let ctx = TraceUploadClaimContext {
        trace_id: None,
        submission_id: None,
        consent_scopes: Vec::new(),
        allowed_uses: Vec::new(),
        scope_dir: None,
        subject: None,
    };
    let req = build_trace_upload_claim_issuer_request(&policy, &ctx);
    let json = serde_json::to_value(&req).unwrap();
    assert!(json.get("subject").is_none(), "subject omitted when None");
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p ironclaw_reborn_traces upload_claim_request_`
Expected: FAIL — `TraceUploadClaimContext` has no field `subject` (compile error).

- [ ] **Step 3: Add the fields and builder copy**

Add to `TraceUploadClaimContext` (after `scope_dir`, line ~4477):

```rust
    /// Per-user pseudonymous subject (from `resolve_trace_credentials`). When
    /// set and auth_mode is DeviceKey, it is sent to the issuer so the minted
    /// claim's principal is per-user under the shared instance device key.
    /// `None` for the personal-invite model (device key already 1:1 with user).
    subject: Option<String>,
```

Add to `TraceUploadClaimIssuerRequest` (after `invite_code`, line ~4567):

```rust
    /// Per-user subject; only sent in DeviceKey mode. The server (Slice 0)
    /// derives a per-user principal from it. Omitted when absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    subject: Option<String>,
```

In `build_trace_upload_claim_issuer_request` (line 5076), compute and set it:

```rust
    // Per-user subject only applies to the device-key (instance) path; in
    // WorkloadTokenEnv mode the workload token already identifies the principal.
    let subject = match policy.auth_mode {
        TraceUploadAuthMode::DeviceKey => context.subject.clone(),
        TraceUploadAuthMode::WorkloadTokenEnv => None,
    };
    TraceUploadClaimIssuerRequest {
        schema_version: "ironclaw.trace_upload_claim_request.v1",
        tenant_id: policy.upload_token_tenant_id.clone(),
        audience: policy.upload_token_audience.clone(),
        trace_id: context.trace_id,
        submission_id: context.submission_id,
        consent_scopes: context.consent_scopes.clone(),
        allowed_uses: context.allowed_uses.clone(),
        requested_at: Utc::now(),
        invite_code,
        subject,
    }
```

Then fix every other construction site of `TraceUploadClaimContext` to set
`subject: None` (search the crate for `TraceUploadClaimContext {` and
`TraceUploadClaimContext::for_envelope`). For `for_envelope` (line ~4481), add
`subject: None` to its struct literal — Task 2 adds the setter.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p ironclaw_reborn_traces upload_claim_request_`
Expected: PASS (2 tests). Also run `cargo test -p ironclaw_reborn_traces` to
confirm no other `TraceUploadClaimContext` construction site is left unfixed.

- [ ] **Step 5: Commit**

```bash
git add crates/ironclaw_reborn_traces/src/contribution.rs
git commit -m "feat(traces): carry optional per-user subject in upload-claim request"
```

---

### Task 2: Thread subject from the resolver into submission

**Files:**
- Modify: `crates/ironclaw_reborn_traces/src/contribution.rs` —
  `TraceUploadClaimContext::for_envelope` (line ~4481) gains a `with_subject`
  builder; the submission entry that builds the context
  (`submit_trace_envelope_to_endpoint_with_credential_provider`, line 5978, and
  its callers) sets the subject from `resolve_trace_credentials`.
- Test: same file, test module.

**Interfaces:**
- Consumes: `resolve_trace_credentials` (Slice 1, Task 1).
- Produces: `TraceUploadClaimContext::with_subject(self, Option<String>) -> Self`
  and a submission path that, given `(tenant_id, user_id)`, attaches
  `resolution.subject` to the claim context.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn context_with_subject_sets_field() {
    let ctx = TraceUploadClaimContext {
        trace_id: None,
        submission_id: None,
        consent_scopes: Vec::new(),
        allowed_uses: Vec::new(),
        scope_dir: None,
        subject: None,
    }
    .with_subject(Some("sha256:abc".to_string()));
    assert_eq!(ctx.subject.as_deref(), Some("sha256:abc"));
}
```

> The end-to-end "submission carries subject" behavior is verified against a
> mock issuer in Task 3; this unit test locks the builder.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p ironclaw_reborn_traces context_with_subject_sets_field`
Expected: FAIL — no method `with_subject`.

- [ ] **Step 3: Add the builder and wire the submission path**

Add near `with_scope_dir` (the existing builder used at line ~5985):

```rust
    fn with_subject(mut self, subject: Option<String>) -> Self {
        self.subject = subject;
        self
    }
```

In the submission path that has `(tenant_id, user_id)` in scope (the scope-aware
submit caller — search for the function that calls
`submit_trace_envelope_to_endpoint_with_credential_provider` with a scope/user),
resolve and attach the subject. Where the context is built with `scope_dir`,
extend it:

```rust
    // Attach the per-user subject so instance-enrolled users are attributed
    // individually. Personal-invite enrollments resolve to `subject: None`.
    // Propagate resolver failures with `?` — do NOT `.ok().flatten()` them
    // into `subject: None`, which would mis-mint instance-enrolled claims
    // and mask backend problems.
    let subject = crate::contribution::resolve_trace_credentials(tenant_id, user_id)?
        .and_then(|r| r.subject);
    let context = TraceUploadClaimContext::for_envelope(envelope)
        .with_scope_dir(dir.to_path_buf())
        .with_subject(subject);
```

> If the submission entry point does not currently receive `(tenant_id, user_id)`
> (only a `scope` string or `scope_dir`), thread them through from the nearest
> caller that has them (the host dispatch / web handler passes user id). Prefer
> adding parameters over re-deriving identity. Keep `subject: None` for any CLI/
> worker path that has no user context — that preserves today's behavior.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p ironclaw_reborn_traces context_with_subject_sets_field`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/ironclaw_reborn_traces/src/contribution.rs
git commit -m "feat(traces): thread resolver subject into submission claim context"
```

---

### Task 3: End-to-end — submission sends subject to the issuer

**Files:**
- Test: `crates/ironclaw_reborn_traces/src/contribution.rs` test module (mirror
  the loopback mock-issuer test at 12558, capturing the request body).

**Interfaces:**
- Consumes: Tasks 1–2.
- Produces: a regression test that the minted-claim request body contains the
  expected `subject` for an instance-enrolled user, and omits it for a
  personal-invite user.

- [ ] **Step 1: Write the failing test**

```rust
#[tokio::test]
async fn fetch_claim_sends_subject_when_present() {
    use std::sync::{Arc, Mutex};
    let captured: Arc<Mutex<Vec<serde_json::Value>>> = Arc::new(Mutex::new(Vec::new()));
    let cap = captured.clone();
    let token = test_jwt_with_header(serde_json::json!({"alg":"EdDSA","kid":"dev-key-1"}));
    let claim_token = token.clone();
    let app = axum::Router::new().route(
        "/v1/trace-upload-claim",
        axum::routing::post(move |axum::Json(body): axum::Json<serde_json::Value>| {
            let cap = cap.clone();
            let token = claim_token.clone();
            async move {
                cap.lock().unwrap().push(body);
                axum::Json(serde_json::json!({
                    "access_token": token, "token_type": "Bearer", "expires_in": 300
                }))
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { let _ = axum::serve(listener, app).await; });

    let scope_dir = tempfile::tempdir().unwrap();
    crate::onboarding::DeviceKeypair::load_or_generate_pending(scope_dir.path(), "h")
        .unwrap()
        .promote(scope_dir.path(), "tenant-dev")
        .unwrap();

    let policy = StandingTraceContributionPolicy {
        enabled: true,
        auth_mode: TraceUploadAuthMode::DeviceKey,
        upload_token_issuer_url: Some(format!("http://{addr}/v1/trace-upload-claim")),
        upload_token_issuer_allowed_hosts: std::collections::BTreeSet::from(["127.0.0.1".to_string()]),
        upload_token_tenant_id: Some("tenant-dev".to_string()),
        upload_token_audience: Some("trace-commons".to_string()),
        ..Default::default()
    };
    let context = TraceUploadClaimContext {
        trace_id: None, submission_id: None,
        consent_scopes: vec![ConsentScope::DebuggingEvaluation],
        allowed_uses: Vec::new(),
        scope_dir: Some(scope_dir.path().to_path_buf()),
        subject: Some("sha256:alice".to_string()),
    };
    let _ = fetch_trace_upload_claim_from_issuer(&policy, &context, None).await.unwrap();

    let bodies = captured.lock().unwrap();
    assert_eq!(bodies.len(), 1);
    assert_eq!(bodies[0]["subject"], "sha256:alice");
}
```

- [ ] **Step 2: Run test to verify it fails (then passes after Tasks 1–2)**

Run: `cargo test -p ironclaw_reborn_traces fetch_claim_sends_subject_when_present`
Expected: with Tasks 1–2 implemented, PASS. (If run before them, it fails to
compile / the body lacks `subject`.)

- [ ] **Step 3: (no new production code)** — this task is the regression lock.

- [ ] **Step 4: Full crate test + clippy**

Run: `cargo test -p ironclaw_reborn_traces` then
`cargo clippy --all --tests`
Expected: PASS; zero warnings.

- [ ] **Step 5: Commit**

```bash
git add crates/ironclaw_reborn_traces/src/contribution.rs
git commit -m "test(traces): claim request carries per-user subject end-to-end"
```

---

## Self-Review

- **Spec coverage:** Spec §"IronClaw changes" item 3 (per-user subject plumbing
  through claim + submission) → Task 1 (request/context field + builder), Task 2
  (resolver→context wiring), Task 3 (e2e regression). Backward-compat → Task 1
  `skip_serializing_if` + `upload_claim_request_omits_subject_when_none`.
- **Placeholder scan:** Task 2 Step 3 flags a threading decision (pass
  `tenant_id`/`user_id` from the nearest caller) with an explicit rule
  (add params, don't re-derive; `None` for CLI/worker) — not an open TODO.
- **Type consistency:** field name `subject` consistent across
  `TraceUploadClaimContext`, `TraceUploadClaimIssuerRequest`, the wire JSON, and
  the Slice 0 server struct. `with_subject(Option<String>) -> Self` matches its
  call in Task 2. `resolve_trace_credentials(tenant, user) -> Option<TraceCredentialResolution>`
  matches Slice 1 Task 1.
