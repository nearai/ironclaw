import { cloneState } from "./state";
import type { RebornMockState, ScenarioName } from "./types";

const THREAD_ID = "thread-001";
const TIMELINE_MSG = {
  messageId: "msg-001",
  threadId: THREAD_ID,
  sequence: 1,
  kind: "Assistant" as const,
  content: "Hello! I'm your IronClaw assistant. How can I help you today?",
  actorId: "assistant",
  status: "completed",
  turnId: "turn-001",
  turnRunId: "run-001",
  createdAt: new Date().toISOString(),
};

function buildHealthyChat(threadId: string, baseState: RebornMockState): RebornMockState {
  const state = cloneState(baseState);
  state.threads = [
    {
      threadId,
      title: "Test Thread",
      createdAt: new Date().toISOString(),
      messages: [TIMELINE_MSG],
    },
  ];
  state.automations = [{ id: "auto-001", name: "Daily Summary", status: "active", isActive: true }];
  state.outboundPrefs = {
    finalReplyTarget: {
      targetId: "target-001",
      channel: "slack",
      displayName: "Engineering Slack",
    },
  };
  state.outboundTargets = [
    {
      target: { targetId: "target-001", channel: "slack", displayName: "Engineering Slack" },
      capabilities: { finalReplies: true, gatePrompts: false, authPrompts: false },
    },
  ];
  state.extensions = [
    {
      packageRef: { kind: "builtin", id: "nova-submit" },
      displayName: "Nova Submit",
      kind: "wasm",
      description: "Encrypt and upload files to NOVA",
      active: true,
    },
  ];
  state.extensionRegistry = [
    {
      packageRef: { kind: "builtin", id: "nova-submit" },
      displayName: "Nova Submit",
      kind: "wasm",
      description: "Encrypt and upload files to NOVA",
      installed: true,
    },
    {
      packageRef: { kind: "builtin", id: "my-channel" },
      displayName: "My Channel",
      kind: "wasm",
      description: "Custom channel extension",
      installed: false,
    },
  ];
  state.skills = [
    {
      name: "hackathon-guide",
      description: "Hackathon participation guide",
      version: "1.0.0",
      trust: "sandboxed",
      source: "local",
    },
  ];
  state.connectableChannels = [{ channel: "slack", displayName: "Slack", strategy: "oauth" }];
  state.authProviders = [{ id: "near", name: "NEAR", type: "wallet" }];
  return state;
}

export function applyScenario(baseState: RebornMockState, name: ScenarioName): RebornMockState {
  const state = cloneState(baseState);

  switch (name) {
    case "healthy-empty":
      return state;

    case "healthy-chat":
      return buildHealthyChat(THREAD_ID, state);

    case "bad-token":
      state.token = "invalid-token-will-not-match";
      return state;

    case "unreachable":
      return state;

    case "stream-final-reply": {
      const s = buildHealthyChat(THREAD_ID, state);
      s.sseHandler = async (write) => {
        write("accepted", {
          cursor: 1,
          type: "accepted",
          ack: {
            outcome: "submitted",
            thread_id: THREAD_ID,
            run_id: "run-001",
            accepted_message_ref: "msg-002",
            status: "running",
            turn_id: "turn-001",
            event_cursor: 1,
          },
        });
        write("running", {
          cursor: 2,
          type: "running",
          progress: {
            kind: "reasoning",
            turn_run_id: "run-001",
            generated_at: new Date().toISOString(),
          },
        });
        write("final_reply", {
          cursor: 3,
          type: "final_reply",
          reply: {
            text: "Here is my final answer! I've analyzed your request.",
            turn_run_id: "run-001",
            generated_at: new Date().toISOString(),
          },
        });
      };
      return s;
    }

    case "stream-gate": {
      const s = buildHealthyChat(THREAD_ID, state);
      s.sseHandler = async (write) => {
        write("accepted", {
          cursor: 1,
          type: "accepted",
          ack: {
            outcome: "submitted",
            thread_id: THREAD_ID,
            run_id: "run-002",
            accepted_message_ref: "msg-003",
            status: "running",
            turn_id: "turn-002",
            event_cursor: 1,
          },
        });
        write("gate", {
          cursor: 2,
          type: "gate",
          prompt: {
            turn_run_id: "run-002",
            gate_ref: "gate-001",
            headline: "Approve tool execution",
            body: "The agent wants to execute external-tool. Allow?",
            allow_always: true,
          },
        });
      };
      return s;
    }

    case "stream-failed": {
      const s = buildHealthyChat(THREAD_ID, state);
      s.sseHandler = async (write) => {
        write("accepted", {
          cursor: 1,
          type: "accepted",
          ack: {
            outcome: "submitted",
            thread_id: THREAD_ID,
            run_id: "run-003",
            accepted_message_ref: "msg-004",
            status: "running",
            turn_id: "turn-003",
            event_cursor: 1,
          },
        });
        write("failed", {
          cursor: 2,
          type: "failed",
          run_state: {
            turn_id: "turn-003",
            run_id: "run-003",
            status: "failed",
            event_cursor: 2,
            accepted_message_ref: "msg-004",
            resolved_run_profile_id: "default",
            resolved_run_profile_version: "1",
            received_at: new Date().toISOString(),
            failure: { kind: "tool_error", message: "External service returned 500" },
          },
        });
      };
      return s;
    }

    case "stream-cancelled": {
      const s = buildHealthyChat(THREAD_ID, state);
      s.sseHandler = async (write) => {
        write("accepted", {
          cursor: 1,
          type: "accepted",
          ack: {
            outcome: "submitted",
            thread_id: THREAD_ID,
            run_id: "run-004",
            accepted_message_ref: "msg-005",
            status: "running",
            turn_id: "turn-004",
            event_cursor: 1,
          },
        });
        write("cancelled", {
          cursor: 2,
          type: "cancelled",
          response: {
            run_id: "run-004",
            status: "cancelled",
            event_cursor: 2,
            already_terminal: false,
          },
        });
      };
      return s;
    }

    case "stream-auth-required": {
      const s = buildHealthyChat(THREAD_ID, state);
      s.sseHandler = async (write) => {
        write("accepted", {
          cursor: 1,
          type: "accepted",
          ack: {
            outcome: "submitted",
            thread_id: THREAD_ID,
            run_id: "run-006",
            accepted_message_ref: "msg-007",
            status: "running",
            turn_id: "turn-006",
            event_cursor: 1,
          },
        });
        write("auth_required", {
          cursor: 2,
          type: "auth_required",
          auth_prompt: {
            turn_run_id: "run-006",
            auth_request_ref: "auth-001",
            headline: "Authentication required",
            body: "The agent needs to authenticate with an external service.",
            provider: "oauth2",
            account_label: "example@test.com",
            authorization_url: "https://test.com/auth",
          },
        });
      };
      return s;
    }

    case "stream-projection": {
      const s = buildHealthyChat(THREAD_ID, state);
      s.sseHandler = async (write) => {
        write("accepted", {
          cursor: 1,
          type: "accepted",
          ack: {
            outcome: "submitted",
            thread_id: THREAD_ID,
            run_id: "run-007",
            accepted_message_ref: "msg-008",
            status: "running",
            turn_id: "turn-007",
            event_cursor: 1,
          },
        });
        write("running", {
          cursor: 2,
          type: "running",
          progress: {
            kind: "reasoning",
            turn_run_id: "run-007",
            generated_at: new Date().toISOString(),
          },
        });
        write("projection_snapshot", {
          cursor: 3,
          type: "projection_snapshot",
          state: {
            thread_id: THREAD_ID,
            items: [{ id: "item-1", label: "Research phase", status: "in_progress" }],
          },
        });
        write("projection_update", {
          cursor: 4,
          type: "projection_update",
          state: {
            thread_id: THREAD_ID,
            items: [
              { id: "item-1", label: "Research phase", status: "completed" },
              { id: "item-2", label: "Implementation", status: "in_progress" },
            ],
          },
        });
        write("final_reply", {
          cursor: 5,
          type: "final_reply",
          reply: {
            text: "Projection-complete reply.",
            turn_run_id: "run-007",
            generated_at: new Date().toISOString(),
          },
        });
      };
      return s;
    }

    case "stream-capability": {
      const s = buildHealthyChat(THREAD_ID, state);
      s.sseHandler = async (write) => {
        write("accepted", {
          cursor: 1,
          type: "accepted",
          ack: {
            outcome: "submitted",
            thread_id: THREAD_ID,
            run_id: "run-008",
            accepted_message_ref: "msg-009",
            status: "running",
            turn_id: "turn-008",
            event_cursor: 1,
          },
        });
        write("running", {
          cursor: 2,
          type: "running",
          progress: {
            kind: "reasoning",
            turn_run_id: "run-008",
            generated_at: new Date().toISOString(),
          },
        });
        write("capability_activity", {
          cursor: 3,
          type: "capability_activity",
          activity: {
            invocation_id: "inv-001",
            turn_run_id: "run-008",
            capability_id: "search-web",
            status: "running",
            provider: "web",
            updated_at: new Date().toISOString(),
          },
        });
        write("capability_display_preview", {
          cursor: 4,
          type: "capability_display_preview",
          preview: {
            invocation_id: "inv-001",
            turn_run_id: "run-008",
            capability_id: "search-web",
            status: "completed",
            title: "Search results for query",
            subtitle: "3 results found",
            output_summary: "Found relevant information",
            output_kind: "markdown",
            output_bytes: 1024,
            truncated: false,
            updated_at: new Date().toISOString(),
          },
        });
        write("final_reply", {
          cursor: 5,
          type: "final_reply",
          reply: {
            text: "Capability-complete reply.",
            turn_run_id: "run-008",
            generated_at: new Date().toISOString(),
          },
        });
      };
      return s;
    }

    case "stream-drop-once": {
      const s = buildHealthyChat(THREAD_ID, state);
      s.dropCount = 0;
      s.sseHandler = async (write) => {
        write("accepted", {
          cursor: 1,
          type: "accepted",
          ack: {
            outcome: "submitted",
            thread_id: THREAD_ID,
            run_id: "run-005",
            accepted_message_ref: "msg-006",
            status: "running",
            turn_id: "turn-005",
            event_cursor: 1,
          },
        });
        write("running", {
          cursor: 2,
          type: "running",
          progress: {
            kind: "reasoning",
            turn_run_id: "run-005",
            generated_at: new Date().toISOString(),
          },
        });
        write("final_reply", {
          cursor: 3,
          type: "final_reply",
          reply: {
            text: "Final answer after reconnection.",
            turn_run_id: "run-005",
            generated_at: new Date().toISOString(),
          },
        });
      };
      return s;
    }

    default:
      return state;
  }
}
