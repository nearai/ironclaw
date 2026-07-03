//! Capability profile conformance scaffolding.
//!
//! This module compares host-defined profile contracts with extension-declared
//! claims. It is zero-behavior prep: it does not execute capabilities, load
//! manifests, or certify third-party providers.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};

use ironclaw_host_api::{
    CapabilityId, CapabilityProfileContract, CapabilityProfileId, CapabilityProfileOperationId,
    CapabilityProfileSchemaRef, HostApiError,
};

/// One operation an extension claims for a profile implementation.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CapabilityProfileClaimedOperation {
    id: CapabilityProfileOperationId,
    input_schema_ref: CapabilityProfileSchemaRef,
    output_schema_ref: CapabilityProfileSchemaRef,
}

impl CapabilityProfileClaimedOperation {
    pub fn new(
        id: CapabilityProfileOperationId,
        input_schema_ref: impl Into<String>,
        output_schema_ref: impl Into<String>,
    ) -> Result<Self, HostApiError> {
        Ok(Self {
            id,
            input_schema_ref: CapabilityProfileSchemaRef::new(input_schema_ref)?,
            output_schema_ref: CapabilityProfileSchemaRef::new(output_schema_ref)?,
        })
    }

    pub fn id(&self) -> &CapabilityProfileOperationId {
        &self.id
    }

    pub fn input_schema_ref(&self) -> &CapabilityProfileSchemaRef {
        &self.input_schema_ref
    }

    pub fn output_schema_ref(&self) -> &CapabilityProfileSchemaRef {
        &self.output_schema_ref
    }
}

/// Extension claim that one provider-prefixed capability implements a profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityProfileClaim {
    capability_id: CapabilityId,
    profile_id: CapabilityProfileId,
    operations: Vec<CapabilityProfileClaimedOperation>,
}

impl CapabilityProfileClaim {
    pub fn new(
        capability_id: CapabilityId,
        profile_id: CapabilityProfileId,
        operations: Vec<CapabilityProfileClaimedOperation>,
    ) -> Result<Self, HostApiError> {
        let mut seen = BTreeSet::new();
        for operation in &operations {
            if !seen.insert(operation.id.clone()) {
                return Err(HostApiError::InvariantViolation {
                    reason: format!(
                        "duplicate claimed capability profile operation {}",
                        operation.id
                    ),
                });
            }
        }
        Ok(Self {
            capability_id,
            profile_id,
            operations,
        })
    }

    pub fn capability_id(&self) -> &CapabilityId {
        &self.capability_id
    }

    pub fn profile_id(&self) -> &CapabilityProfileId {
        &self.profile_id
    }

    pub fn operations(&self) -> &[CapabilityProfileClaimedOperation] {
        &self.operations
    }

    pub fn evaluate_against(
        &self,
        contract: &CapabilityProfileContract,
    ) -> CapabilityProfileConformanceReport {
        evaluate_profile_conformance(contract, self)
    }
}

/// Finding kind from structural profile conformance checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CapabilityProfileConformanceFindingKind {
    ProfileIdMismatch,
    MissingRequiredOperation,
    UnexpectedOperation,
    InputSchemaRefMismatch,
    OutputSchemaRefMismatch,
}

/// One structural profile conformance finding.
///
/// `subject` holds the identifier the finding is about. For
/// [`CapabilityProfileConformanceFindingKind::ProfileIdMismatch`] this is the
/// profile id; for every other variant it is the operation id.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CapabilityProfileConformanceFinding {
    kind: CapabilityProfileConformanceFindingKind,
    subject: String,
}

impl CapabilityProfileConformanceFinding {
    pub fn new(kind: CapabilityProfileConformanceFindingKind, subject: impl Into<String>) -> Self {
        Self {
            kind,
            subject: subject.into(),
        }
    }

    pub fn kind(&self) -> CapabilityProfileConformanceFindingKind {
        self.kind
    }

    pub fn subject(&self) -> &str {
        &self.subject
    }
}

impl fmt::Display for CapabilityProfileConformanceFinding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self.kind {
            CapabilityProfileConformanceFindingKind::ProfileIdMismatch => "profile id mismatch",
            CapabilityProfileConformanceFindingKind::MissingRequiredOperation => {
                "missing required operation"
            }
            CapabilityProfileConformanceFindingKind::UnexpectedOperation => "unexpected operation",
            CapabilityProfileConformanceFindingKind::InputSchemaRefMismatch => {
                "input schema ref mismatch"
            }
            CapabilityProfileConformanceFindingKind::OutputSchemaRefMismatch => {
                "output schema ref mismatch"
            }
        };
        write!(f, "{message}: {}", self.subject)
    }
}

impl std::error::Error for CapabilityProfileConformanceFinding {}

/// Evaluate structural conformance for one claim against one contract.
pub fn evaluate_profile_conformance(
    contract: &CapabilityProfileContract,
    claim: &CapabilityProfileClaim,
) -> CapabilityProfileConformanceReport {
    CapabilityProfileConformanceReport::evaluate(contract, claim)
}

/// Structural conformance report for one claim against one contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityProfileConformanceReport {
    findings: Vec<CapabilityProfileConformanceFinding>,
}

impl CapabilityProfileConformanceReport {
    pub fn evaluate(contract: &CapabilityProfileContract, claim: &CapabilityProfileClaim) -> Self {
        let mut findings = Vec::new();

        if contract.id() != claim.profile_id() {
            findings.push(CapabilityProfileConformanceFinding::new(
                CapabilityProfileConformanceFindingKind::ProfileIdMismatch,
                claim.profile_id().as_str(),
            ));
        }

        let claimed = claim
            .operations()
            .iter()
            .map(|operation| (operation.id().clone(), operation))
            .collect::<BTreeMap<_, _>>();
        let required = contract
            .required_operations()
            .iter()
            .map(|operation| (operation.id().clone(), operation))
            .collect::<BTreeMap<_, _>>();

        for (operation_id, required_operation) in &required {
            let Some(claimed_operation) = claimed.get(operation_id) else {
                findings.push(CapabilityProfileConformanceFinding::new(
                    CapabilityProfileConformanceFindingKind::MissingRequiredOperation,
                    operation_id.as_str(),
                ));
                continue;
            };

            if required_operation.input_schema_ref() != claimed_operation.input_schema_ref() {
                findings.push(CapabilityProfileConformanceFinding::new(
                    CapabilityProfileConformanceFindingKind::InputSchemaRefMismatch,
                    operation_id.as_str(),
                ));
            }
            if required_operation.output_schema_ref() != claimed_operation.output_schema_ref() {
                findings.push(CapabilityProfileConformanceFinding::new(
                    CapabilityProfileConformanceFindingKind::OutputSchemaRefMismatch,
                    operation_id.as_str(),
                ));
            }
        }

        for operation_id in claimed.keys() {
            if !required.contains_key(operation_id) {
                findings.push(CapabilityProfileConformanceFinding::new(
                    CapabilityProfileConformanceFindingKind::UnexpectedOperation,
                    operation_id.as_str(),
                ));
            }
        }

        Self { findings }
    }

    pub fn is_conformant(&self) -> bool {
        self.findings.is_empty()
    }

    pub fn findings(&self) -> &[CapabilityProfileConformanceFinding] {
        &self.findings
    }
}
