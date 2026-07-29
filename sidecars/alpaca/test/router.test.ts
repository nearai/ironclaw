import { describe, expect, it, vi } from "vitest";

import { TOKEN_HEADER, WIRE_VERSION } from "../src/protocol.ts";
import { handle, type RouterOptions } from "../src/router.ts";

const TOKEN = "per-boot-token-value";

function options(overrides: Partial<RouterOptions> = {}): RouterOptions {
  return {
    token: TOKEN,
    buildVersion: "test",
    apis: new Map([
      [
        "ethereum_sepolia",
        {
          craftTransaction: vi.fn(async () => "0xdeadbeef"),
          broadcast: vi.fn(async () => "0xtxhash"),
        } as Record<string, unknown>,
      ],
    ]),
    ...overrides,
  };
}

const authed = { [TOKEN_HEADER]: TOKEN };

function envelope(params: unknown, currencyId = "ethereum_sepolia") {
  return JSON.stringify({ version: WIRE_VERSION, currencyId, params });
}

describe("alpaca router", () => {
  it("serves health and version without a token", async () => {
    const health = await handle(options(), "GET", "/healthz", {}, "");
    expect(health.status).toBe(200);
    const version = await handle(options(), "GET", "/version", {}, "");
    expect(version.status).toBe(200);
    expect(JSON.parse(version.body).result).toEqual({ wire: WIRE_VERSION, build: "test" });
  });

  it("refuses every method call without the token", async () => {
    const response = await handle(options(), "POST", "/v1/craftTransaction", {}, envelope({}));
    expect(response.status).toBe(401);
    expect(JSON.parse(response.body).code).toBe("unauthorized");
  });

  it("refuses a wrong token", async () => {
    const response = await handle(
      options(),
      "POST",
      "/v1/craftTransaction",
      { [TOKEN_HEADER]: "not-the-token" },
      envelope({}),
    );
    expect(response.status).toBe(401);
  });

  /**
   * Auth is checked before the path is validated, so an unauthenticated caller
   * cannot use the response to learn which methods exist.
   */
  it("does not let an unauthenticated caller enumerate methods", async () => {
    const known = await handle(options(), "POST", "/v1/craftTransaction", {}, "{}");
    const unknown = await handle(options(), "POST", "/v1/definitelyNotAMethod", {}, "{}");
    expect(known.status).toBe(unknown.status);
    expect(known.body).toBe(unknown.body);
  });

  it("rejects an unsupported wire version rather than best-effort parsing", async () => {
    const body = JSON.stringify({ version: 999, currencyId: "ethereum_sepolia", params: {} });
    const response = await handle(options(), "POST", "/v1/craftTransaction", authed, body);
    expect(response.status).toBe(400);
    expect(JSON.parse(response.body).message).toContain("unsupported wire version");
  });

  it("rejects malformed JSON and missing envelope fields", async () => {
    const bad = await handle(options(), "POST", "/v1/craftTransaction", authed, "not json");
    expect(bad.status).toBe(400);

    const noCurrency = JSON.stringify({ version: WIRE_VERSION, params: {} });
    const missing = await handle(options(), "POST", "/v1/craftTransaction", authed, noCurrency);
    expect(missing.status).toBe(400);
  });

  it("refuses a currency it was not configured for", async () => {
    const response = await handle(
      options(),
      "POST",
      "/v1/craftTransaction",
      authed,
      envelope({}, "ethereum_mainnet"),
    );
    expect(response.status).toBe(404);
    expect(JSON.parse(response.body).code).toBe("unsupported_chain");
  });

  it("dispatches to the chain api and returns its result", async () => {
    const opts = options();
    const response = await handle(opts, "POST", "/v1/craftTransaction", authed, envelope({ nonce: 7 }));
    expect(response.status).toBe(200);
    expect(JSON.parse(response.body)).toEqual({
      version: WIRE_VERSION,
      ok: true,
      result: "0xdeadbeef",
    });
    const api = opts.apis.get("ethereum_sepolia") as Record<string, ReturnType<typeof vi.fn>>;
    // Mock hygiene: assert the params reached the api unchanged.
    expect(api.craftTransaction).toHaveBeenCalledWith({ nonce: 7 });
  });

  it("only accepts POST for method calls", async () => {
    const response = await handle(options(), "GET", "/v1/craftTransaction", authed, "");
    expect(response.status).toBe(400);
  });

  /**
   * An upstream RPC error must not leak its message across the wire: endpoint
   * detail belongs in the sidecar's log, and the Rust caller only needs the
   * category to map onto its own fail-closed taxonomy.
   */
  it("sanitizes upstream failures", async () => {
    const consoleError = vi.spyOn(console, "error").mockImplementation(() => {});
    const opts = options({
      apis: new Map([
        [
          "ethereum_sepolia",
          {
            broadcast: vi.fn(async () => {
              throw new Error("connect ECONNREFUSED https://secret-rpc.internal:8545");
            }),
          } as Record<string, unknown>,
        ],
      ]),
    });
    const response = await handle(opts, "POST", "/v1/broadcast", authed, envelope({}));
    expect(response.status).toBe(502);
    const payload = JSON.parse(response.body);
    expect(payload.code).toBe("upstream");
    expect(payload.message).not.toContain("secret-rpc.internal");
    expect(payload.message).not.toContain("ECONNREFUSED");
    consoleError.mockRestore();
  });
});
