import { describe, expect, it } from "vitest";
import { startRebornMock } from "../../../tests/reborn-mock/server";
import type { RebornMockHandle } from "../../../tests/reborn-mock/types";

describe("ironclaw chat contract (via mock Reborn)", () => {
  let mock: RebornMockHandle;

  it("create thread returns thread_id and title", async () => {
    mock = await startRebornMock();
    const res = await fetch(`${mock.baseUrl}/api/webchat/v2/threads`, {
      method: "POST",
      headers: { Authorization: `Bearer ${mock.token}`, "Content-Type": "application/json" },
      body: JSON.stringify({ client_action_id: `t-${crypto.randomUUID()}` }),
    });
    expect(res.status).toBe(200);
    const body = await res.json();
    const threadId = body.thread?.thread_id ?? body.thread_id;
    expect(threadId).toBeDefined();
    expect(typeof threadId).toBe("string");
    await mock.stop();
  });

  it("send message returns accepted with runId", async () => {
    mock = await startRebornMock({ scenario: "healthy-chat" });
    const res = await fetch(`${mock.baseUrl}/api/webchat/v2/threads/thread-001/messages`, {
      method: "POST",
      headers: { Authorization: `Bearer ${mock.token}`, "Content-Type": "application/json" },
      body: JSON.stringify({ content: "hello", client_action_id: `t-${crypto.randomUUID()}` }),
    });
    expect(res.status).toBe(200);
    const body = await res.json();
    expect(body.outcome).toBe("submitted");
    expect(body.run_id).toBeDefined();
    expect(body.accepted_message_ref).toBeDefined();
    expect(body.status).toBe("running");
    expect(body.event_cursor).toBeDefined();
    await mock.stop();
  });

  it("list threads after create returns the new thread", async () => {
    mock = await startRebornMock({ scenario: "healthy-empty" });
    const createRes = await fetch(`${mock.baseUrl}/api/webchat/v2/threads`, {
      method: "POST",
      headers: { Authorization: `Bearer ${mock.token}`, "Content-Type": "application/json" },
      body: JSON.stringify({ client_action_id: `t-${crypto.randomUUID()}` }),
    });
    expect(createRes.status).toBe(200);
    const createBody = await createRes.json();
    const threadId = createBody.thread?.thread_id ?? createBody.thread_id;

    const listRes = await fetch(`${mock.baseUrl}/api/webchat/v2/threads`, {
      headers: { Authorization: `Bearer ${mock.token}` },
    });
    expect(listRes.status).toBe(200);
    const listBody = await listRes.json();
    expect(listBody.threads).toHaveLength(1);
    expect(listBody.threads[0].thread_id).toBe(threadId);
    await mock.stop();
  });

  it("timeline returns messages after send", async () => {
    mock = await startRebornMock({ scenario: "healthy-chat" });
    await fetch(`${mock.baseUrl}/api/webchat/v2/threads/thread-001/messages`, {
      method: "POST",
      headers: { Authorization: `Bearer ${mock.token}`, "Content-Type": "application/json" },
      body: JSON.stringify({ content: "hello", client_action_id: `t-${crypto.randomUUID()}` }),
    });

    const timelineRes = await fetch(`${mock.baseUrl}/api/webchat/v2/threads/thread-001/timeline`, {
      headers: { Authorization: `Bearer ${mock.token}` },
    });
    expect(timelineRes.status).toBe(200);
    const timelineBody = await timelineRes.json();
    expect(timelineBody.messages.length).toBeGreaterThan(0);
    await mock.stop();
  });

  it("SSE stream returns accepted and final_reply events", async () => {
    mock = await startRebornMock({ scenario: "stream-final-reply" });
    const sseRes = await fetch(
      `${mock.baseUrl}/api/webchat/v2/threads/thread-001/events?token=${mock.token}`,
    );
    expect(sseRes.status).toBe(200);
    expect(sseRes.headers.get("content-type")).toContain("text/event-stream");

    const text = await sseRes.text();
    const hasAccepted = text.includes('"accepted"') || text.includes("event: accepted");
    const hasFinalReply = text.includes("final_reply");
    expect(hasAccepted).toBe(true);
    expect(hasFinalReply).toBe(true);
    await mock.stop();
  });
});
