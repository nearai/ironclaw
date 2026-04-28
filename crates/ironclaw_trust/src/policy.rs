//! Trust policy evaluation surface.
//!
//! [`TrustPolicy`] turns an untrusted [`TrustPolicyInput`] (manifest identity +
//! requested trust + requested authority) into a host-controlled
//! [`TrustDecision`]. [`HostTrustPolicy`] is the default implementation: it
//! consults a list of [`PolicySource`]s in order; the first source that
//! recognizes the package identity assigns the effective trust. If no source
//! matches, the policy falls through to a non-privileged default.

use chrono::Utc;
use ironclaw_host_api::{
    CapabilityId, EffectKind, PackageIdentity, PackageSource, RequestedTrustClass, ResourceCeiling,
};

use crate::decision::{AuthorityCeiling, EffectiveTrustClass, TrustDecision, TrustProvenance};
use crate::error::TrustError;
use crate::sources::PolicySource;

/// Untrusted input to the policy engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustPolicyInput {
    pub identity: PackageIdentity,
    pub requested_trust: RequestedTrustClass,
    pub requested_authority: Vec<CapabilityId>,
}

/// The host trust policy contract.
pub trait TrustPolicy: Send + Sync {
    fn evaluate(&self, input: &TrustPolicyInput) -> Result<TrustDecision, TrustError>;
}

/// What a [`PolicySource`] says about a package.
///
/// `None` means "this source does not recognize the package" — the policy
/// engine moves on to the next source. `Some` is binding for that source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceMatch {
    pub effective_trust: EffectiveTrustClass,
    pub provenance: TrustProvenance,
    pub allowed_effects: Vec<EffectKind>,
    /// Optional resource ceiling forwarded from the matching source's entry
    /// onto the resulting `AuthorityCeiling`. `None` means the source
    /// imposes no extra resource cap.
    pub max_resource_ceiling: Option<ResourceCeiling>,
}

/// Default host-controlled policy. Composes layered sources in priority order;
/// the first source returning `Some` wins. No source ⇒ non-privileged default.
pub struct HostTrustPolicy {
    sources: Vec<Box<dyn PolicySource>>,
}

impl HostTrustPolicy {
    pub fn new(sources: Vec<Box<dyn PolicySource>>) -> Self {
        Self { sources }
    }

    pub fn empty() -> Self {
        Self {
            sources: Vec::new(),
        }
    }

    pub fn add_source(&mut self, source: Box<dyn PolicySource>) {
        self.sources.push(source);
    }
}

impl TrustPolicy for HostTrustPolicy {
    fn evaluate(&self, input: &TrustPolicyInput) -> Result<TrustDecision, TrustError> {
        for source in &self.sources {
            if let Some(matched) = source.evaluate(input)? {
                return Ok(TrustDecision {
                    effective_trust: matched.effective_trust,
                    authority_ceiling: AuthorityCeiling {
                        allowed_effects: matched.allowed_effects,
                        max_resource_ceiling: matched.max_resource_ceiling,
                    },
                    provenance: matched.provenance,
                    evaluated_at: Utc::now(),
                });
            }
        }

        Ok(default_decision(input))
    }
}

/// Fallback decision when no policy source recognizes the package.
///
/// `LocalManifest` origins drop all the way to `Sandbox`: nothing about a
/// user-controlled file should imply latent trust, so we treat an unmatched
/// local manifest the same as untrusted code.
///
/// Other origins (`Bundled`, `Registry`, `Admin`) cap at `UserTrusted` —
/// they are *capable* of being host-policy-blessed, but the operator hasn't
/// registered them yet. Honoring third-party authority (but no privileged
/// authority) is a defensible default for these origins; PR3 may upgrade
/// unrecognized `Bundled` to a hard error.
fn default_decision(input: &TrustPolicyInput) -> TrustDecision {
    let effective_trust = match input.identity.source {
        PackageSource::LocalManifest { .. } => EffectiveTrustClass::sandbox(),
        PackageSource::Bundled | PackageSource::Registry { .. } | PackageSource::Admin => {
            EffectiveTrustClass::user_trusted()
        }
    };

    TrustDecision {
        effective_trust,
        authority_ceiling: AuthorityCeiling::empty(),
        provenance: TrustProvenance::Default,
        evaluated_at: Utc::now(),
    }
}
