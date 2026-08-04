//! Adapter-scoped identifier vocabulary for conversation binding.
//!
//! The external actor/conversation pair is **not** declared here. It is channel
//! vocabulary, and its one home is
//! [`ironclaw_extension_contracts::external`] — this crate consumes
//! [`ExternalActorRef`] and [`ExternalConversationRef`] from there rather than
//! carrying a parallel copy. What remains here is the vocabulary conversation
//! binding genuinely owns: the adapter-scoped bounded strings, and
//! [`ExternalConversationIdentity`], the *route* projection of a conversation
//! ref (space + conversation + topic, no reply-target hint) that keys durable
//! bindings.
//!
//! [`ExternalActorRef`]: ironclaw_extension_contracts::external::ExternalActorRef

use ironclaw_extension_contracts::external::ExternalConversationRef;
use ironclaw_host_api::product_adapter_error::ProductAdapterError;
use serde::{Deserialize, Serialize};

use crate::InboundTurnError;

macro_rules! bounded_string_id {
    ($name:ident, $kind:literal) => {
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, InboundTurnError> {
                let value = value.into();
                validate_external_id($kind, &value)?;
                Ok(Self(value))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                let value = String::deserialize(deserializer)?;
                Self::new(value).map_err(serde::de::Error::custom)
            }
        }
    };
}

bounded_string_id!(AdapterKind, "adapter_kind");

bounded_string_id!(AdapterInstallationId, "adapter_installation_id");
bounded_string_id!(ExternalEventId, "external_event_id");
bounded_string_id!(InboundMessageContentRef, "inbound_message_content_ref");

/// Translate the channel tier's identifier-validation failure into this
/// crate's error vocabulary.
///
/// Constructing an [`ExternalActorRef`](ironclaw_extension_contracts::external::ExternalActorRef)
/// or an [`ExternalConversationRef`] now fails with
/// [`ProductAdapterError`] because that pair is owned by
/// `ironclaw_extension_contracts`. Conversation callers still see
/// [`InboundTurnError::InvalidExternalRef`], so the error surface every
/// consumer already matches on is unchanged by the unification. A named
/// function rather than a closure so it is directly testable.
pub(crate) fn map_external_ref_error(error: ProductAdapterError) -> InboundTurnError {
    match error {
        ProductAdapterError::InvalidIdentifier { kind, reason } => {
            InboundTurnError::InvalidExternalRef { kind, reason }
        }
        other => InboundTurnError::InvalidExternalRef {
            kind: "external_ref",
            reason: other.to_string(),
        },
    }
}

/// The durable *route* identity of an external conversation: everything that
/// distinguishes one route from another, and nothing that varies per event.
///
/// This is deliberately narrower than [`ExternalConversationRef`], which also
/// carries the per-event `reply_target_message_id`. Bindings are keyed by this
/// projection so two messages in the same conversation resolve to the same
/// binding.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ExternalConversationIdentity {
    pub(crate) space_id: Option<String>,
    pub(crate) conversation_id: String,
    /// The external thread/topic within the conversation. Named `topic_id`
    /// after the `conversations`/`threads` naming-trap fix: it is the *channel's*
    /// sub-conversation, never a canonical [`ThreadId`](ironclaw_host_api::ids::ThreadId).
    ///
    /// The *record* keeps the released spelling, `thread_id`. Both `Serialize`
    /// and `Deserialize` below are hand-written for that reason, and
    /// [`crate::stored_refs`] carries the argument: this identity keys
    /// `BindingKey`, so a spelling the reader on the other side of a deploy
    /// cannot see does not merely misroute a threaded reply — it collides two
    /// bindings onto one key and drops the earlier one.
    pub(crate) topic_id: Option<String>,
}

impl ExternalConversationIdentity {
    /// Project a conversation ref onto its route identity.
    pub(crate) fn from_ref(conversation_ref: &ExternalConversationRef) -> Self {
        Self {
            space_id: conversation_ref.space_id().map(str::to_string),
            conversation_id: conversation_ref.conversation_id().to_string(),
            topic_id: conversation_ref.topic_id().map(str::to_string),
        }
    }
}

impl Serialize for ExternalConversationIdentity {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        /// The durable grammar. Field names here are the *record's*, not the
        /// type's — see [`crate::stored_refs`].
        #[derive(Serialize)]
        struct StoredExternalConversationIdentity<'a> {
            space_id: Option<&'a str>,
            conversation_id: &'a str,
            thread_id: Option<&'a str>,
        }

        StoredExternalConversationIdentity {
            space_id: self.space_id.as_deref(),
            conversation_id: &self.conversation_id,
            thread_id: self.topic_id.as_deref(),
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for ExternalConversationIdentity {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct RawExternalConversationIdentity {
            space_id: Option<String>,
            conversation_id: String,
            /// `thread_id` is the durable spelling, written by released builds
            /// and by this one; `topic_id` is accepted so a record written by
            /// an intermediate build of the rename still loads.
            #[serde(alias = "topic_id")]
            thread_id: Option<String>,
        }

        let raw = RawExternalConversationIdentity::deserialize(deserializer)?;
        // Re-validate through the canonical constructor so a hand-edited or
        // corrupted durable record cannot smuggle an unvalidated identifier
        // back into memory.
        ExternalConversationRef::new(
            raw.space_id.as_deref(),
            raw.conversation_id.as_str(),
            raw.thread_id.as_deref(),
            None,
        )
        .map(|conversation_ref| Self::from_ref(&conversation_ref))
        .map_err(serde::de::Error::custom)
    }
}

pub(crate) fn validate_external_id(
    kind: &'static str,
    value: &str,
) -> Result<(), InboundTurnError> {
    if value.is_empty() {
        return Err(InboundTurnError::InvalidExternalRef {
            kind,
            reason: "must not be empty".to_string(),
        });
    }
    if value.len() > 512 {
        return Err(InboundTurnError::InvalidExternalRef {
            kind,
            reason: "must be at most 512 bytes".to_string(),
        });
    }
    if value.chars().any(|c| c == '\0' || c.is_control()) {
        return Err(InboundTurnError::InvalidExternalRef {
            kind,
            reason: "must not contain NUL/control characters".to_string(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_extension_contracts::external::ExternalActorRef;

    fn conversation_ref(
        topic: Option<&str>,
        reply_target: Option<&str>,
    ) -> ExternalConversationRef {
        ExternalConversationRef::new(Some("space-1"), "conv-1", topic, reply_target)
            .expect("valid conversation ref")
    }

    #[test]
    fn route_identity_drops_the_reply_target_but_keeps_the_topic() {
        let with_reply = conversation_ref(Some("topic-7"), Some("msg-100"));
        let without_reply = conversation_ref(Some("topic-7"), None);
        assert_eq!(
            ExternalConversationIdentity::from_ref(&with_reply),
            ExternalConversationIdentity::from_ref(&without_reply),
            "the per-event reply target must not split a route"
        );

        let other_topic = conversation_ref(Some("topic-8"), None);
        assert_ne!(
            ExternalConversationIdentity::from_ref(&without_reply),
            ExternalConversationIdentity::from_ref(&other_topic),
            "the topic is part of the route identity"
        );
    }

    /// Durable binding keys written before the `thread_id` → `topic_id` rename
    /// must still resolve to the same route, or every existing external
    /// conversation silently re-binds as a fresh conversation-root route.
    #[test]
    fn legacy_persisted_identity_spelling_loads_as_the_same_route() {
        let legacy: ExternalConversationIdentity = serde_json::from_str(
            r#"{"space_id":"space-1","conversation_id":"conv-1","thread_id":"topic-7"}"#,
        )
        .expect("legacy identity must still deserialize");
        assert_eq!(
            legacy,
            ExternalConversationIdentity::from_ref(&conversation_ref(Some("topic-7"), None))
        );

        let current: ExternalConversationIdentity = serde_json::from_str(
            r#"{"space_id":"space-1","conversation_id":"conv-1","topic_id":"topic-7"}"#,
        )
        .expect("current identity must deserialize");
        assert_eq!(legacy, current);
    }

    #[test]
    fn identity_deserialization_revalidates_the_conversation_id() {
        let result: Result<ExternalConversationIdentity, _> =
            serde_json::from_str(r#"{"space_id":null,"conversation_id":"","topic_id":null}"#);
        assert!(
            result.is_err(),
            "an empty conversation id must not survive a durable round trip"
        );
    }

    #[test]
    fn channel_identifier_failures_keep_this_crate_error_vocabulary() {
        let error = ExternalActorRef::new("slack", "", None::<String>)
            .map(|_| ())
            .expect_err("an empty actor id is invalid");
        assert!(matches!(
            map_external_ref_error(error),
            InboundTurnError::InvalidExternalRef {
                kind: "external_actor_id",
                ..
            }
        ));

        // The unification widened the input to the whole `ProductAdapterError`
        // enum, so the mapper needs a total fallback: anything that is not an
        // `InvalidIdentifier` still has to surface as this crate's
        // `InvalidExternalRef` rather than escape as a foreign error. The
        // generic `kind` is what marks it as the non-identifier path, and the
        // source message must survive into `reason` or the caller loses the
        // only description of what actually failed.
        let non_identifier = ProductAdapterError::EgressUndeclaredHost {
            host: "hooks.example.invalid".to_string(),
        };
        let expected_reason = non_identifier.to_string();
        match map_external_ref_error(non_identifier) {
            InboundTurnError::InvalidExternalRef { kind, reason } => {
                assert_eq!(
                    kind, "external_ref",
                    "a non-identifier adapter error maps to the generic ref kind"
                );
                assert_eq!(
                    reason, expected_reason,
                    "the source error's message must be carried through verbatim"
                );
            }
            other => panic!("expected InvalidExternalRef, got {other:?}"),
        }
    }
}
