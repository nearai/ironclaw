//! Canonical stored DM targets include the assigned line and recipient.
use crate::phone;
use ironclaw_extension_contracts::external::ExternalConversationRef;
use ironclaw_extension_contracts::preference_target::{
    PreferenceTargetCodec, PreferenceTargetEncodeRequest,
};
use ironclaw_host_api::turn::ReplyTargetBindingRef;

#[derive(Debug, Default)]
pub struct SendbluePreferenceTargetCodec;
impl PreferenceTargetCodec for SendbluePreferenceTargetCodec {
    fn conversation_for_target(
        &self,
        target: &ReplyTargetBindingRef,
    ) -> Option<ExternalConversationRef> {
        let (line, recipient) = target.as_str().strip_prefix("sendblue:")?.split_once(':')?;
        if !phone(line) || !phone(recipient) {
            return None;
        }
        ExternalConversationRef::new(Some(line), recipient, None, None).ok()
    }
    fn is_personal_direct_message(&self, target: &ReplyTargetBindingRef) -> bool {
        self.conversation_for_target(target).is_some()
    }
    fn direct_message_actor_for_target(&self, target: &ReplyTargetBindingRef) -> Option<String> {
        Some(
            self.conversation_for_target(target)?
                .conversation_id()
                .to_string(),
        )
    }
    fn encode_shared_conversation_target(
        &self,
        _request: PreferenceTargetEncodeRequest<'_>,
    ) -> Option<ReplyTargetBindingRef> {
        None
    }
    fn encode_personal_direct_message_target(
        &self,
        request: PreferenceTargetEncodeRequest<'_>,
        external_actor_id: &str,
    ) -> Option<ReplyTargetBindingRef> {
        let conversation = request.conversation;
        let line = conversation.space_id()?;
        let recipient = conversation.conversation_id();
        if !phone(line)
            || !phone(recipient)
            || recipient != external_actor_id
            || conversation.topic_id().is_some()
        {
            return None;
        }
        ReplyTargetBindingRef::new(format!("sendblue:{line}:{recipient}")).ok()
    }
}
