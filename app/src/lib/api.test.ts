import { describe, expect, it, vi } from "vitest";
import { IronClawApi, providerLoginUrl } from "./api";

describe("IronClawApi", () => {
  it("keeps the bearer on the configured origin", async () => {
    const fetchMock = vi.fn(async (_input: string | URL | Request, _init?: RequestInit) =>
      new Response(JSON.stringify({ tenant_id: "t", user_id: "u" }), {
        status: 200,
        headers: { "content-type": "application/json" }
      })
    );
    vi.stubGlobal("fetch", fetchMock);
    await new IronClawApi("https://agent-stg.near.ai/", "secret").session();
    expect(fetchMock).toHaveBeenCalledOnce();
    const [url, init] = fetchMock.mock.calls[0]!;
    expect(url).toBe("https://agent-stg.near.ai/api/webchat/v2/session");
    expect(new Headers(init?.headers).get("Authorization")).toBe("Bearer secret");
  });

  it("builds hosted provider login URLs without putting a bearer in the URL", () => {
    const url = providerLoginUrl(
      "https://agent.near.ai",
      "github",
      "ironclaw://auth/callback"
    );
    expect(url).toContain("https://agent.near.ai/auth/login/github");
    expect(url).toContain("redirect_after=ironclaw%3A%2F%2Fauth%2Fcallback");
    expect(url).not.toContain("token");
  });

  it("rejects a frontend HTML fallback instead of treating it as API data", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () =>
        new Response("<!doctype html><title>Frontend</title>", {
          status: 200,
          headers: { "content-type": "text/html" }
        })
      )
    );
    await expect(
      new IronClawApi("https://agent-stg.near.ai", "secret").session()
    ).rejects.toThrow("does not expose the IronClaw mobile API");
  });
});
