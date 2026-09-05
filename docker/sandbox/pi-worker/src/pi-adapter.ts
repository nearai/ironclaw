import {
  Agent,
  type AgentMessage,
  type AgentTool,
} from "@earendil-works/pi-agent-core";
import {
  createAssistantMessageEventStream,
  type AssistantMessage,
  type Model,
} from "@earendil-works/pi-ai";
import { HostRpc } from "./host";
import {
  LOOP_WORKER_WIRE_VERSION,
  type CapabilityCallCandidate,
  type GateWaypoint,
  type LoopBlocked,
  type LoopModelResponse,
  type LoopModelUsage,
  type LoopPromptBundle,
  type LoopWorkerBootstrap,
  type LoopWorkerOutcome,
  type Resolution,
  type WireResolvedModelMessage,
} from "./protocol";

const MODEL: Model<string> = {
  id: "ironclaw-host-gateway",
  name: "IronClaw host gateway",
  api: "openai-completions",
  provider: "ironclaw",
  baseUrl: "",
  reasoning: false,
  input: ["text"],
  cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 },
  contextWindow: 200_000,
  maxTokens: 8192,
};
const EMPTY_USAGE: AssistantMessage["usage"] = {
  input: 0,
  output: 0,
  cacheRead: 0,
  cacheWrite: 0,
  totalTokens: 0,
  cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 },
};
type CheckpointKind =
  "before_model" | "before_side_effect" | "before_block" | "final";
interface PendingCall {
  candidate: CapabilityCallCandidate;
  gate?: { kind: LoopBlocked["kind"]; waypoint: GateWaypoint };
  approval?: {
    approval_request_id: string;
    resume_token: string;
    correlation_id: string;
    input_ref: string;
  };
}
interface Session {
  format: "ironclaw-pi-session";
  version: 1;
  run_id: string;
  iteration: number;
  messages: AgentMessage[];
  pending: PendingCall[];
  usage: LoopModelUsage | null;
  result_refs: string[];
  compacted_through: number | null;
}

/** Pi owns model/tool turns. Every model call still uses the host's exact prompt grant. */
export async function runPiWorker(
  bootstrap: LoopWorkerBootstrap,
  host: HostRpc,
): Promise<LoopWorkerOutcome> {
  if (bootstrap.wire_version !== LOOP_WORKER_WIRE_VERSION) {
    return failure(
      "invalid_wire_version",
      `unsupported wire version ${bootstrap.wire_version}`,
    );
  }
  if (bootstrap.content_visibility !== "resolved") {
    return failure(
      "content_visibility_required",
      "Pi requires resolved-content authorization",
    );
  }
  const context = bootstrap.run_context;
  let state: Session = {
    format: "ironclaw-pi-session",
    version: 1,
    run_id: context.run_id,
    iteration: 0,
    messages: [],
    pending: [],
    usage: null,
    result_refs: [],
    compacted_through: null,
  };
  let checkpointId: string | null = null;
  let stateRef: string | null = null;
  let parked: LoopBlocked | null = null;
  let fatal: unknown;
  let terminate = false;
  let systemPrompt = "";
  let bundle: LoopPromptBundle;
  let agent: Agent;
  const exitId = () => `exit:${context.run_id}-${crypto.randomUUID()}`;
  const cancelled = (): LoopWorkerOutcome => ({
    Exit: {
      cancelled: {
        reason_kind: "host_cancellation",
        checkpoint_id: checkpointId,
        interrupted_message_refs: [],
        exit_id: exitId(),
      },
    },
  });

  async function checkpoint(
    kind: CheckpointKind,
    gateRef: string | null = null,
  ): Promise<void> {
    if (agent) state.messages = agent.state.messages;
    const payload = Array.from(new TextEncoder().encode(JSON.stringify(state)));
    const staged = await host.call<string>({
      StageCheckpointPayload: {
        kind,
        schema_id: context.checkpoint_schema_id,
        payload,
      },
    });
    const committed = await host.call<string>({
      Checkpoint: { kind, state_ref: staged, gate_ref: gateRef },
    });
    stateRef = staged;
    checkpointId = committed;
  }

  async function buildPrompt(): Promise<AgentMessage[]> {
    bundle = await host.call<LoopPromptBundle>({
      BuildPrompt: {
        mode: "text_only",
        context_cursor: null,
        surface_version: null,
        checkpoint_state_ref: null,
        max_messages: null,
        inline_messages: [],
      },
    });
    const omitted = bundle.recent_window_truncation as {
      omitted_through_sequence?: number;
    } | null;
    const boundary = omitted?.omitted_through_sequence;
    if (boundary && boundary > (state.compacted_through ?? 0)) {
      const outcome = await host.call<{
        compacted?: unknown;
        deferred?: unknown;
      }>({
        Compact: {
          task_id: `pi-compaction:${crypto.randomUUID()}`,
          thread_id: context.thread_id,
          last_compacted_through_seq: state.compacted_through,
          drop_through_seq: boundary,
          preserve_tail_tokens: 8000,
          mode: "window_eviction",
          deadline_ms: 30_000,
        },
      });
      if (outcome.compacted !== undefined) {
        state.compacted_through = boundary;
        bundle = await host.call<LoopPromptBundle>({
          BuildPrompt: {
            mode: "text_only",
            context_cursor: null,
            surface_version: null,
            checkpoint_state_ref: null,
            max_messages: null,
            inline_messages: [],
          },
        });
      }
    }
    const resolved =
      bundle.messages.length === 0
        ? []
        : await host.call<WireResolvedModelMessage[]>({
            ResolveMessages: { messages: bundle.messages },
          });
    const system = resolved
      .filter((message) => message.role === "system")
      .map((message) => message.content)
      .join("\n\n");
    systemPrompt = system;
    if (agent) agent.state.systemPrompt = system;
    return resolved
      .filter((message) => message.role !== "system")
      .map(toPiMessage);
  }

  async function appendResult(
    call: CapabilityCallCandidate,
    resultRef: string,
    summary: string,
  ): Promise<string> {
    const messageRef = await host.call<string>({
      AppendCapabilityResultRef: {
        result_ref: resultRef,
        safe_summary: summary,
        provider_call: call.provider_replay
          ? { ...call.provider_replay, capability_id: call.capability_id }
          : undefined,
      },
    });
    const resolved = await host.call<WireResolvedModelMessage[]>({
      ResolveMessages: {
        messages: [{ role: "tool_result_reference", content_ref: messageRef }],
      },
    });
    if (resolved.length !== 1 || resolved[0].content_ref !== messageRef)
      throw new Error("host returned a mismatched tool result");
    return resolved[0].tool_result?.content ?? resolved[0].content;
  }

  async function execute(
    pending: PendingCall,
    disposition?: "denied" | null,
  ): Promise<string> {
    const call = pending.candidate;
    await checkpoint("before_side_effect");
    const gate = pending.gate;
    if (gate?.kind === "approval" && disposition === "denied") {
      const text = await appendResult(
        call,
        `result:provider-error-${call.activity_id}`,
        "The user denied this action. Do not repeat this call.",
      );
      state.pending = state.pending.filter((item) => item !== pending);
      return text;
    }
    if (gate?.kind === "approval") {
      const requestId = gate.waypoint.origin?.replace(/^gate:approval-/, "");
      if (
        !requestId ||
        requestId === gate.waypoint.origin ||
        !gate.waypoint.resume
      )
        throw new Error("approval gate has no replay identity");
      pending.approval = {
        approval_request_id: requestId,
        resume_token: gate.waypoint.resume,
        correlation_id: crypto.randomUUID(),
        input_ref: call.input_ref,
      };
    }
    const authResume =
      gate?.kind === "auth"
        ? {
            gate_ref: gate.waypoint.origin,
            resume_token:
              disposition === "denied" ? undefined : gate.waypoint.resume,
            disposition: disposition === "denied" ? "denied" : undefined,
            prior_approval: pending.approval
              ? {
                  approval_request_id: pending.approval.approval_request_id,
                  correlation_id: pending.approval.correlation_id,
                }
              : undefined,
          }
        : undefined;
    const resolution = await host.call<Resolution>({
      InvokeCapability: {
        activity_id: call.activity_id,
        surface_version: call.surface_version,
        capability_id: call.capability_id,
        input_ref: call.input_ref,
        approval_resume: gate?.kind === "auth" ? undefined : pending.approval,
        auth_resume: authResume,
      },
    });
    if ("blocked" in resolution || "suspended" in resolution) {
      let kind: LoopBlocked["kind"];
      let waypoint: GateWaypoint;
      if ("blocked" in resolution) {
        const entry = Object.entries(resolution.blocked)[0];
        if (!entry) throw new Error("host returned an empty gate");
        if (
          entry[0] !== "approval" &&
          entry[0] !== "auth" &&
          entry[0] !== "resource"
        )
          throw new Error("host returned an unknown gate kind");
        kind = entry[0];
        waypoint = entry[1];
      } else if ("dependent_run" in resolution.suspended) {
        kind = "await_dependent_run";
        waypoint = resolution.suspended.dependent_run.waypoint;
      } else if ("external_tool" in resolution.suspended) {
        kind = "external_tool";
        waypoint = resolution.suspended.external_tool;
      } else {
        throw new Error("process waits are not supported by this loop driver");
      }
      if (!waypoint.origin)
        throw new Error("host gate has no originating loop reference");
      pending.gate = { kind, waypoint };
      await checkpoint("before_block", waypoint.origin);
      if (!checkpointId || !stateRef)
        throw new Error("host did not commit the blocking checkpoint");
      parked = {
        kind,
        gate_ref: waypoint.origin,
        blocked_activity_id: call.activity_id,
        credential_requirements: [],
        checkpoint_id: checkpointId,
        state_ref: stateRef,
        exit_id: exitId(),
      };
      return "Execution is waiting for the host gate.";
    }
    let text: string;
    if ("denied" in resolution) {
      text = await appendResult(
        call,
        `result:provider-error-${call.activity_id}`,
        resolution.denied.summary ??
          resolution.denied.reason_kind ??
          "The host denied this action.",
      );
    } else {
      const result = resolution.done;
      if (!result.refs.origin)
        throw new Error(
          "host outcome has no originating loop result reference",
        );
      text = await appendResult(call, result.refs.origin, result.summary);
      if (!state.result_refs.includes(result.refs.origin))
        state.result_refs.push(result.refs.origin);
      terminate ||= result.terminate_hint === "terminate_after_batch";
    }
    state.pending = state.pending.filter((item) => item !== pending);
    await checkpoint("before_model");
    return text;
  }

  const tools: AgentTool[] = bootstrap.tool_definitions.map((definition) => ({
    name: definition.name,
    label: definition.capability_id,
    description: definition.description,
    parameters: definition.parameters as AgentTool["parameters"],
    execute: async (toolCallId) => {
      if (parked || fatal || host.isCancelled)
        return toolResult("Execution stopped before this call.");
      const pending = state.pending.find(
        ({ candidate }) =>
          (candidate.provider_replay?.provider_call_id ??
            candidate.activity_id) === toolCallId,
      );
      try {
        if (
          !pending ||
          pending.candidate.capability_id !== definition.capability_id
        )
          throw new Error(
            "Pi tool call does not match a host-issued candidate",
          );
        return toolResult(await execute(pending));
      } catch (error) {
        // Pi turns tool exceptions into model observations. Host failures must instead stop the run.
        fatal = error;
        return toolResult("The host could not complete the call.");
      }
    },
  }));

  try {
    if ("Resume" in bootstrap.invocation) {
      const resume = bootstrap.invocation.Resume;
      const loaded = await host.call<{
        schema_id: string;
        schema_version: number;
        payload: number[];
      }>({
        LoadCheckpointPayload: {
          checkpoint_id: resume.checkpoint_id,
          expected_schema_id: context.checkpoint_schema_id,
          expected_schema_version: context.checkpoint_schema_version,
        },
      });
      if (
        loaded.schema_id !== context.checkpoint_schema_id ||
        loaded.schema_version !== context.checkpoint_schema_version
      )
        throw new Error("checkpoint schema mismatch");
      const saved: Session = JSON.parse(
        new TextDecoder().decode(new Uint8Array(loaded.payload)),
      );
      if (
        saved.format !== "ironclaw-pi-session" ||
        saved.version !== 1 ||
        saved.run_id !== context.run_id ||
        !Array.isArray(saved.pending) ||
        !Array.isArray(saved.messages)
      )
        throw new Error("invalid Pi checkpoint payload");
      state = saved;
      checkpointId = resume.checkpoint_id;
    }
    const initial = await buildPrompt();
    let firstContext = true;
    agent = new Agent({
      initialState: { model: MODEL, tools, messages: initial, systemPrompt },
      toolExecution: "sequential",
      transformContext: async () => {
        if (firstContext) {
          firstContext = false;
          return agent.state.messages;
        }
        return buildPrompt();
      },
      shouldStopAfterTurn: () =>
        parked !== null || fatal !== undefined || terminate,
      streamFn: () => {
        const stream = createAssistantMessageEventStream();
        void (async () => {
          try {
            if (host.isCancelled) throw new Error("host cancelled the run");
            const limit = bootstrap.settings.default_iteration_limit ?? 100;
            if (state.iteration >= limit)
              throw new Error("iteration limit exceeded");
            await checkpoint("before_model");
            const response = await host.call<LoopModelResponse>({
              StreamModel: {
                messages: bundle.messages,
                inline_messages: [],
                surface_version: bundle.surface_version,
                model_preference: null,
                fallback_index: 0,
                iteration: state.iteration++,
              },
            });
            addUsage(state, response.usage);
            const message = assistantResponse(response, bootstrap);
            if ("capability_calls" in response.output) {
              state.pending = response.output.capability_calls.map(
                (candidate) => ({ candidate }),
              );
            }
            stream.push({
              type: "start",
              partial: { ...message, content: [] },
            });
            const reason =
              message.stopReason === "toolUse" ? "toolUse" : "stop";
            stream.push({ type: "done", reason, message });
            stream.end(message);
          } catch (error) {
            fatal = error;
            const message: AssistantMessage = {
              ...assistant(""),
              stopReason: host.isCancelled ? "aborted" : "error",
              errorMessage: String(error),
            };
            stream.push({
              type: "error",
              reason: host.isCancelled ? "aborted" : "error",
              error: message,
            });
            stream.end(message);
          }
        })();
        return stream;
      },
    });
    const abort = () => agent.abort();
    host.signal.addEventListener("abort", abort);
    try {
      if ("Resume" in bootstrap.invocation) {
        for (const pending of [...state.pending]) {
          if (host.isCancelled || parked) break;
          await execute(
            pending,
            bootstrap.invocation.Resume.auth_resume_disposition,
          );
        }
        agent.state.messages = await buildPrompt();
      }
      if (!parked && !terminate && !host.isCancelled) {
        await agent.continue();
      }
    } finally {
      host.signal.removeEventListener("abort", abort);
    }
    if (host.isCancelled) return cancelled();
    if (fatal) throw fatal;
    if (parked) return { Exit: { blocked: parked } };
    const last = agent.state.messages.findLast(
      (message) => message.role === "assistant",
    ) as AssistantMessage | undefined;
    if (terminate) {
      await checkpoint("final");
      return {
        Exit: {
          completed: {
            completion_kind: "result_only",
            reply_message_refs: [],
            result_refs: state.result_refs,
            final_checkpoint_id: checkpointId,
            model_usage: state.usage,
            exit_id: exitId(),
          },
        },
      };
    }
    if (!last || last.stopReason !== "stop")
      throw new Error(
        last?.errorMessage ?? "Pi ended without a final assistant reply",
      );
    const text = last.content
      .filter((part) => part.type === "text")
      .map((part) => part.text)
      .join("");
    const reply = await host.call<string>({
      FinalizeAssistantMessage: { reply: { content: text } },
    });
    await checkpoint("final");
    return {
      Exit: {
        completed: {
          completion_kind: "final_reply",
          reply_message_refs: [reply],
          result_refs: state.result_refs,
          final_checkpoint_id: checkpointId,
          model_usage: state.usage,
          exit_id: exitId(),
        },
      },
    };
  } catch (error) {
    if (host.isCancelled) return cancelled();
    return failure(
      "pi_worker_failed",
      error instanceof Error ? error.message : String(error),
    );
  }
}

function failure(kind: string, detail: string): LoopWorkerOutcome {
  return { Failed: { kind, detail } };
}
function toolResult(text: string) {
  return { content: [{ type: "text" as const, text }], details: {} };
}
function assistant(text: string): AssistantMessage {
  return {
    role: "assistant",
    content: [{ type: "text", text }],
    api: MODEL.api,
    provider: MODEL.provider,
    model: MODEL.id,
    usage: EMPTY_USAGE,
    stopReason: "stop",
    timestamp: Date.now(),
  };
}
function assistantResponse(
  response: LoopModelResponse,
  bootstrap: LoopWorkerBootstrap,
): AssistantMessage {
  if ("assistant_reply" in response.output)
    return assistant(response.output.assistant_reply.content);
  const content = response.output.capability_calls.map((call) => {
    const definition = bootstrap.tool_definitions.find(
      (tool) => tool.capability_id === call.capability_id,
    );
    if (!definition)
      throw new Error(
        `host returned an unadvertised capability: ${call.capability_id}`,
      );
    return {
      type: "toolCall" as const,
      id: call.provider_replay?.provider_call_id ?? call.activity_id,
      name: definition.name,
      arguments: (call.provider_replay?.arguments ?? {}) as Record<
        string,
        unknown
      >,
    };
  });
  if (content.length === 0)
    throw new Error("host returned an empty capability batch");
  return { ...assistant(""), content, stopReason: "toolUse" };
}
function toPiMessage(message: WireResolvedModelMessage): AgentMessage {
  if (message.role === "assistant") return assistant(message.content);
  if (message.role === "tool_result_reference")
    return {
      role: "toolResult",
      toolCallId: message.tool_result?.provider_call_id ?? message.content_ref,
      toolName: "host",
      content: [
        { type: "text", text: message.tool_result?.content ?? message.content },
      ],
      isError: false,
      timestamp: Date.now(),
    };
  return { role: "user", content: message.content, timestamp: Date.now() };
}
function addUsage(
  state: Session,
  usage: LoopModelUsage | null | undefined,
): void {
  if (!usage) return;
  state.usage ??= { input_tokens: 0, output_tokens: 0 };
  for (const key of [
    "input_tokens",
    "output_tokens",
    "cache_read_input_tokens",
    "cache_creation_input_tokens",
  ] as const) {
    state.usage[key] = Math.min(
      0xffffffff,
      (state.usage[key] ?? 0) + (usage[key] ?? 0),
    );
  }
}
