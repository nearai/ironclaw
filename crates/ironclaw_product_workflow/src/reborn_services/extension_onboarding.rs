use crate::{
    LifecycleExtensionRuntimeKind, LifecycleExtensionSummary, LifecycleInstalledExtensionSummary,
    LifecycleProductPayload, LifecycleProductResponse, LifecyclePublicState,
};

use super::extension_credentials::ExtensionCredentialReadiness;
use super::types::RebornExtensionOnboardingPayload;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ExtensionOnboarding {
    pub(super) onboarding: Option<RebornExtensionOnboardingPayload>,
}

impl ExtensionOnboarding {
    pub(super) fn empty() -> Self {
        Self { onboarding: None }
    }
}

pub(super) fn for_installed(extension: &LifecycleInstalledExtensionSummary) -> ExtensionOnboarding {
    for_summary(&extension.summary, extension.phase)
}

pub(super) fn for_installed_with_credential_status(
    extension: &LifecycleInstalledExtensionSummary,
    readiness: ExtensionCredentialReadiness,
    activation_failed: bool,
) -> ExtensionOnboarding {
    if activation_failed {
        return failed_onboarding(&extension.summary);
    }
    if readiness == ExtensionCredentialReadiness::MissingRequired {
        return credential_onboarding(&extension.summary);
    }
    if readiness == ExtensionCredentialReadiness::Configured
        && extension.phase == LifecyclePublicState::SetupNeeded
    {
        return automatic_setup_onboarding(&extension.summary);
    }
    for_installed(extension)
}

#[cfg(test)]
pub(super) fn from_lifecycle(lifecycle: &LifecycleProductResponse) -> ExtensionOnboarding {
    from_lifecycle_with_credential_status(lifecycle, ExtensionCredentialReadiness::Unknown, false)
}

pub(super) fn from_lifecycle_with_credential_status(
    lifecycle: &LifecycleProductResponse,
    readiness: ExtensionCredentialReadiness,
    activation_failed: bool,
) -> ExtensionOnboarding {
    let Some(LifecycleProductPayload::ExtensionList { extensions, .. }) = &lifecycle.payload else {
        return ExtensionOnboarding::empty();
    };
    let extension = lifecycle
        .package_ref
        .as_ref()
        .and_then(|package_ref| {
            extensions
                .iter()
                .find(|extension| &extension.summary.package_ref == package_ref)
        })
        .or_else(|| extensions.first());
    let Some(extension) = extension else {
        return ExtensionOnboarding::empty();
    };
    for_installed_with_credential_status(extension, readiness, activation_failed)
}

fn for_summary(
    summary: &LifecycleExtensionSummary,
    phase: LifecyclePublicState,
) -> ExtensionOnboarding {
    match phase {
        LifecyclePublicState::Active | LifecyclePublicState::Uninstalled => {
            ExtensionOnboarding::empty()
        }
        LifecyclePublicState::SetupNeeded if summary.credential_requirements.is_empty() => {
            automatic_setup_pending_onboarding(summary)
        }
        LifecyclePublicState::SetupNeeded => credential_onboarding(summary),
    }
}

fn credential_onboarding(summary: &LifecycleExtensionSummary) -> ExtensionOnboarding {
    let credential_instructions = summary
        .onboarding
        .as_ref()
        .and_then(|onboarding| onboarding.credential_instructions.clone())
        .unwrap_or_else(|| format!("Configure the credentials required by {}.", summary.name));
    let credential_next_step = credential_next_step(summary);
    ExtensionOnboarding {
        onboarding: Some(RebornExtensionOnboardingPayload {
            credential_instructions: Some(credential_instructions),
            setup_url: setup_url(summary),
            credential_next_step: Some(credential_next_step),
        }),
    }
}

fn automatic_setup_pending_onboarding(summary: &LifecycleExtensionSummary) -> ExtensionOnboarding {
    let instructions = if let Some(onboarding) = &summary.onboarding {
        Some(onboarding.instructions.clone())
    } else if matches!(
        summary.runtime_kind,
        LifecycleExtensionRuntimeKind::McpServer
    ) {
        Some(format!(
            "IronClaw is finishing {} setup automatically; its MCP tools will appear when ready.",
            summary.name
        ))
    } else {
        Some(format!(
            "IronClaw is finishing {} setup automatically; its tools will appear when ready.",
            summary.name
        ))
    };
    ExtensionOnboarding {
        onboarding: instructions.map(|instructions| RebornExtensionOnboardingPayload {
            credential_instructions: Some(instructions),
            setup_url: None,
            credential_next_step: Some(credential_next_step(summary)),
        }),
    }
}

fn automatic_setup_onboarding(summary: &LifecycleExtensionSummary) -> ExtensionOnboarding {
    let instructions = readiness_instructions(summary);
    ExtensionOnboarding {
        onboarding: Some(RebornExtensionOnboardingPayload {
            credential_instructions: Some(instructions),
            setup_url: None,
            credential_next_step: Some(credential_next_step(summary)),
        }),
    }
}

fn failed_onboarding(summary: &LifecycleExtensionSummary) -> ExtensionOnboarding {
    let instructions = summary
        .onboarding
        .as_ref()
        .map(|onboarding| onboarding.instructions.clone());
    ExtensionOnboarding {
        onboarding: instructions.map(|instructions| RebornExtensionOnboardingPayload {
            credential_instructions: Some(instructions),
            setup_url: None,
            credential_next_step: Some(credential_next_step(summary)),
        }),
    }
}

fn readiness_instructions(summary: &LifecycleExtensionSummary) -> String {
    if matches!(
        summary.runtime_kind,
        LifecycleExtensionRuntimeKind::McpServer
    ) {
        format!(
            "{} setup is complete. IronClaw publishes its MCP tools automatically; no separate activation action is required.",
            summary.name
        )
    } else {
        format!(
            "{} setup is complete. IronClaw publishes its tools automatically; no separate activation action is required.",
            summary.name
        )
    }
}

fn setup_url(summary: &LifecycleExtensionSummary) -> Option<String> {
    summary
        .onboarding
        .as_ref()
        .and_then(|onboarding| onboarding.setup_url.clone())
}

fn credential_next_step(summary: &LifecycleExtensionSummary) -> String {
    summary
        .onboarding
        .as_ref()
        .and_then(|onboarding| onboarding.credential_next_step.clone())
        .unwrap_or_else(|| {
            format!(
                "After configuration completes, IronClaw finishes {} installation automatically and publishes its tools.",
                summary.name
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        LifecycleExtensionCredentialRequirement, LifecycleExtensionCredentialSetup,
        LifecycleExtensionOnboarding, LifecycleExtensionSource, LifecyclePackageKind,
        LifecyclePackageRef,
    };

    #[test]
    fn github_manual_token_projects_setup_required_message() {
        let extension = installed_extension(
            "github",
            "GitHub",
            LifecyclePublicState::SetupNeeded,
            vec![manual_requirement("github_runtime_token", "github")],
            LifecycleExtensionRuntimeKind::WasmTool,
            Some(LifecycleExtensionOnboarding {
                instructions: "GitHub needs a personal access token before its repository and pull request tools can run.".to_string(),
                credential_instructions: Some("Create a GitHub personal access token with the repository permissions you want IronClaw to use, then paste it here.".to_string()),
                setup_url: Some("https://github.com/settings/personal-access-tokens/new".to_string()),
                credential_next_step: Some("After saving the token, IronClaw finishes GitHub installation automatically and publishes its tools.".to_string()),
            }),
        );

        let onboarding = for_installed(&extension);
        let payload = onboarding.onboarding.expect("onboarding payload");
        assert_eq!(
            payload.credential_instructions.as_deref(),
            Some(
                "Create a GitHub personal access token with the repository permissions you want IronClaw to use, then paste it here."
            )
        );
        assert_eq!(
            payload.setup_url.as_deref(),
            Some("https://github.com/settings/personal-access-tokens/new")
        );
    }

    #[test]
    fn google_oauth_projects_auth_required_message() {
        let extension = installed_extension(
            "gmail",
            "Gmail",
            LifecyclePublicState::SetupNeeded,
            vec![oauth_requirement("gmail_account", "google")],
            LifecycleExtensionRuntimeKind::FirstParty,
            Some(LifecycleExtensionOnboarding {
                instructions: "Gmail needs Google OAuth authorization before mail tools can run."
                    .to_string(),
                credential_instructions: Some(
                    "Authorize the Google account that IronClaw should use for Gmail.".to_string(),
                ),
                setup_url: None,
                credential_next_step: Some(
                    "After authorization completes, IronClaw finishes Gmail installation automatically and publishes its tools."
                        .to_string(),
                ),
            }),
        );

        let onboarding = for_installed(&extension);
        let payload = onboarding.onboarding.expect("onboarding payload");
        assert_eq!(
            payload.credential_instructions.as_deref(),
            Some("Authorize the Google account that IronClaw should use for Gmail.")
        );
    }

    #[test]
    fn web_access_projects_automatic_readiness_message_without_credentials() {
        let extension = installed_extension(
            "web-access",
            "Web Access",
            LifecyclePublicState::SetupNeeded,
            Vec::new(),
            LifecycleExtensionRuntimeKind::FirstParty,
            Some(LifecycleExtensionOnboarding {
                instructions: "Web Access does not need credentials and becomes active as soon as it is installed.".to_string(),
                credential_instructions: Some("No credentials are required for Web Access.".to_string()),
                setup_url: None,
                credential_next_step: Some("IronClaw publishes Web Access tools automatically during installation.".to_string()),
            }),
        );

        let onboarding = for_installed(&extension);
        let payload = onboarding.onboarding.expect("onboarding payload");
        assert_eq!(
            payload.credential_instructions.as_deref(),
            Some(
                "Web Access does not need credentials and becomes active as soon as it is installed."
            )
        );
    }

    #[test]
    fn configured_credentialed_extension_projects_automatic_readiness_message() {
        let extension = installed_extension(
            "github",
            "GitHub",
            LifecyclePublicState::SetupNeeded,
            vec![manual_requirement("github_runtime_token", "github")],
            LifecycleExtensionRuntimeKind::WasmTool,
            Some(LifecycleExtensionOnboarding {
                instructions: "GitHub needs a personal access token before its repository and pull request tools can run.".to_string(),
                credential_instructions: Some("Create a GitHub personal access token with the repository permissions you want IronClaw to use, then paste it here.".to_string()),
                setup_url: Some("https://github.com/settings/personal-access-tokens/new".to_string()),
                credential_next_step: Some("After saving the token, IronClaw finishes GitHub installation automatically and publishes its tools.".to_string()),
            }),
        );

        let onboarding = for_installed_with_credential_status(
            &extension,
            ExtensionCredentialReadiness::Configured,
            false,
        );

        let payload = onboarding.onboarding.expect("onboarding payload");
        assert_eq!(
            payload.credential_instructions.as_deref(),
            Some(
                "GitHub setup is complete. IronClaw publishes its tools automatically; no separate activation action is required."
            )
        );
    }

    #[test]
    fn credential_ready_installed_extension_projects_automatic_readiness_message() {
        let extension = installed_extension(
            "gmail",
            "Gmail",
            LifecyclePublicState::SetupNeeded,
            vec![oauth_requirement("gmail_account", "google")],
            LifecycleExtensionRuntimeKind::FirstParty,
            Some(LifecycleExtensionOnboarding {
                instructions: "Gmail needs Google OAuth authorization before mail tools can run."
                    .to_string(),
                credential_instructions: Some(
                    "Authorize the Google account that IronClaw should use for Gmail.".to_string(),
                ),
                setup_url: None,
                credential_next_step: Some(
                    "After authorization completes, IronClaw finishes Gmail installation automatically and publishes its tools."
                        .to_string(),
                ),
            }),
        );

        let onboarding = for_installed_with_credential_status(
            &extension,
            ExtensionCredentialReadiness::Configured,
            false,
        );

        let payload = onboarding.onboarding.expect("onboarding payload");
        assert_eq!(
            payload.credential_instructions.as_deref(),
            Some(
                "Gmail setup is complete. IronClaw publishes its tools automatically; no separate activation action is required."
            )
        );
    }

    #[test]
    fn credential_ready_failed_extension_preserves_failed_state() {
        let extension = installed_extension(
            "gmail",
            "Gmail",
            LifecyclePublicState::SetupNeeded,
            vec![oauth_requirement("gmail_account", "google")],
            LifecycleExtensionRuntimeKind::FirstParty,
            Some(LifecycleExtensionOnboarding {
                instructions: "Gmail setup failed.".to_string(),
                credential_instructions: Some(
                    "Authorize the Google account that IronClaw should use for Gmail.".to_string(),
                ),
                setup_url: None,
                credential_next_step: Some(
                    "After authorization completes, IronClaw finishes Gmail installation automatically and publishes its tools."
                        .to_string(),
                ),
            }),
        );

        let onboarding = for_installed_with_credential_status(
            &extension,
            ExtensionCredentialReadiness::Configured,
            true,
        );

        let payload = onboarding.onboarding.expect("onboarding payload");
        assert_eq!(
            payload.credential_instructions.as_deref(),
            Some("Gmail setup failed.")
        );
    }

    #[test]
    fn lifecycle_projection_uses_matching_package_ref() {
        let lifecycle = LifecycleProductResponse {
            package_ref: Some(
                LifecyclePackageRef::new(LifecyclePackageKind::Extension, "target")
                    .expect("valid package ref"),
            ),
            phase: LifecyclePublicState::SetupNeeded,
            blockers: Vec::new(),
            message: None,
            payload: Some(LifecycleProductPayload::ExtensionList {
                extensions: vec![
                    installed_extension(
                        "other",
                        "Other",
                        LifecyclePublicState::SetupNeeded,
                        Vec::new(),
                        LifecycleExtensionRuntimeKind::FirstParty,
                        Some(LifecycleExtensionOnboarding {
                            instructions: "Other message".to_string(),
                            credential_instructions: None,
                            setup_url: None,
                            credential_next_step: None,
                        }),
                    ),
                    installed_extension(
                        "target",
                        "Target",
                        LifecyclePublicState::SetupNeeded,
                        Vec::new(),
                        LifecycleExtensionRuntimeKind::FirstParty,
                        Some(LifecycleExtensionOnboarding {
                            instructions: "Target message".to_string(),
                            credential_instructions: None,
                            setup_url: None,
                            credential_next_step: None,
                        }),
                    ),
                ],
                count: 2,
            }),
        };

        let onboarding = from_lifecycle(&lifecycle);

        assert_eq!(
            onboarding
                .onboarding
                .expect("onboarding payload")
                .credential_instructions
                .as_deref(),
            Some("Target message")
        );
    }

    fn installed_extension(
        package_id: &str,
        name: &str,
        phase: LifecyclePublicState,
        credential_requirements: Vec<LifecycleExtensionCredentialRequirement>,
        runtime_kind: LifecycleExtensionRuntimeKind,
        onboarding: Option<LifecycleExtensionOnboarding>,
    ) -> LifecycleInstalledExtensionSummary {
        LifecycleInstalledExtensionSummary {
            summary: LifecycleExtensionSummary {
                package_ref: LifecyclePackageRef::new(LifecyclePackageKind::Extension, package_id)
                    .expect("valid package ref"),
                name: name.to_string(),
                version: "1.0.0".to_string(),
                description: "test extension".to_string(),
                source: LifecycleExtensionSource::HostBundled,
                runtime_kind,
                surface_kinds: Vec::new(),
                channel_directions: None,
                channel_connection: None,
                channel_presentation: None,
                visible_capability_ids: Vec::new(),
                visible_read_only_capability_ids: Vec::new(),
                credential_requirements,
                onboarding,
            },
            phase,
            install_scope: None,
        }
    }

    fn manual_requirement(name: &str, provider: &str) -> LifecycleExtensionCredentialRequirement {
        LifecycleExtensionCredentialRequirement {
            name: name.to_string(),
            provider: provider.to_string(),
            required: true,
            setup: LifecycleExtensionCredentialSetup::ManualToken,
        }
    }

    fn oauth_requirement(name: &str, provider: &str) -> LifecycleExtensionCredentialRequirement {
        LifecycleExtensionCredentialRequirement {
            name: name.to_string(),
            provider: provider.to_string(),
            required: true,
            setup: LifecycleExtensionCredentialSetup::OAuth {
                scopes: vec!["scope".to_string()],
            },
        }
    }
}
