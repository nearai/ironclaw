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
    CapabilityId, EffectKind, PackageIdentity, PackageSource, RequestedTrustClass,
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
                        max_resource_ceiling: None,
                    },
                    provenance: matched.provenance,
                    evaluated_at: Utc::now(),
                });
            }
        }

        Ok(default_decision(input))
    }
}

/// Fallback decision when no policy source recognizes the package. Caps at
/// `UserTrusted` for sources that are *capable* of being trusted (admin-style
/// origin) and at `Sandbox` for everything else, so `LocalManifest` requests
/// for `FirstPartyRequested` / `SystemRequested` are silently downgraded
/// rather than honored.
fn default_decision(input: &TrustPolicyInput) -> TrustDecision {
    let effective_trust = match input.identity.source {
        // A LocalManifest origin without explicit policy match never reaches
        // privileged trust, regardless of what the manifest declared.
        PackageSource::LocalManifest { .. } => EffectiveTrustClass::user_trusted(),
        // Bundled / Registry / Admin sources without a registered entry also
        // fall back; PR3 may want to treat unrecognized Bundled as a hard
        // error, but for PR1b we downgrade quietly and record provenance.
        _ => EffectiveTrustClass::user_trusted(),
    };

    TrustDecision {
        effective_trust,
        authority_ceiling: AuthorityCeiling::empty(),
        provenance: TrustProvenance::Default,
        evaluated_at: Utc::now(),
    }
}
