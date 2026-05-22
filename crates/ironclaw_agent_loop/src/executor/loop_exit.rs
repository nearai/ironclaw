use super::*;

impl CanonicalAgentLoopExecutor {
    pub(super) async fn exit_for_stop(
        &self,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
        state: LoopExecutionState,
        kind: StopKind,
    ) -> Result<LoopExit, AgentLoopExecutorError> {
        match kind {
            StopKind::GracefulStop => {
                let checked = self.checkpoint(host, state, CheckpointKind::Final).await?;
                completed_exit(host, checked.state, Some(checked.checkpoint_id))
            }
            StopKind::NoProgressDetected => {
                let checked = self.checkpoint(host, state, CheckpointKind::Final).await?;
                failed_exit(
                    host,
                    checked.state,
                    LoopFailureKind::NoProgressDetected,
                    Some(checked.checkpoint_id),
                )
            }
            StopKind::Aborted(failure_kind) => {
                let checked = self.checkpoint(host, state, CheckpointKind::Final).await?;
                failed_exit(
                    host,
                    checked.state,
                    failure_kind,
                    Some(checked.checkpoint_id),
                )
            }
        }
    }
}
