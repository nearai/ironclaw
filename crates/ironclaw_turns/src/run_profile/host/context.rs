//! Context-loading DTOs, the [`LoopContextPort`], and the run-scoped
//! [`LoopInputCursor`] that paginates loop context and input.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{LoopMessageRef, TurnRunId, TurnScope};

use super::error::AgentLoopHostError;
use super::model::PromptMode;
use super::refs::{LoopInputCursorToken, origin_input_cursor_token};
use super::run_context::LoopRunContext;
use crate::run_profile::SkillTrustLevel;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoopContextRequest {
    pub after: Option<LoopInputCursor>,
    pub limit: usize,
    #[serde(default = "default_prompt_mode")]
    pub mode: PromptMode,
}

fn default_prompt_mode() -> PromptMode {
    PromptMode::TextOnly
}

pub const LOOP_CONTEXT_SNIPPET_MODEL_CONTENT_MAX_BYTES: usize = 64 * 1024;
pub const LOOP_CONTEXT_TOTAL_MODEL_CONTENT_MAX_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LoopContextBundle {
    pub identity_messages: Vec<LoopContextMessage>,
    pub messages: Vec<LoopContextMessage>,
    pub compaction_message_index: Vec<LoopContextCompactionMetadata>,
    pub instruction_snippets: Vec<LoopContextSnippet>,
    pub memory_snippets: Vec<LoopContextSnippet>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoopContextMessage {
    /// Reference to the persisted message content.
    ///
    /// `None` means "summary-only entry; prompt port MUST NOT resolve content —
    /// use `safe_summary` verbatim instead." Mirrors the
    /// `SkillTrustLevel::Installed` carrying `prompt_content: None` pattern.
    pub message_ref: Option<LoopMessageRef>,
    pub role: String,
    pub safe_summary: String,
    pub compaction: Option<LoopContextCompactionMetadata>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoopContextCompactionMetadata {
    pub sequence: u64,
    pub kind: LoopContextCompactionKind,
    pub estimated_tokens: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopContextCompactionKind {
    User,
    Assistant,
    System,
    Summary,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoopContextSnippetMetadata {
    pub source_name: String,
    pub trust_level: SkillTrustLevel,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoopContextSnippet {
    pub snippet_ref: String,
    /// Full model-visible content for this context snippet.
    ///
    /// This is intentionally distinct from `safe_summary`: prompt assembly must
    /// materialize this field, while summaries remain short metadata for
    /// fingerprints, transcript displays, and diagnostics.
    pub model_content: String,
    pub safe_summary: String,
    /// Safe metadata for prompt milestones. Skill snippet producers using the
    /// `skill:` ref namespace must populate this so telemetry can record active
    /// skill name/trust without leaking prompt content.
    pub metadata: Option<LoopContextSnippetMetadata>,
}

#[async_trait]
pub trait LoopContextPort: Send + Sync {
    async fn load_loop_context(
        &self,
        request: LoopContextRequest,
    ) -> Result<LoopContextBundle, AgentLoopHostError>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoopInputCursor {
    scope: TurnScope,
    run_id: TurnRunId,
    token: LoopInputCursorToken,
}

impl LoopInputCursor {
    pub fn origin_for_run(context: &LoopRunContext) -> Self {
        Self {
            scope: context.scope.clone(),
            run_id: context.run_id,
            token: origin_input_cursor_token(),
        }
    }

    pub fn from_host_token(context: &LoopRunContext, token: LoopInputCursorToken) -> Self {
        Self {
            scope: context.scope.clone(),
            run_id: context.run_id,
            token,
        }
    }

    pub fn scope(&self) -> &TurnScope {
        &self.scope
    }

    pub fn run_id(&self) -> TurnRunId {
        self.run_id
    }

    pub fn token(&self) -> &LoopInputCursorToken {
        &self.token
    }

    pub fn is_for_run(&self, context: &LoopRunContext) -> bool {
        self.scope == context.scope && self.run_id == context.run_id
    }
}
