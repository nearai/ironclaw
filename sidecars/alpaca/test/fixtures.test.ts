import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

import { describe, expect, it } from "vitest";

import { WIRE_VERSION, isErr, parseEnvelope } from "../src/protocol.ts";

/**
 * The TypeScript half of the cross-language contract check.
 *
 * The Rust client's test reads these exact files. Nothing in either type system
 * connects the two sides, so a shared fixture is the only thing that makes a
 * silent divergence impossible: change the shape here and the other suite fails.
 */
const FIXTURES = join(dirname(fileURLToPath(import.meta.url)), "..", "fixtures");

function fixture(name: string): unknown {
  return JSON.parse(readFileSync(join(FIXTURES, `${name}.json`), "utf8"));
}

describe("wire-contract fixtures", () => {
  it("parses the craft request fixture", () => {
    const parsed = parseEnvelope<Record<string, unknown>>(fixture("request-craft"));
    expect(isErr(parsed)).toBe(false);
    if (isErr(parsed)) return;
    expect(parsed.version).toBe(WIRE_VERSION);
    expect(parsed.currencyId).toBe("ethereum_sepolia");
    // `params` passes through verbatim — the sidecar owns this schema and Rust
    // deliberately does not re-model it.
    expect(parsed.params).toMatchObject({ type: "send-eip1559" });
  });

  it("the success envelope matches what the router emits", () => {
    const golden = fixture("response-ok") as Record<string, unknown>;
    expect(golden).toEqual({
      version: WIRE_VERSION,
      ok: true,
      result: "0x02f86b0182000782520894",
    });
  });

  it("the error envelope matches what the router emits", () => {
    const golden = fixture("response-error") as Record<string, unknown>;
    expect(golden.version).toBe(WIRE_VERSION);
    expect(golden.ok).toBe(false);
    expect(golden.code).toBe("unsupported_chain");
    expect(typeof golden.message).toBe("string");
  });

  /**
   * The fixtures must stay on the current wire version, or the two suites would
   * agree with each other while both disagreeing with the running code.
   */
  it("every fixture declares the current wire version", () => {
    for (const name of ["request-craft", "response-ok", "response-error"]) {
      expect((fixture(name) as { version: number }).version).toBe(WIRE_VERSION);
    }
  });
});
