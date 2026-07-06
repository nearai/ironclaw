# Trace Commons Slice 4 — Trace inspection Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.
>
> **Also read first:** the `reborn-feature` skill — this slice crosses
> reborn_traces → product_workflow facade → webui_v2 handler → frontend, exactly
> the layering that skill maps. Wire it in one pass per that guidance.

> **Target repo:** `ironclaw`. **Depends on Slice 0** (server `/v1/account/traces*`)
> and **Slices 1–2** (resolver + per-user bearer/subject). Until Slice 0 ships,
> the fetch returns the server's empty/error and the UI shows the zero-state.

**Goal:** Let a user inspect their *submitted* Trace Commons traces (list +
scrubbed content) from the IronClaw web UI, fetched per-user from the server via
the instance device bearer, complementing the existing local held-trace review.

**Architecture:** Mirror the existing read-only credit surface end to end:
`TraceClientHost` gains an account-traces fetch (per-user bearer) → the
`product_workflow` services facade gains a `trace_account_traces(caller)` method →
the `webui_v2` handler `GET /api/webchat/v2/traces/account` returns it → a React
hook/component renders it under the Trace Commons settings tab. Scope is always
derived from the authenticated caller, never the request.

**Tech Stack:** Rust (axum, tokio), React (htm/preact + @tanstack/react-query).
Spec: `docs/superpowers/specs/2026-06-25-trace-commons-instance-enrollment-profiles-inspection-design.md`.

## Global Constraints

- Scope derived from the authenticated caller only — no scope/user input from the
  request (matches `trace_credits`).
- Read-only projection of server state; no raw bearer or device-key material in
  any response; sanitize errors at the HTTP boundary (`WebUiV2HttpError`).
- No `.unwrap()`/`.expect()` in production code; zero clippy warnings.
- Rust tests: `cargo test -p ironclaw_reborn_traces`, and
  `cargo test -p ironclaw_webui_v2 --features webui-v2-beta`. Frontend assets are
  embedded at compile time (`assets.rs`); no JS test harness — verify via the
  handler contract test + manual.

---

### Task 1: `fetch_account_traces` in reborn_traces (per-user list)

**Files:**
- Modify: `crates/ironclaw_reborn_traces/src/client.rs` (add an account-traces
  fetch alongside the existing remote sync) and/or `contribution.rs` for the
  request shaping (mirror `mint_account_login_link_via_sink` from Slice 3).
- Test: the owning file's test module (mock-issuer pattern).

**Interfaces:**
- Consumes: `resolve_trace_credentials` (Slice 1), `ContributionHttpSink`,
  `DefaultTraceUploadCredentialProvider` (per-user bearer).
- Produces:
  ```rust
  #[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
  pub struct AccountTraceItem {
      pub submission_id: String,
      pub status: String,
      pub credit_points_pending: f32,
      pub credit_points_final: Option<f32>,
      pub received_at: Option<String>,
  }

  pub async fn fetch_account_traces_via_sink(
      tenant_id: &str,
      user_id: &str,
      limit: Option<usize>,
      sink: &dyn ContributionHttpSink,
  ) -> anyhow::Result<Vec<AccountTraceItem>>;
  ```
  GETs `<origin>/v1/account/traces?limit=N` with the per-user bearer; maps the
  server's list items to `AccountTraceItem` (only the sanitized projection
  fields the UI needs). `ContributionHttpMethod` has no `Get`; add a `Get`
  variant (and map it in `HostEgressContributionSink`) OR perform the GET via the
  existing hardened reqwest path used by `read_local_records_for_scope`/sync.
  Prefer extending `ContributionHttpMethod` with `Get` so the host-egress path is
  reused — see Step 3.

- [ ] **Step 1: Write the failing test**

```rust
#[tokio::test]
async fn fetch_account_traces_returns_user_submissions() {
    let app = axum::Router::new().route(
        "/v1/account/traces",
        axum::routing::get(|| async {
            axum::Json(serde_json::json!([
                { "submission_id": "s1", "status": "accepted",
                  "credit_points_pending": 1.0, "credit_points_final": 1.0,
                  "received_at": "2026-06-25T00:00:00Z" }
            ]))
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { let _ = axum::serve(listener, app).await; });

    // instance-enrolled policy at scope None (see Slice 3 Task 1 setup), device key promoted.
    // ...write_trace_policy_for_scope(None, &policy) with issuer origin = addr...

    let sink = TestContributionSink::new();
    let items = fetch_account_traces_via_sink("tenant-dev", "alice", Some(50), &sink).await.unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].submission_id, "s1");
    assert_eq!(items[0].status, "accepted");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p ironclaw_reborn_traces fetch_account_traces_returns_user_submissions`
Expected: FAIL — function not found.

- [ ] **Step 3: Implement**

Add `Get` to `ContributionHttpMethod` (contribution.rs:5118) and map it in the
host sink (`HostEgressContributionSink::execute`, first_party_tools/trace_commons.rs:367):

```rust
// in ContributionHttpMethod
    Get,
// in HostEgressContributionSink::execute match
    ContributionHttpMethod::Get => NetworkMethod::Get,
```

Then implement `fetch_account_traces_via_sink` mirroring
`mint_account_login_link_via_sink` (Slice 3 Task 1): resolve credentials, mint the
per-user bearer, build `<origin>/v1/account/traces?limit=N` via the same origin
helper, `sink.execute` a `Get`, parse the JSON array into `Vec<AccountTraceItem>`
(use serde derive on `AccountTraceItem`; unknown server fields ignored). Only
the explicit zero-states return `Ok(vec![])`: unenrolled (resolver returns
`None`) and HTTP 404 (enrolled principal with no account/traces yet). Every
other non-2xx (401/403/429/5xx) and all transport failures return `Err` so
backend outages surface instead of hiding behind an empty-list UI.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p ironclaw_reborn_traces fetch_account_traces_returns_user_submissions`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/ironclaw_reborn_traces/src/
git commit -m "feat(traces): fetch_account_traces_via_sink (GET /v1/account/traces, per-user)"
```

---

### Task 2: product_workflow facade method

**Files:**
- Modify: the services facade trait + default impl that backs
  `state.services().trace_credits(...)` (in `crates/ironclaw_product_workflow/`;
  find it via `grep -rn "fn trace_credits" crates/ironclaw_product_workflow`).
- Test: the facade's own tests if present; otherwise covered by Task 3's contract test.

**Interfaces:**
- Consumes: `fetch_account_traces_via_sink` (Task 1).
- Produces: a facade method
  ```rust
  async fn trace_account_traces(
      &self,
      caller: WebUiAuthenticatedCaller,
  ) -> Result<RebornAccountTracesResponse, ...>;
  ```
  deriving `(tenant_id, user_id)` from `caller`, calling the reborn_traces fetch
  through the host egress sink the facade already uses for trace operations, and
  returning a sanitized `RebornAccountTracesResponse { enrolled: bool, traces: Vec<RebornAccountTrace> }`.

- [ ] **Step 1: Write the failing test (or rely on Task 3)**

If the facade trait has a `StubServices` default (used by the contract test),
add a default `trace_account_traces` returning the unenrolled zero-state
(`{ enrolled: false, traces: [] }`) so the contract test in Task 3 compiles
against the trait. Write the contract assertion in Task 3.

- [ ] **Step 2: Implement the facade method + types**

Define the wire types next to the existing `RebornTraceCreditsResponse`:

```rust
#[derive(Debug, Clone, serde::Serialize)]
pub struct RebornAccountTrace {
    pub submission_id: String,
    pub status: String,
    pub pending_credit: f32,
    pub final_credit: Option<f32>,
    pub received_at: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct RebornAccountTracesResponse {
    pub enrolled: bool,
    pub traces: Vec<RebornAccountTrace>,
}
```

Implement the real (non-stub) method to derive `(tenant, user)` from `caller`,
call `fetch_account_traces_via_sink` via the facade's host-egress sink, map
`AccountTraceItem` → `RebornAccountTrace`, and set `enrolled` from
`resolve_trace_credentials(...).is_some()`. Add the matching default to
`StubServices` returning `{ enrolled: false, traces: vec![] }`.

- [ ] **Step 3: Build**

Run: `cargo build -p ironclaw_product_workflow`
Expected: compiles.

- [ ] **Step 4: Commit**

```bash
git add crates/ironclaw_product_workflow/
git commit -m "feat(reborn): trace_account_traces facade method + wire types"
```

---

### Task 3: webui_v2 handler + route + contract test

**Files:**
- Modify: `crates/ironclaw_webui_v2/src/handlers.rs` (add `trace_account_traces`
  handler mirroring `trace_credits` at 671-686)
- Modify: the v2 router registration (find via
  `grep -rn "traces/credit" crates/ironclaw_webui_v2/src`) to add the new route
- Test: `crates/ironclaw_webui_v2/tests/webui_v2_handlers_contract.rs` (mirror
  `trace_credits_returns_caller_scoped_unenrolled_zero_state` at 1731)

**Interfaces:**
- Consumes: facade `trace_account_traces` (Task 2).
- Produces: `GET /api/webchat/v2/traces/account` →
  `Json<RebornAccountTracesResponse>`.

- [ ] **Step 1: Write the failing contract test**

```rust
#[tokio::test]
async fn trace_account_traces_returns_caller_scoped_unenrolled_zero_state() {
    let user_id = format!(
        "webui-v2-account-traces-{}-{}",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    );
    let caller = WebUiAuthenticatedCaller::new(
        TenantId::new("tenant-alpha").unwrap(),
        UserId::new(user_id.as_str()).unwrap(),
        None, None,
    );
    let router = webui_v2_router(WebUiV2State::new(
        Arc::new(StubServices::default()),
        DEFAULT_SSE_MAX_CONCURRENT_PER_CALLER,
    ))
    .layer(axum::Extension(caller))
    .layer(axum::Extension(WebUiV2Capabilities::default()));

    let response = router.oneshot(
        Request::builder().method(Method::GET)
            .uri("/api/webchat/v2/traces/account")
            .body(Body::empty()).unwrap(),
    ).await.unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = read_json(response).await;
    assert_eq!(body["enrolled"], false);
    assert_eq!(body["traces"].as_array().unwrap().len(), 0);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p ironclaw_webui_v2 --features webui-v2-beta trace_account_traces_returns_caller_scoped`
Expected: FAIL — 404 (route not registered).

- [ ] **Step 3: Implement handler + route**

In `handlers.rs` (after `trace_credits`):

```rust
/// `GET /api/webchat/v2/traces/account`
///
/// Read-only list of the authenticated caller's submitted Trace Commons traces,
/// fetched per-user from the server. Scope is derived from the caller; no input
/// is accepted. Unenrolled callers receive the zero-state, not an error.
pub async fn trace_account_traces(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
) -> Result<Json<RebornAccountTracesResponse>, WebUiV2HttpError> {
    let response = state.services().trace_account_traces(caller).await?;
    Ok(Json(response))
}
```

Register the route next to `traces/credit` (mirror its `.route(...)` line):

```rust
        .route("/api/webchat/v2/traces/account", get(trace_account_traces))
```

(Import `RebornAccountTracesResponse` and `get` as needed.)

- [ ] **Step 4: Run test to verify it passes + clippy**

Run: `cargo test -p ironclaw_webui_v2 --features webui-v2-beta trace_account_traces_returns_caller_scoped`
Then: `cargo clippy --all --tests`
Expected: PASS; zero warnings.

- [ ] **Step 5: Commit**

```bash
git add crates/ironclaw_webui_v2/
git commit -m "feat(reborn): GET /api/webchat/v2/traces/account handler + contract test"
```

---

### Task 4: Frontend — fetch + render submitted traces

**Files:**
- Modify: `crates/ironclaw_webui_v2/static/js/pages/settings/lib/settings-api.js`
  (add `fetchAccountTraces`, mirror `fetchTraceCredits` at 120-133)
- Create: `crates/ironclaw_webui_v2/static/js/pages/settings/hooks/useAccountTraces.js`
  (mirror `useTraceCredits.js`)
- Modify: the Trace Commons settings tab component to render the list (find via
  `grep -rln "useTraceCredits" crates/ironclaw_webui_v2/static/js`)
- No `assets.rs` edit needed for modified files; **new** JS files are picked up by
  `build.rs` automatically (assets are generated from the `static/` tree at
  compile time — see `assets.rs` header).

**Interfaces:**
- Consumes: `GET /api/webchat/v2/traces/account` (Task 3).
- Produces: `useAccountTraces()` hook + a rendered list under the traces tab.

- [ ] **Step 1: Add the API function**

In `settings-api.js`:

```javascript
// Submitted Trace Commons traces for the authenticated caller (read-only,
// server-scoped). Mirrors fetchTraceCredits.
export function fetchAccountTraces() {
  return apiFetch("/api/webchat/v2/traces/account");
}
```

- [ ] **Step 2: Add the hook**

Create `useAccountTraces.js`:

```javascript
import { useQuery } from "@tanstack/react-query";
import { fetchAccountTraces } from "../lib/settings-api.js";

export function useAccountTraces() {
  const query = useQuery({
    queryKey: ["account-traces"],
    queryFn: fetchAccountTraces,
    refetchInterval: 300_000,
    refetchOnWindowFocus: true,
    staleTime: 60_000,
  });
  return { traces: query.data?.traces || [], enrolled: !!query.data?.enrolled, query };
}
```

- [ ] **Step 3: Render in the traces settings tab**

In the Trace Commons settings tab component, import `useAccountTraces` and, when
`enrolled`, render a list of `traces` (submission id, status, pending/final
credit, received_at). Mirror the existing credit/holds rendering markup style in
that file. Show nothing extra when `!enrolled`.

- [ ] **Step 4: Verify build embeds the assets**

Run: `cargo build -p ironclaw_webui_v2`
Expected: compiles (new JS files embedded by `build.rs`). Optionally run the app
(`/run` skill) and open Settings → Traces to confirm the list renders.

- [ ] **Step 5: Commit**

```bash
git add crates/ironclaw_webui_v2/static/js/
git commit -m "feat(reborn-ui): render submitted Trace Commons traces in settings"
```

---

## Self-Review

- **Spec coverage:** Spec §"Trace inspection" (list/detail/scrubbed content) →
  Task 1 (per-user `fetch_account_traces_via_sink`), Task 2 (facade), Task 3
  (v2 endpoint), Task 4 (UI). This slice ships the **list** surface end to end;
  detail/content (`/{id}` and `/{id}/content`) follow the identical pattern and
  are called out as a fast-follow (see note). "Scope from caller only" → Task 3
  contract test + handler signature. "Read-only projection, no credential leak"
  → `AccountTraceItem`/`RebornAccountTrace` carry only sanitized fields.
- **Scope note (no silent cap):** This plan ships the trace *list*. Detail
  (`GET /v1/account/traces/{id}`) and scrubbed content
  (`GET /v1/account/traces/{id}/content`) reuse Tasks 1–4 verbatim with the id in
  the path and a content-type passthrough; add them as Task 5/6 in a follow-up
  rather than expanding this slice. Flagged here so coverage is explicit.
- **Placeholder scan:** Component/file locations are given by grep recipe against
  the exact existing symbols (`useTraceCredits`, `fetchTraceCredits`,
  `trace_credits`). No "add validation"/"TODO" steps.
- **Type consistency:** `AccountTraceItem` (Task 1) → mapped to `RebornAccountTrace`
  (Task 2) → serialized in `RebornAccountTracesResponse` (Tasks 2/3) → consumed by
  `useAccountTraces` (Task 4) reading `data.traces` / `data.enrolled`, matching
  the response field names. `ContributionHttpMethod::Get` added in Task 1 and
  mapped in the host sink in the same task.
