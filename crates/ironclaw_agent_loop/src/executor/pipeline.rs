use super::*;

#[derive(Clone, Copy)]
pub(crate) struct StageContext<'a> {
    pub(crate) planner: &'a dyn AgentLoopPlannerInternal,
    pub(crate) host: &'a (dyn AgentLoopDriverHost + Send + Sync),
}

#[async_trait]
pub(crate) trait ExecutorStage<Input>: Send + Sync {
    type Output;

    async fn process(
        &self,
        ctx: StageContext<'_>,
        input: Input,
    ) -> Result<Self::Output, AgentLoopExecutorError>;
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct ExecutorPipeline<P, M, C, G, K, E> {
    pub(crate) prompt: P,
    pub(crate) model: M,
    pub(crate) capabilities: C,
    pub(crate) gates: G,
    pub(crate) checkpoint: K,
    pub(crate) exit: E,
}

pub(crate) type DefaultExecutorPipeline = ExecutorPipeline<
    PromptStage,
    ModelStage,
    CapabilityStage,
    GateStage,
    CheckpointStage,
    ExitStage,
>;
