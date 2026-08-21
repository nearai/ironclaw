//! #7742: automation authoring performs only persistence-critical discovery
//! and reaches one visible outcome: ready, needs_setup, or needs_input.

use super::reborn_support::assertions::ToolErrorClass;
use super::reborn_support::builder::RebornIntegrationHarness;
use super::reborn_support::group::{HarnessResult, RebornIntegrationGroup};
use super::reborn_support::reply::RebornScriptedReply;
use ironclaw_triggers::TriggerSchedule;
use serde_json::json;

const READY_NAME: &str = "x-releases deployment status";
const SLACK_TARGET: &str = "slack:channel:x-releases";
const TARGETS_LIST: &str = "builtin.outbound_delivery_targets_list";
const TRIGGER_CREATE: &str = "builtin.trigger_create";

pub async fn run(g: &RebornIntegrationGroup) -> HarnessResult<()> {
    ready_named_slack_channel(g).await?;
    needs_input_for_missing_status_endpoint(g).await?;
    needs_setup_for_missing_telegram_destination(g, false).await?;
    needs_setup_for_missing_telegram_destination(g, true).await?;
    needs_setup_after_required_target_auth_failure(g).await?;
    Ok(())
}

async fn ready_named_slack_channel(g: &RebornIntegrationGroup) -> HarnessResult<()> {
    let h = g
        .thread("automation-preflight-ready-slack")
        .script([
            RebornScriptedReply::tool_call(TARGETS_LIST, json!({"channel": "slack"})),
            RebornScriptedReply::tool_call(
                TRIGGER_CREATE,
                json!({
                    "name": READY_NAME,
                    "execution_contract": super::support::trigger_execution_contract(format!(
                        "Check deployment status, then deliver the summary with builtin__outbound_deliver to {SLACK_TARGET}"
                    )),
                    "schedule": {"kind": "cron", "expression": "0 9 * * *", "timezone": "UTC"},
                }),
            ),
            RebornScriptedReply::text("ready: created the deployment status routine"),
        ])
        .build()
        .await?;

    h.submit_turn("Every day at 9 UTC, post deployment status in #x-releases")
        .await?;
    assert_authoring_contract_reached_model(&h)?;
    h.assert_reply_contains("ready:").await?;
    h.assert_tool_invocation_count(TARGETS_LIST, 1).await?;
    h.assert_tool_invocation_count(TRIGGER_CREATE, 1).await?;
    h.assert_only_tools_invoked(&[TARGETS_LIST, TRIGGER_CREATE])
        .await?;
    assert_no_exploratory_side_effects(&h).await?;

    let records = trigger_records(g, &h).await?;
    if records.len() != 1 {
        return Err(
            format!("ready authoring must persist exactly one trigger: {records:#?}").into(),
        );
    }
    let record = &records[0];
    let goal = record
        .execution_spec
        .as_ref()
        .ok_or("created trigger missing structured execution spec")?
        .goal
        .as_str();
    if record.name != READY_NAME || !goal.contains(SLACK_TARGET) {
        return Err(
            format!("created trigger did not pin the resolved destination: {record:#?}").into(),
        );
    }
    match &record.schedule {
        TriggerSchedule::Cron {
            expression,
            timezone,
        } if expression == "0 9 * * *" && timezone == "UTC" => {}
        schedule => {
            return Err(format!(
                "daily authoring must persist its recurring schedule: {schedule:#?}"
            )
            .into());
        }
    }
    Ok(())
}

async fn needs_input_for_missing_status_endpoint(g: &RebornIntegrationGroup) -> HarnessResult<()> {
    let h = g
        .thread("automation-preflight-needs-input-status-url")
        .script([RebornScriptedReply::text(
            "needs_input: What exact status endpoint URL should the routine check?",
        )])
        .build()
        .await?;
    let before = trigger_records(g, &h).await?.len();

    h.submit_turn("Every five minutes, check our status endpoint and alert me if it is down")
        .await?;
    h.assert_reply_contains("needs_input:").await?;
    h.assert_reply_contains("endpoint URL").await?;
    h.assert_only_tools_invoked(&[]).await?;
    assert_no_exploratory_side_effects(&h).await?;
    assert_trigger_count_unchanged(g, &h, before, "needs_input").await
}

async fn needs_setup_for_missing_telegram_destination(
    g: &RebornIntegrationGroup,
    custom_tool_requested: bool,
) -> HarnessResult<()> {
    let thread = if custom_tool_requested {
        "automation-preflight-needs-setup-custom-tool"
    } else {
        "automation-preflight-needs-setup-telegram"
    };
    let request = if custom_tool_requested {
        "Every hour, use the installed custom MCP status tool and send the result to Telegram"
    } else {
        "Every morning, send my routine summary to Telegram"
    };
    let h = g
        .thread(thread)
        .script([
            RebornScriptedReply::tool_call(TARGETS_LIST, json!({"channel": "telegram"})),
            RebornScriptedReply::text(
                "needs_setup: Connect Telegram and expose a delivery destination, then ask me to create this routine again.",
            ),
        ])
        .build()
        .await?;
    let before = trigger_records(g, &h).await?.len();

    h.submit_turn(request).await?;
    let output = h.tool_result_output(TARGETS_LIST).await?;
    if output["targets"] != json!([]) {
        return Err(
            format!("Telegram preflight must observe an empty target list: {output}").into(),
        );
    }
    h.assert_reply_contains("needs_setup:").await?;
    h.assert_reply_contains("Connect Telegram").await?;
    h.assert_only_tools_invoked(&[TARGETS_LIST]).await?;
    assert_no_exploratory_side_effects(&h).await?;
    assert_trigger_count_unchanged(g, &h, before, "needs_setup").await
}

async fn needs_setup_after_required_target_auth_failure(
    g: &RebornIntegrationGroup,
) -> HarnessResult<()> {
    let service = g
        .capability_harness()
        .ok_or("automation authoring group uses HostRuntime")?
        .outbound_preferences_service_for_test()
        .ok_or("automation authoring group exposes its target service")?;
    service.set_list_requires_setup(true);

    let h = g
        .thread("automation-preflight-needs-setup-auth")
        .script([
            RebornScriptedReply::tool_call(TARGETS_LIST, json!({"channel": "slack"})),
            RebornScriptedReply::text(
                "needs_setup: Reconnect the delivery integration, then ask me to create the routine again.",
            ),
        ])
        .build()
        .await?;
    let before = trigger_records(g, &h).await?.len();

    h.submit_turn("Every day, send the status summary to Slack")
        .await?;
    service.set_list_requires_setup(false);
    h.assert_tool_error(
        ToolErrorClass::Denied,
        "not permitted to list outbound delivery targets",
    )
    .await?;
    h.assert_reply_contains("needs_setup:").await?;
    h.assert_reply_contains("Reconnect").await?;
    h.assert_only_tools_invoked(&[TARGETS_LIST]).await?;
    assert_no_exploratory_side_effects(&h).await?;
    assert_trigger_count_unchanged(g, &h, before, "auth needs_setup").await
}

fn assert_authoring_contract_reached_model(h: &RebornIntegrationHarness) -> HarnessResult<()> {
    let definitions = h
        .scripted_llm
        .captured_tool_definitions()
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    for offered in ["builtin__http", "builtin__shell"] {
        if !definitions
            .iter()
            .any(|definition| definition.name == offered)
        {
            return Err(
                format!("negative-control exploration tool {offered} was not offered").into(),
            );
        }
    }
    let definition = definitions
        .into_iter()
        .find(|definition| definition.name == "builtin__trigger_create")
        .ok_or("model was never handed builtin__trigger_create")?;
    for needle in ["ready", "needs_setup", "needs_input", "create immediately"] {
        if !definition.description.contains(needle) {
            return Err(format!("trigger_create description omitted {needle:?}").into());
        }
    }
    let properties = definition.parameters["properties"]
        .as_object()
        .ok_or("trigger_create parameters omitted properties")?;
    let policy_properties = properties["execution_contract"]["properties"]["policy"]["properties"]
        .as_object()
        .ok_or("trigger_create parameters omitted execution policy properties")?;
    if policy_properties.contains_key("allowed_capability_ids")
        || policy_properties.contains_key("capability_ids")
        || policy_properties.contains_key("capability_allowlist")
    {
        return Err("authoring must not pin future capability ids or allowlists".into());
    }
    Ok(())
}

async fn assert_no_exploratory_side_effects(h: &RebornIntegrationHarness) -> HarnessResult<()> {
    h.assert_tool_not_invoked("builtin.http").await?;
    h.assert_tool_not_invoked("builtin.shell").await?;
    h.assert_egress_count(0).await
}

async fn trigger_records(
    g: &RebornIntegrationGroup,
    h: &RebornIntegrationHarness,
) -> HarnessResult<Vec<ironclaw_triggers::TriggerRecord>> {
    g.capability_harness()
        .ok_or("automation authoring group uses HostRuntime")?
        .trigger_repository_for_test()
        .ok_or("automation authoring group exposes its trigger repository")?
        .list_triggers(h.binding.tenant_id.clone())
        .await
        .map_err(Into::into)
}

async fn assert_trigger_count_unchanged(
    g: &RebornIntegrationGroup,
    h: &RebornIntegrationHarness,
    before: usize,
    outcome: &str,
) -> HarnessResult<()> {
    let after = trigger_records(g, h).await?.len();
    if after == before {
        return Ok(());
    }
    Err(format!("{outcome} authoring persisted a trigger: before={before}, after={after}").into())
}
