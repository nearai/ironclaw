# Trace Commons Slice 0 — Server per-user subject Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

> **Target repo:** `trace-commons-server`. Working copy:
> `/Users/zakimanian/code/trace-commons-server/.worktrees/contributor-account-slice1-impl/`
> (branch `contributor-account-slice1`). This is a SERVER plan; the IronClaw
> client slices (1–4) are planned separately and depend on this one being merged
> or contract-frozen.

**Goal:** Let one enrolled device key mint per-user contributor claims by
accepting an optional, opaque `subject` in the upload-claim request, so a single
IronClaw instance fans out to many per-user accounts within its own tenant.

**Architecture:** The account endpoints (`/v1/account/*`) and the submission
endpoint (`/v1/traces`) both derive their principal from the bearer JWT's
`principal_ref` claim (`authenticate` → `TenantAuth.principal_ref`, set from
`issue_claim_for_authorized_actor`'s `principal_ref: actor.actor`). Therefore the
only required change is to make the device-key claim's `actor` per-user when a
`subject` is supplied. Submission attribution, login-link account resolution, and
trace-readback ownership all become per-user automatically — no separate
login-link or submission change. The change is additive: absent `subject`
reproduces today's behavior byte-for-byte.

**Tech Stack:** Rust, axum, jsonwebtoken (EdDSA), tokio, serde. Spec:
`docs/superpowers/specs/2026-06-25-trace-commons-instance-enrollment-profiles-inspection-design.md`
(in the ironclaw repo).

## Global Constraints

- All file paths below are relative to the working copy
  `/Users/zakimanian/code/trace-commons-server/.worktrees/contributor-account-slice1-impl/`.
- Zero clippy warnings: the repo gates with `RUSTFLAGS="-D warnings"`.
- Backward compatibility is mandatory: a request with no `subject` MUST produce
  an identical claim (`sub`/`principal_ref` == raw `device_key_id`) to today.
- `subject` is already an opaque pseudonymous token minted by the client
  (`tenant_sha256:…`-style); the server treats it as opaque and MUST NOT log it
  raw beyond existing claim handling, and MUST NOT trust any client-supplied
  principal prefix — the server derives the namespaced principal itself.
- Test command: `cargo test -p trace-commons-server --lib trace_upload_claim_issuer`
  (compile gate: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --no-run`).

---

### Task 1: Accept and validate an optional `subject` in the claim request

**Files:**
- Modify: `crates/trace-commons-server/src/trace_upload_claim_issuer.rs:536-552` (struct)
- Modify: `crates/trace-commons-server/src/trace_upload_claim_issuer.rs` (add `normalize_subject` helper near `principal_storage_ref` at ~2168)
- Test: `crates/trace-commons-server/src/trace_upload_claim_issuer.rs` (test module starts at line 2260)

**Interfaces:**
- Produces: `TraceUploadClaimRequest.subject: Option<String>` (`#[serde(default)]`).
- Produces: `fn normalize_subject(raw: &str) -> Result<String, IssuerError>` —
  trims, rejects empty / over-128-byte / non-`[A-Za-z0-9:_-]` values, returns the
  normalized subject. Consumed by Task 2.

- [ ] **Step 1: Write the failing test**

Add to the test module (after line 2260):

```rust
#[test]
fn normalize_subject_accepts_pseudonymous_token() {
    let s = normalize_subject("  tenant_sha256:ab12CD_-  ").expect("valid");
    assert_eq!(s, "tenant_sha256:ab12CD_-");
}

#[test]
fn normalize_subject_rejects_empty_and_oversized_and_bad_chars() {
    assert!(normalize_subject("   ").is_err());
    assert!(normalize_subject(&"a".repeat(129)).is_err());
    assert!(normalize_subject("has space").is_err());
    assert!(normalize_subject("bad/slash").is_err());
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p trace-commons-server --lib trace_upload_claim_issuer::tests::normalize_subject`
Expected: FAIL — `cannot find function normalize_subject`.

- [ ] **Step 3: Add the `subject` field and the helper**

Add `subject` to the struct (insert before `requested_at`):

```rust
    #[serde(default)]
    allowed_uses: Vec<TraceAllowedUse>,
    #[serde(default)]
    subject: Option<String>,
    requested_at: DateTime<Utc>,
}
```

Add the helper next to `principal_storage_ref` (~line 2168):

```rust
/// Maximum accepted byte length for a client-supplied subject.
const MAX_SUBJECT_LEN: usize = 128;

/// Validate and normalize an opaque per-user subject. The subject is a
/// pseudonymous token minted by the client; we only enforce a conservative
/// shape so it is safe to embed in a derived principal string. We never trust a
/// client-supplied principal prefix — the namespaced principal is built in
/// `issue_claim_for_device_key`.
fn normalize_subject(raw: &str) -> Result<String, IssuerError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.len() > MAX_SUBJECT_LEN {
        return Err(IssuerError::bad_request("invalid subject"));
    }
    if !trimmed
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b':' | b'_' | b'-'))
    {
        return Err(IssuerError::bad_request("invalid subject"));
    }
    Ok(trimmed.to_string())
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p trace-commons-server --lib trace_upload_claim_issuer::tests::normalize_subject`
Expected: PASS (both tests).

- [ ] **Step 5: Commit**

```bash
git add crates/trace-commons-server/src/trace_upload_claim_issuer.rs
git commit -m "feat(claim): accept optional opaque subject in upload-claim request"
```

---

### Task 2: Derive a per-user principal in device-key claim issuance

**Files:**
- Modify: `crates/trace-commons-server/src/trace_upload_claim_issuer.rs:1405-1443`
  (`issue_claim_for_device_key`)
- Test: `crates/trace-commons-server/src/trace_upload_claim_issuer.rs` (test module)

**Interfaces:**
- Consumes: `TraceUploadClaimRequest.subject` + `normalize_subject` (Task 1).
- Produces: when `subject` is present, the issued claim's `sub` and
  `principal_ref` equal `instance:{tenant_id}:{device_key_id}:user:{subject}`;
  when absent, they equal the raw `device_key_id` (unchanged). The
  tenant-access-grant principal (`grant_principal_ref`) stays device-scoped
  (`principal_sha256:` of `device:{tenant}:{device_key_id}`) in both cases —
  grants are governed at the device level.

- [ ] **Step 1: Write the failing tests**

Add to the test module. (Reuse the existing device-key claim test helpers — find
the existing device-key issue test, e.g. search the module for
`issue_claim_for_device_key` / a `device_claim_request(...)` helper, and mirror
its setup. The two assertions that matter:)

```rust
#[tokio::test]
async fn device_claim_without_subject_uses_raw_device_key_id() {
    // ARRANGE: mirror the existing device-key happy-path test setup, with a
    // request whose `subject` is None.
    let (status, body) = post_device_claim(/* existing helpers */).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let claims = decode_issued_claims(&body);
    assert_eq!(claims["principal_ref"], DEVICE_KEY_ID); // unchanged behavior
    assert_eq!(claims["sub"], DEVICE_KEY_ID);
}

#[tokio::test]
async fn device_claim_with_subject_yields_distinct_per_user_principal() {
    let (s1, b1) = post_device_claim_with_subject("user-alice-hash").await;
    let (s2, b2) = post_device_claim_with_subject("user-bob-hash").await;
    assert_eq!(s1, StatusCode::OK);
    assert_eq!(s2, StatusCode::OK);
    let p1 = decode_issued_claims(&b1)["principal_ref"].as_str().unwrap().to_string();
    let p2 = decode_issued_claims(&b2)["principal_ref"].as_str().unwrap().to_string();
    assert_eq!(
        p1,
        format!("instance:{TENANT_ID}:{DEVICE_KEY_ID}:user:user-alice-hash")
    );
    assert_ne!(p1, p2, "distinct subjects must yield distinct principals");
}
```

> Note for the implementer: the existing module already has device-key issue
> tests with signing helpers (`verify_device_claim_signature`, a test device
> keypair, and a `post_claim`/`post_device_claim` style helper). Use those exact
> helpers and constants rather than inventing new ones; `decode_issued_claims` is
> the same decode block used in
> `eddsa_only_issue_success_returns_bounded_upload_claim` (lines 2475-2512) —
> extract it into a small local helper if it isn't one already.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p trace-commons-server --lib trace_upload_claim_issuer::tests::device_claim`
Expected: FAIL — `device_claim_with_subject_*` fails because `principal_ref`
still equals the raw device key id.

- [ ] **Step 3: Thread the subject into the derived actor**

In `issue_claim_for_device_key`, replace the `actor` / `grant_principal_ref`
block (currently lines ~1437-1438):

```rust
    let actor = auth.device_key_id;
    let grant_principal_ref = principal_storage_ref(&format!("device:{tenant_id}:{actor}"));
```

with:

```rust
    let device_key_id = auth.device_key_id;
    // Grants are governed at the device level regardless of per-user subject.
    let grant_principal_ref =
        principal_storage_ref(&format!("device:{tenant_id}:{device_key_id}"));
    // When the instance asserts a per-user subject, the issued principal is
    // namespaced under the device so subjects cannot collide across
    // instances/tenants and the blast radius stays inside this tenant. Absent a
    // subject, behavior is unchanged (principal == raw device_key_id).
    let actor = match request.subject.as_deref() {
        Some(raw) => {
            let subject = normalize_subject(raw)?;
            format!("instance:{tenant_id}:{device_key_id}:user:{subject}")
        }
        None => device_key_id,
    };
```

(The `AuthorizedUploadClaimActor { actor, ... }` construction below is unchanged;
it already sets `sub`/`principal_ref` from `actor`.)

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p trace-commons-server --lib trace_upload_claim_issuer::tests::device_claim`
Expected: PASS (both new tests + the existing device-key tests).

- [ ] **Step 5: Full compile + lint gate**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --no-run`
Expected: builds with zero warnings.

- [ ] **Step 6: Commit**

```bash
git add crates/trace-commons-server/src/trace_upload_claim_issuer.rs
git commit -m "feat(claim): derive per-user principal from optional subject for device keys"
```

---

### Task 3: Verify per-user claim flows through account resolution (integration)

**Files:**
- Test: add an integration test alongside the existing account/login-link tests
  (find them via `grep -rn "mint_login_link\|create_or_reuse_account" crates/trace-commons-server` —
  likely in the bin's test module or a `tests/` integration file; co-locate with
  the existing login-link test setup so the DB/test harness is reused).

**Interfaces:**
- Consumes: a per-user upload claim from Task 2.
- Produces: proof that a per-user bearer (1) authenticates on
  `/v1/account/login-links`, (2) resolves a per-user account, and (3) two
  distinct subjects under one device key resolve to two distinct accounts, while
  a no-subject bearer resolves to the device-level account (today's behavior).

> If the account endpoints require a different audience/role than the upload
> claim issues (`aud = trace-commons-upload`, `role = contributor`), this test
> will surface it. If they reject the contributor claim, STOP and escalate: the
> design assumed the same bearer works on `/v1/account/*`; the fix is a scoped
> follow-up (mint an account-audience claim or widen the account endpoints'
> accepted audience), not a silent workaround.

- [ ] **Step 1: Write the failing test**

Mirror the existing login-link test setup (real or testcontainer Postgres per the
repo's account-test harness), then:

```rust
#[tokio::test]
async fn per_user_subjects_resolve_to_distinct_accounts_under_one_device_key() {
    // ARRANGE: one enrolled device key in tenant T (reuse the existing
    // account-test harness that seeds a device key + tenant).
    // Mint two per-user claims via the issuer (subjects "alice", "bob") and one
    // no-subject claim, then call POST /v1/account/login-links with each bearer.
    let alice_account = mint_login_link_account_for_subject(Some("alice")).await;
    let bob_account = mint_login_link_account_for_subject(Some("bob")).await;
    let device_account = mint_login_link_account_for_subject(None).await;

    assert_ne!(alice_account, bob_account, "distinct subjects → distinct accounts");
    assert_ne!(alice_account, device_account);
    // Idempotent reuse: same subject → same account.
    assert_eq!(alice_account, mint_login_link_account_for_subject(Some("alice")).await);
}
```

- [ ] **Step 2: Run test to verify it fails (or surfaces the audience gap)**

Run: `cargo test -p trace-commons-server per_user_subjects_resolve_to_distinct_accounts`
Expected: FAIL initially because the helper `mint_login_link_account_for_subject`
does not exist yet; once written, it either PASSES (design confirmed) or surfaces
the audience/role gap noted above.

- [ ] **Step 3: Implement the test helper only (no production change expected)**

Implement `mint_login_link_account_for_subject(subject: Option<&str>) -> Uuid`:
mint a device claim with the given subject via the issuer test helper, POST it to
the login-link handler, then read back the `account_id` from the
`MintLoginLinkResponse` (`account_id: String` → parse `Uuid`). No production code
change is expected — Tasks 1–2 already carry the subject through the principal.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p trace-commons-server per_user_subjects_resolve_to_distinct_accounts`
Expected: PASS. If it fails on audience/role, escalate per the note above before
making any change.

- [ ] **Step 5: Commit**

```bash
git add crates/trace-commons-server
git commit -m "test(account): per-user subjects under one device key resolve to distinct accounts"
```

---

## Self-Review

- **Spec coverage:** Spec §"Server changes" item 1 (optional `subject` field) →
  Task 1. Item 2 (per-user principal in device-key issuance) → Task 2. Item 3
  (login-link account resolution accepts the per-user principal) → Task 3
  *verifies* this is automatic via the shared bearer principal, rather than a
  separate code change, because `authenticate_ctx` derives `principal_ref` from
  the same JWT. Backward-compat requirement → Task 2 Step 1 first assertion +
  Task 3 no-subject branch. Subject-spoofing containment → Task 2 namespaced
  derivation (server builds the principal; client prefix never trusted).
- **Placeholder scan:** The only deferred specifics are test-helper names that
  must match the repo's existing device-key/account test harness; each such step
  names the exact existing symbol to mirror (`eddsa_only_issue_success_…` decode
  block, `mint_login_link_handler`, `MintLoginLinkResponse`) rather than leaving
  it open. No "TODO/handle edge cases" steps.
- **Type consistency:** `subject: Option<String>` (Task 1) is read as
  `request.subject.as_deref()` (Task 2); `normalize_subject(&str) -> Result<String, IssuerError>`
  used consistently; `MintLoginLinkResponse.account_id: String` parsed to `Uuid`
  in Task 3 matches the struct at server line 11837-11840.
