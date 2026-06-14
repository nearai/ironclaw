import { describe, expect, it } from "vitest";
import { startRebornMock } from "../../../tests/reborn-mock/server";
import type { RebornMockHandle } from "../../../tests/reborn-mock/types";

describe("reborn-mock contract: auth and credential resolution", () => {
  let mock: RebornMockHandle;

  it("returns session with valid bearer token", async () => {
    mock = await startRebornMock({ scenario: "healthy-empty" });
    const res = await fetch(`${mock.baseUrl}/api/webchat/v2/session`, {
      headers: { Authorization: `Bearer ${mock.token}` },
    });
    expect(res.status).toBe(200);
    const body = await res.json();
    expect(body.tenant_id).toBeDefined();
    expect(body.user_id).toBeDefined();
    await mock.stop();
  });

  it("returns 401 with invalid bearer token", async () => {
    mock = await startRebornMock({ scenario: "bad-token" });
    const res = await fetch(`${mock.baseUrl}/api/webchat/v2/session`, {
      headers: { Authorization: `Bearer ${mock.token}` },
    });
    expect(res.status).toBe(401);
    await mock.stop();
  });

  it("returns 401 when no auth provided", async () => {
    mock = await startRebornMock({ scenario: "healthy-chat" });
    const res = await fetch(`${mock.baseUrl}/api/webchat/v2/session`);
    expect(res.status).toBe(401);
    await mock.stop();
  });

  it("all endpoints require auth", async () => {
    mock = await startRebornMock({ scenario: "healthy-chat" });

    const sessions = await fetch(`${mock.baseUrl}/api/webchat/v2/session`);
    expect(sessions.status).toBe(401);

    const threads = await fetch(`${mock.baseUrl}/api/webchat/v2/threads`);
    expect(threads.status).toBe(401);

    const skills = await fetch(`${mock.baseUrl}/api/webchat/v2/skills`);
    expect(skills.status).toBe(401);

    const extensions = await fetch(`${mock.baseUrl}/api/webchat/v2/extensions`);
    expect(extensions.status).toBe(401);

    const automations = await fetch(`${mock.baseUrl}/api/webchat/v2/automations`);
    expect(automations.status).toBe(401);

    const outboundTargets = await fetch(`${mock.baseUrl}/api/webchat/v2/outbound/targets`);
    expect(outboundTargets.status).toBe(401);

    const channels = await fetch(`${mock.baseUrl}/api/webchat/v2/channels/connectable`);
    expect(channels.status).toBe(401);

    await mock.stop();
  });

  it("auth providers endpoint returns providers", async () => {
    mock = await startRebornMock({ scenario: "healthy-chat" });
    const res = await fetch(`${mock.baseUrl}/auth/providers`, {
      headers: { Authorization: `Bearer ${mock.token}` },
    });
    expect(res.status).toBe(200);
    const body = await res.json();
    expect(Array.isArray(body)).toBe(true);
    expect(body.length).toBeGreaterThan(0);
    await mock.stop();
  });

  it("logout endpoint returns success", async () => {
    mock = await startRebornMock({ scenario: "healthy-chat" });
    const res = await fetch(`${mock.baseUrl}/auth/logout`, {
      method: "POST",
      headers: { Authorization: `Bearer ${mock.token}` },
    });
    expect(res.status).toBe(200);
    expect((await res.json()).success).toBe(true);
    await mock.stop();
  });

  it("exchange login ticket returns session token", async () => {
    mock = await startRebornMock();
    const res = await fetch(`${mock.baseUrl}/auth/session/exchange`, {
      method: "POST",
      headers: { Authorization: `Bearer ${mock.token}`, "Content-Type": "application/json" },
      body: JSON.stringify({ login_ticket: "test-ticket" }),
    });
    expect(res.status).toBe(200);
    const body = await res.json();
    expect(body.token).toBeDefined();
    await mock.stop();
  });

  it("SSE ?token= query parameter is accepted for auth", async () => {
    mock = await startRebornMock({ scenario: "healthy-chat" });
    const res = await fetch(`${mock.baseUrl}/api/webchat/v2/threads/thread-001/events?token=${mock.token}`);
    expect(res.status).toBe(200);
    expect(res.headers.get("content-type")).toContain("text/event-stream");
    await mock.stop();
  });

  it("SSE rejects invalid ?token=", async () => {
    mock = await startRebornMock({ scenario: "healthy-chat" });
    const res = await fetch(`${mock.baseUrl}/api/webchat/v2/threads/thread-001/events?token=wrong-token`);
    expect(res.status).toBe(401);
    await mock.stop();
  });
});
