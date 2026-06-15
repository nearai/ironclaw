import { describe, expect, it } from "vitest";
import { startRebornMock } from "../../../tests/reborn-mock/server";
import type { RebornMockHandle } from "../../../tests/reborn-mock/types";

describe("reborn-mock contract: settings lifecycle", () => {
  let mock: RebornMockHandle;

  it("session endpoint returns session data with valid auth", async () => {
    mock = await startRebornMock();
    const res = await fetch(`${mock.baseUrl}/api/webchat/v2/session`, {
      headers: { Authorization: `Bearer ${mock.token}` },
    });
    expect(res.status).toBe(200);
    const body = await res.json();
    expect(body.tenant_id).toBe("test-tenant");
    expect(body.user_id).toBe("test-user");
    await mock.stop();
  });

  it("session rejects bad token", async () => {
    mock = await startRebornMock();
    const res = await fetch(`${mock.baseUrl}/api/webchat/v2/session`, {
      headers: { Authorization: "Bearer wrong-token" },
    });
    expect(res.status).toBe(401);
    expect((await res.json()).error).toBe("unauthorized");
    await mock.stop();
  });

  it("threads list is empty in healthy-empty", async () => {
    mock = await startRebornMock({ scenario: "healthy-empty" });
    const res = await fetch(`${mock.baseUrl}/api/webchat/v2/threads`, {
      headers: { Authorization: `Bearer ${mock.token}` },
    });
    expect(res.status).toBe(200);
    expect((await res.json()).threads).toEqual([]);
    await mock.stop();
  });

  it("create thread returns thread_id and title", async () => {
    mock = await startRebornMock();
    const res = await fetch(`${mock.baseUrl}/api/webchat/v2/threads`, {
      method: "POST",
      headers: { Authorization: `Bearer ${mock.token}`, "Content-Type": "application/json" },
    });
    expect(res.status).toBe(200);
    const body = await res.json();
    expect(body.thread_id).toBeDefined();
    expect(body.title).toBe("New Thread");
    await mock.stop();
  });

  it("send message to existing thread returns accepted response", async () => {
    mock = await startRebornMock({ scenario: "healthy-chat" });
    const res = await fetch(`${mock.baseUrl}/api/webchat/v2/threads/thread-001/messages`, {
      method: "POST",
      headers: { Authorization: `Bearer ${mock.token}`, "Content-Type": "application/json" },
      body: JSON.stringify({ content: "Hello" }),
    });
    expect(res.status).toBe(200);
    expect((await res.json()).outcome).toBe("submitted");
    await mock.stop();
  });

  it("delete thread removes it", async () => {
    mock = await startRebornMock({ scenario: "healthy-chat" });
    await fetch(`${mock.baseUrl}/api/webchat/v2/threads/thread-001`, {
      method: "DELETE",
      headers: { Authorization: `Bearer ${mock.token}` },
    });
    const res = await fetch(`${mock.baseUrl}/api/webchat/v2/threads`, {
      headers: { Authorization: `Bearer ${mock.token}` },
    });
    expect((await res.json()).threads).toHaveLength(0);
    await mock.stop();
  });

  it("healthy-chat scenario returns threads, automations, skills, extensions", async () => {
    mock = await startRebornMock({ scenario: "healthy-chat" });

    const threads = await (
      await fetch(`${mock.baseUrl}/api/webchat/v2/threads`, {
        headers: { Authorization: `Bearer ${mock.token}` },
      })
    ).json();
    expect(threads.threads).toHaveLength(1);
    expect(threads.threads[0].thread_id).toBe("thread-001");

    const auto = await (
      await fetch(`${mock.baseUrl}/api/webchat/v2/automations`, {
        headers: { Authorization: `Bearer ${mock.token}` },
      })
    ).json();
    expect(auto.automations).toHaveLength(1);

    const skills = await (
      await fetch(`${mock.baseUrl}/api/webchat/v2/skills`, {
        headers: { Authorization: `Bearer ${mock.token}` },
      })
    ).json();
    expect(skills.skills).toHaveLength(1);

    const ext = await (
      await fetch(`${mock.baseUrl}/api/webchat/v2/extensions`, {
        headers: { Authorization: `Bearer ${mock.token}` },
      })
    ).json();
    expect(ext.extensions).toHaveLength(1);

    await mock.stop();
  });

  it("reset restores initial state", async () => {
    mock = await startRebornMock({ scenario: "healthy-chat" });

    let body = await (
      await fetch(`${mock.baseUrl}/api/webchat/v2/threads`, {
        headers: { Authorization: `Bearer ${mock.token}` },
      })
    ).json();
    expect(body.threads).toHaveLength(1);

    mock.reset();

    body = await (
      await fetch(`${mock.baseUrl}/api/webchat/v2/threads`, {
        headers: { Authorization: `Bearer ${mock.token}` },
      })
    ).json();
    expect(body.threads).toHaveLength(1);

    await mock.stop();
  });

  it("setScenario switches state", async () => {
    mock = await startRebornMock({ scenario: "healthy-empty" });

    let body = await (
      await fetch(`${mock.baseUrl}/api/webchat/v2/threads`, {
        headers: { Authorization: `Bearer ${mock.token}` },
      })
    ).json();
    expect(body.threads).toHaveLength(0);

    mock.setScenario("healthy-chat");

    body = await (
      await fetch(`${mock.baseUrl}/api/webchat/v2/threads`, {
        headers: { Authorization: `Bearer ${mock.token}` },
      })
    ).json();
    expect(body.threads).toHaveLength(1);

    await mock.stop();
  });

  it("timeline returns messages for existing thread", async () => {
    mock = await startRebornMock({ scenario: "healthy-chat" });
    const res = await fetch(`${mock.baseUrl}/api/webchat/v2/threads/thread-001/timeline`, {
      headers: { Authorization: `Bearer ${mock.token}` },
    });
    expect(res.status).toBe(200);
    const body = await res.json();
    expect(body.messages).toHaveLength(1);
    expect(body.messages[0].content).toContain("Hello");
    await mock.stop();
  });

  it("outbound preferences endpoint works", async () => {
    mock = await startRebornMock({ scenario: "healthy-chat" });
    const res = await fetch(`${mock.baseUrl}/api/webchat/v2/outbound/preferences`, {
      headers: { Authorization: `Bearer ${mock.token}` },
    });
    expect(res.status).toBe(200);
    const body = await res.json();
    expect(body.final_reply_target).toBeDefined();
    expect(body.final_reply_target.channel).toBe("slack");
    await mock.stop();
  });

  it("skills search returns installed skills", async () => {
    mock = await startRebornMock({ scenario: "healthy-chat" });
    const res = await fetch(`${mock.baseUrl}/api/webchat/v2/skills/search`, {
      method: "POST",
      headers: { Authorization: `Bearer ${mock.token}`, "Content-Type": "application/json" },
      body: JSON.stringify({ query: "test" }),
    });
    expect(res.status).toBe(200);
    const body = await res.json();
    expect(body.installed).toHaveLength(1);
    await mock.stop();
  });
});
