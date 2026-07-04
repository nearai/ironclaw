//! C-COMMCTX: a recording [`CommunicationContextProvider`] test double.
//!
//! Wired into a harness/group via `with_communication_context_provider` /
//! `RebornIntegrationGroupBuilder::communication_context_provider`, this double
//! returns a fixed delivery-preference / connected-channel slice so a test can
//! prove the wired `communication_context_provider` reaches the turn pipeline —
//! the slice renders into the model request (assert via
//! `assert_model_request_contains`).
//!
//! This is DISTINCT from the outbound delivery **sink** (E-OUTBOUND, a sibling
//! lane): this is prompt **context** (delivery preferences/targets), not a
//! delivery recorder. The production `RuntimeCommunicationContextProvider`'s
//! facade→context mapping is already densely unit-tested in
//! `crates/ironclaw_reborn_composition/src/communication_context.rs`; this double
//! deliberately covers only the int-tier gap — that the `communication_context_provider`
//! field wires through the coordinator path into the model request — without
//! re-authoring that crate-tier mapping coverage.

// Shared integration-test support: not every binary that mounts the
// `reborn_support` tree consumes this module, so its symbols read as dead there
// under the all-features `-D warnings` lane.
#![allow(dead_code)]

use std::sync::Arc;

use ironclaw_turns::run_profile::{
    CommunicationContextFetch, CommunicationContextProvider, CommunicationRuntimeContext,
    ConnectedChannelSummary, ConnectedChannelsState, DeliveryTargetState, DeliveryTargetSummary,
};
use ironclaw_turns::scope::{TurnActor, TurnScope};

/// A [`CommunicationContextProvider`] that returns a pre-resolved
/// [`CommunicationRuntimeContext`] regardless of scope/actor. Mirrors the
/// loop-driver-host `StubCommunicationContextProvider` shape but with a
/// *configured* delivery target + connected channel so the rendered slice
/// carries distinctive sentinels a test can assert on.
pub struct RecordingCommunicationContextProvider {
    context: CommunicationRuntimeContext,
}

impl RecordingCommunicationContextProvider {
    /// Provider that reports a single connected channel `channel_name` and a
    /// configured outbound delivery target (`display_name` on `channel`). The
    /// rendered model-context slice reads
    /// `Connected channels: <channel_name> (authenticated, active).` and
    /// `Outbound delivery target: <display_name> (<channel>) — applies to ...`.
    pub fn with_target_and_channel(
        display_name: impl Into<String>,
        channel: impl Into<String>,
        channel_name: impl Into<String>,
    ) -> Arc<dyn CommunicationContextProvider> {
        Arc::new(Self {
            context: CommunicationRuntimeContext {
                connected_channels: ConnectedChannelsState::Known(vec![ConnectedChannelSummary {
                    name: channel_name.into(),
                    authenticated: true,
                    active: true,
                }]),
                delivery_target: DeliveryTargetState::Set(DeliveryTargetSummary {
                    display_name: display_name.into(),
                    channel: channel.into(),
                }),
                // Placeholder; the host stamps the surface-derived value in
                // `CommunicationContextFetch::resolve`, mirroring production.
                delivery_tools_visible: false,
            },
        })
    }
}

impl CommunicationContextProvider for RecordingCommunicationContextProvider {
    fn begin_communication_context(
        &self,
        _scope: TurnScope,
        _actor: Option<TurnActor>,
    ) -> CommunicationContextFetch {
        CommunicationContextFetch::from_ready(Some(self.context.clone()))
    }
}
