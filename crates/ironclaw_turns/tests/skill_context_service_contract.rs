//! Contract tests for `SkillContextService` and related types.
//!
//! Covers: no skills, skill unavailable, missing/denied trust, hidden capability,
//! deterministic ordering/rebuild, and redaction of non-model-safe metadata.

use ironclaw_turns::run_profile::{
    InstalledSkillSnapshot, NoopSkillContextSource, SkillActivationState, SkillContextBudget,
    SkillContextError, SkillContextService, SkillContextSource, SkillRunSnapshot, SkillTrustLevel,
    SkillVisibility,
};
use sha2::{Digest, Sha256};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn visible_trusted(name: &str, description: &str, prompt: &str) -> InstalledSkillSnapshot {
    InstalledSkillSnapshot {
        name: name.to_string(),
        trust: SkillTrustLevel::Trusted,
        visibility: SkillVisibility::Visible,
        activation_state: SkillActivationState::Loaded,
        prompt_content: Some(prompt.to_string()),
        safe_description: description.to_string(),
        ordering_key: name.to_string(),
    }
}

fn visible_trusted_without_prompt(name: &str, description: &str) -> InstalledSkillSnapshot {
    InstalledSkillSnapshot {
        name: name.to_string(),
        trust: SkillTrustLevel::Trusted,
        visibility: SkillVisibility::Visible,
        activation_state: SkillActivationState::Loaded,
        prompt_content: None,
        safe_description: description.to_string(),
        ordering_key: name.to_string(),
    }
}

fn discoverable_trusted_with_prompt(
    name: &str,
    description: &str,
    prompt: &str,
) -> InstalledSkillSnapshot {
    InstalledSkillSnapshot {
        name: name.to_string(),
        trust: SkillTrustLevel::Trusted,
        visibility: SkillVisibility::Visible,
        activation_state: SkillActivationState::Discoverable,
        prompt_content: Some(prompt.to_string()),
        safe_description: description.to_string(),
        ordering_key: name.to_string(),
    }
}

fn visible_installed(name: &str, description: &str) -> InstalledSkillSnapshot {
    InstalledSkillSnapshot {
        name: name.to_string(),
        trust: SkillTrustLevel::Installed,
        visibility: SkillVisibility::Visible,
        activation_state: SkillActivationState::Loaded,
        prompt_content: Some("secret prompt".to_string()),
        safe_description: description.to_string(),
        ordering_key: name.to_string(),
    }
}

fn hidden_skill(name: &str) -> InstalledSkillSnapshot {
    InstalledSkillSnapshot {
        name: name.to_string(),
        trust: SkillTrustLevel::Trusted,
        visibility: SkillVisibility::Hidden,
        activation_state: SkillActivationState::Loaded,
        prompt_content: Some("hidden prompt".to_string()),
        safe_description: "hidden description".to_string(),
        ordering_key: name.to_string(),
    }
}

fn denied_skill(name: &str) -> InstalledSkillSnapshot {
    InstalledSkillSnapshot {
        name: name.to_string(),
        trust: SkillTrustLevel::Trusted,
        visibility: SkillVisibility::Denied,
        activation_state: SkillActivationState::Loaded,
        prompt_content: Some("denied prompt".to_string()),
        safe_description: "denied description".to_string(),
        ordering_key: name.to_string(),
    }
}

fn legacy_snapshot_version(entries: &[InstalledSkillSnapshot]) -> String {
    let mut digest = Sha256::new();

    for entry in entries {
        feed_legacy_digest_field(&mut digest, entry.name.as_bytes());
        feed_legacy_digest_field(
            &mut digest,
            match entry.trust {
                SkillTrustLevel::Installed => b"installed",
                SkillTrustLevel::Trusted => b"trusted",
            },
        );
        feed_legacy_digest_field(
            &mut digest,
            match entry.visibility {
                SkillVisibility::Visible => b"visible",
                SkillVisibility::Hidden => b"hidden",
                SkillVisibility::Denied => b"denied",
            },
        );
        match entry.prompt_content {
            Some(ref content) => {
                digest.update([1]);
                feed_legacy_digest_field(&mut digest, content.as_bytes());
            }
            None => digest.update([0]),
        }
        feed_legacy_digest_field(&mut digest, entry.safe_description.as_bytes());
        feed_legacy_digest_field(&mut digest, entry.ordering_key.as_bytes());
        digest.update([0xFE]);
    }

    format!("sha256:{}", hex::encode(digest.finalize()))
}

fn feed_legacy_digest_field(digest: &mut Sha256, bytes: &[u8]) {
    digest.update((bytes.len() as u64).to_le_bytes());
    digest.update(bytes);
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn no_skills_produces_empty_ok() {
    let snapshot = SkillRunSnapshot::empty();
    let service = SkillContextService::new(snapshot.clone());
    let result = service.skill_snippets(&snapshot).await;
    assert_eq!(result.unwrap(), vec![]);
}

#[tokio::test]
async fn all_hidden_or_denied_produces_empty_ok() {
    let snapshot = SkillRunSnapshot::from_entries(vec![
        hidden_skill("alpha"),
        denied_skill("beta"),
        hidden_skill("gamma"),
    ]);
    let service = SkillContextService::new(snapshot.clone());
    let result = service.skill_snippets(&snapshot).await.unwrap();
    assert!(result.is_empty());
}

#[tokio::test]
async fn missing_trust_data_fails_closed() {
    let snapshot = SkillRunSnapshot {
        entries: vec![visible_trusted("alpha", "desc", "prompt")],
        snapshot_version: String::new(), // empty = missing
    };
    let service = SkillContextService::new(snapshot.clone());
    let err = service.skill_snippets(&snapshot).await.unwrap_err();
    assert_eq!(err, SkillContextError::TrustDataMissing);
}

#[tokio::test]
async fn denied_visibility_never_in_output() {
    let snapshot = SkillRunSnapshot::from_entries(vec![
        visible_trusted("alpha", "visible skill", "prompt"),
        denied_skill("beta"),
    ]);
    let service = SkillContextService::new(snapshot.clone());
    let snippets = service.skill_snippets(&snapshot).await.unwrap();
    assert_eq!(snippets.len(), 1);
    assert_eq!(snippets[0].snippet_ref, "skill:alpha");
    assert!(!snippets[0].safe_summary.contains("denied"));
}

#[tokio::test]
async fn hidden_visibility_never_in_output() {
    let snapshot = SkillRunSnapshot::from_entries(vec![
        visible_trusted("alpha", "visible skill", "prompt"),
        hidden_skill("beta"),
    ]);
    let service = SkillContextService::new(snapshot.clone());
    let snippets = service.skill_snippets(&snapshot).await.unwrap();
    assert_eq!(snippets.len(), 1);
    assert_eq!(snippets[0].snippet_ref, "skill:alpha");
    assert!(!snippets[0].safe_summary.contains("hidden"));
}

#[tokio::test]
async fn trusted_skill_includes_prompt_content() {
    let snapshot = SkillRunSnapshot::from_entries(vec![visible_trusted(
        "alpha",
        "the description",
        "the prompt content",
    )]);
    let service = SkillContextService::new(snapshot.clone());
    let snippets = service.skill_snippets(&snapshot).await.unwrap();
    assert_eq!(snippets.len(), 1);
    assert!(snippets[0].safe_summary.contains("the description"));
    assert!(!snippets[0].safe_summary.contains("the prompt content"));
    assert!(snippets[0].model_content.contains("the description"));
    assert!(snippets[0].model_content.contains("the prompt content"));
}

#[tokio::test]
async fn discoverable_trusted_skill_excludes_prompt_content() {
    let snapshot = SkillRunSnapshot::from_entries(vec![discoverable_trusted_with_prompt(
        "alpha",
        "the description",
        "the prompt content",
    )]);
    assert_eq!(snapshot.entries[0].prompt_content, None);
    let service = SkillContextService::new(snapshot.clone());

    let snippets = service.skill_snippets(&snapshot).await.unwrap();

    assert_eq!(snippets.len(), 1);
    assert_eq!(snippets[0].safe_summary, "the description");
    assert_eq!(snippets[0].model_content, "the description");
    assert!(
        !snippets[0].model_content.contains("the prompt content"),
        "discoverable skills must not expose prompt content before activation"
    );
}

#[tokio::test]
async fn legacy_skill_snapshot_defaults_missing_activation_state_to_loaded() {
    let entry = visible_trusted("alpha", "legacy description", "legacy prompt");
    let legacy_version = legacy_snapshot_version(std::slice::from_ref(&entry));
    let legacy_wire = serde_json::json!({
        "entries": [{
            "name": entry.name,
            "trust": "trusted",
            "visibility": "visible",
            "prompt_content": "legacy prompt",
            "safe_description": "legacy description",
            "ordering_key": "alpha"
        }],
        "snapshot_version": legacy_version
    });
    let snapshot: SkillRunSnapshot = serde_json::from_value(legacy_wire).unwrap();

    assert_eq!(
        snapshot.entries[0].activation_state,
        SkillActivationState::Loaded
    );
    let service = SkillContextService::new(snapshot.clone());
    let snippets = service.skill_snippets(&snapshot).await.unwrap();

    assert_eq!(snippets.len(), 1);
    assert_eq!(snippets[0].safe_summary, "legacy description");
    assert!(snippets[0].model_content.contains("legacy prompt"));
}

#[tokio::test]
async fn trusted_skill_allows_operational_paths_in_prompt_content() {
    let prompt = concat!(
        "Create a review worktree under /tmp/ironclaw-review-123 and ",
        "write the GitHub payload to /tmp/cr-review-payload.json."
    );
    let snapshot =
        SkillRunSnapshot::from_entries(vec![visible_trusted("alpha", "the description", prompt)]);
    let service = SkillContextService::new(snapshot.clone());

    let snippets = service.skill_snippets(&snapshot).await.unwrap();

    assert_eq!(snippets.len(), 1);
    assert_eq!(snippets[0].safe_summary, "the description");
    assert!(
        snippets[0]
            .model_content
            .contains("/tmp/ironclaw-review-123")
    );
    assert!(
        snippets[0]
            .model_content
            .contains("/tmp/cr-review-payload.json")
    );
}

#[tokio::test]
async fn installed_skill_excludes_prompt_content() {
    let snapshot =
        SkillRunSnapshot::from_entries(vec![visible_installed("alpha", "the description")]);
    let service = SkillContextService::new(snapshot.clone());
    let snippets = service.skill_snippets(&snapshot).await.unwrap();
    assert_eq!(snippets.len(), 1);
    assert!(snippets[0].safe_summary.contains("the description"));
    assert!(snippets[0].model_content.contains("the description"));
    assert!(
        !snippets[0].model_content.contains("secret prompt"),
        "installed skill must not expose prompt content"
    );
}

#[tokio::test]
async fn trusted_skill_without_prompt_uses_description_only() {
    let snapshot = SkillRunSnapshot::from_entries(vec![visible_trusted_without_prompt(
        "alpha",
        "the description",
    )]);
    let service = SkillContextService::new(snapshot.clone());
    let snippets = service.skill_snippets(&snapshot).await.unwrap();
    assert_eq!(snippets.len(), 1);
    assert_eq!(snippets[0].safe_summary, "the description");
}

#[tokio::test]
async fn deterministic_ordering_same_snapshot() {
    let snapshot = SkillRunSnapshot::from_entries(vec![
        visible_trusted("charlie", "desc c", "prompt c"),
        visible_trusted("alpha", "desc a", "prompt a"),
        visible_trusted("bravo", "desc b", "prompt b"),
    ]);
    let service = SkillContextService::new(snapshot.clone());
    let first = service.skill_snippets(&snapshot).await.unwrap();
    let second = service.skill_snippets(&snapshot).await.unwrap();
    assert_eq!(
        first, second,
        "same snapshot must produce byte-equal output"
    );
    // Verify sorted order
    let names: Vec<&str> = first.iter().map(|s| s.snippet_ref.as_str()).collect();
    assert_eq!(names, vec!["skill:alpha", "skill:bravo", "skill:charlie"]);
}

#[tokio::test]
async fn deterministic_ordering_shuffled_input() {
    let entries_a = vec![
        visible_trusted("charlie", "desc c", "prompt c"),
        visible_trusted("alpha", "desc a", "prompt a"),
        visible_trusted("bravo", "desc b", "prompt b"),
    ];
    let entries_b = vec![
        visible_trusted("bravo", "desc b", "prompt b"),
        visible_trusted("charlie", "desc c", "prompt c"),
        visible_trusted("alpha", "desc a", "prompt a"),
    ];
    let snap_a = SkillRunSnapshot::from_entries(entries_a);
    let snap_b = SkillRunSnapshot::from_entries(entries_b);

    let service_a = SkillContextService::new(snap_a.clone());
    let service_b = SkillContextService::new(snap_b.clone());

    let output_a = service_a.skill_snippets(&snap_a).await.unwrap();
    let output_b = service_b.skill_snippets(&snap_b).await.unwrap();
    assert_eq!(output_a, output_b, "insertion order must not affect output");
}

#[tokio::test]
async fn snapshot_version_determinism() {
    let entries_a = vec![
        visible_trusted("charlie", "desc c", "prompt c"),
        visible_trusted("alpha", "desc a", "prompt a"),
    ];
    let entries_b = vec![
        visible_trusted("alpha", "desc a", "prompt a"),
        visible_trusted("charlie", "desc c", "prompt c"),
    ];
    let snap_a = SkillRunSnapshot::from_entries(entries_a);
    let snap_b = SkillRunSnapshot::from_entries(entries_b);
    assert_eq!(
        snap_a.snapshot_version, snap_b.snapshot_version,
        "same entries in different order must produce the same version"
    );
}

#[tokio::test]
async fn snapshot_version_uses_sha256_digest() {
    let snapshot = SkillRunSnapshot::from_entries(vec![visible_trusted("alpha", "desc", "prompt")]);

    assert!(
        snapshot.snapshot_version.starts_with("sha256:"),
        "snapshot version must use collision-resistant digest, got {}",
        snapshot.snapshot_version
    );
    assert_eq!(
        snapshot.snapshot_version.len(),
        "sha256:".len() + 64,
        "SHA-256 digest must be hex-encoded"
    );
}

#[tokio::test]
async fn tampered_snapshot_version_fails_closed() {
    let mut snapshot =
        SkillRunSnapshot::from_entries(vec![visible_trusted("alpha", "desc", "prompt")]);
    snapshot.entries[0].safe_description = "tampered desc".to_string();

    let service = SkillContextService::new(snapshot.clone());
    let err = service.skill_snippets(&snapshot).await.unwrap_err();
    assert_eq!(err, SkillContextError::InvalidSnapshotVersion);
}

#[tokio::test]
async fn oversized_single_snippet_is_allowed_within_aggregate_budget() {
    let safe_description = "desc";
    let prompt = "x".repeat(16 * 1024);
    let model_content_bytes = safe_description.len() + "\n\n".len() + prompt.len();
    let max_context_bytes = "skill:alpha".len() + model_content_bytes;
    let snapshot =
        SkillRunSnapshot::from_entries(vec![visible_trusted("alpha", safe_description, &prompt)]);
    let service = SkillContextService::with_budget(
        snapshot.clone(),
        SkillContextBudget {
            max_snippet_bytes: model_content_bytes + 1,
            max_context_bytes,
        },
    );

    let snippets = service.skill_snippets(&snapshot).await.unwrap();
    assert_eq!(snippets.len(), 1);
    assert_eq!(snippets[0].safe_summary, safe_description);
    assert!(!snippets[0].safe_summary.contains(&prompt));
    assert!(snippets[0].model_content.contains(safe_description));
    assert!(snippets[0].model_content.contains(&prompt));
}

#[tokio::test]
async fn single_snippet_over_per_snippet_budget_fails_budget() {
    let prompt = "x".repeat(128);
    let snapshot = SkillRunSnapshot::from_entries(vec![visible_trusted("alpha", "desc", &prompt)]);
    let service = SkillContextService::with_budget(
        snapshot.clone(),
        SkillContextBudget {
            max_snippet_bytes: 64,
            max_context_bytes: 512,
        },
    );

    let err = service.skill_snippets(&snapshot).await.unwrap_err();
    assert_eq!(err, SkillContextError::ContextBudgetExceeded);
}

#[tokio::test]
async fn single_snippet_at_per_snippet_budget_limit_is_allowed() {
    let max_snippet_bytes = 64;
    let safe_description = "desc";
    let prompt_prefix_bytes = safe_description.len() + "\n\n".len();
    let prompt = "x".repeat(max_snippet_bytes - prompt_prefix_bytes);
    let snapshot =
        SkillRunSnapshot::from_entries(vec![visible_trusted("alpha", safe_description, &prompt)]);
    let service = SkillContextService::with_budget(
        snapshot.clone(),
        SkillContextBudget {
            max_snippet_bytes,
            max_context_bytes: 128,
        },
    );

    let snippets = service.skill_snippets(&snapshot).await.unwrap();
    assert_eq!(snippets.len(), 1);
    assert_eq!(snippets[0].model_content.len(), max_snippet_bytes);
}

#[tokio::test]
async fn aggregate_skill_context_fails_budget() {
    let snapshot = SkillRunSnapshot::from_entries(vec![
        visible_trusted("alpha", "first description", "first prompt"),
        visible_trusted("beta", "second description", "second prompt"),
    ]);
    let service = SkillContextService::with_budget(snapshot.clone(), SkillContextBudget::new(64));

    let err = service.skill_snippets(&snapshot).await.unwrap_err();
    assert_eq!(err, SkillContextError::ContextBudgetExceeded);
}

#[tokio::test]
async fn aggregate_skill_context_allows_exact_budget_limit() {
    let snapshot = SkillRunSnapshot::from_entries(vec![
        visible_trusted_without_prompt("alpha", "first"),
        visible_trusted_without_prompt("beta", "second"),
    ]);
    let max_context_bytes =
        "skill:alpha".len() + "first".len() + "skill:beta".len() + "second".len();
    let service = SkillContextService::with_budget(
        snapshot.clone(),
        SkillContextBudget::new(max_context_bytes),
    );

    let snippets = service.skill_snippets(&snapshot).await.unwrap();
    assert_eq!(snippets.len(), 2);
    let actual_context_bytes: usize = snippets
        .iter()
        .map(|snippet| snippet.snippet_ref.len() + snippet.model_content.len())
        .sum();
    assert_eq!(actual_context_bytes, max_context_bytes);
}

#[tokio::test]
async fn invalid_budget_configuration_is_distinct_from_exceeded_budget() {
    for budget in [SkillContextBudget::new(0)] {
        let snapshot =
            SkillRunSnapshot::from_entries(vec![visible_trusted("alpha", "desc", "prompt")]);
        let service = SkillContextService::with_budget(snapshot.clone(), budget);

        let err = service.skill_snippets(&snapshot).await.unwrap_err();
        assert_eq!(
            err,
            SkillContextError::BudgetMisconfigured,
            "misconfiguration {budget:?} must not be reported as a runtime budget overflow"
        );
    }
}

#[tokio::test]
async fn duplicate_ordering_keys_use_total_order() {
    let mut alpha = visible_trusted("alpha", "desc a", "prompt a");
    alpha.ordering_key = "same".to_string();
    let mut beta = visible_trusted("beta", "desc b", "prompt b");
    beta.ordering_key = "same".to_string();

    let snap_a = SkillRunSnapshot::from_entries(vec![beta.clone(), alpha.clone()]);
    let snap_b = SkillRunSnapshot::from_entries(vec![alpha, beta]);

    assert_eq!(snap_a.snapshot_version, snap_b.snapshot_version);

    let service_a = SkillContextService::new(snap_a.clone());
    let service_b = SkillContextService::new(snap_b.clone());
    let output_a = service_a.skill_snippets(&snap_a).await.unwrap();
    let output_b = service_b.skill_snippets(&snap_b).await.unwrap();

    assert_eq!(output_a, output_b);
    let refs: Vec<&str> = output_a.iter().map(|s| s.snippet_ref.as_str()).collect();
    assert_eq!(refs, vec!["skill:alpha", "skill:beta"]);
}

#[tokio::test]
async fn unsafe_visible_metadata_fails_before_loop_snippet_emission() {
    let cases = vec![
        (
            "unsafe name would leak through snippet_ref",
            SkillRunSnapshot::from_entries(vec![visible_trusted(
                "/Users/alice/.ssh/id_rsa",
                "safe description",
                "safe prompt",
            )]),
        ),
        (
            "unsafe description would leak through safe_summary",
            SkillRunSnapshot::from_entries(vec![visible_trusted(
                "alpha",
                "raw capability handle cap_file_read_123",
                "safe prompt",
            )]),
        ),
        (
            "uppercase capability marker in description would leak through safe_summary",
            SkillRunSnapshot::from_entries(vec![visible_trusted(
                "alpha",
                "raw capability handle CAP_file_read_123",
                "safe prompt",
            )]),
        ),
    ];

    for (case, snapshot) in cases {
        let service = SkillContextService::new(snapshot.clone());
        let err = service.skill_snippets(&snapshot).await.unwrap_err();
        assert_eq!(
            err,
            SkillContextError::UnsafeModelVisibleContent,
            "{case} must fail closed before model-visible snippet emission"
        );
    }
}

#[tokio::test]
async fn redaction_no_raw_paths_or_internals() {
    let snapshot = SkillRunSnapshot::from_entries(vec![
        visible_trusted("alpha", "A helpful skill", "Use this skill to help"),
        visible_installed("beta", "Another helpful skill"),
    ]);
    let service = SkillContextService::new(snapshot.clone());
    let snippets = service.skill_snippets(&snapshot).await.unwrap();
    for snippet in &snippets {
        // No file paths
        assert!(
            !snippet.safe_summary.contains('/'),
            "must not contain file path separators"
        );
        assert!(
            !snippet.safe_summary.contains('\\'),
            "must not contain file path separators"
        );
        // No capability IDs (would look like UUIDs or structured IDs)
        assert!(
            !snippet.safe_summary.contains("cap_"),
            "must not contain capability IDs"
        );
        // No secret handles
        assert!(
            !snippet.safe_summary.contains("secret"),
            "must not contain secret handles"
        );
        // Only contains description/prompt content
        assert!(
            snippet.safe_summary.contains("helpful skill")
                || snippet.safe_summary.contains("Use this skill"),
            "must contain only model-safe content"
        );
    }
}

#[tokio::test]
async fn noop_skill_context_source_returns_empty() {
    let noop = NoopSkillContextSource;
    let snapshot = SkillRunSnapshot::from_entries(vec![visible_trusted("alpha", "desc", "prompt")]);
    let result = noop.skill_snippets(&snapshot).await.unwrap();
    assert!(result.is_empty());
}

#[tokio::test]
async fn mixed_visibility_correct_filtering() {
    let snapshot = SkillRunSnapshot::from_entries(vec![
        visible_trusted("alpha", "trusted visible", "trusted prompt"),
        visible_installed("beta", "installed visible"),
        hidden_skill("gamma"),
        denied_skill("delta"),
    ]);
    let service = SkillContextService::new(snapshot.clone());
    let snippets = service.skill_snippets(&snapshot).await.unwrap();

    // Only visible entries appear
    assert_eq!(snippets.len(), 2);

    // Trusted includes prompt
    let alpha = snippets
        .iter()
        .find(|s| s.snippet_ref == "skill:alpha")
        .unwrap();
    assert!(alpha.safe_summary.contains("trusted visible"));
    assert!(!alpha.safe_summary.contains("trusted prompt"));
    assert!(alpha.model_content.contains("trusted visible"));
    assert!(alpha.model_content.contains("trusted prompt"));

    // Installed excludes prompt
    let beta = snippets
        .iter()
        .find(|s| s.snippet_ref == "skill:beta")
        .unwrap();
    assert!(beta.safe_summary.contains("installed visible"));
    assert!(
        !beta.model_content.contains("secret prompt"),
        "installed skill must not expose prompt content"
    );

    // Hidden and denied are absent
    let refs: Vec<&str> = snippets.iter().map(|s| s.snippet_ref.as_str()).collect();
    assert!(!refs.contains(&"skill:gamma"));
    assert!(!refs.contains(&"skill:delta"));
}

#[tokio::test]
async fn into_loop_snippet_conversion() {
    use ironclaw_turns::run_profile::LoopContextSnippet;

    let snapshot = SkillRunSnapshot::from_entries(vec![visible_trusted("alpha", "desc", "prompt")]);
    let service = SkillContextService::new(snapshot.clone());
    let snippets = service.skill_snippets(&snapshot).await.unwrap();
    let loop_snippet: LoopContextSnippet = snippets.into_iter().next().unwrap().into_loop_snippet();
    assert_eq!(loop_snippet.snippet_ref, "skill:alpha");
    assert!(loop_snippet.safe_summary.contains("desc"));
}
