import { describe, expect, it } from "vitest";
import { startRealStack, type RealStackHandle } from "../helpers/real-stack";

describe("live reborn stream contract", () => {
  let stack: RealStackHandle;

  it("creates thread, sends message, and reads stream events", async () => {
    stack = await startRealStack();

    try {
      const { rebornBaseUrl, rebornToken } = stack;
      const headers = { Authorization: `Bearer ${rebornToken}` };

      // List threads (should be empty)
      const listRes = await fetch(`${rebornBaseUrl}/api/webchat/v2/threads`, { headers });
      expect(listRes.status).toBe(200);
      const listBody = await listRes.json();
      expect(listBody.threads).toEqual([]);

      // Create thread
      const createRes = await fetch(`${rebornBaseUrl}/api/webchat/v2/threads`, {
        method: "POST",
        headers: { ...headers, "Content-Type": "application/json" },
        body: JSON.stringify({ client_action_id: `test-${crypto.randomUUID()}` }),
      });
      expect(createRes.status).toBe(200);
      const createBody = await createRes.json();
      const threadId = createBody.thread?.thread_id ?? createBody.thread_id;
      expect(threadId).toBeDefined();

      // List threads (should have 1)
      const listRes2 = await fetch(`${rebornBaseUrl}/api/webchat/v2/threads`, { headers });
      expect(listRes2.status).toBe(200);
      const listBody2 = await listRes2.json();
      expect(listBody2.threads).toHaveLength(1);
      expect(listBody2.threads[0].thread_id).toBe(threadId);

      // Send message
      const sendRes = await fetch(`${rebornBaseUrl}/api/webchat/v2/threads/${threadId}/messages`, {
        method: "POST",
        headers: { ...headers, "Content-Type": "application/json" },
        body: JSON.stringify({ content: "hello", client_action_id: `act-${crypto.randomUUID()}` }),
      });
      expect(sendRes.status).toBe(200);
      const sendBody = await sendRes.json();
      expect(sendBody.outcome).toBe("submitted");
      const runId = sendBody.run_id;
      expect(runId).toBeDefined();

      // Get timeline (should have message)
      const timelineRes = await fetch(
        `${rebornBaseUrl}/api/webchat/v2/threads/${threadId}/timeline`,
        { headers },
      );
      expect(timelineRes.status).toBe(200);
      const timelineBody = await timelineRes.json();
      expect(timelineBody.messages.length).toBeGreaterThan(0);

      // Get stream events via SSE (token in URL for EventSource compat)
      const sseRes = await fetch(
        `${rebornBaseUrl}/api/webchat/v2/threads/${threadId}/events?token=${rebornToken}`,
        { headers: { Accept: "text/event-stream" } },
      );
      expect(sseRes.status).toBe(200);
      expect(sseRes.headers.get("content-type")).toContain("text/event-stream");
    } finally {
      await stack.stop();
    }
  }, 120000);
});
