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
pub(crate) struct ExecutorPipeline<B, I, P, M, A, C, S, K, E> {
    pub(crate) budget: B,
    pub(crate) input: I,
    pub(crate) prompt: P,
    pub(crate) model: M,
    pub(crate) assistant_reply: A,
    pub(crate) capabilities: C,
    pub(crate) stop: S,
    pub(crate) checkpoint: K,
    pub(crate) exit: E,
}

pub(crate) type DefaultExecutorPipeline = ExecutorPipeline<
    BudgetStage,
    InputStage,
    PromptStage,
    ModelStage,
    AssistantReplyStage,
    CapabilityStage,
    StopStage,
    CheckpointStage,
    ExitStage,
>;
