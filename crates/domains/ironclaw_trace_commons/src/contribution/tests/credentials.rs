//! Credential resolution, opt-out, effective flush targets, and claim subjects.

use crate::contribution::*;

use super::support::*;

#[test]
fn resolver_prefers_personal_invite_enrollment_with_no_subject() {
    let dir = tempfile::tempdir().unwrap();
    let scope = trace_scope_key("tenant-a", "alice");
    let personal = StandingTraceContributionPolicy {
        enabled: true,
        ..Default::default()
    };
    write_policy_at(dir.path(), Some(scope.as_str()), &personal);

    let r = resolve_trace_credentials_at(dir.path(), "tenant-a", "alice")
        .unwrap()
        .unwrap();
    assert_eq!(r.state_scope, scope);
    assert_eq!(r.subject, None, "personal invite carries no subject");
    assert!(r.policy.enabled);
}
#[test]
fn resolver_falls_back_to_instance_enrollment_with_per_user_subject() {
    let dir = tempfile::tempdir().unwrap();
    // No personal policy; only the instance-level (scope None) policy.
    let instance = StandingTraceContributionPolicy {
        enabled: true,
        ..Default::default()
    };
    write_policy_at(dir.path(), None, &instance);

    let r = resolve_trace_credentials_at(dir.path(), "tenant-a", "alice")
        .unwrap()
        .unwrap();
    let expected_scope = trace_scope_key("tenant-a", "alice");
    let subject = r
        .subject
        .clone()
        .expect("instance fallback carries a subject");
    assert!(r.policy.enabled);

    // The subject must be SALTED with per-instance random state: an
    // unsalted hash of the raw scope lets the server dictionary-match
    // guessable tenant/user ids and de-pseudonymize contributors.
    assert_ne!(
        subject,
        local_pseudonymous_contributor_id(&expected_scope),
        "instance subject must not be the unsalted scope hash"
    );

    // Stable within an instance: same (base, scope) → same subject.
    let again = resolve_trace_credentials_at(dir.path(), "tenant-a", "alice")
        .unwrap()
        .unwrap();
    assert_eq!(again.subject.as_deref(), Some(subject.as_str()));

    // Distinct across instances: a different base dir has a different salt.
    let other = tempfile::tempdir().unwrap();
    write_policy_at(other.path(), None, &instance);
    let other_subject = resolve_trace_credentials_at(other.path(), "tenant-a", "alice")
        .unwrap()
        .unwrap()
        .subject
        .expect("other instance resolves a subject");
    assert_ne!(
        other_subject, subject,
        "different instances must derive different subjects for the same scope"
    );
}
#[test]
fn capture_policy_resolves_personal_then_instance_then_none() {
    let dir = tempfile::tempdir().unwrap();
    let scope = trace_scope_key("tenant-a", "alice");

    // Neither enrolled → capture must skip.
    assert!(
        resolve_effective_capture_policy_at(dir.path(), Some(scope.as_str()))
            .unwrap()
            .is_none(),
        "unenrolled scope must yield no capture policy"
    );

    // Instance-only enrollment (scope None), no per-user policy: capture must
    // resolve the instance policy — the P1 the per-user-only gate dropped.
    let instance = StandingTraceContributionPolicy {
        enabled: true,
        upload_token_tenant_id: Some("instance-tenant".to_string()),
        ..Default::default()
    };
    write_policy_at(dir.path(), None, &instance);
    let resolved = resolve_effective_capture_policy_at(dir.path(), Some(scope.as_str()))
        .unwrap()
        .expect("instance-only scope must capture under the instance policy");
    assert!(resolved.enabled);
    assert_eq!(
        resolved.upload_token_tenant_id.as_deref(),
        Some("instance-tenant"),
        "instance-only capture must use the instance policy"
    );

    // A user's own enabled personal-invite policy takes precedence.
    let personal = StandingTraceContributionPolicy {
        enabled: true,
        upload_token_tenant_id: Some("personal-tenant".to_string()),
        ..Default::default()
    };
    write_policy_at(dir.path(), Some(scope.as_str()), &personal);
    let resolved = resolve_effective_capture_policy_at(dir.path(), Some(scope.as_str()))
        .unwrap()
        .expect("personal enrollment resolves");
    assert_eq!(
        resolved.upload_token_tenant_id.as_deref(),
        Some("personal-tenant"),
        "personal-invite policy must take precedence over the instance policy"
    );
}
#[test]
fn resolver_returns_none_when_unenrolled() {
    let dir = tempfile::tempdir().unwrap();
    // Empty dir — no policy files at all.
    assert!(
        resolve_trace_credentials_at(dir.path(), "tenant-a", "alice")
            .unwrap()
            .is_none()
    );
}
#[test]
fn opt_out_user_scope_blocks_only_that_user_never_the_instance() {
    // Regression (PR #5858 review): the CLI's opt-out used to flip the
    // ROOT policy too — which, under instance enrollment, disenrolled the
    // ENTIRE instance when one user opted out. The per-user opt-out
    // primitive must write only the user's scoped policy.
    let dir = tempfile::tempdir().unwrap();
    write_policy_at(
        dir.path(),
        None,
        &StandingTraceContributionPolicy {
            enabled: true,
            ..Default::default()
        },
    );

    let alice = trace_scope_key("tenant-a", "alice");
    opt_out_user_scope_at(dir.path(), &alice).expect("opt-out writes");

    // The instance policy is untouched on disk.
    let instance = read_trace_policy_for_scope_at(dir.path(), None).expect("instance reads");
    assert!(
        instance.enabled,
        "per-user opt-out must never disable the instance enrollment"
    );
    // Alice is out on every resolution surface…
    assert!(
        resolve_trace_credentials_at(dir.path(), "tenant-a", "alice")
            .unwrap()
            .is_none(),
        "opted-out user must not resolve instance credentials"
    );
    // …while other users still inherit the instance enrollment.
    assert!(
        resolve_trace_credentials_at(dir.path(), "tenant-a", "bob")
            .unwrap()
            .is_some(),
        "other users must keep inheriting the instance enrollment"
    );
}
#[test]
fn resolver_explicit_user_opt_out_blocks_instance_fallback() {
    // `traces opt-out` writes the user's scoped policy with enabled=false.
    // That explicit opt-out must win over an enabled instance policy on
    // EVERY resolution surface (credentials, flush, capture) — a disabled
    // scoped policy file is not the same as "never configured".
    let dir = tempfile::tempdir().unwrap();
    let scope = trace_scope_key("tenant-a", "alice");
    write_policy_at(
        dir.path(),
        None,
        &StandingTraceContributionPolicy {
            enabled: true,
            ..Default::default()
        },
    );
    write_policy_at(
        dir.path(),
        Some(scope.as_str()),
        &StandingTraceContributionPolicy {
            enabled: false,
            ..Default::default()
        },
    );

    assert!(
        resolve_trace_credentials_at(dir.path(), "tenant-a", "alice")
            .unwrap()
            .is_none(),
        "explicit per-user opt-out must not resolve to instance credentials"
    );
    assert!(
        resolve_effective_flush_target_at(dir.path(), Some(scope.as_str()))
            .unwrap()
            .is_none(),
        "explicit per-user opt-out must not flush under instance enrollment"
    );
    assert!(
        resolve_effective_capture_policy_at(dir.path(), Some(scope.as_str()))
            .unwrap()
            .is_none(),
        "explicit per-user opt-out must not capture under the instance policy"
    );
}

// --- resolve_effective_flush_target tests ---
// Same isolation contract as the resolver tests: each uses its own tempdir
// passed to the private `_at` core, so they never touch the global
// IRONCLAW_BASE_DIR. These prove the autonomous flush gate is resolver-aware:
// an instance-only enrollment resolves to a contributing target (so the gate
// no longer aborts) carrying the per-user pseudonymous subject and the
// INSTANCE device-key dir.
#[test]
fn effective_flush_target_personal_enabled_uses_scope_dir_and_no_subject() {
    let dir = tempfile::tempdir().unwrap();
    let scope = trace_scope_key("tenant-a", "alice");
    let personal = StandingTraceContributionPolicy {
        enabled: true,
        ..Default::default()
    };
    write_policy_at(dir.path(), Some(scope.as_str()), &personal);

    let target = resolve_effective_flush_target_at(dir.path(), Some(scope.as_str()))
        .unwrap()
        .expect("personal-enabled scope is a contributing target");
    assert!(target.policy.enabled);
    assert_eq!(target.subject, None, "personal invite carries no subject");
    assert_eq!(
        target.device_key_dir,
        trace_contribution_dir_for_scope_at(dir.path(), Some(scope.as_str())),
        "personal enrollment loads its device key from the per-scope dir"
    );
}
#[test]
fn effective_flush_target_instance_only_uses_instance_dir_and_subject() {
    let dir = tempfile::tempdir().unwrap();
    let scope = trace_scope_key("tenant-a", "alice");
    // No personal policy for the scope; only the instance-level (None) policy.
    let instance = StandingTraceContributionPolicy {
        enabled: true,
        ..Default::default()
    };
    write_policy_at(dir.path(), None, &instance);

    let target = resolve_effective_flush_target_at(dir.path(), Some(scope.as_str()))
        .unwrap()
        .expect("instance-enrolled scope is a contributing target (gate must not abort)");
    assert!(target.policy.enabled);
    assert_eq!(
        target.subject,
        Some(salted_pseudonymous_contributor_id_at(dir.path(), &scope).unwrap()),
        "instance enrollment attributes the user via a salted per-user pseudonymous subject"
    );
    assert_eq!(
        target.device_key_dir,
        trace_contribution_dir_for_scope_at(dir.path(), None),
        "instance enrollment loads the shared device key from the instance (None) dir"
    );
}
#[test]
fn effective_flush_target_none_when_unenrolled() {
    let dir = tempfile::tempdir().unwrap();
    let scope = trace_scope_key("tenant-a", "alice");
    // Empty dir — neither a personal nor an instance policy is enabled.
    assert!(
        resolve_effective_flush_target_at(dir.path(), Some(scope.as_str()))
            .unwrap()
            .is_none(),
        "unenrolled scope has no contributing target"
    );
}
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
fn upload_claim_cache_key_separates_subjects_sharing_a_scope_dir() {
    // Instance enrollment: all users share the SAME instance device-key dir
    // (scope None), distinguished only by their per-user subject. The cache
    // key MUST differ per subject, or a claim minted for one user would be
    // served from cache to another (cross-user trace mis-attribution).
    let policy = StandingTraceContributionPolicy {
        enabled: true,
        auth_mode: TraceUploadAuthMode::DeviceKey,
        upload_token_issuer_url: Some("https://issuer.example/v1/trace-upload-claim".to_string()),
        upload_token_tenant_id: Some("tenant-a".to_string()),
        upload_token_audience: Some("trace-commons".to_string()),
        ..Default::default()
    };
    let shared_dir = std::path::PathBuf::from("/instance/trace_contributions");
    let ctx_for = |subject: &str| TraceUploadClaimContext {
        trace_id: None,
        submission_id: None,
        consent_scopes: vec![ConsentScope::DebuggingEvaluation],
        allowed_uses: Vec::new(),
        scope_dir: Some(shared_dir.clone()),
        subject: Some(subject.to_string()),
    };

    let alice = trace_upload_claim_cache_key(&policy, &ctx_for("sha256:alice")).unwrap();
    let bob = trace_upload_claim_cache_key(&policy, &ctx_for("sha256:bob")).unwrap();
    let alice_again = trace_upload_claim_cache_key(&policy, &ctx_for("sha256:alice")).unwrap();

    assert_ne!(
        alice, bob,
        "distinct subjects sharing a scope_dir must get distinct cache keys"
    );
    assert_eq!(
        alice, alice_again,
        "same subject must produce a stable cache key"
    );

    // A no-subject context (personal-invite path) must also differ from the
    // subject-bearing keys so the two models never collide on cache.
    let no_subject = TraceUploadClaimContext {
        trace_id: None,
        submission_id: None,
        consent_scopes: vec![ConsentScope::DebuggingEvaluation],
        allowed_uses: Vec::new(),
        scope_dir: Some(shared_dir.clone()),
        subject: None,
    };
    let none_key = trace_upload_claim_cache_key(&policy, &no_subject).unwrap();
    assert_ne!(alice, none_key);
    assert_ne!(bob, none_key);

    // The key hashes the exact optional bytes with a None/Some discriminator,
    // so `Some("")` and whitespace variants never collide with `None` or with
    // each other (which would let one payload's claim serve another's).
    let empty_key = trace_upload_claim_cache_key(&policy, &ctx_for("")).unwrap();
    assert_ne!(
        empty_key, none_key,
        "Some(\"\") must not share a key with None"
    );
    let padded = trace_upload_claim_cache_key(&policy, &ctx_for("  sha256:alice  ")).unwrap();
    assert_ne!(
        padded, alice,
        "whitespace-padded subject must not collide with its trimmed form"
    );
}
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
