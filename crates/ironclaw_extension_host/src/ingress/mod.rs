//! The generic channel ingress router (overview.md §5.3, implementation.md §8).
//!
//! One host router serves `/webhooks/extensions/{extension_id}/{route_suffix}`
//! for every active extension's declared channel ingress. The route table is
//! the active snapshot — resolution happens per request through
//! [`crate::SnapshotWatch`], so activations and removals take effect without
//! any HTTP-server rebuild.
//!
//! Per-request order (pinned by the router contract tests):
//! match → method / body-limit / rate-limit / deadline enforcement →
//! verification recipe execution (host-side, constant-time; signing secrets
//! never reach the adapter) → `ChannelAdapter::inbound` (pure, panic-isolated,
//! bounded input) → outcome handling (durable dedupe + admission commit
//! before any 2xx).
//!
//! This crate stays transport-neutral: the router consumes
//! [`IngressRequest`]/[`IngressResponse`] values and composition wraps it in
//! the host HTTP server's route mount.

mod router;
mod verifier;

pub use router::{
    ExtensionIngressRouter, ExtensionIngressRouterDeps, InboundAdmission, InboundAdmissionAck,
    InboundSink, InboundSinkError, IngressPortError, IngressRateLimitConfig, IngressRequest,
    IngressResponse, IngressRouterConfig, IngressSecretsPort, ReplyContextKey, ReplyContextStore,
    canonical_ingress_path,
};
pub use verifier::{
    IngressHeaders, MAX_VERIFICATION_CANDIDATES, VerificationCandidate, VerificationFailure,
    VerifiedInstallation, verify_recipe,
};
