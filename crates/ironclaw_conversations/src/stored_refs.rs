//! Durable spelling of the channel-owned external refs this crate persists.
//!
//! [`ExternalConversationRef`] is owned by `ironclaw_extension_contracts`, but
//! the *records* that embed it belong to this crate, and **a record grammar
//! outlives the type that shaped it**. Released builds wrote
//! `{space_id,conversation_id,thread_id,message_id}`; the canonical type spells
//! the last two `topic_id` / `reply_target_message_id`.
//!
//! This adapter keeps the released grammar on the wire and accepts either
//! spelling on the way in. The rename is therefore invisible to durable
//! storage, which is the intent: it is a Rust-side fix for the
//! `conversations`/`threads` naming trap (`topic_id` is the *channel's*
//! sub-conversation, never a canonical
//! [`ThreadId`](ironclaw_host_api::ids::ThreadId)), and a naming trap in Rust
//! is not a reason to rewrite a persisted document.
//!
//! # Why the write side matters as much as the read side
//!
//! Every renamed field is an `Option`. Serde fills a missing `Option` with
//! `None` and does not error, so a spelling mismatch in *either* direction is
//! silent: the reader sees a route with no topic, and every threaded reply
//! collapses to its conversation root. The collapse is worse than misrouting —
//! it is lossy. [`ExternalConversationIdentity`](crate::ExternalConversationIdentity)
//! keys `BindingKey`, and `StoredConversationState::into_state` rebuilds the
//! binding map with `Vec<(K, V)>::into_iter().collect()`, so two threaded
//! bindings in one conversation that both lose their topic become the *same*
//! key and the later one silently wins.
//!
//! That failure mode is symmetric, which is why this module writes the legacy
//! grammar rather than the canonical one:
//!
//! - **Upgrade** (a released record read by this build) — handled by the
//!   `#[serde(alias)]`es below.
//! - **Rollback** (a record written by this build read by a released binary) —
//!   handled by writing `thread_id`/`message_id`. The released reader names
//!   those two fields exactly and carries no aliases, so any other spelling
//!   reads back as `None`.
//!
//! Emitting *both* spellings is a different thing from the write-legacy /
//! read-either shape used here, and it was refused: `#[serde(alias)]` makes a
//! dual-written record self-defeating, because a reader that accepts both
//! spellings rejects a document carrying both as a duplicate field. That
//! rejection is kept deliberately — an ambiguous record fails closed rather
//! than letting field order pick the winner.
//!
//! Binding identity is durable, so nothing ages out of it; the 48-hour
//! `DELIVERED_GATE_ROUTE_TTL` heals the *fingerprint* format change recorded on
//! the same CHECKLIST row, not this.
//!
//! # Scope: no actor adapter
//!
//! Only the conversation ref needs one. [`ExternalActorRef`]'s change was
//! purely *additive* — released builds wrote `{kind,id}`, this build writes
//! `{kind,id,display_name}` — and an added field is safe in both directions:
//! serde ignores unknown fields by default, and a missing `Option` reads as
//! `None`. Its canonical `Serialize`/`Deserialize` already revalidate through
//! `ExternalActorRef::new`, so an adapter here would only restate them.
//! `actor_serde_needs_no_adapter` pins that equivalence, so the claim is
//! checked rather than asserted.
//!
//! [`ExternalActorRef`]: ironclaw_extension_contracts::external::ExternalActorRef

use ironclaw_extension_contracts::external::ExternalConversationRef;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Serde adapter for a persisted [`ExternalConversationRef`] field.
pub(crate) mod conversation_ref {
    use super::*;

    /// The durable grammar, borrowed from the canonical ref for writing. The
    /// field names here are the *record's*, not the type's.
    #[derive(Serialize)]
    struct Written<'a> {
        space_id: Option<&'a str>,
        conversation_id: &'a str,
        thread_id: Option<&'a str>,
        message_id: Option<&'a str>,
    }

    /// The durable grammar for reading. Released records spell the last two
    /// fields `thread_id`/`message_id`; a record written by an intermediate
    /// build of this rename spells them canonically. Both load.
    #[derive(Deserialize)]
    struct Read {
        space_id: Option<String>,
        conversation_id: String,
        #[serde(alias = "topic_id")]
        thread_id: Option<String>,
        #[serde(alias = "reply_target_message_id")]
        message_id: Option<String>,
    }

    pub(crate) fn serialize<S>(
        value: &ExternalConversationRef,
        serializer: S,
    ) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        Written {
            space_id: value.space_id(),
            conversation_id: value.conversation_id(),
            thread_id: value.topic_id(),
            message_id: value.reply_target_message_id(),
        }
        .serialize(serializer)
    }

    pub(crate) fn deserialize<'de, D>(deserializer: D) -> Result<ExternalConversationRef, D::Error>
    where
        D: Deserializer<'de>,
    {
        let stored = Read::deserialize(deserializer)?;
        // Re-validate through the canonical constructor so a hand-edited or
        // corrupted durable record cannot smuggle an unvalidated identifier
        // back into memory.
        ExternalConversationRef::new(
            stored.space_id.as_deref(),
            stored.conversation_id,
            stored.thread_id.as_deref(),
            stored.message_id.as_deref(),
        )
        .map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_extension_contracts::external::ExternalActorRef;

    #[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
    struct StoredConversation {
        #[serde(with = "conversation_ref")]
        external_conversation_ref: ExternalConversationRef,
    }

    /// Deliberately carries no `#[serde(with)]` — the canonical impls are the
    /// subject of `actor_serde_needs_no_adapter`.
    #[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
    struct StoredActor {
        external_actor_ref: ExternalActorRef,
    }

    fn threaded() -> ExternalConversationRef {
        ExternalConversationRef::new(Some("T1"), "C1", Some("1700.1"), Some("1800.2"))
            .expect("valid")
    }

    #[test]
    fn legacy_conversation_record_keeps_its_topic_and_reply_target() {
        let legacy: StoredConversation = serde_json::from_str(
            r#"{"external_conversation_ref":{"space_id":"T1","conversation_id":"C1",
                "thread_id":"1700.1","message_id":"1800.2"}}"#,
        )
        .expect("a record written before the unification must still load");
        let route = &legacy.external_conversation_ref;
        assert_eq!(route.topic_id(), Some("1700.1"));
        assert_eq!(route.reply_target_message_id(), Some("1800.2"));
    }

    /// The rollback half. A released binary's reader names `thread_id` and
    /// `message_id` and carries no aliases, so that is the only spelling it can
    /// see. Asserting the *absence* of the canonical spelling is what fails if
    /// someone reverts `serialize` to delegate to the canonical derive.
    #[test]
    fn current_conversation_record_is_written_in_the_durable_grammar() {
        let json = serde_json::to_string(&StoredConversation {
            external_conversation_ref: threaded(),
        })
        .expect("serialize");
        let written: serde_json::Value =
            serde_json::from_str(&json).expect("the written record parses");
        let route = &written["external_conversation_ref"];
        assert_eq!(route["thread_id"], "1700.1", "writes the durable grammar");
        assert_eq!(route["message_id"], "1800.2", "writes the durable grammar");
        assert!(
            route.get("topic_id").is_none() && route.get("reply_target_message_id").is_none(),
            "a released reader has no alias for the canonical spelling, so writing it \
             would silently strand every threaded route: {json}"
        );
    }

    #[test]
    fn current_conversation_record_round_trips() {
        let record = StoredConversation {
            external_conversation_ref: threaded(),
        };
        let json = serde_json::to_string(&record).expect("serialize");
        let parsed: StoredConversation = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(record, parsed);
    }

    /// The alias is not dead code: an intermediate build of this rename wrote
    /// the canonical spelling, and such a record must still load.
    #[test]
    fn canonically_spelled_conversation_record_still_loads() {
        let canonical: StoredConversation = serde_json::from_str(
            r#"{"external_conversation_ref":{"space_id":"T1","conversation_id":"C1",
                "topic_id":"1700.1","reply_target_message_id":"1800.2"}}"#,
        )
        .expect("the canonical spelling remains readable");
        let route = &canonical.external_conversation_ref;
        assert_eq!(route.topic_id(), Some("1700.1"));
        assert_eq!(route.reply_target_message_id(), Some("1800.2"));
    }

    /// A record carrying both spellings for one field is ambiguous. Fail closed
    /// rather than let serde's field order pick the winner.
    #[test]
    fn a_record_carrying_both_spellings_is_rejected() {
        let result: Result<StoredConversation, _> = serde_json::from_str(
            r#"{"external_conversation_ref":{"space_id":"T1","conversation_id":"C1",
                "thread_id":"1700.1","topic_id":"9900.9"}}"#,
        );
        assert!(
            result.is_err(),
            "an ambiguous record must not silently resolve to one of its two topics"
        );
    }

    #[test]
    fn stored_conversation_ref_revalidates_on_load() {
        let result: Result<StoredConversation, _> = serde_json::from_str(
            r#"{"external_conversation_ref":{"space_id":null,"conversation_id":"",
                "thread_id":null,"message_id":null}}"#,
        );
        assert!(result.is_err(), "an empty conversation id must not load");
    }

    /// The evidence for this module carrying no actor adapter: the canonical
    /// impls already do everything one would — accept a released `{kind,id}`
    /// payload, default the added field, and revalidate through
    /// `ExternalActorRef::new`.
    #[test]
    fn actor_serde_needs_no_adapter() {
        let legacy: StoredActor =
            serde_json::from_str(r#"{"external_actor_ref":{"kind":"slack","id":"U1"}}"#)
                .expect("a record written before the unification must still load");
        assert_eq!(legacy.external_actor_ref.kind(), "slack");
        assert_eq!(legacy.external_actor_ref.id(), "U1");
        assert_eq!(legacy.external_actor_ref.display_name(), None);

        let named = StoredActor {
            external_actor_ref: ExternalActorRef::new("slack", "U1", Some("Alice")).expect("valid"),
        };
        assert_eq!(
            legacy.external_actor_ref, named.external_actor_ref,
            "pairing lookups key on (kind, id), so a display name must not re-key them"
        );

        // Additive, so safe in the rollback direction too: a released reader
        // names `kind`/`id` and ignores the field it does not know.
        let json = serde_json::to_string(&named).expect("serialize");
        let written: serde_json::Value = serde_json::from_str(&json).expect("parses");
        assert_eq!(written["external_actor_ref"]["kind"], "slack");
        assert_eq!(written["external_actor_ref"]["id"], "U1");
    }

    #[test]
    fn stored_actor_ref_revalidates_on_load() {
        let result: Result<StoredActor, _> =
            serde_json::from_str(r#"{"external_actor_ref":{"kind":"slack","id":""}}"#);
        assert!(result.is_err(), "an empty actor id must not load");
    }
}
