//! Core skill types.
//!
//! Contains the data structures for skill manifests, activation criteria,
//! trust levels, and loaded skills.

use std::collections::HashMap;
use std::path::PathBuf;

use regex::Regex;
use serde::{Deserialize, Serialize};

/// Maximum number of keywords allowed per skill to prevent scoring manipulation.
const MAX_KEYWORDS_PER_SKILL: usize = 20;

/// Maximum number of regex patterns allowed per skill.
const MAX_PATTERNS_PER_SKILL: usize = 5;

/// Maximum number of tags allowed per skill to prevent scoring manipulation.
const MAX_TAGS_PER_SKILL: usize = 10;

/// Minimum length for keywords and tags. Short tokens like "a" or "is"
/// match too broadly and can be used to game the scoring system.
const MIN_KEYWORD_TAG_LENGTH: usize = 3;

/// Maximum file size for SKILL.md (64 KiB).
pub const MAX_PROMPT_FILE_SIZE: u64 = 64 * 1024;

/// Trust state for a skill, determining its authority ceiling.
///
/// SAFETY: Variant ordering matters. `Ord` is derived from discriminant values
/// and the security model relies on `Installed < Trusted`. Do NOT reorder
/// variants or change discriminant values without auditing all `min()` /
/// comparison call-sites in attenuation code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillTrust {
    /// Registry/external skill. Read-only tools only.
    Installed = 0,
    /// User-placed skill (local or workspace). Full trust, all tools available.
    Trusted = 1,
}

impl std::fmt::Display for SkillTrust {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Installed => write!(f, "installed"),
            Self::Trusted => write!(f, "trusted"),
        }
    }
}

/// Where a skill was loaded from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillSource {
    /// Workspace skills directory (<workspace>/skills/).
    Workspace(PathBuf),
    /// User skills directory (~/.ironclaw/skills/).
    User(PathBuf),
    /// Bundled with the application.
    Bundled(PathBuf),
}

/// Activation criteria parsed from SKILL.md frontmatter `activation` section.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ActivationCriteria {
    /// Keywords that trigger this skill (exact and substring match).
    /// Capped at `MAX_KEYWORDS_PER_SKILL` during loading.
    #[serde(default)]
    pub keywords: Vec<String>,
    /// Keywords that veto this skill — if any match, score is 0 regardless of
    /// keyword/pattern matches. Prevents cross-skill interference.
    #[serde(default)]
    pub exclude_keywords: Vec<String>,
    /// Regex patterns for more complex matching.
    /// Capped at `MAX_PATTERNS_PER_SKILL` during loading.
    #[serde(default)]
    pub patterns: Vec<String>,
    /// Tags for broad category matching.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Maximum context tokens this skill's prompt should consume.
    #[serde(default = "default_max_context_tokens")]
    pub max_context_tokens: usize,
}

impl ActivationCriteria {
    /// Enforce limits on keywords, patterns, and tags to prevent scoring manipulation.
    ///
    /// Filters out short keywords/tags (< 3 chars) that match too broadly,
    /// then truncates to per-field caps.
    pub fn enforce_limits(&mut self) {
        self.keywords.retain(|k| k.len() >= MIN_KEYWORD_TAG_LENGTH);
        self.keywords.truncate(MAX_KEYWORDS_PER_SKILL);
        self.exclude_keywords
            .retain(|k| k.len() >= MIN_KEYWORD_TAG_LENGTH);
        self.exclude_keywords.truncate(MAX_KEYWORDS_PER_SKILL);
        self.patterns.truncate(MAX_PATTERNS_PER_SKILL);
        self.tags.retain(|t| t.len() >= MIN_KEYWORD_TAG_LENGTH);
        self.tags.truncate(MAX_TAGS_PER_SKILL);
    }
}

fn default_max_context_tokens() -> usize {
    2000
}

/// Parsed skill manifest from SKILL.md YAML frontmatter.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillManifest {
    /// Skill name (validated against SKILL_NAME_PATTERN).
    pub name: String,
    /// Skill version.
    #[serde(default = "default_version")]
    pub version: String,
    /// Short description of the skill.
    #[serde(default)]
    pub description: String,
    /// Activation criteria.
    #[serde(default)]
    pub activation: ActivationCriteria,
    /// Credential requirements for API access.
    /// Parsed at load time; values are never in the LLM context.
    #[serde(default)]
    pub credentials: Vec<SkillCredentialSpec>,
    /// Optional OpenClaw metadata.
    #[serde(default)]
    pub metadata: Option<SkillMetadata>,
}

fn default_version() -> String {
    "0.0.0".to_string()
}

/// Optional metadata section in SKILL.md frontmatter.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SkillMetadata {
    /// OpenClaw-specific metadata.
    #[serde(default)]
    pub openclaw: Option<OpenClawMeta>,
}

/// OpenClaw-specific metadata.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OpenClawMeta {
    /// Gating requirements that must be met for the skill to load.
    #[serde(default)]
    pub requires: GatingRequirements,
}

/// Requirements that must be satisfied for a skill to load.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GatingRequirements {
    /// Required binaries that must be on PATH.
    #[serde(default)]
    pub bins: Vec<String>,
    /// Required environment variables that must be set.
    #[serde(default)]
    pub env: Vec<String>,
    /// Required config file paths that must exist.
    #[serde(default)]
    pub config: Vec<String>,
    /// Required companion skills that should be installed alongside this one.
    ///
    /// Unlike bins/env/config, missing skill dependencies do NOT prevent the
    /// skill from loading — they produce warnings that the agent can surface
    /// to the user. This allows bundle/setup skills to declare which sub-skills
    /// they depend on (e.g., a `ceo-assistant` bundle requires `commitment-triage`,
    /// `commitment-digest`, `decision-capture`, etc.).
    #[serde(default)]
    pub skills: Vec<String>,
}

/// Where to inject a credential in HTTP requests.
///
/// Maps 1:1 to `CredentialLocation` in `src/secrets/types.rs` but is defined
/// here so that `ironclaw_skills` remains independent of the main crate.
/// Conversion happens at registration time in `src/skills/mod.rs`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SkillCredentialLocation {
    /// `Authorization: Bearer {secret}`
    Bearer,
    /// `Authorization: Basic base64(username:secret)`
    BasicAuth { username: String },
    /// Custom header, optionally prefixed (e.g. `X-API-Key: Token {secret}`)
    Header {
        name: String,
        #[serde(default)]
        prefix: Option<String>,
    },
    /// Query parameter (e.g. `?api_key={secret}`)
    QueryParam { name: String },
}

/// How the provider handles token refresh.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(tag = "strategy", rename_all = "snake_case")]
pub enum ProviderRefreshStrategy {
    /// Standard OAuth2 `refresh_token` grant.
    #[default]
    Standard,
    /// Provider does not support refresh — re-authorize when expired.
    ReauthorizeOnly,
    /// Provider-specific refresh endpoint or extra parameters.
    Custom {
        refresh_url: String,
        #[serde(default)]
        extra_params: HashMap<String, String>,
    },
}

/// OAuth configuration for a credential declared by a skill.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillOAuthConfig {
    pub authorization_url: String,
    pub token_url: String,
    #[serde(default)]
    pub scopes: Vec<String>,
    #[serde(default)]
    pub use_pkce: bool,
    #[serde(default)]
    pub extra_params: HashMap<String, String>,
    /// How this provider handles token refresh (default: standard OAuth2).
    #[serde(default)]
    pub refresh: ProviderRefreshStrategy,
    /// Optional endpoint to test the token after exchange (e.g. Google userinfo).
    #[serde(default)]
    pub test_url: Option<String>,
}

/// A credential requirement declared by a skill.
///
/// Skills declare credentials in YAML frontmatter so the system can register
/// host→credential mappings and manage OAuth flows without WASM modules.
/// Credential *values* are never in the LLM's context — only these metadata
/// specs are parsed at skill-load time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillCredentialSpec {
    /// Secret name in the `SecretsStore` (e.g. `google_oauth_token`).
    pub name: String,
    /// Provider hint (e.g. `google`, `github`, `slack`).
    pub provider: String,
    /// Where to inject the credential in HTTP requests.
    pub location: SkillCredentialLocation,
    /// Host patterns this credential applies to (glob syntax, e.g. `*.googleapis.com`).
    pub hosts: Vec<String>,
    /// Optional OAuth configuration for automated token exchange and refresh.
    #[serde(default)]
    pub oauth: Option<SkillOAuthConfig>,
    /// Human-readable setup instructions shown when the credential is missing.
    #[serde(default)]
    pub setup_instructions: Option<String>,
}

/// A fully loaded skill ready for activation.
#[derive(Debug, Clone)]
pub struct LoadedSkill {
    /// Parsed manifest from YAML frontmatter.
    pub manifest: SkillManifest,
    /// Raw prompt content (markdown body after frontmatter).
    pub prompt_content: String,
    /// Trust state (determined by source location).
    pub trust: SkillTrust,
    /// Where this skill was loaded from.
    pub source: SkillSource,
    /// SHA-256 hash of the prompt content (computed at load time).
    pub content_hash: String,
    /// Pre-compiled regex patterns from activation criteria (compiled at load time).
    pub compiled_patterns: Vec<Regex>,
    /// Pre-computed lowercased keywords for scoring (avoids per-message allocation).
    /// Derived from `manifest.activation.keywords` at load time — do not mutate independently.
    pub lowercased_keywords: Vec<String>,
    /// Pre-computed lowercased exclude keywords for veto scoring.
    /// Derived from `manifest.activation.exclude_keywords` at load time.
    pub lowercased_exclude_keywords: Vec<String>,
    /// Pre-computed lowercased tags for scoring (avoids per-message allocation).
    /// Derived from `manifest.activation.tags` at load time — do not mutate independently.
    pub lowercased_tags: Vec<String>,
}

impl LoadedSkill {
    /// Get the skill name.
    pub fn name(&self) -> &str {
        &self.manifest.name
    }

    /// Get the skill version.
    pub fn version(&self) -> &str {
        &self.manifest.version
    }

    /// Compile regex patterns from activation criteria. Invalid or oversized patterns
    /// are logged and skipped. A size limit of 64 KiB is imposed on compiled regex
    /// state to prevent ReDoS via pathological patterns.
    pub fn compile_patterns(patterns: &[String]) -> Vec<Regex> {
        /// Maximum compiled regex size (64 KiB) to prevent ReDoS.
        const MAX_REGEX_SIZE: usize = 1 << 16;

        patterns
            .iter()
            .filter_map(|p| {
                match regex::RegexBuilder::new(p)
                    .size_limit(MAX_REGEX_SIZE)
                    .build()
                {
                    Ok(re) => Some(re),
                    Err(e) => {
                        tracing::warn!("Invalid activation regex pattern '{}': {}", p, e);
                        None
                    }
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_skill_trust_ordering() {
        assert!(SkillTrust::Installed < SkillTrust::Trusted);
    }

    #[test]
    fn test_skill_trust_display() {
        assert_eq!(SkillTrust::Installed.to_string(), "installed");
        assert_eq!(SkillTrust::Trusted.to_string(), "trusted");
    }

    #[test]
    fn test_enforce_keyword_limits() {
        let mut criteria = ActivationCriteria {
            keywords: (0..30).map(|i| format!("kw{}", i)).collect(),
            patterns: (0..10).map(|i| format!("pat{}", i)).collect(),
            tags: (0..20).map(|i| format!("tag{}", i)).collect(),
            ..Default::default()
        };
        criteria.enforce_limits();
        assert_eq!(criteria.keywords.len(), MAX_KEYWORDS_PER_SKILL);
        assert_eq!(criteria.patterns.len(), MAX_PATTERNS_PER_SKILL);
        assert_eq!(criteria.tags.len(), MAX_TAGS_PER_SKILL);
    }

    #[test]
    fn test_enforce_limits_filters_short_keywords() {
        let mut criteria = ActivationCriteria {
            keywords: vec!["a".into(), "be".into(), "cat".into(), "dog".into()],
            tags: vec!["x".into(), "foo".into(), "ab".into(), "bar".into()],
            ..Default::default()
        };
        criteria.enforce_limits();
        assert_eq!(criteria.keywords, vec!["cat", "dog"]);
        assert_eq!(criteria.tags, vec!["foo", "bar"]);
    }

    #[test]
    fn test_activation_criteria_enforce_limits() {
        let mut keywords: Vec<String> = vec!["a".into(), "bb".into()];
        keywords.extend((0..25).map(|i| format!("keyword{}", i)));

        let patterns: Vec<String> = (0..8).map(|i| format!("pattern{}", i)).collect();

        let mut tags: Vec<String> = vec!["x".into(), "ab".into()];
        tags.extend((0..15).map(|i| format!("tag{}", i)));

        let mut criteria = ActivationCriteria {
            keywords,
            patterns,
            tags,
            ..Default::default()
        };

        criteria.enforce_limits();

        assert!(
            !criteria
                .keywords
                .iter()
                .any(|k| k.len() < MIN_KEYWORD_TAG_LENGTH),
            "keywords shorter than {} chars should be filtered out",
            MIN_KEYWORD_TAG_LENGTH
        );
        assert_eq!(
            criteria.keywords.len(),
            MAX_KEYWORDS_PER_SKILL,
            "keywords should be capped at {}",
            MAX_KEYWORDS_PER_SKILL
        );

        assert_eq!(
            criteria.patterns.len(),
            MAX_PATTERNS_PER_SKILL,
            "patterns should be capped at {}",
            MAX_PATTERNS_PER_SKILL
        );
        for i in 0..MAX_PATTERNS_PER_SKILL {
            assert_eq!(criteria.patterns[i], format!("pattern{}", i));
        }

        assert!(
            !criteria
                .tags
                .iter()
                .any(|t| t.len() < MIN_KEYWORD_TAG_LENGTH),
            "tags shorter than {} chars should be filtered out",
            MIN_KEYWORD_TAG_LENGTH
        );
        assert_eq!(
            criteria.tags.len(),
            MAX_TAGS_PER_SKILL,
            "tags should be capped at {}",
            MAX_TAGS_PER_SKILL
        );
    }

    #[test]
    fn test_compile_patterns() {
        let patterns = vec![
            r"(?i)\bwrite\b".to_string(),
            "[invalid".to_string(),
            r"(?i)\bedit\b".to_string(),
        ];
        let compiled = LoadedSkill::compile_patterns(&patterns);
        assert_eq!(compiled.len(), 2);
    }

    #[test]
    fn test_parse_skill_manifest_yaml() {
        let yaml = r#"
name: writing-assistant
version: "1.0.0"
description: Professional writing and editing
activation:
  keywords: ["write", "edit", "proofread"]
  patterns: ["(?i)\\b(write|draft)\\b.*\\b(email|letter)\\b"]
  max_context_tokens: 2000
"#;
        let manifest: SkillManifest = serde_yml::from_str(yaml).expect("parse failed");
        assert_eq!(manifest.name, "writing-assistant");
        assert_eq!(manifest.activation.keywords.len(), 3);
    }

    #[test]
    fn test_parse_openclaw_metadata() {
        let yaml = r#"
name: test-skill
metadata:
  openclaw:
    requires:
      bins: ["vale"]
      env: ["VALE_CONFIG"]
      config: ["/etc/vale.ini"]
"#;
        let manifest: SkillManifest = serde_yml::from_str(yaml).expect("parse failed");
        let meta = manifest.metadata.unwrap();
        let openclaw = meta.openclaw.unwrap();
        assert_eq!(openclaw.requires.bins, vec!["vale"]);
        assert_eq!(openclaw.requires.env, vec!["VALE_CONFIG"]);
        assert_eq!(openclaw.requires.config, vec!["/etc/vale.ini"]);
    }

    #[test]
    fn test_loaded_skill_name_version() {
        let skill = LoadedSkill {
            manifest: SkillManifest {
                name: "test".to_string(),
                version: "1.0.0".to_string(),
                description: String::new(),
                activation: ActivationCriteria::default(),
                credentials: vec![],
                metadata: None,
            },
            prompt_content: "test prompt".to_string(),
            trust: SkillTrust::Trusted,
            source: SkillSource::User(PathBuf::from("/tmp/test")),
            content_hash: "sha256:000".to_string(),
            compiled_patterns: vec![],
            lowercased_keywords: vec![],
            lowercased_exclude_keywords: vec![],
            lowercased_tags: vec![],
        };
        assert_eq!(skill.name(), "test");
        assert_eq!(skill.version(), "1.0.0");
    }

    #[test]
    fn test_parse_credentials_frontmatter() {
        let yaml = r#"
name: gmail
version: "1.0.0"
description: Gmail API integration
activation:
  keywords: ["email", "gmail"]
credentials:
  - name: google_oauth_token
    provider: google
    location:
      type: bearer
    hosts: ["gmail.googleapis.com"]
    oauth:
      authorization_url: "https://accounts.google.com/o/oauth2/v2/auth"
      token_url: "https://oauth2.googleapis.com/token"
      scopes: ["https://www.googleapis.com/auth/gmail.modify"]
      test_url: "https://www.googleapis.com/oauth2/v1/userinfo"
"#;
        let manifest: SkillManifest = serde_yml::from_str(yaml).expect("parse failed");
        assert_eq!(manifest.credentials.len(), 1);
        let cred = &manifest.credentials[0];
        assert_eq!(cred.name, "google_oauth_token");
        assert_eq!(cred.provider, "google");
        assert!(matches!(cred.location, SkillCredentialLocation::Bearer));
        assert_eq!(cred.hosts, vec!["gmail.googleapis.com"]);
        let oauth = cred.oauth.as_ref().unwrap();
        assert_eq!(
            oauth.authorization_url,
            "https://accounts.google.com/o/oauth2/v2/auth"
        );
        assert_eq!(oauth.scopes.len(), 1);
        assert_eq!(
            oauth.test_url.as_deref(),
            Some("https://www.googleapis.com/oauth2/v1/userinfo")
        );
        assert!(matches!(oauth.refresh, ProviderRefreshStrategy::Standard));
    }

    #[test]
    fn test_parse_credentials_header_location() {
        let yaml = r#"
name: custom-api
credentials:
  - name: api_key
    provider: custom
    location:
      type: header
      name: X-API-Key
      prefix: "Token"
    hosts: ["api.custom.com"]
"#;
        let manifest: SkillManifest = serde_yml::from_str(yaml).expect("parse failed");
        let cred = &manifest.credentials[0];
        match &cred.location {
            SkillCredentialLocation::Header { name, prefix } => {
                assert_eq!(name, "X-API-Key");
                assert_eq!(prefix.as_deref(), Some("Token"));
            }
            other => panic!("expected Header, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_credentials_query_param_location() {
        let yaml = r#"
name: legacy-api
credentials:
  - name: api_key
    provider: legacy
    location:
      type: query_param
      name: access_token
    hosts: ["api.legacy.com"]
"#;
        let manifest: SkillManifest = serde_yml::from_str(yaml).expect("parse failed");
        let cred = &manifest.credentials[0];
        match &cred.location {
            SkillCredentialLocation::QueryParam { name } => {
                assert_eq!(name, "access_token");
            }
            other => panic!("expected QueryParam, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_credentials_basic_auth() {
        let yaml = r#"
name: basic-api
credentials:
  - name: basic_cred
    provider: example
    location:
      type: basic_auth
      username: admin
    hosts: ["api.example.com"]
"#;
        let manifest: SkillManifest = serde_yml::from_str(yaml).expect("parse failed");
        let cred = &manifest.credentials[0];
        match &cred.location {
            SkillCredentialLocation::BasicAuth { username } => {
                assert_eq!(username, "admin");
            }
            other => panic!("expected BasicAuth, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_credentials_with_custom_refresh() {
        let yaml = r#"
name: slack
credentials:
  - name: slack_token
    provider: slack
    location:
      type: bearer
    hosts: ["slack.com"]
    oauth:
      authorization_url: "https://slack.com/oauth/v2/authorize"
      token_url: "https://slack.com/api/oauth.v2.access"
      scopes: ["chat:write"]
      refresh:
        strategy: custom
        refresh_url: "https://slack.com/api/oauth.v2.access"
        extra_params:
          grant_type: refresh_token
"#;
        let manifest: SkillManifest = serde_yml::from_str(yaml).expect("parse failed");
        let oauth = manifest.credentials[0].oauth.as_ref().unwrap();
        match &oauth.refresh {
            ProviderRefreshStrategy::Custom {
                refresh_url,
                extra_params,
            } => {
                assert_eq!(refresh_url, "https://slack.com/api/oauth.v2.access");
                assert_eq!(extra_params.get("grant_type").unwrap(), "refresh_token");
            }
            other => panic!("expected Custom, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_credentials_reauthorize_only() {
        let yaml = r#"
name: github
credentials:
  - name: github_token
    provider: github
    location:
      type: bearer
    hosts: ["api.github.com"]
    oauth:
      authorization_url: "https://github.com/login/oauth/authorize"
      token_url: "https://github.com/login/oauth/access_token"
      refresh:
        strategy: reauthorize_only
"#;
        let manifest: SkillManifest = serde_yml::from_str(yaml).expect("parse failed");
        let oauth = manifest.credentials[0].oauth.as_ref().unwrap();
        assert!(matches!(
            oauth.refresh,
            ProviderRefreshStrategy::ReauthorizeOnly
        ));
    }

    #[test]
    fn test_parse_manifest_without_credentials_defaults_empty() {
        let yaml = r#"
name: simple-skill
description: No credentials needed
"#;
        let manifest: SkillManifest = serde_yml::from_str(yaml).expect("parse failed");
        assert!(manifest.credentials.is_empty());
    }

    #[test]
    fn test_credential_spec_serde_roundtrip() {
        let spec = SkillCredentialSpec {
            name: "token".to_string(),
            provider: "github".to_string(),
            location: SkillCredentialLocation::Bearer,
            hosts: vec!["api.github.com".to_string()],
            oauth: None,
            setup_instructions: Some("Go to Settings > Tokens".to_string()),
        };
        let json = serde_json::to_string(&spec).unwrap();
        let back: SkillCredentialSpec = serde_json::from_str(&json).unwrap();
        assert_eq!(back.name, "token");
        assert_eq!(back.provider, "github");
        assert_eq!(back.hosts, vec!["api.github.com"]);
        assert_eq!(
            back.setup_instructions.as_deref(),
            Some("Go to Settings > Tokens")
        );
    }

    #[test]
    fn test_parse_credentials_with_extra_params() {
        let yaml = r#"
name: google-drive
credentials:
  - name: google_oauth_token
    provider: google
    location:
      type: bearer
    hosts: ["www.googleapis.com"]
    oauth:
      authorization_url: "https://accounts.google.com/o/oauth2/v2/auth"
      token_url: "https://oauth2.googleapis.com/token"
      scopes: ["https://www.googleapis.com/auth/drive"]
      use_pkce: true
      extra_params:
        access_type: offline
        prompt: consent
"#;
        let manifest: SkillManifest = serde_yml::from_str(yaml).expect("parse failed");
        let oauth = manifest.credentials[0].oauth.as_ref().unwrap();
        assert!(oauth.use_pkce);
        assert_eq!(oauth.extra_params.get("access_type").unwrap(), "offline");
        assert_eq!(oauth.extra_params.get("prompt").unwrap(), "consent");
    }
}
