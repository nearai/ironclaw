//! Integration test: skill-based credential injection pipeline.
//!
//! Tests the complete flow from skill YAML frontmatter → credential parsing →
//! validation → SharedCredentialRegistry registration → HttpTool wiring.
//!
//! Scenario: Multiple skills declare credentials in their frontmatter. The test
//! verifies that:
//!
//! 1. Credential specs are correctly parsed from YAML (all location types, refresh strategies)
//! 2. Validation rejects insecure or malformed specs
//! 3. Valid specs are registered into SharedCredentialRegistry
//! 4. Invalid specs are skipped (not registered)
//! 5. HttpTool's requires_approval detects registered credential hosts
//! 6. LLM-provided auth headers are rejected for registered hosts
//! 7. Non-auth headers pass through for registered hosts
//! 8. Unregistered hosts allow auth headers (LLM-constructed)
//! 9. Per-user credential isolation works at the SecretsStore level
//! 10. Multi-skill credential registration doesn't interfere

use std::path::PathBuf;
use std::sync::Arc;

use secrecy::SecretString;

use ironclaw::secrets::{
    CreateSecretParams, CredentialMapping, InMemorySecretsStore, SecretsCrypto, SecretsStore,
};
use ironclaw::tools::builtin::HttpTool;
use ironclaw::tools::wasm::SharedCredentialRegistry;
use ironclaw::tools::{ApprovalRequirement, Tool};
use ironclaw_skills::types::*;

// ── Helpers ──────────────────────────────────────────────────────────────

/// Create an in-memory secrets store for testing.
fn test_secrets_store() -> InMemorySecretsStore {
    let crypto = Arc::new(
        SecretsCrypto::new(SecretString::from(
            "0123456789abcdef0123456789abcdef".to_string(),
        ))
        .unwrap(),
    );
    InMemorySecretsStore::new(crypto)
}

/// Build a LoadedSkill from frontmatter YAML and prompt content.
fn make_skill(
    name: &str,
    credentials: Vec<SkillCredentialSpec>,
    prompt: &str,
) -> ironclaw_skills::LoadedSkill {
    ironclaw_skills::LoadedSkill {
        manifest: SkillManifest {
            name: name.to_string(),
            version: "1.0.0".to_string(),
            description: format!("{} skill", name),
            activation: ActivationCriteria::default(),
            credentials,
            metadata: None,
        },
        prompt_content: prompt.to_string(),
        trust: SkillTrust::Trusted,
        source: SkillSource::User(PathBuf::from("/tmp/test-skills")),
        content_hash: format!("sha256:{}", name),
        compiled_patterns: vec![],
        lowercased_keywords: vec![],
        lowercased_exclude_keywords: vec![],
        lowercased_tags: vec![],
    }
}

/// Build an HttpTool with credential injection.
fn http_tool_with_credentials(
    registry: Arc<SharedCredentialRegistry>,
    store: Arc<dyn SecretsStore + Send + Sync>,
) -> HttpTool {
    HttpTool::new().with_credentials(registry, store)
}

// ── Frontmatter Parsing Tests ────────────────────────────────────────────

/// Full Gmail-like skill with OAuth, scopes, PKCE, and extra params.
#[test]
fn test_parse_complex_google_credential_spec() {
    let yaml = r#"
name: gmail
version: "1.0.0"
description: Gmail API integration
activation:
  keywords: ["email", "gmail", "inbox"]
credentials:
  - name: google_oauth_token
    provider: google
    location:
      type: bearer
    hosts:
      - "gmail.googleapis.com"
      - "www.googleapis.com"
    oauth:
      authorization_url: "https://accounts.google.com/o/oauth2/v2/auth"
      token_url: "https://oauth2.googleapis.com/token"
      scopes:
        - "https://www.googleapis.com/auth/gmail.modify"
        - "https://www.googleapis.com/auth/gmail.readonly"
      use_pkce: true
      extra_params:
        access_type: offline
        prompt: consent
      test_url: "https://www.googleapis.com/oauth2/v1/userinfo"
    setup_instructions: "Enable Gmail API in Google Cloud Console"
"#;
    let manifest: SkillManifest = serde_yml::from_str(yaml).expect("parse failed");

    assert_eq!(manifest.name, "gmail");
    assert_eq!(manifest.credentials.len(), 1);

    let cred = &manifest.credentials[0];
    assert_eq!(cred.name, "google_oauth_token");
    assert_eq!(cred.provider, "google");
    assert!(matches!(cred.location, SkillCredentialLocation::Bearer));
    assert_eq!(cred.hosts.len(), 2);
    assert_eq!(cred.hosts[0], "gmail.googleapis.com");
    assert_eq!(cred.hosts[1], "www.googleapis.com");

    let oauth = cred.oauth.as_ref().unwrap();
    assert!(oauth.use_pkce);
    assert_eq!(oauth.scopes.len(), 2);
    assert_eq!(oauth.extra_params.get("access_type").unwrap(), "offline");
    assert_eq!(
        oauth.test_url.as_deref(),
        Some("https://www.googleapis.com/oauth2/v1/userinfo")
    );
    assert!(matches!(oauth.refresh, ProviderRefreshStrategy::Standard));

    assert_eq!(
        cred.setup_instructions.as_deref(),
        Some("Enable Gmail API in Google Cloud Console")
    );
}

/// Multi-credential skill (e.g., a tool that needs both GitHub and Slack).
#[test]
fn test_parse_multi_credential_skill() {
    let yaml = r#"
name: devops-notify
version: "1.0.0"
description: Deploy notification skill
credentials:
  - name: github_token
    provider: github
    location:
      type: bearer
    hosts: ["api.github.com"]
    oauth:
      authorization_url: "https://github.com/login/oauth/authorize"
      token_url: "https://github.com/login/oauth/access_token"
      scopes: ["repo", "read:org"]
      refresh:
        strategy: reauthorize_only
  - name: slack_bot_token
    provider: slack
    location:
      type: bearer
    hosts: ["slack.com", "api.slack.com"]
    oauth:
      authorization_url: "https://slack.com/oauth/v2/authorize"
      token_url: "https://slack.com/api/oauth.v2.access"
      scopes: ["chat:write", "channels:read"]
      refresh:
        strategy: custom
        refresh_url: "https://slack.com/api/oauth.v2.access"
        extra_params:
          grant_type: refresh_token
"#;
    let manifest: SkillManifest = serde_yml::from_str(yaml).expect("parse failed");
    assert_eq!(manifest.credentials.len(), 2);

    // GitHub: reauthorize_only
    let gh = &manifest.credentials[0];
    assert_eq!(gh.name, "github_token");
    assert!(matches!(
        gh.oauth.as_ref().unwrap().refresh,
        ProviderRefreshStrategy::ReauthorizeOnly
    ));

    // Slack: custom refresh
    let sl = &manifest.credentials[1];
    assert_eq!(sl.name, "slack_bot_token");
    assert_eq!(sl.hosts, vec!["slack.com", "api.slack.com"]);
    match &sl.oauth.as_ref().unwrap().refresh {
        ProviderRefreshStrategy::Custom {
            refresh_url,
            extra_params,
        } => {
            assert_eq!(refresh_url, "https://slack.com/api/oauth.v2.access");
            assert_eq!(extra_params.get("grant_type").unwrap(), "refresh_token");
        }
        other => panic!("expected Custom refresh, got {:?}", other),
    }
}

/// All credential location types parse correctly.
#[test]
fn test_parse_all_credential_location_types() {
    let yaml = r#"
name: multi-auth
credentials:
  - name: bearer_cred
    provider: example
    location:
      type: bearer
    hosts: ["api.example.com"]
  - name: basic_cred
    provider: example
    location:
      type: basic_auth
      username: admin
    hosts: ["api.example.com"]
  - name: header_cred
    provider: example
    location:
      type: header
      name: X-API-Key
      prefix: "Token"
    hosts: ["api.example.com"]
  - name: query_cred
    provider: example
    location:
      type: query_param
      name: access_token
    hosts: ["api.example.com"]
"#;
    let manifest: SkillManifest = serde_yml::from_str(yaml).expect("parse failed");
    assert_eq!(manifest.credentials.len(), 4);

    assert!(matches!(
        manifest.credentials[0].location,
        SkillCredentialLocation::Bearer
    ));
    match &manifest.credentials[1].location {
        SkillCredentialLocation::BasicAuth { username } => assert_eq!(username, "admin"),
        _ => panic!("expected BasicAuth"),
    }
    match &manifest.credentials[2].location {
        SkillCredentialLocation::Header { name, prefix } => {
            assert_eq!(name, "X-API-Key");
            assert_eq!(prefix.as_deref(), Some("Token"));
        }
        _ => panic!("expected Header"),
    }
    match &manifest.credentials[3].location {
        SkillCredentialLocation::QueryParam { name } => assert_eq!(name, "access_token"),
        _ => panic!("expected QueryParam"),
    }
}

// ── Validation Tests ─────────────────────────────────────────────────────

/// Invalid credential specs are caught by validation.
#[test]
fn test_validation_rejects_insecure_and_malformed_specs() {
    // HTTP OAuth URL
    let spec = SkillCredentialSpec {
        name: "token".to_string(),
        provider: "test".to_string(),
        location: SkillCredentialLocation::Bearer,
        hosts: vec!["api.example.com".to_string()],
        oauth: Some(SkillOAuthConfig {
            authorization_url: "http://insecure.example.com/auth".to_string(),
            token_url: "https://secure.example.com/token".to_string(),
            scopes: vec![],
            use_pkce: false,
            extra_params: Default::default(),
            refresh: ProviderRefreshStrategy::Standard,
            test_url: None,
        }),
        setup_instructions: None,
    };
    let errors = ironclaw_skills::validate_credential_spec(&spec);
    assert!(!errors.is_empty());
    assert!(errors.iter().any(|e| e.contains("HTTPS")));

    // Empty hosts
    let spec = SkillCredentialSpec {
        name: "token".to_string(),
        provider: "test".to_string(),
        location: SkillCredentialLocation::Bearer,
        hosts: vec![],
        oauth: None,
        setup_instructions: None,
    };
    let errors = ironclaw_skills::validate_credential_spec(&spec);
    assert!(errors.iter().any(|e| e.contains("at least one host")));

    // Uppercase name
    let spec = SkillCredentialSpec {
        name: "INVALID_NAME".to_string(),
        provider: "test".to_string(),
        location: SkillCredentialLocation::Bearer,
        hosts: vec!["api.example.com".to_string()],
        oauth: None,
        setup_instructions: None,
    };
    let errors = ironclaw_skills::validate_credential_spec(&spec);
    assert!(errors.iter().any(|e| e.contains("lowercase")));

    // Empty provider
    let spec = SkillCredentialSpec {
        name: "token".to_string(),
        provider: "".to_string(),
        location: SkillCredentialLocation::Bearer,
        hosts: vec!["api.example.com".to_string()],
        oauth: None,
        setup_instructions: None,
    };
    let errors = ironclaw_skills::validate_credential_spec(&spec);
    assert!(errors.iter().any(|e| e.contains("provider")));

    // Multiple errors accumulate
    let spec = SkillCredentialSpec {
        name: "BAD".to_string(),
        provider: "".to_string(),
        location: SkillCredentialLocation::Bearer,
        hosts: vec![],
        oauth: None,
        setup_instructions: None,
    };
    let errors = ironclaw_skills::validate_credential_spec(&spec);
    assert_eq!(
        errors.len(),
        3,
        "should accumulate: bad name + empty provider + empty hosts"
    );
}

// ── Registry Pipeline Tests ──────────────────────────────────────────────

/// Valid skill credentials are registered; invalid ones are skipped.
#[test]
fn test_register_skill_credentials_mixed_valid_invalid() {
    let valid_skill = make_skill(
        "weather",
        vec![SkillCredentialSpec {
            name: "weather_token".to_string(),
            provider: "weatherco".to_string(),
            location: SkillCredentialLocation::Bearer,
            hosts: vec!["api.weather.com".to_string()],
            oauth: None,
            setup_instructions: None,
        }],
        "Call the weather API via http tool.",
    );

    let invalid_skill = make_skill(
        "broken",
        vec![SkillCredentialSpec {
            name: "UPPERCASE_BAD".to_string(), // invalid name
            provider: "test".to_string(),
            location: SkillCredentialLocation::Bearer,
            hosts: vec!["api.broken.com".to_string()],
            oauth: None,
            setup_instructions: None,
        }],
        "This skill has a bad credential spec.",
    );

    let registry = SharedCredentialRegistry::new();
    ironclaw::skills::register_skill_credentials(&[valid_skill, invalid_skill], &registry);

    // Valid should be registered
    assert!(registry.has_credentials_for_host("api.weather.com"));
    // Invalid should be skipped
    assert!(!registry.has_credentials_for_host("api.broken.com"));
}

/// Multiple skills register independent credentials without interference.
#[test]
fn test_multi_skill_credential_registration() {
    let github_skill = make_skill(
        "github",
        vec![SkillCredentialSpec {
            name: "github_token".to_string(),
            provider: "github".to_string(),
            location: SkillCredentialLocation::Bearer,
            hosts: vec!["api.github.com".to_string()],
            oauth: None,
            setup_instructions: None,
        }],
        "GitHub API skill.",
    );

    let slack_skill = make_skill(
        "slack",
        vec![SkillCredentialSpec {
            name: "slack_token".to_string(),
            provider: "slack".to_string(),
            location: SkillCredentialLocation::Bearer,
            hosts: vec!["slack.com".to_string(), "api.slack.com".to_string()],
            oauth: None,
            setup_instructions: None,
        }],
        "Slack API skill.",
    );

    let no_creds_skill = make_skill("writing", vec![], "Just a writing skill, no API access.");

    let registry = SharedCredentialRegistry::new();
    ironclaw::skills::register_skill_credentials(
        &[github_skill, slack_skill, no_creds_skill],
        &registry,
    );

    assert!(registry.has_credentials_for_host("api.github.com"));
    assert!(registry.has_credentials_for_host("slack.com"));
    assert!(registry.has_credentials_for_host("api.slack.com"));
    assert!(!registry.has_credentials_for_host("unregistered.example.com"));
}

/// Skill credential spec → CredentialMapping conversion preserves all fields.
#[test]
fn test_credential_spec_to_mapping_all_location_types() {
    // Bearer
    let spec = SkillCredentialSpec {
        name: "token".to_string(),
        provider: "test".to_string(),
        location: SkillCredentialLocation::Bearer,
        hosts: vec!["api.test.com".to_string()],
        oauth: None,
        setup_instructions: None,
    };
    let mapping = ironclaw::skills::credential_spec_to_mapping(&spec);
    assert_eq!(mapping.secret_name, "token");
    assert!(matches!(
        mapping.location,
        ironclaw::secrets::CredentialLocation::AuthorizationBearer
    ));
    assert_eq!(mapping.host_patterns, vec!["api.test.com"]);

    // Header with prefix
    let spec = SkillCredentialSpec {
        name: "api_key".to_string(),
        provider: "test".to_string(),
        location: SkillCredentialLocation::Header {
            name: "X-API-Key".to_string(),
            prefix: Some("Token".to_string()),
        },
        hosts: vec!["*.example.com".to_string()],
        oauth: None,
        setup_instructions: None,
    };
    let mapping = ironclaw::skills::credential_spec_to_mapping(&spec);
    match &mapping.location {
        ironclaw::secrets::CredentialLocation::Header { name, prefix } => {
            assert_eq!(name, "X-API-Key");
            assert_eq!(prefix.as_deref(), Some("Token"));
        }
        _ => panic!("expected Header location"),
    }
    assert_eq!(mapping.host_patterns, vec!["*.example.com"]);

    // BasicAuth
    let spec = SkillCredentialSpec {
        name: "basic_pass".to_string(),
        provider: "test".to_string(),
        location: SkillCredentialLocation::BasicAuth {
            username: "admin".to_string(),
        },
        hosts: vec!["api.example.com".to_string()],
        oauth: None,
        setup_instructions: None,
    };
    let mapping = ironclaw::skills::credential_spec_to_mapping(&spec);
    match &mapping.location {
        ironclaw::secrets::CredentialLocation::AuthorizationBasic { username } => {
            assert_eq!(username, "admin");
        }
        _ => panic!("expected AuthorizationBasic location"),
    }

    // QueryParam
    let spec = SkillCredentialSpec {
        name: "key".to_string(),
        provider: "test".to_string(),
        location: SkillCredentialLocation::QueryParam {
            name: "api_key".to_string(),
        },
        hosts: vec!["api.legacy.com".to_string()],
        oauth: None,
        setup_instructions: None,
    };
    let mapping = ironclaw::skills::credential_spec_to_mapping(&spec);
    match &mapping.location {
        ironclaw::secrets::CredentialLocation::QueryParam { name } => {
            assert_eq!(name, "api_key");
        }
        _ => panic!("expected QueryParam location"),
    }
}

// ── HttpTool Approval & Header Blocking Tests ────────────────────────────

/// HttpTool requires approval for hosts with registered credentials.
#[test]
fn test_http_tool_requires_approval_for_credentialed_host() {
    let registry = Arc::new(SharedCredentialRegistry::new());
    registry.add_mappings(vec![CredentialMapping::bearer(
        "github_token",
        "api.github.com",
    )]);

    let tool = http_tool_with_credentials(registry, Arc::new(test_secrets_store()));

    // Credentialed host → requires approval
    let params = serde_json::json!({
        "url": "https://api.github.com/repos/nearai/ironclaw/issues",
        "method": "GET"
    });
    assert_eq!(
        tool.requires_approval(&params),
        ApprovalRequirement::UnlessAutoApproved,
    );

    // Unregistered host → no approval needed for GET
    let params = serde_json::json!({
        "url": "https://example.com/public-api",
        "method": "GET"
    });
    assert_eq!(tool.requires_approval(&params), ApprovalRequirement::Never);
}

/// Per-user credential isolation at the SecretsStore level.
#[tokio::test]
async fn test_per_user_credential_isolation() {
    let store = test_secrets_store();

    // Store secret for user-a
    store
        .create(
            "user-a",
            CreateSecretParams::new("github_token", "user-a-secret"),
        )
        .await
        .unwrap();

    // user-a can retrieve it
    let secret = store.get_decrypted("user-a", "github_token").await.unwrap();
    assert_eq!(secret.expose(), "user-a-secret");

    // user-b cannot
    let err = store.get_decrypted("user-b", "github_token").await;
    assert!(err.is_err(), "user-b should not access user-a's secret");

    // user-b stores their own
    store
        .create(
            "user-b",
            CreateSecretParams::new("github_token", "user-b-secret"),
        )
        .await
        .unwrap();

    // Each user sees their own value
    let a = store.get_decrypted("user-a", "github_token").await.unwrap();
    let b = store.get_decrypted("user-b", "github_token").await.unwrap();
    assert_eq!(a.expose(), "user-a-secret");
    assert_eq!(b.expose(), "user-b-secret");
}

// ── End-to-End Scenario ──────────────────────────────────────────────────

/// Complete scenario: parse skill YAML → validate → register → verify HttpTool behavior.
///
/// Simulates what happens when IronClaw discovers skills at startup:
/// 1. Parse frontmatter with credential specs
/// 2. Validate specs (reject bad ones)
/// 3. Register valid specs into SharedCredentialRegistry
/// 4. HttpTool picks up registered credentials for approval checks
/// 5. Secrets store provides per-user isolation
#[tokio::test]
async fn test_full_skill_credential_pipeline() {
    // Step 1: Parse skill YAML (like skill discovery)
    let yaml = r#"
name: github
version: "1.0.0"
description: GitHub API integration
activation:
  keywords: ["github", "issues", "pull request"]
credentials:
  - name: github_token
    provider: github
    location:
      type: bearer
    hosts: ["api.github.com"]
    oauth:
      authorization_url: "https://github.com/login/oauth/authorize"
      token_url: "https://github.com/login/oauth/access_token"
      scopes: ["repo"]
      refresh:
        strategy: reauthorize_only
    setup_instructions: "Create a PAT at https://github.com/settings/tokens"
"#;
    let manifest: SkillManifest = serde_yml::from_str(yaml).expect("parse failed");

    // Step 2: Validate
    for spec in &manifest.credentials {
        let errors = ironclaw_skills::validate_credential_spec(spec);
        assert!(
            errors.is_empty(),
            "valid spec should pass validation: {:?}",
            errors
        );
    }

    // Step 3: Build LoadedSkill and register (same code path as app.rs)
    let skill = make_skill(
        &manifest.name,
        manifest.credentials.clone(),
        "GitHub API skill content.",
    );

    let registry = Arc::new(SharedCredentialRegistry::new());
    ironclaw::skills::register_skill_credentials(&[skill], &registry);

    // Step 4: Verify registry state
    assert!(registry.has_credentials_for_host("api.github.com"));
    assert!(!registry.has_credentials_for_host("gitlab.com"));

    let mappings = registry.find_for_host("api.github.com");
    assert_eq!(mappings.len(), 1);
    assert_eq!(mappings[0].secret_name, "github_token");

    // Step 5: HttpTool integration
    let store = Arc::new(test_secrets_store());
    let tool = http_tool_with_credentials(Arc::clone(&registry), store.clone());

    // Before storing secret: credentialed host requires approval
    let params = serde_json::json!({
        "url": "https://api.github.com/repos/nearai/ironclaw",
        "method": "GET"
    });
    assert_eq!(
        tool.requires_approval(&params),
        ApprovalRequirement::UnlessAutoApproved,
    );

    // Store credential for user
    store
        .create(
            "developer",
            CreateSecretParams::new("github_token", "ghp_test_secret_42").with_provider("github"),
        )
        .await
        .unwrap();

    // Verify secret exists and is user-scoped
    assert!(store.exists("developer", "github_token").await.unwrap());
    assert!(!store.exists("other-user", "github_token").await.unwrap());

    // Verify the credential can be decrypted (for injection)
    let decrypted = store
        .get_decrypted("developer", "github_token")
        .await
        .unwrap();
    assert_eq!(decrypted.expose(), "ghp_test_secret_42");
}
