export enum ActivityKind {
  TurnStarted = "turn_started",
  PromptPrepared = "prompt_prepared",
  ModelCallStarted = "model_call_started",
  ModelCallCompleted = "model_call_completed",
  ModelCallFailed = "model_call_failed",
  Progress = "progress",
  ToolStarted = "tool_started",
  ToolCompleted = "tool_completed",
  ToolFailed = "tool_failed",
  GateBlocked = "gate_blocked",
  FinalResponseCompleted = "final_response_completed",
  StreamDisconnected = "stream_disconnected",
  StreamResumed = "stream_resumed",
}

export const MAX_INSPECTOR_ACTIVITY_ENTRIES = 1_000;

export function activityKindFromWire(value: unknown): ActivityKind | null {
  switch (value) {
    case ActivityKind.TurnStarted:
    case ActivityKind.PromptPrepared:
    case ActivityKind.ModelCallStarted:
    case ActivityKind.ModelCallCompleted:
    case ActivityKind.ModelCallFailed:
    case ActivityKind.Progress:
    case ActivityKind.ToolStarted:
    case ActivityKind.ToolCompleted:
    case ActivityKind.ToolFailed:
    case ActivityKind.GateBlocked:
    case ActivityKind.FinalResponseCompleted:
    case ActivityKind.StreamDisconnected:
    case ActivityKind.StreamResumed:
      return value;
    default:
      return null;
  }
}
