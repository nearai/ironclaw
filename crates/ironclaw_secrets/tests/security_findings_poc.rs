//! Proof-of-concept tests for the security review of `ironclaw_secrets`.
//!
//! Each `#[test]` here is a failing test that demonstrates a real finding from
//! the 2026-05 review. They live together so the fixes can land alongside
//! green-on-red evidence. Once a finding is fixed, the matching PoC asserts
//! the new invariant rather than removing the test.
//!
//! Findings covered in this file:
//!
//! - **H3** — `reborn_secret_records` rows have no AAD/post-decrypt binding
//!   between ciphertext and `(user_id, name)`. A DB-write adversary can swap
//!   `(encrypted_value, key_salt)` between two rows and `get_decrypted` will
//!   silently return the wrong plaintext. Two follow-up tests extend the same
//!   row-swap guard to `reborn_credential_accounts` and
//!   `reborn_credential_sessions`, which were called out in PR #3592 review
//!   as needing the same AAD-binding regression coverage.
//! - **M1** — `CredentialSessionId` `Display` emits the raw UUID despite
//!   `Debug` being redacted, so `format!("{id}")` in a log line leaks the
//!   bearer-like value.
//! - **M2** — `SecretsCrypto::new` accepts low-entropy 32-byte master keys
//!   (e.g. 32 zero bytes), making captured ciphertext brute-forceable when
//!   an operator copy-pastes a weak key.
//! - **AAD format invariant** — confirms that ciphertext produced without
//!   AAD binding (the pre-fix legacy format) cannot be silently accepted by
//!   the new decrypt path. This locks in the "no on-disk compatibility with
//!   pre-AAD rows" design decision: bootstrap must fail closed instead of
//!   returning unauthenticated plaintext.
//! - **Cross-domain replay** — confirms that ciphertext authenticated under
//!   one AAD domain (e.g. `reborn/v1/secret_record`) cannot be decrypted
//!   under another (e.g. `reborn/v1/credential_account`), even when the
//!   identity bytes after the domain prefix match. The domain tag is the
//!   load-bearing protection against cross-shape replay.

use ironclaw_secrets::{
    CredentialSessionId, SecretError, SecretMaterial, SecretsCrypto, secret_record_aad,
};

#[cfg(feature = "libsql")]
use std::sync::Arc;

#[cfg(feature = "libsql")]
use chrono::Utc;
#[cfg(feature = "libsql")]
use ironclaw_host_api::{
    AgentId, CapabilityId, ExtensionId, InvocationId, MissionId, NetworkMethod, ProjectId,
    ResourceScope, SecretHandle, TenantId, ThreadId, UserId,
};
#[cfg(feature = "libsql")]
use ironclaw_secrets::{
    CreateSecretParams, CredentialAccount, CredentialAccountId, CredentialAccountStatus,
    CredentialAccountStore, CredentialPathPolicy, CredentialSessionRequest, CredentialSessionStore,
    CredentialTargetPolicy, InMemoryCredentialBroker, LibSqlCredentialStore, LibSqlSecretsStore,
    RedactedJson, SecretsStore, credential_account_aad,
};
#[cfg(feature = "libsql")]
use serde_json::json;

// ---------------------------------------------------------------------------
// H3 — row-swap returns the wrong plaintext
// ---------------------------------------------------------------------------

/// **Finding H3.** AES-GCM ciphertext in `reborn_secret_records` is not bound
/// to `(user_id, name)` via additional-authenticated-data or a post-decrypt
/// scope check. An attacker with DB write access (SQL injection elsewhere,
/// compromised replication, admin-with-DB-only-access, etc.) can swap the
/// `(encrypted_value, key_salt)` columns between two rows. The crypto layer
/// has no signal that anything is wrong, so `get_decrypted("low_priv_key")`
/// returns the plaintext that was originally stored under
/// `high_priv_admin_token`.
///
/// The same-user case shown here proves the bug with the minimum setup; the
/// cross-tenant variant (different `user_id` values) is structurally identical
/// because `scoped_legacy_user_id` is itself just another plaintext column.
///
/// Expected behavior after the fix (AES-GCM AAD = scope-key || handle, or an
/// embedded handle inside the encrypted payload that is verified post-decrypt):
/// the lookup either fails outright or returns the original plaintext bound
/// to the row, never the other row's value.
#[cfg(feature = "libsql")]
#[tokio::test]
async fn h3_libsql_secret_row_swap_must_not_return_other_rows_plaintext() {
    let dir = tempfile::tempdir().unwrap().keep();
    let db_path = dir.join("secrets.db");
    let crypto = h3_test_crypto();
    let database = Arc::new(libsql::Builder::new_local(&db_path).build().await.unwrap());
    let store = LibSqlSecretsStore::new(Arc::clone(&database), Arc::clone(&crypto));
    store.run_migrations().await.unwrap();

    let user_id = "tenant-A.user-A";
    let low_priv_plaintext = "PLAIN_A_low_privilege_read_only_token";
    let high_priv_plaintext = "PLAIN_B_admin_root_token";

    store
        .create(
            user_id,
            CreateSecretParams::new("low_priv_key", low_priv_plaintext),
        )
        .await
        .unwrap();
    store
        .create(
            user_id,
            CreateSecretParams::new("high_priv_admin_token", high_priv_plaintext),
        )
        .await
        .unwrap();

    // Simulate an attacker with DB write swapping the ciphertext columns
    // between the two rows. Note that `low_priv_key` keeps its row identity
    // (same user_id, same name, same id) — only the encrypted_value and
    // key_salt are taken from the other row.
    let conn = database.connect().unwrap();
    let mut rows = conn
        .query(
            "SELECT encrypted_value, key_salt FROM reborn_secret_records \
             WHERE user_id = ?1 AND name = ?2",
            libsql::params![user_id, "high_priv_admin_token"],
        )
        .await
        .unwrap();
    let row = rows.next().await.unwrap().expect("high-priv row exists");
    let admin_encrypted: Vec<u8> = row.get(0).unwrap();
    let admin_salt: Vec<u8> = row.get(1).unwrap();

    conn.execute(
        "UPDATE reborn_secret_records SET encrypted_value = ?1, key_salt = ?2 \
         WHERE user_id = ?3 AND name = ?4",
        libsql::params![admin_encrypted, admin_salt, user_id, "low_priv_key"],
    )
    .await
    .unwrap();

    // The fix must reject this lookup (DecryptionFailed / NotFound) or return
    // the original plaintext. Returning the *other* row's plaintext is the
    // bug.
    let lookup = store.get_decrypted(user_id, "low_priv_key").await;
    match lookup {
        Ok(material) => {
            assert_ne!(
                material.expose(),
                high_priv_plaintext,
                "H3: get_decrypted(low_priv_key) returned the high-priv plaintext after a \
                 ciphertext-column swap. AES-GCM has no AAD binding the ciphertext to \
                 (user_id, name), so cross-row swaps decrypt cleanly."
            );
            // Returning the original low-priv plaintext would be acceptable but
            // is not what happens today — the swap replaced the bytes.
            assert_eq!(
                material.expose(),
                low_priv_plaintext,
                "H3: get_decrypted returned an unexpected plaintext after row swap"
            );
        }
        Err(error) => {
            // Acceptable: the fix may reject the row at decrypt time once the
            // ciphertext is bound to (user_id, name).
            assert!(
                matches!(error, SecretError::DecryptionFailed(_)),
                "H3: post-fix, mismatched ciphertext must fail with DecryptionFailed, got {error:?}"
            );
        }
    }
}

#[cfg(feature = "libsql")]
fn h3_test_crypto() -> Arc<SecretsCrypto> {
    Arc::new(
        SecretsCrypto::new(SecretMaterial::from(
            "0123456789abcdef0123456789abcdef".to_string(),
        ))
        .unwrap(),
    )
}

// ---------------------------------------------------------------------------
// M1 — CredentialSessionId Display redaction
// ---------------------------------------------------------------------------

/// **Finding M1.** `CredentialSessionId` is documented as "bearer-like" and
/// "intentionally not `Serialize`"; `Debug` emits `[REDACTED]`. But `Display`
/// (used by `format!("{id}")`, `tracing::info!(%id, ...)`, and any
/// `error.to_string()` interpolation) emits the raw UUID. Any developer who
/// writes the idiomatic `session={session_id}` defeats the redaction.
///
/// Expected behavior after the fix: `Display` must not produce a parseable
/// UUID. A typical fix is to write a stable redacted form (`"[REDACTED]"`)
/// and provide a narrow `fn expose(&self) -> Uuid` for the small number of
/// call sites that genuinely need the value.
#[test]
fn m1_credential_session_id_display_must_not_emit_raw_uuid() {
    let id = CredentialSessionId::new();
    let displayed = format!("{id}");

    assert!(
        uuid::Uuid::parse_str(&displayed).is_err(),
        "M1: Display on CredentialSessionId must not emit a parseable UUID, \
         got {displayed:?}. Debug is redacted but Display leaks the bearer-like \
         value through every `{{}}`-style log/format call."
    );
}

// ---------------------------------------------------------------------------
// M2 — Master-key entropy is not validated
// ---------------------------------------------------------------------------

/// **Finding M2.** `SecretsCrypto::new` only checks that the master key is at
/// least 32 bytes long. An operator copy-pasting a weak key — 32 zero bytes,
/// `a` repeated 32 times, `01` repeated 16 times — passes the length check
/// and is then used as IKM for HKDF. Low-entropy IKM means captured
/// ciphertext is offline-brute-forceable, defeating the entire encrypted-at-
/// rest invariant.
///
/// Expected behavior after the fix: low-entropy master keys are rejected with
/// `SecretError::InvalidMasterKey`. A reasonable bar is "must be at least 32
/// bytes AND have at least N distinct bytes" or "must parse as 64 hex
/// characters / 32 base64-decoded bytes from a sufficiently random source".
/// The exact entropy heuristic is a design choice; the test here only asserts
/// that the three most-obvious weak keys are rejected.
#[test]
fn m2_secrets_crypto_must_reject_low_entropy_master_keys() {
    let weak_inputs = [
        // 32 bytes of '0' — passes length check today.
        "0".repeat(32),
        // 32 bytes of 'a' — same shape.
        "a".repeat(32),
        // 32 bytes of "01" repeated — only two distinct bytes.
        "01".repeat(16),
    ];

    for weak in weak_inputs {
        let result = SecretsCrypto::new(SecretMaterial::from(weak.clone()));
        assert!(
            matches!(result, Err(SecretError::InvalidMasterKey)),
            "M2: SecretsCrypto::new must reject low-entropy master key {weak:?}, \
             but it accepted it. A length-only check lets operators paste keys \
             with trivial entropy that are then used as HKDF input."
        );
    }
}

// ---------------------------------------------------------------------------
// AAD format invariant — pre-AAD ciphertext is rejected at decrypt
// ---------------------------------------------------------------------------

/// PR #3592 review (serrrfirat) flagged that switching to AAD-bound decrypt
/// makes any row written by the previous code path unreadable. The PR's
/// answer is "`reborn-integration` has no live data so we don't need a
/// compatibility/migration path." This test locks that decision in: a
/// ciphertext produced under the legacy "empty AAD" shape cannot be silently
/// accepted by the new AAD-bound decrypt path. If it ever could, an attacker
/// with DB write access could downgrade rows to the pre-fix format and
/// recover plaintext that is no longer bound to `(user_id, name)`.
#[test]
fn aad_binding_rejects_pre_aad_secret_record_ciphertext() {
    let crypto = SecretsCrypto::new(SecretMaterial::from(
        "0123456789abcdef0123456789abcdef".to_string(),
    ))
    .expect("valid 32-byte hex test key");

    // Encrypt with empty AAD — this is what the pre-fix code path produced.
    let plaintext = b"PLAINTEXT_should_not_round_trip";
    let (encrypted_value, key_salt) = crypto.encrypt(plaintext, &[]).unwrap();

    // The new decrypt path always supplies a domain-bound AAD. If the AES-GCM
    // tag check were skipped or if AAD were ignored, this would return the
    // plaintext.
    let aad = secret_record_aad("tenant-a.user-a", "low_priv_key");
    let result = crypto.decrypt(&encrypted_value, &key_salt, &aad);

    assert!(
        matches!(result, Err(SecretError::DecryptionFailed(_))),
        "Pre-AAD ciphertext must be rejected at decrypt — never silently \
         returned to the caller, otherwise an attacker can downgrade rows \
         to the legacy format and bypass the row-binding guard. got={result:?}"
    );
}

// ---------------------------------------------------------------------------
// Cross-domain replay — secret_record ↔ credential_account
// ---------------------------------------------------------------------------

/// PR #3592 review follow-up. Domain separation is the load-bearing
/// protection against cross-shape replay: a `secret_record`-shape ciphertext
/// must not be acceptable under `credential_account` AAD even when the
/// identity bytes after the domain prefix happen to align. This test
/// constructs a ciphertext authenticated under `secret_record_aad(user, name)`
/// and asserts that decrypting it with a `credential_account_aad` that uses
/// the same `name` as the account id fails with `DecryptionFailed`. The
/// reverse direction is symmetric and not retested.
#[cfg(feature = "libsql")]
#[test]
fn cross_domain_replay_secret_record_to_credential_account_fails() {
    let crypto = SecretsCrypto::new(SecretMaterial::from(
        "0123456789abcdef0123456789abcdef".to_string(),
    ))
    .expect("valid 32-byte hex test key");

    // Encrypt under the secret-record domain with identity bytes that match
    // the credential-account row we'll try to "promote" the ciphertext into.
    let plaintext = b"PLAINTEXT_cross_domain_replay_canary";
    let secret_aad = secret_record_aad("tenant-a", "openai_prod");
    let (encrypted_value, key_salt) = crypto.encrypt(plaintext, &secret_aad).unwrap();

    // Build the credential-account AAD whose post-domain identity bytes are
    // intentionally close to the secret-record case. The only thing that
    // should reject decryption is the differing domain tag in `build_aad`.
    let scope = sample_scope("tenant-a", "user-a");
    let account_id = CredentialAccountId::new("openai_prod").unwrap();
    let cred_aad = credential_account_aad(&scope, &account_id);

    let result = crypto.decrypt(&encrypted_value, &key_salt, &cred_aad);
    assert!(
        matches!(result, Err(SecretError::DecryptionFailed(_))),
        "Cross-domain replay must fail closed: secret_record ciphertext was \
         accepted under credential_account AAD. The domain tag in build_aad \
         is the load-bearing protection against cross-shape replay. \
         got={result:?}"
    );

    // Sanity check: decrypt under the original AAD still works, so the
    // failure above is genuinely from the domain mismatch and not from
    // unrelated tampering.
    let ok = crypto
        .decrypt(&encrypted_value, &key_salt, &secret_aad)
        .expect("original-domain decrypt must round-trip");
    assert_eq!(ok.expose().as_bytes(), plaintext);
}

// ---------------------------------------------------------------------------
// H3 follow-up — credential account row-swap
// ---------------------------------------------------------------------------

/// PR #3592 review (serrrfirat) called out that `reborn_credential_accounts`
/// payloads also need a regression guard for the row-swap shape that
/// motivated the H3 fix on `reborn_secret_records`. This extends the same
/// test to the credential account table: two rows in the same scope, swap
/// `(encrypted_payload, payload_key_salt)` between them, and assert that the
/// store refuses to return the other account's payload.
///
/// The fix wires AES-GCM AAD to `(scope, account_id)`, so the swap fails the
/// tag check at decrypt time.
#[cfg(feature = "libsql")]
#[tokio::test]
async fn h3_libsql_credential_account_row_swap_must_not_return_other_accounts_payload() {
    let dir = tempfile::tempdir().unwrap().keep();
    let db_path = dir.join("credentials.db");
    let db = Arc::new(libsql::Builder::new_local(&db_path).build().await.unwrap());
    let store = LibSqlCredentialStore::new(Arc::clone(&db), test_crypto());
    store.run_migrations().await.unwrap();

    let scope = sample_scope("tenant-a", "user-a");
    let low_priv_id = CredentialAccountId::new("low_priv").unwrap();
    let high_priv_id = CredentialAccountId::new("high_priv_admin").unwrap();

    let mut low_priv = sample_account(scope.clone(), low_priv_id.clone());
    low_priv.redacted_metadata = RedactedJson::new(json!({ "marker": "LOW_PRIV_PAYLOAD" }));
    let mut high_priv = sample_account(scope.clone(), high_priv_id.clone());
    high_priv.redacted_metadata = RedactedJson::new(json!({ "marker": "HIGH_PRIV_PAYLOAD" }));

    store.put_account(low_priv).await.unwrap();
    store.put_account(high_priv.clone()).await.unwrap();

    // Read the high-priv ciphertext, then overwrite the low-priv row's
    // ciphertext columns. Row identity (scope + account_id) stays put — only
    // the encrypted blob and salt move.
    let conn = db.connect().unwrap();
    let mut rows = conn
        .query(
            "SELECT encrypted_payload, payload_key_salt FROM reborn_credential_accounts \
             WHERE tenant_id = ?1 AND user_id = ?2 AND agent_id = ?3 AND project_id = ?4 \
               AND account_id = ?5",
            libsql::params![
                scope.tenant_id.as_str(),
                scope.user_id.as_str(),
                scope.agent_id.as_ref().unwrap().as_str(),
                scope.project_id.as_ref().unwrap().as_str(),
                high_priv_id.as_str(),
            ],
        )
        .await
        .unwrap();
    let row = rows.next().await.unwrap().expect("high-priv row");
    let high_priv_blob: Vec<u8> = row.get(0).unwrap();
    let high_priv_salt: Vec<u8> = row.get(1).unwrap();

    conn.execute(
        "UPDATE reborn_credential_accounts SET encrypted_payload = ?1, payload_key_salt = ?2 \
         WHERE tenant_id = ?3 AND user_id = ?4 AND agent_id = ?5 AND project_id = ?6 \
           AND account_id = ?7",
        libsql::params![
            high_priv_blob,
            high_priv_salt,
            scope.tenant_id.as_str(),
            scope.user_id.as_str(),
            scope.agent_id.as_ref().unwrap().as_str(),
            scope.project_id.as_ref().unwrap().as_str(),
            low_priv_id.as_str(),
        ],
    )
    .await
    .unwrap();

    let lookup = store.get_account(&scope, &low_priv_id).await;
    match lookup {
        Ok(Some(account)) => panic!(
            "credential-account row swap leaked another account's payload: {:?}",
            account.redacted_metadata
        ),
        Ok(None) | Err(_) => {
            // Either outcome is acceptable: the AAD-bound decrypt must fail
            // closed (DecryptionFailed) or upstream validation must drop the
            // mismatched row before returning it.
        }
    }
}

// ---------------------------------------------------------------------------
// H3 follow-up — credential session row-swap
// ---------------------------------------------------------------------------

/// Same shape as the credential-account row-swap test, applied to
/// `reborn_credential_sessions`. The AAD binds ciphertext to
/// `(scope, session_id)`, so a write-side adversary cannot lift the
/// ciphertext for a high-privilege session into the row for a
/// low-privilege one and have `get_session` decrypt cleanly.
#[cfg(feature = "libsql")]
#[tokio::test]
async fn h3_libsql_credential_session_row_swap_must_not_return_other_sessions_payload() {
    let dir = tempfile::tempdir().unwrap().keep();
    let db_path = dir.join("credentials.db");
    let db = Arc::new(libsql::Builder::new_local(&db_path).build().await.unwrap());
    let store = LibSqlCredentialStore::new(Arc::clone(&db), test_crypto());
    store.run_migrations().await.unwrap();

    let scope = sample_scope("tenant-a", "user-a");
    let account_id = CredentialAccountId::new("openai_prod").unwrap();
    store
        .put_account(sample_account(scope.clone(), account_id.clone()))
        .await
        .unwrap();

    let low_priv_session = broker_session(scope.clone(), account_id.clone(), Some(1), None);
    let high_priv_session = broker_session(scope.clone(), account_id.clone(), Some(1000), None);
    let low_priv_id = low_priv_session.correlation_id();
    let high_priv_id = high_priv_session.correlation_id();
    store.issue_session(low_priv_session).await.unwrap();
    store.issue_session(high_priv_session).await.unwrap();

    // `CredentialSessionId::Display` is redacted, so the primary-key lookup
    // string is not directly available to the test. Identify the two rows by
    // their distinct `max_uses` values and lift the ciphertext blobs from
    // the high-priv row into the low-priv row.
    let conn = db.connect().unwrap();
    let mut rows = conn
        .query(
            "SELECT session_id, max_uses, encrypted_payload, payload_key_salt \
             FROM reborn_credential_sessions",
            (),
        )
        .await
        .unwrap();
    let mut low_priv_pk: Option<String> = None;
    let mut high_priv_blob: Option<(Vec<u8>, Vec<u8>)> = None;
    while let Some(row) = rows.next().await.unwrap() {
        let sid: String = row.get(0).unwrap();
        let max_uses: Option<i64> = row.get(1).unwrap();
        let blob: Vec<u8> = row.get(2).unwrap();
        let salt: Vec<u8> = row.get(3).unwrap();
        match max_uses {
            Some(1) => low_priv_pk = Some(sid),
            Some(1000) => high_priv_blob = Some((blob, salt)),
            _ => {}
        }
    }
    let low_priv_pk = low_priv_pk.expect("low-priv row exists");
    let (high_priv_blob, high_priv_salt) =
        high_priv_blob.expect("high-priv row exists with captured blob");

    conn.execute(
        "UPDATE reborn_credential_sessions SET encrypted_payload = ?1, payload_key_salt = ?2 \
         WHERE session_id = ?3",
        libsql::params![high_priv_blob, high_priv_salt, low_priv_pk],
    )
    .await
    .unwrap();

    // The low-priv row now carries the high-priv ciphertext but its own
    // session_id. AAD-bound decrypt must reject it; if anything, returning
    // `Ok(None)` or an error is acceptable. Returning the swapped session
    // would be the bug.
    let lookup = store.get_session(&scope, low_priv_id).await;
    match lookup {
        Ok(Some(session)) => {
            assert_ne!(
                session.correlation_id(),
                high_priv_id,
                "credential-session row swap leaked another session's correlation id; \
                 AAD binding to (scope, session_id) is not in effect"
            );
            panic!(
                "credential-session row swap returned a non-empty session that should \
                 have failed the AAD tag check"
            );
        }
        Ok(None) | Err(_) => {
            // Acceptable: AAD-bound decrypt failed closed, or the validate-
            // row pass dropped the mismatched ciphertext.
        }
    }
}

#[cfg(feature = "libsql")]
fn test_crypto() -> Arc<SecretsCrypto> {
    Arc::new(
        SecretsCrypto::new(SecretMaterial::from(
            "0123456789abcdef0123456789abcdef".to_string(),
        ))
        .unwrap(),
    )
}

#[cfg(feature = "libsql")]
fn sample_scope(tenant: &str, user: &str) -> ResourceScope {
    ResourceScope {
        tenant_id: TenantId::new(tenant).unwrap(),
        user_id: UserId::new(user).unwrap(),
        agent_id: Some(AgentId::new("agent-a").unwrap()),
        project_id: Some(ProjectId::new("project-a").unwrap()),
        mission_id: Some(MissionId::new("mission-a").unwrap()),
        thread_id: Some(ThreadId::new("thread-a").unwrap()),
        invocation_id: InvocationId::new(),
    }
}

#[cfg(feature = "libsql")]
fn sample_account(scope: ResourceScope, id: CredentialAccountId) -> CredentialAccount {
    CredentialAccount {
        scope,
        id,
        provider_or_extension_id: ExtensionId::new("openai").unwrap(),
        label: "Production".to_string(),
        status: CredentialAccountStatus::Active,
        secret_handles: vec![SecretHandle::new("openai_key").unwrap()],
        allowed_targets: vec![CredentialTargetPolicy {
            scheme: "https".to_string(),
            host: "api.example.com".to_string(),
            port: Some(443),
            path: CredentialPathPolicy::Prefix("/v1/".to_string()),
            methods: vec![NetworkMethod::Get],
        }],
        redacted_metadata: RedactedJson::new(json!({ "last_four": "1234" })),
        updated_at: Utc::now(),
    }
}

#[cfg(feature = "libsql")]
fn broker_session(
    scope: ResourceScope,
    account_id: CredentialAccountId,
    max_uses: Option<u64>,
    expires_at: Option<chrono::DateTime<Utc>>,
) -> ironclaw_secrets::CredentialSession {
    let broker = InMemoryCredentialBroker::new();
    broker
        .put_account(sample_account(scope.clone(), account_id.clone()))
        .unwrap();
    broker
        .create_session(CredentialSessionRequest {
            invocation_id: scope.invocation_id,
            scope,
            capability_id: CapabilityId::new("openai.chat").unwrap(),
            extension_id: ExtensionId::new("openai").unwrap(),
            account_id,
            method: NetworkMethod::Get,
            url: "https://api.example.com/v1/models".to_string(),
            expires_at,
            max_uses,
        })
        .unwrap()
}

// ---------------------------------------------------------------------------
// F1 — consume_if_matches uses constant-time comparison
// ---------------------------------------------------------------------------
//
// We cannot reliably prove constant-time-ness from a timing measurement on a
// shared CI runner, but we *can* lock in that the production source uses
// `subtle::ConstantTimeEq::ct_eq` rather than `!=` for the
// decrypted-plaintext comparison in `consume_if_matches`. A future regression
// that "simplifies" the comparison back to
// `decrypted.expose() != expected_value` re-opens the timing oracle described
// in F1 of the 2026-05 audit; this test forces such a change to also update
// the allowlisted source files, making the security regression visible in
// review.
//
// See `crates/ironclaw_secrets/src/legacy_store.rs::consume_if_matches` and
// `crates/ironclaw_secrets/src/db.rs::consume_if_matches` (libSQL + Postgres).

#[test]
fn f1_consume_if_matches_uses_constant_time_compare() {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let sources = [
        ("legacy_store.rs", "src/legacy_store.rs"),
        ("db.rs", "src/db.rs"),
    ];
    for (label, relative) in sources {
        let path = std::path::Path::new(manifest_dir).join(relative);
        let body = std::fs::read_to_string(&path).unwrap_or_else(|error| {
            panic!("F1: failed to read {label} at {path:?}: {error}");
        });
        assert!(
            body.contains("use subtle::ConstantTimeEq;"),
            "F1: {label} must import `subtle::ConstantTimeEq` so the production \
             consume_if_matches path can perform constant-time comparison. A \
             non-constant-time `!=` re-opens the timing oracle described in the \
             2026-05 audit (F1, HIGH)."
        );
        assert!(
            body.contains("ConstantTimeEq::ct_eq"),
            "F1: {label} must call `ConstantTimeEq::ct_eq(..)` on the decrypted \
             plaintext. Reverting to `decrypted.expose() != expected_value` \
             short-circuits on the first differing byte and lets a network-side \
             adversary recover the secret byte-by-byte via response-latency \
             measurement."
        );
        assert!(
            !body.contains("decrypted.expose() != expected_value"),
            "F1: {label} still contains a non-constant-time \
             `decrypted.expose() != expected_value` comparison. Use \
             `ConstantTimeEq::ct_eq` instead."
        );
    }
}
