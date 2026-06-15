import { describe, expect, it } from "vitest";
import { messagesToUIMessages, restMessageToParts } from "./ironclaw-message-parts";
import type { ConversationMessageType } from "../../../api/src/contract";

function asToolCall(part: unknown): { type: string; id: string } {
  return part as { type: string; id: string };
}

function msg(overrides: Partial<ConversationMessageType>): ConversationMessageType {
  return {
    id: "",
    threadId: "t-1",
    role: "user",
    text: "",
    createdAt: null,
    status: "finalized",
    sequence: 0,
    runId: null,
    attachments: undefined,
    ...overrides,
  };
}

describe("restMessageToParts", () => {
  it("renders simple tool envelopes as tool parts", () => {
    const parts = restMessageToParts(
      "assistant",
      JSON.stringify({
        title: "Search web",
        input_summary: "ironclaw",
        output: "found it",
        output_kind: "text",
        truncated: false,
      }),
      { toolCallIdFallback: "msg-1" },
    );

    expect(parts).toHaveLength(2);
    expect(asToolCall(parts[0]).type).toBe("tool-call");
    expect(asToolCall(parts[0]).id).toBe("msg-1");
    expect(asToolCall(parts[1]).type).toBe("tool-result");
  });

  it("renders versioned tool envelopes as tool parts", () => {
    const parts = restMessageToParts(
      "assistant",
      JSON.stringify({
        version: 1,
        capability_id: "search-web",
        invocation_id: "inv-1",
        title: "Search web",
        input_summary: "ironclaw",
        output_summary: "found it",
        output_kind: "text",
        truncated: false,
      }),
    );

    expect(parts).toHaveLength(2);
    expect(asToolCall(parts[0]).type).toBe("tool-call");
    expect(asToolCall(parts[0]).id).toBe("inv-1");
  });
});

describe("messagesToUIMessages", () => {
  it("groups consecutive assistant messages with same runId into one UIMessage", () => {
    const result = messagesToUIMessages([
      msg({ id: "user-1", role: "user", text: "hello", runId: null }),
      msg({
        id: "tool-1",
        role: "assistant",
        text: JSON.stringify({ version: 1, capability_id: "read_file", invocation_id: "inv-1", title: "Read file", output_summary: "content", output_kind: "text", truncated: false }),
        runId: "run-1",
        createdAt: "2025-01-01T00:00:00Z",
      }),
      msg({
        id: "reply-1",
        role: "assistant",
        text: "Here is the file content",
        runId: "run-1",
        createdAt: "2025-01-01T00:00:01Z",
      }),
    ]);

    expect(result).toHaveLength(2);
    expect(result[0].role).toBe("user");
    expect(result[1].role).toBe("assistant");
    expect(result[1].id).toBe("assistant:run-1");
    expect(result[1].parts).toHaveLength(3);
    expect(result[1].parts[0].type).toBe("tool-call");
    expect(result[1].parts[1].type).toBe("tool-result");
    expect(result[1].parts[2].type).toBe("text");
    expect((result[1].parts[2] as any).content).toBe("Here is the file content");
  });

  it("does not group assistant messages with different runIds", () => {
    const result = messagesToUIMessages([
      msg({
        id: "tool-1",
        role: "assistant",
        text: JSON.stringify({ title: "Tool A", output: "a", truncated: false }),
        runId: "run-1",
      }),
      msg({
        id: "tool-2",
        role: "assistant",
        text: JSON.stringify({ title: "Tool B", output: "b", truncated: false }),
        runId: "run-2",
      }),
    ]);

    expect(result).toHaveLength(2);
    expect(result[0].id).toBe("assistant:run-1");
    expect(result[1].id).toBe("assistant:run-2");
  });

  it("does not group assistant messages without runId", () => {
    const result = messagesToUIMessages([
      msg({ id: "msg-1", role: "assistant", text: "previous message", runId: null }),
      msg({ id: "msg-2", role: "assistant", text: "another message", runId: null }),
    ]);

    expect(result).toHaveLength(2);
    expect(result[0].id).toBe("msg-1");
    expect(result[1].id).toBe("msg-2");
  });

  it("does not group across a user message boundary", () => {
    const result = messagesToUIMessages([
      msg({
        id: "tool-1",
        role: "assistant",
        text: JSON.stringify({ title: "Tool A", output: "a", truncated: false }),
        runId: "run-1",
      }),
      msg({ id: "user-1", role: "user", text: "follow up", runId: null }),
      msg({
        id: "reply-1",
        role: "assistant",
        text: "follow up reply",
        runId: "run-2",
      }),
    ]);

    expect(result).toHaveLength(3);
    expect(result[0].id).toBe("assistant:run-1");
    expect(result[0].parts).toHaveLength(2);
    expect(result[1].role).toBe("user");
    expect(result[2].id).toBe("assistant:run-2");
  });

  it("preserves submitted (pending) messages instead of filtering them out", () => {
    const result = messagesToUIMessages([
      msg({ id: "user-1", role: "user", text: "hello", runId: null }),
      msg({ id: "pending-1", role: "user", text: "pending message", status: "submitted", runId: null }),
      msg({ id: "assistant-1", role: "assistant", text: "reply", runId: "run-1" }),
      msg({ id: "pending-assistant", role: "assistant", text: "still thinking", status: "submitted", runId: "run-2" }),
    ]);

    // All msgs preserved: user-1, pending-1, assistant-1, pending-assistant
    // Note: pending assistant with runId gets grouped as assistant:run-2 by grouping logic
    expect(result).toHaveLength(4);
    expect(result[0].id).toBe("user-1");
    expect(result[1].id).toBe("pending-1");
    expect(result[1].role).toBe("user");
    expect(result[2].id).toBe("assistant:run-1");
    expect(result[3].id).toBe("assistant:run-2");
    expect(result[3].role).toBe("assistant");
  });
});
