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
pub(crate) struct DefaultExecutorPipeline {
    pub(crate) budget: BudgetStage,
    pub(crate) input: InputStage,
    pub(crate) prompt: PromptStage,
    pub(crate) model: ModelStage,
    pub(crate) assistant_reply: AssistantReplyStage,
    pub(crate) capabilities: CapabilityStage,
    pub(crate) stop: StopStage,
    pub(crate) checkpoint: CheckpointStage,
    pub(crate) exit: ExitStage,
}
