# Trace Commons Slice 3 — Account login-link capability Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

> **Target repo:** `ironclaw`. **Depends on Slice 0** (server `/v1/account/login-links`
> + per-user subject) and **Slices 1–2** (resolver + subject). Until Slice 0 is
> live, the capability returns the server's error verbatim.

**Goal:** Add a consent-gated, model-visible first-party capability
`trace_commons.account_login_link` that mints a one-time Trace Commons browser
login URL for the authenticated user (so they can manage their contributor
profile / account in the web UI), routed through host network egress.

**Architecture:** New capability mirrors `trace_commons.profile_token`: consent
gate → enrollment pre-check via `resolve_trace_credentials` → `HostEgressContributionSink`
→ a new `ironclaw_reborn_traces` sink call that POSTs `/v1/account/login-links`
with the per-user subject and returns the `{ account_id, url }` response. The raw
URL is returned to the user (it is a one-time, user-facing link, not a stored
bearer credential).

**Tech Stack:** Rust, async_trait, serde_json. Spec:
`docs/superpowers/specs/2026-06-25-trace-commons-instance-enrollment-profiles-inspection-design.md`.

## Global Constraints

- No `.unwrap()`/`.expect()` in production code.
- Zero clippy warnings.
- The mint POST MUST route through `RuntimeHttpEgress` (host egress) — never a
  direct client (mirrors `dispatch_onboard`/`dispatch_profile_token`).
- Consent gate is the hard fail-closed boundary: no network call unless
  `confirmed == true`.
- Capability id: `builtin.trace_commons.account_login_link`. Schema file:
  `schemas/builtin/trace_commons-account_login_link.input.v1.json`.
- Tests: `cargo test -p ironclaw_reborn_traces` (sink call) and
  `cargo test --package ironclaw_host_runtime --test trace_commons_dispatch_e2e`
  (dispatch caller).

---

### Task 1: `mint_account_login_link_for_scope_via_sink` in reborn_traces

**Files:**
- Modify: `crates/ironclaw_reborn_traces/src/contribution.rs` (add near
  `mint_profile_attribution_token_for_scope_via_sink` at line 5546)
- Test: same file, test module (mirror the mock-issuer pattern at 12558).

**Interfaces:**
- Consumes: `ContributionHttpSink` (line 5110), `resolve_trace_credentials`
  (Slice 1), `read_trace_policy_for_scope`.
- Produces:
  ```rust
  pub struct AccountLoginLink { pub account_id: String, pub url: String }

  pub async fn mint_account_login_link_via_sink(
      tenant_id: &str,
      user_id: &str,
      sink: &dyn ContributionHttpSink,
  ) -> anyhow::Result<AccountLoginLink>;
  ```
  POSTs `{ "subject": <resolution.subject> }` (subject omitted when `None`) to
  `<issuer-origin>/v1/account/login-links` with the per-user bearer, parses
  `{ account_id, url }`. The login-links URL is derived from the policy's
  issuer/ingest origin (same origin family as the upload-claim issuer).

- [ ] **Step 1: Write the failing test**

```rust
#[tokio::test]
async fn mint_account_login_link_posts_subject_and_returns_url() {
    use std::sync::{Arc, Mutex};
    let captured: Arc<Mutex<Vec<serde_json::Value>>> = Arc::new(Mutex::new(Vec::new()));
    let cap = captured.clone();
    let app = axum::Router::new().route(
        "/v1/account/login-links",
        axum::routing::post(move |axum::Json(b): axum::Json<serde_json::Value>| {
            let cap = cap.clone();
            async move {
                cap.lock().unwrap().push(b);
                axum::Json(serde_json::json!({
                    "account_id": "11111111-1111-1111-1111-111111111111",
                    "url": "/account/login?code=abc"
                }))
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { let _ = axum::serve(listener, app).await; });

    // Instance-enrolled policy at scope None so the resolver yields a subject.
    let base = tempfile::tempdir().unwrap();
    // (isolate base dir as other tests in this module do)
    let mut policy = StandingTraceContributionPolicy::default();
    policy.enabled = true;
    policy.auth_mode = TraceUploadAuthMode::DeviceKey;
    policy.upload_token_issuer_url = Some(format!("http://{addr}/v1/trace-upload-claim"));
    policy.upload_token_issuer_allowed_hosts = std::collections::BTreeSet::from(["127.0.0.1".to_string()]);
    policy.upload_token_tenant_id = Some("tenant-dev".to_string());
    write_trace_policy_for_scope(None, &policy).unwrap();
    // device key + bearer prerequisites mirror fetch-claim tests.

    let sink = TestContributionSink::new(); // crate test sink that hits the mock
    let link = mint_account_login_link_via_sink("tenant-dev", "alice", &sink).await.unwrap();
    assert_eq!(link.url, "/account/login?code=abc");
    let bodies = captured.lock().unwrap();
    assert_eq!(bodies[0]["subject"], local_pseudonymous_contributor_id(&trace_scope_key("tenant-dev","alice")));
}
```

> Reuse whatever in-crate test `ContributionHttpSink` impl the existing
> sink-based tests use (search the test module for an impl of
> `ContributionHttpSink`; the profile-set/profile-token sink tests have one). If
> none exists, add a minimal test sink that forwards to a reqwest call against
> the mock — mirror how `set_community_profile_for_scope_via_sink` is tested.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p ironclaw_reborn_traces mint_account_login_link_posts_subject`
Expected: FAIL — function not found.

- [ ] **Step 3: Implement the sink call**

Add (mirroring `set_community_profile_for_scope_inner`'s URL/derivation + the
`ContributionHttpRequest` shape):

```rust
/// One-time browser login link for the Trace Commons contributor account.
#[derive(Debug, Clone)]
pub struct AccountLoginLink {
    pub account_id: String,
    pub url: String,
}

/// Mint a one-time account login link for `(tenant, user)`. POSTs the per-user
/// subject (when the instance model is active) to `/v1/account/login-links` so
/// the server resolves the correct per-user account. Routes through the caller-
/// supplied sink (host egress on the agent path).
pub async fn mint_account_login_link_via_sink(
    tenant_id: &str,
    user_id: &str,
    sink: &dyn ContributionHttpSink,
) -> anyhow::Result<AccountLoginLink> {
    let resolution = resolve_trace_credentials(tenant_id, user_id)?
        .ok_or_else(|| anyhow::anyhow!("not enrolled in Trace Commons"))?;
    let context = TraceUploadClaimContext::for_account(resolution.subject.clone());
    // Mint the per-user bearer the same way submission does.
    let provider = DefaultTraceUploadCredentialProvider;
    let bearer = provider
        .bearer_token(&resolution.policy, &context, false)
        .await?;
    let url = account_login_links_url(&resolution.policy)?; // origin + /v1/account/login-links
    let body = match &resolution.subject {
        Some(s) => serde_json::json!({ "subject": s }),
        None => serde_json::json!({}),
    };
    let response = sink
        .execute(ContributionHttpRequest {
            method: ContributionHttpMethod::Post,
            url,
            bearer_token: Some(bearer),
            json_body: Some(serde_json::to_vec(&body)?),
            response_body_limit: TRACE_UPLOAD_CLAIM_MAX_RESPONSE_BYTES as u64,
            timeout_ms: 10_000,
        })
        .await
        .map_err(|e| anyhow::anyhow!("login-link request failed: {e}"))?;
    anyhow::ensure!(
        (200..300).contains(&response.status),
        "login-link request returned status {}",
        response.status
    );
    let parsed: serde_json::Value =
        serde_json::from_slice(&response.body).context("login-link response was not valid JSON")?;
    let account_id = parsed["account_id"].as_str().unwrap_or_default().to_string();
    let link = parsed["url"].as_str().unwrap_or_default().to_string();
    anyhow::ensure!(!link.is_empty(), "login-link response missing url");
    Ok(AccountLoginLink { account_id, url: link })
}
```

Add helpers: `account_login_links_url(policy)` derives the origin from
`policy.upload_token_issuer_url` (strip `/v1/trace-upload-claim`, append
`/v1/account/login-links`); `TraceUploadClaimContext::for_account(subject)`
builds a context with no trace/submission id and the given subject (account-mgmt
scope). If a context constructor without an envelope doesn't exist, add a small
one mirroring `for_envelope` but with `trace_id/submission_id = None`.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p ironclaw_reborn_traces mint_account_login_link_posts_subject`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/ironclaw_reborn_traces/src/contribution.rs
git commit -m "feat(traces): mint_account_login_link_via_sink (POST /v1/account/login-links)"
```

---

### Task 2: First-party capability `trace_commons.account_login_link`

**Files:**
- Modify: `crates/ironclaw_host_runtime/src/first_party_tools/trace_commons.rs`
  (add const + manifest + `dispatch_account_login_link`, mirroring
  `dispatch_profile_token` at 657 and `profile_token_manifest`)
- Modify: `crates/ironclaw_host_runtime/src/first_party_tools/mod.rs`
  (register manifest at ~177, handler at ~301, dispatch arm at ~414)
- Modify: `crates/ironclaw_host_runtime/src/first_party_tools/schemas.rs`
  (add input schema arm near 217)
- Test: `crates/ironclaw_host_runtime/tests/trace_commons_dispatch_e2e.rs`

**Interfaces:**
- Consumes: `mint_account_login_link_via_sink` (Task 1), `HostEgressContributionSink`
  (existing, line 350), `resolve_trace_credentials`.
- Produces: capability `builtin.trace_commons.account_login_link`, model-visible,
  `PermissionMode::Ask`, effects `[Network, ExternalWrite]`. Output:
  `{ "minted": true, "url": "...", "message": "..." }` or a sanitized error.

- [ ] **Step 1: Write the failing test**

Mirror `onboard_then_status_through_dispatch` (line 369). After onboarding the
scope, invoke the new capability with `confirmed=true` against a mock that serves
`/v1/account/login-links`, and assert the returned `url`:

```rust
#[tokio::test]
async fn account_login_link_through_dispatch() {
    let _base = setup_base_dir();
    // onboard first (reuse the onboard mock flow from onboard_then_status), then:
    let (addr, _received) = spawn_mock_account_login(/* serves {account_id,url} */).await;
    // point the onboarded policy's issuer origin at `addr` (the onboard mock can
    // serve both /v1/trace-upload-claim and /v1/account/login-links).
    let result = invoke_with_context(
        &rt,
        TRACE_COMMONS_ACCOUNT_LOGIN_LINK_CAPABILITY_ID,
        json!({ "confirmed": true }),
        execution_context_with_network("user_x","caller_x",
            TRACE_COMMONS_ACCOUNT_LOGIN_LINK_CAPABILITY_ID, allow_all_network_policy()),
    ).await.expect("dispatch ok");
    assert_eq!(result["minted"], json!(true));
    assert!(result["url"].as_str().unwrap().contains("/account/login?code="));
}

#[tokio::test]
async fn account_login_link_requires_consent() {
    let _base = setup_base_dir();
    let rt = runtime();
    let result = invoke_with_context(
        &rt, TRACE_COMMONS_ACCOUNT_LOGIN_LINK_CAPABILITY_ID, json!({}),
        execution_context_read_only("u","c", TRACE_COMMONS_ACCOUNT_LOGIN_LINK_CAPABILITY_ID),
    ).await.expect("dispatch ok");
    assert_eq!(result["consent_required"], json!(true));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --package ironclaw_host_runtime --test trace_commons_dispatch_e2e account_login_link`
Expected: FAIL — unknown capability id.

- [ ] **Step 3: Implement the capability**

In `trace_commons.rs`, add the const next to the others (line 55):

```rust
pub const TRACE_COMMONS_ACCOUNT_LOGIN_LINK_CAPABILITY_ID: &str =
    "builtin.trace_commons.account_login_link";
```

Add a manifest builder mirroring `profile_token_manifest`:

```rust
pub(super) fn account_login_link_manifest() -> Result<CapabilityManifest, ExtensionError> {
    first_party_capability_manifest(
        TRACE_COMMONS_ACCOUNT_LOGIN_LINK_CAPABILITY_ID,
        "Mint a one-time Trace Commons browser login link so the user can manage \
         their contributor account/profile in the web UI. Consent-gated: only call \
         with confirmed=true after the user explicitly asks. Routes through host \
         network egress.",
        vec![EffectKind::Network, EffectKind::ExternalWrite],
        PermissionMode::Ask,
        resource_profile(), // reuse the same helper profile_token uses
    )
}
```

Add the dispatch handler mirroring `dispatch_profile_token` (consent gate →
enrollment pre-check via `resolve_trace_credentials` → egress sink → call):

```rust
pub(super) async fn dispatch_account_login_link(
    request: &FirstPartyCapabilityRequest,
) -> Result<Value, FirstPartyCapabilityError> {
    let confirmed = request.input.get("confirmed").and_then(Value::as_bool).unwrap_or(false);
    if !confirmed {
        return Ok(json!({
            "minted": false,
            "consent_required": true,
            "message": "Minting a Trace Commons login link opens a browser session to \
        manage the user's contributor account. Confirm with the user, then call again \
        with confirmed=true."
        }));
    }
    let tenant_id = request.scope.tenant_id.as_str();
    let user_id = request.scope.user_id.as_str();
    match ironclaw_reborn_traces::contribution::resolve_trace_credentials(tenant_id, user_id) {
        Ok(Some(r)) if r.policy.enabled => {}
        Ok(_) => return Ok(account_login_link_error_value("not enrolled in Trace Commons".into())),
        Err(e) => return Ok(account_login_link_error_value(e.to_string())),
    }
    let egress = match request.services.runtime_http_egress.as_ref() {
        Some(e) => e.clone(),
        None => return Err(FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::NetworkDenied)),
    };
    let sink = HostEgressContributionSink {
        egress,
        scope: request.scope.clone(),
        capability_id: request.capability_id.clone(),
    };
    match ironclaw_reborn_traces::contribution::mint_account_login_link_via_sink(
        tenant_id, user_id, &sink,
    ).await {
        Ok(link) => Ok(json!({
            "minted": true,
            "url": link.url,
            "message": "Open this one-time link in a browser to manage your Trace Commons account. \
        It expires shortly and can be used once."
        })),
        Err(e) => Ok(account_login_link_error_value(e.to_string())),
    }
}

fn account_login_link_error_value(error: String) -> Value {
    let (code, message) = if error.contains("not enrolled") {
        ("NotEnrolled", "Trace Commons enrollment was not found for this user.")
    } else {
        ("LoginLinkMintFailed", "Could not mint a Trace Commons login link. Check enrollment and retry.")
    };
    json!({ "minted": false, "error_code": code, "message": message })
}
```

In `mod.rs`: add `trace_commons::account_login_link_manifest()?` to the manifest
list (~177), an `insert_handler` for the new id (~301), and the dispatch arm:

```rust
    TRACE_COMMONS_ACCOUNT_LOGIN_LINK_CAPABILITY_ID => {
        (trace_commons::dispatch_account_login_link(&request).await?, None)
    }
```

(Import the new const alongside the existing `TRACE_COMMONS_*` ids.)

In `schemas.rs`, add the arm near 217:

```rust
"schemas/builtin/trace_commons-account_login_link.input.v1.json" => json!({
    "type": "object",
    "properties": {
        "confirmed": {
            "type": "boolean",
            "description": "Must be true only after the user explicitly asked to open a Trace Commons account/profile login link in this conversation (default: false)"
        }
    },
    "additionalProperties": false
}),
```

- [ ] **Step 4: Run tests to verify they pass + clippy**

Run: `cargo test --package ironclaw_host_runtime --test trace_commons_dispatch_e2e account_login_link`
Then: `cargo clippy --all --tests`
Expected: PASS; zero warnings.

- [ ] **Step 5: Commit**

```bash
git add crates/ironclaw_host_runtime/src/first_party_tools/
git commit -m "feat(traces): trace_commons.account_login_link first-party capability"
```

---

## Self-Review

- **Spec coverage:** Spec §"User profiles — login-link capability" → Task 1 (sink
  call posting subject to `/v1/account/login-links`) + Task 2 (consent-gated,
  egress-routed, model-visible capability). Consent gate (hard boundary) →
  Task 2 `account_login_link_requires_consent`. Subject carried → Task 1 test
  asserts the posted `subject`.
- **Placeholder scan:** Test sink / mock-server helpers are specified by
  reference to the exact existing patterns (`set_community_profile_for_scope_via_sink`
  test, `spawn_mock_issuer`, `onboard_then_status_through_dispatch`). New helpers
  (`account_login_links_url`, `for_account`) have concrete derivation rules. No
  open TODOs.
- **Type consistency:** `AccountLoginLink { account_id, url }` (Task 1) consumed
  in Task 2; capability id const name `TRACE_COMMONS_ACCOUNT_LOGIN_LINK_CAPABILITY_ID`
  identical across trace_commons.rs/mod.rs/tests; `HostEgressContributionSink`
  fields `{ egress, scope, capability_id }` match the existing struct (line 350).
