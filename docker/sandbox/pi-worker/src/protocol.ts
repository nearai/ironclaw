/**
 * TypeScript mirror of the IronClaw loop-worker wire protocol
 * (`crates/loop/ironclaw_loop_host/src/remote_host/protocol.rs`, wire v2).
 *
 * Every type here matches serde's JSON output exactly: snake_case fields,
 * externally tagged enums (`{"HostRequest": {...}}`), and Rust `Result` as
 * `{"Ok": ...}` / `{"Err": ...}`. Opaque host-owned values (checkpoint bytes,
 * capability views, run profiles) are typed `unknown` and passed through
 * verbatim — the worker never interprets them.
 */

/** `LOOP_WORKER_WIRE_VERSION` — v2 adds `content_visibility` and `ResolveMessages`. */
export const LOOP_WORKER_WIRE_VERSION = 2;

/** `LOOP_WORKER_MAX_FRAME_BYTES` = `MAX_SANDBOX_LOOP_WORKER_FRAME_BYTES`. */
export const LOOP_WORKER_MAX_FRAME_BYTES = 1024 * 1024;

// ---------------------------------------------------------------------------
// Bootstrap
// ---------------------------------------------------------------------------

/** `WorkerContentVisibility` — serde `rename_all = "snake_case"`. */
export type WorkerContentVisibility = "blind" | "resolved";

/**
 * `LoopWorkerBootstrap`. Opaque fields (`run_context`, `invocation`,
 * `current_visible_capabilities`) are passed through untouched; only the
 * fields the worker must act on are narrowed.
 */
export interface LoopWorkerBootstrap {
  wire_version: number;
  run_context: LoopRunContext;
  invocation: LoopWorkerInvocation;
  settings: LoopWorkerSettings;
  tool_definitions: ProviderToolDefinition[];
  current_visible_capabilities: unknown | null;
  /** v2; serde default `blind` so v1-shaped bootstraps decode. */
  content_visibility?: WorkerContentVisibility;
}

export interface LoopWorkerSettings {
  default_iteration_limit: number | null;
  model_availability_attempts: number | null;
}

/**
 * `LoopWorkerInvocation { Run(...)|Resume(...) }` — externally tagged.
 * The request payloads (`AgentLoopDriverRunRequest` / `ResumeRequest`) are
 * opaque to the worker: the host drives scoping and the worker never reads
 * `resolved_run_profile` (the host enforces budgets itself).
 */
export type LoopWorkerInvocation =
  { Run: AgentLoopDriverRunRequest } | { Resume: AgentLoopDriverResumeRequest };

/** `AgentLoopDriverRunRequest` shape, kept for the conformance fake host. */
export interface AgentLoopDriverRunRequest {
  turn_id: string;
  run_id: string;
  resolved_run_profile: unknown;
}

/** `AgentLoopDriverResumeRequest` shape, kept for the conformance fake host. */
export interface AgentLoopDriverResumeRequest {
  turn_id: string;
  run_id: string;
  checkpoint_id: string;
  resolved_run_profile: unknown;
  auth_resume_disposition?: "denied" | null;
}

// ---------------------------------------------------------------------------
// Loop context (mirrors `host/run_context.rs`; opaque pass-through)
// ---------------------------------------------------------------------------

/** `LoopRunContext` — opaque to the worker; echoed in `BuildPrompt`. */
export interface LoopRunContext {
  scope: unknown;
  actor?: unknown;
  accepted_message_ref?: unknown;
  thread_id: string;
  turn_id: string;
  run_id: string;
  resolved_run_profile: unknown;
  resolved_model_route?: unknown;
  loop_driver_id: string;
  loop_driver_version: number;
  checkpoint_schema_id: string;
  checkpoint_schema_version: number;
  output_contract?: unknown;
  product_context?: unknown;
}

// ---------------------------------------------------------------------------
// Host calls the worker may issue (subset it uses)
// ---------------------------------------------------------------------------

/** `ProviderToolDefinition` (`description_trust` is `#[serde(skip)]`). */
export interface ProviderToolDefinition {
  capability_id: string;
  name: string;
  description: string;
  parameters: unknown;
}

/**
 * `HostCall` — externally tagged. Only the calls the Pi worker issues are
 * typed precisely; the rest stay `unknown` payloads.
 */
export type HostCall =
  | { BuildPrompt: LoopPromptBundleRequest }
  | { ResolveMessages: ResolveMessagesRequest }
  | { StreamModel: LoopModelRequest }
  | { RegisterProviderToolCall: RegisterProviderToolCallRequest }
  | { InvokeCapability: LoopRequest }
  | { BeginAssistantDraft: BeginAssistantDraft }
  | { UpdateAssistantDraft: UpdateAssistantDraft }
  | { FinalizeAssistantMessage: FinalizeAssistantMessage }
  | { AppendCapabilityResultRef: AppendCapabilityResultRef }
  | { StageCheckpointPayload: StageCheckpointPayloadRequest }
  | { EmitProgress: unknown }
  | { [variant: string]: unknown };

// --- prompt bundle (host/model.rs) ---

/** `PromptMode` — serde snake_case. */
export type PromptMode = "text_only" | "codeact";

/**
 * `LoopInlineMessageRole` — serde snake_case. Leading messages must stay in
 * the provider-cached leading block.
 */
export type LoopInlineMessageRole =
  "leading_system" | "leading_user" | "system" | "user" | "assistant";

export interface LoopInlineMessage {
  role: LoopInlineMessageRole;
  safe_body: string;
}

export interface LoopPromptBundleRequest {
  mode: PromptMode;
  context_cursor: LoopInputCursor | null;
  surface_version: string | null;
  capability_view?: LoopModelCapabilityView | null;
  checkpoint_state_ref: string | null;
  max_messages: number | null;
  inline_messages?: LoopInlineMessage[];
}

export interface LoopInputCursor {
  scope: unknown;
  run_id: string;
  token: string;
}

export interface LoopModelCapabilityView {
  visible_capability_ids: string[];
}

/** `LoopPromptBundle` — the host's prompt bundle response. */
export interface LoopPromptBundle {
  bundle_ref: string;
  messages: LoopModelMessage[];
  surface_version: string | null;
  compaction_message_index?: LoopContextCompactionMetadata[];
  recent_window_truncation?: unknown;
  instruction_fingerprint?: unknown;
  identity_message_count: number;
  instruction_snippet_count: number;
}

export interface LoopContextCompactionMetadata {
  [key: string]: unknown;
}

// --- message refs and resolution (v2) ---

/** `LoopModelMessage` — role + opaque ref. */
export interface LoopModelMessage {
  role: string;
  content_ref: string;
}

/** v2 `ResolveMessages` request. */
export interface ResolveMessagesRequest {
  messages: LoopModelMessage[];
}

/** v2 `WireResolvedToolResult`. */
export interface WireResolvedToolResult {
  provider_call_id: string | null;
  content: string;
}

/** v2 `WireResolvedModelMessage`. */
export interface WireResolvedModelMessage {
  role: string;
  content_ref: string;
  content: string;
  tool_result?: WireResolvedToolResult | null;
}

// --- model call ---

export interface LoopModelRequest {
  messages: LoopModelMessage[];
  inline_messages?: LoopInlineMessage[];
  surface_version: string | null;
  model_preference: string | null;
  fallback_index: number;
  iteration: number;
  capability_view?: LoopModelCapabilityView | null;
  tool_choice?: LoopModelToolChoice | null;
}

/** `LoopModelToolChoice` — serde `tag = "kind"`, snake_case. */
export type LoopModelToolChoice = {
  kind: "forced_capability";
  capability_id: string;
};

/** `LoopModelResponse` returned by `StreamModel`. */
export interface LoopModelResponse {
  chunks: ModelStreamChunk[];
  safe_reasoning_deltas?: string[];
  output: ParentLoopOutput;
  effective_model_profile_id: string;
  usage?: LoopModelUsage | null;
}

export interface ModelStreamChunk {
  safe_text_delta: string;
}

/** `ParentLoopOutput` — externally tagged. */
export type ParentLoopOutput =
  | { assistant_reply: AssistantReply }
  | { capability_calls: CapabilityCallCandidate[] };

export interface AssistantReply {
  content: string;
}

export interface CapabilityCallCandidate {
  activity_id: string;
  surface_version: string;
  capability_id: string;
  input_ref: string;
  effective_capability_ids?: string[];
  provider_replay?: ProviderToolCallReplay | null;
}

export interface ProviderToolCallReplay {
  provider_id: string;
  provider_model_id: string;
  provider_turn_id: string;
  provider_call_id: string;
  provider_tool_name: string;
  arguments: unknown;
  response_reasoning?: string | null;
  reasoning?: string | null;
  signature?: string | null;
}

/** `LoopModelUsage` — `cache_*` fields default 0 and are skipped when 0. */
export interface LoopModelUsage {
  input_tokens: number;
  output_tokens: number;
  cache_read_input_tokens?: number;
  cache_creation_input_tokens?: number;
}

// --- capability invocation ---

export interface ProviderToolCall {
  provider_id: string;
  provider_model_id: string;
  turn_id?: string | null;
  id: string;
  name: string;
  arguments: unknown;
  response_reasoning?: string | null;
  reasoning?: string | null;
  signature?: string | null;
}

export interface RegisterProviderToolCallRequest {
  tool_call: ProviderToolCall;
  activity_id?: string | null;
}

/** `CapabilityCallCandidate` is the response (same shape as above). */

export interface LoopRequest {
  activity_id: string;
  surface_version: string;
  capability_id: string;
  input_ref: string;
  approval_resume?: unknown | null;
  auth_resume?: unknown | null;
}

/** `Resolution` from `ironclaw_host_api::resolution` — externally tagged. */
export type Resolution =
  | { done: Outcome }
  | { denied: { deny: string; reason_kind?: string; summary?: string } }
  | { blocked: Partial<Record<"approval" | "auth" | "resource", GateWaypoint>> }
  | {
      suspended:
        | { process: { process: string; origin?: string } }
        | { dependent_run: { waypoint: GateWaypoint; result: unknown } }
        | { external_tool: GateWaypoint };
    };

export interface GateWaypoint {
  gate: string;
  origin?: string;
  resume?: string;
}

export interface Outcome {
  refs: OutcomeRefs;
  verdict: unknown;
  summary: string;
  progress?: unknown;
  terminate_hint?: unknown;
}

export interface OutcomeRefs {
  result: string;
  byte_len: number;
  preview?: string | null;
  preview_meta?: unknown;
  origin?: string | null;
  output_digest?: unknown;
}

// --- assistant drafts (host/transcript.rs) ---

export interface BeginAssistantDraft {
  reply: AssistantReply;
}

export interface UpdateAssistantDraft {
  message_ref: string;
  reply: AssistantReply;
}

export interface FinalizeAssistantMessage {
  reply: AssistantReply;
}

export interface AppendCapabilityResultRef {
  result_ref: string;
  safe_summary: string;
  provider_call?: ProviderToolCallReference | null;
  model_observation?: unknown | null;
  intrinsic_outcome?: unknown | null;
}

export interface ProviderToolCallReference extends ProviderToolCallReplay {
  capability_id: string;
}

// --- checkpoints (host/checkpoint.rs) ---

export type LoopCheckpointKind =
  "before_model" | "before_side_effect" | "before_block" | "final";

export interface StageCheckpointPayloadRequest {
  kind: LoopCheckpointKind;
  schema_id: string;
  /** Canonical payload bytes — JSON array of byte numbers (serde `Vec<u8>`). */
  payload: number[];
}

// ---------------------------------------------------------------------------
// Outcomes (loop_exit.rs) — serde `rename_all = "snake_case"`
// ---------------------------------------------------------------------------

export type LoopExit =
  | { completed: LoopCompleted }
  | { blocked: LoopBlocked }
  | { cancelled: LoopCancelled }
  | { failed: LoopFailed };

export interface LoopCompleted {
  completion_kind: LoopCompletionKind;
  reply_message_refs: string[];
  result_refs: string[];
  final_checkpoint_id: string | null;
  model_usage?: LoopModelUsage | null;
  exit_id: string;
}

export type LoopCompletionKind =
  | "final_reply"
  | "ask_user_reply"
  | "no_reply"
  | "delegated_result"
  | "result_only"
  | "nothing_to_report";

export interface LoopBlocked {
  kind: LoopBlockedKind;
  gate_ref: string;
  blocked_activity_id?: string | null;
  credential_requirements?: unknown[];
  checkpoint_id: string;
  state_ref: string;
  exit_id: string;
}

export type LoopBlockedKind =
  "approval" | "auth" | "resource" | "await_dependent_run" | "external_tool";

export interface LoopCancelled {
  reason_kind: "host_cancellation" | "host_interrupt";
  checkpoint_id: string | null;
  interrupted_message_refs: string[];
  exit_id: string;
}

export interface LoopFailed {
  reason_kind: LoopFailureKind;
  checkpoint_id: string | null;
  model_usage?: LoopModelUsage | null;
  exit_id: string;
  explanation_message_refs?: string[];
  safe_summary?: unknown | null;
}

export type LoopFailureKind =
  | "model_error"
  | "context_build_failed"
  | "capability_protocol_error"
  | "iteration_limit"
  | "invalid_model_output"
  | "checkpoint_rejected"
  | "checkpoint_unavailable"
  | "transcript_write_failed"
  | "driver_bug"
  | "interrupted_unexpectedly"
  | "no_progress_detected"
  | "policy_denied"
  | "compaction_unavailable"
  | "gate_not_supported"
  | "wall_clock_limit"
  | "model_call_limit"
  | "capability_invocation_limit";

export interface LoopWorkerFailure {
  kind: string;
  detail?: string | null;
}

/** `LoopWorkerOutcome` — externally tagged. */
export type LoopWorkerOutcome =
  { Exit: LoopExit } | { Failed: LoopWorkerFailure };

// ---------------------------------------------------------------------------
// Frames — externally tagged, mirroring `HostFrame` / `WorkerFrame`
// ---------------------------------------------------------------------------

/** `LoopCancellationSignal` (`host/progress.rs`). */
export interface LoopCancellationSignal {
  reason_kind: string;
  detail?: unknown;
}

export type HostFrame =
  | { Bootstrap: LoopWorkerBootstrap }
  | {
      HostResponse: {
        id: number;
        result: { Ok: unknown } | { Err: WireError };
      };
    }
  | { Cancel: LoopCancellationSignal }
  | "OutcomeAck";

export type WorkerFrame =
  | { HostRequest: { id: number; call: HostCall } }
  | { Outcome: LoopWorkerOutcome };

/** `WireError` — externally tagged. */
export type WireError =
  { Host: AgentLoopHostError } | { Compaction: unknown } | { Protocol: string };

/** `AgentLoopHostError` (`host/error.rs`). */
export interface AgentLoopHostError {
  kind: string;
  safe_summary: string;
  reason_kind?: unknown | null;
  gate_ref?: string | null;
  retry_after_ms?: number | null;
  next_fallback_index?: number | null;
  usage?: LoopModelUsage | null;
  detail?: string | null;
}

/** Discriminator for an `Err` wire response, for error handling. */
export function wireErrorSummary(error: WireError): string {
  if ("Host" in error) {
    return `host ${error.Host.kind}: ${error.Host.safe_summary}`;
  }
  if ("Compaction" in error) {
    return `compaction: ${JSON.stringify(error.Compaction)}`;
  }
  return `protocol: ${error.Protocol}`;
}
