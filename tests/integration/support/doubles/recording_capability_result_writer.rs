/// Test double substituting the production `LoopCapabilityResultWriter` impl
/// (`LocalDevCapabilityIo`, `crates/ironclaw_reborn_composition/src/runtime/local_dev.rs`).
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use ironclaw_host_api::CapabilityId;
use ironclaw_loop_support::{
    CapabilityResultWrite, CapabilityWriteResult, LoopCapabilityResultWriter,
};
use ironclaw_turns::{
    LoopResultRef,
    run_profile::{AgentLoopHostError, AgentLoopHostErrorKind, LoopRunContext},
};

use super::super::harness::RecordedCapabilityResult;

/// Wraps whatever real `LoopCapabilityResultWriter` the harness is currently
/// backed by -- production's ephemeral `ProductLiveCapabilityIo` test double
/// by default, or the real `LocalDevCapabilityIo` (durable tool-result
/// projection seam, issue #5838) when the harness opts into
/// `.with_durable_capability_io()`. Trait-object `inner` so this recorder is
/// agnostic to which one is underneath.
pub(crate) struct RecordingCapabilityResultWriter {
    pub(crate) inner: Arc<dyn LoopCapabilityResultWriter>,
    pub(crate) results: Arc<Mutex<Vec<RecordedCapabilityResult>>>,
}

#[async_trait]
impl LoopCapabilityResultWriter for RecordingCapabilityResultWriter {
    async fn write_capability_result(
        &self,
        write: CapabilityResultWrite<'_>,
    ) -> Result<CapabilityWriteResult, AgentLoopHostError> {
        let capability_id = write.capability_id.clone();
        let output = write.output.clone();
        let write_result = self.inner.write_capability_result(write).await?;
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
            .inner
            .update_capability_result(run_context, result_ref, output.clone())
            .await?;
        self.results.lock().unwrap().push(RecordedCapabilityResult {
            capability_id: CapabilityId::new(
                ironclaw_loop_support::DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID,
            )
            .map_err(|error| {
                AgentLoopHostError::new(AgentLoopHostErrorKind::Internal, error.to_string())
            })?,
            output,
        });
        Ok(byte_len)
    }
}
