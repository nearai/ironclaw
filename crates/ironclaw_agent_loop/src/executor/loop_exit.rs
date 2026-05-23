use super::*;

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct ExitStage;

impl ExitStage {
    pub(super) async fn for_stop(
        &self,
        ctx: StageContext<'_>,
        state: LoopExecutionState,
        kind: StopKind,
    ) -> Result<LoopExit, AgentLoopExecutorError> {
        match kind {
            StopKind::GracefulStop => {
                let checked = CheckpointStage
                    .write(ctx, state, CheckpointKind::Final)
                    .await?;
                completed_exit(ctx.host, checked.state, Some(checked.checkpoint_id))
            }
            StopKind::NoProgressDetected => {
                let checked = CheckpointStage
                    .write(ctx, state, CheckpointKind::Final)
                    .await?;
                failed_exit(
                    ctx.host,
                    checked.state,
                    LoopFailureKind::NoProgressDetected,
                    Some(checked.checkpoint_id),
                )
            }
            StopKind::Aborted(failure_kind) => {
                let checked = CheckpointStage
                    .write(ctx, state, CheckpointKind::Final)
                    .await?;
                failed_exit(
                    ctx.host,
                    checked.state,
                    failure_kind,
                    Some(checked.checkpoint_id),
                )
            }
        }
    }
}
