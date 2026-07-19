/// Test double substituting the production `LoopCapabilityResultWriter` impl
/// (`StagedCapabilityIo`, `crates/ironclaw_reborn_composition/src/runtime/local_dev.rs`).
///
/// Also implements `LoopCapabilityInputResolver`, delegating straight to
/// `input_resolver` (no recording — only result writes are recorded).
/// Harness-port-seam P1 Change 2: production assigns ONE `StagedCapabilityIo`
/// to both the `input_resolver` and `result_writer` config roles so
/// input-ref/result-ref correlation by `call_id` works; this double must be
/// usable the same way — `input_resolver` and `result_writer` below must be
/// two `Arc::clone`s (via distinct trait-object views, e.g. the harness's
/// `input_resolver()`/`result_writer_io()` accessors) of the SAME underlying
/// concrete io object, never two independently-sourced ones. Two separate
/// trait-object fields (rather than one field implementing both traits)
/// because the harness's own `io`/`result_writer_io` are themselves already
/// split into two `Mutex<Arc<dyn ...>>` views for the durable-io runtime swap
/// (`install_durable_capability_io`, issue #5838).
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use ironclaw_host_api::CapabilityId;
use ironclaw_loop_host::{
    CapabilityResultWrite, CapabilityWriteResult, LoopCapabilityInputResolver,
    LoopCapabilityResultWriter,
};
use ironclaw_turns::{
    LoopResultRef,
    run_profile::{
        AgentLoopHostError, AgentLoopHostErrorKind, CapabilityInputRef, LoopRunContext,
        ProviderToolCall,
    },
};

use super::super::harness::RecordedCapabilityResult;

/// Wraps whatever real io the harness is currently backed by -- production's
/// ephemeral `ProductLiveCapabilityIo` test double by default, or the real
/// `StagedCapabilityIo` (durable tool-result projection seam, issue #5838)
/// when the harness opts into `.with_durable_capability_io()`. Trait-object
/// fields so this recorder is agnostic to which one is underneath.
pub(crate) struct RecordingCapabilityResultWriter {
    pub(crate) input_resolver: Arc<dyn LoopCapabilityInputResolver>,
    pub(crate) result_writer: Arc<dyn LoopCapabilityResultWriter>,
    pub(crate) results: Arc<Mutex<Vec<RecordedCapabilityResult>>>,
}

#[async_trait]
impl LoopCapabilityInputResolver for RecordingCapabilityResultWriter {
    async fn resolve_capability_input(
        &self,
        run_context: &LoopRunContext,
        input_ref: &CapabilityInputRef,
    ) -> Result<serde_json::Value, AgentLoopHostError> {
        self.input_resolver
            .resolve_capability_input(run_context, input_ref)
            .await
    }

    async fn register_provider_tool_call_input(
        &self,
        run_context: &LoopRunContext,
        tool_call: &ProviderToolCall,
    ) -> Result<CapabilityInputRef, AgentLoopHostError> {
        self.input_resolver
            .register_provider_tool_call_input(run_context, tool_call)
            .await
    }
}

#[async_trait]
impl LoopCapabilityResultWriter for RecordingCapabilityResultWriter {
    async fn write_capability_result(
        &self,
        write: CapabilityResultWrite<'_>,
    ) -> Result<CapabilityWriteResult, AgentLoopHostError> {
        let capability_id = write.capability_id.clone();
        let output = write.output.clone();
        let write_result = self.result_writer.write_capability_result(write).await?;
        self.results.lock().unwrap().push(RecordedCapabilityResult {
            capability_id,
            output,
        });
        Ok(write_result)
    }

    async fn update_capability_result(
        &self,
        run_context: &LoopRunContext,
        result_ref: &LoopResultRef,
        output: serde_json::Value,
    ) -> Result<u64, AgentLoopHostError> {
        let byte_len = self
            .result_writer
            .update_capability_result(run_context, result_ref, output.clone())
            .await?;
        self.results.lock().unwrap().push(RecordedCapabilityResult {
            capability_id: CapabilityId::new(
                ironclaw_loop_host::DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID,
            )
            .map_err(|error| {
                AgentLoopHostError::new(AgentLoopHostErrorKind::Internal, error.to_string())
            })?,
            output,
        });
        Ok(byte_len)
    }
}
