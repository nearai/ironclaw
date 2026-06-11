//! Authorization decision contracts.
//!
//! [`Decision`] is the host-facing result of evaluating an action in context:
//! allow with required [`Obligation`]s, deny with a structured [`DenyReason`],
//! or require a user approval request. Allowing an action is not enough by
//! itself; runtime services must also satisfy attached obligations such as
//! resource reservation, audit events, output limits, secret injection, and
//! scoped mounts.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;

use crate::ApprovalRequest;
use crate::{
    CapabilityId, ExtensionId, HostApiError, MountView, NetworkPolicy, ResourceCeiling,
    ResourceReservationId, RuntimeCredentialAccountProviderId, RuntimeCredentialAccountSetup,
    SecretHandle,
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum Decision {
    Allow { obligations: Obligations },
    Deny { reason: DenyReason },
    RequireApproval { request: ApprovalRequest },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DenyReason {
    MissingGrant,
    InvalidPath,
    PathOutsideMount,
    UnknownCapability,
    UnknownSecret,
    NetworkDenied,
    BudgetDenied,
    ApprovalDenied,
    PolicyDenied,
    ResourceLimitExceeded,
    InternalInvariantViolation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum Obligation {
    AuditBefore,
    AuditAfter,
    RedactOutput,
    ReserveResources {
        reservation_id: ResourceReservationId,
    },
    UseScopedMounts {
        mounts: MountView,
    },
    InjectSecretOnce {
        handle: SecretHandle,
    },
    InjectCredentialAccountOnce {
        handle: SecretHandle,
        provider: RuntimeCredentialAccountProviderId,
        #[serde(default)]
        setup: RuntimeCredentialAccountSetup,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        provider_scopes: Vec<String>,
        requester_extension: ExtensionId,
    },
    FirstPartyCredentialStagedViaHostPort {
        capability_id: CapabilityId,
    },
    ApplyNetworkPolicy {
        policy: NetworkPolicy,
    },
    EnforceResourceCeiling {
        ceiling: ResourceCeiling,
    },
    EnforceOutputLimit {
        bytes: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeCredentialAuthRequirement {
    pub provider: RuntimeCredentialAccountProviderId,
    pub requester_extension: ExtensionId,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub provider_scopes: Vec<String>,
}

/// Canonical obligation evaluation classes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObligationKind {
    ReserveResources,
    UseScopedMounts,
    ApplyNetworkPolicy,
    InjectSecretOnce,
    InjectCredentialAccountOnce,
    FirstPartyCredentialStagedViaHostPort,
    AuditBefore,
    RedactOutput,
    EnforceResourceCeiling,
    EnforceOutputLimit,
    AuditAfter,
}

/// Canonical order runtime handlers must follow when satisfying allow obligations.
pub const OBLIGATION_EVALUATION_ORDER: &[ObligationKind] = &[
    ObligationKind::ReserveResources,
    ObligationKind::UseScopedMounts,
    ObligationKind::ApplyNetworkPolicy,
    ObligationKind::InjectSecretOnce,
    ObligationKind::InjectCredentialAccountOnce,
    ObligationKind::FirstPartyCredentialStagedViaHostPort,
    ObligationKind::AuditBefore,
    ObligationKind::RedactOutput,
    ObligationKind::EnforceResourceCeiling,
    ObligationKind::EnforceOutputLimit,
    ObligationKind::AuditAfter,
];

/// Validated, canonicalized obligation list for an allowed decision.
///
/// `Decision::Allow` uses this wrapper instead of a raw `Vec<Obligation>` so
/// callers cannot accidentally encode duplicate or conflicting obligations, and
/// runtime handlers observe one stable evaluation order. Exact duplicates and
/// same-kind conflicting obligations are rejected at construction/deserialization
/// time; policy code must collapse conflicts before producing an allow decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct Obligations(Vec<Obligation>);

impl Obligations {
    pub fn new(obligations: Vec<Obligation>) -> Result<Self, HostApiError> {
        for obligation in &obligations {
            if !OBLIGATION_EVALUATION_ORDER.contains(&obligation.kind()) {
                return Err(HostApiError::invariant(format!(
                    "{kind:?} is missing from OBLIGATION_EVALUATION_ORDER",
                    kind = obligation.kind()
                )));
            }
        }

        let mut normalized = Vec::with_capacity(obligations.len());
        for kind in OBLIGATION_EVALUATION_ORDER {
            let mut matching = obligations
                .iter()
                .filter(|obligation| obligation.kind() == *kind);
            if let Some(first) = matching.next() {
                if matches!(
                    *kind,
                    ObligationKind::InjectSecretOnce | ObligationKind::InjectCredentialAccountOnce
                ) {
                    normalize_multi_inject(first, matching, kind, &mut normalized)?;
                    continue;
                }
                if matching.next().is_some() {
                    return Err(HostApiError::invariant(format!(
                        "duplicate or conflicting {kind:?} obligations are not allowed"
                    )));
                }
                normalized.push(first.clone());
            }
        }
        Ok(Self(normalized))
    }

    pub fn empty() -> Self {
        Self(Vec::new())
    }

    pub fn as_slice(&self) -> &[Obligation] {
        &self.0
    }

    pub fn into_vec(self) -> Vec<Obligation> {
        self.0
    }
}

impl Default for Obligations {
    fn default() -> Self {
        Self::empty()
    }
}

/// Normalize obligations for multi-inject kinds (`InjectSecretOnce`, `InjectCredentialAccountOnce`).
///
/// Both kinds allow multiple obligations (one per injection slot) and share identical
/// deduplication logic: seed `seen_handles` from the first obligation, then verify no
/// duplicate handles appear among the rest. Only the variant pattern differs.
fn normalize_multi_inject<'a>(
    first: &'a Obligation,
    rest: impl Iterator<Item = &'a Obligation>,
    kind: &ObligationKind,
    normalized: &mut Vec<Obligation>,
) -> Result<(), HostApiError> {
    let mut seen_handles = HashSet::new();
    seen_handles.insert(extract_inject_handle(first, kind));
    normalized.push(first.clone());
    for obligation in rest {
        let handle = extract_inject_handle(obligation, kind);
        if !seen_handles.insert(handle) {
            return Err(HostApiError::invariant(format!(
                "duplicate {kind:?} obligations for the same handle are not allowed"
            )));
        }
        normalized.push(obligation.clone());
    }
    Ok(())
}

fn extract_inject_handle(obligation: &Obligation, kind: &ObligationKind) -> SecretHandle {
    match obligation {
        Obligation::InjectSecretOnce { handle } => handle.clone(),
        Obligation::InjectCredentialAccountOnce { handle, .. } => handle.clone(),
        _ => unreachable!("extract_inject_handle called for {kind:?}"),
    }
}

impl<'de> Deserialize<'de> for Obligations {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let obligations = Vec::<Obligation>::deserialize(deserializer)?;
        Self::new(obligations).map_err(serde::de::Error::custom)
    }
}

impl Obligation {
    pub fn kind(&self) -> ObligationKind {
        match self {
            Self::AuditBefore => ObligationKind::AuditBefore,
            Self::AuditAfter => ObligationKind::AuditAfter,
            Self::RedactOutput => ObligationKind::RedactOutput,
            Self::ReserveResources { .. } => ObligationKind::ReserveResources,
            Self::UseScopedMounts { .. } => ObligationKind::UseScopedMounts,
            Self::InjectSecretOnce { .. } => ObligationKind::InjectSecretOnce,
            Self::InjectCredentialAccountOnce { .. } => ObligationKind::InjectCredentialAccountOnce,
            Self::FirstPartyCredentialStagedViaHostPort { .. } => {
                ObligationKind::FirstPartyCredentialStagedViaHostPort
            }
            Self::ApplyNetworkPolicy { .. } => ObligationKind::ApplyNetworkPolicy,
            Self::EnforceResourceCeiling { .. } => ObligationKind::EnforceResourceCeiling,
            Self::EnforceOutputLimit { .. } => ObligationKind::EnforceOutputLimit,
        }
    }
}
