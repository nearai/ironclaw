use std::path::{Path, PathBuf};

use ironclaw_extensions::{ExtensionManifest, ExtensionPackage, ExtensionRegistry, ManifestSource};
use ironclaw_host_api::{
    CapabilityId, EffectKind, ExtensionId, NetworkPolicy, NetworkScheme, NetworkTargetPattern,
    SecretHandle, VirtualPath,
};
use ironclaw_host_runtime::{default_host_api_contract_registry, default_host_port_catalog};

type GithubSupportResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

pub fn capability_ids() -> GithubSupportResult<Vec<CapabilityId>> {
    Ok(vec![
        CapabilityId::new("github.get_repo")?,
        CapabilityId::new("github.create_repo")?,
        CapabilityId::new("github.list_issues")?,
        CapabilityId::new("github.create_issue")?,
        CapabilityId::new("github.get_issue")?,
        CapabilityId::new("github.list_issue_comments")?,
        CapabilityId::new("github.create_issue_comment")?,
        CapabilityId::new("github.comment_issue")?,
        CapabilityId::new("github.list_pull_requests")?,
        CapabilityId::new("github.create_pull_request")?,
        CapabilityId::new("github.get_pull_request")?,
        CapabilityId::new("github.get_pull_request_files")?,
        CapabilityId::new("github.create_pr_review")?,
        CapabilityId::new("github.list_pull_request_comments")?,
        CapabilityId::new("github.reply_pull_request_comment")?,
        CapabilityId::new("github.get_pull_request_reviews")?,
        CapabilityId::new("github.get_combined_status")?,
        CapabilityId::new("github.merge_pull_request")?,
        CapabilityId::new("github.get_authenticated_user")?,
        CapabilityId::new("github.list_repos")?,
        CapabilityId::new("github.search_repositories")?,
        CapabilityId::new("github.search_code")?,
        CapabilityId::new("github.search_issues")?,
        CapabilityId::new("github.search_issues_pull_requests")?,
        CapabilityId::new("github.list_branches")?,
        CapabilityId::new("github.create_branch")?,
        CapabilityId::new("github.get_file_content")?,
        CapabilityId::new("github.create_or_update_file")?,
        CapabilityId::new("github.delete_file")?,
        CapabilityId::new("github.list_releases")?,
        CapabilityId::new("github.create_release")?,
        CapabilityId::new("github.trigger_workflow")?,
        CapabilityId::new("github.get_workflow_runs")?,
        CapabilityId::new("github.fork_repo")?,
        CapabilityId::new("github.handle_webhook")?,
    ])
}

pub fn effect_kinds() -> Vec<EffectKind> {
    vec![
        EffectKind::DispatchCapability,
        EffectKind::Network,
        EffectKind::UseSecret,
        EffectKind::ExternalWrite,
    ]
}

pub fn provider_id() -> GithubSupportResult<ExtensionId> {
    Ok(ExtensionId::new("github")?)
}

pub fn secret_handles() -> GithubSupportResult<Vec<SecretHandle>> {
    Ok(vec![SecretHandle::new("github_runtime_token")?])
}

pub fn api_policy() -> NetworkPolicy {
    NetworkPolicy {
        allowed_targets: vec![NetworkTargetPattern {
            scheme: Some(NetworkScheme::Https),
            host_pattern: "api.github.com".to_string(),
            port: None,
        }],
        deny_private_ip_ranges: true,
        max_egress_bytes: Some(10_000),
    }
}

pub fn extension_registry() -> GithubSupportResult<ExtensionRegistry> {
    let manifest = ExtensionManifest::parse_with_host_api_contracts(
        &std::fs::read_to_string(asset_root().join("manifest.toml"))?,
        ManifestSource::HostBundled,
        &default_host_port_catalog()?,
        &default_host_api_contract_registry()?,
    )?;
    let package =
        ExtensionPackage::from_manifest(manifest, VirtualPath::new("/system/extensions/github")?)?;
    let mut registry = ExtensionRegistry::new();
    registry.insert(package)?;
    Ok(registry)
}

pub fn asset_root() -> PathBuf {
    repo_root().join("crates/ironclaw_first_party_extensions/assets/github")
}

fn repo_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}
