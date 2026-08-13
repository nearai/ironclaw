//! The web-app reply-target binding codec.
//!
//! Every web-app target is a personal direct-message surface by
//! construction: the binding names exactly one user's own enrolled browsers,
//! so OAuth/auth prompts are admissible on it. Shared-conversation encoding
//! deliberately fails closed — the channel has no shared conversations.

use ironclaw_extension_contracts::external::ExternalConversationRef;
use ironclaw_extension_contracts::preference_target::{
    PreferenceTargetCodec, PreferenceTargetEncodeRequest,
};
use ironclaw_host_api::turn::ReplyTargetBindingRef;
use ironclaw_web_app::{decode_web_app_target_ref, encode_web_app_target_ref};

pub struct WebAppPreferenceTargetCodec;

impl PreferenceTargetCodec for WebAppPreferenceTargetCodec {
    fn conversation_for_target(
        &self,
        target: &ReplyTargetBindingRef,
    ) -> Option<ExternalConversationRef> {
        // Validate the grammar before echoing it as a conversation id.
        decode_web_app_target_ref(target.as_str())?;
        ExternalConversationRef::new(None, target.as_str(), None, None).ok()
    }

    fn is_personal_direct_message(&self, target: &ReplyTargetBindingRef) -> bool {
        decode_web_app_target_ref(target.as_str()).is_some()
    }

    fn direct_message_actor_for_target(&self, target: &ReplyTargetBindingRef) -> Option<String> {
        decode_web_app_target_ref(target.as_str()).map(|(_, user_id)| user_id.to_string())
    }

    fn encode_shared_conversation_target(
        &self,
        _request: PreferenceTargetEncodeRequest<'_>,
    ) -> Option<ReplyTargetBindingRef> {
        // Web push has no shared conversations; fail closed.
        None
    }

    fn encode_personal_direct_message_target(
        &self,
        request: PreferenceTargetEncodeRequest<'_>,
        external_actor_id: &str,
    ) -> Option<ReplyTargetBindingRef> {
        // The conversation id carries the full grammar; re-encode only when
        // it decodes and names the proven actor.
        let (tenant_id, user_id) =
            decode_web_app_target_ref(request.conversation.conversation_id())?;
        if user_id.to_string() != external_actor_id {
            return None;
        }
        encode_web_app_target_ref(&tenant_id, &user_id).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_host_api::ids::{TenantId, UserId};

    fn reference() -> ReplyTargetBindingRef {
        encode_web_app_target_ref(
            &TenantId::new("tenant1").expect("tenant"),
            &UserId::new("user1").expect("user"),
        )
        .expect("encode")
    }

    #[test]
    fn web_app_targets_are_personal_direct_messages() {
        let codec = WebAppPreferenceTargetCodec;
        let reference = reference();
        assert!(codec.is_personal_direct_message(&reference));
        assert_eq!(
            codec.direct_message_actor_for_target(&reference).as_deref(),
            Some("user1")
        );
        let conversation = codec
            .conversation_for_target(&reference)
            .expect("conversation");
        assert_eq!(conversation.conversation_id(), reference.as_str());
    }

    #[test]
    fn foreign_refs_are_refused() {
        let codec = WebAppPreferenceTargetCodec;
        let foreign = ReplyTargetBindingRef::new("slack/v1/team/chan").expect("ref");
        assert!(!codec.is_personal_direct_message(&foreign));
        assert!(codec.conversation_for_target(&foreign).is_none());
        assert!(codec.direct_message_actor_for_target(&foreign).is_none());
    }
}
