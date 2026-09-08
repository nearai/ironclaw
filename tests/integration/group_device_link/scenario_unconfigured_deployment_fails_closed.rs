//! A deployment whose administrator never supplied the MTProto application
//! identity must fail a personal-account link closed **as a configuration
//! gap** — not as the generic internal failure the card used to render as
//! "something went wrong … cannot be completed for this account" (#7955).
//!
//! The scripted adapter stands in for Telegram's at exactly the seam the real
//! one fails on (`TelegramDeviceLinkAdapter::identity`); everything after it
//! is production: the extension host's driver, auth's step machine and
//! durable flow record, and the prompt projection the WebUI card renders.

use ironclaw_auth::{AuthErrorCode, AuthFlowStatus, product_prompt::device_link_view_for_flow};
use ironclaw_extension_contracts::device_link::{
    DeviceLinkError, DeviceLinkErrorCode, DeviceLinkStep,
};

use super::reborn_support::group::{HarnessResult, RebornIntegrationGroup};
use super::reborn_support::harness::profiles::device_link::{
    LINKED_EXTENSION_ID, LINKED_VENDOR_ID, LinkedFixtureHandles,
};
use super::reborn_support::reply::RebornScriptedReply;

/// Its own actor: the failed flow must not share a credential-owner scope
/// with the accounts the other scenarios mint.
const UNCONFIGURED_ACTOR_ID: &str = "device-link-unconfigured-actor";

pub async fn run(
    group: &RebornIntegrationGroup,
    handles: &LinkedFixtureHandles,
) -> HarnessResult<()> {
    let linker = group
        .thread("conv-device-link-unconfigured")
        .with_actor_id(UNCONFIGURED_ACTOR_ID)
        .script([RebornScriptedReply::text("ready to link")])
        .build()
        .await?;
    linker.submit_turn("ready to link").await?;

    handles
        .device_link
        .fail_next_begin(DeviceLinkError::NotConfigured {
            reason: "itest: the deployment has no MTProto application identity",
        });
    let record = linker
        .start_device_link_through_product_auth(LINKED_VENDOR_ID, LINKED_EXTENSION_ID)
        .await?;

    // The durable record: terminal, and filed as configuration — not as the
    // backend outage `Internal` used to become in the audit trail.
    if record.status != AuthFlowStatus::Failed {
        return Err(format!(
            "an unconfigured deployment must terminalize the flow, got {:?}",
            record.status
        )
        .into());
    }
    if record.error != Some(AuthErrorCode::MalformedConfig) {
        return Err(format!(
            "an operator omission must be recorded as malformed_config, got {:?}",
            record.error
        )
        .into());
    }
    match record.device_link_step() {
        Some(DeviceLinkStep::Failed {
            code: DeviceLinkErrorCode::NotConfigured,
            restartable: false,
        }) => {}
        other => {
            return Err(format!("expected a terminal not_configured frame, got {other:?}").into());
        }
    }

    // The projection the card renders: names the administrator, carries the
    // typed code for the remedy line, and never blames the user's account.
    let view = device_link_view_for_flow(&record)
        .ok_or("a device-link flow must project a device-link prompt view")?;
    if view.error_code != Some(DeviceLinkErrorCode::NotConfigured) {
        return Err(format!(
            "the card must receive the not_configured code, got {:?}",
            view.error_code
        )
        .into());
    }
    if view.restartable != Some(false) {
        return Err("an operator omission must not offer the user a retry".into());
    }
    if !view.instructions.contains("administrator")
        || view.instructions.contains("for this account")
    {
        return Err(format!(
            "the card copy must name the administrator, not the account: {:?}",
            view.instructions
        )
        .into());
    }
    Ok(())
}
