//! Slack installation identity vocabulary.
//!
//! The webhook-time installation resolver, envelope-metadata matching, and
//! per-installation rate limiter that used to live here were deleted in the
//! P4 generic-ingress cutover: the generic router resolves the installation
//! by verifying the recipe's signing secret, and per-extension rate limits
//! are host-owned. What remains are the typed Slack identifiers and the
//! installation selector the personal-binding flow still scopes bindings
//! with (deleted with `composition/src/slack/**` in P6).

macro_rules! slack_id_type {
    ($name:ident) => {
        #[derive(Debug, Clone, PartialEq, Eq, Hash)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl std::ops::Deref for $name {
            type Target = str;

            fn deref(&self) -> &Self::Target {
                self.as_str()
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str(self.as_str())
            }
        }
    };
}

slack_id_type!(SlackTeamId);
slack_id_type!(SlackEnterpriseId);
slack_id_type!(SlackApiAppId);
slack_id_type!(SlackUserId);
slack_id_type!(SlackChannelId);

/// Which Slack workspace/app/user an installation is scoped to. Consumed by
/// the personal-binding flow's tenant-app scoping checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlackInstallationSelector {
    Team {
        team_id: SlackTeamId,
    },
    AppTeam {
        api_app_id: SlackApiAppId,
        team_id: SlackTeamId,
    },
    AppEnterpriseTeam {
        api_app_id: SlackApiAppId,
        enterprise_id: SlackEnterpriseId,
        team_id: SlackTeamId,
    },
    EnterpriseTeam {
        enterprise_id: SlackEnterpriseId,
        team_id: SlackTeamId,
    },
    InstallUser {
        team_id: SlackTeamId,
        install_user_id: SlackUserId,
    },
    EnterpriseInstallUser {
        enterprise_id: SlackEnterpriseId,
        team_id: SlackTeamId,
        install_user_id: SlackUserId,
    },
    AppInstallUser {
        api_app_id: SlackApiAppId,
        team_id: SlackTeamId,
        install_user_id: SlackUserId,
    },
    AppEnterpriseInstallUser {
        api_app_id: SlackApiAppId,
        enterprise_id: SlackEnterpriseId,
        team_id: SlackTeamId,
        install_user_id: SlackUserId,
    },
}

impl SlackInstallationSelector {
    pub fn team(team_id: impl Into<String>) -> Self {
        Self::Team {
            team_id: SlackTeamId::new(team_id),
        }
    }

    pub fn app_team(api_app_id: impl Into<String>, team_id: impl Into<String>) -> Self {
        Self::AppTeam {
            api_app_id: SlackApiAppId::new(api_app_id),
            team_id: SlackTeamId::new(team_id),
        }
    }

    pub fn app_enterprise_team(
        api_app_id: impl Into<String>,
        enterprise_id: impl Into<String>,
        team_id: impl Into<String>,
    ) -> Self {
        Self::AppEnterpriseTeam {
            api_app_id: SlackApiAppId::new(api_app_id),
            enterprise_id: SlackEnterpriseId::new(enterprise_id),
            team_id: SlackTeamId::new(team_id),
        }
    }

    pub fn enterprise_team(enterprise_id: impl Into<String>, team_id: impl Into<String>) -> Self {
        Self::EnterpriseTeam {
            enterprise_id: SlackEnterpriseId::new(enterprise_id),
            team_id: SlackTeamId::new(team_id),
        }
    }

    pub fn with_install_user_id(self, install_user_id: impl Into<String>) -> Self {
        match self {
            Self::Team { team_id } => Self::InstallUser {
                team_id,
                install_user_id: SlackUserId::new(install_user_id),
            },
            Self::AppTeam {
                api_app_id,
                team_id,
            } => Self::AppInstallUser {
                api_app_id,
                team_id,
                install_user_id: SlackUserId::new(install_user_id),
            },
            Self::AppEnterpriseTeam {
                api_app_id,
                enterprise_id,
                team_id,
            } => Self::AppEnterpriseInstallUser {
                api_app_id,
                enterprise_id,
                team_id,
                install_user_id: SlackUserId::new(install_user_id),
            },
            Self::EnterpriseTeam {
                enterprise_id,
                team_id,
            } => Self::EnterpriseInstallUser {
                enterprise_id,
                team_id,
                install_user_id: SlackUserId::new(install_user_id),
            },
            Self::InstallUser { team_id, .. } => Self::InstallUser {
                team_id,
                install_user_id: SlackUserId::new(install_user_id),
            },
            Self::EnterpriseInstallUser {
                enterprise_id,
                team_id,
                ..
            } => Self::EnterpriseInstallUser {
                enterprise_id,
                team_id,
                install_user_id: SlackUserId::new(install_user_id),
            },
            Self::AppInstallUser {
                api_app_id,
                team_id,
                ..
            } => Self::AppInstallUser {
                api_app_id,
                team_id,
                install_user_id: SlackUserId::new(install_user_id),
            },
            Self::AppEnterpriseInstallUser {
                api_app_id,
                enterprise_id,
                team_id,
                ..
            } => Self::AppEnterpriseInstallUser {
                api_app_id,
                enterprise_id,
                team_id,
                install_user_id: SlackUserId::new(install_user_id),
            },
        }
    }
}
