import { afterAll, beforeAll, describe, expect, it } from "vitest";
import {
  getRuntimeRemoteScenarios,
  type RuntimeRemoteHost,
  startRuntimeRemoteHost,
} from "../helpers/runtime-remote";

async function expectJsonResponse(response: Response, expectedStatus = 200) {
  const contentType = response.headers.get("content-type") ?? "";
  const body = await response.text();

  expect(response.status).toBe(expectedStatus);

  try {
    return JSON.parse(body) as Record<string, unknown>;
  } catch {
    throw new Error(
      `Expected JSON response but received ${contentType || "unknown content-type"}: ${body.slice(0, 300)}`,
    );
  }
}

async function readText(response: Response) {
  const text = await response.text();
  expect(text.trim().length).toBeGreaterThan(0);
  return text;
}

const scenarios = await getRuntimeRemoteScenarios();

for (const scenario of scenarios) {
  const suite = scenario.available ? describe : describe.skip;

  suite(`Remote runtime smoke: ${scenario.title}`, () => {
    let runtime: RuntimeRemoteHost;

    beforeAll(async () => {
      runtime = await startRuntimeRemoteHost(scenario);
    }, 60000);

    afterAll(async () => {
      await runtime?.stop();
    });

    it("serves host health", async () => {
      const response = await fetch(`${runtime.baseUrl}/health`);

      expect(response.status).toBe(200);
      expect(await response.text()).toBe("OK");
    });

    it("serves remote documentation assets through the host", async () => {
      const [skillResponse, llmsResponse] = await Promise.all([
        fetch(`${runtime.baseUrl}/skill.md`),
        fetch(`${runtime.baseUrl}/llms.txt`),
      ]);

      expect(skillResponse.status).toBe(200);
      expect(llmsResponse.status).toBe(200);

      await Promise.all([readText(skillResponse), readText(llmsResponse)]);
    });

    it("exposes the runtime boot contract on /login", async () => {
      const response = await fetch(`${runtime.baseUrl}/login`);
      const html = await response.text();

      expect(response.status).toBe(200);
      expect(html).toContain("window.__RUNTIME_CONFIG__");
      expect(html).toContain("remoteEntry.js");
      expect(html).toContain('"repository"');

      if (scenario.ssr) {
        expect(html).toContain("connect to everything");
        expect(html).not.toContain("Server Error");
      }
    });

    it("routes api ping through the host", async () => {
      const path = scenario.proxy ? "/api/ping" : "/api/_health";
      const response = await fetch(`${runtime.baseUrl}${path}`);
      const body = await expectJsonResponse(response);

      if (scenario.proxy) {
        expect(body).toMatchObject({ status: "ok" });
      } else {
        expect(body).toMatchObject({ status: "ready" });
      }
    });
  });
}

describe("Remote runtime smoke config", () => {
  it("declares at least one configured scenario", async () => {
    expect(scenarios.length).toBeGreaterThan(0);
  });

  it("reports why unconfigured scenarios are skipped", async () => {
    for (const scenario of scenarios.filter((candidate) => !candidate.available)) {
      expect(scenario.skipReason).toBeTruthy();
    }
  });
});
