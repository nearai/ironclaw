import type { Page } from "@playwright/test";
import type { RebornAppHost } from "./reborn-app";

export async function loginAnonymously(page: Page, baseUrl: string) {
  await page.route("**/api/auth/sign-in/anonymous", async (route) => {
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        user: { id: "test-user-id", email: null, name: null, image: null, isAnonymous: true },
        session: {
          id: "test-session-id",
          expiresAt: new Date(Date.now() + 86400000).toISOString(),
          createdAt: new Date().toISOString(),
          userId: "test-user-id",
        },
      }),
    });
  });

  await page.route("**/api/auth/session", async (route) => {
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        user: { id: "test-user-id", email: null, name: null, image: null, isAnonymous: true },
        session: {
          id: "test-session-id",
          expiresAt: new Date(Date.now() + 86400000).toISOString(),
          createdAt: new Date().toISOString(),
          userId: "test-user-id",
        },
      }),
    });
  });

  await page.route("**/api/auth/context", async (route) => {
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        user: { id: "test-user-id" },
        near: { primaryAccountId: null },
        organization: { activeOrganizationId: null },
        apiKey: null,
      }),
    });
  });

  await page.goto(`${baseUrl}/login`, { waitUntil: "networkidle" });
  await page.getByRole("button", { name: /anonymous/i }).click();
  await page.waitForURL(/\/home/, { timeout: 15000 });
}

export function setupIronclawApiMock(page: Page, app: RebornAppHost) {
  const pingResponse = async (route: any) => {
    try {
      const res = await fetch(`${app.rebornBaseUrl}/api/webchat/v2/session`, {
        headers: { Authorization: `Bearer ${app.rebornToken}` },
      });
      if (!res.ok) throw new Error("mock returned error");
      await route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({ status: "ok", timestamp: new Date().toISOString() }),
      });
    } catch {
      await route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({ status: "ok", timestamp: new Date().toISOString() }),
      });
    }
  };

  const settingsResponse = {
    tunnelUrl: app.rebornBaseUrl,
    apiToken: app.rebornToken,
  };

  let savedSettings: typeof settingsResponse | null = null;

  page.route("**/api/rpc*", async (route) => {
    const url = route.request().url();
    const method = route.request().method();

    if (
      method === "POST" &&
      (url.includes("ping") || url.endsWith("/api/rpc") || url.endsWith("/api/rpc/"))
    ) {
      const body = route.request().postDataJSON
        ? await route
            .request()
            .postDataJSON()
            .catch(() => ({}))
        : {};
      const procedure = body?.procedure ?? "";

      if (typeof procedure === "string" && procedure.includes("ping")) {
        return pingResponse(route);
      }

      if (procedure.includes("settings.get")) {
        if (!savedSettings) {
          return route.fulfill({
            status: 404,
            contentType: "application/json",
            body: JSON.stringify({
              error: "NOT_FOUND",
              message: "No ironclaw settings configured",
            }),
          });
        }
        return route.fulfill({
          status: 200,
          contentType: "application/json",
          body: JSON.stringify(savedSettings),
        });
      }

      if (procedure.includes("settings.update")) {
        savedSettings = settingsResponse;
        return route.fulfill({
          status: 200,
          contentType: "application/json",
          body: JSON.stringify({ success: true }),
        });
      }

      if (procedure.includes("session")) {
        return pingResponse(route);
      }

      if (procedure.includes("logout")) {
        savedSettings = null;
        return route.fulfill({
          status: 200,
          contentType: "application/json",
          body: JSON.stringify({ success: true }),
        });
      }

      if (procedure.includes("threads.list")) {
        const res = await fetch(`${app.rebornBaseUrl}/api/webchat/v2/threads`, {
          headers: { Authorization: `Bearer ${app.rebornToken}` },
        });
        const data = await res.json();
        return route.fulfill({
          status: res.status,
          contentType: "application/json",
          body: JSON.stringify(data),
        });
      }

      if (procedure.includes("threads.create")) {
        const input = body?.input ?? {};
        const clientActionId =
          typeof input?.clientActionId === "string" ? input.clientActionId : crypto.randomUUID();
        const res = await fetch(`${app.rebornBaseUrl}/api/webchat/v2/threads`, {
          method: "POST",
          headers: {
            Authorization: `Bearer ${app.rebornToken}`,
            "Content-Type": "application/json",
          },
          body: JSON.stringify({ client_action_id: clientActionId }),
        });
        const data = await res.json();
        return route.fulfill({
          status: res.status,
          contentType: "application/json",
          body: JSON.stringify(data),
        });
      }

      if (
        procedure.includes("conversation.getMessages") ||
        procedure.includes("threads.getTimeline")
      ) {
        const input = body?.input ?? {};
        const threadId =
          typeof input?.threadId === "string"
            ? input.threadId
            : typeof input?.id === "string"
              ? input.id
              : "thread-001";
        const res = await fetch(
          `${app.rebornBaseUrl}/api/webchat/v2/threads/${threadId}/timeline`,
          {
            headers: { Authorization: `Bearer ${app.rebornToken}` },
          },
        );
        const data = await res.json();
        return route.fulfill({
          status: res.status,
          contentType: "application/json",
          body: JSON.stringify(data),
        });
      }

      if (
        procedure.includes("conversation.sendMessage") ||
        procedure.includes("threads.sendMessage")
      ) {
        const input = body?.input ?? {};
        const threadId =
          typeof input?.threadId === "string"
            ? input.threadId
            : typeof input?.id === "string"
              ? input.id
              : "thread-001";
        const content = typeof input?.content === "string" ? input.content : "";
        const clientActionId =
          typeof input?.clientActionId === "string" ? input.clientActionId : crypto.randomUUID();
        const res = await fetch(
          `${app.rebornBaseUrl}/api/webchat/v2/threads/${threadId}/messages`,
          {
            method: "POST",
            headers: {
              Authorization: `Bearer ${app.rebornToken}`,
              "Content-Type": "application/json",
            },
            body: JSON.stringify({ content, client_action_id: clientActionId }),
          },
        );
        const data = await res.json();
        return route.fulfill({
          status: res.status,
          contentType: "application/json",
          body: JSON.stringify(data),
        });
      }

      if (procedure.includes("threads.delete")) {
        const input = body?.input ?? {};
        const threadId = typeof input?.id === "string" ? input.id : "thread-001";
        const res = await fetch(`${app.rebornBaseUrl}/api/webchat/v2/threads/${threadId}`, {
          method: "DELETE",
          headers: { Authorization: `Bearer ${app.rebornToken}` },
        });
        const data = await res.json();
        return route.fulfill({
          status: res.status,
          contentType: "application/json",
          body: JSON.stringify(data),
        });
      }

      if (
        procedure.includes("conversation.live") ||
        procedure.includes("threads.streamEvents") ||
        procedure.includes("streamEvents")
      ) {
        const input = body?.input ?? {};
        const threadId =
          typeof input?.threadId === "string"
            ? input.threadId
            : typeof input?.id === "string"
              ? input.id
              : "thread-001";
        return route.fulfill({
          status: 200,
          contentType: "text/event-stream",
          body: await (
            await fetch(
              `${app.rebornBaseUrl}/api/webchat/v2/threads/${threadId}/events?token=${app.rebornToken}`,
            )
          ).text(),
        });
      }

      return pingResponse(route);
    }

    return route.continue();
  });
}
