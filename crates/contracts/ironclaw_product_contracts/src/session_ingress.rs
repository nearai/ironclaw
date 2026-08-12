//! Session-channel ingress directory.
//!
//! The generic session-inbound route is parameterized by `extension_id`; the
//! product surface must confirm the named extension actually declares an
//! authenticated-session channel entrypoint before admitting a submission
//! under its identity. The directory is that confirmation — derived from the
//! deployment's resolved channel manifests, implemented by the extension
//! host, and consulted fail-closed (an unknown or non-session extension is a
//! 404, indistinguishable from an absent route).

/// Session-surface identity for submissions that name no channel extension —
/// the OpenAI-compatible API transports, which cannot learn a channel id from
/// `GET /session` (`ProductSubmitTurnRequest::extension_id` documents the
/// lane). Browser ingress never resolves to this: the SPA submits through the
/// `{extension_id}`-parameterized route with the server-advertised session
/// channel id, and this id is deliberately not resolvable through that route.
/// The spelling is a persisted coordinate (pre-existing session threads were
/// bound under it); do not rename it.
pub const BUILTIN_SESSION_SURFACE_ID: &str = "webui";

/// Deployment directory of authenticated-session channel entrypoints.
pub trait SessionChannelDirectory: Send + Sync {
    /// Whether `extension_id` names a deployment channel whose declared
    /// ingress is the authenticated-session entrypoint.
    fn is_session_channel(&self, extension_id: &str) -> bool;
}
