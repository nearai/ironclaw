//! The vendor-authored preference reply-target codec port.
//!
//! Stored outbound preferences persist a [`ReplyTargetBindingRef`] whose
//! grammar is authored by the channel's vendor integration (the channel
//! extension crate), so both halves live there. This port is the only place
//! vendor knowledge enters the generic outbound-target and triggered-delivery
//! paths — which is exactly why it is extension-tier contract vocabulary and
//! not product vocabulary: the implementors are the channel packages, and the
//! callers are the generic delivery paths above them.

use ironclaw_host_api::ids::{AgentId, ProjectId};
use ironclaw_host_api::product_adapter::{AdapterInstallationId, ExternalConversationRef};
use ironclaw_host_api::turn::ReplyTargetBindingRef;

/// Everything the host knows when encoding a preference reply-target
/// binding ref: the durable installation identity plus the external
/// conversation the target delivers to. Vendor codecs turn it into their
/// binding-ref grammar.
#[derive(Debug, Clone, Copy)]
pub struct PreferenceTargetEncodeRequest<'a> {
    pub installation_id: &'a AdapterInstallationId,
    pub agent_id: &'a AgentId,
    pub project_id: Option<&'a ProjectId>,
    pub conversation: &'a ExternalConversationRef,
}

/// Encodes and decodes stored preference reply-target bindings. The
/// binding-ref grammar is authored by the channel's vendor integration (the
/// channel extension crate), so both halves live there; this port is the
/// only place vendor knowledge enters the generic outbound-target and
/// triggered-delivery paths.
pub trait PreferenceTargetCodec: Send + Sync {
    /// Decode a preference binding ref into the conversation it targets.
    fn conversation_for_target(
        &self,
        target: &ReplyTargetBindingRef,
    ) -> Option<ExternalConversationRef>;

    /// Whether the binding ref targets a personal 1:1 direct message (the
    /// only surface OAuth authorization URLs may land on).
    fn is_personal_direct_message(&self, target: &ReplyTargetBindingRef) -> bool;

    /// The external actor id a personal direct-message binding ref carries
    /// (`None` for shared-conversation or malformed refs). The generic
    /// outbound-target provider matches this against the provisioned
    /// DM-target record so a tampered actor segment never resolves.
    fn direct_message_actor_for_target(&self, target: &ReplyTargetBindingRef) -> Option<String>;

    /// Encode a shared-conversation reply-target binding ref (the inverse of
    /// [`Self::conversation_for_target`] for shared conversations). `None`
    /// when the conversation cannot be encoded in the vendor grammar — the
    /// caller offers no target rather than a malformed one (fail closed).
    fn encode_shared_conversation_target(
        &self,
        request: PreferenceTargetEncodeRequest<'_>,
    ) -> Option<ReplyTargetBindingRef>;

    /// Encode a personal direct-message reply-target binding ref carrying
    /// the proven external actor — the only refs
    /// [`Self::is_personal_direct_message`] accepts. `None` when the
    /// conversation or actor cannot be encoded (fail closed).
    fn encode_personal_direct_message_target(
        &self,
        request: PreferenceTargetEncodeRequest<'_>,
        external_actor_id: &str,
    ) -> Option<ReplyTargetBindingRef>;
}
