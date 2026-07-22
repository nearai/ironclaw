//! Slack reply-target binding-ref grammar + the preference-target codec.
//!
//! The binding-ref grammar is authored here (the vendor integration) and
//! consumed by the generic triggered-delivery driver through the
//! [`PreferenceTargetCodec`] port — the only place vendor knowledge enters
//! that path. Stored preferences persist these refs, so the segment
//! vocabulary (including the `slack_v2` adapter segment) is a wire format:
//! changing it requires a data migration.

use ironclaw_host_api::{AgentId, ProjectId};
use ironclaw_product_adapters::{
    AdapterInstallationId, ExternalActorRef, ExternalConversationRef, PreferenceTargetCodec,
    PreferenceTargetEncodeRequest,
};
use ironclaw_turns::ReplyTargetBindingRef;

use crate::{SLACK_USER_ACTOR_KIND, SLACK_V2_ADAPTER_ID};

/// Typed binding-ref construction failure (invalid segment material or an
/// oversized/ill-formed ref). Never carries payload material.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid Slack reply-target binding ref")]
pub struct SlackReplyTargetError;

/// Decodes Slack preference reply-target binding refs for the generic
/// triggered driver (the vendor half of [`PreferenceTargetCodec`]).
pub struct SlackPreferenceTargetCodec;

impl PreferenceTargetCodec for SlackPreferenceTargetCodec {
    fn conversation_for_target(
        &self,
        target: &ReplyTargetBindingRef,
    ) -> Option<ExternalConversationRef> {
        let (conversation_id, space_id) =
            slack_conversation_id_from_reply_target_binding_ref(target)?;
        ExternalConversationRef::new(space_id.as_deref(), &conversation_id, None, None).ok()
    }

    fn is_personal_direct_message(&self, target: &ReplyTargetBindingRef) -> bool {
        slack_reply_target_is_personal_dm(target)
    }

    fn direct_message_actor_for_target(&self, target: &ReplyTargetBindingRef) -> Option<String> {
        if !slack_reply_target_is_personal_dm(target) {
            return None;
        }
        decode_slack_reply_target_binding_ref(target)?.actor_id
    }

    fn encode_shared_conversation_target(
        &self,
        request: PreferenceTargetEncodeRequest<'_>,
    ) -> Option<ReplyTargetBindingRef> {
        // Slack refs always carry the workspace (team) binding; a
        // conversation without a space cannot be encoded (fail closed).
        let team_id = request.conversation.space_id()?;
        slack_shared_channel_reply_target_binding_ref(
            request.installation_id,
            request.agent_id,
            request.project_id,
            team_id,
            request.conversation.conversation_id(),
        )
        .ok()
    }

    fn encode_personal_direct_message_target(
        &self,
        request: PreferenceTargetEncodeRequest<'_>,
        external_actor_id: &str,
    ) -> Option<ReplyTargetBindingRef> {
        let team_id = request.conversation.space_id()?;
        slack_personal_dm_reply_target_binding_ref(
            request.installation_id,
            request.agent_id,
            request.project_id,
            team_id,
            request.conversation.conversation_id(),
            external_actor_id,
        )
        .ok()
    }
}

/// Build a shared-channel reply-target binding ref.
pub fn slack_shared_channel_reply_target_binding_ref(
    installation_id: &AdapterInstallationId,
    agent_id: &AgentId,
    project_id: Option<&ProjectId>,
    team_id: &str,
    channel_id: &str,
) -> Result<ReplyTargetBindingRef, SlackReplyTargetError> {
    let conversation = ExternalConversationRef::new(Some(team_id), channel_id, None, None)
        .map_err(|_| SlackReplyTargetError)?;
    let raw = format!(
        "{}{}{}{}{}",
        product_binding_segment("adapter", SLACK_V2_ADAPTER_ID),
        product_binding_segment("installation", installation_id.as_str()),
        product_binding_segment("agent", agent_id.as_str()),
        product_binding_segment("project", project_id.map_or("", |id| id.as_str())),
        conversation.conversation_fingerprint()
    );
    slack_reply_target_binding_ref_from_raw(raw)
}

/// Build a personal-DM reply-target binding ref (carries the proven Slack
/// actor so the OAuth-DM gate can verify the surface).
pub fn slack_personal_dm_reply_target_binding_ref(
    installation_id: &AdapterInstallationId,
    agent_id: &AgentId,
    project_id: Option<&ProjectId>,
    team_id: &str,
    dm_channel_id: &str,
    slack_user_id: &str,
) -> Result<ReplyTargetBindingRef, SlackReplyTargetError> {
    let conversation = ExternalConversationRef::new(Some(team_id), dm_channel_id, None, None)
        .map_err(|_| SlackReplyTargetError)?;
    let actor = ExternalActorRef::new(SLACK_USER_ACTOR_KIND, slack_user_id, None::<&str>)
        .map_err(|_| SlackReplyTargetError)?;
    let raw = format!(
        "{}{}{}{}{}{}{}",
        product_binding_segment("adapter", SLACK_V2_ADAPTER_ID),
        product_binding_segment("installation", installation_id.as_str()),
        product_binding_segment("agent", agent_id.as_str()),
        product_binding_segment("project", project_id.map_or("", |id| id.as_str())),
        conversation.conversation_fingerprint(),
        product_binding_segment("actor_kind", actor.kind()),
        product_binding_segment("actor", actor.id())
    );
    slack_reply_target_binding_ref_from_raw(raw)
}

/// Wrap raw segment material in the `reply:` binding-ref envelope.
pub fn slack_reply_target_binding_ref_from_raw(
    raw: String,
) -> Result<ReplyTargetBindingRef, SlackReplyTargetError> {
    // Safety: callers must pre-validate segment values (reject control
    // characters including NUL). `ReplyTargetBindingRef::new` enforces the
    // 256-byte limit and rejects control chars as the primary defense —
    // caller-side validators are defense-in-depth.
    ReplyTargetBindingRef::new(format!("reply:{raw}")).map_err(|_| SlackReplyTargetError)
}

// Keep this segment format in parity with
// `ExternalConversationRef::conversation_fingerprint`.
fn product_binding_segment(name: &str, value: &str) -> String {
    format!("{name}:{}:{value};", value.len())
}

fn take_product_binding_segment<'a>(raw: &'a str, name: &str) -> Option<(&'a str, &'a str)> {
    let raw = raw.strip_prefix(name)?.strip_prefix(':')?;
    let (length, raw) = raw.split_once(':')?;
    let length = length.parse::<usize>().ok()?;
    let value = raw.get(..length)?;
    let raw = raw.get(length..)?.strip_prefix(';')?;
    Some((value, raw))
}

/// Decoded fields from a Slack reply-target binding ref segment walk.
///
/// Produced by [`decode_slack_reply_target_binding_ref`]; consumed by both
/// [`slack_conversation_id_from_reply_target_binding_ref`] and
/// [`slack_reply_target_is_personal_dm`] so that the segment format is
/// parsed in exactly one place.
struct DecodedSlackReplyTarget {
    conversation_id: String,
    space_id: Option<String>,
    /// `true` iff a `topic` segment was successfully consumed after
    /// `conversation` — required for the personal-DM predicate to mirror
    /// the original behaviour, which demanded `topic` be present.
    topic_present: bool,
    /// Present only when the ref carries trailing `actor_kind` + `actor`
    /// segments (personal-DM refs).
    actor_kind: Option<String>,
    /// Present only when the ref carries a non-empty `actor` segment value.
    actor_id: Option<String>,
    /// `true` iff the segment walk consumed the entire ref with no leftover
    /// bytes — required for the personal-DM predicate to reject trailing
    /// garbage.
    fully_consumed: bool,
}

/// Walk the length-prefixed segment format shared by both
/// [`slack_shared_channel_reply_target_binding_ref`] and
/// [`slack_personal_dm_reply_target_binding_ref`] and return the decoded
/// fields.
///
/// Returns `None` if the ref does not start with `reply:`, if any of the
/// required prefix segments (adapter / installation / agent / project /
/// space / conversation) are missing or malformed, or if the conversation id
/// is empty. Optional trailing segments (topic / actor_kind / actor) are
/// consumed when present but are not required; whether they were found is
/// recorded in `topic_present`, `actor_kind`, and `actor_id`.
fn decode_slack_reply_target_binding_ref(
    target: &ReplyTargetBindingRef,
) -> Option<DecodedSlackReplyTarget> {
    let mut raw = target.as_str().strip_prefix("reply:")?;
    // Validate the adapter segment — reject non-Slack adapter refs fail-closed.
    let (adapter_id, rest) = take_product_binding_segment(raw, "adapter")?;
    if adapter_id != SLACK_V2_ADAPTER_ID {
        return None;
    }
    raw = rest;
    // Skip installation, agent, project — values not validated here.
    for name in &["installation", "agent", "project"] {
        let (_, rest) = take_product_binding_segment(raw, name)?;
        raw = rest;
    }
    // Consume the required conversation fingerprint segments: space / conversation.
    let (space_id, rest) = take_product_binding_segment(raw, "space")?;
    let (conversation_id, rest) = take_product_binding_segment(rest, "conversation")?;
    if conversation_id.is_empty() {
        return None;
    }
    // Consume the optional topic segment (present in both shared-channel and
    // personal-DM refs built by the constructors in this module).
    let (topic_present, rest) = match take_product_binding_segment(rest, "topic") {
        Some((_, after_topic)) => (true, after_topic),
        None => (false, rest),
    };
    // Consume the optional actor_kind + actor segments (personal-DM only).
    let (actor_kind, actor_id, rest) = match take_product_binding_segment(rest, "actor_kind") {
        Some((kind, after_kind)) => match take_product_binding_segment(after_kind, "actor") {
            Some((id, after_actor)) => {
                // Map an empty actor value to None so the type matches the
                // doc-comment ("Present only when the `actor` segment exists
                // with a non-empty value").
                let actor_id = if id.is_empty() {
                    None
                } else {
                    Some(id.to_string())
                };
                (Some(kind.to_string()), actor_id, after_actor)
            }
            None => (Some(kind.to_string()), None, after_kind),
        },
        None => (None, None, rest),
    };
    Some(DecodedSlackReplyTarget {
        conversation_id: conversation_id.to_string(),
        space_id: if space_id.is_empty() {
            None
        } else {
            Some(space_id.to_string())
        },
        topic_present,
        actor_kind,
        actor_id,
        fully_consumed: rest.is_empty(),
    })
}

/// Extract the Slack channel ID encoded in a Slack reply-target binding ref.
///
/// Parses the length-prefixed segment format produced by
/// [`slack_shared_channel_reply_target_binding_ref`] /
/// [`slack_personal_dm_reply_target_binding_ref`] without requiring the full
/// config for validation. Used by the triggered-run delivery path to
/// reconstruct the outbound `ExternalConversationRef` from a stored
/// preference.
///
/// Returns `None` if the ref is not a Slack reply target or is malformed.
pub fn slack_conversation_id_from_reply_target_binding_ref(
    target: &ReplyTargetBindingRef,
) -> Option<(String, Option<String>)> {
    let decoded = decode_slack_reply_target_binding_ref(target)?;
    Some((decoded.conversation_id, decoded.space_id))
}

/// Returns `true` iff `target` is a verified personal-DM Slack reply-target
/// binding ref.
///
/// A personal-DM ref has two structural markers beyond the shared segment
/// prefix (adapter / installation / agent / project / space / conversation /
/// topic):
///
/// 1. Trailing `actor_kind` + `actor` segments are **present** — this is the
///    distinguishing property that [`slack_personal_dm_reply_target_binding_ref`]
///    adds and [`slack_shared_channel_reply_target_binding_ref`] omits.
/// 2. The decoded conversation id (the DM channel id) **starts with uppercase
///    `'D'`** — the Slack-assigned prefix for all DM channels.
///
/// Any parse failure (malformed segment encoding, missing fields, non-`D`
/// channel id) returns `false` — fail closed.
///
/// This does NOT validate the installation, agent, project, or space values
/// against a live provider config. It is a structural parse only: safe to
/// call before a provider is available.
pub fn slack_reply_target_is_personal_dm(target: &ReplyTargetBindingRef) -> bool {
    let Some(decoded) = decode_slack_reply_target_binding_ref(target) else {
        return false;
    };
    // A well-formed Slack reply target always carries the team/space binding.
    // An empty `space` segment (`space:0:`) decodes to `None` here; reject it
    // so a corrupted or forged preference missing the space cannot satisfy
    // the OAuth-DM gate.
    if decoded.space_id.is_none() {
        return false;
    }
    // DM channel ids start with uppercase 'D'.
    if !decoded.conversation_id.starts_with('D') {
        return false;
    }
    // The conversation fingerprint must include a topic segment (as produced
    // by `ExternalConversationRef::conversation_fingerprint`); refs missing
    // it are not well-formed personal-DM refs.
    if !decoded.topic_present {
        return false;
    }
    // actor_kind must be the Slack user kind; actor_id is Some only when the
    // actor segment is non-empty (invariant enforced at decode time); no
    // further segments are permitted.
    decoded.actor_kind.as_deref() == Some(SLACK_USER_ACTOR_KIND)
        && decoded.actor_id.is_some()
        && decoded.fully_consumed
}

#[cfg(test)]
mod tests {
    use super::*;

    const INSTALLATION: &str = "install_alpha";
    const AGENT: &str = "agent:slack";
    const PROJECT: &str = "project:slack";
    const TEAM: &str = "T-A";
    const SLACK_USER: &str = "U123";

    fn seg(name: &str, value: &str) -> String {
        format!("{}:{}:{};", name, value.len(), value)
    }

    fn dm_binding_ref(dm_channel_id: &str, slack_user_id: &str) -> ReplyTargetBindingRef {
        let installation_id = AdapterInstallationId::new(INSTALLATION).expect("installation");
        let agent_id = AgentId::new(AGENT).expect("agent");
        let project_id = ProjectId::new(PROJECT).expect("project");
        slack_personal_dm_reply_target_binding_ref(
            &installation_id,
            &agent_id,
            Some(&project_id),
            TEAM,
            dm_channel_id,
            slack_user_id,
        )
        .expect("personal DM binding ref")
    }

    fn shared_channel_binding_ref(channel_id: &str) -> ReplyTargetBindingRef {
        let installation_id = AdapterInstallationId::new(INSTALLATION).expect("installation");
        let agent_id = AgentId::new(AGENT).expect("agent");
        let project_id = ProjectId::new(PROJECT).expect("project");
        slack_shared_channel_reply_target_binding_ref(
            &installation_id,
            &agent_id,
            Some(&project_id),
            TEAM,
            channel_id,
        )
        .expect("shared channel binding ref")
    }

    #[test]
    fn codec_decodes_dm_ref_conversation_and_dm_predicate() {
        let codec = SlackPreferenceTargetCodec;
        let binding_ref = dm_binding_ref("D0HOST", SLACK_USER);
        let conversation = codec
            .conversation_for_target(&binding_ref)
            .expect("conversation decodes");
        assert_eq!(conversation.conversation_id(), "D0HOST");
        assert_eq!(conversation.space_id(), Some(TEAM));
        assert!(codec.is_personal_direct_message(&binding_ref));

        let shared = shared_channel_binding_ref("C0HOST");
        assert!(!codec.is_personal_direct_message(&shared));
    }

    #[test]
    fn codec_exposes_dm_actor_only_for_personal_dm_refs() {
        let codec = SlackPreferenceTargetCodec;
        assert_eq!(
            codec.direct_message_actor_for_target(&dm_binding_ref("D0HOST", SLACK_USER)),
            Some(SLACK_USER.to_string()),
        );
        assert_eq!(
            codec.direct_message_actor_for_target(&shared_channel_binding_ref("C0HOST")),
            None,
            "shared refs carry no DM actor"
        );
    }

    #[test]
    fn codec_encode_halves_round_trip_through_the_decode_halves() {
        let codec = SlackPreferenceTargetCodec;
        let installation_id = AdapterInstallationId::new(INSTALLATION).expect("installation");
        let agent_id = AgentId::new(AGENT).expect("agent");
        let project_id = ProjectId::new(PROJECT).expect("project");

        let shared_conversation =
            ExternalConversationRef::new(Some(TEAM), "C0HOST", None, None).expect("conversation");
        let shared = codec
            .encode_shared_conversation_target(PreferenceTargetEncodeRequest {
                installation_id: &installation_id,
                agent_id: &agent_id,
                project_id: Some(&project_id),
                conversation: &shared_conversation,
            })
            .expect("shared ref encodes");
        assert_eq!(shared, shared_channel_binding_ref("C0HOST"));
        assert!(!codec.is_personal_direct_message(&shared));

        let dm_conversation =
            ExternalConversationRef::new(Some(TEAM), "D0HOST", None, None).expect("conversation");
        let dm = codec
            .encode_personal_direct_message_target(
                PreferenceTargetEncodeRequest {
                    installation_id: &installation_id,
                    agent_id: &agent_id,
                    project_id: Some(&project_id),
                    conversation: &dm_conversation,
                },
                SLACK_USER,
            )
            .expect("dm ref encodes");
        assert_eq!(dm, dm_binding_ref("D0HOST", SLACK_USER));
        assert!(codec.is_personal_direct_message(&dm));
        assert_eq!(
            codec.direct_message_actor_for_target(&dm),
            Some(SLACK_USER.to_string())
        );

        // A conversation without the workspace binding cannot be encoded.
        let spaceless =
            ExternalConversationRef::new(None::<&str>, "C0HOST", None, None).expect("conversation");
        assert!(
            codec
                .encode_shared_conversation_target(PreferenceTargetEncodeRequest {
                    installation_id: &installation_id,
                    agent_id: &agent_id,
                    project_id: Some(&project_id),
                    conversation: &spaceless,
                })
                .is_none(),
            "spaceless conversations must not encode"
        );
    }

    #[test]
    fn personal_dm_binding_ref_round_trips_dm_channel_and_slack_user() {
        let binding_ref = dm_binding_ref("D0HOST", SLACK_USER);
        let (conversation_id, space_id) =
            slack_conversation_id_from_reply_target_binding_ref(&binding_ref)
                .expect("conversation id decodes");
        assert_eq!(conversation_id, "D0HOST");
        assert_eq!(space_id.as_deref(), Some(TEAM));
    }

    #[test]
    fn shared_channel_binding_ref_round_trips_channel_id() {
        let binding_ref = shared_channel_binding_ref("C0HOST");
        let (conversation_id, space_id) =
            slack_conversation_id_from_reply_target_binding_ref(&binding_ref)
                .expect("conversation id decodes");
        assert_eq!(conversation_id, "C0HOST");
        assert_eq!(space_id.as_deref(), Some(TEAM));
    }

    #[test]
    fn binding_ref_from_raw_rejects_oversized_and_control_char_raw() {
        let oversized = "x".repeat(300);
        assert!(slack_reply_target_binding_ref_from_raw(oversized).is_err());
        assert!(
            slack_reply_target_binding_ref_from_raw("adapter:5:slack;\x01".to_string()).is_err()
        );
    }

    #[test]
    fn is_personal_dm_returns_true_for_dm_ref() {
        let binding_ref = dm_binding_ref("D0HOST", SLACK_USER);
        assert!(
            slack_reply_target_is_personal_dm(&binding_ref),
            "personal-DM binding ref must be identified as a personal DM"
        );
    }

    #[test]
    fn is_personal_dm_returns_false_for_shared_channel_ref() {
        let binding_ref = shared_channel_binding_ref("C0HOST");
        assert!(
            !slack_reply_target_is_personal_dm(&binding_ref),
            "shared-channel binding ref must NOT be identified as a personal DM"
        );
    }

    #[test]
    fn is_personal_dm_returns_false_for_non_d_prefixed_channel() {
        let raw = format!(
            "{}{}{}{}{}{}{}{}{}",
            seg("adapter", SLACK_V2_ADAPTER_ID),
            seg("installation", INSTALLATION),
            seg("agent", AGENT),
            seg("project", PROJECT),
            seg("space", TEAM),
            seg("conversation", "G0GROUP"),
            seg("topic", ""),
            seg("actor_kind", SLACK_USER_ACTOR_KIND),
            seg("actor", SLACK_USER),
        );
        let binding_ref =
            slack_reply_target_binding_ref_from_raw(raw).expect("builds syntactically");
        assert!(
            !slack_reply_target_is_personal_dm(&binding_ref),
            "non-'D'-prefixed channel must NOT be identified as a personal DM"
        );
    }

    #[test]
    fn is_personal_dm_returns_false_for_malformed_ref() {
        let malformed = ReplyTargetBindingRef::new("reply:not-a-valid-segment-format".to_string())
            .expect("syntactically valid ref");
        assert!(
            !slack_reply_target_is_personal_dm(&malformed),
            "malformed ref must return false (fail closed)"
        );
    }

    #[test]
    fn is_personal_dm_returns_false_for_empty_actor() {
        let raw = format!(
            "{}{}{}{}{}{}{}{}{}",
            seg("adapter", SLACK_V2_ADAPTER_ID),
            seg("installation", INSTALLATION),
            seg("agent", AGENT),
            seg("project", PROJECT),
            seg("space", TEAM),
            seg("conversation", "D0HOST"),
            seg("topic", ""),
            seg("actor_kind", SLACK_USER_ACTOR_KIND),
            seg("actor", ""),
        );
        let binding_ref =
            slack_reply_target_binding_ref_from_raw(raw).expect("builds syntactically");
        assert!(
            !slack_reply_target_is_personal_dm(&binding_ref),
            "ref with empty actor must NOT be identified as a personal DM"
        );
    }

    #[test]
    fn is_personal_dm_rejects_wrong_actor_kind_and_trailing_segments() {
        // (a) Wrong actor_kind: use a non-Slack actor kind.
        let raw_wrong_kind = format!(
            "{}{}{}{}{}{}{}{}{}",
            seg("adapter", SLACK_V2_ADAPTER_ID),
            seg("installation", INSTALLATION),
            seg("agent", AGENT),
            seg("project", PROJECT),
            seg("space", TEAM),
            seg("conversation", "D0HOST"),
            seg("topic", ""),
            seg("actor_kind", "not_a_slack_user"),
            seg("actor", SLACK_USER),
        );
        let binding_ref_wrong_kind =
            slack_reply_target_binding_ref_from_raw(raw_wrong_kind).expect("builds syntactically");
        assert!(
            !slack_reply_target_is_personal_dm(&binding_ref_wrong_kind),
            "actor_kind != slack_user must NOT be identified as a personal DM"
        );

        // (b) Extra trailing segment after the actor segment.
        let raw_trailing = format!(
            "{}{}{}{}{}{}{}{}{}{}",
            seg("adapter", SLACK_V2_ADAPTER_ID),
            seg("installation", INSTALLATION),
            seg("agent", AGENT),
            seg("project", PROJECT),
            seg("space", TEAM),
            seg("conversation", "D0HOST"),
            seg("topic", ""),
            seg("actor_kind", SLACK_USER_ACTOR_KIND),
            seg("actor", SLACK_USER),
            seg("extra", "unexpected"),
        );
        let binding_ref_trailing =
            slack_reply_target_binding_ref_from_raw(raw_trailing).expect("builds syntactically");
        assert!(
            !slack_reply_target_is_personal_dm(&binding_ref_trailing),
            "trailing segment after actor must NOT be identified as a personal DM"
        );
    }

    #[test]
    fn is_personal_dm_returns_false_for_missing_topic_segment() {
        let raw = format!(
            "{}{}{}{}{}{}{}{}",
            seg("adapter", SLACK_V2_ADAPTER_ID),
            seg("installation", INSTALLATION),
            seg("agent", AGENT),
            seg("project", PROJECT),
            seg("space", TEAM),
            seg("conversation", "D0HOST"),
            // topic segment intentionally omitted
            seg("actor_kind", SLACK_USER_ACTOR_KIND),
            seg("actor", SLACK_USER),
        );
        let binding_ref =
            slack_reply_target_binding_ref_from_raw(raw).expect("builds syntactically");
        assert!(
            !slack_reply_target_is_personal_dm(&binding_ref),
            "ref missing topic segment must NOT be identified as a personal DM"
        );
    }

    #[test]
    fn is_personal_dm_returns_false_for_empty_space_segment() {
        let raw = format!(
            "{}{}{}{}{}{}{}{}{}",
            seg("adapter", SLACK_V2_ADAPTER_ID),
            seg("installation", INSTALLATION),
            seg("agent", AGENT),
            seg("project", PROJECT),
            seg("space", ""),
            seg("conversation", "D0HOST"),
            seg("topic", ""),
            seg("actor_kind", SLACK_USER_ACTOR_KIND),
            seg("actor", SLACK_USER),
        );
        let binding_ref =
            slack_reply_target_binding_ref_from_raw(raw).expect("builds syntactically");
        assert!(
            !slack_reply_target_is_personal_dm(&binding_ref),
            "ref with empty space segment must NOT be identified as a personal DM"
        );
    }

    #[test]
    fn is_personal_dm_rejects_non_slack_adapter() {
        let raw = format!(
            "{}{}{}{}{}{}{}{}{}",
            seg("adapter", "other_adapter"),
            seg("installation", INSTALLATION),
            seg("agent", AGENT),
            seg("project", PROJECT),
            seg("space", TEAM),
            seg("conversation", "D0HOST"),
            seg("topic", ""),
            seg("actor_kind", SLACK_USER_ACTOR_KIND),
            seg("actor", SLACK_USER),
        );
        let binding_ref =
            slack_reply_target_binding_ref_from_raw(raw).expect("builds syntactically");
        assert!(
            !slack_reply_target_is_personal_dm(&binding_ref),
            "non-Slack adapter ref must NOT be identified as a personal DM"
        );
    }

    #[test]
    fn is_personal_dm_returns_false_for_missing_actor_segment() {
        let raw = format!(
            "{}{}{}{}{}{}{}{}",
            seg("adapter", SLACK_V2_ADAPTER_ID),
            seg("installation", INSTALLATION),
            seg("agent", AGENT),
            seg("project", PROJECT),
            seg("space", TEAM),
            seg("conversation", "D0HOST"),
            seg("topic", ""),
            seg("actor_kind", SLACK_USER_ACTOR_KIND),
            // actor segment intentionally absent
        );
        let binding_ref =
            slack_reply_target_binding_ref_from_raw(raw).expect("builds syntactically");
        assert!(
            !slack_reply_target_is_personal_dm(&binding_ref),
            "ref with actor_kind but no actor segment must NOT be identified as a personal DM"
        );
    }

    #[test]
    fn conversation_id_returns_none_for_non_slack_adapter() {
        let raw = format!(
            "{}{}{}{}{}{}{}",
            seg("adapter", "not_slack_v2"),
            seg("installation", INSTALLATION),
            seg("agent", AGENT),
            seg("project", PROJECT),
            seg("space", TEAM),
            seg("conversation", "C0HOST"),
            seg("topic", ""),
        );
        let binding_ref =
            slack_reply_target_binding_ref_from_raw(raw).expect("builds syntactically");
        assert!(
            slack_conversation_id_from_reply_target_binding_ref(&binding_ref).is_none(),
            "non-Slack adapter ref must return None"
        );
    }
}
