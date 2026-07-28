//! Telegram preference targets for generic outbound delivery.
//!
//! The protocol crate already owns Telegram's durable reply-target grammar:
//! `tg:<chat_id>:<topic_id|_>:<reply_message_id|_>`. This codec deliberately
//! reuses that format instead of introducing a second wire representation.
//! Preference targets are conversation-level, so a reply-message segment is
//! never accepted. Telegram private-chat ids are the positive user ids they
//! belong to; that protocol invariant supplies the personal-DM actor carried
//! by the ref. Group, supergroup, and channel chat ids are negative.

use ironclaw_product::{
    ExternalConversationRef, PreferenceTargetCodec, PreferenceTargetEncodeRequest,
};
use ironclaw_telegram_v2_adapter::{
    TelegramReplyTarget, build_reply_target_binding, parse_reply_target,
};
use ironclaw_turns::ReplyTargetBindingRef;

/// Telegram's vendor codec for stored outbound preference targets.
#[derive(Debug, Clone, Copy, Default)]
pub struct TelegramPreferenceTargetCodec;

impl PreferenceTargetCodec for TelegramPreferenceTargetCodec {
    fn conversation_for_target(
        &self,
        target: &ReplyTargetBindingRef,
    ) -> Option<ExternalConversationRef> {
        let decoded = decode_preference_target(target)?;
        let chat_id = decoded.chat_id.to_string();
        let topic_id = decoded.topic_id.map(|topic| topic.to_string());
        ExternalConversationRef::new(None::<&str>, chat_id, topic_id.as_deref(), None).ok()
    }

    fn is_personal_direct_message(&self, target: &ReplyTargetBindingRef) -> bool {
        decode_preference_target(target)
            .is_some_and(|decoded| decoded.chat_id > 0 && decoded.topic_id.is_none())
    }

    fn direct_message_actor_for_target(&self, target: &ReplyTargetBindingRef) -> Option<String> {
        let decoded = decode_preference_target(target)?;
        (decoded.chat_id > 0 && decoded.topic_id.is_none()).then(|| decoded.chat_id.to_string())
    }

    fn encode_shared_conversation_target(
        &self,
        request: PreferenceTargetEncodeRequest<'_>,
    ) -> Option<ReplyTargetBindingRef> {
        let (chat_id, topic_id) = conversation_target(request.conversation)?;
        (chat_id < 0).then(|| build_reply_target_binding(chat_id, topic_id, None))
    }

    fn encode_personal_direct_message_target(
        &self,
        request: PreferenceTargetEncodeRequest<'_>,
        external_actor_id: &str,
    ) -> Option<ReplyTargetBindingRef> {
        let (chat_id, topic_id) = conversation_target(request.conversation)?;
        let actor_id = parse_canonical_i64(external_actor_id)?;
        if chat_id <= 0 || actor_id != chat_id || topic_id.is_some() {
            return None;
        }
        Some(build_reply_target_binding(chat_id, None, None))
    }
}

fn conversation_target(conversation: &ExternalConversationRef) -> Option<(i64, Option<i64>)> {
    if conversation.space_id().is_some() {
        return None;
    }
    let chat_id = parse_canonical_i64(conversation.conversation_id())?;
    if chat_id == 0 {
        return None;
    }
    let topic_id = match conversation.topic_id() {
        Some(raw) => Some(parse_canonical_i64(raw).filter(|topic| *topic > 0)?),
        None => None,
    };
    Some((chat_id, topic_id))
}

fn decode_preference_target(target: &ReplyTargetBindingRef) -> Option<TelegramReplyTarget> {
    let decoded = parse_reply_target(target).ok()?;
    if decoded.chat_id == 0
        || decoded.topic_id.is_some_and(|topic| topic <= 0)
        || decoded.reply_message_id.is_some()
    {
        return None;
    }
    // The protocol parser accepts all `i64` spellings. Preference refs use
    // one canonical spelling so equivalent ids cannot acquire distinct
    // durable keys (`+42`, `042`, and `42`).
    if build_reply_target_binding(decoded.chat_id, decoded.topic_id, None) != *target {
        return None;
    }
    Some(decoded)
}

fn parse_canonical_i64(value: &str) -> Option<i64> {
    let parsed = value.parse::<i64>().ok()?;
    (parsed.to_string() == value).then_some(parsed)
}

#[cfg(test)]
mod tests {
    use ironclaw_host_api::{AgentId, ProjectId};
    use ironclaw_product::AdapterInstallationId;

    use super::*;

    fn identities() -> (AdapterInstallationId, AgentId, ProjectId) {
        (
            AdapterInstallationId::new("telegram-install").expect("installation"),
            AgentId::new("agent:telegram").expect("agent"),
            ProjectId::new("project:telegram").expect("project"),
        )
    }

    fn request<'a>(
        installation_id: &'a AdapterInstallationId,
        agent_id: &'a AgentId,
        project_id: &'a ProjectId,
        conversation: &'a ExternalConversationRef,
    ) -> PreferenceTargetEncodeRequest<'a> {
        PreferenceTargetEncodeRequest {
            installation_id,
            agent_id,
            project_id: Some(project_id),
            conversation,
        }
    }

    #[test]
    fn personal_dm_round_trips_canonical_binding_and_implied_actor() {
        let codec = TelegramPreferenceTargetCodec;
        let (installation_id, agent_id, project_id) = identities();
        let conversation =
            ExternalConversationRef::new(None::<&str>, "424242", None, None).expect("conversation");

        let target = codec
            .encode_personal_direct_message_target(
                request(&installation_id, &agent_id, &project_id, &conversation),
                "424242",
            )
            .expect("personal target encodes");

        assert_eq!(target.as_str(), "tg:424242:_:_");
        assert_eq!(
            codec
                .conversation_for_target(&target)
                .expect("conversation decodes")
                .conversation_id(),
            "424242"
        );
        assert!(codec.is_personal_direct_message(&target));
        assert_eq!(
            codec.direct_message_actor_for_target(&target).as_deref(),
            Some("424242")
        );
    }

    #[test]
    fn shared_group_topic_round_trips_without_a_dm_actor() {
        let codec = TelegramPreferenceTargetCodec;
        let (installation_id, agent_id, project_id) = identities();
        let conversation = ExternalConversationRef::new(None::<&str>, "-100123", Some("77"), None)
            .expect("conversation");

        let target = codec
            .encode_shared_conversation_target(request(
                &installation_id,
                &agent_id,
                &project_id,
                &conversation,
            ))
            .expect("shared target encodes");
        let decoded = codec
            .conversation_for_target(&target)
            .expect("conversation decodes");

        assert_eq!(target.as_str(), "tg:-100123:77:_");
        assert_eq!(decoded.conversation_id(), "-100123");
        assert_eq!(decoded.topic_id(), Some("77"));
        assert!(!codec.is_personal_direct_message(&target));
        assert_eq!(codec.direct_message_actor_for_target(&target), None);
    }

    #[test]
    fn personal_dm_encoding_rejects_non_private_or_mismatched_shapes() {
        let codec = TelegramPreferenceTargetCodec;
        let (installation_id, agent_id, project_id) = identities();
        for (chat_id, topic_id, actor_id) in [
            ("424242", None, "999999"),
            ("-100123", None, "424242"),
            ("424242", Some("77"), "424242"),
            ("0424242", None, "424242"),
            ("424242", None, "+424242"),
        ] {
            let conversation = ExternalConversationRef::new(None::<&str>, chat_id, topic_id, None)
                .expect("externally valid conversation");
            assert!(
                codec
                    .encode_personal_direct_message_target(
                        request(&installation_id, &agent_id, &project_id, &conversation),
                        actor_id,
                    )
                    .is_none(),
                "invalid DM shape must fail closed: chat={chat_id}, topic={topic_id:?}, actor={actor_id}"
            );
        }
    }

    #[test]
    fn decoder_rejects_message_scoped_noncanonical_and_malformed_refs() {
        let codec = TelegramPreferenceTargetCodec;
        for raw in [
            "tg:424242:_:99",
            "tg:+424242:_:_",
            "tg:0424242:_:_",
            "tg:424242:0:_",
            "reply:not-telegram",
        ] {
            let target = ReplyTargetBindingRef::new(raw).expect("syntactically valid ref");
            assert!(codec.conversation_for_target(&target).is_none());
            assert!(!codec.is_personal_direct_message(&target));
            assert_eq!(codec.direct_message_actor_for_target(&target), None);
        }
    }
}
