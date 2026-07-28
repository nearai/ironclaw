import { describe, expect, it } from "vitest";
import {
  callbackLoginTicket,
  HostedControlApi,
  hostedControlOrigin,
  hostedLoginUrl,
  preferredIronClawInstance
} from "./hosted";

describe("hosted account bootstrap", () => {
  it("routes production and staging through their account control planes", () => {
    expect(hostedControlOrigin("https://agent.near.ai")).toBe("https://private.near.ai");
    expect(hostedControlOrigin("https://agent-stg.near.ai")).toBe(
      "https://private-chat-stg.near.ai"
    );
  });

  it("builds the hosted OAuth URL and reads query or fragment login tickets", () => {
    const url = hostedLoginUrl(
      "https://private.near.ai",
      "github",
      "ironclaw://auth/callback"
    );
    expect(url).toContain("/v1/auth/github");
    expect(url).toContain("oauth_channel=mobile");
    expect(callbackLoginTicket("ironclaw://auth/callback?login_ticket=query")).toBe("query");
    expect(callbackLoginTicket("ironclaw://auth/callback#login_ticket=fragment")).toBe("fragment");
    expect(callbackLoginTicket("ironclaw://auth/callback?token=bearer")).toBe("");
  });

  it("selects a running IronClaw deployment", () => {
    expect(
      preferredIronClawInstance([
        { id: "open", service_type: "openclaw", dashboard_url: "https://open.example" },
        {
          id: "stopped",
          service_type: "ironclaw",
          status: "stopped",
          dashboard_url: "https://stopped.example"
        },
        {
          id: "running",
          service_type: "ironclaw-dind",
          status: "running",
          dashboard_url: "https://running.example"
        }
      ])?.id
    ).toBe("running");
  });

  it("exchanges a hosted login ticket without putting a bearer in the callback URL", async () => {
    const originalFetch = globalThis.fetch;
    const calls: Array<{ url: string; init?: RequestInit }> = [];
    globalThis.fetch = (async (url: string | URL | Request, init?: RequestInit) => {
      calls.push({ url: String(url), init });
      return new Response(JSON.stringify({ token: "account-token" }), {
        status: 200,
        headers: { "Content-Type": "application/json" }
      });
    }) as typeof fetch;
    try {
      await expect(
        new HostedControlApi("https://private.near.ai", "").exchangeLoginTicket("ticket-1")
      ).resolves.toBe("account-token");
    } finally {
      globalThis.fetch = originalFetch;
    }

    expect(calls).toHaveLength(1);
    expect(calls[0]?.url).toBe("https://private.near.ai/auth/session/exchange");
    expect(calls[0]?.init?.body).toBe(JSON.stringify({ ticket: "ticket-1" }));
    const headers = new Headers(calls[0]?.init?.headers);
    expect(headers.get("Authorization")).toBeNull();
  });
});
