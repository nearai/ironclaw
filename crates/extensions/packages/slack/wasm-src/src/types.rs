//! Types for Slack user-token API requests and responses.
//!
//! Field names and output shapes mirror the standardized messaging
//! framework's canonical contracts
//! (`crates/contracts/ironclaw_host_api/schemas/messaging/*.json`) exactly: the host
//! validates every standard op's input pre-dispatch and its output
//! post-dispatch against those schemas, so a shape drift here becomes a
//! model-visible tool failure, not a silent mismatch.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Invocation context the host passes alongside params. The host selects the
/// operation via the capability id (e.g. `slack.search_messages`); the
/// action is NOT carried in the params object.
#[derive(Debug, Deserialize)]
pub(crate) struct ToolContext {
    pub(crate) capability_id: String,
}

/// A conversation kind, per the standard messaging vocabulary. Slack has no
/// native "other" conversation type; it exists so the enum is total and a
/// future vendor with an unmapped kind has somewhere to put it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConversationKind {
    Dm,
    GroupDm,
    Channel,
    Other,
}

/// Portable ordering for a cross-conversation message search.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SearchMessagesSort {
    Relevance,
    Timestamp,
}

/// Input parameters for the Slack personal (user-token) tool.
///
/// `JsonSchema` is derived so the advertised tool schema mirrors the
/// serde-enforced contract: each variant becomes a `oneOf` entry with
/// its own `required` array. Field names are the standard messaging
/// framework's canonical names (never Slack-native spellings like `channel`
/// or `thread_ts`) — the host's pre-dispatch validation enforces this
/// against the canonical input schema regardless of what this derive
/// advertises, so the two must never drift apart.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum SlackUserAction {
    /// Search across all messages you can see (DMs, group DMs, and
    /// channels you are a member of). Requires the `search:read` user scope.
    SearchMessages {
        /// Search query. Supports Slack search operators: `from:me` for your
        /// own messages (NOT `from:@me` — there is no user named "me"), plus
        /// `from:@username`, `in:#channel`, `after:2024-01-01`, `has:link`.
        query: String,
        /// Result ordering. Relevance is Slack's default; timestamp is
        /// explicitly mapped to newest-first for latest-message questions.
        #[serde(default)]
        sort: Option<SearchMessagesSort>,
        /// Maximum number of matches to return (default: 20, max: 100).
        #[serde(default)]
        limit: Option<u32>,
        /// Opaque pagination cursor from a previous call's `next_cursor`.
        /// Slack paginates search results by page number; this cursor
        /// encodes that page number as an opaque string.
        #[serde(default)]
        cursor: Option<String>,
    },

    /// List conversations visible to you: channels, private channels,
    /// DMs, and group DMs (`is_member` marks which channels you belong
    /// to). Use this to discover DM conversation IDs.
    ListConversations {
        /// Restrict results to these conversation kinds. Omit to return
        /// every kind.
        #[serde(default)]
        kinds: Option<Vec<ConversationKind>>,
        /// Maximum number of conversations to return (default: 200).
        #[serde(default)]
        limit: Option<u32>,
        /// Pagination cursor from a previous call's `next_cursor`.
        #[serde(default)]
        cursor: Option<String>,
    },

    /// Retrieve one exact conversation by its known conversation ID. For a
    /// DM, the returned `counterpart` is the authoritative counterpart.
    GetConversationInfo {
        /// Conversation ref for this extension (e.g. `C123...` for a
        /// channel, `D123...` for a DM).
        conversation: String,
    },

    /// Read message history from any conversation you can see — a channel,
    /// a DM, or a group DM — identified by its conversation ID.
    GetConversationHistory {
        /// Conversation ref (e.g. `C123...` for a channel, `D123...` for a
        /// DM).
        conversation: String,
        /// Maximum number of messages to return (default: 50, max: 999 —
        /// Slack rejects 1000; out-of-range values are clamped).
        #[serde(default)]
        limit: Option<u32>,
        /// Opaque pagination cursor from a previous call's `next_cursor`.
        #[serde(default)]
        cursor: Option<String>,
    },

    /// Read the replies of one thread (`conversations.replies`). Thread
    /// replies are NOT part of conversation history — the parent's
    /// `reply_count`/thread anchor point here.
    GetThreadReplies {
        /// Conversation ref the thread lives in (e.g. `C123...`).
        conversation: String,
        /// The thread parent message's `ts` (opaque thread anchor).
        thread: String,
        /// Maximum number of messages to return (default: 50, max: 999).
        #[serde(default)]
        limit: Option<u32>,
        /// Opaque pagination cursor from a previous call's `next_cursor`.
        #[serde(default)]
        cursor: Option<String>,
    },

    /// Get information about a user (name, real name).
    GetUserInfo {
        /// Opaque user id from this extension's people operations (e.g.
        /// "U1234567890").
        user_ref: String,
    },

    /// Resolve who the connected Slack account is (`auth.test`), with a
    /// best-effort display-name lookup. Takes no parameters.
    Whoami,

    /// Send a message as you to a channel or DM. Requires the `chat:write`
    /// user scope. The message will appear to come from your account.
    SendMessage {
        /// Conversation ref (e.g., a channel id or a DM conversation id).
        conversation: String,
        /// Message text (supports Slack mrkdwn formatting). Never use this
        /// operation for a run's own final reply when outbound delivery is
        /// configured. To notify someone else, mention them as `<@U…>` with
        /// their real user id — a plain `@name` does not notify. Never derive
        /// a user id from a conversation id. For a known DM conversation ID,
        /// call `slack.get_conversation_info` and use its `counterpart`;
        /// use `slack.list_conversations` only when the ID is unknown.
        text: String,
        /// Opaque thread anchor (a message's ts) to post into that
        /// thread/topic container. Distinct from `reply_to`: on Slack both
        /// map onto `thread_ts` (see `api::send_message`).
        #[serde(default)]
        thread: Option<String>,
        /// The specific message being quoted or replied to (pre-merge
        /// amendment W4) — distinct from `thread`. On Slack both map onto
        /// `thread_ts`; when both are supplied and disagree, `thread` wins.
        #[serde(default)]
        reply_to: Option<MessageRefInput>,
    },

    /// Replace the text of one of your own messages (`chat.update`).
    EditMessage {
        /// The exact `message_ref` a prior send or read on this extension
        /// returned — never invented, never borrowed from another extension.
        message_ref: MessageRefInput,
        /// Replacement body (Slack mrkdwn). Replaces the whole message;
        /// Slack has no partial edit.
        text: String,
    },

    /// Permanently delete one of your own messages (`chat.delete`).
    DeleteMessage {
        /// The exact `message_ref` a prior send or read returned.
        message_ref: MessageRefInput,
    },

    /// Add an emoji reaction to a message as you (`reactions.add`).
    AddReaction {
        /// The exact `message_ref` a prior send or read returned.
        message_ref: MessageRefInput,
        /// Slack emoji short name without colons (`thumbsup`). Surrounding
        /// colons are stripped; unicode characters are not accepted.
        emoji: String,
    },

    /// Remove a reaction you added (`reactions.remove`).
    RemoveReaction {
        /// The exact `message_ref` a prior send or read returned.
        message_ref: MessageRefInput,
        /// Slack emoji short name without colons. Omit to remove every
        /// reaction the connected account added to this message, which reads
        /// the message's reactions first (`reactions:read`).
        #[serde(default)]
        emoji: Option<String>,
    },

    /// Open (or fetch the already-open) DM with a person
    /// (`conversations.open`).
    OpenDm {
        /// Opaque Slack user id (`U…`/`W…`) from a people operation — never
        /// derived from a conversation id.
        user_ref: String,
    },

    /// Fetch one message by reference. Slack has no single-message endpoint;
    /// see `api::get_message` for the history/thread lookup this maps onto.
    GetMessage {
        /// The exact `message_ref` a prior send or read returned.
        message_ref: MessageRefInput,
    },

    /// Search the workspace directory for people by name or handle.
    ResolveUser {
        /// Text matched against display name, real name, and handle.
        query: String,
        /// Maximum matches to return AND the number of directory entries
        /// scanned this call (default and max: 200, one full page) — the two
        /// are the same number so `next_cursor` never skips a withheld match.
        #[serde(default)]
        limit: Option<u32>,
        /// Opaque pagination cursor from a previous call's `next_cursor`.
        #[serde(default)]
        cursor: Option<String>,
    },

    /// List the members of a channel, group DM, or DM
    /// (`conversations.members`).
    ListMembers {
        /// Conversation ref (e.g. `C123...`).
        conversation: String,
        /// Maximum members to return (default: 100, max: 999).
        #[serde(default)]
        limit: Option<u32>,
        /// Opaque pagination cursor from a previous call's `next_cursor`.
        #[serde(default)]
        cursor: Option<String>,
    },
}

/// The canonical `message_ref` INPUT shape — the identity of one message, as
/// returned by a prior send or read. Distinct from [`MessageRef`] only in
/// direction: this one deserializes what the model supplies, that one
/// serializes the evidence an operation returns.
///
/// `send_message`'s `reply_to` (the specific message being quoted) uses this
/// same shape, which is why it is not named after any single operation — see
/// `SlackUserAction::SendMessage` for how `reply_to` differs from `thread`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct MessageRefInput {
    pub conversation: String,
    pub message_id: String,
}

/// Provider-issued evidence for a sent/edited message; identifies the
/// message for follow-up edit/delete/reaction operations.
#[derive(Debug, Serialize)]
pub struct MessageRef {
    pub conversation: String,
    pub message_id: String,
}

/// The standard messaging framework's shared person shape
/// (`{ user_ref, display_name? }`). Named for its first use — a message's
/// author — but deliberately reused everywhere that object appears rather
/// than mirrored per operation: a DM `counterpart`, a `list_members` member,
/// and a `resolve_user` match are all byte-identical in the canonical
/// schemas and evolve together, so they share one definition.
#[derive(Debug, Serialize)]
pub struct Author {
    pub user_ref: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
}

/// Opaque thread anchor plus reply count, per the standard messaging
/// framework's shared `message.thread` shape.
#[derive(Debug, Serialize)]
pub struct ThreadRef {
    pub thread: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_count: Option<u64>,
}

/// One message, per the standard messaging framework's shared `message`
/// shape (`crates/contracts/ironclaw_host_api/schemas/messaging/get_conversation_history.output.v1.json`
/// and siblings). Used for history, thread replies, and search matches — the
/// canonical schema is identical across all three.
#[derive(Debug, Serialize)]
pub struct Message {
    pub message_ref: MessageRef,
    pub author: Author,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<String>,
    /// `true` when the CONNECTED account authored this message. Always a
    /// concrete bool (never absent) per the canonical schema's `required`
    /// list — defaults to `false` when the connected identity or the
    /// author is unknown, never fabricated as `true`.
    pub is_self: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread: Option<ThreadRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub edited: Option<bool>,
}

/// Result from search_messages.
#[derive(Debug, Serialize)]
pub struct SearchMessagesResult {
    pub matches: Vec<Message>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<u64>,
}

/// A conversation, per the standard messaging framework's shared
/// `conversation_info` shape. Used both for `list_conversations` items and
/// (top-level) for `get_conversation_info`'s output.
#[derive(Debug, Serialize)]
pub struct ConversationInfo {
    pub conversation: String,
    pub kind: ConversationKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// Whether the connected account is a member of this channel. Slack
    /// lists channels you can SEE, not only ones you're in — this marks the
    /// difference. Absent for DMs (no membership axis).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_member: Option<bool>,
    /// For a DM, the other participant — the authoritative mention target.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub counterpart: Option<Author>,
}

/// Result from list_conversations.
#[derive(Debug, Serialize)]
pub struct ListConversationsResult {
    pub conversations: Vec<ConversationInfo>,
    /// Cursor for the next page (pass as `cursor`). Absent on the last page.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

/// Result from get_conversation_history (also the shape of get_thread_replies).
#[derive(Debug, Serialize)]
pub struct ConversationHistoryResult {
    pub messages: Vec<Message>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

/// Result from get_user_info, per the standard messaging framework's
/// top-level `get_user_info` output shape (no `user` wrapper).
#[derive(Debug, Serialize)]
pub struct GetUserInfoResult {
    pub user_ref: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub real_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_emoji: Option<String>,
    /// IANA timezone (e.g. "America/New_York"). Absent when Slack omits it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timezone: Option<String>,
    /// Job title from the profile. Absent when unset.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub is_bot: bool,
    /// Vendor extras with no canonical field (schema's optional `vendor:
    /// object` passthrough). Absent entirely when there is nothing to carry
    /// — never an empty object.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vendor: Option<GetUserInfoVendor>,
}

/// `get_user_info`'s `vendor` passthrough payload: today, only Slack's
/// status auto-clear timestamp, which has no canonical field
/// (`status_text` does not encode it, and it is the only machine-readable
/// way to know when a status expires).
#[derive(Debug, Serialize)]
pub struct GetUserInfoVendor {
    pub status_expiration: i64,
}

/// Result from send_message.
#[derive(Debug, Serialize)]
pub struct SendMessageResult {
    pub message_ref: MessageRef,
    /// Echoes the input `thread` when supplied, so a silent drop is
    /// checkable (pre-merge amendment W3).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread: Option<String>,
    /// Echoes the input `reply_to` when supplied, so a silent drop is
    /// checkable (pre-merge amendment W3).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<MessageRef>,
}

/// Result from whoami: the CONNECTED account's identity.
#[derive(Debug, Serialize)]
pub struct WhoamiResult {
    pub user_ref: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
}

/// Result from edit_message: the same evidence shape a send returns, so the
/// edited message stays addressable for delete/reaction follow-ups.
#[derive(Debug, Serialize)]
pub struct EditMessageResult {
    pub message_ref: MessageRef,
}

/// Result from delete_message.
#[derive(Debug, Serialize)]
pub struct DeleteMessageResult {
    /// Always `true`. The canonical schema pins `const: true` because "a
    /// delete that did not happen surfaces as an error, never
    /// `deleted: false`" — the single construction site in
    /// `api::delete_message` is on the success path only, and
    /// `delete_message_result_serializes_deleted_true` pins the wire value.
    pub deleted: bool,
    pub message_ref: MessageRef,
}

/// Result from add_reaction: echoes the address and the emoji actually
/// applied (post-normalization), so a stripped `:colon:` form is visible.
#[derive(Debug, Serialize)]
pub struct AddReactionResult {
    pub message_ref: MessageRef,
    pub emoji: String,
}

/// Result from remove_reaction. `emoji` is echoed only when a single named
/// reaction was removed; the omit-emoji variant removes every reaction the
/// connected account added and therefore names none, which is exactly the
/// case the canonical schema leaves optional.
#[derive(Debug, Serialize)]
pub struct RemoveReactionResult {
    pub message_ref: MessageRef,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub emoji: Option<String>,
}

/// Result from open_dm: the DM conversation ref, usable with send_message and
/// every other conversation-scoped operation.
#[derive(Debug, Serialize)]
pub struct OpenDmResult {
    pub conversation: String,
}

/// Result from get_message: one message in the shared canonical shape.
#[derive(Debug, Serialize)]
pub struct GetMessageResult {
    pub message: Message,
}

/// Result from resolve_user. Matches use the shared person shape
/// ([`Author`]).
#[derive(Debug, Serialize)]
pub struct ResolveUserResult {
    pub matches: Vec<Author>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

/// Result from list_members. Members use the shared person shape
/// ([`Author`]).
#[derive(Debug, Serialize)]
pub struct ListMembersResult {
    pub members: Vec<Author>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The host validates every standard op's INPUT against the canonical
    /// schema before dispatch, but serde is what actually binds the params to
    /// a variant here — these pin that the two agree on what is required, so
    /// a schema-valid call can never fail to deserialize (or vice versa).
    fn action(json: &str) -> Result<SlackUserAction, serde_json::Error> {
        serde_json::from_str(json)
    }

    const REF: &str = r#"{"conversation":"C1","message_id":"1751970001.000100"}"#;

    #[test]
    fn message_addressing_ops_require_a_full_message_ref() {
        for op in [
            "edit_message",
            "delete_message",
            "add_reaction",
            "remove_reaction",
            "get_message",
        ] {
            // `text`/`emoji` are supplied so only the ref is under test; the
            // ops that ignore the extra key still deserialize.
            let with_ref =
                format!(r#"{{"action":"{op}","message_ref":{REF},"text":"x","emoji":"thumbsup"}}"#);
            assert!(action(&with_ref).is_ok(), "{op} must accept a full ref");

            let missing = format!(r#"{{"action":"{op}","text":"x","emoji":"thumbsup"}}"#);
            assert!(action(&missing).is_err(), "{op} must reject a missing ref");

            let partial = format!(
                r#"{{"action":"{op}","message_ref":{{"conversation":"C1"}},"text":"x","emoji":"thumbsup"}}"#
            );
            assert!(
                action(&partial).is_err(),
                "{op} must reject a ref without message_id"
            );
        }
    }

    #[test]
    fn reaction_ops_differ_on_whether_emoji_is_required() {
        // add_reaction names the emoji it applies; remove_reaction may omit
        // it to mean "every reaction I added" (canonical schema §4.1).
        assert!(action(&format!(
            r#"{{"action":"add_reaction","message_ref":{REF}}}"#
        ))
        .is_err());
        assert!(action(&format!(
            r#"{{"action":"add_reaction","message_ref":{REF},"emoji":"thumbsup"}}"#
        ))
        .is_ok());
        assert!(action(&format!(
            r#"{{"action":"remove_reaction","message_ref":{REF}}}"#
        ))
        .is_ok());
        assert!(action(&format!(
            r#"{{"action":"remove_reaction","message_ref":{REF},"emoji":"thumbsup"}}"#
        ))
        .is_ok());
    }

    #[test]
    fn discovery_ops_require_their_subject_and_default_their_paging() {
        assert!(action(r#"{"action":"open_dm","user_ref":"U1"}"#).is_ok());
        assert!(action(r#"{"action":"open_dm"}"#).is_err());

        assert!(action(r#"{"action":"resolve_user","query":"alice"}"#).is_ok());
        assert!(action(r#"{"action":"resolve_user"}"#).is_err());

        assert!(action(r#"{"action":"list_members","conversation":"C1"}"#).is_ok());
        assert!(action(r#"{"action":"list_members"}"#).is_err());
    }

    fn value<T: Serialize>(result: &T) -> serde_json::Value {
        serde_json::to_value(result).expect("result serializes")
    }

    fn message_ref() -> MessageRef {
        MessageRef {
            conversation: "C1".to_string(),
            message_id: "1751970001.000100".to_string(),
        }
    }

    /// A delete that did not happen is an error, never `deleted: false` — the
    /// canonical output schema pins `const: true`, so the wire value is
    /// asserted rather than left to the single construction site.
    #[test]
    fn delete_message_result_serializes_deleted_true() {
        assert_eq!(
            value(&DeleteMessageResult {
                deleted: true,
                message_ref: message_ref(),
            }),
            serde_json::json!({
                "deleted": true,
                "message_ref": {
                    "conversation": "C1",
                    "message_id": "1751970001.000100"
                }
            })
        );
    }

    /// Every write returns the address it acted on — "evidence out = address
    /// in". These compare the WHOLE serialized object, so an extra key (which
    /// the canonical schemas reject with `additionalProperties: false`) fails
    /// here rather than post-dispatch as a model-visible InvalidOutput.
    #[test]
    fn write_results_carry_exactly_the_canonical_evidence_keys() {
        let expected_ref = serde_json::json!({
            "conversation": "C1",
            "message_id": "1751970001.000100"
        });

        assert_eq!(
            value(&EditMessageResult {
                message_ref: message_ref()
            }),
            serde_json::json!({ "message_ref": expected_ref })
        );
        assert_eq!(
            value(&AddReactionResult {
                message_ref: message_ref(),
                emoji: "thumbsup".to_string(),
            }),
            serde_json::json!({ "message_ref": expected_ref, "emoji": "thumbsup" })
        );
        assert_eq!(
            value(&OpenDmResult {
                conversation: "D1".to_string()
            }),
            serde_json::json!({ "conversation": "D1" })
        );
    }

    /// `remove_reaction` echoes `emoji` only when a single named reaction was
    /// removed; the omit-emoji variant removes whatever the account reacted
    /// with and names none. An absent optional must be OMITTED, never `null`
    /// — the canonical schemas type these as strings, so a null fails
    /// validation.
    #[test]
    fn remove_reaction_result_omits_an_unnamed_emoji() {
        assert_eq!(
            value(&RemoveReactionResult {
                message_ref: message_ref(),
                emoji: Some("thumbsup".to_string()),
            })["emoji"],
            serde_json::json!("thumbsup")
        );

        let unnamed = value(&RemoveReactionResult {
            message_ref: message_ref(),
            emoji: None,
        });
        assert!(
            unnamed.get("emoji").is_none(),
            "an unnamed removal must omit emoji entirely, got {unnamed}"
        );
    }

    /// `resolve_user` matches and `list_members` members are the same
    /// canonical person object, and both page with an omitted-when-absent
    /// `next_cursor`.
    #[test]
    fn people_results_share_the_person_shape_and_omit_an_absent_cursor() {
        let people = || {
            vec![
                Author {
                    user_ref: "U1".to_string(),
                    display_name: Some("Alice".to_string()),
                },
                Author {
                    user_ref: "U2".to_string(),
                    display_name: None,
                },
            ]
        };
        let expected = serde_json::json!([
            { "user_ref": "U1", "display_name": "Alice" },
            { "user_ref": "U2" }
        ]);

        let resolved = value(&ResolveUserResult {
            matches: people(),
            next_cursor: None,
        });
        assert_eq!(resolved["matches"], expected);
        assert!(resolved.get("next_cursor").is_none());

        let listed = value(&ListMembersResult {
            members: people(),
            next_cursor: Some("dXNlcjpVMDYxTkZUVDI=".to_string()),
        });
        assert_eq!(listed["members"], expected);
        assert_eq!(
            listed["next_cursor"],
            serde_json::json!("dXNlcjpVMDYxTkZUVDI=")
        );
    }

    /// `get_message` wraps the shared message shape under a `message` key
    /// (its canonical output is `{ message }`, not a bare message), and
    /// `is_self` is always present — never omitted, never fabricated true.
    #[test]
    fn get_message_result_wraps_the_message_and_always_states_is_self() {
        let serialized = value(&GetMessageResult {
            message: Message {
                message_ref: message_ref(),
                author: Author {
                    user_ref: "U1".to_string(),
                    display_name: None,
                },
                text: "hello".to_string(),
                timestamp: None,
                is_self: false,
                thread: None,
                edited: None,
            },
        });
        assert_eq!(
            serialized,
            serde_json::json!({
                "message": {
                    "message_ref": {
                        "conversation": "C1",
                        "message_id": "1751970001.000100"
                    },
                    "author": { "user_ref": "U1" },
                    "text": "hello",
                    "is_self": false
                }
            })
        );
    }
}
