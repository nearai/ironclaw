use crate::config::{LegalConfig, LegalHardeningProfile};

const SIDE_EFFECT_TOOLS: &[&str] = &[
    "http",
    "shell",
    "write_file",
    "apply_patch",
    "memory_write",
    "create_job",
    "cancel_job",
    "tool_install",
    "tool_activate",
    "tool_remove",
    "skill_install",
    "skill_remove",
    "routine_create",
    "routine_update",
    "routine_delete",
];

/// True when legal hardening is in max-lockdown mode.
pub fn is_max_lockdown(config: &LegalConfig) -> bool {
    config.enabled && config.hardening == LegalHardeningProfile::MaxLockdown
}

/// Whether a tool should always require explicit approval in legal max-lockdown mode.
pub fn requires_explicit_approval(config: &LegalConfig, tool_name: &str) -> bool {
    is_max_lockdown(config) && SIDE_EFFECT_TOOLS.contains(&tool_name)
}

/// Normalize a domain for allowlist comparisons.
pub fn normalize_domain(domain: &str) -> String {
    domain.trim().trim_end_matches('.').to_ascii_lowercase()
}

/// Check if a target host is allowed by legal network policy.
pub fn is_network_domain_allowed(config: &LegalConfig, host: &str) -> bool {
    if !config.enabled || !config.network.deny_by_default {
        return true;
    }

    let host = normalize_domain(host);
    if host.is_empty() {
        return false;
    }

    config.network.allowed_domains.iter().any(|raw| {
        let allowed = normalize_domain(raw);
        host == allowed || host.ends_with(&format!(".{allowed}"))
    })
}

/// Keep matter IDs filesystem-safe and deterministic.
pub fn sanitize_matter_id(matter_id: &str) -> String {
    matter_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

/// Basic heuristic for identifying non-trivial legal tasks.
pub fn is_non_trivial_request(content: &str) -> bool {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return false;
    }

    if trimmed.len() >= 32 {
        return true;
    }

    let words = trimmed.split_whitespace().count();
    if words > 5 {
        return true;
    }

    let lower = trimmed.to_ascii_lowercase();
    [
        "contract",
        "motion",
        "brief",
        "complaint",
        "citation",
        "deposition",
        "discovery",
        "research",
        "chronology",
        "evidence",
    ]
    .iter()
    .any(|k| lower.contains(k))
}

/// Heuristic citation check for generated responses.
pub fn response_has_citation_markers(response: &str) -> bool {
    let lower = response.to_ascii_lowercase();
    lower.contains("source:")
        || lower.contains("sources:")
        || lower.contains("citation:")
        || lower.contains("citations:")
        || lower.contains("[doc")
        || lower.contains("(doc")
        || lower.contains("[§")
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::config::{LegalConfig, LegalHardeningProfile, LegalNetworkConfig};

    fn legal_for_test() -> LegalConfig {
        LegalConfig {
            enabled: true,
            jurisdiction: "us-general".to_string(),
            hardening: LegalHardeningProfile::MaxLockdown,
            require_matter_context: true,
            citation_required: true,
            matter_root: "matters".to_string(),
            active_matter: Some("demo-matter".to_string()),
            privilege_guard: true,
            conflict_check_enabled: true,
            network: LegalNetworkConfig {
                deny_by_default: true,
                allowed_domains: vec!["example.com".to_string()],
            },
            audit: crate::config::LegalAuditConfig {
                enabled: true,
                path: "logs/legal_audit.jsonl".into(),
                hash_chain: true,
            },
            redaction: crate::config::LegalRedactionConfig {
                pii: true,
                phi: true,
                financial: true,
                government_id: true,
            },
        }
    }

    #[test]
    fn allowlist_domain_check_supports_suffixes() {
        let cfg = legal_for_test();
        assert!(is_network_domain_allowed(&cfg, "example.com"));
        assert!(is_network_domain_allowed(&cfg, "api.example.com"));
        assert!(!is_network_domain_allowed(&cfg, "example.org"));
    }

    #[test]
    fn citation_heuristic_detects_common_markers() {
        assert!(response_has_citation_markers("Source: Contract §2.1"));
        assert!(response_has_citation_markers("See [doc 4 page 2]"));
        assert!(!response_has_citation_markers(
            "This paragraph has no supporting references."
        ));
    }

    #[test]
    fn sanitize_matter_id_removes_unsafe_chars() {
        assert_eq!(sanitize_matter_id(" Acme v. Foo/2026 "), "acme-v--foo-2026");
    }
}
