//! Standard messaging operations: closed vocabulary, canonical contracts, and
//! the error-code taxonomy for the standardized messaging framework
//! (`docs/internal/superpowers/specs/2026-07-27-standardized-messaging-framework-design.md`).
//!
//! Channel extensions bind their own tools to one of these operations via the
//! manifest `standard_op` field (see `ironclaw_extensions`); this module is the
//! host-owned authority for what each operation means, its canonical
//! input/output JSON Schema, and its model-facing description. Extensions
//! implement vendor mechanics only — they cannot define or override an
//! operation's shape.
//!
//! 16 core operations carry a full [`StandardOpContract`] ([`StandardMessagingOp::contract`]
//! returns `Some`); 13 further names are reserved in the closed enum
//! (`contract()` returns `None`) until an implementor lands and they graduate.

use serde::{Deserialize, Serialize};

/// One operation in the standard messaging vocabulary. Snake_case wire tokens
/// (`op_name()`); unknown names are rejected at manifest-parse time by the
/// binding validation in `ironclaw_extensions` (not this crate).
macro_rules! declare_standard_messaging_ops {
    ($($variant:ident),+ $(,)?) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub enum StandardMessagingOp {
            $($variant),+
        }

        impl StandardMessagingOp {
            /// Every variant, core operations first, then reserved names —
            /// generated from the enum's single declaration list so a new
            /// variant cannot silently disappear from registry iteration.
            pub const ALL: &'static [StandardMessagingOp] = &[
                $(Self::$variant),+
            ];
        }
    };
}

declare_standard_messaging_ops!(
    // core writes
    SendMessage,
    EditMessage,
    DeleteMessage,
    AddReaction,
    RemoveReaction,
    OpenDm,
    // core reads
    ListConversations,
    GetConversationInfo,
    GetConversationHistory,
    GetThreadReplies,
    GetMessage,
    SearchMessages,
    // core people
    GetUserInfo,
    ResolveUser,
    ListMembers,
    Whoami,
    // reserved (contract() == None): names claimed, binding rejected until an
    // implementor lands and the op graduates a contract.
    ForwardMessage,
    ScheduleMessage,
    ListReactions,
    PinMessage,
    UnpinMessage,
    ListPins,
    CreateGroup,
    JoinConversation,
    LeaveConversation,
    InviteMember,
    RemoveMember,
    SetTopic,
    ArchiveConversation,
);

impl StandardMessagingOp {
    /// Snake_case wire token; identical to this operation's serde
    /// representation (pinned by `op_names_round_trip_snake_case_serde`).
    pub fn op_name(&self) -> &'static str {
        match self {
            Self::SendMessage => "send_message",
            Self::EditMessage => "edit_message",
            Self::DeleteMessage => "delete_message",
            Self::AddReaction => "add_reaction",
            Self::RemoveReaction => "remove_reaction",
            Self::OpenDm => "open_dm",
            Self::ListConversations => "list_conversations",
            Self::GetConversationInfo => "get_conversation_info",
            Self::GetConversationHistory => "get_conversation_history",
            Self::GetThreadReplies => "get_thread_replies",
            Self::GetMessage => "get_message",
            Self::SearchMessages => "search_messages",
            Self::GetUserInfo => "get_user_info",
            Self::ResolveUser => "resolve_user",
            Self::ListMembers => "list_members",
            Self::Whoami => "whoami",
            Self::ForwardMessage => "forward_message",
            Self::ScheduleMessage => "schedule_message",
            Self::ListReactions => "list_reactions",
            Self::PinMessage => "pin_message",
            Self::UnpinMessage => "unpin_message",
            Self::ListPins => "list_pins",
            Self::CreateGroup => "create_group",
            Self::JoinConversation => "join_conversation",
            Self::LeaveConversation => "leave_conversation",
            Self::InviteMember => "invite_member",
            Self::RemoveMember => "remove_member",
            Self::SetTopic => "set_topic",
            Self::ArchiveConversation => "archive_conversation",
        }
    }

    /// Whether this operation mutates vendor-side state. The manifest binding
    /// validation (`ironclaw_extensions`) requires write-family ops to declare
    /// `external_write`; reads are not forced to (spec §6 rule 4). The 6 core
    /// writes are `send_message`, `edit_message`, `delete_message`,
    /// `add_reaction`, `remove_reaction`, `open_dm`; reserved names are
    /// classified the same way by verb (`list_*` names read, everything else
    /// mutates) even though none can bind yet.
    pub fn is_write(&self) -> bool {
        matches!(
            self,
            Self::SendMessage
                | Self::EditMessage
                | Self::DeleteMessage
                | Self::AddReaction
                | Self::RemoveReaction
                | Self::OpenDm
                | Self::ForwardMessage
                | Self::ScheduleMessage
                | Self::PinMessage
                | Self::UnpinMessage
                | Self::CreateGroup
                | Self::JoinConversation
                | Self::LeaveConversation
                | Self::InviteMember
                | Self::RemoveMember
                | Self::SetTopic
                | Self::ArchiveConversation
        )
    }

    /// The canonical contract for this operation, or `None` for a reserved
    /// name that has not yet graduated (spec §4).
    pub fn contract(&self) -> Option<&'static StandardOpContract> {
        match self {
            Self::SendMessage => Some(&SEND_MESSAGE_CONTRACT),
            Self::EditMessage => Some(&EDIT_MESSAGE_CONTRACT),
            Self::DeleteMessage => Some(&DELETE_MESSAGE_CONTRACT),
            Self::AddReaction => Some(&ADD_REACTION_CONTRACT),
            Self::RemoveReaction => Some(&REMOVE_REACTION_CONTRACT),
            Self::OpenDm => Some(&OPEN_DM_CONTRACT),
            Self::ListConversations => Some(&LIST_CONVERSATIONS_CONTRACT),
            Self::GetConversationInfo => Some(&GET_CONVERSATION_INFO_CONTRACT),
            Self::GetConversationHistory => Some(&GET_CONVERSATION_HISTORY_CONTRACT),
            Self::GetThreadReplies => Some(&GET_THREAD_REPLIES_CONTRACT),
            Self::GetMessage => Some(&GET_MESSAGE_CONTRACT),
            Self::SearchMessages => Some(&SEARCH_MESSAGES_CONTRACT),
            Self::GetUserInfo => Some(&GET_USER_INFO_CONTRACT),
            Self::ResolveUser => Some(&RESOLVE_USER_CONTRACT),
            Self::ListMembers => Some(&LIST_MEMBERS_CONTRACT),
            Self::Whoami => Some(&WHOAMI_CONTRACT),
            Self::ForwardMessage
            | Self::ScheduleMessage
            | Self::ListReactions
            | Self::PinMessage
            | Self::UnpinMessage
            | Self::ListPins
            | Self::CreateGroup
            | Self::JoinConversation
            | Self::LeaveConversation
            | Self::InviteMember
            | Self::RemoveMember
            | Self::SetTopic
            | Self::ArchiveConversation => None,
        }
    }
}

/// The canonical contract for one core standard messaging operation: its
/// input/output JSON Schema (draft-07, `include_str!`-compiled from
/// `schemas/messaging/`) and its model-facing description core
/// (`include_str!`-compiled from `prompts/messaging/`). Composed at descriptor
/// build with the extension's vendor addendum (spec §5.3).
#[derive(Debug, Clone, Copy)]
pub struct StandardOpContract {
    pub op: StandardMessagingOp,
    pub input_schema: &'static str,
    pub output_schema: &'static str,
    pub description_core: &'static str,
    pub is_write: bool,
}

static SEND_MESSAGE_CONTRACT: StandardOpContract = StandardOpContract {
    op: StandardMessagingOp::SendMessage,
    input_schema: include_str!("../schemas/messaging/send_message.input.v1.json"),
    output_schema: include_str!("../schemas/messaging/send_message.output.v1.json"),
    description_core: include_str!("../prompts/messaging/send_message.core.md"),
    is_write: true,
};

static EDIT_MESSAGE_CONTRACT: StandardOpContract = StandardOpContract {
    op: StandardMessagingOp::EditMessage,
    input_schema: include_str!("../schemas/messaging/edit_message.input.v1.json"),
    output_schema: include_str!("../schemas/messaging/edit_message.output.v1.json"),
    description_core: include_str!("../prompts/messaging/edit_message.core.md"),
    is_write: true,
};

static DELETE_MESSAGE_CONTRACT: StandardOpContract = StandardOpContract {
    op: StandardMessagingOp::DeleteMessage,
    input_schema: include_str!("../schemas/messaging/delete_message.input.v1.json"),
    output_schema: include_str!("../schemas/messaging/delete_message.output.v1.json"),
    description_core: include_str!("../prompts/messaging/delete_message.core.md"),
    is_write: true,
};

static ADD_REACTION_CONTRACT: StandardOpContract = StandardOpContract {
    op: StandardMessagingOp::AddReaction,
    input_schema: include_str!("../schemas/messaging/add_reaction.input.v1.json"),
    output_schema: include_str!("../schemas/messaging/add_reaction.output.v1.json"),
    description_core: include_str!("../prompts/messaging/add_reaction.core.md"),
    is_write: true,
};

static REMOVE_REACTION_CONTRACT: StandardOpContract = StandardOpContract {
    op: StandardMessagingOp::RemoveReaction,
    input_schema: include_str!("../schemas/messaging/remove_reaction.input.v1.json"),
    output_schema: include_str!("../schemas/messaging/remove_reaction.output.v1.json"),
    description_core: include_str!("../prompts/messaging/remove_reaction.core.md"),
    is_write: true,
};

static OPEN_DM_CONTRACT: StandardOpContract = StandardOpContract {
    op: StandardMessagingOp::OpenDm,
    input_schema: include_str!("../schemas/messaging/open_dm.input.v1.json"),
    output_schema: include_str!("../schemas/messaging/open_dm.output.v1.json"),
    description_core: include_str!("../prompts/messaging/open_dm.core.md"),
    is_write: true,
};

static LIST_CONVERSATIONS_CONTRACT: StandardOpContract = StandardOpContract {
    op: StandardMessagingOp::ListConversations,
    input_schema: include_str!("../schemas/messaging/list_conversations.input.v1.json"),
    output_schema: include_str!("../schemas/messaging/list_conversations.output.v1.json"),
    description_core: include_str!("../prompts/messaging/list_conversations.core.md"),
    is_write: false,
};

static GET_CONVERSATION_INFO_CONTRACT: StandardOpContract = StandardOpContract {
    op: StandardMessagingOp::GetConversationInfo,
    input_schema: include_str!("../schemas/messaging/get_conversation_info.input.v1.json"),
    output_schema: include_str!("../schemas/messaging/get_conversation_info.output.v1.json"),
    description_core: include_str!("../prompts/messaging/get_conversation_info.core.md"),
    is_write: false,
};

static GET_CONVERSATION_HISTORY_CONTRACT: StandardOpContract = StandardOpContract {
    op: StandardMessagingOp::GetConversationHistory,
    input_schema: include_str!("../schemas/messaging/get_conversation_history.input.v1.json"),
    output_schema: include_str!("../schemas/messaging/get_conversation_history.output.v1.json"),
    description_core: include_str!("../prompts/messaging/get_conversation_history.core.md"),
    is_write: false,
};

static GET_THREAD_REPLIES_CONTRACT: StandardOpContract = StandardOpContract {
    op: StandardMessagingOp::GetThreadReplies,
    input_schema: include_str!("../schemas/messaging/get_thread_replies.input.v1.json"),
    output_schema: include_str!("../schemas/messaging/get_thread_replies.output.v1.json"),
    description_core: include_str!("../prompts/messaging/get_thread_replies.core.md"),
    is_write: false,
};

static GET_MESSAGE_CONTRACT: StandardOpContract = StandardOpContract {
    op: StandardMessagingOp::GetMessage,
    input_schema: include_str!("../schemas/messaging/get_message.input.v1.json"),
    output_schema: include_str!("../schemas/messaging/get_message.output.v1.json"),
    description_core: include_str!("../prompts/messaging/get_message.core.md"),
    is_write: false,
};

static SEARCH_MESSAGES_CONTRACT: StandardOpContract = StandardOpContract {
    op: StandardMessagingOp::SearchMessages,
    input_schema: include_str!("../schemas/messaging/search_messages.input.v1.json"),
    output_schema: include_str!("../schemas/messaging/search_messages.output.v1.json"),
    description_core: include_str!("../prompts/messaging/search_messages.core.md"),
    is_write: false,
};

static GET_USER_INFO_CONTRACT: StandardOpContract = StandardOpContract {
    op: StandardMessagingOp::GetUserInfo,
    input_schema: include_str!("../schemas/messaging/get_user_info.input.v1.json"),
    output_schema: include_str!("../schemas/messaging/get_user_info.output.v1.json"),
    description_core: include_str!("../prompts/messaging/get_user_info.core.md"),
    is_write: false,
};

static RESOLVE_USER_CONTRACT: StandardOpContract = StandardOpContract {
    op: StandardMessagingOp::ResolveUser,
    input_schema: include_str!("../schemas/messaging/resolve_user.input.v1.json"),
    output_schema: include_str!("../schemas/messaging/resolve_user.output.v1.json"),
    description_core: include_str!("../prompts/messaging/resolve_user.core.md"),
    is_write: false,
};

static LIST_MEMBERS_CONTRACT: StandardOpContract = StandardOpContract {
    op: StandardMessagingOp::ListMembers,
    input_schema: include_str!("../schemas/messaging/list_members.input.v1.json"),
    output_schema: include_str!("../schemas/messaging/list_members.output.v1.json"),
    description_core: include_str!("../prompts/messaging/list_members.core.md"),
    is_write: false,
};

static WHOAMI_CONTRACT: StandardOpContract = StandardOpContract {
    op: StandardMessagingOp::Whoami,
    input_schema: include_str!("../schemas/messaging/whoami.input.v1.json"),
    output_schema: include_str!("../schemas/messaging/whoami.output.v1.json"),
    description_core: include_str!("../prompts/messaging/whoami.core.md"),
    is_write: false,
};

/// Prefix for a standard-op schema ref, as synthesized at manifest parse time
/// (spec §6): `standard:messaging/<op_name>.input.v1` /
/// `standard:messaging/<op_name>.output.v1`.
pub const STANDARD_SCHEMA_REF_PREFIX: &str = "standard:messaging/";

/// Resolve a `standard:messaging/...` schema ref to its compiled-in canonical
/// JSON Schema text, the way builtin schema refs already resolve from
/// compiled-in constants (`resolve_builtin_input_schema_ref` in
/// `ironclaw_host_runtime`). Returns `None` for a ref with the wrong prefix, an
/// unknown op name, or a reserved op (no contract to resolve against).
pub fn resolve_standard_schema_ref(schema_ref: &str) -> Option<&'static str> {
    let suffix = schema_ref.strip_prefix(STANDARD_SCHEMA_REF_PREFIX)?;
    if let Some(op_name) = suffix.strip_suffix(".input.v1") {
        return StandardMessagingOp::ALL
            .iter()
            .find(|op| op.op_name() == op_name)
            .and_then(StandardMessagingOp::contract)
            .map(|contract| contract.input_schema);
    }
    if let Some(op_name) = suffix.strip_suffix(".output.v1") {
        return StandardMessagingOp::ALL
            .iter()
            .find(|op| op.op_name() == op_name)
            .and_then(StandardMessagingOp::contract)
            .map(|contract| contract.output_schema);
    }
    None
}

/// The closed standard messaging error-code vocabulary (spec §8). Adapters map
/// vendor errors to these codes once; anything unmapped falls to
/// [`StandardMessagingErrorCode::VendorError`] with the sanitized vendor code
/// carried in the detail. Credential problems (revoked/missing tokens) are not
/// part of this taxonomy — they keep riding the existing `AuthRequired`
/// re-auth gate path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StandardMessagingErrorCode {
    /// Conversation ref doesn't resolve on this extension (invalid input).
    UnknownConversation,
    /// Message ref doesn't resolve (invalid input).
    UnknownMessage,
    /// User ref doesn't resolve (invalid input).
    UnknownUser,
    /// Caller's identity isn't in the conversation (denied, vendor).
    NotAMember,
    /// Vendor-side authz failure, e.g. missing scope or role (denied, vendor).
    PermissionDenied,
    /// DMs closed or the target blocked the sender (denied, vendor).
    CannotMessageUser,
    /// The recipient is reachable, but free-form messaging is closed right
    /// now (e.g. a vendor session-window policy) — denied, vendor. Distinct
    /// from `CannotMessageUser`: nothing is permanently blocked, so a
    /// template/re-engagement message or waiting may still succeed. Never
    /// teaches the model to give up the way folding this into
    /// `CannotMessageUser` would.
    OutsideMessagingWindow,
    /// Over the vendor's message length limit (invalid input).
    MessageTooLong,
    /// Content the vendor can't render (invalid input).
    UnsupportedContent,
    /// Vendor rate limit hit (retryable).
    RateLimited,
    /// Not the caller's own message, or the edit window is over (denied, vendor).
    EditNotAllowed,
    /// Anything else; the closed vocabulary's catch-all (backend). The
    /// sanitized vendor code does NOT ride this taxonomy channel to the
    /// model on any landed implementation — WASM guests carry only the
    /// canonical `messaging.*` string in their structured `{code, kind}`
    /// error, and first-party (non-WASM) adapters follow the same
    /// convention for parity, not because the shape forces it.
    /// Server-side visibility into the original vendor error, where it
    /// exists, comes from the host's own egress/response logging, not from
    /// this taxonomy (see `standard-operations.md` §5.1).
    VendorError,
}

impl StandardMessagingErrorCode {
    pub const ALL: &'static [StandardMessagingErrorCode] = &[
        Self::UnknownConversation,
        Self::UnknownMessage,
        Self::UnknownUser,
        Self::NotAMember,
        Self::PermissionDenied,
        Self::CannotMessageUser,
        Self::OutsideMessagingWindow,
        Self::MessageTooLong,
        Self::UnsupportedContent,
        Self::RateLimited,
        Self::EditNotAllowed,
        Self::VendorError,
    ];

    /// The `messaging.*`-namespaced wire code carried on the structured WASM
    /// guest error channel (`{code, kind}`) or a first-party `ToolError::Failed`
    /// safe summary.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::UnknownConversation => "messaging.unknown_conversation",
            Self::UnknownMessage => "messaging.unknown_message",
            Self::UnknownUser => "messaging.unknown_user",
            Self::NotAMember => "messaging.not_a_member",
            Self::PermissionDenied => "messaging.permission_denied",
            Self::CannotMessageUser => "messaging.cannot_message_user",
            Self::OutsideMessagingWindow => "messaging.outside_messaging_window",
            Self::MessageTooLong => "messaging.message_too_long",
            Self::UnsupportedContent => "messaging.unsupported_content",
            Self::RateLimited => "messaging.rate_limited",
            Self::EditNotAllowed => "messaging.edit_not_allowed",
            Self::VendorError => "messaging.vendor_error",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn validator_for_contract(op: StandardMessagingOp, output: bool) -> jsonschema::Validator {
        let contract = op.contract().expect("core operation has a contract");
        let raw = if output {
            contract.output_schema
        } else {
            contract.input_schema
        };
        let schema: serde_json::Value = serde_json::from_str(raw).expect("schema parses");
        jsonschema::options()
            .should_validate_formats(true)
            .build(&schema)
            .expect("schema compiles")
    }

    #[test]
    fn op_names_round_trip_snake_case_serde() {
        for op in StandardMessagingOp::ALL {
            let token = serde_json::to_value(op).expect("serializes");
            assert_eq!(token, serde_json::Value::String(op.op_name().to_string()));
            let back: StandardMessagingOp = serde_json::from_value(token).expect("deserializes");
            assert_eq!(back, *op);
        }
    }

    #[test]
    fn sixteen_core_ops_have_complete_contracts() {
        let core: Vec<_> = StandardMessagingOp::ALL
            .iter()
            .filter(|op| op.contract().is_some())
            .collect();
        assert_eq!(core.len(), 16, "exactly the 16 core ops carry contracts");
        for op in core {
            let contract = op.contract().expect("core contract");
            for (label, schema) in [
                ("input", contract.input_schema),
                ("output", contract.output_schema),
            ] {
                let parsed: serde_json::Value = serde_json::from_str(schema)
                    .unwrap_or_else(|e| panic!("{} {label} schema parses: {e}", op.op_name()));
                jsonschema::options()
                    .should_validate_formats(true)
                    .build(&parsed)
                    .unwrap_or_else(|e| panic!("{} {label} schema compiles: {e}", op.op_name()));
            }
            assert!(
                !contract.description_core.trim().is_empty(),
                "{} core",
                op.op_name()
            );
            assert_eq!(contract.is_write, op.is_write());
        }
    }

    #[test]
    fn reserved_ops_have_no_contract() {
        assert!(StandardMessagingOp::ForwardMessage.contract().is_none());
        assert!(
            StandardMessagingOp::ArchiveConversation
                .contract()
                .is_none()
        );
        assert_eq!(
            StandardMessagingOp::ALL
                .iter()
                .filter(|op| op.contract().is_none())
                .count(),
            13
        );
    }

    #[test]
    fn standard_schema_refs_resolve() {
        let input = resolve_standard_schema_ref("standard:messaging/send_message.input.v1")
            .expect("send input resolves");
        let input: serde_json::Value = serde_json::from_str(input).expect("input schema parses");
        assert!(input["properties"].get("conversation").is_some());
        assert!(
            input["required"]
                .as_array()
                .is_some_and(|required| required.contains(&serde_json::json!("conversation")))
        );

        let output = resolve_standard_schema_ref("standard:messaging/send_message.output.v1")
            .expect("send output resolves");
        let output: serde_json::Value = serde_json::from_str(output).expect("output schema parses");
        assert!(output["properties"].get("message_ref").is_some());
        assert!(
            output["required"]
                .as_array()
                .is_some_and(|required| required.contains(&serde_json::json!("message_ref")))
        );

        assert!(resolve_standard_schema_ref("standard:messaging/nope.input.v1").is_none());
        assert!(resolve_standard_schema_ref("standard:messaging/send_message.v1").is_none());
        assert!(resolve_standard_schema_ref("schemas/slack/x.json").is_none());
    }

    #[test]
    fn canonical_inputs_reject_empty_references_and_pagination_values() {
        let add_reaction = validator_for_contract(StandardMessagingOp::AddReaction, false);
        assert!(!add_reaction.is_valid(&serde_json::json!({
            "message_ref": { "conversation": "", "message_id": "" },
            "emoji": "thumbsup"
        })));

        for op in [
            StandardMessagingOp::GetConversationHistory,
            StandardMessagingOp::GetThreadReplies,
            StandardMessagingOp::ListConversations,
            StandardMessagingOp::ListMembers,
            StandardMessagingOp::ResolveUser,
            StandardMessagingOp::SearchMessages,
        ] {
            let validator = validator_for_contract(op, false);
            let mut input = match op {
                StandardMessagingOp::GetConversationHistory | StandardMessagingOp::ListMembers => {
                    serde_json::json!({ "conversation": "C1" })
                }
                StandardMessagingOp::GetThreadReplies => {
                    serde_json::json!({ "conversation": "C1", "thread": "T1" })
                }
                StandardMessagingOp::ResolveUser | StandardMessagingOp::SearchMessages => {
                    serde_json::json!({ "query": "alice" })
                }
                StandardMessagingOp::ListConversations => serde_json::json!({}),
                _ => unreachable!("loop contains only paginated core operations"),
            };
            input["cursor"] = serde_json::json!("");
            assert!(
                !validator.is_valid(&input),
                "{} accepted an empty cursor",
                op.op_name()
            );

            input.as_object_mut().expect("object").remove("cursor");
            input["limit"] = serde_json::json!(1_001);
            assert!(
                !validator.is_valid(&input),
                "{} accepted a page larger than the host maximum",
                op.op_name()
            );
        }

        let list = validator_for_contract(StandardMessagingOp::ListConversations, false);
        assert!(!list.is_valid(&serde_json::json!({ "kinds": [] })));
    }

    #[test]
    fn canonical_message_timestamps_enforce_rfc3339() {
        for op in [
            StandardMessagingOp::GetConversationHistory,
            StandardMessagingOp::GetThreadReplies,
            StandardMessagingOp::GetMessage,
            StandardMessagingOp::SearchMessages,
        ] {
            let validator = validator_for_contract(op, true);
            let message = serde_json::json!({
                "message_ref": { "conversation": "C1", "message_id": "M1" },
                "author": { "user_ref": "U1" },
                "text": "hello",
                "timestamp": "yesterday",
                "is_self": false
            });
            let output = if op == StandardMessagingOp::GetMessage {
                serde_json::json!({ "message": message })
            } else if op == StandardMessagingOp::SearchMessages {
                serde_json::json!({ "matches": [message] })
            } else {
                serde_json::json!({ "messages": [message] })
            };
            assert!(
                !validator.is_valid(&output),
                "{} accepted a non-RFC3339 timestamp",
                op.op_name()
            );
        }
    }

    #[test]
    fn direct_conversations_require_counterpart_identity() {
        let validator = validator_for_contract(StandardMessagingOp::ListConversations, true);
        assert!(!validator.is_valid(&serde_json::json!({
            "conversations": [{ "conversation": "D1", "kind": "dm" }]
        })));
        assert!(validator.is_valid(&serde_json::json!({
            "conversations": [{
                "conversation": "D1",
                "kind": "dm",
                "counterpart": { "user_ref": "U1" }
            }]
        })));
    }

    #[test]
    fn error_codes_are_namespaced() {
        for code in StandardMessagingErrorCode::ALL {
            assert!(code.as_str().starts_with("messaging."), "{}", code.as_str());
        }
        // W6 (pre-merge amendment wave): OutsideMessagingWindow brought the
        // vocabulary from 11 to 12 codes.
        assert_eq!(StandardMessagingErrorCode::ALL.len(), 12);
    }

    #[test]
    fn write_output_schemas_require_evidence() {
        for op in [
            StandardMessagingOp::SendMessage,
            StandardMessagingOp::EditMessage,
        ] {
            let schema: serde_json::Value =
                serde_json::from_str(op.contract().unwrap().output_schema).unwrap();
            let required = schema["required"].as_array().unwrap();
            assert!(
                required.iter().any(|r| r == "message_ref"),
                "{}",
                op.op_name()
            );
        }

        // W2 (pre-merge amendment wave): `{"message_ref": {"conversation":
        // "", "message_id": ""}}` satisfies `required: ["message_ref"]`
        // above, but is the exact silent-`ts:""`-class evidence the standard
        // exists to kill. Every identity-bearing string in every one of the
        // 16 core ops' output schemas must additionally carry `minLength: 1`
        // — swept by walking each schema's own structure (not a hand-listed
        // set of JSON paths), so new nesting is covered for free.
        for op in StandardMessagingOp::ALL
            .iter()
            .filter(|op| op.contract().is_some())
        {
            let schema: serde_json::Value =
                serde_json::from_str(op.contract().unwrap().output_schema).unwrap();
            assert_identity_fields_require_min_length(op.op_name(), &schema, "root");
        }
    }

    /// Identity-bearing output string properties whose emptiness would fake
    /// evidence that an operation succeeded — the standard's own nouns
    /// (`conversation`, `message_id`, `user_ref`, the `thread` anchor,
    /// `emoji`) plus the opaque pagination `next_cursor`. Deliberately NOT
    /// `display_name`/`text`/`real_name`/`status_text`/`title`/`timezone`/
    /// `kind` — those are presentation or vendor-content fields, not proof
    /// that a write happened or an identity resolved.
    const IDENTITY_BEARING_OUTPUT_STRING_FIELDS: &[&str] = &[
        "conversation",
        "message_id",
        "user_ref",
        "thread",
        "emoji",
        "next_cursor",
    ];

    /// Recursively asserts that every property of `node` (and, transitively,
    /// every property nested under `properties`/array `items`) named in
    /// [`IDENTITY_BEARING_OUTPUT_STRING_FIELDS`] with a declared
    /// `"type": "string"` carries `"minLength": 1`. A loop over the schema's
    /// own tree, not a hand-listed set of JSON paths.
    fn assert_identity_fields_require_min_length(
        op_name: &str,
        node: &serde_json::Value,
        path: &str,
    ) {
        if let Some(properties) = node
            .get("properties")
            .and_then(serde_json::Value::as_object)
        {
            for (name, property) in properties {
                let property_path = format!("{path}.{name}");
                if IDENTITY_BEARING_OUTPUT_STRING_FIELDS.contains(&name.as_str())
                    && property.get("type").and_then(serde_json::Value::as_str) == Some("string")
                {
                    assert_eq!(
                        property
                            .get("minLength")
                            .and_then(serde_json::Value::as_u64),
                        Some(1),
                        "{op_name}: identity-bearing field {property_path} must carry \
                         minLength: 1 (an empty string here would fake evidence of success)"
                    );
                }
                assert_identity_fields_require_min_length(op_name, property, &property_path);
            }
        }
        if let Some(items) = node.get("items") {
            assert_identity_fields_require_min_length(op_name, items, &format!("{path}[]"));
        }
    }
}
