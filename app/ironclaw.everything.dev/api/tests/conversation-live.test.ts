import { describe, expect, it, vi } from "vitest";
import { createConversationLiveHandler } from "../src/lib/conversation-live";

function event(type: string, overrides: Record<string, unknown> = {}) {
  return { type, ...overrides };
}

function mockIc(events: any[], timelineData?: any[]) {
  const ic = {
    threads: {
      streamEvents: vi.fn().mockResolvedValue(
        (async function* () {
          for (const e of events) yield e;
        })(),
      ),
      getTimeline: vi.fn().mockResolvedValue(
        timelineData ? { data: timelineData } : { data: [] },
      ),
    },
  };

  return { ic };
}

async function collectEvents(
  handler: ReturnType<typeof createConversationLiveHandler>,
  input: { threadId: string; runId?: string; afterCursor?: string },
) {
  const gen = handler({ input, signal: new AbortController().signal, context: {} });
  const events: any[] = [];
  for await (const e of gen) events.push(e);
  return events;
}

describe("createConversationLiveHandler", () => {
  it("maps capability events into AG-UI chunks and custom events", async () => {
    const { ic } = mockIc([
      event("accepted", {
        ack: { runId: "run-1", activeRunId: "run-1", threadId: "thread-1" },
      }),
      event("capability_display_preview", {
        preview: {
          timelineMessageId: "msg-1",
          capabilityId: "search-web",
          title: "Search web",
          inputSummary: "ironclaw",
          outputSummary: "found it",
          outputKind: "text",
          truncated: false,
        },
      }),
      event("capability_activity", {
        activity: {
          timelineMessageId: "msg-1",
          capabilityId: "search-web",
          status: "completed",
          errorKind: undefined,
        },
      }),
      event("final_reply", {
        reply: { text: "final answer", turnRunId: "run-1" },
      }),
    ]);

    const handler = createConversationLiveHandler({ ironclaw: () => ic as any });
    const events = await collectEvents(handler, { threadId: "thread-1", runId: "run-1" });

    expect(events.map((e) => e.type)).toEqual([
      "RUN_STARTED",
      "CUSTOM",
      "TOOL_CALL_START",
      "TOOL_CALL_ARGS",
      "CUSTOM",
      "TOOL_CALL_END",
      "CUSTOM",
      "CUSTOM",
      "TEXT_MESSAGE_START",
      "TEXT_MESSAGE_CONTENT",
      "TEXT_MESSAGE_END",
      "RUN_FINISHED",
    ]);

    expect(events[1]!.name).toBe("ironclaw.accepted");
    expect(events[2]!.toolCallId).toBe("msg-1");
    expect(events[2]!.parentMessageId).toBe("assistant:run-1");
    expect(events[5]!.result).toContain("found it");
    expect(events[7]!.name).toBe("ironclaw.final-reply");
    // Text chunks use the same assistant message id
    expect(events[8]!.type).toBe("TEXT_MESSAGE_START");
    expect(events[8]!.messageId).toBe("assistant:run-1");
    expect(events[9]!.delta).toBe("final answer");
    expect(events[10]!.type).toBe("TEXT_MESSAGE_END");
  });

  it("emits approval requests for gates", async () => {
    const { ic } = mockIc([
      event("accepted", {
        ack: { runId: "run-2", activeRunId: "run-2", threadId: "thread-1" },
      }),
      event("gate", {
        prompt: {
          turnRunId: "run-2",
          gateRef: "gate-1",
          headline: "Need approval",
          body: "Approve the tool call?",
          approvalContext: { toolName: "shell", action: "run", scope: "thread" },
        },
      }),
    ]);

    const handler = createConversationLiveHandler({ ironclaw: () => ic as any });
    const events = await collectEvents(handler, { threadId: "thread-1", runId: "run-2" });

    expect(events.map((e) => e.type)).toContain("TOOL_CALL_START");
    expect(events.map((e) => e.type)).toContain("TOOL_CALL_END");
    expect(events.find((e) => e.type === "CUSTOM" && e.name === "approval-requested")).toBeDefined();
    expect(events.find((e) => e.type === "CUSTOM" && e.name === "ironclaw.gate")).toBeDefined();
  });

  it("processes projection run_status with completed status and reconciles from timeline", async () => {
    const { ic } = mockIc(
      [
        event("projection_snapshot", {
          state: {
            items: [
              { runStatus: { runId: "run-proj-1", status: "completed" } },
            ],
          },
        }),
      ],
      [
        { messageId: "msg-proj-1", kind: "Assistant", content: "projection completed reply", turn_run_id: "run-proj-1" },
      ],
    );

    const handler = createConversationLiveHandler({ ironclaw: () => ic as any });
    const events = await collectEvents(handler, { threadId: "thread-1" });

    expect(events[0]!.type).toBe("RUN_STARTED");
    expect(events[0]!.runId).toBe("run-proj-1");
    expect(events[1]!.type).toBe("CUSTOM");
    expect(events[1]!.name).toBe("ironclaw.running");
    // Should reconcile from timeline
    const textContent = events.find((e) => e.type === "TEXT_MESSAGE_CONTENT");
    expect(textContent).toBeDefined();
    expect(textContent!.delta).toBe("projection completed reply");
    // Should end with RUN_FINISHED
    expect(events[events.length - 1]!.type).toBe("RUN_FINISHED");
    expect(ic.threads.getTimeline).toHaveBeenCalled();
  });

  it("processes projection text items as final reply text", async () => {
    const { ic } = mockIc([
      event("accepted", {
        ack: { runId: "run-proj-2", activeRunId: "run-proj-2", threadId: "thread-1" },
      }),
      event("projection_update", {
        state: {
          items: [
            { text: { id: "txt-1", body: "projection says hello" } },
          ],
        },
      }),
      event("projection_snapshot", {
        state: {
          items: [
            { runStatus: { runId: "run-proj-2", status: "completed" } },
            { text: { id: "txt-1", body: "projection says hello" } },
          ],
        },
      }),
    ]);

    const handler = createConversationLiveHandler({ ironclaw: () => ic as any });
    const events = await collectEvents(handler, { threadId: "thread-1", runId: "run-proj-2" });

    // Text from projection should appear
    const textParts = events.filter((e) => e.type === "TEXT_MESSAGE_CONTENT");
    expect(textParts.length).toBeGreaterThanOrEqual(1);
    expect(textParts[0]!.delta).toBe("projection says hello");
    // No projection-snapshot/update CUSTOM events
    const customNames = events.filter((e) => e.type === "CUSTOM").map((e) => e.name);
    expect(customNames).not.toContain("ironclaw.projection");
  });

  it("processes projection capability_activity items as tool calls", async () => {
    const { ic } = mockIc([
      event("accepted", {
        ack: { runId: "run-proj-3", activeRunId: "run-proj-3", threadId: "thread-1" },
      }),
      event("projection_snapshot", {
        state: {
          items: [
            {
              capabilityActivity: {
                invocationId: "inv-proj-1",
                capabilityId: "search-web",
                status: "running",
              },
            },
            {
              capabilityActivity: {
                invocationId: "inv-proj-1",
                capabilityId: "search-web",
                status: "completed",
              },
            },
            { runStatus: { runId: "run-proj-3", status: "completed" } },
          ],
        },
      }),
    ]);

    const handler = createConversationLiveHandler({ ironclaw: () => ic as any });
    const events = await collectEvents(handler, { threadId: "thread-1", runId: "run-proj-3" });

    expect(events.some((e) => e.type === "TOOL_CALL_START")).toBe(true);
    expect(events.some((e) => e.type === "TOOL_CALL_END")).toBe(true);
    expect(events.some((e) => e.type === "CUSTOM" && e.name === "ironclaw.capability-activity")).toBe(true);
  });

  it("reconciles final text from timeline when no final_reply has text", async () => {
    const { ic } = mockIc(
      [
        event("accepted", {
          ack: { outcome: "submitted", runId: "run-6", activeRunId: "run-6", threadId: "thread-1", acceptedMessageRef: "msg-1", status: "running" },
        }),
        event("capability_display_preview", {
          preview: {
            timelineMessageId: "msg-2",
            capabilityId: "search-web",
            title: "Search",
            inputSummary: "query",
            outputSummary: "result",
          },
        }),
        event("capability_activity", {
          activity: { timelineMessageId: "msg-2", capabilityId: "search-web", status: "completed" },
        }),
        // No final_reply — stream will end naturally
      ],
      [
        { messageId: "msg-3", kind: "Assistant", content: "reconciled reply text", turn_run_id: "run-6" },
      ],
    );

    const handler = createConversationLiveHandler({ ironclaw: () => ic as any });
    const events = await collectEvents(handler, { threadId: "thread-1", runId: "run-6" });

    // Should have emitted TEXT_MESSAGE_* via reconcile
    const textContent = events.find((e) => e.type === "TEXT_MESSAGE_CONTENT");
    expect(textContent).toBeDefined();
    expect(textContent!.delta).toBe("reconciled reply text");
    expect(ic.threads.getTimeline).toHaveBeenCalled();
  });

  it("emits tool activity before final reply text in the same run", async () => {
    const { ic } = mockIc([
      event("accepted", {
        ack: { outcome: "submitted", runId: "run-7", activeRunId: "run-7", threadId: "thread-1", acceptedMessageRef: "msg-1", status: "running" },
      }),
      event("capability_display_preview", {
        preview: {
          timelineMessageId: "msg-4",
          capabilityId: "search-web",
          title: "Search web",
          inputSummary: "test",
          outputSummary: "results",
        },
      }),
      event("capability_activity", {
        activity: { timelineMessageId: "msg-4", capabilityId: "search-web", status: "completed" },
      }),
      event("final_reply", {
        reply: { text: "Here is what I found", turnRunId: "run-7" },
      }),
    ]);

    const handler = createConversationLiveHandler({ ironclaw: () => ic as any });
    const events = await collectEvents(handler, { threadId: "thread-1", runId: "run-7" });

    // Find the last TOOL_CALL_END and first TEXT_MESSAGE_START positions
    const toolEndIdx = events.findLastIndex((e) => e.type === "TOOL_CALL_END");
    const textStartIdx = events.findIndex((e) => e.type === "TEXT_MESSAGE_START");

    expect(toolEndIdx).toBeGreaterThanOrEqual(0);
    expect(textStartIdx).toBeGreaterThanOrEqual(0);
    expect(textStartIdx).toBeGreaterThan(toolEndIdx);
  });

  it("emits text before RUN_FINISHED when projection frame has runStatus(completed) and text item", async () => {
    const { ic } = mockIc([
      event("projection_snapshot", {
        state: {
          items: [
            { runStatus: { runId: "run-ord-1", status: "completed" } },
            { text: { id: "txt-ord-1", body: "ordered text reply" } },
          ],
        },
      }),
    ]);

    const handler = createConversationLiveHandler({ ironclaw: () => ic as any });
    const events = await collectEvents(handler, { threadId: "thread-1" });

    // Text must appear before RUN_FINISHED
    const textIdx = events.findIndex((e) => e.type === "TEXT_MESSAGE_CONTENT");
    const finishIdx = events.findIndex((e) => e.type === "RUN_FINISHED");
    expect(textIdx).toBeGreaterThanOrEqual(0);
    expect(finishIdx).toBeGreaterThan(textIdx);
    expect(events[textIdx].delta).toBe("ordered text reply");
  });

  it("reconciles assistant text for the correct runId from timeline", async () => {
    const { ic } = mockIc(
      [
        event("projection_snapshot", {
          state: {
            items: [
              { runStatus: { runId: "run-reconcile-1", status: "completed" } },
            ],
          },
        }),
      ],
      [
        { messageId: "user-runA", kind: "user", content: "user msg for run A", turn_run_id: "run-reconcile-a" },
        { messageId: "assist-runB", kind: "Assistant", content: "reply for run B", turn_run_id: "run-reconcile-b" },
        { messageId: "assist-runA", kind: "Assistant", content: "reply for run A", turn_run_id: "run-reconcile-1" },
      ],
    );

    const handler = createConversationLiveHandler({ ironclaw: () => ic as any });
    const events = await collectEvents(handler, { threadId: "thread-1" });

    // Should pick text from the entry matching run-reconcile-1, not run-reconcile-b
    const textContent = events.find((e) => e.type === "TEXT_MESSAGE_CONTENT");
    expect(textContent).toBeDefined();
    expect(textContent.delta).toBe("reply for run A");
  });

  it("emits tool events from projection capability activity before terminal status", async () => {
    const { ic } = mockIc([
      event("projection_snapshot", {
        state: {
          items: [
            {
              capabilityActivity: {
                invocationId: "inv-cap-only-1",
                capabilityId: "search-web",
                status: "running",
              },
            },
            {
              capabilityActivity: {
                invocationId: "inv-cap-only-1",
                capabilityId: "search-web",
                status: "completed",
              },
            },
            { runStatus: { runId: "run-cap-only-1", status: "completed" } },
          ],
        },
      }),
    ]);

    const handler = createConversationLiveHandler({ ironclaw: () => ic as any });
    const events = await collectEvents(handler, { threadId: "thread-1" });

    expect(events.some((e) => e.type === "TOOL_CALL_START")).toBe(true);
    expect(events.some((e) => e.type === "TOOL_CALL_END")).toBe(true);
    expect(events[events.length - 1].type).toBe("RUN_FINISHED");
  });

  it("skips events for other runs", async () => {
    const { ic } = mockIc([
      event("accepted", {
        ack: { runId: "run-other", activeRunId: "run-other", threadId: "thread-1" },
      }),
      event("accepted", {
        ack: { runId: "run-3", activeRunId: "run-3", threadId: "thread-1" },
      }),
      event("final_reply", {
        reply: { text: "ok", turnRunId: "run-3" },
      }),
    ]);

    const handler = createConversationLiveHandler({ ironclaw: () => ic as any });
    const events = await collectEvents(handler, { threadId: "thread-1", runId: "run-3" });

    expect(events[0]!.type).toBe("RUN_STARTED");
    expect(events.some((e) => e.runId === "run-other")).toBe(false);
    expect(events[events.length - 1]!.type).toBe("RUN_FINISHED");
  });
});
