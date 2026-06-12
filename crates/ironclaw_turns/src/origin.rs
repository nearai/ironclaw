use serde::{Deserialize, Serialize};

/// How this turn run was initiated.
///
/// Carried through `SubmitTurnRequest` → `TurnRunState` → `LoopRunContext`
/// and rendered into the model-visible runtime context section.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum TurnRunOrigin {
    WebUiChat,
    ProductInbound { adapter: String },
    ScheduledTrigger,
}
