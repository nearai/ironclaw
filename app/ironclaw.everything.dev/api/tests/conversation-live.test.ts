import { describe, expect, it, vi } from "vitest";
import { createThreadChatBridge } from "../src/lib/conversation-live";

function event(type: string, overrides: Record<string, unknown> = {}) {
  return { type, ...overrides };
}

function mockSendMessage() {
  return { runId: "test-run-1", activeRunId: "test-run-1", eventCursor: 0 };
}

function mockIc(events: any[], timelineData?: any[]) {
  const ic = {
    threads: {
      sendMessage: vi.fn().mockResolvedValue(mockSendMessage()),
      streamEvents: vi.fn().mockResolvedValue(
        (async function* () {
          for (const e of events) yield e;
        })(),
      ),
      getTimeline: vi.fn().mockResolvedValue(timelineData ? { data: timelineData } : { data: [] }),
    },
  };

  return { ic };
}

async function collectEvents(
  handler: ReturnType<typeof createThreadChatBridge>,
  input: { threadId: string; messages?: any[] },
) {
  const gen = handler({
    input: {
      threadId: input.threadId,
      messages: input.messages ?? [{ id: "test-1", role: "user", content: "test" }],
    },
    signal: new AbortController().signal,
    context: {},
  });
  const events: any[] = [];
  for await (const e of gen) events.push(e);
  return events;
}

describe("createThreadChatBridge", () => {
  it("sends message and streams events into AG-UI chunks", async () => {
    const { ic } = mockIc([
      event("accepted", {
        ack: { runId: "test-run-1", activeRunId: "test-run-1", threadId: "thread-1" },
      }),
      event("final_reply", {
        reply: { text: "hello from the bridge", turnRunId: "test-run-1" },
      }),
    ]);

    const handler = createThreadChatBridge({ ironclaw: () => ic as any });
    const events = await collectEvents(handler, { threadId: "thread-1" });

    expect(ic.threads.sendMessage).toHaveBeenCalledTimes(1);
    expect(events[0]!.type).toBe("RUN_STARTED");
    expect(events.some((e) => e.type === "TEXT_MESSAGE_CONTENT")).toBe(true);
    expect(events[events.length - 1]!.type).toBe("RUN_FINISHED");
  });

  it("emits approval-requested for gate events", async () => {
    const { ic } = mockIc([
      event("accepted", {
        ack: { runId: "test-run-1", activeRunId: "test-run-1", threadId: "thread-1" },
      }),
      event("gate", {
        prompt: {
          turnRunId: "test-run-1",
          gateRef: "gate-1",
          headline: "Need approval",
          body: "Approve?",
          approvalContext: { toolName: "shell", action: "run", scope: "thread" },
        },
      }),
    ]);

    const handler = createThreadChatBridge({ ironclaw: () => ic as any });
    const events = await collectEvents(handler, { threadId: "thread-1" });

    expect(events.some((e) => e.type === "CUSTOM" && e.name === "approval-requested")).toBe(true);
  });

  it("processes projection items into AG-UI chunks", async () => {
    const { ic } = mockIc([
      event("accepted", {
        ack: { runId: "test-run-1", activeRunId: "test-run-1", threadId: "thread-1" },
      }),
      event("projection_snapshot", {
        state: {
          items: [
            { text: { id: "txt-1", body: "projection says hello" } },
            { runStatus: { runId: "test-run-1", status: "completed" } },
          ],
        },
      }),
    ]);

    const handler = createThreadChatBridge({ ironclaw: () => ic as any });
    const events = await collectEvents(handler, { threadId: "thread-1" });

    expect(events.some((e) => e.type === "TEXT_MESSAGE_CONTENT")).toBe(true);
    expect(events.some((e) => e.type === "RUN_FINISHED")).toBe(true);
  });
});
