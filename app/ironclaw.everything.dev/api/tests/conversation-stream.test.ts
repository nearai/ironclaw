import { describe, expect, it, vi } from "vitest";
import { createConversationStreamHandler } from "../src/lib/conversation-stream";

function makeTimelineResponse(messages: any[]) {
  return {
    data: messages.map((m) => ({
      message_id: m.id,
      thread_id: "t-1",
      kind: m.role === "assistant" ? "Assistant" : "User",
      content: m.text ?? "",
      status: m.status ?? "finalized",
      sequence: m.sequence ?? 0,
      turn_run_id: m.runId ?? undefined,
    })),
    meta: { total: messages.length, has_more: false, next_cursor: null },
  };
}

function mockIc(options: {
  events?: any[];
  timelineOverride?: () => any;
}) {
  let getTimelineCallCount = 0;

  const getTimeline = options.timelineOverride ?? (() => makeTimelineResponse([]));

  const ic = {
    threads: {
      streamEvents: vi.fn().mockResolvedValue(
        (async function* () {
          for (const event of options.events ?? []) {
            yield event;
          }
        })(),
      ),
      getTimeline: vi.fn().mockImplementation(async () => {
        getTimelineCallCount++;
        return getTimeline();
      }),
    },
  };

  return { ic, getTimelineCallCount: () => getTimelineCallCount };
}

async function collectEvents(
  handler: ReturnType<typeof createConversationStreamHandler>,
  input: { threadId: string; afterCursor?: string },
) {
  const gen = handler({ input, signal: new AbortController().signal, context: {} });
  const events: any[] = [];
  for await (const event of gen) {
    events.push(event);
  }
  return events;
}

function event(type: string, overrides: Record<string, unknown> = {}) {
  return { type, ...overrides };
}

function passthroughEvent(type: string) {
  return event(type, {
    ack: { outcome: "accepted", status: "running" },
    runState: { runId: "run-1", status: "running" },
  });
}

describe("createConversationStreamHandler", () => {
  it("skips accepted, running, and other runtime events", async () => {
    const { ic } = mockIc({
      events: [event("accepted"), event("running"), event("final_reply", { reply: { text: "ok" } })],
    });
    const handler = createConversationStreamHandler({ ironclaw: () => ic as any });
    const events = await collectEvents(handler, { threadId: "t-1" });

    // Runtime events are no longer forwarded by the passive stream
    expect(events).toHaveLength(0);
  });

  it("forwards keep_alive events unchanged", async () => {
    const { ic } = mockIc({ events: [event("keep_alive")] });
    const handler = createConversationStreamHandler({ ironclaw: () => ic as any });
    const events = await collectEvents(handler, { threadId: "t-1" });

    expect(events).toHaveLength(1);
    expect(events[0]!.type).toBe("keep_alive");
  });

  it("skips getTimeline when projection event arrives during active run", async () => {
    const { ic, getTimelineCallCount } = mockIc({
      events: [
        event("accepted"),
        event("projection_update"),
        event("running"),
        event("projection_snapshot"),
        event("final_reply", { reply: { text: "done" } }),
      ],
      timelineOverride: () => makeTimelineResponse([
        { id: "msg-1", role: "assistant", text: "done", sequence: 0, runId: "run-1" },
      ]),
    });

    const handler = createConversationStreamHandler({ ironclaw: () => ic as any });
    await collectEvents(handler, { threadId: "t-1" });

    // First projection during active run: no call
    // Second projection during active run: no call
    // Terminal event triggers reconcile because needsReconcile was set: one call
    expect(getTimelineCallCount()).toBe(1);
  });

  it("calls getTimeline for idle projection events", async () => {
    const { ic, getTimelineCallCount } = mockIc({
      events: [event("projection_snapshot")],
    });

    const handler = createConversationStreamHandler({ ironclaw: () => ic as any });
    await collectEvents(handler, { threadId: "t-1" });

    expect(getTimelineCallCount()).toBe(1);
  });

  it("emits snapshot for first idle projection event", async () => {
    const { ic } = mockIc({
      events: [event("projection_snapshot")],
      timelineOverride: () => makeTimelineResponse([
        { id: "msg-1", role: "assistant", text: "hello", sequence: 0, runId: null },
      ]),
    });

    const handler = createConversationStreamHandler({ ironclaw: () => ic as any });
    const events = await collectEvents(handler, { threadId: "t-1" });

    const snapshot = events.find((e) => e.type === "snapshot");
    expect(snapshot).toBeDefined();
    expect(snapshot!.messages).toHaveLength(1);
    expect(snapshot!.messages[0]!.id).toBe("msg-1");
  });

  it("does not call getTimeline for terminal without deferred projections", async () => {
    const { ic, getTimelineCallCount } = mockIc({
      events: [
        passthroughEvent("accepted"),
        passthroughEvent("running"),
        event("final_reply", { reply: { text: "done" } }),
      ],
    });

    const handler = createConversationStreamHandler({ ironclaw: () => ic as any });
    await collectEvents(handler, { threadId: "t-1" });

    // No projection events were deferred, so reconcile is skipped
    expect(getTimelineCallCount()).toBe(0);
  });

  it("does not forward terminal events directly", async () => {
    const { ic } = mockIc({
      events: [
        event("failed", { runState: { status: "failed", failure: "error" } }),
      ],
    });

    const handler = createConversationStreamHandler({ ironclaw: () => ic as any });
    const events = await collectEvents(handler, { threadId: "t-1" });

    // Terminal events are no longer forwarded; reconcile is skipped since no projections were deferred
    expect(events).toHaveLength(0);
  });

  it("handles error in stream connection", async () => {
    const ic = {
      threads: {
        streamEvents: vi.fn().mockRejectedValue(new Error("connection lost")),
        getTimeline: vi.fn(),
      },
    };

    const handler = createConversationStreamHandler({ ironclaw: () => ic as any });
    const events = await collectEvents(handler, { threadId: "t-1" });

    expect(events).toHaveLength(1);
    expect(events[0]!.type).toBe("error");
    expect(events[0]!.error).toBe("connection lost");
  });

  it("skips runtime events like capability_progress", async () => {
    const { ic } = mockIc({
      events: [event("capability_progress", { progress: { kind: "thinking" } })],
    });

    const handler = createConversationStreamHandler({ ironclaw: () => ic as any });
    const events = await collectEvents(handler, { threadId: "t-1" });

    expect(events).toHaveLength(0);
  });
});
