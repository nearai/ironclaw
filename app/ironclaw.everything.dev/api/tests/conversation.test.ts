import { describe, expect, it } from "vitest";
import { ConversationEventSchema, ConversationSendAckSchema } from "../src/contract";
import { normalizeTimelineEntry } from "../src/lib/conversation";

describe("normalizeTimelineEntry", () => {
  it("includes attachment refs when present in raw entry", () => {
    const raw = {
      message_id: "msg-1",
      thread_id: "t-1",
      kind: "User",
      content: "hello with files",
      status: "finalized",
      sequence: 5,
      attachments: [
        {
          id: "att-1",
          kind: "image",
          mime_type: "image/png",
          filename: "screenshot.png",
          size_bytes: 204800,
        },
        {
          id: "att-2",
          kind: "document",
          mime_type: "application/pdf",
          filename: "report.pdf",
        },
      ],
    };

    const result = normalizeTimelineEntry(raw, "t-1");
    expect(result.attachments).toBeDefined();
    expect(result.attachments).toHaveLength(2);
    const first = result.attachments![0]!;
    expect(first.id).toBe("att-1");
    expect(first.kind).toBe("image");
    expect(first.mimeType).toBe("image/png");
    expect(first.filename).toBe("screenshot.png");
    expect(first.sizeBytes).toBe(204800);
    const second = result.attachments![1]!;
    expect(second.id).toBe("att-2");
    expect(second.kind).toBe("document");
    expect(second.mimeType).toBe("application/pdf");
    expect(second.sizeBytes).toBeUndefined();
  });

  it("handles camelCase keys too", () => {
    const raw = {
      messageId: "msg-2",
      thread_id: "t-1",
      kind: "Assistant",
      content: "reply",
      status: "finalized",
      sequence: 6,
      attachments: [
        {
          id: "att-3",
          kind: "audio",
          mimeType: "audio/wav",
          filename: "recording.wav",
          sizeBytes: 512000,
        },
      ],
    };

    const result = normalizeTimelineEntry(raw, "t-1");
    expect(result.attachments).toBeDefined();
    expect(result.attachments).toHaveLength(1);
    const att = result.attachments![0]!;
    expect(att.id).toBe("att-3");
    expect(att.kind).toBe("audio");
  });

  it("defaults to empty array when no attachments", () => {
    const raw = {
      message_id: "msg-3",
      thread_id: "t-1",
      kind: "User",
      content: "hello",
      status: "finalized",
      sequence: 0,
    };

    const result = normalizeTimelineEntry(raw, "t-1");
    expect(result.attachments).toEqual([]);
  });

  it.each([
    { kind: "user", expectedRole: "user" },
    { kind: "User", expectedRole: "user" },
    { kind: "user_message", expectedRole: "user" },
    { kind: "assistant", expectedRole: "assistant" },
    { kind: "Assistant", expectedRole: "assistant" },
    { kind: "assistant_message", expectedRole: "assistant" },
    { kind: "tool_result", expectedRole: "assistant" },
  ])("normalizes kind=$kind to role=$expectedRole", ({ kind, expectedRole }) => {
    const raw = {
      message_id: "msg-role-1",
      thread_id: "t-1",
      kind,
      content: "test",
      status: "finalized",
      sequence: 0,
    };
    const result = normalizeTimelineEntry(raw, "t-1");
    expect(result.role).toBe(expectedRole);
  });

  it("prefers raw.role over kind when present", () => {
    const raw = {
      message_id: "msg-role-2",
      thread_id: "t-1",
      kind: "assistant",
      role: "user",
      content: "forced user text",
      status: "finalized",
      sequence: 0,
    };
    const result = normalizeTimelineEntry(raw, "t-1");
    expect(result.role).toBe("user");
  });

  it("treats unknown kind with actorId as user", () => {
    const raw = {
      message_id: "msg-unknown-actor",
      thread_id: "t-1",
      kind: "unknown_custom_kind",
      actor_id: "someone",
      content: "custom content",
      status: "finalized",
      sequence: 0,
    };
    const result = normalizeTimelineEntry(raw, "t-1");
    expect(result.role).toBe("user");
  });

  it("never coerces user-like rows into assistant", () => {
    for (const kind of ["user", "User", "user_message"]) {
      const raw = {
        message_id: `msg-nope-${kind}`,
        thread_id: "t-1",
        kind,
        content: "user text",
        status: "finalized",
        sequence: 0,
      };
      const result = normalizeTimelineEntry(raw, "t-1");
      expect(result.role).toBe("user");
    }
  });
});

describe("ConversationEventSchema", () => {
  it("accepts a snapshot event", () => {
    const input = {
      type: "snapshot",
      threadId: "t-1",
      messages: [
        {
          id: "msg-1",
          threadId: "t-1",
          role: "assistant",
          text: "hello",
          createdAt: "2024-01-01T00:00:00Z",
          status: "finalized",
          sequence: 0,
          runId: null,
        },
      ],
    };
    const result = ConversationEventSchema.parse(input);
    expect(result.type).toBe("snapshot");
    expect(result.messages).toHaveLength(1);
  });

  it("accepts a keep_alive event", () => {
    const input = { type: "keep_alive", threadId: "t-1" };
    const result = ConversationEventSchema.parse(input);
    expect(result.type).toBe("keep_alive");
  });
});

describe("ConversationSendAckSchema", () => {
  it("accepts richer fields", () => {
    const input = {
      threadId: "t-1",
      runId: "run-1",
      acceptedMessageRef: "ref-1",
      pendingMessageId: "pending-abc",
      submittedAt: "2024-01-01T00:00:00Z",
      outcome: "submitted",
      status: "running",
      activeRunId: "run-1",
      eventCursor: 1,
    };
    const result = ConversationSendAckSchema.parse(input);
    expect(result.outcome).toBe("submitted");
    expect(result.status).toBe("running");
    expect(result.activeRunId).toBe("run-1");
  });
});
