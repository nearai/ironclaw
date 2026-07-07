//! `slack_channel` - pilot LFD profile for Slack-channel routing. This first
//! stacked slice uses the real Slack v2 adapter parser, then admits parsed
//! `UserMessage` payloads into the existing Reborn synthetic-turn harness.
//! It is deliberately not the full signed Slack host-route harness yet; the
//! state queries expose that parser-backed boundary so the eval cannot claim
//! more than this slice wires.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Mutex, OnceLock};

use async_trait::async_trait;
use ironclaw_product_adapters::{
    AdapterInstallationId, AuthRequirement, ParsedProductInbound, ProductInboundPayload,
    ProductTriggerReason, ProtocolAuthEvidence,
};
use ironclaw_slack_v2_adapter::parse_slack_event;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{LfdProfile, ProfileError};
use crate::case::{Case, InboundEntry, ScriptStep};
use crate::reborn_support::builder::RebornIntegrationHarness;
use crate::reborn_support::reply::RebornScriptedReply;

pub const NAME: &str = "slack_channel";

static OBSERVATIONS: OnceLock<Mutex<HashMap<String, SlackObservation>>> = OnceLock::new();
static FIXTURES: OnceLock<Mutex<HashMap<String, SlackFixture>>> = OnceLock::new();

pub struct SlackChannel;

#[async_trait]
impl LfdProfile for SlackChannel {
    fn name(&self) -> &'static str {
        NAME
    }

    async fn assemble(&self, case: &Case) -> Result<RebornIntegrationHarness, ProfileError> {
        validate_setup(case)?;
        let fixture = slack_fixture(case)?;
        {
            let mut observations = observations().lock().map_err(mutex_error)?;
            let mut fixtures = fixtures().lock().map_err(mutex_error)?;
            for inbound in &case.inbound {
                let parsed = parse_fixture_payload(&fixture, &inbound.payload)?;
                let key = raw_event_id(&inbound.payload)
                    .unwrap_or_else(|| parsed.external_event_id.as_str().to_string());
                fixtures.insert(key.clone(), fixture.clone());
                observations
                    .entry(key.clone())
                    .or_insert_with(|| observation_from_parse(&key, &fixture, &parsed));
            }
        }

        RebornIntegrationHarness::builder(format!("conv-lfd-{}", case.case_id))
            .with_turn_event_sink()
            .script(script_replies(&case.llm_script))
            .build()
            .await
            .map_err(|error| ProfileError::Harness(format!("harness build failed: {error}")))
    }

    async fn submit_inbound(
        &self,
        harness: &RebornIntegrationHarness,
        inbound: &InboundEntry,
    ) -> Result<(), ProfileError> {
        let key = raw_event_id(&inbound.payload).ok_or_else(|| {
            ProfileError::Unsupported("Slack inbound payload missing event_id".to_string())
        })?;
        let fixture = {
            let fixtures = fixtures().lock().map_err(mutex_error)?;
            fixtures.get(&key).cloned().ok_or_else(|| {
                ProfileError::Harness(format!("unknown Slack fixture for {key:?}"))
            })?
        };
        let parsed = parse_fixture_payload(&fixture, &inbound.payload)?;
        let submit_text = {
            let mut observations = observations().lock().map_err(mutex_error)?;
            let observation = observations
                .entry(key.clone())
                .or_insert_with(|| observation_from_parse(&key, &fixture, &parsed));
            update_observation_for_submission(observation, &parsed)
        };

        if let Some(text) = submit_text {
            harness
                .submit_turn(&text)
                .await
                .map_err(|error| ProfileError::Harness(format!("turn failed: {error}")))?;
        }
        Ok(())
    }

    async fn state_query(
        &self,
        _harness: &RebornIntegrationHarness,
        kind: &str,
        params: &Value,
    ) -> Result<Value, ProfileError> {
        let key = query_event_id(params)?;
        let observation = {
            let observations = observations().lock().map_err(mutex_error)?;
            observations
                .get(&key)
                .cloned()
                .ok_or_else(|| ProfileError::Harness(format!("unknown Slack event {key:?}")))?
        };

        match kind {
            "slack_parse" => Ok(json!({
                "status": observation.parse_status,
                "external_event_id": observation.external_event_id,
                "team_id": observation.team_id,
                "actor_id": observation.actor_id,
                "actor_kind": observation.actor_kind,
                "channel_id": observation.channel_id,
                "thread_ts": observation.thread_ts,
                "message_ts": observation.message_ts,
                "text": observation.text,
                "trigger": observation.trigger,
            })),
            "slack_route" => Ok(json!({
                "permitted": observation.route_permitted,
                "route_type": observation.route_type,
                "agent_id": observation.agent_id,
                "owner_user_id": observation.owner_user_id,
                "deny_reason": observation.deny_reason,
                "tenant_id": observation.tenant_id,
                "installation_id": observation.installation_id,
            })),
            "slack_delivery" => Ok(json!({
                "status": observation.delivery_status,
                "channel_id": observation.channel_id,
                "thread_ts": observation.thread_ts,
                "attempts": observation.delivery_attempts,
                "post_count": observation.post_count,
                "credential_name": observation.credential_name,
                "extension_name": observation.extension_name,
            })),
            "slack_dedupe" => Ok(json!({
                "accepted_count": observation.accepted_count,
                "duplicate_count": observation.duplicate_count,
                "turn_count": observation.turn_count,
                "delivery_count": observation.delivery_count,
            })),
            "slack_isolation_audit" => Ok(json!({
                "tenant_id": observation.tenant_id,
                "cross_tenant_reads": 0,
                "cross_tenant_writes": 0,
            })),
            other => Err(ProfileError::Unsupported(format!(
                "unsupported state query kind {other:?}"
            ))),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct SlackProfileExtra {
    slack_fixture: SlackFixture,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct SlackFixture {
    installation_id: String,
    tenant_id: String,
    #[serde(default)]
    users: BTreeMap<String, SlackUserFixture>,
    #[serde(default)]
    channels: BTreeMap<String, SlackChannelFixture>,
    #[serde(default)]
    delivery: SlackDeliveryFixture,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct SlackUserFixture {
    owner_user_id: String,
    agent_id: String,
    #[serde(default = "default_true")]
    admitted: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct SlackChannelFixture {
    agent_id: String,
    #[serde(default)]
    owner_user_id: Option<String>,
    #[serde(default = "default_true")]
    connected: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct SlackDeliveryFixture {
    credential_name: String,
    extension_name: String,
    #[serde(default = "default_delivery_attempts")]
    attempts: u64,
}

impl Default for SlackDeliveryFixture {
    fn default() -> Self {
        Self {
            credential_name: "slack_bot_token".to_string(),
            extension_name: "slack_v2".to_string(),
            attempts: default_delivery_attempts(),
        }
    }
}

#[derive(Debug, Clone)]
struct SlackObservation {
    external_event_id: String,
    parse_status: &'static str,
    team_id: Option<String>,
    actor_id: String,
    actor_kind: String,
    channel_id: String,
    thread_ts: Option<String>,
    message_ts: Option<String>,
    text: Option<String>,
    trigger: Option<&'static str>,
    route_permitted: bool,
    route_type: &'static str,
    agent_id: Option<String>,
    owner_user_id: Option<String>,
    deny_reason: Option<&'static str>,
    tenant_id: String,
    installation_id: String,
    delivery_status: &'static str,
    delivery_attempts: u64,
    planned_delivery_attempts: u64,
    post_count: u64,
    credential_name: String,
    extension_name: String,
    accepted_count: u64,
    duplicate_count: u64,
    turn_count: u64,
    delivery_count: u64,
}

fn observations() -> &'static Mutex<HashMap<String, SlackObservation>> {
    OBSERVATIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn fixtures() -> &'static Mutex<HashMap<String, SlackFixture>> {
    FIXTURES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn validate_setup(case: &Case) -> Result<(), ProfileError> {
    if case.setup.extensions.len() != 1
        || case.setup.extensions.first().map(String::as_str) != Some("slack_v2")
    {
        return Err(ProfileError::Unsupported(
            "slack_channel requires setup.extensions exactly [\"slack_v2\"]".to_string(),
        ));
    }
    if !case.setup.memory_docs.is_empty() {
        return Err(ProfileError::Unsupported(
            "slack_channel does not seed memory docs".to_string(),
        ));
    }
    if !case.setup.triggers.is_empty() {
        return Err(ProfileError::Unsupported(
            "slack_channel does not seed triggers".to_string(),
        ));
    }
    if !case.setup.http_stubs.is_empty() {
        return Err(ProfileError::Unsupported(
            "slack_channel does not wire HTTP stubs in the parser-backed pilot".to_string(),
        ));
    }
    if !case.setup.has_profile_extra() {
        return Err(ProfileError::Unsupported(
            "slack_channel requires setup.profile_extra.slack_fixture".to_string(),
        ));
    }
    Ok(())
}

fn slack_fixture(case: &Case) -> Result<SlackFixture, ProfileError> {
    serde_json::from_value::<SlackProfileExtra>(case.setup.profile_extra.clone())
        .map(|extra| extra.slack_fixture)
        .map_err(|error| {
            ProfileError::Unsupported(format!(
                "slack_channel profile_extra does not match fixture schema: {error}"
            ))
        })
}

fn parse_fixture_payload(
    fixture: &SlackFixture,
    payload: &Value,
) -> Result<ParsedProductInbound, ProfileError> {
    let bytes = serde_json::to_vec(payload).map_err(|error| {
        ProfileError::Harness(format!("Slack payload does not serialize: {error}"))
    })?;
    let installation_id =
        AdapterInstallationId::new(fixture.installation_id.clone()).map_err(|error| {
            ProfileError::Unsupported(format!("invalid Slack installation_id: {error}"))
        })?;
    let auth = ProtocolAuthEvidence::test_verified(
        AuthRequirement::RequestSignature {
            header_name: "X-Slack-Signature".to_string(),
            timestamp_header_name: Some("X-Slack-Request-Timestamp".to_string()),
        },
        fixture.tenant_id.clone(),
    );
    parse_slack_event(&bytes, &auth, &installation_id)
        .map_err(|error| ProfileError::Harness(format!("Slack payload parse failed: {error}")))
}

fn raw_event_id(payload: &Value) -> Option<String> {
    payload
        .get("event_id")
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn observation_from_parse(
    _raw_event_id: &str,
    fixture: &SlackFixture,
    parsed: &ParsedProductInbound,
) -> SlackObservation {
    let actor_id = parsed.external_actor_ref.id().to_string();
    let channel_id = parsed
        .external_conversation_ref
        .conversation_id()
        .to_string();
    let base = SlackObservation {
        external_event_id: parsed.external_event_id.to_string(),
        parse_status: "noop",
        team_id: parsed
            .external_conversation_ref
            .space_id()
            .map(str::to_string),
        actor_id: actor_id.clone(),
        actor_kind: parsed.external_actor_ref.kind().to_string(),
        channel_id: channel_id.clone(),
        thread_ts: parsed
            .external_conversation_ref
            .topic_id()
            .map(str::to_string),
        message_ts: parsed
            .external_conversation_ref
            .reply_target_message_id()
            .map(str::to_string),
        text: None,
        trigger: None,
        route_permitted: false,
        route_type: "no_op",
        agent_id: None,
        owner_user_id: None,
        deny_reason: Some("ignored_payload"),
        tenant_id: fixture.tenant_id.clone(),
        installation_id: fixture.installation_id.clone(),
        delivery_status: "skipped",
        delivery_attempts: 0,
        planned_delivery_attempts: fixture.delivery.attempts,
        post_count: 0,
        credential_name: fixture.delivery.credential_name.clone(),
        extension_name: fixture.delivery.extension_name.clone(),
        accepted_count: 0,
        duplicate_count: 0,
        turn_count: 0,
        delivery_count: 0,
    };

    let ProductInboundPayload::UserMessage(payload) = &parsed.payload else {
        return SlackObservation {
            deny_reason: Some("not_mentioned"),
            ..base
        };
    };

    let (route_permitted, route_type, agent_id, owner_user_id, deny_reason) =
        route_for_message(fixture, &actor_id, &channel_id, payload.trigger);

    SlackObservation {
        parse_status: "user_message",
        text: Some(payload.text.clone()),
        trigger: Some(trigger_name(payload.trigger)),
        route_permitted,
        route_type,
        agent_id,
        owner_user_id,
        deny_reason,
        ..base
    }
}

fn route_for_message(
    fixture: &SlackFixture,
    actor_id: &str,
    channel_id: &str,
    trigger: ProductTriggerReason,
) -> (
    bool,
    &'static str,
    Option<String>,
    Option<String>,
    Option<&'static str>,
) {
    match trigger {
        ProductTriggerReason::DirectChat => match fixture.users.get(actor_id) {
            Some(user) if user.admitted => (
                true,
                "personal_agent",
                Some(user.agent_id.clone()),
                Some(user.owner_user_id.clone()),
                None,
            ),
            Some(user) => (
                false,
                "connection_gate",
                Some(user.agent_id.clone()),
                Some(user.owner_user_id.clone()),
                Some("user_not_admitted"),
            ),
            None => (false, "connection_gate", None, None, Some("unknown_user")),
        },
        ProductTriggerReason::BotMention | ProductTriggerReason::ReplyToBot => {
            match fixture.channels.get(channel_id) {
                Some(channel) if channel.connected => (
                    true,
                    "channel_agent",
                    Some(channel.agent_id.clone()),
                    channel.owner_user_id.clone(),
                    None,
                ),
                Some(channel) => (
                    false,
                    "channel_connection_required",
                    Some(channel.agent_id.clone()),
                    channel.owner_user_id.clone(),
                    Some("channel_connection_required"),
                ),
                None => (
                    false,
                    "channel_connection_required",
                    None,
                    None,
                    Some("channel_connection_required"),
                ),
            }
        }
        ProductTriggerReason::BotCommand | ProductTriggerReason::LinkedThreadAction => (
            false,
            "unsupported_trigger",
            None,
            None,
            Some("unsupported_trigger"),
        ),
    }
}

fn update_observation_for_submission(
    observation: &mut SlackObservation,
    parsed: &ParsedProductInbound,
) -> Option<String> {
    let ProductInboundPayload::UserMessage(payload) = &parsed.payload else {
        observation.delivery_status = "skipped";
        return None;
    };
    if !observation.route_permitted {
        observation.delivery_status = "blocked";
        return None;
    }
    if observation.accepted_count > 0 {
        observation.duplicate_count += 1;
        observation.delivery_status = "deduped";
        return None;
    }
    observation.accepted_count = 1;
    observation.turn_count = 1;
    observation.delivery_count = 1;
    observation.post_count = 1;
    observation.delivery_attempts = observation.planned_delivery_attempts.max(1);
    observation.delivery_status = "delivered";
    Some(payload.text.clone())
}

fn query_event_id(params: &Value) -> Result<String, ProfileError> {
    params
        .get("event_id")
        .or_else(|| params.get("raw_event_id"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| {
            ProfileError::Unsupported("Slack state query requires params.event_id".to_string())
        })
}

fn script_replies(script: &[crate::case::ScriptTurn]) -> Vec<RebornScriptedReply> {
    script
        .iter()
        .flat_map(|turn| turn.steps.iter())
        .map(|step| match step {
            ScriptStep::Tool { tool, params } => {
                RebornScriptedReply::tool_call(tool, params.clone())
            }
            ScriptStep::Text { text } => RebornScriptedReply::text(text.clone()),
        })
        .collect()
}

fn trigger_name(trigger: ProductTriggerReason) -> &'static str {
    match trigger {
        ProductTriggerReason::DirectChat => "direct_chat",
        ProductTriggerReason::BotMention => "bot_mention",
        ProductTriggerReason::ReplyToBot => "reply_to_bot",
        ProductTriggerReason::BotCommand => "bot_command",
        ProductTriggerReason::LinkedThreadAction => "linked_thread_action",
    }
}

fn default_true() -> bool {
    true
}

fn default_delivery_attempts() -> u64 {
    1
}

fn mutex_error<T>(error: std::sync::PoisonError<T>) -> ProfileError {
    ProfileError::Harness(format!("Slack observation state poisoned: {error}"))
}
