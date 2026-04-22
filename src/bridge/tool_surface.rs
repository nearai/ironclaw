#![cfg_attr(not(test), allow(dead_code))]

use ironclaw_engine::CapabilityStatus;

/// How the subject can be invoked from the model/runtime boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InvocationMode {
    Direct,
    RoutedOnly,
}

/// Bridge-owned subject categories for surface placement policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SurfaceSubjectKind {
    BuiltinDirectTool,
    ExtensionDirectAction,
    EngineNativeDirectAction,
    Channel,
    LatentProviderAction,
    AvailableNotInstalledProviderEntry,
}

/// Pure input to the bridge-owned surface assignment policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SurfacePolicyInput {
    pub(crate) kind: SurfaceSubjectKind,
    pub(crate) status: CapabilityStatus,
    pub(crate) invocation_mode: InvocationMode,
    /// Direct actions may require approval at execution time, but that does
    /// not remove them from `available_actions`.
    pub(crate) approval_gated: bool,
    /// Engine-native direct actions also need a current callable lease before
    /// they belong in `available_actions`.
    pub(crate) leased_and_callable: bool,
}

/// Pure result describing which bridge surfaces should include the subject.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SurfaceAssignment {
    pub(crate) available_actions: bool,
    pub(crate) available_capabilities: bool,
}

impl SurfaceAssignment {
    const fn actions_only() -> Self {
        Self {
            available_actions: true,
            available_capabilities: false,
        }
    }

    const fn capabilities_only() -> Self {
        Self {
            available_actions: false,
            available_capabilities: true,
        }
    }

    const fn neither() -> Self {
        Self {
            available_actions: false,
            available_capabilities: false,
        }
    }
}

pub(crate) fn assign_surface(subject: SurfacePolicyInput) -> SurfaceAssignment {
    if matches!(subject.invocation_mode, InvocationMode::RoutedOnly) {
        return SurfaceAssignment::capabilities_only();
    }

    match subject.kind {
        SurfaceSubjectKind::LatentProviderAction
        | SurfaceSubjectKind::AvailableNotInstalledProviderEntry
        | SurfaceSubjectKind::Channel => SurfaceAssignment::capabilities_only(),
        SurfaceSubjectKind::BuiltinDirectTool | SurfaceSubjectKind::ExtensionDirectAction => {
            if is_direct_ready(subject.status) {
                let _approval_gated = subject.approval_gated;
                SurfaceAssignment::actions_only()
            } else {
                fallback_assignment(subject.status)
            }
        }
        SurfaceSubjectKind::EngineNativeDirectAction => {
            if is_direct_ready(subject.status) && subject.leased_and_callable {
                let _approval_gated = subject.approval_gated;
                SurfaceAssignment::actions_only()
            } else if is_direct_ready(subject.status) {
                SurfaceAssignment::capabilities_only()
            } else {
                fallback_assignment(subject.status)
            }
        }
    }
}

const fn is_direct_ready(status: CapabilityStatus) -> bool {
    matches!(
        status,
        CapabilityStatus::Ready | CapabilityStatus::ReadyScoped
    )
}

const fn fallback_assignment(status: CapabilityStatus) -> SurfaceAssignment {
    match status {
        CapabilityStatus::NeedsAuth
        | CapabilityStatus::NeedsSetup
        | CapabilityStatus::Inactive
        | CapabilityStatus::Latent
        | CapabilityStatus::Error
        | CapabilityStatus::AvailableNotInstalled => SurfaceAssignment::capabilities_only(),
        CapabilityStatus::Ready | CapabilityStatus::ReadyScoped => SurfaceAssignment::neither(),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        InvocationMode, SurfaceAssignment, SurfacePolicyInput, SurfaceSubjectKind, assign_surface,
    };
    use ironclaw_engine::CapabilityStatus;

    #[test]
    fn assigns_surface_matrix_rows() {
        struct Case {
            name: &'static str,
            subject: SurfacePolicyInput,
            expected: SurfaceAssignment,
        }

        let cases = [
            Case {
                name: "ready built-in direct tool",
                subject: SurfacePolicyInput {
                    kind: SurfaceSubjectKind::BuiltinDirectTool,
                    status: CapabilityStatus::Ready,
                    invocation_mode: InvocationMode::Direct,
                    approval_gated: false,
                    leased_and_callable: false,
                },
                expected: SurfaceAssignment::actions_only(),
            },
            Case {
                name: "approval-gated direct tool",
                subject: SurfacePolicyInput {
                    kind: SurfaceSubjectKind::BuiltinDirectTool,
                    status: CapabilityStatus::Ready,
                    invocation_mode: InvocationMode::Direct,
                    approval_gated: true,
                    leased_and_callable: false,
                },
                expected: SurfaceAssignment::actions_only(),
            },
            Case {
                name: "ready extension direct action",
                subject: SurfacePolicyInput {
                    kind: SurfaceSubjectKind::ExtensionDirectAction,
                    status: CapabilityStatus::Ready,
                    invocation_mode: InvocationMode::Direct,
                    approval_gated: false,
                    leased_and_callable: false,
                },
                expected: SurfaceAssignment::actions_only(),
            },
            Case {
                name: "needs-auth extension direct action",
                subject: SurfacePolicyInput {
                    kind: SurfaceSubjectKind::ExtensionDirectAction,
                    status: CapabilityStatus::NeedsAuth,
                    invocation_mode: InvocationMode::Direct,
                    approval_gated: false,
                    leased_and_callable: false,
                },
                expected: SurfaceAssignment::capabilities_only(),
            },
            Case {
                name: "needs-setup extension direct action",
                subject: SurfacePolicyInput {
                    kind: SurfaceSubjectKind::ExtensionDirectAction,
                    status: CapabilityStatus::NeedsSetup,
                    invocation_mode: InvocationMode::Direct,
                    approval_gated: false,
                    leased_and_callable: false,
                },
                expected: SurfaceAssignment::capabilities_only(),
            },
            Case {
                name: "inactive extension direct action",
                subject: SurfacePolicyInput {
                    kind: SurfaceSubjectKind::ExtensionDirectAction,
                    status: CapabilityStatus::Inactive,
                    invocation_mode: InvocationMode::Direct,
                    approval_gated: false,
                    leased_and_callable: false,
                },
                expected: SurfaceAssignment::capabilities_only(),
            },
            Case {
                name: "error extension direct action",
                subject: SurfacePolicyInput {
                    kind: SurfaceSubjectKind::ExtensionDirectAction,
                    status: CapabilityStatus::Error,
                    invocation_mode: InvocationMode::Direct,
                    approval_gated: false,
                    leased_and_callable: false,
                },
                expected: SurfaceAssignment::capabilities_only(),
            },
            Case {
                name: "latent provider action",
                subject: SurfacePolicyInput {
                    kind: SurfaceSubjectKind::LatentProviderAction,
                    status: CapabilityStatus::Latent,
                    invocation_mode: InvocationMode::Direct,
                    approval_gated: false,
                    leased_and_callable: false,
                },
                expected: SurfaceAssignment::capabilities_only(),
            },
            Case {
                name: "available-not-installed provider entry",
                subject: SurfacePolicyInput {
                    kind: SurfaceSubjectKind::AvailableNotInstalledProviderEntry,
                    status: CapabilityStatus::AvailableNotInstalled,
                    invocation_mode: InvocationMode::Direct,
                    approval_gated: false,
                    leased_and_callable: false,
                },
                expected: SurfaceAssignment::capabilities_only(),
            },
            Case {
                name: "routed-only channel",
                subject: SurfacePolicyInput {
                    kind: SurfaceSubjectKind::Channel,
                    status: CapabilityStatus::ReadyScoped,
                    invocation_mode: InvocationMode::RoutedOnly,
                    approval_gated: false,
                    leased_and_callable: false,
                },
                expected: SurfaceAssignment::capabilities_only(),
            },
            Case {
                name: "ready leased engine-native direct action",
                subject: SurfacePolicyInput {
                    kind: SurfaceSubjectKind::EngineNativeDirectAction,
                    status: CapabilityStatus::Ready,
                    invocation_mode: InvocationMode::Direct,
                    approval_gated: false,
                    leased_and_callable: true,
                },
                expected: SurfaceAssignment::actions_only(),
            },
            Case {
                name: "engine-native direct action without current callable lease",
                subject: SurfacePolicyInput {
                    kind: SurfaceSubjectKind::EngineNativeDirectAction,
                    status: CapabilityStatus::Ready,
                    invocation_mode: InvocationMode::Direct,
                    approval_gated: false,
                    leased_and_callable: false,
                },
                expected: SurfaceAssignment::capabilities_only(),
            },
        ];

        for case in cases {
            assert_eq!(assign_surface(case.subject), case.expected, "{}", case.name);
        }
    }

    #[test]
    fn approval_gated_direct_actions_still_land_in_available_actions() {
        let assignment = assign_surface(SurfacePolicyInput {
            kind: SurfaceSubjectKind::ExtensionDirectAction,
            status: CapabilityStatus::Ready,
            invocation_mode: InvocationMode::Direct,
            approval_gated: true,
            leased_and_callable: false,
        });

        assert!(assignment.available_actions);
        assert!(!assignment.available_capabilities);
    }
}
