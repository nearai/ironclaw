//! Telegram reply-target binding-ref grammar for proactive delivery.
//!
//! Reactive replies keep using the compact `tg:` grammar owned by the
//! protocol renderer. Stored preferences need one additional authority bit:
//! the proven Telegram actor for a personal DM. The generic outbound-target
//! registry compares that actor with its caller-scoped DM record before it
//! resolves a target, so this grammar must preserve it.

use ironclaw_product_adapters::{
    ExternalConversationRef, PreferenceTargetCodec, PreferenceTargetEncodeRequest,
};
use ironclaw_turns::ReplyTargetBindingRef;

const TELEGRAM_PREFERENCE_PREFIX: &str = "reply:telegram:";

pub struct TelegramPreferenceTargetCodec;

impl PreferenceTargetCodec for TelegramPreferenceTargetCodec {
    fn conversation_for_target(
        &self,
        target: &ReplyTargetBindingRef,
    ) -> Option<ExternalConversationRef> {
        let decoded = decode_target(target)?;
        ExternalConversationRef::new(None, decoded.chat_id, decoded.topic_id.as_deref(), None).ok()
    }

    fn is_personal_direct_message(&self, target: &ReplyTargetBindingRef) -> bool {
        decode_target(target).is_some_and(|decoded| decoded.actor_id.is_some())
    }

    fn direct_message_actor_for_target(&self, target: &ReplyTargetBindingRef) -> Option<String> {
        decode_target(target)?.actor_id
    }

    fn encode_shared_conversation_target(
        &self,
        request: PreferenceTargetEncodeRequest<'_>,
    ) -> Option<ReplyTargetBindingRef> {
        encode_target(request.conversation, None)
    }

    fn encode_personal_direct_message_target(
        &self,
        request: PreferenceTargetEncodeRequest<'_>,
        external_actor_id: &str,
    ) -> Option<ReplyTargetBindingRef> {
        external_actor_id.parse::<i64>().ok()?;
        encode_target(request.conversation, Some(external_actor_id))
    }
}

struct DecodedTarget {
    chat_id: String,
    topic_id: Option<String>,
    actor_id: Option<String>,
}

fn encode_target(
    conversation: &ExternalConversationRef,
    actor_id: Option<&str>,
) -> Option<ReplyTargetBindingRef> {
    if conversation.space_id().is_some() || conversation.reply_target_message_id().is_some() {
        return None;
    }
    let chat_id = conversation.conversation_id();
    chat_id.parse::<i64>().ok()?;
    let topic_id = conversation.topic_id().unwrap_or("_");
    if topic_id != "_" {
        topic_id.parse::<i64>().ok()?;
    }
    let actor_id = actor_id.unwrap_or("_");
    ReplyTargetBindingRef::new(format!(
        "{TELEGRAM_PREFERENCE_PREFIX}{chat_id}:{topic_id}:{actor_id}"
    ))
    .ok()
}

fn decode_target(target: &ReplyTargetBindingRef) -> Option<DecodedTarget> {
    let raw = target.as_str().strip_prefix(TELEGRAM_PREFERENCE_PREFIX)?;
    let mut segments = raw.split(':');
    let chat_id = segments.next()?;
    let topic_id = segments.next()?;
    let actor_id = segments.next()?;
    if segments.next().is_some() || chat_id.parse::<i64>().is_err() {
        return None;
    }
    let topic_id = match topic_id {
        "_" => None,
        value if value.parse::<i64>().is_ok() => Some(value.to_string()),
        _ => return None,
    };
    let actor_id = match actor_id {
        "_" => None,
        value if value.parse::<i64>().is_ok() => Some(value.to_string()),
        _ => return None,
    };
    Some(DecodedTarget {
        chat_id: chat_id.to_string(),
        topic_id,
        actor_id,
    })
}

#[cfg(test)]
mod tests {
    use ironclaw_host_api::AgentId;
    use ironclaw_product_adapters::{AdapterInstallationId, PreferenceTargetEncodeRequest};

    use super::*;

    fn request<'a>(
        installation_id: &'a AdapterInstallationId,
        agent_id: &'a AgentId,
        conversation: &'a ExternalConversationRef,
    ) -> PreferenceTargetEncodeRequest<'a> {
        PreferenceTargetEncodeRequest {
            installation_id,
            agent_id,
            project_id: None,
            conversation,
        }
    }

    #[test]
    fn personal_dm_round_trips_with_the_proven_actor() {
        let codec = TelegramPreferenceTargetCodec;
        let installation = AdapterInstallationId::new("telegram").expect("installation");
        let agent = AgentId::new("agent").expect("agent");
        let conversation =
            ExternalConversationRef::new(None, "-100123", Some("42"), None).expect("conversation");

        let target = codec
            .encode_personal_direct_message_target(
                request(&installation, &agent, &conversation),
                "987654",
            )
            .expect("personal target");

        assert!(codec.is_personal_direct_message(&target));
        assert_eq!(
            codec.direct_message_actor_for_target(&target).as_deref(),
            Some("987654")
        );
        assert_eq!(codec.conversation_for_target(&target), Some(conversation));
    }

    #[test]
    fn malformed_or_non_telegram_actor_targets_fail_closed() {
        let codec = TelegramPreferenceTargetCodec;
        let malformed =
            ReplyTargetBindingRef::new("reply:telegram:123:_:not-a-telegram-user".to_string())
                .expect("bounded ref");
        let reactive = ironclaw_telegram_v2_adapter::build_reply_target_binding(123, None, None);

        assert!(codec.conversation_for_target(&malformed).is_none());
        assert!(!codec.is_personal_direct_message(&malformed));
        assert!(codec.conversation_for_target(&reactive).is_none());
    }

    #[test]
    fn shared_conversation_round_trips_without_claiming_a_personal_actor() {
        let codec = TelegramPreferenceTargetCodec;
        let installation = AdapterInstallationId::new("telegram").expect("installation");
        let agent = AgentId::new("agent").expect("agent");
        let conversation =
            ExternalConversationRef::new(None, "-100456", None, None).expect("conversation");

        let target = codec
            .encode_shared_conversation_target(request(&installation, &agent, &conversation))
            .expect("shared target");

        assert!(!codec.is_personal_direct_message(&target));
        assert!(codec.direct_message_actor_for_target(&target).is_none());
        assert_eq!(codec.conversation_for_target(&target), Some(conversation));
    }

}
