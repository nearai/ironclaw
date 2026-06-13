use crate::{
    LifecycleExtensionCredentialSetup, LifecycleExtensionRuntimeKind, LifecycleExtensionSummary,
    LifecycleInstalledExtensionSummary, LifecyclePhase, LifecycleProductPayload,
    LifecycleProductResponse,
};

use super::extension_credentials::ExtensionCredentialReadiness;
use super::types::{RebornExtensionOnboardingPayload, RebornExtensionOnboardingState};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ExtensionOnboarding {
    pub(super) state: Option<RebornExtensionOnboardingState>,
    pub(super) onboarding: Option<RebornExtensionOnboardingPayload>,
    pub(super) instructions: Option<String>,
    pub(super) awaiting_token: Option<bool>,
}

impl ExtensionOnboarding {
    pub(super) fn empty() -> Self {
        Self {
            state: None,
            onboarding: None,
            instructions: None,
            awaiting_token: None,
        }
    }
}

pub(super) fn for_installed(extension: &LifecycleInstalledExtensionSummary) -> ExtensionOnboarding {
    for_summary(&extension.summary, extension.phase)
}

pub(super) fn for_installed_with_credential_status(
    extension: &LifecycleInstalledExtensionSummary,
    readiness: ExtensionCredentialReadiness,
) -> ExtensionOnboarding {
    if readiness == ExtensionCredentialReadiness::MissingRequired {
        return credential_onboarding(&extension.summary);
    }
    if readiness == ExtensionCredentialReadiness::Configured
        && matches!(
            extension.phase,
            LifecyclePhase::Installed | LifecyclePhase::Configured | LifecyclePhase::Failed
        )
    {
        let phase = if extension.phase == LifecyclePhase::Failed {
            LifecyclePhase::Failed
        } else {
            LifecyclePhase::Configured
        };
        return no_credential_onboarding(&extension.summary, phase);
    }
    for_installed(extension)
}

pub(super) fn from_lifecycle(lifecycle: &LifecycleProductResponse) -> ExtensionOnboarding {
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
    for_installed(extension)
}

fn for_summary(summary: &LifecycleExtensionSummary, phase: LifecyclePhase) -> ExtensionOnboarding {
    if phase == LifecyclePhase::Active {
        return ExtensionOnboarding::empty();
    }
    if phase == LifecyclePhase::Configured || summary.credential_requirements.is_empty() {
        return no_credential_onboarding(summary, phase);
    }
    credential_onboarding(summary)
}

fn credential_onboarding(summary: &LifecycleExtensionSummary) -> ExtensionOnboarding {
    let has_oauth = summary.credential_requirements.iter().any(|requirement| {
        matches!(
            requirement.setup,
            LifecycleExtensionCredentialSetup::OAuth { .. }
        )
    });
    let state = if has_oauth {
        RebornExtensionOnboardingState::AuthRequired
    } else {
        RebornExtensionOnboardingState::SetupRequired
    };
    let instructions = instructions(summary);
    let credential_instructions = summary
        .onboarding
        .as_ref()
        .and_then(|onboarding| onboarding.credential_instructions.clone())
        .unwrap_or_else(|| format!("Configure the credentials required by {}.", summary.name));
    let credential_next_step = credential_next_step(summary);
    ExtensionOnboarding {
        state: Some(state),
        onboarding: Some(RebornExtensionOnboardingPayload {
            credential_instructions: Some(credential_instructions),
            setup_url: setup_url(summary),
            credential_next_step: Some(credential_next_step),
        }),
        instructions: Some(instructions),
        awaiting_token: Some(!has_oauth),
    }
}

fn no_credential_onboarding(
    summary: &LifecycleExtensionSummary,
    phase: LifecyclePhase,
) -> ExtensionOnboarding {
    let state = match phase {
        LifecyclePhase::Installed | LifecyclePhase::Configured => {
            Some(RebornExtensionOnboardingState::Installed)
        }
        LifecyclePhase::Failed => Some(RebornExtensionOnboardingState::Failed),
        _ => None,
    };
    let instructions = if phase == LifecyclePhase::Configured {
        Some(activation_instructions(summary))
    } else if let Some(onboarding) = &summary.onboarding {
        Some(onboarding.instructions.clone())
    } else if matches!(
        summary.runtime_kind,
        LifecycleExtensionRuntimeKind::McpServer
    ) {
        Some(format!(
            "{} is installed. Activate it to make its MCP tools available.",
            summary.name
        ))
    } else if phase == LifecyclePhase::Installed {
        Some(format!(
            "{} is installed. Activate it to make its tools available.",
            summary.name
        ))
    } else {
        None
    };
    ExtensionOnboarding {
        state,
        onboarding: instructions
            .as_ref()
            .map(|instructions| RebornExtensionOnboardingPayload {
                credential_instructions: Some(instructions.clone()),
                setup_url: None,
                credential_next_step: Some(credential_next_step(summary)),
            }),
        instructions,
        awaiting_token: None,
    }
}

fn activation_instructions(summary: &LifecycleExtensionSummary) -> String {
    if matches!(
        summary.runtime_kind,
        LifecycleExtensionRuntimeKind::McpServer
    ) {
        format!(
            "{} is installed. Activate it to make its MCP tools available.",
            summary.name
        )
    } else {
        format!(
            "{} is installed. Activate it to make its tools available.",
            summary.name
        )
    }
}

fn instructions(summary: &LifecycleExtensionSummary) -> String {
    summary
        .onboarding
        .as_ref()
        .map(|onboarding| onboarding.instructions.clone())
        .unwrap_or_else(|| {
            format!(
                "{} needs configuration before its tools can run.",
                summary.name
            )
        })
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
                "After configuration completes, activate {} to publish its tools.",
                summary.name
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        LifecycleExtensionCredentialRequirement, LifecycleExtensionOnboarding,
        LifecycleExtensionSource, LifecyclePackageKind, LifecyclePackageRef,
    };

    #[test]
    fn github_manual_token_projects_setup_required_message() {
        let extension = installed_extension(
            "github",
            "GitHub",
            LifecyclePhase::Installed,
            vec![manual_requirement("github_runtime_token", "github")],
            LifecycleExtensionRuntimeKind::WasmTool,
            Some(LifecycleExtensionOnboarding {
                instructions: "GitHub needs a personal access token before its repository and pull request tools can run.".to_string(),
                credential_instructions: Some("Create a GitHub personal access token with the repository permissions you want IronClaw to use, then paste it here.".to_string()),
                setup_url: Some("https://github.com/settings/personal-access-tokens/new".to_string()),
                credential_next_step: Some("After saving the token, activate GitHub to publish its tools.".to_string()),
            }),
        );

        let onboarding = for_installed(&extension);

        assert_eq!(
            onboarding.state,
            Some(RebornExtensionOnboardingState::SetupRequired)
        );
        assert_eq!(onboarding.awaiting_token, Some(true));
        assert_eq!(
            onboarding.instructions.as_deref(),
            Some(
                "GitHub needs a personal access token before its repository and pull request tools can run."
            )
        );
        assert_eq!(
            onboarding
                .onboarding
                .expect("onboarding payload")
                .setup_url
                .as_deref(),
            Some("https://github.com/settings/personal-access-tokens/new")
        );
    }

    #[test]
    fn google_oauth_projects_auth_required_message() {
        let extension = installed_extension(
            "gmail",
            "Gmail",
            LifecyclePhase::Installed,
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
                    "After authorization completes, activate Gmail to publish its tools."
                        .to_string(),
                ),
            }),
        );

        let onboarding = for_installed(&extension);

        assert_eq!(
            onboarding.state,
            Some(RebornExtensionOnboardingState::AuthRequired)
        );
        assert_eq!(onboarding.awaiting_token, Some(false));
        assert_eq!(
            onboarding.instructions.as_deref(),
            Some("Gmail needs Google OAuth authorization before mail tools can run.")
        );
    }

    #[test]
    fn web_access_projects_activation_message_without_credentials() {
        let extension = installed_extension(
            "web-access",
            "Web Access",
            LifecyclePhase::Installed,
            Vec::new(),
            LifecycleExtensionRuntimeKind::FirstParty,
            Some(LifecycleExtensionOnboarding {
                instructions: "Web Access does not need credentials. Activate it to make web search and saved-result retrieval tools available.".to_string(),
                credential_instructions: Some("No credentials are required for Web Access.".to_string()),
                setup_url: None,
                credential_next_step: Some("Activate Web Access to publish its tools.".to_string()),
            }),
        );

        let onboarding = for_installed(&extension);

        assert_eq!(
            onboarding.state,
            Some(RebornExtensionOnboardingState::Installed)
        );
        assert_eq!(onboarding.awaiting_token, None);
        assert_eq!(
            onboarding.instructions.as_deref(),
            Some(
                "Web Access does not need credentials. Activate it to make web search and saved-result retrieval tools available."
            )
        );
    }

    #[test]
    fn configured_credentialed_extension_projects_activation_message() {
        let extension = installed_extension(
            "github",
            "GitHub",
            LifecyclePhase::Configured,
            vec![manual_requirement("github_runtime_token", "github")],
            LifecycleExtensionRuntimeKind::WasmTool,
            Some(LifecycleExtensionOnboarding {
                instructions: "GitHub needs a personal access token before its repository and pull request tools can run.".to_string(),
                credential_instructions: Some("Create a GitHub personal access token with the repository permissions you want IronClaw to use, then paste it here.".to_string()),
                setup_url: Some("https://github.com/settings/personal-access-tokens/new".to_string()),
                credential_next_step: Some("After saving the token, activate GitHub to publish its tools.".to_string()),
            }),
        );

        let onboarding = for_installed(&extension);

        assert_eq!(
            onboarding.state,
            Some(RebornExtensionOnboardingState::Installed)
        );
        assert_eq!(onboarding.awaiting_token, None);
        assert_eq!(
            onboarding.instructions.as_deref(),
            Some("GitHub is installed. Activate it to make its tools available.")
        );
    }

    #[test]
    fn credential_ready_installed_extension_projects_activation_message() {
        let extension = installed_extension(
            "gmail",
            "Gmail",
            LifecyclePhase::Installed,
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
                    "After authorization completes, activate Gmail to publish its tools."
                        .to_string(),
                ),
            }),
        );

        let onboarding = for_installed_with_credential_status(
            &extension,
            ExtensionCredentialReadiness::Configured,
        );

        assert_eq!(
            onboarding.state,
            Some(RebornExtensionOnboardingState::Installed)
        );
        assert_eq!(onboarding.awaiting_token, None);
        assert_eq!(
            onboarding.instructions.as_deref(),
            Some("Gmail is installed. Activate it to make its tools available.")
        );
    }

    #[test]
    fn credential_ready_failed_extension_preserves_failed_state() {
        let extension = installed_extension(
            "gmail",
            "Gmail",
            LifecyclePhase::Failed,
            vec![oauth_requirement("gmail_account", "google")],
            LifecycleExtensionRuntimeKind::FirstParty,
            Some(LifecycleExtensionOnboarding {
                instructions: "Gmail activation failed.".to_string(),
                credential_instructions: Some(
                    "Authorize the Google account that IronClaw should use for Gmail.".to_string(),
                ),
                setup_url: None,
                credential_next_step: Some(
                    "After authorization completes, activate Gmail to publish its tools."
                        .to_string(),
                ),
            }),
        );

        let onboarding = for_installed_with_credential_status(
            &extension,
            ExtensionCredentialReadiness::Configured,
        );

        assert_eq!(
            onboarding.state,
            Some(RebornExtensionOnboardingState::Failed)
        );
        assert_eq!(onboarding.awaiting_token, None);
        assert_eq!(
            onboarding.instructions.as_deref(),
            Some("Gmail activation failed.")
        );
    }

    #[test]
    fn lifecycle_projection_uses_matching_package_ref() {
        let lifecycle = LifecycleProductResponse {
            package_ref: Some(
                LifecyclePackageRef::new(LifecyclePackageKind::Extension, "target")
                    .expect("valid package ref"),
            ),
            phase: LifecyclePhase::Installed,
            blockers: Vec::new(),
            message: None,
            payload: Some(LifecycleProductPayload::ExtensionList {
                extensions: vec![
                    installed_extension(
                        "other",
                        "Other",
                        LifecyclePhase::Installed,
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
                        LifecyclePhase::Installed,
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

        assert_eq!(onboarding.instructions.as_deref(), Some("Target message"));
    }

    fn installed_extension(
        package_id: &str,
        name: &str,
        phase: LifecyclePhase,
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
                visible_capability_ids: Vec::new(),
                visible_read_only_capability_ids: Vec::new(),
                credential_requirements,
                onboarding,
            },
            phase,
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
