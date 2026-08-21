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

/// One published version of a canonical schema. Published schema files are
/// **immutable**: a shape change ships as a new version file, never an
/// in-place edit, and every version that was ever published keeps resolving
/// forever so a binding that pinned it can never silently re-resolve to a
/// different shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StandardSchemaVersion {
    V1,
    V2,
}

impl StandardSchemaVersion {
    /// Every version this crate knows how to name, oldest first.
    pub const ALL: &'static [StandardSchemaVersion] =
        &[StandardSchemaVersion::V1, StandardSchemaVersion::V2];

    /// The trailing segment a canonical schema ref carries
    /// (`standard:messaging/send_message.output.v2` → `"v2"`).
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::V1 => "v1",
            Self::V2 => "v2",
        }
    }

    /// Parses the trailing version segment of a canonical schema ref. `None`
    /// for anything this crate never published — the ref is then unresolvable
    /// rather than silently downgraded to a version that does exist.
    pub fn from_wire(value: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|version| version.as_str() == value)
    }
}

/// Which half of an operation's contract a canonical schema ref names.
/// Crate-internal: outside callers never spell a ref, they mint one from a
/// typed operation through [`crate::capability_profile::CapabilityProfileSchemaRef`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum StandardSchemaDirection {
    Input,
    Output,
}

impl StandardSchemaDirection {
    pub(crate) const fn as_str(&self) -> &'static str {
        match self {
            Self::Input => "input",
            Self::Output => "output",
        }
    }

    fn from_wire(value: &str) -> Option<Self> {
        match value {
            "input" => Some(Self::Input),
            "output" => Some(Self::Output),
            _ => None,
        }
    }
}

/// One superseded-but-still-served published schema, kept beside the current
/// one so its ref resolves forever (see [`StandardSchemaVersion`]).
#[derive(Debug, Clone, Copy)]
pub struct PublishedSchema {
    pub version: StandardSchemaVersion,
    pub schema: &'static str,
}

/// The canonical contract for one core standard messaging operation: its
/// input/output JSON Schema (draft-07, `include_str!`-compiled from
/// `schemas/messaging/`) and its model-facing description core
/// (`include_str!`-compiled from `prompts/messaging/`). Composed at descriptor
/// build with the extension's vendor addendum (spec §5.3).
///
/// Output schemas are **version-plural**: `output_schema` is the current
/// version — the one a new binding pins and the one the runtime validator
/// enforces — and `superseded_output_schemas` holds every earlier published
/// version so existing bindings keep resolving exactly the shape they pinned.
#[derive(Debug, Clone, Copy)]
pub struct StandardOpContract {
    pub op: StandardMessagingOp,
    /// The canonical input schema at [`StandardOpContract::INPUT_SCHEMA_VERSION`].
    pub input_schema: &'static str,
    /// The canonical output schema at `output_schema_version`.
    pub output_schema: &'static str,
    /// The version `output_schema` was published under — the version
    /// [`crate::capability_profile::CapabilityProfileSchemaRef::standard_messaging_output`]
    /// mints for a new binding.
    pub output_schema_version: StandardSchemaVersion,
    /// Output schema versions this op published and has since superseded,
    /// oldest first. Empty for every op whose shape has never changed.
    pub superseded_output_schemas: &'static [PublishedSchema],
    pub description_core: &'static str,
    pub is_write: bool,
}

impl StandardOpContract {
    /// Every canonical *input* schema published so far is `v1`; no input shape
    /// has graduated, so there is one version rather than a per-op field. When
    /// the first input graduates, give it an `input_schema_version` +
    /// `superseded_input_schemas` pair mirroring its output siblings — do not
    /// edit a published `.input.v1` file in place.
    pub const INPUT_SCHEMA_VERSION: StandardSchemaVersion = StandardSchemaVersion::V1;

    /// The input schema published under `version`, or `None` when this op
    /// never published that version.
    pub fn input_schema_for(&self, version: StandardSchemaVersion) -> Option<&'static str> {
        (version == Self::INPUT_SCHEMA_VERSION).then_some(self.input_schema)
    }

    /// The output schema published under `version`, or `None` when this op
    /// never published that version. Both the current version and every
    /// superseded one resolve.
    pub fn output_schema_for(&self, version: StandardSchemaVersion) -> Option<&'static str> {
        if version == self.output_schema_version {
            return Some(self.output_schema);
        }
        self.superseded_output_schemas
            .iter()
            .find(|published| published.version == version)
            .map(|published| published.schema)
    }

    /// Every published output schema version, oldest first (superseded first,
    /// current last).
    pub fn published_output_schemas(&self) -> impl Iterator<Item = PublishedSchema> + '_ {
        self.superseded_output_schemas
            .iter()
            .copied()
            .chain(std::iter::once(PublishedSchema {
                version: self.output_schema_version,
                schema: self.output_schema,
            }))
    }
}

/// `send_message` is the one op whose output shape has graduated: `.v2` adds
/// the `sent_unverified` evidence branch for a send the provider accepted but
/// could not correlate to a message identity. `.v1` is untouched and keeps
/// resolving for every binding that pinned it; `.v2` is a strict superset of
/// `.v1` (pinned by `send_message_v2_accepts_every_v1_valid_output`), which is
/// what makes the op-keyed runtime validator in `ironclaw_host_runtime` safe
/// to graduate wholesale.
static SEND_MESSAGE_CONTRACT: StandardOpContract = StandardOpContract {
    op: StandardMessagingOp::SendMessage,
    input_schema: include_str!("../schemas/messaging/send_message.input.v1.json"),
    output_schema: include_str!("../schemas/messaging/send_message.output.v2.json"),
    output_schema_version: StandardSchemaVersion::V2,
    superseded_output_schemas: &[PublishedSchema {
        version: StandardSchemaVersion::V1,
        schema: include_str!("../schemas/messaging/send_message.output.v1.json"),
    }],
    description_core: include_str!("../prompts/messaging/send_message.core.md"),
    is_write: true,
};

static EDIT_MESSAGE_CONTRACT: StandardOpContract = StandardOpContract {
    op: StandardMessagingOp::EditMessage,
    input_schema: include_str!("../schemas/messaging/edit_message.input.v1.json"),
    output_schema: include_str!("../schemas/messaging/edit_message.output.v1.json"),
    output_schema_version: StandardSchemaVersion::V1,
    superseded_output_schemas: &[],
    description_core: include_str!("../prompts/messaging/edit_message.core.md"),
    is_write: true,
};

static DELETE_MESSAGE_CONTRACT: StandardOpContract = StandardOpContract {
    op: StandardMessagingOp::DeleteMessage,
    input_schema: include_str!("../schemas/messaging/delete_message.input.v1.json"),
    output_schema: include_str!("../schemas/messaging/delete_message.output.v1.json"),
    output_schema_version: StandardSchemaVersion::V1,
    superseded_output_schemas: &[],
    description_core: include_str!("../prompts/messaging/delete_message.core.md"),
    is_write: true,
};

static ADD_REACTION_CONTRACT: StandardOpContract = StandardOpContract {
    op: StandardMessagingOp::AddReaction,
    input_schema: include_str!("../schemas/messaging/add_reaction.input.v1.json"),
    output_schema: include_str!("../schemas/messaging/add_reaction.output.v1.json"),
    output_schema_version: StandardSchemaVersion::V1,
    superseded_output_schemas: &[],
    description_core: include_str!("../prompts/messaging/add_reaction.core.md"),
    is_write: true,
};

static REMOVE_REACTION_CONTRACT: StandardOpContract = StandardOpContract {
    op: StandardMessagingOp::RemoveReaction,
    input_schema: include_str!("../schemas/messaging/remove_reaction.input.v1.json"),
    output_schema: include_str!("../schemas/messaging/remove_reaction.output.v1.json"),
    output_schema_version: StandardSchemaVersion::V1,
    superseded_output_schemas: &[],
    description_core: include_str!("../prompts/messaging/remove_reaction.core.md"),
    is_write: true,
};

static OPEN_DM_CONTRACT: StandardOpContract = StandardOpContract {
    op: StandardMessagingOp::OpenDm,
    input_schema: include_str!("../schemas/messaging/open_dm.input.v1.json"),
    output_schema: include_str!("../schemas/messaging/open_dm.output.v1.json"),
    output_schema_version: StandardSchemaVersion::V1,
    superseded_output_schemas: &[],
    description_core: include_str!("../prompts/messaging/open_dm.core.md"),
    is_write: true,
};

static LIST_CONVERSATIONS_CONTRACT: StandardOpContract = StandardOpContract {
    op: StandardMessagingOp::ListConversations,
    input_schema: include_str!("../schemas/messaging/list_conversations.input.v1.json"),
    output_schema: include_str!("../schemas/messaging/list_conversations.output.v1.json"),
    output_schema_version: StandardSchemaVersion::V1,
    superseded_output_schemas: &[],
    description_core: include_str!("../prompts/messaging/list_conversations.core.md"),
    is_write: false,
};

static GET_CONVERSATION_INFO_CONTRACT: StandardOpContract = StandardOpContract {
    op: StandardMessagingOp::GetConversationInfo,
    input_schema: include_str!("../schemas/messaging/get_conversation_info.input.v1.json"),
    output_schema: include_str!("../schemas/messaging/get_conversation_info.output.v1.json"),
    output_schema_version: StandardSchemaVersion::V1,
    superseded_output_schemas: &[],
    description_core: include_str!("../prompts/messaging/get_conversation_info.core.md"),
    is_write: false,
};

static GET_CONVERSATION_HISTORY_CONTRACT: StandardOpContract = StandardOpContract {
    op: StandardMessagingOp::GetConversationHistory,
    input_schema: include_str!("../schemas/messaging/get_conversation_history.input.v1.json"),
    output_schema: include_str!("../schemas/messaging/get_conversation_history.output.v1.json"),
    output_schema_version: StandardSchemaVersion::V1,
    superseded_output_schemas: &[],
    description_core: include_str!("../prompts/messaging/get_conversation_history.core.md"),
    is_write: false,
};

static GET_THREAD_REPLIES_CONTRACT: StandardOpContract = StandardOpContract {
    op: StandardMessagingOp::GetThreadReplies,
    input_schema: include_str!("../schemas/messaging/get_thread_replies.input.v1.json"),
    output_schema: include_str!("../schemas/messaging/get_thread_replies.output.v1.json"),
    output_schema_version: StandardSchemaVersion::V1,
    superseded_output_schemas: &[],
    description_core: include_str!("../prompts/messaging/get_thread_replies.core.md"),
    is_write: false,
};

static GET_MESSAGE_CONTRACT: StandardOpContract = StandardOpContract {
    op: StandardMessagingOp::GetMessage,
    input_schema: include_str!("../schemas/messaging/get_message.input.v1.json"),
    output_schema: include_str!("../schemas/messaging/get_message.output.v1.json"),
    output_schema_version: StandardSchemaVersion::V1,
    superseded_output_schemas: &[],
    description_core: include_str!("../prompts/messaging/get_message.core.md"),
    is_write: false,
};

static SEARCH_MESSAGES_CONTRACT: StandardOpContract = StandardOpContract {
    op: StandardMessagingOp::SearchMessages,
    input_schema: include_str!("../schemas/messaging/search_messages.input.v1.json"),
    output_schema: include_str!("../schemas/messaging/search_messages.output.v1.json"),
    output_schema_version: StandardSchemaVersion::V1,
    superseded_output_schemas: &[],
    description_core: include_str!("../prompts/messaging/search_messages.core.md"),
    is_write: false,
};

static GET_USER_INFO_CONTRACT: StandardOpContract = StandardOpContract {
    op: StandardMessagingOp::GetUserInfo,
    input_schema: include_str!("../schemas/messaging/get_user_info.input.v1.json"),
    output_schema: include_str!("../schemas/messaging/get_user_info.output.v1.json"),
    output_schema_version: StandardSchemaVersion::V1,
    superseded_output_schemas: &[],
    description_core: include_str!("../prompts/messaging/get_user_info.core.md"),
    is_write: false,
};

static RESOLVE_USER_CONTRACT: StandardOpContract = StandardOpContract {
    op: StandardMessagingOp::ResolveUser,
    input_schema: include_str!("../schemas/messaging/resolve_user.input.v1.json"),
    output_schema: include_str!("../schemas/messaging/resolve_user.output.v1.json"),
    output_schema_version: StandardSchemaVersion::V1,
    superseded_output_schemas: &[],
    description_core: include_str!("../prompts/messaging/resolve_user.core.md"),
    is_write: false,
};

static LIST_MEMBERS_CONTRACT: StandardOpContract = StandardOpContract {
    op: StandardMessagingOp::ListMembers,
    input_schema: include_str!("../schemas/messaging/list_members.input.v1.json"),
    output_schema: include_str!("../schemas/messaging/list_members.output.v1.json"),
    output_schema_version: StandardSchemaVersion::V1,
    superseded_output_schemas: &[],
    description_core: include_str!("../prompts/messaging/list_members.core.md"),
    is_write: false,
};

static WHOAMI_CONTRACT: StandardOpContract = StandardOpContract {
    op: StandardMessagingOp::Whoami,
    input_schema: include_str!("../schemas/messaging/whoami.input.v1.json"),
    output_schema: include_str!("../schemas/messaging/whoami.output.v1.json"),
    output_schema_version: StandardSchemaVersion::V1,
    superseded_output_schemas: &[],
    description_core: include_str!("../prompts/messaging/whoami.core.md"),
    is_write: false,
};

/// Prefix for a standard-op schema ref, as synthesized at manifest parse time
/// (spec §6): `standard:messaging/<op_name>.<input|output>.<version>`, e.g.
/// `standard:messaging/send_message.input.v1` /
/// `standard:messaging/send_message.output.v2`.
pub const STANDARD_SCHEMA_REF_PREFIX: &str = "standard:messaging/";

/// Resolve a `standard:messaging/...` schema ref to its compiled-in canonical
/// JSON Schema text, the way builtin schema refs already resolve from
/// compiled-in constants (`resolve_builtin_input_schema_ref` in
/// `ironclaw_host_runtime`). Returns `None` for a ref with the wrong prefix, a
/// malformed suffix, an unknown op name, a reserved op (no contract to resolve
/// against), or a version that op never published.
///
/// **Every published version resolves forever.** A binding that pinned
/// `send_message.output.v1` keeps getting the exact `.v1` text after the op
/// graduated to `.v2`; that is the whole reason a shape change ships as a new
/// file instead of an in-place edit.
pub fn resolve_standard_schema_ref(schema_ref: &str) -> Option<&'static str> {
    let suffix = schema_ref.strip_prefix(STANDARD_SCHEMA_REF_PREFIX)?;
    // `<op_name>.<direction>.<version>`; op names carry underscores, never
    // dots, so splitting from the right cannot eat part of one.
    let (op_and_direction, version) = suffix.rsplit_once('.')?;
    let version = StandardSchemaVersion::from_wire(version)?;
    let (op_name, direction) = op_and_direction.rsplit_once('.')?;
    let direction = StandardSchemaDirection::from_wire(direction)?;
    let contract = StandardMessagingOp::ALL
        .iter()
        .find(|op| op.op_name() == op_name)
        .and_then(StandardMessagingOp::contract)?;
    match direction {
        StandardSchemaDirection::Input => contract.input_schema_for(version),
        StandardSchemaDirection::Output => contract.output_schema_for(version),
    }
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

    /// The `messaging.*`-namespaced wire code carried by the structured WASM
    /// guest failure or a first-party `ToolError::Rejected` provider
    /// diagnostic. The host recognizes only exact members of [`Self::ALL`].
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
            // Every *published* schema must parse and compile, not only the
            // current one: a superseded version still resolves for every
            // binding that pinned it, so a broken `.v1` is a live defect.
            let published = std::iter::once(("input.v1".to_string(), contract.input_schema)).chain(
                contract.published_output_schemas().map(|published| {
                    (
                        format!("output.{}", published.version.as_str()),
                        published.schema,
                    )
                }),
            );
            for (label, schema) in published {
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

        // `.v1` keeps resolving to the exact shape it always had, unchanged by
        // the `.v2` graduation: `message_ref` required, single-branch.
        let output = resolve_standard_schema_ref("standard:messaging/send_message.output.v1")
            .expect("send output v1 resolves");
        let output: serde_json::Value = serde_json::from_str(output).expect("output schema parses");
        assert!(output["properties"].get("message_ref").is_some());
        assert!(
            output["required"]
                .as_array()
                .is_some_and(|required| required.contains(&serde_json::json!("message_ref")))
        );
        assert!(
            output["properties"].get("sent_unverified").is_none(),
            "`.v1` is immutable — the graduation must not have leaked into it"
        );

        // `.v2` resolves alongside it, and is what a new binding pins.
        let v2 = resolve_standard_schema_ref("standard:messaging/send_message.output.v2")
            .expect("send output v2 resolves");
        let v2: serde_json::Value = serde_json::from_str(v2).expect("output schema parses");
        assert!(v2["properties"].get("sent_unverified").is_some());
        assert_eq!(
            crate::capability_profile::CapabilityProfileSchemaRef::standard_messaging_output(
                StandardMessagingOp::SendMessage
            )
            .expect("send_message has a canonical output ref")
            .as_str(),
            "standard:messaging/send_message.output.v2",
            "a new binding pins the op's current version"
        );

        // A version an op never published does not resolve — it is not
        // silently served the nearest one that exists.
        assert!(resolve_standard_schema_ref("standard:messaging/edit_message.output.v2").is_none());
        assert!(resolve_standard_schema_ref("standard:messaging/send_message.input.v2").is_none());

        assert!(resolve_standard_schema_ref("standard:messaging/nope.input.v1").is_none());
        assert!(resolve_standard_schema_ref("standard:messaging/send_message.v1").is_none());
        assert!(resolve_standard_schema_ref("standard:messaging/send_message.output.v3").is_none());
        assert!(
            resolve_standard_schema_ref("standard:messaging/send_message.sideways.v1").is_none()
        );
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
        // `edit_message` proves the write with one required `message_ref`.
        // So did `send_message` until its `.v2` graduation, and its `.v1` —
        // which every already-installed binding still pins — must still say
        // exactly that.
        for (op, schema) in [
            (
                StandardMessagingOp::EditMessage,
                StandardMessagingOp::EditMessage
                    .contract()
                    .unwrap()
                    .output_schema,
            ),
            (
                StandardMessagingOp::SendMessage,
                StandardMessagingOp::SendMessage
                    .contract()
                    .unwrap()
                    .output_schema_for(StandardSchemaVersion::V1)
                    .expect("send_message keeps publishing .v1 forever"),
            ),
        ] {
            let schema: serde_json::Value = serde_json::from_str(schema).unwrap();
            let required = schema["required"].as_array().unwrap();
            assert!(
                required.iter().any(|r| r == "message_ref"),
                "{}",
                op.op_name()
            );
        }

        // `send_message.output.v2` widens evidence to a CLOSED disjunction
        // (design §6.2): a provider-issued `message_ref`, or the explicit
        // `sent_unverified` marker for a send the provider accepted but could
        // not correlate. Both branches are required-bearing — there is no
        // third, evidence-free way to report a completed send, which is the
        // property the single `required` above used to carry alone.
        let send_v2: serde_json::Value =
            serde_json::from_str(SEND_MESSAGE_CONTRACT.output_schema).unwrap();
        assert!(
            send_v2.get("required").is_none(),
            "v2 states evidence as `oneOf`; a leftover top-level `required` would \
             silently re-narrow one branch"
        );
        let branches: Vec<Vec<&str>> = send_v2["oneOf"]
            .as_array()
            .expect("v2 states evidence as a oneOf disjunction")
            .iter()
            .map(|branch| {
                branch["required"]
                    .as_array()
                    .unwrap_or_else(|| panic!("every evidence branch requires a field: {branch}"))
                    .iter()
                    .map(|field| field.as_str().expect("required entries are strings"))
                    .collect()
            })
            .collect();
        assert_eq!(
            branches,
            vec![vec!["message_ref"], vec!["sent_unverified"]],
            "the evidence disjunction is closed at exactly these two branches"
        );

        // W2 (pre-merge amendment wave): `{"message_ref": {"conversation":
        // "", "message_id": ""}}` satisfies `required: ["message_ref"]`
        // above, but is the exact silent-`ts:""`-class evidence the standard
        // exists to kill. Every identity-bearing string in every one of the
        // 16 core ops' output schemas must additionally carry `minLength: 1`
        // — swept by walking each schema's own structure (not a hand-listed
        // set of JSON paths), so new nesting is covered for free. The sweep
        // covers EVERY published version, so a graduation cannot smuggle an
        // unbounded identity field into a new file.
        for op in StandardMessagingOp::ALL
            .iter()
            .filter(|op| op.contract().is_some())
        {
            for published in op.contract().unwrap().published_output_schemas() {
                let schema: serde_json::Value = serde_json::from_str(published.schema).unwrap();
                assert_identity_fields_require_min_length(
                    &format!("{}.output.{}", op.op_name(), published.version.as_str()),
                    &schema,
                    "root",
                );
            }
        }
    }

    /// The `.v2` graduation (design §6.2), at the validator rather than the
    /// schema text: the `sent_unverified` branch is accepted, an output
    /// carrying NEITHER branch is still a violation, and `.v2` is a strict
    /// superset of `.v1`.
    ///
    /// The superset property is load-bearing, not decorative: the runtime
    /// validator (`standard_op_output.rs`) keys `VALIDATORS` by *op*, not by
    /// version, so the single compiled `.v2` schema enforces every binding
    /// including the ones that pinned `.v1`. If `.v2` ever rejected something
    /// `.v1` allowed, that would fail Slack's and acme's already-shipped
    /// adapters at dispatch time without either package changing a line.
    #[test]
    fn send_message_v2_accepts_every_v1_valid_output() {
        let v1 = compile(
            SEND_MESSAGE_CONTRACT
                .output_schema_for(StandardSchemaVersion::V1)
                .expect("v1 stays published"),
        );
        let v2 = compile(SEND_MESSAGE_CONTRACT.output_schema);
        assert_eq!(
            SEND_MESSAGE_CONTRACT.output_schema_version,
            StandardSchemaVersion::V2,
            "send_message's current output version is what the op-keyed runtime validator compiles"
        );

        // Superset: every optional-field combination a `.v1` adapter can emit
        // is still valid under `.v2`. Walked as the power set of the optional
        // fields rather than a hand-picked example, so a `.v2` that narrowed
        // any one of them fails here.
        let message_ref = serde_json::json!({ "conversation": "C1", "message_id": "M1" });
        let optional = [
            ("thread", serde_json::json!("T1")),
            ("reply_to", message_ref.clone()),
            ("vendor", serde_json::json!({ "ts": "1.2" })),
        ];
        for mask in 0..(1u8 << optional.len()) {
            let mut output = serde_json::json!({ "message_ref": message_ref.clone() });
            for (index, (name, value)) in optional.iter().enumerate() {
                if mask & (1 << index) != 0 {
                    output[*name] = value.clone();
                }
            }
            assert!(
                v1.is_valid(&output),
                "v1 must accept its own shape: {output}"
            );
            assert!(
                v2.is_valid(&output),
                "v2 must accept every v1-valid output: {output}"
            );
        }

        // The new branch.
        assert!(v2.is_valid(&serde_json::json!({ "sent_unverified": true })));
        assert!(v2.is_valid(&serde_json::json!({
            "sent_unverified": true,
            "thread": "T1",
            "vendor": { "peer": "1" }
        })));
        assert!(
            !v1.is_valid(&serde_json::json!({ "sent_unverified": true })),
            "the branch is exactly what v1 could not express — that is why v2 exists"
        );

        // Neither branch present is STILL a violation: the graduation widened
        // what counts as evidence, it did not make evidence optional.
        for evidence_free in [
            serde_json::json!({}),
            serde_json::json!({ "thread": "T1" }),
            serde_json::json!({ "vendor": { "ts": "1.2" } }),
            // `sent_unverified` is `const: true`; a false marker is not a
            // second way to say "no evidence".
            serde_json::json!({ "sent_unverified": false }),
        ] {
            assert!(
                !v2.is_valid(&evidence_free),
                "v2 accepted an output with no evidence branch: {evidence_free}"
            );
        }

        // The branches are mutually exclusive: a correlated send carries a
        // ref, an uncorrelated one carries the marker, never both.
        assert!(!v2.is_valid(&serde_json::json!({
            "message_ref": message_ref,
            "sent_unverified": true
        })));

        // Still closed: `sent_unverified` did not open the object up.
        assert!(!v2.is_valid(&serde_json::json!({
            "sent_unverified": true,
            "unknown_field": 1
        })));

        // Empty identity strings still fake nothing under v2.
        assert!(!v2.is_valid(&serde_json::json!({
            "message_ref": { "conversation": "", "message_id": "" }
        })));
    }

    fn compile(raw: &str) -> jsonschema::Validator {
        let schema: serde_json::Value = serde_json::from_str(raw).expect("schema parses");
        jsonschema::options()
            .should_validate_formats(true)
            .build(&schema)
            .expect("schema compiles")
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
