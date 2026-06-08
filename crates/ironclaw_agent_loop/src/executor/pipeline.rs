use async_trait::async_trait;
use ironclaw_turns::run_profile::AgentLoopDriverHost;

use crate::planner::AgentLoopPlannerInternal;

use super::{
    AgentLoopExecutorError, AssistantReplyStage, BudgetStage, CapabilityStage, ExitStage,
    InputStage, ModelStage, PostCapabilityStage, PromptStage, ReplyAdmissionStage, StopStage,
};

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

#[derive(Debug, Clone)]
pub(crate) struct DefaultExecutorPipeline {
    pub(crate) budget: BudgetStage,
    pub(crate) input: InputStage,
    pub(crate) prompt: PromptStage,
    pub(crate) model: ModelStage,
    pub(crate) reply_admission: ReplyAdmissionStage,
    pub(crate) assistant_reply: AssistantReplyStage,
    pub(crate) capabilities: CapabilityStage,
    pub(crate) post_capability: PostCapabilityStage,
    pub(crate) stop: StopStage,
    pub(crate) exit: ExitStage,
}

impl Default for DefaultExecutorPipeline {
    fn default() -> Self {
        Self {
            budget: BudgetStage,
            input: InputStage,
            prompt: PromptStage,
            model: ModelStage,
            reply_admission: ReplyAdmissionStage,
            assistant_reply: AssistantReplyStage,
            capabilities: CapabilityStage,
            post_capability: PostCapabilityStage::default(),
            stop: StopStage,
            exit: ExitStage,
        }
    }
}
