//! Slice-C kernel vocabulary — the "render from record" result contract.
//!
//! Part of the capability-path result collapse
//! (`docs/reborn/2026-07-17-architecture-simplification-dto-dyn-local.md`
//! §5.2.9). [`Resolution`](crate::Resolution)'s control-plane arms carry only
//! *opaque* refs — [`GateRef`](crate::GateRef), [`DenyRef`](crate::DenyRef),
//! [`ResultRef`](crate::ResultRef) — never inline content. The records in this
//! module are the durably-stored, model-visible payloads those refs point at:
//! the loop renders a pending gate or a denial **from** the referenced record,
//! and never reconstructs it from data it already had in hand.
//!
//! ## The rendering contract (§5.2.9)
//!
//! - A [`Resolution::Denied(DenyRef)`](crate::Resolution::Denied) renders its
//!   model-visible denial from the [`DenyRecord`] keyed by that `DenyRef`.
//! - A [`Blocked`](crate::Blocked) / gate-shaped
//!   [`Suspension`](crate::Suspension) renders its pending-gate content from the
//!   [`GateRecord`] keyed by that `GateRef`. The `GateRecord` is also where the
//!   resume/credential payload (the auth gate's
//!   [`RuntimeCredentialAuthRequirement`](crate::RuntimeCredentialAuthRequirement)s,
//!   G3) and the dependent-run staged result (G2) live — content that today
//!   rides inline on the old `CapabilityOutcome` variants.
//!
//! ## Redaction invariant
//!
//! These records are **model-visible** and therefore MUST already be redacted:
//! every one carries a [`SafeSummary`](crate::SafeSummary), never raw text. The
//! loop renders credential requirements FROM the [`GateRecord::Auth`] record —
//! it never reconstructs a credential demand from model-visible data
//! (`tool-evidence.md`, `safety-and-sandbox.md`). Keeping the requirement on the
//! host-owned record, not on any model-derived value, is what makes an auth gate
//! forge-proof.
//!
//! Introduced additively (§9): nothing produces or renders these yet — the
//! result-channel migration slice wires them alongside the `Resolution` umbrella.

use serde::{Deserialize, Serialize};

use crate::{DenyReason, ResultRef, RuntimeCredentialAuthRequirement, SafeSummary};

/// The model-visible denial content a [`Resolution::Denied(DenyRef)`] renders
/// from (§5.2.9). Keyed by [`DenyRef`](crate::DenyRef); the ref rides the
/// sanitized boundary, this record stays host-owned.
///
/// [`DenyReason`] is already a model-visible enum (`decision.rs`); the
/// [`SafeSummary`] is redacted by construction. A denial is terminal, so there
/// is no resume payload here — only what the model is allowed to see about why.
///
/// [`Resolution::Denied(DenyRef)`]: crate::Resolution::Denied
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DenyRecord {
    /// The structured, model-visible reason the action was denied.
    pub reason: DenyReason,
    /// A bounded, redacted, model-visible summary of the denial.
    pub summary: SafeSummary,
}

/// The content behind a pending gate — the record a gate ref renders from
/// (§5.2.9), one enum over the gate kinds. Keyed by
/// [`GateRef`](crate::GateRef).
///
/// This is where the resume/credential payload (G3) and the dependent-run staged
/// result (G2) live — the inline cargo that today rides the old
/// `CapabilityOutcome` gate variants. Every variant carries a redacted
/// [`SafeSummary`]; the [`Auth`](GateRecord::Auth) variant additionally carries
/// the host-owned [`RuntimeCredentialAuthRequirement`]s the loop renders the
/// credential prompt from — never reconstructed from model-visible data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GateRecord {
    /// Awaiting human approval before the call may run.
    Approval { summary: SafeSummary },
    /// Awaiting a credential the caller has not supplied. Carries the host-owned
    /// requirements the credential prompt is rendered from (G3).
    Auth {
        summary: SafeSummary,
        credential_requirements: Vec<RuntimeCredentialAuthRequirement>,
    },
    /// Awaiting resource budget currently unavailable.
    Resource { summary: SafeSummary },
    /// Awaiting a dependent child run; carries the staged result handle and its
    /// byte length (G2).
    DependentRun {
        summary: SafeSummary,
        result: ResultRef,
        byte_len: u64,
        /// The preserved originating loop result ref the staged result was
        /// keyed under (§5.3 Stage 1 non-lossy carry): `result` is a freshly
        /// minted uuid handle, so without this the child output the loop staged
        /// under its own ref would become unreachable from the durable record a
        /// later resume turn renders from. `None` on records persisted before
        /// this field existed (serde default keeps old rows rehydratable).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        result_origin: Option<crate::LoopRef>,
    },
    /// Awaiting a client-executed external tool the host does not run.
    ExternalTool { summary: SafeSummary },
}

impl GateRecord {
    /// The redacted, model-visible summary carried by every gate kind.
    pub fn summary(&self) -> &SafeSummary {
        match self {
            GateRecord::Approval { summary }
            | GateRecord::Auth { summary, .. }
            | GateRecord::Resource { summary }
            | GateRecord::DependentRun { summary, .. }
            | GateRecord::ExternalTool { summary } => summary,
        }
    }

    /// Stable discriminant (matches the serde tag) for logs/routing.
    pub fn kind(&self) -> &'static str {
        match self {
            GateRecord::Approval { .. } => "approval",
            GateRecord::Auth { .. } => "auth",
            GateRecord::Resource { .. } => "resource",
            GateRecord::DependentRun { .. } => "dependent_run",
            GateRecord::ExternalTool { .. } => "external_tool",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ExtensionId, RuntimeCredentialAccountProviderId, RuntimeCredentialAccountSetup};

    fn summary() -> SafeSummary {
        SafeSummary::new("awaiting decision").unwrap()
    }

    fn credential_requirement() -> RuntimeCredentialAuthRequirement {
        RuntimeCredentialAuthRequirement {
            provider: RuntimeCredentialAccountProviderId::new("github").unwrap(),
            setup: RuntimeCredentialAccountSetup::ManualToken,
            requester_extension: ExtensionId::new("github").unwrap(),
            provider_scopes: vec!["repo".to_string()],
        }
    }

    #[test]
    fn gate_record_roundtrips_snake_case_for_every_variant() {
        let result = ResultRef::parse("018f6a00-0000-7000-8000-000000000001").unwrap();
        let variants = [
            (GateRecord::Approval { summary: summary() }, "approval"),
            (
                GateRecord::Auth {
                    summary: summary(),
                    credential_requirements: vec![credential_requirement()],
                },
                "auth",
            ),
            (GateRecord::Resource { summary: summary() }, "resource"),
            (
                GateRecord::DependentRun {
                    summary: summary(),
                    result,
                    byte_len: 2048,
                    result_origin: Some(crate::LoopRef::new("result:child-1").unwrap()),
                },
                "dependent_run",
            ),
            (
                GateRecord::ExternalTool { summary: summary() },
                "external_tool",
            ),
        ];
        for (record, tag) in variants {
            let wire = serde_json::to_value(&record).unwrap();
            let tag_on_wire = wire.as_object().unwrap().keys().next().unwrap().clone();
            assert_eq!(record.kind(), tag, "kind() must match the serde tag");
            assert_eq!(tag_on_wire, tag, "wire tag must be snake_case");
            assert_eq!(record.summary(), &summary(), "summary() must be reachable");
            let back: GateRecord = serde_json::from_value(wire).unwrap();
            assert_eq!(
                back, record,
                "{tag}: round-trip must reconstruct the record"
            );
        }
    }

    #[test]
    fn auth_gate_record_carries_the_credential_requirements() {
        let record = GateRecord::Auth {
            summary: summary(),
            credential_requirements: vec![credential_requirement()],
        };
        // The host-owned requirement is rendered FROM the record, never
        // reconstructed from model-visible data.
        let back: GateRecord =
            serde_json::from_value(serde_json::to_value(&record).unwrap()).unwrap();
        match back {
            GateRecord::Auth {
                credential_requirements,
                ..
            } => {
                assert_eq!(credential_requirements.len(), 1);
                assert_eq!(credential_requirements[0], credential_requirement());
            }
            other => panic!("expected Auth, got {other:?}"),
        }
    }

    #[test]
    fn deny_record_roundtrips_carrying_a_deny_reason() {
        let record = DenyRecord {
            reason: DenyReason::PolicyDenied,
            summary: SafeSummary::new("blocked by policy").unwrap(),
        };
        let wire = serde_json::to_value(&record).unwrap();
        let back: DenyRecord = serde_json::from_value(wire).unwrap();
        assert_eq!(back, record);
        assert_eq!(back.reason, DenyReason::PolicyDenied);
    }
}
